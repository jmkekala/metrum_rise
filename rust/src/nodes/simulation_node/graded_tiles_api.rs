// SPDX-License-Identifier: GPL-2.0-only

//! Graded terrain residency (`TERRAIN-07-D`, `docs/terrain.md` "Residency and eviction"). Godot
//! owns residency: it requests the graded tile lists of its resident patches and the bakes of
//! the tiles its distance rule selects, each with a token it compares on arrival. The payload
//! workers take a bounded snapshot under `try_lock` (the roadbed and the heightmap in `O(1)`
//! shared chunks, the sites of the requested windows copied), work off the lock and publish the
//! lists and the packed slot bytes, which Godot polls within its install budget. Rust keeps no
//! result once it is polled; eviction is Godot freeing the slot.
//!
//! Edits (item 5): the poll takes the core's graded edit journal under `try_lock` and a worker
//! selects each edit's dirty tiles at every level from the edit's own snapshots; Godot rebakes
//! the dirty tiles it holds at those levels and installs them together. A road commit's or
//! undo's install carries the tiles it staged (`TERRAIN-07-E`), packed by the same worker:
//! Godot installs those with the edit instead of rebaking them.

use super::*;
use crate::nodes::sim::core::{GradedEditSelection, GradedInstallTiles};
use crate::simulation::buildings::allocator::SiteSnapshot;
use crate::simulation::terrain::graded::{
    BakeError, CAP_M, Level, PatchTilesScratch, SelectScratch, TILE_M, TileKey, TilePayload,
    WALL_FACE_BYTES, bake_tiles, graded_tiles_in_rect, pack_slot, pack_walls, slot_row_texels,
    slot_side,
};
use std::cell::RefCell;
use std::thread::LocalKey;

/// Most tiles in flight at once, from the request until Godot polls the result (baking or
/// finished): the request budget that bounds the payloads Rust holds and the packing buffers its
/// pool keeps (`TERRAIN-07-G` round 6: a finished result left the count before its poll, so the
/// results queued behind Godot's install budget after a load grew without bound, and the pool
/// kept their buffers).
const MAX_IN_FLIGHT_TILES: usize = 96;
/// Most patch tile lists in flight at once.
const MAX_IN_FLIGHT_PATCHES: usize = 32;
// Window growth beyond what a bake or a list reads: a larger value only copies more sites.
const SNAPSHOT_PAD_M: f64 = 2.0;
// Godot's level codes: `GL0`-`GL2`, then `GL3` with its step in base cells.
const LEVEL_CODES: usize = 4;
const GL3_STEPS: [u8; 3] = [2, 4, 8];

/// One tile bake Godot asked for, with the token it compares on arrival.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GradedTileRequest {
    tile: TileKey,
    level: Level,
    token: i64,
}

/// One patch tile list Godot asked for.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GradedPatchRequest {
    patch: (usize, usize),
    token: i64,
}

// Why a request produced nothing: a busy lock or an unready site index is retried by Godot; a
// representation error (`T7`) is not.
#[derive(Clone, Copy, Debug)]
enum Failure {
    Busy,
    Bake(BakeError),
}

// A baked tile packed for upload (its wall faces as `pack_walls` packs them); the byte buffers
// return to the pool when polled.
struct TileResult {
    request: GradedTileRequest,
    samples: [usize; 2],
    graded: bool,
    // Its cells draw the behind-the-road variant (T49 (c)).
    behind: bool,
    seams: usize,
    edges: usize,
    texels: Vec<u8>,
    rows: Vec<u8>,
    walls: Vec<u8>,
}

// One edit's selection with its install's staged tiles packed (token: the edit's serial), which
// of them the preview showed (`GradedInstallTiles::adopted`) and the preview epoch they release.
struct EditResult {
    selection: GradedEditSelection,
    staged: Vec<TileResult>,
    adopted: Vec<bool>,
    epoch: Option<u64>,
}

