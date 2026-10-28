// SPDX-License-Identifier: GPL-2.0-only

//! Emission of roadbed piece triangles into the network mesh layers.
//!
//! Band kinds map to layers as the legacy top-surface renderer did: carriageway to the road
//! layer with road-space UVs, curb or shoulder to the curb layer, every other band to the
//! sidewalk layer (road-space footpath colour on footpath edges), and curb faces to the
//! raised-step layer. Node pieces use world UVs throughout, and their carriageway keeps
//! `road_color`'s zero alpha: junction asphalt has no road space. The triangles are the pieces'
//! own, so queries answer from what renders. Grounded pieces' skirts (`TERRAIN-07-B`) go to the
//! skirt layer in the curb's colour; a bridge's fascias go with its concrete (`bridge.rs`).

use super::super::{
    MeshLayer, NetworkMeshData, NetworkMeshOwner, curb_color, footpath_color, push_triangle,
    push_triangle_preserving_winding_with_exact_normal, render_point, road_color,
    road_space_color, sidewalk_color,
};
use super::coverage::CompiledSurfaceCoverage;
use super::geometry::{emit_chain_strips, triangle_is_too_small};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::roadbed::{BandKind, PieceMesh, RoadbedSystem, StripKind};
use crate::simulation::network::types::TransitType;
use godot::prelude::{Color, Vector2};

// Bands other than carriageway and curb draw on the sidewalk layer.
const NON_ROAD_KINDS: [BandKind; 6] = [
    BandKind::Sidewalk,
    BandKind::Footpath,
    BandKind::Median,
    BandKind::Parking,
    BandKind::CycleTrack,
    BandKind::TramReservation,
];

#[derive(Clone, Copy)]
enum Uvs {
    // World XZ metres.
    World,
    // Station along the edge and signed lateral offset.
    RoadSpace,
}

pub(in crate::simulation::network::render::road) fn emit_piece_surfaces(
    mesh: &mut NetworkMeshData,
    graph: &RegionGraph,
    roadbed: &RoadbedSystem,
    coverage: &CompiledSurfaceCoverage,
) {
    for &edge_idx in &coverage.edge_indices {
        let Some(span) = roadbed.span_piece(edge_idx) else {
            continue;
        };
        mesh.set_owner(NetworkMeshOwner::Edge(edge_idx));
        let edge = graph.edge(edge_idx);
        let piece = span.mesh();
        emit_band(
            mesh,
            piece,
            BandKind::CurbOrShoulder,
            MeshLayer::Curb,
            Uvs::World,
            curb_color(),
        );
        emit_curb_faces(mesh, piece);
        // Footpaths draw their own path surface (road_path.gdshader) in road space.
        let (uvs, color) = if edge.primary_type == TransitType::Foot {
            (Uvs::RoadSpace, footpath_color(edge))
        } else {
            (Uvs::World, sidewalk_color())
        };
        for kind in NON_ROAD_KINDS {
            emit_band(mesh, piece, kind, MeshLayer::Sidewalk, uvs, color);
        }
        emit_band(
            mesh,
            piece,
            BandKind::Carriageway,
            MeshLayer::Road,
            Uvs::RoadSpace,
            road_space_color(edge),
        );
        emit_skirts(mesh, piece);
    }
    for &node in &coverage.node_ids {
        let Some(piece) = roadbed.node_piece(node) else {
            continue;
        };
        mesh.set_owner(NetworkMeshOwner::Node(node));
        let piece = piece.mesh();
        emit_band(
            mesh,
            piece,
            BandKind::CurbOrShoulder,
            MeshLayer::Curb,
            Uvs::World,
            curb_color(),
        );
        emit_curb_faces(mesh, piece);
        for kind in NON_ROAD_KINDS {
            emit_band(
                mesh,
                piece,
                kind,
                MeshLayer::Sidewalk,
                Uvs::World,
                sidewalk_color(),
            );
        }
        emit_band(
            mesh,
            piece,
            BandKind::Carriageway,
            MeshLayer::Road,
            Uvs::World,
            road_color(),
        );
        emit_skirts(mesh, piece);
    }
}

// Skirts, and a tunnel's free-end cap at the skirt depth (`TERRAIN-07-C`; a bridge's cap goes
// with its fascias). A piece has one or the other.
fn emit_skirts(mesh: &mut NetworkMeshData, piece: &PieceMesh) {
    if piece.strip_kind() == Some(StripKind::Skirt) || piece.cap_kind() == Some(StripKind::Skirt) {
        emit_chain_strips(mesh, piece, MeshLayer::Skirt, curb_color());
    }
}

fn emit_band(
    mesh: &mut NetworkMeshData,
    piece: &PieceMesh,
    kind: BandKind,
    layer: MeshLayer,
    uvs: Uvs,
    color: Color,
) {
    for triangle in piece.band_triangles(kind) {
        let vertices = triangle.map(|vertex| piece.positions()[vertex as usize]);
        let uvs = match uvs {
            Uvs::World => vertices.map(|vertex| Vector2::new(vertex.x as f32, vertex.z as f32)),
            Uvs::RoadSpace => triangle.map(|vertex| {
                let [s_m, lateral_m] = piece.road_space()[vertex as usize];
                Vector2::new(s_m, lateral_m)
            }),
        };
        push_triangle(mesh, layer, vertices, uvs, color);
    }
}

