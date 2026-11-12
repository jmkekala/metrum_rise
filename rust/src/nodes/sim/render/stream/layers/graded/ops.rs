// SPDX-License-Identifier: GPL-2.0-only

//! The rest of the graded state's operations (`RENDER-32-I`): setup and reset, a resident patch's
//! change, lists, failures, the install check, the preview's tiles, the edit batch's
//! reservation and dirty tiles, ported from `terrain_graded_tiles.gd` with their order of
//! effects (`_reset_pages`, `clear`, `_patch_changed`, `_take_list`, `_failed`, `_install`,
//! `_show_preview_tile`, `_drop_preview`, `_adopt_preview`, `_reserve_edit_copies`,
//! `_apply_edit`, `_dirty_tile`).

use super::copies::PlaceFailure;
use super::pages::Copy;
use super::state::{Block, Effect, Entry, Graded, LevelCaps, PageRef, PatchKey, Shape, TILE_M, Tile};
use super::tiles::cell_code;

/// A staged edit tile for the reservation: its tile, level code, step, token and atlas layers.
pub(crate) type Staged = (Tile, u8, u8, i64, u32);

impl Graded {
    /// Setup: the patch grid and per level its slot side, the most slots a page holds and the
    /// most row texels a tile takes. Drops every page and level.
    pub(crate) fn setup(&mut self, shape: Shape, levels: &[(u64, u32, u64)]) {
        self.shape = shape;
        self.pages.reset(false);
        self.caps.clear();
        for &(side, cap, max_row_texels) in levels {
            self.pages.add_level(side);
            self.caps.push(LevelCaps {
                cap,
                max_layers: max_row_texels.div_ceil(super::pages::ATLAS_WIDTH) as u32,
            });
        }
        self.reset_pages();
    }

    /// `_reset_pages`: every page dropped; the world pages of `GL0` and `GL1` made, sized by
    /// their bands' bound and the most rows a tile holds.
    pub(crate) fn reset_pages(&mut self) {
        self.pages.reset(true);
        for code in 0..4 {
            self.block_page[code].clear();
            self.block_target[code].clear();
            self.block_waiting[code].clear();
        }
        self.growth_waits.clear();
        self.growth_dirty.clear();
        self.holders.clear();
        for code in 0..self.caps.len().min(2) as u8 {
            let (slots, layers) = self.page_size(code, (0, 0), (0, 0));
            let page = self.pages.create(code, (0, 0), slots, layers);
            self.page_events();
            if let Some(page) = page {
                self.block_page[usize::from(code)].insert((0, 0), page);
                self.holders_mut(page);
            }
        }
    }

    /// `clear`'s part: every copy freed (block pages go with their last), every growth, request,
    /// refusal, token, list, count, desire and park dropped; the world pages stay.
    pub(crate) fn clear(&mut self) {
        let tiles: Vec<Tile> = self.installed.keys().copied().collect();
        for tile in tiles {
            self.free_copies(tile, false);
        }
        self.installed.clear();
        let tiles: Vec<Tile> = self.preview.keys().copied().collect();
        for tile in tiles {
            self.free_copies(tile, true);
        }
        self.preview.clear();
        for code in 0..4 {
            let targets: Vec<u32> = self.block_target[code].iter().map(|(_, &page)| page).collect();
            for page in targets {
                self.pages.release(page);
                self.page_events();
                self.holders.remove(&page);
            }
            self.block_target[code].clear();
            self.block_waiting[code].clear();
        }
        self.growth_waits.clear();
        self.growth_dirty.clear();
        let requests = &mut self.requests;
        requests.requested.clear();
        requests.tokens.clear();
        requests.edit_batch.clear();
        requests.edit_levels.clear();
        requests.refused.clear();
        requests.exhausted.clear();
        for order in &mut requests.exhausted_order {
            order.clear();
        }
        requests.tile_lists.clear();
        requests.counted.clear();
        requests.counts.clear();
        requests.page_parked.clear();
        requests.parked.clear();
        requests.parked_flags = -1;
        self.lists.clear();
        self.lists_refused.clear();
        self.levels.clear();
        self.wanted.clear();
    }

