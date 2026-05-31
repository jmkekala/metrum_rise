//! Road-aligned parcel zoning system state and high-level operations.

use super::constants::{
    DEFAULT_PARCEL_DEPTH_M, DEFAULT_PARCEL_FRONTAGE_M, MAX_PARCEL_DEPTH_M, MAX_PARCEL_FRONTAGE_M,
    MAX_PARCEL_GAP_M, MIN_PARCEL_DEPTH_M, MIN_PARCEL_FRONTAGE_M, MIN_PARCEL_GAP_M,
};
use super::parcels::{self, ParcelGeometry, ParcelId, ParcelPlacementError};
use super::profiles::{ZoningProfileRegistry, load_builtin_profile_registry};
use super::{ParcelStore, ZoningParcel};
use crate::simulation::core::config::WorldConfig;
use crate::simulation::network::graph::RegionGraph;
use godot::prelude::Vector2;
use std::sync::Arc;

/// Road-aligned parcel zoning system.
#[derive(Clone)]
pub struct ZoningSystem {
    /// Validated built-in zoning-profile registry shared by the parcel tool, allocator, and saves.
    pub profiles: Arc<ZoningProfileRegistry>,
    /// Stable road-aligned parcel store used as zoning authority.
    pub parcels: ParcelStore,
    /// World configuration used for parcel bounds validation.
    pub config: WorldConfig,
}

impl ZoningSystem {
    /// Creates a new, empty parcel zoning system for `config`.
    pub fn new(config: &WorldConfig) -> Self {
        let profiles = load_builtin_profile_registry()
            .unwrap_or_else(|err| panic!("could not load built-in zoning profiles: {err}"));
        Self {
            profiles,
            parcels: ParcelStore::default(),
            config: *config,
        }
    }

    /// Clears all authored zoning parcels.
    pub fn clear(&mut self) {
        self.parcels.clear();
    }

    /// Remaps parcel road-edge attachments after network compaction.
    pub fn update_edge_indices(&mut self, mapping: &std::collections::HashMap<usize, usize>) {
        self.parcels.remove_edges_not_in_mapping(mapping);
    }

    /// Returns every authored zoning parcel.
    pub fn parcels(&self) -> &[ZoningParcel] {
        self.parcels.parcels()
    }

    /// Returns one authored parcel by stable raw id.
    pub fn parcel_by_raw_id(&self, parcel_id: u64) -> Option<&ZoningParcel> {
        self.parcels.get(ParcelId::from_raw(parcel_id))
    }

    /// Returns one mutable authored parcel by stable raw id.
    pub fn parcel_by_raw_id_mut(&mut self, parcel_id: u64) -> Option<&mut ZoningParcel> {
        self.parcels.get_mut(ParcelId::from_raw(parcel_id))
    }

    /// Projects a default 20 x 30 m parcel at a world position without mutating storage.
    pub fn preview_default_parcel_at(
        &self,
        world_x: f32,
        world_z: f32,
        graph: &RegionGraph,
    ) -> Result<ParcelGeometry, ParcelPlacementError> {
        self.preview_parcel_at(
            world_x,
            world_z,
            DEFAULT_PARCEL_FRONTAGE_M,
            DEFAULT_PARCEL_DEPTH_M,
            graph,
        )
    }

    /// Projects a parcel with caller-selected dimensions without mutating storage.
    pub fn preview_parcel_at(
        &self,
        world_x: f32,
        world_z: f32,
        frontage_m: f32,
        depth_m: f32,
        graph: &RegionGraph,
    ) -> Result<ParcelGeometry, ParcelPlacementError> {
        Self::validate_parcel_dimensions(frontage_m, depth_m)?;
        let geometry = parcels::project_default_parcel_at(
            graph,
            Vector2::new(world_x, world_z),
            frontage_m,
            depth_m,
        )?;
        if !parcels::geometry_inside_world(&geometry, self.config.width_m, self.config.height_m) {
            return Err(ParcelPlacementError::OutsideWorld);
        }
        if self.parcels.overlaps_existing(&geometry) {
            return Err(ParcelPlacementError::OverlapsExistingParcel);
        }
        if parcels::geometry_overlaps_road(graph, &geometry) {
            return Err(ParcelPlacementError::OverlapsRoad);
        }
        Ok(geometry)
    }

