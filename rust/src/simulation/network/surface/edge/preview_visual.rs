// SPDX-License-Identifier: GPL-2.0-only

//! Terrain-draped, material-tagged display geometry; never used for placement or validation.

use super::super::{RoadSurfaceBand, RoadSurfaceBandKind, RoadSurfaceSection, RoadSurfaceSystem};
use crate::config::HEIGHT_SCALE;
use crate::simulation::network::build_surface_edge;
use crate::simulation::network::types::EdgeClass;
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::{Color, Vector2, Vector3};

const DISPLAY_LIFT_M: f32 = 0.15;

/// One preview draw surface with metre-based lane coordinates and constant per-band material tags.
#[derive(Clone, Debug, Default)]
pub(crate) struct RoadPreviewVisualMesh {
    /// Display-only positions, raised above the current visual terrain.
    pub(crate) vertices: Vec<Vector3>,
    /// Longitudinal distance and signed lateral offset, both in metres.
    pub(crate) uvs: Vec<Vector2>,
    /// Red identifies asphalt (0), sidewalk/footpath (0.5), or curb (1).
    pub(crate) colors: Vec<Color>,
}

#[derive(Clone, Copy)]
struct Frame {
    center: Vector3,
    lateral: Vector3,
    distance: f32,
    profile_origin_height: f32,
}

impl RoadSurfaceSystem {
    /// Builds only the hovered stroke: O(samples × lateral strips), independent of city size.
    /// Exact previews reuse compiled cross-sections; moving previews reuse prepared heights and
    /// the authoritative band specification without constructing a graph or compiling junctions.
    pub(crate) fn build_preview_visual_mesh(
        &self,
        points: &[Vector3],
        sections: &[RoadSurfaceSection],
        fwd_lanes: u8,
        bkw_lanes: u8,
        terrain: &TerrainSystem,
    ) -> RoadPreviewVisualMesh {
        let mut mesh = RoadPreviewVisualMesh::default();
        if points.len() < 2 {
            return mesh;
        }
        let step = (terrain.cell_size_m() * 0.25).clamp(0.25, 2.0);
        let edge = build_surface_edge(0, 1, Vec::new(), fwd_lanes, bkw_lanes, EdgeClass::Standard);
        let bands = self.build_lateral_bands(
            &edge,
            super::super::RoadVec3::ZERO,
            super::super::RoadVec2::Y,
            None,
            None,
        );
        // Reserve once for the stroke, not inside the per-quad emission loop. Sequential emission
        // shares longitudinal coordinates and is a single UI stroke, not a city-wide element loop.
        let mut capacity = 0;
        visit_spans(points, sections, &bands, |a, b, bands_a, bands_b| {
            for (band_a, band_b) in bands_a.iter().zip(bands_b) {
                let (along, across) = band_subdivisions(a, b, band_a, band_b, step);
                capacity += along * across * 6;
            }
        });
        mesh.vertices.reserve(capacity);
        mesh.uvs.reserve(capacity);
        mesh.colors.reserve(capacity);
        visit_spans(points, sections, &bands, |a, b, bands_a, bands_b| {
            append_span(&mut mesh, a, b, bands_a, bands_b, terrain, step);
        });
        mesh
    }
}

fn visit_spans(
    points: &[Vector3],
    sections: &[RoadSurfaceSection],
    bands: &[RoadSurfaceBand],
    mut visit: impl FnMut(Frame, Frame, &[RoadSurfaceBand], &[RoadSurfaceBand]),
) {
    if sections.len() >= 2 {
        for pair in sections.windows(2) {
            visit(
                section_frame(&pair[0]),
                section_frame(&pair[1]),
                &pair[0].bands,
                &pair[1].bands,
            );
        }
    } else {
        let mut distance = 0.0;
        for index in 0..points.len() - 1 {
            let a = point_frame(points, index, distance);
            distance += points[index].distance_to(points[index + 1]);
            visit(a, point_frame(points, index + 1, distance), bands, bands);
        }
    }
}

fn section_frame(section: &RoadSurfaceSection) -> Frame {
    Frame {
        center: Vector3::new(
            section.center_xz.x as f32,
            section.center_height_m,
            section.center_xz.y as f32,
        ),
        lateral: Vector3::new(
            section.lateral_xz.x as f32,
            0.0,
            section.lateral_xz.y as f32,
        ),
        distance: section.s_m,
        profile_origin_height: section.center_height_m,
    }
}