impl TileResult {
    // Packs `payload` for upload into `buffers` (texels, rows, walls).
    fn packed(
        request: GradedTileRequest,
        payload: &TilePayload,
        side: usize,
        buffers: [Vec<u8>; 3],
    ) -> Self {
        let [mut texels, mut rows, mut walls] = buffers;
        pack_slot(payload, side, &mut texels, &mut rows);
        pack_walls(payload, &mut walls);
        Self {
            request,
            samples: payload.samples,
            graded: payload.graded,
            behind: payload.behind,
            seams: payload.seams.len(),
            edges: payload.edges.len(),
            texels,
            rows,
            walls,
        }
    }

    // The tile as `poll_graded_tile_results` returns it.
    fn to_dict(&self) -> VarDictionary {
        let (code, step) = level_code(self.request.level);
        let mut dict = VarDictionary::new();
        dict.set("tile_x", i64::from(self.request.tile.x));
        dict.set("tile_z", i64::from(self.request.tile.z));
        dict.set("level", code);
        dict.set("step", step);
        dict.set("token", self.request.token);
        dict.set("samples_x", self.samples[0] as i64);
        dict.set("samples_z", self.samples[1] as i64);
        dict.set("graded", self.graded);
        dict.set("behind", self.behind);
        dict.set("seam_count", self.seams as i64);
        dict.set("edge_count", self.edges as i64);
        dict.set("wall_count", (self.walls.len() / WALL_FACE_BYTES) as i64);
        dict.set("texels", PackedByteArray::from(self.texels.as_slice()));
        dict.set("rows", PackedByteArray::from(self.rows.as_slice()));
        dict.set("walls", PackedByteArray::from(self.walls.as_slice()));
        dict
    }
}

/// The slot side of `level`'s pool: one pool per level code, `GL3` layers sized for its finest
/// step.
pub(crate) fn pool_side(level: Level, cell_m: f64) -> usize {
    let pool_level = match level {
        Level::Gl3 { .. } => Level::Gl3 { step: GL3_STEPS[0] },
        level => level,
    };
    slot_side(pool_level, cell_m)
}

/// Staged tiles packed as `poll_graded_tile_results` returns its tiles, each with `token`
/// (`TERRAIN-07-E`: the road preview's shown tiles, a commit's installed tiles).
pub(crate) fn packed_tile_dicts(
    tiles: &[crate::simulation::terrain::graded::StagedTile],
    token: i64,
    cell_m: f64,
) -> VarArray {
    let mut out = VarArray::new();
    for staged in tiles {
        let request = GradedTileRequest {
            tile: staged.tile,
            level: staged.level,
            token,
        };
        let side = pool_side(staged.level, cell_m);
        let result = TileResult::packed(request, &staged.payload, side, Default::default());
        out.push(&result.to_dict().to_variant());
    }
    out
}

/// The graded residency jobs of one node: requests in flight, finished results and failures
/// waiting for Godot's poll, and the byte buffers results are packed into, reused.
#[derive(Default)]
pub(crate) struct GradedResidencyJobs {
    tiles_in_flight: HashSet<(TileKey, Level)>,
    patches_in_flight: HashSet<(usize, usize)>,
    tiles: Vec<TileResult>,
    patches: Vec<(GradedPatchRequest, Vec<TileKey>)>,
    edits: Vec<EditResult>,
    failed_tiles: Vec<(GradedTileRequest, Failure)>,
    failed_patches: Vec<(GradedPatchRequest, Failure)>,
    buffers: Vec<Vec<u8>>,
}

// What one worker batch reads: shared in `O(1)` or copied for its windows only.
struct GradedInputs {
    terrain: crate::simulation::terrain::TerrainSystem,
    roadbed: crate::simulation::network::roadbed::RoadbedSystem,
    sites: SiteSnapshot,
}

thread_local! {
    // Per pool thread, reused between batches.
    static BAKED: RefCell<Vec<TilePayload>> = RefCell::default();
    static LISTS: RefCell<PatchTilesScratch> = RefCell::default();
    static SELECT: RefCell<SelectScratch> = RefCell::default();
}

