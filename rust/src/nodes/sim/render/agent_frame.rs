// SPDX-License-Identifier: GPL-2.0-only

//! The agents' frame prepared a frame ahead on the background pool (`RENDER-47`).
//!
//! A frame of moving agents is one [`AgentFrame`]: the walkers' drawn buffers, the cars'
//! smoothing with its off-lane ground solve, the parked publication of the same snapshot, and
//! the shadow-caster candidates (`RENDER-15`'s `agents_near`). The [`AgentFrameLane`] hands it
//! to a background-pool job in frame N, and frame N+1's main thread installs what the job left
//! and starts the next: the main thread never smooths, solves or joins a Rayon pass for the
//! agents.
//!
//! **Latency rule.** The job started in frame N takes the snapshot newest when it runs and frame
//! N's delta, and computes exactly what the main thread computed in frame N before: frame N+1
//! draws the poses frame N drew before, one frame later, the sequence of drawn poses unchanged.
//! A job still running when the next frame asks is late: that frame draws what it drew, starts
//! nothing and carries its delta to the next job (one smoothing step over both deltas). The
//! parked chunks of the job's snapshot install with its cars, so a car crossing a bay's edge is
//! drawn exactly once.
//!
//! **Caster candidates.** For the shadow-only twins the stream node draws (`stream/layers/
//! agent_casters.rs`), the job keeps, per bucket, the drawn agents within
//! [`candidate_radius_m`] of its eye: the twins' exact reach plus [`CAMERA_SLACK_M`] of camera
//! travel before the frame draws. The twins' exact test ([`within_reach`]) then runs on the
//! main thread over the candidates only.

use super::car_visual::{CarVisualSmoother, GroundView};
use super::walker_visual::DrawnWalkers;
use crate::nodes::sim::core::RenderSnapshot;
use crate::simulation::parking::ParkedPublication;
use glam::Vec3;
use std::sync::{Arc, Condvar, Mutex, PoisonError};

/// Floats per car row (a 3D transform) and per walker row (transform, then custom data).
pub(crate) const CAR_STRIDE: usize = 12;
/// See [`CAR_STRIDE`].
pub(crate) const WALKER_STRIDE: usize = 16;
/// The longest shadow a 2 m agent throws with the sun above 4 degrees, metres: the twins keep
/// agents this far beyond the shadowed slice of the view (SH2's prototype).
pub(crate) const TWIN_MARGIN_M: f32 = 30.0;
/// A shadow split's far corners lie up to 1.86 times its depth from the eye at 75 degrees of
/// vertical field and 16:9 (SH2's prototype).
pub(crate) const TWIN_RADIAL_FACTOR: f32 = 1.9;
/// Camera travel, metres, between the job's eye and the frame that draws its result (two frames
/// at most): the same bound as the snapshot's cull padding (`AGENT_CULL_PADDING_M`).
pub(crate) const CAMERA_SLACK_M: f32 = crate::config::AGENT_CULL_PADDING_M;

/// Whether an agent at `offset` from the eye can shade ground the sun's shadow reaches
/// (`range` metres of view depth) seen along unit `forward`: view depth within the range plus
/// [`TWIN_MARGIN_M`] either way, and distance within [`TWIN_RADIAL_FACTOR`] times the range plus
/// the margin. An agent outside it throws no shadow on anything that is shadowed. `O(1)`.
pub(crate) fn within_reach(offset: Vec3, forward: Vec3, range: f32) -> bool {
    let depth = offset.dot(forward);
    (-TWIN_MARGIN_M..=range + TWIN_MARGIN_M).contains(&depth)
        && offset.length() <= range * TWIN_RADIAL_FACTOR + TWIN_MARGIN_M
}

/// The candidates' radius about the job's eye for a shadow range of `range` metres: every
/// agent [`within_reach`] of an eye up to [`CAMERA_SLACK_M`] away. Zero for no range.
pub(crate) fn candidate_radius_m(range: f32) -> f32 {
    if range > 0.0 {
        range * TWIN_RADIAL_FACTOR + TWIN_MARGIN_M + CAMERA_SLACK_M
    } else {
        0.0
    }
}

