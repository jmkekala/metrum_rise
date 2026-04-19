//! Blank-world creation and reusable world-definition bridge methods.

use crate::config::HEIGHT_SCALE;
use crate::debug_log;
use crate::nodes::sim::core::{CityTreasury, SimCore};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::core::time::TimeSystem;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::definitions::load_runtime_economy_tuning;
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
use crate::simulation::world_definition::{
    AuthoredLakeFill, AuthoredWaterBoundaryKind, AuthoredWaterBoundaryPoint, LoadedWorldDefinition,
    WorldDefinitionView, load_world_definition_from_sqlite, save_world_definition_to_sqlite,
};
use godot::prelude::Vector2;
use std::collections::VecDeque;
use std::path::PathBuf;

const AUTHORING_WATER_PREVIEW_DT: f32 = 0.25;
const AUTHORING_WATER_PREVIEW_STEPS: usize = 48;

impl SimCore {
    /// Resets the live runtime to one fresh blank world with the given terrain settings.
    pub(crate) fn create_blank_world_internal(
        &mut self,
        width_m: f32,
        height_m: f32,
        terrain_cell_m: f32,
        terrain_chunk_m: f32,
        base_elevation_m: f32,
    ) -> Result<(), String> {
        validate_positive_f32(width_m, "width_m")?;
        validate_positive_f32(height_m, "height_m")?;
        validate_positive_f32(terrain_cell_m, "terrain_cell_m")?;
        validate_positive_f32(terrain_chunk_m, "terrain_chunk_m")?;
        if !base_elevation_m.is_finite() {
            return Err("base_elevation_m must be finite".to_owned());
        }

        let config = WorldConfig::new(
            width_m,
            height_m,
            self.config.env_cell_m,
            self.config.zone_cell_m,
        )
        .with_terrain_resolution(terrain_cell_m)
        .with_chunking(terrain_chunk_m, base_elevation_m);
        debug_log!(
            "world-editor",
            "create_blank_world width_m={:.1} height_m={:.1} terrain_cell_m={:.1} terrain_chunk_m={:.1} base_elevation_m={:.1}",
            width_m,
            height_m,
            terrain_cell_m,
            terrain_chunk_m,
            base_elevation_m
        );
        let terrain = TerrainSystem::from_world_config(&config);
        self.reset_to_blank_world_runtime(config, terrain);
        Ok(())
    }

    /// Saves the current authored world state as a reusable world-definition asset.
    pub(crate) fn save_world_definition_internal(
        &self,
        path: &str,
        name: &str,
    ) -> Result<(), String> {
        debug_log!(
            "world-editor",
            "save_world_definition path={} name={} world=({:.1}m x {:.1}m) terrain_cell_m={:.1} terrain_chunk_m={:.1}",
            path,
            name,
            self.config.width_m,
            self.config.height_m,
            self.config.terrain_cell_m,
            self.config.terrain_chunk_m
        );
        save_world_definition_to_sqlite(
            &PathBuf::from(path),
            WorldDefinitionView {
                name,
                config: &self.config,
                terrain: &self.heightmap,
                water_boundary_points: &self.world_water_boundary_points,
                lake_fills: &self.world_lake_fills,
            },
        )
        .map_err(|err| err.to_string())
    }

    /// Adds or strengthens one authored world-water source at the clicked terrain cell.
    pub(crate) fn add_world_water_source_internal(
        &mut self,
        pos: Vector2,
        rate_m_per_tick: f32,
    ) -> Result<(), String> {
        self.place_world_water_boundary_internal(
            AuthoredWaterBoundaryKind::Source,
            pos,
            rate_m_per_tick,
        )
    }

    /// Adds or strengthens one authored world-water sink at the clicked terrain cell.
    pub(crate) fn add_world_water_sink_internal(
        &mut self,
        pos: Vector2,
        rate_m_per_tick: f32,
    ) -> Result<(), String> {
        self.place_world_water_boundary_internal(
            AuthoredWaterBoundaryKind::Sink,
            pos,
            rate_m_per_tick,
        )
    }