    /// Returns true when one world-space point is inside an authored parcel.
    pub fn has_parcel_at(&self, world_x: f32, world_z: f32) -> bool {
        self.parcels
            .find_at_point(Vector2::new(world_x, world_z))
            .is_some()
    }

    /// Returns the runtime zoning-profile id of the parcel under one world-space point.
    pub fn parcel_profile_runtime_id_at(&self, world_x: f32, world_z: f32) -> Option<u16> {
        let id = self.parcels.find_at_point(Vector2::new(world_x, world_z))?;
        self.parcels
            .get(id)
            .map(|parcel| parcel.zone_profile_runtime_id())
    }

    /// Returns the authored parcel geometry under one world-space point.
    pub fn parcel_geometry_at(&self, world_x: f32, world_z: f32) -> Option<ParcelGeometry> {
        let id = self.parcels.find_at_point(Vector2::new(world_x, world_z))?;
        self.parcels.get(id).map(parcels::geometry_for_parcel)
    }

    /// Projects an all-or-nothing same-road parcel run without mutating storage.
    pub fn preview_parcel_run_at(
        &self,
        start_x: f32,
        start_z: f32,
        end_x: f32,
        end_z: f32,
        frontage_m: f32,
        depth_m: f32,
        gap_m: f32,
        graph: &RegionGraph,
    ) -> Result<Vec<ParcelGeometry>, ParcelPlacementError> {
        Self::validate_parcel_dimensions(frontage_m, depth_m)?;
        Self::validate_parcel_gap(gap_m)?;
        let start_point = Vector2::new(start_x, start_z);
        let end_point = Vector2::new(end_x, end_z);
        let existing_start = self
            .parcels
            .find_at_point(start_point)
            .and_then(|id| self.parcels.get(id))
            .map(parcels::geometry_for_parcel);
        let geometries = if let Some(existing_geometry) = existing_start.as_ref() {
            parcels::project_parcel_run_from_existing(
                graph,
                existing_geometry,
                end_point,
                frontage_m,
                depth_m,
                gap_m,
            )?
        } else {
            parcels::project_parcel_run_at(
                graph,
                start_point,
                end_point,
                frontage_m,
                depth_m,
                gap_m,
            )?
        };
        self.validate_parcel_run_geometries(&geometries, graph)?;
        Ok(geometries)
    }

    /// Returns authored parcel geometries touched by one world-space zoning paint stroke.
    pub fn preview_rezone_stroke(
        &self,
        start_x: f32,
        start_z: f32,
        end_x: f32,
        end_z: f32,
    ) -> Vec<ParcelGeometry> {
        self.parcels
            .find_touching_segment(Vector2::new(start_x, start_z), Vector2::new(end_x, end_z))
            .into_iter()
            .filter_map(|id| self.parcels.get(id).map(parcels::geometry_for_parcel))
            .collect()
    }

    /// Creates a new parcel or changes the profile of the parcel under the given world position.
    ///
    /// Runtime id `0` creates or assigns a free/unzoned parcel.
    pub fn place_or_rezone_default_parcel_at(
        &mut self,
        world_x: f32,
        world_z: f32,
        runtime_id: u16,
        graph: &RegionGraph,
    ) -> Result<ParcelId, ParcelPlacementError> {
        self.place_or_rezone_parcel_at(
            world_x,
            world_z,
            runtime_id,
            DEFAULT_PARCEL_FRONTAGE_M,
            DEFAULT_PARCEL_DEPTH_M,
            graph,
        )
    }

