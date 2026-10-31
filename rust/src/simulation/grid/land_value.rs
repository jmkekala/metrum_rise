// SPDX-License-Identifier: GPL-2.0-only

//! Unimproved spatial Land Value Grid (`LandValueGrid`) and Land Value Tax (LVT) assessment.
//!
//! Computes unimproved location value strictly from spatial factors:
//! 1. Road and transit infrastructure access (`infra_grid`)
//! 2. Public service and utility accessibility (`service_grid`)
//! 3. Surrounding neighborhood agglomeration density (`density_grid`)
//! 4. Environmental externalities (pollution and noise penalties)
//!
//! Crucially, a parcel's own building level (`Building::level`) and improvement value never
//! enter the unimproved land value calculation. An empty or level-1 site next to a level-5
//! skyscraper in a well-served downtown pays the exact same LVT per unit of lot area.

use std::ops::Range;

use rayon::prelude::*;

use super::data_grid::DataGrid;
use super::noise::NoiseSystem;
use super::pollution::PollutionSystem;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::core::config::WorldConfig;
use crate::simulation::economy::fiscal::CityFiscalPolicy;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::zoning::ZoneType;

/// Baseline unimproved land value for an accessible cell with zero pollution and noise.
pub const BASE_UNIMPROVED_LAND_VALUE: f32 = 10.0;

const WEIGHT_INFRA: f32 = 0.45;
const WEIGHT_SERVICE: f32 = 0.35;
const WEIGHT_DENSITY: f32 = 0.30;
const PENALTY_POLLUTION: f32 = 0.25;
const PENALTY_NOISE: f32 = 0.15;

/// Spatial unimproved land value grid and double-buffered rolling sweep state (`SIM-02-H`).
#[derive(Clone)]
pub struct LandValueGrid {
    /// Front buffer: authoritative unimproved land value per cell (`0.0..=100.0`), read by LVT and UI.
    pub grid: DataGrid<f32>,
    /// Back buffer: composed row-by-row during the second half of the daily `EnvironmentSweep`.
    pub back: DataGrid<f32>,
    /// Front diffused road/transit infrastructure access score (`0.0..=100.0`).
    pub infra_grid: DataGrid<f32>,
    /// Back road/transit infrastructure access accumulator and diffusion target.
    pub(crate) infra_back: DataGrid<f32>,
    /// Front diffused public service and utility accessibility score (`0.0..=100.0`).
    pub service_grid: DataGrid<f32>,
    /// Back service accessibility accumulator and diffusion target.
    pub(crate) service_back: DataGrid<f32>,
    /// Front diffused surrounding neighborhood density score (`0.0..=100.0`).
    pub density_grid: DataGrid<f32>,
    /// Back surrounding neighborhood density accumulator and diffusion target.
    pub(crate) density_back: DataGrid<f32>,
    /// Citywide sum of assessed unimproved land value across all grid cells from the last completed sweep.
    pub total_assessed_land_value: f64,
}

/// Type alias matching the naming convention of [`PollutionSystem`] and [`NoiseSystem`].
pub type LandValueSystem = LandValueGrid;

impl LandValueGrid {
    /// Creates a new unimproved land value grid sized from the world configuration.
    pub fn new(config: &WorldConfig) -> Self {
        let (w, h) = config.get_env_grid_size();
        let total_cells = (w * h) as f64;
        Self {
            grid: DataGrid::new(w, h, BASE_UNIMPROVED_LAND_VALUE),
            back: DataGrid::new(w, h, BASE_UNIMPROVED_LAND_VALUE),
            infra_grid: DataGrid::new(w, h, 0.0),
            infra_back: DataGrid::new(w, h, 0.0),
            service_grid: DataGrid::new(w, h, 0.0),
            service_back: DataGrid::new(w, h, 0.0),
            density_grid: DataGrid::new(w, h, 0.0),
            density_back: DataGrid::new(w, h, 0.0),
            total_assessed_land_value: total_cells * f64::from(BASE_UNIMPROVED_LAND_VALUE),
        }
    }

    /// Advances the unimproved land value simulation by one full step: emits spatial factors
    /// from buildings and road edges, diffuses factor grids, composes the unimproved land value
    /// grid against pollution and noise, and swaps front/back buffers.
    ///
    /// One full day of [`super::sweep::EnvironmentSweep`] produces the exact same bit-for-bit result.
    pub fn tick(
        &mut self,
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
        pollution: &PollutionSystem,
        noise: &NoiseSystem,
        config: &WorldConfig,
    ) {
        self.clear_back();
        for building in &allocator.buildings {
            let is_service = allocator.is_city_service_building(building);
            self.emit_building(building, is_service, config);
        }
        for edge in graph.edges() {
            self.emit_edge(edge, config);
        }
        let rows = 0..self.back.height;
        self.diffuse_and_compose_back_rows(rows, &pollution.grid, &noise.grid);
        self.swap_buffers();
    }

