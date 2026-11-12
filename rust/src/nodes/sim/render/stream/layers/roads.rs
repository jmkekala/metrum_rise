// SPDX-License-Identifier: GPL-2.0-only

//! Road chunk hydration at a load (`RENDER-36`, `RENDER-32-P`, `docs/terrain.md` "Streaming
//! core"): the chunks of a whole network installed one unit at a time, nearest first, under a
//! budget a frame, instead of every chunk's mesh in the load's first frame (one 1.5-2 s frame at
//! 100k).
//!
//! A unit is one road chunk, atomic: its executor (`road_chunk_hydration.gd`) packs it, builds
//! its mesh and installs it through the road tool's own commit, so a chunk draws whole or not
//! at all, and the chunks a hydration has not reached yet draw nothing (they never drew in this
//! world). Units are taken in [`Class`] order (in the view, within the near ring, the rest),
//! nearest first by exact squared ground distance from the eye to the chunk's centre, then by
//! key: the near ring is drawn first, so the load's hand-over (`TERRAIN-15`, which also waits
//! for the near ring's road chunks) is not later. A unit runs if its predicted cost fits what is
//! left of the frame's hydration budget, the frame's first unit always; the prediction is the
//! chunk's vertex count times a fixed constant measured on the 100k city.
//!
//! The executor answers each unit: installed, gone (the chunk left the network since the
//! hydration began: a newer revision installed or removed it) or busy (a newer network revision
//! waits to be installed: no unit runs until it is, so a chunk never draws a revision its
//! terrain has not). Costs: nothing on a frame without pending chunks; with `p` pending, a frame
//! orders them, `O(p log p)`, and runs its units, `O(units)` here. Nothing allocates once the
//! queue's storage has grown.

use crate::nodes::sim::render::stream::schedule::{Class, Clock, FrameBudget};

/// A road chunk's key on the roadbed's chunk grid.
pub(crate) type RoadKey = [i32; 2];

/// The hydration's budget a frame, microseconds: what its units may spend beside the other
/// layers' (the load's hydration is bounded, once a world; `RENDER-32-Q` owns the sum).
pub(crate) const ROAD_HYDRATION_BUDGET_US: f64 = 4000.0;
/// Predicted cost of one road vertex packed, built into a mesh and installed, microseconds.
pub(crate) const ROAD_US_PER_VERTEX: f64 = 0.1;
/// The near ring, metres: the load's hand-over radius (`WORLD_READY_RADIUS_M`, `TERRAIN-15`).
pub(crate) const NEAR_RING_M: f32 = 2000.0;
// The vertical extent a chunk's view test assumes (the terrain patches' bounds).
const CHUNK_LOW_M: f32 = -500.0;
const CHUNK_HIGH_M: f32 = 2000.0;

/// What the executor did with a unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RoadInstall {
    /// The chunk draws.
    Installed,
    /// The chunk is no longer the hydration's: it leaves the queue.
    Gone,
    /// A newer network revision waits: no unit runs this frame.
    Busy,
    /// The road tool's chunks were reset (a world replaced): the hydration ends.
    Cancelled,
}

/// Executes hydration units.
pub(crate) trait RoadExecutor {
    /// Installs chunk `key`.
    fn install(&mut self, key: RoadKey) -> RoadInstall;
}

/// A frame's hydration work (the perf channel and tests).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct RoadWork {
    /// Chunks installed.
    pub(crate) installed: usize,
    /// Chunks that left the queue without an install.
    pub(crate) gone: usize,
    /// Whether a busy answer stopped the frame.
    pub(crate) busy: bool,
    /// Measured microseconds of the units.
    pub(crate) units_us: f64,
}

#[derive(Clone, Copy, Debug)]
struct Pending {
    key: RoadKey,
    vertices: u32,
    done: bool,
}

/// The chunks of a load's network still to install.
#[derive(Default)]
pub(crate) struct RoadHydration {
    span: f32,
    origin: [f32; 2],
    pending: Vec<Pending>,
    // This frame's order: class, squared distance, key, index into `pending` (reused storage).
    order: Vec<(Class, f64, RoadKey, usize)>,
    installed: usize,
}

impl RoadHydration {
    /// Starts a hydration of `chunks` (key and vertex count) on a grid of `span`-metre chunks
    /// whose chunk `[0, 0]` starts at `origin`; whatever was pending is dropped. `O(chunks)`.
    pub(crate) fn begin(
        &mut self,
        span: f32,
        origin: [f32; 2],
        chunks: impl Iterator<Item = (RoadKey, u32)>,
    ) {
        self.span = span;
        self.origin = origin;
        self.pending.clear();
        self.pending.extend(chunks.map(|(key, vertices)| Pending {
            key,
            vertices,
            done: false,
        }));
        self.installed = 0;
    }

