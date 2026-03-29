//! Road network graph data structures and spatial indexing.

/// Core data structures for nodes and edges.
pub mod data;
/// Logic for rebuilding adjacency lists and connectivity.
pub mod rebuild;
/// Chunk-based spatial index for fast distance and AABB queries.
pub mod spatial;
/// Topology operations (edge splitting, merging).
pub mod topology;

pub use data::{verify_intersection_geometry, Edge, Node, RegionGraph};
