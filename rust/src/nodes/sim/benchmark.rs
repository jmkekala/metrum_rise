//! Benchmarking and performance profiling for simulation.

use godot::prelude::*;
extern crate chrono;
use crate::nodes::sim::core::SimCore;
use crate::nodes::simulation_node::SimulationNode;
use std::time::Instant;

/// Reads the resident set size of this process in megabytes from `/proc/self/status`.
pub(crate) fn rss_mb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse::<u64>().ok())
        })
        .unwrap_or(0)
        / 1024
}

// ---------------------------------------------------------------------------
// SimCore methods — pure simulation operations, callable from any thread.
// ---------------------------------------------------------------------------

impl SimCore {
    /// Sets up a large-scale benchmark city with a grid of roads and agents.
    pub fn setup_benchmark_city_internal(&mut self, grid_size: i32, agent_count: i32) {
        godot_print!(
            "SimulationNode: Setting up benchmark city (Grid: {}x{}, Agents: {})...",
            grid_size,
            grid_size,
            agent_count
        );

        let t0 = Instant::now();
        let mut pts = PackedVector3Array::new();

        self.transit_network.bulk_load = true;

        let world_w = self.config.width_m;
        let world_h = self.config.height_m;
        let step = (world_w - 40.0) / grid_size as f32;
        let start_x = -world_w * 0.5 + 20.0;
        let start_z = -world_h * 0.5 + 20.0;

        // Horizontal roads
        for i in 0..=grid_size {
            pts.clear();
            pts.push(Vector3::new(start_x, 0.0, start_z + i as f32 * step));
            pts.push(Vector3::new(
                start_x + world_w - 40.0,
                0.0,
                start_z + i as f32 * step,
            ));
            self.add_road_internal(pts.to_vec(), 2, 2);
        }

        // Vertical roads
        for i in 0..=grid_size {
            pts.clear();
            pts.push(Vector3::new(start_x + i as f32 * step, 0.0, start_z));
            pts.push(Vector3::new(
                start_x + i as f32 * step,
                0.0,
                start_z + world_h - 40.0,
            ));
            self.add_road_internal(pts.to_vec(), 2, 2);
        }

        godot_print!(
            "[bench] roads built: {:.1}s  RSS {}MB  edges={} nodes={}",
            t0.elapsed().as_secs_f32(),
            rss_mb(),
            self.region_graph.edge_count(),
            self.region_graph.node_count()
        );

        let t1 = Instant::now();
        self.transit_network
            .finalize_bulk_load(&mut self.region_graph, &mut self.allocator);
        godot_print!(
            "[bench] finalize_bulk_load: {:.1}s  RSS {}MB  lanes={}",
            t1.elapsed().as_secs_f32(),
            rss_mb(),
            self.transit_network.lane_system.lanes.len()
        );

        godot_print!(
            "[bench] TOTAL setup_benchmark_city_internal: {:.1}s  RSS {}MB",
            t0.elapsed().as_secs_f32(),
            rss_mb()
        );
    }

    /// Spawns 100 k pre-pathed network agents for the benchmark run.
    ///
    /// Called from `run_benchmark_from_save` on the Godot main thread (while
    /// holding the `SimCore` lock).
    pub(crate) fn spawn_benchmark_agents(&mut self) {
        self.spawn_looping_car_traffic(100_000);
    }

    /// Puts `count` cars on the existing road graph, each looping a fixed route.
    ///
    /// Used by the benchmark and by `SimCommand::SpawnTestTraffic`, which is how
    /// a test or a tool gets moving traffic without a full economy behind it.
    /// The routes bounce back and forth so a car never runs out of path and
    /// despawns, which would otherwise empty the roads mid-measurement.
    pub(crate) fn spawn_test_traffic_internal(&mut self, count: i32) {
        if count > 0 {
            self.spawn_looping_car_traffic(count as usize);
        }
    }

