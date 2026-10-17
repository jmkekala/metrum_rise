// SPDX-License-Identifier: GPL-2.0-only

//! The day-long rolling sweep of pollution, noise and desirability (`SIM-02-H`,
//! `docs/simulation.md#snapshots`).
//!
//! Each grid keeps a front, which every reader uses, and a back that the sweep builds over one
//! day of slots. The day's first slot freezes the building handle capacity `Hb` and the road
//! edge count `He` and zeroes the pollution and noise backs. With `E` = half the day's slots,
//! chunk `k < E` folds the emissions of building handle slots `[k·Hb/E, (k+1)·Hb/E)` and of road
//! edges `[k·He/E, (k+1)·He/E)`; buildings and roads added later count next day. Chunk `E + j`
//! diffuses rows `[j·R/D, (j+1)·R/D)` of the `R`-row grids from their fronts, with `D` the
//! remaining slots, and composes desirability for the same rows. The day's last slot swaps every
//! back in, so one sweep yields the grids of one whole-grid `tick` from the same fronts.

use std::ops::Range;

use super::desirability::DesirabilitySystem;
use super::noise::NoiseSystem;
use super::pollution::PollutionSystem;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::core::scheduler::SLOTS_PER_DAY;
use crate::simulation::network::graph::RegionGraph;

// Chunks of a day that fold emissions; the rest diffuse rows.
const EMISSION_CHUNKS: u64 = SLOTS_PER_DAY / 2;
const DIFFUSION_CHUNKS: u64 = SLOTS_PER_DAY - EMISSION_CHUNKS;

/// Entity counts frozen at a sweep's first slot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SweepCapacities {
    /// Building handle slots, live or free.
    pub(crate) building_handles: u64,
    /// Road edges, deleted ones included.
    pub(crate) edges: u64,
}

/// Progress of the day's environment sweep; saved with the back grids, so a loaded game
/// continues it bit-identically.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct EnvironmentSweep {
    /// Capacities of the sweep in progress; `None` between sweeps, from the day's last slot (or
    /// a world's start) to the next day's first.
    pub(crate) frozen: Option<SweepCapacities>,
    /// Chunk the sweep expects next. A slot that skips a chunk, as when the clock is set
    /// directly, abandons the sweep, so readers never see a partial one.
    pub(crate) next_chunk: u64,
    /// First road edge the next emission chunk folds: the start of the chunk's edge range,
    /// unless a save compacted deleted edges below it.
    pub(crate) next_edge: u64,
}

impl EnvironmentSweep {
    /// Runs chunk `slot mod SLOTS_PER_DAY` of the day's sweep; the caller registers appended
    /// buildings' handles first. O(Hb/E + edges' points in He/E) in the first half and
    /// O(R/D × width) in the second; the first slot also zeroes two back grids, O(grid).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn advance(
        &mut self,
        slot: u64,
        pollution: &mut PollutionSystem,
        noise: &mut NoiseSystem,
        desirability: &mut DesirabilitySystem,
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
        config: &WorldConfig,
    ) {
        let chunk = slot % SLOTS_PER_DAY;
        if chunk == 0 {
            *self = Self {
                frozen: Some(SweepCapacities {
                    building_handles: allocator.building_handles.capacity() as u64,
                    edges: graph.edge_count() as u64,
                }),
                next_chunk: 1,
                next_edge: 0,
            };
            pollution.clear_back();
            noise.clear_back();
        } else if chunk != self.next_chunk {
            self.frozen = None;
        }
        let Some(frozen) = self.frozen else {
            return;
        };
        self.next_chunk = chunk + 1;
        if chunk < EMISSION_CHUNKS {
            for index in part(chunk, EMISSION_CHUNKS, frozen.building_handles) {
                // A freed slot has nothing to emit; a reused one emits its current building.
                if let Some(building_idx) = allocator.building_handles.dense_at(index as usize) {
                    let building = &allocator.buildings[building_idx];
                    pollution.emit_building(building, config);
                    noise.emit_building(building, config);
                }
            }
            let end = part(chunk, EMISSION_CHUNKS, frozen.edges).end;
            let edges = graph.edges();
            let live_end = (end as usize).min(edges.len());
            if let Some(chunk_edges) = edges.get(self.next_edge as usize..live_end) {
                for edge in chunk_edges {
                    noise.emit_edge(edge, config);
                }
            }
            self.next_edge = self.next_edge.max(end);
            return;
        }
        let diffusion = chunk - EMISSION_CHUNKS;
        pollution.diffuse_back_rows(rows(diffusion, pollution.back.height));
        noise.diffuse_back_rows(rows(diffusion, noise.back.height));
        desirability.compose_back_rows(rows(diffusion, desirability.back.height), pollution, noise);
        if chunk == SLOTS_PER_DAY - 1 {
            pollution.swap_buffers();
            noise.swap_buffers();
            desirability.swap_buffers();
            self.frozen = None;
        }
    }
}

// Part `chunk` of `chunks` equal parts of `0..total`.
fn part(chunk: u64, chunks: u64, total: u64) -> Range<u64> {
    chunk * total / chunks..(chunk + 1) * total / chunks
}