/// One car bucket of a snapshot as the smoothing reads it.
#[derive(Default)]
pub(crate) struct CarInput {
    /// The bucket key (`vehicle_type * 10 + colour variant`).
    pub(crate) key: u8,
    /// Snapshot poses, 12 floats each.
    pub(crate) targets: Vec<f32>,
    /// Render IDs aligned with the poses.
    pub(crate) ids: Vec<i64>,
    /// Off-lane flags aligned with the poses.
    pub(crate) flags: Vec<u8>,
}

impl CarInput {
    // Whether the counts agree (else the bucket is drawn as given, unsmoothed).
    fn aligned(&self) -> bool {
        self.targets.len() == self.ids.len() * CAR_STRIDE && self.flags.len() == self.ids.len()
    }
}

/// The car buckets of one frame, storage kept between frames.
#[derive(Default)]
pub(crate) struct CarInputs {
    buckets: Vec<CarInput>,
    used: usize,
}

impl CarInputs {
    /// Empties the set, keeping the storage.
    pub(crate) fn clear(&mut self) {
        self.used = 0;
    }

    /// The next bucket's storage, emptied, keyed `key`.
    pub(crate) fn push(&mut self, key: u8) -> &mut CarInput {
        if self.used == self.buckets.len() {
            self.buckets.push(CarInput::default());
        }
        let bucket = &mut self.buckets[self.used];
        self.used += 1;
        bucket.key = key;
        bucket.targets.clear();
        bucket.ids.clear();
        bucket.flags.clear();
        bucket
    }

    // Copies a snapshot's car buckets, every key including the emptied ones (an empty bucket
    // draws none). `O(cars)` copies.
    fn copy_snapshot(&mut self, snapshot: &RenderSnapshot) {
        self.clear();
        for (&key, targets) in &snapshot.car_transforms {
            let bucket = self.push(key);
            bucket.targets.extend_from_slice(targets);
            if let Some(ids) = snapshot.car_render_ids.get(&key) {
                bucket.ids.extend_from_slice(ids);
            }
            if let Some(flags) = snapshot.car_ground_flags.get(&key) {
                bucket.flags.extend_from_slice(flags);
            }
        }
    }

    fn iter(&self) -> impl Iterator<Item = &CarInput> {
        self.buckets[..self.used].iter()
    }
}

/// The shadow-caster candidates of a frame: per bucket key, the drawn rows within the
/// candidate radius of the job's eye (cars 12 floats a row, walkers 16).
#[derive(Default)]
pub(crate) struct CasterCandidates {
    /// By car bucket key.
    pub(crate) cars: Vec<Vec<f32>>,
    /// By walker type.
    pub(crate) walkers: Vec<Vec<f32>>,
}

impl CasterCandidates {
    /// Copies `other` into this set, keeping the storage. `O(candidates)`.
    pub(crate) fn copy_from(&mut self, other: &CasterCandidates) {
        copy_rows(&mut self.cars, &other.cars);
        copy_rows(&mut self.walkers, &other.walkers);
    }
}

fn copy_rows(to: &mut Vec<Vec<f32>>, from: &[Vec<f32>]) {
    if to.len() < from.len() {
        to.resize_with(from.len(), Vec::new);
    }
    for (index, rows) in to.iter_mut().enumerate() {
        rows.clear();
        if let Some(source) = from.get(index) {
            rows.extend_from_slice(source);
        }
    }
}

// Keeps the rows of `drawn` (`stride` floats each) whose origin lies within `radius` of `eye`.
fn keep_candidates(out: &mut Vec<f32>, drawn: &[f32], stride: usize, eye: Vec3, radius: f32) {
    out.clear();
    for row in drawn.chunks_exact(stride) {
        if Vec3::new(row[3], row[7], row[11]).distance_squared(eye) <= radius * radius {
            out.extend_from_slice(row);
        }
    }
}

