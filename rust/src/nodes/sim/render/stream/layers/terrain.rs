// SPDX-License-Identifier: GPL-3.0-or-later

//! Terrain and water patches on the streaming core (`RENDER-32-M`, `docs/terrain.md` "Streaming
//! core" 5.6): residency, mesh level of detail and prewarm of the 512 m render patches, decided
//! here and executed by `terrain.gd` and `water.gd` on their pooled nodes, one unit at a time
//! under the stream node's frame budget ([`FrameBudget`]).
//!
//! The rules are the scripts' (`dev` `daec355a`), so what is drawn once settled does not change:
//!
//! - **Residency.** The wanted terrain patches are the square about the camera of half-side
//!   `min(far, max(8 km, 4 x height))`, padded by one patch and widened by two of hysteresis
//!   ([`PatchGrid::target`]); yaw never enters. The water's wanted patches are the terrain's
//!   resident ones. A change of the square is applied as the two strips between the old and the
//!   new square, `O(boundary patches)`; nothing is walked while the square holds.
//! - **Level of detail.** A patch's step is 1, 2, 4 or 8 by the 3D distance from the eye to its
//!   centre on the ground (2, 5 and 12 km). An activation takes the step of the eye at hand. The
//!   resident steps are re-checked when the view has moved 96 m since the last check and 0.2 s
//!   have passed (the scripts' gate): the steps at the check's eye are kept by a [`RingSet`]
//!   over the patch lattice, so a check visits the cells near the three thresholds, and a patch
//!   whose installed step differs from its ring step is queued; a queued patch takes the step of
//!   the eye when its unit runs.
//! - **Prewarm.** Once nothing is missing, queued or leaving, the water's patches one beyond
//!   the terrain's square are built hidden, nearest first ([`Limits::halo`]); the terrain's
//!   prewarm square is its wanted square, as its scripts' was in effect.
//! - **Budget** (`TERRAIN-41` B1, [`budget`]). Given places ([`PatchStream::set_places`]), the
//!   prewarm square is the keep rectangle (the fill: kept patches built hidden, nearest first,
//!   once nothing else waits, releases included) and built patches beyond the release rectangle
//!   are released, farthest first: in a frame of holes only as a hole build's donor (released
//!   at once inside the build's price, so a burst releases as fast as it builds), otherwise as a
//!   unit of their own after the hides at a fixed price. The water keeps and releases with the
//!   terrain (`StreamNode`). Without places the layer keeps its own halo and releases nothing.
//!
//! The units run in the scheduler's order (`schedule::Class`): holes in view, holes within the
//! near ring (the 2 km of `TERRAIN-15`'s hand-over), other holes, level changes in view, other
//! level changes; then patches leaving, farthest first; then prewarm. Inside a class nearest
//! first by exact squared distance, then by key. A hole whose payload has not arrived is skipped
//! (its request is out); payload requests are bounded per frame ([`Limits::requests`]), as jobs
//! in flight, not in milliseconds.
//!
//! Storage is dense per patch, sized once per world; a frame allocates nothing.

use super::super::rings::{Cell, Lattice, RingBand, RingSet};
use super::super::schedule::{Class, Clock, FrameBudget};

mod budget;
mod units;

pub(crate) use budget::{Keep, RELEASE_RING, keep, place_bytes, places};
pub(crate) use units::{UNIT_KIND_NAMES, UNIT_KINDS, UnitKind, UnitTimes};

/// The scripts' patch grid: `terrain.gd`'s layout (`get_terrain_patch_layout`) and world size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PatchGrid {
    /// Patches along x.
    pub(crate) cols: i32,
    /// Patches along z.
    pub(crate) rows: i32,
    /// Terrain cells a patch spans.
    pub(crate) interval: i32,
    /// The terrain cell, metres.
    pub(crate) cell: f32,
    /// The terrain's world size, metres (the heightmap's, `get_terrain_world_size`).
    pub(crate) world: [f32; 2],
}

/// A patch: column and row.
pub(crate) type PatchKey = [i32; 2];

impl PatchGrid {
    /// The patch side, metres, as `terrain.gd` computes it.
    pub(crate) fn span(&self) -> f64 {
        f64::from(self.cell) * f64::from(self.interval)
    }

    /// `rect` widened by `by` patches on every side, clamped to the grid.
    pub(crate) fn widen(&self, rect: [i32; 4], by: i32) -> [i32; 4] {
        [
            (rect[0] - by).max(0),
            (rect[1] - by).max(0),
            (rect[2] + by).min(self.cols - 1),
            (rect[3] + by).min(self.rows - 1),
        ]
    }

    fn len(&self) -> usize {
        (self.cols.max(0) * self.rows.max(0)) as usize
    }

    fn contains(&self, key: PatchKey) -> bool {
        key[0] >= 0 && key[1] >= 0 && key[0] < self.cols && key[1] < self.rows
    }

    fn index(&self, key: PatchKey) -> usize {
        (key[1] * self.cols + key[0]) as usize
    }

    fn key(&self, index: usize) -> PatchKey {
        let index = index as i32;
        [index % self.cols, index / self.cols]
    }

    /// The centre of `key`'s node: its payload's origin plus half its size, in the payload's
    /// single precision widened as GDScript adds them (`_create_patch`), then stored as the
    /// node's single precision position. The last patch of a row is shorter.
    pub(crate) fn centre(&self, key: PatchKey) -> [f32; 2] {
        let [x, z, width, depth] = self.rect(key);
        [(f64::from(x) + f64::from(width) * 0.5) as f32, (f64::from(z) + f64::from(depth) * 0.5) as f32]
    }

    /// `key`'s payload rectangle `[origin x, origin z, size x, size z]` in the payload's single
    /// precision (`world_origin_*`, `world_size_*`: the terrain's grid origin of the patch's first
    /// cell, and its cells' extent). The last patch of a row is shorter.
    pub(crate) fn rect(&self, key: PatchKey) -> [f32; 4] {
        let mut rect = [0.0; 4];
        for axis in 0..2 {
            let intervals = (self.world[axis] / self.cell).round() as i32;
            let start = key[axis] * self.interval;
            let end = (start + self.interval).min(intervals);
            rect[axis] = start as f32 * self.cell - self.world[axis] * 0.5;
            rect[axis + 2] = (end - start) as f32 * self.cell;
        }
        rect
    }

    /// The wanted square for an eye (`terrain.gd`'s `_camera_patch_bounds` widened by the
    /// residency hysteresis), as inclusive `[min_x, min_z, max_x, max_z]`; the whole grid for
    /// `full_world`. The arithmetic is the script's: the eye's corners in single precision
    /// (`Vector2 - Vector2.ONE * far`), the rest in double.
    pub(crate) fn target(&self, eye: [f32; 3], camera_far: f32, full_world: bool) -> [i32; 4] {
        if full_world {
            return [0, 0, self.cols - 1, self.rows - 1];
        }
        let far = f64::from(camera_far).min((f64::from(eye[1]).abs() * 4.0).max(CULL_FAR_M));
        let far32 = far as f32;
        let low = [eye[0] - far32, eye[2] - far32];
        let high = [eye[0] + far32, eye[2] + far32];
        let span = self.span();
        let limit = [self.cols - 1, self.rows - 1];
        let patch = |axis: usize, at: f32, pad: f64| {
            let half = f64::from(self.world[axis]) * 0.5;
            let index = ((f64::from(at) + pad + half) / span).floor();
            (index.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32).clamp(0, limit[axis])
        };
        let desired = [
            patch(0, low[0], -span),
            patch(1, low[1], -span),
            patch(0, high[0], span),
            patch(1, high[1], span),
        ];
        [
            (desired[0] - HYSTERESIS_PATCHES).max(0),
            (desired[1] - HYSTERESIS_PATCHES).max(0),
            (desired[2] + HYSTERESIS_PATCHES).min(limit[0]),
            (desired[3] + HYSTERESIS_PATCHES).min(limit[1]),
        ]
    }
}

