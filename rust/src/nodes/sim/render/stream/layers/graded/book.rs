// SPDX-License-Identifier: GPL-2.0-only

//! `GradedTileBook`, the graded tile renderer's state in Rust (`RENDER-32-I`): the pages, a
//! block's pages and growth, the installed and preview tiles' copies, the requests, refusals and
//! exhaustion, the desired levels and counts, and the hand-over, as `terrain_graded_tiles.gd`
//! kept them ([`super::state`]). The renderer calls it where its GDScript did the same work and
//! then applies the effects ([`GradedTileBook::take_effects`]) in order: page arrays, slot uploads
//! (from the result it holds), binds, Rust queue cancels, cell marks and its mirrors of the
//! installed and preview entries, which its cell code reads until `RENDER-32-J`.
//!
//! Tiles are `Vector2i`, levels `(code, step)` as `request_graded_tiles` takes them, a copy
//! `(page, slot, first atlas layer, layers)`.

use super::copies::PlaceFailure;
use super::pages::{Copy, PageEvent, PoolPolicy};
use super::state::{Block, Effect, Graded, PageRef, Shape, Tile};
use crate::nodes::sim::render::stream::levels::level_code;
use godot::prelude::*;

/// Integers per effect in `take_effects`.
const EFFECT_FIELDS: usize = 7;

fn tile_of(tile: Vector2i) -> Tile {
    Tile {
        x: tile.x,
        z: tile.y,
    }
}

fn level_vector(level: Option<crate::simulation::terrain::graded::Level>) -> Vector2i {
    level.map_or(Vector2i::new(-1, -1), |level| {
        let (code, step) = level_code(level);
        Vector2i::new(code as i32, step as i32)
    })
}

fn place_code(result: Result<(), PlaceFailure>) -> i64 {
    match result {
        Ok(()) => 0,
        Err(PlaceFailure::Exhausted) => 2,
        Err(_) => 1,
    }
}

/// The graded tile renderer's book (`terrain_graded_tiles.gd`, `TERRAIN-13` part 2).
#[derive(GodotClass)]
#[class(init, base = RefCounted)]
pub struct GradedTileBook {
    state: Graded,
    scratch: Vec<(PageRef, Block)>,
    tiles: Vec<Tile>,
    flat: Vec<i64>,
    // The hand-over's answers, reused.
    results: Vec<i64>,
    considered: usize,
}

#[godot_api]
impl GradedTileBook {
    /// The last hand-over applied by `apply`, which takes the stale cancels and the flat
    /// requests and writes the queue's answers (per cancel, then per request) into the reused
    /// buffer; the book then takes them. No allocation once warm.
    pub(crate) fn hand_over_with(&mut self, apply: impl FnOnce(&[Tile], &[i64], &mut Vec<i64>)) {
        let mut results = std::mem::take(&mut self.results);
        results.clear();
        apply(&self.state.requests.stale_cancels, &self.flat, &mut results);
        self.state.hand_over_results(&results);
        self.results = results;
    }

    /// Sets the world up: the patch grid's lower corner, a patch's side, patches per axis, the
    /// block side in patches, the device's layer limit, and per level code `(slot side, most
    /// slots a page holds, most row texels a tile takes)` as flat triples. Drops every page and
    /// makes the world pages of `GL0` and `GL1` (effects).
    #[func]
    pub fn setup(
        &mut self,
        world_min: Vector2,
        patch_span: f64,
        patch_grid: Vector2i,
        block_patches: i64,
        layer_limit: i64,
        levels: PackedInt64Array,
    ) {
        let shape = Shape {
            world_min: [world_min.x, world_min.y],
            patch_span: patch_span as f32,
            patch_grid: [patch_grid.x, patch_grid.y],
            block_patches: block_patches as i32,
            layer_limit: layer_limit as u32,
        };
        let levels: Vec<(u64, u32, u64)> = levels
            .as_slice()
            .as_chunks::<3>().0.iter()
            .map(|level| (level[0] as u64, level[1] as u32, level[2] as u64))
            .collect();
        self.state.setup(shape, &levels);
    }

