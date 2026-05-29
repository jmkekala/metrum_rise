//! Road-surface system state, dirty rebuild orchestration, and shared ordering helpers.

use super::{
    ChunkCacheKind, NodeOwnedRegion, PARALLEL_SURFACE_COMPILE_MIN_ITEMS,
    RoadEarthworkChunkCacheEntry, RoadSurfaceChunkCacheEntry, RoadSurfaceSection,
    RoadSurfaceTerrainClipLoop, RoadSurfaceVisualNodePiece, RoadSurfaceVisualPolygon,
    RoadSurfaceVisualSpanPiece, SAMPLE_EPSILON_M, SurfaceChunkKey,
};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::terrain::TerrainSystem;
use rayon::prelude::*;
use std::collections::{BTreeSet, HashMap, HashSet};

/// Ownership cache and compiler for the road-surface pipeline.
pub struct RoadSurfaceSystem {
    pub(crate) chunk_span_m: f32,
    pub(crate) compiled_once: bool,
    pub(crate) dirty_edges: HashSet<usize>,
    pub(crate) dirty_nodes: HashSet<u32>,
    pub(crate) dirty_surface_chunks: HashSet<SurfaceChunkKey>,
    pub(crate) dirty_terrain_chunks: HashSet<SurfaceChunkKey>,
    pub(crate) node_validation_logging_enabled: bool,
    pub(crate) compiled_sections: HashMap<usize, Vec<RoadSurfaceSection>>,
    pub(crate) compiled_visual_span_pieces: HashMap<usize, RoadSurfaceVisualSpanPiece>,
    pub(crate) compiled_visual_node_pieces: HashMap<u32, RoadSurfaceVisualNodePiece>,
    pub(crate) surface_span_chunks: HashMap<usize, Vec<SurfaceChunkKey>>,
    pub(crate) surface_node_chunks: HashMap<u32, Vec<SurfaceChunkKey>>,
    pub(crate) earthwork_span_chunks: HashMap<usize, Vec<SurfaceChunkKey>>,
    pub(crate) earthwork_node_chunks: HashMap<u32, Vec<SurfaceChunkKey>>,
    pub(crate) surface_chunk_spans: HashMap<SurfaceChunkKey, BTreeSet<usize>>,
    pub(crate) surface_chunk_nodes: HashMap<SurfaceChunkKey, BTreeSet<u32>>,
    pub(crate) earthwork_chunk_spans: HashMap<SurfaceChunkKey, BTreeSet<usize>>,
    pub(crate) earthwork_chunk_nodes: HashMap<SurfaceChunkKey, BTreeSet<u32>>,
    pub(crate) surface_chunk_cache: HashMap<SurfaceChunkKey, RoadSurfaceChunkCacheEntry>,
    pub(crate) earthwork_chunk_cache: HashMap<SurfaceChunkKey, RoadEarthworkChunkCacheEntry>,
    pub(crate) last_rebuilt_surface_chunks: Vec<SurfaceChunkKey>,
    pub(crate) last_rebuilt_terrain_chunks: Vec<SurfaceChunkKey>,
}

impl RoadSurfaceSystem {
    /// Creates an empty road-surface system using the given chunk span in world metres.
    pub fn new(chunk_span_m: f32) -> Self {
        Self {
            chunk_span_m: chunk_span_m.max(f32::EPSILON),
            compiled_once: false,
            dirty_edges: HashSet::new(),
            dirty_nodes: HashSet::new(),
            dirty_surface_chunks: HashSet::new(),
            dirty_terrain_chunks: HashSet::new(),
            node_validation_logging_enabled: true,
            compiled_sections: HashMap::new(),
            compiled_visual_span_pieces: HashMap::new(),
            compiled_visual_node_pieces: HashMap::new(),
            surface_span_chunks: HashMap::new(),
            surface_node_chunks: HashMap::new(),
            earthwork_span_chunks: HashMap::new(),
            earthwork_node_chunks: HashMap::new(),
            surface_chunk_spans: HashMap::new(),
            surface_chunk_nodes: HashMap::new(),
            earthwork_chunk_spans: HashMap::new(),
            earthwork_chunk_nodes: HashMap::new(),
            surface_chunk_cache: HashMap::new(),
            earthwork_chunk_cache: HashMap::new(),
            last_rebuilt_surface_chunks: Vec::new(),
            last_rebuilt_terrain_chunks: Vec::new(),
        }
    }

    /// Returns the configured chunk span in world metres.
    pub fn chunk_span_m(&self) -> f32 {
        self.chunk_span_m
    }

    /// Returns the set of edge ids that need road-surface recompilation.
    pub fn dirty_edges(&self) -> &HashSet<usize> {
        &self.dirty_edges
    }