/// The view a frame is prepared for, set by the main thread when it starts the job.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct FrameView {
    /// The smoothing's per-frame weight ([`CarVisualSmoother::alpha`] of the frame's delta).
    pub(crate) alpha: f32,
    /// The ground solve's camera XZ (its level of detail); `None` solves exactly everywhere.
    pub(crate) camera: Option<[f32; 2]>,
    /// The eye the caster candidates are kept about.
    pub(crate) eye: Option<[f32; 3]>,
    /// The candidates' radius ([`candidate_radius_m`]); zero keeps none.
    pub(crate) candidate_radius_m: f32,
    /// The render ID whose drawn origin the frame reports (the agent inspector's), or -1.
    pub(crate) watch: i64,
}

/// One frame of moving agents, owned by the main thread or by the job preparing it.
#[derive(Default)]
pub(crate) struct AgentFrame {
    /// The cars' smoothing and solve state.
    pub(crate) smoother: CarVisualSmoother,
    /// The walkers' drawn buffers.
    pub(crate) walkers: DrawnWalkers,
    /// The cars to smooth this frame.
    pub(crate) cars: CarInputs,
    /// The view the frame is prepared for.
    pub(crate) view: FrameView,
    /// The parked chunks of the snapshot the cars came from, and its vehicle world epoch.
    pub(crate) parked: Option<(ParkedPublication, u64)>,
    /// The snapshot's ground key ([`GroundView::key`]).
    pub(crate) ground_key: [u64; 3],
    /// Where [`FrameView::watch`] was drawn, if it was.
    pub(crate) watched: Option<Vec3>,
    /// The caster candidates, and whether this frame changed them.
    pub(crate) casters: CasterCandidates,
    pub(crate) casters_changed: bool,
    // The eye and radius the candidates were last kept for.
    casters_view: Option<([f32; 3], f32)>,
    /// Microseconds the job took (the perf channel).
    pub(crate) job_us: i64,
}

impl AgentFrame {
    /// Takes the snapshot's agents: the walkers (compared, changed buckets copied), the cars'
    /// buckets, the parked publication and the ground key. `O(drawn agents)`.
    pub(crate) fn take_snapshot(&mut self, snapshot: &RenderSnapshot) {
        self.walkers.update(&snapshot.pedestrian_transforms);
        self.cars.copy_snapshot(snapshot);
        self.parked = Some((snapshot.parked.clone(), snapshot.vehicle_world_epoch));
        self.ground_key = snapshot.vehicle_ground_key;
    }

    /// Smooths every car bucket toward its snapshot poses; the off-lane cars not at rest then
    /// wait for [`CarVisualSmoother::solve_pending`]. `O(cars)` for buckets with a moving car.
    pub(crate) fn smooth_cars(&mut self) {
        self.smoother.begin_frame(GroundView {
            key: self.ground_key,
            camera: self.view.camera,
        });
        for bucket in self.cars.iter() {
            if bucket.aligned() {
                self.smoother.smooth_bucket(
                    bucket.key,
                    &bucket.targets,
                    &bucket.ids,
                    &bucket.flags,
                    self.view.alpha,
                );
            } else {
                self.smoother.pass_through(bucket.key, &bucket.targets);
            }
        }
    }

    /// Ends the frame after the solve: records the cars, the watched car's origin and the
    /// caster candidates. `O(cars)` for the recording; the candidates `O(drawn agents)` when an
    /// agent, the eye or the radius changed, else nothing.
    pub(crate) fn finish(&mut self) {
        self.smoother.finish_frame();
        self.watched = (self.view.watch >= 0)
            .then(|| self.smoother.drawn_origin(self.view.watch))
            .flatten();
        self.keep_casters();
    }

