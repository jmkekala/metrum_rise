//! World-space zoning grid and built-in zoning-profile registry.
//!
//! The authoritative painted world stores dense runtime `ZoneProfile` ids (`u16`) in one global
//! grid. Broad [`ZoneType`] values remain as derived helpers for systems that still consume the
//! larger residential/commercial/industrial families. Buildings still spawn along road frontage,
//! but legality now reads the profile-derived zone family and density from the global painted grid
//! rather than from edge-local cells.

use crate::simulation::core::config::MapConfig;
use crate::simulation::grid::data_grid::DataGrid;
use godot::prelude::Vector2;
use rayon::prelude::*;
use std::sync::Arc;

pub mod profiles;

pub use profiles::{
    ZoneDensity, ZoneProfileRuntime, ZoningProfileRegistry, load_builtin_profile_registry,
};

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
    /// Office employment — treated as commercial demand at 50% weight currently.
    Office = 4,
    /// Dual-use: serves as both residential and commercial, consumes both demands.
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

/// World-space zoning system built on three flat grids.
///
/// Replaces the legacy per-edge `EdgeZoning` / `flush_zoning_updates` pipeline.
/// All grid coordinates use the map-centred origin: cell (cx, cy) sits at world
/// position `((cx - hw) * zone_cell_m, (cy - hh) * zone_cell_m)` where
/// `hw = (width - 1) / 2`.
#[derive(Clone)]
pub struct ZoningSystem {
    /// Validated built-in zoning-profile registry shared by the zoning grid, UI bridge, and saves.
    pub profiles: Arc<ZoningProfileRegistry>,
    /// Dense runtime zoning-profile id for every painted world cell. `0` means unpainted / none.
    pub grid: DataGrid<u16>,
    /// Building footprint occupancy. True when a placed building covers this cell.
    pub occupied: DataGrid<bool>,
    /// Distance to the nearest road edge in metres, clamped to 255.
    /// Updated after every road placement; drives shader-side roadless-zone dimming.
    pub distance_to_road: DataGrid<u8>,
    /// True for cells within building-spawn depth of a `no_building_spawn` edge.
    /// The shader suppresses zone tint here so the player reads no-build flanks as unbuildable.
    pub no_build_mask: DataGrid<bool>,
    /// Map configuration (dimensions, cell sizes).
    pub config: MapConfig,
}

impl ZoningSystem {
    /// Creates a new, empty zoning system sized to the map in `config`.
    pub fn new(config: &MapConfig) -> Self {
        let w = config.zone_grid_width();
        let h = config.zone_grid_height();
        let profiles = load_builtin_profile_registry()
            .unwrap_or_else(|err| panic!("could not load built-in zoning profiles: {err}"));
        Self {
            profiles,
            grid: DataGrid::new(w, h, 0),
            occupied: DataGrid::new(w, h, false),
            distance_to_road: DataGrid::new(w, h, 255u8),
            no_build_mask: DataGrid::new(w, h, false),
            config: *config,
        }
    }

    /// Clears all zone, occupancy, distance, and no-build mask data.
    pub fn clear(&mut self) {
        self.grid.data.fill(0);
        self.occupied.data.fill(false);
        self.distance_to_road.data.fill(255);
        self.no_build_mask.data.fill(false);
    }

    /// No-op kept for call-site compatibility with network compaction.
    ///
    /// The global grid has no per-edge keys to remap; only `BuildingAllocator::edge_occupancy`
    /// needs remapping (handled in `BuildingAllocator::update_edge_indices`).
    pub fn update_edge_indices(&mut self, _mapping: &std::collections::HashMap<usize, usize>) {}

    // ── Coordinate helpers ──────────────────────────────────────────────────