    /// Returns the set of node ids that need node-patch recompilation.
    pub fn dirty_nodes(&self) -> &HashSet<u32> {
        &self.dirty_nodes
    }

    /// Returns the set of road-surface chunks that need render-cache rebuild.
    pub fn dirty_surface_chunks(&self) -> &HashSet<SurfaceChunkKey> {
        &self.dirty_surface_chunks
    }

    /// Returns the set of terrain chunks that need road-earthwork rebuild.
    pub fn dirty_terrain_chunks(&self) -> &HashSet<SurfaceChunkKey> {
        &self.dirty_terrain_chunks
    }

    /// Returns the currently cached compiled sections by edge id.
    pub fn compiled_sections(&self) -> &HashMap<usize, Vec<RoadSurfaceSection>> {
        &self.compiled_sections
    }

    /// Returns the currently cached explicit visual span pieces by edge id.
    pub fn compiled_visual_span_pieces(&self) -> &HashMap<usize, RoadSurfaceVisualSpanPiece> {
        &self.compiled_visual_span_pieces
    }

    /// Returns the currently cached explicit visual node pieces by node id.
    pub fn compiled_visual_node_pieces(&self) -> &HashMap<u32, RoadSurfaceVisualNodePiece> {
        &self.compiled_visual_node_pieces
    }

    /// Returns the current per-chunk surface cache shell.
    pub fn surface_chunk_cache(&self) -> &HashMap<SurfaceChunkKey, RoadSurfaceChunkCacheEntry> {
        &self.surface_chunk_cache
    }

    /// Returns the current per-chunk earthwork cache shell.
    pub fn earthwork_chunk_cache(&self) -> &HashMap<SurfaceChunkKey, RoadEarthworkChunkCacheEntry> {
        &self.earthwork_chunk_cache
    }

    /// Compiles the road-surface cache if it is dirty or has not been built yet.
    pub fn compile_dirty(&mut self, graph: &RegionGraph, terrain: &TerrainSystem) {
        if !self.compiled_once {
            self.compile_all(graph, terrain);
            return;
        }

        if self.dirty_edges.is_empty()
            && self.dirty_nodes.is_empty()
            && self.dirty_surface_chunks.is_empty()
            && self.dirty_terrain_chunks.is_empty()
        {
            return;
        }

        self.prune_stale_cache_entries(graph);

        let mut edge_ids: Vec<usize> = self.dirty_edges.iter().copied().collect();
        edge_ids.sort_unstable();

        let mut node_ids: HashSet<u32> = self
            .dirty_nodes
            .iter()
            .copied()
            .map(|node_id| graph.get_valid_node(node_id))
            .collect();
        for &edge_idx in &edge_ids {
            if edge_idx >= graph.edge_count() {
                continue;
            }
            let edge = graph.edge(edge_idx);
            node_ids.insert(graph.get_valid_node(edge.start_node));
            node_ids.insert(graph.get_valid_node(edge.end_node));
        }

        let mut sorted_nodes: Vec<u32> = node_ids.into_iter().collect();
        sorted_nodes.sort_unstable();
        sorted_nodes.dedup();

        let mut span_edge_ids: HashSet<usize> = self.dirty_edges.iter().copied().collect();
        for &node_id in &sorted_nodes {
            if node_id as usize >= graph.node_adjacency_count() {
                continue;
            }
            for &edge_idx in graph.node_adjacency(node_id) {
                span_edge_ids.insert(edge_idx);
            }
        }
        let mut sorted_span_edges: Vec<usize> = span_edge_ids.into_iter().collect();
        sorted_span_edges.sort_unstable();
        sorted_span_edges.dedup();
        let section_results: Vec<(usize, Option<Vec<RoadSurfaceSection>>)> =
            Self::collect_surface_compile_work(&sorted_span_edges, |edge_idx| {
                if edge_idx >= graph.edge_count() {
                    return (edge_idx, None);
                }
                let edge = graph.edge(edge_idx);
                if !Self::is_surface_edge(edge) {
                    return (edge_idx, None);
                }
                (edge_idx, Some(self.compile_edge_sections(graph, edge_idx)))
            });
        for (edge_idx, sections) in section_results {
            if let Some(sections) = sections {
                self.compiled_sections.insert(edge_idx, sections);
            } else {
                self.compiled_sections.remove(&edge_idx);
            }
        }

        let mut span_candidates = Vec::new();
        for &edge_idx in &sorted_span_edges {
            self.remove_span_piece_coverage(edge_idx);
            if edge_idx >= graph.edge_count() {
                self.compiled_visual_span_pieces.remove(&edge_idx);
                continue;
            }
            let edge = graph.edge(edge_idx);
            if !Self::is_surface_edge(edge) {
                self.compiled_visual_span_pieces.remove(&edge_idx);
                continue;
            }
            span_candidates.push(edge_idx);
        }
        let span_results: Vec<(usize, Option<RoadSurfaceVisualSpanPiece>)> =
            Self::collect_surface_compile_work(&span_candidates, |edge_idx| {
                (
                    edge_idx,
                    self.compile_visual_span_piece(graph, terrain, edge_idx),
                )
            });
        for (edge_idx, span_piece) in span_results {
            if let Some(span_piece) = span_piece {
                self.insert_span_piece_coverage(&span_piece);
                self.compiled_visual_span_pieces
                    .insert(edge_idx, span_piece);
            } else {
                self.compiled_visual_span_pieces.remove(&edge_idx);
            }
        }

        let mut node_candidates = Vec::new();
        for &node_id in &sorted_nodes {
            self.remove_node_piece_coverage(node_id);
            if self.node_has_surface_edges(graph, node_id) {
                node_candidates.push(node_id);
            } else {
                self.compiled_visual_node_pieces.remove(&node_id);
            }
        }
        let node_results: Vec<(u32, Option<RoadSurfaceVisualNodePiece>)> =
            Self::collect_surface_compile_work(&node_candidates, |node_id| {
                (
                    node_id,
                    self.compile_visual_node_piece(graph, terrain, node_id),
                )
            });
        for (node_id, visual_piece) in node_results {
            if let Some(visual_piece) = visual_piece {
                self.insert_node_piece_coverage(&visual_piece);
                self.compiled_visual_node_pieces
                    .insert(node_id, visual_piece);
            } else {
                self.compiled_visual_node_pieces.remove(&node_id);
            }
        }
        let dirty_surface_chunks = self.sorted_chunk_keys(&self.dirty_surface_chunks);
        let dirty_terrain_chunks = self.sorted_chunk_keys(&self.dirty_terrain_chunks);
        self.rebuild_surface_chunk_cache(&dirty_surface_chunks);
        self.rebuild_earthwork_chunk_cache(&dirty_terrain_chunks);
        self.last_rebuilt_surface_chunks = dirty_surface_chunks;
        self.last_rebuilt_terrain_chunks = dirty_terrain_chunks;
        self.compiled_once = true;
        self.clear_dirty_tracking();
    }

