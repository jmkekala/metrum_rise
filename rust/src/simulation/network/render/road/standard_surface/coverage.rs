// SPDX-License-Identifier: GPL-2.0-only

//! Selection of the edge spans and node pieces visible to the renderer.

use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::roadbed::{PieceId, RoadbedSystem, SurfaceChunkKey};
use crate::simulation::network::types::TransitType;
use std::collections::BTreeSet;

pub(in crate::simulation::network::render::road) struct CompiledSurfaceCoverage {
    pub(in crate::simulation::network::render::road) edge_indices: Vec<usize>,
    pub(in crate::simulation::network::render::road) node_ids: Vec<u32>,
}

// Every published piece.
pub(in crate::simulation::network::render::road) fn build_compiled_surface_coverage(
    graph: &RegionGraph,
    roadbed: &RoadbedSystem,
) -> CompiledSurfaceCoverage {
    let world = [
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::INFINITY,
    ];
    coverage_from_bounds(graph, roadbed, [world])
}

// Pieces touching the requested render chunks; triangles route to their chunks afterwards.
pub(in crate::simulation::network::render::road) fn build_compiled_surface_coverage_for_chunks(
    graph: &RegionGraph,
    roadbed: &RoadbedSystem,
    chunks: &BTreeSet<SurfaceChunkKey>,
) -> CompiledSurfaceCoverage {
    coverage_from_bounds(
        graph,
        roadbed,
        chunks.iter().map(|&chunk| roadbed.chunk_bounds(chunk)),
    )
}

fn coverage_from_bounds(
    graph: &RegionGraph,
    roadbed: &RoadbedSystem,
    bounds: impl IntoIterator<Item = [f64; 4]>,
) -> CompiledSurfaceCoverage {
    let mut edge_indices = BTreeSet::new();
    let mut node_ids = BTreeSet::new();
    for bounds in bounds {
        for (piece, _) in roadbed.pieces_in_bounds(bounds) {
            match piece {
                PieceId::Span(edge_idx) => {
                    if edge_idx < graph.edge_count()
                        && edge_uses_compiled_surface(graph.edge(edge_idx))
                    {
                        edge_indices.insert(edge_idx);
                    }
                }
                PieceId::Node(node_id) => {
                    if node_id < graph.node_count() as u32 {
                        node_ids.insert(graph.get_valid_node(node_id));
                    }
                }
            }
        }
    }
    CompiledSurfaceCoverage {
        edge_indices: edge_indices.into_iter().collect(),
        node_ids: node_ids.into_iter().collect(),
    }
}

fn edge_uses_compiled_surface(edge: &Edge) -> bool {
    !edge.deleted && matches!(edge.primary_type, TransitType::Road | TransitType::Foot)
}
