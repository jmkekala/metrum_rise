//! Road-surface system state, dirty rebuild orchestration, and shared ordering helpers.

use super::{
    CompiledNodeKind, NodeCanonicalTopologyCache, NodeOwnedRegion, NodeVisualCompileResult,
    PARALLEL_NODE_COMPILE_MIN_ITEMS, PARALLEL_SURFACE_COMPILE_MIN_ITEMS,
    RoadEarthworkChunkCacheEntry, RoadSurfaceChunkCacheEntry, RoadSurfaceEarthworkBoundarySegment,
    RoadSurfaceSection, RoadSurfaceTerrainClipLoop, RoadSurfaceVisualNodeCompileInput,
    RoadSurfaceVisualNodePiece, RoadSurfaceVisualNodePieceKind, RoadSurfaceVisualPolygon,
    RoadSurfaceVisualSpanPiece, SAMPLE_EPSILON_M, SurfaceChunkKey,
};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::terrain::TerrainSystem;
use rayon::prelude::*;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

fn elapsed_ms(start: Option<Instant>) -> f64 {
    start
        .map(|start| start.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

/// Ownership cache and compiler for the road-surface pipeline.
#[derive(Clone)]
pub struct RoadSurfaceSystem {
    pub(crate) chunk_span_m: f32,
    pub(crate) chunk_origin_x_m: f32,
    pub(crate) chunk_origin_z_m: f32,
    pub(crate) compiled_once: bool,
    pub(crate) compile_invalidation_generation: u64,
    pub(crate) failed_compile_generation: Option<u64>,
    pub(in crate::simulation::network::surface) last_compile_failure_label: Option<String>,
    pub(in crate::simulation::network::surface) last_failed_span_ids: Vec<usize>,
    pub(in crate::simulation::network::surface) last_failed_node_ids: Vec<u32>,
    pub(crate) dirty_edges: HashSet<usize>,
    pub(crate) dirty_nodes: HashSet<u32>,
    pub(crate) dirty_surface_chunks: HashSet<SurfaceChunkKey>,
    pub(crate) dirty_terrain_chunks: HashSet<SurfaceChunkKey>,
    pub(crate) dirty_query_chunks: HashSet<SurfaceChunkKey>,
    pub(crate) node_validation_logging_enabled: bool,
    pub(crate) compiled_sections: HashMap<usize, Vec<RoadSurfaceSection>>,
    pub(crate) compiled_visual_span_pieces: HashMap<usize, RoadSurfaceVisualSpanPiece>,
    pub(crate) compiled_visual_node_pieces: HashMap<u32, RoadSurfaceVisualNodePiece>,
    pub(crate) compiled_visual_node_inputs: HashMap<u32, RoadSurfaceVisualNodeCompileInput>,
    pub(crate) compiled_visual_node_earthwork_boundaries:
        HashMap<u32, Arc<Vec<Vec<RoadSurfaceEarthworkBoundarySegment>>>>,
    pub(crate) compiled_visual_node_topologies: HashMap<u32, Arc<NodeCanonicalTopologyCache>>,
    pub(crate) surface_span_chunks: HashMap<usize, Vec<SurfaceChunkKey>>,
    pub(crate) surface_node_chunks: HashMap<u32, Vec<SurfaceChunkKey>>,
    pub(crate) earthwork_span_chunks: HashMap<usize, Vec<SurfaceChunkKey>>,
    pub(crate) earthwork_node_chunks: HashMap<u32, Vec<SurfaceChunkKey>>,
    pub(crate) surface_chunk_spans: HashMap<SurfaceChunkKey, BTreeSet<usize>>,
    pub(crate) surface_chunk_nodes: HashMap<SurfaceChunkKey, BTreeSet<u32>>,
    pub(crate) earthwork_chunk_spans: HashMap<SurfaceChunkKey, BTreeSet<usize>>,
    pub(crate) earthwork_chunk_nodes: HashMap<SurfaceChunkKey, BTreeSet<u32>>,
    pub(crate) query_span_chunks: HashMap<usize, Vec<SurfaceChunkKey>>,
    pub(crate) query_node_chunks: HashMap<u32, Vec<SurfaceChunkKey>>,
    pub(crate) query_chunk_spans: HashMap<SurfaceChunkKey, BTreeSet<usize>>,
    pub(crate) query_chunk_nodes: HashMap<SurfaceChunkKey, BTreeSet<u32>>,
    pub(crate) surface_chunk_cache: HashMap<SurfaceChunkKey, RoadSurfaceChunkCacheEntry>,
    pub(crate) earthwork_chunk_cache: HashMap<SurfaceChunkKey, RoadEarthworkChunkCacheEntry>,
    pub(crate) last_rebuilt_surface_chunks: Vec<SurfaceChunkKey>,
    pub(crate) last_rebuilt_terrain_chunks: Vec<SurfaceChunkKey>,
    pub(crate) last_rebuilt_query_chunks: Vec<SurfaceChunkKey>,
    pub(crate) last_reused_node_topology_count: usize,
    pub(crate) last_reused_node_height_topology_count: usize,
    pub(crate) last_reused_node_ownership_topology_count: usize,
}

/// Runtime caller category attached to road-surface compile timing logs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RoadSurfaceCompileReason {
    /// Compile was requested through the legacy/default entry point.
    Unspecified,
    /// Async road-tool preview worker compiled transient surface geometry.
    PreviewWorker,
    /// Diagnostic or commit validation compiled transient candidate geometry.
    CommitValidator,
    /// Simulation thread compiled committed road edits.
    SimCommit,
    /// Road mesh generation compiled or refreshed surface cache before rendering.
    MeshPrecompute,
    /// Terrain earthwork or refined terrain preparation needed current road ownership.
    TerrainEarthwork,
}

impl RoadSurfaceCompileReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Unspecified => "unspecified",
            Self::PreviewWorker => "preview_worker",
            Self::CommitValidator => "commit_validator",
            Self::SimCommit => "sim_commit",
            Self::MeshPrecompute => "mesh_precompute",
            Self::TerrainEarthwork => "terrain_earthwork",
        }
    }
}

