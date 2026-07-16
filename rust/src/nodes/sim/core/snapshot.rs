//! Undo and render snapshots produced from authoritative simulation state.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use super::state::{PendingDemandSpawnAction, SimCore};
use super::terrain_payloads::{CachedRefinedTerrainPatch, RefinedTerrainPatchCacheKey};
use crate::config::HEIGHT_SCALE;
use crate::nodes::sim::render::lane_pose::sample_lane_pose;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::{AgentSystem, MODE_CAR, transit_is_visible};
use crate::simulation::economy::households::HouseholdSystem;
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::network::lanes::{Lane, LaneType};
use crate::simulation::network::surface::{CURB_STEP_HEIGHT_M, RoadSurfaceSystem};
use crate::simulation::zoning::ZoningSystem;
use godot::prelude::{Vector2, Vector3};

const PEDESTRIAN_SURFACE_CLEARANCE_M: f32 = 0.02;

fn access_phase_target(core: &SimCore, agent_idx: usize, egress: bool) -> Option<Vector3> {
    let building_id = if egress {
        core.agents.current_building[agent_idx]
    } else {
        core.agents.target_building[agent_idx]
    };
    let entrance = core.allocator.entrances.get(building_id)?;
    if egress {
        if core.agents.transit_mode[agent_idx] == MODE_CAR {
            let lane_id = core.agents.planned_attach_lane_id[agent_idx] as usize;
            let lane_d = core.agents.planned_attach_lane_d[agent_idx];
            let lane = core.transit_network.lane_system.lanes.get(lane_id)?;
            let lane_pos = BuildingAllocator::sample_pos_on_lane(lane, lane_d);
            Some(Vector3::new(lane_pos.x, 0.0, lane_pos.y))
        } else {
            Some(Vector3::new(entrance.curb_pos.x, 0.0, entrance.curb_pos.y))
        }
    } else {
        Some(Vector3::new(entrance.door_pos.x, 0.0, entrance.door_pos.y))
    }
}

fn access_phase_direction(
    core: &SimCore,
    agent_idx: usize,
    world_x: f32,
    world_z: f32,
) -> Option<Vector3> {
    use crate::simulation::economy::agents::{TRANSIT_ACCESS_EGRESS, TRANSIT_ACCESS_INGRESS};

    let target = match core.agents.transit[agent_idx] {
        TRANSIT_ACCESS_EGRESS => access_phase_target(core, agent_idx, true),
        TRANSIT_ACCESS_INGRESS => access_phase_target(core, agent_idx, false),
        _ => None,
    }?;
    let direction = Vector3::new(target.x - world_x, 0.0, target.z - world_z);
    (direction.length_squared() > 1e-6).then(|| direction.normalized())
}

fn model_basis(basis_z: Vector3) -> [Vector3; 3] {
    let mut basis_x = Vector3::RIGHT;
    let mut basis_y = Vector3::UP;
    let right = Vector3::UP.cross(basis_z);
    if right.length_squared() > 1e-6 {
        basis_x = right.normalized();
        basis_y = basis_z.cross(basis_x).normalized();
    }
    [basis_x, basis_y, basis_z]
}

fn default_model_basis() -> [Vector3; 3] {
    [Vector3::RIGHT, Vector3::UP, Vector3::BACK]
}

fn push_transform(buffer: &mut Vec<f32>, basis: [Vector3; 3], origin: Vector3) {
    let [basis_x, basis_y, basis_z] = basis;
    buffer.extend_from_slice(&[
        basis_x.x, basis_y.x, basis_z.x, origin.x, basis_x.y, basis_y.y, basis_z.y, origin.y,
        basis_x.z, basis_y.z, basis_z.z, origin.z,
    ]);
}

pub(super) fn pedestrian_lane_surface_height(lane: &Lane, lane_y: f32) -> f32 {
    if lane.lane_type == LaneType::Foot
        && lane.edge_id != usize::MAX
        && lane.lane_idx.unsigned_abs() == 100
    {
        lane_y + CURB_STEP_HEIGHT_M
    } else {
        lane_y
    }
}