/// The residency's least half-side, metres (`PATCH_RESIDENCY_CULL_FAR_M`).
pub(crate) const CULL_FAR_M: f64 = 8000.0;
/// Patches of hysteresis about the wanted square (`PATCH_RESIDENCY_HYSTERESIS_PATCHES`).
pub(crate) const HYSTERESIS_PATCHES: i32 = 2;
/// The level of detail thresholds, metres of 3D distance: steps 1, 2 and 4 within them, 8 beyond.
pub(crate) const LOD_DISTANCES_M: [f64; 3] = [2000.0, 5000.0, 12000.0];
/// The level of detail check's gate: seconds since the last check (`..._REFRESH_INTERVAL_S`).
pub(crate) const LOD_GATE_S: f64 = 0.20;
/// The level of detail check's gate: metres of eye travel since the last check.
pub(crate) const LOD_GATE_M: f32 = 96.0;
/// The near ring of `TERRAIN-15`'s hand-over (`input_manager.gd`'s `WORLD_READY_RADIUS_M`).
pub(crate) const NEAR_RING_M: f64 = 2000.0;

/// The step for a 3D distance (`_mesh_lod_step_for_distance`).
pub(crate) fn lod_step(distance: f64) -> u8 {
    match LOD_DISTANCES_M.iter().position(|&limit| distance <= limit) {
        Some(at) => 1 << at,
        None => 8,
    }
}

/// The eye's distance to a patch centre on the ground, as Godot's single precision
/// `Vector3.distance_to` (`(x² + y²) + z²`, then the root).
pub(crate) fn lod_distance(eye: [f32; 3], centre: [f32; 2]) -> f64 {
    let x = centre[0] - eye[0];
    let y = 0.0 - eye[1];
    let z = centre[1] - eye[2];
    f64::from((x * x + y * y + z * z).sqrt())
}

/// The scripts' level of detail gate: with residents, every frame adds its time; it opens when
/// 0.2 s have passed and the eye moved 96 m since it last opened (or never did), and then starts
/// over. Without residents the time starts over.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct LodGate {
    elapsed: f64,
    eye: Option<[f32; 3]>,
}

impl LodGate {
    /// Due at the next frame with residents (a debug mode changed).
    pub(crate) fn force(&mut self) {
        self.elapsed = LOD_GATE_S;
        self.eye = None;
    }

    /// A frame the scripts deferred their check in (no headroom left): its time counts, the
    /// gate stays shut (the control's recordings).
    #[cfg(test)]
    pub(crate) fn defer(&mut self, delta: f64) {
        self.elapsed += delta;
    }

    /// One frame of `delta` seconds at `eye`. Returns whether the gate opened. `O(1)`.
    pub(crate) fn frame(&mut self, residents: bool, eye: Option<[f32; 3]>, delta: f64) -> bool {
        if !residents {
            self.elapsed = 0.0;
            return false;
        }
        self.elapsed += delta;
        if self.elapsed < LOD_GATE_S {
            return false;
        }
        let moved = match (eye, self.eye) {
            (_, None) | (None, _) => true,
            (Some(eye), Some(last)) => {
                let d = [eye[0] - last[0], eye[1] - last[1], eye[2] - last[2]];
                d[0] * d[0] + d[1] * d[1] + d[2] * d[2] >= LOD_GATE_M * LOD_GATE_M
            }
        };
        if !moved {
            return false;
        }
        if eye.is_some() {
            self.eye = eye;
        }
        self.elapsed = 0.0;
        true
    }
}

/// What one layer bounds per frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Limits {
    /// Payload requests handed out per frame (the scripts' request budgets: jobs, not time).
    pub(crate) requests: usize,
    /// Patches of prewarm halo about the wanted square.
    pub(crate) halo: i32,
}

/// Terrain's limits (`PATCH_PAYLOAD_REQUEST_BUDGET_PER_FRAME`). No halo: the scripts' terrain
/// prewarm built nothing beyond the square, because their payload prune dropped every payload
/// outside it each frame; the core keeps that.
pub(crate) const TERRAIN_LIMITS: Limits = Limits {
    requests: 32,
    halo: 0,
};
/// Water's limits (`WATER_PATCH_PAYLOAD_REQUEST_BUDGET_PER_FRAME`): its halo is one patch about
/// the terrain's square (`WATER_PATCH_PREWARM_HALO_PATCHES`), which its scripts did build.
pub(crate) const WATER_LIMITS: Limits = Limits {
    requests: 64,
    halo: 1,
};

/// Predicted costs of the units, microseconds: fixed, from the scripts' measured unit times
/// with a margin (`RENDER-32-M` record), never adapted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct UnitCosts {
    /// Activating a patch whose node is built (prewarmed or kept).
    pub(crate) activate_us: f64,
    /// Building a patch's node from its payload (an activation of an unbuilt patch, a prewarm).
    pub(crate) build_us: f64,
    /// A level of detail change.
    pub(crate) lod_us: f64,
    /// Hiding a patch that left.
    pub(crate) free_us: f64,
    /// Releasing a kept patch outside the budget's release rectangle (`TERRAIN-41` B1): its
    /// node, material and textures back to the pool. Priced at its 95th percentile on the 60 km
    /// flight at a 256 MB budget: terrain 155-170 us (mean 114), water 137-149 us (mean 68, its
    /// mesh cache's entries going with it). A release that runs as a build's donor is charged
    /// inside the build's price.
    pub(crate) release_us: f64,
}

/// Terrain's unit costs, measured on Kuopio's load (`RENDER-32-M` record): a build (a height
/// image and texture, a mask layer, the land cover from its payload and the patch's own material
/// parameters in `terrain.gd`) takes 0.16 ms a build (0.41 before the land cover left the main
/// thread and the constant parameters moved to the pooled material); an activation of a built
/// patch is a node shown, a level change and the graded hand-over's bookkeeping. A level change
/// (terrain or water) measured 11 us at the median, 25-36 at the 95th percentile (frames of
/// level changes only, 30k, 100k and Kuopio turns, `RENDER-32-Q`): 20 us, against 60 and 40
/// before, which spent Kuopio's turn frames' budget while measured 0.2-0.4 ms.
pub(crate) const TERRAIN_COSTS: UnitCosts = UnitCosts {
    activate_us: 100.0,
    build_us: 200.0,
    lod_us: 20.0,
    free_us: 40.0,
    release_us: 160.0,
};
/// Water's unit costs, measured as terrain's: an activation that builds the node (a depth image
/// and texture, a material) takes 0.064 ms at the median, 0.21 ms at most.
pub(crate) const WATER_COSTS: UnitCosts = UnitCosts {
    activate_us: 60.0,
    build_us: 120.0,
    lod_us: 20.0,
    free_us: 30.0,
    release_us: 140.0,
};

/// The post-load window's budget for the patch units, microseconds (`RENDER-32-M`): from a world
/// load (after the load's hold, `set_load_settling`) until the terrain and water first settle,
/// their units may spend what the scripts spent a frame on Kuopio's load (`terrain.gd` without
/// its graded renderer 3.95 ms, `water.gd` 1.27 ms, `dev` `daec355a`, instrumented); afterwards
/// the stream node's 2.0 ms frame budget applies.
pub(crate) const POST_LOAD_PATCH_BUDGET_US: f64 = 5200.0;

/// What an activation did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Activation {
    /// The patch is resident and drawn.
    Done,
    /// It has no payload any more: it waits for its request.
    NotReady,
    /// A network publication is pending: no activation this frame.
    Blocked,
}

