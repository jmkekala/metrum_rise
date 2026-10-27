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
//! the dirty tiles it holds at those levels and installs them together.

use super::*;
use crate::nodes::sim::core::GradedEditSelection;
use crate::simulation::buildings::allocator::SiteSnapshot;
use crate::simulation::terrain::graded::{
    BakeError, CAP_M, Level, PatchTilesScratch, SelectScratch, TILE_M, TileKey, TilePayload,
    WALL_FACE_BYTES, bake_tiles, graded_tiles_in_rect, pack_slot, pack_walls, slot_row_texels,
    slot_side,
};
use std::cell::RefCell;
use std::thread::LocalKey;

/// Most tile bakes in flight at once: the request budget that bounds the payloads Rust holds.
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
    seams: usize,
    edges: usize,
    texels: Vec<u8>,
    rows: Vec<u8>,
    walls: Vec<u8>,
}

/// The graded residency jobs of one node: requests in flight, finished results and failures
/// waiting for Godot's poll, and the byte buffers results are packed into, reused.
#[derive(Default)]
pub(crate) struct GradedResidencyJobs {
    tiles_in_flight: HashSet<(TileKey, Level)>,
    patches_in_flight: HashSet<(usize, usize)>,
    tiles: Vec<TileResult>,
    patches: Vec<(GradedPatchRequest, Vec<TileKey>)>,
    edits: Vec<GradedEditSelection>,
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
    // One pool per level code: `GL3` layers are sized for its finest step.
    let pool_level = match level {
        Level::Gl3 { .. } => Level::Gl3 { step: GL3_STEPS[0] },
        level => level,
    };
    let side = slot_side(pool_level, cell_m);
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
        for (((request, outcome), payload), [texels, rows, walls]) in
            requests.iter().zip(outcomes).zip(baked.iter()).zip(buffers)
        {
            state.tiles_in_flight.remove(&(request.tile, request.level));
            if let Err(error) = outcome {
                state.failed_tiles.push((*request, Failure::Bake(error)));
                state.buffers.extend([texels, rows, walls]);
                continue;
            }
            state.tiles.push(TileResult {
                request: *request,
                samples: payload.samples,
                graded: payload.graded,
                seams: payload.seams.len(),
                edges: payload.edges.len(),
                texels,
                rows,
                walls,
            });
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
                if jobs.tiles_in_flight.len() >= MAX_IN_FLIGHT_TILES
                    || !jobs.tiles_in_flight.insert((tile, level))
                {
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
    /// key order; Godot rebakes the tiles it holds at that level), `patches` (per list `patch_x`, `patch_z`,
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
        for edit in std::mem::take(&mut jobs.edits) {
            let mut dict = VarDictionary::new();
            dict.set("serial", edit.serial as i64);
            dict.set("all", edit.all);
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
        let take = usize::try_from(max_tiles)
            .unwrap_or(0)
            .min(jobs.tiles.len());
        let mut tiles = VarArray::new();
        let ready: Vec<TileResult> = jobs.tiles.drain(..take).collect();
        for result in ready {
            let (code, step) = level_code(result.request.level);
            let mut dict = VarDictionary::new();
            dict.set("tile_x", i64::from(result.request.tile.x));
            dict.set("tile_z", i64::from(result.request.tile.z));
            dict.set("level", code);
            dict.set("step", step);
            dict.set("token", result.request.token);
            dict.set("samples_x", result.samples[0] as i64);
            dict.set("samples_z", result.samples[1] as i64);
            dict.set("graded", result.graded);
            dict.set("seam_count", result.seams as i64);
            dict.set("edge_count", result.edges as i64);
            dict.set("wall_count", (result.walls.len() / WALL_FACE_BYTES) as i64);
            dict.set("texels", PackedByteArray::from(result.texels.as_slice()));
            dict.set("rows", PackedByteArray::from(result.rows.as_slice()));
            dict.set("walls", PackedByteArray::from(result.walls.as_slice()));
            jobs.buffers.extend([result.texels, result.rows, result.walls]);
            tiles.push(&dict.to_variant());
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
            jobs.lock().unwrap().edits.extend(selections);
        });
    }
}
