// SPDX-License-Identifier: GPL-2.0-only

//! `RoadbedSystem` state: published pieces, the chunk index, dirty marks and the source /
//! compiled generation gate.

use super::index::{PieceFootprint, PieceIndex, RenderGrid};
use super::layout::{ArmKey, EdgeEnd, NodeKind};
use super::mesh::PieceMesh;
use super::node::{NodeError, NodePiece, NodePlan, NodeScratch, build_node_piece, plan_node};
use super::query::PieceId;
use super::span::{MouthSection, SpanPiece, SpanScratch, build_span_piece};
use super::{RoadSection, SurfaceChunkKey};
use crate::simulation::network::graph::RegionGraph;
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

// Two mouths of one edge closer than this share one section and leave no span (the span
// model's station tolerance).
const SHARED_MOUTH_M: f32 = 0.001;

// A closure of at most this many pieces (an edit's, a preview's) builds on the calling thread:
// waking pool workers costs more than a handful of pieces (`ROAD-68-E` stage timings). Larger
// closures (loads, terrain syncs) build on the shared pool. Same functions, same order, so the
// products do not depend on it.
const INLINE_CLOSURE_PIECES: usize = 16;

// A published piece.
#[derive(Clone)]
pub(super) enum Piece {
    Span(Arc<SpanPiece>),
    Node(Arc<NodePiece>),
}

// A published piece with the generation that built it.
#[derive(Clone)]
pub(super) struct PublishedPiece {
    pub(super) build_generation: u64,
    pub(super) piece: Piece,
}

impl PublishedPiece {
    pub(super) fn mesh(&self) -> &PieceMesh {
        self.parts().0
    }

    fn parts(&self) -> (&Arc<PieceMesh>, &PieceFootprint) {
        match &self.piece {
            Piece::Span(span) => (span.shared_mesh(), span.footprint()),
            Piece::Node(node) => (node.shared_mesh(), node.footprint()),
        }
    }
}

// This compile's node plans, by node.
type Plans = BTreeMap<u32, Result<Option<NodePlan>, NodeError>>;

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
    // XZ bounds of the old and new pieces replaced since terrain last took them (`ROAD-68-D`).
    changed_bounds: Vec<[f64; 4]>,
    // Nodes whose piece failed in the published generation, with the reason, and their ids.
    failed: imbl::OrdMap<u32, NodeError>,
    failed_ids: Arc<Vec<u32>>,
    // Owners whose pieces installs replaced since the road tool last took them (`ROAD-68-E`).
    rebuilt: RebuiltOwners,
}

/// Owners (nodes and edges) whose pieces were rebuilt, swapped or dropped since the last
/// [`RoadbedSystem::take_rebuilt_owners`] (`ROAD-68-E`). Every graph edit that can change a
/// piece marks its owners dirty and compiles, so these are the graph records a replica of the
/// graph must copy to match it in every value a piece reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RebuiltOwners {
    /// Every owner, after the first compile or a clear.
    pub all: bool,
    /// Nodes in ascending order.
    pub nodes: BTreeSet<u32>,
    /// Edges in ascending order.
    pub edges: BTreeSet<usize>,
}

impl RebuiltOwners {
    /// True when nothing was rebuilt.
    pub fn is_empty(&self) -> bool {
        !self.all && self.nodes.is_empty() && self.edges.is_empty()
    }
}

/// The pieces one compile built for its dirty closure, not yet installed (`ROAD-68-E`).
///
/// A preview compiles it on an `O(1)` copy of the published roadbed and the edited graph; the
/// commit installs the same pieces, without rebuilding them, when its roadbed still stands at
/// the generation and dirty marks the delta was compiled from ([`RoadbedSystem::can_install`]).
#[derive(Clone)]
pub struct RoadbedDelta {
    base_generation: u64,
    base_compiled: bool,
    dirty_edges: BTreeSet<usize>,
    dirty_nodes: BTreeSet<u32>,
    spans: Vec<(usize, Option<Arc<SpanPiece>>)>,
    nodes: Vec<(u32, BuiltNode)>,
}

// A closure node's built piece, `None` for a node without one, or why it failed.
type BuiltNode = Result<Option<Arc<NodePiece>>, NodeError>;