    fn compile_all(&mut self, graph: &RegionGraph, terrain: &TerrainSystem) {
        self.prune_stale_cache_entries(graph);
        self.clear_piece_chunk_coverage();
        self.surface_chunk_cache.clear();
        self.earthwork_chunk_cache.clear();

        let edge_ids = self.all_surface_edge_ids(graph);
        let section_results: Vec<(usize, Vec<RoadSurfaceSection>)> =
            Self::collect_surface_compile_work(&edge_ids, |edge_idx| {
                (edge_idx, self.compile_edge_sections(graph, edge_idx))
            });
        for (edge_idx, sections) in section_results {
            self.compiled_sections.insert(edge_idx, sections);
        }

        let span_results: Vec<(usize, Option<RoadSurfaceVisualSpanPiece>)> =
            Self::collect_surface_compile_work(&edge_ids, |edge_idx| {
                (
                    edge_idx,
                    self.compile_visual_span_piece(graph, terrain, edge_idx),
                )
            });
        for (edge_idx, span_piece) in span_results {
            if let Some(span_piece) = span_piece {
                self.insert_span_piece_coverage(&span_piece);
                self.compiled_visual_span_pieces
                    .insert(edge_idx, span_piece);
            } else {
                self.compiled_visual_span_pieces.remove(&edge_idx);
            }
        }

        let node_ids = self.all_surface_node_ids(graph);
        let node_results: Vec<(u32, Option<RoadSurfaceVisualNodePiece>)> =
            Self::collect_surface_compile_work(&node_ids, |node_id| {
                (
                    node_id,
                    self.compile_visual_node_piece(graph, terrain, node_id),
                )
            });
        for (node_id, visual_piece) in node_results {
            if let Some(visual_piece) = visual_piece {
                self.insert_node_piece_coverage(&visual_piece);
                self.compiled_visual_node_pieces
                    .insert(node_id, visual_piece);
            } else {
                self.compiled_visual_node_pieces.remove(&node_id);
            }
        }
        let all_surface_chunks = self.collect_all_chunks(ChunkCacheKind::Surface);
        let all_earthwork_chunks = self.collect_all_chunks(ChunkCacheKind::Earthwork);
        self.rebuild_surface_chunk_cache(&all_surface_chunks);
        self.rebuild_earthwork_chunk_cache(&all_earthwork_chunks);
        self.last_rebuilt_surface_chunks = all_surface_chunks;
        self.last_rebuilt_terrain_chunks = all_earthwork_chunks;
        self.compiled_once = true;
        self.clear_dirty_tracking();
    }

