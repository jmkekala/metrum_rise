// SPDX-License-Identifier: GPL-2.0-only

//! Road network graph data structures and spatial indexing.

mod cross_section;
/// Core data structures for nodes and edges.
pub mod data;
/// Logic for rebuilding adjacency lists and connectivity.
pub mod rebuild;
/// Chunk-based spatial index for fast distance and AABB queries.
pub mod spatial;

pub use cross_section::{MedianStyle, RoadMedian};
pub(crate) use data::RegionGraphUndoDelta;
pub use data::{Edge, Node, RegionGraph};