    /// The pooled free pages' bytes bound (`TERRAIN-09`, 32 MiB by default); negative: none but
    /// four a size (the allocator before the bound, for replays of its traces).
    #[func]
    pub fn set_pool_kept_bytes(&mut self, bytes: i64) {
        self.state.pages.set_policy(PoolPolicy {
            kept_bytes: u64::try_from(bytes).ok(),
            ..PoolPolicy::default()
        });
    }

    /// Drops every tile's copies, request, token, list, count and desire, as the renderer resets.
    #[func]
    pub fn clear(&mut self) {
        self.state.clear();
    }

    /// The resident patch dictionary's mutation: `key` resident or not.
    #[func]
    pub fn set_resident(&mut self, key: Vector2i, resident: bool) {
        self.state.set_resident((key.x, key.y), resident);
    }

    /// The resident patches, as the dictionary holds them (its first frame).
    #[func]
    pub fn sync_resident(&mut self, keys: PackedInt32Array) {
        self.state.resident.clear();
        for key in keys.as_slice().as_chunks::<2>().0 {
            self.state.resident.insert((key[0], key[1]), ());
        }
    }

    /// The load's settle flags as the renderer holds them (a block's page waits while they say
    /// a load settles).
    #[func]
    pub fn set_settle(&mut self, unbudgeted: bool, residency_settling: bool) {
        self.state.unbudgeted = unbudgeted;
        self.state.residency_settling = residency_settling;
    }

    /// The frame's point where a change of the settle flags releases the parked blocks whose
    /// lists are in.
    #[func]
    pub fn release_parked(&mut self) {
        let (unbudgeted, settling) = (self.state.unbudgeted, self.state.residency_settling);
        self.state.set_flags(unbudgeted, settling);
    }

    /// The desired tiles.
    #[func]
    pub fn desired_len(&self) -> i64 {
        self.state.levels.len() as i64
    }

    /// Whether a tile wants a request.
    #[func]
    pub fn has_wanted(&self) -> bool {
        self.state.wanted.len() > 0
    }

    /// Moves the camera (XZ, height above the ground as `y`): the tiles whose level changed are
    /// wanted. Returns the tiles re-levelled.
    #[func]
    pub fn move_camera(&mut self, camera: Vector3) -> i64 {
        let changed = self
            .state
            .levels
            .move_camera([camera.x, camera.y, camera.z].map(f64::from));
        self.tiles.clear();
        self.tiles.extend_from_slice(changed);
        for &tile in &self.tiles {
            self.state.wanted.insert(tile, ());
        }
        self.state.levels.last_move_work().0 as i64
    }

    /// Whether counts are queued.
    #[func]
    pub fn has_counts(&self) -> bool {
        !self.state.requests.counts.is_empty()
    }

    /// Applies the next queued count (a tile entering or leaving the desired set).
    #[func]
    pub fn apply_count(&mut self) -> bool {
        self.state.apply_count()
    }

    /// Patch `key`'s list, flat `(x, z)` pairs.
    #[func]
    pub fn take_list(&mut self, key: Vector2i, token: i64, tiles: PackedInt32Array) {
        let tiles = tiles
            .as_slice()
            .as_chunks::<2>().0.iter()
            .map(|tile| Tile { x: tile[0], z: tile[1] })
            .collect();
        self.state.take_list((key.x, key.y), token, tiles);
    }

    /// Patch `key`'s list refused at `token`.
    #[func]
    pub fn list_refused(&mut self, key: Vector2i, token: i64) {
        self.state.list_refused((key.x, key.y), token);
    }

    /// A tile's bake failed (refused by contract unless `retry`).
    #[func]
    pub fn failed_tile(&mut self, tile: Vector2i, code: i64, step: i64, token: i64, retry: bool) {
        self.state
            .failed_tile((tile_of(tile), code as u8, step as u8), token, retry);
    }

    /// The token `tile` is requested with at `code`/`step`, -1 when none.
    #[func]
    pub fn requested_token(&self, tile: Vector2i, code: i64, step: i64) -> i64 {
        self.state
            .requests
            .requested
            .get(&(tile_of(tile), code as u8, step as u8))
            .copied()
            .unwrap_or(-1)
    }