    /// Zeroes all back factor accumulators at the start of a rolling sweep. O(grid).
    pub(crate) fn clear_back(&mut self) {
        self.infra_back.data.fill(0.0);
        self.service_back.data.fill(0.0);
        self.density_back.data.fill(0.0);
    }

    /// Accumulates spatial accessibility and neighborhood presence from `building` into the back
    /// buffers.
    ///
    /// **Georgist Invariant:** `building.level` and building financial metrics are intentionally
    /// never read here. Every active site contributes purely by its spatial presence and service
    /// role so parcel upgrades/downgrades never alter the parcel's own assessed land value. O(1).
    pub(crate) fn emit_building(
        &mut self,
        building: &Building,
        is_city_service: bool,
        config: &WorldConfig,
    ) {
        if building.broken || building.is_deserted || building.is_under_construction() {
            return;
        }
        let w = self.back.width;
        let h = self.back.height;
        let (gx_raw, gy_raw) = config.world_to_env_grid(building.center_x, building.center_y);
        let gx = gx_raw.round() as i32;
        let gy = gy_raw.round() as i32;
        if gx < 0 || gx >= w as i32 || gy < 0 || gy >= h as i32 {
            return;
        }
        let gx = gx as usize;
        let gy = gy as usize;

        if is_city_service {
            if let Some(val) = self.service_back.get_mut(gx, gy) {
                *val = (*val + 25.0).min(100.0);
            }
        } else if building.zone_type == ZoneType::Commercial
            && let Some(val) = self.service_back.get_mut(gx, gy)
        {
            *val = (*val + 8.0).min(100.0);
        }

        if matches!(
            building.zone_type,
            ZoneType::Residential | ZoneType::Commercial | ZoneType::Industrial
        ) && let Some(val) = self.density_back.get_mut(gx, gy)
        {
            *val = (*val + 6.0).min(100.0);
        }
    }

    /// Accumulates road/transit infrastructure access along `edge`'s physical geometry. O(points).
    pub(crate) fn emit_edge(&mut self, edge: &Edge, config: &WorldConfig) {
        if edge.deleted {
            return;
        }
        let w = self.infra_back.width;
        let h = self.infra_back.height;
        let lanes = f32::from((edge.fwd_lanes + edge.bkw_lanes).max(1));
        let emission = (2.5 * lanes).min(15.0);

        for pt in &edge.physical_geometry {
            let (gx_raw, gy_raw) = config.world_to_env_grid(pt.x, pt.z);
            let gx = gx_raw.round() as i32;
            let gy = gy_raw.round() as i32;
            if gx >= 0
                && gx < w as i32
                && gy >= 0
                && gy < h as i32
                && let Some(val) = self.infra_back.get_mut(gx as usize, gy as usize)
            {
                *val = (*val + emission).min(100.0);
            }
        }
    }

    /// Diffuses `rows` of the spatial factor back grids from their fronts and composes the
    /// corresponding rows of `self.back` using the same rows of `p_grid` and `n_grid`.
    /// O(rows × width), parallelized across rows via Rayon (`par_chunks_mut`).
    pub(crate) fn diffuse_and_compose_back_rows(
        &mut self,
        rows: Range<usize>,
        p_grid: &DataGrid<f32>,
        n_grid: &DataGrid<f32>,
    ) {
        if rows.is_empty() {
            return;
        }
        diffuse_emissions!(
            self.infra_back,
            self.infra_grid,
            rows.clone(),
            0.55,
            0.45,
            0.99
        );
        diffuse_emissions!(
            self.service_back,
            self.service_grid,
            rows.clone(),
            0.50,
            0.50,
            0.99
        );
        // `own_weight = 0.0, neighbor_weight = 1.0`: a cell's density score comes strictly from
        // its surrounding neighbors rather than its own parcel.
        diffuse_emissions!(
            self.density_back,
            self.density_grid,
            rows.clone(),
            0.0,
            1.0,
            0.985
        );
        compose_rows(
            &mut self.back,
            rows,
            &self.infra_back,
            &self.service_back,
            &self.density_back,
            p_grid,
            n_grid,
        );
    }