    fn keep_casters(&mut self) {
        let view = self
            .view
            .eye
            .filter(|_| self.view.candidate_radius_m > 0.0)
            .map(|eye| (eye, self.view.candidate_radius_m));
        self.casters_changed = view != self.casters_view
            || !self.smoother.changed().is_empty()
            || !self.smoother.left().is_empty()
            || !self.walkers.changed().is_empty();
        if !self.casters_changed {
            return;
        }
        self.casters_view = view;
        let Some((eye, radius)) = view else {
            self.casters.cars.iter_mut().for_each(Vec::clear);
            self.casters.walkers.iter_mut().for_each(Vec::clear);
            return;
        };
        let eye = Vec3::from_array(eye);
        let cars = &mut self.casters.cars;
        if cars.len() < self.smoother.bucket_count() {
            cars.resize_with(self.smoother.bucket_count(), Vec::new);
        }
        for (key, out) in cars.iter_mut().enumerate() {
            let drawn = if key < self.smoother.bucket_count() {
                self.smoother.drawn_bucket(key as u8)
            } else {
                &[]
            };
            keep_candidates(out, drawn, CAR_STRIDE, eye, radius);
        }
        let walkers = &mut self.casters.walkers;
        for (key, _) in self.walkers.buckets() {
            if walkers.len() <= usize::from(key) {
                walkers.resize_with(usize::from(key) + 1, Vec::new);
            }
        }
        for (key, out) in walkers.iter_mut().enumerate() {
            keep_candidates(
                out,
                self.walkers.drawn(key as u8),
                WALKER_STRIDE,
                eye,
                radius,
            );
        }
    }
}

// Where a frame is: on the main thread (or before the first), at a job, or back from one.
enum LaneState {
    Resident(Box<AgentFrame>),
    Running,
    Landed(Box<AgentFrame>),
}

// The lane's state with the condition a waiter (tests' synchronous path) blocks on.
type Shared = Arc<(Mutex<LaneState>, Condvar)>;

/// Where the agents' frame is between the main thread and its job, and the deltas of frames
/// whose job was late. Every operation is `O(1)` on the main thread.
pub(crate) struct AgentFrameLane {
    shared: Shared,
    // Seconds of frames that started no job, carried into the next.
    carried_delta: f64,
    late_frames: u64,
}

impl Default for AgentFrameLane {
    fn default() -> Self {
        Self {
            shared: Arc::new((
                Mutex::new(LaneState::Resident(Box::default())),
                Condvar::new(),
            )),
            carried_delta: 0.0,
            late_frames: 0,
        }
    }
}

/// What [`AgentFrameLane::take`] found.
pub(crate) enum Taken {
    /// A frame a job prepared: install it, then start the next.
    Landed(Box<AgentFrame>),
    /// A frame that was not at a job (the first, or after the synchronous path): start it.
    Resident(Box<AgentFrame>),
    /// The job still runs: draw what was drawn.
    Late,
}

impl AgentFrameLane {
    fn state(&self) -> std::sync::MutexGuard<'_, LaneState> {
        self.shared.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Takes the frame for this frame's `delta` seconds, never waiting: returns it with the
    /// delta its job is to smooth over (this one's plus every late frame's since the last
    /// start), or [`Taken::Late`] and carries `delta` on.
    pub(crate) fn take(&mut self, delta: f64) -> (Taken, f64) {
        let mut state = self.state();
        let taken = match std::mem::replace(&mut *state, LaneState::Running) {
            LaneState::Running => {
                drop(state);
                self.late_frames += 1;
                self.carried_delta += delta;
                return (Taken::Late, 0.0);
            }
            LaneState::Resident(frame) => Taken::Resident(frame),
            LaneState::Landed(frame) => Taken::Landed(frame),
        };
        drop(state);
        (taken, std::mem::take(&mut self.carried_delta) + delta)
    }

