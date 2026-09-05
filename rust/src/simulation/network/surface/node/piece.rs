//! Node visual-piece DTOs and exported provenance contracts.

use super::{
    IncidentEdgeSide, NodeOverlayContour, NodeOverlayShape, NodeOverlayShapes, RoadSurfaceBandKind,
    RoadSurfaceEarthworkBoundarySegment, RoadSurfaceEarthworkRenderFace, RoadSurfaceSystem,
    RoadSurfaceTerrainClipLoop, RoadSurfaceVisualNodePieceKind, RoadSurfaceVisualPolygon,
    arrangement,
    height::NodeGradeVertexAuthority,
    joins::NodeInputSideJoinGapRole,
    ownership::NodeBooleanOwnership,
    rails::{
        NodeGeneratedContour, NodeGeneratedContourClaimPriority, NodeGeneratedContourKind,
        NodeGeneratedContourPurpose, NodeGeneratedCornerTrim, NodeGeneratedSideJoinGap,
        NodeRailContourSet,
    },
};
use crate::simulation::network::{
    surface::{RoadSurfaceTriangleQueryIndex, band_semantics::ordered_raised_step_kinds},
    types::EdgeClass,
};
use i_overlay::core::overlay_rule::OverlayRule;
use std::sync::Arc;

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
    pub(in crate::simulation::network::surface) surface_query: Arc<RoadSurfaceTriangleQueryIndex>,
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
    pub(crate) side_join_gaps: Vec<NodeSideJoinGapDebug>,
    pub(crate) side_join_material_trims: Vec<NodeSideJoinMaterialTrimDebug>,
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
pub(crate) struct NodeSideJoinGapDebug {
    pub(crate) from_mouth_order_index: usize,
    pub(crate) to_mouth_order_index: usize,
    pub(crate) from_edge_idx: usize,
    pub(crate) to_edge_idx: usize,
    pub(crate) from_side: IncidentEdgeSide,
    pub(crate) to_side: IncidentEdgeSide,
    pub(crate) angle_rad: f64,
    pub(crate) role: NodeInputSideJoinGapRole,
    pub(crate) emitted_band_kinds: Vec<RoadSurfaceBandKind>,
    pub(crate) suppressed_band_kinds: Vec<RoadSurfaceBandKind>,
    pub(crate) final_asphalt_area_m2: f32,
    pub(crate) final_curb_area_m2: f32,
    pub(crate) final_sidewalk_area_m2: f32,
    pub(crate) final_non_road_area_m2: f32,
    pub(crate) final_total_area_m2: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeSideJoinMaterialTrimDebug {
    pub(crate) kind: RoadSurfaceBandKind,
    pub(crate) purpose: NodeGeneratedContourPurpose,
    pub(crate) source_mouth_order_index: usize,
    pub(crate) source_band_index: Option<usize>,
    pub(crate) role: Option<NodeInputSideJoinGapRole>,
    pub(crate) raw_area_m2: f32,
    pub(crate) blocker_overlap_area_m2: f32,
    pub(crate) trimmed_area_m2: f32,
    pub(crate) final_owned_area_m2: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeCornerTrimDebug {
    pub(crate) source_mouth_order_index: usize,
    pub(crate) source_band_index: usize,
    pub(crate) source_band_kind: RoadSurfaceBandKind,
    pub(crate) source_owner: arrangement::NodeBandOwner,
    pub(crate) points_xz: Vec<super::backend::RoadVec2>,
    pub(crate) side_join_intersections: Vec<NodeCornerTrimSideJoinIntersectionDebug>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeCornerTrimSideJoinIntersectionDebug {
    pub(crate) contour_index: usize,
    pub(crate) kind: NodeGeneratedContourKind,
    pub(crate) purpose: NodeGeneratedContourPurpose,
    pub(crate) source_mouth_order_index: usize,
    pub(crate) source_band_index: Option<usize>,
    pub(crate) owner: Option<arrangement::NodeBandOwner>,
    pub(crate) claim_priority: NodeGeneratedContourClaimPriority,
    pub(crate) contributes_to_footprint: bool,
    pub(crate) contributes_to_asphalt: bool,
    pub(crate) contributes_to_non_road_band: bool,
    pub(crate) area_m2: f32,
}

impl NodeBooleanDebugSnapshot {
    pub(crate) fn from_rails_and_ownership(
        rails: &NodeRailContourSet,
        ownership: &NodeBooleanOwnership,
        corner_trims_apply_to_footprint: bool,
    ) -> Self {
        let owned_regions: Vec<_> = ownership
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
        let side_join_contours: Vec<_> = rails
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
                source_mouth_order_index: trim.source_mouth_order_index,
                source_band_index: trim.source_band_index,
                source_band_kind: trim.source_band_kind,
                source_owner: trim.source_owner,
                points_xz: trim.points_xz.clone(),
                side_join_intersections: corner_trim_side_join_intersections(
                    trim,
                    &side_join_contours,
                ),
            })
            .collect();
        let side_join_gaps = rails
            .side_join_gaps
            .iter()
            .map(|gap| side_join_gap_debug(gap, &owned_regions))
            .collect();
        let side_join_material_trims = side_join_material_trim_debug_records(rails, &owned_regions);

        Self {
            footprint_shapes: ownership.footprint_shapes.clone(),
            asphalt_shapes: ownership.asphalt_shapes.clone(),
            non_road_shapes: ownership.non_road_shapes.clone(),
            owned_regions,
            side_join_contours,
            side_join_gaps,
            side_join_material_trims,
            corner_trims,
            corner_trims_apply_to_footprint,
        }
    }
}

fn side_join_gap_debug(
    gap: &NodeGeneratedSideJoinGap,
    owned_regions: &[NodePostBooleanOwnedRegionDebug],
) -> NodeSideJoinGapDebug {
    let mut final_asphalt_area_m2 = 0.0;
    let mut final_curb_area_m2 = 0.0;
    let mut final_sidewalk_area_m2 = 0.0;
    let mut final_non_road_area_m2 = 0.0;
    for region in owned_regions.iter().filter(|region| {
        region.claim_priority == NodeGeneratedContourClaimPriority::SideJoin
            && region.source_mouth_order_index == gap.from_mouth_order_index
    }) {
        match region.kind {
            RoadSurfaceBandKind::Carriageway => final_asphalt_area_m2 += region.area_m2,
            RoadSurfaceBandKind::CurbOrShoulder => final_curb_area_m2 += region.area_m2,
            RoadSurfaceBandKind::Sidewalk => final_sidewalk_area_m2 += region.area_m2,
            _ => final_non_road_area_m2 += region.area_m2,
        }
    }
    NodeSideJoinGapDebug {
        from_mouth_order_index: gap.from_mouth_order_index,
        to_mouth_order_index: gap.to_mouth_order_index,
        from_edge_idx: gap.from_edge_idx,
        to_edge_idx: gap.to_edge_idx,
        from_side: gap.from_side,
        to_side: gap.to_side,
        angle_rad: gap.angle_rad,
        role: gap.role,
        emitted_band_kinds: gap.emitted_band_kinds.clone(),
        suppressed_band_kinds: gap.suppressed_band_kinds.clone(),
        final_asphalt_area_m2,
        final_curb_area_m2,
        final_sidewalk_area_m2,
        final_non_road_area_m2,
        final_total_area_m2: final_asphalt_area_m2
            + final_curb_area_m2
            + final_sidewalk_area_m2
            + final_non_road_area_m2,
    }
}

fn side_join_material_trim_debug_records(
    rails: &NodeRailContourSet,
    owned_regions: &[NodePostBooleanOwnedRegionDebug],
) -> Vec<NodeSideJoinMaterialTrimDebug> {
    let blocker_contours = rails
        .contours
        .iter()
        .filter(|contour| {
            contour.contributes_to_non_road_band()
                && matches!(
                    contour.purpose,
                    NodeGeneratedContourPurpose::BendSideJoin
                        | NodeGeneratedContourPurpose::JunctionSideJoin
                )
        })
        .map(overlay_contour_from_generated_contour)
        .collect::<Vec<_>>();
    let blocker_shapes = if blocker_contours.is_empty() {
        Vec::new()
    } else {
        RoadSurfaceSystem::overlay_union_contours(&blocker_contours).unwrap_or_default()
    };

    rails
        .contours
        .iter()
        .filter_map(|contour| {
            let NodeGeneratedContourKind::Band { kind } = contour.kind else {
                return None;
            };
            if !matches!(
                contour.purpose,
                NodeGeneratedContourPurpose::BendSideJoin
                    | NodeGeneratedContourPurpose::JunctionSideJoin
            ) {
                return None;
            }

            let contour_shape = RoadSurfaceSystem::overlay_union_contours(&[
                overlay_contour_from_generated_contour(contour),
            ])
            .unwrap_or_default();
            let raw_area_m2 = debug_overlay_shapes_area_m2(&contour_shape);
            let (blocker_overlap_area_m2, trimmed_area_m2) =
                if kind == RoadSurfaceBandKind::Carriageway && !blocker_shapes.is_empty() {
                    let blocker_overlap = RoadSurfaceSystem::overlay_binary_shapes(
                        &contour_shape,
                        &blocker_shapes,
                        OverlayRule::Intersect,
                    )
                    .unwrap_or_default();
                    let trimmed = RoadSurfaceSystem::overlay_binary_shapes(
                        &contour_shape,
                        &blocker_shapes,
                        OverlayRule::Difference,
                    )
                    .unwrap_or_default();
                    (
                        debug_overlay_shapes_area_m2(&blocker_overlap),
                        debug_overlay_shapes_area_m2(&trimmed),
                    )
                } else {
                    (0.0, raw_area_m2)
                };
            Some(NodeSideJoinMaterialTrimDebug {
                kind,
                purpose: contour.purpose,
                source_mouth_order_index: contour.source_mouth_order_index,
                source_band_index: contour.source_band_index,
                role: side_join_gap_role_for_source(rails, contour.source_mouth_order_index),
                raw_area_m2,
                blocker_overlap_area_m2,
                trimmed_area_m2,
                final_owned_area_m2: final_side_join_owned_area_m2(
                    owned_regions,
                    kind,
                    contour.source_mouth_order_index,
                    contour.source_band_index,
                ),
            })
        })
        .collect()
}

fn side_join_gap_role_for_source(
    rails: &NodeRailContourSet,
    source_mouth_order_index: usize,
) -> Option<NodeInputSideJoinGapRole> {
    rails
        .side_join_gaps
        .iter()
        .find(|gap| gap.from_mouth_order_index == source_mouth_order_index)
        .map(|gap| gap.role)
}

fn final_side_join_owned_area_m2(
    owned_regions: &[NodePostBooleanOwnedRegionDebug],
    kind: RoadSurfaceBandKind,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
) -> f32 {
    owned_regions
        .iter()
        .filter(|region| {
            region.kind == kind
                && region.claim_priority == NodeGeneratedContourClaimPriority::SideJoin
                && region.source_mouth_order_index == source_mouth_order_index
                && region.source_band_index == source_band_index
        })
        .map(|region| region.area_m2)
        .sum()
}

fn debug_overlay_shapes_area_m2(shapes: &NodeOverlayShapes) -> f32 {
    shapes
        .iter()
        .map(RoadSurfaceSystem::overlay_shape_area_m2)
        .sum()
}

fn overlay_contour_from_generated_contour(contour: &NodeGeneratedContour) -> NodeOverlayContour {
    contour
        .points_xz
        .iter()
        .map(|point| [point.x, point.y])
        .collect()
}

fn corner_trim_side_join_intersections(
    trim: &NodeGeneratedCornerTrim,
    side_join_contours: &[NodeSideJoinContourDebug],
) -> Vec<NodeCornerTrimSideJoinIntersectionDebug> {
    let trim_contour = overlay_contour_from_road_vec2_points(&trim.points_xz);
    let Some(trim_shapes) = RoadSurfaceSystem::overlay_union_contours(&[trim_contour]) else {
        return Vec::new();
    };

    let mut intersections = Vec::new();
    for (contour_index, contour) in side_join_contours.iter().enumerate() {
        let side_join_contour = overlay_contour_from_road_vec2_points(&contour.points_xz);
        let Some(side_join_shapes) =
            RoadSurfaceSystem::overlay_union_contours(&[side_join_contour])
        else {
            continue;
        };
        let Some(overlap_shapes) = RoadSurfaceSystem::overlay_binary_shapes(
            &trim_shapes,
            &side_join_shapes,
            OverlayRule::Intersect,
        ) else {
            continue;
        };
        let area_m2 = overlap_shapes
            .iter()
            .map(RoadSurfaceSystem::overlay_shape_area_m2)
            .sum::<f32>();
        let area_budget_m2 =
            RoadSurfaceSystem::overlay_numeric_area_budget_for_shapes(&overlap_shapes);
        if area_m2 <= area_budget_m2 {
            continue;
        }
        intersections.push(NodeCornerTrimSideJoinIntersectionDebug {
            contour_index,
            kind: contour.kind,
            purpose: contour.purpose,
            source_mouth_order_index: contour.source_mouth_order_index,
            source_band_index: contour.source_band_index,
            owner: contour.owner,
            claim_priority: contour.claim_priority,
            contributes_to_footprint: contour.contributes_to_footprint,
            contributes_to_asphalt: contour.contributes_to_asphalt,
            contributes_to_non_road_band: contour.contributes_to_non_road_band,
            area_m2,
        });
    }
    intersections
}

fn overlay_contour_from_road_vec2_points(
    points: &[super::backend::RoadVec2],
) -> NodeOverlayContour {
    points.iter().map(|point| [point.x, point.y]).collect()
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
