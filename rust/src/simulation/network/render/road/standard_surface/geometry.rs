// SPDX-License-Identifier: GPL-2.0-only

//! Shared triangle emission, section projection, winding, normals, and UV helpers.

use super::super::{
    MeshLayer, NetworkMeshData, push_triangle, push_triangle_preserving_winding_with_exact_normal,
    push_triangle_with_normal,
};
use crate::simulation::network::surface::{
    RoadSurfaceSection, RoadSurfaceSystem, RoadSurfaceVisualPolygon, RoadVec3,
};
use godot::prelude::{Color, Vector2, Vector3};

const BAND_EPSILON_M: f32 = 0.001;
const MIN_RENDER_TRIANGLE_DOUBLE_AREA_M2: f32 = 1.0e-8;

pub(super) fn section_world_point_at_lateral_offset(
    section: &RoadSurfaceSection,
    lateral_offset_m: f32,
) -> Option<Vector3> {
    for band in &section.bands {
        let start = band.lateral_start_m.min(band.lateral_end_m);
        let end = band.lateral_start_m.max(band.lateral_end_m);
        if lateral_offset_m < start - BAND_EPSILON_M || lateral_offset_m > end + BAND_EPSILON_M {
            continue;
        }

        let span = (band.lateral_end_m - band.lateral_start_m).abs();
        let t = if span <= BAND_EPSILON_M {
            0.0
        } else {
            ((lateral_offset_m - band.lateral_start_m)
                / (band.lateral_end_m - band.lateral_start_m))
                .clamp(0.0, 1.0)
        };
        let height_m = band.height_start_m + (band.height_end_m - band.height_start_m) * t;
        return Some(section_boundary_world_point(
            section,
            lateral_offset_m,
            height_m,
        ));
    }

    None
}

pub(super) fn emit_surface_polygon(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    polygon: &RoadSurfaceVisualPolygon,
    color: Color,
) {
    emit_surface_polygon_with_group_normal(mesh, layer, polygon, color, None);
}

pub(super) fn emit_node_top_surface_polygons(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    polygons: &[RoadSurfaceVisualPolygon],
    color: Color,
) {
    let group_normal = stable_surface_group_normal(polygons);
    for polygon in polygons {
        emit_surface_polygon_with_group_normal(mesh, layer, polygon, color, group_normal);
    }
}

fn emit_surface_polygon_with_group_normal(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    polygon: &RoadSurfaceVisualPolygon,
    color: Color,
    group_normal: Option<Vector3>,
) {
    if polygon.triangles_world.is_empty() {
        return;
    }

    for triangle in &polygon.triangles_world {
        if !RoadSurfaceSystem::top_surface_triangle_is_renderable_xz(*triangle) {
            continue;
        }
        let triangle = road_triangle_to_render(*triangle);
        if let Some(normal) = group_normal {
            push_triangle_with_normal(
                mesh,
                layer,
                triangle,
                world_xz_uvs_for_triangle(triangle),
                color,
                normal,
            );
        } else {
            push_triangle(
                mesh,
                layer,
                triangle,
                world_xz_uvs_for_triangle(triangle),
                color,
            );
        }
    }
}

pub(super) fn emit_vertical_surface_polygon(
    mesh: &mut NetworkMeshData,
    polygon: &RoadSurfaceVisualPolygon,
    color: Color,
) {
    if !polygon.triangles_world.is_empty() {
        for triangle in &polygon.triangles_world {
            let triangle = road_triangle_to_render(*triangle);
            if triangle_is_too_small(triangle[0], triangle[1], triangle[2]) {
                continue;
            }
            let normal = vertical_surface_visible_normal(triangle);
            push_triangle_preserving_winding_with_exact_normal(
                mesh,
                MeshLayer::RaisedStep,
                triangle,
                [
                    Vector2::ZERO,
                    Vector2::new(1.0, 0.0),
                    Vector2::new(1.0, 1.0),
                ],
                color,
                normal,
            );
        }
        return;
    }

    let [upper_start, lower_start, lower_end, upper_end] = polygon.points_world.as_slice() else {
        return;
    };

    let vertices = [
        road_vec3_to_render(*upper_start),
        road_vec3_to_render(*lower_start),
        road_vec3_to_render(*lower_end),
        road_vec3_to_render(*upper_end),
    ];
    let uvs = [
        Vector2::ZERO,
        Vector2::new(1.0, 0.0),
        Vector2::new(1.0, 1.0),
        Vector2::new(0.0, 1.0),
    ];

    for (triangle, triangle_uvs) in [
        (
            [vertices[0], vertices[1], vertices[2]],
            [uvs[0], uvs[1], uvs[2]],
        ),
        (
            [vertices[0], vertices[2], vertices[3]],
            [uvs[0], uvs[2], uvs[3]],
        ),
    ] {
        if triangle_is_too_small(triangle[0], triangle[1], triangle[2]) {
            continue;
        }
        let normal = vertical_surface_visible_normal(triangle);
        push_triangle_preserving_winding_with_exact_normal(
            mesh,
            MeshLayer::RaisedStep,
            triangle,
            triangle_uvs,
            color,
            normal,
        );
    }
}

