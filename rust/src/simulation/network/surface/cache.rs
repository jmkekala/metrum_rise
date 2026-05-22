//! Dirty tracking, chunk ownership, and cache rebuild helpers for road-surface pieces.

use super::{
    RoadSurfaceSystem, RoadSurfaceVisualNodePiece, RoadSurfaceVisualSpanPiece, SurfaceChunkKey,
};
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{EdgeClass, TransitType};
use godot::prelude::{Vector2, Vector3};
use std::collections::{BTreeSet, HashMap, HashSet};

mod bounds;
mod coverage;
mod dirty;
mod rebuild;

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
