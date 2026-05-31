//! Road-aligned zoning parcels and built-in zoning-profile registry.
//!
//! User-authored parcels are the zoning authority for private building spawn. Broad zoning-family
//! values remain derived helpers for systems that consume residential/commercial/industrial
//! families. The dense profile grid remains only for deprecated paint-patch tooling while the
//! simulation, demand, allocator, and saves consume stable parcel ids.

use crate::simulation::core::config::WorldConfig;
use crate::simulation::grid::data_grid::DataGrid;
use godot::prelude::Vector2;
use rayon::prelude::*;
use std::sync::Arc;

pub mod parcels;
pub mod profiles;

pub use parcels::{ParcelGeometry, ParcelId, ParcelPlacementError, ParcelStore, ZoningParcel};
pub use profiles::{
    ZoneDensity, ZoneProfileRuntime, ZoningProfileRegistry, load_builtin_profile_registry,
};

/// First-slice authored parcel frontage in metres.
pub const DEFAULT_PARCEL_FRONTAGE_M: f32 = 20.0;
/// First-slice authored parcel depth in metres.
pub const DEFAULT_PARCEL_DEPTH_M: f32 = 30.0;
/// Smallest player-authored parcel frontage accepted by the Rust placement path.
pub const MIN_PARCEL_FRONTAGE_M: f32 = 5.0;
/// Largest player-authored parcel frontage accepted by the Rust placement path.
pub const MAX_PARCEL_FRONTAGE_M: f32 = 80.0;
/// Smallest player-authored parcel depth accepted by the Rust placement path.
pub const MIN_PARCEL_DEPTH_M: f32 = 5.0;
/// Largest player-authored parcel depth accepted by the Rust placement path.
pub const MAX_PARCEL_DEPTH_M: f32 = 120.0;
/// Smallest spacing between generated drag-run parcels.
pub const MIN_PARCEL_GAP_M: f32 = 0.0;
/// Largest spacing between generated drag-run parcels.
pub const MAX_PARCEL_GAP_M: f32 = 20.0;

/// Land-use category painted onto a zoning grid cell.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
#[repr(u8)]
pub enum ZoneType {
    /// No zoning — cell is unbuildable and transparent in the UI.
    #[default]
    None = 0,
    /// Residential housing — agents live here, consumes residential demand.
    Residential = 1,
    /// Retail / services — agents shop and work here, consumes commercial demand.
    Commercial = 2,
    /// Manufacturing / logistics — agents work here, consumes industrial demand.
    Industrial = 3,
    /// Office employment reserved for a later explicit extension.
    Office = 4,
    /// Mixed residential/commercial use reserved for a later explicit extension.
    Mixed = 5,
}

impl ZoneType {
    /// Converts a raw `u8` to a `ZoneType`. Unknown values map to `None`.
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Residential,
            2 => Self::Commercial,
            3 => Self::Industrial,
            4 => Self::Office,
            5 => Self::Mixed,
            _ => Self::None,
        }
    }

    /// Returns the canonical snake-case string key for this zone family.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Residential => "residential",
            Self::Commercial => "commercial",
            Self::Industrial => "industrial",
            Self::Office => "office",
            Self::Mixed => "mixed",
        }
    }
}

/// Road-aligned parcel zoning system with supporting debug/display grids.
///
/// Parcels use stable ids and road attachment metadata. Supporting grids use the map-centred
/// origin: cell (cx, cy) sits at world position
/// `(-width_m / 2 + (cx + 0.5) * zone_cell_m, -height_m / 2 + (cy + 0.5) * zone_cell_m)`.
#[derive(Clone)]
pub struct ZoningSystem {
    /// Validated built-in zoning-profile registry shared by the zoning grid, UI bridge, and saves.
    pub profiles: Arc<ZoningProfileRegistry>,
    /// Stable road-aligned parcel store used as zoning authority.
    pub parcels: ParcelStore,
    /// Deprecated dense runtime zoning-profile id buffer for old patch APIs. Not simulation authority.
    pub grid: DataGrid<u16>,
    /// Building footprint occupancy. True when a placed building covers this cell.
    pub occupied: DataGrid<bool>,
    /// Distance to the nearest road edge in metres, clamped to 255.
    /// Updated after every road placement; drives shader-side roadless-zone dimming.
    pub distance_to_road: DataGrid<u8>,
    /// True for cells within building-spawn depth of a `no_building_spawn` edge.
    /// The shader suppresses zone tint here so the player reads no-build flanks as unbuildable.
    pub no_build_mask: DataGrid<bool>,
    /// World configuration (extent, chunk metadata, cell sizes).
    pub config: WorldConfig,
}

