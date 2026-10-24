// SPDX-License-Identifier: GPL-2.0-only

//! `RoadbedSystem` state: published pieces, the chunk index, dirty marks and the source /
//! compiled generation gate.

use super::index::{PieceFootprint, PieceIndex, RenderGrid};
use super::layout::{ArmKey, EdgeEnd};
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
        }
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
        // The shared pool, never Rayon's global one (`CODE-17`); inline on a pool worker.
        let (built_spans, built_nodes): (Vec<_>, Vec<_>) = crate::thread_pools::install(|| {
            let spans = edges
                .par_iter()
                .zip(&span_mouths)
                .map_init(SpanScratch::default, |scratch, (&edge_idx, mouths)| {
                    build_span_piece(
                        graph,
                        edge_idx,
                        [&mouths[0], &mouths[1]].map(Option::as_ref),
                        grid,
                        scratch,
                    )
                })
                .collect();
            let nodes = plans
                .into_par_iter()
                .map_init(NodeScratch::default, |scratch, (node, plan)| {
                    let built = match plan {
                        Ok(Some(plan)) => build_node_piece(plan, grid, scratch).map(Some),
                        Ok(None) => Ok(None),
                        Err(error) => Err(error),
                    };
                    (node, built)
                })
                .collect();
            (spans, nodes)
        });
        self.generation = self.generation.wrapping_add(1);
        let mut changed = BTreeSet::new();
        for (&edge_idx, span) in edges.iter().zip(built_spans) {
            let piece = span.map(|span| Piece::Span(Arc::new(span)));
            self.replace_piece(PieceId::Span(edge_idx), piece, &mut changed);
        }
        let mut failures_changed = false;
        for (node, built) in built_nodes {
            failures_changed |= self.failed.remove(&node).is_some();
            let piece = match built {
                Ok(piece) => piece.map(|piece| Piece::Node(Arc::new(piece))),
                Err(error) => {
                    self.failed.insert(node, error);
                    failures_changed = true;
                    None
                }
            };
            self.replace_piece(PieceId::Node(node), piece, &mut changed);
        }
        if failures_changed {
            self.failed_ids = Arc::new(self.failed.keys().copied().collect());
        }
        self.changed_chunks.extend(changed.iter().copied());
        self.dirty_edges.clear();
        self.dirty_nodes.clear();
        self.compiled = true;
        changed.into_iter().collect()
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

    fn replace_piece(
        &mut self,
        id: PieceId,
        piece: Option<Piece>,
        changed: &mut BTreeSet<SurfaceChunkKey>,
    ) {
        if let Some(old) = self.pieces.remove(&id) {
            let footprint = old.parts().1;
            self.index.remove(id, footprint);
            changed.extend(footprint.render_chunks().iter().copied());
            self.changed_bounds.push(old.mesh().bounds_xz());
        }
        let Some(piece) = piece else {
            return;
        };
        let published = PublishedPiece {
            build_generation: self.generation,
            piece,
        };
        let (mesh, footprint) = published.parts();
        self.index.insert(id, mesh, footprint);
        changed.extend(footprint.render_chunks().iter().copied());
        self.changed_bounds.push(published.mesh().bounds_xz());
        self.pieces.insert(id, published);
    }

    /// XZ bounds `[min_x, min_z, max_x, max_z]` of every old and new piece the compiles since
    /// the last call replaced, added or removed, in compile order: the footprints whose terrain
    /// patches an edit touches (`ROAD-68-D`). Taking them clears the list.
    pub fn take_changed_bounds(&mut self) -> Vec<[f64; 4]> {
        std::mem::take(&mut self.changed_bounds)
    }

    /// The bounds [`Self::take_changed_bounds`] would return, left in place.
    pub fn changed_bounds(&self) -> &[[f64; 4]] {
        &self.changed_bounds
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
