// SPDX-License-Identifier: GPL-3.0-or-later

//! The agents' frame bridge (`RENDER-47`, `docs/terrain.md` "Car smoothing a frame ahead"):
//! `agents.gd` installs, each frame, the walkers, smoothed cars and parked chunks a background
//! job prepared from the snapshot in the frame before, and starts the next job. The main thread
//! copies only the changed buckets into Godot arrays; the walkers' compare, the cars' smoothing
//! and off-lane ground solve, and the shadow casters' candidates run on the background pool
//! (`nodes/sim/render/agent_frame.rs` states the latency rule). The synchronous
//! `smooth_car_render_data` runs the same frame on the caller's thread for tests and tools.

use super::*;
use crate::nodes::sim::render::agent_frame::{
    AgentFrame, FrameView, SharedCasterFeed, Taken, candidate_radius_m,
};
use crate::nodes::sim::render::car_visual::{CarFrameParts, CarVisualSmoother};

/// The installed frame's split for the perf channel: the cars' parts
/// ([`CarFrameParts`]), the job's time on the pool and the main thread's install, microseconds,
/// and whether this frame installed one.
#[derive(Clone, Copy, Default)]
pub(crate) struct AgentParts {
    car: CarFrameParts,
    job_us: i64,
    install_us: i64,
    installed: bool,
}

#[godot_api(secondary)]
impl SimulationNode {
    /// Installs the agents' frame the background job prepared and starts the next one for this
    /// frame's `delta` seconds (`RENDER-47`). Returns `installed` (false while the job still
    /// runs: draw what was drawn), `cars` and `walkers` (the drawn buffers by bucket, the node's
    /// own dictionaries: 16 floats an agent, a transform then custom data), `changed_cars` and
    /// `changed_walkers` (the buckets to upload, those that left now absent: draw none) and,
    /// when the frame's snapshot holds parked chunks not installed, `parked` (the payload
    /// `get_vehicle_frame` gives; `full_parked` asks for a complete republication), and
    /// `vehicle_revision`, the vehicle catalogue's revision the car buckets were keyed by (a
    /// renderer with meshes of another revision rebuilds them, `RENDER-65`). The frame
    /// drawn is the one the main thread drew a frame earlier before `RENDER-47`. Main thread
    /// `O(changed buckets' agents)` copies and `O(1)` to start the job; the job `O(drawn
    /// agents)` on the background pool.
    #[func]
    pub fn advance_agent_frame(&mut self, delta: f64, full_parked: bool) -> VarDictionary {
        let started = Instant::now();
        let mut result = VarDictionary::new();
        let (taken, delta) = self.agent_lane.take(delta);
        self.changed_car_buckets.clear();
        self.changed_walker_buckets.clear();
        let installed = match taken {
            Taken::Late => None,
            Taken::Resident(frame) => Some((frame, false)),
            Taken::Landed(frame) => {
                self.install_agent_frame(&frame, Some((&mut result, full_parked)));
                Some((frame, true))
            }
        };
        self.agent_parts.installed = installed.as_ref().is_some_and(|(_, landed)| *landed);
        if let Some((frame, _)) = installed {
            self.start_agent_frame(frame, delta);
        }
        result.set("installed", self.agent_parts.installed);
        result.set("cars", self.car_buffers.clone());
        result.set("walkers", self.walker_buffers.clone());
        result.set("changed_cars", self.changed_car_buckets.clone());
        result.set("changed_walkers", self.changed_walker_buckets.clone());
        result.set("vehicle_revision", self.vehicle_revision as i64);
        self.agent_parts.install_us = started.elapsed().as_micros() as i64;
        result
    }