/// What a release did (`TERRAIN-41` B1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Release {
    /// The patch is not built any more (released, or it had no node).
    Done,
    /// Not this patch now (a commit of its own is staged or its own request is in flight): the
    /// next candidate may go.
    Busy,
    /// A network publication is pending: no release this frame.
    Blocked,
}

/// What executes a layer's units: the scripts' pooled nodes (`terrain.gd`, `water.gd`) in the
/// game, a recorder in tests.
pub(crate) trait PatchExecutor {
    /// Requests the payloads of `keys` (flat x, z pairs), for prewarm builds when `prewarm`. A
    /// key the scripts already hold a payload or node for, or refused, is answered at once:
    /// appends `x, z, news` (as [`Payload::from_code`]) to `news`.
    fn request(&mut self, keys: &[i32], prewarm: bool, news: &mut Vec<i32>);
    /// Makes `key` resident at `step`, building its node from its payload when needed; with the
    /// surface generation the node now carries (-1: unknown).
    fn activate(&mut self, key: PatchKey, step: u8) -> (Activation, i64);
    /// Hides `key`, which left.
    fn deactivate(&mut self, key: PatchKey);
    /// Builds `key`'s node hidden at `step` from its payload. Returns its surface generation
    /// when it was built.
    fn build(&mut self, key: PatchKey, step: u8) -> Option<i64>;
    /// Gives resident `key` the mesh of `step`.
    fn set_lod(&mut self, key: PatchKey, step: u8);
    /// Releases built, hidden `key`: its node, material and textures go back to the pool. Calls
    /// nothing back; what the core must know is the answer.
    fn release(&mut self, key: PatchKey) -> Release;
}

// Per-patch flags.
const WANTED: u8 = 1;
const RESIDENT: u8 = 2;
const READY: u8 = 4;
const BUILT: u8 = 8;
const REQUESTED: u8 = 16;
const REFUSED: u8 = 32;
const MISMATCH: u8 = 64;
const QUEUED: u8 = 128;

/// A payload's news from the scripts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Payload {
    /// The request was dropped (a retry, a prune) or a ready payload was dropped: ask again.
    Dropped,
    /// The request's payload arrived.
    Ready,
    /// The request was refused: not asked again.
    Refused,
}

impl Payload {
    /// The scripts' code: 0 dropped, 1 ready, 2 refused (any other: dropped).
    pub(crate) fn from_code(code: i64) -> Self {
        match code {
            1 => Self::Ready,
            2 => Self::Refused,
            _ => Self::Dropped,
        }
    }
}

/// A resident patch's change, for a layer that follows the entries themselves (the graded book,
/// `RENDER-32-K` part 6), in the order the units ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Follow {
    /// Became resident at `step`, its node carrying surface `generation`.
    Resident {
        /// The mesh step it was activated at.
        step: u8,
        /// The surface generation its node carries.
        generation: i64,
    },
    /// A resident patch took the mesh of this step.
    Step(u8),
    /// Left (hidden, or its node freed).
    Left,
}

/// The counts of one frame's work and the layer's state, for the perf channel and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct PatchWork {
    /// Patches whose wanted state changed since the last frame (this frame's square and the
    /// followed layer's news since then; not a running total).
    pub(crate) wanted_changes: usize,
    /// Ring cells the level of detail checks visited.
    pub(crate) ring_cells: usize,
    /// Level of detail checks (the gate opened).
    pub(crate) lod_checks: usize,
    /// Units run: activations, level changes, hides, builds.
    pub(crate) activations: usize,
    /// Level changes run.
    pub(crate) lod_changes: usize,
    /// Hides run.
    pub(crate) frees: usize,
    /// Prewarm builds run.
    pub(crate) builds: usize,
    /// Payloads requested.
    pub(crate) requests: usize,
    /// Units the budget held back.
    pub(crate) deferred: usize,
    /// Of them holes (any class; a hole skipped for its payload or behind a blocked activation
    /// is not counted): what the graded book's builds wait behind (`TERRAIN-41` L2).
    pub(crate) holes_deferred: usize,
    /// Kept patches released (`TERRAIN-41` B1), of them `donors` as a hole build's donor.
    pub(crate) releases: usize,
    /// Releases that ran as a build's donor in a frame of holes.
    pub(crate) donors: usize,
    /// Units run by [`UnitKind`] (`TERRAIN-41` L1).
    pub(crate) unit_count: [usize; UNIT_KINDS],
    /// Their measured microseconds by kind.
    pub(crate) unit_us: [f64; UNIT_KINDS],
    /// The longest unit by kind, microseconds.
    pub(crate) unit_max_us: [f64; UNIT_KINDS],
}

/// One layer's patches: terrain's (wanted: the square) or water's (wanted: terrain's resident).
pub(crate) struct PatchStream {
    grid: Option<PatchGrid>,
    limits: Limits,
    costs: UnitCosts,
    // What the level changes (upgrades) leave for the frame's holes (`RENDER-32-Q`).
    upgrade_reserve_us: f64,
    flags: Vec<u8>,
    // The installed step of a resident patch; the ring's step at the last check.
    step: Vec<u8>,
    ring_step: Vec<u8>,
    ring: RingSet<u8>,
    // Wanted and not resident; resident and not wanted; resident and queued for a check. Lists
    // may hold stale entries (filtered by the flags when used); they are rebuilt, never grown
    // past the grid.
    holes: Vec<u32>,
    leaving: Vec<u32>,
    mismatched: Vec<u32>,
    queue: Vec<u32>,
    prewarm: Vec<u32>,
    missing: usize,
    // Of the missing, those whose payload Rust refused: never asked again, so no settled view
    // waits for them (AUDIT-stream-2 finding 5).
    refused: usize,
    extra: usize,
    resident: usize,
    // Patches marked requested: asked for, no news yet (`TERRAIN-35`'s guard).
    requested: usize,
    // Built patches (`BUILT`), and the budget's state (`TERRAIN-41` B1): the places it holds
    // (none: the layer's own prewarm halo, no release), the keep it gives about the square, the
    // release rectangle (none: nothing is released), the built patches that may lie outside it
    // (stale entries filtered when used; nearest first after `sort_releasable`, so the farthest
    // pops off the end) and whether that list needs sorting.
    built: usize,
    places: Option<u64>,
    keep: Option<Keep>,
    release: Option<[i32; 4]>,
    releasable: Vec<u32>,
    releasable_dirty: bool,
    // Built patches released since the last take (their surface generation goes, -1).
    released: Vec<PatchKey>,
    // Candidates a release found busy this frame (scratch, returned to the list at its end).
    release_busy: Vec<u32>,
    // The other layer's holes wait: releases run only as donors; unless the counted bytes are
    // over the budget (its extras grew), when releases run in the room the holes left.
    releases_held: bool,
    over_budget: bool,
    // The wanted square (terrain) and the prewarm square.
    square: Option<[i32; 4]>,
    halo: Option<[i32; 4]>,
    prewarm_dirty: bool,
    gate: LodGate,
    ring_height: Option<f32>,
    // Residency changes and built nodes' surface generations of this frame, for the layers
    // that follow (water, vegetation).
    changes: Vec<(PatchKey, bool)>,
    generations: Vec<(PatchKey, i64)>,
    // The resident patches' changes of this frame, while a follower takes them (`follow`).
    follow: Option<Vec<(PatchKey, Follow)>>,
    news: Vec<i32>,
    // Scratch.
    order: Vec<(Class, f64, u32)>,
    requests: Vec<i32>,
    // Wanted changes since the last frame; the frame's work; units' times since the last take.
    wanted_changes: usize,
    work: PatchWork,
    times: UnitTimes,
    force_lod1: bool,
    prewarm_held: bool,
}

