//! Building-site footprint, surface, and terrain-query helpers.

use crate::assets::{
    Anchor, AnchorType, AssetManifest, MeshPart, SiteSurface, SiteSurfaceMaterial,
};
use crate::config::SIDEWALK_WIDTH;
use crate::simulation::buildings::allocator::entrance::{
    building_local_xz_basis, building_local_xz_pos,
};
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::surface::RoadSurfaceSystem;
use crate::simulation::network::types::{TransitFlags, TransitType};
use crate::simulation::terrain::cdt::{
    MAX_TERRAIN_TIE_IN_SLOPE_RATIO, TerrainCdtRoadBoundarySource, TerrainCdtRoadLoop,
    TerrainCdtRoadLoopSourceEdge, TerrainCdtTieInGuideConstraint, TerrainCdtTieInGuideSample,
    TerrainCdtVertex,
};
use crate::simulation::terrain::{TerrainSystem, terrain_cdt_local_sample_margin_m};
use godot::prelude::{Vector2, Vector3};
use std::collections::BTreeMap;

const DEFAULT_ANCHOR_FORWARD: [f32; 3] = [0.0, 0.0, 1.0];
const BUILDING_SITE_FOOTPRINT_GROUP_MASK: u64 = 0x8000_0000_0000_0000;
const BUILDING_SITE_GRADING_SAMPLE_KEY_SCALE: f64 = 1000.0;
const BUILDING_SITE_GRADING_RING_MULTIPLIERS: [f32; 5] = [0.5, 1.0, 2.0, 4.0, 8.0];
const BUILDING_SITE_SUPPORT_TIE_IN_SAMPLE_STEP_M: f32 = 2.0;
const BUILDING_SITE_SUPPORT_TIE_IN_EPS_M: f32 = 0.05;
const BUILDING_SITE_NEAREST_ROAD_SURFACE_MIN_RADIUS_M: f32 = 3.0;
const BUILDING_SITE_NEAREST_ROAD_SURFACE_MAX_RADIUS_M: f32 = 8.0;
const BUILDING_SITE_ROAD_SURFACE_PROBE_INSET_M: f32 = 0.05;
const BUILDING_SITE_MESH_PART_SUPPORT_MARGIN_M: f32 = 5.25;
const BUILDING_SITE_SURFACE_SUPPORT_MARGIN_M: f32 = 0.35;
const BUILDING_SITE_ACCESS_SUPPORT_MARGIN_M: f32 = 0.25;
const BUILDING_SITE_ROAD_ACCESS_CLEARANCE_M: f32 = 1.0;
const BUILDING_SITE_ENTRANCE_SUPPORT_WIDTH_M: f32 = 2.0;
const BUILDING_SITE_ENTRANCE_SUPPORT_LENGTH_M: f32 = 2.0;
const BUILDING_SITE_DEFAULT_SUPPORT_INSET_M: f32 = 2.0;
const SITE_POINT_EPS_M: f32 = 0.001;

/// Runtime surface polygon authored inside a building site.
#[derive(Clone, Debug)]
pub(crate) struct BuildingSiteSurfaceClient {
    /// Material class for the surface.
    pub(crate) material: SiteSurfaceMaterial,
    /// Editor-authored label used by diagnostics.
    pub(crate) name: String,
    /// World-space height of the surface top.
    pub(crate) height_m: f32,
    /// World-space polygon vertices.
    pub(crate) vertices_world: Vec<Vector2>,
}

/// Runtime client that owns the required flat building-site support surface.
#[derive(Clone, Debug)]
pub(crate) struct BuildingSiteClient {
    /// World-space flat support footprint corners.
    pub(crate) footprint_world: Vec<Vector2>,
    /// World-space lot reservation corners.
    pub(crate) lot_footprint_world: [Vector2; 4],
    /// Flat support height shared by the building and authored site surfaces.
    pub(crate) support_height_m: f32,
    /// Authored site surface polygons transformed into world space.
    pub(crate) surfaces: Vec<BuildingSiteSurfaceClient>,
}

impl BuildingSiteClient {
    pub(crate) fn bounds(&self) -> (f32, f32, f32, f32) {
        polygon_slice_bounds(&self.footprint_world)
    }

    pub(crate) fn lot_bounds(&self) -> (f32, f32, f32, f32) {
        polygon_quad_bounds(self.lot_footprint_world)
    }

    fn overlaps_bounds(&self, min_x: f32, min_z: f32, max_x: f32, max_z: f32) -> bool {
        let (site_min_x, site_min_z, site_max_x, site_max_z) = self.bounds();
        site_min_x <= max_x && site_max_x >= min_x && site_min_z <= max_z && site_max_z >= min_z
    }

    pub(crate) fn contains_point(&self, pos: Vector2) -> bool {
        point_in_polygon_slice(pos, &self.footprint_world)
    }

    fn height_at(&self, pos: Vector2) -> Option<f32> {
        for surface in &self.surfaces {
            if point_in_polygon_slice(pos, &surface.vertices_world) {
                return Some(surface.height_m);
            }
        }
        self.contains_point(pos).then_some(self.support_height_m)
    }

    pub(crate) fn raycast(&self, ray_origin: Vector3, ray_dir: Vector3) -> Option<Vector3> {
        if ray_dir.length_squared() <= f32::EPSILON {
            return None;
        }

        let mut best: Option<(f32, Vector3)> = None;
        for surface in &self.surfaces {
            update_site_plane_ray_hit(
                &mut best,
                ray_origin,
                ray_dir,
                surface.height_m,
                &surface.vertices_world,
            );
        }
        update_site_plane_ray_hit(
            &mut best,
            ray_origin,
            ray_dir,
            self.support_height_m,
            &self.footprint_world,
        );
        best.map(|(_, hit)| hit)
    }

    pub(crate) fn surface_debug_summary(&self) -> String {
        if self.surfaces.is_empty() {
            return "none".to_owned();
        }
        self.surfaces
            .iter()
            .map(|surface| {
                let material = match surface.material {
                    SiteSurfaceMaterial::Asphalt => "asphalt",
                    SiteSurfaceMaterial::Concrete => "concrete",
                };
                if surface.name.is_empty() {
                    material.to_owned()
                } else {
                    format!("{}:{}", material, surface.name)
                }
            })
            .collect::<Vec<_>>()
            .join(",")
    }
}

impl BuildingAllocator {
    /// Rebuilds all derived building-site clients from the current building list.
    pub(crate) fn rebuild_building_site_clients(&mut self, zone_cell_m: f32) {
        self.building_sites = self
            .buildings
            .iter()
            .map(|building| self.derive_building_site_client(building, zone_cell_m))
            .collect();
        self.recompute_max_site_radius_m();
    }

    pub(crate) fn rebuild_building_site_client(&mut self, building_idx: usize, zone_cell_m: f32) {
        if building_idx >= self.buildings.len() {
            return;
        }
        let client = self.derive_building_site_client(&self.buildings[building_idx], zone_cell_m);
        if self.building_sites.len() != self.buildings.len() {
            self.rebuild_building_site_clients(zone_cell_m);
        } else if let Some(slot) = self.building_sites.get_mut(building_idx) {
            *slot = client;
            self.recompute_max_site_radius_m();
        }
    }

    pub(crate) fn push_building_site_client(&mut self, building_idx: usize, zone_cell_m: f32) {
        let client = self.derive_building_site_client(&self.buildings[building_idx], zone_cell_m);
        if self.building_sites.len() == building_idx {
            self.max_site_radius_m = self.max_site_radius_m.max(site_radius_m(&client));
            self.building_sites.push(client);
        } else {
            self.rebuild_building_site_clients(zone_cell_m);
        }
    }

