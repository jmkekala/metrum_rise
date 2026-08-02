//! World editor and save/load Godot API methods.

use super::*;

#[godot_api(secondary)]
impl SimulationNode {
    /// Resets the runtime to a new blank authored world with the given terrain settings.
    #[func]
    pub fn create_blank_world(
        &mut self,
        width_m: f32,
        height_m: f32,
        terrain_cell_m: f32,
        terrain_chunk_m: f32,
        base_elevation_m: f32,
    ) -> bool {
        let result = {
            let mut core = self.lock_core();
            core.create_blank_world_internal(
                width_m,
                height_m,
                terrain_cell_m,
                terrain_chunk_m,
                base_elevation_m,
            )
        };
        match result {
            Ok(()) => {
                self.clear_runtime_render_async_jobs();
                self.refresh_snapshot_from_core();
                true
            }
            Err(err) => {
                godot_error!("Create blank world failed: {}", err);
                false
            }
        }
    }

    /// Saves the current simulation into a single SQLite snapshot file.
    #[func]
    pub fn save_game(&self, path: GString) -> bool {
        match self.lock_core().save_game_internal(&path.to_string()) {
            Ok(()) => true,
            Err(err) => {
                godot_error!("Save failed: {}", err);
                false
            }
        }
    }

    /// Saves the current blank-world authoring state as a reusable world-definition asset.
    #[func]
    pub fn save_world_definition(&self, path: GString, name: GString) -> bool {
        match self
            .lock_core()
            .save_world_definition_internal(&path.to_string(), &name.to_string())
        {
            Ok(()) => true,
            Err(err) => {
                godot_error!("Save world definition failed: {}", err);
                false
            }
        }
    }

    /// Loads a SQLite save snapshot and replaces the live simulation state.
    #[func]
    pub fn load_game(&mut self, path: GString) -> bool {
        let result = {
            let mut core = self.lock_core();
            core.load_game_internal(&path.to_string())
        };
        match result {
            Ok(()) => {
                self.clear_runtime_render_async_jobs();
                self.refresh_snapshot_from_core();
                true
            }
            Err(err) => {
                godot_error!("Load failed: {}", err);
                false
            }
        }
    }

    /// Loads a reusable world-definition asset and replaces the live runtime with a blank city.
    #[func]
    pub fn load_world_definition(&mut self, path: GString) -> bool {
        let result = {
            let mut core = self.lock_core();
            core.load_world_definition_internal(&path.to_string())
        };
        match result {
            Ok(()) => {
                self.clear_runtime_render_async_jobs();
                self.refresh_snapshot_from_core();
                true
            }
            Err(err) => {
                godot_error!("Load world definition failed: {}", err);
                false
            }
        }
    }

    /// Starts one transient authored lake-fill preview at the clicked terrain cell.
    #[func]
    pub fn begin_world_lake_fill_preview(
        &mut self,
        pos: Vector2,
        surface_elevation_m: f32,
    ) -> VarDictionary {
        let result = {
            let mut core = self.lock_core();
            core.begin_world_lake_fill_preview_internal(pos, surface_elevation_m)
        };
        self.refresh_snapshot_from_core();
        match result {
            Ok(preview) => {
                Self::world_lake_fill_preview_dict(Some(preview), true, "lake fill preview updated")
            }
            Err(err) => {
                godot_error!("Begin world lake fill preview failed: {}", err);
                Self::world_lake_fill_preview_dict(None, false, &err)
            }
        }
    }

    /// Starts one transient authored open-water preview at the clicked terrain cell.
    #[func]
    pub fn begin_world_open_water_fill_preview(
        &mut self,
        pos: Vector2,
        surface_elevation_m: f32,
    ) -> VarDictionary {
        let result = {
            let mut core = self.lock_core();
            core.begin_world_open_water_fill_preview_internal(pos, surface_elevation_m)
        };
        self.refresh_snapshot_from_core();
        match result {
            Ok(preview) => Self::world_lake_fill_preview_dict(
                Some(preview),
                true,
                "open water preview updated",
            ),
            Err(err) => {
                godot_error!("Begin world open water preview failed: {}", err);
                Self::world_lake_fill_preview_dict(None, false, &err)
            }
        }
    }

    /// Updates the active transient lake-fill preview surface elevation.
    #[func]
    pub fn update_world_lake_fill_preview(&mut self, surface_elevation_m: f32) -> VarDictionary {
        let result = {
            let mut core = self.lock_core();
            core.update_world_lake_fill_preview_internal(surface_elevation_m)
        };
        match result {
            Ok(preview) => {
                self.refresh_snapshot_from_core();
                Self::world_lake_fill_preview_dict(Some(preview), true, "lake fill preview updated")
            }
            Err(err) => {
                godot_error!("Update world lake fill preview failed: {}", err);
                let active_preview = self.lock_core().world_water_fill_preview_internal();
                Self::world_lake_fill_preview_dict(active_preview, false, &err)
            }
        }
    }

