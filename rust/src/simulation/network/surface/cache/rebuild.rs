//! Chunk cache rebuild and stale-entry pruning.

use super::*;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn rebuild_surface_chunk_cache(
        &mut self,
        chunks: &[SurfaceChunkKey],
    ) {
        for &chunk in chunks {
            let edge_indices = Self::sorted_owner_set(self.surface_chunk_spans.get(&chunk));
            let node_ids = Self::sorted_owner_set(self.surface_chunk_nodes.get(&chunk));
            if edge_indices.is_empty() && node_ids.is_empty() {
                self.surface_chunk_cache.remove(&chunk);
                continue;
            }
            self.surface_chunk_cache.insert(
                chunk,
                RoadSurfaceChunkCacheEntry {
                    chunk,
                    edge_indices,
                    node_ids,
                },
            );
        }
    }

    pub(in crate::simulation::network::surface) fn rebuild_earthwork_chunk_cache(
        &mut self,
        chunks: &[SurfaceChunkKey],
    ) {
        for &chunk in chunks {
            let edge_indices = Self::sorted_owner_set(self.earthwork_chunk_spans.get(&chunk));
            let node_ids = Self::sorted_owner_set(self.earthwork_chunk_nodes.get(&chunk));
            if edge_indices.is_empty() && node_ids.is_empty() {
                self.earthwork_chunk_cache.remove(&chunk);
                continue;
            }
            self.earthwork_chunk_cache.insert(
                chunk,
                RoadEarthworkChunkCacheEntry {
                    chunk,
                    edge_indices,
                    node_ids,
                },
            );
        }
    }

    fn sorted_owner_set<T: Copy + Ord>(owners: Option<&BTreeSet<T>>) -> Vec<T> {
        match owners {
            Some(owners) => owners.iter().copied().collect(),
            None => Vec::new(),
        }
    }

    pub(in crate::simulation::network::surface) fn prune_stale_cache_entries(
        &mut self,
        graph: &RegionGraph,
    ) {
        let stale_span_ids: Vec<usize> = self
            .surface_span_chunks
            .keys()
            .chain(self.earthwork_span_chunks.keys())
            .chain(self.compiled_visual_span_pieces.keys())
            .copied()
            .filter(|edge_idx| {
                *edge_idx >= graph.edge_count() || !Self::is_surface_edge(graph.edge(*edge_idx))
            })
            .collect();
        for edge_idx in stale_span_ids {
            self.remove_span_piece_coverage(edge_idx);
            self.compiled_visual_span_pieces.remove(&edge_idx);
            self.compiled_sections.remove(&edge_idx);
        }

        let stale_node_ids: Vec<u32> = self
            .surface_node_chunks
            .keys()
            .chain(self.earthwork_node_chunks.keys())
            .chain(self.compiled_visual_node_pieces.keys())
            .copied()
            .filter(|node_id| (*node_id as usize) >= graph.node_count())
            .collect();
        for node_id in stale_node_ids {
            self.remove_node_piece_coverage(node_id);
            self.compiled_visual_node_pieces.remove(&node_id);
            self.compiled_visual_node_inputs.remove(&node_id);
        }

        self.compiled_sections.retain(|edge_idx, _| {
            *edge_idx < graph.edge_count() && Self::is_surface_edge(graph.edge(*edge_idx))
        });
        self.compiled_visual_span_pieces.retain(|edge_idx, _| {
            *edge_idx < graph.edge_count() && Self::is_surface_edge(graph.edge(*edge_idx))
        });
        self.compiled_visual_node_pieces
            .retain(|node_id, _| (*node_id as usize) < graph.node_count());
        self.compiled_visual_node_inputs
            .retain(|node_id, _| (*node_id as usize) < graph.node_count());
        self.surface_chunk_cache
            .retain(|_, entry| !entry.edge_indices.is_empty() || !entry.node_ids.is_empty());
        self.earthwork_chunk_cache
            .retain(|_, entry| !entry.edge_indices.is_empty() || !entry.node_ids.is_empty());
    }

    pub(in crate::simulation::network::surface) fn collect_all_chunks(
        &self,
        kind: ChunkCacheKind,
    ) -> Vec<SurfaceChunkKey> {
        let mut chunks = HashSet::new();
        match kind {
            ChunkCacheKind::Surface => {
                chunks.extend(self.surface_chunk_spans.keys().copied());
                chunks.extend(self.surface_chunk_nodes.keys().copied());
            }
            ChunkCacheKind::Earthwork => {
                chunks.extend(self.earthwork_chunk_spans.keys().copied());
                chunks.extend(self.earthwork_chunk_nodes.keys().copied());
            }
        }
        self.sorted_chunk_keys(&chunks)
    }
}
