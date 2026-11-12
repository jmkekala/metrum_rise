// SPDX-License-Identifier: GPL-2.0-only

//! A block's pages, a tile's copies in them and a block's growth (`RENDER-32-I`, WIP: not wired
//! yet), ported from `terrain_graded_tiles.gd` (`_block_of`, `_patch_page`, `_block_listed`,
//! `_page_size`, `_first_page`, `_copy_pages`, `_block_lists_pending`, `_wait_or_grow`,
//! `_tile_complete`, `_place_copies`, `_free_copy`, `_page_emptied`, `_free_copies`, `_grow`,
//! `_move_block`, `_switch_block`, `_end_waits`) with their order of effects. Each function's
//! comment names the GDScript one; read docs/terrain.md "Pages and the row atlas" for the rules.
//!
//! Bounds: a tile's pages `O(patches meeting it, at most 4)`; a placement `O(its copies x the
//! pages' free runs)`; a block's page size `O(the block's listed tiles)`; a move `O(the moved
//! pages' holders)`.

use super::pages::Copy;
use super::state::{Block, Effect, Entry, Graded, PAGE_LAYERS_PER_SLOT, PageRef, PatchKey, TILE_M, Tile};

/// Why a placement failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PlaceFailure {
    // A page is full and its block grows (or waits for a move already started): the tile waits.
    Waits,
    // The block's page waits for its lists: the tile is wanted again then.
    Pending,
    // No larger page can be made (cap, layer limit or budget): the level is exhausted.
    Exhausted,
}

impl Graded {
    /// `_block_of`: the block of patch `key` at level `code`.
    pub(crate) fn block_of(&self, key: PatchKey, code: u8) -> Block {
        if code < 2 {
            return (0, 0);
        }
        // Vector2i division truncates; patch keys are never negative.
        (key.0 / self.shape.block_patches, key.1 / self.shape.block_patches)
    }

    // The patches of block `block`, row by row as the GDScript walked them.
    fn block_patches(&self, block: Block) -> impl Iterator<Item = PatchKey> + use<> {
        let side = self.shape.block_patches;
        (0..side).flat_map(move |z| (0..side).map(move |x| (block.0 * side + x, block.1 * side + z)))
    }

    /// `_block_listed`: the distinct tiles the block's resident patches' lists name.
    pub(crate) fn block_listed(&mut self, block: Block) -> usize {
        let mut seen = std::mem::take(&mut self.listed_scratch);
        seen.clear();
        for key in self.block_patches(block) {
            if !self.resident.contains_key(&key) {
                continue;
            }
            if let Some((_, tiles)) = self.lists.get(&key) {
                seen.extend(tiles.iter().copied());
            }
        }
        let count = seen.len();
        self.listed_scratch = seen;
        count
    }

    /// `_page_size`: block `block`'s page at level `code` (slots, atlas layers), at least `least`.
    pub(crate) fn page_size(&mut self, code: u8, block: Block, least: (u32, u32)) -> (u32, u32) {
        let caps = self.caps[usize::from(code)];
        let limit = self.shape.layer_limit;
        if code < 2 {
            return (caps.cap, limit.min(least.1.max(caps.cap * caps.max_layers)));
        }
        let listed = self.block_listed(block) as u32;
        let slots = caps.cap.min(least.0.max((listed + listed / 8 + 8).div_ceil(32) * 32));
        (slots, limit.min(least.1.max(slots * PAGE_LAYERS_PER_SLOT)))
    }

    /// `_first_page`: block `block`'s first page at level `code`, which its sets bind.
    fn first_page(&mut self, code: u8, block: Block) -> Option<u32> {
        let (slots, layers) = self.page_size(code, block, (0, 0));
        let page = self.pages.create(code, block, slots, layers);
        self.page_events();
        if let Some(page) = page {
            self.block_page[usize::from(code)].insert(block, page);
            self.holders_mut(page);
            self.effects.push(Effect::Bind {
                code,
                block,
                rewrite: false,
                page: Some(page),
            });
        }
        page
    }

    // The patch range a tile meets, in the GDScript's real_t arithmetic.
    fn tile_patch_range(&self, tile: Tile) -> ([i32; 2], [i32; 2]) {
        let span = self.shape.patch_span;
        let origin = [
            tile.x as f32 * TILE_M - self.shape.world_min[0],
            tile.z as f32 * TILE_M - self.shape.world_min[1],
        ];
        let first = origin.map(|value| (value / span).floor() as i32);
        let last = origin.map(|value| ((value + TILE_M) / span).ceil() as i32 - 1);
        (first, last)
    }

