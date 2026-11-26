// SPDX-License-Identifier: GPL-3.0-or-later

//! Kept water meshes (`TERRAIN-63`, `docs/terrain.md` "Kept water meshes"): which built, hidden
//! water patches are given a mesh of the coarsest detail step before they are shown, and which
//! such meshes go again when the base terrain's memory target is passed.
//!
//! A water patch the budget keeps beyond the view (built hidden by the fill) has no mesh until it
//! is first shown, so a zoom-out over it waits for its mesh (`TERRAIN-58` holds its ground as a
//! backdrop cell meanwhile). The book asks `water.gd` for a mesh of [`KEPT_WATER_STEP`] for every
//! such patch, nearest the eye first, in frames in which nothing the view waits for is pending; a
//! patch shown with it draws its water at once, and its own finer step arrives as an upgrade.
//!
//! **Pricing** (decision D5 (a)): the places, the ring and the keep and release rectangles are the
//! budget's as before, counted without the kept meshes' bytes `H`; a kept mesh only takes the room
//! the places leave. A request is handed out only while `counted + H + W x (asked + 1) <= B`,
//! with `W` the largest kept mesh the grid can give ([`kept_mesh_bytes_max`]); while
//! `counted + H > B` kept meshes are dropped, farthest first. Between the two lies a band of `W`,
//! so a still camera neither asks nor drops in a loop, and the end state does not depend on the
//! order or the pace in which the meshes arrive. Without a budget every wanting patch is asked for.
//!
//! **Kept** means the patch's mesh came by this cause and the patch has not been shown since; a
//! show makes it an ordinary mesh (counted with the extras, kept after a later hide, as before).
//!
//! The book holds no Godot type: it is fed the water's answers and reports (the same events the
//! backdrop's holds read) and acts through a [`KeptExecutor`] (`water.gd`'s `stream_request_meshes`
//! and `stream_drop_mesh` in the game, a recorder in tests). Storage is one byte a patch and two
//! lists of patch indices, sized once a world; a frame allocates nothing once they have grown.

use super::{PatchGrid, PatchKey, ground_distance_sq};
use crate::nodes::sim::render::stream::layers::backdrop::WaterState;
use crate::nodes::sim::render::stream::schedule::{Clock, FrameBudget};

/// The detail step of a kept mesh (decision D6 (a)): the coarsest. A patch enters the wanted
/// square at least 9.5 km from the eye, where its own step is 4 or 8, so it draws this at once and
/// a finer step follows as an upgrade, never as a hole. `water.gd`'s `KEPT_WATER_STEP`.
pub(crate) const KEPT_WATER_STEP: usize = 8;
/// Requests handed out a frame, and asked at once, at most: a camera move finds little in flight.
pub(crate) const ASK_PER_FRAME: usize = 8;
/// Kept requests in flight at once, at most.
pub(crate) const ASKED_MAX: usize = 16;
/// Behind the loading screen (the load's preload, `TERRAIN-63` round W2) the counts a frame give
/// way to the scripts' time slice: requests handed out a frame and out at once, at most (the water
/// mesh pipeline's in-flight bound in its preload caps). What is asked and in which order is the
/// same; only how much a frame takes changes.
pub(crate) const PRELOAD_ASKED_MAX: usize = 256;
/// The predicted price of one kept mesh's apply in `water.gd`, microseconds: the book asks only
/// in a frame whose install ledger has this much room (the apply itself is admitted by the
/// ledger's room in `water.gd`, charged as measured). Measured (`TERRAIN-63`, release library, the
/// 60 km map's background fill after a player's load): 84 us at the mean, 98 at the 95th
/// percentile, a step-8 mesh (an `ArrayMesh` of at most 384 vertices, or the shared full grid).
pub(crate) const APPLY_PRICE_US: f64 = 85.0;
/// The predicted price of a drop (its node's mesh emptied), as a water hide's.
pub(crate) const DROP_PRICE_US: f64 = 40.0;

// Bytes of one mesh vertex as `water.gd` counts them: position and normal (3 x f32 each), UV
// (2 x f32).
const VERTEX_BYTES: u64 = 12 + 12 + 8;

