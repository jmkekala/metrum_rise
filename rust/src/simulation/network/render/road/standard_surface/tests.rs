// SPDX-License-Identifier: GPL-2.0-only

//! Compiled road-surface renderer regression tests.

use super::geometry::{
    stable_surface_group_normal, triangle_is_too_small, world_xz_uvs_for_triangle,
};
use crate::simulation::network::surface::{RoadSurfaceSystem, RoadSurfaceVisualPolygon, RoadVec3};
use godot::prelude::Vector3;

#[test]
fn compiled_surface_renderer_has_no_artificial_top_surface_offset_path() {
    let source = concat!(
        include_str!("../standard_surface.rs"),
        include_str!("coverage.rs"),
        include_str!("top_surface.rs"),
        include_str!("bridge.rs"),
        include_str!("markings.rs"),
        include_str!("earthwork.rs"),
        include_str!("geometry.rs"),
    );
    let render_prefix = concat!("ren", "der_");
    let vertical_offset_token = concat!("b", "ias");
    let forbidden = [
        concat!("ROAD_TOP_SURFACE_RENDER_", "Z_", "BIAS_M").to_owned(),
        format!("{render_prefix}z_{vertical_offset_token}_for_layer"),
        format!("apply_{render_prefix}z_{vertical_offset_token}"),
    ];
    for forbidden in forbidden {
        assert!(
            !source.contains(forbidden.as_str()),
            "compiled road surfaces must render at solved physical coordinates, not through `{forbidden}`"
        );
    }
}

#[test]
fn renderer_keeps_valid_skinny_surface_triangles() {
    let a = Vector3::new(0.0, 0.0, 0.0);
    let b = Vector3::new(2.0, 0.0, 0.0);
    let c = Vector3::new(2.0, 0.0, 0.0005);

    assert!(
        !triangle_is_too_small(a, b, c),
        "compiled road surfaces must not drop valid millimetre-scale closure triangles"
    );
}

#[test]
fn renderer_drops_top_surface_needle_triangles() {
    let triangle = [
        RoadVec3::new(0.0, 0.0, 0.0),
        RoadVec3::new(3.687, 0.0, 0.0),
        RoadVec3::new(0.0, 0.0, 0.000002826),
    ];

    assert!(!RoadSurfaceSystem::top_surface_triangle_is_renderable_xz(
        triangle
    ));
}

#[test]
fn renderer_keeps_stable_top_surface_triangles() {
    let triangle = [
        RoadVec3::new(0.0, 0.0, 0.0),
        RoadVec3::new(2.0, 0.0, 0.0),
        RoadVec3::new(0.0, 0.0, 2.0),
    ];

    assert!(RoadSurfaceSystem::top_surface_triangle_is_renderable_xz(
        triangle
    ));
}

#[test]
fn renderer_drops_degenerate_surface_triangles() {
    let a = Vector3::new(0.0, 0.0, 0.0);
    let b = Vector3::new(1.0, 0.0, 0.0);
    let c = Vector3::new(2.0, 0.0, 0.0);

    assert!(triangle_is_too_small(a, b, c));
}

#[test]
fn renderer_keeps_valid_vertical_earthwork_triangles() {
    let a = Vector3::new(0.0, 0.12, 0.0);
    let b = Vector3::new(1.0, 0.12, 0.0);
    let c = Vector3::new(1.0, 0.0, 0.0);

    assert!(
        !triangle_is_too_small(a, b, c),
        "retaining and wall faces are vertical and must survive render culling"
    );
}

#[test]
fn renderer_uses_world_xz_uvs_for_compiled_top_surfaces() {
    let triangle = [
        Vector3::new(10.0, 1.0, -2.0),
        Vector3::new(12.5, 1.4, -2.0),
        Vector3::new(12.5, 1.8, 3.5),
    ];
    let uvs = world_xz_uvs_for_triangle(triangle);

    assert_eq!(uvs[0].x, 10.0);
    assert_eq!(uvs[0].y, -2.0);
    assert_eq!(uvs[1].x, 12.5);
    assert_eq!(uvs[1].y, -2.0);
    assert_eq!(uvs[2].x, 12.5);
    assert_eq!(uvs[2].y, 3.5);
}

#[test]
fn renderer_uses_group_normal_for_node_top_surface_slivers() {
    let flat = RoadSurfaceVisualPolygon::from_parts(
        Vec::new(),
        vec![
            [
                RoadVec3::new(0.0, 0.0, 0.0),
                RoadVec3::new(6.0, 0.0, 0.0),
                RoadVec3::new(0.0, 0.0, 6.0),
            ],
            [
                RoadVec3::new(6.0, 0.0, 0.0),
                RoadVec3::new(6.0, 0.0, 6.0),
                RoadVec3::new(0.0, 0.0, 6.0),
            ],
        ],
    );
    let skinny_mouth = RoadSurfaceVisualPolygon::from_parts(
        Vec::new(),
        vec![[
            RoadVec3::new(0.0, 0.0, 0.0),
            RoadVec3::new(0.002, 2.0, 0.0),
            RoadVec3::new(0.0, 0.0, 0.002),
        ]],
    );

    let normal = stable_surface_group_normal(&[flat, skinny_mouth])
        .expect("dominant node top surface should provide a stable render normal");

    assert!(
        normal.y > 0.99,
        "stable group normal should be dominated by real top surface area, got {normal:?}"
    );
}
