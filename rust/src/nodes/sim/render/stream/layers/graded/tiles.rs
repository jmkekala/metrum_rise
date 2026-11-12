// SPDX-License-Identifier: GPL-2.0-only

//! The tiles' requests, refusals, exhaustion and the hand-over (`RENDER-32-I`, WIP: not wired
//! yet), ported from `terrain_graded_tiles.gd` (`_refused_at_level`, `_exhaust`, the exhausted
//! order of `_free_copy`, `_hold`, `_desire`, `_undesire`, `_dirty_tile`, `_drop_requests`,
//! `_recount`, `_apply_count`, `_park_tile`, `_release_block`, `_advance_growth`, the entry part of
//! `_upload` and `_request_tiles`) with their order of effects. A request key is (tile, level
//! code, `GL3` step); tokens are edit serials (0 before any edit).
//!
//! Bounds: a hand-over `O(wanted tiles x the patches meeting each)`, each tile handed once per
//! change; a count `O(1)`; a list's recount `O(its old and new tiles)`; the exhausted order
//! rebuilt past twice its records plus 64 (amortised `O(1)`).

use super::copies::PlaceFailure;
use super::ordered::OrderedMap;
use super::pages::Copy;
use super::state::{Block, Effect, Entry, Graded, PageRef, PatchKey, Tile};
use crate::nodes::sim::render::stream::levels::level_code;
use std::collections::VecDeque;

/// A request: tile, level code, `GL3` step (0 below `GL3`).
pub(crate) type RequestKey = (Tile, u8, u8);

// Every (level code, step) a tile can be requested at, in `request_graded_tiles`' replaced-request
// order (the GDScript `LEVEL_STEPS`).
const LEVEL_STEPS: [(u8, u8); 6] = [(0, 0), (1, 0), (2, 0), (3, 2), (3, 4), (3, 8)];

/// A tile whose level's pool was exhausted: at which request and token, and whether a freed slot
/// already asked it again.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Exhausted {
    pub code: u8,
    pub key: RequestKey,
    pub token: i64,
    pub asked: bool,
}

/// The requests' half of the state.
#[derive(Debug, Default)]
pub(crate) struct Requests {
    pub(crate) tokens: std::collections::HashMap<Tile, i64>,
    pub(crate) requested: OrderedMap<RequestKey, i64>,
    pub(crate) refused: OrderedMap<RequestKey, i64>,
    pub(crate) exhausted: OrderedMap<Tile, Exhausted>,
    pub(crate) exhausted_order: [Vec<Tile>; 4],
    pub(crate) page_parked: OrderedMap<(u8, Block), OrderedMap<Tile, ()>>,
    pub(crate) parked: OrderedMap<Tile, (u8, Block)>,
    pub(crate) parked_flags: i32,
    // The desired set's list counts per tile, the counted list per patch, and the counts queued.
    pub(crate) tile_lists: std::collections::HashMap<Tile, u32>,
    pub(crate) counted: OrderedMap<PatchKey, Vec<Tile>>,
    pub(crate) counts: VecDeque<(Tile, i8)>,
    // The edit batch (tile -> the token its rebake carries) and the levels of staged tiles
    // invalidated before their first install (the GDScript keeps the staged results).
    pub(crate) edit_batch: OrderedMap<Tile, i64>,
    pub(crate) edit_levels: std::collections::HashMap<Tile, (u8, u8)>,
    // The hand-over's cancels whose dropped level erases a stale request, in order.
    pub(crate) stale_cancels: Vec<Tile>,
    // The hand-over's requests, in order, until Rust's queue answers.
    pub(crate) sent: Vec<(Tile, RequestKey, i64)>,
}

impl Graded {
    /// `_refused_at_level`: whether `tile` cannot show at its desired level and token (refused by
    /// contract, or its level's pool exhausted).
    pub(crate) fn refused_at_level(&self, tile: Tile) -> bool {
        let requests = &self.requests;
        if requests.refused.len() == 0 && !requests.exhausted.contains_key(&tile) {
            return false;
        }
        let Some(level) = self.levels.level_of(tile) else {
            return false;
        };
        let (code, step) = level_code(level);
        let key = (tile, code as u8, step as u8);
        let token = requests.tokens.get(&tile).copied().unwrap_or(0);
        if let Some(record) = requests.exhausted.get(&tile)
            && record.key == key
            && record.token == token
        {
            return true;
        }
        requests.refused.get(&key) == Some(&token)
    }

