// SPDX-License-Identifier: GPL-2.0-only

//! Dirty tracking, chunk ownership, and cache rebuild helpers for road-surface pieces.

use super::{
    NodeCanonicalTopologyCache, RoadSurfaceSection, RoadSurfaceSystem,
    RoadSurfaceVisualNodeCompileInput, RoadSurfaceVisualNodePiece, RoadSurfaceVisualSpanPiece,
    SurfaceChunkKey,
    backend::{RoadVec2, RoadVec3, godot_vec2_to_road, godot_vec3_to_road},
};
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{EdgeClass, TransitType};
use godot::prelude::{Vector2, Vector3};
use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

mod bounds;
mod coverage;
mod dirty;
mod rebuild;
mod undo;

const SURFACE_QUERY_CHUNK_SPAN_M: f64 = 32.0;

/// Existing chunk-to-owner grid with O(1) snapshot cloning and branch-local updates.
/// Owner sets stay sorted and shared; only a touched chunk's set is copied after publication.
pub(crate) type ChunkOwnerIndex<T> = imbl::HashMap<SurfaceChunkKey, Arc<BTreeSet<T>>>;

/// Reverse coverage records share immutable sorted chunk lists across road snapshots.
/// Replacing a piece copies its map path, without cloning any other piece's coverage.
pub(crate) type OwnerChunkIndex<T> = imbl::HashMap<T, Arc<Vec<SurfaceChunkKey>>>;

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

/// Bounded pre-edit road-surface compiler state retained by one graph undo entry.
pub(crate) struct RoadSurfaceTopologyUndo {
    chunk_span_m_bits: u32,
    chunk_origin_x_m_bits: u32,
    chunk_origin_z_m_bits: u32,
    edges: Vec<RoadSurfaceEdgeTopologyUndo>,
    nodes: Vec<RoadSurfaceNodeTopologyUndo>,
}

struct RoadSurfaceEdgeTopologyUndo {
    edge_idx: usize,
    sections: Option<Arc<Vec<RoadSurfaceSection>>>,
    span_piece: Option<Arc<RoadSurfaceVisualSpanPiece>>,
}

struct RoadSurfaceNodeTopologyUndo {
    node_id: u32,
    piece: Option<Arc<RoadSurfaceVisualNodePiece>>,
    input: Option<Arc<RoadSurfaceVisualNodeCompileInput>>,
    earthwork_boundaries: Option<Arc<Vec<Vec<super::RoadSurfaceEarthworkBoundarySegment>>>>,
    topology: Option<Arc<NodeCanonicalTopologyCache>>,
}