    /// Removes the nearest authored world-water source within the given radius.
    pub(crate) fn remove_world_water_source_near_internal(
        &mut self,
        pos: Vector2,
        radius_m: f32,
    ) -> bool {
        self.remove_world_water_boundary_near_internal(
            AuthoredWaterBoundaryKind::Source,
            pos,
            radius_m,
        )
    }

    /// Removes the nearest authored world-water sink within the given radius.
    pub(crate) fn remove_world_water_sink_near_internal(
        &mut self,
        pos: Vector2,
        radius_m: f32,
    ) -> bool {
        self.remove_world_water_boundary_near_internal(
            AuthoredWaterBoundaryKind::Sink,
            pos,
            radius_m,
        )
    }

    /// Adds or updates one authored lake fill at the clicked terrain cell.
    pub(crate) fn add_world_lake_fill_internal(
        &mut self,
        pos: Vector2,
        surface_elevation_m: f32,
    ) -> Result<(), String> {
        if !surface_elevation_m.is_finite() {
            return Err("surface_elevation_m must be finite".to_owned());
        }

        let seed_height = self.heightmap.sample_height_world(pos.x, pos.y) * HEIGHT_SCALE;
        if surface_elevation_m <= seed_height {
            return Err("lake surface must be above the seed terrain height".to_owned());
        }

        let (world_x, world_z) = self.snap_world_position_to_terrain_cell(pos);
        if let Some(existing_idx) = self.world_lake_fill_index_at_position(world_x, world_z) {
            self.world_lake_fills[existing_idx].surface_elevation_m = surface_elevation_m;
        } else {
            self.world_lake_fills.push(AuthoredLakeFill {
                world_x,
                world_z,
                surface_elevation_m,
            });
        }

        debug_log!(
            "world-editor",
            "add_lake_fill world_x={:.1} world_z={:.1} surface={:.1}",
            world_x,
            world_z,
            surface_elevation_m
        );
        self.rebuild_authored_water_preview_internal()
    }

    /// Removes the nearest authored lake fill within the given radius.
    pub(crate) fn remove_world_lake_fill_near_internal(
        &mut self,
        pos: Vector2,
        radius_m: f32,
    ) -> bool {
        if !radius_m.is_finite() || radius_m <= 0.0 {
            return false;
        }
        let Some(idx) = nearest_lake_fill_index(&self.world_lake_fills, pos, radius_m) else {
            return false;
        };
        let removed = self.world_lake_fills.remove(idx);
        debug_log!(
            "world-editor",
            "remove_lake_fill world_x={:.1} world_z={:.1}",
            removed.world_x,
            removed.world_z
        );
        self.rebuild_authored_water_preview_internal().is_ok()
    }

    /// Rebuilds the runtime water preview from authored water records.
    pub(crate) fn rebuild_authored_water_preview_internal(&mut self) -> Result<(), String> {
        let mut water = WaterSystem::from_world_config(&self.config);
        if self.world_water_boundary_points.is_empty() && self.world_lake_fills.is_empty() {
            self.watermap = water;
            self.water_dirty = true;
            return Ok(());
        }

        let terrain_world = authored_water_terrain_world_heights(&self.heightmap);
        let mut depth = vec![0.0; terrain_world.len()];
        for lake in &self.world_lake_fills {
            let (seed_x, seed_z) = water.world_to_grid_cell_clamped(lake.world_x, lake.world_z);
            apply_lake_fill_to_depth(
                &terrain_world,
                water.width,
                water.height,
                seed_x,
                seed_z,
                lake.surface_elevation_m,
                &mut depth,
            );
        }
        water
            .replace_depth_from_dense(&depth)
            .map_err(|err| format!("failed to apply authored lake fills: {err}"))?;

        for point in &self.world_water_boundary_points {
            let (grid_x, grid_z) = water.world_to_grid_cell_clamped(point.world_x, point.world_z);
            let signed_rate = match point.kind {
                AuthoredWaterBoundaryKind::Source => point.rate_m_per_tick,
                AuthoredWaterBoundaryKind::Sink => -point.rate_m_per_tick,
            };
            water.update_source(grid_x, grid_z, signed_rate);
        }

        if !self.world_water_boundary_points.is_empty() {
            for _ in 0..AUTHORING_WATER_PREVIEW_STEPS {
                water.tick(&terrain_world, AUTHORING_WATER_PREVIEW_DT);
            }
        }

        self.watermap = water;
        self.water_dirty = true;
        Ok(())
    }

