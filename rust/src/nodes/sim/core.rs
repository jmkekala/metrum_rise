//! Background simulation thread, `SimCore` state bundle, and `RenderSnapshot`.
//!
//! `SimCore` owns all simulation state. The background thread continuously ticks
//! it at ~60 Hz, writes a `RenderSnapshot` after every tick, and never touches
//! Godot objects. The Godot main thread reads only from the snapshot for rendering
//! and locks the `Arc<Mutex<SimCore>>` briefly for mutations (road edits, etc.).

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use crate::debug_log;
use godot::prelude::{Vector3, godot_error};

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::config::MapConfig;
use crate::simulation::core::time::TimeSystem;
use crate::simulation::economy::agents::{AgentSystem, MODE_CAR, transit_is_visible};
use crate::simulation::economy::demand::DemandSystem;
use crate::simulation::economy::households::HouseholdSystem;
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::grid::desirability::DesirabilitySystem;
use crate::simulation::grid::noise::NoiseSystem;
use crate::simulation::grid::pollution::PollutionSystem;
use crate::simulation::grid::zoning::ZoningSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::water::WaterSystem;

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

/// A snapshot of simulation state for undo history.
pub struct SimulationSnapshot {
    /// Terrain heightmap data.
    pub terrain: Option<Vec<f32>>,
    /// Water depth data.
    pub water: Option<Vec<f32>>,
    /// Road network graph state.
    pub trans_graph: Option<crate::simulation::network::graph::RegionGraph>,
    /// Zoning system state.
    pub zoning: Option<ZoningSystem>,
}

/// All simulation state — owned exclusively by the background sim thread when running.
///
/// The main thread accesses this via `Arc<Mutex<SimCore>>`. The lock is held for at
/// most one tick duration (~7 ms at 100 k agents) per mutation.
pub struct SimCore {
    /// Simulation clock and day counter.
    pub time: TimeSystem,
    /// Terrain heightmap.
    pub heightmap: TerrainSystem,
    /// Shallow-water simulation.
    pub watermap: WaterSystem,
    /// Road topology graph.
    pub region_graph: crate::simulation::network::graph::RegionGraph,
    /// Lane system, CCH pathfinder, and road mutation helpers.
    pub transit_network: TransitNetwork,
    /// Edge-aligned zoning grid.
    pub zoning: ZoningSystem,
    /// Pollution diffusion grid.
    pub pollution: PollutionSystem,
    /// Traffic noise grid.
    pub noise: NoiseSystem,
    /// Composite desirability grid.
    pub desirability: DesirabilitySystem,
    /// Global R/C/I demand counters.
    pub demand: DemandSystem,
    /// Building placement and vacancy index.
    pub allocator: BuildingAllocator,
    /// Agent FSM in Structure-of-Arrays layout.
    pub agents: AgentSystem,
    /// Explicit household runtime records and first-pass daily economy logic.
    pub households: HouseholdSystem,
    /// Active building-level freight reservations and delayed deliveries.
    pub logistics: ShipmentSystem,
    /// Map configuration (dimensions, cell sizes).
    pub config: MapConfig,
    /// Undo history stack — kept in SimCore so all mutations are co-located.
    pub undo_stack: VecDeque<SimulationSnapshot>,
    /// Set by terrain mutations; cleared by the Godot render layer.
    pub terrain_dirty: bool,
    /// Set by water mutations; cleared by the Godot render layer.
    pub water_dirty: bool,
    /// Set by any network mutation (road, rail); cleared by `clear_network_dirty()` after
    /// `NetworkRenderer` finishes rebuilding the visual mesh. Stays `true` until GDScript
    /// explicitly clears it — same pattern as `terrain_dirty` and `water_dirty`.
    pub network_dirty: bool,
    /// True when running in benchmark mode (skips undo stack on road placement).
    pub benchmark_mode: bool,
    /// Duration of the last daily economy tick in milliseconds.
    pub last_tick_duration: f64,
    /// Duration of the last agent movement tick in microseconds.
    pub last_agent_tick_us: u64,
    /// Per-phase timing breakdown from the last road placement, for profiling.
    pub last_road_timing: String,
    /// World-space AABB for frustum culling: (x_min, x_max, z_min, z_max).
    /// Agents outside this rect are excluded from `RenderSnapshot` transforms.
    /// Updated each frame via `SimCommand::SetCameraAabb`. Defaults to "show all".
    pub camera_aabb: (f32, f32, f32, f32),
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
    /// Mirrors `SimCore::terrain_dirty` at snapshot time.
    pub terrain_dirty: bool,
    /// Mirrors `SimCore::water_dirty` at snapshot time.
    pub water_dirty: bool,
    /// Mirrors `SimCore::network_dirty` at snapshot time; cleared the same frame.
    pub network_dirty: bool,
    /// Current simulation day.
    pub current_day: u32,
    /// Duration of the last daily tick in milliseconds.
    pub last_tick_ms: f64,
    /// Duration of the last agent tick in microseconds.
    pub last_agent_tick_us: u64,
    /// Number of CCH pathfinding calls since the last daily tick reset.
    pub pathfind_count: u32,
    /// Total number of live agents.
    pub agent_count: i32,
    /// Heightmap width in cells (for CSV logging on the main thread).
    pub heightmap_width: usize,
    /// Heightmap height in cells (for CSV logging on the main thread).
    pub heightmap_height: usize,
    /// World-space positions of all canonical (non-virtual) network nodes.
    /// Pre-computed here so `get_network_nodes()` reads the snapshot (RwLock)
    /// instead of locking SimCore — avoids main-thread stalls during road placement.
    pub node_positions: Vec<godot::prelude::Vector3>,
}

