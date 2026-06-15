//! Building-site footprint, surface, and terrain-query helpers.

use crate::assets::SiteSurfaceMaterial;
use crate::simulation::buildings::allocator::entrance::{
    building_local_xz_basis, building_local_xz_pos, main_entrance_anchor,
};
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::terrain::cdt::{
    TerrainCdtRoadBoundarySource, TerrainCdtRoadLoop, TerrainCdtRoadLoopSourceEdge,
    TerrainCdtVertex,
};
use godot::prelude::{Vector2, Vector3};

const DEFAULT_ANCHOR_FORWARD: [f32; 3] = [0.0, 0.0, 1.0];
const BUILDING_SITE_FOOTPRINT_GROUP_MASK: u64 = 0x8000_0000_0000_0000;
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

/// Runtime client that owns the flat whole-lot building-site surface.
#[derive(Clone, Debug)]
pub(crate) struct BuildingSiteClient {
    /// World-space lot footprint corners.
    pub(crate) footprint_world: [Vector2; 4],
    /// Flat support height of the whole site.
    pub(crate) support_height_m: f32,
    /// Authored site surface polygons transformed into world space.
    pub(crate) surfaces: Vec<BuildingSiteSurfaceClient>,
}

impl BuildingSiteClient {
    pub(crate) fn bounds(&self) -> (f32, f32, f32, f32) {
        polygon_bounds(self.footprint_world)
    }

    fn overlaps_bounds(&self, min_x: f32, min_z: f32, max_x: f32, max_z: f32) -> bool {
        let (site_min_x, site_min_z, site_max_x, site_max_z) = self.bounds();
        site_min_x <= max_x && site_max_x >= min_x && site_min_z <= max_z && site_max_z >= min_z
    }

    pub(crate) fn contains_point(&self, pos: Vector2) -> bool {
        point_in_polygon(pos, &self.footprint_world)
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
            .map(|site| site.bounds())
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
        self.site_candidate_indices_for_bounds(pos.x, pos.y, pos.x, pos.y)
            .into_iter()
            .find_map(|idx| self.building_sites.get(idx)?.height_at(pos))
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

    fn site_candidate_indices_for_bounds(
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
        let anchor_forward = self
            .registry
            .get(&building.asset_id)
            .and_then(|entry| main_entrance_anchor(&entry.manifest.anchors))
            .map(|anchor| anchor.forward)
            .unwrap_or(DEFAULT_ANCHOR_FORWARD);
        let (basis_x, basis_z) = building_local_xz_basis(building.facing_dir, anchor_forward);
        let center = Vector2::new(building.center_x, building.center_y);
        let half_width = building.width_cells as f32 * zone_cell_m * 0.5;
        let half_depth = building.depth_cells as f32 * zone_cell_m * 0.5;
        let footprint_world = [
            center + basis_x * -half_width + basis_z * -half_depth,
            center + basis_x * -half_width + basis_z * half_depth,
            center + basis_x * half_width + basis_z * half_depth,
            center + basis_x * half_width + basis_z * -half_depth,
        ];
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
                                    anchor_forward,
                                )
                            })
                            .collect(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        BuildingSiteClient {
            footprint_world,
            support_height_m: building.support_height_m,
            surfaces,
        }
    }
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

fn polygon_bounds<const N: usize>(points: [Vector2; N]) -> (f32, f32, f32, f32) {
    let mut min_x = f32::INFINITY;
    let mut min_z = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_z = f32::NEG_INFINITY;
    for point in points {
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

fn point_in_polygon<const N: usize>(pos: Vector2, polygon: &[Vector2; N]) -> bool {
    point_in_polygon_slice(pos, polygon)
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
            footprint_world: [
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
}