    // `_patch_tiles`: the tiles whose interior meets a rectangle (real_t corners, float math).
    fn patch_tiles(rect: [f32; 4], mut visit: impl FnMut(Tile)) {
        let end = [rect[0] + rect[2], rect[1] + rect[3]];
        let tile_m = f64::from(TILE_M);
        let first = [(f64::from(rect[0]) / tile_m).floor() as i32, (f64::from(rect[1]) / tile_m).floor() as i32];
        let last = [(f64::from(end[0]) / tile_m).ceil() as i32 - 1, (f64::from(end[1]) / tile_m).ceil() as i32 - 1];
        for z in first[1]..=last[1] {
            for x in first[0]..=last[0] {
                visit(Tile { x, z });
            }
        }
    }

    // `_patches_meeting` for a tile's rectangle: the patches whose rectangle meets it (the end
    // floored, so a patch touching it counts).
    fn tile_patches_meeting(&self, tile: Tile) -> ([i32; 2], [i32; 2]) {
        let span = self.shape.patch_span;
        let position = [tile.x as f32 * TILE_M, tile.z as f32 * TILE_M];
        let first = [0, 1].map(|axis| ((position[axis] - self.shape.world_min[axis]) / span).floor() as i32);
        let last = [0, 1].map(|axis| ((position[axis] + TILE_M - self.shape.world_min[axis]) / span).floor() as i32);
        (first, last)
    }

    /// `_patch_changed`'s part: the patch's counted tiles follow its residency (the live
    /// dictionary's, already set), its blocks' parked tiles are released; a patch that left
    /// evicts the commit-only tiles no other resident patch meets, a resident one (`rect`, its
    /// entry's rectangle) wants its installed tiles again.
    pub(crate) fn patch_changed(&mut self, key: PatchKey, rect: Option<[f32; 4]>) {
        self.recount(key);
        self.release_patch_blocks(key);
        let Some(rect) = rect else {
            let span = self.shape.patch_span;
            let origin = [
                self.shape.world_min[0] + key.0 as f32 * span,
                self.shape.world_min[1] + key.1 as f32 * span,
            ];
            let mut tiles = Vec::new();
            Self::patch_tiles([origin[0], origin[1], span, span], |tile| tiles.push(tile));
            for tile in tiles {
                if !self.installed.contains_key(&tile) || self.levels.level_of(tile).is_some() {
                    continue;
                }
                let (first, last) = self.tile_patches_meeting(tile);
                let retained = (first[1]..=last[1])
                    .any(|z| (first[0]..=last[0]).any(|x| self.resident.contains_key(&(x, z))));
                if !retained {
                    self.undesire(tile);
                }
            }
            return;
        };
        let mut tiles = Vec::new();
        Self::patch_tiles(rect, |tile| tiles.push(tile));
        for tile in tiles {
            if self.installed.contains_key(&tile) {
                self.wanted.insert(tile, ());
            }
        }
    }

    /// `_take_list`'s part: patch `key`'s list recorded and counted; its blocks' parked tiles
    /// released.
    pub(crate) fn take_list(&mut self, key: PatchKey, token: i64, tiles: Vec<Tile>) {
        self.lists.insert(key, (token, tiles));
        self.recount(key);
        self.release_patch_blocks(key);
    }

    /// `_failed`'s part for a refused list.
    pub(crate) fn list_refused(&mut self, key: PatchKey, token: i64) {
        self.lists_refused.insert(key, token);
        self.release_patch_blocks(key);
    }

    /// `_failed`'s part for a tile: its request dropped, a refusal by contract recorded (the
    /// edit batch drops it); the tile is wanted again.
    pub(crate) fn failed_tile(&mut self, key: (Tile, u8, u8), token: i64, retry: bool) {
        let tile = key.0;
        if self.requests.requested.get(&key) == Some(&token) {
            self.requests.requested.remove(&key);
        }
        if !retry {
            self.requests.refused.insert(key, token);
            self.requests.edit_batch.remove(&tile);
            self.requests.edit_levels.remove(&tile);
        }
        self.wanted.insert(tile, ());
    }

