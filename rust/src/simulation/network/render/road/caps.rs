//! Terminal cap geometry and circular fill primitives.

use super::*;
use godot::prelude::*;
use std::f32::consts::TAU;

pub(super) fn emit_disk(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    center: Vector3,
    radius: f32,
    y_offset: f32,
    color: Color,
) {
    if radius <= 0.0 {
        return;
    }
    let center = Vector3::new(center.x, center.y + y_offset, center.z);
    let sectors = circle_segments(radius);
    let mut previous = circle_point(center, radius, 0.0);
    let center_uv = if color.a > 0.9 {
        Vector2::new(0.0, 1.0)
    } else {
        Vector2::ZERO
    };
    let rim_uv = Vector2::new(1.0, 1.0);
    for step in 1..=sectors {
        let angle = step as f32 / sectors as f32 * TAU;
        let current = circle_point(center, radius, angle);
        push_triangle(
            mesh,
            layer,
            [center, previous, current],
            [center_uv, rim_uv, rim_uv],
            color,
        );
        previous = current;
    }
}

fn circle_segments(radius: f32) -> usize {
    ((radius * 2.0).ceil() as usize).clamp(12, 40)
}

fn circle_point(center: Vector3, radius: f32, angle: f32) -> Vector3 {
    Vector3::new(
        center.x + angle.cos() * radius,
        center.y,
        center.z + angle.sin() * radius,
    )
}