    /// Recomposes both the front and back land value grids from the current factor, pollution,
    /// and noise grids after loading a save.
    pub(crate) fn recompose_from_factors(
        &mut self,
        pollution: &PollutionSystem,
        noise: &NoiseSystem,
    ) {
        let front_rows = 0..self.grid.height;
        compose_rows(
            &mut self.grid,
            front_rows,
            &self.infra_grid,
            &self.service_grid,
            &self.density_grid,
            &pollution.grid,
            &noise.grid,
        );
        let back_rows = 0..self.back.height;
        compose_rows(
            &mut self.back,
            back_rows,
            &self.infra_back,
            &self.service_back,
            &self.density_back,
            &pollution.back,
            &noise.back,
        );
        self.refresh_total_assessed_land_value();
    }

    /// Swaps the finished back grids into the front buffers at the end of the daily sweep. O(grid)
    /// for the deterministic integer sum of `total_assessed_land_value`.
    pub(crate) fn swap_buffers(&mut self) {
        std::mem::swap(&mut self.infra_grid, &mut self.infra_back);
        std::mem::swap(&mut self.service_grid, &mut self.service_back);
        std::mem::swap(&mut self.density_grid, &mut self.density_back);
        std::mem::swap(&mut self.grid, &mut self.back);
        self.refresh_total_assessed_land_value();
    }

    fn refresh_total_assessed_land_value(&mut self) {
        let total_cents: i64 = self
            .grid
            .data
            .iter()
            .map(|&v| (f64::from(v.max(0.0)) * 100.0).round() as i64)
            .sum();
        self.total_assessed_land_value = total_cents as f64 / 100.0;
    }

    /// Samples the unimproved land value at world coordinates `(world_x, world_z)` using
    /// bilinear interpolation across environmental grid cells. O(1).
    #[inline]
    pub fn sample_world(&self, world_x: f32, world_z: f32, config: &WorldConfig) -> f32 {
        let (gx, gz) = config.world_to_env_grid(world_x, world_z);
        self.grid.sample_bilinear(gx, gz).max(0.0)
    }

    /// Computes the daily Land Value Tax (LVT) owed by a site from its unimproved spatial
    /// land value and lot footprint area (`width_cells × depth_cells`), completely independent
    /// of `building.level` or building improvement value. O(1).
    #[inline]
    pub(crate) fn daily_site_lvt(
        &self,
        building: &Building,
        config: &WorldConfig,
        fiscal_policy: &CityFiscalPolicy,
    ) -> f32 {
        if fiscal_policy.lvt_rate <= 0.0 {
            return 0.0;
        }
        let land_value_per_cell = self.sample_world(building.center_x, building.center_y, config);
        if land_value_per_cell <= 0.0 {
            return 0.0;
        }
        let lot_cells =
            f32::from(building.width_cells.max(1)) * f32::from(building.depth_cells.max(1));
        lot_cells * land_value_per_cell * fiscal_policy.lvt_rate.clamp(0.0, 1.0)
    }
}