impl RoadSurfaceSystem {
    /// Creates an empty road-surface system using the given chunk span in world metres.
    pub fn new(chunk_span_m: f32) -> Self {
        Self::new_with_chunk_grid(chunk_span_m, 0.0, 0.0)
    }

    /// Creates an empty road-surface system with an explicit world-space chunk-grid origin.
    pub fn new_with_chunk_grid(
        chunk_span_m: f32,
        chunk_origin_x_m: f32,
        chunk_origin_z_m: f32,
    ) -> Self {
        Self {
            chunk_span_m: chunk_span_m.max(f32::EPSILON),
            chunk_origin_x_m: if chunk_origin_x_m.is_finite() {
                chunk_origin_x_m
            } else {
                0.0
            },
            chunk_origin_z_m: if chunk_origin_z_m.is_finite() {
                chunk_origin_z_m
            } else {
                0.0
            },
            compiled_once: false,
            compile_invalidation_generation: 0,
            failed_compile_generation: None,
            last_compile_failure_label: None,
            last_failed_span_ids: Vec::new(),
            last_failed_node_ids: Vec::new(),
            dirty_edges: HashSet::new(),
            dirty_nodes: HashSet::new(),
            dirty_surface_chunks: HashSet::new(),
            dirty_terrain_chunks: HashSet::new(),
            dirty_query_chunks: HashSet::new(),
            node_validation_logging_enabled: true,
            compiled_sections: HashMap::new(),
            compiled_visual_span_pieces: HashMap::new(),
            compiled_visual_node_pieces: HashMap::new(),
            compiled_visual_node_inputs: HashMap::new(),
            compiled_visual_node_earthwork_boundaries: HashMap::new(),
            compiled_visual_node_topologies: HashMap::new(),
            surface_span_chunks: HashMap::new(),
            surface_node_chunks: HashMap::new(),
            earthwork_span_chunks: HashMap::new(),
            earthwork_node_chunks: HashMap::new(),
            surface_chunk_spans: HashMap::new(),
            surface_chunk_nodes: HashMap::new(),
            earthwork_chunk_spans: HashMap::new(),
            earthwork_chunk_nodes: HashMap::new(),
            query_span_chunks: HashMap::new(),
            query_node_chunks: HashMap::new(),
            query_chunk_spans: HashMap::new(),
            query_chunk_nodes: HashMap::new(),
            surface_chunk_cache: HashMap::new(),
            earthwork_chunk_cache: HashMap::new(),
            last_rebuilt_surface_chunks: Vec::new(),
            last_rebuilt_terrain_chunks: Vec::new(),
            last_rebuilt_query_chunks: Vec::new(),
            last_reused_node_topology_count: 0,
            last_reused_node_height_topology_count: 0,
            last_reused_node_ownership_topology_count: 0,
        }
    }

    /// Returns the configured chunk span in world metres.
    pub fn chunk_span_m(&self) -> f32 {
        self.chunk_span_m
    }

