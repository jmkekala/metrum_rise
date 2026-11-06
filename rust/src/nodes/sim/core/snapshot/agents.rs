// SPDX-License-Identifier: GPL-2.0-only

//! Agent instances of the render snapshot: camera cull, pose and buckets (`SIM-02-K`).
//!
//! Posing an agent samples its lane; at 100k about 45,700 agents are visible and about 4,050
//! are inside the camera. The cull therefore runs before posing, on bounds that contain the
//! pose the snapshot computes:
//!
//! - An agent without a valid lane is posed at its stored position, which the cull tests
//!   exactly.
//! - An agent on a lane is posed on that lane's geometry, or, while a car changes lanes, on a
//!   blend of the source and target lane at the same distance. Either pose lies in the planar
//!   box of those lanes' points ([`LaneSystem::planar_bounds`]), widened by
//!   [`CULL_ROUNDING_MARGIN_M`] for interpolation rounding. Lanes the box cannot bound, and
//!   non-finite lane distances, are posed.
//!
//! The stored position cannot bound a lane agent's pose by any fixed margin: overlap
//! correction moves a car back along its lane without rewriting it, by up to one car spacing
//! per stacked car, and an agent that waits for a replan keeps it until it moves again; a
//! walker that has just left its door is stored at the curb while its lane distance is already
//! on the sidewalk. Daily happiness reads the stored position, so the snapshot leaves it alone.
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
    model_basis, pedestrian_lane_surface_height, transform_floats,
};
use crate::nodes::sim::core::state::SimCore;
use crate::nodes::sim::render::lane_pose::{sample_lane_change_pose, sample_lane_pose};
use crate::simulation::economy::agents::{MODE_CAR, TRANSIT_NETWORK, transit_is_visible};
use crate::simulation::network::lanes::LanePlanarBox;
use godot::prelude::{Vector2, Vector3};

// Agents per posing task: about 70 tasks at 100k, enough to balance eight workers.
const POSE_CHUNK_AGENTS: usize = 2_048;

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
}