    pub(crate) fn recompute_max_site_radius_m(&mut self) {
        self.max_site_radius_m = self
            .building_sites
            .iter()
            .map(site_radius_m)
            .fold(0.0, f32::max);
    }

    pub(crate) fn site_world_bounds(&self, building_idx: usize) -> Option<(f32, f32, f32, f32)> {
        self.building_sites
            .get(building_idx)
            .map(|site| site.lot_bounds())
    }

    pub(crate) fn accumulate_pending_site_dirty_bounds(
        &mut self,
        bounds: Option<(f32, f32, f32, f32)>,
    ) {
        let Some(bounds) = bounds else {
            return;
        };
        if let Some(existing) = &mut self.building_site_dirty_bounds {
            existing.0 = existing.0.min(bounds.0);
            existing.1 = existing.1.min(bounds.1);
            existing.2 = existing.2.max(bounds.2);
            existing.3 = existing.3.max(bounds.3);
        } else {
            self.building_site_dirty_bounds = Some(bounds);
        }
    }

    pub(crate) fn take_pending_site_dirty_bounds(&mut self) -> Option<(f32, f32, f32, f32)> {
        self.building_site_dirty_bounds.take()
    }

    pub(crate) fn sample_building_site_height(&self, pos: Vector2) -> Option<f32> {
        if self.dirty_index
            || self.building_sites.len() != self.buildings.len()
            || self.building_chunks.is_empty()
        {
            return self
                .building_sites
                .iter()
                .find_map(|site| site.height_at(pos));
        }

        let margin_m = self.max_site_radius_m.max(0.0);
        let chunk_size = RegionGraph::CHUNK_SIZE;
        let min_chunk_x = ((pos.x - margin_m) / chunk_size).floor() as i32;
        let max_chunk_x = ((pos.x + margin_m) / chunk_size).floor() as i32;
        let min_chunk_z = ((pos.y - margin_m) / chunk_size).floor() as i32;
        let max_chunk_z = ((pos.y + margin_m) / chunk_size).floor() as i32;

        let mut best_idx = usize::MAX;
        let mut best_height = None;
        for chunk_x in min_chunk_x..=max_chunk_x {
            for chunk_z in min_chunk_z..=max_chunk_z {
                let Some(indices) = self.building_chunks.get(&(chunk_x, chunk_z)) else {
                    continue;
                };
                for &idx in indices {
                    if idx >= best_idx || idx >= self.building_sites.len() {
                        continue;
                    }
                    let Some(height) = self.building_sites[idx].height_at(pos) else {
                        continue;
                    };
                    best_idx = idx;
                    best_height = Some(height);
                }
            }
        }

        best_height
    }

    pub(crate) fn raycast_building_site_surface(
        &self,
        ray_origin: Vector3,
        ray_dir: Vector3,
    ) -> Option<Vector3> {
        let mut best: Option<(f32, Vector3)> = None;
        for site in &self.building_sites {
            let Some(hit) = site.raycast(ray_origin, ray_dir) else {
                continue;
            };
            let denom = ray_dir.length_squared().max(f32::EPSILON);
            let t = (hit - ray_origin).dot(ray_dir) / denom;
            if t < 0.0 {
                continue;
            }
            if best.as_ref().is_none_or(|(best_t, _)| t < *best_t) {
                best = Some((t, hit));
            }
        }
        best.map(|(_, hit)| hit)
    }

    pub(crate) fn terrain_render_patch_keys_with_building_site_margin(
        &self,
        terrain: &TerrainSystem,
        margin_m: f32,
    ) -> Vec<(usize, usize)> {
        let margin_m = margin_m.max(0.0);
        let mut keys = Vec::new();
        for site in &self.building_sites {
            let (min_x, min_z, max_x, max_z) = site.bounds();
            keys.extend(terrain.render_patch_keys_for_world_bounds(
                min_x - margin_m,
                min_z - margin_m,
                max_x + margin_m,
                max_z + margin_m,
            ));
        }
        keys.sort_unstable();
        keys.dedup();
        keys
    }

    pub(crate) fn mark_building_site_terrain_bounds_dirty(
        &self,
        terrain: &mut TerrainSystem,
        bounds: (f32, f32, f32, f32),
        margin_m: f32,
    ) {
        let (min_x, min_z, max_x, max_z) = bounds;
        let margin_m = margin_m.max(0.0);
        terrain.mark_render_patches_for_world_bounds(
            min_x - margin_m,
            min_z - margin_m,
            max_x + margin_m,
            max_z + margin_m,
        );
    }