// Runs `f` with this thread's scratch taken out of `key` and put back after. The bakes and the
// edit selections run Rayon inside, where this thread may steal another job of the same kind:
// that job finds an empty scratch, where a `RefCell` borrow held across the join panicked
// ("already borrowed", round 4, the Kuopio reference save).
fn with_scratch<T: Default + 'static, R>(
    key: &'static LocalKey<RefCell<T>>,
    f: impl FnOnce(&mut T) -> R,
) -> R {
    let mut scratch = key.take();
    let out = f(&mut scratch);
    key.set(scratch);
    out
}

fn level_code(level: Level) -> (i64, i64) {
    match level {
        Level::Gl0 => (0, 0),
        Level::Gl1 => (1, 0),
        Level::Gl2 => (2, 0),
        Level::Gl3 { step } => (3, i64::from(step)),
    }
}

fn level_from_code(code: i64, step: i64) -> Option<Level> {
    match code {
        0 => Some(Level::Gl0),
        1 => Some(Level::Gl1),
        2 => Some(Level::Gl2),
        3 => GL3_STEPS
            .into_iter()
            .find(|&s| i64::from(s) == step)
            .map(|step| Level::Gl3 { step }),
        _ => None,
    }
}

fn failure_label(failure: Failure) -> &'static str {
    match failure {
        Failure::Busy => "busy",
        Failure::Bake(BakeError::SitesUnavailable) => "sites_unavailable",
        Failure::Bake(BakeError::SeamCapacity { .. }) => "seam_capacity",
        Failure::Bake(BakeError::EdgeCapacity { .. }) => "edge_capacity",
        Failure::Bake(BakeError::ExtraCapacity { .. }) => "extra_capacity",
    }
}

// The rectangle a bake of `tile` at `level` reads, grown: the lattice with its apron, the cap
// or the band, the bake's own window pad.
fn tile_window(tile: TileKey, level: Level, cell_m: f64) -> [f64; 4] {
    let [x, z] = tile.origin();
    let grow = level.spacing_m(cell_m) + CAP_M.max(level.band_m(cell_m)) + 1.0 + SNAPSHOT_PAD_M;
    [x - grow, z - grow, x + TILE_M + grow, z + TILE_M + grow]
}

fn patch_rect(
    terrain: &crate::simulation::terrain::TerrainSystem,
    patch: (usize, usize),
) -> Option<[f64; 4]> {
    terrain
        .render_patch_world_bounds(patch.0, patch.1)
        .map(|(a, b, c, d)| [a, b, c, d].map(f64::from))
}

impl GradedInputs {
    // Takes the snapshot under `try_lock`: `None` when the core is busy, its roadbed does not
    // match the graph yet, or its site index is being rebuilt.
    fn take(
        core: &Mutex<SimCore>,
        windows: impl Fn(&crate::simulation::terrain::TerrainSystem) -> Vec<[f64; 4]>,
    ) -> Option<Self> {
        let mut core = SimulationNode::try_lock_shared_core(core)?;
        if !core.transit_network.roadbed.is_current() {
            return None;
        }
        // The site chunk index is derived and rebuilt on demand, as the refined terrain
        // payloads prepare it.
        let zone_cell_m = core.config.zone_cell_m;
        core.allocator
            .prepare_building_site_query_index(zone_cell_m);
        let sites = core
            .allocator
            .graded_site_snapshot(windows(&core.heightmap))?;
        let roadbed = &core.transit_network.roadbed;
        Some(Self {
            terrain: core.heightmap.clone(),
            roadbed: roadbed.clone(),
            sites,
        })
    }
}

impl GradedResidencyJobs {
    fn buffer(&mut self) -> Vec<u8> {
        self.buffers.pop().unwrap_or_default()
    }

    // Admits a tile request within the in-flight bound, unless the same tile and level is in
    // flight already.
    fn admit(&mut self, tile: TileKey, level: Level) -> bool {
        self.tiles_in_flight.len() < MAX_IN_FLIGHT_TILES
            && self.tiles_in_flight.insert((tile, level))
    }

