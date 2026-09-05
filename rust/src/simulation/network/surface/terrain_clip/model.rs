// SPDX-License-Identifier: GPL-2.0-only

//! Terrain-clip source model and local export diagnostics.

use super::super::backend::RoadVec3;
use super::super::{
    NodeFootprintBoundarySegmentSource, NodeOverlayPoint, RoadSurfaceBandKind,
    earthwork::RoadSurfaceEarthworkFaceSource, keys::SurfaceSegmentParameter,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum RoadSurfaceTerrainClipEdgeKind {
    SidewalkOuter,
    ShoulderOuter,
    FootprintBoundary,
    SpanHandoff,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RoadSurfaceTerrainClipSourceEdge {
    pub(crate) start: RoadVec3,
    pub(crate) end: RoadVec3,
    pub(crate) kind: RoadSurfaceTerrainClipEdgeKind,
    pub(crate) source: RoadSurfaceEarthworkFaceSource,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RoadSurfaceTerrainClipLoop {
    pub(crate) points_world: Vec<RoadVec3>,
    pub(crate) source_edges: Vec<RoadSurfaceTerrainClipSourceEdge>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RoadSurfaceTerrainClipExport {
    pub(crate) loops: Vec<RoadSurfaceTerrainClipLoop>,
    pub(crate) loop_topologies: Vec<RoadSurfaceTerrainClipLoopTopology>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RoadSurfaceTerrainClipExportError {
    OverlayUnionFailed {
        source_loop_count: usize,
    },
    MissingOuterBoundaryOwner {
        shape_index: usize,
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
        context: String,
    },
    MissingOutputBoundaryOwner {
        shape_index: usize,
        start: RoadVec3,
        end: RoadVec3,
    },
    AmbiguousOutputBoundaryOwner {
        shape_index: usize,
        start: RoadVec3,
        end: RoadVec3,
        context: String,
    },
    UnclosedOutputBoundary {
        shape_index: usize,
        start: RoadVec3,
        end: RoadVec3,
    },
    RepeatedOverlayPointCycle {
        shape_index: usize,
        contour_index: usize,
        x_key: i64,
        z_key: i64,
        cycle_area_m2: f64,
        remainder_area_m2: f64,
        dust_budget_m2: f64,
    },
    AmbiguousDustConnectorHeight {
        shape_index: usize,
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
        context: String,
    },
}

impl RoadSurfaceTerrainClipExportError {
    pub(crate) fn debug_label(&self) -> &'static str {
        match self {
            Self::OverlayUnionFailed { .. } => "terrain_clip_overlay_union_failed",
            Self::MissingOuterBoundaryOwner { .. } => "terrain_clip_missing_outer_boundary_owner",
            Self::MissingOutputBoundaryOwner { .. } => "terrain_clip_missing_output_boundary_owner",
            Self::AmbiguousOutputBoundaryOwner { .. } => {
                "terrain_clip_ambiguous_output_boundary_owner"
            }
            Self::UnclosedOutputBoundary { .. } => "terrain_clip_unclosed_output_boundary",
            Self::RepeatedOverlayPointCycle { .. } => "terrain_clip_repeated_overlay_point_cycle",
            Self::AmbiguousDustConnectorHeight { .. } => {
                "terrain_clip_ambiguous_dust_connector_height"
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RoadSurfaceTerrainClipContourRole {
    Outer,
    Hole,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RoadSurfaceTerrainClipLoopTopology {
    pub(crate) shape_index: usize,
    pub(crate) contour_index: usize,
    pub(crate) role: RoadSurfaceTerrainClipContourRole,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct TerrainClipOutputContour {
    pub(super) boundary_loop: RoadSurfaceTerrainClipLoop,
    pub(super) topology: RoadSurfaceTerrainClipLoopTopology,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct TerrainClipContourCompactError {
    pub(super) x_key: i64,
    pub(super) z_key: i64,
    pub(super) cycle_area_m2: f64,
    pub(super) remainder_area_m2: f64,
    pub(super) dust_budget_m2: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum TerrainClipDustConnectorRecovery {
    Missing,
    Ambiguous(String),
    Covered(Vec<RoadVec3>),
}

#[derive(Clone, Copy)]
pub(super) struct TerrainClipSourceEdge {
    pub(super) start: RoadVec3,
    pub(super) end: RoadVec3,
    pub(super) kind: RoadSurfaceTerrainClipEdgeKind,
    pub(super) source: RoadSurfaceEarthworkFaceSource,
    pub(super) source_index: usize,
    pub(super) edge_index: usize,
}

#[derive(Clone, Copy)]
pub(super) struct TerrainClipSegmentHeights {
    pub(super) start_y: f64,
    pub(super) end_y: f64,
}

#[derive(Clone, Copy)]
pub(super) struct TerrainClipEndpointSample {
    pub(super) kind: RoadSurfaceTerrainClipEdgeKind,
    pub(super) source_index: usize,
    pub(super) edge_index: usize,
    pub(super) y: f64,
}

#[derive(Clone, Copy)]
pub(super) struct TerrainClipSourceInterval {
    pub(super) start_t: f64,
    pub(super) end_t: f64,
    pub(super) start_y: f64,
    pub(super) end_y: f64,
}

#[derive(Clone, Copy)]
pub(super) struct TerrainClipPreparedSource {
    pub(super) edge: TerrainClipSourceEdge,
    pub(super) interval: TerrainClipSourceInterval,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum TerrainClipSegmentPointRecovery {
    Degenerate,
    Covered(Vec<RoadVec3>),
    Partial,
    Missing,
}

pub(super) type OverlaySegmentParameter = SurfaceSegmentParameter;

pub(crate) fn terrain_clip_edge_kind_for_band(
    kind: RoadSurfaceBandKind,
) -> RoadSurfaceTerrainClipEdgeKind {
    match kind {
        RoadSurfaceBandKind::Sidewalk => RoadSurfaceTerrainClipEdgeKind::SidewalkOuter,
        RoadSurfaceBandKind::CurbOrShoulder => RoadSurfaceTerrainClipEdgeKind::ShoulderOuter,
        _ => RoadSurfaceTerrainClipEdgeKind::FootprintBoundary,
    }
}

pub(super) fn terrain_clip_edge_kind_priority(kind: RoadSurfaceTerrainClipEdgeKind) -> u8 {
    match kind {
        RoadSurfaceTerrainClipEdgeKind::SidewalkOuter => 0,
        RoadSurfaceTerrainClipEdgeKind::ShoulderOuter => 1,
        RoadSurfaceTerrainClipEdgeKind::FootprintBoundary => 2,
        RoadSurfaceTerrainClipEdgeKind::SpanHandoff => 3,
    }
}

pub(super) fn terrain_clip_source_edge_ordering(
    a: TerrainClipSourceEdge,
    b: TerrainClipSourceEdge,
) -> std::cmp::Ordering {
    terrain_clip_edge_kind_priority(a.kind)
        .cmp(&terrain_clip_edge_kind_priority(b.kind))
        .then_with(|| a.source.source_ordering(b.source))
        .then(a.source_index.cmp(&b.source_index))
        .then(a.edge_index.cmp(&b.edge_index))
}

pub(super) fn terrain_clip_source_edges_same_provenance(
    a: TerrainClipSourceEdge,
    b: TerrainClipSourceEdge,
) -> bool {
    if a.kind == RoadSurfaceTerrainClipEdgeKind::SpanHandoff
        && b.kind == RoadSurfaceTerrainClipEdgeKind::SpanHandoff
        && terrain_clip_span_handoff_sources_same_provenance(a.source, b.source)
    {
        return true;
    }
    a.kind == b.kind && terrain_clip_sources_same_provenance(a.source, b.source)
}

fn terrain_clip_span_handoff_sources_same_provenance(
    a: RoadSurfaceEarthworkFaceSource,
    b: RoadSurfaceEarthworkFaceSource,
) -> bool {
    match (a, b) {
        (
            RoadSurfaceEarthworkFaceSource::SpanSupportBoundary {
                edge_idx: edge_idx_a,
                edge_class: edge_class_a,
                support_policy: support_policy_a,
                start_section_index: start_section_index_a,
                end_section_index: end_section_index_a,
                start_s_m: start_s_m_a,
                end_s_m: end_s_m_a,
                ..
            },
            RoadSurfaceEarthworkFaceSource::SpanSupportBoundary {
                edge_idx: edge_idx_b,
                edge_class: edge_class_b,
                support_policy: support_policy_b,
                start_section_index: start_section_index_b,
                end_section_index: end_section_index_b,
                start_s_m: start_s_m_b,
                end_s_m: end_s_m_b,
                ..
            },
        ) => {
            edge_idx_a == edge_idx_b
                && edge_class_a == edge_class_b
                && support_policy_a == support_policy_b
                && start_section_index_a == start_section_index_b
                && end_section_index_a == end_section_index_b
                && start_s_m_a.to_bits() == start_s_m_b.to_bits()
                && end_s_m_a.to_bits() == end_s_m_b.to_bits()
        }
        _ => false,
    }
}

fn terrain_clip_sources_same_provenance(
    a: RoadSurfaceEarthworkFaceSource,
    b: RoadSurfaceEarthworkFaceSource,
) -> bool {
    match (a, b) {
        (
            RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
                node_id: node_id_a,
                kind: kind_a,
                owner_kind: owner_kind_a,
                owner_index: owner_index_a,
                boundary_source: boundary_source_a,
            },
            RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
                node_id: node_id_b,
                kind: kind_b,
                owner_kind: owner_kind_b,
                owner_index: owner_index_b,
                boundary_source: boundary_source_b,
            },
        ) => {
            node_id_a == node_id_b
                && kind_a == kind_b
                && owner_kind_a == owner_kind_b
                && owner_index_a == owner_index_b
                && terrain_clip_boundary_sources_same_undirected(
                    boundary_source_a,
                    boundary_source_b,
                )
        }
        (
            RoadSurfaceEarthworkFaceSource::NodeSameMaterialBoundaryHandoff {
                node_id: node_id_a,
                kind: kind_a,
                owner_kind: owner_kind_a,
                owner_index_a: owner_index_a_a,
                owner_index_b: owner_index_b_a,
                boundary_source: boundary_source_a,
            },
            RoadSurfaceEarthworkFaceSource::NodeSameMaterialBoundaryHandoff {
                node_id: node_id_b,
                kind: kind_b,
                owner_kind: owner_kind_b,
                owner_index_a: owner_index_a_b,
                owner_index_b: owner_index_b_b,
                boundary_source: boundary_source_b,
            },
        ) => {
            node_id_a == node_id_b
                && kind_a == kind_b
                && owner_kind_a == owner_kind_b
                && owner_index_a_a == owner_index_a_b
                && owner_index_b_a == owner_index_b_b
                && terrain_clip_boundary_sources_same_undirected(
                    boundary_source_a,
                    boundary_source_b,
                )
        }
        _ => a == b,
    }
}

fn terrain_clip_boundary_sources_same_undirected(
    a: Option<NodeFootprintBoundarySegmentSource>,
    b: Option<NodeFootprintBoundarySegmentSource>,
) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a == b || (a.start == b.end && a.end == b.start),
        _ => a == b,
    }
}
