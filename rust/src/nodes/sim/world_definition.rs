//! Blank-world creation and reusable world-definition bridge methods.

use crate::config::HEIGHT_SCALE;
use crate::debug_log;
use crate::nodes::sim::core::{
    CityTreasury, SimCore, WorldLakeFillPreview, WorldLakeFillPreviewStatus, WorldWaterFillKind,
};
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
    AuthoredLakeFill, AuthoredOpenWaterFill, AuthoredWaterBoundaryKind, AuthoredWaterBoundaryPoint,
    LoadedWorldDefinition, WorldDefinitionView, load_world_definition_from_sqlite,
    save_world_definition_to_sqlite,
};
use godot::prelude::Vector2;
use std::collections::VecDeque;
use std::path::PathBuf;

const AUTHORING_WATER_PREVIEW_DT: f32 = 0.25;
const AUTHORING_WATER_PREVIEW_STEPS: usize = 48;

#[derive(Clone, Copy, Debug, Default)]
struct LakeFillApplication {
    touches_world_edge: bool,
    filled_cells: usize,
}

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
                open_water_fills: &self.world_open_water_fills,
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

    /// Starts one transient authored lake-fill preview at the clicked terrain cell.
    pub(crate) fn begin_world_lake_fill_preview_internal(
        &mut self,
        pos: Vector2,
        surface_elevation_m: f32,
    ) -> Result<WorldLakeFillPreview, String> {
        let (world_x, world_z) = self.snap_world_position_to_terrain_cell(pos);
        self.set_world_water_fill_preview_internal(
            WorldWaterFillKind::Lake,
            world_x,
            world_z,
            surface_elevation_m,
        )
    }

    /// Starts one transient authored open-water preview at the clicked terrain cell.
    pub(crate) fn begin_world_open_water_fill_preview_internal(
        &mut self,
        pos: Vector2,
        surface_elevation_m: f32,
    ) -> Result<WorldLakeFillPreview, String> {
        let (world_x, world_z) = self.snap_world_position_to_terrain_cell(pos);
        self.set_world_water_fill_preview_internal(
            WorldWaterFillKind::OpenWater,
            world_x,
            world_z,
            surface_elevation_m,
        )
    }

    /// Updates the active transient lake-fill preview surface.
    pub(crate) fn update_world_lake_fill_preview_internal(
        &mut self,
        surface_elevation_m: f32,
    ) -> Result<WorldLakeFillPreview, String> {
        self.update_world_water_fill_preview_internal(WorldWaterFillKind::Lake, surface_elevation_m)
    }

    /// Updates the active transient open-water preview surface.
    pub(crate) fn update_world_open_water_fill_preview_internal(
        &mut self,
        surface_elevation_m: f32,
    ) -> Result<WorldLakeFillPreview, String> {
        self.update_world_water_fill_preview_internal(
            WorldWaterFillKind::OpenWater,
            surface_elevation_m,
        )
    }

    fn update_world_water_fill_preview_internal(
        &mut self,
        expected_kind: WorldWaterFillKind,
        surface_elevation_m: f32,
    ) -> Result<WorldLakeFillPreview, String> {
        let Some(preview) = self.world_lake_fill_preview else {
            return Err("no world water fill preview is active".to_owned());
        };
        if preview.kind != expected_kind {
            return Err("active world water fill preview kind does not match tool".to_owned());
        }
        self.set_world_water_fill_preview_internal(
            expected_kind,
            preview.seed_world_x,
            preview.seed_world_z,
            surface_elevation_m,
        )
    }

    /// Commits the active transient lake-fill preview into authored world state.
    pub(crate) fn commit_world_lake_fill_preview_internal(&mut self) -> Result<(), String> {
        self.commit_world_water_fill_preview_internal(WorldWaterFillKind::Lake)
    }

    /// Commits the active transient open-water preview into authored world state.
    pub(crate) fn commit_world_open_water_fill_preview_internal(&mut self) -> Result<(), String> {
        self.commit_world_water_fill_preview_internal(WorldWaterFillKind::OpenWater)
    }

    fn commit_world_water_fill_preview_internal(
        &mut self,
        expected_kind: WorldWaterFillKind,
    ) -> Result<(), String> {
        let Some(preview) = self.world_lake_fill_preview else {
            return Err("no world water fill preview is active".to_owned());
        };
        if preview.kind != expected_kind {
            return Err("active world water fill preview kind does not match tool".to_owned());
        };
        if !preview.is_valid() {
            return Err(lake_fill_preview_status_message(preview.status).to_owned());
        }

        match preview.kind {
            WorldWaterFillKind::Lake => {
                if let Some(existing_idx) = self
                    .world_lake_fill_index_at_position(preview.seed_world_x, preview.seed_world_z)
                {
                    self.world_lake_fills[existing_idx].surface_elevation_m =
                        preview.surface_elevation_m;
                } else {
                    self.world_lake_fills.push(AuthoredLakeFill {
                        world_x: preview.seed_world_x,
                        world_z: preview.seed_world_z,
                        surface_elevation_m: preview.surface_elevation_m,
                    });
                }
            }
            WorldWaterFillKind::OpenWater => {
                if let Some(existing_idx) = self.world_open_water_fill_index_at_position(
                    preview.seed_world_x,
                    preview.seed_world_z,
                ) {
                    self.world_open_water_fills[existing_idx].surface_elevation_m =
                        preview.surface_elevation_m;
                } else {
                    self.world_open_water_fills.push(AuthoredOpenWaterFill {
                        world_x: preview.seed_world_x,
                        world_z: preview.seed_world_z,
                        surface_elevation_m: preview.surface_elevation_m,
                    });
                }
            }
        }

        debug_log!(
            "world-editor",
            "commit_{}_fill world_x={:.1} world_z={:.1} surface={:.1} cells={}",
            match preview.kind {
                WorldWaterFillKind::Lake => "lake",
                WorldWaterFillKind::OpenWater => "open_water",
            },
            preview.seed_world_x,
            preview.seed_world_z,
            preview.surface_elevation_m,
            preview.filled_cells
        );
        self.world_lake_fill_preview = None;
        self.rebuild_authored_water_preview_internal()
    }

    /// Cancels the active transient lake-fill preview.
    pub(crate) fn cancel_world_water_fill_preview_internal(&mut self) -> bool {
        if self.world_lake_fill_preview.is_none() {
            return false;
        }
        self.world_lake_fill_preview = None;
        self.rebuild_authored_water_preview_internal().is_ok()
    }

    /// Returns the active transient water-fill preview, if any.
    pub(crate) fn world_water_fill_preview_internal(&self) -> Option<WorldLakeFillPreview> {
        self.world_lake_fill_preview
    }

    fn set_world_water_fill_preview_internal(
        &mut self,
        kind: WorldWaterFillKind,
        world_x: f32,
        world_z: f32,
        surface_elevation_m: f32,
    ) -> Result<WorldLakeFillPreview, String> {
        if !surface_elevation_m.is_finite() {
            return Err("surface_elevation_m must be finite".to_owned());
        }

        let preview = evaluate_world_water_fill_preview(
            &self.heightmap,
            &self.watermap,
            kind,
            world_x,
            world_z,
            surface_elevation_m,
        );
        debug_log!(
            "world-editor",
            "preview_{}_fill world_x={:.1} world_z={:.1} surface={:.1} status={} cells={}",
            match preview.kind {
                WorldWaterFillKind::Lake => "lake",
                WorldWaterFillKind::OpenWater => "open_water",
            },
            preview.seed_world_x,
            preview.seed_world_z,
            preview.surface_elevation_m,
            lake_fill_preview_status_code(preview.status),
            preview.filled_cells
        );
        self.world_lake_fill_preview = Some(preview);
        self.rebuild_authored_water_preview_internal()?;
        Ok(preview)
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

    /// Removes the nearest authored open-water fill within the given radius.
    pub(crate) fn remove_world_open_water_fill_near_internal(
        &mut self,
        pos: Vector2,
        radius_m: f32,
    ) -> bool {
        if !radius_m.is_finite() || radius_m <= 0.0 {
            return false;
        }
        let Some(idx) = nearest_open_water_fill_index(&self.world_open_water_fills, pos, radius_m)
        else {
            return false;
        };
        let removed = self.world_open_water_fills.remove(idx);
        debug_log!(
            "world-editor",
            "remove_open_water_fill world_x={:.1} world_z={:.1}",
            removed.world_x,
            removed.world_z
        );
        self.rebuild_authored_water_preview_internal().is_ok()
    }

    /// Rebuilds the runtime water preview from authored water records.
    pub(crate) fn rebuild_authored_water_preview_internal(&mut self) -> Result<(), String> {
        let mut water = WaterSystem::from_world_config(&self.config);
        self.refresh_world_water_fill_preview_state_internal();
        if self.world_water_boundary_points.is_empty()
            && self.world_lake_fills.is_empty()
            && self.world_open_water_fills.is_empty()
            && self.world_lake_fill_preview.is_none()
        {
            self.watermap = water;
            self.water_dirty = true;
            return Ok(());
        }

        let terrain_world = authored_water_terrain_world_heights(&self.heightmap);
        let mut depth = vec![0.0; terrain_world.len()];
        for lake in &self.world_lake_fills {
            merge_surface_fill_depth(
                &terrain_world,
                &water,
                &mut depth,
                WorldWaterFillKind::Lake,
                lake.world_x,
                lake.world_z,
                lake.surface_elevation_m,
                "skip_lake_fill",
            );
        }
        for open_water in &self.world_open_water_fills {
            merge_surface_fill_depth(
                &terrain_world,
                &water,
                &mut depth,
                WorldWaterFillKind::OpenWater,
                open_water.world_x,
                open_water.world_z,
                open_water.surface_elevation_m,
                "skip_open_water_fill",
            );
        }
        if let Some(preview) = self.world_lake_fill_preview {
            if preview.is_valid() {
                merge_surface_fill_depth(
                    &terrain_world,
                    &water,
                    &mut depth,
                    preview.kind,
                    preview.seed_world_x,
                    preview.seed_world_z,
                    preview.surface_elevation_m,
                    match preview.kind {
                        WorldWaterFillKind::Lake => "skip_lake_fill_preview",
                        WorldWaterFillKind::OpenWater => "skip_open_water_fill_preview",
                    },
                );
            }
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
            open_water_fills,
        } = loaded;
        self.reset_to_blank_world_runtime(config, terrain);
        self.world_water_boundary_points = water_boundary_points;
        self.world_lake_fills = lake_fills;
        self.world_open_water_fills = open_water_fills;
        self.world_lake_fill_preview = None;
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
        self.world_open_water_fills.clear();
        self.world_lake_fill_preview = None;
        self.terrain_stroke_active = false;
        self.terrain_stroke_has_changes = false;
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

    fn world_open_water_fill_index_at_position(&self, world_x: f32, world_z: f32) -> Option<usize> {
        self.world_open_water_fills
            .iter()
            .position(|water| water.world_x == world_x && water.world_z == world_z)
    }

    pub(crate) fn has_authored_water_internal(&self) -> bool {
        !self.world_water_boundary_points.is_empty()
            || !self.world_lake_fills.is_empty()
            || !self.world_open_water_fills.is_empty()
            || self.world_lake_fill_preview.is_some()
    }

    fn refresh_world_water_fill_preview_state_internal(&mut self) {
        let Some(preview) = self.world_lake_fill_preview else {
            return;
        };
        self.world_lake_fill_preview = Some(evaluate_world_water_fill_preview(
            &self.heightmap,
            &self.watermap,
            preview.kind,
            preview.seed_world_x,
            preview.seed_world_z,
            preview.surface_elevation_m,
        ));
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
    terrain.clone_source_dense_world_heights()
}

fn evaluate_world_water_fill_preview(
    terrain: &TerrainSystem,
    water: &WaterSystem,
    kind: WorldWaterFillKind,
    world_x: f32,
    world_z: f32,
    surface_elevation_m: f32,
) -> WorldLakeFillPreview {
    let seed_height_m = terrain.sample_height_world(world_x, world_z) * HEIGHT_SCALE;
    if surface_elevation_m <= seed_height_m {
        return WorldLakeFillPreview {
            kind,
            seed_world_x: world_x,
            seed_world_z: world_z,
            seed_height_m,
            surface_elevation_m,
            status: WorldLakeFillPreviewStatus::SurfaceBelowSeedTerrain,
            filled_cells: 0,
        };
    }

    let terrain_world = authored_water_terrain_world_heights(terrain);
    let (seed_x, seed_z) = water.world_to_grid_cell_clamped(world_x, world_z);
    let mut preview_depth = vec![0.0; terrain_world.len()];
    let application = apply_lake_fill_to_depth(
        &terrain_world,
        water.width,
        water.height,
        seed_x,
        seed_z,
        surface_elevation_m,
        &mut preview_depth,
    );
    let status = match kind {
        WorldWaterFillKind::Lake => {
            if application.touches_world_edge {
                WorldLakeFillPreviewStatus::EscapesWorldEdge
            } else {
                WorldLakeFillPreviewStatus::Ready
            }
        }
        WorldWaterFillKind::OpenWater => {
            if application.touches_world_edge {
                WorldLakeFillPreviewStatus::Ready
            } else {
                WorldLakeFillPreviewStatus::DoesNotReachWorldEdge
            }
        }
    };

    WorldLakeFillPreview {
        kind,
        seed_world_x: world_x,
        seed_world_z: world_z,
        seed_height_m,
        surface_elevation_m,
        status,
        filled_cells: application.filled_cells,
    }
}

fn merge_surface_fill_depth(
    terrain_world: &[f32],
    water: &WaterSystem,
    depth: &mut [f32],
    kind: WorldWaterFillKind,
    world_x: f32,
    world_z: f32,
    surface_elevation_m: f32,
    skip_log_label: &str,
) {
    let (seed_x, seed_z) = water.world_to_grid_cell_clamped(world_x, world_z);
    let mut lake_depth = vec![0.0; depth.len()];
    let application = apply_lake_fill_to_depth(
        terrain_world,
        water.width,
        water.height,
        seed_x,
        seed_z,
        surface_elevation_m,
        &mut lake_depth,
    );
    if application.touches_world_edge {
        if kind == WorldWaterFillKind::OpenWater {
            for (dst, src) in depth.iter_mut().zip(lake_depth.into_iter()) {
                *dst = f32::max(*dst, src);
            }
            return;
        }
        debug_log!(
            "world-editor",
            "{} world_x={:.1} world_z={:.1} surface={:.1} reason=edge_escape",
            skip_log_label,
            world_x,
            world_z,
            surface_elevation_m
        );
        return;
    }
    if kind == WorldWaterFillKind::OpenWater {
        debug_log!(
            "world-editor",
            "{} world_x={:.1} world_z={:.1} surface={:.1} reason=not_edge_connected",
            skip_log_label,
            world_x,
            world_z,
            surface_elevation_m
        );
        return;
    }
    for (dst, src) in depth.iter_mut().zip(lake_depth.into_iter()) {
        *dst = f32::max(*dst, src);
    }
}

fn lake_fill_preview_status_code(status: WorldLakeFillPreviewStatus) -> &'static str {
    match status {
        WorldLakeFillPreviewStatus::Ready => "ready",
        WorldLakeFillPreviewStatus::SurfaceBelowSeedTerrain => "below_seed",
        WorldLakeFillPreviewStatus::EscapesWorldEdge => "edge_escape",
        WorldLakeFillPreviewStatus::DoesNotReachWorldEdge => "not_edge_connected",
    }
}

fn lake_fill_preview_status_message(status: WorldLakeFillPreviewStatus) -> &'static str {
    match status {
        WorldLakeFillPreviewStatus::Ready => "lake preview ready",
        WorldLakeFillPreviewStatus::SurfaceBelowSeedTerrain => {
            "lake surface must be above the seed terrain height"
        }
        WorldLakeFillPreviewStatus::EscapesWorldEdge => {
            "lake fill escapes the basin and reaches the world edge"
        }
        WorldLakeFillPreviewStatus::DoesNotReachWorldEdge => {
            "open water fill does not connect to the world edge"
        }
    }
}