    // Publishes one baked request: a finished tile packed into `buffers` stays in flight until
    // its poll (take_tiles); a failure leaves the count at once and returns the buffers.
    fn publish(
        &mut self,
        request: &GradedTileRequest,
        outcome: Result<(), BakeError>,
        payload: &TilePayload,
        [texels, rows, walls]: [Vec<u8>; 3],
    ) {
        if let Err(error) = outcome {
            self.tiles_in_flight.remove(&(request.tile, request.level));
            self.failed_tiles.push((*request, Failure::Bake(error)));
            self.buffers.extend([texels, rows, walls]);
            return;
        }
        self.tiles.push(TileResult {
            request: *request,
            samples: payload.samples,
            graded: payload.graded,
            behind: payload.behind,
            seams: payload.seams.len(),
            edges: payload.edges.len(),
            texels,
            rows,
            walls,
        });
    }

    // Takes at most `max` finished tiles for Godot's poll, oldest first; they leave the
    // in-flight count here.
    fn take_tiles(&mut self, max: usize) -> Vec<TileResult> {
        let take = max.min(self.tiles.len());
        let ready: Vec<TileResult> = self.tiles.drain(..take).collect();
        for result in &ready {
            self.tiles_in_flight.remove(&(result.request.tile, result.request.level));
        }
        ready
    }
}

// Bakes one level's requests, a failing batch tile by tile so each failure is its own.
fn bake_level(
    inputs: &GradedInputs,
    level: Level,
    requests: &[GradedTileRequest],
    jobs: &Mutex<GradedResidencyJobs>,
) {
    let tiles: Vec<TileKey> = requests.iter().map(|request| request.tile).collect();
    let cell_m = f64::from(inputs.terrain.cell_size_m());
    let side = pool_side(level, cell_m);
    with_scratch(&BAKED, |baked| {
        let mut outcomes: Vec<Result<(), BakeError>> = Vec::with_capacity(tiles.len());
        match bake_tiles(
            &inputs.terrain,
            &inputs.roadbed,
            &inputs.sites,
            &tiles,
            level,
            baked,
        ) {
            Ok(()) => outcomes.resize(tiles.len(), Ok(())),
            Err(_) => {
                // Rare: a representation error. Bake one by one to tell the failing tiles apart.
                let mut single = Vec::with_capacity(1);
                let mut payloads = Vec::with_capacity(tiles.len());
                for &tile in &tiles {
                    let outcome = bake_tiles(
                        &inputs.terrain,
                        &inputs.roadbed,
                        &inputs.sites,
                        &[tile],
                        level,
                        &mut single,
                    );
                    payloads.push(single.pop().unwrap_or_default());
                    outcomes.push(outcome);
                }
                *baked = payloads;
            }
        }
        // Pack off the jobs lock, which Godot's poll takes on the main thread: only taking the
        // pooled buffers and publishing the results hold it.
        let mut buffers: Vec<[Vec<u8>; 3]> = {
            let mut state = jobs.lock().unwrap();
            (0..requests.len())
                .map(|_| [state.buffer(), state.buffer(), state.buffer()])
                .collect()
        };
        for ((outcome, payload), [texels, rows, walls]) in
            outcomes.iter().zip(baked.iter()).zip(buffers.iter_mut())
        {
            if outcome.is_ok() {
                pack_slot(payload, side, texels, rows);
                pack_walls(payload, walls);
            }
        }
        let mut state = jobs.lock().unwrap();
        for (((request, outcome), payload), buffers) in
            requests.iter().zip(outcomes).zip(baked.iter()).zip(buffers)
        {
            state.publish(request, outcome, payload, buffers);
        }
    });
}