/// `W`: the largest kept mesh on `grid`, bytes as `water.gd` counts them
/// (`mesh_estimated_bytes`). A mesh cell is drawn as two triangles of their own vertices (6) unless
/// the whole patch is wet, which draws an indexed grid of fewer bytes; so the largest is every
/// cell of the patch's mesh at [`KEPT_WATER_STEP`] drawn on its own. `O(1)`.
pub(crate) fn kept_mesh_bytes_max(grid: &PatchGrid) -> u64 {
    let samples = grid.interval.max(1) as u64 + 1;
    // The mesh builder's `mesh_interval_count`.
    let intervals = (samples - 1).div_ceil(KEPT_WATER_STEP as u64).max(1);
    intervals * intervals * 6 * VERTEX_BYTES
}

// Per-patch flags. BUILT: the water patch's node exists; SHOWN: it is resident; WET: it has water
// to draw; MESH: its node holds a mesh of its cover and depth (any step); ASKED: a kept request is
// out; KEPT: its mesh came by a kept request and it has not been shown since; BARRED: the script
// refused a request the book's bits wanted (they drifted), not asked again until its next news.
const BUILT: u8 = 1;
const SHOWN: u8 = 2;
const WET: u8 = 4;
const MESH: u8 = 8;
const ASKED: u8 = 16;
const KEPT: u8 = 32;
const BARRED: u8 = 64;

fn wanting(flags: u8) -> bool {
    flags & (BUILT | SHOWN | WET | MESH | ASKED | BARRED) == BUILT | WET
}

fn kept(flags: u8) -> bool {
    flags & KEPT != 0
}

/// What executes the book's units: `water.gd` in the game, a recorder in tests. Neither calls the
/// stream node back.
pub(crate) trait KeptExecutor {
    /// Asks for a kept mesh of each of `keys` (flat x, z pairs). Appends `x, z, state` for each key
    /// it did not take to `refused` (`state` as `water.gd`'s `_water_state`, -1: not built or
    /// shown).
    fn request(&mut self, keys: &[i32], refused: &mut Vec<i32>);
    /// Drops `key`'s kept mesh (its node draws none, hidden). Returns the bytes freed, or `None`
    /// when it held no kept mesh.
    fn drop_mesh(&mut self, key: PatchKey) -> Option<u64>;
}

/// One frame's inputs.
#[derive(Clone, Copy, Debug)]
pub(crate) struct KeptFrame {
    /// The base terrain's memory target, bytes (none: every patch fits).
    pub(crate) budget: Option<u64>,
    /// The budget's counted bytes without the kept meshes: built places and extras.
    pub(crate) counted: u64,
    /// The kept meshes' bytes `H`, as `water.gd` last reported them.
    pub(crate) kept_bytes: u64,
    /// Nothing the view waits for is pending and the ledger has room for an apply: requests may
    /// be handed out this frame.
    pub(crate) may_ask: bool,
    /// The load's preload runs (behind the loading screen): up to [`PRELOAD_ASKED_MAX`] requests
    /// a frame and out at once instead of [`ASK_PER_FRAME`] and [`ASKED_MAX`].
    pub(crate) preload: bool,
}

/// One frame's work.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct KeptWork {
    /// Requests handed out.
    pub(crate) asked: usize,
    /// Kept meshes dropped.
    pub(crate) dropped: usize,
    /// Their bytes.
    pub(crate) dropped_bytes: u64,
}

/// The kept water meshes' book.
#[derive(Default)]
pub(crate) struct KeptMeshes {
    grid: Option<PatchGrid>,
    flags: Vec<u8>,
    // Patches that turned wanting (stale entries filtered when used); nearest last after a sort.
    wanting: Vec<u32>,
    wanting_dirty: bool,
    // Patches holding a kept mesh (stale entries filtered when used); farthest last after a sort.
    kept: Vec<u32>,
    kept_dirty: bool,
    // The eye's patch at the lists' last sort.
    sorted_at: Option<PatchKey>,
    wanting_count: usize,
    asked_count: usize,
    kept_count: usize,
    asked_total: u64,
    dropped_total: u64,
    refused_total: u64,
    // Scratch: the frame's request keys and the script's refusals.
    requests: Vec<i32>,
    refused: Vec<i32>,
}