    /// Drops `tile`'s request at `code`/`step` if it carries `token`.
    #[func]
    pub fn requested_erase(&mut self, tile: Vector2i, code: i64, step: i64, token: i64) {
        let key = (tile_of(tile), code as u8, step as u8);
        if self.state.requests.requested.get(&key) == Some(&token) {
            self.state.requests.requested.remove(&key);
        }
    }

    /// The edit batch's token of `tile`, -1 when it is not in the batch.
    #[func]
    pub fn edit_batch_token(&self, tile: Vector2i) -> i64 {
        self.state
            .requests
            .edit_batch
            .get(&tile_of(tile))
            .copied()
            .unwrap_or(-1)
    }

    /// The edit batch's tiles.
    #[func]
    pub fn edit_batch_len(&self) -> i64 {
        self.state.requests.edit_batch.len() as i64
    }

    /// Puts `tile` in the edit batch with `token`. For tests.
    #[func]
    pub fn edit_batch_set(&mut self, tile: Vector2i, token: i64) {
        self.state.requests.edit_batch.insert(tile_of(tile), token);
    }

    /// No resident patch lists `tile` any more (`_undesire`, effects). For tests.
    #[func]
    pub fn undesire(&mut self, tile: Vector2i) {
        self.state.undesire(tile_of(tile));
    }

    /// The tiles waiting for a slot of their exhausted level, flat `(x, z, code, asked again)`.
    #[func]
    pub fn exhausted_tiles(&self) -> PackedInt64Array {
        self.state
            .requests
            .exhausted
            .iter()
            .flat_map(|(tile, record)| [i64::from(tile.x), i64::from(tile.z), i64::from(record.code), i64::from(record.asked)])
            .collect()
    }

    /// Whether `tile` wants a request.
    #[func]
    pub fn is_wanted(&self, tile: Vector2i) -> bool {
        self.state.wanted.contains_key(&tile_of(tile))
    }

    /// The edit batch installed: it and the staged tiles' levels empty.
    #[func]
    pub fn edit_batch_clear(&mut self) {
        self.state.requests.edit_batch.clear();
        self.state.requests.edit_levels.clear();
    }

    /// Installs `tile` at `code`/`step`/`token` with `layers` atlas layers. False where its
    /// pages could not take it (it waits, or is exhausted: effects).
    #[func]
    pub fn upload(&mut self, tile: Vector2i, level: Vector4i, layers: i64) -> bool {
        let level = (level.x as u8, level.y as u8, i64::from(level.z), level.w != 0);
        self.state
            .upload(tile_of(tile), level, layers as u32, None, &mut self.scratch)
    }

    /// Installs an edit batch's `tile` into its reserved copies (flat page, slot, first, layers).
    #[func]
    pub fn upload_reserved(&mut self, tile: Vector2i, level: Vector4i, layers: i64, copies: PackedInt64Array) -> bool {
        let level = (level.x as u8, level.y as u8, i64::from(level.z), level.w != 0);
        let copies = copies
            .as_slice()
            .as_chunks::<4>().0.iter()
            .map(|copy| {
                (copy[0] as u32, Copy { slot: copy[1] as u32, first: copy[2] as u32, layers: copy[3] as u32 })
            })
            .collect();
        self.state
            .upload(tile_of(tile), level, layers as u32, Some(copies), &mut self.scratch)
    }

    /// Places the preview's `tile`: 0 placed, 1 a full page waits (its block grows), 2 exhausted.
    #[func]
    pub fn place_preview(&mut self, tile: Vector2i, level: Vector4i, layers: i64) -> i64 {
        let level = (level.x as u8, level.y as u8, i64::from(level.z), level.w != 0);
        place_code(self.state.place_preview(tile_of(tile), level, layers as u32, &mut self.scratch))
    }

    /// Drops the preview's `tile`; false when it showed none.
    #[func]
    pub fn drop_preview(&mut self, tile: Vector2i) -> bool {
        self.state.drop_preview(tile_of(tile))
    }

    /// A committed tile adopts the preview's copies (`_adopt_preview`).
    #[func]
    pub fn adopt_preview(&mut self, tile: Vector2i, level: Vector3i, epoch: i64) -> bool {
        self.state
            .adopt_preview(tile_of(tile), (level.x as u8, level.y as u8, i64::from(level.z)), epoch)
    }

