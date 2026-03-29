//! Benchmarking and performance profiling for simulation.

use godot::prelude::*;
extern crate chrono;
use crate::nodes::simulation_node::SimulationNode;

impl SimulationNode {
    /// Sets up a large-scale benchmark city with a grid of roads and agents.
    pub fn setup_benchmark_city_internal(&mut self, grid_size: i32, agent_count: i32) {
        godot_print!(
            "SimulationNode: Setting up benchmark city (Grid: {}x{}, Agents: {})...",
            grid_size,
            grid_size,
            agent_count
        );

        let mut pts = PackedVector3Array::new();

        // 1. Create a large grid of roads (use Zoning units, not meters, as add_road_internal expects)
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
            self.add_road_internal(pts.clone(), 2, 2, true, true);
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
            self.add_road_internal(pts.clone(), 2, 2, true, true);
        }

        // 2. Force Zoning Rebuild
        self.transit_network
            .flush_zoning_updates(&mut self.zoning, &self.region_graph);

        // 3. Fill with buildings
        self.allocator.tick(
            &mut self.demand,
            &mut self.zoning,
            &self.desirability,
            &self.noise,
            &mut self.agents,
            &mut self.transit_network,
            &mut self.region_graph,
            &self.config,
        );

        // 4. Batch Spawn Agents
        self.agents
            .spawn_random_agents(agent_count as usize, &self.region_graph, &self.allocator);
        godot_print!("Benchmark city ready. Agents: {}", self.agents.len());
    }

    /// Returns performance statistics for the simulation.
    pub fn get_perf_stats_internal(&self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        let _ = dict.insert("agent_count", self.agents.len() as i32);
        let _ = dict.insert("cell_size", self.config.zone_cell_m);
        let _ = dict.insert("last_tick_ms", self.last_tick_duration);
        let _ = dict.insert("pathfind_calls", self.agents.pathfind_count as i32);
        let _ = dict.insert(
            "fps",
            godot::classes::Engine::singleton().get_frames_per_second(),
        );
        dict
    }

    /// Logs benchmark results to a CSV file.
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

            let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
            let version = env!("CARGO_PKG_VERSION");
            let agents = self.agents.len();
            let map_size = format!("{}x{}", self.heightmap.width, self.heightmap.height);
            let tick_ms = self.last_tick_duration;
            let fps = godot::classes::Engine::singleton().get_frames_per_second();
            let paths = self.agents.pathfind_count;

            let _ = writeln!(
                file,
                "{},{},{},{},{:.4},{:.1},{}",
                now, version, agents, map_size, tick_ms, fps, paths
            );
        }
    }
}