pub(super) fn pedestrian_needs_access_surface(transit: u8) -> bool {
    use crate::simulation::economy::agents::{TRANSIT_ACCESS_EGRESS, TRANSIT_ACCESS_INGRESS};

    transit == TRANSIT_ACCESS_EGRESS || transit == TRANSIT_ACCESS_INGRESS
}

fn pedestrian_access_surface_height(core: &SimCore, world_x: f32, world_z: f32) -> f32 {
    core.transit_network
        .road_surface
        .sample_visible_surface_height(&core.region_graph, &core.heightmap, world_x, world_z)
        .or_else(|| {
            core.allocator
                .sample_building_site_height(Vector2::new(world_x, world_z))
        })
        .unwrap_or_else(|| {
            core.heightmap.sample_visual_height_world(world_x, world_z) * HEIGHT_SCALE
        })
}

/// Full water runtime snapshot for undo history.
pub(crate) struct WaterRuntimeSnapshot {
    /// Flat authored or loaded baseline water depth above terrain.
    pub baseline_depth: Vec<f32>,
}

/// Derived road render state that matches an undo graph snapshot.
pub(crate) struct NetworkRenderRuntimeSnapshot {
    /// Compiled road-surface cache for the snapped road graph.
    pub(crate) road_surface: RoadSurfaceSystem,
    /// Refined terrain patches prepared for road-locked render patches.
    pub(crate) refined_terrain_patch_cache:
        HashMap<RefinedTerrainPatchCacheKey, CachedRefinedTerrainPatch>,
    /// Render patches that were road-locked when the snapshot was captured.
    pub(crate) road_locked_terrain_patch_keys: Vec<(usize, usize)>,
}

/// Building and economy runtime state that must move together for entity deletion undo.
pub(crate) struct SimulationRuntimeSnapshot {
    /// Building allocator, indices, derived site data, and occupancy-facing metadata.
    pub(crate) allocator: BuildingAllocator,
    /// Live agent SoA state after lifecycle eviction/remapping.
    pub(crate) agents: AgentSystem,
    /// Household records that reference building indices.
    pub(crate) households: HouseholdSystem,
    /// Freight reservations and shipment state that reference buildings.
    pub(crate) logistics: ShipmentSystem,
    /// Delayed demand spawns that can later mutate allocator and zoning state.
    pub(crate) pending_demand_spawns: VecDeque<PendingDemandSpawnAction>,
}

/// A snapshot of simulation state for undo history.
pub(crate) struct SimulationSnapshot {
    /// Terrain heightmap data.
    pub(crate) terrain: Option<Vec<f32>>,
    /// Water runtime state.
    pub(crate) water: Option<WaterRuntimeSnapshot>,
    /// Road network graph state.
    pub(crate) trans_graph: Option<crate::simulation::network::graph::RegionGraph>,
    /// Derived road render state matching `trans_graph`.
    pub(crate) network_render: Option<NetworkRenderRuntimeSnapshot>,
    /// Zoning system state.
    pub(crate) zoning: Option<ZoningSystem>,
    /// Building/economy runtime state.
    pub(crate) runtime: Option<SimulationRuntimeSnapshot>,
}

impl SimCore {
    fn network_node_positions_snapshot(&mut self) -> Arc<Vec<Vector3>> {
        if self.cached_network_node_positions_dirty {
            self.cached_network_node_positions = Arc::new(self.build_network_node_positions());
            self.cached_network_node_positions_dirty = false;
        }
        Arc::clone(&self.cached_network_node_positions)
    }

    fn build_network_node_positions(&self) -> Vec<Vector3> {
        self.region_graph
            .nodes()
            .iter()
            .enumerate()
            .filter(|(i, _)| {
                let node_id = *i as u32;
                self.region_graph.get_valid_node(node_id) == node_id
                    && self.region_graph.node_has_live_incident_edge(node_id)
            })
            .map(|(_, n)| n.pos)
            .collect()
    }
}