    /// Loads one reusable world-definition asset and resets runtime state to a fresh blank city.
    pub(crate) fn load_world_definition_internal(&mut self, path: &str) -> Result<(), String> {
        debug_log!("world-editor", "load_world_definition path={}", path);
        let loaded = load_world_definition_from_sqlite(&PathBuf::from(path))
            .map_err(|err| err.to_string())?;
        self.apply_loaded_world_definition(loaded);
        Ok(())
    }

    fn apply_loaded_world_definition(&mut self, loaded: LoadedWorldDefinition) {
        let LoadedWorldDefinition {
            name: _name,
            config,
            terrain,
            water_boundary_points,
            lake_fills,
        } = loaded;
        self.reset_to_blank_world_runtime(config, terrain);
        self.world_water_boundary_points = water_boundary_points;
        self.world_lake_fills = lake_fills;
        self.rebuild_authored_water_preview_internal()
            .expect("loaded world definition water preview should rebuild");
    }

    fn reset_to_blank_world_runtime(&mut self, config: WorldConfig, terrain: TerrainSystem) {
        let registry = self.allocator.registry.clone();
        debug_log!(
            "world-editor",
            "reset_blank_world_runtime width_m={:.1} height_m={:.1} terrain_cell_m={:.1} terrain_chunk_m={:.1} base_elevation_m={:.1}",
            config.width_m,
            config.height_m,
            config.terrain_cell_m,
            config.terrain_chunk_m,
            config.terrain_base_elevation_m
        );

        self.time = TimeSystem::new();
        self.config = config;
        self.heightmap = terrain;
        self.watermap = WaterSystem::from_world_config(&self.config);
        self.region_graph = crate::simulation::network::graph::RegionGraph::new();
        self.transit_network = TransitNetwork::new();
        self.zoning = ZoningSystem::new(&self.config);
        self.pollution = PollutionSystem::new(&self.config);
        self.noise = NoiseSystem::new(&self.config);
        self.desirability = DesirabilitySystem::new(&self.config);
        self.demand = DemandSystem::new();
        self.agents = AgentSystem::new();
        self.households = HouseholdSystem::new();
        self.logistics = ShipmentSystem::new();

        let mut allocator = BuildingAllocator::new();
        allocator.registry = registry;
        self.allocator = allocator;

        self.treasury = CityTreasury::new(startup_treasury_balance());
        self.undo_stack.clear();
        self.world_water_boundary_points.clear();
        self.world_lake_fills.clear();
        self.terrain_dirty = true;
        self.water_dirty = true;
        self.network_dirty = true;
        self.last_tick_duration = 0.0;
        self.last_agent_tick_us = 0;
        self.last_road_timing.clear();
        self.camera_aabb = (0.0, 0.0, 0.0, 0.0);
    }

