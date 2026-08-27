//! Selection of compiled edge spans and node pieces visible to the renderer.

use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::surface::RoadSurfaceSystem;
use crate::simulation::network::types::TransitType;
use crate::simulation::terrain::TerrainSystem;
use std::collections::BTreeSet;

pub(in crate::simulation::network::render::road) struct CompiledSurfaceCoverage {
    pub(in crate::simulation::network::render::road) edge_indices: Vec<usize>,
    pub(in crate::simulation::network::render::road) node_ids: Vec<u32>,
}

pub(in crate::simulation::network::render::road) fn build_compiled_surface_coverage(
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
    terrain: &TerrainSystem,
) -> CompiledSurfaceCoverage {
    let mut edge_indices = road_surface
        .compiled_visual_span_pieces()
        .keys()
        .copied()
        .filter(|&edge_idx| {
            edge_idx < graph.edge_count() && edge_uses_compiled_surface(graph.edge(edge_idx))
        })
        .collect::<Vec<_>>();
    edge_indices.sort_unstable();

    let mut node_ids = road_surface
        .compiled_visual_node_pieces()
        .keys()
        .copied()
        .filter(|&node_id| node_id < graph.node_count() as u32)
        .map(|node_id| graph.get_valid_node(node_id))
        .filter(|&node_id| road_surface.node_uses_visible_surface(graph, terrain, node_id))
        .collect::<Vec<_>>();
    node_ids.sort_unstable();
    node_ids.dedup();

    node_ids.retain(|node_id| {
        *node_id as usize >= graph.node_adjacency_count()
            || graph
                .node_adjacency(*node_id)
                .iter()
                .any(|edge_idx| edge_indices.binary_search(edge_idx).is_ok())
    });

    CompiledSurfaceCoverage {
        edge_indices,
        node_ids,
    }
}

pub(in crate::simulation::network::render::road) fn build_compiled_surface_coverage_for_chunks(
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
    terrain: &TerrainSystem,
    chunks: &BTreeSet<crate::simulation::network::surface::SurfaceChunkKey>,
) -> CompiledSurfaceCoverage {
    let mut edge_indices = BTreeSet::new();
    let mut node_ids = BTreeSet::new();
    for chunk in chunks {
        if let Some(entry) = road_surface.surface_chunk_cache().get(chunk) {
            edge_indices.extend(entry.edge_indices.iter().copied());
            node_ids.extend(entry.node_ids.iter().copied());
        }
        if let Some(entry) = road_surface.earthwork_chunk_cache().get(chunk) {
            edge_indices.extend(entry.edge_indices.iter().copied());
            node_ids.extend(entry.node_ids.iter().copied());
        }
    }

    let edge_indices = edge_indices
        .into_iter()
        .filter(|&edge_idx| {
            edge_idx < graph.edge_count() && edge_uses_compiled_surface(graph.edge(edge_idx))
        })
        .collect::<Vec<_>>();
    let mut node_ids = node_ids
        .into_iter()
        .filter(|&node_id| node_id < graph.node_count() as u32)
        .map(|node_id| graph.get_valid_node(node_id))
        .filter(|&node_id| road_surface.node_uses_visible_surface(graph, terrain, node_id))
        .collect::<Vec<_>>();
    node_ids.sort_unstable();
    node_ids.dedup();

    CompiledSurfaceCoverage {
        edge_indices,
        node_ids,
    }
}

fn edge_uses_compiled_surface(edge: &Edge) -> bool {
    !edge.deleted && matches!(edge.primary_type, TransitType::Road | TransitType::Foot)
}
