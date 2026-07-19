//! Piece-to-chunk coverage index maintenance.

use super::*;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn clear_piece_chunk_coverage(&mut self) {
        self.surface_span_chunks.clear();
        self.surface_node_chunks.clear();
        self.earthwork_span_chunks.clear();
        self.earthwork_node_chunks.clear();
        self.surface_chunk_spans.clear();
        self.surface_chunk_nodes.clear();
        self.earthwork_chunk_spans.clear();
        self.earthwork_chunk_nodes.clear();
        self.query_span_chunks.clear();
        self.query_node_chunks.clear();
        self.query_chunk_spans.clear();
        self.query_chunk_nodes.clear();
    }

    pub(in crate::simulation::network::surface) fn remove_span_piece_coverage(
        &mut self,
        edge_idx: usize,
    ) -> (Vec<SurfaceChunkKey>, Vec<SurfaceChunkKey>) {
        let surface_chunks = self
            .surface_span_chunks
            .remove(&edge_idx)
            .unwrap_or_default();
        let terrain_chunks = self
            .earthwork_span_chunks
            .remove(&edge_idx)
            .unwrap_or_default();
        Self::remove_owner_chunk_coverage(&mut self.surface_chunk_spans, edge_idx, &surface_chunks);
        Self::remove_owner_chunk_coverage(
            &mut self.earthwork_chunk_spans,
            edge_idx,
            &terrain_chunks,
        );
        let query_chunks = self.query_span_chunks.remove(&edge_idx).unwrap_or_default();
        Self::remove_owner_chunk_coverage(&mut self.query_chunk_spans, edge_idx, &query_chunks);
        self.extend_dirty_piece_chunks(&surface_chunks, &terrain_chunks, &query_chunks);
        (surface_chunks, terrain_chunks)
    }

    pub(in crate::simulation::network::surface) fn remove_node_piece_coverage(
        &mut self,
        node_id: u32,
    ) -> (Vec<SurfaceChunkKey>, Vec<SurfaceChunkKey>) {
        let surface_chunks = self
            .surface_node_chunks
            .remove(&node_id)
            .unwrap_or_default();
        let terrain_chunks = self
            .earthwork_node_chunks
            .remove(&node_id)
            .unwrap_or_default();
        Self::remove_owner_chunk_coverage(&mut self.surface_chunk_nodes, node_id, &surface_chunks);
        Self::remove_owner_chunk_coverage(
            &mut self.earthwork_chunk_nodes,
            node_id,
            &terrain_chunks,
        );
        let query_chunks = self.query_node_chunks.remove(&node_id).unwrap_or_default();
        Self::remove_owner_chunk_coverage(&mut self.query_chunk_nodes, node_id, &query_chunks);
        self.extend_dirty_piece_chunks(&surface_chunks, &terrain_chunks, &query_chunks);
        (surface_chunks, terrain_chunks)
    }

    pub(in crate::simulation::network::surface) fn insert_span_piece_coverage(
        &mut self,
        piece: &RoadSurfaceVisualSpanPiece,
    ) -> (Vec<SurfaceChunkKey>, Vec<SurfaceChunkKey>) {
        let surface_chunks = Self::canonical_chunk_vec(
            self.visual_span_piece_chunks(piece, ChunkCacheKind::Surface),
        );
        let terrain_chunks = Self::canonical_chunk_vec(
            self.visual_span_piece_chunks(piece, ChunkCacheKind::Earthwork),
        );
        self.surface_span_chunks
            .insert(piece.edge_idx, surface_chunks.clone());
        self.earthwork_span_chunks
            .insert(piece.edge_idx, terrain_chunks.clone());
        Self::insert_owner_chunk_coverage(
            &mut self.surface_chunk_spans,
            piece.edge_idx,
            &surface_chunks,
        );
        Self::insert_owner_chunk_coverage(
            &mut self.earthwork_chunk_spans,
            piece.edge_idx,
            &terrain_chunks,
        );
        let query_chunks = Self::canonical_chunk_vec(self.visual_span_piece_query_chunks(piece));
        self.query_span_chunks
            .insert(piece.edge_idx, query_chunks.clone());
        Self::insert_owner_chunk_coverage(
            &mut self.query_chunk_spans,
            piece.edge_idx,
            &query_chunks,
        );
        self.extend_dirty_piece_chunks(&surface_chunks, &terrain_chunks, &query_chunks);
        (surface_chunks, terrain_chunks)
    }

    pub(in crate::simulation::network::surface) fn insert_node_piece_coverage(
        &mut self,
        piece: &RoadSurfaceVisualNodePiece,
    ) -> (Vec<SurfaceChunkKey>, Vec<SurfaceChunkKey>) {
        let surface_chunks = Self::canonical_chunk_vec(
            self.visual_node_piece_chunks(piece, ChunkCacheKind::Surface),
        );
        let terrain_chunks = Self::canonical_chunk_vec(
            self.visual_node_piece_chunks(piece, ChunkCacheKind::Earthwork),
        );
        self.surface_node_chunks
            .insert(piece.node_id, surface_chunks.clone());
        self.earthwork_node_chunks
            .insert(piece.node_id, terrain_chunks.clone());
        Self::insert_owner_chunk_coverage(
            &mut self.surface_chunk_nodes,
            piece.node_id,
            &surface_chunks,
        );
        Self::insert_owner_chunk_coverage(
            &mut self.earthwork_chunk_nodes,
            piece.node_id,
            &terrain_chunks,
        );
        let query_chunks = Self::canonical_chunk_vec(self.visual_node_piece_query_chunks(piece));
        self.query_node_chunks
            .insert(piece.node_id, query_chunks.clone());
        Self::insert_owner_chunk_coverage(
            &mut self.query_chunk_nodes,
            piece.node_id,
            &query_chunks,
        );
        self.extend_dirty_piece_chunks(&surface_chunks, &terrain_chunks, &query_chunks);
        (surface_chunks, terrain_chunks)
    }

    fn extend_dirty_piece_chunks(
        &mut self,
        surface_chunks: &[SurfaceChunkKey],
        terrain_chunks: &[SurfaceChunkKey],
        query_chunks: &[SurfaceChunkKey],
    ) {
        self.dirty_surface_chunks
            .extend(surface_chunks.iter().copied());
        self.dirty_terrain_chunks
            .extend(terrain_chunks.iter().copied());
        self.dirty_query_chunks.extend(query_chunks.iter().copied());
    }

    fn insert_owner_chunk_coverage<T: Copy + Ord>(
        chunk_owners: &mut HashMap<SurfaceChunkKey, BTreeSet<T>>,
        owner: T,
        chunks: &[SurfaceChunkKey],
    ) {
        for &chunk in chunks {
            chunk_owners.entry(chunk).or_default().insert(owner);
        }
    }

    fn remove_owner_chunk_coverage<T: Copy + Ord>(
        chunk_owners: &mut HashMap<SurfaceChunkKey, BTreeSet<T>>,
        owner: T,
        chunks: &[SurfaceChunkKey],
    ) {
        for &chunk in chunks {
            let Some(owners) = chunk_owners.get_mut(&chunk) else {
                continue;
            };
            owners.remove(&owner);
            let remove_chunk = owners.is_empty();
            if remove_chunk {
                chunk_owners.remove(&chunk);
            }
        }
    }

    fn visual_span_piece_chunks(
        &self,
        piece: &RoadSurfaceVisualSpanPiece,
        kind: ChunkCacheKind,
    ) -> Vec<SurfaceChunkKey> {
        self.visual_span_piece_bounds(piece, kind)
            .map(|(min, max)| self.bounds_to_chunk_keys(min, max))
            .unwrap_or_default()
    }

    fn visual_node_piece_chunks(
        &self,
        piece: &RoadSurfaceVisualNodePiece,
        kind: ChunkCacheKind,
    ) -> Vec<SurfaceChunkKey> {
        self.visual_node_piece_bounds(piece, kind)
            .map(|(min, max)| self.bounds_to_chunk_keys(min, max))
            .unwrap_or_default()
    }

    fn visual_span_piece_query_chunks(
        &self,
        piece: &RoadSurfaceVisualSpanPiece,
    ) -> Vec<SurfaceChunkKey> {
        let mut chunks = Vec::new();
        for kind in [ChunkCacheKind::Surface, ChunkCacheKind::Earthwork] {
            if let Some((min, max)) = self.visual_span_piece_bounds(piece, kind) {
                chunks.extend(Self::bounds_to_query_chunk_keys(min, max));
            }
        }
        chunks
    }

    fn visual_node_piece_query_chunks(
        &self,
        piece: &RoadSurfaceVisualNodePiece,
    ) -> Vec<SurfaceChunkKey> {
        let mut chunks = Vec::new();
        for kind in [ChunkCacheKind::Surface, ChunkCacheKind::Earthwork] {
            if let Some((min, max)) = self.visual_node_piece_bounds(piece, kind) {
                chunks.extend(Self::bounds_to_query_chunk_keys(min, max));
            }
        }
        chunks
    }

    fn canonical_chunk_vec(mut chunks: Vec<SurfaceChunkKey>) -> Vec<SurfaceChunkKey> {
        chunks.sort_unstable();
        chunks.dedup();
        chunks
    }
}