    /// Updates the active transient open-water preview surface elevation.
    #[func]
    pub fn update_world_open_water_fill_preview(
        &mut self,
        surface_elevation_m: f32,
    ) -> VarDictionary {
        let result = {
            let mut core = self.lock_core();
            core.update_world_open_water_fill_preview_internal(surface_elevation_m)
        };
        match result {
            Ok(preview) => {
                self.refresh_snapshot_from_core();
                Self::world_lake_fill_preview_dict(
                    Some(preview),
                    true,
                    "open water preview updated",
                )
            }
            Err(err) => {
                godot_error!("Update world open water preview failed: {}", err);
                let active_preview = self.lock_core().world_water_fill_preview_internal();
                Self::world_lake_fill_preview_dict(active_preview, false, &err)
            }
        }
    }

    /// Returns the current transient lake-fill preview state.
    #[func]
    pub fn get_world_lake_fill_preview(&self) -> VarDictionary {
        let preview = self.lock_core().world_water_fill_preview_internal();
        let message = if preview.is_some() {
            "surface fill preview active"
        } else {
            "no surface fill preview is active"
        };
        Self::world_lake_fill_preview_dict(preview, true, message)
    }

    /// Returns the current transient open-water preview state.
    #[func]
    pub fn get_world_open_water_fill_preview(&self) -> VarDictionary {
        let preview = self.lock_core().world_water_fill_preview_internal();
        let message = if preview.is_some() {
            "open water preview active"
        } else {
            "no open water preview is active"
        };
        Self::world_lake_fill_preview_dict(preview, true, message)
    }

    /// Returns committed authored-world water markers for world-editor overlays.
    #[func]
    pub fn get_world_water_authoring_markers(&self) -> VarArray {
        let core = self.lock_core();
        let mut markers = VarArray::new();

        for lake in &core.world_lake_fills {
            let terrain_height_m = core
                .heightmap
                .sample_height_world(lake.world_x, lake.world_z)
                * config::HEIGHT_SCALE;
            markers.push(
                &Self::world_water_authoring_marker_dict(
                    "lake_fill",
                    lake.world_x,
                    lake.world_z,
                    terrain_height_m,
                    Some(lake.surface_elevation_m),
                )
                .to_variant(),
            );
        }

        for open_water in &core.world_open_water_fills {
            let terrain_height_m = core
                .heightmap
                .sample_height_world(open_water.world_x, open_water.world_z)
                * config::HEIGHT_SCALE;
            markers.push(
                &Self::world_water_authoring_marker_dict(
                    "open_water_fill",
                    open_water.world_x,
                    open_water.world_z,
                    terrain_height_m,
                    Some(open_water.surface_elevation_m),
                )
                .to_variant(),
            );
        }

        markers
    }

    /// Paints authored coal richness in a circular world-editor brush footprint.
    #[func]
    pub fn paint_world_coal_deposit(
        &mut self,
        pos: Vector2,
        radius_m: f32,
        richness_percent: f32,
    ) -> bool {
        let mut core = self.lock_core();
        core.paint_world_coal_deposit_internal(pos, radius_m, richness_percent)
    }

    /// Erases authored coal richness in a circular world-editor brush footprint.
    #[func]
    pub fn erase_world_coal_deposit(&mut self, pos: Vector2, radius_m: f32) -> bool {
        let mut core = self.lock_core();
        core.erase_world_coal_deposit_internal(pos, radius_m)
    }

    /// Returns authored coal richness as a terrain-sized RGBA8 overlay texture payload.
    #[func]
    pub fn get_world_coal_deposit_overlay_data(&self) -> PackedByteArray {
        self.lock_core()
            .get_world_coal_deposit_overlay_data_internal()
    }

    /// Returns committed coal extraction polygons as a high-resolution L8 mask.
    #[func]
    pub fn get_coal_pit_overlay_data(&self) -> PackedByteArray {
        self.lock_core().get_coal_pit_overlay_data_internal()
    }

    /// Returns committed agricultural fields as a high-resolution L8 mask.
    #[func]
    pub fn get_agriculture_field_overlay_data(&self) -> PackedByteArray {
        self.lock_core()
            .get_agriculture_field_overlay_data_internal()
    }

    /// Returns the pixel dimensions of the committed coal extraction mask.
    #[func]
    pub fn get_coal_pit_overlay_size(&self) -> Vector2 {
        self.lock_core().get_coal_pit_overlay_size_internal()
    }

