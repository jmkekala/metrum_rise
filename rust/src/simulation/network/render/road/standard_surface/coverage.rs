// SPDX-License-Identifier: GPL-2.0-only

//! Selection of the edge spans and node pieces visible to the renderer.

use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::render::NetworkMeshOwner;
use crate::simulation::network::roadbed::{PieceId, RoadbedSystem, SurfaceChunkKey};
use crate::simulation::network::types::TransitType;
use std::collections::BTreeSet;

pub(in crate::simulation::network::render::road) struct CompiledSurfaceCoverage {
    pub(in crate::simulation::network::render::road) edge_indices: Vec<usize>,
    pub(in crate::simulation::network::render::road) node_ids: Vec<u32>,
    // The covered pieces built by a compile after the requested roadbed generation (all of
    // them without one): the owners whose lanes must follow their new surface.
    pub(in crate::simulation::network::render::road) rebuilt_edges: Vec<usize>,
    pub(in crate::simulation::network::render::road) rebuilt_nodes: Vec<u32>,
}

impl CompiledSurfaceCoverage {
    // The rebuilt pieces alone, as a coverage to emit.
    pub(in crate::simulation::network::render::road) fn rebuilt(&self) -> Self {
        Self {
            edge_indices: self.rebuilt_edges.clone(),
            node_ids: self.rebuilt_nodes.clone(),
            rebuilt_edges: Vec::new(),
            rebuilt_nodes: Vec::new(),
        }
    }

    // Whether a published chunk keeps `owner`'s triangles: covered and not rebuilt.
    pub(in crate::simulation::network::render::road) fn keeps(&self, owner: NetworkMeshOwner) -> bool {
        match owner {
            NetworkMeshOwner::Edge(edge) => {
                self.edge_indices.binary_search(&edge).is_ok()
                    && self.rebuilt_edges.binary_search(&edge).is_err()
            }
            NetworkMeshOwner::Node(node) => {
                self.node_ids.binary_search(&node).is_ok()
                    && self.rebuilt_nodes.binary_search(&node).is_err()
            }
        }
    }
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
    coverage_from_bounds(graph, roadbed, [world], None)
}

// Pieces touching the requested render chunks; triangles route to their chunks afterwards.
// `built_after` selects the rebuilt owners, as on `CompiledSurfaceCoverage`.
pub(in crate::simulation::network::render::road) fn build_compiled_surface_coverage_for_chunks(
    graph: &RegionGraph,
    roadbed: &RoadbedSystem,
    chunks: &BTreeSet<SurfaceChunkKey>,
    built_after: Option<u64>,
) -> CompiledSurfaceCoverage {
    coverage_from_bounds(
        graph,
        roadbed,
        chunks.iter().map(|&chunk| roadbed.chunk_bounds(chunk)),
        built_after,
    )
}

fn coverage_from_bounds(
    graph: &RegionGraph,
    roadbed: &RoadbedSystem,
    bounds: impl IntoIterator<Item = [f64; 4]>,
    built_after: Option<u64>,
) -> CompiledSurfaceCoverage {
    let mut edge_indices = BTreeSet::new();
    let mut node_ids = BTreeSet::new();
    let mut rebuilt_edges = BTreeSet::new();
    let mut rebuilt_nodes = BTreeSet::new();
    for bounds in bounds {
        for (piece, build_generation) in roadbed.pieces_in_bounds(bounds) {
            let rebuilt = built_after.is_none_or(|after| build_generation > after);
            match piece {
                PieceId::Span(edge_idx) => {
                    if edge_idx < graph.edge_count()
                        && edge_uses_compiled_surface(graph.edge(edge_idx))
                    {
                        edge_indices.insert(edge_idx);
                        if rebuilt {
                            rebuilt_edges.insert(edge_idx);
                        }
                    }
                }
                PieceId::Node(node_id) => {
                    if node_id < graph.node_count() as u32 {
                        let node = graph.get_valid_node(node_id);
                        node_ids.insert(node);
                        if rebuilt {
                            rebuilt_nodes.insert(node);
                        }
                    }
                }
            }
        }
    }
    CompiledSurfaceCoverage {
        edge_indices: edge_indices.into_iter().collect(),
        node_ids: node_ids.into_iter().collect(),
        rebuilt_edges: rebuilt_edges.into_iter().collect(),
        rebuilt_nodes: rebuilt_nodes.into_iter().collect(),
    }
}

fn edge_uses_compiled_surface(edge: &Edge) -> bool {
    !edge.deleted && matches!(edge.primary_type, TransitType::Road | TransitType::Foot)
}