impl PatchStream {
    /// An empty layer with `limits` and unit `costs`.
    pub(crate) fn new(limits: Limits, costs: UnitCosts) -> Self {
        Self {
            grid: None,
            limits,
            costs,
            upgrade_reserve_us: 0.0,
            flags: Vec::new(),
            step: Vec::new(),
            ring_step: Vec::new(),
            ring: RingSet::new(Lattice {
                origin: [0.0; 2],
                cell: 1.0,
            }),
            holes: Vec::new(),
            leaving: Vec::new(),
            mismatched: Vec::new(),
            queue: Vec::new(),
            prewarm: Vec::new(),
            missing: 0,
            refused: 0,
            extra: 0,
            resident: 0,
            requested: 0,
            built: 0,
            places: None,
            keep: None,
            release: None,
            releasable: Vec::new(),
            releasable_dirty: false,
            released: Vec::new(),
            release_busy: Vec::new(),
            releases_held: false,
            over_budget: false,
            square: None,
            halo: None,
            prewarm_dirty: false,
            gate: LodGate::default(),
            ring_height: None,
            changes: Vec::new(),
            generations: Vec::new(),
            follow: None,
            news: Vec::new(),
            order: Vec::new(),
            requests: Vec::new(),
            wanted_changes: 0,
            work: PatchWork::default(),
            times: UnitTimes::default(),
            force_lod1: false,
            prewarm_held: false,
        }
    }

    /// Starts over on `grid` (a world load, a renderer reset): nothing wanted, resident or built.
    /// Sizes the storage once for the grid.
    pub(crate) fn reset(&mut self, grid: Option<PatchGrid>) {
        let len = grid.map_or(0, |grid| grid.len());
        self.grid = grid;
        for list in [&mut self.flags, &mut self.ring_step] {
            list.clear();
            list.resize(len, 0);
        }
        self.ring_step.fill(8);
        self.step.clear();
        self.step.resize(len, 1);
        for list in [
            &mut self.holes,
            &mut self.leaving,
            &mut self.mismatched,
            &mut self.queue,
            &mut self.prewarm,
            &mut self.releasable,
        ] {
            list.clear();
            list.reserve(len);
        }
        self.order.clear();
        self.order.reserve(len);
        self.changes.clear();
        self.changes.reserve(len);
        self.generations.clear();
        self.generations.reserve(len);
        if let Some(follow) = self.follow.as_mut() {
            follow.clear();
            follow.reserve(len);
        }
        self.news.clear();
        self.requests.clear();
        let lattice = grid.map_or(
            Lattice {
                origin: [0.0; 2],
                cell: 1.0,
            },
            |grid| Lattice {
                origin: [
                    -f64::from(grid.world[0]) * 0.5,
                    -f64::from(grid.world[1]) * 0.5,
                ],
                cell: grid.span(),
            },
        );
        self.ring = RingSet::new(lattice);
        self.missing = 0;
        self.refused = 0;
        self.extra = 0;
        self.resident = 0;
        self.requested = 0;
        self.built = 0;
        self.keep = None;
        self.release = None;
        self.releasable_dirty = false;
        self.released.clear();
        self.released.reserve(len);
        self.square = None;
        self.halo = None;
        self.prewarm_dirty = false;
        self.gate = LodGate::default();
        self.ring_height = None;
    }

    /// The `terrain_force_lod1` debug mode: every step 1; the next frame checks every patch.
    pub(crate) fn set_force_lod1(&mut self, force: bool) {
        if self.force_lod1 != force {
            self.force_lod1 = force;
            self.ring_height = None;
            self.gate.force();
        }
    }

    /// Holds the prewarm back (water: while the terrain still streams, so the water's halo
    /// builds do not take the frames the terrain's holes need).
    pub(crate) fn hold_prewarm(&mut self, held: bool) {
        self.prewarm_held = held;
    }

    /// Holds the releases that are not a build's donor back while the other patch layer has work
    /// a settled view waits for (`TERRAIN-41` B1: no hole, hide or level change of either layer
    /// waits behind a release).
    pub(crate) fn hold_releases(&mut self, held: bool) {
        self.releases_held = held;
    }

    /// The counted bytes are over the budget (`TERRAIN-41` B1: the extras grew, so the places
    /// shrank faster than the donors give back): releases run in the room the frame's holes left,
    /// holes waiting or not. A release comes after every hole of the frame, so it takes no room a
    /// hole could have had.
    pub(crate) fn set_over_budget(&mut self, over: bool) {
        self.over_budget = over;
    }

    /// The grid, once a world is set.
    pub(crate) fn grid(&self) -> Option<PatchGrid> {
        self.grid
    }

    /// Wanted patches not resident (refused payloads excluded), resident patches not wanted,
    /// and patches queued for a check.
    pub(crate) fn pending(&self) -> [usize; 3] {
        [self.missing - self.refused, self.extra, self.queue.len()]
    }

    /// Wanted patches whose payload was refused (holes for good, a contract fault the scripts
    /// report).
    pub(crate) fn refused_count(&self) -> usize {
        self.refused
    }

    /// Resident patches.
    pub(crate) fn resident_count(&self) -> usize {
        self.resident
    }

    /// Patches marked requested: asked for through [`PatchExecutor::request`] and not yet
    /// answered by [`Self::payload`] news. Each is a request its executor still tracks; one it
    /// dropped without news is never asked for again (`TERRAIN-35`), so the scripts check this
    /// count against their own. `O(1)`.
    pub(crate) fn requested_count(&self) -> usize {
        self.requested
    }

    /// Built patches (resident or kept). `O(1)`.
    pub(crate) fn built_count(&self) -> usize {
        self.built
    }

    /// The budget's keep about the wanted square (`TERRAIN-41` B1; none without a budget).
    pub(crate) fn keep(&self) -> Option<Keep> {
        self.keep
    }

    /// The release rectangle: built patches outside it are released (none: nothing is).
    pub(crate) fn release_rect(&self) -> Option<[i32; 4]> {
        self.release
    }

    /// Patches whose fill (prewarm) build is still to come: unbuilt, not refused, in the
    /// prewarm square. `O(prewarm square)`: a status read, not a frame's.
    pub(crate) fn fill_left(&self) -> usize {
        let (Some(grid), Some(halo)) = (self.grid, self.halo) else {
            return 0;
        };
        (halo[1]..=halo[3])
            .flat_map(|z| (halo[0]..=halo[2]).map(move |x| [x, z]))
            .filter(|&key| self.flags[grid.index(key)] & (BUILT | REFUSED) == 0)
            .count()
    }