    /// Reserves the edit batch's copies, flat `(x, z, code, step, token, layers)` per staged tile
    /// in order. Returns `[0, then per tile its count and (page, slot, first, layers)]`, or `[1]`
    /// (nothing reserved) or `[2, x, z]` (that tile's level is exhausted; it left the batch).
    #[func]
    pub fn reserve_edit(&mut self, staged: PackedInt64Array) -> PackedInt64Array {
        let staged: Vec<super::ops::Staged> = staged
            .as_slice()
            .as_chunks::<6>().0.iter()
            .map(|tile| {
                (Tile { x: tile[0] as i32, z: tile[1] as i32 }, tile[2] as u8, tile[3] as u8, tile[4], tile[5] as u32)
            })
            .collect();
        let mut out = Vec::new();
        match self.state.reserve_edit(&staged, &mut self.scratch) {
            Ok(tiles) => {
                out.push(0);
                for copies in tiles {
                    out.push(copies.len() as i64);
                    for (page, copy) in copies {
                        out.extend([i64::from(page), i64::from(copy.slot), i64::from(copy.first), i64::from(copy.layers)]);
                    }
                }
            }
            Err(None) => out.push(1),
            Err(Some(tile)) => out.extend([2, i64::from(tile.x), i64::from(tile.z)]),
        }
        PackedInt64Array::from(out.as_slice())
    }

    /// Dirties `tile` with the edit's `serial` (`_dirty_tile`'s part).
    #[func]
    pub fn dirty_tile(&mut self, tile: Vector2i, serial: i64) {
        self.state.dirty_tile(tile_of(tile), serial);
    }

    /// A staged result invalidated before its first install keeps its level to rebake.
    #[func]
    pub fn set_edit_level(&mut self, tile: Vector2i, code: i64, step: i64) {
        self.state
            .requests
            .edit_levels
            .insert(tile_of(tile), (code as u8, step as u8));
    }

    /// A reset edit: every batched, installed, requested and refused tile dirtied. Returns them in
    /// order, flat `(x, z)`.
    #[func]
    pub fn dirty_all(&mut self, serial: i64) -> PackedInt32Array {
        self.state.dirty_all(serial, &mut self.tiles);
        self.tiles.iter().flat_map(|tile| [tile.x, tile.z]).collect()
    }

    /// An edit's dirty tiles at `code`/`step` (flat `(x, z)`): those held, batched, requested or
    /// refused there are dirtied and returned.
    #[func]
    pub fn dirty_level(&mut self, code: i64, step: i64, tiles: PackedInt32Array, serial: i64) -> PackedInt32Array {
        let tiles: Vec<Tile> = tiles
            .as_slice()
            .as_chunks::<2>().0.iter()
            .map(|tile| Tile { x: tile[0], z: tile[1] })
            .collect();
        self.state
            .dirty_level(code as u8, step as u8, &tiles, serial, &mut self.tiles);
        self.tiles.iter().flat_map(|tile| [tile.x, tile.z]).collect()
    }

    /// A commit's own `tile` joins the edit batch (held, desired, or shown graded by the
    /// preview): its token, or -1 to skip it.
    #[func]
    pub fn stage_edit_tile(&mut self, tile: Vector2i, graded: bool, serial: i64) -> i64 {
        self.state
            .stage_edit_tile(tile_of(tile), graded, serial)
            .unwrap_or(-1)
    }

    /// The hand-over: every wanted tile that can install is to be requested, and a complete
    /// tile's stale pending request cancelled. Rust's queue applies both in one call
    /// (`SimulationNode.hand_over_graded_tiles`, which answers the book), or a trace reads them
    /// (`hand_over_stale`, `hand_over_requests`, `hand_over_results`). Returns the requests.
    #[func]
    pub fn request_tiles(&mut self) -> i64 {
        self.considered = self.state.request_tiles(&mut self.flat, &mut self.scratch);
        (self.flat.len() / 5) as i64
    }

    /// The last hand-over's stale cancels, flat `(x, z)` in order (for a trace).
    #[func]
    pub fn hand_over_stale(&self) -> PackedInt32Array {
        self.state
            .requests
            .stale_cancels
            .iter()
            .flat_map(|tile| [tile.x, tile.z])
            .collect()
    }