    /// Returns the world-space minimum corner from which road chunk keys are measured.
    pub fn chunk_origin_m(&self) -> (f32, f32) {
        (self.chunk_origin_x_m, self.chunk_origin_z_m)
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

    /// Returns fixed-size query chunks touched by either removed or inserted road coverage.
    pub fn dirty_query_chunks(&self) -> &HashSet<SurfaceChunkKey> {
        &self.dirty_query_chunks
    }

    /// Returns the sorted old-or-new query chunks covered by the last completed compile.
    pub fn last_rebuilt_query_chunks(&self) -> &[SurfaceChunkKey] {
        &self.last_rebuilt_query_chunks
    }

    /// Returns whether the next compile represents changed source coverage.
    pub(crate) fn has_pending_rebuild_work(&self) -> bool {
        !self.compiled_once
            || self.compile_generation_is_latched()
            || !self.dirty_edges.is_empty()
            || !self.dirty_nodes.is_empty()
            || !self.dirty_surface_chunks.is_empty()
            || !self.dirty_terrain_chunks.is_empty()
            || !self.dirty_query_chunks.is_empty()
    }

    /// Returns whether compiled owners and chunk indexes match the current invalidation generation.
    pub(crate) fn published_generation_matches_source(&self) -> bool {
        self.compiled_once && !self.has_pending_rebuild_work()
    }

    /// Returns the last compiler failure summary for the currently latched generation.
    pub(crate) fn last_compile_failure_label(&self) -> Option<&str> {
        self.last_compile_failure_label.as_deref()
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
        self.compile_dirty_with_reason(graph, terrain, RoadSurfaceCompileReason::Unspecified);
    }

    pub(crate) fn compile_dirty_with_reason(
        &mut self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        reason: RoadSurfaceCompileReason,
    ) {
        if self.compile_generation_is_latched() {
            return;
        }

        if !self.compiled_once {
            self.compile_all_with_reason(graph, terrain, reason);
            return;
        }

        if self.dirty_edges.is_empty()
            && self.dirty_nodes.is_empty()
            && self.dirty_surface_chunks.is_empty()
            && self.dirty_terrain_chunks.is_empty()
            && self.dirty_query_chunks.is_empty()
        {
            return;
        }

        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let dirty_edge_count = self.dirty_edges.len();
        let dirty_node_count = self.dirty_nodes.len();
        let dirty_surface_chunk_count = self.dirty_surface_chunks.len();
        let dirty_terrain_chunk_count = self.dirty_terrain_chunks.len();
        let allow_node_reuse = dirty_terrain_chunk_count == 0;

        let ordering_start = road_debug.then(Instant::now);
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
        let ordering_ms = elapsed_ms(ordering_start);

        let sections_start = road_debug.then(Instant::now);
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
        // Compiler methods read only the staged sections/spans, chunk scale, validation policy,
        // and explicitly supplied prior node topology. Keep that dirty-only state isolated so a
        // failed generation cannot mutate any published cache or coverage index.
        let mut staging = RoadSurfaceSystem::new_with_chunk_grid(
            self.chunk_span_m,
            self.chunk_origin_x_m,
            self.chunk_origin_z_m,
        );
        staging.node_validation_logging_enabled = self.node_validation_logging_enabled;
        for (edge_idx, sections) in section_results {
            if let Some(sections) = sections {
                staging.compiled_sections.insert(edge_idx, sections);
            }
        }
        let sections_ms = elapsed_ms(sections_start);

        let spans_start = road_debug.then(Instant::now);
        let span_candidates: Vec<usize> = sorted_span_edges
            .iter()
            .copied()
            .filter(|edge_idx| {
                *edge_idx < graph.edge_count() && Self::is_surface_edge(graph.edge(*edge_idx))
            })
            .collect();
        let span_results: Vec<(usize, Option<RoadSurfaceVisualSpanPiece>)> =
            Self::collect_surface_compile_work(&span_candidates, |edge_idx| {
                (
                    edge_idx,
                    staging.compile_visual_span_piece(graph, terrain, edge_idx),
                )
            });
        let mut failed_span_ids = Vec::new();
        for (edge_idx, span_piece) in span_results {
            if let Some(span_piece) = span_piece {
                staging
                    .compiled_visual_span_pieces
                    .insert(edge_idx, span_piece);
            } else {
                failed_span_ids.push(edge_idx);
            }
        }
        let spans_ms = elapsed_ms(spans_start);
        if !failed_span_ids.is_empty() {
            let failure_label = format!(
                "stage=dirty_spans compile_reason={} failed_spans={:?} dirty_edges={} dirty_nodes={} span_edges={}",
                reason.as_str(),
                failed_span_ids,
                dirty_edge_count,
                dirty_node_count,
                sorted_span_edges.len()
            );
            self.latch_compile_failure(failure_label.clone());
            self.last_failed_span_ids = failed_span_ids.clone();
            self.last_failed_node_ids.clear();
            if road_debug {
                crate::debug_log!(
                    "road",
                    "surface_compile_dirty_incomplete {} total_ms={:.3}",
                    failure_label,
                    elapsed_ms(total_start)
                );
            }
            return;
        }

        let nodes_start = road_debug.then(Instant::now);
        let mut node_candidates = Vec::new();
        let mut nodes_to_remove = Vec::new();
        let mut failed_node_ids = Vec::new();
        let mut reused_node_count = 0usize;
        for &node_id in &sorted_nodes {
            let Ok(expected_kind) = staging.expected_visual_node_piece_kind(graph, node_id) else {
                failed_node_ids.push(node_id);
                continue;
            };
            let Some((expected_kind, expected_mouth_count)) = expected_kind else {
                nodes_to_remove.push(node_id);
                continue;
            };
            if staging.structural_terminal_piece_is_span_owned(graph, node_id, expected_kind) {
                nodes_to_remove.push(node_id);
                continue;
            }
            let Some(input) = staging.visual_node_compile_input(graph, node_id) else {
                failed_node_ids.push(node_id);
                continue;
            };
            if input.kind != expected_kind || input.mouths.len() != expected_mouth_count {
                failed_node_ids.push(node_id);
                continue;
            }
            let input_matches = self
                .compiled_visual_node_inputs
                .get(&node_id)
                .is_some_and(|previous| previous == &input);
            if input_matches && self.compiled_visual_node_pieces.contains_key(&node_id) {
                if allow_node_reuse {
                    reused_node_count += 1;
                    continue;
                }
                if self
                    .compiled_visual_node_earthwork_boundaries
                    .contains_key(&node_id)
                {
                    node_candidates.push((node_id, input, true));
                    continue;
                }
            }
            node_candidates.push((node_id, input, false));
        }
        let node_results: Vec<(
            u32,
            RoadSurfaceVisualNodeCompileInput,
            Option<NodeVisualCompileResult>,
            bool,
            f64,
        )> = Self::collect_node_compile_work(&node_candidates, |node_id| {
            let earthwork_refresh_start = (road_debug && node_id.2).then(Instant::now);
            let reused_result = node_id.2.then(|| {
                self.compiled_visual_node_pieces
                    .get(&node_id.0)
                    .zip(
                        self.compiled_visual_node_earthwork_boundaries
                            .get(&node_id.0),
                    )
                    .and_then(|(piece, earthwork_boundaries)| {
                        staging
                            .refresh_visual_node_piece_earthwork_from_cached_top(
                                graph,
                                terrain,
                                node_id.0,
                                &node_id.1,
                                piece,
                                earthwork_boundaries,
                            )
                            .map(|piece| NodeVisualCompileResult {
                                piece,
                                earthwork_boundaries: Arc::clone(earthwork_boundaries),
                                topology_cache: self
                                    .compiled_visual_node_topologies
                                    .get(&node_id.0)
                                    .cloned(),
                                rail_topology_reused: false,
                                ownership_reused: false,
                                export_reuse_stats: Default::default(),
                            })
                    })
            });
            let earthwork_refresh_ms = elapsed_ms(earthwork_refresh_start);
            let (result, topology_reused) = if let Some(result) = reused_result.flatten() {
                (Some(result), true)
            } else {
                (
                    staging.compile_visual_node_piece_with_earthwork_boundaries(
                        graph,
                        terrain,
                        node_id.0,
                        &node_id.1,
                        self.compiled_visual_node_topologies
                            .get(&node_id.0)
                            .map(Arc::as_ref),
                    ),
                    false,
                )
            };
            (
                node_id.0,
                node_id.1.clone(),
                result,
                topology_reused,
                earthwork_refresh_ms,
            )
        });
        failed_node_ids.extend(
            node_results
                .iter()
                .filter_map(|result| result.2.is_none().then_some(result.0)),
        );
        failed_node_ids.sort_unstable();
        failed_node_ids.dedup();
        let nodes_ms = elapsed_ms(nodes_start);
        if !failed_node_ids.is_empty() {
            let failure_label = format!(
                "stage=dirty_nodes compile_reason={} failed_nodes={:?} dirty_edges={} dirty_nodes={} span_edges={} node_candidates={}",
                reason.as_str(),
                failed_node_ids,
                dirty_edge_count,
                dirty_node_count,
                sorted_span_edges.len(),
                node_candidates.len()
            );
            self.latch_compile_failure(failure_label.clone());
            self.last_failed_span_ids.clear();
            self.last_failed_node_ids = failed_node_ids.clone();
            if road_debug {
                crate::debug_log!(
                    "road",
                    "surface_compile_dirty_incomplete {} total_ms={:.3}",
                    failure_label,
                    elapsed_ms(total_start)
                );
            }
            return;
        }
        let reused_node_topology_count = node_results.iter().filter(|result| result.3).count();
        let reused_node_height_topology_count = node_results
            .iter()
            .filter(|result| {
                result
                    .2
                    .as_ref()
                    .is_some_and(|compile| compile.rail_topology_reused)
            })
            .count();
        let reused_node_ownership_topology_count = node_results
            .iter()
            .filter(|result| {
                result
                    .2
                    .as_ref()
                    .is_some_and(|compile| compile.ownership_reused)
            })
            .count();
        let node_earthwork_refresh_ms = node_results.iter().map(|result| result.4).sum::<f64>();

        let prune_start = road_debug.then(Instant::now);
        self.prune_stale_cache_entries(graph);
        let prune_ms = elapsed_ms(prune_start);
        for &edge_idx in &sorted_span_edges {
            if let Some(sections) = staging.compiled_sections.remove(&edge_idx) {
                self.compiled_sections.insert(edge_idx, sections);
            } else {
                self.compiled_sections.remove(&edge_idx);
            }
            let span_piece = staging.compiled_visual_span_pieces.remove(&edge_idx);
            self.apply_span_compile_result(edge_idx, span_piece);
        }
        for node_id in nodes_to_remove {
            self.remove_node_piece_coverage(node_id);
            self.compiled_visual_node_pieces.remove(&node_id);
            self.compiled_visual_node_inputs.remove(&node_id);
            self.compiled_visual_node_earthwork_boundaries
                .remove(&node_id);
            self.compiled_visual_node_topologies.remove(&node_id);
        }
        for (node_id, input, visual_piece, _, _) in node_results {
            self.apply_node_compile_result_with_earthwork_boundaries(node_id, input, visual_piece);
        }
        self.last_reused_node_topology_count = reused_node_topology_count;
        self.last_reused_node_height_topology_count = reused_node_height_topology_count;
        self.last_reused_node_ownership_topology_count = reused_node_ownership_topology_count;

        let chunk_cache_start = road_debug.then(Instant::now);
        let dirty_surface_chunks = self.sorted_chunk_keys(&self.dirty_surface_chunks);
        let dirty_terrain_chunks = self.sorted_chunk_keys(&self.dirty_terrain_chunks);
        let dirty_query_chunks = self.sorted_chunk_keys(&self.dirty_query_chunks);
        self.rebuild_surface_chunk_cache(&dirty_surface_chunks);
        self.rebuild_earthwork_chunk_cache(&dirty_terrain_chunks);
        let chunk_cache_ms = elapsed_ms(chunk_cache_start);
        self.last_rebuilt_surface_chunks = dirty_surface_chunks;
        self.last_rebuilt_terrain_chunks = dirty_terrain_chunks;
        self.last_rebuilt_query_chunks = dirty_query_chunks;
        self.compiled_once = true;
        self.failed_compile_generation = None;
        self.last_compile_failure_label = None;
        self.last_failed_span_ids.clear();
        self.last_failed_node_ids.clear();
        self.clear_dirty_tracking();

        if road_debug {
            let total_ms = elapsed_ms(total_start);
            if total_ms >= 50.0 {
                crate::debug_log!(
                    "road",
                    "surface_compile_dirty_detail compile_reason={} dirty_edges={} dirty_nodes={} dirty_surface_chunks={} dirty_terrain_chunks={} span_edges={} nodes={} span_candidates={} node_candidates={} node_reused={} node_topology_reused={} node_height_topology_reused={} node_ownership_topology_reused={} rebuilt_surface_chunks={} rebuilt_terrain_chunks={} prune_ms={:.3} ordering_ms={:.3} sections_ms={:.3} spans_ms={:.3} nodes_ms={:.3} node_earthwork_refresh_ms={:.3} chunk_cache_ms={:.3} total_ms={:.3}",
                    reason.as_str(),
                    dirty_edge_count,
                    dirty_node_count,
                    dirty_surface_chunk_count,
                    dirty_terrain_chunk_count,
                    sorted_span_edges.len(),
                    sorted_nodes.len(),
                    span_candidates.len(),
                    node_candidates.len(),
                    reused_node_count,
                    reused_node_topology_count,
                    reused_node_height_topology_count,
                    reused_node_ownership_topology_count,
                    self.last_rebuilt_surface_chunks.len(),
                    self.last_rebuilt_terrain_chunks.len(),
                    prune_ms,
                    ordering_ms,
                    sections_ms,
                    spans_ms,
                    nodes_ms,
                    node_earthwork_refresh_ms,
                    chunk_cache_ms,
                    total_ms
                );
            }
        }
    }

    fn expected_visual_node_piece_kind(
        &self,
        graph: &RegionGraph,
        node_id: u32,
    ) -> Result<Option<(RoadSurfaceVisualNodePieceKind, usize)>, ()> {
        if !self.node_has_surface_edges(graph, node_id) {
            return Ok(None);
        }
        let incident_count = graph
            .node_adjacency(node_id)
            .iter()
            .copied()
            .filter(|edge_idx| {
                if *edge_idx >= graph.edge_count() {
                    return false;
                }
                let edge = graph.edge(*edge_idx);
                Self::is_surface_edge(edge)
                    && (graph.get_valid_node(edge.start_node) == node_id
                        || graph.get_valid_node(edge.end_node) == node_id)
            })
            .count();
        match incident_count {
            0 => Ok(None),
            1 => Ok(Some((
                RoadSurfaceVisualNodePieceKind::Terminal,
                incident_count,
            ))),
            2 => match self.classify_surface_node_kind_from_graph_geometry(graph, node_id) {
                Some(CompiledNodeKind::PassThrough) => Ok(None),
                Some(CompiledNodeKind::Bend) => {
                    Ok(Some((RoadSurfaceVisualNodePieceKind::Bend, incident_count)))
                }
                _ => Err(()),
            },
            _ => Ok(Some((
                RoadSurfaceVisualNodePieceKind::JunctionN,
                incident_count,
            ))),
        }
    }

    fn structural_terminal_piece_is_span_owned(
        &self,
        graph: &RegionGraph,
        node_id: u32,
        expected_kind: RoadSurfaceVisualNodePieceKind,
    ) -> bool {
        if expected_kind != RoadSurfaceVisualNodePieceKind::Terminal
            || node_id as usize >= graph.node_adjacency_count()
        {
            return false;
        }
        let mut incidents = graph
            .node_adjacency(node_id)
            .iter()
            .copied()
            .filter_map(|edge_idx| {
                if edge_idx >= graph.edge_count() {
                    return None;
                }
                let edge = graph.edge(edge_idx);
                (Self::is_surface_edge(edge)
                    && (graph.get_valid_node(edge.start_node) == node_id
                        || graph.get_valid_node(edge.end_node) == node_id))
                    .then_some(edge.class)
            });
        // Non-standard terminal spans have zero node handoff: the span closes the endpoint, so
        // submitting a zero-depth terminal cap would make omission depend on compiler failure.
        matches!(
            (incidents.next(), incidents.next()),
            (
                Some(
                    crate::simulation::network::types::EdgeClass::Bridge
                        | crate::simulation::network::types::EdgeClass::Tunnel
                ),
                None
            )
        )
    }

    fn compile_all_with_reason(
        &mut self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        reason: RoadSurfaceCompileReason,
    ) {
        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);

        let edge_start = road_debug.then(Instant::now);
        let edge_ids = self.all_surface_edge_ids(graph);
        let edge_ms = elapsed_ms(edge_start);

        // Build the first published generation in isolation. Initial compilation is not allowed to
        // expose a subset of spans or nodes merely because one owner failed to materialize.
        let staging_start = road_debug.then(Instant::now);
        let mut staging = RoadSurfaceSystem::new_with_chunk_grid(
            self.chunk_span_m,
            self.chunk_origin_x_m,
            self.chunk_origin_z_m,
        );
        staging.node_validation_logging_enabled = self.node_validation_logging_enabled;
        let staging_ms = elapsed_ms(staging_start);

        let sections_start = road_debug.then(Instant::now);
        let section_results: Vec<(usize, Vec<RoadSurfaceSection>)> =
            Self::collect_surface_compile_work(&edge_ids, |edge_idx| {
                (edge_idx, self.compile_edge_sections(graph, edge_idx))
            });
        for (edge_idx, sections) in section_results {
            staging.compiled_sections.insert(edge_idx, sections);
        }
        let sections_ms = elapsed_ms(sections_start);

        let spans_start = road_debug.then(Instant::now);
        let span_results: Vec<(usize, Option<RoadSurfaceVisualSpanPiece>)> =
            Self::collect_surface_compile_work(&edge_ids, |edge_idx| {
                (
                    edge_idx,
                    staging.compile_visual_span_piece(graph, terrain, edge_idx),
                )
            });
        let mut failed_span_ids = Vec::new();
        for (edge_idx, span_piece) in span_results {
            if let Some(span_piece) = span_piece {
                staging.apply_span_compile_result(edge_idx, Some(span_piece));
            } else {
                failed_span_ids.push(edge_idx);
            }
        }
        let spans_ms = elapsed_ms(spans_start);
        if !failed_span_ids.is_empty() {
            let failure_label = format!(
                "stage=all_spans compile_reason={} failed_spans={:?} edges={}",
                reason.as_str(),
                failed_span_ids,
                edge_ids.len()
            );
            self.latch_compile_failure(failure_label.clone());
            self.last_failed_span_ids = failed_span_ids.clone();
            self.last_failed_node_ids.clear();
            if road_debug {
                crate::debug_log!(
                    "road",
                    "surface_compile_all_incomplete {} total_ms={:.3}",
                    failure_label,
                    elapsed_ms(total_start)
                );
            }
            return;
        }

        let nodes_start = road_debug.then(Instant::now);
        let node_ids = self.all_surface_node_ids(graph);
        let mut node_candidates = Vec::new();
        let mut failed_node_ids = Vec::new();
        for &node_id in &node_ids {
            let Ok(expected_kind) = staging.expected_visual_node_piece_kind(graph, node_id) else {
                failed_node_ids.push(node_id);
                continue;
            };
            let Some((expected_kind, expected_mouth_count)) = expected_kind else {
                continue;
            };
            if staging.structural_terminal_piece_is_span_owned(graph, node_id, expected_kind) {
                continue;
            }
            let Some(input) = staging.visual_node_compile_input(graph, node_id) else {
                failed_node_ids.push(node_id);
                continue;
            };
            if input.kind != expected_kind || input.mouths.len() != expected_mouth_count {
                failed_node_ids.push(node_id);
                continue;
            }
            node_candidates.push((node_id, input));
        }
        let node_results: Vec<(
            u32,
            RoadSurfaceVisualNodeCompileInput,
            Option<NodeVisualCompileResult>,
        )> = Self::collect_node_compile_work(&node_candidates, |node_id| {
            (
                node_id.0,
                node_id.1.clone(),
                staging.compile_visual_node_piece_with_earthwork_boundaries(
                    graph, terrain, node_id.0, &node_id.1, None,
                ),
            )
        });
        failed_node_ids.extend(
            node_results
                .iter()
                .filter_map(|result| result.2.is_none().then_some(result.0)),
        );
        failed_node_ids.sort_unstable();
        failed_node_ids.dedup();
        let nodes_ms = elapsed_ms(nodes_start);
        if !failed_node_ids.is_empty() {
            let failure_label = format!(
                "stage=all_nodes compile_reason={} failed_nodes={:?} edges={} nodes={} node_candidates={}",
                reason.as_str(),
                failed_node_ids,
                edge_ids.len(),
                node_ids.len(),
                node_candidates.len()
            );
            self.latch_compile_failure(failure_label.clone());
            self.last_failed_span_ids.clear();
            self.last_failed_node_ids = failed_node_ids.clone();
            if road_debug {
                crate::debug_log!(
                    "road",
                    "surface_compile_all_incomplete {} total_ms={:.3}",
                    failure_label,
                    elapsed_ms(total_start)
                );
            }
            return;
        }
        for (node_id, input, visual_piece) in node_results {
            staging.apply_node_compile_result_with_earthwork_boundaries(
                node_id,
                input,
                visual_piece,
            );
        }

        let chunk_cache_start = road_debug.then(Instant::now);
        staging
            .dirty_surface_chunks
            .extend(self.dirty_surface_chunks.iter().copied());
        staging
            .dirty_surface_chunks
            .extend(self.surface_chunk_cache.keys().copied());
        staging
            .dirty_terrain_chunks
            .extend(self.dirty_terrain_chunks.iter().copied());
        staging
            .dirty_terrain_chunks
            .extend(self.earthwork_chunk_cache.keys().copied());
        staging
            .dirty_query_chunks
            .extend(self.dirty_query_chunks.iter().copied());
        staging
            .dirty_query_chunks
            .extend(self.query_chunk_spans.keys().copied());
        staging
            .dirty_query_chunks
            .extend(self.query_chunk_nodes.keys().copied());
        let rebuilt_surface_chunks = staging.sorted_chunk_keys(&staging.dirty_surface_chunks);
        let rebuilt_terrain_chunks = staging.sorted_chunk_keys(&staging.dirty_terrain_chunks);
        let rebuilt_query_chunks = staging.sorted_chunk_keys(&staging.dirty_query_chunks);
        staging.rebuild_surface_chunk_cache(&rebuilt_surface_chunks);
        staging.rebuild_earthwork_chunk_cache(&rebuilt_terrain_chunks);
        let chunk_cache_ms = elapsed_ms(chunk_cache_start);
        staging.last_rebuilt_surface_chunks = rebuilt_surface_chunks;
        staging.last_rebuilt_terrain_chunks = rebuilt_terrain_chunks;
        staging.last_rebuilt_query_chunks = rebuilt_query_chunks;
        staging.compiled_once = true;
        staging.compile_invalidation_generation = self.compile_invalidation_generation;
        staging.failed_compile_generation = None;
        staging.last_compile_failure_label = None;
        staging.last_failed_span_ids.clear();
        staging.last_failed_node_ids.clear();
        staging.clear_dirty_tracking();
        *self = staging;

        if road_debug {
            let total_ms = elapsed_ms(total_start);
            if total_ms >= 50.0 {
                crate::debug_log!(
                    "road",
                    "surface_compile_all_detail compile_reason={} edges={} nodes={} rebuilt_surface_chunks={} rebuilt_terrain_chunks={} staging_ms={:.3} edge_collect_ms={:.3} sections_ms={:.3} spans_ms={:.3} nodes_ms={:.3} chunk_cache_ms={:.3} total_ms={:.3}",
                    reason.as_str(),
                    edge_ids.len(),
                    node_ids.len(),
                    self.last_rebuilt_surface_chunks.len(),
                    self.last_rebuilt_terrain_chunks.len(),
                    staging_ms,
                    edge_ms,
                    sections_ms,
                    spans_ms,
                    nodes_ms,
                    chunk_cache_ms,
                    total_ms
                );
            }
        }
    }