impl RoadbedDelta {
    /// Generation of the roadbed the delta was compiled from.
    pub fn base_generation(&self) -> u64 {
        self.base_generation
    }

    /// The rebuilt span and node pieces, by owner in install order; `None` where the owner has
    /// no piece. A failed node is absent.
    pub fn pieces(&self) -> impl Iterator<Item = (PieceId, Option<&PieceMesh>)> + '_ {
        let spans = self
            .spans
            .iter()
            .map(|(edge, span)| (PieceId::Span(*edge), span.as_deref().map(SpanPiece::mesh)));
        let nodes = self.nodes.iter().filter_map(|(node, built)| {
            let piece = built.as_ref().ok()?;
            Some((PieceId::Node(*node), piece.as_deref().map(NodePiece::mesh)))
        });
        spans.chain(nodes)
    }

    /// Nodes whose piece failed in this compile, with the reason, in ascending order.
    pub fn failed_nodes(&self) -> impl Iterator<Item = (u32, NodeError)> + '_ {
        self.nodes
            .iter()
            .filter_map(|(node, built)| built.as_ref().err().map(|error| (*node, *error)))
    }
}

/// What one install replaced: undo swaps it back (`ROAD-68-E`). The restored pieces are the
/// pre-edit `Arc`s themselves, published in the restoring generation, so every consumer that
/// follows build generations (render publication, terrain) sees them as rebuilt.
#[derive(Clone)]
pub struct RoadbedRestore {
    installed_generation: u64,
    // The piece each owner held before the install, and whether the install published one.
    pieces: Vec<(PieceId, Option<PublishedPiece>, bool)>,
    // Each closure node's failure before and after the install.
    failures: Vec<(u32, Option<NodeError>, Option<NodeError>)>,
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
            changed_bounds: Vec::new(),
            failed: imbl::OrdMap::new(),
            failed_ids: Arc::default(),
            rebuilt: RebuiltOwners::default(),
        }
    }

    /// An `O(1)` copy of the published pieces without the change logs (render chunks, terrain
    /// bounds and rebuilt owners), for the road tool's context (`ROAD-68-E`).
    pub fn snapshot(&self) -> Self {
        Self {
            changed_chunks: BTreeSet::new(),
            changed_bounds: Vec::new(),
            rebuilt: RebuiltOwners::default(),
            dirty_edges: self.dirty_edges.clone(),
            dirty_nodes: self.dirty_nodes.clone(),
            pieces: self.pieces.clone(),
            index: self.index.clone(),
            failed: self.failed.clone(),
            failed_ids: Arc::clone(&self.failed_ids),
            ..*self
        }
    }

    /// Records that every owner counts as rebuilt, for edits that rewrite the whole graph
    /// (terrain sync, load), so the road tool copies the graph once instead of record by record.
    pub fn note_full_rebuild(&mut self) {
        self.rebuilt.all = true;
    }

    /// Owners rebuilt since the last call; taking them clears the list.
    pub fn take_rebuilt_owners(&mut self) -> RebuiltOwners {
        std::mem::take(&mut self.rebuilt)
    }

    /// Drops every piece and dirty mark; the next compile rebuilds everything from the graph.
    /// The dropped pieces' chunks count as changed.
    pub fn clear(&mut self) {
        self.changed_chunks.extend(self.index.occupied_chunks());
        let dropped = self.pieces.values().map(|piece| piece.mesh().bounds_xz());
        self.changed_bounds.extend(dropped);
        self.compiled = false;
        self.generation = self.generation.wrapping_add(1);
        self.dirty_edges.clear();
        self.dirty_nodes.clear();
        self.pieces = imbl::OrdMap::new();
        self.index = PieceIndex::default();
        self.failed = imbl::OrdMap::new();
        self.failed_ids = Arc::default();
        self.rebuilt.all = true;
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
    /// The closure is the dirty edges, every node incident to them and every edge incident to
    /// those nodes, whose mouths may move. Nodes are planned first (layout, plane and mouth
    /// sections), then node pieces and the spans between their mouths build, all in parallel on
    /// the shared pool and published in ascending piece order, so the result does not depend on
    /// the worker count. `O(closure nodes * node piece + closure spans * sections * bands)`.
    pub fn compile_dirty(&mut self, graph: &RegionGraph) -> Vec<SurfaceChunkKey> {
        if self.is_current() {
            return Vec::new();
        }
        let delta = self.compile_delta(graph);
        self.install(&delta).0
    }

    /// Builds the pieces [`Self::compile_dirty`] would publish, without publishing them: the
    /// same closure, plans and parallel builds, so installing the delta equals that compile
    /// bitwise. Empty when nothing is pending.
    pub fn compile_delta(&self, graph: &RegionGraph) -> RoadbedDelta {
        let mut delta = RoadbedDelta {
            base_generation: self.generation,
            base_compiled: self.compiled,
            dirty_edges: self.dirty_edges.clone(),
            dirty_nodes: self.dirty_nodes.clone(),
            spans: Vec::new(),
            nodes: Vec::new(),
        };
        if self.is_current() {
            return delta;
        }
        let (nodes, edges) = self.dirty_closure(graph);
        let mut plans = plan_nodes(graph, &nodes);
        // An edge whose two mouths coincide shares one mouth section; its end node rebuilds with
        // the start node's.
        let far: BTreeSet<u32> = edges
            .iter()
            .filter_map(|&edge_idx| self.shared_mouth(graph, edge_idx, &plans))
            .map(|(end_node, _)| end_node)
            .filter(|end_node| !plans.contains_key(end_node))
            .collect();
        plans.extend(plan_nodes(graph, &far));
        for &edge_idx in &edges {
            let Some((end_node, start)) = self.shared_mouth(graph, edge_idx, &plans) else {
                continue;
            };
            if let Some(Ok(Some(plan))) = plans.get_mut(&end_node) {
                let (edge, end) = (edge_idx, EdgeEnd::End);
                plan.share_mouth(ArmKey { edge, end }, start);
            }
        }
        let span_mouths: Vec<_> = edges
            .iter()
            .map(|&edge_idx| {
                self.edge_mouths(graph, edge_idx, &plans)
                    .unwrap_or_default()
            })
            .collect();
        let grid = self.grid;
        let span = |scratch: &mut SpanScratch, (&edge_idx, mouths): (&usize, &[Option<_>; 2])| {
            build_span_piece(
                graph,
                edge_idx,
                [&mouths[0], &mouths[1]].map(Option::as_ref),
                grid,
                scratch,
            )
        };
        let node = |scratch: &mut NodeScratch, (node, plan): (u32, Result<Option<NodePlan>, _>)| {
            let built = match plan {
                Ok(Some(plan)) => build_node_piece(plan, grid, scratch).map(Some),
                Ok(None) => Ok(None),
                Err(error) => Err(error),
            };
            (node, built)
        };
        let (built_spans, built_nodes): (Vec<_>, Vec<_>) =
            if edges.len() + plans.len() <= INLINE_CLOSURE_PIECES {
                let (mut span_scratch, mut node_scratch) = Default::default();
                (
                    edges
                        .iter()
                        .zip(&span_mouths)
                        .map(|item| span(&mut span_scratch, item))
                        .collect(),
                    plans
                        .into_iter()
                        .map(|item| node(&mut node_scratch, item))
                        .collect(),
                )
            } else {
                // The shared pool, never Rayon's global one (`CODE-17`); inline on a pool worker.
                crate::thread_pools::install(|| {
                    let spans = edges
                        .par_iter()
                        .zip(&span_mouths)
                        .map_init(SpanScratch::default, span)
                        .collect();
                    let nodes = plans.into_par_iter().map_init(NodeScratch::default, node).collect();
                    (spans, nodes)
                })
            };
        delta.spans = edges
            .into_iter()
            .zip(built_spans)
            .map(|(edge_idx, span)| (edge_idx, span.map(Arc::new)))
            .collect();
        delta.nodes = built_nodes
            .into_iter()
            .map(
                |(node, built): (u32, Result<Option<NodePiece>, NodeError>)| {
                    (node, built.map(|piece| piece.map(Arc::new)))
                },
            )
            .collect();
        delta
    }

    /// True when this roadbed stands where `delta` was compiled: the same published generation
    /// and dirty marks, so installing it equals compiling here.
    pub fn can_install(&self, delta: &RoadbedDelta) -> bool {
        self.generation == delta.base_generation
            && self.compiled == delta.base_compiled
            && self.dirty_edges == delta.dirty_edges
            && self.dirty_nodes == delta.dirty_nodes
    }

    /// Publishes a delta's pieces as one compile would: returns the render chunks they changed
    /// (also accumulated for the mesh publisher) and what they replaced, for undo. The caller
    /// checks [`Self::can_install`] first. `O(rebuilt pieces * log pieces)`.
    pub fn install(&mut self, delta: &RoadbedDelta) -> (Vec<SurfaceChunkKey>, RoadbedRestore) {
        debug_assert!(
            self.can_install(delta),
            "a roadbed delta installs where it was compiled"
        );
        if self.is_current() {
            let generation = self.generation;
            let restore = RoadbedRestore {
                installed_generation: generation,
                pieces: Vec::new(),
                failures: Vec::new(),
            };
            return (Vec::new(), restore);
        }
        self.generation = self.generation.wrapping_add(1);
        if !self.compiled {
            self.rebuilt.all = true;
        }
        let mut changed = BTreeSet::new();
        let mut replaced = Vec::with_capacity(delta.spans.len() + delta.nodes.len());
        for (edge_idx, span) in &delta.spans {
            let id = PieceId::Span(*edge_idx);
            let piece = span.as_ref().map(|span| Piece::Span(Arc::clone(span)));
            let published = piece.is_some();
            let old = self.replace_piece(id, piece, &mut changed);
            replaced.push((id, old, published));
            self.rebuilt.edges.insert(*edge_idx);
        }
        let mut failures = Vec::with_capacity(delta.nodes.len());
        let mut failures_changed = false;
        for (node, built) in &delta.nodes {
            let before = self.failed.remove(node);
            failures_changed |= before.is_some();
            let piece = match built {
                Ok(piece) => piece.as_ref().map(|piece| Piece::Node(Arc::clone(piece))),
                Err(error) => {
                    self.failed.insert(*node, *error);
                    failures_changed = true;
                    None
                }
            };
            failures.push((*node, before, built.as_ref().err().copied()));
            let id = PieceId::Node(*node);
            let published = piece.is_some();
            let old = self.replace_piece(id, piece, &mut changed);
            replaced.push((id, old, published));
            self.rebuilt.nodes.insert(*node);
        }
        if failures_changed {
            self.failed_ids = Arc::new(self.failed.keys().copied().collect());
        }
        self.changed_chunks.extend(changed.iter().copied());
        self.dirty_edges.clear();
        self.dirty_nodes.clear();
        self.compiled = true;
        let restore = RoadbedRestore {
            installed_generation: self.generation,
            pieces: replaced,
            failures,
        };
        (changed.into_iter().collect(), restore)
    }

    /// True when every owner a restore covers still holds what its install published, so
    /// swapping the restore back equals recompiling the restored graph.
    pub fn can_restore(&self, restore: &RoadbedRestore) -> bool {
        self.is_current()
            && restore.pieces.iter().all(|(id, _, published)| {
                let current = self.pieces.get(id).map(|piece| piece.build_generation);
                current == published.then_some(restore.installed_generation)
            })
            && restore
                .failures
                .iter()
                .all(|(node, _, after)| self.failed.get(node).copied() == *after)
    }

    /// Swaps an install's replaced pieces and failures back as a new generation, which the
    /// restored pieces take as their build generation. The caller checks [`Self::can_restore`]
    /// first. `O(restored pieces * log pieces)`.
    pub fn restore(&mut self, restore: &RoadbedRestore) {
        debug_assert!(
            self.can_restore(restore),
            "a restore swaps back its own install"
        );
        if restore.pieces.is_empty() {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        let mut changed = BTreeSet::new();
        for (id, old, _) in &restore.pieces {
            let piece = old.as_ref().map(|old| old.piece.clone());
            self.replace_piece(*id, piece, &mut changed);
            match id {
                PieceId::Span(edge_idx) => self.rebuilt.edges.insert(*edge_idx),
                PieceId::Node(node) => self.rebuilt.nodes.insert(*node),
            };
        }
        for (node, before, _) in &restore.failures {
            match before {
                Some(error) => self.failed.insert(*node, *error),
                None => self.failed.remove(node),
            };
        }
        if !restore.failures.is_empty() {
            self.failed_ids = Arc::new(self.failed.keys().copied().collect());
        }
        self.changed_chunks.extend(changed);
    }

    // The node mouth sections at an edge's two ends: from this compile's plans, else from the
    // published node pieces; `None` at an end without a node piece. `None` overall for a missing
    // edge.
    fn edge_mouths(
        &self,
        graph: &RegionGraph,
        edge_idx: usize,
        plans: &Plans,
    ) -> Option<[Option<Arc<MouthSection>>; 2]> {
        let edge = graph.get_edge(edge_idx)?;
        let mouth = |node: u32, end: EdgeEnd| {
            let node = graph.get_valid_node(node);
            let key = ArmKey {
                edge: edge_idx,
                end,
            };
            match plans.get(&node) {
                Some(Ok(Some(plan))) => plan.mouth(key).cloned(),
                Some(_) => None,
                None => self
                    .node_piece(node)
                    .and_then(|piece| piece.mouth(key).cloned()),
            }
        };
        Some([
            mouth(edge.start_node, EdgeEnd::Start),
            mouth(edge.end_node, EdgeEnd::End),
        ])
    }

    // The end node and start mouth of an edge whose two mouths lie within `SHARED_MOUTH_M`.
    fn shared_mouth(
        &self,
        graph: &RegionGraph,
        edge_idx: usize,
        plans: &Plans,
    ) -> Option<(u32, Arc<MouthSection>)> {
        let [start, end] = self.edge_mouths(graph, edge_idx, plans)?;
        let (start, end) = (start?, end?);
        let end_node = graph.get_valid_node(graph.edge(edge_idx).end_node);
        (end.section.s_m - start.section.s_m <= SHARED_MOUTH_M).then_some((end_node, start))
    }

    /// Builds one edge's span piece between the published node pieces' mouths without
    /// publishing it, for benchmarks and isolated checks; `None` when the edge has no span.
    pub fn build_span_piece(&self, graph: &RegionGraph, edge_idx: usize) -> Option<SpanPiece> {
        let mouths = self
            .edge_mouths(graph, edge_idx, &Plans::new())
            .unwrap_or_default();
        build_span_piece(
            graph,
            edge_idx,
            [&mouths[0], &mouths[1]].map(Option::as_ref),
            self.grid,
            &mut SpanScratch::default(),
        )
    }

    /// Plans and builds one node's piece without publishing it, for benchmarks and isolated
    /// checks; `Ok(None)` when the node has no piece.
    pub fn build_node_piece(
        &self,
        graph: &RegionGraph,
        node: u32,
    ) -> Result<Option<NodePiece>, NodeError> {
        plan_node(graph, node)?
            .map(|plan| build_node_piece(plan, self.grid, &mut NodeScratch::default()))
            .transpose()
    }

    // Sorted closure nodes and edges: the dirty nodes and the nodes of dirty edges, the edges
    // incident to them, the dirty edges, and published pieces the graph no longer has; every node
    // and edge on the first compile.
    fn dirty_closure(&self, graph: &RegionGraph) -> (BTreeSet<u32>, Vec<usize>) {
        let node_limit = graph.node_adjacency_count().min(graph.node_count()) as u32;
        if !self.compiled {
            let nodes = (0..node_limit)
                .filter(|&node| graph.get_valid_node(node) == node)
                .collect();
            return (nodes, (0..graph.edge_count()).collect());
        }
        let mut nodes = self.dirty_nodes.clone();
        for &edge_idx in &self.dirty_edges {
            if let Some(edge) = graph.get_edge(edge_idx) {
                nodes.insert(graph.get_valid_node(edge.start_node));
                nodes.insert(graph.get_valid_node(edge.end_node));
            }
            // A reconnected or deleted edge's previous nodes rebuild too.
            if let Some(span) = self.span_piece(edge_idx) {
                nodes.extend(span.nodes());
            }
        }
        nodes.extend(self.pieces.range(PieceId::Node(node_limit)..).filter_map(
            |(id, _)| match id {
                PieceId::Node(node) => Some(*node),
                PieceId::Span(_) => None,
            },
        ));
        let mut closure = self.dirty_edges.clone();
        for &node in &nodes {
            let valid = graph.get_valid_node(node);
            if valid < node_limit {
                closure.extend(graph.node_adjacency(valid).iter().copied());
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
        (nodes, closure.into_iter().collect())
    }

    // Publishes a piece built in this generation in place of the owner's current one, which
    // it returns.
    fn replace_piece(
        &mut self,
        id: PieceId,
        piece: Option<Piece>,
        changed: &mut BTreeSet<SurfaceChunkKey>,
    ) -> Option<PublishedPiece> {
        let build_generation = self.generation;
        let published = piece.map(|piece| PublishedPiece {
            build_generation,
            piece,
        });
        self.replace_published(id, published, changed)
    }

    fn replace_published(
        &mut self,
        id: PieceId,
        published: Option<PublishedPiece>,
        changed: &mut BTreeSet<SurfaceChunkKey>,
    ) -> Option<PublishedPiece> {
        let old = self.pieces.remove(&id);
        if let Some(old) = &old {
            let footprint = old.parts().1;
            self.index.remove(id, footprint);
            changed.extend(footprint.render_chunks().iter().copied());
            self.changed_bounds.push(old.mesh().bounds_xz());
        }
        if let Some(published) = published {
            let (mesh, footprint) = published.parts();
            self.index.insert(id, mesh, footprint);
            changed.extend(footprint.render_chunks().iter().copied());
            self.changed_bounds.push(published.mesh().bounds_xz());
            self.pieces.insert(id, published);
        }
        old
    }

    /// XZ bounds `[min_x, min_z, max_x, max_z]` of every old and new piece the compiles since
    /// the last call replaced, added or removed, in compile order: the footprints whose terrain
    /// patches an edit touches (`ROAD-68-D`). Taking them clears the list.
    pub fn take_changed_bounds(&mut self) -> Vec<[f64; 4]> {
        std::mem::take(&mut self.changed_bounds)
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
    pub fn failed_nodes(&self) -> &[u32] {
        &self.failed_ids
    }

    /// Nodes that fail the commit's node check (`check_node`) although their piece built: a
    /// bend or junction the profile solve leaves without a plane, which an edit rejects with
    /// `NoPlane` before publishing. A load runs it on every published node piece, in parallel on
    /// the shared pool, so a saved graph is validated like a commit (`ROAD-68-E`). Ascending.
    pub fn unpinned_nodes(&self, graph: &RegionGraph) -> Vec<u32> {
        let nodes: Vec<u32> = self
            .pieces
            .range(PieceId::Node(0)..)
            .filter_map(|(id, published)| match (id, &published.piece) {
                (PieceId::Node(node), Piece::Node(piece)) if piece.kind() != NodeKind::Terminal => {
                    Some(*node)
                }
                _ => None,
            })
            .collect();
        crate::thread_pools::install(|| {
            nodes
                .into_par_iter()
                .filter(|&node| graph.junction_profile_solve_fails(node))
                .collect()
        })
    }

    /// Why a node's piece failed in the published generation.
    pub fn node_error(&self, node: u32) -> Option<NodeError> {
        self.failed.get(&node).copied()
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
        match &self.pieces.get(&PieceId::Span(edge_idx))?.piece {
            Piece::Span(span) => Some(span),
            Piece::Node(_) => None,
        }
    }

    /// The published junction, bend or terminal piece of a node.
    pub fn node_piece(&self, node: u32) -> Option<&NodePiece> {
        match &self.pieces.get(&PieceId::Node(node))?.piece {
            Piece::Node(node) => Some(node),
            Piece::Span(_) => None,
        }
    }
}

// Plans the given nodes in parallel on the shared pool.
fn plan_nodes(graph: &RegionGraph, nodes: &BTreeSet<u32>) -> Plans {
    if nodes.is_empty() {
        return Plans::new();
    }
    let nodes: Vec<u32> = nodes.iter().copied().collect();
    let planned: Vec<_> = crate::thread_pools::install(|| {
        nodes
            .par_iter()
            .map(|&node| (node, plan_node(graph, node)))
            .collect()
    });
    planned.into_iter().collect()
}