    /// Creates or rezones a parcel using caller-selected frontage and depth.
    ///
    /// Runtime id `0` creates or assigns a free/unzoned parcel.
    pub fn place_or_rezone_parcel_at(
        &mut self,
        world_x: f32,
        world_z: f32,
        runtime_id: u16,
        frontage_m: f32,
        depth_m: f32,
        graph: &RegionGraph,
    ) -> Result<ParcelId, ParcelPlacementError> {
        self.validate_profile_id(runtime_id)?;
        Self::validate_parcel_dimensions(frontage_m, depth_m)?;
        let point = Vector2::new(world_x, world_z);
        if let Some(existing_id) = self.parcels.find_at_point(point) {
            self.parcels
                .set_zone_profile_runtime_id(existing_id, runtime_id);
            return Ok(existing_id);
        }

        let geometry = self.preview_parcel_at(world_x, world_z, frontage_m, depth_m, graph)?;
        Ok(self.parcels.insert_new(geometry, runtime_id))
    }

    /// Creates a same-road parcel run using the requested gap as minimum center spacing.
    ///
    /// Existing-parcel overlap or out-of-bounds geometry rejects the whole run. On curved roads,
    /// Rust may widen spacing between generated parcels to preserve non-overlap, then stops when
    /// no further parcel can fit inside the drag span.
    pub fn place_parcel_run_at(
        &mut self,
        start_x: f32,
        start_z: f32,
        end_x: f32,
        end_z: f32,
        runtime_id: u16,
        frontage_m: f32,
        depth_m: f32,
        gap_m: f32,
        graph: &RegionGraph,
    ) -> Result<Vec<ParcelId>, ParcelPlacementError> {
        self.validate_profile_id(runtime_id)?;
        let geometries = self.preview_parcel_run_at(
            start_x, start_z, end_x, end_z, frontage_m, depth_m, gap_m, graph,
        )?;
        let mut ids = Vec::with_capacity(geometries.len());
        for geometry in geometries {
            ids.push(self.parcels.insert_new(geometry, runtime_id));
        }
        Ok(ids)
    }

    /// Changes the zoning profile of every authored parcel touched by one world-space stroke.
    pub fn rezone_stroke(
        &mut self,
        start_x: f32,
        start_z: f32,
        end_x: f32,
        end_z: f32,
        runtime_id: u16,
    ) -> Result<Vec<ParcelId>, ParcelPlacementError> {
        self.validate_profile_id(runtime_id)?;
        let ids = self
            .parcels
            .find_touching_segment(Vector2::new(start_x, start_z), Vector2::new(end_x, end_z));
        if ids.is_empty() {
            return Err(ParcelPlacementError::NoRoadAttachment);
        }
        for id in &ids {
            self.parcels.set_zone_profile_runtime_id(*id, runtime_id);
        }
        Ok(ids)
    }

    /// Restores one saved parcel from road attachment data.
    pub fn restore_parcel_from_attachment(
        &mut self,
        parcel_id: u64,
        edge_idx: usize,
        side: i8,
        frontage_center_t: f32,
        frontage_m: f32,
        depth_m: f32,
        runtime_id: u16,
        graph: &RegionGraph,
    ) -> Result<ParcelId, ParcelPlacementError> {
        self.validate_profile_id(runtime_id)?;
        Self::validate_parcel_dimensions(frontage_m, depth_m)?;
        if edge_idx >= graph.edge_count() {
            return Err(ParcelPlacementError::NoRoadAttachment);
        }
        let edge = graph.edge(edge_idx);
        if edge.deleted
            || edge.no_building_spawn
            || edge.physical_geometry.len() < 2
            || edge.physical_length <= frontage_m
        {
            return Err(ParcelPlacementError::NoRoadAttachment);
        }
        let s_m = frontage_center_t.clamp(0.0, 1.0) * edge.physical_length;
        if s_m < frontage_m * 0.5 || s_m > edge.physical_length - frontage_m * 0.5 {
            return Err(ParcelPlacementError::FrontageOutOfBounds);
        }
        let id = ParcelId::from_raw(parcel_id);
        if id.is_none() {
            return Err(ParcelPlacementError::NoRoadAttachment);
        }
        let geometry = parcels::geometry_from_attachment(
            graph,
            edge_idx,
            if side >= 0 { 1 } else { -1 },
            frontage_center_t,
            frontage_m,
            depth_m,
        );
        if !parcels::geometry_inside_world(&geometry, self.config.width_m, self.config.height_m) {
            return Err(ParcelPlacementError::OutsideWorld);
        }
        if self.parcels.overlaps_existing(&geometry) {
            return Err(ParcelPlacementError::OverlapsExistingParcel);
        }
        if parcels::geometry_overlaps_road(graph, &geometry) {
            return Err(ParcelPlacementError::OverlapsRoad);
        }
        self.parcels.insert_loaded(id, geometry, runtime_id);
        Ok(id)
    }