impl KeptMeshes {
    /// Starts over on a world's patch grid (none: no world): nothing is built. `O(patches)`, once
    /// a world or a water reset.
    pub(crate) fn reset(&mut self, grid: Option<PatchGrid>) {
        let requests = std::mem::take(&mut self.requests);
        let refused = std::mem::take(&mut self.refused);
        let mut wanting = std::mem::take(&mut self.wanting);
        let mut kept = std::mem::take(&mut self.kept);
        wanting.clear();
        kept.clear();
        let [asked_total, dropped_total, refused_total] =
            [self.asked_total, self.dropped_total, self.refused_total];
        *self = Self {
            grid,
            flags: vec![0; grid.map_or(0, |grid| grid.len())],
            wanting,
            kept,
            requests,
            refused,
            asked_total,
            dropped_total,
            refused_total,
            ..Self::default()
        };
    }

    /// The water answered a build or an activation of `key` with `state` (its node exists).
    /// `O(1)`.
    pub(crate) fn answer(&mut self, key: PatchKey, state: WaterState) {
        self.update(key, |flags| with_state(flags | BUILT, state) & !BARRED);
    }

    /// The water layer showed (`shown`) or hid `key`. A show ends a kept request (the script turns
    /// it into the show's own) and makes a kept mesh an ordinary one. `O(1)`.
    pub(crate) fn set_shown(&mut self, key: PatchKey, shown: bool) {
        self.update(key, |flags| {
            if shown {
                (flags | SHOWN) & !(ASKED | KEPT | BARRED)
            } else {
                flags & !(SHOWN | BARRED)
            }
        });
    }

    /// `water.gd`'s report of built `key`'s water (`water_patch_state`). A report of a hidden
    /// patch ends its kept request: the script reports a hidden patch only when its request was
    /// answered (a mesh) or dropped with a stale mesh (its cover or depth changed). A mesh that
    /// answers a request is kept. `O(1)`.
    pub(crate) fn report(&mut self, key: PatchKey, state: WaterState) {
        self.update(key, |flags| {
            let mut next = with_state(flags | BUILT, state) & !BARRED;
            if flags & SHOWN == 0 {
                next &= !(ASKED | KEPT);
                if flags & ASKED != 0 && state == WaterState::Mesh {
                    next |= KEPT;
                }
            }
            next
        });
    }

    /// The water forgot `key`'s node (released, or freed by its script). `O(1)`.
    pub(crate) fn forget(&mut self, key: PatchKey) {
        self.update(key, |_| 0);
    }

    /// Patches built, hidden and wet without a mesh, not yet asked for.
    pub(crate) fn wanting_count(&self) -> usize {
        self.wanting_count
    }

    /// Kept requests out.
    pub(crate) fn asked_count(&self) -> usize {
        self.asked_count
    }

    /// Patches holding a kept mesh.
    pub(crate) fn kept_count(&self) -> usize {
        self.kept_count
    }

    /// Whether one more request fits the target now: `counted + H + W x (asked + 1) <= B` with
    /// the book's requests out (always without a target): the rule [`Self::frame`] asks by.
    /// `O(1)`.
    pub(crate) fn admits_one(&self, budget: Option<u64>, counted: u64, kept_bytes: u64) -> bool {
        self.grid.is_some_and(|grid| {
            let more = kept_mesh_bytes_max(&grid) * (self.asked_count as u64 + 1);
            budget.is_none_or(|budget| counted + kept_bytes + more <= budget)
        })
    }

    /// What the load's preload waits for (`TERRAIN-63` round W2, decision D7 (a)): requests out,
    /// or wanting patches while the target `admits` one more ([`Self::admits_one`]). Patches the
    /// admission rule leaves out for lack of room are not waited for: the target is full. `O(1)`.
    pub(crate) fn waits(&self, admits: bool) -> bool {
        self.asked_count > 0 || self.wanting_count > 0 && admits
    }

    /// Whether built `key`'s water waits for the book ([`Self::waits`] for one patch: asked, or
    /// wanting while the target `admits` one more): the loading screen's water unit. `O(1)`.
    pub(crate) fn waits_for(&self, key: PatchKey, admits: bool) -> bool {
        self.grid
            .filter(|grid| grid.contains(key))
            .is_some_and(|grid| {
                let flags = self.flags[grid.index(key)];
                flags & ASKED != 0 || wanting(flags) && admits
            })
    }