fn apply_lake_fill_to_depth(
    terrain_world: &[f32],
    width: usize,
    height: usize,
    seed_x: usize,
    seed_z: usize,
    surface_elevation_m: f32,
    depth: &mut [f32],
) -> LakeFillApplication {
    if seed_x >= width || seed_z >= height || depth.len() != terrain_world.len() {
        return LakeFillApplication::default();
    }

    let seed_idx = seed_z * width + seed_x;
    if terrain_world[seed_idx] >= surface_elevation_m {
        return LakeFillApplication::default();
    }

    let mut visited = vec![false; width * height];
    let mut queue = VecDeque::new();
    let mut application = LakeFillApplication::default();
    visited[seed_idx] = true;
    queue.push_back((seed_x, seed_z));

    while let Some((x, z)) = queue.pop_front() {
        let idx = z * width + x;
        let terrain_height = terrain_world[idx];
        if terrain_height > surface_elevation_m {
            continue;
        }

        if x == 0 || z == 0 || x + 1 == width || z + 1 == height {
            application.touches_world_edge = true;
        }

        depth[idx] = depth[idx].max(surface_elevation_m - terrain_height);
        application.filled_cells += 1;

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

    application
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

fn nearest_open_water_fill_index(
    waters: &[AuthoredOpenWaterFill],
    pos: Vector2,
    radius_m: f32,
) -> Option<usize> {
    let radius_sq = radius_m * radius_m;
    let mut best: Option<(usize, f32)> = None;
    for (idx, water) in waters.iter().enumerate() {
        let dx = water.world_x - pos.x;
        let dz = water.world_z - pos.y;
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
    use super::{apply_lake_fill_to_depth, evaluate_world_water_fill_preview};
    use crate::nodes::sim::core::{
        CityTreasury, SimCore, WorldLakeFillPreviewStatus, WorldWaterFillKind,
    };
    use crate::simulation::buildings::allocator::BuildingAllocator;
    use crate::simulation::core::config::WorldConfig;
    use crate::simulation::core::time::TimeSystem;
    use crate::simulation::economy::agents::AgentSystem;
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
    use godot::prelude::Vector2;
    use std::collections::VecDeque;

    fn test_core_with_small_world() -> SimCore {
        let config = WorldConfig::new(40.0, 40.0, 40.0, 10.0)
            .with_terrain_resolution(10.0)
            .with_chunking(512.0, 0.0);
        SimCore {
            time: TimeSystem::new(),
            heightmap: TerrainSystem::from_world_config(&config),
            watermap: WaterSystem::from_world_config(&config),
            region_graph: crate::simulation::network::graph::RegionGraph::new(),
            transit_network: TransitNetwork::new(),
            zoning: ZoningSystem::new(&config),
            pollution: PollutionSystem::new(&config),
            noise: NoiseSystem::new(&config),
            desirability: DesirabilitySystem::new(&config),
            demand: DemandSystem::new(),
            allocator: BuildingAllocator::new(),
            agents: AgentSystem::new(),
            households: HouseholdSystem::new(),
            logistics: ShipmentSystem::new(),
            config,
            treasury: CityTreasury::new(0.0),
            undo_stack: VecDeque::new(),
            world_water_boundary_points: Vec::new(),
            world_lake_fills: Vec::new(),
            world_open_water_fills: Vec::new(),
            world_lake_fill_preview: None,
            terrain_stroke_active: false,
            terrain_stroke_has_changes: false,
            water_runtime_realtime_when_paused: true,
            terrain_dirty: false,
            water_dirty: false,
            network_dirty: false,
            benchmark_mode: true,
            last_tick_duration: 0.0,
            last_agent_tick_us: 0,
            last_road_timing: String::new(),
            camera_aabb: (0.0, 0.0, 0.0, 0.0),
        }
    }

    fn carve_closed_basin(core: &mut SimCore) {
        let heights = [
            [10.0, 10.0, 10.0, 10.0, 10.0],
            [10.0, 2.0, 2.0, 2.0, 10.0],
            [10.0, 2.0, 1.0, 2.0, 10.0],
            [10.0, 2.0, 2.0, 2.0, 10.0],
            [10.0, 10.0, 10.0, 10.0, 10.0],
        ];
        for (z, row) in heights.iter().enumerate() {
            for (x, height) in row.iter().copied().enumerate() {
                core.heightmap.set_height(x, z, height);
            }
        }
    }

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

        let application =
            apply_lake_fill_to_depth(&terrain_world, width, height, 2, 2, 5.0, &mut depth);

        assert!(!application.touches_world_edge);
        assert_eq!(depth[2 * width + 2], 4.0);
        assert_eq!(depth[width + 1], 3.0);
        assert_eq!(depth[0], 0.0);
        assert_eq!(depth[4], 0.0);
        assert_eq!(depth[4 * width + 4], 0.0);
    }

    #[test]
    fn lake_fill_reports_when_basin_reaches_world_edge() {
        let width = 5;
        let height = 5;
        let terrain_world = vec![
            1.0, 1.0, 1.0, 1.0, 1.0, //
            1.0, 2.0, 2.0, 2.0, 1.0, //
            1.0, 2.0, 3.0, 2.0, 1.0, //
            1.0, 2.0, 2.0, 2.0, 1.0, //
            1.0, 1.0, 1.0, 1.0, 1.0,
        ];
        let mut depth = vec![0.0; terrain_world.len()];

        let application =
            apply_lake_fill_to_depth(&terrain_world, width, height, 2, 2, 3.5, &mut depth);

        assert!(application.touches_world_edge);
        assert!(application.filled_cells > 0);
    }

    #[test]
    fn lake_fill_preview_below_seed_is_invalid() {
        let config = WorldConfig::new(40.0, 40.0, 40.0, 10.0)
            .with_terrain_resolution(10.0)
            .with_chunking(512.0, 0.0);
        let terrain = TerrainSystem::from_world_config(&config);
        let water = WaterSystem::from_world_config(&config);

        let preview = evaluate_world_water_fill_preview(
            &terrain,
            &water,
            WorldWaterFillKind::Lake,
            0.0,
            0.0,
            0.0,
        );

        assert_eq!(
            preview.status,
            WorldLakeFillPreviewStatus::SurfaceBelowSeedTerrain
        );
        assert_eq!(preview.filled_cells, 0);
    }

    #[test]
    fn lake_fill_preview_stays_transient_until_commit() {
        let mut core = test_core_with_small_world();
        carve_closed_basin(&mut core);

        let preview = core
            .begin_world_lake_fill_preview_internal(Vector2::ZERO, 100.0)
            .expect("preview should start");

        assert_eq!(preview.status, WorldLakeFillPreviewStatus::Ready);
        assert!(core.world_lake_fill_preview.is_some());
        assert!(core.world_lake_fills.is_empty());
        assert!(
            core.watermap
                .clone_depth_dense()
                .iter()
                .any(|depth| *depth > 0.0)
        );

        core.commit_world_lake_fill_preview_internal()
            .expect("preview should commit");

        assert!(core.world_lake_fill_preview.is_none());
        assert_eq!(core.world_lake_fills.len(), 1);
    }

    #[test]
    fn invalid_lake_fill_preview_does_not_modify_water_depth() {
        let mut core = test_core_with_small_world();
        let preview = core
            .begin_world_lake_fill_preview_internal(Vector2::ZERO, 5.0)
            .expect("preview should start");

        assert_eq!(preview.status, WorldLakeFillPreviewStatus::EscapesWorldEdge);
        assert!(core.world_lake_fill_preview.is_some());
        assert!(core.world_lake_fills.is_empty());
        assert!(
            core.watermap
                .clone_depth_dense()
                .iter()
                .all(|depth| *depth == 0.0)
        );
    }

    #[test]
    fn open_water_preview_requires_world_edge_connection() {
        let mut core = test_core_with_small_world();
        carve_closed_basin(&mut core);

        let preview = core
            .begin_world_open_water_fill_preview_internal(Vector2::ZERO, 100.0)
            .expect("preview should start");

        assert_eq!(
            preview.status,
            WorldLakeFillPreviewStatus::DoesNotReachWorldEdge
        );
        assert!(core.world_open_water_fills.is_empty());
    }
}