    /// `_exhaust`: `tile` found level `code`'s pool exhausted at `key` and `token`.
    pub(crate) fn exhaust(&mut self, tile: Tile, code: u8, key: RequestKey, token: i64) {
        self.wanted.remove(&tile);
        let requests = &mut self.requests;
        requests.exhausted.insert(tile, Exhausted { code, key, token, asked: false });
        let order = &mut requests.exhausted_order[usize::from(code)];
        order.push(tile);
        // A tile recorded again leaves its earlier entry behind: past twice the records the
        // order is rebuilt from them, so it stays bounded however long the pools stay exhausted.
        if order.len() > 2 * requests.exhausted.len() + 64 {
            order.clear();
            for (waiting, record) in requests.exhausted.iter() {
                if record.code == code && !record.asked {
                    order.push(*waiting);
                }
            }
        }
        self.effects.push(Effect::Exhausted { tile, code });
    }

    /// The end of `_free_copy`: a freed slot of level `code` asks one exhausted tile of the level
    /// again (it stays recorded until it installs or finds no slot again).
    pub(crate) fn ask_exhausted(&mut self, code: u8) {
        let requests = &mut self.requests;
        while let Some(waiting) = requests.exhausted_order[usize::from(code)].pop() {
            let Some(record) = requests.exhausted.get_mut(&waiting) else {
                continue;
            };
            if record.code != code || record.asked {
                continue;
            }
            record.asked = true;
            self.wanted.insert(waiting, ());
            return;
        }
    }

    /// `_hold`: `tile` is installed at `code` and `step` with `token`; the levels hold it there,
    /// and a tile that wants another level or token, or whose level changed and is not `complete`,
    /// wants a request.
    pub(crate) fn hold(&mut self, tile: Tile, code: u8, step: u8, token: i64, complete: bool) {
        let Some((level, changed)) = self.levels.hold(tile, Some(code)) else {
            return;
        };
        let (want_code, want_step) = level_code(level);
        let token_wanted = self.requests.tokens.get(&tile).copied().unwrap_or(0);
        if (changed && !complete)
            || (want_code as u8, want_step as u8) != (code, step)
            || token_wanted != token
        {
            self.wanted.insert(tile, ());
        }
    }

    /// `_desire`.
    fn desire(&mut self, tile: Tile) {
        let held = self.installed.get(&tile).map(|entry| entry.level);
        self.levels.desire(tile, held);
        self.wanted.insert(tile, ());
    }

    /// `_undesire`: no resident patch lists `tile` any more: its copies are freed and its
    /// requests, token and refusals dropped.
    pub(crate) fn undesire(&mut self, tile: Tile) {
        self.levels.undesire(tile);
        self.wanted.remove(&tile);
        self.unpark(tile);
        if let Some((code, block)) = self.growth_waits.remove(&tile)
            && let Some(waiting) = self.block_waiting[usize::from(code)].get_mut(&block)
        {
            waiting.remove(&tile);
        }
        if self.installed.contains_key(&tile) {
            self.free_copies(tile, false);
            self.installed.remove(&tile);
            self.effects.push(Effect::Evicted { tile });
        }
        let requests = &mut self.requests;
        requests.edit_batch.remove(&tile);
        requests.edit_levels.remove(&tile);
        requests.exhausted.remove(&tile);
        self.effects.push(Effect::EditDropped { tile });
        self.drop_requests(tile);
        self.requests.tokens.remove(&tile);
    }

    /// `_drop_requests`: every request and refusal of `tile` dropped; its pending bake cancelled.
    pub(crate) fn drop_requests(&mut self, tile: Tile) {
        for (code, step) in LEVEL_STEPS {
            self.requests.requested.remove(&(tile, code, step));
            self.requests.refused.remove(&(tile, code, step));
        }
        self.effects.push(Effect::Cancel { tile, stale: false });
    }

    /// `_dirty_tile`: `tile` takes the edit's serial as its token; its requests drop and an
    /// installed or batched tile joins the edit batch. (The GDScript moves a staged result's
    /// level into `edit_levels` itself, `set_edit_level`.)
    pub(crate) fn dirty_tile(&mut self, tile: Tile, serial: i64) {
        let token = self.requests.tokens.get(&tile).copied().unwrap_or(0).max(serial);
        self.drop_requests(tile);
        self.requests.tokens.insert(tile, token);
        if self.installed.contains_key(&tile) || self.requests.edit_batch.contains_key(&tile) {
            self.requests.edit_batch.insert(tile, token);
            self.wanted.insert(tile, ());
        }
        if self.levels.level_of(tile).is_some() {
            self.wanted.insert(tile, ());
        }
    }

