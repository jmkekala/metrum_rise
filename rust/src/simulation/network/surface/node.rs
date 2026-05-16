//! Explicit visual node-piece construction and incident-edge classification.

use super::band_semantics::ordered_raised_step_kinds;
use super::{
    CompiledNodeKind, IncidentEdgeSide, IncidentMouthProfile, IncidentSurfaceEdge,
    NODE_OVERLAY_MIN_AREA_M2, NodeOverlayContour, NodeOverlayShapes, NodeOwnedRegion,
    NodeSurfaceRegionResult, NodeTopSurfacePolygonSource, NodeTopSurfaceVertexSource,
    OrderedIncidentPieceMouth, RoadSurfaceBandKind, RoadSurfaceEarthworkBoundarySegment,
    RoadSurfaceEarthworkFaceSource, RoadSurfaceEarthworkRenderFace, RoadSurfaceSystem,
    RoadSurfaceTerrainClipEdgeKind, RoadSurfaceTerrainClipLoop, RoadSurfaceTerrainClipSourceEdge,
    RoadSurfaceVerticalFaceSource, RoadSurfaceVisualNodePiece, RoadSurfaceVisualNodePieceKind,
    RoadSurfaceVisualPolygon, SAMPLE_EPSILON_M,
    arrangement::{
        self, NodeArrangement, NodeArrangementFace, NodeArrangementKey, NodeBandOwner,
        NodeExplicitVerticalStepSegment,
    },
    backend::{self, RoadVec2, road_vec2_to_overlay_point},
    edge::VISUAL_MIN_SPAN_LENGTH_M,
    input::NodeInputExtractionError,
    keys::surface_overlay_grid_collinearity_error_bound,
    node_boundary::{
        ArrangementBoundaryPointKey, ArrangementSegmentParameter, NodeBoundaryExportError,
        arrangement_boundary_point_to_world, arrangement_key_lies_exactly_on_segment,
        arrangement_key_lies_on_segment, boundary_points_numeric_area_budget_m2,
        boundary_segment_parameter_xz, interpolate_missing_footprint_boundary_heights,
        interpolated_segment_height_mm, interpolated_segment_point_key,
        node_earthwork_boundary_segments_from_footprint_loops,
        remove_unsupported_numeric_boundary_vertices,
    },
    node_grade, terrain_clip_edge_kind_for_band,
    validation::NodeValidationReport,
};
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::{Vector2, Vector3};
use std::collections::{BTreeMap, BTreeSet};

// Node-piece classification threshold.
const PASS_THROUGH_DOT_THRESHOLD: f32 = 0.98;
const VERTICAL_STEP_MIN_SPAN_M: f32 = 1.0e-6;
const VISUAL_DOMINANT_HANDOFF_REJECTION_RATIO: f32 = 3.0;

mod arrangement_faces;
mod boundary_edges;
mod compile;
mod export;
mod incident;
#[cfg(test)]
mod tests;
mod vertical_faces;
