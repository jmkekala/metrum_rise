// SPDX-License-Identifier: GPL-2.0-only

//! Shared section projection and quad emission helpers for section-based road meshes, and the
//! emission of the strips a piece hangs from its outer chains.

use super::super::{
    MeshLayer, NetworkMeshData, push_triangle, push_triangle_preserving_winding_with_exact_normal,
    render_point, world_point,
};
use crate::simulation::network::roadbed::{PieceMesh, RoadSection};
use godot::prelude::{Color, Vector2, Vector3};

const BAND_EPSILON_M: f32 = 0.001;
const MIN_RENDER_TRIANGLE_DOUBLE_AREA_M2: f32 = 1.0e-8;

pub(super) fn section_world_point_at_lateral_offset(
    section: &RoadSection,
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

pub(super) fn section_boundary_world_point(
    section: &RoadSection,
    lateral_offset_m: f32,
    height_m: f32,
) -> Vector3 {
    Vector3::new(
        (section.center_xz.x + section.lateral_xz.x * f64::from(lateral_offset_m)) as f32,
        height_m,
        (section.center_xz.y + section.lateral_xz.y * f64::from(lateral_offset_m)) as f32,
    )
}

pub(super) fn triangle_is_too_small(a: Vector3, b: Vector3, c: Vector3) -> bool {
    let double_area_squared = (b - a).cross(c - a).length_squared();
    double_area_squared <= MIN_RENDER_TRIANGLE_DOUBLE_AREA_M2 * MIN_RENDER_TRIANGLE_DOUBLE_AREA_M2
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
            [vertices[0], vertices[1], vertices[2]].map(world_point),
            [uvs[0], uvs[1], uvs[2]],
            color,
        );
    }
    if !triangle_is_too_small(vertices[0], vertices[2], vertices[3]) {
        push_triangle(
            mesh,
            layer,
            [vertices[0], vertices[2], vertices[3]].map(world_point),
            [uvs[0], uvs[2], uvs[3]],
            color,
        );
    }
}

// Emits the piece's strips (`TERRAIN-07-B`: skirts, or a bridge's fascias) into `layer` as the
// piece wound them, facing away from the road, with UV `y` 1 on bottom vertices and 0 on the
// chain's: the skirt shader's bottom-vertex attribute. Only exactly degenerate triangles are
// dropped (by the chunk router), so every strip edge meets its neighbour's.
pub(super) fn emit_chain_strips(
    mesh: &mut NetworkMeshData,
    piece: &PieceMesh,
    layer: MeshLayer,
    color: Color,
) {
    piece.for_each_strip_triangle(|world, bottoms| {
        let [a, b, c] = world.map(render_point);
        push_triangle_preserving_winding_with_exact_normal(
            mesh,
            layer,
            world,
            bottoms.map(|bottom| Vector2::new(0.0, f32::from(u8::from(bottom)))),
            color,
            -((b - a).cross(c - a)),
        );
    });
}