// Vertical faces keep their winding; `-(b - a) x (c - a)` faces the carriageway.
fn emit_curb_faces(mesh: &mut NetworkMeshData, piece: &PieceMesh) {
    for triangle in piece.curb_face_triangles() {
        let world = triangle.map(|vertex| piece.positions()[vertex as usize]);
        let [a, b, c] = world.map(render_point);
        if triangle_is_too_small(a, b, c) {
            continue;
        }
        push_triangle_preserving_winding_with_exact_normal(
            mesh,
            MeshLayer::RaisedStep,
            world,
            [
                Vector2::ZERO,
                Vector2::new(1.0, 0.0),
                Vector2::new(1.0, 1.0),
            ],
            curb_color(),
            -((b - a).cross(c - a)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::RoadRenderer;
    use crate::simulation::network::lanes::LaneSystem;
    use crate::simulation::network::roadbed::fixtures::mixed_profiles;
    use crate::simulation::network::roadbed::{RoadbedSystem, SurfaceChunkKey};
    use crate::simulation::terrain::{TerrainSystem, render_frame_centre, render_frame_local};
    use godot::prelude::Vector3;
    use std::collections::{BTreeMap, BTreeSet};

    type Triangle = [[u32; 3]; 3];

    // Vertex order is not part of the payload contract: the renderer may flip top windings.
    fn key(vertices: [Vector3; 3]) -> Triangle {
        let mut key = vertices.map(|v| [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()]);
        key.sort_unstable();
        key
    }

    // Every piece triangle inside one chunk comes out whole in that chunk's frame, bitwise; one
    // crossing chunk lines comes out clipped at them, covering the same plan area.
    #[test]
    fn chunk_partition_keeps_whole_triangles_and_clips_at_chunk_lines() {
        let graph = mixed_profiles(true);
        let mut roadbed = RoadbedSystem::new_with_chunk_grid(32.0, -7.0, 3.0);
        roadbed.compile_dirty(&graph);
        let targets: BTreeSet<SurfaceChunkKey> = roadbed.occupied_chunks().collect();
        let chunks = RoadRenderer.generate_mesh_chunks_with_surface(
            &graph,
            &mut LaneSystem::new(),
            &TerrainSystem::new(4, 4),
            &crate::simulation::terrain::graded::NoSites,
            &roadbed,
            &targets,
            None,
            &BTreeMap::new(),
        );

        let (span_m, (origin_x, origin_z)) = (roadbed.chunk_span_m(), roadbed.chunk_origin_m());
        let centre = |chunk: SurfaceChunkKey| {
            (
                render_frame_centre(origin_x + chunk.0 as f32 * span_m, span_m),
                render_frame_centre(origin_z + chunk.1 as f32 * span_m, span_m),
            )
        };
        let plan_area = |[a, b, c]: [[f64; 2]; 3]| {
            ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs() * 0.5
        };
        let mut expected: BTreeMap<SurfaceChunkKey, Vec<Triangle>> = BTreeMap::new();
        let mut piece_area = 0.0;
        let mut clipped = 0;
        let spans = (0..graph.edge_count()).map(|edge| roadbed.span_piece(edge).unwrap().mesh());
        let nodes = (0..graph.node_count() as u32)
            .filter_map(|node| roadbed.node_piece(node).map(|piece| piece.mesh()));
        for piece in spans.chain(nodes) {
            for triangle in piece.triangles() {
                let world = triangle.map(|vertex| piece.positions()[vertex as usize]);
                piece_area += plan_area(world.map(|p| [p.x, p.z]));
                let keys = world.map(|p| roadbed.chunk_key_at(p.x, p.z));
                if keys.iter().any(|&key| key != keys[0]) {
                    clipped += 1;
                    continue;
                }
                let (cx, cz) = centre(keys[0]);
                let local = world.map(|p| {
                    Vector3::new(
                        render_frame_local(p.x, cx),
                        p.y as f32,
                        render_frame_local(p.z, cz),
                    )
                });
                expected.entry(keys[0]).or_default().push(key(local));
            }
        }
        let mut emitted_area = 0.0;
        for (&chunk, mesh) in &chunks {
            let (cx, cz) = centre(chunk);
            for layer in [
                &mesh.curb_vertices,
                &mesh.raised_step_vertices,
                &mesh.sidewalk_vertices,
                &mesh.road_vertices,
            ] {
                for triangle in layer.as_chunks::<3>().0 {
                    let triangle_key = key(*triangle);
                    if let Some(whole) = expected.get_mut(&chunk)
                        && let Some(index) = whole.iter().position(|t| *t == triangle_key)
                    {
                        whole.swap_remove(index);
                    }
                    emitted_area += plan_area(triangle.map(|v| {
                        [f64::from(v.x) + f64::from(cx), f64::from(v.z) + f64::from(cz)]
                    }));
                }
            }
        }
        assert!(
            expected.values().all(Vec::is_empty),
            "whole triangles missing from their chunk: {:?}",
            expected
                .iter()
                .filter(|(_, left)| !left.is_empty())
                .map(|(chunk, left)| (chunk, left.len()))
                .collect::<Vec<_>>()
        );
        assert!(
            (emitted_area - piece_area).abs() <= piece_area * 1e-6,
            "plan area {emitted_area} emitted for {piece_area}"
        );
        assert!(
            clipped > 10 && chunks.len() > 10,
            "{clipped} clipped triangles in {} chunks",
            chunks.len()
        );
    }
}