    pub(crate) fn collect_surface_compile_work<I, O, F>(items: &[I], work: F) -> Vec<O>
    where
        I: Copy + Send + Sync,
        O: Send,
        F: Fn(I) -> O + Sync,
    {
        // Slice parallel iterators are indexed; collecting into Vec preserves input order, so
        // the serial commit phase remains deterministic without re-sorting by id.
        if items.len() >= PARALLEL_SURFACE_COMPILE_MIN_ITEMS {
            items.par_iter().copied().map(&work).collect()
        } else {
            items.iter().copied().map(&work).collect()
        }
    }

    pub(crate) fn section_index_range_for_s_bounds(
        sections: &[RoadSurfaceSection],
        start_s_m: f32,
        end_s_m: f32,
    ) -> Option<(usize, usize)> {
        if sections.len() < 2 || end_s_m - start_s_m <= SAMPLE_EPSILON_M {
            return None;
        }

        let start_index = sections
            .iter()
            .position(|section| section.s_m + SAMPLE_EPSILON_M >= start_s_m)
            .unwrap_or(0);
        let end_index = sections
            .iter()
            .rposition(|section| section.s_m - SAMPLE_EPSILON_M <= end_s_m)
            .unwrap_or(sections.len().saturating_sub(1));
        (end_index > start_index).then_some((start_index, end_index))
    }

    pub(crate) fn sort_visual_polygons(polygons: &mut [RoadSurfaceVisualPolygon]) {
        polygons.sort_by(Self::visual_polygon_ordering);
    }

    pub(crate) fn visual_polygon_ordering(
        a: &RoadSurfaceVisualPolygon,
        b: &RoadSurfaceVisualPolygon,
    ) -> std::cmp::Ordering {
        match (a.points_world.first(), b.points_world.first()) {
            (Some(point_a), Some(point_b)) => point_a
                .x
                .total_cmp(&point_b.x)
                .then(point_a.z.total_cmp(&point_b.z))
                .then(point_a.y.total_cmp(&point_b.y)),
            (None, Some(_)) => std::cmp::Ordering::Less,
            (Some(_), None) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
        .then(a.points_world.len().cmp(&b.points_world.len()))
        .then_with(|| {
            a.points_world
                .iter()
                .zip(&b.points_world)
                .find_map(|(point_a, point_b)| {
                    let ordering = point_a
                        .x
                        .total_cmp(&point_b.x)
                        .then(point_a.z.total_cmp(&point_b.z))
                        .then(point_a.y.total_cmp(&point_b.y));
                    (ordering != std::cmp::Ordering::Equal).then_some(ordering)
                })
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }

    pub(crate) fn sort_terrain_clip_loops(loops: &mut [RoadSurfaceTerrainClipLoop]) {
        loops.sort_by(|a, b| {
            match (a.points_world.first(), b.points_world.first()) {
                (Some(point_a), Some(point_b)) => point_a
                    .x
                    .total_cmp(&point_b.x)
                    .then(point_a.z.total_cmp(&point_b.z))
                    .then(point_a.y.total_cmp(&point_b.y)),
                (None, Some(_)) => std::cmp::Ordering::Less,
                (Some(_), None) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
            .then(a.points_world.len().cmp(&b.points_world.len()))
            .then_with(|| {
                a.points_world
                    .iter()
                    .zip(&b.points_world)
                    .find_map(|(point_a, point_b)| {
                        let ordering = point_a
                            .x
                            .total_cmp(&point_b.x)
                            .then(point_a.z.total_cmp(&point_b.z))
                            .then(point_a.y.total_cmp(&point_b.y));
                        (ordering != std::cmp::Ordering::Equal).then_some(ordering)
                    })
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        });
    }

    pub(crate) fn node_owned_region_ordering(
        a: &NodeOwnedRegion,
        b: &NodeOwnedRegion,
    ) -> std::cmp::Ordering {
        Self::band_kind_sort_key(a.kind)
            .cmp(&Self::band_kind_sort_key(b.kind))
            .then(a.owner_index.cmp(&b.owner_index))
            .then_with(|| {
                match (
                    a.polygon.points_world.first(),
                    b.polygon.points_world.first(),
                ) {
                    (Some(point_a), Some(point_b)) => point_a
                        .x
                        .total_cmp(&point_b.x)
                        .then(point_a.z.total_cmp(&point_b.z))
                        .then(point_a.y.total_cmp(&point_b.y)),
                    (None, Some(_)) => std::cmp::Ordering::Less,
                    (Some(_), None) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                }
            })
            .then(
                a.polygon
                    .points_world
                    .len()
                    .cmp(&b.polygon.points_world.len()),
            )
    }
}
