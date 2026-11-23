// SPDX-License-Identifier: GPL-3.0-or-later

//! Agent instances of the render snapshot: camera cull, pose and buckets (`SIM-02-K`).
//!
//! Posing an agent samples its lane; at 100k about 45,700 agents are visible and about 4,050
//! are inside the camera. The cull therefore runs before posing, on bounds that contain the
//! pose the snapshot computes:
//!
//! - An agent without a valid lane is posed at its stored position, which the cull tests
//!   exactly; a car off every lane may be drawn round the bends of its manoeuvre (`PARK-29`) or
//!   on its gate turn (`PARK-31`), within `MAX_DRAWN_OFFSET_M` of it, so the cull tests that
//!   square about it.
//! - An agent on a lane is posed on that lane's geometry, or, while a car changes lanes, on a
//!   blend of the source and target lane at the same distance. Either pose lies in the planar
//!   box of those lanes' points ([`LaneSystem::planar_bounds`]), widened by
//!   [`CULL_ROUNDING_MARGIN_M`] for interpolation rounding, and for a car by
//!   `GATE_LANE_OFFSET_M`: near its lane point a car is drawn on its gate turn, off its lane pose
//!   (`PARK-31`). Lanes the box cannot bound, and non-finite lane distances, are posed.
//! - A walker on a foot lane is drawn beside the lane's centre line, on its own side of the
//!   sidewalk (`walker_gait`), so its box is widened by that side's reach too.
//!
//! The stored position cannot bound a lane agent's pose by any fixed margin: overlap
//! correction moves a car back along its lane without rewriting it, by up to one car spacing
//! per stacked car, and an agent that waits for a replan keeps it until it moves again; a
//! walker that has just left its door is stored at the curb while its lane distance is already
//! on the sidewalk. Daily happiness reads the stored position, so the snapshot leaves it alone.
//!
//! The agents' draw distances (`RENDER-28`) cut the same way: an agent whose bounds lie beyond
//! its kind's reach from the camera eye in the ground plane, its draw distance plus
//! `AGENT_CULL_PADDING_M`, is not posed; a posed agent is cut by the distance of its drawn origin,
//! in space, or in the ground plane for an off-lane car, which is grounded later. The ground-plane
//! distance to a box never exceeds that to a point inside it, nor that in space, so the early test
//! is conservative. Shaders fade the agents out over the band before the draw distance; the
//! padding keeps an agent the camera has neared since the snapshot inside its fade, not popping in.
//!
//! Survivors are posed by the code that posed every agent before and tested against the camera
//! exactly, so the instances are those of a full pose. Agents are posed in fixed contiguous
//! index chunks on the shared pool, and each bucket appends its instances in chunk order, which
//! is agent order: the instance order is the serial loop's, which interpolation relies on.
//!
//! [`LaneSystem::planar_bounds`]: crate::simulation::network::lanes::LaneSystem::planar_bounds

use std::collections::HashMap;

use rayon::prelude::*;

use super::{
    AGENT_SURFACE_CLEARANCE_M, RenderSnapshot, access_phase_direction, default_model_basis,
    lane_car_gate_pose, model_basis, off_lane_car_drawn, pedestrian_lane_surface_height,
    transform_floats,
};
use crate::nodes::sim::render::vehicle_signals::CarPath;
use crate::config::{AGENT_CULL_PADDING_M, CAR_DRAW_DISTANCE_M, WALKER_DRAW_DISTANCE_M};
use crate::nodes::sim::core::state::SimCore;
use crate::nodes::sim::render::lane_pose::{sample_lane_change_pose, sample_lane_pose};
use crate::simulation::economy::agents::data::AgentSystem;
use crate::simulation::economy::agents::walker_crowd::{Opposing, walker_lateral_m};
use crate::simulation::economy::agents::walker_gait::{
    SIDE_OFFSET_MAX_M, walker_is_child, walker_look, walker_pace, walker_side_offset_m,
    walker_size, walker_step_beat,
};
use crate::simulation::economy::agents::{MODE_CAR, TRANSIT_NETWORK, transit_is_visible};
use crate::simulation::network::lanes::LanePlanarBox;
use crate::simulation::network::lanes::LaneSystem;
use crate::simulation::parking::turns::{GATE_LANE_OFFSET_M, MAX_DRAWN_OFFSET_M};
use godot::prelude::{Vector2, Vector3};

// Agents per posing task. Agents inside the camera sit together in the index, so a task's cost
// runs from nothing to all of it: at 256, about 120 tasks at 30k keep eight workers level.
const POSE_CHUNK_AGENTS: usize = 256;

/// Widening of lane boxes for interpolation rounding. A lane sample is `p0 + (p1 - p0) * t`
/// with `t` in `[0, 1]`, at most about two ulps of the coordinate outside its points' box, and
/// a lane-change blend interpolates two such samples. 1 m covers coordinates below 2^20 m.
const CULL_ROUNDING_MARGIN_M: f32 = 1.0;

// One posed instance before bucketing.
struct PosedAgent {
    key: u8,
    car: bool,
    ground_flag: u8,
    render_id: i64,
    transform: [f32; 12],
    walk_phase: f32,
    // A car's instance custom data (`RENDER-65`).
    custom: [f32; 4],
    // The walker's clothes and hair (`walker_gait::walker_look`).
    look: f32,
}

// Why an agent has no instance in this snapshot.
enum Cut {
    Hidden,
    Outside,
    Far,
}

/// Per-chunk instance buffers kept across snapshots, so steady-state posing allocates nothing.
#[derive(Default)]
pub(crate) struct AgentPoseScratch {
    chunks: Vec<PoseChunk>,
    // Per chunk, the walkers and cars its draw distances cut.
    far: Vec<[u32; 2]>,
    // The last snapshot's off-lane ground solves (`PERF-33`).
    ground: GroundPoseCache,
}