/// Pre-computed rendering data written by the sim thread and read by the render thread.
///
/// Contains only pure Rust types so the struct is `Send + Sync` without unsafe.
/// The Godot main thread converts these `Vec<f32>` buffers to `PackedFloat32Array`
/// when the `#[func]` render getters are called.
pub struct RenderSnapshot {
    /// Per `pedestrian_type` → flat 12-float `Transform3D` buffer.
    pub pedestrian_transforms: HashMap<u8, Vec<f32>>,
    /// Per `(vehicle_type * 10 + color_variant)` → flat 12-float `Transform3D` buffer.
    pub car_transforms: HashMap<u8, Vec<f32>>,
    /// Per car transform bucket → render IDs matching `car_transforms` instance order.
    pub car_render_ids: HashMap<u8, Vec<i64>>,
    /// Mirrors `SimCore::terrain_dirty` at snapshot time.
    pub terrain_dirty: bool,
    /// Mirrors `SimCore::water_dirty` at snapshot time.
    pub water_dirty: bool,
    /// Mirrors `SimCore::network_dirty` at snapshot time; cleared the same frame.
    pub network_dirty: bool,
    /// Current simulation day.
    pub current_day: u32,
    /// Current minute since operational midnight.
    pub current_minute_of_day: u16,
    /// Duration of the last daily tick in milliseconds.
    pub last_tick_ms: f64,
    /// Duration of the last agent tick in microseconds.
    pub last_agent_tick_us: u64,
    /// Number of CCH pathfinding calls since the last daily tick reset.
    pub pathfind_count: u32,
    /// Total number of live agents.
    pub agent_count: i32,
    /// Current city treasury balance in currency units.
    pub treasury_balance: f64,
    /// Heightmap width in cells (for CSV logging on the main thread).
    pub heightmap_width: usize,
    /// Heightmap height in cells (for CSV logging on the main thread).
    pub heightmap_height: usize,
    /// Terrain world extent in metres, cached so Godot tools do not lock `SimCore` per frame.
    pub terrain_world_size: godot::prelude::Vector2,
    /// Revision of zoning overlay-visible parcel geometry and zoning profiles.
    pub zoning_overlay_revision: u64,
    /// Revision of zoning occupancy that affects overlay parcel coloring.
    pub zoning_overlay_occupancy_revision: u64,
    /// World-space positions of all live canonical network nodes.
    /// Pre-computed here so `get_network_nodes()` reads the snapshot (RwLock)
    /// instead of locking SimCore — avoids main-thread stalls during road placement.
    pub node_positions: Arc<Vec<godot::prelude::Vector3>>,
}

impl Default for RenderSnapshot {
    fn default() -> Self {
        Self {
            pedestrian_transforms: HashMap::new(),
            car_transforms: HashMap::new(),
            car_render_ids: HashMap::new(),
            terrain_dirty: true,
            water_dirty: true,
            network_dirty: false,
            current_day: 1,
            current_minute_of_day: 0,
            last_tick_ms: 0.0,
            last_agent_tick_us: 0,
            pathfind_count: 0,
            agent_count: 0,
            treasury_balance: 0.0,
            heightmap_width: 0,
            terrain_world_size: godot::prelude::Vector2::ZERO,
            zoning_overlay_revision: 0,
            zoning_overlay_occupancy_revision: 0,
            node_positions: Arc::new(Vec::new()),
            heightmap_height: 0,
        }
    }
}

impl SimCore {
    /// Pre-computes all per-frame rendering data into a `RenderSnapshot`.
    ///
    /// Called from the background thread at the end of every movement tick.
    /// Uses only pure Rust types so the resulting snapshot is `Send`.
    pub fn build_snapshot(&mut self) -> RenderSnapshot {
        self.build_snapshot_reusing(RenderSnapshot::default())
    }

