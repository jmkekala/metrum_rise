//! Node visual-piece DTOs and exported provenance contracts.

use super::{
    NodeOverlayShape, NodeOverlayShapes, RoadSurfaceBandKind, RoadSurfaceEarthworkBoundarySegment,
    RoadSurfaceEarthworkRenderFace, RoadSurfaceTerrainClipLoop, RoadSurfaceVisualNodePieceKind,
    RoadSurfaceVisualPolygon, arrangement,
    height::NodeGradeVertexAuthority,
    ownership::NodeBooleanOwnership,
    rails::{
        NodeGeneratedContourClaimPriority, NodeGeneratedContourKind, NodeGeneratedContourPurpose,
        NodeRailContourSet,
    },
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
    pub(crate) boolean_debug: Option<NodeBooleanDebugSnapshot>,
    pub(crate) earthwork_owner_sources: Vec<NodeEarthworkOwnerSource>,
    pub(crate) earthwork_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) earthwork_outer_boundary_loops: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) render_earthwork_faces: Vec<RoadSurfaceEarthworkRenderFace>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeBooleanDebugSnapshot {
    pub(crate) footprint_shapes: NodeOverlayShapes,
    pub(crate) asphalt_shapes: NodeOverlayShapes,
    pub(crate) non_road_shapes: NodeOverlayShapes,
    pub(crate) owned_regions: Vec<NodePostBooleanOwnedRegionDebug>,
    pub(crate) side_join_contours: Vec<NodeSideJoinContourDebug>,
    pub(crate) corner_trims: Vec<NodeCornerTrimDebug>,
    pub(crate) corner_trims_apply_to_footprint: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodePostBooleanOwnedRegionDebug {
    pub(crate) kind: RoadSurfaceBandKind,
    pub(crate) owner: arrangement::NodeBandOwner,
    pub(crate) claim_priority: NodeGeneratedContourClaimPriority,
    pub(crate) source_mouth_order_index: usize,
    pub(crate) source_band_index: Option<usize>,
    pub(crate) area_m2: f32,
    pub(crate) shape: NodeOverlayShape,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeSideJoinContourDebug {
    pub(crate) kind: NodeGeneratedContourKind,
    pub(crate) purpose: NodeGeneratedContourPurpose,
    pub(crate) source_mouth_order_index: usize,
    pub(crate) source_band_index: Option<usize>,
    pub(crate) owner: Option<arrangement::NodeBandOwner>,
    pub(crate) claim_priority: NodeGeneratedContourClaimPriority,
    pub(crate) points_xz: Vec<super::backend::RoadVec2>,
    pub(crate) height_points_world: Option<Vec<super::backend::RoadVec3>>,
    pub(crate) contributes_to_footprint: bool,
    pub(crate) contributes_to_asphalt: bool,
    pub(crate) contributes_to_non_road_band: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeCornerTrimDebug {
    pub(crate) points_xz: Vec<super::backend::RoadVec2>,
}

impl NodeBooleanDebugSnapshot {
    pub(crate) fn from_rails_and_ownership(
        rails: &NodeRailContourSet,
        ownership: &NodeBooleanOwnership,
        corner_trims_apply_to_footprint: bool,
    ) -> Self {
        let owned_regions = ownership
            .owned_regions
            .iter()
            .map(|region| NodePostBooleanOwnedRegionDebug {
                kind: region.kind,
                owner: region.owner,
                claim_priority: region.claim_priority,
                source_mouth_order_index: region.source_mouth_order_index,
                source_band_index: region.source_band_index,
                area_m2: region.area_m2,
                shape: region.shape.clone(),
            })
            .collect();
        let side_join_contours = rails
            .contours
            .iter()
            .filter(|contour| {
                matches!(
                    contour.purpose,
                    NodeGeneratedContourPurpose::BendSideJoin
                        | NodeGeneratedContourPurpose::JunctionSideJoin
                )
            })
            .map(|contour| NodeSideJoinContourDebug {
                kind: contour.kind,
                purpose: contour.purpose,
                source_mouth_order_index: contour.source_mouth_order_index,
                source_band_index: contour.source_band_index,
                owner: contour.owner,
                claim_priority: contour.claim_priority,
                points_xz: contour.points_xz.clone(),
                height_points_world: contour.height_points_world.clone(),
                contributes_to_footprint: contour.contributes_to_footprint(),
                contributes_to_asphalt: contour.contributes_to_asphalt(),
                contributes_to_non_road_band: contour.contributes_to_non_road_band(),
            })
            .collect();
        let corner_trims = rails
            .corner_trims
            .iter()
            .map(|trim| NodeCornerTrimDebug {
                points_xz: trim.points_xz.clone(),
            })
            .collect();

        Self {
            footprint_shapes: ownership.footprint_shapes.clone(),
            asphalt_shapes: ownership.asphalt_shapes.clone(),
            non_road_shapes: ownership.non_road_shapes.clone(),
            owned_regions,
            side_join_contours,
            corner_trims,
            corner_trims_apply_to_footprint,
        }
    }
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
    pub(crate) boolean_debug: Option<NodeBooleanDebugSnapshot>,
}