    /// `_recount`: the tiles patch `key` adds to the desired set (its list while resident),
    /// queued as counts. `O(the patch's old and new tiles)`.
    pub(crate) fn recount(&mut self, key: PatchKey) {
        let tiles: Vec<Tile> = if self.resident.contains_key(&key) {
            self.lists.get(&key).map(|(_, tiles)| tiles.clone()).unwrap_or_default()
        } else {
            Vec::new()
        };
        let old = self.requests.counted.get(&key).cloned().unwrap_or_default();
        if old == tiles {
            return;
        }
        // The new list counts first, so a tile on both lists never leaves.
        let counts = &mut self.requests.counts;
        counts.extend(tiles.iter().map(|&tile| (tile, 1)));
        counts.extend(old.iter().map(|&tile| (tile, -1)));
        if tiles.is_empty() {
            self.requests.counted.remove(&key);
        } else {
            self.requests.counted.insert(key, tiles);
        }
    }

    /// `_apply_count`: the next queued count. False when none is queued.
    pub(crate) fn apply_count(&mut self) -> bool {
        let Some((tile, delta)) = self.requests.counts.pop_front() else {
            return false;
        };
        let count = self.requests.tile_lists.get(&tile).copied().unwrap_or(0) as i64 + i64::from(delta);
        if count > 0 {
            self.requests.tile_lists.insert(tile, count as u32);
            if count == 1 && delta > 0 {
                self.desire(tile);
            }
            return true;
        }
        self.requests.tile_lists.remove(&tile);
        self.undesire(tile);
        true
    }

    /// `_park_tile`: `tile` waits for block `block`'s page at level `code` (its lists).
    fn park(&mut self, tile: Tile, code: u8, block: Block) {
        self.unpark(tile);
        let parked = &mut self.requests.page_parked;
        if !parked.contains_key(&(code, block)) {
            parked.insert((code, block), OrderedMap::default());
        }
        parked.get_mut(&(code, block)).expect("just ensured").insert(tile, ());
        self.requests.parked.insert(tile, (code, block));
    }

    /// `_unpark_tile`.
    fn unpark(&mut self, tile: Tile) {
        let Some(marked) = self.requests.parked.remove(&tile) else {
            return;
        };
        let parked = &mut self.requests.page_parked;
        let tiles = parked.get_mut(&marked).expect("a parked tile's block");
        tiles.remove(&tile);
        if tiles.len() == 0 {
            parked.remove(&marked);
        }
    }

    /// `_release_patch_blocks`: patch `key`'s list arrived or was refused, or its residency
    /// changed: its blocks' parked tiles are wanted again once those blocks' lists are in.
    pub(crate) fn release_patch_blocks(&mut self, key: PatchKey) {
        if self.requests.page_parked.len() == 0 {
            return;
        }
        for code in [2, 3] {
            let block = self.block_of(key, code);
            self.release_block((code, block));
        }
    }

    /// `_release_block`.
    pub(crate) fn release_block(&mut self, marked: (u8, Block)) {
        if !self.requests.page_parked.contains_key(&marked) || self.block_lists_pending(marked.0, marked.1) {
            return;
        }
        let tiles = self.requests.page_parked.remove(&marked).expect("checked");
        for (tile, _) in tiles.iter() {
            self.requests.parked.remove(tile);
            self.wanted.insert(*tile, ());
        }
    }

    /// The settle flags changed (`unbudgeted * 2 + residency_settling`): every parked block is
    /// released if its lists are in.
    pub(crate) fn set_flags(&mut self, unbudgeted: bool, residency_settling: bool) {
        self.unbudgeted = unbudgeted;
        self.residency_settling = residency_settling;
        let flags = i32::from(unbudgeted) * 2 + i32::from(residency_settling);
        if flags == self.requests.parked_flags {
            return;
        }
        self.requests.parked_flags = flags;
        let marked: Vec<(u8, Block)> = self.requests.page_parked.keys().copied().collect();
        for block in marked {
            self.release_block(block);
        }
    }

    /// `_advance_growth`: a growing block whose target holds a copy of every installed tile its
    /// page holds (but those refused or exhausted at their level) switches to it.
    pub(crate) fn advance_growth(&mut self) {
        if self.growth_dirty.len() == 0 {
            return;
        }
        let marked: Vec<(u8, Block)> = self.growth_dirty.keys().copied().collect();
        for (code, block) in marked {
            let Some(target) = self.block_target[usize::from(code)].get(&block).copied() else {
                continue;
            };
            let bound = self.block_page[usize::from(code)].get(&block).copied();
            let complete = bound.and_then(|bound| self.holders.get(&bound)).is_none_or(|(tiles, _)| {
                tiles.iter().all(|(tile, _)| {
                    self.installed.get(tile).is_some_and(|entry| entry.copy_in(target).is_some())
                        || self.refused_at_level(*tile)
                })
            });
            if complete {
                self.switch_block(code, block);
            }
        }
        self.growth_dirty.clear();
    }

