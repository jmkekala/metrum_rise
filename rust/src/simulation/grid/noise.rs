// SPDX-License-Identifier: GPL-2.0-only

//! Traffic and building noise emission/diffusion system.

use std::ops::Range;

use super::data_grid::DataGrid;
use crate::config::HIGH_NOISE_ROAD_THRESHOLD_MS;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::core::config::WorldConfig;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::zoning::ZoneType;

/// A grid-based system that simulates noise pollution.
///
/// Noise is emitted at road geometry points (based on speed limit) and by commercial/industrial buildings,
/// then diffuses and decays across the environmental grid per tick.
pub struct NoiseSystem {
    /// The current noise level grid (0-100), which every reader uses.
    pub grid: DataGrid<f32>,
    /// The next grid being built (`SIM-02-H`): the emissions folded so far, then diffused rows.
    pub back: DataGrid<f32>,
}

impl NoiseSystem {
    /// Creates a new noise system for the given map dimensions.
    pub fn new(config: &WorldConfig) -> Self {
        let (w, h) = config.get_env_grid_size();
        Self {
            grid: DataGrid::new(w, h, 0.0),
            back: DataGrid::new(w, h, 0.0),
        }
    }

    /// Simulates one whole step of noise emission, diffusion, and decay: buildings in dense
    /// order, then road edges, then every row. One rolling sweep ([`super::sweep`]) produces the
    /// same grid.
    pub fn tick(
        &mut self,
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
        config: &WorldConfig,
    ) {
        self.clear_back();
        // Sequential: building and edge counts are small compared to the grid.
        for building in &allocator.buildings {
            self.emit_building(building, config);
        }
        for edge in graph.edges() {
            self.emit_edge(edge, config);
        }
        self.diffuse_back_rows(0..self.back.height);
        self.swap_buffers();
    }

    /// Zeroes the back grid before the first emission of a step. O(grid).
    pub(crate) fn clear_back(&mut self) {
        self.back.data.fill(0.0);
    }

    /// Adds `building`'s emission to the back grid at its cell: 30 when commercial, 80 when
    /// industrial, saturating at 100. O(1).
    pub(crate) fn emit_building(&mut self, building: &Building, config: &WorldConfig) {
        let w = self.back.width;
        let h = self.back.height;
        let (gx_raw, gy_raw) = config.world_to_env_grid(building.center_x, building.center_y);
        let gx = gx_raw.round() as i32;
        let gy = gy_raw.round() as i32;

        if gx >= 0 && gx < w as i32 && gy >= 0 && gy < h as i32 {
            let gx = gx as usize;
            let gy = gy as usize;
            if building.zone_type == ZoneType::Commercial
                && let Some(val) = self.back.get_mut(gx, gy)
            {
                *val = (*val + 30.0).min(100.0);
            }
            if building.zone_type == ZoneType::Industrial
                && let Some(val) = self.back.get_mut(gx, gy)
            {
                *val = (*val + 80.0).min(100.0);
            }
        }
    }

    /// Adds a live road edge's emission to the back grid: 1, or 4 above the high-noise speed,
    /// at each physical geometry point, saturating at 100. O(points).
    pub(crate) fn emit_edge(&mut self, edge: &Edge, config: &WorldConfig) {
        if edge.deleted {
            return;
        }
        let w = self.back.width;
        let h = self.back.height;
        let road_noise = if edge.speed_limit > HIGH_NOISE_ROAD_THRESHOLD_MS {
            4.0
        } else {
            1.0
        };
        for p in &edge.physical_geometry {
            let (gx_raw, gz_raw) = config.world_to_env_grid(p.x, p.z);
            let gx = gx_raw.round() as i32;
            let gz = gz_raw.round() as i32;
            if gx >= 0
                && gx < w as i32
                && gz >= 0
                && gz < h as i32
                && let Some(val) = self.back.get_mut(gx as usize, gz as usize)
            {
                *val = (*val + road_noise).min(100.0);
            }
        }
    }

    /// Diffuses `rows` of the back grid, which hold their emissions, from the front grid and
    /// clamps them to 0-100. O(rows × width).
    pub(crate) fn diffuse_back_rows(&mut self, rows: Range<usize>) {
        diffuse_emissions!(self.back, self.grid, rows, 0.50, 0.50, 0.90);
    }

    /// Makes the finished back grid the front; the old front becomes the next back. O(1).
    pub(crate) fn swap_buffers(&mut self) {
        std::mem::swap(&mut self.grid, &mut self.back);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DEFAULT_URBAN_ROAD_SPEED_MS, KMH_TO_MPS};
    use crate::simulation::buildings::allocator::BuildingAllocator;
    use crate::simulation::core::config::WorldConfig;
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
    use godot::prelude::Vector3;

    #[test]
    fn test_noise_emission_and_diffusion() {
        let config = WorldConfig::default();
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
            speed_limit: DEFAULT_URBAN_ROAD_SPEED_MS, // Should emit '1.0' units
            base_cost: 0.0,
            physical_length: 50.0,
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
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access:
                crate::simulation::network::types::VehicleFrontageAccess::BothSides,
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
        let config = WorldConfig::default();
        let mut noise_low = NoiseSystem::new(&config);
        let mut noise_high = NoiseSystem::new(&config);
        let allocator = BuildingAllocator::new();

        let mut graph_low = RegionGraph::new();

        let n1_l = graph_low.add_node(Vector3::ZERO, NodeType::Junction);
        let n2_l = graph_low.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
        graph_low.add_edge(crate::simulation::network::graph::Edge {
            start_node: n1_l,
            end_node: n2_l,
            speed_limit: 40.0 * KMH_TO_MPS, // Low speed
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR,
            width: 10.0,
            class: EdgeClass::Standard,
            fwd_lanes: 1,
            bkw_lanes: 1,
            base_cost: 0.0,
            physical_length: 10.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::ZERO, Vector3::new(10.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::ZERO, Vector3::new(10.0, 0.0, 0.0)],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access:
                crate::simulation::network::types::VehicleFrontageAccess::BothSides,
        });

        let mut graph_high = graph_low.clone();
        graph_high.edge_mut(0).speed_limit = 100.0 * KMH_TO_MPS;

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