    /// The last hand-over's requests, flat `(x, z, code, step, token)` (for a trace).
    #[func]
    pub fn hand_over_requests(&self) -> PackedInt64Array {
        PackedInt64Array::from(self.flat.as_slice())
    }

    /// Rust's queue's answers to the last hand-over (per stale cancel, then per request).
    #[func]
    pub fn hand_over_results(&mut self, results: PackedInt64Array) {
        self.state.hand_over_results(results.as_slice());
    }

    /// A trace records the hand-over's stale cancels in their place among the effects.
    #[func]
    pub fn set_markers(&mut self, markers: bool) {
        self.state.markers = markers;
    }

    /// The wanted tiles the last hand-over took.
    #[func]
    pub fn considered(&self) -> i64 {
        self.considered as i64
    }


    /// Growing blocks whose target holds every copy switch to it (effects).
    #[func]
    pub fn advance_growth(&mut self) {
        self.state.advance_growth();
    }

    /// A resident patch entry changed (`rect` its rectangle) or left (`resident` false).
    #[func]
    pub fn patch_changed(&mut self, key: Vector2i, resident: bool, rect: Rect2) {
        let rect = resident.then_some([rect.position.x, rect.position.y, rect.size.x, rect.size.y]);
        self.state.patch_changed((key.x, key.y), rect);
    }

    /// Whether the book's queued work will change what is drawn.
    #[func]
    pub fn has_pending_work(&self) -> bool {
        self.state.has_pending_work()
    }

    /// Wanted and parked tiles (`wanted_count`'s part).
    #[func]
    pub fn wanted_count(&self) -> i64 {
        (self.state.wanted.len() + self.state.requests.parked.len()) as i64
    }

    /// The desired level of `tile`, `(-1, -1)` when not desired.
    #[func]
    pub fn level_of(&self, tile: Vector2i) -> Vector2i {
        level_vector(self.state.levels.level_of(tile_of(tile)))
    }

    /// The level the rule gives `tile` held at `held` (-1: none), evaluated directly. For tests.
    #[func]
    pub fn rule_level(&self, tile: Vector2i, held: i64) -> Vector2i {
        let held = u8::try_from(held).ok();
        level_vector(Some(self.state.levels.rule_level(tile_of(tile), held)))
    }

    /// Every desired tile, flat `(x, z)`, in no particular order. For tests.
    #[func]
    pub fn desired_tiles(&self) -> PackedInt32Array {
        self.state.levels.tiles().flat_map(|tile| [tile.x, tile.z]).collect()
    }

    /// Whether `tile` cannot show at its desired level and token (refused or exhausted).
    #[func]
    pub fn refused_at_level(&self, tile: Vector2i) -> bool {
        self.state.refused_at_level(tile_of(tile))
    }

    /// The page block `block`'s sets bind at level `code`, -1 when none.
    #[func]
    pub fn block_page(&self, code: i64, block: Vector2i) -> i64 {
        self.state.block_page[code as usize]
            .get(&(block.x, block.y))
            .map_or(-1, |&page| i64::from(page))
    }

    /// The page block `block` moves to at level `code`, -1 when none.
    #[func]
    pub fn block_target(&self, code: i64, block: Vector2i) -> i64 {
        self.state.block_target[code as usize]
            .get(&(block.x, block.y))
            .map_or(-1, |&page| i64::from(page))
    }

    /// The pages a copy of `tile` at `code` goes into (-2: its block's lists pending, -1: refused).
    #[func]
    pub fn copy_pages(&mut self, tile: Vector2i, code: i64) -> PackedInt64Array {
        self.state.copy_pages(tile_of(tile), code as u8, &mut self.scratch);
        self.scratch
            .iter()
            .map(|(page, _)| match page {
                PageRef::Page(id) => i64::from(*id),
                PageRef::Pending => -2,
                PageRef::Refused => -1,
            })
            .collect()
    }