#[godot_api(secondary)]
impl SimulationNode {
    /// The graded tile slot layout Godot sizes its pools by: `tile_m`, and per level code
    /// (0-2 `GL0`-`GL2`, 3 `GL3`) a dictionary with `spacing_m` (`GL3`: at its finest step),
    /// `side` (texel layer side, apron included), `row_texels` (seam row, edge row, then the
    /// seams' heights), `seam_capacity`, `edge_capacity` and `band_m`; plus `gl3_steps`.
    #[func]
    pub fn get_graded_tile_layout(&self) -> VarDictionary {
        let cell_m = f64::from(self.snapshot.read().unwrap().terrain_cell_m);
        let mut levels = VarArray::new();
        for code in 0..LEVEL_CODES as i64 {
            let Some(level) = level_from_code(code, i64::from(GL3_STEPS[0])) else {
                continue;
            };
            let mut dict = VarDictionary::new();
            dict.set("code", code);
            dict.set("spacing_m", level.spacing_m(cell_m));
            dict.set("side", slot_side(level, cell_m) as i64);
            dict.set("row_texels", slot_row_texels(level) as i64);
            dict.set("seam_capacity", level.seam_capacity() as i64);
            dict.set("edge_capacity", level.edge_capacity() as i64);
            dict.set("band_m", level.band_m(cell_m));
            levels.push(&dict.to_variant());
        }
        let mut layout = VarDictionary::new();
        layout.set("tile_m", TILE_M);
        layout.set("levels", levels);
        layout.set(
            "gl3_steps",
            PackedInt32Array::from(GL3_STEPS.map(i32::from).as_slice()),
        );
        layout
    }

    /// Requests the sorted graded tile lists of render patches, from flat `(patch_x, patch_z,
    /// token)` triples. Returns one byte per request, 1 where it was queued; a patch already in
    /// flight, or one beyond the in-flight budget, is not, and Godot asks again later.
    #[func]
    pub fn request_graded_patch_tiles(
        &mut self,
        flat_requests: PackedInt64Array,
    ) -> PackedByteArray {
        let mut requests = Vec::new();
        let mut accepted = Vec::new();
        {
            let mut jobs = self.graded_jobs.lock().unwrap();
            for chunk in flat_requests.as_slice().as_chunks::<3>().0 {
                let (Ok(x), Ok(z)) = (usize::try_from(chunk[0]), usize::try_from(chunk[1])) else {
                    accepted.push(0);
                    continue;
                };
                if jobs.patches_in_flight.len() >= MAX_IN_FLIGHT_PATCHES
                    || !jobs.patches_in_flight.insert((x, z))
                {
                    accepted.push(0);
                    continue;
                }
                accepted.push(1);
                requests.push(GradedPatchRequest {
                    patch: (x, z),
                    token: chunk[2],
                });
            }
        }
        let accepted = PackedByteArray::from(accepted.as_slice());
        if requests.is_empty() {
            return accepted;
        }
        let (core, jobs) = (Arc::clone(&self.core), Arc::clone(&self.graded_jobs));
        self.async_jobs.spawn(move || {
            let inputs = GradedInputs::take(&core, |terrain| {
                requests
                    .iter()
                    .filter_map(|request| patch_rect(terrain, request.patch))
                    .map(|[a, b, c, d]| {
                        // `graded_tiles_in_rect` reads the patch grown by the cap and 1 m.
                        let grow = CAP_M + 1.0 + SNAPSHOT_PAD_M;
                        [a - grow, b - grow, c + grow, d + grow]
                    })
                    .collect()
            });
            let lists: Vec<_> = requests
                .iter()
                .map(|request| {
                    let inputs = inputs.as_ref().ok_or(Failure::Busy)?;
                    let rect = patch_rect(&inputs.terrain, request.patch).ok_or(Failure::Busy)?;
                    let mut tiles = Vec::new();
                    LISTS
                        .with_borrow_mut(|scratch| {
                            graded_tiles_in_rect(
                                &inputs.terrain,
                                &inputs.roadbed,
                                &inputs.sites,
                                rect,
                                scratch,
                                &mut tiles,
                            )
                        })
                        .map_err(Failure::Bake)?;
                    Ok(tiles)
                })
                .collect();
            let mut state = jobs.lock().unwrap();
            for (request, list) in requests.into_iter().zip(lists) {
                state.patches_in_flight.remove(&request.patch);
                match list {
                    Ok(tiles) => state.patches.push((request, tiles)),
                    Err(failure) => state.failed_patches.push((request, failure)),
                }
            }
        });
        accepted
    }

