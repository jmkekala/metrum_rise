//! Bridge underside concrete and terrain-supported pier emission.

use super::super::{
    MeshLayer, NetworkMeshData, concrete_color, push_triangle_preserving_winding_with_exact_normal,
};
use super::geometry::{emit_surface_quad, section_boundary_world_point, triangle_is_too_small};
use crate::config;
use crate::simulation::network::surface::RoadSurfaceSection;
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::{Vector2, Vector3};

const BRIDGE_CONCRETE_THICKNESS_M: f32 = 0.35;
const BRIDGE_PIER_GROUND_EMBED_M: f32 = 0.08;
const BRIDGE_PIER_HALF_DEPTH_M: f32 = 0.55;
const BRIDGE_PIER_HALF_WIDTH_M: f32 = 0.55;
const BRIDGE_PIER_MIN_CLEARANCE_M: f32 = 1.0;
const BRIDGE_PIER_SPACING_M: f32 = 28.0;

pub(super) fn emit_compiled_bridge_concrete(
    mesh: &mut NetworkMeshData,
    terrain: &TerrainSystem,
    sections: &[RoadSurfaceSection],
) {
    if sections.len() < 2 {
        return;
    }

    for pair in sections.windows(2) {
        let Some((left_a, right_a)) = outer_surface_bounds(&pair[0]) else {
            continue;
        };
        let Some((left_b, right_b)) = outer_surface_bounds(&pair[1]) else {
            continue;
        };

        let a_left = section_boundary_world_point(
            &pair[0],
            left_a.0,
            left_a.1 - BRIDGE_CONCRETE_THICKNESS_M,
        );
        let a_right = section_boundary_world_point(
            &pair[0],
            right_a.0,
            right_a.1 - BRIDGE_CONCRETE_THICKNESS_M,
        );
        let b_left = section_boundary_world_point(
            &pair[1],
            left_b.0,
            left_b.1 - BRIDGE_CONCRETE_THICKNESS_M,
        );
        let b_right = section_boundary_world_point(
            &pair[1],
            right_b.0,
            right_b.1 - BRIDGE_CONCRETE_THICKNESS_M,
        );
        if triangle_is_too_small(a_left, b_left, b_right)
            && triangle_is_too_small(a_left, b_right, a_right)
        {
            continue;
        }

        emit_surface_quad(
            mesh,
            MeshLayer::Concrete,
            [a_left, b_left, b_right, a_right],
            [
                Vector2::new(pair[0].s_m, 0.0),
                Vector2::new(pair[1].s_m, 0.0),
                Vector2::new(pair[1].s_m, 1.0),
                Vector2::new(pair[0].s_m, 1.0),
            ],
            concrete_color(),
        );
    }

    emit_compiled_bridge_piers(mesh, terrain, sections);
}

fn emit_compiled_bridge_piers(
    mesh: &mut NetworkMeshData,
    terrain: &TerrainSystem,
    sections: &[RoadSurfaceSection],
) {
    let mut last_emitted_s_m = f32::NEG_INFINITY;
    for (section_index, section) in sections.iter().enumerate() {
        let is_endpoint = section_index == 0 || section_index + 1 == sections.len();
        if !is_endpoint && section.s_m - last_emitted_s_m < BRIDGE_PIER_SPACING_M {
            continue;
        }

        let top_y = section.center_height_m - BRIDGE_CONCRETE_THICKNESS_M;
        let base_y = terrain
            .sample_visual_height_world(section.center_xz.x as f32, section.center_xz.y as f32)
            * config::HEIGHT_SCALE
            - BRIDGE_PIER_GROUND_EMBED_M;
        if top_y - base_y < BRIDGE_PIER_MIN_CLEARANCE_M {
            continue;
        }

        emit_bridge_pier(mesh, section, base_y, top_y);
        last_emitted_s_m = section.s_m;
    }
}

fn emit_bridge_pier(
    mesh: &mut NetworkMeshData,
    section: &RoadSurfaceSection,
    base_y: f32,
    top_y: f32,
) {
    let tangent = Vector2::new(section.tangent_xz.x as f32, section.tangent_xz.y as f32);
    let lateral = Vector2::new(section.lateral_xz.x as f32, section.lateral_xz.y as f32);
    if tangent.length_squared() <= 1e-8 || lateral.length_squared() <= 1e-8 {
        return;
    }

    let tangent = tangent.normalized();
    let lateral = lateral.normalized();
    let center = Vector2::new(section.center_xz.x as f32, section.center_xz.y as f32);
    let tangent_offset = tangent * BRIDGE_PIER_HALF_DEPTH_M;
    let lateral_offset = lateral * BRIDGE_PIER_HALF_WIDTH_M;
    let footprint = [
        center - tangent_offset - lateral_offset,
        center + tangent_offset - lateral_offset,
        center + tangent_offset + lateral_offset,
        center - tangent_offset + lateral_offset,
    ];
    let bottom = footprint.map(|point| Vector3::new(point.x, base_y, point.y));
    let top = footprint.map(|point| Vector3::new(point.x, top_y, point.y));
    let tangent_normal = Vector3::new(tangent.x, 0.0, tangent.y);
    let lateral_normal = Vector3::new(lateral.x, 0.0, lateral.y);

    emit_bridge_pier_quad(
        mesh,
        [top[0], top[1], bottom[1], bottom[0]],
        -lateral_normal,
    );
    emit_bridge_pier_quad(mesh, [top[1], top[2], bottom[2], bottom[1]], tangent_normal);
    emit_bridge_pier_quad(mesh, [top[2], top[3], bottom[3], bottom[2]], lateral_normal);
    emit_bridge_pier_quad(
        mesh,
        [top[3], top[0], bottom[0], bottom[3]],
        -tangent_normal,
    );
    emit_bridge_pier_quad(mesh, [top[0], top[1], top[2], top[3]], Vector3::UP);
}

fn emit_bridge_pier_quad(mesh: &mut NetworkMeshData, vertices: [Vector3; 4], normal: Vector3) {
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
        push_triangle_preserving_winding_with_exact_normal(
            mesh,
            MeshLayer::Concrete,
            triangle,
            triangle_uvs,
            concrete_color(),
            normal,
        );
    }
}

fn outer_surface_bounds(section: &RoadSurfaceSection) -> Option<((f32, f32), (f32, f32))> {
    let first_band = section.bands.first()?;
    let last_band = section.bands.last()?;
    Some((
        (first_band.lateral_start_m, first_band.height_start_m),
        (last_band.lateral_end_m, last_band.height_end_m),
    ))
}