// The off-lane cars' ground solves of the last snapshot, by car bucket (`PERF-33`). A solve
// (`SimCore::vehicle_ground_pose`) is a pure function of the bucket's model, the drawn position
// and heading, which branch the camera's distance picks, and the ground: the world, the ground
// generation that counts every change of it, the full-detail radius and the catalogue a bucket
// key maps through (the stamp). A car whose inputs are bitwise those of its last solve, under
// the same stamp, takes that solve: a car standing in its bay, in the leaving line or at its
// mouth is solved once. 72 bytes a cached car in each of two lists (the last snapshot's and the
// one being built), whose storage is kept; nothing per frame once they have grown.
#[derive(Default)]
struct GroundPoseCache {
    stamp: [u64; 4],
    buckets: Vec<GroundBucket>,
}

#[derive(Default)]
struct GroundBucket {
    // The last snapshot's solves, by render id.
    last: Vec<GroundSolve>,
    // This snapshot's, in row order until sorted.
    next: Vec<GroundSolve>,
    // How many of the last snapshot's were solved afresh.
    solved: usize,
}

#[derive(Clone, Copy)]
struct GroundSolve {
    render_id: i64,
    row: u32,
    // The drawn position x and z and the heading's x and z (the basis' z column before the
    // solve), bitwise, and whether the exact solve applies.
    input: [u32; 4],
    exact: bool,
    // The solved floats of the instance's row, in `GROUND_FLOATS` order.
    output: [f32; 10],
}

// The floats of a 12-float transform row the ground solve writes: the basis and the height.
const GROUND_FLOATS: [usize; 10] = [0, 1, 2, 4, 5, 6, 8, 9, 10, 7];

// One posing task's instances.
#[derive(Default)]
struct PoseChunk {
    posed: Vec<PosedAgent>,
    // The posed agents by bucket: `bucket_code(car, key) << 8 | slot`, sorted, so each bucket's
    // agents form one run in agent order (a chunk poses at most 256 agents).
    order: Vec<u32>,
}

// A bucket's sort code in a chunk's order: walkers' buckets first, each kind by key.
fn bucket_code(car: bool, key: u8) -> u32 {
    u32::from(car) << 8 | u32::from(key)
}

#[derive(Clone, Copy)]
struct Camera {
    x_min: f32,
    x_max: f32,
    z_min: f32,
    z_max: f32,
}

impl Camera {
    // The snapshot's camera test, unchanged.
    fn excludes(self, x: f32, z: f32) -> bool {
        x < self.x_min || x > self.x_max || z < self.z_min || z > self.z_max
    }

    // True only when no point of the widened box passes the camera test.
    fn misses_box(self, bounds: LanePlanarBox) -> bool {
        let [min_x, min_z, max_x, max_z] = bounds;
        max_x + CULL_ROUNDING_MARGIN_M < self.x_min
            || min_x - CULL_ROUNDING_MARGIN_M > self.x_max
            || max_z + CULL_ROUNDING_MARGIN_M < self.z_min
            || min_z - CULL_ROUNDING_MARGIN_M > self.z_max
    }
}

// The agents' draw distances from the camera eye (module docs).
#[derive(Clone, Copy)]
struct DrawEye(Vector3);

impl DrawEye {
    // The squared reach of an agent kind: its draw distance plus the latency padding.
    fn reach_sq(car: bool) -> f32 {
        let reach = AGENT_CULL_PADDING_M
            + if car {
                CAR_DRAW_DISTANCE_M
            } else {
                WALKER_DRAW_DISTANCE_M
            };
        reach * reach
    }

    // True when the drawn origin (x, y, z) lies beyond the kind's reach; `y` is `None` for an
    // off-lane car, measured in the ground plane. The ground terms are summed first, so the sum
    // is never below the ground-plane distance `misses_box` rounds.
    fn excludes(self, x: f32, y: Option<f32>, z: f32, car: bool) -> bool {
        self.distance_sq(x, y, z) > Self::reach_sq(car)
    }

    // The squared distance `excludes` compares; a car's tier is chosen by it too.
    fn distance_sq(self, x: f32, y: Option<f32>, z: f32) -> f32 {
        let (dx, dz) = (x - self.0.x, z - self.0.z);
        let dy = y.map_or(0.0, |y| y - self.0.y);
        dx * dx + dz * dz + dy * dy
    }

    // True only when no point of the widened box lies within the kind's reach in the ground
    // plane, so none lies within it in space either.
    fn misses_box(self, bounds: LanePlanarBox, car: bool) -> bool {
        let [min_x, min_z, max_x, max_z] = bounds;
        let dx = ((min_x - CULL_ROUNDING_MARGIN_M) - self.0.x)
            .max(self.0.x - (max_x + CULL_ROUNDING_MARGIN_M))
            .max(0.0);
        let dz = ((min_z - CULL_ROUNDING_MARGIN_M) - self.0.z)
            .max(self.0.z - (max_z + CULL_ROUNDING_MARGIN_M))
            .max(0.0);
        dx * dx + dz * dz > Self::reach_sq(car)
    }
}

// What the snapshot cuts agents to: the camera's ground rectangle and the draw distances.
#[derive(Clone, Copy)]
struct AgentView {
    rect: Option<Camera>,
    eye: Option<DrawEye>,
}

impl AgentView {
    fn cuts_anything(self) -> bool {
        self.rect.is_some() || self.eye.is_some()
    }

    // The cut of an agent whose pose lies in the widened `bounds`, if certain.
    fn cut_box(self, bounds: LanePlanarBox, car: bool) -> Option<Cut> {
        if self.rect.is_some_and(|rect| rect.misses_box(bounds)) {
            Some(Cut::Outside)
        } else if self.eye.is_some_and(|eye| eye.misses_box(bounds, car)) {
            Some(Cut::Far)
        } else {
            None
        }
    }

    // The cut of an agent drawn at (x, y, z), `y` as in `DrawEye::excludes`.
    fn cut_point(self, x: f32, y: Option<f32>, z: f32, car: bool) -> Option<Cut> {
        if self.rect.is_some_and(|rect| rect.excludes(x, z)) {
            Some(Cut::Outside)
        } else if self.eye.is_some_and(|eye| eye.excludes(x, y, z, car)) {
            Some(Cut::Far)
        } else {
            None
        }
    }
}

