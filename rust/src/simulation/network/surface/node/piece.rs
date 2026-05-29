//! Node visual-piece DTOs and exported provenance contracts.

use super::{
    RoadSurfaceBandKind, RoadSurfaceEarthworkBoundarySegment, RoadSurfaceEarthworkRenderFace,
    RoadSurfaceTerrainClipLoop, RoadSurfaceVisualNodePieceKind, RoadSurfaceVisualPolygon,
    arrangement, height::NodeGradeVertexAuthority,
};
use crate::simulation::network::{
    surface::band_semantics::ordered_raised_step_kinds, types::EdgeClass,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RoadSurfaceVerticalFaceSource {
    CanonicalStep {
        explicit_vertical_step_index: usize,
        segment: arrangement::NodeExplicitVerticalStepSegment,
    },
    CanonicalStepSameMaterialHandoff {
        explicit_vertical_step_index: usize,
        segment: arrangement::NodeExplicitVerticalStepSegment,
        lower_owner: arrangement::NodeBandOwner,
        raised_owner: arrangement::NodeBandOwner,
    },
}

impl RoadSurfaceVerticalFaceSource {
    pub(crate) fn explicit_vertical_step_index(self) -> Option<usize> {
        match self {
            Self::CanonicalStep {
                explicit_vertical_step_index,
                ..
            }
            | Self::CanonicalStepSameMaterialHandoff {
                explicit_vertical_step_index,
                ..
            } => Some(explicit_vertical_step_index),
        }
    }

    pub(crate) fn segment(self) -> arrangement::NodeExplicitVerticalStepSegment {
        match self {
            Self::CanonicalStep { segment, .. } => segment,
            Self::CanonicalStepSameMaterialHandoff { segment, .. } => segment,
        }
    }

    pub(crate) fn lower_and_raised_owners(
        self,
    ) -> Option<(arrangement::NodeBandOwner, arrangement::NodeBandOwner)> {
        match self {
            Self::CanonicalStep { segment, .. } => {
                let owner = segment.owner();
                let opposite_owner = segment.opposite_owner();
                let (lower_kind, _) =
                    ordered_raised_step_kinds(owner.kind(), opposite_owner.kind())?;
                Some(if owner.kind() == lower_kind {
                    (owner, opposite_owner)
                } else {
                    (opposite_owner, owner)
                })
            }
            Self::CanonicalStepSameMaterialHandoff {
                lower_owner,
                raised_owner,
                ..
            } => Some((lower_owner, raised_owner)),
        }
    }

    pub(crate) fn sort_key(
        self,
    ) -> (
        u8,
        arrangement::NodeExplicitVerticalStepSegment,
        Option<usize>,
    ) {
        match self {
            Self::CanonicalStep {
                explicit_vertical_step_index,
                segment,
            } => (0, segment, Some(explicit_vertical_step_index)),
            Self::CanonicalStepSameMaterialHandoff {
                explicit_vertical_step_index,
                segment,
                lower_owner,
                raised_owner,
            } => (
                1,
                segment,
                Some(
                    explicit_vertical_step_index
                        ^ lower_owner.owner_index()
                        ^ raised_owner.owner_index(),
                ),
            ),
        }
    }
}

/// Explicit visual node piece compiled from the solved roadbed.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadSurfaceVisualNodePiece {
    /// Owning node id.
    pub node_id: u32,
    /// Piece classification for rendering and debug.
    pub kind: RoadSurfaceVisualNodePieceKind,
    /// Outer piece-owned boundaries used for debug, surface chunk bounds, and terrain clipping.
    pub outer_boundary_loops: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) terrain_clip_boundary_loops: Vec<RoadSurfaceTerrainClipLoop>,
    /// Explicit asphalt-owned polygons for the node piece.
    pub road_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    /// Explicit curb / shoulder-owned polygons for the node piece.
    pub curb_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    /// Explicit vertical faces at raised owner-pair material contacts.
    pub raised_step_face_polygons: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) raised_step_face_sources: Vec<RoadSurfaceVerticalFaceSource>,
    /// Explicit sidewalk-owned polygons for the node piece.
    pub sidewalk_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) explicit_vertical_step_segments: Vec<arrangement::NodeExplicitVerticalStepSegment>,
    pub(crate) node_grade_authorities: Vec<NodeGradeVertexAuthority>,
    pub(crate) node_top_surface_sources: Vec<NodeTopSurfacePolygonSource>,
    pub(crate) owned_regions: Vec<NodeOwnedRegion>,
    pub(crate) earthwork_owner_sources: Vec<NodeEarthworkOwnerSource>,
    pub(crate) earthwork_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) earthwork_outer_boundary_loops: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) render_earthwork_faces: Vec<RoadSurfaceEarthworkRenderFace>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeOwnedRegion {
    pub(crate) kind: RoadSurfaceBandKind,
    pub(crate) owner_index: usize,
    pub(crate) polygon: RoadSurfaceVisualPolygon,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NodeEarthworkOwnerSource {
    pub(crate) owner_kind: RoadSurfaceBandKind,
    pub(crate) owner_index: usize,
    pub(crate) mouth_order_index: usize,
    pub(crate) edge_idx: usize,
    pub(crate) edge_class: EdgeClass,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct NodeTopSurfaceVertexSource {
    pub(crate) grade_authority_index: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct NodeFootprintBoundaryDirectSource {
    pub(crate) top_surface_source_index: usize,
    pub(crate) grade_authority_index: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum NodeFootprintBoundaryVertexSource {
    Direct(NodeFootprintBoundaryDirectSource),
    CanonicalBoundaryPoint {
        x_key: i64,
        z_key: i64,
        y_mm: i64,
    },
    BoundaryInterpolation {
        owning_segment_start: NodeFootprintBoundaryDirectSource,
        owning_segment_end: NodeFootprintBoundaryDirectSource,
        height_mm: i64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct NodeFootprintBoundarySegmentSource {
    pub(crate) start: NodeFootprintBoundaryVertexSource,
    pub(crate) end: NodeFootprintBoundaryVertexSource,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeTopSurfacePolygonSource {
    pub(crate) kind: RoadSurfaceBandKind,
    pub(crate) owner_index: usize,
    pub(crate) height_field_id: arrangement::NodeBandHeightFieldId,
    pub(crate) vertex_keys: Vec<arrangement::NodeArrangementKey>,
    pub(crate) vertex_height_mm: Vec<i64>,
    pub(crate) vertex_sources: Vec<NodeTopSurfaceVertexSource>,
    pub(crate) triangle_sources: Vec<[NodeTopSurfaceVertexSource; 3]>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeSurfaceRegionResult {
    pub(crate) outer_boundary_loops: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) earthwork_boundary_segments: Vec<Vec<RoadSurfaceEarthworkBoundarySegment>>,
    pub(crate) terrain_clip_boundary_loops: Vec<RoadSurfaceTerrainClipLoop>,
    pub(crate) road_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) curb_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) raised_step_faces: Vec<(RoadSurfaceVisualPolygon, RoadSurfaceVerticalFaceSource)>,
    pub(crate) sidewalk_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) explicit_vertical_step_segments: Vec<arrangement::NodeExplicitVerticalStepSegment>,
    pub(crate) node_grade_authorities: Vec<NodeGradeVertexAuthority>,
    pub(crate) node_top_surface_sources: Vec<NodeTopSurfacePolygonSource>,
    pub(crate) owned_regions: Vec<NodeOwnedRegion>,
}