    fn spawn_looping_car_traffic(&mut self, agent_count: usize) {
        use crate::config::{CAR_LENGTH, DEFAULT_URBAN_ROAD_SPEED_MS};
        use crate::simulation::economy::agents::data::Agent;
        use crate::simulation::economy::agents::{AGE_ADULT, MODE_CAR, TRANSIT_NETWORK};
        use crate::simulation::network::types::TransitFlags;

        let t_spawn = Instant::now();
        let node_count = self.region_graph.node_count();
        if node_count == 0 {
            godot_print!("[bench] ERROR: no nodes in graph, cannot spawn agents");
            return;
        }

        // One route per pair of distant nodes, capped so a large graph does not
        // pathfind hundreds of times. A small graph gets fewer, because a route
        // whose endpoints collide is skipped and would otherwise leave none.
        // Every ordered pair of distinct nodes on a small graph, so a cross gets
        // routes along both streets rather than several along one. The old
        // stride pairing produced `node_count` routes that mostly shared an
        // axis, which put all 120 cars on one road and none on the other, and
        // made a junction test measure nothing about the junction.
        // Route endpoints must be places a car can legally turn around, which
        // means a road end rather than a junction: a four-way node has no
        // U-turn connector, so a bounce route that reverses there strands the
        // car at the mouth forever with `reason=no-connection-lane`.
        let is_turnaround_node = |n: usize| -> bool {
            self.region_graph.node_adjacency_count_at(n as u32) <= 2
        };
        let mut pairs: Vec<(u32, u32)> = Vec::new();
        'outer: for s in 0..node_count {
            if !is_turnaround_node(s) {
                continue;
            }
            for e in 0..node_count {
                if s == e || !is_turnaround_node(e) {
                    continue;
                }
                pairs.push((s as u32, e as u32));
                if pairs.len() >= 200 {
                    break 'outer;
                }
            }
        }
        let mut routes: Vec<Vec<u32>> = Vec::with_capacity(pairs.len());
        let t_routes = Instant::now();
        for &(s, e) in &pairs {
            if let Some((_, _, path)) = self.transit_network.cch_graph.find_path(
                s,
                e,
                usize::MAX,
                &self.region_graph,
                TransitFlags::CAR,
            ) {
                if path.len() > 1 {
                    // Bounce the path so cars keep driving instead of reaching a
                    // destination and leaving the road empty.
                    //
                    // The reversal turns the car around at the far end, which is
                    // a U-turn. That is legal at a road end and impossible at a
                    // four-way junction, so the turnaround must land on the
                    // endpoints. `path` runs between two chosen nodes, and the
                    // reversal is appended at those endpoints only, never
                    // mid-route.
                    let mut bounce = path.clone();
                    for _ in 0..4 {
                        let mut rev = path.clone();
                        rev.reverse();
                        bounce.extend_from_slice(&rev[1..]);
                        bounce.extend_from_slice(&path[1..]);
                    }
                    routes.push(bounce);
                }
            }
        }
        godot_print!(
            "[bench] Pre-computed {} routes in {:.2}s",
            routes.len(),
            t_routes.elapsed().as_secs_f32()
        );

        if routes.is_empty() {
            godot_print!("[bench] ERROR: no routes found — check CCH and graph connectivity");
            return;
        }

        for i in 0..agent_count {
            let route_idx = i % routes.len();
            let route = routes[route_idx].clone();
            let path_len = route.len();

            // Spread agents along their own route rather than across the whole
            // fleet. Deriving the start from `i` made every agent sharing a
            // route land on the same index, so cars spawned inside each other
            // and stayed stacked.
            let nth_on_route = i / routes.len();
            let per_route = agent_count.div_ceil(routes.len()).max(1);
            let start_idx =
                (nth_on_route * path_len / per_route).min(path_len.saturating_sub(2));
            let current_node = route[start_idx];
            let node_pos = self.region_graph.node(current_node).pos;
            let render_id = self.agents.allocate_render_id();
            self.agents.agents.push(Agent {
                home_building: usize::MAX,
                household_id: usize::MAX,
                age_group: AGE_ADULT,
                pending_household_size: 0,
                freight_shipment_id: u64::MAX,
                work_building: usize::MAX,
                pos_x: node_pos.x,
                pos_y: node_pos.z,
                render_id,
                activity: 0,
                transit: TRANSIT_NETWORK,
                happiness: 50.0,
                money: 100.0,
                journey_start_time: 0.0,
                current_building: usize::MAX,
                target_building: usize::MAX,
                planned_target_building: usize::MAX,
                freight_target_border_node: u32::MAX,
                current_node,
                planned_attach_node: u32::MAX,
                planned_detach_node: u32::MAX,
                planned_attach_lane_id: u32::MAX,
                planned_detach_lane_id: u32::MAX,
                planned_attach_lane_d: 0.0,
                planned_detach_lane_d: 0.0,
                access_flags: 0,
                next_replan_time: 0.0,
                network_replan_failures: 0,
                current_edge: usize::MAX,
                current_lane_id: usize::MAX,
                // Staggered so cars sharing a route segment do not all attach at
                // the same point. Lane attachment clamps this to the lane, and
                // a car whose offset exceeds the segment simply starts at its
                // end rather than inside the car ahead.
                lane_distance: (nth_on_route % 8) as f32 * (CAR_LENGTH * 2.0),
                lane_change_from_lane_id: u32::MAX,
                lane_change_start_d: 0.0,
                lane_change_length_m: 0.0,
                overtake_blocked_time_s: 0.0,
                overtake_cooldown_s: 0.0,
                junction_release_time_s: f32::MIN,
                next_reroute_time_s: 0.0,
                speed: DEFAULT_URBAN_ROAD_SPEED_MS,
                transit_mode: MODE_CAR,
                planned_activity: 0,
                current_path: route,
                current_path_index: (start_idx + 1).min(path_len - 1),
                has_car: true,
                vehicle_type: (i % 4) as u8,
                pedestrian_type: 0,
                walk_phase: 0.0,
                schedule_seed: i as u32,
                cached_commute_minutes: 0,
                next_commute_refresh_time: 0.0,
                next_departure_day: u32::MAX,
                next_departure_minute: 0,
                next_departure_origin_building: usize::MAX,
                next_departure_target_building: usize::MAX,
                next_departure_activity: 0,
                cached_schedule_work_building: usize::MAX,
                cached_work_profile_index: u16::MAX,
                job_lock_days: 0,
                consecutive_unpaid_days: 0,
            });
        }
        godot_print!(
            "[bench] Spawned {} agents (NETWORK): {:.2}s  RSS {}MB",
            self.agents.len(),
            t_spawn.elapsed().as_secs_f32(),
            rss_mb()
        );

