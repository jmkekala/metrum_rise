//! Zoning and service-building Godot API methods.

use crate::simulation::extraction::{
    EXTRACTOR_POLYGON_LINK_DISTANCE_M, building_footprint_polygon,
};

use super::*;

#[godot_api(secondary)]
impl SimulationNode {
    /// Returns available zoning profiles for tool palettes and asset editing.
    #[func]
    pub fn get_zone_profiles(&self) -> VarArray {
        let core = self.lock_core();
        let mut arr = VarArray::new();
        for profile in core.zoning.profiles.profiles() {
            let mut dict = VarDictionary::new();
            dict.set("id", GString::from(profile.id.as_str()));
            dict.set("runtime_id", i64::from(profile.runtime_id));
            dict.set("display_name", GString::from(profile.display_name.as_str()));
            dict.set("ui_order", i64::from(profile.ui_order));
            dict.set("zone_type", GString::from(profile.zone_type.as_str()));
            dict.set("density", GString::from(profile.density.as_str()));
            dict.set(
                "ui_color",
                GString::from(
                    format!(
                        "#{:02X}{:02X}{:02X}",
                        profile.ui_color_rgb[0], profile.ui_color_rgb[1], profile.ui_color_rgb[2]
                    )
                    .as_str(),
                ),
            );
            dict.set("ui_icon", GString::from(profile.ui_icon.as_str()));
            dict.set(
                "ui_description",
                GString::from(profile.ui_description.as_str()),
            );
            arr.push(&dict.to_variant());
        }
        arr
    }