    /// Requests tile bakes from flat `(tile_x, tile_z, level_code, gl3_step, token)` quintuples
    /// (level codes as [`Self::get_graded_tile_layout`]). Returns one byte per request, 1 where
    /// it was queued; a `(tile, level)` already in flight, an unknown level, or a request beyond
    /// the in-flight budget is not, and Godot asks again on a later frame.
    #[func]
    pub fn request_graded_tiles(&mut self, flat_requests: PackedInt64Array) -> PackedByteArray {
        let mut requests = Vec::new();
        let mut accepted = Vec::new();
        {
            let mut jobs = self.graded_jobs.lock().unwrap();
            for chunk in flat_requests.as_slice().as_chunks::<5>().0 {
                let (Ok(x), Ok(z)) = (i32::try_from(chunk[0]), i32::try_from(chunk[1])) else {
                    accepted.push(0);
                    continue;
                };
                let Some(level) = level_from_code(chunk[2], chunk[3]) else {
                    accepted.push(0);
                    continue;
                };
                let tile = TileKey { x, z };
                if !jobs.admit(tile, level) {
                    accepted.push(0);
                    continue;
                }
                accepted.push(1);
                requests.push(GradedTileRequest {
                    tile,
                    level,
                    token: chunk[4],
                });
            }
        }
        let accepted = PackedByteArray::from(accepted.as_slice());
        if requests.is_empty() {
            return accepted;
        }
        let (core, jobs) = (Arc::clone(&self.core), Arc::clone(&self.graded_jobs));
        self.async_jobs.spawn(move || {
            let inputs = GradedInputs::take(&core, |terrain| {
                let cell_m = f64::from(terrain.cell_size_m());
                requests
                    .iter()
                    .map(|request| tile_window(request.tile, request.level, cell_m))
                    .collect()
            });
            let Some(inputs) = inputs else {
                let mut state = jobs.lock().unwrap();
                for request in requests {
                    state.tiles_in_flight.remove(&(request.tile, request.level));
                    state.failed_tiles.push((request, Failure::Busy));
                }
                return;
            };
            // One batch per level, in a fixed order: each tile's bytes depend on nothing else.
            let mut by_level: Vec<GradedTileRequest> = requests;
            by_level.sort_by_key(|request| (level_code(request.level), request.tile));
            for group in by_level.chunk_by(|a, b| a.level == b.level) {
                bake_level(&inputs, group[0].level, group, &jobs);
            }
        });
        accepted
    }