        self.time.speed_multiplier = 1.0;
    }
}

// ---------------------------------------------------------------------------
// SimulationNode methods — need Godot base access or read from snapshot.
// ---------------------------------------------------------------------------

impl SimulationNode {
    /// Builds the 20 km × 20 km benchmark city, saves it to `benchmark.sav`, then exits.
    ///
    /// Run once with `./run.sh --generate-benchmark --headless`.
    pub fn generate_benchmark_map(&mut self) {
        let t0 = Instant::now();
        godot_print!("[gen] Starting benchmark map generation (20×20 grid, 20 km map)");

        {
            let mut core = self.core.lock().unwrap();
            core.setup_benchmark_city_internal(20, 0);
            let t_cch = Instant::now();
            {
                let c = &mut *core;
                c.transit_network.rebuild_pathing(&mut c.region_graph);
            }
            godot_print!(
                "[gen] CCH built: {:.1}s  RSS {}MB  shortcuts={}",
                t_cch.elapsed().as_secs_f32(),
                rss_mb(),
                core.transit_network.cch_graph.shortcuts.len()
            );

            let save_path = "benchmark.sav";
            match core.save_game_internal(save_path) {
                Ok(()) => godot_print!(
                    "[gen] Saved to '{}'  total: {:.1}s",
                    save_path,
                    t0.elapsed().as_secs_f32()
                ),
                Err(e) => godot_print!("[gen] ERROR saving: {}", e),
            }
        }

        self.base_mut().get_tree().unwrap().quit();
    }

    /// Loads `benchmark.sav`, spawns 100 k ON_ROAD agents, and begins the simulation loop.
    ///
    /// Run with `./run.sh --benchmark --headless`.
    pub fn run_benchmark_from_save(&mut self) {
        let save_path = "benchmark.sav";
        godot_print!("[bench] Loading benchmark map from '{}'", save_path);
        let t0 = Instant::now();

        {
            let mut core = self.core.lock().unwrap();
            match core.load_game_internal(save_path) {
                Ok(()) => godot_print!(
                    "[bench] Loaded: {:.2}s  RSS {}MB  edges={} lanes={}",
                    t0.elapsed().as_secs_f32(),
                    rss_mb(),
                    core.region_graph.edge_count(),
                    core.transit_network.lane_system.lanes.len()
                ),
                Err(e) => {
                    godot_print!("[bench] ERROR loading '{}': {}", save_path, e);
                    godot_print!("[bench] Run ./run.sh --generate-benchmark --headless first.");
                    return;
                }
            }
            core.spawn_benchmark_agents();
        }
    }

    /// Returns performance statistics for the simulation.
    pub fn get_perf_stats_internal(&self) -> VarDictionary {
        let snap = self.snapshot.read().unwrap();
        let mut dict = VarDictionary::new();
        let _ = dict.insert("agent_count", snap.agent_count);
        let _ = dict.insert("last_tick_ms", snap.last_tick_ms);
        let _ = dict.insert("pathfind_calls", snap.pathfind_count as i32);
        let _ = dict.insert(
            "fps",
            godot::classes::Engine::singleton().get_frames_per_second(),
        );
        // cell_size requires locking core — only needed for debug, do it cheaply.
        let _ = dict.insert("cell_size", self.core.lock().unwrap().config.zone_cell_m);
        dict
    }

    /// Logs benchmark results to a CSV file. Called from `_process` once per in-game day.
    pub fn log_benchmark_to_csv(&self) {
        use std::io::Write;
        let path = "benchmark_results.csv";
        let file_exists = std::path::Path::new(path).exists();

        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            if !file_exists {
                let _ = writeln!(
                    file,
                    "timestamp,version,agents,map_size,tick_ms,fps,pathfind_calls"
                );
            }

            let snap = self.snapshot.read().unwrap();
            let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
            let version = env!("CARGO_PKG_VERSION");
            let agents = snap.agent_count;
            let map_size = format!("{}x{}", snap.heightmap_width, snap.heightmap_height);
            let tick_ms = snap.last_tick_ms;
            let fps = godot::classes::Engine::singleton().get_frames_per_second();
            let paths = snap.pathfind_count;

            let _ = writeln!(
                file,
                "{},{},{},{},{:.4},{:.1},{}",
                now, version, agents, map_size, tick_ms, fps, paths
            );
        }
    }
}
