//! Traffic and building noise emission/diffusion system.

use super::data_grid::DataGrid;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::grid::zoning::ZoneType;
use crate::simulation::network::graph::RegionGraph;
use rayon::prelude::*;

/// A grid-based system that simulates noise pollution.
///
/// Noise is emitted by vehicles (based on speed) and commercial/industrial buildings,
/// then diffuses and decays across the environmental grid per tick.
pub struct NoiseSystem {
    /// The underlying 2D grid storing noise levels.
    pub grid: DataGrid<f32>,
    /// Temporary buffer used during the diffusion pass.
    pub swap: DataGrid<f32>,
}

impl NoiseSystem {
    /// Creates a new noise system for the given map dimensions.
    pub fn new(config: &crate::simulation::core::config::MapConfig) -> Self {
        let (w, h) = config.get_env_grid_size();
        Self {
            grid: DataGrid::new(w, h, 0.0),
            swap: DataGrid::new(w, h, 0.0),
        }
    }

    /// Simulates one step of noise emission, diffusion, and decay.
    pub fn tick(
        &mut self,
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
        config: &crate::simulation::core::config::MapConfig,
    ) {
        std::mem::swap(&mut self.grid, &mut self.swap);
        self.grid.data.fill(0.0);

        let w = self.grid.width;
        let h = self.grid.height;
        let _world_size_x = config.width_m;
        let _world_size_y = config.height_m;

        // 1. Emission (Sequential - small count)
        for b in &allocator.buildings {
            let (gx_raw, gy_raw) = config.world_to_env_grid(b.center_x, b.center_y, w, h);
            let gx = gx_raw.round() as i32;
            let gy = gy_raw.round() as i32;

            if gx >= 0 && gx < w as i32 && gy >= 0 && gy < h as i32 {
                let gx = gx as usize;
                let gy = gy as usize;
                if b.zone_type == ZoneType::Commercial {
                    if let Some(val) = self.grid.get_mut(gx, gy) {
                        *val = (*val + 30.0).min(100.0);
                    }
                }
                if b.zone_type == ZoneType::Industrial {
                    if let Some(val) = self.grid.get_mut(gx, gy) {
                        *val = (*val + 80.0).min(100.0);
                    }
                }
            }
        }

        // 1B. Emission (Roads)
        for edge in graph.edges() {
            if edge.deleted {
                continue;
            }
            let road_noise = if edge.speed_limit > 60.0 { 4.0 } else { 1.0 };
            for p in &edge.physical_geometry {
                let (gx_raw, gz_raw) = config.world_to_env_grid(p.x, p.z, w, h);
                let gx = gx_raw.round() as i32;
                let gz = gz_raw.round() as i32;
                if gx >= 0 && gx < w as i32 && gz >= 0 && gz < h as i32 {
                    if let Some(val) = self.grid.get_mut(gx as usize, gz as usize) {
                        *val = (*val + road_noise).min(100.0);
                    }
                }
            }
        }

        // 2. Diffusion & 3. Decay (Parallelized)
        let old_grid_ref = &self.swap;

        self.grid
            .data
            .par_chunks_mut(w)
            .enumerate()
            .for_each(|(y, row)| {
                for x in 0..w {
                    let current = *old_grid_ref.get(x, y).unwrap_or(&0.0);

                    let mut neighbor_sum = 0.0;
                    let mut count = 0.0;

                    if x > 0 {
                        neighbor_sum += *old_grid_ref.get(x - 1, y).unwrap_or(&0.0);
                        count += 1.0;
                    }
                    if x < w - 1 {
                        neighbor_sum += *old_grid_ref.get(x + 1, y).unwrap_or(&0.0);
                        count += 1.0;
                    }
                    if y > 0 {
                        neighbor_sum += *old_grid_ref.get(x, y - 1).unwrap_or(&0.0);
                        count += 1.0;
                    }
                    if y < h - 1 {
                        neighbor_sum += *old_grid_ref.get(x, y + 1).unwrap_or(&0.0);
                        count += 1.0;
                    }

                    let avg = if count > 0.0 {
                        neighbor_sum / count
                    } else {
                        0.0
                    };

                    // Noise diffuses very fast, but naturally decays over distance.
                    let propagated = (current * 0.50 + avg * 0.50) * 0.90;

                    row[x] = (row[x] + propagated).min(100.0).max(0.0);
                }
            });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::buildings::allocator::BuildingAllocator;
    use crate::simulation::core::config::MapConfig;
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
    use godot::prelude::Vector3;

    #[test]
    fn test_noise_emission_and_diffusion() {
        let config = MapConfig::default();
        let mut noise = NoiseSystem::new(&config);
        let allocator = BuildingAllocator::new();
        let mut graph = RegionGraph::new();

        // 1. Create a road edge (50m long) near center
        let n1 = graph.add_node(Vector3::new(-25.0, 0.0, 0.0), NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(25.0, 0.0, 0.0), NodeType::Junction);
        graph.add_edge(crate::simulation::network::graph::Edge {
            start_node: n1,
            end_node: n2,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR,
            width: 10.0,
            class: EdgeClass::Standard,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0, // Should emit '1.0' units
            base_cost: 0.0,
            physical_length: 50.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![
                Vector3::new(-25.0, 0.0, 0.0),
                Vector3::ZERO,
                Vector3::new(25.0, 0.0, 0.0),
            ],
            physical_geometry: vec![
                Vector3::new(-25.0, 0.0, 0.0),
                Vector3::ZERO,
                Vector3::new(25.0, 0.0, 0.0),
            ],
            deleted: false, no_building_spawn: false,
        });

        // 2. Tick once - source cells should be positive
        noise.tick(&allocator, &graph, &config);

        // Find center cell (0,0) world -> middle of grid
        let (w, h) = config.get_env_grid_size();
        let cx = w / 2;
        let cy = h / 2;
        let source_val = *noise.grid.get(cx, cy).unwrap();
        assert!(
            source_val > 0.0,
            "Road should emit noise at its nodes/points"
        );

        // 3. Tick multiple times for diffusion
        for _ in 0..10 {
            noise.tick(&allocator, &graph, &config);
        }

        // Verify diffusion 5 cells away
        let diffused_val = *noise.grid.get(cx + 5, cy).unwrap();
        assert!(
            diffused_val > 0.0,
            "Noise should diffuse 5 cells away after 10 ticks"
        );
        assert!(
            !diffused_val.is_infinite() && !diffused_val.is_nan(),
            "Noise values must be finite"
        );

        // 4. Test Decay: remove road and observe values dropping
        graph.edge_mut(0).deleted = true;
        let prev_sum: f32 = noise.grid.data.iter().sum();

        for _ in 0..20 {
            noise.tick(&allocator, &graph, &config);
        }

        let final_sum: f32 = noise.grid.data.iter().sum();
        assert!(
            final_sum < prev_sum * 0.5,
            "Total noise should decay significantly after source is removed"
        );
    }

    #[test]
    fn test_noise_speed_sensitivity() {
        let config = MapConfig::default();
        let mut noise_low = NoiseSystem::new(&config);
        let mut noise_high = NoiseSystem::new(&config);
        let allocator = BuildingAllocator::new();

        let mut graph_low = RegionGraph::new();
        let mut graph_high = RegionGraph::new();

        let n1_l = graph_low.add_node(Vector3::ZERO, NodeType::Junction);
        let n2_l = graph_low.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
        graph_low.add_edge(crate::simulation::network::graph::Edge {
            start_node: n1_l,
            end_node: n2_l,
            speed_limit: 40.0, // Low speed
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR,
            width: 10.0,
            class: EdgeClass::Standard,
            fwd_lanes: 1,
            bkw_lanes: 1,
            base_cost: 0.0,
            physical_length: 10.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::ZERO, Vector3::new(10.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::ZERO, Vector3::new(10.0, 0.0, 0.0)],
            deleted: false, no_building_spawn: false,
        });

        let n1_h = graph_high.add_node(Vector3::ZERO, NodeType::Junction);
        let n2_h = graph_high.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
        graph_high.add_edge(crate::simulation::network::graph::Edge {
            start_node: n1_h,
            end_node: n2_h,
            speed_limit: 100.0, // High speed
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR,
            width: 10.0,
            class: EdgeClass::Standard,
            fwd_lanes: 1,
            bkw_lanes: 1,
            base_cost: 0.0,
            physical_length: 10.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::ZERO, Vector3::new(10.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::ZERO, Vector3::new(10.0, 0.0, 0.0)],
            deleted: false, no_building_spawn: false,
        });

        noise_low.tick(&allocator, &graph_low, &config);
        noise_high.tick(&allocator, &graph_high, &config);

        let (w, h) = config.get_env_grid_size();
        let val_low = noise_low.grid.get(w / 2, h / 2).unwrap();
        let val_high = noise_high.grid.get(w / 2, h / 2).unwrap();

        assert!(
            *val_high > *val_low,
            "High speed road should emit more noise than low speed road"
        );
    }
}