    /// Smooths `car_data` (buckets shaped like `get_car_render_data`) as one agents' frame on
    /// the calling thread, waiting for a job in flight, and returns the drawn car buffers by
    /// bucket like `advance_agent_frame`'s `cars` (`get_changed_car_buckets`: the buckets to
    /// upload). For tests and tools that feed their own poses; the game's renderer uses
    /// `advance_agent_frame`. Poses move toward their snapshot poses and off-lane cars are
    /// grounded; if the support solve is busy they show their validated snapshot pose. A bucket
    /// with mismatched identity or flag counts is drawn unsmoothed. O(cars).
    #[func]
    pub fn smooth_car_render_data(&mut self, car_data: VarDictionary, delta: f64) -> VarDictionary {
        let (mut frame, landed) = self.agent_lane.take_waiting();
        self.changed_car_buckets.clear();
        self.changed_walker_buckets.clear();
        if landed {
            self.install_agent_frame(&frame, None);
        }
        frame.view = FrameView {
            alpha: CarVisualSmoother::alpha(delta),
            camera: self
                .render_camera_position
                .map(|camera| [camera.x, camera.y]),
            eye: None,
            candidate_radius_m: 0.0,
            watch: self.watched_car.get(),
        };
        frame.cars.clear();
        for (key, bucket) in car_data.iter_shared() {
            let (Ok(bucket), Some(key)) = (
                bucket.try_to::<VarDictionary>(),
                key.try_to::<i64>()
                    .ok()
                    .and_then(|key| u8::try_from(key).ok()),
            ) else {
                continue;
            };
            let field = |name: &str| bucket.get(name).unwrap_or_default();
            let input = frame.cars.push(key);
            if let Ok(targets) = field("transforms").try_to::<PackedFloat32Array>() {
                input.targets.extend_from_slice(targets.as_slice());
            }
            if let Ok(ids) = field("ids").try_to::<PackedInt64Array>() {
                input.ids.extend_from_slice(ids.as_slice());
            }
            if let Ok(flags) = field("ground_flags").try_to::<PackedByteArray>() {
                input.flags.extend_from_slice(flags.as_slice());
            }
            // Custom data as given, or zero for each car (`RENDER-65`).
            match field("custom").try_to::<PackedFloat32Array>() {
                Ok(custom) => input.custom.extend_from_slice(custom.as_slice()),
                Err(_) => input.custom.resize(input.ids.len() * 4, 0.0),
            }
        }
        frame.smooth_cars();
        solve_off_lane(&mut frame, &self.core);
        frame.finish();
        self.install_cars(&frame.smoother);
        self.agent_parts.car = frame.smoother.parts();
        self.watched_origin = frame.watched.map(|origin| (frame.view.watch, origin));
        self.agent_lane.keep(frame);
        self.car_buffers.clone()
    }

    /// The buckets whose drawn buffer the last `smooth_car_render_data` call changed, including
    /// those that left (now absent from its dictionary: draw none). O(changed buckets).
    #[func]
    pub fn get_changed_car_buckets(&self) -> PackedInt64Array {
        self.changed_car_buckets.clone()
    }

    /// The installed frame's car split (`RENDER-28`), for the perf channel: microseconds
    /// smoothing (with the copy of the snapshot poses), in the off-lane ground solve (with its
    /// core attempt) and recording, all on the job's thread; then cars, off-lane cars, off-lane
    /// cars solved, cars at rest, such off-lane cars, buckets uploaded and buckets left
    /// untouched. O(1).
    #[func]
    pub fn get_car_frame_parts(&self) -> PackedInt64Array {
        PackedInt64Array::from(self.agent_parts.car.as_slice())
    }

    /// The last `advance_agent_frame` call for the perf channel (`RENDER-47`): the installed
    /// frame's job time on the background pool and the call's own time on the main thread,
    /// microseconds; 1 when it installed a frame, else 0; frames whose job was late since the
    /// node began. O(1).
    #[func]
    pub fn get_agent_frame_parts(&self) -> PackedInt64Array {
        let parts = self.agent_parts;
        PackedInt64Array::from(
            [
                parts.job_us,
                parts.install_us,
                i64::from(parts.installed),
                self.agent_lane.late_frames() as i64,
            ]
            .as_slice(),
        )
    }
}

impl SimulationNode {
    /// The feed of shadow-caster candidates the stream node's twins read (`RENDER-15`).
    pub(crate) fn agent_caster_feed(&self) -> SharedCasterFeed {
        Arc::clone(&self.agent_caster_feed)
    }

    /// Where car `render_id` was drawn in the frame on screen, if it was, and the car whose
    /// origin the next frames report. `O(1)`.
    pub(crate) fn drawn_car_origin(&self, render_id: i64) -> Option<glam::Vec3> {
        self.watched_car.set(render_id);
        self.agent_lane
            .resident(|frame| frame.smoother.drawn_origin(render_id))
            .unwrap_or_else(|| {
                self.watched_origin
                    .filter(|&(id, _)| id == render_id)
                    .map(|(_, origin)| origin)
            })
    }