fn point_frame(points: &[Vector3], index: usize, distance: f32) -> Frame {
    let direction = points[(index + 1).min(points.len() - 1)] - points[index.saturating_sub(1)];
    let lateral = Vector3::new(-direction.z, 0.0, direction.x)
        .try_normalized()
        .unwrap_or(Vector3::ZERO);
    Frame {
        center: points[index],
        lateral,
        distance,
        profile_origin_height: 0.0,
    }
}

#[allow(clippy::too_many_arguments)]
fn append_span(
    mesh: &mut RoadPreviewVisualMesh,
    a: Frame,
    b: Frame,
    bands_a: &[RoadSurfaceBand],
    bands_b: &[RoadSurfaceBand],
    terrain: &TerrainSystem,
    step: f32,
) {
    if b.distance <= a.distance || a.lateral == Vector3::ZERO || b.lateral == Vector3::ZERO {
        return;
    }
    for (band_a, band_b) in bands_a.iter().zip(bands_b) {
        let (along, across) = band_subdivisions(a, b, band_a, band_b, step);
        let tag = match band_a.kind {
            RoadSurfaceBandKind::Carriageway => 0.0,
            RoadSurfaceBandKind::CurbOrShoulder => 1.0,
            _ => 0.5,
        };
        let color = Color::from_rgba(tag, 0.0, 0.0, 1.0);
        for lateral in 0..across {
            let v0 = lateral as f32 / across as f32;
            let v1 = (lateral + 1) as f32 / across as f32;
            let sample = |u: f32, v: f32| {
                let mut point = band_point(a, band_a, v).lerp(band_point(b, band_b, v), u);
                let height_offset = band_height_offset(a, band_a, v) * (1.0 - u)
                    + band_height_offset(b, band_b, v) * u;
                let ground = terrain.sample_visual_height_world(point.x, point.z) * HEIGHT_SCALE;
                point.y = point.y.max(ground + height_offset.max(0.0)) + DISPLAY_LIFT_M;
                let lateral_a =
                    band_a.lateral_start_m + (band_a.lateral_end_m - band_a.lateral_start_m) * v;
                let lateral_b =
                    band_b.lateral_start_m + (band_b.lateral_end_m - band_b.lateral_start_m) * v;
                (
                    point,
                    Vector2::new(
                        a.distance + (b.distance - a.distance) * u,
                        lateral_a + (lateral_b - lateral_a) * u,
                    ),
                )
            };
            let mut left = sample(0.0, v0);
            let mut right = sample(0.0, v1);
            for longitudinal in 1..=along {
                let u = longitudinal as f32 / along as f32;
                let next_left = sample(u, v0);
                let next_right = sample(u, v1);
                for (position, uv) in [left, next_left, right, right, next_left, next_right] {
                    mesh.vertices.push(position);
                    mesh.uvs.push(uv);
                    mesh.colors.push(color);
                }
                left = next_left;
                right = next_right;
            }
        }
    }
}

fn band_subdivisions(
    a: Frame,
    b: Frame,
    band_a: &RoadSurfaceBand,
    band_b: &RoadSurfaceBand,
    step: f32,
) -> (usize, usize) {
    let width = (band_a.lateral_end_m - band_a.lateral_start_m)
        .abs()
        .max((band_b.lateral_end_m - band_b.lateral_start_m).abs());
    // Bound spacing at the outside of a bend too, not just along its centreline.
    let length = [0.0, 1.0]
        .into_iter()
        .map(|v| band_point(a, band_a, v).distance_to(band_point(b, band_b, v)))
        .fold(b.distance - a.distance, f32::max);
    (
        (length / step).ceil().max(1.0) as usize,
        (width / step).ceil().max(1.0) as usize,
    )
}

fn band_point(frame: Frame, band: &RoadSurfaceBand, v: f32) -> Vector3 {
    let lateral = band.lateral_start_m + (band.lateral_end_m - band.lateral_start_m) * v;
    let mut point = frame.center + frame.lateral * lateral;
    point.y = frame.center.y + band_height_offset(frame, band, v);
    point
}