    /// Returns the pixel dimensions of the committed agricultural field mask.
    #[func]
    pub fn get_agriculture_field_overlay_size(&self) -> Vector2 {
        self.lock_core()
            .get_agriculture_field_overlay_size_internal()
    }

    /// Returns `(min_x, min_z, width, height)` for the committed coal extraction mask.
    #[func]
    pub fn get_coal_pit_overlay_world_bounds(&self) -> Vector4 {
        self.lock_core()
            .get_coal_pit_overlay_world_bounds_internal()
    }

    /// Returns `(min_x, min_z, width, height)` for the committed agricultural field mask.
    #[func]
    pub fn get_agriculture_field_overlay_world_bounds(&self) -> Vector4 {
        self.lock_core()
            .get_agriculture_field_overlay_world_bounds_internal()
    }

    /// Returns a monotonic revision for committed coal extraction polygon visuals.
    #[func]
    pub fn get_coal_pit_overlay_revision(&self) -> i64 {
        self.lock_core().get_coal_pit_overlay_revision_internal() as i64
    }

    /// Returns committed agricultural field polygons for diagnostics and editor tools.
    #[func]
    pub fn get_agriculture_field_polygons(&self) -> VarArray {
        let core = self.lock_core();
        let mut arr = VarArray::new();
        for site in core.agriculture.sites() {
            if site.polygon_world.len() < 3 {
                continue;
            }
            let mut points = PackedVector2Array::new();
            for point in &site.polygon_world {
                points.push(*point);
            }
            let mut dict = VarDictionary::new();
            dict.set("building_id", site.building_idx as i64);
            dict.set("resource_id", GString::from(site.resource_id.as_str()));
            dict.set("area_m2", f64::from(site.area_m2.max(0.0)));
            dict.set("points", points);
            arr.push(&dict.to_variant());
        }
        arr
    }

    /// Returns a monotonic revision for committed agricultural field visuals.
    #[func]
    pub fn get_agriculture_field_overlay_revision(&self) -> i64 {
        self.lock_core()
            .get_agriculture_field_overlay_revision_internal() as i64
    }

    /// Commits the active transient lake-fill preview into authored world state.
    #[func]
    pub fn commit_world_lake_fill_preview(&mut self) -> bool {
        let result = {
            let mut core = self.lock_core();
            core.commit_world_lake_fill_preview_internal()
        };
        match result {
            Ok(()) => {
                self.refresh_snapshot_from_core();
                true
            }
            Err(err) => {
                godot_error!("Commit world lake fill preview failed: {}", err);
                false
            }
        }
    }

    /// Commits the active transient open-water preview into authored world state.
    #[func]
    pub fn commit_world_open_water_fill_preview(&mut self) -> bool {
        let result = {
            let mut core = self.lock_core();
            core.commit_world_open_water_fill_preview_internal()
        };
        match result {
            Ok(()) => {
                self.refresh_snapshot_from_core();
                true
            }
            Err(err) => {
                godot_error!("Commit world open water preview failed: {}", err);
                false
            }
        }
    }

    /// Cancels the active transient lake-fill preview.
    #[func]
    pub fn cancel_world_lake_fill_preview(&mut self) -> bool {
        let cancelled = {
            let mut core = self.lock_core();
            core.cancel_world_water_fill_preview_internal()
        };
        if cancelled {
            self.refresh_snapshot_from_core();
        }
        cancelled
    }

    /// Cancels the active transient open-water preview.
    #[func]
    pub fn cancel_world_open_water_fill_preview(&mut self) -> bool {
        let cancelled = {
            let mut core = self.lock_core();
            core.cancel_world_water_fill_preview_internal()
        };
        if cancelled {
            self.refresh_snapshot_from_core();
        }
        cancelled
    }

    /// Removes the nearest authored lake fill within the given radius.
    #[func]
    pub fn remove_world_lake_fill_near(&mut self, pos: Vector2, radius_m: f32) -> bool {
        let removed = {
            let mut core = self.lock_core();
            core.remove_world_lake_fill_near_internal(pos, radius_m)
        };
        if removed {
            self.refresh_snapshot_from_core();
        }
        removed
    }

    /// Removes the nearest authored open-water fill within the given radius.
    #[func]
    pub fn remove_world_open_water_fill_near(&mut self, pos: Vector2, radius_m: f32) -> bool {
        let removed = {
            let mut core = self.lock_core();
            core.remove_world_open_water_fill_near_internal(pos, radius_m)
        };
        if removed {
            self.refresh_snapshot_from_core();
        }
        removed
    }
}