    /// Requests handed out, kept meshes dropped and requests the script refused since the node
    /// was made.
    pub(crate) fn totals(&self) -> [u64; 3] {
        [self.asked_total, self.dropped_total, self.refused_total]
    }

    /// Whether built `key` is believed wet and holding a mesh: `(wet, mesh)`, none when the book
    /// holds no node for it (the debug cross-check against the backdrop's bits).
    #[cfg(debug_assertions)]
    pub(crate) fn water_bits(&self, key: PatchKey) -> Option<(bool, bool)> {
        let grid = self.grid.filter(|grid| grid.contains(key))?;
        let flags = self.flags[grid.index(key)];
        (flags & BUILT != 0).then_some((flags & WET != 0, flags & MESH != 0))
    }

    /// One frame (after the patch units): over the target, kept meshes are dropped farthest first
    /// while the frame's budget admits a drop, until `counted + H` fits; then, when `may_ask`, up
    /// to [`ASK_PER_FRAME`] requests (and [`ASKED_MAX`] out at once; [`PRELOAD_ASKED_MAX`] both
    /// during the load's preload) go to the nearest wanting patches, each only while
    /// `counted + H + W x (asked + 1)` fits the target. Bounds: nothing
    /// without a drop or a request to make; `O(requests + drops)` and the stale entries passed,
    /// plus a sort of a list (`O(n log n)`) when it changed or the eye's patch moved since its last
    /// sort, only in a frame that asks or drops.
    pub(crate) fn frame(
        &mut self,
        eye: Option<[f32; 3]>,
        frame: KeptFrame,
        budget: &mut FrameBudget,
        clock: &dyn Clock,
        exec: &mut dyn KeptExecutor,
    ) -> KeptWork {
        let mut work = KeptWork::default();
        let Some(grid) = self.grid else {
            return work;
        };
        let mut held = frame.kept_bytes;
        let over = |held: u64| {
            frame
                .budget
                .is_some_and(|budget| frame.counted + held > budget)
        };
        if over(held) && self.kept_count > 0 {
            self.sort_lists(grid, eye);
            while over(held) && budget.admits(DROP_PRICE_US) {
                let Some(index) = self.kept.pop() else {
                    break;
                };
                let index = index as usize;
                if !kept(self.flags[index]) {
                    continue;
                }
                let key = grid.key(index);
                let start = clock.now_us();
                let freed = exec.drop_mesh(key);
                budget.record(DROP_PRICE_US, clock.now_us() - start);
                // A drop the script did not do (no kept mesh there) only clears the mark.
                let freed = freed.unwrap_or(0);
                held = held.saturating_sub(freed);
                self.set_flags(index, self.flags[index] & !(MESH | KEPT));
                work.dropped += 1;
                work.dropped_bytes += freed;
            }
            self.dropped_total += work.dropped as u64;
        }
        let (per_frame, asked_max) = if frame.preload {
            (PRELOAD_ASKED_MAX, PRELOAD_ASKED_MAX)
        } else {
            (ASK_PER_FRAME, ASKED_MAX)
        };
        if !frame.may_ask || self.wanting_count == 0 || self.asked_count >= asked_max {
            return work;
        }
        let room = kept_mesh_bytes_max(&grid);
        let fits = |asked: usize| {
            frame
                .budget
                .is_none_or(|budget| frame.counted + held + room * (asked as u64 + 1) <= budget)
        };
        if !fits(self.asked_count) {
            return work;
        }
        self.sort_lists(grid, eye);
        self.requests.clear();
        while work.asked < per_frame && self.asked_count < asked_max && fits(self.asked_count) {
            let Some(index) = self.wanting.pop() else {
                break;
            };
            let index = index as usize;
            if !wanting(self.flags[index]) {
                continue;
            }
            self.set_flags(index, self.flags[index] | ASKED);
            self.requests.extend_from_slice(&grid.key(index));
            work.asked += 1;
        }
        if self.requests.is_empty() {
            return work;
        }
        self.asked_total += work.asked as u64;
        self.refused.clear();
        exec.request(&self.requests, &mut self.refused);
        let refused = std::mem::take(&mut self.refused);
        for &[x, z, state] in refused.as_chunks::<3>().0 {
            let Some(index) = grid.contains([x, z]).then(|| grid.index([x, z])) else {
                continue;
            };
            self.refused_total += 1;
            let mut flags = self.flags[index] & !ASKED;
            match WaterState::from_code(i64::from(state)) {
                Some(state) => flags = with_state(flags, state),
                None => flags = 0,
            }
            self.set_flags(index, flags | if flags & BUILT != 0 { BARRED } else { 0 });
        }
        self.refused = refused;
        work
    }