    // Copies a prepared frame's changed buckets into the node's dictionaries and hands its
    // caster candidates to the twins; with `parked`, packs its parked chunks into the result.
    // `O(changed buckets' agents + candidates when they changed)`.
    fn install_agent_frame(
        &mut self,
        frame: &AgentFrame,
        parked: Option<(&mut VarDictionary, bool)>,
    ) {
        self.install_cars(&frame.smoother);
        self.vehicle_revision = frame.vehicle_revision;
        for &key in frame.walkers.changed() {
            let drawn = frame.walkers.drawn(key);
            if drawn.is_empty() {
                self.walker_buffers.remove(i64::from(key));
            } else {
                self.walker_buffers
                    .set(i64::from(key), PackedFloat32Array::from(drawn));
            }
            self.changed_walker_buckets.push(i64::from(key));
        }
        if let (Some((result, full)), Some((publication, epoch))) = (parked, &frame.parked)
            && publication.offer(full)
        {
            result.set("parked", parking_api::pack_parked(publication, *epoch));
        }
        self.watched_origin = frame.watched.map(|origin| (frame.view.watch, origin));
        self.agent_parts.car = frame.smoother.parts();
        self.agent_parts.job_us = frame.job_us;
        if frame.casters_changed {
            let mut feed = self.agent_caster_feed.lock().unwrap();
            feed.candidates.copy_from(&frame.casters);
            feed.serial += 1;
        }
    }

    // The smoothed buckets that changed or left, into the node's car dictionary.
    fn install_cars(&mut self, smoother: &CarVisualSmoother) {
        for &slot in smoother.changed() {
            self.car_buffers.set(
                i64::from(slot),
                PackedFloat32Array::from(smoother.drawn_bucket(slot)),
            );
            self.changed_car_buckets.push(i64::from(slot));
        }
        for &slot in smoother.left() {
            self.car_buffers.remove(i64::from(slot));
            self.changed_car_buckets.push(i64::from(slot));
        }
    }

    // Starts `frame`'s job on the background pool for `delta` seconds of smoothing, at the
    // view the agents last reported. `O(1)` here.
    fn start_agent_frame(&mut self, mut frame: Box<AgentFrame>, delta: f64) {
        let range = self.agent_caster_feed.lock().unwrap().range_m;
        frame.view = FrameView {
            alpha: CarVisualSmoother::alpha(delta),
            camera: self
                .render_camera_position
                .map(|camera| [camera.x, camera.y]),
            eye: self.render_eye.map(|eye| eye.to_array()),
            candidate_radius_m: candidate_radius_m(range),
            watch: self.watched_car.get(),
        };
        // A view with no agent to draw prepares in microseconds and without the pool: it runs
        // here and lands as a job would, for the next frame, rather than wake a pool worker every
        // frame (an empty world, a view beyond every agent's draw distance). `O(buckets)` to see.
        let snapshot = self.snapshot.read().unwrap();
        if holds_no_agents(&snapshot) {
            let started = Instant::now();
            frame.take_snapshot(&snapshot);
            drop(snapshot);
            finish_agent_frame(&mut frame, &self.core, started);
            self.agent_lane.land(frame);
            return;
        }
        drop(snapshot);
        let (snapshot, core) = (Arc::clone(&self.snapshot), Arc::clone(&self.core));
        let job = self.agent_lane.job(frame, move |frame| {
            let started = Instant::now();
            frame.take_snapshot(&snapshot.read().unwrap());
            finish_agent_frame(frame, &core, started);
        });
        self.async_jobs.spawn(job);
    }
}

// Whether a snapshot draws no walker and no moving car. `O(buckets)`.
fn holds_no_agents(snapshot: &RenderSnapshot) -> bool {
    snapshot.car_transforms.values().all(Vec::is_empty)
        && snapshot.pedestrian_transforms.values().all(Vec::is_empty)
}

// The frame's work after it took its snapshot: the smoothing, the solve and the recording, and
// the time since `started`.
fn finish_agent_frame(frame: &mut AgentFrame, core: &Mutex<SimCore>, started: Instant) {
    frame.smooth_cars();
    solve_off_lane(frame, core);
    frame.finish();
    frame.job_us = started.elapsed().as_micros() as i64;
}

// Grounds the frame's off-lane cars not at rest in one pass under one non-blocking core
// attempt; a busy core leaves them on their snapshot poses.
fn solve_off_lane(frame: &mut AgentFrame, core: &Mutex<SimCore>) {
    if !frame.smoother.needs_solve() {
        return;
    }
    let camera = frame.view.camera.map(|[x, z]| Vector2::new(x, z));
    let core = SimulationNode::try_lock_shared_core(core);
    frame.smoother.solve_pending(|slot, poses, solve| {
        core.as_ref()
            .is_some_and(|core| {
                core.ground_vehicle_transforms(core.vehicle_bucket_model(slot), poses, solve, camera)
            })
    });
}