    fn validate_parcel_dimensions(
        frontage_m: f32,
        depth_m: f32,
    ) -> Result<(), ParcelPlacementError> {
        if !frontage_m.is_finite()
            || !depth_m.is_finite()
            || !(MIN_PARCEL_FRONTAGE_M..=MAX_PARCEL_FRONTAGE_M).contains(&frontage_m)
            || !(MIN_PARCEL_DEPTH_M..=MAX_PARCEL_DEPTH_M).contains(&depth_m)
        {
            return Err(ParcelPlacementError::InvalidDimensions);
        }
        Ok(())
    }

    fn validate_parcel_gap(gap_m: f32) -> Result<(), ParcelPlacementError> {
        if !gap_m.is_finite() || !(MIN_PARCEL_GAP_M..=MAX_PARCEL_GAP_M).contains(&gap_m) {
            return Err(ParcelPlacementError::InvalidGap);
        }
        Ok(())
    }

    fn validate_parcel_run_geometries(
        &self,
        geometries: &[ParcelGeometry],
        graph: &RegionGraph,
    ) -> Result<(), ParcelPlacementError> {
        for geometry in geometries {
            if !parcels::geometry_inside_world(geometry, self.config.width_m, self.config.height_m)
            {
                return Err(ParcelPlacementError::OutsideWorld);
            }
        }
        if parcels::any_geometry_overlaps_road(graph, geometries) {
            return Err(ParcelPlacementError::OverlapsRoad);
        }
        if self.parcels.overlaps_any_existing(geometries)
            || parcels::geometries_have_overlap(geometries)
        {
            return Err(ParcelPlacementError::OverlapsExistingParcel);
        }
        Ok(())
    }

    /// Claims one parcel for a building index.
    pub fn occupy_parcel(&mut self, parcel_id: u64, building_idx: usize) -> bool {
        self.parcels
            .set_occupied_building(ParcelId::from_raw(parcel_id), building_idx)
    }

    /// Clears a parcel building claim.
    pub fn clear_parcel_occupancy(&mut self, parcel_id: u64) -> bool {
        self.parcels
            .clear_occupied_building(ParcelId::from_raw(parcel_id))
    }

    /// Remaps a building index inside parcel occupancy after allocator swap-remove.
    pub fn remap_parcel_occupancy(&mut self, old_idx: usize, new_idx: usize) {
        self.parcels.remap_occupied_building(old_idx, new_idx);
    }

    /// Clears every parcel occupancy claim.
    pub fn clear_all_parcel_occupancy(&mut self) {
        self.parcels.clear_all_occupancy();
    }

    fn validate_profile_id(&self, runtime_id: u16) -> Result<(), ParcelPlacementError> {
        if runtime_id == 0 || self.profiles.profile_by_runtime_id(runtime_id).is_some() {
            Ok(())
        } else {
            Err(ParcelPlacementError::UnknownProfile)
        }
    }
}