    /// `_upload`'s placement and entry: installs `tile` at `code`/`step`/`token` with `layers`
    /// atlas layers (`reserved`: an edit batch's copies, already placed). Returns whether it
    /// installed; the GDScript writes the effects' uploads from the result it holds and keeps
    /// the walls.
    pub(crate) fn upload(
        &mut self,
        tile: Tile,
        level: (u8, u8, i64, bool),
        layers: u32,
        reserved: Option<Vec<(u32, Copy)>>,
        scratch: &mut Vec<(PageRef, Block)>,
    ) -> bool {
        let (code, step, token, behind) = level;
        let current = self.installed.get(&tile).cloned();
        let same_level = current.as_ref().is_some_and(|entry| entry.level == code);
        let held: Vec<(u32, Copy)> = if same_level {
            current.as_ref().map(|entry| entry.copies.clone()).unwrap_or_default()
        } else {
            Vec::new()
        };
        let fresh = same_level
            && current
                .as_ref()
                .is_some_and(|entry| entry.step == step && entry.token == token);
        let copies = match reserved {
            None => match self.place_copies(tile, code, layers, &held, fresh, false, scratch) {
                Ok(copies) => copies,
                Err(failure) => {
                    if failure == PlaceFailure::Exhausted {
                        self.exhaust(tile, code, (tile, code, step), token);
                    }
                    return false;
                }
            },
            Some(copies) => {
                for &(page, copy) in &copies {
                    if !(fresh && held.iter().any(|&(at, kept)| at == page && kept == copy)) {
                        self.effects.push(Effect::Write { page, copy });
                    }
                }
                for &(page, copy) in &held {
                    if !copies.iter().any(|(at, _)| *at == page) {
                        self.free_copy(tile, page, copy, false);
                    }
                }
                copies
            }
        };
        if same_level {
            self.effects.push(Effect::SameLevel);
        }
        self.requests.exhausted.remove(&tile);
        let cell_code = cell_code(code, step);
        let pages_changed = current.as_ref().is_none_or(|entry| {
            entry.copies.len() != copies.len()
                || copies.iter().any(|(page, _)| entry.copy_in(*page).is_none())
        });
        // A tile moving between levels frees its copies in the old level's pages.
        if current.is_some() && !same_level {
            self.free_copies(tile, false);
        }
        self.installed.insert(
            tile,
            Entry { cell_code, level: code, step, token, behind, copies, adoptable: true },
        );
        self.effects.push(Effect::Installed { tile, preview: false });
        let previewed = self.preview.contains_key(&tile);
        if same_level && !previewed {
            self.effects.push(Effect::MaskChanged { tile });
        }
        self.hold(tile, code, step, token, true);
        let redraw = current.as_ref().is_none_or(|entry| {
            entry.level != code || entry.cell_code != cell_code || entry.behind != behind
        }) || pages_changed;
        if redraw {
            if !previewed {
                let sides = current.as_ref().is_none_or(|entry| entry.cell_code != cell_code);
                self.effects.push(Effect::CellChanged { tile, sides });
            }
            self.effects.push(Effect::AwaitCells { tile });
        }
        true
    }