// Rows diffused by diffusion chunk `chunk` of a grid of `height` rows.
fn rows(chunk: u64, height: usize) -> Range<usize> {
    let range = part(chunk, DIFFUSION_CHUNKS, height as u64);
    range.start as usize..range.end as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DEFAULT_URBAN_ROAD_SPEED_MS, KMH_TO_MPS};
    use crate::simulation::buildings::allocator::indexed_test_building;
    use crate::simulation::grid::data_grid::DataGrid;
    use crate::simulation::network::graph::Edge;
    use crate::simulation::network::types::{
        EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
    };
    use crate::simulation::zoning::ZoneType;
    use godot::prelude::Vector3;

    // A 5 × 4 grid of 40 m cells over x in [-100, 100] and z in [-80, 80].
    fn small_world() -> (WorldConfig, BuildingAllocator, RegionGraph) {
        let config = WorldConfig {
            width_m: 200.0,
            height_m: 160.0,
            env_cell_m: 40.0,
            ..WorldConfig::default()
        };
        let mut allocator = BuildingAllocator::new();
        for (zone_type, x, z) in [
            (ZoneType::Industrial, -60.0, -60.0),
            (ZoneType::Commercial, 20.0, 0.0),
            // Two industrial sources in one cell saturate the noise emission at 100.
            (ZoneType::Industrial, 60.0, 60.0),
            (ZoneType::Residential, -20.0, 20.0),
            (ZoneType::Industrial, 60.0, 60.0),
            (ZoneType::Commercial, -60.0, -60.0),
        ] {
            let mut building = indexed_test_building(String::new(), zone_type, 0);
            building.center_x = x;
            building.center_y = z;
            allocator.buildings.push(building);
        }
        // A removal frees a handle slot and moves the last building into it, so handle order
        // differs from dense order.
        let len = allocator.buildings.len();
        allocator.building_handles.register_appended(len);
        allocator.building_handles.swap_remove(1, len);
        allocator.buildings.swap_remove(1);

        let mut graph = RegionGraph::new();
        let west = graph.add_node(Vector3::new(-100.0, 0.0, 0.0), NodeType::Junction);
        let east = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
        for (speed_limit, z, deleted) in [
            (DEFAULT_URBAN_ROAD_SPEED_MS, 0.0, false),
            (100.0 * KMH_TO_MPS, 40.0, false),
            (DEFAULT_URBAN_ROAD_SPEED_MS, -40.0, true),
            (DEFAULT_URBAN_ROAD_SPEED_MS, -20.0, false),
        ] {
            // Nine points 20 m apart, several in each cell they cross.
            let points: Vec<Vector3> = (0..9)
                .map(|i| Vector3::new(-80.0 + 20.0 * i as f32, 0.0, z))
                .collect();
            let edge = graph.add_edge(Edge {
                start_node: west,
                end_node: east,
                primary_type: TransitType::Road,
                allowed_types: TransitFlags::CAR,
                width: 10.0,
                class: EdgeClass::Standard,
                fwd_lanes: 1,
                bkw_lanes: 1,
                speed_limit,
                base_cost: 0.0,
                physical_length: 160.0,
                current_congestion: 0.0,
                start_clip: 0.0,
                end_clip: 0.0,
                geometry: points.clone(),
                physical_geometry: points,
                deleted: false,
                no_building_spawn: false,
                vehicle_frontage_access: VehicleFrontageAccess::BothSides,
            });
            graph.edge_mut(edge).deleted = deleted;
        }
        (config, allocator, graph)
    }

    fn bits(grid: &DataGrid<f32>) -> Vec<u32> {
        grid.data.iter().map(|value| value.to_bits()).collect()
    }

    #[test]
    fn one_sweep_equals_one_whole_grid_step() {
        let (config, allocator, graph) = small_world();
        let mut pollution = PollutionSystem::new(&config);
        let mut noise = NoiseSystem::new(&config);
        let mut desirability = DesirabilitySystem::new(&config);
        // Fronts with interior values and values that clamp at 100 after diffusion.
        for (index, cell) in pollution.grid.data.iter_mut().enumerate() {
            *cell = (index * 37 % 101) as f32 * 0.99;
        }
        for (index, cell) in noise.grid.data.iter_mut().enumerate() {
            *cell = (index * 53 % 101) as f32;
        }
        desirability.tick(&pollution, &noise);
        let start = (
            bits(&pollution.grid),
            bits(&noise.grid),
            bits(&desirability.grid),
        );

        let mut expected_pollution = PollutionSystem::new(&config);
        let mut expected_noise = NoiseSystem::new(&config);
        let mut expected_desirability = DesirabilitySystem::new(&config);
        expected_pollution
            .grid
            .data
            .clone_from(&pollution.grid.data);
        expected_noise.grid.data.clone_from(&noise.grid.data);
        expected_pollution.tick(&allocator, &config);
        expected_noise.tick(&allocator, &graph, &config);
        expected_desirability.tick(&expected_pollution, &expected_noise);

        // A sweep that starts mid-day finishes nothing, and readers keep the old fronts until
        // the next day's last slot.
        let mut sweep = EnvironmentSweep::default();
        let last = 2 * SLOTS_PER_DAY - 1;
        for slot in SLOTS_PER_DAY / 4..=last {
            if slot == last {
                assert_eq!(
                    (
                        bits(&pollution.grid),
                        bits(&noise.grid),
                        bits(&desirability.grid)
                    ),
                    start
                );
            }
            sweep.advance(
                slot,
                &mut pollution,
                &mut noise,
                &mut desirability,
                &allocator,
                &graph,
                &config,
            );
        }
        assert_eq!(sweep.frozen, None);
        assert_eq!(bits(&pollution.grid), bits(&expected_pollution.grid));
        assert_eq!(bits(&noise.grid), bits(&expected_noise.grid));
        assert_eq!(bits(&desirability.grid), bits(&expected_desirability.grid));
        assert!(noise.grid.data.contains(&100.0));
        assert_ne!(bits(&pollution.grid), start.0);
    }
}
