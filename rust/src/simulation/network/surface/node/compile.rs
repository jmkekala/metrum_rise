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
    arrangement: Option<Arc<arrangement::NodeArrangement>>,
    base_explicit_vertical_step_segments:
        Option<Arc<Vec<arrangement::NodeExplicitVerticalStepSegment>>>,
    ownership_incremental: Arc<ownership::NodeOwnershipIncrementalCache>,
    export_incremental: Arc<export::NodeExportIncrementalCache>,
}

impl NodeCanonicalTopologyCache {
    fn into_for_committed_node(mut self, kind: RoadSurfaceVisualNodePieceKind) -> Self {
        if kind != RoadSurfaceVisualNodePieceKind::JunctionN {
            self.rail_topology = self.rail_topology.into_incremental_only();
            self.ownership = None;
            self.arrangement = None;
            self.base_explicit_vertical_step_segments = None;
        }
        self
    }
}

pub(super) struct NodeCanonicalSurfaceCompileResult {
    pub(super) regions: NodeSurfaceRegionResult,
    pub(super) topology_cache: Option<NodeCanonicalTopologyCache>,
    pub(super) rail_topology_reused: bool,
    pub(super) ownership_reused: bool,
    #[cfg(test)]
    pub(super) export_reuse_stats: export::NodeExportReuseStats,
}

pub(crate) struct NodeVisualCompileResult {
    pub(crate) piece: Arc<RoadSurfaceVisualNodePiece>,
    pub(crate) earthwork_boundaries: Arc<Vec<Vec<RoadSurfaceEarthworkBoundarySegment>>>,
    pub(crate) topology_cache: Option<Arc<NodeCanonicalTopologyCache>>,
    pub(crate) rail_topology_reused: bool,
    pub(crate) ownership_reused: bool,
    pub(crate) preview_artifact_zero_copy: bool,
    /// Semantic node-export products reused while producing this piece.
    #[cfg(test)]
    pub(crate) export_reuse_stats: export::NodeExportReuseStats,
}