    // The lists without stale entries, sorted by squared ground distance from the eye (the fill's
    // order: nearest first, then index), the wanting list nearest last and the kept list farthest
    // last, when either changed or the eye's patch moved since the last sort. `O(n log n)` then.
    fn sort_lists(&mut self, grid: PatchGrid, eye: Option<[f32; 3]>) {
        let at = eye.map(|eye| eye_patch(&grid, eye));
        let moved = at != self.sorted_at;
        self.sorted_at = at;
        let distance = |index: u32| ground_distance_sq(grid, eye, index as usize);
        if self.wanting_dirty || moved {
            self.wanting_dirty = false;
            let flags = &self.flags;
            self.wanting.retain(|&index| wanting(flags[index as usize]));
            self.wanting.sort_unstable();
            self.wanting.dedup();
            self.wanting
                .sort_by(|&a, &b| distance(b).total_cmp(&distance(a)).then(b.cmp(&a)));
        }
        if self.kept_dirty || moved {
            self.kept_dirty = false;
            let flags = &self.flags;
            self.kept.retain(|&index| kept(flags[index as usize]));
            self.kept.sort_unstable();
            self.kept.dedup();
            self.kept
                .sort_by(|&a, &b| distance(a).total_cmp(&distance(b)).then(a.cmp(&b)));
        }
    }

    // Applies `change` to `key`'s flags (a key off the grid is ignored). `O(1)`.
    fn update(&mut self, key: PatchKey, change: impl FnOnce(u8) -> u8) {
        let Some(grid) = self.grid.filter(|grid| grid.contains(key)) else {
            return;
        };
        let index = grid.index(key);
        self.set_flags(index, change(self.flags[index]));
    }

    // Sets the flags of `index`, keeping the counts and the lists. `O(1)`.
    fn set_flags(&mut self, index: usize, next: u8) {
        let before = self.flags[index];
        if before == next {
            return;
        }
        self.flags[index] = next;
        let count = |flags: u8, test: fn(u8) -> bool| usize::from(test(flags));
        self.wanting_count = self.wanting_count + count(next, wanting) - count(before, wanting);
        let asked = |flags: u8| flags & ASKED != 0;
        self.asked_count = self.asked_count + count(next, asked) - count(before, asked);
        self.kept_count = self.kept_count + count(next, kept) - count(before, kept);
        if wanting(next) && !wanting(before) {
            self.wanting.push(index as u32);
            self.wanting_dirty = true;
        }
        if kept(next) && !kept(before) {
            self.kept.push(index as u32);
            self.kept_dirty = true;
        }
    }
}

// `flags` with the water's `state`: wet or not, holding a mesh or not.
fn with_state(flags: u8, state: WaterState) -> u8 {
    let flags = flags & !(WET | MESH);
    match state {
        WaterState::Dry => flags,
        WaterState::Mesh => flags | WET | MESH,
        WaterState::Empty => flags | WET,
    }
}

// The patch under the eye, clamped to the grid (as `PatchGrid::target` maps a point).
fn eye_patch(grid: &PatchGrid, eye: [f32; 3]) -> PatchKey {
    let span = grid.span();
    let at = |axis: usize, value: f32| {
        let half = f64::from(grid.world[axis]) * 0.5;
        let limit = if axis == 0 {
            grid.cols - 1
        } else {
            grid.rows - 1
        };
        (((f64::from(value) + half) / span)
            .floor()
            .clamp(0.0, f64::from(limit.max(0)))) as i32
    };
    [at(0, eye[0]), at(1, eye[2])]
}

#[cfg(test)]
mod tests;
