// SPDX-License-Identifier: GPL-2.0-only

//! `RoadbedSystem` state: published pieces, the chunk index, dirty marks and the source /
//! compiled generation gate.

use super::index::{PieceIndex, RenderGrid};
use super::query::PieceId;
use super::span::{SpanPiece, SpanScratch, build_span_piece};
use super::{RoadSection, SurfaceChunkKey};
use crate::simulation::network::graph::RegionGraph;
use rayon::prelude::*;
use std::collections::BTreeSet;
use std::sync::Arc;

// A published piece with the generation that built it.
#[derive(Clone)]
pub(super) struct PublishedPiece {
    pub(super) build_generation: u64,
    pub(super) span: Arc<SpanPiece>,
}

/// Roadbed pieces derived from the road graph, with their chunk index and generation gate.
///
/// Owned by `TransitNetwork`. Graph edits mark edges and nodes dirty; [`Self::compile_dirty`]
/// rebuilds the pieces of their dirty closure and publishes a new generation. Clones share every
/// piece and index node, so a snapshot costs `O(1)`.
#[derive(Clone)]
pub struct RoadbedSystem {
    grid: RenderGrid,
    // False until the first compile; a fresh system has nothing published.
    compiled: bool,
    // Advances once per compile that had work; rebuilt pieces take the new value.
    generation: u64,
    // Sorted, so compiles visit dirty owners in a deterministic order.
    dirty_edges: BTreeSet<usize>,
    dirty_nodes: BTreeSet<u32>,
    pub(super) pieces: imbl::OrdMap<PieceId, PublishedPiece>,
    pub(super) index: PieceIndex,
    // Render chunks changed since the mesh publisher last took them.
    changed_chunks: BTreeSet<SurfaceChunkKey>,
}

impl RoadbedSystem {
    /// Creates an empty roadbed whose render chunks span `chunk_span_m` metres from the origin.
    pub fn new(chunk_span_m: f32) -> Self {
        Self::new_with_chunk_grid(chunk_span_m, 0.0, 0.0)
    }

    /// Creates an empty roadbed with an explicit world-space chunk-grid origin.
    pub fn new_with_chunk_grid(chunk_span_m: f32, origin_x_m: f32, origin_z_m: f32) -> Self {
        let finite_or_zero = |value: f32| if value.is_finite() { value } else { 0.0 };
        Self {
            grid: RenderGrid {
                span_m: chunk_span_m.max(f32::EPSILON),
                origin_m: (finite_or_zero(origin_x_m), finite_or_zero(origin_z_m)),
            },
            compiled: false,
            generation: 0,
            dirty_edges: BTreeSet::new(),
            dirty_nodes: BTreeSet::new(),
            pieces: imbl::OrdMap::new(),
            index: PieceIndex::default(),
            changed_chunks: BTreeSet::new(),
        }
    }

    /// Drops every piece and dirty mark; the next compile rebuilds everything from the graph.
    /// The dropped pieces' chunks count as changed.
    pub fn clear(&mut self) {
        self.changed_chunks.extend(self.index.occupied_chunks());
        self.compiled = false;
        self.generation = self.generation.wrapping_add(1);
        self.dirty_edges.clear();
        self.dirty_nodes.clear();
        self.pieces = imbl::OrdMap::new();
        self.index = PieceIndex::default();
    }

    /// Marks an edge's span piece, and through it the dirty closure, for the next compile.
    pub fn mark_edge_dirty(&mut self, edge_idx: usize) {
        self.dirty_edges.insert(edge_idx);
    }

    /// Marks a node's piece, and through it the dirty closure, for the next compile.
    pub fn mark_node_dirty(&mut self, node_id: u32) {
        self.dirty_nodes.insert(node_id);
    }

    /// Nodes marked dirty since the last compile, in ascending order.
    pub fn dirty_nodes(&self) -> &BTreeSet<u32> {
        &self.dirty_nodes
    }

    /// Edges marked dirty since the last compile, in ascending order.
    #[cfg(test)]
    pub(crate) fn dirty_edges(&self) -> &BTreeSet<usize> {
        &self.dirty_edges
    }

    /// Rebuilds the pieces of the dirty closure, publishes a new generation and returns the
    /// sorted render chunks this compile changed; they also accumulate in
    /// [`Self::changed_chunks`] until the mesh publisher takes them. A compile with nothing
    /// pending changes nothing. The first compile of a fresh or cleared system builds every
    /// piece.
    ///
    /// In `ROAD-68-B` the closure is the dirty edges plus the edges incident to dirty nodes, and
    /// only span pieces exist. Spans build in parallel on the shared pool and publish in
    /// ascending edge order, so the result does not depend on the worker count. `O(closure spans * sections * bands)`.
    pub fn compile_dirty(&mut self, graph: &RegionGraph) -> Vec<SurfaceChunkKey> {
        if self.is_current() {
            return Vec::new();
        }
        let closure = self.dirty_closure(graph);
        let grid = self.grid;
        // The shared pool, never Rayon's global one (`CODE-17`); inline on a pool worker.
        let built: Vec<_> = crate::thread_pools::install(|| {
            closure
                .par_iter()
                .map_init(SpanScratch::default, |scratch, &edge_idx| {
                    build_span_piece(graph, edge_idx, grid, scratch)
                })
                .collect()
        });
        self.generation = self.generation.wrapping_add(1);
        let mut changed = BTreeSet::new();
        for (&edge_idx, span) in closure.iter().zip(built) {
            self.replace_piece(PieceId::Span(edge_idx), span, &mut changed);
        }
        self.changed_chunks.extend(changed.iter().copied());
        self.dirty_edges.clear();
        self.dirty_nodes.clear();
        self.compiled = true;
        changed.into_iter().collect()
    }