    fn service_placement_error_message(
        rejection: ExplicitServicePlacementRejection,
    ) -> &'static str {
        match rejection {
            ExplicitServicePlacementRejection::AssetUnavailable => "service asset unavailable",
            ExplicitServicePlacementRejection::NotServiceBuilding => {
                "selected asset is not an explicit service building"
            }
            ExplicitServicePlacementRejection::NotIndustryBuilding => {
                "selected asset is not an explicit industry building"
            }
            ExplicitServicePlacementRejection::UtilityProfileUnavailable => {
                "service building has no supported utility profile"
            }
            ExplicitServicePlacementRejection::ExtractorProfileUnavailable => {
                "industry building has no supported extractor profile"
            }
            ExplicitServicePlacementRejection::RoadFrontageUnavailable => {
                "no nearby road frontage can fit this building"
            }
            ExplicitServicePlacementRejection::DrivewayRoadSurfaceMissing => {
                "driveway cannot resolve road surface height"
            }
            ExplicitServicePlacementRejection::DrivewayHeightConflict => {
                "driveway anchors require incompatible site heights"
            }
            ExplicitServicePlacementRejection::DrivewayConnectionMissing => {
                "driveway anchors do not connect to the frontage road"
            }
            ExplicitServicePlacementRejection::FrontageRoadSurfaceMissing => {
                "frontage road surface height is unavailable"
            }
            ExplicitServicePlacementRejection::NeighborSiteHeightConflict => {
                "building site height conflicts with a neighbor"
            }
            ExplicitServicePlacementRejection::SiteSupportTieInInvalid => {
                "building site cannot tie into surrounding terrain"
            }
            ExplicitServicePlacementRejection::SiteOverlap => {
                "building footprint overlaps an existing site"
            }
            ExplicitServicePlacementRejection::RoadOverlap => {
                "building footprint overlaps an existing road"
            }
        }
    }

    fn industry_placement_error_message(
        rejection: ExplicitServicePlacementRejection,
    ) -> &'static str {
        match rejection {
            ExplicitServicePlacementRejection::AssetUnavailable => "industry asset unavailable",
            ExplicitServicePlacementRejection::NotIndustryBuilding => {
                "selected asset is not an explicit industry building"
            }
            ExplicitServicePlacementRejection::ExtractorProfileUnavailable => {
                "industry building has no supported extractor profile"
            }
            ExplicitServicePlacementRejection::NotServiceBuilding => {
                "selected asset is not an explicit service building"
            }
            ExplicitServicePlacementRejection::UtilityProfileUnavailable => {
                "service building has no supported utility profile"
            }
            ExplicitServicePlacementRejection::RoadFrontageUnavailable => {
                "no nearby road frontage can fit this building"
            }
            ExplicitServicePlacementRejection::DrivewayRoadSurfaceMissing => {
                "driveway cannot resolve road surface height"
            }
            ExplicitServicePlacementRejection::DrivewayHeightConflict => {
                "driveway anchors require incompatible site heights"
            }
            ExplicitServicePlacementRejection::DrivewayConnectionMissing => {
                "driveway anchors do not connect to the frontage road"
            }
            ExplicitServicePlacementRejection::FrontageRoadSurfaceMissing => {
                "frontage road surface height is unavailable"
            }
            ExplicitServicePlacementRejection::NeighborSiteHeightConflict => {
                "building site height conflicts with a neighbor"
            }
            ExplicitServicePlacementRejection::SiteSupportTieInInvalid => {
                "building site cannot tie into surrounding terrain"
            }
            ExplicitServicePlacementRejection::SiteOverlap => {
                "building footprint overlaps an existing site"
            }
            ExplicitServicePlacementRejection::RoadOverlap => {
                "building footprint overlaps an existing road"
            }
        }
    }

    fn empty_service_preview_dict(error: &str) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set("valid", false);
        dict.set("error", GString::from(error));
        dict.set("corners", PackedVector3Array::new());
        dict.set("center_x", 0.0f64);
        dict.set("center_z", 0.0f64);
        dict.set("support_height_m", 0.0f64);
        dict.set("facing_dir", Vector2::ZERO);
        dict.set("part_transforms", PackedFloat32Array::new());
        dict
    }

    /// Returns explicit service-building assets available for the Services toolbar.
    #[func]
    pub fn get_service_building_assets(&self) -> VarArray {
        let core = self.lock_core();
        let catalog = load_runtime_economy_catalog().ok();
        let mut ids = core
            .allocator
            .registry
            .qualified_ids()
            .filter(|asset_id| core.allocator.registry.is_city_service_asset(asset_id))
            .collect::<Vec<_>>();
        ids.sort_unstable();

        let mut arr = VarArray::new();
        for asset_id in ids {
            let Some(entry) = core.allocator.registry.get(asset_id) else {
                continue;
            };
            let Some(building) = entry.manifest.building.as_ref() else {
                continue;
            };
            let Some(service_class) = core.allocator.registry.service_class(asset_id) else {
                continue;
            };
            let worker_capacity = catalog
                .as_ref()
                .and_then(|catalog| {
                    core.allocator
                        .worker_capacity_for_asset_with_catalog(asset_id, catalog)
                })
                .unwrap_or_else(|| core.allocator.worker_capacity_for_asset(asset_id));

            let mut dict = VarDictionary::new();
            dict.set("asset_id", GString::from(asset_id));
            dict.set(
                "display_name",
                GString::from(entry.manifest.display_name.as_str()),
            );
            dict.set("service_class", GString::from(service_class));
            dict.set("lot_width_cells", i64::from(building.lot_width_cells));
            dict.set("lot_depth_cells", i64::from(building.lot_depth_cells));
            dict.set("worker_capacity", i64::from(worker_capacity));
            arr.push(&dict.to_variant());
        }
        arr
    }

    /// Returns explicit resource-extractor building assets for the Industry toolbar.
    #[func]
    pub fn get_industry_building_assets(&self) -> VarArray {
        let core = self.lock_core();
        let catalog = load_runtime_economy_catalog().ok();
        let mut ids = core
            .allocator
            .registry
            .qualified_ids()
            .filter(|asset_id| {
                core.allocator
                    .registry
                    .is_resource_extractor_asset(asset_id)
            })
            .collect::<Vec<_>>();
        ids.sort_unstable();

        let mut arr = VarArray::new();
        for asset_id in ids {
            let Some(entry) = core.allocator.registry.get(asset_id) else {
                continue;
            };
            let Some(building) = entry.manifest.building.as_ref() else {
                continue;
            };
            let Some(resource_id) = core.allocator.registry.extractor_resource(asset_id) else {
                continue;
            };
            let worker_capacity = catalog
                .as_ref()
                .and_then(|catalog| {
                    core.allocator
                        .worker_capacity_for_asset_with_catalog(asset_id, catalog)
                })
                .unwrap_or_else(|| core.allocator.worker_capacity_for_asset(asset_id));

            let mut dict = VarDictionary::new();
            dict.set("asset_id", GString::from(asset_id));
            dict.set(
                "display_name",
                GString::from(entry.manifest.display_name.as_str()),
            );
            dict.set("resource_id", GString::from(resource_id));
            dict.set("lot_width_cells", i64::from(building.lot_width_cells));
            dict.set("lot_depth_cells", i64::from(building.lot_depth_cells));
            dict.set("worker_capacity", i64::from(worker_capacity));
            arr.push(&dict.to_variant());
        }
        arr
    }

    /// Returns a road-frontage snapped footprint preview for one service building asset.
    #[func]
    pub fn get_service_building_placement_preview(
        &self,
        asset_id: GString,
        world_x: f32,
        world_z: f32,
    ) -> VarDictionary {
        let asset_id = asset_id.to_string();
        if asset_id.is_empty() {
            return Self::empty_service_preview_dict("no service building selected");
        }
        let core = self.lock_core();
        match core.get_service_building_placement_preview_internal(&asset_id, world_x, world_z) {
            Ok(preview) => {
                let part_transforms = core.get_service_building_preview_part_transforms_internal(
                    &asset_id,
                    preview.center_2d,
                    preview.support_height_m,
                    preview.facing_dir,
                );
                let mut corners = PackedVector3Array::new();
                for corner in preview.corners {
                    corners.push(Vector3::new(
                        corner.x,
                        preview.support_height_m + 0.08,
                        corner.y,
                    ));
                }
                let mut dict = VarDictionary::new();
                dict.set("valid", preview.valid);
                let error = preview
                    .rejection
                    .map(Self::service_placement_error_message)
                    .unwrap_or("");
                dict.set("error", GString::from(error));
                dict.set("corners", corners);
                dict.set("center_x", f64::from(preview.center_2d.x));
                dict.set("center_z", f64::from(preview.center_2d.y));
                dict.set("support_height_m", f64::from(preview.support_height_m));
                dict.set("facing_dir", preview.facing_dir);
                dict.set("part_transforms", part_transforms);
                dict
            }
            Err(rejection) => {
                Self::empty_service_preview_dict(Self::service_placement_error_message(rejection))
            }
        }
    }

    /// Places one explicit service building and returns an empty string on success.
    #[func]
    pub fn place_service_building(
        &mut self,
        asset_id: GString,
        world_x: f32,
        world_z: f32,
    ) -> GString {
        let asset_id = asset_id.to_string();
        if asset_id.is_empty() {
            return GString::from("no service building selected");
        }
        let result = {
            let mut core = self.lock_core();
            core.place_service_building_internal(&asset_id, world_x, world_z)
        };
        match result {
            Ok(_) => {
                self.refresh_snapshot_from_core();
                GString::new()
            }
            Err(rejection) => GString::from(Self::service_placement_error_message(rejection)),
        }
    }

    /// Returns a road-frontage snapped footprint preview for one industry building asset.
    #[func]
    pub fn get_industry_building_placement_preview(
        &self,
        asset_id: GString,
        world_x: f32,
        world_z: f32,
    ) -> VarDictionary {
        let asset_id = asset_id.to_string();
        if asset_id.is_empty() {
            return Self::empty_service_preview_dict("no industry building selected");
        }
        let core = self.lock_core();
        match core.get_industry_building_placement_preview_internal(&asset_id, world_x, world_z) {
            Ok(preview) => {
                let part_transforms = core.get_service_building_preview_part_transforms_internal(
                    &asset_id,
                    preview.center_2d,
                    preview.support_height_m,
                    preview.facing_dir,
                );
                let mut corners = PackedVector3Array::new();
                for corner in preview.corners {
                    corners.push(Vector3::new(
                        corner.x,
                        preview.support_height_m + 0.08,
                        corner.y,
                    ));
                }
                let mut dict = VarDictionary::new();
                dict.set("valid", preview.valid);
                let error = preview
                    .rejection
                    .map(Self::industry_placement_error_message)
                    .unwrap_or("");
                dict.set("error", GString::from(error));
                dict.set("corners", corners);
                dict.set("center_x", f64::from(preview.center_2d.x));
                dict.set("center_z", f64::from(preview.center_2d.y));
                dict.set("support_height_m", f64::from(preview.support_height_m));
                dict.set("facing_dir", preview.facing_dir);
                dict.set("part_transforms", part_transforms);
                dict
            }
            Err(rejection) => {
                Self::empty_service_preview_dict(Self::industry_placement_error_message(rejection))
            }
        }
    }

    /// Places one explicit industry building and returns placement metadata on success.
    #[func]
    pub fn place_industry_building(
        &mut self,
        asset_id: GString,
        world_x: f32,
        world_z: f32,
    ) -> VarDictionary {
        let mut dict = VarDictionary::new();
        let asset_id = asset_id.to_string();
        if asset_id.is_empty() {
            dict.set("ok", false);
            dict.set("error", GString::from("no industry building selected"));
            dict.set("building_id", -1_i64);
            return dict;
        }
        let result = {
            let mut core = self.lock_core();
            core.place_industry_building_internal(&asset_id, world_x, world_z)
        };
        match result {
            Ok(building_id) => {
                let footprint_corners = {
                    let core = self.lock_core();
                    let mut corners = PackedVector3Array::new();
                    if let Some(building) = core.allocator.buildings.get(building_id) {
                        for corner in
                            building_footprint_polygon(building, core.zoning.config.zone_cell_m)
                        {
                            corners.push(Vector3::new(
                                corner.x,
                                building.support_height_m + 0.08,
                                corner.y,
                            ));
                        }
                    }
                    corners
                };
                self.refresh_snapshot_from_core();
                dict.set("ok", true);
                dict.set("error", GString::new());
                dict.set("building_id", building_id as i64);
                dict.set("footprint_corners", footprint_corners);
                dict.set(
                    "polygon_link_distance_m",
                    f64::from(EXTRACTOR_POLYGON_LINK_DISTANCE_M),
                );
            }
            Err(rejection) => {
                dict.set("ok", false);
                dict.set(
                    "error",
                    GString::from(Self::industry_placement_error_message(rejection)),
                );
                dict.set("building_id", -1_i64);
            }
        }
        dict
    }

    /// Commits or replaces an extractor polygon for one placed industry building.
    #[func]
    pub fn commit_extractor_polygon(
        &mut self,
        building_id: i32,
        polygon_points: PackedVector2Array,
    ) -> VarDictionary {
        let mut dict = VarDictionary::new();
        if building_id < 0 {
            dict.set("ok", false);
            dict.set("error", GString::from("invalid extractor building id"));
            dict.set("total_reserve_units", 0.0f64);
            dict.set("remaining_reserve_units", 0.0f64);
            return dict;
        }
        let polygon = polygon_points.as_slice().to_vec();
        let result = {
            let mut core = self.lock_core();
            core.commit_extractor_polygon_internal(building_id as usize, polygon)
        };
        match result {
            Ok(summary) => {
                self.refresh_snapshot_from_core();
                dict.set("ok", true);
                dict.set("error", GString::new());
                dict.set(
                    "total_reserve_units",
                    f64::from(summary.total_reserve_units),
                );
                dict.set(
                    "remaining_reserve_units",
                    f64::from(summary.remaining_reserve_units),
                );
            }
            Err(err) => {
                dict.set("ok", false);
                dict.set("error", GString::from(err.as_str()));
                dict.set("total_reserve_units", 0.0f64);
                dict.set("remaining_reserve_units", 0.0f64);
            }
        }
        dict
    }

    /// Removes a pending unfinalized industry placement.
    #[func]
    pub fn cancel_pending_industry_building(&mut self, building_id: i32) -> bool {
        if building_id < 0 {
            return false;
        }
        let removed = {
            let mut core = self.lock_core();
            core.cancel_pending_industry_building_internal(building_id as usize)
        };
        if removed {
            self.refresh_snapshot_from_core();
        }
        removed
    }

    /// Returns the deterministic bulldoze target under one world-space point.
    #[func]
    pub fn get_bulldoze_target_at(&self, world_x: f32, world_z: f32) -> VarDictionary {
        match self.try_lock_core() {
            Some(mut core) => core.get_bulldoze_target_at_internal(world_x, world_z),
            None => {
                let mut dict = VarDictionary::new();
                dict.set("valid", false);
                dict.set("deleted", false);
                dict
            }
        }
    }

    /// Queues one building or road deletion on the simulation thread.
    #[func]
    pub fn bulldoze_at(&mut self, world_x: f32, world_z: f32) -> VarDictionary {
        let prepared = {
            let Some(mut core) = self.try_lock_core() else {
                let mut payload = VarDictionary::new();
                payload.set("valid", false);
                payload.set("deleted", false);
                payload.set("queued", false);
                payload.set("reason", GString::from("simulation busy"));
                return payload;
            };
            core.prepare_bulldoze_command_internal(world_x, world_z)
        };
        let Some((target, mut payload)) = prepared else {
            let mut payload = VarDictionary::new();
            payload.set("valid", false);
            payload.set("deleted", false);
            payload.set("queued", false);
            payload.set("reason", GString::from("nothing targetable"));
            return payload;
        };
        if self.cmd_tx.send(SimCommand::Bulldoze { target }).is_err() {
            payload.set("queued", false);
            payload.set("reason", GString::from("simulation thread unavailable"));
            return payload;
        }
        payload.set("queued", true);
        self.clear_terrain_patch_payload_jobs();
        payload
    }

    /// Creates or rezones a road-aligned zoning parcel at one world-space point.
    #[func]
    pub fn apply_zoning_parcel_at(
        &mut self,
        world_x: f32,
        world_z: f32,
        target_profile_runtime_id: i32,
        frontage_cells: i32,
        depth_cells: i32,
    ) -> bool {
        let Ok(runtime_id) = u16::try_from(target_profile_runtime_id) else {
            return false;
        };
        let mut core = self.lock_core();
        let core = &mut *core;
        let Some((frontage_m, depth_m)) =
            zoning_parcel_cell_dimensions(&core.config, frontage_cells, depth_cells)
        else {
            return false;
        };
        let geometry = if let Some(geometry) = core.zoning.parcel_geometry_at(world_x, world_z) {
            geometry
        } else {
            let Ok(geometry) = core.zoning.preview_parcel_at(
                world_x,
                world_z,
                frontage_m,
                depth_m,
                &core.region_graph,
            ) else {
                return false;
            };
            geometry
        };
        if core
            .allocator
            .parcel_geometry_overlaps_explicit_site(&geometry)
        {
            return false;
        }
        let result = core.zoning.place_or_rezone_parcel_at(
            world_x,
            world_z,
            runtime_id,
            frontage_m,
            depth_m,
            &core.region_graph,
        );
        match result {
            Ok(_) => {
                core.allocator.dirty = true;
                core.allocator.dirty_index = true;
                true
            }
            Err(_) => false,
        }
    }

    /// Returns preview geometry for a road-aligned zoning parcel.
    #[func]
    pub fn get_zoning_parcel_preview(
        &self,
        world_x: f32,
        world_z: f32,
        target_profile_runtime_id: i32,
        frontage_cells: i32,
        depth_cells: i32,
    ) -> VarDictionary {
        let core = self.lock_core();
        let Ok(runtime_id) = u16::try_from(target_profile_runtime_id) else {
            return VarDictionary::new();
        };
        let Some((frontage_m, depth_m)) =
            zoning_parcel_cell_dimensions(&core.config, frontage_cells, depth_cells)
        else {
            return VarDictionary::new();
        };
        if let Some(geometry) = core.zoning.parcel_geometry_at(world_x, world_z) {
            if core
                .allocator
                .parcel_geometry_overlaps_explicit_site(&geometry)
            {
                return VarDictionary::new();
            }
            return zoning_parcel_geometry_dict(&core, &geometry, runtime_id, false, 0);
        }
        let Ok(geometry) = core.zoning.preview_parcel_at(
            world_x,
            world_z,
            frontage_m,
            depth_m,
            &core.region_graph,
        ) else {
            return VarDictionary::new();
        };
        if core
            .allocator
            .parcel_geometry_overlaps_explicit_site(&geometry)
        {
            return VarDictionary::new();
        }
        zoning_parcel_geometry_dict(&core, &geometry, runtime_id, false, 0)
    }

    /// Returns true when one world-space point is inside an authored zoning parcel.
    #[func]
    pub fn has_zoning_parcel_at(&self, world_x: f32, world_z: f32) -> bool {
        self.lock_core().zoning.has_parcel_at(world_x, world_z)
    }

    /// Returns the parcel profile id at a world-space point, or `-1` when no parcel is present.
    #[func]
    pub fn get_zoning_parcel_profile_runtime_id_at(&self, world_x: f32, world_z: f32) -> i32 {
        self.lock_core()
            .zoning
            .parcel_profile_runtime_id_at(world_x, world_z)
            .map(i32::from)
            .unwrap_or(-1)
    }

    /// Returns preview geometry for legal parcels in a road-side parcel drag run.
    #[func]
    pub fn get_zoning_parcel_drag_preview(
        &self,
        start_x: f32,
        start_z: f32,
        end_x: f32,
        end_z: f32,
        target_profile_runtime_id: i32,
        frontage_cells: i32,
        depth_cells: i32,
        gap_m: f32,
    ) -> VarArray {
        let core = self.lock_core();
        let Ok(runtime_id) = u16::try_from(target_profile_runtime_id) else {
            return VarArray::new();
        };
        let Some((frontage_m, depth_m)) =
            zoning_parcel_cell_dimensions(&core.config, frontage_cells, depth_cells)
        else {
            return VarArray::new();
        };
        let Ok(geometries) = core.zoning.preview_parcel_run_at(
            start_x,
            start_z,
            end_x,
            end_z,
            frontage_m,
            depth_m,
            gap_m,
            &core.region_graph,
        ) else {
            return VarArray::new();
        };
        let geometries = zoning_geometries_without_explicit_sites(&core, geometries);
        zoning_parcel_geometries_array(&core, &geometries, runtime_id)
    }

    /// Returns packed preview geometry for legal parcels in a road-side parcel drag run.
    #[func]
    pub fn get_zoning_parcel_drag_preview_packed(
        &self,
        start_x: f32,
        start_z: f32,
        end_x: f32,
        end_z: f32,
        target_profile_runtime_id: i32,
        frontage_cells: i32,
        depth_cells: i32,
        gap_m: f32,
    ) -> VarDictionary {
        let core = self.lock_core();
        let Ok(runtime_id) = u16::try_from(target_profile_runtime_id) else {
            return VarDictionary::new();
        };
        let Some((frontage_m, depth_m)) =
            zoning_parcel_cell_dimensions(&core.config, frontage_cells, depth_cells)
        else {
            return VarDictionary::new();
        };
        let Ok(geometries) = core.zoning.preview_parcel_run_at(
            start_x,
            start_z,
            end_x,
            end_z,
            frontage_m,
            depth_m,
            gap_m,
            &core.region_graph,
        ) else {
            return VarDictionary::new();
        };
        let geometries = zoning_geometries_without_explicit_sites(&core, geometries);
        if geometries.is_empty() {
            return VarDictionary::new();
        }
        zoning_parcel_geometries_packed_dict(&core, &geometries, runtime_id)
    }

    /// Returns packed preview geometry for existing parcels touched by a zoning paint stroke.
    #[func]
    pub fn get_zoning_parcel_rezone_drag_preview_packed(
        &self,
        start_x: f32,
        start_z: f32,
        end_x: f32,
        end_z: f32,
        target_profile_runtime_id: i32,
    ) -> VarDictionary {
        let core = self.lock_core();
        let Ok(runtime_id) = u16::try_from(target_profile_runtime_id) else {
            return VarDictionary::new();
        };
        if core
            .zoning
            .profiles
            .profile_by_runtime_id(runtime_id)
            .is_none()
            && runtime_id != 0
        {
            return VarDictionary::new();
        }
        let geometries = core
            .zoning
            .preview_rezone_stroke(start_x, start_z, end_x, end_z);
        let geometries = zoning_geometries_without_explicit_sites(&core, geometries);
        if geometries.is_empty() {
            return VarDictionary::new();
        }
        zoning_parcel_geometries_packed_dict(&core, &geometries, runtime_id)
    }

    /// Creates legal parcels in a road-side parcel drag run.
    #[func]
    pub fn apply_zoning_parcel_drag(
        &mut self,
        start_x: f32,
        start_z: f32,
        end_x: f32,
        end_z: f32,
        target_profile_runtime_id: i32,
        frontage_cells: i32,
        depth_cells: i32,
        gap_m: f32,
    ) -> bool {
        let Ok(runtime_id) = u16::try_from(target_profile_runtime_id) else {
            return false;
        };
        let mut core = self.lock_core();
        let core = &mut *core;
        let Some((frontage_m, depth_m)) =
            zoning_parcel_cell_dimensions(&core.config, frontage_cells, depth_cells)
        else {
            return false;
        };
        let Ok(geometries) = core.zoning.preview_parcel_run_at(
            start_x,
            start_z,
            end_x,
            end_z,
            frontage_m,
            depth_m,
            gap_m,
            &core.region_graph,
        ) else {
            return false;
        };
        let geometries = zoning_geometries_without_explicit_sites(core, geometries);
        let result = core
            .zoning
            .place_prevalidated_parcel_geometries(geometries, runtime_id);
        match result {
            Ok(ids) if !ids.is_empty() => {
                core.allocator.dirty = true;
                core.allocator.dirty_index = true;
                true
            }
            _ => false,
        }
    }

    /// Rezones every existing parcel touched by a world-space zoning paint stroke.
    #[func]
    pub fn apply_zoning_parcel_rezone_drag(
        &mut self,
        start_x: f32,
        start_z: f32,
        end_x: f32,
        end_z: f32,
        target_profile_runtime_id: i32,
    ) -> bool {
        let Ok(runtime_id) = u16::try_from(target_profile_runtime_id) else {
            return false;
        };
        let mut core = self.lock_core();
        let core = &mut *core;
        let geometries = core
            .zoning
            .preview_rezone_stroke(start_x, start_z, end_x, end_z);
        let geometries = zoning_geometries_without_explicit_sites(core, geometries);
        if geometries.is_empty() {
            return false;
        }
        let result = core
            .zoning
            .rezone_prevalidated_parcel_geometries(&geometries, runtime_id);
        match result {
            Ok(ids) if !ids.is_empty() => {
                core.allocator.dirty = true;
                core.allocator.dirty_index = true;
                true
            }
            _ => false,
        }
    }

    /// Returns committed zoning parcels for the Godot overlay mesh.
    #[func]
    pub fn get_zoning_parcels_overlay(&self) -> VarArray {
        let core = self.lock_core();
        let mut arr = VarArray::new();
        for parcel in core.zoning.parcels() {
            let geometry = crate::simulation::zoning::ParcelGeometry {
                edge_idx: parcel.edge_idx(),
                side: parcel.side(),
                frontage_center_t: parcel.frontage_center_t(),
                frontage_m: parcel.frontage_m(),
                depth_m: parcel.depth_m(),
                front_center: parcel.front_center(),
                center: parcel.center(),
                tangent: parcel.tangent(),
                normal: parcel.normal(),
                corners: parcel.corners(),
                aabb_min: parcel.aabb_min(),
                aabb_max: parcel.aabb_max(),
            };
            let dict = zoning_parcel_geometry_dict(
                &core,
                &geometry,
                parcel.zone_profile_runtime_id(),
                parcel.occupied_building().is_some(),
                parcel.id().raw(),
            );
            arr.push(&dict.to_variant());
        }
        arr
    }

    /// Returns the revision that changes whenever zoning overlay geometry or profiles change.
    #[func]
    pub fn get_zoning_overlay_revision(&self) -> i64 {
        let revision = self.snapshot.read().unwrap().zoning_overlay_revision;
        i64::try_from(revision).unwrap_or(i64::MAX)
    }

    /// Returns the revision that changes whenever zoning occupancy affects overlay coloring.
    #[func]
    pub fn get_zoning_overlay_occupancy_revision(&self) -> i64 {
        let revision = self
            .snapshot
            .read()
            .unwrap()
            .zoning_overlay_occupancy_revision;
        i64::try_from(revision).unwrap_or(i64::MAX)
    }

    /// Returns committed zoning parcel buffers without waiting when the simulation is busy.
    #[func]
    pub fn try_get_zoning_parcels_overlay_packed(&self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set("busy", true);
        let Some(core) = self.try_lock_core() else {
            return dict;
        };
        let revision = core.zoning.overlay_revision();
        let occupancy_revision = core.zoning.overlay_occupancy_revision();
        let parcel_count = core.zoning.parcels().len();
        let mut triangle_vertices = PackedVector3Array::new();
        let mut triangle_colors = PackedColorArray::new();
        let mut line_vertices = PackedVector3Array::new();
        let mut line_colors = PackedColorArray::new();

        for parcel in core.zoning.parcels() {
            let geometry = crate::simulation::zoning::ParcelGeometry {
                edge_idx: parcel.edge_idx(),
                side: parcel.side(),
                frontage_center_t: parcel.frontage_center_t(),
                frontage_m: parcel.frontage_m(),
                depth_m: parcel.depth_m(),
                front_center: parcel.front_center(),
                center: parcel.center(),
                tangent: parcel.tangent(),
                normal: parcel.normal(),
                corners: parcel.corners(),
                aabb_min: parcel.aabb_min(),
                aabb_max: parcel.aabb_max(),
            };
            let corners = zoning_parcel_surface_corners(&core, &geometry);
            let color = zoning_parcel_color(
                &core,
                parcel.zone_profile_runtime_id(),
                parcel.occupied_building().is_some(),
            );
            for corner_idx in [0_usize, 1, 2, 0, 2, 3] {
                triangle_vertices.push(corners[corner_idx]);
                triangle_colors.push(color);
            }
            let line_color = Color::from_rgba(color.r, color.g, color.b, 0.9);
            for corner_idx in 0..4 {
                line_vertices.push(corners[corner_idx]);
                line_vertices.push(corners[(corner_idx + 1) % 4]);
                line_colors.push(line_color);
                line_colors.push(line_color);
            }
        }

        dict.set("busy", false);
        dict.set("revision", i64::try_from(revision).unwrap_or(i64::MAX));
        dict.set(
            "occupancy_revision",
            i64::try_from(occupancy_revision).unwrap_or(i64::MAX),
        );
        dict.set(
            "parcel_count",
            i64::try_from(parcel_count).unwrap_or(i64::MAX),
        );
        dict.set("triangle_vertices", triangle_vertices);
        dict.set("triangle_colors", triangle_colors);
        dict.set("line_vertices", line_vertices);
        dict.set("line_colors", line_colors);
        dict
    }
}
