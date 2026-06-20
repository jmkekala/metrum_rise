//! Explicit visual node-piece construction and incident-edge classification.

use self::{
    arrangement::{
        NodeArrangement, NodeArrangementKey, NodeBandOwner, NodeExplicitVerticalStepSegment,
    },
    boundary::{
        ArrangementBoundaryPointKey, ArrangementSegmentParameter, NodeBoundaryExportError,
        NodeFootprintBoundaryExportSources, arrangement_boundary_point_to_world,
        boundary_points_numeric_area_budget_m2,
        node_earthwork_boundary_segments_from_footprint_loops,
    },
    input::NodeInputExtractionError,
    validation::NodeValidationReport,
};
use super::{
    CompiledNodeKind, IncidentEdgeSide, IncidentMouthProfile, IncidentSurfaceEdge,
    OrderedIncidentPieceMouth, RoadSurfaceBandKind, RoadSurfaceEarthworkBoundarySegment,
    RoadSurfaceEarthworkFaceSource, RoadSurfaceEarthworkRenderFace, RoadSurfaceSystem,
    RoadSurfaceTerrainClipLoop, RoadSurfaceVisualNodePieceKind, RoadSurfaceVisualPolygon,
    SAMPLE_EPSILON_M, band_semantics::ordered_raised_step_kinds,
};
pub(super) use super::{
    IncidentMouthBand, NODE_OVERLAY_MIN_AREA_M2, NODE_OVERLAY_NUMERIC_DUST_WIDTH_M,
    NodeOverlayContour, NodeOverlayPoint, NodeOverlayShape, NodeOverlayShapes,
    RoadSurfaceVisualNodeCompileInput, SurfaceCdt,
    backend::{self, RoadVec2, RoadVec3},
    band_semantics, indices, keys, paths, segments,
};
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::terrain::TerrainSystem;
use std::collections::BTreeSet;

// Node-piece classification threshold.
const PASS_THROUGH_DOT_THRESHOLD: f32 = 0.98;

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
    NodeBooleanDebugSnapshot, NodeCornerTrimDebug, NodeEarthworkOwnerSource,
    NodeFootprintBoundaryDirectSource, NodeFootprintBoundarySegmentSource,
    NodeFootprintBoundaryVertexSource, NodeOwnedRegion, NodePostBooleanOwnedRegionDebug,
    NodeSideJoinContourDebug, NodeSurfaceRegionResult, NodeTopSurfacePolygonSource,
    NodeTopSurfaceVertexSource, RoadSurfaceVerticalFaceSource,
};
pub(in crate::simulation::network::surface::node) use vertical_faces::RoadSurfaceRaisedStepFace;
