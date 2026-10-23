// SPDX-License-Identifier: GPL-2.0-only

//! `RoadbedSystem` state: the chunk grid, dirty marks and the source / compiled generation gate.

use super::{RoadSection, SurfaceChunkKey};
use crate::simulation::network::graph::RegionGraph;
use std::collections::BTreeSet;

/// Roadbed pieces derived from the road graph, with their chunk index and generation gate.
///
/// Owned by `TransitNetwork`. Graph edits mark edges and nodes dirty; [`Self::compile_dirty`]
/// rebuilds their pieces and publishes a new generation. In `ROAD-68-A` no pieces exist yet.
#[derive(Clone)]
pub struct RoadbedSystem {
    chunk_span_m: f32,
    chunk_origin_m: (f32, f32),
    // False until the first compile; a fresh system has nothing published.
    compiled: bool,
    // Advances once per compile that published a change.
    generation: u64,
    // Sorted, so compiles visit dirty owners in a deterministic order.
    dirty_edges: BTreeSet<usize>,
    dirty_nodes: BTreeSet<u32>,
    // Sorted render chunks changed by the last compile that had work.
    last_changed_chunks: Vec<SurfaceChunkKey>,
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
            chunk_span_m: chunk_span_m.max(f32::EPSILON),
            chunk_origin_m: (finite_or_zero(origin_x_m), finite_or_zero(origin_z_m)),
            compiled: false,
            generation: 0,
            dirty_edges: BTreeSet::new(),
            dirty_nodes: BTreeSet::new(),
            last_changed_chunks: Vec::new(),
        }
    }

    /// Drops every piece and dirty mark; the next compile starts from an empty roadbed.
    pub fn clear(&mut self) {
        self.compiled = false;
        self.generation = self.generation.wrapping_add(1);
        self.dirty_edges.clear();
        self.dirty_nodes.clear();
        self.last_changed_chunks.clear();
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

    // Edges marked dirty since the last compile; tests assert edit marking through it.
    #[cfg(test)]
    pub(crate) fn dirty_edges(&self) -> &BTreeSet<usize> {
        &self.dirty_edges
    }

    /// Rebuilds the pieces of the dirty closure and returns the sorted render chunks they changed.
    /// A compile with nothing pending changes nothing and keeps [`Self::last_changed_chunks`].
    ///
    /// `ROAD-68-A` builds no pieces: it clears the dirty marks, publishes a new generation when
    /// anything was pending, and changes no chunks.
    pub fn compile_dirty(&mut self, _graph: &RegionGraph) -> Vec<SurfaceChunkKey> {
        if self.is_current() {
            return Vec::new();
        }
        self.dirty_edges.clear();
        self.dirty_nodes.clear();
        self.compiled = true;
        self.generation = self.generation.wrapping_add(1);
        self.last_changed_chunks.clear();
        self.last_changed_chunks.clone()
    }

    /// Sorted render chunks changed by the last compile that had work.
    pub fn last_changed_chunks(&self) -> &[SurfaceChunkKey] {
        &self.last_changed_chunks
    }

    /// Sorted render chunks that hold any published piece.
    pub fn occupied_chunks(&self) -> impl Iterator<Item = SurfaceChunkKey> + '_ {
        std::iter::empty()
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
        &[]
    }

    /// Span of one render chunk, in metres.
    pub fn chunk_span_m(&self) -> f32 {
        self.chunk_span_m
    }

    /// World-space minimum corner from which render chunk keys are measured.
    pub fn chunk_origin_m(&self) -> (f32, f32) {
        self.chunk_origin_m
    }

    /// Render chunk containing a world XZ point.
    pub fn chunk_key_at(&self, x: f64, z: f64) -> SurfaceChunkKey {
        let span = f64::from(self.chunk_span_m);
        (
            ((x - f64::from(self.chunk_origin_m.0)) / span).floor() as i32,
            ((z - f64::from(self.chunk_origin_m.1)) / span).floor() as i32,
        )
    }

    /// World XZ bounds `[min_x, min_z, max_x, max_z]` of one render chunk.
    pub fn chunk_bounds(&self, key: SurfaceChunkKey) -> [f64; 4] {
        let span = f64::from(self.chunk_span_m);
        let min_x = f64::from(self.chunk_origin_m.0) + f64::from(key.0) * span;
        let min_z = f64::from(self.chunk_origin_m.1) + f64::from(key.1) * span;
        [min_x, min_z, min_x + span, min_z + span]
    }

    /// Sampled cross-sections of an edge's span piece; `None` when the edge has no span piece.
    pub fn sections(&self, _edge_idx: usize) -> Option<&[RoadSection]> {
        None
    }
}
