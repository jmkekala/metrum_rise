// ========================================================================
//  MANIFEST
// ========================================================================
//  script_name: mod.rs
//  script_path: rust/src/simulation/network/graph/mod.rs
//  module_name: graph
//  version: 0.1.0
//  description: Road network graph module root. Re-exports the node, edge
//  kind: module
//  spec: none
//  internal_dependencies: [data, lane_spec]
//  external_dependencies: []
//  features: [module-root, re-exports, graph-api]
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-24
// ========================================================================

//! Road network graph data structures and spatial indexing.

// ========================================================================
// SUBMODULES
// ========================================================================

/// Per-junction traffic control: priority signs and timed signals.
pub mod control;
/// Core data structures for nodes and edges.
pub mod data;
/// Per-lane identity: width, direction, modes, markings, turns, and extent.
pub mod lane_spec;
/// Logic for rebuilding adjacency lists and connectivity.
pub mod rebuild;
/// Chunk-based spatial index for fast distance and AABB queries.
pub mod spatial;

// ========================================================================
// RE-EXPORTS
// ========================================================================

pub(crate) use data::RegionGraphUndoDelta;
pub use control::{
    JunctionControl, PrioritySign, SignalAspect, SignalPhase, SignalProgram,
};
pub use data::{Edge, Node, RegionGraph, verify_intersection_geometry};
pub use lane_spec::{
    LaneDirection, LaneKind, LaneLayout, LaneMarking, LaneRange, LaneSpec, LaneVec, ParkingAngle,
    TurnSet,
};