    /// Builds one edge's span piece without publishing it, for benchmarks and isolated checks;
    /// `None` when the edge has no span.
    pub fn build_span_piece(&self, graph: &RegionGraph, edge_idx: usize) -> Option<SpanPiece> {
        build_span_piece(graph, edge_idx, self.grid, &mut SpanScratch::default())
    }

    // Sorted closure: dirty edges, edges incident to dirty nodes and published spans of edges
    // the graph no longer has; every surface edge on the first compile.
    fn dirty_closure(&self, graph: &RegionGraph) -> Vec<usize> {
        if !self.compiled {
            return (0..graph.edge_count()).collect();
        }
        let mut closure = self.dirty_edges.clone();
        for &node_id in &self.dirty_nodes {
            let node_id = graph.get_valid_node(node_id);
            if (node_id as usize) < graph.node_adjacency_count() {
                closure.extend(graph.node_adjacency(node_id).iter().copied());
            }
        }
        closure.extend(
            self.pieces
                .range(PieceId::Span(graph.edge_count())..PieceId::Node(0))
                .filter_map(|(id, _)| match id {
                    PieceId::Span(edge_idx) => Some(*edge_idx),
                    PieceId::Node(_) => None,
                }),
        );
        closure.into_iter().collect()
    }

    fn replace_piece(
        &mut self,
        id: PieceId,
        span: Option<SpanPiece>,
        changed: &mut BTreeSet<SurfaceChunkKey>,
    ) {
        if let Some(old) = self.pieces.remove(&id) {
            self.index.remove(id, old.span.footprint());
            changed.extend(old.span.footprint().render_chunks().iter().copied());
        }
        let Some(span) = span else {
            return;
        };
        self.index.insert(id, span.shared_mesh(), span.footprint());
        changed.extend(span.footprint().render_chunks().iter().copied());
        self.pieces.insert(
            id,
            PublishedPiece {
                build_generation: self.generation,
                span: Arc::new(span),
            },
        );
    }

    /// Sorted render chunks changed by compiles since the mesh publisher last took them.
    pub fn changed_chunks(&self) -> impl Iterator<Item = SurfaceChunkKey> + '_ {
        self.changed_chunks.iter().copied()
    }

    /// Forgets the changed chunks once the mesh publisher has rebuilt them.
    pub fn clear_changed_chunks(&mut self) {
        self.changed_chunks.clear();
    }

    /// Sorted render chunks that hold any published piece.
    pub fn occupied_chunks(&self) -> impl Iterator<Item = SurfaceChunkKey> + '_ {
        self.index.occupied_chunks()
    }

    /// True when published pieces match the graph: compiled once, with no dirty marks pending.
    pub fn is_current(&self) -> bool {
        self.compiled && self.dirty_edges.is_empty() && self.dirty_nodes.is_empty()
    }

    /// Published generation; it changes whenever published pieces may have changed.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Nodes whose pieces failed to build in the published generation, in ascending order.
    /// Node pieces arrive in `ROAD-68-C`.
    pub fn failed_nodes(&self) -> &[u32] {
        &[]
    }

    /// Span of one render chunk, in metres.
    pub fn chunk_span_m(&self) -> f32 {
        self.grid.span_m
    }

    /// World-space minimum corner from which render chunk keys are measured.
    pub fn chunk_origin_m(&self) -> (f32, f32) {
        self.grid.origin_m
    }

    /// Render chunk containing a world XZ point.
    pub fn chunk_key_at(&self, x: f64, z: f64) -> SurfaceChunkKey {
        self.grid.key_at(x, z)
    }

    /// World XZ bounds `[min_x, min_z, max_x, max_z]` of one render chunk.
    pub fn chunk_bounds(&self, key: SurfaceChunkKey) -> [f64; 4] {
        let span = f64::from(self.grid.span_m);
        let min_x = f64::from(self.grid.origin_m.0) + f64::from(key.0) * span;
        let min_z = f64::from(self.grid.origin_m.1) + f64::from(key.1) * span;
        [min_x, min_z, min_x + span, min_z + span]
    }

    /// Sampled cross-sections of an edge's span piece, from its start mouth to its end mouth;
    /// `None` when the edge has no span piece.
    pub fn sections(&self, edge_idx: usize) -> Option<&[RoadSection]> {
        self.span_piece(edge_idx).map(SpanPiece::sections)
    }

    /// The published span piece of an edge.
    pub fn span_piece(&self, edge_idx: usize) -> Option<&SpanPiece> {
        self.pieces
            .get(&PieceId::Span(edge_idx))
            .map(|published| published.span.as_ref())
    }
}