    /// The built patches released since the last call; taken by the caller (their surface
    /// generation goes, as when a script frees a node itself).
    pub(crate) fn take_released(&mut self) -> std::vec::Drain<'_, PatchKey> {
        self.released.drain(..)
    }

    /// Whether the layer has work left that a settled view waits for (prewarm excluded, as the
    /// scripts' `has_pending_render_work(false)`).
    pub(crate) fn has_pending_work(&self) -> bool {
        self.missing > self.refused || self.extra > 0 || !self.queue.is_empty()
    }

    /// `key`'s installed step (resident patches).
    #[cfg(test)]
    pub(crate) fn installed_step(&self, key: PatchKey) -> Option<u8> {
        let grid = self.grid?;
        (grid.contains(key) && self.flags[grid.index(key)] & RESIDENT != 0)
            .then(|| self.step[grid.index(key)])
    }

    /// The residency changes of the last frame (and of the calls since), for the layers that
    /// follow; taken by the caller.
    pub(crate) fn take_changes(&mut self) -> std::vec::Drain<'_, (PatchKey, bool)> {
        self.changes.drain(..)
    }

    /// The surface generations of the nodes built since the last call, in build order; taken by
    /// the caller (they precede the residency changes of the same frame).
    pub(crate) fn take_generations(&mut self) -> std::vec::Drain<'_, (PatchKey, i64)> {
        self.generations.drain(..)
    }

    /// Records the resident patches' changes ([`Follow`]) from now on for a follower that takes
    /// them each frame (`take_follow`), or stops.
    pub(crate) fn set_followed(&mut self, followed: bool) {
        self.follow = followed.then(Vec::new);
    }

    /// The resident patches' changes since the last call, in the order the units ran; taken by
    /// the follower.
    pub(crate) fn take_follow(&mut self) -> std::vec::Drain<'_, (PatchKey, Follow)> {
        self.follow.get_or_insert_with(Vec::new).drain(..)
    }

    /// The resident patches. `O(grid)`.
    pub(crate) fn resident_keys(&self) -> impl Iterator<Item = PatchKey> + '_ {
        let grid = self.grid;
        self.flags
            .iter()
            .enumerate()
            .filter(|(_, flags)| **flags & RESIDENT != 0)
            .filter_map(move |(index, _)| grid.map(|grid| grid.key(index)))
    }

    /// The wanted patches with whether each is resident (drawn) and its payload ready (it
    /// waits for its unit only), for the stream benchmark's drawn probe (`RENDER-32-N`).
    /// `O(grid)`.
    pub(crate) fn probe_wanted(&self) -> impl Iterator<Item = (PatchKey, bool, bool)> + '_ {
        let grid = self.grid;
        self.flags
            .iter()
            .enumerate()
            .filter(|(_, flags)| **flags & WANTED != 0)
            .filter_map(move |(index, &flags)| {
                grid.map(|grid| (grid.key(index), flags & RESIDENT != 0, flags & READY != 0))
            })
    }

    /// A payload's news (`terrain.gd` / `water.gd` poll). `O(1)`.
    pub(crate) fn payload(&mut self, key: PatchKey, news: Payload) {
        let Some(grid) = self.grid.filter(|grid| grid.contains(key)) else {
            return;
        };
        let index = grid.index(key);
        self.requested -= usize::from(self.flags[index] & REQUESTED != 0);
        let flags = self.flags[index] & !REQUESTED;
        if news == Payload::Refused && flags & (WANTED | RESIDENT | REFUSED) == WANTED {
            self.refused += 1;
        }
        // The prewarm list keeps its order: news changes no patch's membership (a refused
        // one is skipped when reached), and a list as wide as the keep rectangle is not sorted
        // again for every payload of a fill (`TERRAIN-41` B1).
        self.flags[index] = match news {
            Payload::Ready => flags | READY,
            Payload::Dropped => flags & !READY,
            Payload::Refused => (flags & !READY) | REFUSED,
        };
    }

    /// The script freed `key`'s node itself (a failed upload): it is neither built nor
    /// resident; a wanted patch becomes a hole again. `O(1)`.
    pub(crate) fn patch_removed(&mut self, key: PatchKey) {
        let Some(grid) = self.grid.filter(|grid| grid.contains(key)) else {
            return;
        };
        let index = grid.index(key);
        if self.flags[index] & RESIDENT != 0 {
            self.set_resident(index, false);
        }
        self.set_built(index, false);
        self.flags[index] &= !READY;
        self.prewarm_dirty = true;
    }

    // Marks `index` built or not, keeping the count. `O(1)`.
    fn set_built(&mut self, index: usize, built: bool) {
        let was = self.flags[index] & BUILT != 0;
        if built && !was {
            self.flags[index] |= BUILT;
            self.built += 1;
        } else if !built && was {
            self.flags[index] &= !BUILT;
            self.built -= 1;
        }
    }

    /// Sets whether `key` is wanted (water: terrain's residency). `O(1)`.
    pub(crate) fn set_wanted(&mut self, key: PatchKey, wanted: bool) {
        let Some(grid) = self.grid.filter(|grid| grid.contains(key)) else {
            return;
        };
        self.mark_wanted(grid.index(key), wanted);
        self.prewarm_dirty = true;
    }

    /// Sets the prewarm square directly (water: terrain's halo).
    pub(crate) fn set_halo(&mut self, halo: Option<[i32; 4]>) {
        if self.halo != halo {
            self.halo = halo;
            self.prewarm_dirty = true;
        }
    }

    /// The prewarm square: the wanted square widened by the halo.
    pub(crate) fn halo(&self) -> Option<[i32; 4]> {
        self.halo
    }

    /// The wanted square (terrain).
    pub(crate) fn square(&self) -> Option<[i32; 4]> {
        self.square
    }

    fn mark_wanted(&mut self, index: usize, wanted: bool) {
        let flags = self.flags[index];
        if (flags & WANTED != 0) == wanted {
            return;
        }
        self.wanted_changes += 1;
        let resident = flags & RESIDENT != 0;
        if wanted {
            self.flags[index] |= WANTED;
            if resident {
                self.extra -= 1;
            } else {
                self.missing += 1;
                self.refused += usize::from(flags & REFUSED != 0);
                self.holes.push(index as u32);
            }
        } else {
            self.flags[index] &= !WANTED;
            if resident {
                self.extra += 1;
                self.leaving.push(index as u32);
            } else {
                self.missing -= 1;
                self.refused -= usize::from(flags & REFUSED != 0);
            }
        }
    }

    fn set_resident(&mut self, index: usize, resident: bool) {
        let grid = self.grid.expect("a resident patch has a grid");
        let flags = self.flags[index];
        let wanted = flags & WANTED != 0;
        if resident {
            self.set_built(index, true);
            self.flags[index] |= RESIDENT;
            self.flags[index] &= !READY;
            self.resident += 1;
            if wanted {
                self.missing -= 1;
            } else {
                self.extra += 1;
                self.leaving.push(index as u32);
            }
            self.update_mismatch(index);
        } else {
            self.flags[index] &= !(RESIDENT | MISMATCH | QUEUED);
            self.resident -= 1;
            if let Some(follow) = self.follow.as_mut() {
                follow.push((grid.key(index), Follow::Left));
            }
            if wanted {
                self.missing += 1;
                self.holes.push(index as u32);
            } else {
                self.extra -= 1;
            }
        }
        self.changes.push((grid.key(index), resident));
    }

    // A resident patch's installed step against its ring step: a mismatch is queued at the next
    // check, as the scripts' sweep would have found it.
    fn update_mismatch(&mut self, index: usize) {
        let mismatch =
            self.flags[index] & RESIDENT != 0 && self.step[index] != self.ring_step[index];
        let was = self.flags[index] & MISMATCH != 0;
        if mismatch && !was {
            self.flags[index] |= MISMATCH;
            self.mismatched.push(index as u32);
        } else if !mismatch && was {
            self.flags[index] &= !MISMATCH;
        }
    }

    // The step at `eye` of the patch at `index`.
    fn rule(&self, grid: PatchGrid, eye: Option<[f32; 3]>, index: usize) -> u8 {
        match eye {
            Some(eye) if !self.force_lod1 => lod_step(lod_distance(eye, grid.centre(grid.key(index)))),
            _ => 1,
        }
    }

    /// Moves the wanted square to `square` (terrain): the patches of the two strips between the
    /// old and the new square change. `O(boundary patches)`; nothing when it holds.
    pub(crate) fn set_square(&mut self, square: [i32; 4]) {
        let Some(grid) = self.grid else {
            return;
        };
        let old = self.square;
        if old == Some(square) {
            return;
        }
        self.square = Some(square);
        // The fill's order is nearest the eye: sorted again when the fill next runs.
        self.prewarm_dirty = true;
        self.apply_keep(grid, square);
        if let Some(old) = old {
            for_each_in_difference(old, square, |x, z| {
                self.mark_wanted(grid.index([x, z]), false);
            });
            for_each_in_difference(square, old, |x, z| {
                self.mark_wanted(grid.index([x, z]), true);
            });
        } else {
            for z in square[1]..=square[3] {
                for x in square[0]..=square[2] {
                    self.mark_wanted(grid.index([x, z]), true);
                }
            }
        }
    }

    /// The budget's places (`TERRAIN-41` B1; none: no budget, the layer's own prewarm halo and no
    /// release; `u64::MAX`: every patch fits). The keep and release rectangles follow at once.
    /// `O(K)`, plus the release strips (`set_release`).
    pub(crate) fn set_places(&mut self, places: Option<u64>) {
        if self.places == places {
            return;
        }
        self.places = places;
        if let (Some(grid), Some(square)) = (self.grid, self.square) {
            self.apply_keep(grid, square);
        }
    }

    // The prewarm square and the release rectangle for `square`: the keep rectangle and its
    // ring with a budget, the layer's own halo without. `O(K)` and the release strips.
    fn apply_keep(&mut self, grid: PatchGrid, square: [i32; 4]) {
        let Some(places) = self.places else {
            self.keep = None;
            self.set_halo(Some(grid.widen(square, self.limits.halo)));
            self.set_release(None);
            return;
        };
        let kept = keep(&grid, square, places);
        self.keep = Some(kept);
        self.set_halo(Some(grid.widen(square, kept.ring.max(self.limits.halo))));
        // Every patch fits: nothing is released.
        let release = (places != u64::MAX).then(|| grid.widen(square, kept.ring + RELEASE_RING));
        self.set_release(release);
    }

    /// Sets the release rectangle (none: nothing is released). The built patches of the strips
    /// that leave it become candidates. `O(boundary patches)` for a move, `O(old rectangle)` for a
    /// jump; nothing when it holds.
    pub(crate) fn set_release(&mut self, release: Option<[i32; 4]>) {
        let Some(grid) = self.grid else {
            return;
        };
        if self.release == release {
            return;
        }
        let whole = [0, 0, grid.cols - 1, grid.rows - 1];
        let old = self.release.unwrap_or(whole);
        let new = release.unwrap_or(whole);
        self.release = release;
        let mut found = false;
        for_each_in_difference(old, new, |x, z| {
            let index = grid.index([x, z]);
            if self.flags[index] & BUILT != 0 {
                self.releasable.push(index as u32);
                found = true;
            }
        });
        self.releasable_dirty |= found;
    }

    // Whether `index` lies outside the release rectangle.
    fn outside_release(&self, grid: PatchGrid, index: usize) -> bool {
        self.release.is_some_and(|rect| {
            let [x, z] = grid.key(index);
            x < rect[0] || z < rect[1] || x > rect[2] || z > rect[3]
        })
    }

    // Whether `index` may be released now: built and outside the release rectangle (so not
    // wanted; a resident one is still leaving and is hidden first).
    fn releasable(&self, grid: PatchGrid, index: usize) -> bool {
        self.flags[index] & BUILT != 0 && self.outside_release(grid, index)
    }

    // The candidates without stale entries, nearest first (so the farthest pops off the end),
    // then by index. `O(R log R)` for R candidates, only when the list changed.
    fn sort_releasable(&mut self, grid: PatchGrid, distance_sq: &impl Fn(usize) -> f64) {
        if !self.releasable_dirty {
            return;
        }
        self.releasable_dirty = false;
        let mut list = std::mem::take(&mut self.releasable);
        list.retain(|&index| self.releasable(grid, index as usize));
        list.sort_unstable();
        list.dedup();
        list.sort_by(|&a, &b| {
            distance_sq(a as usize)
                .total_cmp(&distance_sq(b as usize))
                .then(b.cmp(&a))
        });
        self.releasable = list;
    }

    // Releases the farthest candidate that goes, charged at `predicted_us` (0 inside a build's
    // price). Returns whether one went, or `None` when a publication blocks every release this
    // frame. `O(1)` amortised; a busy candidate is kept for a later frame.
    fn release_one(
        &mut self,
        grid: PatchGrid,
        budget: &mut FrameBudget,
        clock: &impl Clock,
        predicted_us: f64,
        exec: &mut impl PatchExecutor,
        busy: &mut Vec<u32>,
    ) -> Option<bool> {
        while let Some(index) = self.releasable.pop() {
            let index = index as usize;
            if !self.releasable(grid, index) {
                continue;
            }
            let key = grid.key(index);
            let start = clock.now_us();
            if self.flags[index] & RESIDENT != 0 {
                // Still drawn, beyond the release rectangle (a jump whose hides wait behind its
                // holes): hidden first, inside the release's price, so a burst releases as fast
                // as it builds.
                exec.deactivate(key);
                self.set_resident(index, false);
            }
            let done = exec.release(key);
            self.account(budget, UnitKind::Release, predicted_us, clock.now_us() - start);
            match done {
                Release::Done => {
                    self.set_built(index, false);
                    self.flags[index] &= !READY;
                    self.released.push(key);
                    self.work.releases += 1;
                    return Some(true);
                }
                Release::Busy => busy.push(index as u32),
                Release::Blocked => {
                    self.releasable.push(index as u32);
                    return None;
                }
            }
        }
        Some(false)
    }

    /// Keeps `reserve_us` of the frame's budget from the level changes (the holes' reserve of
    /// the install ledger, `RENDER-32-Q`); the first unit of a frame still runs.
    pub(crate) fn set_upgrade_reserve(&mut self, reserve_us: f64) {
        self.upgrade_reserve_us = reserve_us;
    }

    /// One frame: the level of detail gate, then units in order while the budget admits them,
    /// then payload requests. `eye` is the view's (none before a camera); `in_view` orders work
    /// only. Returns the frame's counts.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn frame(
        &mut self,
        eye: Option<[f32; 3]>,
        delta: f64,
        budget: &mut FrameBudget,
        clock: &impl Clock,
        unbudgeted: bool,
        in_view: &impl Fn(PatchKey) -> bool,
        exec: &mut impl PatchExecutor,
    ) -> PatchWork {
        self.work = PatchWork {
            wanted_changes: std::mem::take(&mut self.wanted_changes),
            ..PatchWork::default()
        };
        let Some(grid) = self.grid else {
            return self.work;
        };
        self.ring.reset_visited();
        self.lod_gate(grid, eye, delta);
        self.work.ring_cells = self.ring.visited();
        let admit = |budget: &FrameBudget, cost: f64| unbudgeted || budget.admits(cost);
        let upgrade_reserve_us = self.upgrade_reserve_us;
        let mut blocked = false;
        let mut held = false;
        let mut release_blocked = false;
        let mut busy = std::mem::take(&mut self.release_busy);
        busy.clear();

        // Holes, then level changes, nearest first inside their classes.
        self.order.clear();
        let point = eye.map(|eye| [eye[0], eye[2]]);
        let distance_sq = |index: usize| match point {
            Some(point) => {
                let centre = grid.centre(grid.key(index));
                let dx = f64::from(centre[0]) - f64::from(point[0]);
                let dz = f64::from(centre[1]) - f64::from(point[1]);
                dx * dx + dz * dz
            }
            None => 0.0,
        };
        let mut holes = std::mem::take(&mut self.holes);
        holes.retain(|&index| self.flags[index as usize] & (WANTED | RESIDENT) == WANTED);
        holes.sort_unstable();
        holes.dedup();
        for &index in &holes {
            let flags = self.flags[index as usize];
            if flags & (READY | BUILT) == 0 {
                continue;
            }
            let key = grid.key(index as usize);
            let distance = distance_sq(index as usize);
            let class = if in_view(key) {
                Class::HoleInView
            } else if distance <= NEAR_RING_M * NEAR_RING_M {
                Class::HoleNear
            } else {
                Class::Hole
            };
            self.order.push((class, distance, index));
        }
        self.holes = holes;
        let mut queue = std::mem::take(&mut self.queue);
        queue.retain(|&index| self.flags[index as usize] & (RESIDENT | QUEUED) == RESIDENT | QUEUED);
        for &index in &queue {
            let key = grid.key(index as usize);
            let class = if in_view(key) {
                Class::UpgradeInView
            } else {
                Class::Upgrade
            };
            self.order.push((class, distance_sq(index as usize), index));
        }
        self.queue = queue;
        self.order.sort_unstable_by(|a, b| {
            a.0.cmp(&b.0)
                .then(a.1.total_cmp(&b.1))
                .then(grid.key(a.2 as usize)[1].cmp(&grid.key(b.2 as usize)[1]))
                .then(grid.key(a.2 as usize)[0].cmp(&grid.key(b.2 as usize)[0]))
        });
        let order = std::mem::take(&mut self.order);
        for &(class, _, index) in &order {
            let index = index as usize;
            let key = grid.key(index);
            if class >= Class::UpgradeInView {
                // A queued check: the eye's step now; no unit when it holds.
                self.flags[index] &= !QUEUED;
                let target = self.rule(grid, eye, index);
                if target == self.step[index] {
                    continue;
                }
                if !(unbudgeted || budget.admits_upgrade(self.costs.lod_us, upgrade_reserve_us)) {
                    self.flags[index] |= QUEUED;
                    self.work.deferred += 1;
                    held = true;
                    continue;
                }
                let start = clock.now_us();
                exec.set_lod(key, target);
                self.account(budget, UnitKind::Step, self.costs.lod_us, clock.now_us() - start);
                self.step[index] = target;
                if let Some(follow) = self.follow.as_mut() {
                    follow.push((key, Follow::Step(target)));
                }
                self.update_mismatch(index);
                self.work.lod_changes += 1;
                continue;
            }
            if blocked {
                continue;
            }
            let (kind, cost) = if self.flags[index] & BUILT != 0 {
                (UnitKind::Show, self.costs.activate_us)
            } else {
                (UnitKind::Build, self.costs.build_us)
            };
            if !admit(budget, cost) {
                self.work.deferred += 1;
                // A hole the budget refused: counted only, the order and admission are as
                // before (`TERRAIN-41` L2). O(1).
                self.work.holes_deferred += 1;
                held = true;
                continue;
            }
            if kind == UnitKind::Build && !release_blocked && !self.releasable.is_empty() {
                // The donor rule (`TERRAIN-41` B1): a build first releases the farthest kept
                // patch outside the release rectangle, inside the build's price, so its
                // resources are built into at once and a burst releases as fast as it builds.
                self.sort_releasable(grid, &distance_sq);
                match self.release_one(grid, budget, clock, 0.0, exec, &mut busy) {
                    Some(true) => self.work.donors += 1,
                    Some(false) => {}
                    None => release_blocked = true,
                }
            }
            let step = self.rule(grid, eye, index);
            let start = clock.now_us();
            let (done, generation) = exec.activate(key, step);
            self.account(budget, kind, cost, clock.now_us() - start);
            if done == Activation::Done && self.flags[index] & BUILT == 0 {
                self.generations.push((key, generation));
            }
            match done {
                Activation::Done => {
                    self.step[index] = step;
                    if let Some(follow) = self.follow.as_mut() {
                        follow.push((key, Follow::Resident { step, generation }));
                    }
                    self.set_resident(index, true);
                    self.work.activations += 1;
                }
                Activation::NotReady => {
                    self.set_built(index, false);
                    self.flags[index] &= !READY;
                }
                Activation::Blocked => blocked = true,
            }
        }
        self.order = order;
        self.queue.retain(|&index| self.flags[index as usize] & QUEUED != 0);

        // Patches that left, farthest first.
        let mut leaving = std::mem::take(&mut self.leaving);
        leaving.retain(|&index| self.flags[index as usize] & (WANTED | RESIDENT) == RESIDENT);
        leaving.sort_unstable();
        leaving.dedup();
        leaving.sort_by(|&a, &b| {
            distance_sq(b as usize)
                .total_cmp(&distance_sq(a as usize))
                .then(a.cmp(&b))
        });
        let mut freed = 0;
        for &index in &leaving {
            if !admit(budget, self.costs.free_us) {
                self.work.deferred += 1;
                held = true;
                break;
            }
            let start = clock.now_us();
            exec.deactivate(grid.key(index as usize));
            self.account(budget, UnitKind::Hide, self.costs.free_us, clock.now_us() - start);
            self.set_resident(index as usize, false);
            if self.outside_release(grid, index as usize) {
                // Kept, hidden, beyond the release rectangle (a jump): released later.
                self.releasable.push(index);
                self.releasable_dirty = true;
            }
            freed += 1;
        }
        // The hides' own pushes were onto the emptied list; the rest wait, farthest first.
        let pushed = std::mem::take(&mut self.leaving);
        leaving.drain(..freed);
        leaving.extend(pushed);
        self.leaving = leaving;
        self.work.frees = freed;

        // Releases on their own (`TERRAIN-41` B1): after holes, level changes and hides, before
        // the fill, at a fixed price, never as the frame's exempt first unit, and only while no
        // hole of this layer and no pending work of the other waits (in a frame of holes a
        // release is a build's donor) or while the counted bytes are over the budget. Farthest first. `O(released + R log R)` while
        // candidates exist, nothing otherwise.
        if !blocked
            && !release_blocked
            && (self.over_budget || !self.releases_held && self.missing == self.refused)
            && !self.releasable.is_empty()
        {
            self.sort_releasable(grid, &distance_sq);
            while !self.releasable.is_empty()
                && (unbudgeted || budget.fits(self.costs.release_us))
            {
                match self.release_one(grid, budget, clock, self.costs.release_us, exec, &mut busy) {
                    Some(true) => {}
                    Some(false) => break,
                    None => {
                        release_blocked = true;
                        break;
                    }
                }
            }
            if !self.releasable.is_empty() && !release_blocked {
                self.work.deferred += 1;
            }
        }
        // Busy candidates wait for a later frame.
        if !busy.is_empty() {
            self.releasable.extend_from_slice(&busy);
            self.releasable_dirty = true;
        }
        self.release_busy = busy;

        // Prewarm (the budget's fill) once nothing else waits, releases included: a fill never
        // builds past the places the releases still have to give back.
        if !blocked
            && !held
            && !self.prewarm_held
            && self.releasable.is_empty()
            && self.missing == 0
            && self.extra == 0
            && self.queue.is_empty()
        {
            self.prewarm_units(grid, eye, budget, clock, unbudgeted, &distance_sq, exec);
        }
        self.request_payloads(grid, &distance_sq, exec);
        self.work
    }

    // Charges a unit that ran to the frame's budget and counts it by kind.
    fn account(&mut self, budget: &mut FrameBudget, kind: UnitKind, predicted_us: f64, us: f64) {
        budget.record(predicted_us, us);
        let at = kind as usize;
        self.work.unit_count[at] += 1;
        self.work.unit_us[at] += us;
        self.work.unit_max_us[at] = self.work.unit_max_us[at].max(us);
        self.times.add(kind, us);
    }

    /// The units' measured times by kind since the last call, then starts over (the stream
    /// benchmark's samples, `TERRAIN-41` L1).
    pub(crate) fn take_unit_times(&mut self, out: &mut impl FnMut(&UnitTimes)) {
        out(&self.times);
        self.times.clear();
    }

    #[allow(clippy::too_many_arguments)]
    fn prewarm_units(
        &mut self,
        grid: PatchGrid,
        eye: Option<[f32; 3]>,
        budget: &mut FrameBudget,
        clock: &impl Clock,
        unbudgeted: bool,
        distance_sq: &impl Fn(usize) -> f64,
        exec: &mut impl PatchExecutor,
    ) {
        if self.prewarm_dirty {
            self.prewarm_dirty = false;
            self.prewarm.clear();
            if let Some(halo) = self.halo {
                for z in halo[1]..=halo[3] {
                    for x in halo[0]..=halo[2] {
                        let index = grid.index([x, z]);
                        if self.flags[index] & (BUILT | REFUSED) == 0 {
                            self.prewarm.push(index as u32);
                        }
                    }
                }
            }
            // Farthest first, so the nearest pops off the end.
            self.prewarm.sort_by(|&a, &b| {
                distance_sq(b as usize)
                    .total_cmp(&distance_sq(a as usize))
                    .then(b.cmp(&a))
            });
        }
        while let Some(&index) = self.prewarm.last() {
            let index = index as usize;
            if self.flags[index] & (BUILT | REFUSED) != 0 {
                self.prewarm.pop();
                continue;
            }
            if self.flags[index] & READY == 0 {
                // Its payload is requested below; the nearer ones first.
                break;
            }
            if !(unbudgeted || budget.admits(self.costs.build_us)) {
                self.work.deferred += 1;
                break;
            }
            self.prewarm.pop();
            let key = grid.key(index);
            let step = self.rule(grid, eye, index);
            let start = clock.now_us();
            let built = exec.build(key, step);
            self.account(budget, UnitKind::Prewarm, self.costs.build_us, clock.now_us() - start);
            self.flags[index] &= !READY;
            if let Some(generation) = built {
                self.set_built(index, true);
                self.step[index] = step;
                self.generations.push((key, generation));
                self.work.builds += 1;
            }
        }
    }

    // Payloads for the holes, nearest first, then for the prewarm square once nothing is
    // missing, up to the frame's request limit.
    fn request_payloads(
        &mut self,
        grid: PatchGrid,
        distance_sq: &impl Fn(usize) -> f64,
        exec: &mut impl PatchExecutor,
    ) {
        const NEEDS: u8 = READY | BUILT | REQUESTED | REFUSED | RESIDENT;
        self.requests.clear();
        let mut wanting = std::mem::take(&mut self.order);
        wanting.clear();
        let prewarm = self.missing == 0;
        if prewarm && self.prewarm_held {
            self.order = wanting;
            return;
        }
        if !prewarm {
            for &index in &self.holes {
                if self.flags[index as usize] & NEEDS == 0 {
                    wanting.push((Class::Hole, distance_sq(index as usize), index));
                }
            }
        } else {
            // The prewarm list is nearest last: walked from the end, it stops once the frame's
            // request limit is reached counting the prewarm payloads already asked for or held
            // and not yet built, so a fill as wide as the keep rectangle holds at most that many
            // payloads and a frame visits `O(requests)` entries, not the whole list (`TERRAIN-41`
            // B1); it takes the nearest by the list's last sort.
            let mut pending = 0;
            for &index in self.prewarm.iter().rev() {
                if pending + wanting.len() >= self.limits.requests {
                    break;
                }
                let flags = self.flags[index as usize];
                if flags & (BUILT | REFUSED) != 0 {
                    continue;
                }
                if flags & (READY | REQUESTED) != 0 {
                    pending += 1;
                } else {
                    wanting.push((Class::Hole, distance_sq(index as usize), index));
                }
            }
        }
        wanting.sort_unstable_by(|a, b| a.1.total_cmp(&b.1).then(a.2.cmp(&b.2)));
        for &(_, _, index) in wanting.iter().take(self.limits.requests) {
            let key = grid.key(index as usize);
            self.flags[index as usize] |= REQUESTED;
            self.requested += 1;
            self.requests.extend_from_slice(&key);
        }
        self.order = wanting;
        if !self.requests.is_empty() {
            self.work.requests = self.requests.len() / 2;
            let mut news = std::mem::take(&mut self.news);
            news.clear();
            exec.request(&self.requests, prewarm, &mut news);
            for &[x, z, code] in news.as_chunks::<3>().0 {
                self.payload([x, z], Payload::from_code(i64::from(code)));
            }
            self.news = news;
        }
    }

    // The scripts' level of detail gate: with residents, every frame adds its time; when 0.2 s
    // have passed and the eye moved 96 m since the last check (or there was none), the ring
    // moves to the eye and the patches whose installed step differs from their ring step are
    // queued, replacing the queue.
    fn lod_gate(&mut self, grid: PatchGrid, eye: Option<[f32; 3]>, delta: f64) {
        if self.resident == 0 {
            for &index in &self.queue {
                self.flags[index as usize] &= !QUEUED;
            }
            self.queue.clear();
        }
        if !self.gate.frame(self.resident > 0, eye, delta) {
            return;
        }
        self.work.lod_checks += 1;
        self.move_ring(grid, eye);
        for &index in &self.queue {
            self.flags[index as usize] &= !QUEUED;
        }
        self.queue.clear();
        let mut mismatched = std::mem::take(&mut self.mismatched);
        mismatched.retain(|&index| self.flags[index as usize] & MISMATCH != 0);
        mismatched.sort_unstable();
        mismatched.dedup();
        for &index in &mismatched {
            self.flags[index as usize] |= QUEUED;
            self.queue.push(index);
        }
        self.mismatched = mismatched;
    }

    // Moves the ring's evaluation point to `eye`: the cells whose step can have changed are
    // re-evaluated by the exact rule; a height change re-evaluates the whole disc.
    fn move_ring(&mut self, grid: PatchGrid, eye: Option<[f32; 3]>) {
        let Some(eye) = eye.filter(|_| !self.force_lod1) else {
            // No camera or every step 1: the rule is step 1 for every patch.
            for index in 0..self.ring_step.len() {
                self.ring_step[index] = 1;
                self.update_mismatch(index);
            }
            self.ring.clear();
            self.ring_height = None;
            return;
        };
        if self.ring.point().is_none() {
            // A fresh ring holds no cell: every ring step is the default's until evaluated.
            for index in 0..self.ring_step.len() {
                self.ring_step[index] = 8;
                self.update_mismatch(index);
            }
        }
        let full = self.ring_height != Some(eye[1]);
        self.ring_height = Some(eye[1]);
        let height = f64::from(eye[1]);
        let reach = grid.span();
        let mut bands = [RingBand {
            radius: 0.0,
            reach,
        }; 3];
        for (band, limit) in bands.iter_mut().zip(LOD_DISTANCES_M) {
            band.radius = (limit * limit - height * height).max(0.0).sqrt();
        }
        let mut ring = std::mem::replace(
            &mut self.ring,
            RingSet::new(Lattice {
                origin: [0.0; 2],
                cell: 1.0,
            }),
        );
        // A cell's state is its tier: 0 beyond the last threshold (step 8, the default), 3
        // within the first. Cells off the grid stay at the default.
        let tier = |step: u8| match step {
            1 => 3,
            2 => 2,
            4 => 1,
            _ => 0,
        };
        let step_of = |tier: u8| match tier {
            3 => 1,
            2 => 2,
            1 => 4,
            _ => 8,
        };
        ring.update(
            [f64::from(eye[0]), f64::from(eye[2])],
            &bands,
            full,
            |cell: Cell| {
                if grid.contains(cell) {
                    tier(lod_step(lod_distance(eye, grid.centre(cell))))
                } else {
                    0
                }
            },
            |cell, _, new| {
                let index = grid.index(cell);
                self.ring_step[index] = step_of(new);
                self.update_mismatch(index);
            },
        );
        self.ring = ring;
    }
}

// Visits the cells of `a` that are not in `b` (inclusive rectangles), row by row.
fn for_each_in_difference(a: [i32; 4], b: [i32; 4], mut visit: impl FnMut(i32, i32)) {
    for z in a[1]..=a[3] {
        let row_in_b = z >= b[1] && z <= b[3];
        if !row_in_b {
            for x in a[0]..=a[2] {
                visit(x, z);
            }
            continue;
        }
        for x in a[0]..=a[2].min(b[0] - 1) {
            visit(x, z);
        }
        for x in a[0].max(b[2] + 1)..=a[2] {
            visit(x, z);
        }
    }
}

#[cfg(test)]
mod tests;