    /// `_copy_pages`: the pages a copy of `tile` at level `code` goes into, with their blocks:
    /// per block of the resident patches drawing it its page (made as the block first needs one,
    /// once its lists are in), and a growing block's target. Into `out`.
    pub(crate) fn copy_pages(&mut self, tile: Tile, code: u8, out: &mut Vec<(PageRef, Block)>) {
        out.clear();
        let (first, last) = self.tile_patch_range(tile);
        let has = |out: &Vec<(PageRef, Block)>, page: u32| out.iter().any(|(at, _)| *at == PageRef::Page(page));
        for z in first[1]..=last[1] {
            for x in first[0]..=last[0] {
                let key = (x, z);
                if !self.resident.contains_key(&key) {
                    continue;
                }
                let block = self.block_of(key, code);
                let target = self.block_target[usize::from(code)].get(&block).copied();
                let mut bound = self.block_page[usize::from(code)].get(&block).copied();
                if bound.is_none() && target.is_none() {
                    if self.block_lists_pending(code, block) {
                        out.push((PageRef::Pending, block));
                        continue;
                    }
                    bound = self.first_page(code, block);
                    if bound.is_none() {
                        out.push((PageRef::Refused, block));
                        continue;
                    }
                }
                if let Some(target) = target.filter(|&target| !has(out, target)) {
                    out.push((PageRef::Page(target), block));
                }
                if let Some(bound) = bound.filter(|&bound| !has(out, bound)) {
                    out.push((PageRef::Page(bound), block));
                }
            }
        }
    }

    /// `_block_lists_pending`: whether block `block`'s page (`GL2`, `GL3`) must wait for its
    /// patches' lists (or, while a load settles, for its patches to become resident).
    pub(crate) fn block_lists_pending(&self, code: u8, block: Block) -> bool {
        if code < 2 {
            return false;
        }
        let grid = self.shape.patch_grid;
        for key in self.block_patches(block) {
            if !self.resident.contains_key(&key) {
                if self.unbudgeted && self.residency_settling && key.0 < grid[0] && key.1 < grid[1] {
                    return true;
                }
                continue;
            }
            if !self.lists.contains_key(&key) && !self.lists_refused.contains_key(&key) {
                return true;
            }
        }
        false
    }

    /// `_wait_or_grow`: whether a placement that found `page` of `block` full waits for the
    /// block's move (`page` is the bound page of a moving block) or starts one.
    pub(crate) fn wait_or_grow(&mut self, code: u8, block: Block, page: u32, layers: u32) -> bool {
        let target = self.block_target[usize::from(code)].get(&block).copied();
        if target.is_some_and(|target| target != page) {
            return true;
        }
        self.grow(code, block, layers, None)
    }

    /// `_tile_complete`: whether `entry` of `tile` has a copy in every page `copy_pages` names.
    pub(crate) fn tile_complete(&mut self, tile: Tile, level: u8, scratch: &mut Vec<(PageRef, Block)>) -> bool {
        self.copy_pages(tile, level, scratch);
        let Some(entry) = self.installed.get(&tile) else {
            return false;
        };
        scratch.iter().all(|(page, _)| match page {
            PageRef::Page(page) => entry.copy_in(*page).is_some(),
            _ => false,
        })
    }