impl ZoningSystem {
    /// Creates a new, empty zoning system sized to the map in `config`.
    pub fn new(config: &WorldConfig) -> Self {
        let w = config.zone_grid_width();
        let h = config.zone_grid_height();
        let profiles = load_builtin_profile_registry()
            .unwrap_or_else(|err| panic!("could not load built-in zoning profiles: {err}"));
        Self {
            profiles,
            parcels: ParcelStore::default(),
            grid: DataGrid::new(w, h, 0),
            occupied: DataGrid::new(w, h, false),
            distance_to_road: DataGrid::new(w, h, 255u8),
            no_build_mask: DataGrid::new(w, h, false),
            config: *config,
        }
    }

    /// Clears all zone, occupancy, distance, and no-build mask data.
    pub fn clear(&mut self) {
        self.parcels.clear();
        self.grid.data.fill(0);
        self.occupied.data.fill(false);
        self.distance_to_road.data.fill(255);
        self.no_build_mask.data.fill(false);
    }

    /// Remaps parcel road-edge attachments after network compaction.
    pub fn update_edge_indices(&mut self, mapping: &std::collections::HashMap<usize, usize>) {
        self.parcels.remove_edges_not_in_mapping(mapping);
    }

    // -- Parcel authority ----------------------------------------------------

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
        graph: &crate::simulation::network::graph::RegionGraph,
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
        graph: &crate::simulation::network::graph::RegionGraph,
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
        Ok(geometry)
    }

    /// Returns true when one world-space point is inside an authored parcel.
    pub fn has_parcel_at(&self, world_x: f32, world_z: f32) -> bool {
        self.parcels
            .find_at_point(Vector2::new(world_x, world_z))
            .is_some()
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
        graph: &crate::simulation::network::graph::RegionGraph,
    ) -> Result<Vec<ParcelGeometry>, ParcelPlacementError> {
        Self::validate_parcel_dimensions(frontage_m, depth_m)?;
        Self::validate_parcel_gap(gap_m)?;
        let geometries = parcels::project_parcel_run_at(
            graph,
            Vector2::new(start_x, start_z),
            Vector2::new(end_x, end_z),
            frontage_m,
            depth_m,
            gap_m,
        )?;
        self.validate_parcel_run_geometries(&geometries)?;
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
        graph: &crate::simulation::network::graph::RegionGraph,
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
        graph: &crate::simulation::network::graph::RegionGraph,
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

    /// Creates an all-or-nothing same-road parcel run.
    ///
    /// Drag-run placement never silently skips invalid or occupied space. Any overlap or out of
    /// bounds geometry rejects the whole run.
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
        graph: &crate::simulation::network::graph::RegionGraph,
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
        graph: &crate::simulation::network::graph::RegionGraph,
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
    ) -> Result<(), ParcelPlacementError> {
        for geometry in geometries {
            if !parcels::geometry_inside_world(geometry, self.config.width_m, self.config.height_m)
            {
                return Err(ParcelPlacementError::OutsideWorld);
            }
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

    // ── Coordinate helpers ──────────────────────────────────────────────────

    /// Converts a world-space position to a grid cell, returning `None` if out of bounds.
    fn world_to_cell(&self, x: f32, z: f32) -> Option<(usize, usize)> {
        let w = self.grid.width;
        let h = self.grid.height;
        let half_w = self.config.width_m * 0.5;
        let half_h = self.config.height_m * 0.5;
        let cell = self.config.zone_cell_m;
        let cx = ((x + half_w) / cell - 0.5).round();
        let cy = ((z + half_h) / cell - 0.5).round();
        if cx < 0.0 || cy < 0.0 || cx >= w as f32 || cy >= h as f32 {
            return None;
        }
        Some((cx as usize, cy as usize))
    }

    /// World → cell, clamping to grid bounds instead of returning `None`.
    fn world_to_cell_clamped(&self, x: f32, z: f32) -> (usize, usize) {
        let w = self.grid.width as i64;
        let h = self.grid.height as i64;
        let half_w = self.config.width_m * 0.5;
        let half_h = self.config.height_m * 0.5;
        let cell = self.config.zone_cell_m;
        let cx = ((x + half_w) / cell - 0.5).round() as i64;
        let cy = ((z + half_h) / cell - 0.5).round() as i64;
        (cx.clamp(0, w - 1) as usize, cy.clamp(0, h - 1) as usize)
    }

    /// Returns the world-space centre of grid cell `(cx, cy)`.
    fn cell_to_world(&self, cx: usize, cy: usize) -> (f32, f32) {
        (
            -self.config.width_m * 0.5 + (cx as f32 + 0.5) * self.config.zone_cell_m,
            -self.config.height_m * 0.5 + (cy as f32 + 0.5) * self.config.zone_cell_m,
        )
    }

    // ── Zone read / write ───────────────────────────────────────────────────

    /// Returns the distance-to-road (in metres) at the given world-space position.
    /// Returns 255 if the position is out of bounds or the grid is uninitialized.
    pub fn distance_to_road_world(&self, x: f32, z: f32) -> u8 {
        match self.world_to_cell(x, z) {
            Some((cx, cy)) => *self.distance_to_road.get(cx, cy).unwrap_or(&255),
            None => 255,
        }
    }

    /// Returns the dense runtime profile id at one world-space position.
    pub fn get_zone_profile_runtime_id_world(&self, x: f32, z: f32) -> u16 {
        match self.world_to_cell(x, z) {
            Some((cx, cy)) => *self.grid.get(cx, cy).unwrap_or(&0),
            None => 0,
        }
    }

    /// Returns the dense runtime profile id at one grid-space cell.
    pub fn get_zone_profile_runtime_id_cell(&self, cx: usize, cy: usize) -> u16 {
        *self.grid.get(cx, cy).unwrap_or(&0)
    }

    /// Paints a world-space rectangle with one runtime zoning-profile id.
    ///
    /// Cell boundaries are determined by snapping to the nearest zoning grid boundary.
    pub fn set_zone_profile_rect(
        &mut self,
        x_min: f32,
        z_min: f32,
        x_max: f32,
        z_max: f32,
        runtime_id: u16,
    ) {
        let (cx_min, cy_min) = self.world_to_cell_clamped(x_min.min(x_max), z_min.min(z_max));
        let (cx_max, cy_max) = self.world_to_cell_clamped(x_min.max(x_max), z_min.max(z_max));
        let gw = self.grid.width;
        let gh = self.grid.height;
        for cy in cy_min..=cy_max.min(gh.saturating_sub(1)) {
            for cx in cx_min..=cx_max.min(gw.saturating_sub(1)) {
                self.grid.set(cx, cy, runtime_id);
            }
        }
    }

    /// Captures one patch bounding box as little-endian runtime profile ids in row-major order.
    pub fn capture_patch(
        &self,
        grid_x: i32,
        grid_y: i32,
        width_cells: usize,
        height_cells: usize,
    ) -> Vec<u8> {
        let mut out = Vec::with_capacity(width_cells * height_cells * 2);
        for dy in 0..height_cells {
            for dx in 0..width_cells {
                let cx = grid_x + dx as i32;
                let cy = grid_y + dy as i32;
                let runtime_id = if cx >= 0
                    && cy >= 0
                    && (cx as usize) < self.grid.width
                    && (cy as usize) < self.grid.height
                {
                    self.get_zone_profile_runtime_id_cell(cx as usize, cy as usize)
                } else {
                    0
                };
                out.extend_from_slice(&runtime_id.to_le_bytes());
            }
        }
        out
    }

    /// Restores one full patch bounding box from little-endian runtime profile ids.
    pub fn restore_patch(
        &mut self,
        grid_x: i32,
        grid_y: i32,
        width_cells: usize,
        height_cells: usize,
        profile_ids_le_u16: &[u8],
    ) {
        let mut idx = 0;
        for dy in 0..height_cells {
            for dx in 0..width_cells {
                if idx + 1 >= profile_ids_le_u16.len() {
                    return;
                }
                let cx = grid_x + dx as i32;
                let cy = grid_y + dy as i32;
                if cx >= 0
                    && cy >= 0
                    && (cx as usize) < self.grid.width
                    && (cy as usize) < self.grid.height
                {
                    let runtime_id =
                        u16::from_le_bytes([profile_ids_le_u16[idx], profile_ids_le_u16[idx + 1]]);
                    self.grid.set(cx as usize, cy as usize, runtime_id);
                }
                idx += 2;
            }
        }
    }

    /// Applies one masked paint patch using row-major `0/1` bytes.
    pub fn apply_patch(
        &mut self,
        grid_x: i32,
        grid_y: i32,
        width_cells: usize,
        height_cells: usize,
        runtime_id: u16,
        write_mask: &[u8],
    ) {
        let mut idx = 0;
        for dy in 0..height_cells {
            for dx in 0..width_cells {
                if idx >= write_mask.len() {
                    return;
                }
                if write_mask[idx] != 0 {
                    let cx = grid_x + dx as i32;
                    let cy = grid_y + dy as i32;
                    if cx >= 0
                        && cy >= 0
                        && (cx as usize) < self.grid.width
                        && (cy as usize) < self.grid.height
                    {
                        self.grid.set(cx as usize, cy as usize, runtime_id);
                    }
                }
                idx += 1;
            }
        }
    }

    // ── Texture data for Godot uploads ──────────────────────────────────────

    /// Returns the authoritative profile-id grid as RG8 bytes for overlay upload.
    pub fn get_zone_profile_texture_data_rg8(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.grid.data.len() * 2);
        for &runtime_id in &self.grid.data {
            out.extend_from_slice(&runtime_id.to_le_bytes());
        }
        out
    }

    /// Returns the one-row RGBA8 style LUT for the profile-aware overlay shader.
    pub fn get_zone_profile_style_lut_rgba8(&self) -> Vec<u8> {
        self.profiles.style_lut_rgba8()
    }

    /// Returns the occupancy grid as a flat `u8` byte array (0 or 1 per cell).
    pub fn get_occupied_texture_data(&self) -> Vec<u8> {
        self.occupied
            .data
            .iter()
            .map(|&b| if b { 255 } else { 0 })
            .collect()
    }

    /// Returns the distance-to-road grid as a flat `u8` byte array.
    pub fn get_distance_texture_data(&self) -> Vec<u8> {
        self.distance_to_road.data.clone()
    }

    /// Returns the no-build mask as a flat `u8` byte array (0 or 255 per cell).
    pub fn get_no_build_mask_texture_data(&self) -> Vec<u8> {
        self.no_build_mask
            .data
            .iter()
            .map(|&b| if b { 255 } else { 0 })
            .collect()
    }

    // ── Occupancy (rotated-rect helpers) ────────────────────────────────────

    /// Marks or clears all world-grid cells covered by an oriented building footprint.
    ///
    /// `tangent` is the unit road-direction vector at the building's frontage point.
    /// `width_m` spans along the tangent; `depth_m` spans along the outward normal.
    pub fn mark_occupied_rect(
        &mut self,
        center_x: f32,
        center_z: f32,
        tangent: Vector2,
        width_m: f32,
        depth_m: f32,
        val: bool,
    ) {
        let half_w = width_m * 0.5;
        let half_d = depth_m * 0.5;
        let cell = self.config.zone_cell_m;
        let normal = Vector2::new(-tangent.y, tangent.x);
        // AABB padding: worst-case rotation adds half_w + half_d to each axis.
        let aabb_half = half_w + half_d + cell;
        let (ax_min, az_min) =
            self.world_to_cell_clamped(center_x - aabb_half, center_z - aabb_half);
        let (ax_max, az_max) =
            self.world_to_cell_clamped(center_x + aabb_half, center_z + aabb_half);
        let gw = self.occupied.width;
        let gh = self.occupied.height;
        for cy in az_min..=az_max.min(gh.saturating_sub(1)) {
            for cx in ax_min..=ax_max.min(gw.saturating_sub(1)) {
                let (wx, wz) = self.cell_to_world(cx, cy);
                let dx = wx - center_x;
                let dz = wz - center_z;
                let along = dx * tangent.x + dz * tangent.y;
                let perp = dx * normal.x + dz * normal.y;
                if along.abs() <= half_w && perp.abs() <= half_d {
                    self.occupied.set(cx, cy, val);
                }
            }
        }
    }

    /// Returns `true` if any world-grid cell inside an oriented building footprint is occupied.
    pub fn is_rect_occupied(
        &self,
        center_x: f32,
        center_z: f32,
        tangent: Vector2,
        width_m: f32,
        depth_m: f32,
    ) -> bool {
        let half_w = width_m * 0.5;
        let half_d = depth_m * 0.5;
        let cell = self.config.zone_cell_m;
        let normal = Vector2::new(-tangent.y, tangent.x);
        let aabb_half = half_w + half_d + cell;
        let (ax_min, az_min) =
            self.world_to_cell_clamped(center_x - aabb_half, center_z - aabb_half);
        let (ax_max, az_max) =
            self.world_to_cell_clamped(center_x + aabb_half, center_z + aabb_half);
        let gw = self.occupied.width;
        let gh = self.occupied.height;
        for cy in az_min..=az_max.min(gh.saturating_sub(1)) {
            for cx in ax_min..=ax_max.min(gw.saturating_sub(1)) {
                let (wx, wz) = self.cell_to_world(cx, cy);
                let dx = wx - center_x;
                let dz = wz - center_z;
                let along = dx * tangent.x + dz * tangent.y;
                let perp = dx * normal.x + dz * normal.y;
                if along.abs() <= half_w && perp.abs() <= half_d {
                    if *self.occupied.get(cx, cy).unwrap_or(&false) {
                        return true;
                    }
                }
            }
        }
        false
    }

    // ── Distance to road ────────────────────────────────────────────────────

    /// Recomputes the distance-to-road grid from the current road network.
    ///
    /// For each cell, stores the metres to the nearest road edge's boundary
    /// (centreline distance minus half-width, clamped to 0). Clamped to 255.
    /// Called once after every road placement. O(cells × edge segments) with rayon.
    pub fn update_distance_to_road(
        &mut self,
        graph: &crate::simulation::network::graph::RegionGraph,
    ) {
        // Collect edge segments: (ax, az, bx, bz, half_width)
        let segments: Vec<(f32, f32, f32, f32, f32)> = graph
            .edges()
            .iter()
            .filter(|e| !e.deleted)
            .flat_map(|e| {
                let hw = e.width * 0.5;
                e.physical_geometry
                    .windows(2)
                    .map(move |w| (w[0].x, w[0].z, w[1].x, w[1].z, hw))
            })
            .collect();

        let w = self.grid.width;
        let h = self.grid.height;
        let cell = self.config.zone_cell_m;
        let half_w = self.config.width_m * 0.5;
        let half_h = self.config.height_m * 0.5;

        let distances: Vec<u8> = (0..w * h)
            .into_par_iter()
            .map(|i| {
                let cx = i % w;
                let cy = i / w;
                let px = -half_w + (cx as f32 + 0.5) * cell;
                let pz = -half_h + (cy as f32 + 0.5) * cell;

                if segments.is_empty() {
                    return 255u8;
                }
                let min_dist = segments
                    .iter()
                    .map(|&(ax, az, bx, bz, half_w)| {
                        let abx = bx - ax;
                        let abz = bz - az;
                        let len_sq = abx * abx + abz * abz;
                        let (cx2, cz2) = if len_sq < 1e-6 {
                            (ax, az)
                        } else {
                            let t = ((px - ax) * abx + (pz - az) * abz) / len_sq;
                            let t = t.clamp(0.0, 1.0);
                            (ax + t * abx, az + t * abz)
                        };
                        let dx = px - cx2;
                        let dz = pz - cz2;
                        let dist = (dx * dx + dz * dz).sqrt();
                        (dist - half_w).max(0.0)
                    })
                    .fold(f32::MAX, f32::min);
                min_dist.min(255.0) as u8
            })
            .collect();

        self.distance_to_road.data = distances;
        self.update_no_build_mask(graph);
    }

    /// Recomputes the no-build mask from the current set of `no_building_spawn` edges.
    ///
    /// Marks cells within `SIDEWALK_WIDTH + 3 × zone_cell_m` (~31.5 m) of the carriageway
    /// edge of any flagged road. The shader suppresses zone tint on these cells so the player
    /// can read that the zone alongside a no-build road will not develop.
    ///
    /// Call whenever the `no_building_spawn` flag changes on any edge (in addition to the
    /// automatic call at the end of `update_distance_to_road`).
    pub fn update_no_build_mask(&mut self, graph: &crate::simulation::network::graph::RegionGraph) {
        // Distance from road surface (carriageway edge) within which zone tint is suppressed.
        // Covers SIDEWALK_WIDTH (1.5 m) + 3 plot rows (30 m) = 31.5 m → use 32 m.
        const SUPPRESS_M: f32 = 32.0;

        let segments: Vec<(f32, f32, f32, f32, f32)> = graph
            .edges()
            .iter()
            .filter(|e| !e.deleted && e.no_building_spawn)
            .flat_map(|e| {
                let hw = e.width * 0.5;
                e.physical_geometry
                    .windows(2)
                    .map(move |w| (w[0].x, w[0].z, w[1].x, w[1].z, hw))
            })
            .collect();

        let w = self.grid.width;
        let h = self.grid.height;
        let cell = self.config.zone_cell_m;
        let half_w = self.config.width_m * 0.5;
        let half_h = self.config.height_m * 0.5;

        let mask: Vec<bool> = (0..w * h)
            .into_par_iter()
            .map(|i| {
                if segments.is_empty() {
                    return false;
                }
                let cx = i % w;
                let cy = i / w;
                let px = -half_w + (cx as f32 + 0.5) * cell;
                let pz = -half_h + (cy as f32 + 0.5) * cell;
                segments.iter().any(|&(ax, az, bx, bz, half_w)| {
                    let abx = bx - ax;
                    let abz = bz - az;
                    let len_sq = abx * abx + abz * abz;
                    let (cx2, cz2) = if len_sq < 1e-6 {
                        (ax, az)
                    } else {
                        let t = ((px - ax) * abx + (pz - az) * abz) / len_sq;
                        let t = t.clamp(0.0, 1.0);
                        (ax + t * abx, az + t * abz)
                    };
                    let dx = px - cx2;
                    let dz = pz - cz2;
                    let dist = (dx * dx + dz * dz).sqrt() - half_w;
                    dist.max(0.0) <= SUPPRESS_M
                })
            })
            .collect();

        self.no_build_mask.data = mask;
    }
}

/// Unit tests for the world-space zoning system.
#[cfg(test)]
pub mod tests;