    pub(super) fn build_snapshot_reusing(
        &mut self,
        mut snapshot: RenderSnapshot,
    ) -> RenderSnapshot {
        for buffer in snapshot.pedestrian_transforms.values_mut() {
            buffer.clear();
        }
        for buffer in snapshot.car_transforms.values_mut() {
            buffer.clear();
        }
        for ids in snapshot.car_render_ids.values_mut() {
            ids.clear();
        }

        let (aabb_x_min, aabb_x_max, aabb_z_min, aabb_z_max) = self.camera_aabb;
        let cull = aabb_x_min < aabb_x_max; // false when default "show all"

        // Stable per-bucket instance order is the interpolation identity contract; parallel
        // folds would require sorting or merging every frame and reintroduce allocations.
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
                lane_pose = sample_lane_pose(lane, self.agents.lane_distance[i]);
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
                // Pedestrian / walker — use variant MMI and oriented basis.
                let p_type = self.agents.pedestrian_type[i];
                let walk_cycle = self.agents.walk_phase[i];
                let world_y = pedestrian_lane_surface_y.unwrap_or_else(|| {
                    if pedestrian_needs_access_surface(self.agents.transit[i]) {
                        // Door-to-curb walkers are off-lane; keep this point query allocation-free.
                        pedestrian_access_surface_height(self, world_x, world_z)
                    } else {
                        self.heightmap.sample_visual_height_world(world_x, world_z) * HEIGHT_SCALE
                    }
                }) + PEDESTRIAN_SURFACE_CLEARANCE_M;
                let forward = lane_pose
                    .map(|(_, tangent)| tangent)
                    .or_else(|| access_phase_direction(self, i, world_x, world_z));
                // GLTF export converts Blender -Y (character facing) to +Z, so the
                // model faces +Z in Godot. basis_z = fwd aligns +Z with travel dir.
                let basis = forward.map(model_basis).unwrap_or_else(default_model_basis);
                let buffer = snapshot.pedestrian_transforms.entry(p_type).or_default();
                push_transform(buffer, basis, Vector3::new(world_x, world_y, world_z));

                // Add walk_phase in CUSTOM_DATA0.x (requires MultiMesh use_custom_data = true)
                buffer.push(walk_cycle);
                buffer.push(0.0);
                buffer.push(0.0);
                buffer.push(0.0);
            } else {
                // Car — oriented along lane geometry.
                let v_type = self.agents.vehicle_type[i];
                let render_id = self.agents.render_id[i];
                let variant_id = (render_id % 5) as u8;
                let model_key = (v_type * 10) + variant_id;
                snapshot
                    .car_render_ids
                    .entry(model_key)
                    .or_default()
                    .push(render_id.min(i64::MAX as u64) as i64);
                let (world_y, forward) = if let Some((pos, tangent)) = lane_pose {
                    (pos.y + 0.02, Some(-tangent))
                } else {
                    let terrain_y =
                        self.heightmap.sample_height_world(world_x, world_z) * HEIGHT_SCALE;
                    let forward = access_phase_direction(self, i, world_x, world_z).map(|v| -v);
                    (terrain_y + 0.02, forward)
                };
                let basis = forward.map(model_basis).unwrap_or_else(default_model_basis);
                let buffer = snapshot.car_transforms.entry(model_key).or_default();
                push_transform(buffer, basis, Vector3::new(world_x, world_y, world_z));
            }
        }

        let node_positions = self.network_node_positions_snapshot();

        let (terrain_world_w, terrain_world_h) = self.heightmap.world_size();

        snapshot.terrain_dirty = self.terrain_dirty;
        snapshot.water_dirty = self.water_dirty;
        snapshot.network_dirty = self.network_dirty;
        snapshot.node_positions = node_positions;
        snapshot.current_day = self.time.day_index;
        snapshot.current_minute_of_day = self.time.minute_of_day;
        snapshot.last_tick_ms = self.last_tick_duration;
        snapshot.last_agent_tick_us = self.last_agent_tick_us;
        snapshot.pathfind_count = self
            .agents
            .pathfind_count
            .load(std::sync::atomic::Ordering::Relaxed);
        snapshot.agent_count = self.agents.len() as i32;
        snapshot.treasury_balance = self.treasury.balance;
        snapshot.heightmap_width = self.heightmap.width;
        snapshot.heightmap_height = self.heightmap.height;
        snapshot.terrain_world_size =
            godot::prelude::Vector2::new(terrain_world_w, terrain_world_h);
        snapshot.zoning_overlay_revision = self.zoning.overlay_revision();
        snapshot.zoning_overlay_occupancy_revision = self.zoning.overlay_occupancy_revision();
        snapshot
    }
}
