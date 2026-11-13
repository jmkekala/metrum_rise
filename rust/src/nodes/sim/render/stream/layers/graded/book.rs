// SPDX-License-Identifier: GPL-2.0-only

//! `GradedTileBook`, the graded tile renderer's state in Rust (`RENDER-32-I`): the pages, a
//! block's pages and growth, the installed and preview tiles' copies, the requests, refusals and
//! exhaustion, the desired levels and counts, and the hand-over, as `terrain_graded_tiles.gd`
//! kept them ([`super::state`]). The renderer calls it where its GDScript did the same work and
//! then applies the effects ([`GradedTileBook::take_effects`]) in order: page arrays, the runs of
//! slot uploads and cell effects the book applies itself, Rust queue cancels and draw states. The
//! cells, the edit batch and the preview's tiles are the book's (`RENDER-32-J`, `-K`).
//!
//! Tiles are `Vector2i`, levels `(code, step)` as `request_graded_tiles` takes them, a copy
//! `(page, slot, first atlas layer, layers)`.

use super::cells::{Patch, Shown, WALL_INSTANCE_FLOATS};
use super::objects::{Applied, CellCatalogue, CellObjects, PatchDraw};
use super::copies::PlaceFailure;
use super::pages::{Copy, PageEvent, PoolPolicy};
use super::state::{Block, Effect, Graded, PageRef, Shape, Tile};
use super::uploads::{self, ATLAS_WIDTH};
use crate::nodes::sim::render::stream::backend::{GodotBackend, RenderBackend};
use crate::nodes::sim::render::stream::levels::{
    GL3_END_M, LEVEL_END_M, desired_level, held_code, level_vector,
};
use crate::nodes::sim::render::stream::layers::graded::ordered::OrderedMap;
use crate::nodes::sim::render::vegetation_product::godot_math::Aabb as Bounds;
use godot::prelude::*;

/// Integers per effect in `take_effects`.
const EFFECT_FIELDS: usize = 7;

fn tile_of(tile: Vector2i) -> Tile {
    Tile {
        x: tile.x,
        z: tile.y,
    }
}

// The cells' effects the objects apply (every cell kind but the draw state, which the GDScript
// applies to its patch node).
fn is_cell_effect(effect: &Effect) -> bool {
    matches!(
        effect,
        Effect::CellAcquired { .. }
            | Effect::CellPages { .. }
            | Effect::CellPark { .. }
            | Effect::CellBegin { .. }
            | Effect::CellBaseMesh { .. }
            | Effect::CellCountsCleared { .. }
            | Effect::CellMeshes { .. }
            | Effect::CellPublish { .. }
            | Effect::CellCount { .. }
            | Effect::CellWalls { .. }
            | Effect::CellShow { .. }
            | Effect::CellHide { .. }
            | Effect::CellVariant { .. }
            | Effect::CellRebuilt { .. }
            | Effect::CellBuilt { .. }
    )
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
    // What a tile showed before a preview change; the effects encoded.
    before: Shown,
    encoded: Vec<i64>,
    // The renderer's trace (record or replay): the cell builds' budget answers go through it.
    trace: Option<Gd<RefCounted>>,
    // The cells' `RenderingServer` objects and the backend they go through (`RENDER-32-K`),
    // attached by the renderer's setup; the material sync's state; the cell effects of the last
    // take, applied in runs (`apply_cells`); the cell counters (`cell_counters`).
    objects: Option<CellObjects>,
    backend: Option<GodotBackend>,
    sync: MaterialSync,
    cell_effects: Vec<Effect>,
    cell_counts: [i64; CELL_COUNTERS],
    // Every page's textures as the renderer made them (kept across an attach).
    page_textures: std::collections::HashMap<i64, [u64; 2]>,
    // The page uploads (`RENDER-32-K` part 2): every page's shape as made (slot side, slots,
    // atlas layers), the bake the renderer is placing, the last take's writes (applied in runs,
    // effect 48) with the one bake they all take, and what they counted.
    page_shapes: std::collections::HashMap<u32, [u32; 3]>,
    staged: Option<Bake>,
    writes: Vec<PageWrite>,
    write_bake: Option<Bake>,
    // Tiles whose write found no bake of theirs in the last take (refused, baked again).
    unwritten: Vec<Tile>,
    upload_counts: [i64; 3],
    // The tile result path (`RENDER-32-K` part 3, [`results`]): its deferred results, the
    // results waiting for their sets' variant, its scratch and counters, and the owned bake
    // buffers written, for the simulation's pool.
    taken: results::Results,
    // The edit batch and the preview's waits (`RENDER-32-K` part 4, [`edits`]).
    edits: edits::Edits,
    // A placed tile's wall faces as floats, reused.
    walls_scratch: Vec<f32>,
    // The patches whose resident entry the streaming core changed since the renderer's last
    // frame ([`residency`]).
    patch_news: OrderedMap<super::state::PatchKey, ()>,
    // The patch lists' requests ([`lists`]).
    list_requests: lists::Lists,
}

mod edits;
mod lists;
mod residency;
mod results;
#[cfg(test)]
mod tests;
pub(crate) use edits::Edit;
pub(crate) use results::{Baked, Bytes, Polled};

