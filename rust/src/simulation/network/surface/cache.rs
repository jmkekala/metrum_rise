//! Dirty tracking, chunk ownership, and cache rebuild helpers for road-surface pieces.

use super::{
    RoadSurfaceSystem, RoadSurfaceVisualNodePiece, RoadSurfaceVisualSpanPiece, SurfaceChunkKey,
};
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{EdgeClass, TransitType};
use godot::prelude::{Vector2, Vector3};
use std::collections::{BTreeSet, HashMap, HashSet};

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum ChunkCacheKind {
    Surface,
    Earthwork,
}

/// Cached render-side surface ownership for one chunk.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RoadSurfaceChunkCacheEntry {
    /// Owning chunk key.
    pub chunk: SurfaceChunkKey,
    /// Surface edges contributing cached geometry to this chunk.
    pub edge_indices: Vec<usize>,
    /// Surface nodes contributing cached patches to this chunk.
    pub node_ids: Vec<u32>,
}

/// Cached terrain-earthwork ownership for one chunk.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RoadEarthworkChunkCacheEntry {
    /// Owning chunk key.
    pub chunk: SurfaceChunkKey,
    /// Surface edges contributing earthworks to this chunk.
    pub edge_indices: Vec<usize>,
    /// Surface nodes contributing earthworks to this chunk.
    pub node_ids: Vec<u32>,
}

impl RoadSurfaceSystem {
    /// Clears compiled caches and dirty tracking without changing the configured chunk span.
    pub fn clear(&mut self) {
        self.clear_dirty_tracking();
        self.compiled_sections.clear();
        self.compiled_visual_span_pieces.clear();
        self.compiled_visual_node_pieces.clear();
        self.clear_piece_chunk_coverage();
        self.surface_chunk_cache.clear();
        self.earthwork_chunk_cache.clear();
        self.last_rebuilt_surface_chunks.clear();
        self.last_rebuilt_terrain_chunks.clear();
        self.compiled_once = false;
    }

    /// Clears only the dirty tracking sets.
    pub fn clear_dirty_tracking(&mut self) {
        self.dirty_edges.clear();
        self.dirty_nodes.clear();
        self.dirty_surface_chunks.clear();
        self.dirty_terrain_chunks.clear();
    }

    /// Reconfigures the chunk span and clears all caches and dirty sets.
    pub fn set_chunk_span_m(&mut self, chunk_span_m: f32) {
        self.chunk_span_m = chunk_span_m.max(f32::EPSILON);
        self.clear();
    }

    /// Marks one world-space point as dirty for both surface and terrain chunk rebuilds.
    pub fn mark_world_point_dirty(&mut self, pos: Vector3) {
        let chunk = self.chunk_coords_for_world(pos.x, pos.z);
        self.dirty_surface_chunks.insert(chunk);
        self.dirty_terrain_chunks.insert(chunk);
    }

    /// Marks one world-space AABB as dirty for both surface and terrain chunk rebuilds.
    pub fn mark_world_aabb_dirty(&mut self, min: Vector3, max: Vector3) {
        let min_chunk = self.chunk_coords_for_world(min.x.min(max.x), min.z.min(max.z));
        let max_chunk = self.chunk_coords_for_world(min.x.max(max.x), min.z.max(max.z));
        for cx in min_chunk.0..=max_chunk.0 {
            for cz in min_chunk.1..=max_chunk.1 {
                let chunk = (cx, cz);
                self.dirty_surface_chunks.insert(chunk);
                self.dirty_terrain_chunks.insert(chunk);
            }
        }
    }

    /// Marks one edge dirty; chunk invalidation is derived from compiled piece coverage.
    pub fn mark_edge_dirty(&mut self, graph: &RegionGraph, edge_idx: usize) {
        if edge_idx >= graph.edge_count() {
            return;
        }
        self.dirty_edges.insert(edge_idx);
    }

