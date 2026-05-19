//! Terrain visual-height stamping for road-owned earthwork support.

use super::super::{
    RoadSurfaceSection, RoadSurfaceSpanOwnedRegion, RoadSurfaceSystem, RoadSurfaceVisualPolygon,
    SurfaceChunkKey,
};
use crate::config;
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::{Vector2, Vector3};
use std::collections::BTreeMap;

impl RoadSurfaceSystem {
    pub(super) fn section_is_tunnel_surface_visible(
        &self,
        section: &RoadSurfaceSection,
        terrain: &TerrainSystem,
    ) -> bool {
        let terrain_height = terrain.sample_height_world(section.center_xz.x, section.center_xz.y)
            * config::HEIGHT_SCALE;
        section.center_height_m >= terrain_height - super::TUNNEL_PORTAL_STAMP_DEPTH_M
    }

    pub(in crate::simulation::network::surface) fn tunnel_throat_is_visible(
        &self,
        edge_idx: usize,
        at_start: bool,
        terrain: &TerrainSystem,
    ) -> bool {
        let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
            return false;
        };
        let mouth = if at_start {
            piece.start_mouth_profile.as_ref()
        } else {
            piece.end_mouth_profile.as_ref()
        };
        let Some(mouth) = mouth else {
            return false;
        };
        if mouth.boundary_points_world.len() < 2 {
            return false;
        }
        let Some(sections) = self.compiled_sections.get(&edge_idx) else {
            return false;
        };
        let section = if at_start {
            sections.first()
        } else {
            sections.last()
        };
        let Some(section) = section else {
            return false;
        };
        self.section_is_tunnel_surface_visible(section, terrain)
    }

    pub(super) fn stamp_piece_top_surface_clearance_for_chunk(
        &self,
        road_surface_polygons: &[RoadSurfaceVisualPolygon],
        curb_surface_polygons: &[RoadSurfaceVisualPolygon],
        sidewalk_surface_polygons: &[RoadSurfaceVisualPolygon],
        chunk: SurfaceChunkKey,
        terrain: &mut TerrainSystem,
        height_offset_m: f32,
    ) {
        let conservative_margin_m = terrain.cell_size_m() * std::f32::consts::SQRT_2 * 0.5;
        let mut candidates: BTreeMap<(usize, usize), (f32, f32)> = BTreeMap::new();

        for polygon in road_surface_polygons
            .iter()
            .chain(curb_surface_polygons)
            .chain(sidewalk_surface_polygons)
        {
            Self::visit_visual_polygon_triangles(polygon, &mut |triangle| {
                self.collect_top_surface_support_triangle_candidates(
                    terrain,
                    chunk,
                    triangle,
                    conservative_margin_m,
                    height_offset_m,
                    &mut candidates,
                );
            });
        }

        for ((grid_x, grid_z), (_, height_sample)) in candidates {
            terrain.set_visual_height_at_grid(grid_x, grid_z, height_sample);
        }
    }

    pub(super) fn stamp_span_top_surface_support_for_chunk(
        &self,
        regions: &[RoadSurfaceSpanOwnedRegion],
        chunk: SurfaceChunkKey,
        terrain: &mut TerrainSystem,
        height_offset_m: f32,
    ) {
        let conservative_margin_m = terrain.cell_size_m() * std::f32::consts::SQRT_2 * 0.5;
        let mut candidates: BTreeMap<(usize, usize), (f32, f32)> = BTreeMap::new();

        for region in regions {
            Self::visit_visual_polygon_triangles(&region.polygon, &mut |triangle| {
                self.collect_top_surface_support_triangle_candidates(
                    terrain,
                    chunk,
                    triangle,
                    conservative_margin_m,
                    height_offset_m,
                    &mut candidates,
                );
            });
        }

        for ((grid_x, grid_z), (_, height_sample)) in candidates {
            terrain.set_visual_height_at_grid(grid_x, grid_z, height_sample);
        }
    }

    fn collect_top_surface_support_triangle_candidates(
        &self,
        terrain: &TerrainSystem,
        chunk: SurfaceChunkKey,
        triangle: [Vector3; 3],
        conservative_margin_m: f32,
        height_offset_m: f32,
        candidates: &mut BTreeMap<(usize, usize), (f32, f32)>,
    ) {
        if !Self::triangle_has_area_xz(triangle) {
            return;
        }

        let (chunk_min, chunk_max) = self.chunk_bounds(chunk);
        let min_x = triangle
            .iter()
            .map(|point| point.x)
            .fold(chunk_max.x, f32::min)
            .max(chunk_min.x - conservative_margin_m);
        let max_x = triangle
            .iter()
            .map(|point| point.x)
            .fold(chunk_min.x, f32::max)
            .min(chunk_max.x + conservative_margin_m);
        let min_z = triangle
            .iter()
            .map(|point| point.z)
            .fold(chunk_max.z, f32::min)
            .max(chunk_min.z - conservative_margin_m);
        let max_z = triangle
            .iter()
            .map(|point| point.z)
            .fold(chunk_min.z, f32::max)
            .min(chunk_max.z + conservative_margin_m);
        let Some((min_grid_x, max_grid_x, min_grid_z, max_grid_z)) =
            terrain.grid_rect_for_world_bounds(min_x, min_z, max_x, max_z)
        else {
            return;
        };
        let (grid_width, grid_height) = terrain.grid_dimensions();
        if grid_width == 0 || grid_height == 0 {
            return;
        }
        let max_grid_x_index = grid_width.saturating_sub(1);
        let max_grid_z_index = grid_height.saturating_sub(1);
        let grid_min_x = min_grid_x.saturating_sub(1).min(max_grid_x_index);
        let grid_max_x = max_grid_x.saturating_add(1).min(max_grid_x_index);
        let grid_min_z = min_grid_z.saturating_sub(1).min(max_grid_z_index);
        let grid_max_z = max_grid_z.saturating_add(1).min(max_grid_z_index);

        for grid_z in grid_min_z..=grid_max_z {
            for grid_x in grid_min_x..=grid_max_x {
                let (world_x, world_z) = terrain.grid_to_world_coords(grid_x, grid_z);
                let point_xz = Vector2::new(world_x, world_z);
                if !Self::point_is_inside_or_near_triangle_xz(
                    triangle,
                    point_xz,
                    conservative_margin_m,
                ) {
                    continue;
                }
                let Some((distance_squared, height_sample)) =
                    Self::top_surface_support_candidate_from_triangle(
                        triangle,
                        point_xz,
                        height_offset_m,
                    )
                else {
                    continue;
                };
                let entry = candidates
                    .entry((grid_x, grid_z))
                    .or_insert((distance_squared, height_sample));
                if Self::top_surface_support_candidate_replaces(
                    *entry,
                    (distance_squared, height_sample),
                ) {
                    *entry = (distance_squared, height_sample);
                }
            }
        }
    }

    fn top_surface_support_candidate_replaces(existing: (f32, f32), candidate: (f32, f32)) -> bool {
        let (existing_distance_squared, existing_height_sample) = existing;
        let (candidate_distance_squared, candidate_height_sample) = candidate;
        candidate_distance_squared < existing_distance_squared - 0.0001
            || ((candidate_distance_squared - existing_distance_squared).abs() <= 0.0001
                && candidate_height_sample < existing_height_sample)
    }

    fn top_surface_support_candidate_from_triangle(
        triangle: [Vector3; 3],
        point_xz: Vector2,
        height_offset_m: f32,
    ) -> Option<(f32, f32)> {
        let sample_point_xz = Self::closest_point_on_triangle_xz(triangle, point_xz);
        let (wa, wb, wc) = Self::triangle_barycentric_weights_xz(triangle, sample_point_xz)?;
        let support_height_m = triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc;
        let clearance_sample = (support_height_m - height_offset_m) / config::HEIGHT_SCALE;
        Some((
            point_xz.distance_squared_to(sample_point_xz),
            clearance_sample,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn earthwork_support_candidates_use_lower_envelope_for_overlapping_top_surfaces() {
        assert!(RoadSurfaceSystem::top_surface_support_candidate_replaces(
            (0.0, 12.0),
            (0.0, 10.0),
        ));
        assert!(!RoadSurfaceSystem::top_surface_support_candidate_replaces(
            (0.0, 10.0),
            (0.0, 12.0),
        ));
    }

    #[test]
    fn earthwork_support_candidates_prefer_smaller_distance_before_height() {
        assert!(RoadSurfaceSystem::top_surface_support_candidate_replaces(
            (4.0, 1.0),
            (1.0, 10.0),
        ));
        assert!(!RoadSurfaceSystem::top_surface_support_candidate_replaces(
            (1.0, 10.0),
            (4.0, 1.0),
        ));
    }

    #[test]
    fn earthwork_hardcut_has_no_per_material_sequential_stamping_path() {
        let source = include_str!("../earthwork.rs");
        for forbidden in [
            concat!("stamp_piece_surface_", "geometry_for_chunk"),
            concat!("profile_clearance_", "candidate_from_triangle"),
            concat!("collect_profile_clearance_", "triangle_candidates"),
        ] {
            assert!(
                !source.contains(forbidden),
                "road-touched terrain support must use one canonical lower-envelope pass, not `{forbidden}`"
            );
        }
    }
}