// A bake as the renderer places it: its slot layer and row atlas run (little-endian RGBAF,
// shared with the polled result or moved out of it, not copied) and what a trace's `write` line
// names: the tile's x and z, level code, step and token.
struct Bake {
    texels: Bytes,
    rows: Bytes,
    names: [i64; 5],
}

// One copy to write: its page and the copy; it takes the take's bake (`write_bake`).
struct PageWrite {
    page: u32,
    copy: Copy,
}

// `cell_counters`: the frame's material sync, wall and flip microseconds (reset with the frame's
// counters), then walls truncated, sets given the variant, multimeshes and materials made, and
// set builds completed.
const CELL_COUNTERS: usize = 8;
const SYNC_US: usize = 0;
const WALLS_US: usize = 1;
const FLIP_US: usize = 2;
const TRUNCATED: usize = 3;
const BUILDS: usize = 7;

// The patch material parameters whose change means a drawing set's materials must sync: the
// textures and per-patch values `terrain.gd` swaps on commits, masks, water and overlays.
const WATCHED: [&str; 8] = [
    "heightmap",
    "terrain_mask_layer",
    "terrain_masks",
    "watermap",
    "land_cover_texture",
    "overlay_mode",
    "overlay_texture",
    "terrain_debug_lod_step",
];

/// The graded materials' parameter sync (`_sync_material`): per set the values its materials
/// hold and the watched values its last full pass started from, read and written through the
/// `RenderingServer` (textures as their RIDs, as a `ShaderMaterial` forwards them).
#[derive(Default)]
struct MaterialSync {
    names: Vec<StringName>,
    watched: Vec<StringName>,
    values: Vec<Vec<Variant>>,
    held: Vec<Vec<Variant>>,
}

impl MaterialSync {
    fn ensure(&mut self, set: usize) {
        while self.values.len() <= set {
            self.values.push(vec![Variant::nil(); self.names.len()]);
            self.held.push(Vec::new());
        }
    }

    // Whether the watched parameters of `material` differ from those set `set`'s last pass
    // started from. O(8).
    fn watched_moved(&mut self, server: &Gd<godot::classes::RenderingServer>, set: usize, material: Rid) -> bool {
        self.ensure(set);
        let held = &self.held[set];
        held.len() != self.watched.len()
            || self
                .watched
                .iter()
                .zip(held)
                .any(|(name, held)| !same(&server.material_get_param(material, name), held))
    }

    // Copies `source`'s parameters into the set's materials from `next` on; with `over` it stops
    // once the budget says so, after one parameter at least. Returns where it stopped (`None`:
    // finished). O(parameters x the set's materials).
    fn run(
        &mut self,
        server: &mut Gd<godot::classes::RenderingServer>,
        set: usize,
        materials: &[Rid],
        source: Rid,
        next: usize,
        mut over: Option<&mut dyn FnMut() -> bool>,
    ) -> Option<usize> {
        self.ensure(set);
        if next == 0 {
            // The watched values as the pass starts (the set's list reused once it has one).
            let held = &mut self.held[set];
            held.clear();
            held.extend(self.watched.iter().map(|name| server.material_get_param(source, name)));
        }
        let mut index = next;
        while index < self.names.len() {
            if index > next
                && let Some(over) = over.as_mut()
                && over()
            {
                return Some(index);
            }
            let name = &self.names[index];
            let value = server.material_get_param(source, name);
            index += 1;
            let held = &mut self.values[set][index - 1];
            if same(&value, held) {
                continue;
            }
            *held = value.clone();
            for &material in materials {
                server.material_set_param(material, name, &value);
            }
        }
        None
    }
}

// GDScript's `typeof(a) == typeof(b) and a == b`.
fn same(a: &Variant, b: &Variant) -> bool {
    a.get_type() == b.get_type() && a == b
}

// The trace's budget answer (a replay's recorded one), as `_over_budget` gives it.
fn over_budget(trace: &mut Option<Gd<RefCounted>>, unbudgeted: bool, start_us: i64) -> bool {
    let elapsed = godot::classes::Time::singleton().get_ticks_usec() as i64 - start_us;
    let over = !unbudgeted && elapsed as f64 / 1000.0 >= 1.0;
    match trace.as_mut() {
        Some(trace) => {
            let answer = trace.call("value", &["budget".to_variant(), GString::from(if over { "1" } else { "0" }).to_variant()]);
            answer.to::<GString>() == GString::from("1")
        }
        None => over,
    }
}