    pub(crate) fn terrain_cdt_site_loops_for_world_bounds(
        &self,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> Vec<TerrainCdtRoadLoop> {
        let mut loops = Vec::new();
        for building_idx in self.site_candidate_indices_for_bounds(min_x, min_z, max_x, max_z) {
            let Some(site) = self.building_sites.get(building_idx) else {
                continue;
            };
            if !site.overlaps_bounds(min_x, min_z, max_x, max_z) {
                continue;
            }
            loops.push(building_site_cdt_loop(building_idx, site));
        }
        loops
    }

    pub(crate) fn append_terrain_cdt_site_grading_guides_for_world_bounds(
        &self,
        terrain: &TerrainSystem,
        graph: &RegionGraph,
        road_surface: &RoadSurfaceSystem,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
        render_step_m: f32,
        tie_in_guide_samples: &mut Vec<TerrainCdtTieInGuideSample>,
        tie_in_guide_constraints: &mut Vec<TerrainCdtTieInGuideConstraint>,
        sample_keys: &mut BTreeMap<(i64, i64), ()>,
    ) {
        let safe_step_m = render_step_m.max(f32::EPSILON);
        let max_distance_m = terrain_cdt_local_sample_margin_m(terrain, safe_step_m);
        for building_idx in self.site_candidate_indices_for_bounds(min_x, min_z, max_x, max_z) {
            let Some(site) = self.building_sites.get(building_idx) else {
                continue;
            };
            if !site.overlaps_bounds(min_x, min_z, max_x, max_z) {
                continue;
            }
            append_building_site_grading_guides(
                site,
                terrain,
                graph,
                road_surface,
                safe_step_m,
                max_distance_m,
                tie_in_guide_samples,
                tie_in_guide_constraints,
                sample_keys,
            );
        }
    }

    /// Returns building-site indices whose chunk coverage may overlap the given world bounds.
    pub(crate) fn site_candidate_indices_for_bounds(
        &self,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> Vec<usize> {
        if self.dirty_index
            || self.building_sites.len() != self.buildings.len()
            || self.building_chunks.is_empty()
        {
            return (0..self.building_sites.len()).collect();
        }

        let margin_m = self.max_site_radius_m.max(0.0);
        let chunk_size = RegionGraph::CHUNK_SIZE;
        let min_chunk_x = ((min_x - margin_m) / chunk_size).floor() as i32;
        let max_chunk_x = ((max_x + margin_m) / chunk_size).floor() as i32;
        let min_chunk_z = ((min_z - margin_m) / chunk_size).floor() as i32;
        let max_chunk_z = ((max_z + margin_m) / chunk_size).floor() as i32;

        let mut candidates = Vec::new();
        for chunk_x in min_chunk_x..=max_chunk_x {
            for chunk_z in min_chunk_z..=max_chunk_z {
                let Some(indices) = self.building_chunks.get(&(chunk_x, chunk_z)) else {
                    continue;
                };
                candidates.extend(indices.iter().copied());
            }
        }
        candidates.sort_unstable();
        candidates.dedup();
        candidates.retain(|&idx| idx < self.building_sites.len());
        candidates
    }

    fn derive_building_site_client(
        &self,
        building: &Building,
        zone_cell_m: f32,
    ) -> BuildingSiteClient {
        let frontage_forward = self
            .registry
            .get(&building.asset_id)
            .map(|entry| entry.manifest.building_frontage_forward())
            .unwrap_or(DEFAULT_ANCHOR_FORWARD);
        let (basis_x, basis_z) = building_local_xz_basis(building.facing_dir, frontage_forward);
        let center = Vector2::new(building.center_x, building.center_y);
        let lot_half_width = building.width_cells as f32 * zone_cell_m * 0.5;
        let lot_half_depth = building.depth_cells as f32 * zone_cell_m * 0.5;
        let lot_footprint_world = [
            center + basis_x * -lot_half_width + basis_z * -lot_half_depth,
            center + basis_x * -lot_half_width + basis_z * lot_half_depth,
            center + basis_x * lot_half_width + basis_z * lot_half_depth,
            center + basis_x * lot_half_width + basis_z * -lot_half_depth,
        ];
        let footprint_world = self.required_flat_support_footprint_world(
            &building.asset_id,
            center,
            building.facing_dir,
            building.width_cells as usize,
            building.depth_cells as usize,
            zone_cell_m,
        );
        let surfaces = self
            .registry
            .get(&building.asset_id)
            .map(|entry| {
                entry
                    .manifest
                    .site_surfaces
                    .iter()
                    .map(|surface| BuildingSiteSurfaceClient {
                        material: surface.material,
                        name: surface.name.clone(),
                        height_m: building.support_height_m + surface.y_m,
                        vertices_world: surface
                            .vertices
                            .iter()
                            .map(|vertex| {
                                building_local_xz_pos(
                                    building,
                                    [vertex[0], 0.0, vertex[1]],
                                    frontage_forward,
                                )
                            })
                            .collect(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        BuildingSiteClient {
            footprint_world,
            lot_footprint_world,
            support_height_m: building.support_height_m,
            surfaces,
        }
    }

    pub(crate) fn required_flat_support_footprint_world(
        &self,
        asset_id: &str,
        center: Vector2,
        facing_dir: Vector2,
        width_cells: usize,
        depth_cells: usize,
        zone_cell_m: f32,
    ) -> Vec<Vector2> {
        let lot_half_width = width_cells as f32 * zone_cell_m * 0.5;
        let lot_half_depth = depth_cells as f32 * zone_cell_m * 0.5;
        let frontage_forward = self
            .registry
            .get(asset_id)
            .map(|entry| entry.manifest.building_frontage_forward())
            .unwrap_or(DEFAULT_ANCHOR_FORWARD);
        let (basis_x, basis_z) = building_local_xz_basis(facing_dir, frontage_forward);
        let footprint_local = self
            .registry
            .get(asset_id)
            .map(|entry| {
                required_flat_support_footprint_local(
                    &entry.manifest,
                    lot_half_width,
                    lot_half_depth,
                )
            })
            .unwrap_or_else(|| lot_footprint_local(lot_half_width, lot_half_depth));
        footprint_local
            .into_iter()
            .map(|point| center + basis_x * point.x + basis_z * point.y)
            .collect()
    }
}

fn required_flat_support_footprint_local(
    manifest: &AssetManifest,
    lot_half_width: f32,
    lot_half_depth: f32,
) -> Vec<Vector2> {
    let frontage_dir = asset_frontage_dir_local(manifest);
    let mut points = Vec::new();
    for part in &manifest.mesh_parts {
        append_mesh_part_support_points(part, lot_half_width, lot_half_depth, &mut points);
    }
    for anchor in &manifest.anchors {
        append_anchor_support_points(
            anchor,
            frontage_dir,
            lot_half_width,
            lot_half_depth,
            &mut points,
        );
    }
    for surface in &manifest.site_surfaces {
        append_site_surface_support_points(surface, lot_half_width, lot_half_depth, &mut points);
    }
    if points.is_empty() {
        return default_support_footprint_local(lot_half_width, lot_half_depth);
    }

    points.sort_by(|left, right| {
        left.x
            .total_cmp(&right.x)
            .then_with(|| left.y.total_cmp(&right.y))
    });
    points.dedup_by(|left, right| left.distance_squared_to(*right) <= SITE_POINT_EPS_M);
    let mut hull = convex_hull_local(points);
    if hull.len() < 3 || signed_polygon_area(&hull).abs() <= SITE_POINT_EPS_M * SITE_POINT_EPS_M {
        hull = default_support_footprint_local(lot_half_width, lot_half_depth);
    }
    if signed_polygon_area(&hull) < 0.0 {
        hull.reverse();
    }
    hull
}

fn asset_frontage_dir_local(manifest: &AssetManifest) -> Vector2 {
    let front = manifest.building_frontage_forward();
    let front = Vector2::new(front[0], front[2]);
    if front.length_squared() > f32::EPSILON {
        front.normalized()
    } else {
        Vector2::new(DEFAULT_ANCHOR_FORWARD[0], DEFAULT_ANCHOR_FORWARD[2])
    }
}

fn append_mesh_part_support_points(
    part: &MeshPart,
    lot_half_width: f32,
    lot_half_depth: f32,
    points: &mut Vec<Vector2>,
) {
    let half_extent = (part.scale * 0.75).max(BUILDING_SITE_MESH_PART_SUPPORT_MARGIN_M);
    append_axis_aligned_support_rect(
        Vector2::new(part.position[0], part.position[2]),
        half_extent,
        half_extent,
        lot_half_width,
        lot_half_depth,
        points,
    );
}

fn append_anchor_support_points(
    anchor: &Anchor,
    frontage_dir: Vector2,
    lot_half_width: f32,
    lot_half_depth: f32,
    points: &mut Vec<Vector2>,
) {
    match anchor.anchor_type {
        AnchorType::Entrance => append_oriented_support_rect(
            anchor.position,
            anchor.forward,
            anchor
                .width_m
                .unwrap_or(BUILDING_SITE_ENTRANCE_SUPPORT_WIDTH_M),
            BUILDING_SITE_ENTRANCE_SUPPORT_LENGTH_M,
            BUILDING_SITE_ACCESS_SUPPORT_MARGIN_M,
            lot_half_width,
            lot_half_depth,
            frontage_dir,
            points,
        ),
        AnchorType::Driveway => {
            let width = anchor.width_m.unwrap_or(0.0);
            append_oriented_support_rect(
                anchor.position,
                anchor.forward,
                width,
                (width * 1.4).max(1.5),
                BUILDING_SITE_ACCESS_SUPPORT_MARGIN_M,
                lot_half_width,
                lot_half_depth,
                frontage_dir,
                points,
            );
        }
        AnchorType::Parking | AnchorType::LoadingBay => append_oriented_support_rect(
            anchor.position,
            anchor.forward,
            anchor.width_m.unwrap_or(0.0),
            anchor.length_m.unwrap_or(0.0),
            BUILDING_SITE_ACCESS_SUPPORT_MARGIN_M,
            lot_half_width,
            lot_half_depth,
            frontage_dir,
            points,
        ),
        AnchorType::Wheel | AnchorType::Light => {}
    }
}

fn append_site_surface_support_points(
    surface: &SiteSurface,
    lot_half_width: f32,
    lot_half_depth: f32,
    points: &mut Vec<Vector2>,
) {
    if surface.vertices.is_empty() {
        return;
    }
    let mut min_x = f32::INFINITY;
    let mut min_z = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_z = f32::NEG_INFINITY;
    for vertex in &surface.vertices {
        min_x = min_x.min(vertex[0]);
        min_z = min_z.min(vertex[1]);
        max_x = max_x.max(vertex[0]);
        max_z = max_z.max(vertex[1]);
    }
    min_x -= BUILDING_SITE_SURFACE_SUPPORT_MARGIN_M;
    min_z -= BUILDING_SITE_SURFACE_SUPPORT_MARGIN_M;
    max_x += BUILDING_SITE_SURFACE_SUPPORT_MARGIN_M;
    max_z += BUILDING_SITE_SURFACE_SUPPORT_MARGIN_M;
    for point in [
        Vector2::new(min_x, min_z),
        Vector2::new(min_x, max_z),
        Vector2::new(max_x, max_z),
        Vector2::new(max_x, min_z),
    ] {
        points.push(clamp_local_support_point(
            point,
            lot_half_width,
            lot_half_depth,
        ));
    }
}

fn append_axis_aligned_support_rect(
    center: Vector2,
    half_width: f32,
    half_depth: f32,
    lot_half_width: f32,
    lot_half_depth: f32,
    points: &mut Vec<Vector2>,
) {
    for point in [
        Vector2::new(center.x - half_width, center.y - half_depth),
        Vector2::new(center.x - half_width, center.y + half_depth),
        Vector2::new(center.x + half_width, center.y + half_depth),
        Vector2::new(center.x + half_width, center.y - half_depth),
    ] {
        points.push(clamp_local_support_point(
            point,
            lot_half_width,
            lot_half_depth,
        ));
    }
}

fn append_oriented_support_rect(
    position: [f32; 3],
    forward: [f32; 3],
    width_m: f32,
    length_m: f32,
    margin_m: f32,
    lot_half_width: f32,
    lot_half_depth: f32,
    frontage_dir: Vector2,
    points: &mut Vec<Vector2>,
) {
    let forward = Vector2::new(forward[0], forward[2]);
    if forward.length_squared() <= f32::EPSILON {
        return;
    }
    let forward = forward.normalized();
    let side = Vector2::new(-forward.y, forward.x);
    let origin = Vector2::new(position[0], position[2]);
    let half_width = width_m.max(0.0) * 0.5 + margin_m.max(0.0);
    let back = margin_m.max(0.0);
    let front = length_m.max(0.0) + margin_m.max(0.0);
    for point in [
        origin - side * half_width - forward * back,
        origin + side * half_width - forward * back,
        origin + side * half_width + forward * front,
        origin - side * half_width + forward * front,
    ] {
        points.push(clamp_road_access_support_point(
            point,
            frontage_dir,
            lot_half_width,
            lot_half_depth,
        ));
    }
}

fn clamp_road_access_support_point(
    point: Vector2,
    frontage_dir: Vector2,
    lot_half_width: f32,
    lot_half_depth: f32,
) -> Vector2 {
    let point = clamp_local_support_point(point, lot_half_width, lot_half_depth);
    if frontage_dir.length_squared() <= f32::EPSILON {
        return point;
    }
    let frontage_dir = frontage_dir.normalized();
    let front_limit = frontage_projection_limit(frontage_dir, lot_half_width, lot_half_depth);
    let clearance = BUILDING_SITE_ROAD_ACCESS_CLEARANCE_M.min(front_limit.max(0.0) * 0.5);
    let support_limit = front_limit - clearance;
    let projection = frontage_projection(point, frontage_dir);
    if projection <= support_limit {
        return point;
    }
    clamp_local_support_point(
        point - frontage_dir * (projection - support_limit),
        lot_half_width,
        lot_half_depth,
    )
}

fn frontage_projection(point: Vector2, frontage_dir: Vector2) -> f32 {
    point.x * frontage_dir.x + point.y * frontage_dir.y
}

fn frontage_projection_limit(
    frontage_dir: Vector2,
    lot_half_width: f32,
    lot_half_depth: f32,
) -> f32 {
    frontage_dir.x.abs() * lot_half_width + frontage_dir.y.abs() * lot_half_depth
}

fn default_support_footprint_local(lot_half_width: f32, lot_half_depth: f32) -> Vec<Vector2> {
    let inset = BUILDING_SITE_DEFAULT_SUPPORT_INSET_M
        .min(lot_half_width.max(0.0) * 0.5)
        .min(lot_half_depth.max(0.0) * 0.5);
    lot_footprint_local(lot_half_width - inset, lot_half_depth - inset)
}

fn lot_footprint_local(lot_half_width: f32, lot_half_depth: f32) -> Vec<Vector2> {
    vec![
        Vector2::new(-lot_half_width, -lot_half_depth),
        Vector2::new(-lot_half_width, lot_half_depth),
        Vector2::new(lot_half_width, lot_half_depth),
        Vector2::new(lot_half_width, -lot_half_depth),
    ]
}

fn clamp_local_support_point(point: Vector2, lot_half_width: f32, lot_half_depth: f32) -> Vector2 {
    Vector2::new(
        point.x.clamp(-lot_half_width, lot_half_width),
        point.y.clamp(-lot_half_depth, lot_half_depth),
    )
}

fn convex_hull_local(points: Vec<Vector2>) -> Vec<Vector2> {
    if points.len() <= 3 {
        return points;
    }
    let mut lower = Vec::new();
    for point in points.iter().copied() {
        while lower.len() >= 2
            && local_cross(lower[lower.len() - 2], lower[lower.len() - 1], point)
                <= SITE_POINT_EPS_M
        {
            lower.pop();
        }
        lower.push(point);
    }
    let mut upper = Vec::new();
    for point in points.iter().rev().copied() {
        while upper.len() >= 2
            && local_cross(upper[upper.len() - 2], upper[upper.len() - 1], point)
                <= SITE_POINT_EPS_M
        {
            upper.pop();
        }
        upper.push(point);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

fn local_cross(a: Vector2, b: Vector2, c: Vector2) -> f32 {
    let ab = b - a;
    let ac = c - a;
    ab.x * ac.y - ab.y * ac.x
}

fn building_site_cdt_loop(building_idx: usize, site: &BuildingSiteClient) -> TerrainCdtRoadLoop {
    let stable_piece_id = BUILDING_SITE_FOOTPRINT_GROUP_MASK | building_idx as u64;
    let vertices = site
        .footprint_world
        .iter()
        .map(|point| TerrainCdtVertex::new(point.x as f64, site.support_height_m, point.y as f64))
        .collect::<Vec<_>>();
    let source_edges = vertices
        .iter()
        .copied()
        .enumerate()
        .map(|(edge_idx, start)| TerrainCdtRoadLoopSourceEdge {
            start,
            end: vertices[(edge_idx + 1) % vertices.len()],
            source: TerrainCdtRoadBoundarySource::BuildingSiteBoundary {
                building_idx: building_idx as u64,
                local_loop_index: 0,
                local_edge_index: u32::try_from(edge_idx).unwrap_or(u32::MAX),
            },
        })
        .collect();
    TerrainCdtRoadLoop::new_with_source_edges_and_topology(
        stable_piece_id,
        stable_piece_id,
        0,
        false,
        vertices,
        source_edges,
    )
}

pub(super) fn building_site_support_tie_in_is_valid(
    footprint_world: &[Vector2],
    support_height_m: f32,
    terrain: &TerrainSystem,
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
) -> bool {
    if !support_height_m.is_finite() {
        return false;
    }
    let signed_area = signed_polygon_area(footprint_world);
    if signed_area.abs() <= f32::EPSILON {
        return false;
    }

    let safe_step_m = BUILDING_SITE_SUPPORT_TIE_IN_SAMPLE_STEP_M.max(f32::EPSILON);
    let max_distance_m = terrain_cdt_local_sample_margin_m(terrain, safe_step_m);
    let loop_is_ccw = signed_area > 0.0;
    let mut edge_outward_dirs = Vec::with_capacity(footprint_world.len());

    for edge_idx in 0..footprint_world.len() {
        let start = footprint_world[edge_idx];
        let end = footprint_world[(edge_idx + 1) % footprint_world.len()];
        let delta = end - start;
        let length_m = delta.length();
        if length_m <= f32::EPSILON {
            return false;
        }
        let mut outward = if loop_is_ccw {
            Vector2::new(delta.y, -delta.x)
        } else {
            Vector2::new(-delta.y, delta.x)
        } / length_m;
        outward = corrected_footprint_outward(footprint_world, (start + end) * 0.5, outward);
        edge_outward_dirs.push(outward);

        let sample_count = ((length_m / safe_step_m).ceil() as u32).max(1);
        for sample_idx in 0..=sample_count {
            let t = sample_idx as f32 / sample_count as f32;
            let seam = start.lerp(end, t);
            if !building_site_support_tie_in_ray_is_valid(
                support_height_m,
                seam,
                outward,
                terrain,
                graph,
                road_surface,
                safe_step_m,
                max_distance_m,
            ) {
                return false;
            }
        }
    }

    if edge_outward_dirs.len() != footprint_world.len() {
        return false;
    }
    for vertex_idx in 0..footprint_world.len() {
        let previous =
            edge_outward_dirs[(vertex_idx + footprint_world.len() - 1) % footprint_world.len()];
        let next = edge_outward_dirs[vertex_idx];
        let bisector = previous + next;
        if bisector.length_squared() <= f32::EPSILON {
            continue;
        }
        let vertex = footprint_world[vertex_idx];
        let outward = corrected_footprint_outward(footprint_world, vertex, bisector.normalized());
        if !building_site_support_tie_in_ray_is_valid(
            support_height_m,
            vertex,
            outward,
            terrain,
            graph,
            road_surface,
            safe_step_m,
            max_distance_m,
        ) {
            return false;
        }
    }

    true
}

fn append_building_site_grading_guides(
    site: &BuildingSiteClient,
    terrain: &TerrainSystem,
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
    safe_step_m: f32,
    max_distance_m: f32,
    tie_in_guide_samples: &mut Vec<TerrainCdtTieInGuideSample>,
    tie_in_guide_constraints: &mut Vec<TerrainCdtTieInGuideConstraint>,
    sample_keys: &mut BTreeMap<(i64, i64), ()>,
) {
    let signed_area = signed_polygon_area(&site.footprint_world);
    if signed_area.abs() <= f32::EPSILON {
        return;
    }
    let loop_is_ccw = signed_area > 0.0;
    let mut edge_outward_dirs = Vec::with_capacity(site.footprint_world.len());

    for edge_idx in 0..site.footprint_world.len() {
        let start = site.footprint_world[edge_idx];
        let end = site.footprint_world[(edge_idx + 1) % site.footprint_world.len()];
        let delta = end - start;
        let length_m = delta.length();
        if length_m <= f32::EPSILON {
            continue;
        }
        let mut outward = if loop_is_ccw {
            Vector2::new(delta.y, -delta.x)
        } else {
            Vector2::new(-delta.y, delta.x)
        } / length_m;
        outward = corrected_site_outward(site, (start + end) * 0.5, outward);
        edge_outward_dirs.push(outward);

        let sample_count = ((length_m / safe_step_m).ceil() as u32).max(1);
        let mut previous_ring_vertices = Vec::new();
        for sample_idx in 0..=sample_count {
            let t = sample_idx as f32 / sample_count as f32;
            let seam = start.lerp(end, t);
            let ring_vertices = building_site_grading_ray_vertices(
                site.support_height_m,
                seam,
                outward,
                terrain,
                graph,
                road_surface,
                safe_step_m,
                max_distance_m,
            );
            for vertex in &ring_vertices {
                push_building_site_grading_sample(*vertex, tie_in_guide_samples, sample_keys);
            }
            for (previous, current) in previous_ring_vertices.iter().zip(ring_vertices.iter()) {
                push_building_site_grading_constraint(
                    *previous,
                    *current,
                    tie_in_guide_constraints,
                );
            }
            push_building_site_grading_ray_constraints(
                site.support_height_m,
                seam,
                &ring_vertices,
                tie_in_guide_constraints,
            );
            previous_ring_vertices = ring_vertices;
        }
    }

    if edge_outward_dirs.len() != site.footprint_world.len() {
        return;
    }
    for vertex_idx in 0..site.footprint_world.len() {
        let previous = edge_outward_dirs
            [(vertex_idx + site.footprint_world.len() - 1) % site.footprint_world.len()];
        let next = edge_outward_dirs[vertex_idx];
        let bisector = previous + next;
        if bisector.length_squared() <= f32::EPSILON {
            continue;
        }
        let vertex = site.footprint_world[vertex_idx];
        let outward = corrected_site_outward(site, vertex, bisector.normalized());
        append_building_site_grading_ray(
            site.support_height_m,
            vertex,
            outward,
            terrain,
            graph,
            road_surface,
            safe_step_m,
            max_distance_m,
            tie_in_guide_samples,
            tie_in_guide_constraints,
            sample_keys,
        );
    }
}

fn append_building_site_grading_ray(
    seam_height_m: f32,
    seam: Vector2,
    outward: Vector2,
    terrain: &TerrainSystem,
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
    safe_step_m: f32,
    max_distance_m: f32,
    tie_in_guide_samples: &mut Vec<TerrainCdtTieInGuideSample>,
    tie_in_guide_constraints: &mut Vec<TerrainCdtTieInGuideConstraint>,
    sample_keys: &mut BTreeMap<(i64, i64), ()>,
) {
    let vertices = building_site_grading_ray_vertices(
        seam_height_m,
        seam,
        outward,
        terrain,
        graph,
        road_surface,
        safe_step_m,
        max_distance_m,
    );
    for vertex in &vertices {
        push_building_site_grading_sample(*vertex, tie_in_guide_samples, sample_keys);
    }
    push_building_site_grading_ray_constraints(
        seam_height_m,
        seam,
        &vertices,
        tie_in_guide_constraints,
    );
}

fn building_site_grading_ray_vertices(
    seam_height_m: f32,
    seam: Vector2,
    outward: Vector2,
    terrain: &TerrainSystem,
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
    safe_step_m: f32,
    max_distance_m: f32,
) -> Vec<TerrainCdtVertex> {
    let mut vertices = Vec::new();
    let mut previous_distance_m = 0.0_f32;
    for multiplier in BUILDING_SITE_GRADING_RING_MULTIPLIERS {
        let distance_m = (safe_step_m * multiplier).min(max_distance_m);
        if distance_m <= previous_distance_m + f32::EPSILON {
            continue;
        }
        previous_distance_m = distance_m;
        let pos = seam + outward * distance_m;
        let height_m = building_site_grading_target_height(
            seam_height_m,
            pos,
            distance_m,
            terrain,
            graph,
            road_surface,
        );
        vertices.push(TerrainCdtVertex::new(pos.x as f64, height_m, pos.y as f64));
    }
    vertices
}

fn push_building_site_grading_ray_constraints(
    seam_height_m: f32,
    seam: Vector2,
    ring_vertices: &[TerrainCdtVertex],
    tie_in_guide_constraints: &mut Vec<TerrainCdtTieInGuideConstraint>,
) {
    let mut previous = TerrainCdtVertex::new(seam.x as f64, seam_height_m, seam.y as f64);
    for &current in ring_vertices {
        push_building_site_grading_constraint(previous, current, tie_in_guide_constraints);
        previous = current;
    }
}

fn building_site_support_tie_in_ray_is_valid(
    seam_height_m: f32,
    seam: Vector2,
    outward: Vector2,
    terrain: &TerrainSystem,
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
    safe_step_m: f32,
    max_distance_m: f32,
) -> bool {
    let mut previous_distance_m = 0.0_f32;
    for multiplier in BUILDING_SITE_GRADING_RING_MULTIPLIERS {
        let distance_m = (safe_step_m * multiplier).min(max_distance_m);
        if distance_m <= previous_distance_m + f32::EPSILON {
            continue;
        }
        previous_distance_m = distance_m;
        let pos = seam + outward * distance_m;
        let target_height_m =
            building_site_raw_tie_in_target_height(pos, distance_m, terrain, graph, road_surface);
        let max_delta_m = distance_m.max(0.0) * MAX_TERRAIN_TIE_IN_SLOPE_RATIO
            + BUILDING_SITE_SUPPORT_TIE_IN_EPS_M;
        if (target_height_m - seam_height_m).abs() <= max_delta_m {
            return true;
        }
    }
    false
}

fn building_site_grading_target_height(
    seam_height_m: f32,
    pos: Vector2,
    distance_m: f32,
    terrain: &TerrainSystem,
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
) -> f32 {
    let raw_height_m =
        building_site_raw_tie_in_target_height(pos, distance_m, terrain, graph, road_surface);
    grade_limited_site_tie_in_height(seam_height_m, raw_height_m, distance_m)
}

fn building_site_raw_tie_in_target_height(
    pos: Vector2,
    distance_m: f32,
    terrain: &TerrainSystem,
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
) -> f32 {
    let nearest_radius_m = distance_m
        .max(BUILDING_SITE_NEAREST_ROAD_SURFACE_MIN_RADIUS_M)
        .min(BUILDING_SITE_NEAREST_ROAD_SURFACE_MAX_RADIUS_M);
    if let Some(road_height_m) =
        building_site_visible_road_height(terrain, graph, road_surface, pos, nearest_radius_m)
    {
        return road_height_m;
    }
    terrain.sample_visual_height_world(pos.x, pos.y) * crate::config::HEIGHT_SCALE
}

fn building_site_visible_road_height(
    terrain: &TerrainSystem,
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
    pos: Vector2,
    nearest_radius_m: f32,
) -> Option<f32> {
    if let Some(height_m) = road_surface.sample_visible_surface_height(graph, terrain, pos.x, pos.y)
    {
        return Some(height_m);
    }
    nearest_building_site_road_surface_sample(terrain, graph, road_surface, pos, nearest_radius_m)
        .map(|(_, height_m)| height_m)
}

fn nearest_building_site_road_surface_sample(
    terrain: &TerrainSystem,
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
    pos: Vector2,
    nearest_radius_m: f32,
) -> Option<(Vector2, f32)> {
    let radius_m = nearest_radius_m.max(0.0);
    if radius_m <= f32::EPSILON {
        return None;
    }
    let mut candidates = graph.get_edges_near_point(Vector3::new(pos.x, 0.0, pos.y), radius_m);
    candidates.sort_unstable();
    candidates.dedup();

    let mut best: Option<(f32, usize, Vector2, f32)> = None;
    for edge_idx in candidates {
        let Some(edge) = graph.get_edge(edge_idx) else {
            continue;
        };
        if edge.deleted || edge.physical_geometry.len() < 2 || edge.physical_length <= 1e-6 {
            continue;
        }
        let Some(projection) =
            BuildingAllocator::project_point_to_edge_centerline(edge_idx, edge, pos)
        else {
            continue;
        };
        let center = BuildingAllocator::sample_pos_on_edge(graph, edge_idx, projection.t);
        let tangent = BuildingAllocator::sample_tangent_on_edge(graph, edge_idx, projection.t);
        if tangent.length_squared() <= 1e-12 {
            continue;
        }
        let normal = Vector2::new(tangent.y, -tangent.x) * projection.side as f32;
        let probe = center + normal * building_site_road_connection_lateral_offset_m(edge);
        let dist_sq = probe.distance_squared_to(pos);
        if dist_sq > radius_m * radius_m {
            continue;
        }
        let Some(height_m) =
            road_surface.sample_visible_surface_height(graph, terrain, probe.x, probe.y)
        else {
            continue;
        };
        let replace = best
            .as_ref()
            .is_none_or(|(best_dist_sq, best_edge_idx, _, _)| {
                dist_sq
                    .total_cmp(best_dist_sq)
                    .then(edge_idx.cmp(best_edge_idx))
                    .is_lt()
            });
        if replace {
            best = Some((dist_sq, edge_idx, probe, height_m));
        }
    }
    best.map(|(_, _, probe, height_m)| (probe, height_m))
}

fn building_site_road_connection_lateral_offset_m(
    edge: &crate::simulation::network::graph::Edge,
) -> f32 {
    let sidewalk_m = if edge.primary_type == TransitType::Foot
        || (edge.allowed_types & TransitFlags::FOOT) == 0
    {
        0.0
    } else {
        SIDEWALK_WIDTH
    };
    (edge.width * 0.5 + sidewalk_m - BUILDING_SITE_ROAD_SURFACE_PROBE_INSET_M).max(0.0)
}

fn grade_limited_site_tie_in_height(
    seam_height_m: f32,
    terrain_height_m: f32,
    distance_m: f32,
) -> f32 {
    let max_delta_m = distance_m.max(0.0) * MAX_TERRAIN_TIE_IN_SLOPE_RATIO;
    let delta_m = terrain_height_m - seam_height_m;
    if delta_m.abs() <= max_delta_m {
        terrain_height_m
    } else {
        seam_height_m + delta_m.signum() * max_delta_m
    }
}

fn corrected_site_outward(site: &BuildingSiteClient, seam: Vector2, outward: Vector2) -> Vector2 {
    corrected_footprint_outward(&site.footprint_world, seam, outward)
}

fn corrected_footprint_outward(
    footprint_world: &[Vector2],
    seam: Vector2,
    outward: Vector2,
) -> Vector2 {
    if point_in_polygon_slice(seam + outward * SITE_POINT_EPS_M * 8.0, footprint_world) {
        -outward
    } else {
        outward
    }
}

fn push_building_site_grading_sample(
    vertex: TerrainCdtVertex,
    tie_in_guide_samples: &mut Vec<TerrainCdtTieInGuideSample>,
    sample_keys: &mut BTreeMap<(i64, i64), ()>,
) {
    if !vertex.x.is_finite() || !vertex.z.is_finite() || !vertex.height_m.is_finite() {
        return;
    }
    let key = building_site_grading_sample_key(vertex.x, vertex.z);
    if sample_keys.insert(key, ()).is_some() {
        return;
    }
    tie_in_guide_samples.push(TerrainCdtTieInGuideSample { vertex });
}

fn push_building_site_grading_constraint(
    start: TerrainCdtVertex,
    end: TerrainCdtVertex,
    tie_in_guide_constraints: &mut Vec<TerrainCdtTieInGuideConstraint>,
) {
    if !start.x.is_finite()
        || !start.z.is_finite()
        || !start.height_m.is_finite()
        || !end.x.is_finite()
        || !end.z.is_finite()
        || !end.height_m.is_finite()
    {
        return;
    }
    if building_site_grading_sample_key(start.x, start.z)
        == building_site_grading_sample_key(end.x, end.z)
    {
        return;
    }
    tie_in_guide_constraints.push(TerrainCdtTieInGuideConstraint { start, end });
}

fn building_site_grading_sample_key(x: f64, z: f64) -> (i64, i64) {
    (
        (x * BUILDING_SITE_GRADING_SAMPLE_KEY_SCALE).round() as i64,
        (z * BUILDING_SITE_GRADING_SAMPLE_KEY_SCALE).round() as i64,
    )
}

fn signed_polygon_area(points: &[Vector2]) -> f32 {
    let mut area = 0.0;
    for idx in 0..points.len() {
        let start = points[idx];
        let end = points[(idx + 1) % points.len()];
        area += start.x * end.y - end.x * start.y;
    }
    area * 0.5
}

fn polygon_quad_bounds(points: [Vector2; 4]) -> (f32, f32, f32, f32) {
    polygon_slice_bounds(&points)
}

fn polygon_slice_bounds(points: &[Vector2]) -> (f32, f32, f32, f32) {
    let mut min_x = f32::INFINITY;
    let mut min_z = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_z = f32::NEG_INFINITY;
    for &point in points {
        min_x = min_x.min(point.x);
        min_z = min_z.min(point.y);
        max_x = max_x.max(point.x);
        max_z = max_z.max(point.y);
    }
    (min_x, min_z, max_x, max_z)
}

fn site_radius_m(site: &BuildingSiteClient) -> f32 {
    let (min_x, min_z, max_x, max_z) = site.bounds();
    ((max_x - min_x) * 0.5).hypot((max_z - min_z) * 0.5)
}

fn update_site_plane_ray_hit(
    best: &mut Option<(f32, Vector3)>,
    ray_origin: Vector3,
    ray_dir: Vector3,
    height_m: f32,
    polygon: &[Vector2],
) {
    if ray_dir.y.abs() <= f32::EPSILON {
        return;
    }
    let t = (height_m - ray_origin.y) / ray_dir.y;
    if t < 0.0 {
        return;
    }
    let hit = ray_origin + ray_dir * t;
    if !point_in_polygon_slice(Vector2::new(hit.x, hit.z), polygon) {
        return;
    }
    if best.as_ref().is_none_or(|(best_t, _)| t < *best_t) {
        *best = Some((t, hit));
    }
}

fn point_in_polygon_slice(pos: Vector2, polygon: &[Vector2]) -> bool {
    if polygon.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut prev = polygon[polygon.len() - 1];
    for &current in polygon {
        if point_on_segment(pos, prev, current) {
            return true;
        }
        let crosses = (current.y > pos.y) != (prev.y > pos.y);
        if crosses {
            let t = (pos.y - current.y) / (prev.y - current.y);
            let x_at_y = current.x + (prev.x - current.x) * t;
            if pos.x < x_at_y {
                inside = !inside;
            }
        }
        prev = current;
    }
    inside
}

fn point_on_segment(pos: Vector2, a: Vector2, b: Vector2) -> bool {
    let ab = b - a;
    let ap = pos - a;
    let length_sq = ab.length_squared();
    if length_sq <= f32::EPSILON {
        return pos.distance_to(a) <= SITE_POINT_EPS_M;
    }
    let cross = ab.x * ap.y - ab.y * ap.x;
    if cross.abs() > SITE_POINT_EPS_M * length_sq.sqrt().max(SITE_POINT_EPS_M) {
        return false;
    }
    let dot = ap.dot(ab);
    dot >= -SITE_POINT_EPS_M && dot <= length_sq + SITE_POINT_EPS_M
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square_site_with_surface() -> BuildingSiteClient {
        BuildingSiteClient {
            footprint_world: vec![
                Vector2::new(-5.0, -5.0),
                Vector2::new(-5.0, 5.0),
                Vector2::new(5.0, 5.0),
                Vector2::new(5.0, -5.0),
            ],
            lot_footprint_world: [
                Vector2::new(-5.0, -5.0),
                Vector2::new(-5.0, 5.0),
                Vector2::new(5.0, 5.0),
                Vector2::new(5.0, -5.0),
            ],
            support_height_m: 2.0,
            surfaces: vec![BuildingSiteSurfaceClient {
                material: SiteSurfaceMaterial::Asphalt,
                name: "asphalt".to_owned(),
                height_m: 2.4,
                vertices_world: vec![
                    Vector2::new(-1.0, -1.0),
                    Vector2::new(-1.0, 1.0),
                    Vector2::new(1.0, 1.0),
                    Vector2::new(1.0, -1.0),
                ],
            }],
        }
    }

    #[test]
    fn site_height_prefers_authored_surface_offset() {
        let site = square_site_with_surface();

        assert_eq!(site.height_at(Vector2::new(0.0, 0.0)), Some(2.4));
        assert_eq!(site.height_at(Vector2::new(4.0, 4.0)), Some(2.0));
    }

    #[test]
    fn site_height_includes_surface_and_footprint_boundaries() {
        let site = square_site_with_surface();

        assert_eq!(site.height_at(Vector2::new(1.0, 0.0)), Some(2.4));
        assert_eq!(site.height_at(Vector2::new(5.0, 0.0)), Some(2.0));
    }

    #[test]
    fn site_raycast_hits_authored_surface_before_support_plane() {
        let site = square_site_with_surface();

        let hit = site
            .raycast(Vector3::new(0.0, 10.0, 0.0), Vector3::DOWN)
            .expect("ray should hit site surface");

        assert!((hit.y - 2.4).abs() <= f32::EPSILON);
    }

    #[test]
    fn site_grading_guides_start_outside_flat_support() {
        let site = BuildingSiteClient {
            support_height_m: 4.0,
            surfaces: Vec::new(),
            ..square_site_with_surface()
        };
        let terrain = TerrainSystem::with_chunking(8, 8, 1.0, 4, 0.0);
        let graph = RegionGraph::new();
        let road_surface = RoadSurfaceSystem::new(RegionGraph::CHUNK_SIZE);
        let mut samples = Vec::new();
        let mut constraints = Vec::new();
        let mut sample_keys = BTreeMap::new();

        append_building_site_grading_guides(
            &site,
            &terrain,
            &graph,
            &road_surface,
            2.0,
            16.0,
            &mut samples,
            &mut constraints,
            &mut sample_keys,
        );

        assert!(
            samples.iter().any(|sample| {
                (sample.vertex.x + 6.0).abs() <= 0.001
                    && sample.vertex.z.abs() <= 1.001
                    && (sample.vertex.height_m - 3.5).abs() <= 0.001
            }),
            "first apron ring should sit outside the footprint and respect the tie-in slope budget"
        );
        assert!(samples.iter().all(|sample| {
            !site.contains_point(Vector2::new(sample.vertex.x as f32, sample.vertex.z as f32))
        }));
        assert!(
            !constraints.is_empty(),
            "apron guide rails should be constrained so the CDT cannot collapse the support edge into one cap"
        );
        assert!(
            constraints.iter().any(|constraint| {
                (constraint.start.x + 5.0).abs() <= 0.001
                    && constraint.start.z.abs() <= 1.001
                    && (constraint.start.height_m - 4.0).abs() <= 0.001
                    && (constraint.end.x + 6.0).abs() <= 0.001
                    && constraint.end.z.abs() <= 1.001
                    && (constraint.end.height_m - 3.5).abs() <= 0.001
            }),
            "apron rays should be constrained radially from the support edge to the first ring"
        );
    }

    #[test]
    fn support_tie_in_accepts_flat_surroundings() {
        let terrain = TerrainSystem::with_chunking(32, 32, 1.0, 8, 0.0);
        let graph = RegionGraph::new();
        let road_surface = RoadSurfaceSystem::new(RegionGraph::CHUNK_SIZE);

        assert!(building_site_support_tie_in_is_valid(
            &square_site_with_surface().footprint_world,
            0.0,
            &terrain,
            &graph,
            &road_surface,
        ));
    }

    #[test]
    fn support_tie_in_rejects_oversteep_surroundings() {
        let terrain = TerrainSystem::with_chunking(32, 32, 1.0, 8, 0.0);
        let graph = RegionGraph::new();
        let road_surface = RoadSurfaceSystem::new(RegionGraph::CHUNK_SIZE);

        assert!(!building_site_support_tie_in_is_valid(
            &square_site_with_surface().footprint_world,
            5.0,
            &terrain,
            &graph,
            &road_surface,
        ));
    }

    #[test]
    fn derived_site_client_uses_required_flat_support_footprint() {
        let allocator = BuildingAllocator::new();
        let building = Building {
            center_x: 0.0,
            center_y: 0.0,
            support_height_m: 7.0,
            width_cells: 2,
            depth_cells: 2,
            zone_profile_runtime_id: 0,
            parcel_id: 0,
            zone_type: crate::simulation::zoning::ZoneType::Residential,
            facing_dir: Vector2::new(0.0, 1.0),
            frontage_t: 0.0,
            side_offset: 0.0,
            is_deserted: false,
            budget_distress: false,
            edge_idx: 0,
            side: 1,
            cell_x: 0,
            cell_y: 0,
            occupancy: 0,
            worker_count: 0,
            service_funding_override: -1.0,
            asset_id: String::new(),
            level: 1,
            construction_total_hours: 0,
            construction_remaining_hours: 0,
            broken: false,
            economy_profile_runtime_id: 0,
            economy_broken: false,
            resource_inventory: Vec::new(),
            revenue: 0.0,
            operating_budget: 0.0,
            profit_tax_budget_baseline: 0.0,
            last_day_profit: 0.0,
            shipment_cooldown_hours: 0,
            daily_owa_input_value: 0.0,
            daily_local_input_value: 0.0,
            daily_city_funded_input_cost: 0.0,
            daily_household_sales_value: 0.0,
            daily_power_service_units: 0.0,
            daily_power_served_units: 0.0,
            recent_power_service_units: 0.0,
            recent_power_served_units: 0.0,
            recent_household_sales_value: 0.0,
            commercial_activity_floor_scale: 0.0,
            pending_redevelopment: false,
            rezone_grace_days_remaining: 0,
        };

        let site = allocator.derive_building_site_client(&building, 10.0);

        assert!((signed_polygon_area(&site.footprint_world).abs() - 400.0).abs() <= 0.001);
        assert!((signed_polygon_area(&site.lot_footprint_world).abs() - 400.0).abs() <= 0.001);
        assert!(site.contains_point(Vector2::new(9.9, 0.0)));
        assert!(!site.contains_point(Vector2::new(10.1, 0.0)));
        assert_eq!(site.support_height_m, 7.0);
    }

    #[test]
    fn required_support_footprint_keeps_driveway_clear_of_road_boundary() {
        use crate::assets::BuildingData;
        use crate::assets::asset::PlacementMode;

        let mut mesh_part = MeshPart::single_lod0("main", "main.glb");
        mesh_part.position = [7.0, 0.0, 0.0];
        mesh_part.scale = 2.0;
        let manifest = AssetManifest {
            asset_id: "building.test.site".to_owned(),
            display_name: "Site Test".to_owned(),
            asset_set: None,
            tags: Vec::new(),
            thumbnail: None,
            lods: Vec::new(),
            mesh_parts: vec![mesh_part],
            anchors: vec![
                Anchor {
                    anchor_type: AnchorType::Entrance,
                    name: "main".to_owned(),
                    position: [4.0, 0.0, -2.0],
                    forward: [0.0, 0.0, -1.0],
                    width_m: None,
                    length_m: None,
                    vehicle_class: None,
                },
                Anchor {
                    anchor_type: AnchorType::Driveway,
                    name: String::new(),
                    position: [0.0, 0.0, -15.0],
                    forward: [0.0, 0.0, 1.0],
                    width_m: Some(3.0),
                    length_m: None,
                    vehicle_class: Some("car".to_owned()),
                },
            ],
            site_surfaces: Vec::new(),
            building: Some(BuildingData {
                placement_mode: PlacementMode::Explicit,
                zone_type: None,
                density: None,
                lot_width_cells: 4,
                lot_depth_cells: 3,
                frontage_forward: None,
                min_zone_width_cells: None,
                min_zone_depth_cells: None,
                level: 1,
                household_capacity: None,
                worker_capacity: Some(1),
                flat_size_m2: None,
                service_class: None,
                economy_profile: None,
            }),
            prop: None,
            vehicle: None,
            character: None,
        };

        let support = required_flat_support_footprint_local(&manifest, 20.0, 15.0);
        let frontage_dir = Vector2::new(0.0, -1.0);
        let frontage_limit = frontage_projection_limit(frontage_dir, 20.0, 15.0);
        let support_limit = frontage_limit - BUILDING_SITE_ROAD_ACCESS_CLEARANCE_M;
        let max_frontage_projection = support
            .iter()
            .map(|point| frontage_projection(*point, frontage_dir))
            .fold(f32::NEG_INFINITY, f32::max);
        let access_edge_points = support
            .iter()
            .filter(|point| {
                (frontage_projection(**point, frontage_dir) - support_limit).abs() <= 0.001
            })
            .collect::<Vec<_>>();

        assert!(
            max_frontage_projection <= support_limit + 0.001,
            "access support must stay behind the road boundary: {support:?}"
        );
        assert!(
            !access_edge_points.is_empty(),
            "driveway support should still define an interior access edge: {support:?}"
        );
        assert!(
            access_edge_points.iter().all(|point| point.x.abs() <= 2.0),
            "road-facing access support should stay near the driveway width: {support:?}"
        );
        assert!(
            signed_polygon_area(&support).abs() < 40.0 * 30.0,
            "required support must not silently become the full lot"
        );
    }

    #[test]
    fn site_grading_nearest_road_sample_uses_visible_surface_edge() {
        use crate::simulation::core::config::WorldConfig;
        use crate::simulation::network::TransitNetwork;
        use crate::simulation::network::types::EdgeClass;
        use crate::simulation::zoning::ZoningSystem;

        let mut network = TransitNetwork::new();
        let mut graph = RegionGraph::new();
        let mut zoning = ZoningSystem::new(&WorldConfig::default());
        let mut allocator = BuildingAllocator::new();
        network.add_road(
            &mut graph,
            vec![Vector3::new(0.0, 6.0, 20.0), Vector3::new(60.0, 6.0, 20.0)],
            1,
            1,
            EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );
        let terrain = TerrainSystem::with_chunking(96, 96, 1.0, 16, 0.0);
        network.road_surface.compile_dirty(&graph, &terrain);

        let edge_idx = graph.edge_count() - 1;
        let edge = graph.edge(edge_idx);
        let center = BuildingAllocator::sample_pos_on_edge(&graph, edge_idx, 0.5);
        let tangent = BuildingAllocator::sample_tangent_on_edge(&graph, edge_idx, 0.5);
        let normal = Vector2::new(tangent.y, -tangent.x).normalized();
        let road_edge_probe =
            center + normal * building_site_road_connection_lateral_offset_m(edge);
        let apron_probe = road_edge_probe + normal * 0.5;
        let expected_height_m = network
            .road_surface
            .sample_visible_surface_height(&graph, &terrain, road_edge_probe.x, road_edge_probe.y)
            .expect("road surface edge should be queryable");

        let (probe, height_m) = nearest_building_site_road_surface_sample(
            &terrain,
            &graph,
            &network.road_surface,
            apron_probe,
            BUILDING_SITE_NEAREST_ROAD_SURFACE_MAX_RADIUS_M,
        )
        .expect("nearby apron guide should find the road surface edge");

        assert!(probe.distance_to(road_edge_probe) <= 0.001);
        assert!((height_m - expected_height_m).abs() <= 0.001);
    }
}