fn band_height_offset(frame: Frame, band: &RoadSurfaceBand, v: f32) -> f32 {
    // Fast bands are authored about zero; compiled bands carry absolute heights.
    let height = band.height_start_m + (band.height_end_m - band.height_start_m) * v;
    height - frame.profile_origin_height
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_visual_drapes_full_width_without_changing_planned_heights() {
        let mut terrain = TerrainSystem::with_chunking(33, 33, 8.0, 8, 0.0);
        for z in 0..33 {
            for x in 0..33 {
                let crest = (5.0 - (x as f32 - 16.0).abs() * 0.5).max(0.0);
                terrain.set_height(x, z, (crest + z as f32 * 0.1) / HEIGHT_SCALE);
            }
        }
        let points = [Vector3::new(-40.0, 0.0, 0.0), Vector3::new(40.0, 0.0, 0.0)];
        let surface = RoadSurfaceSystem::new(128.0);
        let mesh = surface.build_preview_visual_mesh(&points, &[], 2, 1, &terrain);
        assert!(!mesh.vertices.is_empty());
        assert_eq!(mesh.vertices.len(), mesh.uvs.len());
        assert_eq!(mesh.vertices.len(), mesh.colors.len());
        assert_eq!(points[0].y, 0.0);
        for point in &mesh.vertices {
            let ground = terrain.sample_visual_height_world(point.x, point.z) * HEIGHT_SCALE;
            assert!(point.y >= ground + DISPLAY_LIFT_M - 0.0001);
        }
        // Check triangle interiors too: endpoints alone missed the hill and the cross-slope.
        for triangle in mesh.vertices.chunks_exact(3) {
            for i in 0..=4 {
                for j in 0..=4 - i {
                    let u = i as f32 / 4.0;
                    let v = j as f32 / 4.0;
                    let p = triangle[0] * (1.0 - u - v) + triangle[1] * u + triangle[2] * v;
                    assert!(p.y > terrain.sample_visual_height_world(p.x, p.z) * HEIGHT_SCALE);
                }
            }
        }
        assert!(mesh.colors.iter().any(|color| color.r == 0.0));
        assert!(mesh.colors.iter().any(|color| color.r == 0.5));
        assert!(mesh.colors.iter().any(|color| color.r == 1.0));
    }

    #[test]
    fn preview_visual_preserves_elevation_and_walkway_width() {
        let terrain = TerrainSystem::with_chunking(33, 33, 8.0, 8, 0.0);
        let surface = RoadSurfaceSystem::new(128.0);
        let points = [
            Vector3::new(-20.0, 12.0, 0.0),
            Vector3::new(20.0, 12.0, 0.0),
        ];
        let mesh = surface.build_preview_visual_mesh(&points, &[], 0, 0, &terrain);
        assert!(!mesh.vertices.is_empty());
        for ((point, uv), color) in mesh.vertices.iter().zip(&mesh.uvs).zip(&mesh.colors) {
            assert!((point.y - 12.0 - DISPLAY_LIFT_M).abs() < 0.0001);
            assert!(uv.y.abs() <= 1.0);
            assert_eq!(color.r, 0.5);
        }
        for (fwd, bkw) in [(1, 0), (0, 1), (1, 1), (3, 1), (4, 4)] {
            let mesh = surface.build_preview_visual_mesh(&points, &[], fwd, bkw, &terrain);
            let expected = f32::from(fwd + bkw) * crate::config::LANE_WIDTH * 0.5
                + crate::config::SIDEWALK_WIDTH;
            let width = mesh.uvs.iter().map(|uv| uv.y.abs()).fold(0.0_f32, f32::max);
            assert!((width - expected).abs() < 0.0001);
            assert!(
                mesh.vertices
                    .iter()
                    .all(|point| point.y >= 12.0 + DISPLAY_LIFT_M - 0.0001)
            );
        }
    }

    #[test]
    fn preview_visual_exact_sections_keep_curved_band_coordinates() {
        let terrain = TerrainSystem::with_chunking(33, 33, 8.0, 8, 0.0);
        let surface = RoadSurfaceSystem::new(128.0);
        let points = [
            Vector3::new(-30.0, 0.0, 0.0),
            Vector3::new(0.0, 0.0, 10.0),
            Vector3::new(30.0, 0.0, 0.0),
        ];
        let compiled = surface.compile_preview_surface(&points, 2, 1, &terrain);
        let mesh = surface.build_preview_visual_mesh(
            &compiled.prepared_points,
            &compiled.compiled_sections,
            2,
            1,
            &terrain,
        );
        assert!(!mesh.vertices.is_empty());
        assert_eq!(mesh.vertices.len() % 3, 0);
        for triangle in mesh.uvs.chunks_exact(3) {
            assert!(triangle.iter().all(|uv| uv.is_finite()));
            let max_s = triangle
                .iter()
                .map(|uv| uv.x)
                .fold(f32::NEG_INFINITY, f32::max);
            let min_s = triangle.iter().map(|uv| uv.x).fold(f32::INFINITY, f32::min);
            assert!(max_s - min_s <= 2.01);
        }
    }
}