/// Per-chunk instance buffers kept across snapshots, so steady-state posing allocates nothing.
#[derive(Default)]
pub(crate) struct AgentPoseScratch {
    chunks: Vec<Vec<PosedAgent>>,
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

fn union(a: LanePlanarBox, b: LanePlanarBox) -> LanePlanarBox {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

impl SimCore {
    // `None` while the camera box is the default "show all".
    fn snapshot_camera(&self) -> Option<Camera> {
        let (x_min, x_max, z_min, z_max) = self.camera_aabb;
        (x_min < x_max).then_some(Camera {
            x_min,
            x_max,
            z_min,
            z_max,
        })
    }

    /// Fills the cleared agent buckets of `snapshot` and grounds its off-lane cars.
    ///
    /// O(agents) cheap cull tests and O(in-camera agents) poses, both on the shared pool, plus
    /// O(lane points) when the lane revision changed.
    pub(super) fn build_agent_instances(&mut self, snapshot: &mut RenderSnapshot) {
        let camera = self.snapshot_camera();
        if camera.is_some() {
            self.transit_network.lane_system.refresh_planar_bounds();
        }
        let mut scratch = std::mem::take(&mut self.snapshot_agent_scratch);
        let core = &*self;
        let agent_count = core.agents.len();
        scratch
            .chunks
            .resize_with(agent_count.div_ceil(POSE_CHUNK_AGENTS), Vec::new);
        scratch
            .chunks
            .par_iter_mut()
            .enumerate()
            .for_each(|(chunk, posed)| {
                posed.clear();
                let start = chunk * POSE_CHUNK_AGENTS;
                let end = (start + POSE_CHUNK_AGENTS).min(agent_count);
                posed.extend((start..end).filter_map(|i| core.cull_and_pose_agent(i, camera)));
            });
        bucket_posed_agents(&scratch.chunks, snapshot);
        self.snapshot_agent_scratch = scratch;
        self.ground_off_lane_cars(snapshot);
    }

    fn cull_and_pose_agent(&self, i: usize, camera: Option<Camera>) -> Option<PosedAgent> {
        if !transit_is_visible(self.agents.transit[i]) {
            return None;
        }
        if let Some(camera) = camera
            && !self.pose_may_be_inside(i, camera)
        {
            return None;
        }
        self.pose_agent(i, camera)
    }

    // False only when agent `i`'s pose cannot pass the camera test (module docs). O(1).
    fn pose_may_be_inside(&self, i: usize, camera: Camera) -> bool {
        let boxes = self.transit_network.lane_system.planar_bounds();
        let Some(&lane_box) = boxes.get(self.agents.current_lane_id[i]) else {
            return !camera.excludes(self.agents.pos_x[i], self.agents.pos_y[i]);
        };
        if !self.agents.lane_distance[i].is_finite() {
            return true;
        }
        let mut bounds = lane_box;
        // A superset of the lane-change blend's condition in `pose_agent`.
        if self.agents.transit_mode[i] == MODE_CAR
            && self.agents.transit[i] == TRANSIT_NETWORK
            && let Some(&source_box) = boxes.get(self.agents.lane_change_from_lane_id[i] as usize)
        {
            if !self.agents.lane_change_start_d[i].is_finite() {
                return true;
            }
            bounds = union(bounds, source_box);
        }
        !camera.misses_box(bounds)
    }

    // Agent `i`'s instance, or `None` when the camera excludes it. The pose of the full-pose
    // loop, unchanged.
    fn pose_agent(&self, i: usize, camera: Option<Camera>) -> Option<PosedAgent> {
        let mut world_x = self.agents.pos_x[i];
        let mut world_z = self.agents.pos_y[i];
        let mut lane_pose = None;
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
            if let Some((pos, _)) = lane_pose {
                world_x = pos.x;
                world_z = pos.z;
                pedestrian_lane_surface_y = Some(pedestrian_lane_surface_height(lane, pos.y));
            }
        }

        if camera.is_some_and(|camera| camera.excludes(world_x, world_z)) {
            return None;
        }
        // Lane geometry owns network height. Both off-lane modes use the same current
        // road/pad/CDT ownership query in ingress and egress, including cut-away terrain.
        if self.agents.transit_mode[i] != MODE_CAR {
            let world_y = pedestrian_lane_surface_y.unwrap_or_else(|| {
                self.get_world_surface_height_internal(Vector2::new(world_x, world_z))
            }) + AGENT_SURFACE_CLEARANCE_M;
            let forward = lane_pose
                .map(|(_, tangent)| tangent)
                .or_else(|| access_phase_direction(self, i, world_x, world_z));
            // GLTF export converts Blender -Y (character facing) to +Z, so the
            // model faces +Z in Godot. basis_z = fwd aligns +Z with travel dir.
            let basis = forward.map(model_basis).unwrap_or_else(default_model_basis);
            Some(PosedAgent {
                key: self.agents.pedestrian_type[i],
                car: false,
                ground_flag: 0,
                render_id: 0,
                transform: transform_floats(basis, Vector3::new(world_x, world_y, world_z)),
                walk_phase: self.agents.walk_phase[i],
            })
        } else {
            // Off-lane footprints are grounded per bucket after bucketing.
            let render_id = self.agents.render_id[i];
            let (v_type, variant_id, _) = self.parking.vehicle_visual(self.agents.parking_trip[i].vehicle)
                .unwrap_or((self.agents.vehicle_type[i], (render_id % 5) as u8, render_id));
            let (world_y, basis) = if let Some((pos, tangent)) = lane_pose {
                (pos.y + AGENT_SURFACE_CLEARANCE_M, model_basis(-tangent))
            } else {
                let heading = access_phase_direction(self, i, world_x, world_z)
                    .map(|direction| -direction)
                    .unwrap_or(Vector3::BACK);
                (0.0, model_basis(heading))
            };
            Some(PosedAgent {
                key: (v_type * 10) + variant_id,
                car: true,
                ground_flag: u8::from(lane_pose.is_none()),
                render_id: render_id.min(i64::MAX as u64) as i64,
                transform: transform_floats(basis, Vector3::new(world_x, world_y, world_z)),
                walk_phase: 0.0,
            })
        }
    }

    // Grounds each car bucket's off-lane instances; buckets are independent.
    fn ground_off_lane_cars(&self, snapshot: &mut RenderSnapshot) {
        let flags = &snapshot.car_ground_flags;
        par_for_each_bucket(&mut snapshot.car_transforms, |key, buffer| {
            self.solve_vehicle_transforms(
                key / 10,
                buffer,
                &flags[&key],
                self.render_camera_position,
            );
        });
    }
}

// Appends every posed instance to its bucket, each bucket on its own task in chunk order.
// Buckets are created for the keys present, as the serial fill created them.
fn bucket_posed_agents(chunks: &[Vec<PosedAgent>], snapshot: &mut RenderSnapshot) {
    let mut present = [[false; 256]; 2];
    for agent in chunks.iter().flatten() {
        present[usize::from(agent.car)][usize::from(agent.key)] = true;
    }
    for key in 0..=u8::MAX {
        if present[0][usize::from(key)] {
            snapshot.pedestrian_transforms.entry(key).or_default();
        }
        if present[1][usize::from(key)] {
            snapshot.car_transforms.entry(key).or_default();
            snapshot.car_render_ids.entry(key).or_default();
            snapshot.car_ground_flags.entry(key).or_default();
        }
    }
    let RenderSnapshot {
        pedestrian_transforms,
        car_transforms,
        car_render_ids,
        car_ground_flags,
        ..
    } = snapshot;
    rayon::join(
        || {
            fill_buckets(pedestrian_transforms, chunks, false, |buffer, agent| {
                buffer.extend_from_slice(&agent.transform);
                // Walk phase in CUSTOM_DATA0.x (the MultiMesh uses custom data).
                buffer.extend_from_slice(&[agent.walk_phase, 0.0, 0.0, 0.0]);
            })
        },
        || {
            rayon::join(
                || {
                    fill_buckets(car_transforms, chunks, true, |buffer, agent| {
                        buffer.extend_from_slice(&agent.transform)
                    })
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
    chunks: &[Vec<PosedAgent>],
    cars: bool,
    push: impl Fn(&mut Vec<T>, &PosedAgent) + Sync,
) {
    par_for_each_bucket(buckets, |key, bucket| {
        for agent in chunks.iter().flatten() {
            if agent.car == cars && agent.key == key {
                push(bucket, agent);
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
        for ids in snapshot.car_render_ids.values_mut() {
            ids.clear();
        }
        for flags in snapshot.car_ground_flags.values_mut() {
            flags.clear();
        }

        let (aabb_x_min, aabb_x_max, aabb_z_min, aabb_z_max) = self.camera_aabb;
        let cull = aabb_x_min < aabb_x_max; // false when default "show all"

        for i in 0..self.agents.len() {
            if !transit_is_visible(self.agents.transit[i]) {
                continue;
            }

            let mut world_x = self.agents.pos_x[i];
            let mut world_z = self.agents.pos_y[i];
            let mut lane_pose = None;
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
                if let Some((pos, _)) = lane_pose {
                    world_x = pos.x;
                    world_z = pos.z;
                    pedestrian_lane_surface_y = Some(pedestrian_lane_surface_height(lane, pos.y));
                }
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
                let walk_cycle = self.agents.walk_phase[i];
                let world_y = pedestrian_lane_surface_y.unwrap_or_else(|| {
                    self.get_world_surface_height_internal(Vector2::new(world_x, world_z))
                }) + AGENT_SURFACE_CLEARANCE_M;
                let forward = lane_pose
                    .map(|(_, tangent)| tangent)
                    .or_else(|| access_phase_direction(self, i, world_x, world_z));
                let basis = forward.map(model_basis).unwrap_or_else(default_model_basis);
                let buffer = snapshot.pedestrian_transforms.entry(p_type).or_default();
                push_transform(buffer, basis, Vector3::new(world_x, world_y, world_z));
                buffer.push(walk_cycle);
                buffer.push(0.0);
                buffer.push(0.0);
                buffer.push(0.0);
            } else {
                let render_id = self.agents.render_id[i];
                let (v_type, variant_id, _) = self.parking.vehicle_visual(self.agents.parking_trip[i].vehicle)
                    .unwrap_or((self.agents.vehicle_type[i], (render_id % 5) as u8, render_id));
                let model_key = (v_type * 10) + variant_id;
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
                    let heading = access_phase_direction(self, i, world_x, world_z)
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