    /// Drops every pending chunk.
    pub(crate) fn clear(&mut self) {
        self.pending.clear();
    }

    /// Chunks still to install.
    pub(crate) fn pending(&self) -> usize {
        self.pending.len()
    }

    /// Chunks installed since the hydration began.
    pub(crate) fn installed(&self) -> usize {
        self.installed
    }

    /// Pending chunks any point of which lies within `radius` of `eye` on the ground (the
    /// load's hand-over waits for none within the near ring). `O(pending)`.
    pub(crate) fn holes_near(&self, eye: [f32; 3], radius: f32) -> usize {
        let limit = f64::from(radius) * f64::from(radius);
        self.pending
            .iter()
            .filter(|chunk| self.nearest_sq(chunk.key, eye) <= limit)
            .count()
    }

    /// Runs the frame's units: in class order, nearest first, while `budget` admits each
    /// unit's predicted cost. `in_view` tests a box (low and high corners) against the view;
    /// without an eye every chunk is one class and the order is by key.
    pub(crate) fn frame(
        &mut self,
        eye: Option<[f32; 3]>,
        budget: &mut FrameBudget,
        clock: &impl Clock,
        in_view: &impl Fn([f32; 3], [f32; 3]) -> bool,
        exec: &mut impl RoadExecutor,
    ) -> RoadWork {
        let mut work = RoadWork::default();
        if self.pending.is_empty() {
            return work;
        }
        self.order.clear();
        let limit = f64::from(NEAR_RING_M) * f64::from(NEAR_RING_M);
        for (at, chunk) in self.pending.iter().enumerate() {
            let (class, distance) = match eye {
                Some(eye) => {
                    let [low, high] = self.bounds(chunk.key);
                    let class = if in_view(low, high) {
                        Class::HoleInView
                    } else if self.nearest_sq(chunk.key, eye) <= limit {
                        Class::HoleNear
                    } else {
                        Class::Hole
                    };
                    (class, self.centre_sq(chunk.key, eye))
                }
                None => (Class::Hole, 0.0),
            };
            self.order.push((class, distance, chunk.key, at));
        }
        self.order.sort_unstable_by(|a, b| {
            a.0.cmp(&b.0)
                .then(a.1.total_cmp(&b.1))
                .then(a.2.cmp(&b.2))
        });
        for &(_, _, key, at) in &self.order {
            let predicted = f64::from(self.pending[at].vertices) * ROAD_US_PER_VERTEX;
            if !budget.admits(predicted) {
                break;
            }
            let start = clock.now_us();
            let result = exec.install(key);
            let measured = clock.now_us() - start;
            work.units_us += measured;
            match result {
                RoadInstall::Installed => {
                    budget.record(predicted, measured);
                    self.pending[at].done = true;
                    self.installed += 1;
                    work.installed += 1;
                }
                RoadInstall::Gone => {
                    self.pending[at].done = true;
                    work.gone += 1;
                }
                RoadInstall::Busy => {
                    work.busy = true;
                    break;
                }
                RoadInstall::Cancelled => {
                    self.pending.clear();
                    return work;
                }
            }
        }
        self.pending.retain(|chunk| !chunk.done);
        work
    }

    // A chunk's box: its square on the grid, the terrain patches' vertical extent.
    fn bounds(&self, key: RoadKey) -> [[f32; 3]; 2] {
        let x = self.origin[0] + key[0] as f32 * self.span;
        let z = self.origin[1] + key[1] as f32 * self.span;
        [
            [x, CHUNK_LOW_M, z],
            [x + self.span, CHUNK_HIGH_M, z + self.span],
        ]
    }

    // Squared ground distance from the eye to the chunk's centre, in double precision.
    fn centre_sq(&self, key: RoadKey, eye: [f32; 3]) -> f64 {
        let span = f64::from(self.span);
        let cx = f64::from(self.origin[0]) + (f64::from(key[0]) + 0.5) * span;
        let cz = f64::from(self.origin[1]) + (f64::from(key[1]) + 0.5) * span;
        let (dx, dz) = (cx - f64::from(eye[0]), cz - f64::from(eye[2]));
        dx * dx + dz * dz
    }

    // Squared ground distance from the eye to the chunk's nearest point.
    fn nearest_sq(&self, key: RoadKey, eye: [f32; 3]) -> f64 {
        let span = f64::from(self.span);
        let low_x = f64::from(self.origin[0]) + f64::from(key[0]) * span;
        let low_z = f64::from(self.origin[1]) + f64::from(key[1]) * span;
        let gap = |eye: f64, low: f64| (low - eye).max(eye - (low + span)).max(0.0);
        let (dx, dz) = (gap(f64::from(eye[0]), low_x), gap(f64::from(eye[2]), low_z));
        dx * dx + dz * dz
    }
}

#[cfg(test)]
mod tests;
