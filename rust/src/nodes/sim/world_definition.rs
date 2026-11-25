// SPDX-License-Identifier: GPL-3.0-or-later

//! Blank-world creation and reusable world-definition bridge methods.

use crate::debug_log;
use crate::nodes::sim::core::{
    AuthoredWaterPatchFillDebug, CityTreasury, SimCore, WorldLakeFillPreview,
    WorldLakeFillPreviewStatus, WorldWaterFillKind,
};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::core::time::TimeSystem;
use crate::simulation::core::weather::WeatherState;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::demand::DemandSystem;
use crate::simulation::economy::households::HouseholdSystem;
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::grid::desirability::DesirabilitySystem;
use crate::simulation::grid::noise::NoiseSystem;
use crate::simulation::grid::pollution::PollutionSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::resources::{RESOURCE_RICHNESS_MAX, ResourceDepositSystem};
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::vegetation::{VegetationConfig, VegetationGenerator};
use crate::simulation::water::WaterSystem;
use crate::simulation::world_definition::{
    AuthoredLakeFill, AuthoredOpenWaterFill, LoadedWorldDefinition, WorldDefinitionView,
    load_world_definition_from_sqlite, save_world_definition_to_sqlite,
};
use crate::simulation::zoning::ZoningSystem;
use godot::prelude::Vector2;
use std::collections::HashMap;
use std::path::PathBuf;
use water_fill::{FillDebugSource, FillFlood, PatchFillDebug};

mod water_fill;

impl SimCore {
    /// Resets the live runtime to one fresh blank world with the given terrain settings.
    ///
    /// The world rules apply (`WorldConfig::validate`): a cell of 2, 4 or 8 m and sides that
    /// are whole multiples of 128 m; the patch and storage chunk are always 512 m.
    pub(crate) fn create_blank_world_internal(
        &mut self,
        width_m: f32,
        height_m: f32,
        terrain_cell_m: f32,
        base_elevation_m: f32,
    ) -> Result<(), String> {
        // The reset's rebuilds are parallel: shared pool (CODE-17).
        crate::thread_pools::install(|| {
            self.create_blank_world_in_pool(width_m, height_m, terrain_cell_m, base_elevation_m)
        })
    }