impl Default for RenderSnapshot {
    fn default() -> Self {
        Self {
            pedestrian_transforms: HashMap::new(),
            car_transforms: HashMap::new(),
            terrain_dirty: true,
            water_dirty: true,
            network_dirty: false,
            current_day: 1,
            last_tick_ms: 0.0,
            last_agent_tick_us: 0,
            pathfind_count: 0,
            agent_count: 0,
            heightmap_width: 0,
            node_positions: Vec::new(),
            heightmap_height: 0,
        }
    }
}

/// Commands sent from the Godot main thread to the sim background thread.
pub enum SimCommand {
    /// Update the simulation speed multiplier.
    SetSpeed(f32),
    /// Update the camera world-space AABB used for agent frustum culling.
    /// Values: (x_min, x_max, z_min, z_max) in world units, padded by ~200 m.
    SetCameraAabb(f32, f32, f32, f32),
    /// Place a new road segment.  Executed in the sim thread so the main thread
    /// never blocks on the expensive lane-rebuild and zoning-obstruction passes.
    AddRoad {
        /// World-space polyline points.
        points: Vec<godot::prelude::Vector3>,
        /// Forward lane count.
        fwd_lanes: i32,
        /// Backward lane count.
        bkw_lanes: i32,
    },
    /// Ask the background thread to exit cleanly.
    Quit,
}

impl SimCore {
    /// Executes one full economy / daily tick (called once per in-game day).
    pub fn simulate_tick_internal(&mut self) {
        let tick_start = Instant::now();

        debug_log!(
            "economy",
            "daily tick start: buildings={} households={} agents={}",
            self.allocator.buildings.len(),
            self.households
                .households
                .iter()
                .filter(|h| h.member_count > 0)
                .count(),
            self.agents.len(),
        );
        self.allocator.tick(
            &mut self.zoning,
            &mut self.agents,
            &mut self.households,
            &mut self.logistics,
            &mut self.transit_network,
            &mut self.region_graph,
        );
        // Drain building dirty-zone flags → mark matching flow fields for rebuild.
        {
            use crate::simulation::grid::zoning::ZoneType;
            let zones = [
                ZoneType::None,
                ZoneType::Residential,
                ZoneType::Commercial,
                ZoneType::Industrial,
                ZoneType::Office,
                ZoneType::Mixed,
            ];
            for (z, zone) in zones.iter().enumerate() {
                if self.allocator.dirty_zones[z] {
                    self.allocator.dirty_zones[z] = false;
                    self.transit_network.flow_fields.mark_zone_dirty(*zone);
                }
            }
        }

        self.pollution.tick(&self.allocator, &self.config);
        self.noise
            .tick(&self.allocator, &self.region_graph, &self.config);
        self.desirability
            .tick(&self.zoning, &self.pollution, &self.noise);
        self.households.daily_tick(
            &mut self.agents,
            &mut self.allocator,
            &mut self.logistics,
            &self.transit_network,
            &self.region_graph,
        );
        self.demand
            .run_daily_pass(&self.allocator, &self.households, &self.region_graph);
        self.allocator.execute_demand_household_admission(
            self.demand.households_to_admit_today,
            &mut self.agents,
            &mut self.households,
        );
        debug_log!(
            "economy",
            "daily tick end: buildings={} households={} agents={} demand=(R {:.0}%, C {:.0}%, I {:.0}%) admit={} remove={}",
            self.allocator.buildings.len(),
            self.households
                .households
                .iter()
                .filter(|h| h.member_count > 0)
                .count(),
            self.agents.len(),
            self.demand.residential * 100.0,
            self.demand.commercial * 100.0,
            self.demand.industrial * 100.0,
            self.demand.households_to_admit_today,
            self.demand.households_to_remove_today
        );
        self.agents.daily_update(&self.pollution, &self.config);
        self.agents
            .pathfind_count
            .store(0, std::sync::atomic::Ordering::Relaxed);

        self.last_tick_duration = tick_start.elapsed().as_secs_f64() * 1000.0;
    }

