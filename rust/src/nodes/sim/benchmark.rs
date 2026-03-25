//! Benchmarking and performance profiling for simulation.

use godot::prelude::*;
extern crate chrono;
use crate::nodes::simulation_node::SimulationNode;

impl SimulationNode {
    /// Sets up a large-scale benchmark city with a grid of roads and agents.
    pub fn setup_benchmark_city_internal(&mut self, grid_size: i32, agent_count: i32) {
        godot_print!("Setting up benchmark city: {}x{} grid, {} agents", grid_size, grid_size, agent_count);
        self.transit_network.clear(&mut self.zoning, &mut self.allocator);
        self.agents.clear();

        let cell_size = self.config.zone_cell_m;
        let spacing = cell_size * 10.0; // Roads every 10 cells
        let start_offset = -(grid_size as f32 * spacing * 0.5);

        // 1. Create Road Grid
        for i in 0..=grid_size {
            let offset = start_offset + (i as f32 * spacing);
            // Horizontal
            let mut h_pts = PackedVector3Array::new();
            h_pts.push(Vector3::new(start_offset, 0.0, offset));
            h_pts.push(Vector3::new(-start_offset, 0.0, offset));
            self.add_road_internal(h_pts, 2, 2, true, true);

            // Vertical
            let mut v_pts = PackedVector3Array::new();
            v_pts.push(Vector3::new(offset, 0.0, start_offset));
            v_pts.push(Vector3::new(offset, 0.0, -start_offset));
            self.add_road_internal(v_pts, 2, 2, true, true);
        }

        // 2. Initial Tick to build zoning/pathing
        self.simulate_tick();

        // 3. Fill with buildings (forced growth)
        self.demand.residential = 1000.0;
        self.demand.commercial = 1000.0;
        self.demand.industrial = 1000.0;
        for _ in 0..10 { // Burst growth
            self.allocator.tick(&mut self.demand, &mut self.zoning, &self.desirability, &self.noise, &mut self.agents, &mut self.transit_network, &self.config);
        }

        // 4. Batch Spawn Agents
        self.agents.spawn_random_agents(agent_count as usize, &self.transit_network.graph, &self.allocator);
        godot_print!("Benchmark city ready. Agents: {}", self.agents.count);
    }

    /// Returns performance statistics for the simulation.
    pub fn get_perf_stats_internal(&self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        let _ = dict.insert("agent_count", self.agents.count as i32);
        let _ = dict.insert("cell_size", self.config.zone_cell_m);
        let _ = dict.insert("last_tick_ms", self.last_tick_duration);
        let _ = dict.insert("pathfind_calls", self.agents.pathfind_count as i32);
        let _ = dict.insert("fps", godot::classes::Engine::singleton().get_frames_per_second());
        dict
    }

    /// Logs benchmark results to a CSV file.
    pub fn log_benchmark_to_csv(&self) {
        use std::io::Write;
        let path = "benchmark_results.csv";
        let file_exists = std::path::Path::new(path).exists();
        
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            if !file_exists {
                let _ = writeln!(file, "timestamp,version,agents,map_size,tick_ms,fps,pathfind_calls");
            }
            
            let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
            let version = env!("CARGO_PKG_VERSION");
            let agents = self.agents.count;
            let map_size = format!("{}x{}", self.heightmap.width, self.heightmap.height);
            let tick_ms = self.last_tick_duration;
            let fps = godot::classes::Engine::singleton().get_frames_per_second();
            let paths = self.agents.pathfind_count;
            
            let _ = writeln!(file, "{},{},{},{},{:.4},{:.1},{}", now, version, agents, map_size, tick_ms, fps, paths);
        }
    }
}