fn compose_rows(
    target: &mut DataGrid<f32>,
    rows: Range<usize>,
    infra: &DataGrid<f32>,
    service: &DataGrid<f32>,
    density: &DataGrid<f32>,
    pollution: &DataGrid<f32>,
    noise: &DataGrid<f32>,
) {
    if rows.is_empty() {
        return;
    }
    let w = target.width;
    let first = rows.start;
    target.data[rows.start * w..rows.end * w]
        .par_chunks_mut(w)
        .enumerate()
        .for_each(|(offset, row)| {
            let y = first + offset;
            for (x, cell) in row.iter_mut().enumerate() {
                let inf = *infra.get(x, y).unwrap_or(&0.0);
                let srv = *service.get(x, y).unwrap_or(&0.0);
                let den = *density.get(x, y).unwrap_or(&0.0);
                let pol = *pollution.get(x, y).unwrap_or(&0.0);
                let nse = *noise.get(x, y).unwrap_or(&0.0);

                let lv = BASE_UNIMPROVED_LAND_VALUE
                    + inf * WEIGHT_INFRA
                    + srv * WEIGHT_SERVICE
                    + den * WEIGHT_DENSITY
                    - pol * PENALTY_POLLUTION
                    - nse * PENALTY_NOISE;
                *cell = lv.clamp(0.0, 100.0);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DEFAULT_URBAN_ROAD_SPEED_MS;
    use crate::simulation::buildings::allocator::indexed_test_building;
    use crate::simulation::network::types::{
        EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
    };
    use godot::prelude::Vector3;

    #[test]
    fn land_value_and_lvt_are_strictly_independent_of_building_level() {
        let config = WorldConfig {
            width_m: 400.0,
            height_m: 400.0,
            env_cell_m: 40.0,
            ..WorldConfig::default()
        };
        let mut graph = RegionGraph::new();
        let n0 = graph.add_node(Vector3::new(-100.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
        graph.add_edge(Edge {
            start_node: n0,
            end_node: n1,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR,
            width: 10.0,
            class: EdgeClass::Standard,
            fwd_lanes: 2,
            bkw_lanes: 2,
            speed_limit: DEFAULT_URBAN_ROAD_SPEED_MS,
            base_cost: 0.0,
            physical_length: 200.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(-100.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            physical_geometry: vec![
                Vector3::new(-80.0, 0.0, 0.0),
                Vector3::new(0.0, 0.0, 0.0),
                Vector3::new(80.0, 0.0, 0.0),
            ],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
        });

        let build_world = |target_level: u8| {
            let mut allocator = BuildingAllocator::new();
            let mut target = indexed_test_building(String::new(), ZoneType::Commercial, 0);
            target.center_x = 0.0;
            target.center_y = 0.0;
            target.width_cells = 2;
            target.depth_cells = 3;
            target.level = target_level;
            allocator.buildings.push(target);

            let mut neighbor = indexed_test_building(String::new(), ZoneType::Residential, 1);
            neighbor.center_x = 40.0;
            neighbor.center_y = 0.0;
            neighbor.level = 3;
            allocator.buildings.push(neighbor);

            let mut pollution = PollutionSystem::new(&config);
            let mut noise = NoiseSystem::new(&config);
            let mut lv = LandValueSystem::new(&config);
            for _ in 0..5 {
                pollution.tick(&allocator, &config);
                noise.tick(&allocator, &graph, &config);
                lv.tick(&allocator, &graph, &pollution, &noise, &config);
            }
            (allocator, lv)
        };

        let (alloc_l1, lv_l1) = build_world(1);
        let (alloc_l5, lv_l5) = build_world(5);

        let bits_l1: Vec<u32> = lv_l1.grid.data.iter().map(|v| v.to_bits()).collect();
        let bits_l5: Vec<u32> = lv_l5.grid.data.iter().map(|v| v.to_bits()).collect();
        assert_eq!(
            bits_l1, bits_l5,
            "unimproved land value grid must be bit-identical regardless of building level"
        );

        let policy = CityFiscalPolicy {
            lvt_rate: 0.15,
            ..CityFiscalPolicy::default()
        };
        let tax_l1 = lv_l1.daily_site_lvt(&alloc_l1.buildings[0], &config, &policy);
        let tax_l5 = lv_l5.daily_site_lvt(&alloc_l5.buildings[0], &config, &policy);
        assert!(tax_l1 > 0.0);
        assert_eq!(
            tax_l1.to_bits(),
            tax_l5.to_bits(),
            "LVT owed by a level-1 site and a level-5 site on the same lot must be identical"
        );
    }

    #[test]
    fn road_infrastructure_and_services_increase_land_value_while_pollution_reduces_it() {
        let config = WorldConfig {
            width_m: 400.0,
            height_m: 400.0,
            env_cell_m: 40.0,
            ..WorldConfig::default()
        };
        let allocator = BuildingAllocator::new();
        let mut graph = RegionGraph::new();
        let n0 = graph.add_node(Vector3::new(-40.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(40.0, 0.0, 0.0), NodeType::Junction);
        graph.add_edge(Edge {
            start_node: n0,
            end_node: n1,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR,
            width: 14.0,
            class: EdgeClass::Standard,
            fwd_lanes: 2,
            bkw_lanes: 2,
            speed_limit: DEFAULT_URBAN_ROAD_SPEED_MS,
            base_cost: 0.0,
            physical_length: 80.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(-40.0, 0.0, 0.0), Vector3::new(40.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::ZERO],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
        });

        let pollution = PollutionSystem::new(&config);
        let noise = NoiseSystem::new(&config);
        let mut lv = LandValueSystem::new(&config);
        let baseline_center = lv.sample_world(0.0, 0.0, &config);

        for _ in 0..4 {
            lv.tick(&allocator, &graph, &pollution, &noise, &config);
        }
        let road_center = lv.sample_world(0.0, 0.0, &config);
        let remote_corner = lv.sample_world(-180.0, -180.0, &config);
        assert!(road_center > baseline_center);
        assert!(road_center > remote_corner);

        let mut polluted = PollutionSystem::new(&config);
        polluted.grid.data.fill(80.0);
        lv.tick(&allocator, &graph, &polluted, &noise, &config);
        let polluted_center = lv.sample_world(0.0, 0.0, &config);
        assert!(polluted_center < road_center);
    }
}