    /// `_show_preview_tile`'s placement: the preview's tile at `code`/`step`/`token`. Ok, or
    /// why it could not place (a waiting page, a pending block page: `Waits`).
    pub(crate) fn place_preview(
        &mut self,
        tile: Tile,
        level: (u8, u8, i64, bool),
        layers: u32,
        scratch: &mut Vec<(PageRef, Block)>,
    ) -> Result<(), PlaceFailure> {
        let (code, step, token, behind) = level;
        let current = self.preview.get(&tile).map(|entry| (entry.level, entry.copies.clone()));
        let same_level = current.as_ref().is_some_and(|(held, _)| *held == code);
        let held = if same_level { current.as_ref().map(|(_, copies)| copies.clone()).unwrap_or_default() } else { Vec::new() };
        let copies = match self.place_copies(tile, code, layers, &held, false, true, scratch) {
            Ok(copies) => copies,
            Err(failure) => {
                if let Some(entry) = self.preview.get_mut(&tile) {
                    entry.adoptable = false;
                }
                return Err(failure);
            }
        };
        if current.is_some() && !same_level {
            self.free_copies(tile, true);
        }
        self.preview.insert(
            tile,
            Entry { cell_code: cell_code(code, step), level: code, step, token, behind, copies, adoptable: true, walls: Vec::new() },
        );
        self.effects.push(Effect::Installed { tile, preview: true });
        Ok(())
    }

    /// `_drop_preview`'s part: the preview's tile and its copies go.
    pub(crate) fn drop_preview(&mut self, tile: Tile) -> bool {
        if !self.preview.contains_key(&tile) {
            return false;
        }
        self.free_copies(tile, true);
        self.preview.remove(&tile);
        self.effects.push(Effect::Installed { tile, preview: true });
        true
    }

    /// `_adopt_preview`: a committed tile that is the payload the preview shows takes over the
    /// preview's copies as the installed tile; nothing is uploaded.
    pub(crate) fn adopt_preview(&mut self, tile: Tile, level: (u8, u8, i64), epoch: i64) -> bool {
        let (code, step, token) = level;
        let Some(shown) = self.preview.get(&tile).cloned() else {
            return false;
        };
        if !shown.adoptable || shown.token != epoch || shown.level != code || shown.step != step {
            return false;
        }
        if self.installed.contains_key(&tile) {
            self.free_copies(tile, false);
        }
        for &(page, _) in &shown.copies {
            let holders = self.holders_mut(page);
            holders.1.remove(&tile);
            holders.0.insert(tile, ());
        }
        self.installed.insert(tile, Entry { token, adoptable: true, ..shown });
        self.preview.remove(&tile);
        self.effects.push(Effect::Installed { tile, preview: false });
        self.effects.push(Effect::Installed { tile, preview: true });
        self.hold(tile, code, step, token, false);
        true
    }

    /// `_reserve_edit_copies`: the edit batch's copies placed in the book's pages, all or none.
    /// On a failure the pages go back; a page that cannot grow exhausts its tile, which leaves
    /// the batch (Err(Some(tile)): the GDScript drops its staged result).
    pub(crate) fn reserve_edit(
        &mut self,
        staged: &[Staged],
        scratch: &mut Vec<(PageRef, Block)>,
    ) -> Result<Vec<Vec<(u32, Copy)>>, Option<Tile>> {
        let mut kept: Vec<(u32, super::pages::PageLists)> = Vec::new();
        let mut out = Vec::with_capacity(staged.len());
        let abort = |graded: &mut Graded, kept: Vec<(u32, super::pages::PageLists)>| {
            for (page, lists) in kept {
                graded.pages.page_mut(page).expect("live page").set_lists(lists);
            }
        };
        for &(tile, code, step, token, layers) in staged {
            let held: Vec<(u32, Copy)> = self
                .installed
                .get(&tile)
                .filter(|entry| entry.level == code)
                .map(|entry| entry.copies.clone())
                .unwrap_or_default();
            self.copy_pages(tile, code, scratch);
            let mut copies = Vec::with_capacity(scratch.len());
            for &(page, block) in scratch.iter() {
                let PageRef::Page(id) = page else {
                    abort(self, kept);
                    return Err(None);
                };
                if !kept.iter().any(|(at, _)| *at == id) {
                    kept.push((id, self.pages.page(id).expect("live page").lists()));
                }
                let before = held.iter().find(|(at, _)| *at == id).map(|(_, copy)| *copy);
                let live = self.pages.page(id).expect("live page");
                if !live.can_place(before, layers) {
                    let plan = (live.free_slots() == 0, !live.has_run(layers.max(1)));
                    abort(self, kept);
                    let target = self.block_target[usize::from(code)].get(&block).copied();
                    if target.is_none_or(|target| target == id) && !self.grow(code, block, layers, Some(plan)) {
                        // The explicit pool-exhaustion contract at the device or budget cap.
                        self.exhaust(tile, code, (tile, code, step), token);
                        self.requests.edit_batch.remove(&tile);
                        self.requests.edit_levels.remove(&tile);
                        return Err(Some(tile));
                    }
                    return Err(None);
                }
                copies.push((id, self.pages.page_mut(id).expect("live page").place(before, layers)));
            }
            out.push(copies);
        }
        // Every incoming holder registered before the uploads free old-level copies.
        for (&(tile, ..), copies) in staged.iter().zip(&out) {
            for &(page, _) in copies {
                self.holders_mut(page).0.insert(tile, ());
            }
        }
        Ok(out)
    }