    /// Pre-computes all per-frame rendering data into a `RenderSnapshot`.
    ///
    /// Called from the background thread at the end of every movement tick.
    /// Uses only pure Rust types so the resulting snapshot is `Send`.
    pub fn build_snapshot(&self) -> RenderSnapshot {
        use crate::simulation::economy::agents::{TRANSIT_ACCESS_EGRESS, TRANSIT_ACCESS_INGRESS};

        let mut pedestrian_transforms: HashMap<u8, Vec<f32>> = HashMap::new();
        let mut car_transforms: HashMap<u8, Vec<f32>> = HashMap::new();

        let w = self.heightmap.width as f32;
        let h = self.heightmap.height as f32;
        let hw = (w - 1.0) * 0.5;
        let hh = (h - 1.0) * 0.5;

        let (aabb_x_min, aabb_x_max, aabb_z_min, aabb_z_max) = self.camera_aabb;
        let cull = aabb_x_min < aabb_x_max; // false when default "show all"

        for i in 0..self.agents.len() {
            if !transit_is_visible(self.agents.transit[i]) {
                continue;
            }

            let world_x = self.agents.pos_x[i];
            let world_z = self.agents.pos_y[i];

            if cull
                && (world_x < aabb_x_min
                    || world_x > aabb_x_max
                    || world_z < aabb_z_min
                    || world_z > aabb_z_max)
            {
                continue;
            }
            let map_x = (world_x + hw).clamp(0.0, w - 1.0) as usize;
            let map_z = (world_z + hh).clamp(0.0, h - 1.0) as usize;
            let terrain_y = self.heightmap.get_height(map_x, map_z) * 20.0;

            if self.agents.transit_mode[i] != MODE_CAR {
                // Pedestrian / walker — use variant MMI and oriented basis.
                let p_type = self.agents.pedestrian_type[i];
                let walk_cycle = self.agents.walk_phase[i];
                let buffer = pedestrian_transforms.entry(p_type).or_default();

                let mut basis_x = Vector3::RIGHT;
                let mut basis_y = Vector3::UP;
                let mut basis_z = Vector3::BACK;
                let world_y = terrain_y + 0.05; // small lift so feet clear terrain surface

                let lane_id = self.agents.current_lane_id[i];
                if lane_id != usize::MAX && lane_id < self.transit_network.lane_system.lanes.len() {
                    let l = &self.transit_network.lane_system.lanes[lane_id];
                    let dist = self.agents.lane_distance[i];
                    if l.geometry.len() >= 2 && !l.cum_dist.is_empty() {
                        let seg = l.cum_dist.partition_point(|&d| d <= dist).saturating_sub(1);
                        let seg = seg.min(l.geometry.len() - 2);
                        let raw = l.geometry[seg + 1] - l.geometry[seg];
                        if raw.length_squared() > 1e-6 {
                            // GLTF export converts Blender -Y (character facing) to +Z, so the
                            // model faces +Z in Godot. basis_z = fwd aligns +Z with travel dir.
                            basis_z = raw.normalized();
                            let right = Vector3::UP.cross(basis_z);
                            if right.length_squared() > 1e-6 {
                                basis_x = right.normalized();
                                basis_y = basis_z.cross(basis_x).normalized();
                            }
                        }
                    }
                } else {
                    let transit = self.agents.transit[i];
                    if transit == TRANSIT_ACCESS_EGRESS {
                        if let Some(target) = access_phase_target(self, i, true) {
                            let dir = Vector3::new(target.x - world_x, 0.0, target.z - world_z);
                            if dir.length_squared() > 1e-6 {
                                basis_z = dir.normalized();
                                let right = Vector3::UP.cross(basis_z);
                                if right.length_squared() > 1e-6 {
                                    basis_x = right.normalized();
                                    basis_y = basis_z.cross(basis_x).normalized();
                                }
                            }
                        }
                    } else if transit == TRANSIT_ACCESS_INGRESS {
                        if let Some(target) = access_phase_target(self, i, false) {
                            let dir = Vector3::new(target.x - world_x, 0.0, target.z - world_z);
                            if dir.length_squared() > 1e-6 {
                                basis_z = dir.normalized();
                                let right = Vector3::UP.cross(basis_z);
                                if right.length_squared() > 1e-6 {
                                    basis_x = right.normalized();
                                    basis_y = basis_z.cross(basis_x).normalized();
                                }
                            }
                        }
                    }
                }

                buffer.push(basis_x.x);
                buffer.push(basis_y.x);
                buffer.push(basis_z.x);
                buffer.push(world_x);
                buffer.push(basis_x.y);
                buffer.push(basis_y.y);
                buffer.push(basis_z.y);
                buffer.push(world_y);
                buffer.push(basis_x.z);
                buffer.push(basis_y.z);
                buffer.push(basis_z.z);
                buffer.push(world_z);

                // Add walk_phase in CUSTOM_DATA0.x (requires MultiMesh use_custom_data = true)
                buffer.push(walk_cycle);
                buffer.push(0.0);
                buffer.push(0.0);
                buffer.push(0.0);
            } else {
                // Car — oriented along lane geometry.
                let v_type = self.agents.vehicle_type[i];
                let variant_id = (i % 5) as u8;
                let model_key = (v_type * 10) + variant_id;
                let buffer = car_transforms.entry(model_key).or_default();

                let mut basis_x = Vector3::RIGHT;
                let mut basis_y = Vector3::UP;
                let mut basis_z = Vector3::BACK;
                let mut world_y = terrain_y + 0.02;

                let lane_id = self.agents.current_lane_id[i];
                if lane_id != usize::MAX && lane_id < self.transit_network.lane_system.lanes.len() {
                    let l = &self.transit_network.lane_system.lanes[lane_id];
                    let dist = self.agents.lane_distance[i];
                    if l.geometry.len() >= 2 {
                        let mut curr = 0.0_f32;
                        for j in 0..l.geometry.len() - 1 {
                            let p0 = l.geometry[j];
                            let p1 = l.geometry[j + 1];
                            let d = p0.distance_to(p1);
                            if curr + d >= dist || j == l.geometry.len() - 2 {
                                let t = if d > 1e-5 { (dist - curr) / d } else { 0.0 };
                                world_y = p0.y + (p1.y - p0.y) * t.clamp(0.0, 1.0) + 0.02;
                                let raw = p1 - p0;
                                if raw.length_squared() > 1e-6 {
                                    let fwd = raw.normalized();
                                    basis_z = -fwd;
                                    let right = Vector3::UP.cross(basis_z);
                                    if right.length_squared() > 1e-6 {
                                        basis_x = right.normalized();
                                        basis_y = basis_z.cross(basis_x).normalized();
                                    }
                                }
                                break;
                            }
                            curr += d;
                        }
                    } else if !l.geometry.is_empty() {
                        world_y = l.geometry[0].y + 0.02;
                    }
                } else {
                    let transit = self.agents.transit[i];
                    if transit == TRANSIT_ACCESS_EGRESS {
                        if let Some(target) = access_phase_target(self, i, true) {
                            let dir = Vector3::new(target.x - world_x, 0.0, target.z - world_z);
                            if dir.length_squared() > 1e-6 {
                                basis_z = -dir.normalized();
                                let right = Vector3::UP.cross(basis_z);
                                if right.length_squared() > 1e-6 {
                                    basis_x = right.normalized();
                                    basis_y = basis_z.cross(basis_x).normalized();
                                }
                            }
                        }
                    } else if transit == TRANSIT_ACCESS_INGRESS {
                        if let Some(target) = access_phase_target(self, i, false) {
                            let dir = Vector3::new(target.x - world_x, 0.0, target.z - world_z);
                            if dir.length_squared() > 1e-6 {
                                basis_z = -dir.normalized();
                                let right = Vector3::UP.cross(basis_z);
                                if right.length_squared() > 1e-6 {
                                    basis_x = right.normalized();
                                    basis_y = basis_z.cross(basis_x).normalized();
                                }
                            }
                        }
                    }
                }

                buffer.push(basis_x.x);
                buffer.push(basis_y.x);
                buffer.push(basis_z.x);
                buffer.push(world_x);
                buffer.push(basis_x.y);
                buffer.push(basis_y.y);
                buffer.push(basis_z.y);
                buffer.push(world_y);
                buffer.push(basis_x.z);
                buffer.push(basis_y.z);
                buffer.push(basis_z.z);
                buffer.push(world_z);
            }
        }

        let node_positions: Vec<godot::prelude::Vector3> = self
            .region_graph
            .nodes()
            .iter()
            .enumerate()
            .filter(|(i, _)| self.region_graph.get_valid_node(*i as u32) == *i as u32)
            .map(|(_, n)| n.pos)
            .collect();

        RenderSnapshot {
            pedestrian_transforms,
            car_transforms,
            terrain_dirty: self.terrain_dirty,
            water_dirty: self.water_dirty,
            network_dirty: self.network_dirty,
            node_positions,
            current_day: self.time.current_day,
            last_tick_ms: self.last_tick_duration,
            last_agent_tick_us: self.last_agent_tick_us,
            pathfind_count: self
                .agents
                .pathfind_count
                .load(std::sync::atomic::Ordering::Relaxed),
            agent_count: self.agents.len() as i32,
            heightmap_width: self.heightmap.width,
            heightmap_height: self.heightmap.height,
        }
    }
}