// Over this distance from its attach and detach points a walker eases between its side of the
// sidewalk and the centre line its door's path meets, rather than stepping sideways.
const WALKER_SIDE_EASE_M: f32 = 3.0;
// A walker faces where its drawn side takes it this far ahead, seconds: it turns into a
// side-step instead of sliding sideways.
const WALKER_TURN_AHEAD_S: f32 = 0.5;
// The widest such turn from the lane's direction, radians (about 20 degrees).
const WALKER_TURN_MAX_RAD: f32 = 0.35;
// Below this speed, m/s, a walker faces along its lane.
const WALKER_TURN_MIN_MS: f32 = 0.2;

// Walker `i`'s drawn shift from foot lane `lane_id`'s centre line at its lane distance, along
// `tangent`, and the direction it faces: its own side, or the room it makes for the walkers
// around it (`walker_crowd`), turned toward where that side is going. Presentation only, never
// stored. O(log bucket).
fn walker_side_pose(
    agents: &AgentSystem,
    lanes: &LaneSystem,
    i: usize,
    lane_id: usize,
    tangent: Vector3,
) -> (Vector3, Vector3) {
    // Left of travel in the ground plane (Y up): (t.z, -t.x).
    let left = Vector3::new(tangent.z, 0.0, -tangent.x);
    let len = left.length();
    if len.is_nan() || len <= 1e-6 {
        return (Vector3::ZERO, tangent);
    }
    let left = left / len;
    let speed = agents.speed[i];
    let side_now = walker_side_m(agents, lanes, i, lane_id, 0.0);
    let mut forward = tangent;
    if speed >= WALKER_TURN_MIN_MS {
        let side_ahead = walker_side_m(agents, lanes, i, lane_id, WALKER_TURN_AHEAD_S);
        let slope = (side_ahead - side_now) / (speed * WALKER_TURN_AHEAD_S);
        let turn = slope
            .atan()
            .clamp(-WALKER_TURN_MAX_RAD, WALKER_TURN_MAX_RAD);
        let flat = Vector3::new(tangent.x, 0.0, tangent.z) / len;
        forward = flat * turn.cos() + left * turn.sin();
        forward.y = tangent.y;
    }
    (left * side_now, forward)
}

// Walker `i`'s drawn side, metres left of lane `lane_id`'s centre line, `ahead_s` seconds on
// with everyone at their last speed: eased to the centre line near its door's attach and detach
// points. O(log bucket).
fn walker_side_m(
    agents: &AgentSystem,
    lanes: &LaneSystem,
    i: usize,
    lane_id: usize,
    ahead_s: f32,
) -> f32 {
    let speed_of = |agent: usize| agents.speed[agent];
    let d = agents.lane_distance[i];
    let d_ahead = d + speed_of(i) * ahead_s;
    let mut ease = 1.0_f32;
    for (plan_lane, plan_d) in [
        (
            agents.planned_attach_lane_id[i],
            agents.planned_attach_lane_d[i],
        ),
        (
            agents.planned_detach_lane_id[i],
            agents.planned_detach_lane_d[i],
        ),
    ] {
        if plan_lane as usize == lane_id {
            ease = ease.min((d_ahead - plan_d).abs() / WALKER_SIDE_EASE_M);
        }
    }
    let ease = ease.clamp(0.0, 1.0);
    let ease = ease * ease * (3.0 - 2.0 * ease);
    let pace_of = |agent: usize| walker_pace(agents.schedule_seed[agent]);
    let preferred = walker_side_offset_m(agents.schedule_seed[i]);
    // The retained buckets name agents by index. After agents were removed outside the tick (a
    // demolition, a road edit) they are stale until the next tick rebuilds them, and an entry
    // may name an index past the end; the walker keeps its own side meanwhile.
    let buckets = agents
        .lane_bucket_snapshot_valid
        .then_some(&agents.lane_buckets);
    let side = match (lanes.lanes.get(lane_id), buckets.and_then(|b| b.get(lane_id))) {
        (Some(lane), Some(bucket)) => {
            let opposing = lanes.opposing_foot_lane(lane_id).and_then(|id| {
                Some(Opposing {
                    bucket: agents.lane_buckets.get(id)?,
                    length: lanes.lanes[id].length,
                })
            });
            let pace = pace_of(i);
            walker_lateral_m(
                preferred,
                d,
                i,
                pace,
                lane.length,
                bucket,
                opposing,
                pace_of,
                ahead_s,
                speed_of,
            )
        }
        _ => preferred,
    };
    side * ease
}

fn widen(b: LanePlanarBox, m: f32) -> LanePlanarBox {
    [b[0] - m, b[1] - m, b[2] + m, b[3] + m]
}