    /// Whether installed `tile` has a copy in every page that draws it.
    #[func]
    pub fn tile_complete(&mut self, tile: Vector2i) -> bool {
        let tile = tile_of(tile);
        let Some(level) = self.state.installed.get(&tile).map(|entry| entry.level) else {
            return false;
        };
        self.state.tile_complete(tile, level, &mut self.scratch)
    }

    /// Starts block `block`'s move at `code` to a page of `size` (slots, layers). For tests.
    #[func]
    pub fn move_block(&mut self, code: i64, block: Vector2i, size: Vector2i) -> bool {
        self.state
            .move_block(code as u8, (block.x, block.y), (size.x as u32, size.y as u32))
    }

    /// Page `page`: `[code, block x, block z, slots, layers, free slots, free layers]`, empty
    /// when it is not live.
    #[func]
    pub fn page_info(&self, page: i64) -> PackedInt64Array {
        let Some(live) = self.state.pages.page(page as u32) else {
            return PackedInt64Array::new();
        };
        PackedInt64Array::from(
            [
                i64::from(live.code),
                i64::from(live.block.0),
                i64::from(live.block.1),
                i64::from(live.slots),
                i64::from(live.layers),
                live.free_slots() as i64,
                i64::from(live.free_layers()),
            ]
            .as_slice(),
        )
    }

    /// `tile`'s installed (or preview's) entry: `[cell code, level, step, token, behind, copies,
    /// then (page, slot, first, layers) per copy]`, empty when none.
    #[func]
    pub fn entry(&self, tile: Vector2i, preview: bool) -> PackedInt64Array {
        let entries = if preview { &self.state.preview } else { &self.state.installed };
        let Some(entry) = entries.get(&tile_of(tile)) else {
            return PackedInt64Array::new();
        };
        let mut out = vec![
            i64::from(entry.cell_code),
            i64::from(entry.level),
            i64::from(entry.step),
            entry.token,
            i64::from(entry.behind),
            entry.copies.len() as i64,
        ];
        for (page, copy) in &entry.copies {
            out.extend([i64::from(*page), i64::from(copy.slot), i64::from(copy.first), i64::from(copy.layers)]);
        }
        PackedInt64Array::from(out.as_slice())
    }

    /// The pools' sizes and use for `get_pool_counters`: per level code `pages_N`, `slots_N`,
    /// `free_N`, `layers_N`, `free_layers_N` (the pages blocks bind or move to), `pool_bytes`,
    /// `pooled_bytes`, `growing`, `installed`, `preview`, `refused` (tiles), `desired`,
    /// `exhausted`, `desired_by_level`.
    #[func]
    pub fn pool_counters(&self) -> VarDictionary {
        let state = &self.state;
        let mut out = VarDictionary::new();
        for code in 0..state.caps.len() {
            let (mut pages, mut slots, mut free, mut layers, mut free_layers) = (0, 0, 0, 0, 0);
            for (_, &page) in state.block_page[code].iter().chain(state.block_target[code].iter()) {
                let live = state.pages.page(page).expect("bound pages are live");
                pages += 1;
                slots += i64::from(live.slots);
                free += live.free_slots() as i64;
                layers += i64::from(live.layers);
                free_layers += i64::from(live.free_layers());
            }
            out.set(format!("pages_{code}").as_str(), pages);
            out.set(format!("slots_{code}").as_str(), slots);
            out.set(format!("free_{code}").as_str(), free);
            out.set(format!("layers_{code}").as_str(), layers);
            out.set(format!("free_layers_{code}").as_str(), free_layers);
        }
        out.set("pool_bytes", state.pages.bytes() as i64);
        out.set("pooled_bytes", state.pages.pooled_bytes() as i64);
        out.set("growing", state.growth_waits.len() as i64);
        out.set("installed", state.installed.len() as i64);
        out.set("preview", state.preview.len() as i64);
        out.set("refused", state.requests.refused.len() as i64);
        out.set("desired", state.levels.len() as i64);
        out.set("exhausted", state.requests.exhausted.len() as i64);
        let by_code: PackedInt32Array = state.levels.by_code().map(|count| count as i32).into_iter().collect();
        out.set("desired_by_level", by_code);
        out
    }