    fn create_blank_world_in_pool(
        &mut self,
        width_m: f32,
        height_m: f32,
        terrain_cell_m: f32,
        base_elevation_m: f32,
    ) -> Result<(), String> {
        let config = WorldConfig::new(
            width_m,
            height_m,
            self.config.env_cell_m,
            self.config.zone_cell_m,
        )
        .with_terrain_resolution(terrain_cell_m)
        .with_base_elevation(base_elevation_m);
        config.validate()?;
        debug_log!(
            "world-editor",
            "create_blank_world width_m={:.1} height_m={:.1} terrain_cell_m={:.1} base_elevation_m={:.1}",
            width_m,
            height_m,
            terrain_cell_m,
            base_elevation_m
        );
        let terrain = TerrainSystem::from_world_config(&config);
        // The world editor authors land, not a game, so it takes the shipped forest.
        self.reset_to_blank_world_runtime(config, terrain, VegetationConfig::default());
        self.precompute_road_mesh_data();
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
            "save_world_definition path={} name={} world=({:.1}m x {:.1}m) terrain_cell_m={:.1}",
            path,
            name,
            self.config.width_m,
            self.config.height_m,
            self.config.terrain_cell_m
        );
        save_world_definition_to_sqlite(
            &PathBuf::from(path),
            WorldDefinitionView {
                name,
                config: &self.config,
                terrain: &self.heightmap,
                lake_fills: &self.world_lake_fills,
                open_water_fills: &self.world_open_water_fills,
                resource_deposits: &self.resource_deposits,
            },
        )
        .map_err(|err| err.to_string())
    }

    /// Paints authored coal deposit richness into the world.
    pub(crate) fn paint_world_coal_deposit_internal(
        &mut self,
        pos: Vector2,
        radius_m: f32,
        richness_percent: f32,
    ) -> bool {
        let richness = richness_percent_to_storage(richness_percent);
        self.resource_deposits
            .paint_coal_circle_world(pos.x, pos.y, radius_m, richness)
    }

    /// Erases authored coal deposit richness from the world.
    pub(crate) fn erase_world_coal_deposit_internal(
        &mut self,
        pos: Vector2,
        radius_m: f32,
    ) -> bool {
        self.resource_deposits
            .erase_coal_circle_world(pos.x, pos.y, radius_m)
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
        // A commit's deferred lots first: this edit reads or writes them (`ROAD-68-G` part 7).
        self.adopt_pending_cell_lots();
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

        let terrain_world = self.heightmap.clone_source_dense();
        let preview = evaluate_world_water_fill_preview(
            &self.heightmap,
            &terrain_world,
            &mut FillFlood::new(terrain_world.len()),
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
    ///
    /// Cost (`A4c`): `O(W)` for the `W` world samples, each touched by one pass: the dense
    /// terrain copy, the zeroed depth buffer, and the sparse rebuild of the result; plus
    /// `O(k)` per fill for its `k` flooded samples (the flood, the depth merge and the debug
    /// cache's row-major sort, `O(k + 4096)` per sort pass), plus `O(fills)`. The visit stamps
    /// and work lists are allocated once and reused by every fill; no fill allocates, clears or
    /// scans a world-sized buffer. Every edit re-solves every fill.
    pub(crate) fn rebuild_authored_water_preview_internal(&mut self) -> Result<(), String> {
        let mut water = WaterSystem::from_world_config(&self.config);
        if self.world_lake_fills.is_empty()
            && self.world_open_water_fills.is_empty()
            && self.world_lake_fill_preview.is_none()
        {
            self.watermap = water;
            self.authored_water_patch_fill_debug_cache.clear();
            self.water_dirty = true;
            self.bump_road_tool_query_generation();
            return Ok(());
        }

        let terrain_world = self.heightmap.clone_source_dense();
        let mut flood = FillFlood::new(terrain_world.len());
        self.refresh_world_water_fill_preview_state_internal(&terrain_world, &mut flood);
        let mut solve = FillSolve {
            terrain_world: &terrain_world,
            water: &water,
            flood,
            patch_debug: PatchFillDebug::new(&water),
            depth: vec![0.0; terrain_world.len()],
            debug_cache: HashMap::new(),
        };
        let source =
            |kind, fill_index: usize, world_x, world_z, surface_elevation_m| FillDebugSource {
                kind,
                fill_index: fill_index as i32,
                preview: false,
                world_x,
                world_z,
                surface_elevation_m,
            };
        for (fill_index, lake) in self.world_lake_fills.iter().enumerate() {
            solve.add_fill(
                source(
                    WorldWaterFillKind::Lake,
                    fill_index,
                    lake.world_x,
                    lake.world_z,
                    lake.surface_elevation_m,
                ),
                "skip_lake_fill",
            );
        }
        for (fill_index, open_water) in self.world_open_water_fills.iter().enumerate() {
            solve.add_fill(
                source(
                    WorldWaterFillKind::OpenWater,
                    fill_index,
                    open_water.world_x,
                    open_water.world_z,
                    open_water.surface_elevation_m,
                ),
                "skip_open_water_fill",
            );
        }
        if let Some(preview) = self.world_lake_fill_preview
            && preview.is_valid()
        {
            solve.add_fill(
                FillDebugSource {
                    kind: preview.kind,
                    fill_index: -1,
                    preview: true,
                    world_x: preview.seed_world_x,
                    world_z: preview.seed_world_z,
                    surface_elevation_m: preview.surface_elevation_m,
                },
                match preview.kind {
                    WorldWaterFillKind::Lake => "skip_lake_fill_preview",
                    WorldWaterFillKind::OpenWater => "skip_open_water_fill_preview",
                },
            );
        }
        let FillSolve {
            depth,
            mut debug_cache,
            ..
        } = solve;
        for contributors in debug_cache.values_mut() {
            sort_authored_water_patch_fill_debug(contributors);
        }
        water
            .replace_baseline_depth_from_dense(&depth)
            .map_err(|err| format!("failed to apply authored baseline water: {err}"))?;

        self.watermap = water;
        self.authored_water_patch_fill_debug_cache = debug_cache;
        self.water_dirty = true;
        self.bump_road_tool_query_generation();
        Ok(())
    }

    /// Loads one reusable world-definition asset and resets runtime state to a fresh blank city.
    ///
    /// The vegetation parameters are supplied by the caller rather than read from the asset: a
    /// world definition owns the land, while the forest standing on it at the first tick is a
    /// property of the game being started, so one authored world serves an empty start and a
    /// dense one.
    pub(crate) fn load_world_definition_internal(
        &mut self,
        path: &str,
        vegetation: VegetationConfig,
    ) -> Result<(), String> {
        debug_log!("world-editor", "load_world_definition path={}", path);
        // Decoding and the reset's rebuilds are parallel: shared pool (CODE-17).
        crate::thread_pools::install(|| {
            let loaded = load_world_definition_from_sqlite(&PathBuf::from(path))
                .map_err(|err| err.to_string())?;
            self.apply_loaded_world_definition(loaded, vegetation);
            Ok(())
        })
    }

    fn apply_loaded_world_definition(
        &mut self,
        loaded: LoadedWorldDefinition,
        vegetation: VegetationConfig,
    ) {
        let LoadedWorldDefinition {
            name: _name,
            config,
            terrain,
            lake_fills,
            open_water_fills,
            resource_deposits,
        } = loaded;
        self.reset_to_blank_world_runtime(config, terrain, vegetation);
        self.world_lake_fills = lake_fills;
        self.world_open_water_fills = open_water_fills;
        self.resource_deposits = resource_deposits;
        self.world_lake_fill_preview = None;
        self.rebuild_authored_water_preview_internal()
            .expect("loaded world definition water preview should rebuild");
        self.precompute_road_mesh_data();
    }

    fn reset_to_blank_world_runtime(
        &mut self,
        config: WorldConfig,
        terrain: TerrainSystem,
        vegetation: VegetationConfig,
    ) {
        debug_log!(
            "world-editor",
            "reset_blank_world_runtime width_m={:.1} height_m={:.1} terrain_cell_m={:.1} base_elevation_m={:.1}",
            config.width_m,
            config.height_m,
            config.terrain_cell_m,
            config.terrain_base_elevation_m
        );

        // A commit's deferred lots belong to the world the reset replaces. Dropped here, past
        // every refusal: a refused load or create leaves the running world with its plan.
        self.pending_cell_lots = None;
        self.time = TimeSystem::new();
        self.weather = WeatherState::default();
        self.world_epoch = self.world_epoch.wrapping_add(1);
        self.config = config;
        self.heightmap = terrain;
        // Resolved against the new extent rather than carried over: coverage is an area
        // fraction of this world, and stand size is compressed on a world too small to hold
        // one, so a generator resolved for a different extent is wrong for this one.
        self.vegetation = VegetationGenerator::resolve(vegetation.sanitized(), &self.config);
        self.vegetation_edits = Default::default();
        self.watermap = WaterSystem::from_world_config(&self.config);
        self.region_graph = crate::simulation::network::graph::RegionGraph::new();
        self.parking = Default::default();
        self.transit_network = TransitNetwork::new_for_world(&self.config);
        self.zoning = ZoningSystem::new(&self.config);
        self.pollution = PollutionSystem::new(&self.config);
        self.noise = NoiseSystem::new(&self.config);
        self.desirability = DesirabilitySystem::new(&self.config);
        self.environment_sweep = Default::default();
        self.demand = DemandSystem::new();
        self.pending_demand_spawns.clear();
        self.next_spawn_id = 0;
        self.events.reset(self.time.absolute_slot() + 1);
        self.agents = AgentSystem::new();
        self.households = HouseholdSystem::new();
        self.logistics = ShipmentSystem::new();

        let mut allocator = BuildingAllocator::new();
        std::mem::swap(&mut allocator.registry, &mut self.allocator.registry);
        self.allocator = allocator;
        self.allocator.now_slot = self.time.absolute_slot();

        self.treasury = CityTreasury::default();
        self.service_policy = Default::default();
        self.fiscal_policy = Default::default();
        self.budget_history.clear();
        self.debug_household_admissions_since_daily = 0;
        self.resource_deposits = ResourceDepositSystem::from_world_config(&self.config);
        self.resource_extraction.clear();
        self.agriculture.clear();
        self.clear_world_transient_state();
        // World replacement explicitly rebuilds renderer residency. Dirty flags describe actual
        // patch ledgers; an empty flat world has no payload acknowledgements to clear a forced flag.
        self.terrain_dirty = !self.heightmap.dirty_render_patches().is_empty();
        self.water_dirty = !self.watermap.dirty_render_patches().is_empty();
        self.mark_network_render_dirty();
    }

    /// Drops state tied to the previous world before the caller publishes fresh render generations.
    pub(crate) fn clear_world_transient_state(&mut self) {
        self.undo_stack.clear();
        self.world_lake_fills.clear();
        self.world_open_water_fills.clear();
        self.world_lake_fill_preview = None;
        self.authored_water_patch_fill_debug_cache.clear();
        self.terrain_payload_patch_generations.clear();
        self.road_terrain_inputs.note_all();
        self.terrain_stroke_active = false;
        self.terrain_stroke_has_changes = false;
        self.last_tick_duration = 0.0;
        self.last_agent_tick_us = 0;
        self.last_road_timing.clear();
        self.last_road_edit_metrics = Default::default();
        self.last_surface_debug_edges.clear();
        self.camera_aabb = (0.0, 0.0, 0.0, 0.0);
        self.render_camera_position = None;
        self.agent_draw_eye = None;
    }

    fn snap_world_position_to_terrain_cell(&self, pos: Vector2) -> (f32, f32) {
        let (grid_x, grid_z) = self.watermap.world_to_grid_cell_clamped(pos.x, pos.y);
        self.heightmap.grid_to_world_coords(grid_x, grid_z)
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

    /// Brings the authored water up to date with the terrain after anything wrote it: a brush
    /// edit or stroke, and the undo that restores the terrain from its record (`TERRAIN-37`).
    /// The one water update every terrain change goes through.
    pub(crate) fn refresh_authored_water_after_terrain_change(&mut self) {
        if self.has_authored_water_internal()
            && let Err(err) = self.rebuild_authored_water_preview_internal()
        {
            debug_log!(
                "world-editor",
                "rebuild_authored_water_after_terrain_change failed: {}",
                err
            );
        }
    }

    pub(crate) fn has_authored_water_internal(&self) -> bool {
        !self.world_lake_fills.is_empty()
            || !self.world_open_water_fills.is_empty()
            || self.world_lake_fill_preview.is_some()
    }

    /// Returns debug-only authored fill contributors for one visible water render patch.
    pub(crate) fn authored_water_patch_fill_debug_internal(
        &self,
        patch_x: usize,
        patch_z: usize,
    ) -> Vec<AuthoredWaterPatchFillDebug> {
        self.authored_water_patch_fill_debug_cache
            .get(&(patch_x, patch_z))
            .cloned()
            .unwrap_or_default()
    }

    fn refresh_world_water_fill_preview_state_internal(
        &mut self,
        terrain_world: &[f32],
        flood: &mut FillFlood,
    ) {
        let Some(preview) = self.world_lake_fill_preview else {
            return;
        };
        self.world_lake_fill_preview = Some(evaluate_world_water_fill_preview(
            &self.heightmap,
            terrain_world,
            flood,
            &self.watermap,
            preview.kind,
            preview.seed_world_x,
            preview.seed_world_z,
            preview.surface_elevation_m,
        ));
    }
}

fn richness_percent_to_storage(richness_percent: f32) -> u16 {
    if !richness_percent.is_finite() {
        return 0;
    }
    ((richness_percent.clamp(0.0, 100.0) / 100.0) * f32::from(RESOURCE_RICHNESS_MAX)).round() as u16
}

// Evaluates a transient fill preview: one flood, `O(k)` with the caller's dense terrain and
// scratch.
#[allow(clippy::too_many_arguments)]
fn evaluate_world_water_fill_preview(
    terrain: &TerrainSystem,
    terrain_world: &[f32],
    flood: &mut FillFlood,
    water: &WaterSystem,
    kind: WorldWaterFillKind,
    world_x: f32,
    world_z: f32,
    surface_elevation_m: f32,
) -> WorldLakeFillPreview {
    let seed_height_m = terrain.sample_height_world(world_x, world_z);
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

    let (seed_x, seed_z) = water.world_to_grid_cell_clamped(world_x, world_z);
    let application = flood.flood(
        terrain_world,
        water.width,
        water.height,
        seed_x,
        seed_z,
        surface_elevation_m,
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

// One water solve: the dense inputs and outputs and the scratch its fills share.
struct FillSolve<'a> {
    terrain_world: &'a [f32],
    water: &'a WaterSystem,
    flood: FillFlood,
    patch_debug: PatchFillDebug,
    depth: Vec<f32>,
    debug_cache: HashMap<(usize, usize), Vec<AuthoredWaterPatchFillDebug>>,
}

impl FillSolve<'_> {
    // Floods one fill and merges it when its kind accepts where it reached: a lake must stay
    // inside the world, open water must reach the world edge. An accepted fill adds its depth
    // and its debug contributions. `O(k)` for its `k` flooded samples.
    fn add_fill(&mut self, source: FillDebugSource, skip_log_label: &str) {
        let (seed_x, seed_z) = self
            .water
            .world_to_grid_cell_clamped(source.world_x, source.world_z);
        let application = self.flood.flood(
            self.terrain_world,
            self.water.width,
            self.water.height,
            seed_x,
            seed_z,
            source.surface_elevation_m,
        );
        let skip_reason = match (source.kind, application.touches_world_edge) {
            (WorldWaterFillKind::Lake, true) => Some("edge_escape"),
            (WorldWaterFillKind::OpenWater, false) => Some("not_edge_connected"),
            _ => None,
        };
        if let Some(reason) = skip_reason {
            debug_log!(
                "world-editor",
                "{} world_x={:.1} world_z={:.1} surface={:.1} reason={}",
                skip_log_label,
                source.world_x,
                source.world_z,
                source.surface_elevation_m,
                reason
            );
            return;
        }
        self.flood.merge_depth(
            self.terrain_world,
            source.surface_elevation_m,
            &mut self.depth,
        );
        self.patch_debug.push_fill(
            &mut self.debug_cache,
            &mut self.flood,
            self.terrain_world,
            self.water,
            source,
            application,
        );
    }
}

fn sort_authored_water_patch_fill_debug(contributors: &mut [AuthoredWaterPatchFillDebug]) {
    contributors.sort_by(|a, b| {
        b.patch_sum_depth_m
            .total_cmp(&a.patch_sum_depth_m)
            .then_with(|| b.patch_nonzero_samples.cmp(&a.patch_nonzero_samples))
            .then_with(|| a.fill_index.cmp(&b.fill_index))
    });
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
    use super::evaluate_world_water_fill_preview;
    use super::water_fill::{FillFlood, LakeFillApplication};
    use crate::nodes::sim::core::{
        CityTreasury, SimCore, WorldLakeFillPreviewStatus, WorldWaterFillKind,
    };

    use crate::simulation::core::config::WorldConfig;

    use crate::nodes::sim::core::AuthoredWaterPatchFillDebug;
    use crate::simulation::resources::ResourceDepositSystem;
    use crate::simulation::terrain::TerrainSystem;
    use crate::simulation::water::WaterSystem;
    use crate::simulation::world_definition::{
        AuthoredLakeFill, AuthoredOpenWaterFill, LoadedWorldDefinition,
    };

    use godot::prelude::Vector2;

    // The smallest legal world: 128 m at 8 m, 17 x 17 samples, the origin at sample (8, 8).
    fn test_core_with_small_world() -> SimCore {
        let config = WorldConfig::new(128.0, 128.0, 40.0, 10.0);
        SimCore {
            treasury: CityTreasury::new(0.0),
            terrain_dirty: false,
            water_dirty: false,
            benchmark_mode: true,
            ..SimCore::new(config)
        }
    }

    // One fill flooded and merged into a zeroed depth buffer.
    fn apply_lake_fill_to_depth(
        terrain_world: &[f32],
        width: usize,
        height: usize,
        seed_x: usize,
        seed_z: usize,
        surface_elevation_m: f32,
        depth: &mut [f32],
    ) -> LakeFillApplication {
        let mut flood = FillFlood::new(terrain_world.len());
        let application = flood.flood(
            terrain_world,
            width,
            height,
            seed_x,
            seed_z,
            surface_elevation_m,
        );
        flood.merge_depth(terrain_world, surface_elevation_m, depth);
        application
    }

    fn carve_closed_basin(core: &mut SimCore) {
        let heights = [
            [200.0, 200.0, 200.0, 200.0, 200.0],
            [200.0, 40.0, 40.0, 40.0, 200.0],
            [200.0, 40.0, 20.0, 40.0, 200.0],
            [200.0, 40.0, 40.0, 40.0, 200.0],
            [200.0, 200.0, 200.0, 200.0, 200.0],
        ];
        // Centred on the origin's sample (8, 8); the ground around the rim stays at 0.
        for (z, row) in heights.iter().enumerate() {
            for (x, height) in row.iter().copied().enumerate() {
                core.heightmap.set_height(x + 6, z + 6, height);
            }
        }
    }

    // `ROAD-114`: a water edit replaces the water whole and restarts its render generation, so
    // two edits in a row leave equal generations; the road tool's view keys its water copy as
    // the published surface does and sees the second edit's water.
    #[test]
    fn the_road_tool_view_follows_two_water_edits_in_a_row() {
        let mut core = test_core_with_small_world();
        carve_closed_basin(&mut core);
        // A loaded world publishes a current empty road surface.
        let loaded = LoadedWorldDefinition {
            name: "water-test".to_owned(),
            config: core.config,
            terrain: core.heightmap.clone(),
            lake_fills: Vec::new(),
            open_water_fills: Vec::new(),
            resource_deposits: ResourceDepositSystem::from_world_config(&core.config),
        };
        core.apply_loaded_world_definition(
            loaded,
            crate::simulation::vegetation::VegetationConfig::default(),
        );
        let lake = |surface_elevation_m| AuthoredLakeFill {
            world_x: 0.0,
            world_z: 0.0,
            surface_elevation_m,
        };
        core.publish_road_tool_view().expect("current roadbed");

        core.world_lake_fills = vec![lake(60.0)];
        core.rebuild_authored_water_preview_internal()
            .expect("first water edit");
        let first_depths = core.watermap.clone_baseline_depth_dense();
        let first_generation = core.watermap.render_generation();
        let publication = core.publish_road_tool_view().expect("current roadbed");
        assert_eq!(
            publication.view.water.clone_baseline_depth_dense(),
            first_depths
        );

        core.world_lake_fills = vec![lake(150.0)];
        core.rebuild_authored_water_preview_internal()
            .expect("second water edit");
        let second_depths = core.watermap.clone_baseline_depth_dense();
        assert_ne!(second_depths, first_depths, "the edits fill differently");
        assert_eq!(
            core.watermap.render_generation(),
            first_generation,
            "the render generation alone cannot tell the edits apart"
        );
        let publication = core.publish_road_tool_view().expect("current roadbed");
        assert_eq!(
            publication.view.water.clone_baseline_depth_dense(),
            second_depths
        );
    }

    // `TERRAIN-37`: undoing a terrain edit leaves the water (depth bits and the debug cache)
    // equal to a fresh solve on the restored ground. A cut through the basin's rim lets the
    // lake reach the world edge, so the cut drains it and the undo must bring it back.
    #[test]
    fn undoing_a_shore_cut_leaves_the_water_a_fresh_solve() {
        let mut core = test_core_with_small_world();
        carve_closed_basin(&mut core);
        let loaded = LoadedWorldDefinition {
            name: "terrain-37".to_owned(),
            config: core.config,
            terrain: core.heightmap.clone(),
            lake_fills: vec![AuthoredLakeFill {
                world_x: 0.0,
                world_z: 0.0,
                surface_elevation_m: 60.0,
            }],
            open_water_fills: Vec::new(),
            resource_deposits: ResourceDepositSystem::from_world_config(&core.config),
        };
        core.apply_loaded_world_definition(
            loaded,
            crate::simulation::vegetation::VegetationConfig::default(),
        );
        let products = |core: &SimCore| {
            let depth: Vec<u32> = core
                .watermap
                .clone_baseline_depth_dense()
                .iter()
                .map(|depth| depth.to_bits())
                .collect();
            let debug: std::collections::BTreeMap<_, _> = core
                .authored_water_patch_fill_debug_cache
                .iter()
                .map(|(key, contributors)| (*key, format!("{contributors:?}")))
                .collect();
            (depth, debug)
        };
        let before = products(&core);
        assert!(before.0.iter().any(|&bits| f32::from_bits(bits) > 0.0));
        // The rim sample (6, 8), world (-16, 0): a brush of half a cell lowers it alone.
        core.sculpt_terrain_internal(Vector2::new(-16.0, 0.0), 4.0, -190.0);
        assert_ne!(products(&core), before, "the cut drains the lake");
        assert!(core.undo_action_internal());
        let undone = products(&core);
        core.rebuild_authored_water_preview_internal()
            .expect("fresh solve");
        assert_eq!(undone, products(&core), "undo leaves a fresh solve's water");
        assert_eq!(undone, before);
    }

    #[test]
    fn loaded_authored_water_publishes_matching_empty_road_generation() {
        let mut core = test_core_with_small_world();
        carve_closed_basin(&mut core);
        let loaded = LoadedWorldDefinition {
            name: "water-test".to_owned(),
            config: core.config,
            terrain: core.heightmap.clone(),
            lake_fills: vec![AuthoredLakeFill {
                world_x: 0.0,
                world_z: 0.0,
                surface_elevation_m: 100.0,
            }],
            open_water_fills: Vec::new(),
            resource_deposits: ResourceDepositSystem::from_world_config(&core.config),
        };

        core.apply_loaded_world_definition(
            loaded,
            crate::simulation::vegetation::VegetationConfig::default(),
        );

        assert!(
            core.watermap
                .clone_baseline_depth_dense()
                .iter()
                .any(|depth| *depth > 0.0),
            "the authored lake must survive world instantiation"
        );
        assert!(
            core.transit_network.roadbed.is_current(),
            "the replacement empty road surface must be published after authored water rebuild"
        );
        assert!(core.cached_road_mesh_chunks.is_empty());
        assert!(core.road_mesh_full_replace);
        assert_eq!(
            core.cached_road_mesh_generation, core.road_tool_surface_generation,
            "the snapshot token must identify the final post-water world generation"
        );
    }

    #[test]
    fn new_game_vegetation_parameters_survive_world_definition_instantiation() {
        use crate::simulation::vegetation::{VegetationConfig, VegetationGenerator};

        let mut core = test_core_with_small_world();
        let authored = VegetationConfig {
            enabled: true,
            seed: 0x0bad_cafe,
            coverage: 0.24,
            canopy_stems_per_ha: 60.0,
        };
        let loaded = LoadedWorldDefinition {
            name: "vegetation-test".to_owned(),
            config: core.config,
            terrain: core.heightmap.clone(),
            lake_fills: Vec::new(),
            open_water_fills: Vec::new(),
            resource_deposits: ResourceDepositSystem::from_world_config(&core.config),
        };

        core.apply_loaded_world_definition(loaded, authored);

        // The asset carries no vegetation, so what the player chose is what the world gets.
        assert_eq!(core.vegetation.config, authored);
        assert!(core.vegetation.canopy_cell_m < VegetationGenerator::default().canopy_cell_m);

        // An out-of-range request is corrected on the way in rather than stored as authored.
        let loaded = LoadedWorldDefinition {
            name: "vegetation-test".to_owned(),
            config: core.config,
            terrain: core.heightmap.clone(),
            lake_fills: Vec::new(),
            open_water_fills: Vec::new(),
            resource_deposits: ResourceDepositSystem::from_world_config(&core.config),
        };
        core.apply_loaded_world_definition(
            loaded,
            VegetationConfig {
                coverage: 9.0,
                ..authored
            },
        );
        assert_eq!(core.vegetation.config.coverage, 1.0);
    }

    #[test]
    fn blank_world_reset_uses_the_fixed_512_m_render_span() {
        let mut core = test_core_with_small_world();

        core.create_blank_world_internal(128.0, 128.0, 8.0, 0.0)
            .expect("the smallest legal world should reset");

        assert_eq!(
            core.terrain_dirty,
            !core.heightmap.dirty_render_patches().is_empty()
        );
        assert_eq!(
            core.water_dirty,
            !core.watermap.dirty_render_patches().is_empty()
        );

        assert_eq!(core.config.terrain_render_chunk_span_m(), 512.0);
        assert_eq!(core.heightmap.chunk_span_m(), 512.0);
        assert_eq!(core.transit_network.roadbed.chunk_span_m(), 512.0);
        for (width_m, cell_m) in [(40.0, 8.0), (128.0, 10.0), (128.0, 4.5)] {
            let error = core
                .create_blank_world_internal(width_m, 128.0, cell_m, 0.0)
                .unwrap_err();
            assert!(error.contains("must be"), "{error}");
        }
    }

    #[test]
    fn new_world_rejects_terrain_payloads_from_previous_world() {
        let mut core = test_core_with_small_world();
        let first_generation = core.terrain_payload_generation_for_patch(0, 0);
        core.bump_terrain_payload_patch_generations(&[(1, 1)]);
        let edited_generation = core.terrain_payload_generation_for_patch(1, 1);
        // Two by two 512 m patches, so patch (1, 1) exists in the new world.
        core.create_blank_world_internal(1024.0, 1024.0, 8.0, 1000.0)
            .unwrap();
        for (key, stale) in [((0, 0), first_generation), ((1, 1), edited_generation)] {
            core.heightmap.mark_render_patch_dirty(key.0, key.1);
            assert!(core.terrain_payload_generation_for_patch(key.0, key.1) > stale);
            assert!(!core.acknowledge_terrain_render_patch(key.0, key.1, stale));
            assert!(core.heightmap.dirty_render_patches().contains(&key));
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
        let config = WorldConfig::new(128.0, 128.0, 40.0, 10.0);
        let terrain = TerrainSystem::from_world_config(&config);
        let water = WaterSystem::from_world_config(&config);

        let terrain_world = terrain.clone_source_dense();
        let preview = evaluate_world_water_fill_preview(
            &terrain,
            &terrain_world,
            &mut FillFlood::new(terrain_world.len()),
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
                .clone_baseline_depth_dense()
                .iter()
                .any(|depth| *depth > 0.0)
        );

        core.commit_world_lake_fill_preview_internal()
            .expect("preview should commit");

        assert!(core.world_lake_fill_preview.is_none());
        assert_eq!(core.world_lake_fills.len(), 1);
    }

    #[test]
    fn authored_water_patch_debug_identifies_committed_lake_fill() {
        let mut core = test_core_with_small_world();
        carve_closed_basin(&mut core);

        core.begin_world_lake_fill_preview_internal(Vector2::ZERO, 100.0)
            .expect("preview should start");
        core.commit_world_lake_fill_preview_internal()
            .expect("preview should commit");

        let contributors = core.authored_water_patch_fill_debug_internal(0, 0);

        assert_eq!(contributors.len(), 1);
        assert_eq!(contributors[0].kind, WorldWaterFillKind::Lake);
        assert_eq!(contributors[0].fill_index, 0);
        assert!(!contributors[0].preview);
        assert!(contributors[0].patch_nonzero_samples > 0);
        assert!(contributors[0].patch_max_depth_m > 0.0);
        assert!(contributors[0].patch_sum_depth_m > 0.0);
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
                .clone_baseline_depth_dense()
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

    // The dense solve the per-fill floods replaced (`A4c`), kept as the reference: per fill a
    // world-sized depth buffer and breadth-first flood, merged by `max`, and per render patch a
    // row-major scan of that buffer for the debug cache.
    fn dense_reference_solve(
        core: &SimCore,
    ) -> (Vec<f32>, std::collections::BTreeMap<(usize, usize), String>) {
        let terrain = core.heightmap.clone_source_dense();
        let water = &core.watermap;
        let (width, height) = (water.width, water.height);
        let mut depth = vec![0.0f32; terrain.len()];
        let mut debug = std::collections::BTreeMap::<(usize, usize), Vec<_>>::new();
        let mut fills: Vec<(WorldWaterFillKind, i32, bool, f32, f32, f32)> = Vec::new();
        for (index, fill) in core.world_lake_fills.iter().enumerate() {
            let (x, z, level) = (fill.world_x, fill.world_z, fill.surface_elevation_m);
            fills.push((WorldWaterFillKind::Lake, index as i32, false, x, z, level));
        }
        for (index, fill) in core.world_open_water_fills.iter().enumerate() {
            let (x, z, level) = (fill.world_x, fill.world_z, fill.surface_elevation_m);
            fills.push((
                WorldWaterFillKind::OpenWater,
                index as i32,
                false,
                x,
                z,
                level,
            ));
        }
        if let Some(preview) = core
            .world_lake_fill_preview
            .filter(|preview| preview.is_valid())
        {
            let (x, z) = (preview.seed_world_x, preview.seed_world_z);
            fills.push((preview.kind, -1, true, x, z, preview.surface_elevation_m));
        }
        for (kind, fill_index, preview, world_x, world_z, level) in fills {
            let (seed_x, seed_z) = water.world_to_grid_cell_clamped(world_x, world_z);
            let mut fill = vec![0.0f32; terrain.len()];
            let mut visited = vec![false; terrain.len()];
            let mut queue = std::collections::VecDeque::new();
            let (mut filled, mut edge) = (0usize, false);
            let seed = seed_z * width + seed_x;
            if terrain[seed] < level {
                visited[seed] = true;
                queue.push_back((seed_x, seed_z));
            }
            while let Some((x, z)) = queue.pop_front() {
                let idx = z * width + x;
                edge |= x == 0 || z == 0 || x + 1 == width || z + 1 == height;
                fill[idx] = fill[idx].max(level - terrain[idx]);
                filled += 1;
                for (nx, nz, ok) in [
                    (x.wrapping_sub(1), z, x > 0),
                    (x + 1, z, x + 1 < width),
                    (x, z.wrapping_sub(1), z > 0),
                    (x, z + 1, z + 1 < height),
                ] {
                    let n = nz.wrapping_mul(width).wrapping_add(nx);
                    if ok && !visited[n] && terrain[n] <= level {
                        visited[n] = true;
                        queue.push_back((nx, nz));
                    }
                }
            }
            if edge != (kind == WorldWaterFillKind::OpenWater) {
                continue;
            }
            for (dst, src) in depth.iter_mut().zip(&fill) {
                *dst = dst.max(*src);
            }
            for patch_z in 0..water.render_patch_rows() {
                for patch_x in 0..water.render_patch_cols() {
                    let (x0, x1, z0, z1) = water
                        .render_patch_owned_sample_bounds(patch_x, patch_z)
                        .unwrap();
                    let (mut count, mut max, mut sum) = (0usize, 0.0f32, 0.0f32);
                    for z in z0..=z1 {
                        for x in x0..=x1 {
                            let d = fill[z * width + x];
                            if d > 0.001 {
                                count += 1;
                                max = max.max(d);
                                sum += d;
                            }
                        }
                    }
                    if count > 0 {
                        debug.entry((patch_x, patch_z)).or_default().push(
                            AuthoredWaterPatchFillDebug {
                                kind,
                                fill_index,
                                preview,
                                world_x,
                                world_z,
                                surface_elevation_m: level,
                                filled_cells: filled,
                                touches_world_edge: edge,
                                patch_nonzero_samples: count,
                                patch_max_depth_m: max,
                                patch_sum_depth_m: sum,
                            },
                        );
                    }
                }
            }
        }
        let debug = debug
            .into_iter()
            .map(|(key, mut contributors)| {
                super::sort_authored_water_patch_fill_debug(&mut contributors);
                (key, format!("{contributors:?}"))
            })
            .collect();
        (depth, debug)
    }

    // `A4c`: the per-fill floods give the dense solve's products bit for bit (depth bits; the
    // debug cache's every field, printed round-trip exact), on overlapping and nested lakes, an
    // edge-escaping lake, open water on and off the edge and a live preview, after load and
    // after brush edits that cut a shore, dam the strait and raise an island. Two fills flood
    // 4096 samples or more, so the radix sort runs.
    #[test]
    fn authored_water_solve_matches_the_dense_reference_bitwise() {
        let config = WorldConfig::new(1280.0, 1280.0, 1280.0, 10.0).with_terrain_resolution(8.0);
        let mut core = SimCore {
            treasury: CityTreasury::new(0.0),
            terrain_dirty: false,
            water_dirty: false,
            benchmark_mode: true,
            ..SimCore::new(config)
        };
        let (width, height) = (core.heightmap.width, core.heightmap.height);
        for z in 0..height {
            for x in 0..width {
                let (fx, fz) = (x as f32, z as f32);
                // A wide basin with a pillar at its centre, a small basin east of it behind a
                // strait, and a low corner on the world edge. Sample (g, h) is world
                // (8 g - 640, 8 h - 640).
                let big = ((fx - 64.0).powi(2) + (fz - 80.0).powi(2)).sqrt();
                let small = ((fx - 136.0).powi(2) + (fz - 80.0).powi(2)).sqrt();
                let mut ground = 200.0f32;
                if big < 56.0 {
                    ground = 40.0 + big * 0.6 + (fx * 0.37).sin() * 2.0;
                }
                if small < 12.0 {
                    ground = ground.min(50.0 + small * 2.0);
                }
                if (fz - 80.0).abs() < 1.5 && (119.0..=125.0).contains(&fx) {
                    ground = 60.0;
                }
                if (fx - 64.0).abs() < 2.0 && (fz - 80.0).abs() < 2.0 {
                    ground = 70.0;
                }
                if x + z < 12 {
                    ground = 10.0 + (x + z) as f32;
                }
                core.heightmap.set_height(x, z, ground);
            }
        }
        let lake = |world_x, surface_elevation_m| AuthoredLakeFill {
            world_x,
            world_z: 0.0,
            surface_elevation_m,
        };
        let loaded = LoadedWorldDefinition {
            name: "a4c".to_owned(),
            config: core.config,
            terrain: core.heightmap.clone(),
            lake_fills: vec![
                lake(-128.0, 72.0),
                lake(-208.0, 66.0),
                lake(448.0, 58.0),
                AuthoredLakeFill {
                    world_x: -632.0,
                    world_z: -632.0,
                    surface_elevation_m: 30.0,
                },
            ],
            open_water_fills: vec![
                AuthoredOpenWaterFill {
                    world_x: -632.0,
                    world_z: -632.0,
                    surface_elevation_m: 18.0,
                },
                AuthoredOpenWaterFill {
                    world_x: 448.0,
                    world_z: 0.0,
                    surface_elevation_m: 65.0,
                },
            ],
            resource_deposits: ResourceDepositSystem::from_world_config(&core.config),
        };
        core.apply_loaded_world_definition(
            loaded,
            crate::simulation::vegetation::VegetationConfig::default(),
        );
        core.begin_world_lake_fill_preview_internal(Vector2::new(-160.0, 0.0), 75.0)
            .expect("preview should start");
        let check = |core: &SimCore, step: &str| {
            let (depth, debug) = dense_reference_solve(core);
            let solved = core.watermap.clone_baseline_depth_dense();
            assert!(
                solved
                    .iter()
                    .map(|d| d.to_bits())
                    .eq(depth.iter().map(|d| d.to_bits())),
                "{step}: depth bits"
            );
            let cache: std::collections::BTreeMap<_, _> = core
                .authored_water_patch_fill_debug_cache
                .iter()
                .map(|(key, contributors)| (*key, format!("{contributors:?}")))
                .collect();
            assert_eq!(cache, debug, "{step}: debug cache");
            depth
        };
        let loaded = check(&core, "load");
        assert!(
            core.authored_water_patch_fill_debug_cache
                .values()
                .flatten()
                .filter(|contributor| contributor.filled_cells >= 4096)
                .map(|contributor| (contributor.fill_index, contributor.preview))
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                >= 2,
            "two fills must flood enough samples to take the radix sort"
        );
        let mut depths = vec![loaded];
        for (step, (x, z), radius, strength) in [
            ("cut shore", (-552.0, 0.0), 24.0, -6.0),
            ("dam strait", (336.0, 0.0), 16.0, 30.0),
            ("raise island", (-128.0, 0.0), 32.0, 25.0),
        ] {
            core.sculpt_terrain_internal(Vector2::new(x, z), radius, strength);
            depths.push(check(&core, step));
        }
        assert!(
            depths.windows(2).all(|pair| pair[0] != pair[1]),
            "every edit must change the water"
        );
    }
}