fn union(a: LanePlanarBox, b: LanePlanarBox) -> LanePlanarBox {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

impl SimCore {
    // The rectangle is `None` while the camera box is the default "show all", the eye until a
    // renderer reports one.
    fn agent_view(&self) -> AgentView {
        let (x_min, x_max, z_min, z_max) = self.camera_aabb;
        AgentView {
            rect: (x_min < x_max).then_some(Camera {
                x_min,
                x_max,
                z_min,
                z_max,
            }),
            eye: self.agent_draw_eye.map(DrawEye),
        }
    }

    /// Fills the cleared agent buckets of `snapshot` and grounds its off-lane cars.
    ///
    /// O(agents) cheap cull tests and O(in-camera agents within their draw distance) poses, both
    /// on the shared pool, plus O(lane points) when the lane revision changed.
    pub(super) fn build_agent_instances(&mut self, snapshot: &mut RenderSnapshot) {
        let view = self.agent_view();
        if view.cuts_anything() {
            self.transit_network.lane_system.refresh_planar_bounds();
        }
        let mut scratch = std::mem::take(&mut self.snapshot_agent_scratch);
        let core = &*self;
        let agent_count = core.agents.len();
        let chunks = agent_count.div_ceil(POSE_CHUNK_AGENTS);
        scratch.chunks.resize_with(chunks, PoseChunk::default);
        scratch.far.resize(chunks, [0; 2]);
        scratch
            .chunks
            .par_iter_mut()
            .zip(scratch.far.par_iter_mut())
            .enumerate()
            .for_each(|(chunk, (PoseChunk { posed, order }, far))| {
                posed.clear();
                *far = [0; 2];
                let start = chunk * POSE_CHUNK_AGENTS;
                for i in start..(start + POSE_CHUNK_AGENTS).min(agent_count) {
                    match core.cull_and_pose_agent(i, view) {
                        Ok(agent) => posed.push(agent),
                        Err(Cut::Far) => {
                            far[usize::from(core.agents.transit_mode[i] == MODE_CAR)] += 1
                        }
                        Err(Cut::Hidden | Cut::Outside) => {}
                    }
                }
                // Slots below 256 keep each bucket's run in agent order.
                order.clear();
                order.extend(
                    posed
                        .iter()
                        .enumerate()
                        .map(|(slot, agent)| bucket_code(agent.car, agent.key) << 8 | slot as u32),
                );
                order.sort_unstable();
            });
        snapshot.agent_draw_cut = scratch
            .far
            .iter()
            .fold([0; 2], |sum, far| [sum[0] + far[0], sum[1] + far[1]]);
        bucket_posed_agents(&scratch.chunks, snapshot);
        self.ground_off_lane_cars_cached(snapshot, &mut scratch.ground);
        self.snapshot_agent_scratch = scratch;
    }

    fn cull_and_pose_agent(&self, i: usize, view: AgentView) -> Result<PosedAgent, Cut> {
        if !transit_is_visible(self.agents.transit[i]) {
            return Err(Cut::Hidden);
        }
        if view.cuts_anything()
            && let Some(cut) = self.cut_before_posing(i, view)
        {
            return Err(cut);
        }
        self.pose_agent(i, view)
    }

    // The cut agent `i`'s pose cannot escape, if any (module docs). O(1).
    fn cut_before_posing(&self, i: usize, view: AgentView) -> Option<Cut> {
        let car = self.agents.transit_mode[i] == MODE_CAR;
        let boxes = self.transit_network.lane_system.planar_bounds();
        let Some(&lane_box) = boxes.get(self.agents.current_lane_id[i]) else {
            let (x, z) = (self.agents.pos_x[i], self.agents.pos_y[i]);
            if car {
                // Drawn on its manoeuvre's arcs or its gate turn, near the stored position
                // (`PARK-29`, `PARK-31`).
                let m = MAX_DRAWN_OFFSET_M;
                return view.cut_box([x - m, z - m, x + m, z + m], car);
            }
            // Posed at the stored position; a walker's height comes later, never nearer.
            return view.cut_point(x, None, z, car);
        };
        if !self.agents.lane_distance[i].is_finite() {
            return None;
        }
        // A car may be drawn on its gate turn, off its lane pose (`PARK-31`); a walker up to its
        // side offset beside the lane.
        let mut bounds = widen(
            lane_box,
            if car {
                GATE_LANE_OFFSET_M
            } else {
                SIDE_OFFSET_MAX_M
            },
        );
        // A superset of the lane-change blend's condition in `pose_agent`.
        if car
            && self.agents.transit[i] == TRANSIT_NETWORK
            && let Some(&source_box) = boxes.get(self.agents.lane_change_from_lane_id[i] as usize)
        {
            if !self.agents.lane_change_start_d[i].is_finite() {
                return None;
            }
            let m = GATE_LANE_OFFSET_M;
            let source_box = [source_box[0] - m, source_box[1] - m, source_box[2] + m, source_box[3] + m];
            bounds = union(bounds, source_box);
        }
        view.cut_box(bounds, car)
    }

    // Agent `i`'s instance, or why the view cuts it. The pose of the full-pose loop, unchanged.
    fn pose_agent(&self, i: usize, view: AgentView) -> Result<PosedAgent, Cut> {
        let mut world_x = self.agents.pos_x[i];
        let mut world_z = self.agents.pos_y[i];
        let mut lane_pose = None;
        let mut changing = false;
        let mut gate = None;
        let mut pedestrian_lane_surface_y = None;
        let lane_id = self.agents.current_lane_id[i];
        if lane_id != usize::MAX && lane_id < self.transit_network.lane_system.lanes.len() {
            let lane = &self.transit_network.lane_system.lanes[lane_id];
            lane_pose = if self.agents.transit_mode[i] == MODE_CAR
                && self.agents.transit[i] == TRANSIT_NETWORK
                && let Some(source_lane) = self
                    .transit_network
                    .lane_system
                    .lanes
                    .get(self.agents.lane_change_from_lane_id[i] as usize)
                && source_lane.edge_id != usize::MAX
                && source_lane.edge_id == lane.edge_id
                && source_lane.is_fwd == lane.is_fwd
                && source_lane.lane_type == lane.lane_type
            {
                changing = true;
                sample_lane_change_pose(
                    source_lane,
                    lane,
                    self.agents.lane_distance[i],
                    self.agents.lane_change_start_d[i],
                    self.agents.lane_change_length_m[i],
                )
            } else {
                sample_lane_pose(lane, self.agents.lane_distance[i])
            };
            // Near the lane point it leaves or joined its lane at, a car is drawn on its gate
            // turn (`PARK-31`).
            if let Some((pos, tangent)) = lane_pose
                && self.agents.transit_mode[i] == MODE_CAR
                && self.agents.transit[i] == TRANSIT_NETWORK
                && let Some((pos, tangent, curvature)) =
                    lane_car_gate_pose(self, i, changing, pos, tangent)
            {
                lane_pose = Some((pos, tangent));
                gate = Some(curvature);
            }
            if let Some((pos, tangent)) = lane_pose {
                let mut shift = Vector3::ZERO;
                if self.agents.transit_mode[i] != MODE_CAR {
                    let lanes = &self.transit_network.lane_system;
                    let facing;
                    (shift, facing) = walker_side_pose(&self.agents, lanes, i, lane_id, tangent);
                    lane_pose = Some((pos, facing));
                }
                world_x = pos.x + shift.x;
                world_z = pos.z + shift.z;
                pedestrian_lane_surface_y = Some(pedestrian_lane_surface_height(lane, pos.y));
            }
        }
        // Off every lane a car is drawn round the bends of its manoeuvre (`PARK-29`).
        let mut off_lane_facing = None;
        let mut path = CarPath::Lane { changing, gate };
        if lane_pose.is_none() && self.agents.transit_mode[i] == MODE_CAR {
            let drawn = off_lane_car_drawn(self, i, world_x, world_z);
            (world_x, world_z, off_lane_facing) = (drawn.x, drawn.z, drawn.facing);
            path = CarPath::OffLane {
                facing: drawn.facing,
                curvature: drawn.curvature,
            };
        }

        if view
            .rect
            .is_some_and(|rect| rect.excludes(world_x, world_z))
        {
            return Err(Cut::Outside);
        }
        // Lane geometry owns network height. Both off-lane modes use the same current
        // road/pad/CDT ownership query in ingress and egress, including cut-away terrain.
        if self.agents.transit_mode[i] != MODE_CAR {
            let world_y = pedestrian_lane_surface_y.unwrap_or_else(|| {
                self.get_world_surface_height_internal(Vector2::new(world_x, world_z))
            }) + AGENT_SURFACE_CLEARANCE_M;
            if view
                .eye
                .is_some_and(|eye| eye.excludes(world_x, Some(world_y), world_z, false))
            {
                return Err(Cut::Far);
            }
            let forward = lane_pose
                .map(|(_, tangent)| tangent)
                .or_else(|| access_phase_direction(self, i, world_x, world_z));
            // GLTF export converts Blender -Y (character facing) to +Z, so the
            // model faces +Z in Godot. basis_z = fwd aligns +Z with travel dir.
            let basis = forward.map(model_basis).unwrap_or_else(default_model_basis);
            let seed = self.agents.schedule_seed[i];
            let child = walker_is_child(self.agents.age_group[i], self.agents.household_id[i]);
            let basis = basis.map(|axis| axis * walker_size(seed, child));
            Ok(PosedAgent {
                key: self.agents.pedestrian_type[i],
                car: false,
                ground_flag: 0,
                render_id: 0,
                transform: transform_floats(basis, Vector3::new(world_x, world_y, world_z)),
                walk_phase: (self.agents.walk_phase[i] + walker_step_beat(seed)).fract(),
                custom: [0.0; 4],
                look: walker_look(seed),
            })
        } else {
            // Off-lane footprints are grounded per bucket after bucketing.
            let render_id = self.agents.render_id[i];
            let (world_y, basis) = if let Some((pos, tangent)) = lane_pose {
                (pos.y + AGENT_SURFACE_CLEARANCE_M, model_basis(-tangent))
            } else {
                let heading = off_lane_facing
                    .map(|direction| -direction)
                    .unwrap_or(Vector3::BACK);
                (0.0, model_basis(heading))
            };
            // An off-lane car is grounded later: its ground-plane distance.
            let drawn_y = lane_pose.is_some().then_some(world_y);
            if view
                .eye
                .is_some_and(|eye| eye.excludes(world_x, drawn_y, world_z, true))
            {
                return Err(Cut::Far);
            }
            // The tier by the distance the draw distance measures (`RENDER-65`).
            let distance_sq = view.eye.map(|eye| eye.distance_sq(world_x, drawn_y, world_z));
            let (key, custom) = self.drawn_car(i, distance_sq, path);
            Ok(PosedAgent {
                key,
                car: true,
                ground_flag: u8::from(lane_pose.is_none()),
                render_id: render_id.min(i64::MAX as u64) as i64,
                transform: transform_floats(basis, Vector3::new(world_x, world_y, world_z)),
                walk_phase: 0.0,
                custom,
                look: 0.0,
            })
        }
    }

    // Grounds each car bucket's off-lane instances as `ground_off_lane_cars` does, taking the
    // last snapshot's solve wherever a car's inputs are unchanged (`GroundPoseCache`). Buckets
    // are independent. O(off-lane cars) to list them, O(log bucket) a lookup, a solve for each
    // car whose inputs changed, and O(m log m) to order a bucket's m solves by render id.
    fn ground_off_lane_cars_cached(&self, snapshot: &mut RenderSnapshot, cache: &mut GroundPoseCache) {
        let stamp = [
            self.world_epoch,
            self.vehicle_ground_generation,
            u64::from(self.vehicle_ground_full_detail_m.to_bits()),
            self.vehicle_catalogue.revision,
        ];
        cache.buckets.resize_with(256, GroundBucket::default);
        if cache.stamp != stamp {
            cache.stamp = stamp;
            for bucket in &mut cache.buckets {
                bucket.last.clear();
            }
        }
        let mut rows: [Option<&mut Vec<f32>>; 256] = std::array::from_fn(|_| None);
        for (&key, buffer) in snapshot.car_transforms.iter_mut() {
            rows[usize::from(key)] = Some(buffer);
        }
        // The buckets drawn, each with its cache; a bucket no car is drawn in forgets its solves.
        let mut entries: [Option<(u8, &mut Vec<f32>, &mut GroundBucket)>; 256] =
            std::array::from_fn(|_| None);
        let mut count = 0;
        for (key, (buffer, bucket)) in rows.iter_mut().zip(&mut cache.buckets).enumerate() {
            if let Some(buffer) = buffer.take() {
                entries[count] = Some((key as u8, buffer, bucket));
                count += 1;
            } else {
                bucket.last.clear();
                bucket.solved = 0;
            }
        }
        let (flags, ids) = (&snapshot.car_ground_flags, &snapshot.car_render_ids);
        let camera = self.render_camera_position;
        crate::thread_pools::install(|| {
            entries[..count]
                .par_iter_mut()
                .with_max_len(1)
                .flatten()
                .for_each(|(key, buffer, bucket)| {
                    self.ground_bucket(*key, buffer, &flags[key], &ids[key], bucket, camera)
                })
        });
    }

    /// The off-lane cars the last snapshot drew, and how many of them it solved afresh rather
    /// than took from the snapshot before (`PERF-33`).
    #[cfg(test)]
    pub(crate) fn snapshot_ground_solves(&self) -> (usize, usize) {
        let buckets = &self.snapshot_agent_scratch.ground.buckets;
        (
            buckets.iter().map(|bucket| bucket.last.len()).sum(),
            buckets.iter().map(|bucket| bucket.solved).sum(),
        )
    }

    // One bucket of `ground_off_lane_cars_cached`.
    fn ground_bucket(
        &self,
        key: u8,
        transforms: &mut [f32],
        flags: &[u8],
        ids: &[i64],
        bucket: &mut GroundBucket,
        camera: Option<Vector2>,
    ) {
        let full_detail_sq = self.vehicle_ground_full_detail_m * self.vehicle_ground_full_detail_m;
        bucket.next.clear();
        for (row, (&flag, &render_id)) in flags.iter().zip(ids).enumerate() {
            if flag == 0 {
                continue;
            }
            let t = &transforms[row * 12..row * 12 + 12];
            let position = Vector2::new(t[3], t[11]);
            bucket.next.push(GroundSolve {
                render_id,
                row: row as u32,
                input: [t[3], t[11], t[2], t[10]].map(f32::to_bits),
                // `vehicle_ground_pose`'s own test.
                exact: camera.is_none_or(|camera| {
                    camera.distance_squared_to(position) <= full_detail_sq
                }),
                output: [0.0; 10],
            });
        }
        if bucket.next.is_empty() {
            bucket.last.clear();
            bucket.solved = 0;
            return;
        }
        let vehicle = self.vehicle_bucket_model(key);
        let last = &bucket.last;
        // No minimum split: a solve costs microseconds (`solve_vehicle_transforms`).
        bucket.solved = crate::thread_pools::install(|| {
            bucket.next.par_iter_mut().map(|solve| {
                let kept = last
                    .binary_search_by_key(&solve.render_id, |kept| kept.render_id)
                    .ok()
                    .map(|k| &last[k])
                    .filter(|kept| kept.input == solve.input && kept.exact == solve.exact);
                if let Some(kept) = kept {
                    solve.output = kept.output;
                    return 0;
                }
                let [x, z, heading_x, heading_z] = solve.input.map(f32::from_bits);
                let (height, [x_axis, y_axis, z_axis]) = self.vehicle_ground_pose(
                    vehicle,
                    Vector2::new(x, z),
                    Vector3::new(heading_x, 0.0, heading_z),
                    camera,
                );
                solve.output = [
                    x_axis.x, y_axis.x, z_axis.x, x_axis.y, y_axis.y, z_axis.y, x_axis.z,
                    y_axis.z, z_axis.z, height,
                ];
                1
            })
            .sum::<usize>()
        });
        for solve in &bucket.next {
            let row = solve.row as usize * 12;
            for (&float, &value) in GROUND_FLOATS.iter().zip(&solve.output) {
                transforms[row + float] = value;
            }
        }
        // Render ids are unique to an agent.
        bucket.next.sort_unstable_by_key(|solve| solve.render_id);
        std::mem::swap(&mut bucket.last, &mut bucket.next);
    }

    // Grounds each car bucket's off-lane instances, solving every one: the reference the
    // cached grounding must reproduce. Buckets are independent.
    #[cfg(test)]
    fn ground_off_lane_cars(&self, snapshot: &mut RenderSnapshot) {
        let flags = &snapshot.car_ground_flags;
        par_for_each_bucket(&mut snapshot.car_transforms, |key, buffer| {
            self.solve_vehicle_transforms(
                self.vehicle_bucket_model(key),
                buffer,
                &flags[&key],
                self.render_camera_position,
            );
        });
    }
}

// Appends every posed instance to its bucket, each bucket on its own task in chunk order, from
// each chunk's run of it. Buckets are created for the keys present, as the serial fill created
// them. O(P) to list the keys and copy, plus O(buckets x chunks x log 256) for the runs, P posed
// agents: a bucket no longer scans every posed agent.
fn bucket_posed_agents(chunks: &[PoseChunk], snapshot: &mut RenderSnapshot) {
    let mut present = [[false; 256]; 2];
    for agent in chunks.iter().flat_map(|chunk| &chunk.posed) {
        present[usize::from(agent.car)][usize::from(agent.key)] = true;
    }
    for key in 0..=u8::MAX {
        if present[0][usize::from(key)] {
            snapshot.pedestrian_transforms.entry(key).or_default();
        }
        if present[1][usize::from(key)] {
            snapshot.car_transforms.entry(key).or_default();
            snapshot.car_custom.entry(key).or_default();
            snapshot.car_render_ids.entry(key).or_default();
            snapshot.car_ground_flags.entry(key).or_default();
        }
    }
    let RenderSnapshot {
        pedestrian_transforms,
        car_transforms,
        car_custom,
        car_render_ids,
        car_ground_flags,
        ..
    } = snapshot;
    rayon::join(
        || {
            fill_buckets(pedestrian_transforms, chunks, false, |buffer, agent| {
                buffer.extend_from_slice(&agent.transform);
                // Walk phase in CUSTOM_DATA0.x (the MultiMesh uses custom data).
                buffer.extend_from_slice(&[agent.walk_phase, agent.look, 0.0, 0.0]);
            })
        },
        || {
            rayon::join(
                || {
                    // The custom data after the poses on the same task: each fill is parallel
                    // over its buckets already.
                    fill_buckets(car_transforms, chunks, true, |buffer, agent| {
                        buffer.extend_from_slice(&agent.transform)
                    });
                    fill_buckets(car_custom, chunks, true, |buffer, agent| {
                        buffer.extend_from_slice(&agent.custom)
                    });
                },
                || {
                    rayon::join(
                        || {
                            fill_buckets(car_render_ids, chunks, true, |ids, agent| {
                                ids.push(agent.render_id)
                            })
                        },
                        || {
                            fill_buckets(car_ground_flags, chunks, true, |flags, agent| {
                                flags.push(agent.ground_flag)
                            })
                        },
                    )
                },
            )
        },
    );
}

fn fill_buckets<T: Send>(
    buckets: &mut HashMap<u8, Vec<T>>,
    chunks: &[PoseChunk],
    cars: bool,
    push: impl Fn(&mut Vec<T>, &PosedAgent) + Sync,
) {
    par_for_each_bucket(buckets, |key, bucket| {
        let code = bucket_code(cars, key);
        for PoseChunk { posed, order } in chunks {
            let start = order.partition_point(|&entry| entry >> 8 < code);
            let run = order[start..]
                .iter()
                .take_while(|&&entry| entry >> 8 == code);
            for &entry in run {
                push(bucket, &posed[(entry & 0xff) as usize]);
            }
        }
    });
}

// Runs `visit` on every bucket on the current pool without allocating: rayon's map adapter
// stages the entries in a `Vec` and `par_bridge` allocates its thread flags on every call, so
// the buckets (at most one per `u8` key) are borrowed into a stack array instead. One bucket per
// task, as `par_bridge` handed them out: the buckets' costs differ widely.
fn par_for_each_bucket<T: Send>(
    buckets: &mut HashMap<u8, Vec<T>>,
    visit: impl Fn(u8, &mut Vec<T>) + Sync,
) {
    let mut entries: [Option<(u8, &mut Vec<T>)>; 256] = std::array::from_fn(|_| None);
    let count = buckets.len();
    for (entry, (&key, bucket)) in entries.iter_mut().zip(buckets.iter_mut()) {
        *entry = Some((key, bucket));
    }
    entries[..count]
        .par_iter_mut()
        .with_max_len(1)
        .flatten()
        .for_each(|(key, bucket)| visit(*key, bucket));
}

#[cfg(test)]
impl SimCore {
    /// Agent `i`'s car bucket and instance custom data as the snapshot poses it with nothing cut
    /// (`RENDER-66` tests); `None` for a walker or a hidden agent.
    pub(crate) fn posed_car_custom(&self, i: usize) -> Option<(u8, [f32; 4])> {
        let view = AgentView {
            rect: None,
            eye: None,
        };
        (transit_is_visible(self.agents.transit[i]) && self.agents.transit_mode[i] == MODE_CAR)
            .then(|| self.pose_agent(i, view).ok())
            .flatten()
            .map(|agent| (agent.key, agent.custom))
    }

    /// Car `i` as the snapshot poses it with nothing cut (`PARK-31` tests): its drawn ground
    /// point, its nose direction on the ground plane, whether it is posed on a lane, and its
    /// instance custom data. `None` for a walker or a hidden agent.
    pub(crate) fn posed_car_pose(&self, i: usize) -> Option<(Vector2, Vector2, bool, [f32; 4])> {
        let view = AgentView {
            rect: None,
            eye: None,
        };
        let agent = (transit_is_visible(self.agents.transit[i])
            && self.agents.transit_mode[i] == MODE_CAR)
            .then(|| self.pose_agent(i, view).ok())
            .flatten()?;
        let t = agent.transform;
        // The basis' z column points against the nose (`model_basis(-heading)`).
        let nose = Vector2::new(-t[2], -t[10]);
        Some((
            Vector2::new(t[3], t[11]),
            nose.normalized(),
            agent.ground_flag == 0,
            agent.custom,
        ))
    }

    /// The serial loop that posed every visible agent before culling (before `SIM-02-K`),
    /// kept as the reference the culled, parallel path must reproduce bit for bit.
    pub(in crate::nodes::sim::core) fn build_agent_instances_full_pose(
        &self,
        snapshot: &mut RenderSnapshot,
    ) {
        use super::push_transform;

        for buffer in snapshot.pedestrian_transforms.values_mut() {
            buffer.clear();
        }
        for buffer in snapshot.car_transforms.values_mut() {
            buffer.clear();
        }
        for buffer in snapshot.car_custom.values_mut() {
            buffer.clear();
        }
        for ids in snapshot.car_render_ids.values_mut() {
            ids.clear();
        }
        for flags in snapshot.car_ground_flags.values_mut() {
            flags.clear();
        }

        let (aabb_x_min, aabb_x_max, aabb_z_min, aabb_z_max) = self.camera_aabb;
        let cull = aabb_x_min < aabb_x_max; // false when default "show all"
        let eye = self.agent_draw_eye.map(DrawEye);

        for i in 0..self.agents.len() {
            if !transit_is_visible(self.agents.transit[i]) {
                continue;
            }

            let mut world_x = self.agents.pos_x[i];
            let mut world_z = self.agents.pos_y[i];
            let mut lane_pose = None;
            let mut changing = false;
            let mut gate = None;
            let mut pedestrian_lane_surface_y = None;
            let lane_id = self.agents.current_lane_id[i];
            if lane_id != usize::MAX && lane_id < self.transit_network.lane_system.lanes.len() {
                let lane = &self.transit_network.lane_system.lanes[lane_id];
                lane_pose = if self.agents.transit_mode[i] == MODE_CAR
                    && self.agents.transit[i] == TRANSIT_NETWORK
                    && let Some(source_lane) = self
                        .transit_network
                        .lane_system
                        .lanes
                        .get(self.agents.lane_change_from_lane_id[i] as usize)
                    && source_lane.edge_id != usize::MAX
                    && source_lane.edge_id == lane.edge_id
                    && source_lane.is_fwd == lane.is_fwd
                    && source_lane.lane_type == lane.lane_type
                {
                    changing = true;
                    sample_lane_change_pose(
                        source_lane,
                        lane,
                        self.agents.lane_distance[i],
                        self.agents.lane_change_start_d[i],
                        self.agents.lane_change_length_m[i],
                    )
                } else {
                    sample_lane_pose(lane, self.agents.lane_distance[i])
                };
                if let Some((pos, tangent)) = lane_pose
                    && self.agents.transit_mode[i] == MODE_CAR
                    && self.agents.transit[i] == TRANSIT_NETWORK
                    && let Some((pos, tangent, curvature)) =
                        lane_car_gate_pose(self, i, changing, pos, tangent)
                {
                    lane_pose = Some((pos, tangent));
                    gate = Some(curvature);
                }
                if let Some((pos, tangent)) = lane_pose {
                    let mut shift = Vector3::ZERO;
                    if self.agents.transit_mode[i] != MODE_CAR {
                        let lanes = &self.transit_network.lane_system;
                        let facing;
                        (shift, facing) =
                            walker_side_pose(&self.agents, lanes, i, lane_id, tangent);
                        lane_pose = Some((pos, facing));
                    }
                    world_x = pos.x + shift.x;
                    world_z = pos.z + shift.z;
                    pedestrian_lane_surface_y = Some(pedestrian_lane_surface_height(lane, pos.y));
                }
            }
            let mut off_lane_facing = None;
            let mut path = CarPath::Lane { changing, gate };
            if lane_pose.is_none() && self.agents.transit_mode[i] == MODE_CAR {
                let drawn = off_lane_car_drawn(self, i, world_x, world_z);
                (world_x, world_z, off_lane_facing) = (drawn.x, drawn.z, drawn.facing);
                path = CarPath::OffLane {
                    facing: drawn.facing,
                    curvature: drawn.curvature,
                };
            }

            if cull
                && (world_x < aabb_x_min
                    || world_x > aabb_x_max
                    || world_z < aabb_z_min
                    || world_z > aabb_z_max)
            {
                continue;
            }
            if self.agents.transit_mode[i] != MODE_CAR {
                let p_type = self.agents.pedestrian_type[i];
                let seed = self.agents.schedule_seed[i];
                let walk_cycle = (self.agents.walk_phase[i] + walker_step_beat(seed)).fract();
                let world_y = pedestrian_lane_surface_y.unwrap_or_else(|| {
                    self.get_world_surface_height_internal(Vector2::new(world_x, world_z))
                }) + AGENT_SURFACE_CLEARANCE_M;
                if eye.is_some_and(|eye| eye.excludes(world_x, Some(world_y), world_z, false)) {
                    continue;
                }
                let forward = lane_pose
                    .map(|(_, tangent)| tangent)
                    .or_else(|| access_phase_direction(self, i, world_x, world_z));
                let basis = forward.map(model_basis).unwrap_or_else(default_model_basis);
                let child = walker_is_child(self.agents.age_group[i], self.agents.household_id[i]);
                let basis = basis.map(|axis| axis * walker_size(seed, child));
                let buffer = snapshot.pedestrian_transforms.entry(p_type).or_default();
                push_transform(buffer, basis, Vector3::new(world_x, world_y, world_z));
                buffer.push(walk_cycle);
                buffer.push(walker_look(seed));
                buffer.push(0.0);
                buffer.push(0.0);
            } else {
                let lane_y = lane_pose.map(|(pos, _)| pos.y + AGENT_SURFACE_CLEARANCE_M);
                if eye.is_some_and(|eye| eye.excludes(world_x, lane_y, world_z, true)) {
                    continue;
                }
                let render_id = self.agents.render_id[i];
                let distance_sq = eye.map(|eye| eye.distance_sq(world_x, lane_y, world_z));
                let (model_key, custom) = self.drawn_car(i, distance_sq, path);
                snapshot
                    .car_custom
                    .entry(model_key)
                    .or_default()
                    .extend_from_slice(&custom);
                snapshot
                    .car_render_ids
                    .entry(model_key)
                    .or_default()
                    .push(render_id.min(i64::MAX as u64) as i64);
                snapshot
                    .car_ground_flags
                    .entry(model_key)
                    .or_default()
                    .push(u8::from(lane_pose.is_none()));
                let (world_y, basis) = if let Some((pos, tangent)) = lane_pose {
                    (pos.y + AGENT_SURFACE_CLEARANCE_M, model_basis(-tangent))
                } else {
                    let heading = off_lane_facing
                        .map(|direction| -direction)
                        .unwrap_or(Vector3::BACK);
                    (0.0, model_basis(heading))
                };
                let buffer = snapshot.car_transforms.entry(model_key).or_default();
                push_transform(buffer, basis, Vector3::new(world_x, world_y, world_z));
            }
        }
        self.ground_off_lane_cars(snapshot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::economy::agents::MODE_WALK;
    use crate::simulation::network::lanes::{Lane, LaneType};

    // The retained lane buckets name agents by index. A demolition that removes agents between
    // two ticks leaves them stale, and a snapshot drawn before the next tick read an entry past
    // the end of the agents (a panic in the render snapshot). A walker keeps its own side then.
    #[test]
    fn a_walker_keeps_its_own_side_while_the_lane_buckets_are_stale() {
        let mut lanes = LaneSystem::new();
        lanes.lanes.push(Lane {
            lane_type: LaneType::Foot,
            length: 100.0,
            ..Default::default()
        });
        let mut agents = AgentSystem::new();
        let i = agents.spawn_border_arrival_agent(usize::MAX, 0, 0.0, 0.0);
        agents.transit[i] = TRANSIT_NETWORK;
        agents.transit_mode[i] = MODE_WALK;
        agents.current_lane_id[i] = 0;
        agents.lane_distance[i] = 50.0;
        // The walker and one 2 m ahead that has since been removed with every agent after it.
        agents.lane_buckets = vec![vec![(50.0, i, 0.25), (52.0, 138, 0.25)]];
        agents.lane_bucket_snapshot_valid = false;
        let own = walker_side_offset_m(agents.schedule_seed[i]);
        assert_eq!(walker_side_m(&agents, &lanes, i, 0, 0.0), own);
        assert_eq!(walker_side_m(&agents, &lanes, i, 0, 0.5), own);
    }
}