    /// Takes the frame, waiting for a job in flight (the synchronous path: tests and tools),
    /// and whether a job prepared it (its outputs not yet installed).
    pub(crate) fn take_waiting(&mut self) -> (Box<AgentFrame>, bool) {
        let (lock, landed) = &*self.shared;
        let mut state = landed
            .wait_while(
                lock.lock().unwrap_or_else(PoisonError::into_inner),
                |state| matches!(state, LaneState::Running),
            )
            .unwrap_or_else(PoisonError::into_inner);
        match std::mem::replace(&mut *state, LaneState::Running) {
            LaneState::Resident(frame) => (frame, false),
            LaneState::Landed(frame) => (frame, true),
            LaneState::Running => unreachable!("waited while running"),
        }
    }

    /// Keeps `frame` on the main thread (the synchronous path), readable by [`Self::resident`].
    pub(crate) fn keep(&mut self, frame: Box<AgentFrame>) {
        *self.state() = LaneState::Resident(frame);
    }

    /// The job that runs `prepare` on `frame` and lands it for the next [`Self::take`]: the
    /// caller queues it on the background pool. The frame must have come from `take`.
    pub(crate) fn job(
        &self,
        mut frame: Box<AgentFrame>,
        prepare: impl FnOnce(&mut AgentFrame) + Send + 'static,
    ) -> impl FnOnce() + Send + 'static {
        let shared = Arc::clone(&self.shared);
        move || {
            prepare(&mut frame);
            let (lock, landed) = &*shared;
            *lock.lock().unwrap_or_else(PoisonError::into_inner) = LaneState::Landed(frame);
            landed.notify_all();
        }
    }

    /// The frame kept on the main thread by the synchronous path, if it is there.
    pub(crate) fn resident<R>(&self, read: impl FnOnce(&AgentFrame) -> R) -> Option<R> {
        match &*self.state() {
            LaneState::Resident(frame) => Some(read(frame)),
            _ => None,
        }
    }

    /// Waits until no job runs (tests).
    #[cfg(test)]
    pub(crate) fn wait_landed(&self) {
        let (lock, landed) = &*self.shared;
        drop(
            landed
                .wait_while(lock.lock().unwrap(), |state| {
                    matches!(state, LaneState::Running)
                })
                .unwrap(),
        );
    }

    /// Frames whose job was still running when they asked for it, since the lane began.
    pub(crate) fn late_frames(&self) -> u64 {
        self.late_frames
    }
}

/// The candidates handed from the agents' frame to the shadow-caster twins, and the shadow
/// range the twins last drew with (the next job's candidate radius). Main thread only.
#[derive(Default)]
pub(crate) struct CasterFeed {
    /// The latest installed frame's candidates.
    pub(crate) candidates: CasterCandidates,
    /// Advances whenever the candidates change.
    pub(crate) serial: u64,
    /// The sun's shadow range the twins drew with last, metres; zero while no twin draws.
    pub(crate) range_m: f32,
}

/// The shared feed (`SimulationNode` writes, `StreamNode` reads; both on the main thread).
pub(crate) type SharedCasterFeed = Arc<Mutex<CasterFeed>>;

/// A test bucket of cars: key, poses, render IDs, off-lane flags.
#[cfg(test)]
pub(crate) type TestCars = (u8, Vec<f32>, Vec<i64>, Vec<u8>);

/// A snapshot holding only `cars`, for tests.
#[cfg(test)]
pub(crate) fn snapshot_with_cars(cars: &[TestCars]) -> RenderSnapshot {
    use std::collections::HashMap;
    let mut snapshot = RenderSnapshot::default();
    let (mut transforms, mut ids, mut flags) = (HashMap::new(), HashMap::new(), HashMap::new());
    for (key, poses, render_ids, ground) in cars {
        transforms.insert(*key, poses.clone());
        ids.insert(*key, render_ids.clone());
        flags.insert(*key, ground.clone());
    }
    snapshot.car_transforms = transforms;
    snapshot.car_render_ids = ids;
    snapshot.car_ground_flags = flags;
    snapshot
}

#[cfg(test)]
mod tests;