    /// `_request_tiles`: hands every wanted tile that is not installed at its level and token,
    /// not requested or refused at that token and not blocked on a page to Rust's queue (into
    /// `flat`, `REQUEST_FIELDS` a request); a complete tile cancels a pending request of another
    /// level (an `Effect::Cancel { stale: true }`, answered by `cancel_results`). Every wanted
    /// tile leaves `wanted`. Returns the tiles considered.
    pub(crate) fn request_tiles(&mut self, flat: &mut Vec<i64>, scratch: &mut Vec<(PageRef, Block)>) -> usize {
        flat.clear();
        self.requests.sent.clear();
        self.requests.stale_cancels.clear();
        // As the GDScript, the wanted tiles as they stand (the hand-over adds none), all taken.
        let mut taken = std::mem::take(&mut self.wanted_scratch);
        taken.clear();
        taken.extend(self.wanted.keys().copied());
        self.wanted.clear();
        let considered = taken.len();
        for &tile in &taken {
            let level = match self.levels.level_of(tile) {
                Some(level) => {
                    let (code, step) = level_code(level);
                    (code as u8, step as u8)
                }
                None => {
                    // A committed apron tile can be installed without ever appearing in a
                    // patch list. It still needs invalidation while resident.
                    match (self.installed.get(&tile), self.requests.edit_levels.get(&tile)) {
                        (Some(held), _) => (held.level, held.step),
                        (None, Some(&level)) => level,
                        (None, None) => continue,
                    }
                }
            };
            let token = self.requests.tokens.get(&tile).copied().unwrap_or(0);
            let current = self
                .installed
                .get(&tile)
                .map(|entry| (entry.level, entry.step, entry.token));
            if current == Some((level.0, level.1, token)) && self.tile_complete(tile, level.0, scratch) {
                // It shows what it wants: a request of another level still pending is dropped
                // (the hand-over's cancels; a trace marks each in its place among the effects).
                if self.markers {
                    self.effects.push(Effect::Cancel { tile, stale: true });
                }
                self.requests.stale_cancels.push(tile);
                continue;
            }
            if self.growth_waits.contains_key(&tile) {
                continue;
            }
            let key = (tile, level.0, level.1);
            if self.requests.requested.get(&key) == Some(&token) || self.requests.refused.get(&key) == Some(&token) {
                continue;
            }
            match self.requests.exhausted.get(&tile).copied() {
                Some(record) if record.key == key && record.token == token => {
                    if !record.asked {
                        continue;
                    }
                }
                _ => {
                    self.requests.exhausted.remove(&tile);
                }
            }
            // A new copy needs a free slot in its page (a rebake at the installed level rewrites
            // its own): a full page grows (the tile waits for the move), or the tile is exhausted.
            let same_level = current.is_some_and(|(held, _, _)| held == level.0);
            self.copy_pages(tile, level.0, scratch);
            let mut blocked = false;
            for &(page, block) in scratch.iter() {
                let free = match page {
                    PageRef::Page(id) => {
                        let held = same_level
                            && self.installed.get(&tile).is_some_and(|entry| entry.copy_in(id).is_some());
                        held || self.pages.page(id).is_some_and(|live| live.free_slots() > 0)
                    }
                    _ => false,
                };
                if free {
                    continue;
                }
                blocked = true;
                match page {
                    PageRef::Pending => self.park(tile, level.0, block),
                    PageRef::Page(id) if self.wait_or_grow(level.0, block, id, 0) => {
                        self.wait_for_growth(tile, level.0, block);
                    }
                    _ => self.exhaust(tile, level.0, key, token),
                }
                break;
            }
            if blocked {
                continue;
            }
            flat.extend([i64::from(tile.x), i64::from(tile.z), i64::from(level.0), i64::from(level.1), token]);
            self.requests.sent.push((tile, key, token));
        }
        self.wanted_scratch = taken;
        considered
    }

    /// The hand-over's answers from Rust's queue, in order: per stale cancel the dropped
    /// request's `LEVEL_STEPS` index (or -1), then per request its acceptance code.
    pub(crate) fn hand_over_results(&mut self, results: &[i64]) {
        let cancels = self.requests.stale_cancels.len().min(results.len());
        self.cancel_results(&results[..cancels]);
        self.requests_accepted(&results[cancels..]);
    }

    /// The stale cancels' answers, in order: the dropped request's `LEVEL_STEPS` index, or -1.
    fn cancel_results(&mut self, dropped: &[i64]) {
        let tiles = std::mem::take(&mut self.requests.stale_cancels);
        for (tile, &index) in tiles.iter().zip(dropped) {
            if let Some(&(code, step)) = usize::try_from(index).ok().and_then(|at| LEVEL_STEPS.get(at)) {
                self.requests.requested.remove(&(*tile, code, step));
            }
        }
        self.requests.stale_cancels = tiles;
    }

    /// Rust's queue's answers to the hand-over's requests, in order: 0 not queued, 1 queued,
    /// `2 + k` queued in place of a pending request at `LEVEL_STEPS[k]`.
    fn requests_accepted(&mut self, accepted: &[i64]) {
        let sent = std::mem::take(&mut self.requests.sent);
        for (index, &(tile, key, token)) in sent.iter().enumerate() {
            let code = accepted.get(index).map_or(0, |&code| u8::try_from(code).unwrap_or(0));
            if code == 0 {
                continue;
            }
            if code >= 2
                && let Some(&(level, step)) = LEVEL_STEPS.get(usize::from(code - 2))
            {
                self.requests.requested.remove(&(tile, level, step));
            }
            self.requests.requested.insert(key, token);
        }
        self.requests.sent = sent;
    }
}

/// A level's cell lattice code: the level below `GL3`, else 3 + log2(step).
pub(crate) fn cell_code(code: u8, step: u8) -> u8 {
    if code < 3 { code } else { 3 + step.max(1).ilog2() as u8 }
}