    /// `_apply_edit` for a reset ("all"): every batched, installed, requested and refused tile
    /// dirtied, in that order. Returns them, in order, for the staged results' part.
    pub(crate) fn dirty_all(&mut self, serial: i64, out: &mut Vec<Tile>) {
        out.clear();
        let batch: Vec<Tile> = self.requests.edit_batch.keys().copied().collect();
        out.extend(batch.into_iter().filter(|tile| !self.installed.contains_key(tile)));
        out.extend(self.installed.keys().copied());
        for &tile in out.iter() {
            self.dirty_tile(tile, serial);
        }
        // The requests and refusals as they stand after those (their own were dropped).
        let first = out.len();
        out.extend(self.requests.requested.keys().map(|key| key.0));
        out.extend(self.requests.refused.keys().map(|key| key.0));
        for &tile in &out[first..] {
            self.dirty_tile(tile, serial);
        }
    }

    /// `_apply_edit` for one level's dirty tiles: those held at `code`/`step`, batched,
    /// requested or refused there are dirtied. Returns them, in order.
    pub(crate) fn dirty_level(&mut self, code: u8, step: u8, tiles: &[Tile], serial: i64, out: &mut Vec<Tile>) {
        out.clear();
        for &tile in tiles {
            let held = self
                .installed
                .get(&tile)
                .is_some_and(|entry| entry.level == code && entry.step == step);
            let key = (tile, code, step);
            if held
                || self.requests.edit_batch.contains_key(&tile)
                || self.requests.requested.contains_key(&key)
                || self.requests.refused.contains_key(&key)
            {
                self.dirty_tile(tile, serial);
                out.push(tile);
            }
        }
    }

    /// `_apply_edit` for a commit's or undo's own tile: installed with the batch when held or
    /// desired, or shown graded by the preview. Returns the token it carries, None to skip.
    pub(crate) fn stage_edit_tile(&mut self, tile: Tile, graded: bool, serial: i64) -> Option<i64> {
        let held = self.installed.contains_key(&tile) || self.levels.level_of(tile).is_some();
        if !held && !(self.preview.contains_key(&tile) && graded) {
            return None;
        }
        self.dirty_tile(tile, serial);
        let token = self.requests.tokens.get(&tile).copied().unwrap_or(0);
        self.requests.edit_batch.insert(tile, token);
        Some(token)
    }

    /// Whether queued work will change what the graded cells draw (`has_pending_work`'s part).
    pub(crate) fn has_pending_work(&self) -> bool {
        let requests = &self.requests;
        requests.requested.len() > 0
            || !requests.counts.is_empty()
            || self.wanted.len() > 0
            || requests.page_parked.len() > 0
            || requests.edit_batch.len() > 0
            || self.growth_waits.len() > 0
            || self.block_target.iter().any(|targets| targets.len() > 0)
    }
}
