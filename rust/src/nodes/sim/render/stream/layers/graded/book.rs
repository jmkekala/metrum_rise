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

use super::cells::{Patch, Shown, WALL_INSTANCE_FLOATS};
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
    // What a tile showed before a preview change; the effects encoded; a published buffer.
    before: Shown,
    encoded: Vec<i64>,
    buffer: PackedFloat32Array,
    // The renderer's trace (record or replay): the cell builds' budget answers go through it.
    trace: Option<Gd<RefCounted>>,
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

    /// The resident patch dictionary's mutation: `key` resident (its entry's rectangle, LOD
    /// step and generation) or erased.
    #[func]
    pub fn set_patch(&mut self, key: Vector2i, resident: bool, rect: Rect2, step: i64, generation: i64) {
        let patch = resident.then_some(Patch {
            rect: [rect.position.x, rect.position.y, rect.size.x, rect.size.y],
            step: step as i32,
            generation,
        });
        self.state.set_patch((key.x, key.y), patch);
    }

    /// The resident dictionary cleared (its first frame fills it again with `set_patch`).
    #[func]
    pub fn clear_resident(&mut self) {
        self.state.resident.clear();
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

    /// Installs `tile` at `code`/`step`/`token` with `layers` atlas layers and its bake's wall
    /// faces. False where its pages could not take it (it waits, or is exhausted: effects).
    #[func]
    pub fn upload(&mut self, tile: Vector2i, level: Vector4i, layers: i64, walls: PackedFloat32Array) -> bool {
        self.upload_with(tile, level, layers, None, walls)
    }

    /// Installs an edit batch's `tile` into its reserved copies (flat page, slot, first, layers).
    #[func]
    pub fn upload_reserved(
        &mut self,
        tile: Vector2i,
        level: Vector4i,
        layers: i64,
        copies: PackedInt64Array,
        walls: PackedFloat32Array,
    ) -> bool {
        let copies = copies
            .as_slice()
            .as_chunks::<4>().0.iter()
            .map(|copy| {
                (copy[0] as u32, Copy { slot: copy[1] as u32, first: copy[2] as u32, layers: copy[3] as u32 })
            })
            .collect();
        self.upload_with(tile, level, layers, Some(copies), walls)
    }

    // `_upload`: the walls staged for the entry it makes; a placed tile whose faces changed
    // rewrites its patches' walls.
    fn upload_with(
        &mut self,
        tile: Vector2i,
        level: Vector4i,
        layers: i64,
        reserved: Option<Vec<(u32, Copy)>>,
        walls: PackedFloat32Array,
    ) -> bool {
        let tile = tile_of(tile);
        let level = (level.x as u8, level.y as u8, i64::from(level.z), level.w != 0);
        self.state.settle();
        let before = self.state.installed.get(&tile).map(|entry| entry.walls.clone()).unwrap_or_default();
        self.state.cells.staged_walls = Some((tile, walls.as_slice().to_vec()));
        let done = self.state.upload(tile, level, layers as u32, reserved, &mut self.scratch);
        self.state.settle();
        self.state.cells.staged_walls = None;
        if done && walls.as_slice() != &before[..] {
            self.state.walls_changed(tile);
        }
        done
    }

    /// Places the preview's `tile` with its wall faces: 0 placed (what it draws changed), 1 a
    /// full page waits (its block grows), 2 exhausted.
    #[func]
    pub fn place_preview(&mut self, tile: Vector2i, level: Vector4i, layers: i64, walls: PackedFloat32Array) -> i64 {
        let tile = tile_of(tile);
        let level = (level.x as u8, level.y as u8, i64::from(level.z), level.w != 0);
        self.state.settle();
        self.state.shown_copy(tile, &mut self.before);
        self.state.cells.staged_walls = Some((tile, walls.as_slice().to_vec()));
        let placed = place_code(self.state.place_preview(tile, level, layers as u32, &mut self.scratch));
        self.state.settle();
        self.state.cells.staged_walls = None;
        if placed == 0 {
            let before = std::mem::take(&mut self.before);
            self.state.shown_changed(tile, &before);
            self.before = before;
        }
        placed
    }

    /// Drops the preview's `tile` (what it draws changed); false when it showed none.
    #[func]
    pub fn drop_preview(&mut self, tile: Vector2i) -> bool {
        let tile = tile_of(tile);
        self.state.settle();
        if !self.state.preview.contains_key(&tile) {
            return false;
        }
        self.state.shown_copy(tile, &mut self.before);
        self.state.drop_preview(tile);
        self.state.settle();
        let before = std::mem::take(&mut self.before);
        self.state.shown_changed(tile, &before);
        self.before = before;
        true
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

    /// A resident patch entry changed (`rect` its rectangle) or left (`resident` false): its
    /// tiles, its blocks and its set follow (`node_moved`: a drawing set's patch node changed).
    #[func]
    pub fn patch_changed(&mut self, key: Vector2i, resident: bool, rect: Rect2, node_moved: bool) {
        let rect = resident.then_some([rect.position.x, rect.position.y, rect.size.x, rect.size.y]);
        self.state.patch_changed_cells((key.x, key.y), rect, node_moved);
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
        out.set("mask_revision_bumps", state.cells.mask_bumps as i64);
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

    /// The renderer's trace (`terrain_graded_trace.gd`), null when none: a cell build asks it for
    /// each budget answer, as `_over_budget` did.
    #[func]
    pub fn set_trace(&mut self, trace: Option<Gd<RefCounted>>) {
        self.trace = trace;
    }

    /// Setup's cell sets: `count` of `per_patch` tile places, prewarmed and free.
    #[func]
    pub fn cells_setup(&mut self, per_patch: i64, count: i64) {
        self.state.cells_setup(per_patch as usize, count as usize);
    }

    /// `clear`'s cell part, after every set was released: every queue and mark dropped.
    #[func]
    pub fn clear_cells(&mut self) {
        self.state.settle();
        self.state.clear_cells();
    }

    /// Patch `key`'s list arrived (`arrived`: a patch without a set builds one) or was refused:
    /// what it draws follows its list.
    #[func]
    pub fn list_cells(&mut self, key: Vector2i, arrived: bool) {
        self.state.settle();
        if arrived {
            self.state.list_arrived((key.x, key.y));
        } else {
            self.state.cells.dirty_shown.insert((key.x, key.y), ());
        }
    }

    /// A tile refused by contract: the sets of the patches meeting it may draw without it.
    #[func]
    pub fn tile_refused(&mut self, tile: Vector2i) {
        self.state.settle();
        self.state.tile_shown_dirty(tile_of(tile));
    }

    /// `_variant_missing`: whether a set of the patches meeting `tile` lacks the behind-the-road
    /// variant (each such set is queued for it).
    #[func]
    pub fn variant_missing(&mut self, tile: Vector2i) -> bool {
        self.state.settle();
        self.state.variant_missing(tile_of(tile))
    }

    /// The first set queued for the variant takes it: 1 added, 0 not needed, -1 none queued.
    #[func]
    pub fn add_next_variant(&mut self) -> i64 {
        self.state.settle();
        self.state.add_next_variant().map_or(-1, i64::from)
    }

    /// The blocks bound at level `code`, flat `(x, z, page)` in order. For tests.
    #[func]
    pub fn bound_blocks(&self, code: i64) -> PackedInt32Array {
        self.state.block_page[code as usize]
            .iter()
            .flat_map(|(block, &page)| [block.0, block.1, page as i32])
            .collect()
    }

    /// Tiles of every set (resident patches) not drawn as they would be written now: from
    /// another mesh, or with other custom data. For tests.
    #[func]
    pub fn stale_cells(&self) -> i64 {
        let state = &self.state;
        let mut stale = 0;
        for (&key, &set) in state.cells.by_patch.iter() {
            let Some(patch) = state.resident.get(&key) else {
                continue;
            };
            let set = &state.cells.sets[set as usize];
            for &tile in &set.tiles {
                let (mesh, index) = set.at[set.place(tile)];
                let fresh = mesh >= 0
                    && mesh as usize == state.cell_mesh(tile, key)
                    && set.custom[mesh as usize][index as usize] == state.cell_custom(tile, set.first, patch, key);
                stale += i64::from(!fresh);
            }
        }
        stale
    }

    /// Patch `key`'s set's grass mask revision (0 without a set): grass renders the patch's
    /// mask again when it moves.
    #[func]
    pub fn mask_revision(&self, key: Vector2i) -> i64 {
        self.state
            .cells
            .by_patch
            .get(&(key.x, key.y))
            .map_or(0, |&set| self.state.cells.sets[set as usize].mask_revision)
    }

    /// The last grass mask revision given to any set.
    #[func]
    pub fn mask_revision_last(&self) -> i64 {
        self.state.cells.mask_revision
    }

    /// Whether a set is queued for the behind-the-road variant.
    #[func]
    pub fn has_variants_wanted(&self) -> bool {
        self.state.cells.variants_wanted.len() > 0
    }

    /// Installed (or preview) tiles.
    #[func]
    pub fn entry_count(&self, preview: bool) -> i64 {
        (if preview { self.state.preview.len() } else { self.state.installed.len() }) as i64
    }

    /// Set `set`'s instance count per cell mesh.
    #[func]
    pub fn set_counts(&self, set: i64) -> PackedInt32Array {
        let set = &self.state.cells.sets[set as usize];
        set.counts[..if set.variant { 9 } else { 5 }].iter().map(|&count| count as i32).collect()
    }

    /// Set `set`'s written instances, six floats each: mesh, instance, custom data. For the
    /// probe's check of what the `MultiMesh`es hold.
    #[func]
    pub fn set_instances(&self, set: i64) -> PackedFloat32Array {
        let set = &self.state.cells.sets[set as usize];
        let mut out = Vec::with_capacity(set.tiles.len() * 6);
        for &tile in &set.tiles {
            let (mesh, index) = set.at[set.place(tile)];
            if mesh < 0 {
                continue;
            }
            out.extend([f32::from(mesh), index as f32]);
            out.extend(set.custom[mesh as usize][index as usize]);
        }
        PackedFloat32Array::from(out.as_slice())
    }

    /// The next queued set build `(x, z, urgent)`; `urgent` -1 when none is queued.
    #[func]
    pub fn next_build(&self) -> Vector3i {
        self.state
            .next_build()
            .map_or(Vector3i::new(0, 0, -1), |(key, urgent)| Vector3i::new(key.0, key.1, i32::from(urgent)))
    }

    /// Patch `key`'s queued build ended (`built`: its set may start drawing), or its patch left.
    #[func]
    pub fn build_finished(&mut self, key: Vector2i, built: bool) {
        self.state.settle();
        self.state.build_finished((key.x, key.y), built);
    }

    /// Patch `key`'s set for a build: started for the patch as it stands where it has none or
    /// its build was dropped. Returns the set.
    #[func]
    pub fn begin_build(&mut self, key: Vector2i) -> i64 {
        self.state.settle();
        let key = (key.x, key.y);
        let set = self.state.cells.by_patch.get(&key).copied();
        let set = match set {
            Some(set) if self.state.cells.sets[set as usize].built >= 0 => set,
            _ => self.state.begin_build(key),
        };
        i64::from(set)
    }

    /// Whether set `set` holds every tile of its patch.
    #[func]
    pub fn set_built(&self, set: i64) -> bool {
        self.state.cells.sets[set as usize].is_built()
    }

    /// Writes set `set`'s next tiles until the frame's install budget (from `start_us`, Godot's
    /// ticks) is spent and at least `minimum` were written. Returns whether the set is built.
    #[func]
    pub fn build_tiles(&mut self, set: i64, start_us: i64, minimum: i64) -> bool {
        self.state.settle();
        let unbudgeted = self.state.unbudgeted;
        let mut trace = self.trace.clone();
        let mut over = move || {
            let elapsed = godot::classes::Time::singleton().get_ticks_usec() as i64 - start_us;
            let over = !unbudgeted && elapsed as f64 / 1000.0 >= 1.0;
            match trace.as_mut() {
                Some(trace) => {
                    let answer = trace.call("value", &["budget".to_variant(), GString::from(if over { "1" } else { "0" }).to_variant()]);
                    answer.to::<GString>() == GString::from("1")
                }
                None => over,
            }
        };
        self.state.build_tiles(set as u32, Some((&mut over, minimum as u32)))
    }

    /// `_rebuild_patch_cells`: patch `key`'s set built whole now.
    #[func]
    pub fn rebuild_set(&mut self, key: Vector2i) {
        self.state.settle();
        self.state.rebuild_set((key.x, key.y));
    }

    /// `_apply_cell_changes`: the queued cell rewrites, draw states and flips.
    #[func]
    pub fn apply_cell_changes(&mut self) {
        self.state.apply_cell_changes();
    }

    /// `_apply_wall_changes`: the queued wall rewrites.
    #[func]
    pub fn apply_wall_changes(&mut self) {
        self.state.apply_wall_changes();
    }

    /// `release_patch`'s part: patch `key`'s set returns to the pool. Returns it, -1 when none.
    #[func]
    pub fn release_set(&mut self, key: Vector2i) -> i64 {
        self.state.release_set((key.x, key.y)).map_or(-1, i64::from)
    }

    /// Patch `key`'s set, -1 when none.
    #[func]
    pub fn patch_set(&self, key: Vector2i) -> i64 {
        self.state.cells.by_patch.get(&(key.x, key.y)).map_or(-1, |&set| i64::from(set))
    }

    /// Whether queued cell work will change what is drawn.
    #[func]
    pub fn cells_pending(&self) -> bool {
        self.state.cells_pending()
    }

    /// Cell set builds still queued.
    #[func]
    pub fn builds_queued(&self) -> i64 {
        (self.state.cells.builds.len() + self.state.cells.builds_urgent.len()) as i64
    }

    /// Patches queued for a whole rewrite and for their walls.
    #[func]
    pub fn dirty_counts(&self) -> Vector2i {
        Vector2i::new(self.state.cells.dirty_patches.len() as i32, self.state.cells.dirty_walls.len() as i32)
    }

    /// The buffer a taken publication effect names (`at`), in a reused array.
    #[func]
    pub fn cell_buffer(&mut self, at: i64) -> PackedFloat32Array {
        let len = self.state.cells.per_patch * 16;
        let at = at as usize;
        if self.buffer.len() != len {
            self.buffer.resize(len);
        }
        self.buffer
            .as_mut_slice()
            .copy_from_slice(&self.state.cells.taken_publish[at..at + len]);
        self.buffer.clone()
    }

    /// The taken wall effects' faces, nine floats each (three basis columns).
    #[func]
    pub fn wall_floats(&self) -> PackedFloat32Array {
        debug_assert_eq!(self.state.cells.taken_walls.len() % WALL_INSTANCE_FLOATS, 0);
        PackedFloat32Array::from(self.state.cells.taken_walls.as_slice())
    }

    /// The frame's cell counters since the last reset: tiles updated, sets flipped, instances
    /// written, instances moved between meshes.
    #[func]
    pub fn frame_counters(&self) -> PackedInt64Array {
        self.state.cells.frame.iter().map(|&count| count as i64).collect()
    }

    /// Resets the frame's cell counters.
    #[func]
    pub fn reset_frame_counters(&mut self) {
        self.state.cells.frame = [0; 4];
    }

    /// The preview's tiles of epochs `first` to `last`, flat `(x, z)` in order.
    #[func]
    pub fn preview_tiles(&self, first: i64, last: i64) -> PackedInt32Array {
        self.state
            .preview
            .iter()
            .filter(|(_, entry)| entry.token >= first && entry.token <= last)
            .flat_map(|(tile, _)| [tile.x, tile.z])
            .collect()
    }

    /// Whether a preview tile meets `rect`.
    #[func]
    pub fn previews_in(&self, rect: Rect2) -> bool {
        self.state.previews_in([rect.position.x, rect.position.y, rect.size.x, rect.size.y])
    }

    /// Drawing sets with a listed tile they do not show. For tests and the probe.
    #[func]
    pub fn partial_sets(&self) -> i64 {
        self.state.partial_sets() as i64
    }

    /// The state half of `tile_draw_violations`: (holes, stitches). For tests and the probe.
    #[func]
    pub fn draw_violations(&self) -> Vector2i {
        let (holes, stitches) = self.state.draw_violations();
        Vector2i::new(holes as i32, stitches as i32)
    }

    /// Every installed (or preview) tile, flat `(x, z)` in order. For tests and the probe.
    #[func]
    pub fn entry_tiles(&self, preview: bool) -> PackedInt32Array {
        let entries = if preview { &self.state.preview } else { &self.state.installed };
        entries.keys().flat_map(|tile| [tile.x, tile.z]).collect()
    }

    /// `tile`'s installed (or preview) wall faces.
    #[func]
    pub fn entry_walls(&self, tile: Vector2i, preview: bool) -> PackedFloat32Array {
        let entries = if preview { &self.state.preview } else { &self.state.installed };
        entries
            .get(&tile_of(tile))
            .map_or_else(PackedFloat32Array::new, |entry| PackedFloat32Array::from(entry.walls.as_slice()))
    }

    /// Set `set`'s state for tests: `tiles`, `first`, `last`, `built`, `shown`, `complete`,
    /// `variant`, `step`, `pages`, `counts`, per mesh its written custom data (`custom`, an
    /// array of colours per mesh) and per tile place `at` (mesh, instance).
    #[func]
    pub fn cell_state(&self, set: i64) -> VarDictionary {
        let set = &self.state.cells.sets[set as usize];
        let mut out = VarDictionary::new();
        let tiles: Array<Vector2i> = set.tiles.iter().map(|tile| Vector2i::new(tile.x, tile.z)).collect();
        out.set("tiles", tiles);
        out.set("first", Vector2i::new(set.first.x, set.first.z));
        out.set("last", Vector2i::new(set.last.x, set.last.z));
        out.set("built", set.built);
        out.set("shown", set.shown);
        out.set("complete", set.complete);
        out.set("variant", set.variant);
        out.set("step", set.step);
        let pages: PackedInt64Array = set.pages.iter().copied().collect();
        out.set("pages", pages);
        let counts: PackedInt32Array = set.counts.iter().map(|&count| count as i32).collect();
        out.set("counts", counts);
        let mut custom = VarArray::new();
        for (mesh, records) in set.custom.iter().enumerate() {
            let written: Array<Color> = records[..set.counts[mesh] as usize]
                .iter()
                .map(|data| Color::from_rgba(data[0], data[1], data[2], data[3]))
                .collect();
            custom.push(&written.to_variant());
        }
        out.set("custom", custom);
        let at: Array<Vector2i> = set.at.iter().map(|&(mesh, index)| Vector2i::new(i32::from(mesh), index)).collect();
        out.set("at", at);
        out
    }

    /// Whether a tile shown or installed in set `set`'s patch is flagged behind the road. For
    /// tests.
    #[func]
    pub fn set_flagged(&self, set: i64) -> bool {
        let set = &self.state.cells.sets[set as usize];
        set.tiles.iter().any(|tile| {
            self.state.preview.get(tile).is_some_and(|entry| entry.behind)
                || self.state.installed.get(tile).is_some_and(|entry| entry.behind)
        })
    }

    /// `_shown_in`: what `tile` draws in patch `key`'s cells as `[level, behind, slot]`, empty
    /// when it draws its base cell. For tests.
    #[func]
    pub fn shown_in(&self, tile: Vector2i, key: Vector2i) -> PackedInt64Array {
        let key = (key.x, key.y);
        let Some(entry) = self.state.shown_in(tile_of(tile), key) else {
            return PackedInt64Array::new();
        };
        let page = self.state.cells.bound[usize::from(entry.level)]
            .get(&self.state.block_of(key, entry.level))
            .copied();
        let slot = page.and_then(|page| entry.copy_in(page)).map_or(-1, |copy| i64::from(copy.slot));
        PackedInt64Array::from([i64::from(entry.level), i64::from(entry.behind), slot].as_slice())
    }

    /// The effects since the last take, `EFFECT_FIELDS` integers each: kind, then
    /// 0 page made `(id, code, block x, block z, slots, layers)`, 1 page reused (the same),
    /// 2 released `(id, kept)`, 3 dropped `(id)`, 4 copied `(from, to)`, 5 refused `(-1, code,
    /// block x, block z, slots, layers)`, 10 write `(page, slot, first, layers)`, 11 bind `(code,
    /// block x, block z, rewrite, page or -1)`, 12 cancel `(x, z, stale)`, 13 exhausted `(x, z,
    /// code)`, 14 evicted `(x, z)`, 15 edit dropped `(x, z)`, 16 same-level upload, 17 entry
    /// changed `(x, z, preview)`, 18 mask `(x, z)`, 19 cell `(x, z, sides)`, 20 await cells
    /// `(x, z)`, 21 grew `(code, block x, block z, slots, layers)`, 22 moved; the cells' Godot
    /// calls (`RENDER-32-J`): 30 set acquired `(set, x, z)`, 31 pages `(set, code, page or -1)`,
    /// 32 park `(set, x, z, under a patch)`, 33 begin `(set, x, z, step)`, 34 base mesh `(set,
    /// code, reset)`, 35 counts cleared `(set)`, 36 meshes `(set, first)`, 37 publish `(set, mesh,
    /// at)` (`cell_buffer`), 38 count `(set, mesh, count)`, 39 walls `(set, start, count,
    /// capacity, truncated)` (`wall_floats`), 40 draw `(x, z, state)`, 41 show `(set, x, z)`, 42
    /// hide `(set)`, 44 variant `(set, four pages, shown)`, 45 rebuilt
    /// `(set, x, z)`, 46 built `(set)`. The kinds the cells apply themselves (binds, entry
    /// changes, marks) are settled first and not returned.
    #[func]
    pub fn take_effects(&mut self) -> PackedInt64Array {
        self.state.settle();
        let cells = &mut self.state.cells;
        std::mem::swap(&mut cells.taken_publish, &mut cells.publish_floats);
        cells.publish_floats.clear();
        std::mem::swap(&mut cells.taken_walls, &mut cells.wall_floats);
        cells.wall_floats.clear();
        let mut out = std::mem::take(&mut self.encoded);
        out.clear();
        for effect in self.state.out.drain(..) {
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
                Effect::Cancel { tile, stale } => [12, i64::from(tile.x), i64::from(tile.z), i64::from(stale), 0, 0, 0],
                Effect::Exhausted { tile, code } => [13, i64::from(tile.x), i64::from(tile.z), i64::from(code), 0, 0, 0],
                Effect::Evicted { tile, .. } => [14, i64::from(tile.x), i64::from(tile.z), 0, 0, 0, 0],
                Effect::EditDropped { tile } => [15, i64::from(tile.x), i64::from(tile.z), 0, 0, 0, 0],
                Effect::SameLevel => [16, 0, 0, 0, 0, 0, 0],
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
                Effect::CellAcquired { set, key } => [30, set.into(), key.0.into(), key.1.into(), 0, 0, 0],
                Effect::CellPages { set, code, page } => [31, set.into(), code.into(), page, 0, 0, 0],
                Effect::CellPark { set, key } => {
                    let (x, z) = key.unwrap_or((0, 0));
                    [32, set.into(), x.into(), z.into(), i64::from(key.is_some()), 0, 0]
                }
                Effect::CellBegin { set, key, step } => [33, set.into(), key.0.into(), key.1.into(), step.into(), 0, 0],
                Effect::CellBaseMesh { set, code, reset } => [34, set.into(), code.into(), reset.into(), 0, 0, 0],
                Effect::CellCountsCleared { set } => [35, set.into(), 0, 0, 0, 0, 0],
                Effect::CellMeshes { set, first } => [36, set.into(), first.into(), 0, 0, 0, 0],
                Effect::CellPublish { set, mesh, at } => [37, set.into(), mesh.into(), at.into(), 0, 0, 0],
                Effect::CellCount { set, mesh, count } => [38, set.into(), mesh.into(), count.into(), 0, 0, 0],
                Effect::CellWalls { set, start, count, capacity, truncated } => {
                    [39, set.into(), start.into(), count.into(), capacity.into(), truncated.into(), 0]
                }
                Effect::Draw { key, state } => [40, key.0.into(), key.1.into(), state.into(), 0, 0, 0],
                Effect::CellShow { set, key } => [41, set.into(), key.0.into(), key.1.into(), 0, 0, 0],
                Effect::CellHide { set } => [42, set.into(), 0, 0, 0, 0, 0],
                Effect::CellVariant { set, pages, shown } => {
                    [44, set.into(), pages[0], pages[1], pages[2], pages[3], shown.into()]
                }
                Effect::CellRebuilt { set, key } => [45, set.into(), key.0.into(), key.1.into(), 0, 0, 0],
                Effect::CellBuilt { set } => [46, set.into(), 0, 0, 0, 0, 0],
                Effect::Bind { .. }
                | Effect::Installed { .. }
                | Effect::MaskChanged { .. }
                | Effect::CellChanged { .. }
                | Effect::AwaitCells { .. } => unreachable!("settled"),
            };
            out.extend(fields);
        }
        let packed = PackedInt64Array::from(out.as_slice());
        self.encoded = out;
        packed
    }
}
