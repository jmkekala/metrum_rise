// SPDX-License-Identifier: GPL-2.0-only

//! Bounded, nonmutating surface compilation for road-node reservation checks.

use super::*;

impl RoadSurfaceSystem {
    /// Compiles the moved node's roads and junctions without changing live graph or caches.
    /// An exact span conflict returns an intentionally partial surface sufficient to reject
    /// the move; otherwise required junctions are compiled before the caller checks their shapes.
    /// Reuses preview's fixed incidence halo; O(K log K + P + local surface compilation)
    /// for copied local edges K and their profile points P, independent of distant roads.
    pub(crate) fn compile_node_move_surface(
        &self,
        source: &RegionGraph,
        terrain: &TerrainSystem,
        zoning: &ZoningSystem,
        node: u32,
        position: Vector3,
    ) -> Option<Self> {
        if node as usize >= source.node_count() || !position.is_finite() {
            return None;
        }
        let node = source.get_valid_node(node);
        let mut graph = RegionGraph::new();
        let mut nodes = HashMap::new();
        let mut edges = HashMap::new();
        let mut incident = source.node_adjacency(node).to_vec();
        incident.sort_unstable();
        incident.dedup();
        for edge in incident {
            Self::copy_validation_graph_edge(
                &mut graph, source, &mut nodes, &mut edges, edge, None,
            );
        }
        let mut surface = Self::new_with_chunk_grid(
            self.chunk_span_m,
            self.chunk_origin_x_m,
            self.chunk_origin_z_m,
        );
        if edges.is_empty() {
            return Some(surface);
        }
        let moved_node = *nodes.get(&node)?;
        let mut required_edges: Vec<_> = edges.values().copied().collect();
        required_edges.sort_unstable();
        let mut required_nodes: Vec<_> = nodes.values().copied().collect();
        required_nodes.sort_unstable();
        Self::complete_validation_surface_context(
            &mut graph,
            source,
            &mut nodes,
            &mut edges,
            &required_edges,
            &required_nodes,
            None,
        );
        graph.move_node(moved_node, position);
        graph.rebuild_intersection_clips_for_nodes(&required_nodes.iter().copied().collect());

        surface.node_validation_logging_enabled = false;
        surface.retain_partial_validation_artifacts = true;
        // Compile the actual span polygons first. A conflict already proves rejection, so
        // neither terminal nor junction solving can change that outcome. No corridor proxy.
        for &edge in &required_edges {
            let sections = surface.compile_edge_sections(&graph, edge);
            surface.compiled_sections.insert(edge, Arc::new(sections));
            let piece = surface.compile_visual_span_piece(&graph, terrain, edge)?;
            let overlaps = piece
                .surface_polygons()
                .any(|quad| zoning.cells_overlap_road_points_world(quad.points()));
            surface
                .compiled_visual_span_pieces
                .insert(edge, Arc::new(piece));
            if overlaps {
                return Some(surface);
            }
        }
        // This certificate visits only the already-compiled local spans. Carry them into the
        // full solve and offer bounded live-node candidates; exact fresh inputs gate each reuse.
        if let Some(mut reuse) = surface.preview_topology_reuse(&graph, terrain, &[]) {
            let mut source_nodes: Vec<_> = nodes.keys().copied().collect();
            source_nodes.sort_unstable();
            reuse.include_live_nodes(self, source, &source_nodes);
            surface.enqueue_preview_topology_reuse(reuse);
        }
        for (source, local) in nodes {
            if let Some(topology) = self.compiled_visual_node_topologies.get(&source) {
                surface
                    .compiled_visual_node_topologies
                    .insert(local, Arc::clone(topology));
            }
        }
        surface.compile_validation_neighborhood_with_reason(
            &graph,
            terrain,
            &required_edges,
            &required_nodes,
            RoadSurfaceCompileReason::CommitValidator,
        );
        // The compiler's failure ledger distinguishes a missing required node piece from a
        // bridge/tunnel terminal intentionally owned by its span, or a pass-through node.
        if Self::explicit_surface_validation_failure(&surface, &required_edges, &required_nodes)
            .is_some()
            || required_edges
                .iter()
                .any(|id| !surface.compiled_visual_span_pieces.contains_key(id))
        {
            return None;
        }
        Some(surface)
    }
}
