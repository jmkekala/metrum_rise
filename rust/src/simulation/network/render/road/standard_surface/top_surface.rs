// SPDX-License-Identifier: GPL-2.0-only

//! Emission of roadbed piece triangles into the network mesh layers.
//!
//! Band kinds map to layers as the legacy top-surface renderer did: carriageway to the road
//! layer with road-space UVs, curb or shoulder to the curb layer, every other band to the
//! sidewalk layer (road-space footpath colour on footpath edges), and curb faces to the
//! raised-step layer. The triangles are the pieces' own, so queries answer from what renders.

use super::super::{
    MeshLayer, NetworkMeshData, NetworkMeshOwner, curb_color, footpath_color, push_triangle,
    push_triangle_preserving_winding_with_exact_normal, road_space_color, sidewalk_color,
};
use super::coverage::CompiledSurfaceCoverage;
use super::geometry::triangle_is_too_small;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::roadbed::{BandKind, PieceMesh, RoadVec3, RoadbedSystem};
use crate::simulation::network::types::TransitType;
use godot::prelude::{Color, Vector2, Vector3};

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
    }
}

fn render_point(point: RoadVec3) -> Vector3 {
    Vector3::new(point.x as f32, point.y as f32, point.z as f32)
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
        let vertices = triangle.map(|vertex| render_point(piece.positions()[vertex as usize]));
        let uvs = match uvs {
            Uvs::World => vertices.map(|vertex| Vector2::new(vertex.x, vertex.z)),
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
        let [a, b, c] = triangle.map(|vertex| render_point(piece.positions()[vertex as usize]));
        if triangle_is_too_small(a, b, c) {
            continue;
        }
        push_triangle_preserving_winding_with_exact_normal(
            mesh,
            MeshLayer::RaisedStep,
            [a, b, c],
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
    use crate::simulation::terrain::TerrainSystem;
    use godot::prelude::Vector3;
    use std::collections::{BTreeMap, BTreeSet};

    type Triangle = [[u32; 3]; 3];

    // Vertex order is not part of the payload contract: the renderer may flip top windings.
    fn key(vertices: [Vector3; 3]) -> Triangle {
        let mut key = vertices.map(|v| [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()]);
        key.sort_unstable();
        key
    }

    #[test]
    fn chunk_partition_preserves_the_piece_triangle_multiset() {
        let graph = mixed_profiles(true);
        let mut roadbed = RoadbedSystem::new_with_chunk_grid(32.0, -7.0, 3.0);
        roadbed.compile_dirty(&graph);
        let targets: BTreeSet<SurfaceChunkKey> = roadbed.occupied_chunks().collect();
        let chunks = RoadRenderer.generate_mesh_chunks_with_surface(
            &graph,
            &mut LaneSystem::new(),
            &TerrainSystem::new(4, 4),
            &roadbed,
            &targets,
        );

        let (span_m, (origin_x, origin_z)) = (roadbed.chunk_span_m(), roadbed.chunk_origin_m());
        let mut expected: BTreeMap<SurfaceChunkKey, Vec<Triangle>> = BTreeMap::new();
        for edge in 0..graph.edge_count() {
            let piece = roadbed.span_piece(edge).unwrap().mesh();
            for triangle in piece.triangles() {
                let world = triangle.map(|vertex| {
                    let p = piece.positions()[vertex as usize];
                    Vector3::new(p.x as f32, p.y as f32, p.z as f32)
                });
                let centroid = |axis: fn(&Vector3) -> f32| {
                    world.iter().map(|v| f64::from(axis(v))).sum::<f64>() / 3.0
                };
                let chunk = roadbed.chunk_key_at(centroid(|v| v.x), centroid(|v| v.z));
                assert!(targets.contains(&chunk), "home chunk {chunk:?} is occupied");
                let (cx, cz) = (
                    origin_x + chunk.0 as f32 * span_m,
                    origin_z + chunk.1 as f32 * span_m,
                );
                let local = world.map(|v| Vector3::new(v.x - cx, v.y, v.z - cz));
                expected.entry(chunk).or_default().push(key(local));
            }
        }
        let mut actual: BTreeMap<SurfaceChunkKey, Vec<Triangle>> = BTreeMap::new();
        for (&chunk, mesh) in &chunks {
            for layer in [
                &mesh.curb_vertices,
                &mesh.raised_step_vertices,
                &mesh.sidewalk_vertices,
                &mesh.road_vertices,
            ] {
                for triangle in layer.as_chunks::<3>().0 {
                    actual.entry(chunk).or_default().push(key(*triangle));
                }
            }
        }
        for triangles in expected.values_mut().chain(actual.values_mut()) {
            triangles.sort_unstable();
        }
        assert_eq!(actual, expected);
        let total: usize = expected.values().map(Vec::len).sum();
        assert!(
            total > 1000 && expected.len() > 10,
            "{total} triangles in {} chunks",
            expected.len()
        );
    }
}