    /// Marks one node dirty; chunk invalidation is derived from compiled piece coverage.
    pub fn mark_node_dirty(&mut self, graph: &RegionGraph, node_id: u32) {
        if node_id as usize >= graph.node_count() {
            return;
        }
        let valid = graph.get_valid_node(node_id);
        self.dirty_nodes.insert(valid);
    }

    pub(super) fn clear_piece_chunk_coverage(&mut self) {
        self.surface_span_chunks.clear();
        self.surface_node_chunks.clear();
        self.earthwork_span_chunks.clear();
        self.earthwork_node_chunks.clear();
        self.surface_chunk_spans.clear();
        self.surface_chunk_nodes.clear();
        self.earthwork_chunk_spans.clear();
        self.earthwork_chunk_nodes.clear();
    }

    pub(super) fn remove_span_piece_coverage(
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
        self.extend_dirty_piece_chunks(&surface_chunks, &terrain_chunks);
        (surface_chunks, terrain_chunks)
    }

    pub(super) fn remove_node_piece_coverage(
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
        self.extend_dirty_piece_chunks(&surface_chunks, &terrain_chunks);
        (surface_chunks, terrain_chunks)
    }

    pub(super) fn insert_span_piece_coverage(
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
        self.extend_dirty_piece_chunks(&surface_chunks, &terrain_chunks);
        (surface_chunks, terrain_chunks)
    }

    pub(super) fn insert_node_piece_coverage(
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
        self.extend_dirty_piece_chunks(&surface_chunks, &terrain_chunks);
        (surface_chunks, terrain_chunks)
    }