    /// Returns finished graded residency work: `edits` (per published edit its `serial`, `all`
    /// for a reset, and per level `level`, `step` and the dirty `tiles`, flat `(x, z)` pairs in
    /// key order; Godot rebakes the tiles it holds at that level; a commit's or undo's own tiles
    /// in `staged`, packed as `tiles` are, each with `adopted`, true where it is the payload the
    /// road preview showed, and the `preview_epoch` it releases), `patches` (per list `patch_x`, `patch_z`,
    /// `token` and `tiles`, flat `(x, z)` pairs in key order), at most `max_tiles` baked tiles in
    /// `tiles` (per tile `tile_x`, `tile_z`, `level`, `step`, `token`, `samples_x`, `samples_z`,
    /// `graded`, `seam_count`, `edge_count`, `wall_count`, the slot layers' little-endian
    /// RGBAF bytes `texels` and `rows`, packed as `pack_slot` documents, and the wall faces'
    /// bytes `walls`, packed as `pack_walls` documents), and `failed` (per
    /// failure its kind, keys, `token`, `reason` and `retry`). The rest stays queued.
    #[func]
    pub fn poll_graded_tile_results(&mut self, max_tiles: i32) -> VarDictionary {
        self.spawn_graded_edit_selection();
        let mut jobs = self.graded_jobs.lock().unwrap();
        let mut edits = VarArray::new();
        for result in std::mem::take(&mut jobs.edits) {
            let edit = result.selection;
            let mut dict = VarDictionary::new();
            dict.set("serial", edit.serial as i64);
            dict.set("all", edit.all);
            let mut staged = VarArray::new();
            for (index, tile) in result.staged.into_iter().enumerate() {
                let mut packed = tile.to_dict();
                let adopted = result.adopted.get(index).copied().unwrap_or(false);
                packed.set("adopted", adopted);
                staged.push(&packed.to_variant());
                jobs.buffers.extend([tile.texels, tile.rows, tile.walls]);
            }
            dict.set("staged", staged);
            dict.set(
                "preview_epoch",
                result.epoch.map_or(-1, |epoch| epoch as i64),
            );
            let mut levels = VarArray::new();
            for (level, tiles) in &edit.levels {
                let (code, step) = level_code(*level);
                let mut entry = VarDictionary::new();
                entry.set("level", code);
                entry.set("step", step);
                let flat: Vec<i32> = tiles.iter().flat_map(|tile| [tile.x, tile.z]).collect();
                entry.set("tiles", PackedInt32Array::from(flat.as_slice()));
                levels.push(&entry.to_variant());
            }
            dict.set("levels", levels);
            edits.push(&dict.to_variant());
        }
        let mut patches = VarArray::new();
        for (request, tiles) in std::mem::take(&mut jobs.patches) {
            let mut dict = VarDictionary::new();
            dict.set("patch_x", request.patch.0 as i64);
            dict.set("patch_z", request.patch.1 as i64);
            dict.set("token", request.token);
            let flat: Vec<i32> = tiles.iter().flat_map(|tile| [tile.x, tile.z]).collect();
            dict.set("tiles", PackedInt32Array::from(flat.as_slice()));
            patches.push(&dict.to_variant());
        }
        let mut tiles = VarArray::new();
        for result in jobs.take_tiles(usize::try_from(max_tiles).unwrap_or(0)) {
            tiles.push(&result.to_dict().to_variant());
            jobs.buffers
                .extend([result.texels, result.rows, result.walls]);
        }
        let mut failed = VarArray::new();
        let failures = std::mem::take(&mut jobs.failed_tiles)
            .into_iter()
            .map(|(request, failure)| {
                let (code, step) = level_code(request.level);
                (
                    "tile",
                    [
                        i64::from(request.tile.x),
                        i64::from(request.tile.z),
                        code,
                        step,
                    ],
                    request.token,
                    failure,
                )
            })
            .chain(std::mem::take(&mut jobs.failed_patches).into_iter().map(
                |(request, failure)| {
                    (
                        "patch",
                        [request.patch.0 as i64, request.patch.1 as i64, 0, 0],
                        request.token,
                        failure,
                    )
                },
            ));
        for (kind, keys, token, failure) in failures {
            let mut dict = VarDictionary::new();
            dict.set("kind", kind);
            dict.set("keys", PackedInt64Array::from(keys.as_slice()));
            dict.set("token", token);
            dict.set("reason", failure_label(failure));
            dict.set(
                "retry",
                matches!(
                    failure,
                    Failure::Busy | Failure::Bake(BakeError::SitesUnavailable)
                ),
            );
            failed.push(&dict.to_variant());
        }
        let mut out = VarDictionary::new();
        out.set("edits", edits);
        out.set("patches", patches);
        out.set("tiles", tiles);
        out.set("failed", failed);
        out.set("queued_tiles", jobs.tiles.len() as i64);
        out.set("tiles_in_flight", jobs.tiles_in_flight.len() as i64);
        out.set("patches_in_flight", jobs.patches_in_flight.len() as i64);
        out
    }
}

