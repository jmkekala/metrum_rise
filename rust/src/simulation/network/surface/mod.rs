//! Public road-surface contracts, module wiring, and shared numeric constants.
//!
//! The sibling modules own the concrete edge, span, node, overlay, query,
//! earthwork, geometry, cache, system, and debug implementations. This file
//! keeps only the public contracts and stage re-exports that cross those owners.

use spade::{ConstrainedDelaunayTriangulation, Point2};

mod backend;
mod band_semantics;
mod cache;
mod debug;
mod earthwork;
mod edge;
mod geometry;
mod incident;
mod indices;
mod keys;
mod node;
mod overlay;
mod paths;
mod query;
mod segments;
mod span;
mod system;
mod terrain_clip;

pub use backend::{RoadVec2, RoadVec3};
pub use cache::{RoadEarthworkChunkCacheEntry, RoadSurfaceChunkCacheEntry};
pub use edge::{PreviewRoadSurfaceResult, RoadPreviewValidation};
pub use node::RoadSurfaceVisualNodePiece;
pub use span::RoadSurfaceVisualSpanPiece;
pub use system::RoadSurfaceSystem;

pub(crate) use cache::ChunkCacheKind;
pub(crate) use cache::RoadSurfaceTopologyUndo;
pub(crate) use earthwork::{
    RoadSurfaceEarthworkBoundarySegment, RoadSurfaceEarthworkFaceKind,
    RoadSurfaceEarthworkFaceSource, RoadSurfaceEarthworkRenderFace,
    RoadSurfaceEarthworkSupportPolicy,
};
pub(crate) use edge::{CURB_STEP_HEIGHT_M, RoadExtensionReprofile};
pub(crate) use incident::{
    CompiledNodeKind, IncidentEdgeSide, IncidentMouthBand, IncidentMouthProfile,
    IncidentSurfaceEdge, OrderedIncidentPieceMouth, RoadSurfaceVisualNodeCompileInput,
};
pub(crate) use node::{
    NodeCanonicalTopologyCache, NodeFootprintBoundaryDirectSource,
    NodeFootprintBoundarySegmentSource, NodeFootprintBoundaryVertexSource, NodeOwnedRegion,
    NodeTopSurfacePolygonSource, NodeVisualCompileResult, RoadSurfaceVerticalFaceSource,
    rounded_sidewalk_corner_path_xz,
};
pub(crate) use node::{arrangement, height};
#[cfg(test)]
pub(crate) use node::{input, ownership, rails, terminal, triangulation, validation};
pub(crate) use span::{
    RoadSurfaceSpanBandOwner, RoadSurfaceSpanOwnedRegion, RoadSurfaceSpanRegionRole,
};
pub(crate) use system::{RoadPreviewTopologyReuse, RoadSurfaceCompileReason};
pub(crate) use terrain_clip::{
    RoadSurfaceTerrainClipContourRole, RoadSurfaceTerrainClipEdgeKind,
    RoadSurfaceTerrainClipExport, RoadSurfaceTerrainClipExportError, RoadSurfaceTerrainClipLoop,
    RoadSurfaceTerrainClipLoopTopology, RoadSurfaceTerrainClipSourceEdge,
    terrain_clip_edge_kind_for_band,
};

// Shared geometric tolerances used across surface compilation, overlay solving, and queries.
const SAMPLE_EPSILON_M: f32 = 0.001;
const WORLD_POINT_DEDUP_DISTANCE_M: f32 = 1.0e-4;
const WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2: f32 =
    WORLD_POINT_DEDUP_DISTANCE_M * WORLD_POINT_DEDUP_DISTANCE_M;
