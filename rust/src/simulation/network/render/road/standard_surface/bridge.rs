// SPDX-License-Identifier: GPL-2.0-only

//! Bridge concrete: the deck's fascias, the slab under the deck and the terrain-supported piers.
//!
//! The fascias are the span piece's strips (`TERRAIN-07-B`), from the deck's outer edges down to
//! the slab's edges, and the slab spans between their bottom vertices row by row, so deck,
//! fascias and slab share their vertices bitwise. At a mouth shared with a grounded piece the
//! abutment face hangs from the slab's end edge to the skirts' depth (`TERRAIN-07-C`).

use super::super::{
    MeshLayer, NetworkMeshData, NetworkMeshOwner, concrete_color, curb_color, push_triangle,
    push_triangle_preserving_winding_with_exact_normal, render_point, world_point,
};
use super::coverage::CompiledSurfaceCoverage;
use super::geometry::{emit_abutment_faces, emit_chain_strips, triangle_is_too_small};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::roadbed::{
    BRIDGE_DECK_DEPTH_M, RoadSection, RoadVec3, RoadbedSystem, SpanPiece, StripKind,
};
use crate::simulation::network::types::EdgeClass;
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::terrain::graded::{GradedSurface, QueryScratch, SiteSource};
use godot::prelude::{Vector2, Vector3};

const BRIDGE_PIER_GROUND_EMBED_M: f32 = 0.08;
const BRIDGE_PIER_HALF_DEPTH_M: f32 = 0.55;
const BRIDGE_PIER_HALF_WIDTH_M: f32 = 0.55;
const BRIDGE_PIER_MIN_CLEARANCE_M: f32 = 1.0;
const BRIDGE_PIER_SPACING_M: f32 = 28.0;

/// Emits fascias, the slab and piers for every covered bridge edge that has a span piece.
pub(in crate::simulation::network::render::road) fn emit_bridge_structures(
    mesh: &mut NetworkMeshData,
    graph: &RegionGraph,
    roadbed: &RoadbedSystem,
    terrain: &TerrainSystem,
    sites: &dyn SiteSource,
    coverage: &CompiledSurfaceCoverage,
) {
    let surface = GradedSurface {
        terrain,
        roadbed,
        sites,
        query_index: None,
    };
    let mut scratch = QueryScratch::default();
    for &edge_idx in &coverage.edge_indices {
        if graph.edge(edge_idx).class != EdgeClass::Bridge {
            continue;
        }
        let Some(span) = roadbed.span_piece(edge_idx) else {
            continue;
        };
        mesh.set_owner(NetworkMeshOwner::Edge(edge_idx));
        emit_compiled_bridge_concrete(mesh, &surface, &mut scratch, span);
    }
}

fn emit_compiled_bridge_concrete(
    mesh: &mut NetworkMeshData,
    surface: &GradedSurface<'_>,
    scratch: &mut QueryScratch,
    span: &SpanPiece,
) {
    let sections = span.sections();
    if sections.len() < 2 {
        return;
    }
    let Some(rows) = span.strip_bottom_rows() else {
        return;
    };
    debug_assert_eq!(span.mesh().strip_kind(), Some(StripKind::Fascia));
    emit_chain_strips(mesh, span.mesh(), MeshLayer::Concrete, concrete_color());
    // Below the slab's end edge at a mouth shared with a grounded piece, in the skirt layer.
    emit_abutment_faces(mesh, span.mesh(), curb_color());

    // The slab: one quad per row pair between the fascias' bottom edges.
    let mut previous: Option<(f32, [RoadVec3; 2])> = None;
    for (section, [left, right]) in sections.iter().zip(rows) {
        if let Some((s_m, [left_a, right_a])) = previous {
            let quad = [left_a, left, right, right_a];
            let uvs = [
                Vector2::new(s_m, 0.0),
                Vector2::new(section.s_m, 0.0),
                Vector2::new(section.s_m, 1.0),
                Vector2::new(s_m, 1.0),
            ];
            for corners in [[0, 1, 2], [0, 2, 3]] {
                let triangle = corners.map(|corner| quad[corner]);
                let [a, b, c] = triangle.map(render_point);
                if !triangle_is_too_small(a, b, c) {
                    push_triangle(
                        mesh,
                        MeshLayer::Concrete,
                        triangle,
                        corners.map(|corner| uvs[corner]),
                        concrete_color(),
                    );
                }
            }
        }
        previous = Some((section.s_m, [left, right]));
    }

    emit_compiled_bridge_piers(mesh, surface, scratch, sections);
}

fn emit_compiled_bridge_piers(
    mesh: &mut NetworkMeshData,
    surface: &GradedSurface<'_>,
    scratch: &mut QueryScratch,
    sections: &[RoadSection],
) {
    let mut last_emitted_s_m = f32::NEG_INFINITY;
    for (section_index, section) in sections.iter().enumerate() {
        let is_endpoint = section_index == 0 || section_index + 1 == sections.len();
        if !is_endpoint && section.s_m - last_emitted_s_m < BRIDGE_PIER_SPACING_M {
            continue;
        }

        let top_y = section.center_height_m - BRIDGE_DECK_DEPTH_M as f32;
        let base_y = surface.ground_height_at(section.center_xz.x, section.center_xz.y, scratch)
            as f32
            - BRIDGE_PIER_GROUND_EMBED_M;
        if top_y - base_y < BRIDGE_PIER_MIN_CLEARANCE_M {
            continue;
        }

        emit_bridge_pier(mesh, section, base_y, top_y);
        last_emitted_s_m = section.s_m;
    }
}

fn emit_bridge_pier(mesh: &mut NetworkMeshData, section: &RoadSection, base_y: f32, top_y: f32) {
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
            triangle.map(world_point),
            triangle_uvs,
            concrete_color(),
            normal,
        );
    }
}