impl SimulationNode {
    // Takes the core's graded edit journal (starting it on the first call) without waiting for
    // the core, and selects the edits' dirty tiles on a payload worker.
    fn spawn_graded_edit_selection(&self) {
        let Some(edits) =
            Self::try_lock_shared_core(&self.core).map(|mut core| core.take_graded_edits())
        else {
            return;
        };
        if edits.is_empty() {
            return;
        }
        let jobs = Arc::clone(&self.graded_jobs);
        self.async_jobs.spawn(move || {
            let selections: Vec<GradedEditSelection> = with_scratch(&SELECT, |scratch| {
                edits.iter().map(|edit| edit.select(scratch)).collect()
            });
            let results: Vec<EditResult> = selections
                .into_iter()
                .map(|mut selection| {
                    let (staged, adopted, epoch) = match selection.staged.take() {
                        Some(GradedInstallTiles {
                            stage,
                            epoch,
                            adopted,
                        }) => (
                            pack_install_tiles(&stage, selection.serial, &edits),
                            adopted,
                            epoch,
                        ),
                        None => (Vec::new(), Vec::new(), None),
                    };
                    EditResult {
                        selection,
                        staged,
                        adopted,
                        epoch,
                    }
                })
                .collect();
            jobs.lock().unwrap().edits.extend(results);
        });
    }
}

// Packs an install's staged tiles with the edit's serial as their token, at the base cell size
// of the edit's own heightmap.
fn pack_install_tiles(
    stage: &crate::simulation::terrain::graded::TileStage,
    serial: u64,
    edits: &[crate::nodes::sim::core::GradedEdit],
) -> Vec<TileResult> {
    let Some(edit) = edits.iter().find(|edit| edit.serial == serial) else {
        return Vec::new();
    };
    let cell_m = f64::from(edit.terrain.cell_size_m());
    let token = i64::try_from(serial).unwrap_or(i64::MAX);
    stage
        .tiles()
        .iter()
        .map(|staged| {
            let request = GradedTileRequest {
                tile: staged.tile,
                level: staged.level,
                token,
            };
            let side = pool_side(staged.level, cell_m);
            TileResult::packed(request, &staged.payload, side, Default::default())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(x: i32) -> GradedTileRequest {
        GradedTileRequest {
            tile: TileKey { x, z: 0 },
            level: Level::Gl0,
            token: 1,
        }
    }

    // `TERRAIN-07-G` round 6: a finished result counts in flight until it is polled, so the
    // results waiting for Godot's install budget (and the buffers the pool keeps for them) stay
    // within MAX_IN_FLIGHT_TILES however far the bakes run ahead of the installs.
    #[test]
    fn finished_tiles_count_in_flight_until_polled() {
        let mut jobs = GradedResidencyJobs::default();
        let payload = TilePayload::default();
        for x in 0..MAX_IN_FLIGHT_TILES as i32 {
            assert!(jobs.admit(TileKey { x, z: 0 }, Level::Gl0));
        }
        let next = MAX_IN_FLIGHT_TILES as i32;
        assert!(!jobs.admit(TileKey { x: next, z: 0 }, Level::Gl0));
        for x in 0..MAX_IN_FLIGHT_TILES as i32 {
            jobs.publish(&request(x), Ok(()), &payload, Default::default());
        }
        assert!(
            !jobs.admit(TileKey { x: next, z: 0 }, Level::Gl0),
            "a finished, unpolled result still holds its place"
        );
        assert_eq!(jobs.take_tiles(4).len(), 4);
        for x in next..next + 4 {
            assert!(jobs.admit(TileKey { x, z: 0 }, Level::Gl0));
        }
        assert!(!jobs.admit(TileKey { x: next + 4, z: 0 }, Level::Gl0));
        // A failure returns its place at once.
        jobs.publish(
            &request(next),
            Err(BakeError::SitesUnavailable),
            &payload,
            Default::default(),
        );
        assert!(jobs.admit(TileKey { x: next + 4, z: 0 }, Level::Gl0));
    }
}