// Shared overlay/geometry area floor: one 1 mm quantized square keeps closure slivers visible.
const NODE_OVERLAY_MIN_AREA_M2: f32 = 1.0e-6;
// Self-checks that compare two backend results allow a small fixed-grid residual budget.
const NODE_OVERLAY_NUMERIC_AREA_EPS_M2: f32 = NODE_OVERLAY_MIN_AREA_M2 * 16.0;
const NODE_OVERLAY_NUMERIC_DUST_WIDTH_M: f32 = WORLD_POINT_DEDUP_DISTANCE_M;
const NODE_OVERLAY_NUMERIC_AREA_CAP_M2: f32 = 1.0e-3;
// Avoid Rayon setup overhead for the small edge/node sets common in single-edit rebuilds.
const PARALLEL_SURFACE_COMPILE_MIN_ITEMS: usize = 16;
// Node pieces are much heavier than edge/span pieces; parallelize as soon as two dirty nodes exist.
const PARALLEL_NODE_COMPILE_MIN_ITEMS: usize = 2;

type SurfaceCdt = ConstrainedDelaunayTriangulation<Point2<f64>>;
type NodeOverlayPoint = [f64; 2];
type NodeOverlayPointKey = (i64, i64);
type NodeOverlayContour = Vec<NodeOverlayPoint>;
type NodeOverlayShape = Vec<NodeOverlayContour>;
type NodeOverlayShapes = Vec<NodeOverlayShape>;

/// Chunk key used by the road-surface and earthwork caches.
pub type SurfaceChunkKey = (i32, i32);

/// Ordered lateral surface-band kinds supported by the compiled roadbed.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RoadSurfaceBandKind {
    /// Main drivable carriageway surface.
    Carriageway,
    /// Curb or shoulder transition surface adjacent to the carriageway.
    CurbOrShoulder,
    /// Walkable sidewalk surface.
    Sidewalk,
    /// Dedicated pedestrian corridor that is not a roadside sidewalk band.
    Footpath,
    /// Reserved central median or separator.
    Median,
    /// Reserved parking band.
    Parking,
    /// Reserved bicycle band.
    CycleTrack,
    /// Reserved tram corridor.
    TramReservation,
}

/// One ordered lateral band inside a compiled roadbed section.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadSurfaceBand {
    /// Surface-band classification.
    pub kind: RoadSurfaceBandKind,
    /// Inclusive lateral start offset from the section centerline in world metres.
    pub lateral_start_m: f32,
    /// Inclusive lateral end offset from the section centerline in world metres.
    pub lateral_end_m: f32,
    /// Height in world metres at `lateral_start_m`.
    pub height_start_m: f32,
    /// Height in world metres at `lateral_end_m`.
    pub height_end_m: f32,
}

/// One sampled cross-section along an edge in the compiled roadbed model.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadSurfaceSection {
    /// Owning edge id.
    pub edge_idx: usize,
    /// Longitudinal distance from the edge start in world metres.
    pub s_m: f32,
    /// Section center point in world-space XZ metres.
    pub center_xz: RoadVec2,
    /// Solved center height in world metres.
    pub center_height_m: f32,
    /// Unit tangent vector in XZ.
    pub tangent_xz: RoadVec2,
    /// Unit lateral axis in XZ.
    pub lateral_xz: RoadVec2,
    /// Ordered lateral bands for this section.
    pub bands: Vec<RoadSurfaceBand>,
}

/// Piece classification for explicit visual node ownership during the graph/visual split.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoadSurfaceVisualNodePieceKind {
    /// One incident surface edge ends here and requires a terminal visual piece.
    Terminal,
    /// Two non-pass-through incident edges require one explicit bend visual piece.
    Bend,
    /// Three or more incident edges require an explicit multi-mouth junction visual piece.
    JunctionN,
}

impl RoadSurfaceVisualNodePieceKind {
    pub(crate) fn sort_key(self) -> u8 {
        match self {
            Self::Terminal => 0,
            Self::Bend => 1,
            Self::JunctionN => 2,
        }
    }
}

/// One explicit polygon owned by the visual road carrier.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadSurfaceVisualPolygon {
    /// Ordered world-space polygon points.
    pub points_world: Vec<RoadVec3>,
    /// Deterministic cached triangles covering the polygon in world space.
    pub triangles_world: Vec<[RoadVec3; 3]>,
}

#[cfg(test)]
mod tests;
