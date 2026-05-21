//! Explicit visual node-piece construction and incident-edge classification.

use self::{
    arrangement::{
        NodeArrangement, NodeArrangementKey, NodeBandOwner, NodeExplicitVerticalStepSegment,
    },
    boundary::{
        ArrangementBoundaryPointKey, ArrangementSegmentParameter, NodeBoundaryExportError,
        NodeFootprintBoundaryExportSources, NodeFootprintBoundaryPoint,
        arrangement_boundary_point_to_world, boundary_points_numeric_area_budget_m2,
        boundary_segment_parameter_xz, node_earthwork_boundary_segments_from_footprint_loops,
        remove_subbudget_unsupported_numeric_boundary_vertices,
        same_winding_boundary_point_loops_from_loop,
    },
    input::NodeInputExtractionError,
    validation::NodeValidationReport,
};
use super::{
    CompiledNodeKind, IncidentEdgeSide, IncidentMouthProfile, IncidentSurfaceEdge,
    OrderedIncidentPieceMouth, RoadSurfaceBandKind, RoadSurfaceEarthworkBoundarySegment,
    RoadSurfaceEarthworkFaceSource, RoadSurfaceEarthworkRenderFace, RoadSurfaceSystem,
    RoadSurfaceTerrainClipEdgeKind, RoadSurfaceTerrainClipLoop, RoadSurfaceTerrainClipSourceEdge,
    RoadSurfaceVisualNodePieceKind, RoadSurfaceVisualPolygon, SAMPLE_EPSILON_M,
    band_semantics::ordered_raised_step_kinds, edge::VISUAL_MIN_SPAN_LENGTH_M,
    terrain_clip_edge_kind_for_band,
};
pub(super) use super::{
    IncidentMouthBand, NODE_OVERLAY_MIN_AREA_M2, NodeOverlayContour, NodeOverlayPoint,
    NodeOverlayShape, NodeOverlayShapes, SurfaceCdt, backend, band_semantics, indices, keys, paths,
    segments,
};
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::{Vector2, Vector3};
use std::collections::{BTreeMap, BTreeSet};

// Node-piece classification threshold.
const PASS_THROUGH_DOT_THRESHOLD: f32 = 0.98;
const VISUAL_DOMINANT_HANDOFF_REJECTION_RATIO: f32 = 3.0;

pub(crate) mod arrangement;
mod arrangement_faces;
pub(crate) mod boundary;
mod boundary_edges;
mod compile;
mod export;
pub(crate) mod height;
mod incident;
pub(crate) mod input;
pub(crate) mod joins;
pub(crate) mod ownership;
mod piece;
pub(crate) mod rails;
pub(crate) mod terminal;
#[cfg(test)]
mod tests;
pub(crate) mod triangulation;
pub(crate) mod validation;
mod vertical_faces;

pub use piece::RoadSurfaceVisualNodePiece;
pub(crate) use piece::{
    NodeEarthworkOwnerSource, NodeFootprintBoundaryDirectSource,
    NodeFootprintBoundarySegmentSource, NodeFootprintBoundaryVertexSource, NodeOwnedRegion,
    NodeSurfaceRegionResult, NodeTopSurfacePolygonSource, NodeTopSurfaceVertexSource,
    RoadSurfaceVerticalFaceSource,
};
