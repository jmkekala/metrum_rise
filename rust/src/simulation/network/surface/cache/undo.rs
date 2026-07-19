//! Bounded road-surface compiler cache capture and graph-undo restoration.

use super::*;

impl RoadSurfaceSystem {
    /// Captures only compiler records owned by the graph records stored for one local edit.
    pub(crate) fn capture_topology_undo(
        &self,
        edge_ids: &HashSet<usize>,
        node_ids: &HashSet<u32>,
    ) -> Option<RoadSurfaceTopologyUndo> {
        if !self.compiled_once || self.has_pending_rebuild_work() {
            return None;
        }

        let mut edge_ids = edge_ids.iter().copied().collect::<Vec<_>>();
        edge_ids.sort_unstable();
        let edges = edge_ids
            .into_iter()
            .map(|edge_idx| RoadSurfaceEdgeTopologyUndo {
                edge_idx,
                sections: self.compiled_sections.get(&edge_idx).cloned(),
                span_piece: self.compiled_visual_span_pieces.get(&edge_idx).cloned(),
            })
            .collect();

        let mut node_ids = node_ids.iter().copied().collect::<Vec<_>>();
        node_ids.sort_unstable();
        let nodes = node_ids
            .into_iter()
            .map(|node_id| RoadSurfaceNodeTopologyUndo {
                node_id,
                piece: self.compiled_visual_node_pieces.get(&node_id).cloned(),
                input: self.compiled_visual_node_inputs.get(&node_id).cloned(),
                earthwork_boundaries: self
                    .compiled_visual_node_earthwork_boundaries
                    .get(&node_id)
                    .cloned(),
                topology: self.compiled_visual_node_topologies.get(&node_id).cloned(),
            })
            .collect();

        Some(RoadSurfaceTopologyUndo {
            chunk_span_m_bits: self.chunk_span_m.to_bits(),
            edges,
            nodes,
        })
    }

    /// Restores a bounded pre-edit compiler checkpoint while dirtying old and new coverage.
    pub(crate) fn restore_topology_undo(
        &mut self,
        undo: RoadSurfaceTopologyUndo,
        affected_edge_ids: &HashSet<usize>,
        affected_node_ids: &HashSet<u32>,
    ) -> bool {
        if !self.topology_undo_is_valid(&undo, affected_edge_ids, affected_node_ids) {
            return false;
        }
        self.note_compile_invalidation();

        let mut affected_edges = affected_edge_ids.iter().copied().collect::<Vec<_>>();
        affected_edges.sort_unstable();
        for edge_idx in affected_edges.iter().copied() {
            self.remove_span_piece_coverage(edge_idx);
            self.compiled_sections.remove(&edge_idx);
            self.compiled_visual_span_pieces.remove(&edge_idx);
        }

        let mut affected_nodes = affected_node_ids.iter().copied().collect::<Vec<_>>();
        affected_nodes.sort_unstable();
        for node_id in affected_nodes.iter().copied() {
            self.remove_node_piece_coverage(node_id);
            self.compiled_visual_node_pieces.remove(&node_id);
            self.compiled_visual_node_inputs.remove(&node_id);
            self.compiled_visual_node_earthwork_boundaries
                .remove(&node_id);
            self.compiled_visual_node_topologies.remove(&node_id);
        }

        let restored_edge_count = undo.edges.len();
        let restored_node_count = undo.nodes.len();
        for edge in undo.edges {
            if let Some(sections) = edge.sections {
                self.compiled_sections.insert(edge.edge_idx, sections);
            }
            if let Some(piece) = edge.span_piece {
                self.insert_span_piece_coverage(&piece);
                self.compiled_visual_span_pieces
                    .insert(edge.edge_idx, piece);
            }
        }
        for node in undo.nodes {
            if let Some(piece) = node.piece {
                self.insert_node_piece_coverage(&piece);
                self.compiled_visual_node_pieces.insert(node.node_id, piece);
            }
            if let Some(input) = node.input {
                self.compiled_visual_node_inputs.insert(node.node_id, input);
            }
            if let Some(boundaries) = node.earthwork_boundaries {
                self.compiled_visual_node_earthwork_boundaries
                    .insert(node.node_id, boundaries);
            }
            if let Some(topology) = node.topology {
                self.compiled_visual_node_topologies
                    .insert(node.node_id, topology);
            }
        }

        for edge_idx in affected_edge_ids {
            self.dirty_edges.remove(edge_idx);
        }
        for node_id in affected_node_ids {
            self.dirty_nodes.remove(node_id);
        }

        crate::debug_log!(
            "road",
            "surface_topology_undo_restore_detail affected_edges={} affected_nodes={} restored_edges={} restored_nodes={} dirty_surface_chunks={} dirty_terrain_chunks={} dirty_query_chunks={}",
            affected_edge_ids.len(),
            affected_node_ids.len(),
            restored_edge_count,
            restored_node_count,
            self.dirty_surface_chunks.len(),
            self.dirty_terrain_chunks.len(),
            self.dirty_query_chunks.len()
        );
        true
    }

    fn topology_undo_is_valid(
        &self,
        undo: &RoadSurfaceTopologyUndo,
        affected_edge_ids: &HashSet<usize>,
        affected_node_ids: &HashSet<u32>,
    ) -> bool {
        if !self.compiled_once || undo.chunk_span_m_bits != self.chunk_span_m.to_bits() {
            return false;
        }

        let mut seen_edges = HashSet::with_capacity(undo.edges.len());
        for edge in &undo.edges {
            if !affected_edge_ids.contains(&edge.edge_idx)
                || !seen_edges.insert(edge.edge_idx)
                || edge
                    .span_piece
                    .as_ref()
                    .is_some_and(|piece| piece.edge_idx != edge.edge_idx)
            {
                return false;
            }
        }

        let mut seen_nodes = HashSet::with_capacity(undo.nodes.len());
        for node in &undo.nodes {
            if !affected_node_ids.contains(&node.node_id)
                || !seen_nodes.insert(node.node_id)
                || node
                    .piece
                    .as_ref()
                    .is_some_and(|piece| piece.node_id != node.node_id)
            {
                return false;
            }
        }
        true
    }
}