// Godot's `hash()` of `floats` as bytes (`PackedFloat32Array.to_byte_array()`).
fn bytes_hash(floats: &[f32]) -> u32 {
    let bytes: Vec<u8> = floats.iter().flat_map(|value| value.to_le_bytes()).collect();
    PackedByteArray::from(bytes.as_slice()).to_variant().hash_u32()
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
        self.list_requests.clear();
        self.edits.clear(&mut self.taken);
        self.taken.clear();
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
        self.patch_news.clear();
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

    /// Where `GL0`-`GL2` end for a tile not held there, metres (the road preview's view,
    /// `set_road_preview_terrain_view`).
    #[func]
    pub fn level_end_m() -> PackedFloat64Array {
        PackedFloat64Array::from(LEVEL_END_M.as_slice())
    }

    /// Where `GL3` ends at steps 2, 4 and 8 (the last infinite), metres.
    #[func]
    pub fn gl3_end_m() -> PackedFloat64Array {
        PackedFloat64Array::from(GL3_END_M.as_slice())
    }

    /// The level the distance rule gives a tile `distance` metres from the camera, held at level
    /// code `held` (-1: none), as `Vector2i(code, step)` (the stream benchmark's hysteresis ends).
    #[func]
    pub fn level_at(distance: f64, held: i64) -> Vector2i {
        level_vector(Some(desired_level(distance, held_code(held))))
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

    /// Installs `tile` at `code`/`step`/`token` with `layers` atlas layers and its bake's wall
    /// faces; the copies it writes take the bake's slot layer `texels` and row atlas run `rows`
    /// (effect 48). False where its pages could not take it (it waits, or is exhausted: effects).
    #[func]
    pub fn upload(
        &mut self,
        tile: Vector2i,
        level: Vector4i,
        layers: i64,
        walls: PackedFloat32Array,
        texels: PackedByteArray,
        rows: PackedByteArray,
    ) -> bool {
        self.stage(tile, level, Bytes::Shared(texels), Bytes::Shared(rows));
        self.upload_with(tile, level, layers, None, walls.as_slice())
    }

    // The bake whose copies the next take's writes take (one placed before it unwritten goes back
    // to the pool).
    fn stage(&mut self, tile: Vector2i, level: Vector4i, texels: Bytes, rows: Bytes) {
        let names = [tile.x, tile.y, level.x, level.y, level.z].map(i64::from);
        if let Some(old) = self.staged.replace(Bake { texels, rows, names }) {
            self.taken.recycle_bake(old.texels, old.rows);
        }
    }

    // `_upload`: the walls staged for the entry it makes; a placed tile whose faces changed
    // rewrites its patches' walls. The staged faces' buffer is reused.
    fn upload_with(
        &mut self,
        tile: Vector2i,
        level: Vector4i,
        layers: i64,
        reserved: Option<Vec<(u32, Copy)>>,
        walls: &[f32],
    ) -> bool {
        let tile = tile_of(tile);
        let level = (level.x as u8, level.y as u8, i64::from(level.z), level.w != 0);
        self.state.settle();
        // The faces differ from the installed entry's (none: from no faces).
        let changed = self.state.installed.get(&tile).map_or(!walls.is_empty(), |entry| entry.walls[..] != *walls);
        let mut staged = std::mem::take(&mut self.walls_scratch);
        staged.clear();
        staged.extend_from_slice(walls);
        self.state.cells.staged_walls = Some((tile, staged));
        let done = self.state.upload(tile, level, layers as u32, reserved, &mut self.scratch);
        self.state.settle();
        if let Some((_, staged)) = self.state.cells.staged_walls.take() {
            self.walls_scratch = staged;
        }
        if done && changed {
            self.state.walls_changed(tile);
        }
        done
    }

    /// Drops the preview's `tile` (what it draws changed); false when it showed none.
    fn drop_preview(&mut self, tile: Vector2i) -> bool {
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

    /// A committed tile adopts the preview's copies (the batch's adoption; tests).
    #[func]
    pub fn adopt_preview(&mut self, tile: Vector2i, level: Vector3i, epoch: i64) -> bool {
        self.state
            .adopt_preview(tile_of(tile), (level.x as u8, level.y as u8, i64::from(level.z)), epoch)
    }

    // Reserves the edit batch's copies, flat `(x, z, code, step, token, layers)` per staged tile
    // in order. Into `out`: `[0, then per tile its count and (page, slot, first, layers)]`, or
    // `[1]` (nothing reserved) or `[2, x, z]` (that tile's level is exhausted; it left the batch).
    fn reserve_edit(&mut self, staged: &[i64], out: &mut Vec<i64>) {
        let staged: Vec<super::ops::Staged> = staged
            .as_chunks::<6>().0.iter()
            .map(|tile| {
                (Tile { x: tile[0] as i32, z: tile[1] as i32 }, tile[2] as u8, tile[3] as u8, tile[4], tile[5] as u32)
            })
            .collect();
        out.clear();
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
        self.list_requests.patch_changed((key.x, key.y), resident);
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
    fn variant_missing(&mut self, tile: Vector2i) -> bool {
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
        let mut over = move || over_budget(&mut trace, unbudgeted, start_us);
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
        let set = self.state.release_set((key.x, key.y));
        if let (Some(set), Some(objects), Some(backend)) = (set, self.objects.as_mut(), self.backend.as_mut()) {
            objects.release(backend, set);
        }
        set.map_or(-1, i64::from)
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
        self.list_requests.reset_frame();
        self.cell_counts[SYNC_US..=FLIP_US].fill(0);
    }

    // The preview's tiles of epochs `first` to `last`, in order.
    fn preview_tiles(&self, first: i64, last: i64) -> Vec<Tile> {
        self.state
            .preview
            .iter()
            .filter(|(_, entry)| entry.token >= first && entry.token <= last)
            .map(|(tile, _)| *tile)
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
    /// `(x, z)`, 21 grew `(code, block x, block z, slots, layers)`, 22 moved, 40 draw `(x, z,
    /// state)`; the runs the book applies itself: 47 cell effects `(first, end)` (`apply_cells`,
    /// the cells' objects attached by `cells_attach`; a book without them drops them) and 48
    /// writes `(first, end)` (`apply_writes`). The kinds the cells apply themselves (binds, entry
    /// changes, marks) are settled first and not returned.
    #[func]
    pub fn take_effects(&mut self) -> PackedInt64Array {
        self.encode_effects();
        PackedInt64Array::from(self.encoded.as_slice())
    }

    // `take_effects` into `encoded` (reused), the writes keeping the take's bake.
    fn encode_effects(&mut self) {
        self.state.settle();
        let cells = &mut self.state.cells;
        std::mem::swap(&mut cells.taken_publish, &mut cells.publish_floats);
        cells.publish_floats.clear();
        std::mem::swap(&mut cells.taken_walls, &mut cells.wall_floats);
        cells.wall_floats.clear();
        let mut out = std::mem::take(&mut self.encoded);
        out.clear();
        self.cell_effects.clear();
        self.writes.clear();
        if let Some(old) = self.write_bake.take() {
            self.taken.recycle_bake(old.texels, old.rows);
        }
        let staged = self.staged.take();
        let attached = self.objects.is_some();
        for effect in self.state.out.drain(..) {
            // The cells' effects run in Rust (`apply_cells`), as runs in their place.
            if is_cell_effect(&effect) {
                if attached {
                    push_run(&mut out, 47, self.cell_effects.len() as i64);
                    self.cell_effects.push(effect);
                }
                continue;
            }
            // So do the uploads (`apply_writes`), each with the bake being placed.
            if let Effect::Write { tile, page, copy } = effect {
                // A write takes the bake staged for its own tile: one without it would draw the
                // slot's old bytes as this tile (AUDIT-stream 3), so it is refused loudly and the
                // tile baked again.
                let own = staged.as_ref().is_some_and(|bake| bake.names[..2] == [i64::from(tile.x), i64::from(tile.z)]);
                if !own {
                    debug_assert!(false, "a write of tile ({}, {}) without its staged bake", tile.x, tile.z);
                    godot_error!("graded write of tile ({}, {}) without its staged bake: baked again", tile.x, tile.z);
                    self.unwritten.push(tile);
                    continue;
                }
                push_run(&mut out, 48, self.writes.len() as i64);
                self.writes.push(PageWrite { page, copy });
                continue;
            }
            // An eviction's dropped staged result leaves the edit batch here ([`edits`]).
            if let Effect::EditDropped { tile } = effect {
                self.edits.dropped(tile, &mut self.taken);
                continue;
            }
            if let Effect::PageMade { id, code, slots, layers, .. } = effect {
                let side = self.state.pages.level(code).side as u32;
                self.page_shapes.insert(id, [side, slots, layers]);
            }
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
                Effect::Write { .. } => unreachable!("taken as a run"),
                Effect::Cancel { tile, stale } => [12, i64::from(tile.x), i64::from(tile.z), i64::from(stale), 0, 0, 0],
                Effect::Exhausted { tile, code } => [13, i64::from(tile.x), i64::from(tile.z), i64::from(code), 0, 0, 0],
                Effect::Evicted { tile, .. } => [14, i64::from(tile.x), i64::from(tile.z), 0, 0, 0, 0],
                Effect::EditDropped { .. } => unreachable!("taken by the edit batch"),
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
                Effect::Draw { key, state } => [40, key.0.into(), key.1.into(), state.into(), 0, 0, 0],
                Effect::Bind { .. }
                | Effect::Installed { .. }
                | Effect::MaskChanged { .. }
                | Effect::CellChanged { .. }
                | Effect::AwaitCells { .. } => unreachable!("settled"),
                effect => unreachable!("a cell effect, applied by the objects: {effect:?}"),
            };
            out.extend(fields);
        }
        self.encoded = out;
        for tile in self.unwritten.drain(..) {
            self.state.rewrite(tile);
        }
        match staged {
            Some(bake) if !self.writes.is_empty() => self.write_bake = Some(bake),
            Some(bake) => self.taken.recycle_bake(bake.texels, bake.rows),
            None => {}
        }
    }
    // --- The cells' objects (`RENDER-32-K`). ---

    /// Attaches the cells' `RenderingServer` objects after `cells_setup`: drawn in `scenario`
    /// with the seven lattice meshes and the wall quad (`meshes`, in that order), the graded
    /// shader and its behind-the-road variant, the wall material, the base lattice's anchor in
    /// half-metres, the patch material's parameters a set's materials sync (`uniforms`) and the
    /// triangles of each lattice mesh (the perf channel's count). Makes every prewarmed set's
    /// objects. O(sets).
    #[func]
    #[allow(clippy::too_many_arguments)]
    pub fn cells_attach(
        &mut self,
        scenario: Rid,
        meshes: Array<Rid>,
        graded_shader: Rid,
        behind_shader: Rid,
        wall_material: Rid,
        anchor: Vector2i,
        uniforms: PackedStringArray,
        triangles: PackedInt64Array,
    ) {
        self.cells_detach();
        let mut backend = GodotBackend::new(scenario);
        let names = ["cell_mode", "cell_patch_rect_hm", "cell_lattice_anchor_hm", "graded_texels", "graded_atlas"];
        let range = backend.add_catalogue(
            meshes.iter_shared().collect(),
            Vec::new(),
            names.iter().map(|name| StringName::from(*name)).collect(),
        );
        let catalogue = CellCatalogue {
            lattice: std::array::from_fn(|code| range.mesh + code as u32),
            wall: range.mesh + 7,
            mode: range.parameter,
            rect: range.parameter + 1,
            anchor: range.parameter + 2,
            texels: range.parameter + 3,
            atlas: range.parameter + 4,
            graded_shader: graded_shader.to_u64(),
            behind_shader: behind_shader.to_u64(),
            wall_material: wall_material.to_u64(),
            anchor_hm: [anchor.x, anchor.y],
            triangles: std::array::from_fn(|code| triangles.get(code).unwrap_or(0)),
        };
        let mut objects = CellObjects::new(catalogue, self.state.cells.per_patch as u32);
        objects.ensure(&mut backend, self.state.cells.sets.len());
        objects.pages = self.page_textures.clone();
        self.sync = MaterialSync {
            names: uniforms.as_slice().iter().map(StringName::from).collect(),
            watched: WATCHED.iter().map(|name| StringName::from(*name)).collect(),
            ..MaterialSync::default()
        };
        self.objects = Some(objects);
        self.backend = Some(backend);
    }

    /// Whether the cells draw at all (the terrain node's visibility). O(sets) on a change.
    #[func]
    pub fn cells_set_drawn(&mut self, drawn: bool) {
        if let (Some(objects), Some(backend)) = (self.objects.as_mut(), self.backend.as_mut()) {
            objects.set_drawn(backend, drawn);
        }
    }

    /// Frees the cells' objects (the renderer's disposal). O(sets).
    #[func]
    pub fn cells_detach(&mut self) {
        if let (Some(objects), Some(backend)) = (self.objects.as_mut(), self.backend.as_mut()) {
            objects.free_all(backend);
        }
        self.objects = None;
        self.backend = None;
        self.sync = MaterialSync::default();
        self.cell_effects.clear();
    }

    /// What resident patch `key`'s set draws with, from `terrain.gd`'s entry: the patch node's
    /// instance id, its material and its culling box.
    #[func]
    pub fn set_patch_draw(&mut self, key: Vector2i, node: i64, material: Rid, aabb: Aabb) {
        if let Some(objects) = self.objects.as_mut() {
            let aabb = Bounds {
                position: [aabb.position.x, aabb.position.y, aabb.position.z],
                size: [aabb.size.x, aabb.size.y, aabb.size.z],
            };
            objects.patches.insert((key.x, key.y), PatchDraw { node, material: material.to_u64(), aabb });
        }
    }

    /// Page `page`'s two textures (slot texels, row atlas), which a set's materials bind.
    #[func]
    pub fn set_page_textures(&mut self, page: i64, texels: Rid, atlas: Rid) {
        let textures = [texels.to_u64(), atlas.to_u64()];
        self.page_textures.insert(page, textures);
        if let Some(objects) = self.objects.as_mut() {
            objects.pages.insert(page, textures);
        }
    }

    /// Applies the writes `first` to `end` of the last take (a run, effect 48): each copy takes
    /// its bake's slot layer and row atlas run through the backend (`write_copy` of
    /// `terrain_graded_pages.gd`), traced as a `write` line. O(the copies' bytes).
    #[func]
    pub fn apply_writes(&mut self, first: i64, end: i64) {
        let Self { backend, writes, write_bake, page_textures, page_shapes, trace, upload_counts, .. } = self;
        let Some(bake) = write_bake.as_ref() else {
            return;
        };
        for write in &writes[first as usize..end as usize] {
            let (copy, names) = (write.copy, bake.names);
            if let Some(trace) = trace.as_mut() {
                trace_out(
                    trace,
                    format!(
                        "write {} {} {} {} {} {} {} {} {}",
                        write.page, copy.slot, copy.first, copy.layers, names[0], names[1], names[2], names[3], names[4]
                    ),
                );
            }
            upload_counts[0] += 1;
            upload_counts[1] += (bake.texels.as_slice().len() + bake.rows.as_slice().len()) as i64;
            let (Some(backend), Some(textures), Some(shape)) = (
                backend.as_mut(),
                page_textures.get(&i64::from(write.page)),
                page_shapes.get(&write.page),
            ) else {
                continue;
            };
            let start = std::time::Instant::now();
            uploads::write_copy(
                backend,
                *textures,
                shape[0],
                copy,
                bake.texels.as_slice(),
                bake.rows.as_slice(),
            );
            upload_counts[2] = upload_counts[2].max(start.elapsed().as_micros() as i64);
        }
    }

    /// The copies written since the book was made (the renderer's `uploads` counter).
    #[func]
    pub fn uploads(&self) -> i64 {
        self.upload_counts[0]
    }

    /// The bytes those copies wrote (`upload_bytes`).
    #[func]
    pub fn upload_bytes(&self) -> i64 {
        self.upload_counts[1]
    }

    /// The longest copy's write, microseconds (`upload_ms_max`).
    #[func]
    pub fn upload_us_max(&self) -> i64 {
        self.upload_counts[2]
    }

    /// Copies page `source`'s contents into page `target` (a block's move to a larger page:
    /// every slot layer and atlas layer the source has, into the same layers). O(the source's
    /// layers), device to device where the renderer has a device.
    #[func]
    pub fn copy_page(&mut self, source: i64, target: i64) {
        let (Some(backend), Some(from), Some(to), Some(&[side, slots, layers])) = (
            self.backend.as_mut(),
            self.page_textures.get(&source),
            self.page_textures.get(&target),
            self.page_shapes.get(&(source as u32)),
        ) else {
            return;
        };
        backend.texture_copy_layers(from[0], to[0], [side, side + 1], slots);
        backend.texture_copy_layers(from[1], to[1], [ATLAS_WIDTH, 1], layers);
    }

    /// Page `page` holds no copy any more (released, pooled when `kept`) or a pooled page was
    /// freed: what the backend keeps of its layers goes (Compatibility's CPU copies), and a page
    /// that is not kept leaves the book's maps (AUDIT-stream 13), so no freed texture's handle
    /// stays bindable. `O(1)`.
    #[func]
    pub fn forget_page(&mut self, page: i64, kept: bool) {
        if let (Some(backend), Some(textures)) = (self.backend.as_mut(), self.page_textures.get(&page)) {
            backend.texture_forget(textures[0]);
            backend.texture_forget(textures[1]);
        }
        if kept {
            return;
        }
        self.page_textures.remove(&page);
        self.page_shapes.remove(&(page as u32));
        if let Some(objects) = self.objects.as_mut() {
            objects.pages.remove(&page);
        }
    }

    /// The pages whose textures the book holds (tests: freed pages leave it).
    #[func]
    pub fn page_texture_count(&self) -> i64 {
        self.page_textures.len() as i64
    }

    /// The CPU copy of page `page`'s slot layer (`texels`) or atlas layer `layer` where the
    /// renderer keeps one (Compatibility, `TERRAIN-22`); empty where none was written. For tests.
    #[func]
    pub fn page_layer(&self, page: i64, texels: bool, layer: i64) -> PackedByteArray {
        match (self.backend.as_ref(), self.page_textures.get(&page)) {
            (Some(backend), Some(textures)) => backend.mirror_layer(textures[usize::from(!texels)], layer as u32),
            _ => PackedByteArray::new(),
        }
    }

    /// The bytes of every page's CPU copies (Compatibility; 0 elsewhere).
    #[func]
    pub fn page_mirror_bytes(&self) -> i64 {
        self.backend.as_ref().map_or(0, GodotBackend::mirror_bytes)
    }

    /// Applies the cell effects `first` to `end` of the last take (a run, effect 47) to the
    /// sets' objects, in order: the trace's publication and wall lines, the material syncs of
    /// sets that start drawing or take the variant. O(the effects + their publications).
    #[func]
    pub fn apply_cells(&mut self, first: i64, end: i64) {
        let Self { state, objects, backend, sync, cell_effects, cell_counts, trace, .. } = self;
        let (Some(objects), Some(backend)) = (objects.as_mut(), backend.as_mut()) else {
            return;
        };
        let mut server = godot::classes::RenderingServer::singleton();
        for &effect in &cell_effects[first as usize..end as usize] {
            let start = std::time::Instant::now();
            let applied = objects.apply(
                backend,
                &effect,
                &state.resident,
                &state.cells.taken_publish,
                &state.cells.taken_walls,
            );
            let mut materials = [Rid::Invalid; 8];
            match applied {
                Applied::None | Applied::Begun { .. } => {}
                Applied::Published { key, mesh, at } => {
                    if let Some(trace) = trace.as_mut() {
                        let length = state.cells.per_patch * 16;
                        let at = at as usize;
                        let hash = bytes_hash(&state.cells.taken_publish[at..at + length]);
                        trace_out(trace, format!("cells {} {} {} {}", key.0, key.1, mesh, hash));
                    }
                }
                Applied::Walls { key, start: from, count, instances, truncated } => {
                    if let Some(trace) = trace.as_mut() {
                        let from = from as usize;
                        let faces = &state.cells.taken_walls[from..from + count as usize * WALL_INSTANCE_FLOATS];
                        trace_out(
                            trace,
                            format!("walls {} {} {} {} {} {}", key.0, key.1, count, instances, truncated, bytes_hash(faces)),
                        );
                    }
                    if truncated > 0 {
                        cell_counts[TRUNCATED] += i64::from(truncated);
                        crate::debug_log!(
                            "terrain",
                            "graded walls: {truncated} faces past the patch capacity {instances}"
                        );
                    }
                    cell_counts[WALLS_US] += start.elapsed().as_micros() as i64;
                }
                Applied::Shown { set, key } => {
                    let source = objects.patches.get(&key).map(|patch| Rid::new(patch.material));
                    let count = collect(objects, set, &mut materials);
                    let sync_start = std::time::Instant::now();
                    if let Some(source) = source
                        && sync.watched_moved(&server, set as usize, source)
                    {
                        sync.run(&mut server, set as usize, &materials[..count], source, 0, None);
                        objects.sets[set as usize].sync_next = 0;
                    }
                    cell_counts[SYNC_US] += sync_start.elapsed().as_micros() as i64;
                    cell_counts[FLIP_US] += start.elapsed().as_micros() as i64;
                }
                Applied::Rebuilt { set, key } => {
                    let source = objects.patches.get(&key).map(|patch| Rid::new(patch.material));
                    let count = collect(objects, set, &mut materials);
                    let sync_start = std::time::Instant::now();
                    if let Some(source) = source {
                        sync.run(&mut server, set as usize, &materials[..count], source, 0, None);
                        objects.sets[set as usize].sync_next = 0;
                    }
                    cell_counts[SYNC_US] += sync_start.elapsed().as_micros() as i64;
                }
                Applied::Variant { set } => {
                    // The variant's new materials take the values the set holds.
                    sync.ensure(set as usize);
                    let count = collect(objects, set, &mut materials);
                    for material in materials[..count].iter().skip(count.saturating_sub(4)) {
                        for (name, value) in sync.names.iter().zip(&sync.values[set as usize]) {
                            if !value.is_nil() {
                                server.material_set_param(*material, name, value);
                            }
                        }
                    }
                }
                Applied::Hidden => cell_counts[FLIP_US] += start.elapsed().as_micros() as i64,
                Applied::Built => cell_counts[BUILDS] += 1,
            }
        }
    }

    /// Set `set`'s material sync for its build (`_sync_material` from the build path): goes on
    /// where the last call stopped and stops once the frame's install budget (from `start_us`)
    /// is spent, after one parameter at least. Returns whether the set's materials are synced
    /// for this build. O(parameters synced x the set's materials).
    #[func]
    pub fn sync_build(&mut self, set: i64, start_us: i64) -> bool {
        let Self { state, objects, sync, trace, .. } = self;
        let Some(objects) = objects.as_mut() else {
            return true;
        };
        let at = set as usize;
        if objects.sets[at].synced {
            return true;
        }
        let key = state.cells.sets[at].key;
        let Some(source) = objects.patches.get(&key).map(|patch| Rid::new(patch.material)) else {
            return false;
        };
        let mut materials = [Rid::Invalid; 8];
        let count = collect(objects, set as u32, &mut materials);
        let unbudgeted = state.unbudgeted;
        let mut trace = trace.clone();
        let mut over = move || over_budget(&mut trace, unbudgeted, start_us);
        let mut server = godot::classes::RenderingServer::singleton();
        let next = objects.sets[at].sync_next;
        let stopped = sync.run(&mut server, at, &materials[..count], source, next, Some(&mut over));
        let objects = &mut objects.sets[at];
        objects.sync_next = stopped.unwrap_or(0);
        objects.synced = stopped.is_none();
        objects.synced
    }

    /// Patch `key`'s material may have changed (`process`'s changed materials): its drawing
    /// set's materials sync where the watched parameters moved. O(1), and the parameters when
    /// they moved.
    #[func]
    pub fn material_changed(&mut self, key: Vector2i) {
        let key = (key.x, key.y);
        let Self { state, objects, sync, cell_counts, .. } = self;
        let Some(objects) = objects.as_mut() else {
            return;
        };
        let Some(&set) = state.cells.by_patch.get(&key) else {
            return;
        };
        if !state.resident.contains_key(&key) || !objects.parent(set).1 {
            return;
        }
        let Some(source) = objects.patches.get(&key).map(|patch| Rid::new(patch.material)) else {
            return;
        };
        let start = std::time::Instant::now();
        let mut server = godot::classes::RenderingServer::singleton();
        if sync.watched_moved(&server, set as usize, source) {
            let mut materials = [Rid::Invalid; 8];
            let count = collect(objects, set, &mut materials);
            sync.run(&mut server, set as usize, &materials[..count], source, 0, None);
            objects.sets[set as usize].sync_next = 0;
        }
        cell_counts[SYNC_US] += start.elapsed().as_micros() as i64;
    }

    /// Whether patch `key`'s set draws under a node other than `node` (the patch's node changed).
    #[func]
    pub fn cells_node_moved(&self, key: Vector2i, node: i64) -> bool {
        let (Some(objects), Some(&set)) = (self.objects.as_ref(), self.state.cells.by_patch.get(&(key.x, key.y))) else {
            return false;
        };
        let (parent, shown) = objects.parent(set);
        shown && parent != node
    }

    /// Whether patch `key` has a set drawing under patch node `node` (`hole_patches`).
    #[func]
    pub fn cells_drawn_under(&self, key: Vector2i, node: i64) -> bool {
        let (Some(objects), Some(&set)) = (self.objects.as_ref(), self.state.cells.by_patch.get(&(key.x, key.y))) else {
            return false;
        };
        objects.drawn_under(set, node)
    }

    /// The cell counters: the frame's material sync, wall and flip microseconds, then walls
    /// truncated, sets given the variant, multimeshes and materials made, set builds completed.
    #[func]
    pub fn cell_counters(&self) -> PackedInt64Array {
        let mut out = self.cell_counts;
        if let Some(objects) = self.objects.as_ref() {
            out[4] = objects.variants_created as i64;
            out[5] = objects.multimeshes_created as i64;
            out[6] = objects.materials_created as i64;
        }
        PackedInt64Array::from(out.as_slice())
    }

    /// Set `set`'s objects for tests, grass and the probe: `multimeshes` (RIDs by cell mesh),
    /// `counts` (instances drawn as sent), `has` (a mesh with instances), `drawn` (its instance
    /// draws), `bound` (its material override, or an invalid RID), `textures` (per mesh the
    /// (texels, atlas) its material binds, invalid RIDs for none or the base cells), `walls`
    /// (wall instances drawn as sent), `walls_drawn`, `parent` (the patch node's instance id, 0
    /// none) and `shown`.
    #[func]
    pub fn cell_objects(&self, set: i64) -> VarDictionary {
        let mut out = VarDictionary::new();
        let Some(objects) = self.objects.as_ref() else {
            return out;
        };
        let set = set as u32;
        let mut multimeshes = Array::<Rid>::new();
        let mut counts = PackedInt32Array::new();
        let mut has = PackedByteArray::new();
        let mut drawn = PackedByteArray::new();
        let mut bound = Array::<Rid>::new();
        for (multimesh, count, with, draws, material) in objects.parts(set) {
            multimeshes.push(Rid::new(multimesh));
            counts.push(count as i32);
            has.push(u8::from(with));
            drawn.push(u8::from(draws));
            bound.push(material.map_or(Rid::Invalid, Rid::new));
        }
        let mut textures = Array::<Rid>::new();
        for bound in objects.textures(set) {
            for texture in bound {
                textures.push(texture.map_or(Rid::Invalid, Rid::new));
            }
        }
        let (walls, walls_drawn) = objects.walls(set);
        let (parent, shown) = objects.parent(set);
        out.set("multimeshes", multimeshes);
        out.set("counts", counts);
        out.set("has", has);
        out.set("drawn", drawn);
        out.set("bound", bound);
        out.set("textures", textures);
        out.set("walls", walls as i64);
        out.set("walls_drawn", walls_drawn);
        out.set("parent", parent);
        out.set("shown", shown);
        out
    }

    /// The draw census of every set's cell meshes and walls (`RENDER-44`, the stream benchmark's
    /// census only): parallel arrays `multimesh`, `instance`, `count` (instances as sent),
    /// `drawn` (the instance draws), `wall`, `parent` (the patch node's instance id) and
    /// `shown` (its set draws). Empty without cells. `O(sets' meshes)`.
    #[func]
    pub fn draw_census(&self) -> VarDictionary {
        let mut out = VarDictionary::new();
        let Some(objects) = self.objects.as_ref() else {
            return out;
        };
        let (mut multimesh, mut instance, mut parent) =
            (PackedInt64Array::new(), PackedInt64Array::new(), PackedInt64Array::new());
        let (mut count, mut drawn, mut wall, mut shown) = (
            PackedInt32Array::new(),
            PackedByteArray::new(),
            PackedByteArray::new(),
            PackedByteArray::new(),
        );
        for (mesh, at, sent, draws, walls, node, set_shown) in objects.census() {
            multimesh.push(mesh as i64);
            instance.push(at as i64);
            count.push(sent as i32);
            drawn.push(u8::from(draws));
            wall.push(u8::from(walls));
            parent.push(node);
            shown.push(u8::from(set_shown));
        }
        out.set("multimesh", multimesh);
        out.set("instance", instance);
        out.set("count", count);
        out.set("drawn", drawn);
        out.set("wall", wall);
        out.set("parent", parent);
        out.set("shown", shown);
        out
    }

    /// Patch `key`'s cell meshes for grass's mask mirrors, flat per mesh `(multimesh RID id,
    /// has instances, page its material binds or -1)`; empty without a set. O(the set's meshes).
    #[func]
    pub fn mask_sources(&self, key: Vector2i) -> PackedInt64Array {
        let (Some(objects), Some(&set)) = (self.objects.as_ref(), self.state.cells.by_patch.get(&(key.x, key.y))) else {
            return PackedInt64Array::new();
        };
        let pages = self.state.cells.sets[set as usize].pages;
        let mut out = Vec::with_capacity(27);
        for (index, (multimesh, _, has, _, _)) in objects.parts(set).enumerate() {
            let level = if index >= CELL_BEHIND_INDEX { index - CELL_BEHIND_INDEX } else { index };
            let page = if index == CELL_BASE_INDEX { -1 } else { pages[level] };
            out.extend([multimesh as i64, i64::from(has), page]);
        }
        PackedInt64Array::from(out.as_slice())
    }

    /// What the drawing sets submit (before culling), instances then triangles per cell mesh,
    /// for the perf channel. O(drawing sets' meshes).
    #[func]
    pub fn submitted_cells(&self) -> PackedInt64Array {
        let mut out = [0i64; 18];
        if let Some(objects) = self.objects.as_ref() {
            for (_, &set) in self.state.cells.by_patch.iter() {
                objects.submitted(set, &mut out);
            }
        }
        PackedInt64Array::from(out.as_slice())
    }
}

// The set's graded materials, in mesh order, into `out`; returns how many.
fn collect(objects: &CellObjects, set: u32, out: &mut [Rid; 8]) -> usize {
    let mut count = 0;
    for material in objects.materials(set) {
        out[count] = Rid::new(material);
        count += 1;
    }
    count
}

// Encodes effect `index` of a run kind (47 cells, 48 writes) into `out`: extends the last run
// when it is of the same kind and ends at `index`, else starts one.
fn push_run(out: &mut Vec<i64>, kind: i64, index: i64) {
    let last = out.len().saturating_sub(EFFECT_FIELDS);
    if out.len() >= EFFECT_FIELDS && out[last] == kind && out[last + 2] == index {
        out[last + 2] = index + 1;
    } else {
        out.extend([kind, index, index + 1, 0, 0, 0, 0]);
    }
}

fn trace_out(trace: &mut Gd<RefCounted>, line: String) {
    trace.call("out", &[GString::from(line.as_str()).to_variant()]);
}

const CELL_BASE_INDEX: usize = super::cells::CELL_BASE;
const CELL_BEHIND_INDEX: usize = super::cells::CELL_BEHIND;