fn vertical_surface_visible_normal(triangle: [Vector3; 3]) -> Vector3 {
    -((triangle[1] - triangle[0]).cross(triangle[2] - triangle[0]))
}

pub(super) fn section_boundary_world_point(
    section: &RoadSurfaceSection,
    lateral_offset_m: f32,
    height_m: f32,
) -> Vector3 {
    Vector3::new(
        (section.center_xz.x + section.lateral_xz.x * f64::from(lateral_offset_m)) as f32,
        height_m,
        (section.center_xz.y + section.lateral_xz.y * f64::from(lateral_offset_m)) as f32,
    )
}

fn road_vec3_to_render(point: RoadVec3) -> Vector3 {
    Vector3::new(point.x as f32, point.y as f32, point.z as f32)
}

fn road_triangle_to_render(triangle: [RoadVec3; 3]) -> [Vector3; 3] {
    [
        road_vec3_to_render(triangle[0]),
        road_vec3_to_render(triangle[1]),
        road_vec3_to_render(triangle[2]),
    ]
}

pub(super) fn triangle_is_too_small(a: Vector3, b: Vector3, c: Vector3) -> bool {
    let double_area_squared = (b - a).cross(c - a).length_squared();
    double_area_squared <= MIN_RENDER_TRIANGLE_DOUBLE_AREA_M2 * MIN_RENDER_TRIANGLE_DOUBLE_AREA_M2
}

pub(super) fn stable_surface_group_normal(
    polygons: &[RoadSurfaceVisualPolygon],
) -> Option<Vector3> {
    let mut normal = Vector3::ZERO;
    for polygon in polygons {
        for triangle in &polygon.triangles_world {
            if !RoadSurfaceSystem::top_surface_triangle_is_renderable_xz(*triangle) {
                continue;
            }
            let triangle = road_triangle_to_render(*triangle);
            let mut triangle_normal = (triangle[1] - triangle[0]).cross(triangle[2] - triangle[0]);
            if triangle_normal.y < 0.0 {
                triangle_normal = -triangle_normal;
            }
            normal += triangle_normal;
        }
    }
    (normal.length_squared() > 1e-8).then(|| normal.normalized())
}

pub(super) fn world_xz_uvs_for_triangle(triangle: [Vector3; 3]) -> [Vector2; 3] {
    [
        Vector2::new(triangle[0].x, triangle[0].z),
        Vector2::new(triangle[1].x, triangle[1].z),
        Vector2::new(triangle[2].x, triangle[2].z),
    ]
}

pub(super) fn emit_surface_quad(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    vertices: [Vector3; 4],
    uvs: [Vector2; 4],
    color: Color,
) {
    if !triangle_is_too_small(vertices[0], vertices[1], vertices[2]) {
        push_triangle(
            mesh,
            layer,
            [vertices[0], vertices[1], vertices[2]],
            [uvs[0], uvs[1], uvs[2]],
            color,
        );
    }
    if !triangle_is_too_small(vertices[0], vertices[2], vertices[3]) {
        push_triangle(
            mesh,
            layer,
            [vertices[0], vertices[2], vertices[3]],
            [uvs[0], uvs[2], uvs[3]],
            color,
        );
    }
}