    /// Converts a world-space position to a grid cell, returning `None` if out of bounds.
    fn world_to_cell(&self, x: f32, z: f32) -> Option<(usize, usize)> {
        let w = self.grid.width;
        let h = self.grid.height;
        let hw = (w as f32 - 1.0) * 0.5;
        let hh = (h as f32 - 1.0) * 0.5;
        // World coords are in heightmap pixels (1 m per pixel). Zone grid has one
        // cell per heightmap pixel, so scale is 1:1.
        let cx = (x + hw).round();
        let cy = (z + hh).round();
        if cx < 0.0 || cy < 0.0 || cx >= w as f32 || cy >= h as f32 {
            return None;
        }
        Some((cx as usize, cy as usize))
    }

    /// World → cell, clamping to grid bounds instead of returning `None`.
    fn world_to_cell_clamped(&self, x: f32, z: f32) -> (usize, usize) {
        let w = self.grid.width as i64;
        let h = self.grid.height as i64;
        let hw = (self.grid.width as f32 - 1.0) * 0.5;
        let hh = (self.grid.height as f32 - 1.0) * 0.5;
        let cx = (x + hw).round() as i64;
        let cy = (z + hh).round() as i64;
        (cx.clamp(0, w - 1) as usize, cy.clamp(0, h - 1) as usize)
    }

    /// Returns the world-space centre of grid cell `(cx, cy)`.
    fn cell_to_world(&self, cx: usize, cy: usize) -> (f32, f32) {
        let hw = (self.grid.width as f32 - 1.0) * 0.5;
        let hh = (self.grid.height as f32 - 1.0) * 0.5;
        (cx as f32 - hw, cy as f32 - hh)
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

    /// Returns the broad zone type at the given world-space position.
    ///
    /// Test-only compatibility helper kept for older migration coverage. Normal runtime zoning
    /// work should read profile runtime ids instead.
    #[cfg(test)]
    pub fn get_zone_world(&self, x: f32, z: f32) -> ZoneType {
        match self.world_to_cell(x, z) {
            Some((cx, cy)) => self
                .profiles
                .zone_type_for_runtime_id(*self.grid.get(cx, cy).unwrap_or(&0)),
            None => ZoneType::None,
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

    /// Paints a world-space rectangle with one broad zone family.
    ///
    /// Test-only compatibility helper kept for migration coverage. Broad-family paints map to the
    /// default low-density runtime profile for that family.
    #[cfg(test)]
    pub fn set_zone_rect(
        &mut self,
        x_min: f32,
        z_min: f32,
        x_max: f32,
        z_max: f32,
        zone: ZoneType,
    ) {
        let runtime_id = self
            .profiles
            .default_runtime_id_for_zone_type(zone)
            .unwrap_or(0);
        self.set_zone_profile_rect(x_min, z_min, x_max, z_max, runtime_id);
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

    /// Returns the derived broad zone-family grid as a flat `u8` byte array for legacy callers.
    ///
    /// Test-only compatibility helper kept while migration coverage still reads the old texture
    /// shape. Normal runtime uploads use [`Self::get_zone_profile_texture_data_rg8`].
    #[cfg(test)]
    pub fn get_zone_texture_data(&self) -> Vec<u8> {
        self.grid
            .data
            .iter()
            .map(|&runtime_id| self.profiles.zone_type_for_runtime_id(runtime_id) as u8)
            .collect()
    }

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
        let hw_grid = (w as f32 - 1.0) * 0.5;
        let hh_grid = (h as f32 - 1.0) * 0.5;

        let distances: Vec<u8> = (0..w * h)
            .into_par_iter()
            .map(|i| {
                let cx = i % w;
                let cy = i / w;
                let px = (cx as f32 - hw_grid) * cell;
                let pz = (cy as f32 - hh_grid) * cell;

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
        let hw_grid = (w as f32 - 1.0) * 0.5;
        let hh_grid = (h as f32 - 1.0) * 0.5;

        let mask: Vec<bool> = (0..w * h)
            .into_par_iter()
            .map(|i| {
                if segments.is_empty() {
                    return false;
                }
                let cx = i % w;
                let cy = i / w;
                let px = (cx as f32 - hw_grid) * cell;
                let pz = (cy as f32 - hh_grid) * cell;
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