/// Background simulation thread loop.
///
/// Runs at ~60 Hz, decoupled from Godot's render frame. The `core` mutex is held
/// for the duration of each movement tick; main-thread `#[func]` calls block for
/// at most one tick duration while the lock is held (~7 ms at 100 k agents).
/// After the tick the snapshot is written while the lock is *not* held, so render
/// reads are completely non-blocking.
pub fn run_sim_thread(
    core: Arc<Mutex<SimCore>>,
    snapshot: Arc<RwLock<RenderSnapshot>>,
    cmd_rx: std::sync::mpsc::Receiver<SimCommand>,
) {
    const TARGET_DT: f64 = 1.0 / 60.0;
    let target = Duration::from_micros(16_667); // ~60 Hz

    loop {
        let frame_start = Instant::now();

        // Drain all pending commands — non-blocking.
        let mut should_quit = false;
        loop {
            match cmd_rx.try_recv() {
                Ok(SimCommand::Quit) => {
                    should_quit = true;
                    break;
                }
                Ok(SimCommand::SetSpeed(s)) => {
                    core.lock().unwrap().time.speed_multiplier = s;
                }
                Ok(SimCommand::SetCameraAabb(x0, x1, z0, z1)) => {
                    core.lock().unwrap().camera_aabb = (x0, x1, z0, z1);
                }
                Ok(SimCommand::AddRoad {
                    points,
                    fwd_lanes,
                    bkw_lanes,
                }) => {
                    let road_total = Instant::now();
                    let mut c = core.lock().unwrap();
                    // Bulk-load defers per-edge rebuilds until finalization.
                    c.transit_network.bulk_load = true;
                    c.add_road_internal(points, fwd_lanes, bkw_lanes);
                    {
                        let c = &mut *c;
                        c.transit_network.bulk_load = false;

                        // Take dirty edges first so we can derive the affected nodes
                        // for the incremental clips pass.
                        let dirty = std::mem::take(&mut c.transit_network.bulk_dirty_edges);
                        let dirty_count = dirty.len();

                        // Collect nodes touched by the new/split edges.
                        let mut affected_nodes = std::collections::HashSet::new();
                        for &e_id in &dirty {
                            if e_id < c.region_graph.edge_count()
                                && !c.region_graph.edge(e_id).deleted
                            {
                                let e = c.region_graph.edge(e_id);
                                affected_nodes.insert(c.region_graph.get_valid_node(e.start_node));
                                affected_nodes.insert(c.region_graph.get_valid_node(e.end_node));
                            }
                        }

                        let t_clips = Instant::now();
                        c.region_graph
                            .rebuild_intersection_clips_for_nodes(&affected_nodes);
                        let dt_clips_us = t_clips.elapsed().as_micros();

                        let t_inv = Instant::now();
                        // Invalidate agents BEFORE lane rebuild so old lane IDs are still valid.
                        c.agents
                            .invalidate_lane_ids_for_edges(&dirty, &c.transit_network.lane_system);
                        let dt_inv_us = t_inv.elapsed().as_micros();

                        let t_lanes = Instant::now();
                        c.transit_network
                            .lane_system
                            .rebuild_edges_incremental(&mut c.region_graph, &dirty);
                        let dt_lanes_us = t_lanes.elapsed().as_micros();
                        c.allocator.rebuild_entrance_cache(
                            &c.region_graph,
                            &c.transit_network.lane_system,
                        );

                        // Zone flush is deferred to the next simulate_tick_internal call
                        // so it does not block road placement. zoning_dirty_edges accumulates.

                        let total_us = road_total.elapsed().as_micros();
                        let msg = format!(
                            "TOTAL={}µs  {}  clips={}µs  lanes={}µs({}e)  invalidate={}µs",
                            total_us,
                            c.last_road_timing,
                            dt_clips_us,
                            dt_lanes_us,
                            dirty_count,
                            dt_inv_us
                        );
                        debug_log!("road", "{}", msg);
                        c.last_road_timing = msg;
                    }
                    c.network_dirty = true;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    should_quit = true;
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
            }
        }
        if should_quit {
            return;
        }

        // Tick and build snapshot inside one lock acquisition.
        let new_snapshot = {
            // Recover from a poisoned mutex (caused by a previous tick panic) rather
            // than propagating a PoisonError cascade to every main-thread call.
            let mut core = match core.lock() {
                Ok(g) => g,
                Err(e) => {
                    godot_error!("[sim] mutex was poisoned by a previous tick panic — recovering");
                    e.into_inner()
                }
            };
            let speed = core.time.speed_multiplier;

            if speed > 0.0 {
                // Rebuild CCH if dirty, then rebuild any dirty flow fields.
                let c = &mut *core;
                c.transit_network
                    .rebuild_pathing_if_dirty(&mut c.region_graph);
                {
                    let alloc = &c.allocator;
                    let graph = &c.region_graph;
                    c.transit_network
                        .flow_fields
                        .rebuild_dirty(graph, |zone, mode_flags| {
                            alloc.get_sources_for_zone(zone, graph, mode_flags)
                        });
                }

                let dt = (TARGET_DT * speed as f64) as f32;
                let t_agent = Instant::now();

                // Wrap the tick in catch_unwind so that a panic inside the agent loop
                // does NOT poison the mutex.  The MutexGuard stays alive in the outer
                // frame, so the lock is still held across the catch boundary.
                let tick_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let c = &mut *core;
                    c.agents.tick(
                        &c.allocator,
                        &mut c.transit_network,
                        &mut c.region_graph,
                        dt,
                    );
                }));
                if let Err(e) = tick_result {
                    let msg = e
                        .downcast_ref::<&str>()
                        .copied()
                        .or_else(|| e.downcast_ref::<String>().map(String::as_str))
                        .unwrap_or("(non-string payload)");
                    godot_error!("[sim] tick panicked — skipping frame: {}", msg);
                }

                core.last_agent_tick_us = t_agent.elapsed().as_micros() as u64;

                if core.time.process_delta(TARGET_DT) {
                    // Wrap daily tick so a panic here does not poison the mutex.
                    // Without this, the first daily-tick panic would permanently
                    // crash every subsequent Godot main-thread lock().unwrap() call.
                    let daily_result =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            core.simulate_tick_internal()
                        }));
                    if let Err(e) = daily_result {
                        let msg = e
                            .downcast_ref::<&str>()
                            .copied()
                            .or_else(|| e.downcast_ref::<String>().map(String::as_str))
                            .unwrap_or("(non-string payload)");
                        godot_error!("[sim] daily tick panicked — skipping day: {}", msg);
                    }
                }
            }

            // build_snapshot only reads state; wrap anyway so a panic here does
            // not poison the mutex and kill the render thread.
            let snap_result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| core.build_snapshot()));
            match snap_result {
                Ok(s) => s,
                Err(e) => {
                    let msg = e
                        .downcast_ref::<&str>()
                        .copied()
                        .or_else(|| e.downcast_ref::<String>().map(String::as_str))
                        .unwrap_or("(non-string payload)");
                    godot_error!("[sim] build_snapshot panicked — using default: {}", msg);
                    RenderSnapshot::default()
                }
            }
        };

        // Write snapshot — outside the sim lock so render reads are non-blocking.
        *snapshot.write().unwrap() = new_snapshot;

        // Sleep to maintain ~60 Hz.
        let elapsed = frame_start.elapsed();
        if elapsed < target {
            std::thread::sleep(target - elapsed);
        }
    }
}
