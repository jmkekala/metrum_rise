// SPDX-License-Identifier: GPL-2.0-only

//! Shared section projection and quad emission helpers for section-based road meshes, and the
//! emission of the strips a piece hangs from its outer chains.

use super::super::{
    MeshLayer, NetworkMeshData, concrete_color, lining_color, push_triangle,
    push_triangle_preserving_winding_with_exact_normal, render_point, world_point,
};
use crate::simulation::network::roadbed::{LiningPart, PieceMesh, RoadSection, RoadVec3};
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
        emit_strip_triangle(mesh, layer, color, world, bottoms);
    });
}

// Emits a bridge span's abutment faces (`TERRAIN-07-C`) into the skirt layer as the piece wound
// them, facing the space under the bridge, with the skirts' bottom-vertex attribute: their
// bottoms are the grounded skirts' bottoms below the mouth row's corners and deepen with them.
pub(super) fn emit_abutment_faces(mesh: &mut NetworkMeshData, piece: &PieceMesh, color: Color) {
    piece.for_each_abutment_triangle(|world, bottoms| {
        emit_strip_triangle(mesh, MeshLayer::Skirt, color, world, bottoms);
    });
}

/// The spacing of a tunnel's lamp strips along its bore (`ROAD-61` round R8b), metres; the
/// lining shader's `lamp_spacing_m` must match it.
const TUNNEL_LAMP_SPACING_M: f64 = 10.0;

// Emits a buried piece's lining, a tunnel span's collars and the lining's shell (`ROAD-61`)
// into the lining layer as the piece wound them, facing into the bore, out of the portal and
// away from the bore; the concrete material is drawn on both sides, so the lining also closes
// the bore seen from outside. On a span's walls and ceiling the UVs carry the lamp station and
// the height above the deck (round R8b): the station along the bore scaled so that a whole
// number of lamp spacings fits the bore (at least one), so lamps at the middle of each spacing
// sit symmetric in every span and evenly spaced within 5% of `TUNNEL_LAMP_SPACING_M`. Every
// other face takes `y = -1`: no lamps. The concrete shader is triplanar in world space.
// Round R8d: only the faces inside the bore, the walls, the ceiling and the end walls, take the
// lining's colour (alpha 0: no sun, no sky, the lining's ambient); a collar and the shell face
// the open air (the shell where the hill leaves it bare) and take the outdoor concrete's.
pub(super) fn emit_lining(mesh: &mut NetworkMeshData, piece: &PieceMesh) {
    let scale = piece.lining_bore_m().filter(|&bore| bore > 0.0).map(|bore| {
        (bore / TUNNEL_LAMP_SPACING_M).round().max(1.0) * TUNNEL_LAMP_SPACING_M / bore
    });
    piece.for_each_lining_triangle(|world, part, along| {
        let [a, b, c] = world.map(render_point);
        let uvs = match (part, scale) {
            (LiningPart::Bore, Some(scale)) => {
                along.map(|at| Vector2::new((at.x * scale) as f32, at.y as f32))
            }
            _ => [Vector2::new(0.0, -1.0); 3],
        };
        let color = match part {
            LiningPart::Bore | LiningPart::EndWall => lining_color(),
            LiningPart::Collar | LiningPart::Shell => concrete_color(),
        };
        push_triangle_preserving_winding_with_exact_normal(
            mesh,
            MeshLayer::Lining,
            world,
            uvs,
            color,
            -((b - a).cross(c - a)),
        );
    });
}

// One strip-layer triangle in its own winding, with UV `y` the bottom-vertex attribute.
fn emit_strip_triangle(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    color: Color,
    world: [RoadVec3; 3],
    bottoms: [bool; 3],
) {
    let [a, b, c] = world.map(render_point);
    push_triangle_preserving_winding_with_exact_normal(
        mesh,
        layer,
        world,
        bottoms.map(|bottom| Vector2::new(0.0, f32::from(u8::from(bottom)))),
        color,
        -((b - a).cross(c - a)),
    );
}