    /// The effects since the last take, `EFFECT_FIELDS` integers each: kind, then
    /// 0 page made `(id, code, block x, block z, slots, layers)`, 1 page reused (the same),
    /// 2 released `(id, kept)`, 3 dropped `(id)`, 4 copied `(from, to)`, 5 refused `(-1, code,
    /// block x, block z, slots, layers)`, 10 write `(page, slot, first, layers)`, 11 bind `(code,
    /// block x, block z, rewrite, page or -1)`, 12 cancel `(x, z, stale)`, 13 exhausted `(x, z,
    /// code)`, 14 evicted `(x, z)`, 15 edit dropped `(x, z)`, 16 same-level upload, 17 entry
    /// changed `(x, z, preview)`, 18 mask `(x, z)`, 19 cell `(x, z, sides)`, 20 await cells
    /// `(x, z)`, 21 grew `(code, block x, block z, slots, layers)`, 22 moved.
    #[func]
    pub fn take_effects(&mut self) -> PackedInt64Array {
        let mut out = Vec::with_capacity(self.state.effects.len() * EFFECT_FIELDS);
        for effect in self.state.effects.drain(..) {
            let fields: [i64; EFFECT_FIELDS] = match effect {
                Effect::PageMade { id, reused, code, block, slots, layers } => [
                    i64::from(reused),
                    i64::from(id),
                    i64::from(code),
                    i64::from(block.0),
                    i64::from(block.1),
                    i64::from(slots),
                    i64::from(layers),
                ],
                Effect::Page(PageEvent::Release { id, kept }) => [2, i64::from(id), i64::from(kept), 0, 0, 0, 0],
                Effect::Page(PageEvent::Drop(id)) => [3, i64::from(id), 0, 0, 0, 0, 0],
                Effect::Page(PageEvent::Copy { from, to }) => [4, i64::from(from), i64::from(to), 0, 0, 0, 0],
                Effect::Page(PageEvent::Refused { code, block, slots, layers }) => [
                    5,
                    -1,
                    i64::from(code),
                    i64::from(block.0),
                    i64::from(block.1),
                    i64::from(slots),
                    i64::from(layers),
                ],
                Effect::Page(PageEvent::New(_) | PageEvent::Reuse(_)) => unreachable!("made into PageMade"),
                Effect::Write { page, copy } => [
                    10,
                    i64::from(page),
                    i64::from(copy.slot),
                    i64::from(copy.first),
                    i64::from(copy.layers),
                    0,
                    0,
                ],
                Effect::Bind { code, block, rewrite, page } => [
                    11,
                    i64::from(code),
                    i64::from(block.0),
                    i64::from(block.1),
                    i64::from(rewrite),
                    page.map_or(-1, i64::from),
                    0,
                ],
                Effect::Cancel { tile, stale } => [12, i64::from(tile.x), i64::from(tile.z), i64::from(stale), 0, 0, 0],
                Effect::Exhausted { tile, code } => [13, i64::from(tile.x), i64::from(tile.z), i64::from(code), 0, 0, 0],
                Effect::Evicted { tile } => [14, i64::from(tile.x), i64::from(tile.z), 0, 0, 0, 0],
                Effect::EditDropped { tile } => [15, i64::from(tile.x), i64::from(tile.z), 0, 0, 0, 0],
                Effect::SameLevel => [16, 0, 0, 0, 0, 0, 0],
                Effect::Installed { tile, preview } => [17, i64::from(tile.x), i64::from(tile.z), i64::from(preview), 0, 0, 0],
                Effect::MaskChanged { tile } => [18, i64::from(tile.x), i64::from(tile.z), 0, 0, 0, 0],
                Effect::CellChanged { tile, sides } => [19, i64::from(tile.x), i64::from(tile.z), i64::from(sides), 0, 0, 0],
                Effect::AwaitCells { tile } => [20, i64::from(tile.x), i64::from(tile.z), 0, 0, 0, 0],
                Effect::Grew { code, block, slots, layers } => [
                    21,
                    i64::from(code),
                    i64::from(block.0),
                    i64::from(block.1),
                    i64::from(slots),
                    i64::from(layers),
                    0,
                ],
                Effect::Moved => [22, 0, 0, 0, 0, 0, 0],
            };
            out.extend(fields);
        }
        PackedInt64Array::from(out.as_slice())
    }
}