    fn place_world_water_boundary_internal(
        &mut self,
        kind: AuthoredWaterBoundaryKind,
        pos: Vector2,
        rate_m_per_tick: f32,
    ) -> Result<(), String> {
        validate_positive_f32(rate_m_per_tick, "rate_m_per_tick")?;
        let (world_x, world_z) = self.snap_world_position_to_terrain_cell(pos);
        if let Some(existing_idx) =
            self.world_water_boundary_index_at_position(kind, world_x, world_z)
        {
            self.world_water_boundary_points[existing_idx].rate_m_per_tick += rate_m_per_tick;
        } else {
            self.world_water_boundary_points
                .push(AuthoredWaterBoundaryPoint {
                    kind,
                    world_x,
                    world_z,
                    rate_m_per_tick,
                });
        }

        debug_log!(
            "world-editor",
            "add_water_boundary kind={} world_x={:.1} world_z={:.1} rate={:.2}",
            match kind {
                AuthoredWaterBoundaryKind::Source => "source",
                AuthoredWaterBoundaryKind::Sink => "sink",
            },
            world_x,
            world_z,
            rate_m_per_tick
        );
        self.rebuild_authored_water_preview_internal()
    }

    fn remove_world_water_boundary_near_internal(
        &mut self,
        kind: AuthoredWaterBoundaryKind,
        pos: Vector2,
        radius_m: f32,
    ) -> bool {
        if !radius_m.is_finite() || radius_m <= 0.0 {
            return false;
        }
        let Some(idx) =
            nearest_water_boundary_index(&self.world_water_boundary_points, kind, pos, radius_m)
        else {
            return false;
        };
        let removed = self.world_water_boundary_points.remove(idx);
        debug_log!(
            "world-editor",
            "remove_water_boundary kind={} world_x={:.1} world_z={:.1}",
            match removed.kind {
                AuthoredWaterBoundaryKind::Source => "source",
                AuthoredWaterBoundaryKind::Sink => "sink",
            },
            removed.world_x,
            removed.world_z
        );
        self.rebuild_authored_water_preview_internal().is_ok()
    }

    fn snap_world_position_to_terrain_cell(&self, pos: Vector2) -> (f32, f32) {
        let (grid_x, grid_z) = self.watermap.world_to_grid_cell_clamped(pos.x, pos.y);
        self.heightmap.grid_to_world_coords(grid_x, grid_z)
    }

    fn world_water_boundary_index_at_position(
        &self,
        kind: AuthoredWaterBoundaryKind,
        world_x: f32,
        world_z: f32,
    ) -> Option<usize> {
        self.world_water_boundary_points.iter().position(|point| {
            point.kind == kind && point.world_x == world_x && point.world_z == world_z
        })
    }

    fn world_lake_fill_index_at_position(&self, world_x: f32, world_z: f32) -> Option<usize> {
        self.world_lake_fills
            .iter()
            .position(|lake| lake.world_x == world_x && lake.world_z == world_z)
    }
}

fn startup_treasury_balance() -> f64 {
    load_runtime_economy_tuning()
        .map(|tuning| tuning.startup_treasury_balance)
        .unwrap_or(100_000.0)
}

fn validate_positive_f32(value: f32, label: &str) -> Result<(), String> {
    if !value.is_finite() || value <= 0.0 {
        return Err(format!("{label} must be finite and > 0"));
    }
    Ok(())
}

fn authored_water_terrain_world_heights(terrain: &TerrainSystem) -> Vec<f32> {
    terrain
        .clone_source_dense()
        .into_iter()
        .map(|sample| sample * HEIGHT_SCALE)
        .collect()
}