    /// `_place_copies`: places `tile`'s copies at level `code` (`layers` atlas layers) in every
    /// page `copy_pages` names, all or none; a held copy of this level and token (`fresh`) is
    /// kept unwritten, every other is written (an `Effect::Write`); held copies no page wants are
    /// freed. `held` is the tile's copies at this level. Err: nothing changed, why.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn place_copies(
        &mut self,
        tile: Tile,
        code: u8,
        layers: u32,
        held: &[(u32, Copy)],
        fresh: bool,
        is_preview: bool,
        scratch: &mut Vec<(PageRef, Block)>,
    ) -> Result<Vec<(u32, Copy)>, PlaceFailure> {
        self.copy_pages(tile, code, scratch);
        let held_in = |page: u32| held.iter().find(|(at, _)| *at == page).map(|(_, copy)| *copy);
        for &(page, block) in scratch.iter() {
            let page_id = match page {
                PageRef::Page(id) => id,
                PageRef::Pending => {
                    // Asked again once the block's lists are in.
                    self.wanted.insert(tile, ());
                    return Err(PlaceFailure::Pending);
                }
                PageRef::Refused => return Err(PlaceFailure::Exhausted),
            };
            let fits = self
                .pages
                .page(page_id)
                .is_some_and(|live| live.can_place(held_in(page_id), layers));
            if fits {
                continue;
            }
            let waits = self.pages.page(page_id).is_some() && self.wait_or_grow(code, block, page_id, layers);
            if waits && !is_preview {
                self.wait_for_growth(tile, code, block);
            }
            return Err(if waits { PlaceFailure::Waits } else { PlaceFailure::Exhausted });
        }
        let mut copies = Vec::with_capacity(scratch.len());
        for &(page, block) in scratch.iter() {
            let PageRef::Page(page_id) = page else {
                unreachable!("checked above");
            };
            let before = held_in(page_id);
            let copy = self.pages.page_mut(page_id).expect("live page").place(before, layers);
            if !(fresh && Some(copy) == before) {
                self.effects.push(Effect::Write { tile, page: page_id, copy });
            }
            copies.push((page_id, copy));
            let holders = self.holders_mut(page_id);
            if is_preview { &mut holders.1 } else { &mut holders.0 }.insert(tile, ());
            if self.block_target[usize::from(code)].get(&block) == Some(&page_id) {
                self.growth_dirty.insert((code, block), ());
            }
        }
        for &(page_id, copy) in held {
            if !copies.iter().any(|(at, _)| *at == page_id) {
                self.free_copy(tile, page_id, copy, is_preview);
            }
        }
        Ok(copies)
    }

    /// `_wait_for_growth`.
    pub(crate) fn wait_for_growth(&mut self, tile: Tile, code: u8, block: Block) {
        let waiting = &mut self.block_waiting[usize::from(code)];
        if !waiting.contains_key(&block) {
            waiting.insert(block, Default::default());
        }
        waiting.get_mut(&block).expect("just ensured").insert(tile, ());
        self.growth_waits.insert(tile, (code, block));
        self.wanted.remove(&tile);
    }

    /// `_free_copy`: frees `tile`'s copy in `page`: its slot and run, and with its last copy the
    /// page (a block's own page, or a growing block's target; the world pages stay). The level's
    /// exhausted tiles are asked again by the caller (`freed_slot`).
    pub(crate) fn free_copy(&mut self, tile: Tile, page: u32, copy: Copy, is_preview: bool) {
        let Some(live) = self.pages.page_mut(page) else {
            return;
        };
        let (code, block) = (live.code, live.block);
        live.vacate(copy);
        let holders = self.holders_mut(page);
        if is_preview { &mut holders.1 } else { &mut holders.0 }.remove(&tile);
        if self.block_target[usize::from(code)].contains_key(&block) {
            self.growth_dirty.insert((code, block), ());
        }
        if code >= 2 && self.holders_empty(page) {
            self.page_emptied(code, block, page);
        }
        self.ask_exhausted(code);
    }

    /// `_page_emptied`: a block page left without copies.
    fn page_emptied(&mut self, code: u8, block: Block, page: u32) {
        let target = self.block_target[usize::from(code)].get(&block).copied();
        if target == Some(page) {
            let bound = self.block_page[usize::from(code)].get(&block).copied();
            if bound.is_some_and(|bound| self.holders.get(&bound).is_some_and(|holders| holders.0.len() > 0)) {
                return;
            }
            self.block_target[usize::from(code)].remove(&block);
            self.release_page(page);
            self.end_waits(code, block);
            return;
        }
        if target.is_some() {
            return;
        }
        self.block_page[usize::from(code)].remove(&block);
        self.release_page(page);
        self.effects.push(Effect::Bind {
            code,
            block,
            rewrite: false,
            page: None,
        });
    }

    fn release_page(&mut self, page: u32) {
        self.pages.release(page);
        self.page_events();
        self.holders.remove(&page);
    }

    /// `_free_copies`: frees every copy of `tile`'s entry and empties its copies.
    pub(crate) fn free_copies(&mut self, tile: Tile, is_preview: bool) {
        let copies = match self.entries_mut(is_preview).get_mut(&tile) {
            Some(entry) => std::mem::take(&mut entry.copies),
            None => return,
        };
        for (page, copy) in copies {
            self.free_copy(tile, page, copy, is_preview);
        }
    }

    /// `_grow`: block `block`'s page at level `code` cannot place a copy of `layers` atlas
    /// layers: it moves to a larger page. `plan` is what an edit reservation's plan found the
    /// page lacking (no free slot, no run), the page itself back as it was. False where no
    /// larger page can be made.
    pub(crate) fn grow(&mut self, code: u8, block: Block, layers: u32, plan: Option<(bool, bool)>) -> bool {
        let target = self.block_target[usize::from(code)].get(&block).copied();
        let from = target.or_else(|| self.block_page[usize::from(code)].get(&block).copied());
        let Some(from) = from.and_then(|id| self.pages.page(id).map(|page| (id, page))) else {
            return false;
        };
        let (_, page) = from;
        let (slots, atlas) = (page.slots, page.layers);
        let (full, no_run) = plan.unwrap_or((page.free_slots() == 0, !page.has_run(layers.max(1))));
        let least = (if full { slots * 2 } else { slots }, if no_run { atlas * 2 } else { atlas });
        let size = self.page_size(code, block, least);
        if size.0 <= slots && size.1 <= atlas {
            return false;
        }
        self.move_block(code, block, (size.0.max(slots), size.1.max(atlas)))
    }

    /// `_move_block`: block `block` starts its move to a new page of `size`, which becomes its
    /// target (a target that ran out too is dropped), keeping every installed and preview copy:
    /// the bound page's arrays and layout are copied. False where the budget refuses the page.
    pub(crate) fn move_block(&mut self, code: u8, block: Block, size: (u32, u32)) -> bool {
        let target = self.block_target[usize::from(code)].get(&block).copied();
        let page = self.pages.create(code, block, size.0, size.1);
        self.page_events();
        let Some(page) = page else {
            return false;
        };
        self.holders_mut(page);
        if let Some(old) = target {
            // A move to a target that ran out too starts again at the larger page.
            let (tiles, previews) = self.holders.remove(&old).unwrap_or_default();
            self.drop_copies(&tiles, &previews, old);
            self.pages.release(old);
            self.page_events();
        }
        self.block_target[usize::from(code)].insert(block, page);
        self.effects.push(Effect::Grew {
            code,
            block,
            slots: size.0,
            layers: size.1,
        });
        if let Some(bound) = self.block_page[usize::from(code)].get(&block).copied() {
            self.pages.copy_layout(bound, page);
            self.page_events();
            let (tiles, previews) = self.holders.get(&bound).cloned().unwrap_or_default();
            for (tile, _) in tiles.iter() {
                copy_into(self.installed.get_mut(tile), bound, page);
                self.holders_mut(page).0.insert(*tile, ());
                self.effects.push(Effect::Installed { tile: *tile, preview: false });
            }
            for (tile, _) in previews.iter() {
                copy_into(self.preview.get_mut(tile), bound, page);
                self.holders_mut(page).1.insert(*tile, ());
                self.effects.push(Effect::Installed { tile: *tile, preview: true });
            }
        }
        self.growth_dirty.insert((code, block), ());
        true
    }

    /// `_switch_block`: block `block` binds its target at level `code`; the old page's copies go
    /// with it and the tiles that waited are asked again.
    pub(crate) fn switch_block(&mut self, code: u8, block: Block) {
        let level = usize::from(code);
        let mut target = self.block_target[level].remove(&block);
        let old = self.block_page[level].get(&block).copied();
        if let Some(page) = target.filter(|&page| code >= 2 && self.holders_empty(page)) {
            // Every tile left the block meanwhile: neither page holds a copy it draws.
            self.release_page(page);
            target = None;
            self.block_page[level].remove(&block);
        } else if let Some(page) = target {
            self.block_page[level].insert(block, page);
        }
        if let Some(old) = old {
            let (tiles, previews) = self.holders.remove(&old).unwrap_or_default();
            self.drop_copies(&tiles, &previews, old);
            self.pages.release(old);
            self.page_events();
        }
        self.effects.push(Effect::Bind {
            code,
            block,
            rewrite: true,
            page: target,
        });
        self.end_waits(code, block);
        self.effects.push(Effect::Moved);
    }

    /// `_end_waits`: the tiles that waited for block `block`'s move are wanted again.
    pub(crate) fn end_waits(&mut self, code: u8, block: Block) {
        if let Some(waiting) = self.block_waiting[usize::from(code)].remove(&block) {
            for (tile, _) in waiting.iter() {
                self.growth_waits.remove(tile);
                self.wanted.insert(*tile, ());
            }
        }
    }
}

impl Graded {
    // Drops the copies in `page` of its holders' entries (their mirrors follow).
    fn drop_copies(&mut self, tiles: &super::ordered::OrderedMap<Tile, ()>, previews: &super::ordered::OrderedMap<Tile, ()>, page: u32) {
        for (tile, _) in tiles.iter() {
            drop_copy(self.installed.get_mut(tile), page);
            self.effects.push(Effect::Installed { tile: *tile, preview: false });
        }
        for (tile, _) in previews.iter() {
            drop_copy(self.preview.get_mut(tile), page);
            self.effects.push(Effect::Installed { tile: *tile, preview: true });
        }
    }
}

// Drops an entry's copy in `page`.
fn drop_copy(entry: Option<&mut Entry>, page: u32) {
    if let Some(entry) = entry {
        entry.copies.retain(|(at, _)| *at != page);
    }
}

// An entry's copy in `from` is its copy in `to` too (a move keeps the layout).
fn copy_into(entry: Option<&mut Entry>, from: u32, to: u32) {
    if let Some(entry) = entry
        && let Some(copy) = entry.copy_in(from)
    {
        entry.copies.push((to, copy));
    }
}

#[cfg(test)]
mod tests;