    pub(in crate::simulation::network::surface) fn apply_span_compile_result(
        &mut self,
        edge_idx: usize,
        span_piece: Option<RoadSurfaceVisualSpanPiece>,
    ) {
        self.remove_span_piece_coverage(edge_idx);
        if let Some(span_piece) = span_piece {
            self.insert_span_piece_coverage(&span_piece);
            self.compiled_visual_span_pieces
                .insert(edge_idx, span_piece);
        } else {
            self.compiled_visual_span_pieces.remove(&edge_idx);
        }
    }

    #[cfg(test)]
    pub(in crate::simulation::network::surface) fn apply_node_compile_result(
        &mut self,
        node_id: u32,
        input: RoadSurfaceVisualNodeCompileInput,
        visual_piece: Option<RoadSurfaceVisualNodePiece>,
    ) {
        self.apply_node_compile_result_with_earthwork_boundaries(
            node_id,
            input,
            visual_piece.map(|piece| NodeVisualCompileResult {
                piece,
                earthwork_boundaries: Arc::new(Vec::new()),
                topology_cache: None,
                rail_topology_reused: false,
                ownership_reused: false,
                export_reuse_stats: Default::default(),
            }),
        );
    }

    fn apply_node_compile_result_with_earthwork_boundaries(
        &mut self,
        node_id: u32,
        input: RoadSurfaceVisualNodeCompileInput,
        visual_piece: Option<NodeVisualCompileResult>,
    ) {
        self.remove_node_piece_coverage(node_id);
        if let Some(visual_piece) = visual_piece {
            self.insert_node_piece_coverage(&visual_piece.piece);
            self.compiled_visual_node_pieces
                .insert(node_id, visual_piece.piece);
            self.compiled_visual_node_inputs.insert(node_id, input);
            if visual_piece.earthwork_boundaries.is_empty() {
                self.compiled_visual_node_earthwork_boundaries
                    .remove(&node_id);
            } else {
                self.compiled_visual_node_earthwork_boundaries
                    .insert(node_id, visual_piece.earthwork_boundaries);
            }
            if let Some(topology) = visual_piece.topology_cache {
                self.compiled_visual_node_topologies
                    .insert(node_id, topology);
            } else {
                self.compiled_visual_node_topologies.remove(&node_id);
            }
        } else {
            self.compiled_visual_node_pieces.remove(&node_id);
            self.compiled_visual_node_inputs.remove(&node_id);
            self.compiled_visual_node_earthwork_boundaries
                .remove(&node_id);
            self.compiled_visual_node_topologies.remove(&node_id);
        }
    }

    pub(crate) fn collect_surface_compile_work<I, O, F>(items: &[I], work: F) -> Vec<O>
    where
        I: Clone + Send + Sync,
        O: Send,
        F: Fn(I) -> O + Sync,
    {
        // Slice parallel iterators are indexed; collecting into Vec preserves input order, so
        // the serial commit phase remains deterministic without re-sorting by id.
        if items.len() >= PARALLEL_SURFACE_COMPILE_MIN_ITEMS {
            items.par_iter().cloned().map(&work).collect()
        } else {
            items.iter().cloned().map(&work).collect()
        }
    }

    pub(crate) fn collect_node_compile_work<I, O, F>(items: &[I], work: F) -> Vec<O>
    where
        I: Clone + Send + Sync,
        O: Send,
        F: Fn(I) -> O + Sync,
    {
        // Node compilation dominates road-edit latency; two independent dirty nodes are already
        // worth Rayon scheduling overhead. Indexed collection keeps commit order deterministic.
        if items.len() >= PARALLEL_NODE_COMPILE_MIN_ITEMS {
            items.par_iter().cloned().map(&work).collect()
        } else {
            items.iter().cloned().map(&work).collect()
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
