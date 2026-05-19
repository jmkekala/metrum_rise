//! Terrain-clip source model and local export diagnostics.

use super::super::{
    NodeOverlayPoint, RoadSurfaceBandKind, RoadSurfaceVisualPolygon,
    earthwork::RoadSurfaceEarthworkFaceSource, keys::SurfaceSegmentParameter,
};
use godot::prelude::Vector3;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum RoadSurfaceTerrainClipEdgeKind {
    SidewalkOuter,
    ShoulderOuter,
    FootprintBoundary,
    SpanHandoff,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RoadSurfaceTerrainClipSourceEdge {
    pub(crate) start: Vector3,
    pub(crate) end: Vector3,
    pub(crate) kind: RoadSurfaceTerrainClipEdgeKind,
    pub(crate) source: RoadSurfaceEarthworkFaceSource,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RoadSurfaceTerrainClipLoop {
    pub(crate) points_world: Vec<Vector3>,
    pub(crate) source_edges: Vec<RoadSurfaceTerrainClipSourceEdge>,
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::simulation::network::surface) struct RoadSurfaceTerrainClipExport {
    pub(in crate::simulation::network::surface) loops: Vec<RoadSurfaceTerrainClipLoop>,
    pub(in crate::simulation::network::surface) polygons: Vec<RoadSurfaceVisualPolygon>,
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
        start: Vector3,
        end: Vector3,
    },
    UnclosedOutputBoundary {
        shape_index: usize,
        start: Vector3,
        end: Vector3,
    },
}

impl RoadSurfaceTerrainClipExportError {
    pub(crate) fn debug_label(&self) -> &'static str {
        match self {
            Self::OverlayUnionFailed { .. } => "terrain_clip_overlay_union_failed",
            Self::MissingOuterBoundaryOwner { .. } => "terrain_clip_missing_outer_boundary_owner",
            Self::MissingOutputBoundaryOwner { .. } => "terrain_clip_missing_output_boundary_owner",
            Self::UnclosedOutputBoundary { .. } => "terrain_clip_unclosed_output_boundary",
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct TerrainClipSourceEdge {
    pub(super) start: Vector3,
    pub(super) end: Vector3,
    pub(super) kind: RoadSurfaceTerrainClipEdgeKind,
    pub(super) source: RoadSurfaceEarthworkFaceSource,
    pub(super) source_index: usize,
    pub(super) edge_index: usize,
}

#[derive(Clone, Copy)]
pub(super) struct TerrainClipSegmentHeights {
    pub(super) start_y: f32,
    pub(super) end_y: f32,
}

#[derive(Clone, Copy)]
pub(super) struct TerrainClipEndpointSample {
    pub(super) kind: RoadSurfaceTerrainClipEdgeKind,
    pub(super) source_index: usize,
    pub(super) edge_index: usize,
    pub(super) y: f32,
}

#[derive(Clone, Copy)]
pub(super) struct TerrainClipSourceInterval {
    pub(super) start_t: f64,
    pub(super) end_t: f64,
    pub(super) start_y: f32,
    pub(super) end_y: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum TerrainClipSegmentPointRecovery {
    Degenerate,
    Covered(Vec<Vector3>),
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
        .then(a.source_index.cmp(&b.source_index))
        .then(a.edge_index.cmp(&b.edge_index))
}