fn apply_lake_fill_to_depth(
    terrain_world: &[f32],
    width: usize,
    height: usize,
    seed_x: usize,
    seed_z: usize,
    surface_elevation_m: f32,
    depth: &mut [f32],
) {
    if seed_x >= width || seed_z >= height || depth.len() != terrain_world.len() {
        return;
    }

    let seed_idx = seed_z * width + seed_x;
    if terrain_world[seed_idx] >= surface_elevation_m {
        return;
    }

    let mut visited = vec![false; width * height];
    let mut queue = VecDeque::new();
    visited[seed_idx] = true;
    queue.push_back((seed_x, seed_z));

    while let Some((x, z)) = queue.pop_front() {
        let idx = z * width + x;
        let terrain_height = terrain_world[idx];
        if terrain_height > surface_elevation_m {
            continue;
        }

        depth[idx] = depth[idx].max(surface_elevation_m - terrain_height);

        if x > 0 {
            enqueue_lake_fill_neighbor(
                x - 1,
                z,
                width,
                surface_elevation_m,
                terrain_world,
                &mut visited,
                &mut queue,
            );
        }
        if x + 1 < width {
            enqueue_lake_fill_neighbor(
                x + 1,
                z,
                width,
                surface_elevation_m,
                terrain_world,
                &mut visited,
                &mut queue,
            );
        }
        if z > 0 {
            enqueue_lake_fill_neighbor(
                x,
                z - 1,
                width,
                surface_elevation_m,
                terrain_world,
                &mut visited,
                &mut queue,
            );
        }
        if z + 1 < height {
            enqueue_lake_fill_neighbor(
                x,
                z + 1,
                width,
                surface_elevation_m,
                terrain_world,
                &mut visited,
                &mut queue,
            );
        }
    }
}

fn enqueue_lake_fill_neighbor(
    x: usize,
    z: usize,
    width: usize,
    surface_elevation_m: f32,
    terrain_world: &[f32],
    visited: &mut [bool],
    queue: &mut VecDeque<(usize, usize)>,
) {
    let idx = z * width + x;
    if visited[idx] || terrain_world[idx] > surface_elevation_m {
        return;
    }
    visited[idx] = true;
    queue.push_back((x, z));
}

fn nearest_water_boundary_index(
    points: &[AuthoredWaterBoundaryPoint],
    kind: AuthoredWaterBoundaryKind,
    pos: Vector2,
    radius_m: f32,
) -> Option<usize> {
    let radius_sq = radius_m * radius_m;
    let mut best: Option<(usize, f32)> = None;
    for (idx, point) in points.iter().enumerate() {
        if point.kind != kind {
            continue;
        }
        let dx = point.world_x - pos.x;
        let dz = point.world_z - pos.y;
        let dist_sq = dx * dx + dz * dz;
        if dist_sq > radius_sq {
            continue;
        }
        if best.is_none_or(|(_, best_dist_sq)| dist_sq < best_dist_sq) {
            best = Some((idx, dist_sq));
        }
    }
    best.map(|(idx, _)| idx)
}

fn nearest_lake_fill_index(
    lakes: &[AuthoredLakeFill],
    pos: Vector2,
    radius_m: f32,
) -> Option<usize> {
    let radius_sq = radius_m * radius_m;
    let mut best: Option<(usize, f32)> = None;
    for (idx, lake) in lakes.iter().enumerate() {
        let dx = lake.world_x - pos.x;
        let dz = lake.world_z - pos.y;
        let dist_sq = dx * dx + dz * dz;
        if dist_sq > radius_sq {
            continue;
        }
        if best.is_none_or(|(_, best_dist_sq)| dist_sq < best_dist_sq) {
            best = Some((idx, dist_sq));
        }
    }
    best.map(|(idx, _)| idx)
}

#[cfg(test)]
mod tests {
    use super::apply_lake_fill_to_depth;

    #[test]
    fn lake_fill_stays_inside_basin_below_surface() {
        let width = 5;
        let height = 5;
        let terrain_world = vec![
            10.0, 10.0, 10.0, 10.0, 10.0, //
            10.0, 2.0, 2.0, 2.0, 10.0, //
            10.0, 2.0, 1.0, 2.0, 10.0, //
            10.0, 2.0, 2.0, 2.0, 10.0, //
            10.0, 10.0, 10.0, 10.0, 10.0,
        ];
        let mut depth = vec![0.0; terrain_world.len()];

        apply_lake_fill_to_depth(&terrain_world, width, height, 2, 2, 5.0, &mut depth);

        assert_eq!(depth[2 * width + 2], 4.0);
        assert_eq!(depth[width + 1], 3.0);
        assert_eq!(depth[0], 0.0);
        assert_eq!(depth[4], 0.0);
        assert_eq!(depth[4 * width + 4], 0.0);
    }
}