    fn extend_dirty_piece_chunks(
        &mut self,
        surface_chunks: &[SurfaceChunkKey],
        terrain_chunks: &[SurfaceChunkKey],
    ) {
        self.dirty_surface_chunks
            .extend(surface_chunks.iter().copied());
        self.dirty_terrain_chunks
            .extend(terrain_chunks.iter().copied());
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

    /// Marks a terrain edit dirty using the brush center and radius in world metres.
    ///
    /// This marks both touched terrain chunks and any nearby road edges / nodes whose compiled
    /// roadbed may need recompilation when terrain-dependent grades are rebuilt.
    pub fn mark_terrain_edit_dirty(&mut self, graph: &RegionGraph, center: Vector2, radius_m: f32) {
        let radius_m = radius_m.max(0.0);
        let min = Vector3::new(center.x - radius_m, 0.0, center.y - radius_m);
        let max = Vector3::new(center.x + radius_m, 0.0, center.y + radius_m);
        self.mark_world_aabb_dirty(min, max);

        for edge_idx in graph.get_edges_near_aabb(min, max) {
            self.mark_edge_dirty(graph, edge_idx);
            let edge = graph.edge(edge_idx);
            self.mark_node_dirty(graph, edge.start_node);
            self.mark_node_dirty(graph, edge.end_node);
        }
    }
    pub(super) fn rebuild_surface_chunk_cache(&mut self, chunks: &[SurfaceChunkKey]) {
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

    pub(super) fn rebuild_earthwork_chunk_cache(&mut self, chunks: &[SurfaceChunkKey]) {
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

    pub(super) fn prune_stale_cache_entries(&mut self, graph: &RegionGraph) {
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
        }

        self.compiled_sections.retain(|edge_idx, _| {
            *edge_idx < graph.edge_count() && Self::is_surface_edge(graph.edge(*edge_idx))
        });
        self.compiled_visual_span_pieces.retain(|edge_idx, _| {
            *edge_idx < graph.edge_count() && Self::is_surface_edge(graph.edge(*edge_idx))
        });
        self.compiled_visual_node_pieces
            .retain(|node_id, _| (*node_id as usize) < graph.node_count());
        self.surface_chunk_cache
            .retain(|_, entry| !entry.edge_indices.is_empty() || !entry.node_ids.is_empty());
        self.earthwork_chunk_cache
            .retain(|_, entry| !entry.edge_indices.is_empty() || !entry.node_ids.is_empty());
    }

    pub(super) fn all_surface_edge_ids(&self, graph: &RegionGraph) -> Vec<usize> {
        graph
            .edges()
            .iter()
            .enumerate()
            .filter_map(|(edge_idx, edge)| Self::is_surface_edge(edge).then_some(edge_idx))
            .collect()
    }

    pub(super) fn all_surface_node_ids(&self, graph: &RegionGraph) -> Vec<u32> {
        let mut node_ids = HashSet::new();
        for edge in graph.edges() {
            if !Self::is_surface_edge(edge) {
                continue;
            }
            node_ids.insert(graph.get_valid_node(edge.start_node));
            node_ids.insert(graph.get_valid_node(edge.end_node));
        }
        let mut node_ids: Vec<u32> = node_ids.into_iter().collect();
        node_ids.sort_unstable();
        node_ids
    }

    pub(super) fn collect_all_chunks(&self, kind: ChunkCacheKind) -> Vec<SurfaceChunkKey> {
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

    pub(super) fn visual_node_piece_bounds(
        &self,
        piece: &RoadSurfaceVisualNodePiece,
        kind: ChunkCacheKind,
    ) -> Option<(Vector3, Vector3)> {
        let mut min_x = f32::MAX;
        let mut max_x = f32::MIN;
        let mut min_z = f32::MAX;
        let mut max_z = f32::MIN;
        let mut saw_point = false;

        match kind {
            ChunkCacheKind::Surface => {
                for point in piece
                    .outer_boundary_loops
                    .iter()
                    .flat_map(|polygon| polygon.points_world.iter())
                {
                    min_x = min_x.min(point.x);
                    max_x = max_x.max(point.x);
                    min_z = min_z.min(point.z);
                    max_z = max_z.max(point.z);
                    saw_point = true;
                }
            }
            ChunkCacheKind::Earthwork => {
                for point in piece
                    .earthwork_outer_boundary_loops
                    .iter()
                    .flat_map(|polygon| polygon.points_world.iter())
                {
                    min_x = min_x.min(point.x);
                    max_x = max_x.max(point.x);
                    min_z = min_z.min(point.z);
                    max_z = max_z.max(point.z);
                    saw_point = true;
                }
                if !saw_point {
                    for point in piece
                        .earthwork_surface_polygons
                        .iter()
                        .flat_map(|polygon| polygon.points_world.iter())
                    {
                        min_x = min_x.min(point.x);
                        max_x = max_x.max(point.x);
                        min_z = min_z.min(point.z);
                        max_z = max_z.max(point.z);
                        saw_point = true;
                    }
                }
            }
        }

        saw_point.then_some((
            Vector3::new(min_x, 0.0, min_z),
            Vector3::new(max_x, 0.0, max_z),
        ))
    }

    pub(super) fn visual_span_piece_bounds(
        &self,
        piece: &RoadSurfaceVisualSpanPiece,
        kind: ChunkCacheKind,
    ) -> Option<(Vector3, Vector3)> {
        let mut min_x = f32::MAX;
        let mut max_x = f32::MIN;
        let mut min_z = f32::MAX;
        let mut max_z = f32::MIN;
        let mut saw_point = false;

        match kind {
            ChunkCacheKind::Surface => {
                for point in piece
                    .outer_boundary_loops
                    .iter()
                    .flat_map(|polygon| polygon.points_world.iter())
                {
                    min_x = min_x.min(point.x);
                    max_x = max_x.max(point.x);
                    min_z = min_z.min(point.z);
                    max_z = max_z.max(point.z);
                    saw_point = true;
                }
            }
            ChunkCacheKind::Earthwork => {
                for point in piece
                    .earthwork_outer_boundary_loops
                    .iter()
                    .flat_map(|polygon| polygon.points_world.iter())
                {
                    min_x = min_x.min(point.x);
                    max_x = max_x.max(point.x);
                    min_z = min_z.min(point.z);
                    max_z = max_z.max(point.z);
                    saw_point = true;
                }
                if !saw_point {
                    for point in piece
                        .earthwork_surface_polygons
                        .iter()
                        .flat_map(|polygon| polygon.points_world.iter())
                    {
                        min_x = min_x.min(point.x);
                        max_x = max_x.max(point.x);
                        min_z = min_z.min(point.z);
                        max_z = max_z.max(point.z);
                        saw_point = true;
                    }
                }
            }
        }

        saw_point.then_some((
            Vector3::new(min_x, 0.0, min_z),
            Vector3::new(max_x, 0.0, max_z),
        ))
    }

    pub(super) fn sorted_chunk_keys(
        &self,
        chunks: &HashSet<SurfaceChunkKey>,
    ) -> Vec<SurfaceChunkKey> {
        let mut chunks: Vec<SurfaceChunkKey> = chunks.iter().copied().collect();
        chunks.sort_unstable();
        chunks
    }

    fn canonical_chunk_vec(mut chunks: Vec<SurfaceChunkKey>) -> Vec<SurfaceChunkKey> {
        chunks.sort_unstable();
        chunks.dedup();
        chunks
    }

    pub(super) fn node_has_surface_edges(&self, graph: &RegionGraph, node_id: u32) -> bool {
        (node_id as usize) < graph.node_adjacency_count()
            && graph.node_adjacency(node_id).iter().any(|&edge_idx| {
                edge_idx < graph.edge_count() && Self::is_surface_edge(graph.edge(edge_idx))
            })
    }

    pub(super) fn node_has_standard_surface_edges(
        &self,
        graph: &RegionGraph,
        node_id: u32,
    ) -> bool {
        (node_id as usize) < graph.node_adjacency_count()
            && graph.node_adjacency(node_id).iter().any(|&edge_idx| {
                if edge_idx >= graph.edge_count() {
                    return false;
                }
                let edge = graph.edge(edge_idx);
                Self::is_surface_edge(edge) && edge.class == EdgeClass::Standard
            })
    }

    pub(super) fn is_surface_edge(edge: &Edge) -> bool {
        !edge.deleted && matches!(edge.primary_type, TransitType::Road | TransitType::Foot)
    }

    pub(super) fn edge_points<'a>(&self, edge: &'a Edge) -> &'a [Vector3] {
        if edge.physical_geometry.is_empty() {
            &edge.geometry
        } else {
            &edge.physical_geometry
        }
    }

    pub(super) fn chunk_coords_for_world(&self, world_x: f32, world_z: f32) -> SurfaceChunkKey {
        (
            (world_x / self.chunk_span_m).floor() as i32,
            (world_z / self.chunk_span_m).floor() as i32,
        )
    }

    pub(super) fn chunk_bounds(&self, chunk: SurfaceChunkKey) -> (Vector3, Vector3) {
        let min_x = chunk.0 as f32 * self.chunk_span_m;
        let min_z = chunk.1 as f32 * self.chunk_span_m;
        let max_x = min_x + self.chunk_span_m;
        let max_z = min_z + self.chunk_span_m;
        (
            Vector3::new(min_x, 0.0, min_z),
            Vector3::new(max_x, 0.0, max_z),
        )
    }

    pub(super) fn bounds_to_chunk_keys(&self, min: Vector3, max: Vector3) -> Vec<SurfaceChunkKey> {
        let min_chunk = self.chunk_coords_for_world(min.x, min.z);
        let max_chunk = self.chunk_coords_for_world(max.x, max.z);
        let mut chunks = Vec::new();
        for cx in min_chunk.0..=max_chunk.0 {
            for cz in min_chunk.1..=max_chunk.1 {
                chunks.push((cx, cz));
            }
        }
        chunks
    }
}
