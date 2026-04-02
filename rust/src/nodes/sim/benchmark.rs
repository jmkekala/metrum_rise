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

        let w_units = self.config.zone_grid_width() as f32;
        let h_units = self.config.zone_grid_height() as f32;
        let step = (w_units - 40.0) / grid_size as f32;
        let start_x = -w_units * 0.5 + 20.0;
        let start_z = -h_units * 0.5 + 20.0;

        // Horizontal roads
        for i in 0..=grid_size {
            pts.clear();
            pts.push(Vector3::new(start_x, 0.0, start_z + i as f32 * step));
            pts.push(Vector3::new(
                start_x + w_units - 40.0,
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
                start_z + h_units - 40.0,
            ));
            self.add_road_internal(pts.to_vec(), 2, 2);
        }

        godot_print!(
            "[bench] roads built: {:.1}s  RSS {}MB  edges={} nodes={}",
            t0.elapsed().as_secs_f32(),
            rss_mb(),
            self.region_graph.edges.len(),
            self.region_graph.nodes.len()
        );

        let t1 = Instant::now();
        self.transit_network
            .finalize_bulk_load(&mut self.region_graph, &mut self.zoning);
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

    /// Spawns 100 k pre-pathed ON_ROAD agents for the benchmark run.
    ///
    /// Called from `run_benchmark_from_save` on the Godot main thread (while
    /// holding the `SimCore` lock).
    pub(crate) fn spawn_benchmark_agents(&mut self) {
        use crate::simulation::economy::agents::data::Agent;
        use crate::simulation::economy::agents::{MODE_CAR, TRANSIT_ON_ROAD};
        use crate::simulation::network::types::TransitFlags;

        let t_spawn = Instant::now();
        let agent_count = 100_000usize;
        let node_count = self.region_graph.nodes.len();
        if node_count == 0 {
            godot_print!("[bench] ERROR: no nodes in graph, cannot spawn agents");
            return;
        }

        let n_routes = 200usize;
        let mut routes: Vec<Vec<u32>> = Vec::with_capacity(n_routes);
        let t_routes = Instant::now();
        for i in 0..n_routes {
            let s = (i * node_count / n_routes) as u32;
            let e = ((i + n_routes / 2) * node_count / n_routes % node_count) as u32;
            if s == e {
                continue;
            }
            if let Some((_, _, path)) = self.transit_network.cch_graph.find_path(
                s,
                e,
                usize::MAX,
                &self.region_graph,
                TransitFlags::CAR,
            ) {
                if path.len() > 1 {
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
            let route = routes[i % routes.len()].clone();
            let path_len = route.len();
            let start_idx = (i * path_len / agent_count).min(path_len.saturating_sub(2));
            let current_node = route[start_idx];
            let target_node = *route.last().unwrap();
            let node_pos = self.region_graph.nodes[current_node as usize].pos;
            self.agents.agents.push(Agent {
                home_building: usize::MAX,
                work_building: usize::MAX,
                pos_x: node_pos.x,
                pos_y: node_pos.z,
                is_visible: true,
                activity: 0,
                transit: TRANSIT_ON_ROAD,
                happiness: 50.0,
                money: 100.0,
                journey_start_time: 0.0,
                current_building: usize::MAX,
                target_building: usize::MAX,
                current_node,
                target_node,
                current_edge: usize::MAX,
                current_lane_id: usize::MAX,
                lane_distance: 0.0,
                speed: 20.0,
                transit_mode: MODE_CAR,
                current_path: route,
                current_path_index: (start_idx + 1).min(path_len - 1),
                has_car: true,
                vehicle_type: (i % 4) as u8,
                pedestrian_type: 0,
                walk_phase: 0.0,
            });
        }
        godot_print!(
            "[bench] Spawned {} agents (ON_ROAD): {:.2}s  RSS {}MB",
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
                    core.region_graph.edges.len(),
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
        let _ = dict.insert(
            "cell_size",
            self.core.lock().unwrap().config.zone_cell_m,
        );
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
