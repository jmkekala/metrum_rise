//! Visual node-piece compilation orchestration.

use super::super::edge::edge_class_sort_key;
use super::*;
use std::sync::Arc;

mod logging;
mod pieces;
mod pipeline;
mod smoothness;

#[derive(Clone)]
pub(crate) struct NodeCanonicalTopologyCache {
    rail_topology: rails::NodeRailTopologyCache,
    ownership: Option<Arc<ownership::NodeBooleanOwnership>>,
    ownership_incremental: Arc<ownership::NodeOwnershipIncrementalCache>,
    export_incremental: Arc<export::NodeExportIncrementalCache>,
}

pub(super) struct NodeCanonicalSurfaceCompileResult {
    pub(super) regions: NodeSurfaceRegionResult,
    pub(super) topology_cache: Option<NodeCanonicalTopologyCache>,
    pub(super) rail_topology_reused: bool,
    pub(super) ownership_reused: bool,
    pub(super) export_reuse_stats: export::NodeExportReuseStats,
}

pub(crate) struct NodeVisualCompileResult {
    pub(crate) piece: RoadSurfaceVisualNodePiece,
    pub(crate) earthwork_boundaries: Arc<Vec<Vec<RoadSurfaceEarthworkBoundarySegment>>>,
    pub(crate) topology_cache: Option<Arc<NodeCanonicalTopologyCache>>,
    pub(crate) rail_topology_reused: bool,
    pub(crate) ownership_reused: bool,
    /// Semantic node-export products reused while producing this piece.
    #[allow(dead_code)]
    pub(crate) export_reuse_stats: export::NodeExportReuseStats,
}
