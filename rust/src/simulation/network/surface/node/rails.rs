//! Library-backed rail and contour generation for canonical node arrangements.

use super::arrangement::NodeBandOwner;
use super::backend::{
    ROAD_OVERLAY_COORDINATE_SCALE, RoadPolyline, RoadVec2, RoadVec3, polyline_to_road_points,
    road_points_to_polyline, road_vec3_xz as xz,
};
use super::input::{
    NodeArrangementInput, NodeInputBandInterval, NodeInputBoundaryRailRole, NodeInputMouth,
    NodeInputProfileRail,
};
use super::joins::{
    NodeInputSideJoinBand, NodeInputSideJoinBandBoundaryMode, side_join_bands_by_mouth,
};
use super::keys::{SURFACE_POLYLINE_POINT_EQUAL_EPS_M, SurfaceXzKey};
use super::segments::{
    raw_tuple_key_lies_on_segment as generated_point_key_lies_on_segment,
    raw_tuple_segment_parameter_key as generated_segment_parameter_key,
};
use super::terminal::{
    NodeTerminalCapBand, TerminalCapBandRole, TerminalCapGenerationError,
    terminal_cap_bands_by_mouth,
};
use super::{
    NODE_OVERLAY_MIN_AREA_M2, NodeOverlayContour, NodeOverlayShapes, RoadSurfaceBandKind,
    RoadSurfaceSystem, RoadSurfaceVisualNodePieceKind,
};
use cavalier_contours::polyline::{PlineCreation, PlineSource, PlineSourceMut};
use i_overlay::core::overlay_rule::OverlayRule;
use std::collections::{BTreeMap, BTreeSet};

mod caps_and_joins;
mod contacts;

use caps_and_joins::{push_side_join_band_contours, push_terminal_cap_band_contours};
use contacts::{
    append_generated_material_point_contact_constraints,
    append_generated_same_band_contact_constraints,
    append_source_authorized_raised_step_point_contacts,
    generated_raised_step_boundary_role_for_owner, node_generated_contact_contours,
    node_generated_contact_source_constraints,
    node_generated_contact_sources_from_contour_backed_contacts,
    raised_step_band_kinds_can_contact, retain_source_authorized_generated_contact_constraints,
    validate_generated_contact_constraint_endpoints_from_sources,
};

const RAIL_CONTOUR_POINT_EQUAL_EPS_M: f64 = SURFACE_POLYLINE_POINT_EQUAL_EPS_M;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) enum NodeGeneratedContourKind {
    FullRoadbed,
    Band { kind: RoadSurfaceBandKind },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) enum NodeGeneratedContourPurpose {
    FullRoadbedCorridor,
    CarriagewayCorridor,
    CarriagewayOwnerCarrier,
    NonRoadBand,
    TerminalCap,
    BendSideJoin,
    JunctionSideJoin,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) enum NodeRailConstraintKind {
    FullRoadbedContour,
    BandContour {
        kind: RoadSurfaceBandKind,
    },
    SpanHandoff {
        kind: RoadSurfaceBandKind,
    },
    FootprintSeam {
        adjacent_kind: RoadSurfaceBandKind,
    },
    AsphaltBoundary {
        adjacent_kind: RoadSurfaceBandKind,
    },
    RaisedStepContact,
    BandBoundary {
        left_kind: RoadSurfaceBandKind,
        right_kind: RoadSurfaceBandKind,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct NodeRailContourSet {
    pub(crate) node_id: u32,
    pub(crate) piece_kind: RoadSurfaceVisualNodePieceKind,
    pub(crate) contours: Vec<NodeGeneratedContour>,
    pub(crate) constraints: Vec<NodeRailConstraint>,
    pub(crate) height_carrier_points_by_source:
        BTreeMap<(RoadSurfaceBandKind, usize, usize), Vec<RoadVec2>>,
}

#[derive(Clone, Debug)]
pub(crate) struct NodeGeneratedContour {
    pub(crate) kind: NodeGeneratedContourKind,
    pub(crate) purpose: NodeGeneratedContourPurpose,
    pub(crate) source_mouth_order_index: usize,
    pub(crate) source_band_index: Option<usize>,
    pub(crate) owner: Option<NodeBandOwner>,
    pub(crate) claim_priority: NodeGeneratedContourClaimPriority,
    pub(crate) points_xz: Vec<RoadVec2>,
    pub(crate) height_points_world: Option<Vec<RoadVec3>>,
    pub(crate) backend_polyline: RoadPolyline,
}

impl NodeGeneratedContour {
    pub(crate) fn contributes_to_footprint(&self) -> bool {
        self.kind == NodeGeneratedContourKind::FullRoadbed
            && matches!(
                self.purpose,
                NodeGeneratedContourPurpose::FullRoadbedCorridor
                    | NodeGeneratedContourPurpose::TerminalCap
                    | NodeGeneratedContourPurpose::BendSideJoin
            )
    }

    pub(crate) fn contributes_to_asphalt(&self) -> bool {
        matches!(
            self.kind,
            NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::Carriageway,
            }
        ) && matches!(
            self.purpose,
            NodeGeneratedContourPurpose::CarriagewayCorridor
                | NodeGeneratedContourPurpose::BendSideJoin
        )
    }

    pub(crate) fn claims_asphalt_owner_region(&self) -> bool {
        matches!(
            self.kind,
            NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::Carriageway,
            }
        ) && self.owner.is_some()
            && matches!(
                self.purpose,
                NodeGeneratedContourPurpose::CarriagewayCorridor
                    | NodeGeneratedContourPurpose::CarriagewayOwnerCarrier
                    | NodeGeneratedContourPurpose::TerminalCap
                    | NodeGeneratedContourPurpose::BendSideJoin
            )
    }

    pub(crate) fn contributes_to_non_road_band(&self) -> bool {
        matches!(
            self.kind,
            NodeGeneratedContourKind::Band { kind }
                if kind != RoadSurfaceBandKind::Carriageway
        ) && matches!(
            self.purpose,
            NodeGeneratedContourPurpose::NonRoadBand
                | NodeGeneratedContourPurpose::TerminalCap
                | NodeGeneratedContourPurpose::BendSideJoin
                | NodeGeneratedContourPurpose::JunctionSideJoin
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) enum NodeGeneratedContourClaimPriority {
    JoinOrCap,
    SideJoin,
    MouthBand,
    Footprint,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeRailConstraint {
    pub(crate) constraint_index: usize,
    pub(crate) kind: NodeRailConstraintKind,
    pub(crate) source_mouth_order_index: usize,
    pub(crate) source_band_index: Option<usize>,
    pub(crate) source_boundary_index: Option<usize>,
    pub(crate) owner: Option<NodeBandOwner>,
    pub(crate) opposite_owner: Option<NodeBandOwner>,
    pub(crate) points_xz: Vec<RoadVec2>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NodeRailGenerationError {
    EmptyInput {
        node_id: u32,
    },
    DegenerateContour {
        kind: NodeGeneratedContourKind,
        mouth_order_index: usize,
        band_index: Option<usize>,
        area_m2: f64,
        vertex_count: usize,
    },
    DegenerateConstraint {
        kind: NodeRailConstraintKind,
        mouth_order_index: usize,
        band_index: Option<usize>,
        boundary_index: Option<usize>,
        path_length_m: f64,
        vertex_count: usize,
    },
    InvalidHeightCarrier {
        kind: NodeGeneratedContourKind,
        mouth_order_index: usize,
        band_index: Option<usize>,
        reason: &'static str,
    },
    NonCanonicalGeneratedContactEndpoint {
        kind: NodeRailConstraintKind,
        mouth_order_index: usize,
        band_index: Option<usize>,
        owner: Option<NodeBandOwner>,
        opposite_owner: Option<NodeBandOwner>,
        point_x_key: i64,
        point_z_key: i64,
    },
    TerminalCapGeneration {
        error: TerminalCapGenerationError,
    },
}

struct MouthOwners {
    band_owners: Vec<NodeBandOwner>,
    terminal_cap_band_owners: Vec<NodeBandOwner>,
    side_join_band_owners: Vec<NodeBandOwner>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum GeneratedSameBandBoundaryRole {
    LowerSide,
    RaisedSide,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct GeneratedRaisedStepOwnerPair {
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GeneratedPointContourLocation {
    Outside,
    Boundary,
    Inside,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct GeneratedContourEdgeKey {
    start: NodeRailPointKey,
    end: NodeRailPointKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct GeneratedContourDirectedEdge {
    start: NodeRailPointKey,
    end: NodeRailPointKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct GeneratedSameBandContactConstraint {
    kind: NodeRailConstraintKind,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    start: NodeRailPointKey,
    end: NodeRailPointKey,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct GeneratedSameBandContactConstraintKey {
    kind: NodeRailConstraintKind,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    start: NodeRailPointKey,
    end: NodeRailPointKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct GeneratedMaterialPointContactAuthority {
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
    owner: Option<NodeBandOwner>,
    opposite_owner: Option<NodeBandOwner>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct GeneratedRaisedStepEndpointSource {
    constraint_index: usize,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
    owners: [NodeBandOwner; 2],
}

#[derive(Clone, Copy)]
struct RaisedStepSourceConstraint<'a> {
    source: GeneratedRaisedStepEndpointSource,
    constraint: &'a NodeRailConstraint,
}

struct RaisedStepSourceAuthority<'a> {
    constraints: Vec<RaisedStepSourceConstraint<'a>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct SourceAuthorizedTargetGroupKey {
    owner: NodeBandOwner,
    kind: RoadSurfaceBandKind,
    claim_priority: NodeGeneratedContourClaimPriority,
}

#[derive(Clone, Debug)]
struct SourceAuthorizedTargetGroup {
    key: SourceAuthorizedTargetGroupKey,
    contour_indices: Vec<usize>,
    shapes: NodeOverlayShapes,
    shape_edges: Vec<GeneratedContourDirectedEdge>,
}

type NodeRailPointKey = (i64, i64);

impl GeneratedContourEdgeKey {
    fn new(a: NodeRailPointKey, b: NodeRailPointKey) -> Self {
        if a <= b {
            Self { start: a, end: b }
        } else {
            Self { start: b, end: a }
        }
    }
}

impl GeneratedSameBandContactConstraint {
    fn key(self) -> GeneratedSameBandContactConstraintKey {
        let edge = GeneratedContourEdgeKey::new(self.start, self.end);
        GeneratedSameBandContactConstraintKey {
            kind: self.kind,
            owner: self.owner,
            opposite_owner: self.opposite_owner,
            start: edge.start,
            end: edge.end,
        }
    }
}

impl GeneratedRaisedStepOwnerPair {
    fn new(a: NodeBandOwner, b: NodeBandOwner) -> Option<Self> {
        if a == b || !raised_step_band_kinds_can_contact(a.kind(), b.kind()) {
            return None;
        }
        let (owner, opposite_owner) = if a <= b { (a, b) } else { (b, a) };
        Some(Self {
            owner,
            opposite_owner,
        })
    }
}

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn build_node_rail_contours_from_input(
        input: &NodeArrangementInput,
    ) -> Result<NodeRailContourSet, NodeRailGenerationError> {
        NodeRailContourSet::from_input(input)
    }
}

impl NodeRailContourSet {
    pub(crate) fn from_input(
        input: &NodeArrangementInput,
    ) -> Result<Self, NodeRailGenerationError> {
        if input.mouths.is_empty() {
            return Err(NodeRailGenerationError::EmptyInput {
                node_id: input.node_id,
            });
        }

        let terminal_cap_bands_by_mouth = terminal_cap_bands_by_mouth(input)
            .map_err(|error| NodeRailGenerationError::TerminalCapGeneration { error })?;
        let side_join_bands_by_mouth = side_join_bands_by_mouth(input);
        let owners_by_mouth = owners_by_mouth(
            input,
            &terminal_cap_bands_by_mouth,
            &side_join_bands_by_mouth,
        );
        let mut contours = Vec::new();
        let mut constraints = Vec::new();
        let mut height_carrier_points_by_source =
            BTreeMap::<(RoadSurfaceBandKind, usize, usize), Vec<RoadVec2>>::new();

        for (mouth_index, (mouth, mouth_owners)) in
            input.mouths.iter().zip(&owners_by_mouth).enumerate()
        {
            let side_join_bands = side_join_bands_by_mouth
                .get(mouth_index)
                .map_or(&[] as &[NodeInputSideJoinBand], Vec::as_slice);
            let terminal_cap_bands = terminal_cap_bands_by_mouth
                .get(mouth_index)
                .map_or(&[] as &[NodeTerminalCapBand], Vec::as_slice);
            push_full_roadbed_contour(mouth, &mut contours, &mut constraints)?;
            push_raw_carriageway_corridor_contour(
                input.piece_kind,
                mouth,
                &mut contours,
                &mut constraints,
            )?;

            for (band_index, interval) in mouth.band_intervals.iter().enumerate() {
                push_band_height_carrier_points(
                    &mut height_carrier_points_by_source,
                    mouth.order_index,
                    interval.band_index,
                    interval.band_kind,
                    interval_height_carrier_points(interval),
                );
                let owner = mouth_owners.band_owners[band_index];
                push_band_contour(
                    input.piece_kind,
                    mouth,
                    interval,
                    owner,
                    &mut contours,
                    &mut constraints,
                )?;
            }
            for cap_band in terminal_cap_bands {
                push_band_height_carrier_points(
                    &mut height_carrier_points_by_source,
                    mouth.order_index,
                    cap_band.source_band_index,
                    cap_band.band_kind,
                    cap_band
                        .contour_world
                        .iter()
                        .chain(&cap_band.inner_path_world)
                        .chain(&cap_band.outer_path_world)
                        .copied(),
                );
            }
            for side_join_band in side_join_bands {
                push_band_height_carrier_points(
                    &mut height_carrier_points_by_source,
                    mouth.order_index,
                    side_join_band.source_band_index,
                    side_join_band.band_kind,
                    side_join_band.contour_world.iter().copied(),
                );
            }

            push_terminal_cap_band_contours(
                input.piece_kind,
                mouth,
                terminal_cap_bands,
                mouth_owners,
                &mouth_owners.terminal_cap_band_owners,
                &mut contours,
                &mut constraints,
            )?;
            push_side_join_band_contours(
                input.piece_kind,
                mouth,
                side_join_bands,
                mouth_owners,
                &mouth_owners.side_join_band_owners,
                &mut contours,
                &mut constraints,
            )?;
            for boundary_rail in &mouth.boundary_rails {
                let (owner, opposite_owner) =
                    boundary_owners(boundary_rail.boundary_index, &mouth_owners.band_owners);
                push_boundary_constraint(
                    mouth,
                    boundary_rail.boundary_index,
                    boundary_rail.role,
                    owner,
                    opposite_owner,
                    &mut constraints,
                )?;
            }

            for profile_rail in &mouth.mouth_rails {
                let owner = mouth_owners.band_owners[profile_rail.band_index];
                push_span_handoff_constraint(mouth, profile_rail, owner, &mut constraints)?;
            }
        }
        let source_constraint_count = constraints.len();
        node_generated_contact_contours(&mut contours, &mut constraints)?;
        append_source_authorized_raised_step_point_contacts(
            input.piece_kind,
            &contours,
            &mut constraints,
        );
        append_generated_material_point_contact_constraints(&contours, &mut constraints);
        append_source_authorized_raised_step_point_contacts(
            input.piece_kind,
            &contours,
            &mut constraints,
        );
        node_generated_contact_contours(&mut contours, &mut constraints)?;
        append_generated_same_band_contact_constraints(
            input.piece_kind,
            &contours,
            &mut constraints,
        );
        node_generated_contact_contours(&mut contours, &mut constraints)?;
        let mut validation_constraints = constraints.clone();
        node_generated_contact_source_constraints(
            &contours,
            &mut validation_constraints,
            source_constraint_count,
        );
        node_generated_contact_sources_from_contour_backed_contacts(
            &contours,
            &mut validation_constraints,
            source_constraint_count,
        );
        let authority_constraints = validation_constraints.clone();
        retain_source_authorized_generated_contact_constraints(
            &contours,
            &authority_constraints,
            &mut constraints,
            source_constraint_count,
        );
        retain_source_authorized_generated_contact_constraints(
            &contours,
            &authority_constraints,
            &mut validation_constraints,
            source_constraint_count,
        );
        validate_generated_contact_constraint_endpoints_from_sources(
            &contours,
            &validation_constraints,
            source_constraint_count,
        )?;
        Ok(Self {
            node_id: input.node_id,
            piece_kind: input.piece_kind,
            contours,
            constraints,
            height_carrier_points_by_source,
        })
    }
}

fn push_full_roadbed_contour(
    mouth: &NodeInputMouth,
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let first = mouth
        .boundary_rails
        .first()
        .expect("validated input has rails");
    let last = mouth
        .boundary_rails
        .last()
        .expect("validated input has rails");
    let mut points = Vec::new();
    push_road_path_point(&mut points, xz(first.mouth_world));
    push_road_path_point(&mut points, xz(last.mouth_world));
    append_world_path_xz(&mut points, last.path_world.iter().skip(1));
    append_world_path_xz(&mut points, first.path_world.iter().rev());
    remove_closing_road_path_duplicate(&mut points);
    push_generated_contour(
        NodeGeneratedContourKind::FullRoadbed,
        mouth.order_index,
        None,
        None,
        NodeGeneratedContourClaimPriority::Footprint,
        NodeRailConstraintKind::FullRoadbedContour,
        points,
        None,
        contours,
        constraints,
    )
}

fn push_raw_carriageway_corridor_contour(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    mouth: &NodeInputMouth,
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    if piece_kind == RoadSurfaceVisualNodePieceKind::Terminal {
        return Ok(());
    }

    let Some(first_carriageway_index) = mouth
        .band_intervals
        .iter()
        .position(|interval| interval.band_kind == RoadSurfaceBandKind::Carriageway)
    else {
        return Ok(());
    };
    let Some(last_carriageway_index) = mouth
        .band_intervals
        .iter()
        .rposition(|interval| interval.band_kind == RoadSurfaceBandKind::Carriageway)
    else {
        return Ok(());
    };

    let first = &mouth.band_intervals[first_carriageway_index];
    let last = &mouth.band_intervals[last_carriageway_index];
    let mut points_world = Vec::new();
    push_world_path_point(&mut points_world, first.mouth_start_world);
    push_world_path_point(&mut points_world, last.mouth_end_world);
    append_world_path_points(&mut points_world, last.end_path_world.iter().skip(1));
    append_world_path_points(&mut points_world, first.start_path_world.iter().rev());
    remove_closing_world_path_duplicate(&mut points_world);
    let points = points_world.iter().copied().map(xz).collect::<Vec<_>>();

    push_generated_contour_with_purpose(
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::Carriageway,
        },
        NodeGeneratedContourPurpose::CarriagewayCorridor,
        mouth.order_index,
        None,
        None,
        NodeGeneratedContourClaimPriority::Footprint,
        NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::Carriageway,
        },
        points,
        None,
        contours,
        constraints,
    )
}

fn push_band_contour(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    mouth: &NodeInputMouth,
    interval: &NodeInputBandInterval,
    owner: NodeBandOwner,
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let kind = NodeGeneratedContourKind::Band {
        kind: interval.band_kind,
    };
    let purpose = band_contour_purpose(piece_kind, interval.band_kind);
    let last_band_index = mouth.band_intervals.len().saturating_sub(1);
    if mouth.uses_sampled_band_domain_paths {
        let uses_paired_sampled_paths =
            interval.start_path_world.len() > 2 || interval.end_path_world.len() > 2;
        let uses_explicit_outer_chord = interval.band_index == 0
            && interval.start_path_world.len() > 2
            && interval.end_path_world.len() == 2
            || interval.band_index == last_band_index
                && interval.start_path_world.len() == 2
                && interval.end_path_world.len() > 2;
        if uses_paired_sampled_paths
            && interval.start_path_world.len() != interval.end_path_world.len()
            && !uses_explicit_outer_chord
        {
            return Err(NodeRailGenerationError::InvalidHeightCarrier {
                kind,
                mouth_order_index: mouth.order_index,
                band_index: Some(interval.band_index),
                reason: "mismatched_path_height_carrier_lengths",
            });
        }
        if uses_paired_sampled_paths
            && interval.start_path_world.len() == interval.end_path_world.len()
        {
            return push_path_band_contour(
                kind,
                purpose,
                mouth.order_index,
                Some(interval.band_index),
                Some(owner),
                NodeGeneratedContourClaimPriority::MouthBand,
                NodeRailConstraintKind::BandContour {
                    kind: interval.band_kind,
                },
                &interval.start_path_world,
                &interval.end_path_world,
                contours,
                constraints,
            );
        }
    }
    if interval.band_index == 0 && interval.start_path_world.len() > 2 {
        let inner_path = subdivided_world_chord(
            interval.mouth_end_world,
            interval.endpoint_end_world,
            interval.start_path_world.len(),
        );
        return push_path_strip_contours(
            kind,
            purpose,
            mouth.order_index,
            Some(interval.band_index),
            Some(owner),
            NodeGeneratedContourClaimPriority::MouthBand,
            NodeRailConstraintKind::BandContour {
                kind: interval.band_kind,
            },
            &interval.start_path_world,
            &inner_path,
            contours,
            constraints,
        );
    }
    if interval.band_index == last_band_index && interval.end_path_world.len() > 2 {
        let inner_path = subdivided_world_chord(
            interval.mouth_start_world,
            interval.endpoint_start_world,
            interval.end_path_world.len(),
        );
        return push_path_strip_contours(
            kind,
            purpose,
            mouth.order_index,
            Some(interval.band_index),
            Some(owner),
            NodeGeneratedContourClaimPriority::MouthBand,
            NodeRailConstraintKind::BandContour {
                kind: interval.band_kind,
            },
            &inner_path,
            &interval.end_path_world,
            contours,
            constraints,
        );
    }
    let mut points_world = Vec::new();
    push_world_path_point(&mut points_world, interval.mouth_start_world);
    push_world_path_point(&mut points_world, interval.mouth_end_world);
    if interval.band_index == last_band_index {
        append_world_path_points(&mut points_world, interval.end_path_world.iter().skip(1));
    } else {
        push_world_path_point(&mut points_world, interval.endpoint_end_world);
    }
    if interval.band_index == 0 {
        append_world_path_points(&mut points_world, interval.start_path_world.iter().rev());
    } else {
        push_world_path_point(&mut points_world, interval.endpoint_start_world);
    }
    remove_closing_world_path_duplicate(&mut points_world);
    let points = points_world.iter().copied().map(xz).collect::<Vec<_>>();
    push_generated_contour_with_purpose(
        kind,
        purpose,
        mouth.order_index,
        Some(interval.band_index),
        Some(owner),
        NodeGeneratedContourClaimPriority::MouthBand,
        NodeRailConstraintKind::BandContour {
            kind: interval.band_kind,
        },
        points,
        Some(points_world),
        contours,
        constraints,
    )
}

fn band_contour_purpose(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    band_kind: RoadSurfaceBandKind,
) -> NodeGeneratedContourPurpose {
    if piece_kind != RoadSurfaceVisualNodePieceKind::Terminal
        && band_kind == RoadSurfaceBandKind::Carriageway
    {
        NodeGeneratedContourPurpose::CarriagewayOwnerCarrier
    } else {
        default_generated_contour_purpose(NodeGeneratedContourKind::Band { kind: band_kind })
    }
}

fn interval_height_carrier_points(
    interval: &NodeInputBandInterval,
) -> impl Iterator<Item = RoadVec3> + '_ {
    [
        interval.endpoint_start_world,
        interval.endpoint_end_world,
        interval.mouth_end_world,
        interval.mouth_start_world,
    ]
    .into_iter()
    .chain(interval.start_path_world.iter().copied())
    .chain(interval.end_path_world.iter().copied())
}

fn push_band_height_carrier_points(
    points_by_source: &mut BTreeMap<(RoadSurfaceBandKind, usize, usize), Vec<RoadVec2>>,
    mouth_order_index: usize,
    source_band_index: usize,
    kind: RoadSurfaceBandKind,
    points_world: impl IntoIterator<Item = RoadVec3>,
) {
    let points = points_by_source
        .entry((kind, mouth_order_index, source_band_index))
        .or_default();
    for point in points_world {
        push_road_path_point(points, xz(point));
    }
}

fn push_path_strip_contours(
    kind: NodeGeneratedContourKind,
    purpose: NodeGeneratedContourPurpose,
    mouth_order_index: usize,
    band_index: Option<usize>,
    owner: Option<NodeBandOwner>,
    claim_priority: NodeGeneratedContourClaimPriority,
    constraint_kind: NodeRailConstraintKind,
    start_path_world: &[RoadVec3],
    end_path_world: &[RoadVec3],
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    validate_paired_path_band_height_carrier(
        kind,
        mouth_order_index,
        band_index,
        start_path_world,
        end_path_world,
    )?;
    let mut first_error = None;
    let mut pushed = false;
    for points_world in path_strip_contours_world(start_path_world, end_path_world) {
        let points = points_world.iter().copied().map(xz).collect::<Vec<_>>();
        match push_generated_contour_with_purpose(
            kind,
            purpose,
            mouth_order_index,
            band_index,
            owner,
            claim_priority,
            constraint_kind,
            points,
            Some(points_world),
            contours,
            constraints,
        ) {
            Ok(()) => pushed = true,
            Err(error) => {
                first_error.get_or_insert(error);
            }
        };
    }
    if pushed {
        Ok(())
    } else {
        Err(
            first_error.unwrap_or(NodeRailGenerationError::DegenerateContour {
                kind,
                mouth_order_index,
                band_index,
                area_m2: 0.0,
                vertex_count: 0,
            }),
        )
    }
}

fn push_generated_contour(
    kind: NodeGeneratedContourKind,
    mouth_order_index: usize,
    band_index: Option<usize>,
    owner: Option<NodeBandOwner>,
    claim_priority: NodeGeneratedContourClaimPriority,
    constraint_kind: NodeRailConstraintKind,
    points: Vec<RoadVec2>,
    height_points_world: Option<Vec<RoadVec3>>,
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let purpose = default_generated_contour_purpose(kind);
    push_generated_contour_with_purpose(
        kind,
        purpose,
        mouth_order_index,
        band_index,
        owner,
        claim_priority,
        constraint_kind,
        points,
        height_points_world,
        contours,
        constraints,
    )
}

fn push_generated_contour_with_purpose(
    kind: NodeGeneratedContourKind,
    purpose: NodeGeneratedContourPurpose,
    mouth_order_index: usize,
    band_index: Option<usize>,
    owner: Option<NodeBandOwner>,
    claim_priority: NodeGeneratedContourClaimPriority,
    constraint_kind: NodeRailConstraintKind,
    points: Vec<RoadVec2>,
    height_points_world: Option<Vec<RoadVec3>>,
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let contour = cleaned_closed_contour(kind, mouth_order_index, band_index, points)?;
    let points_xz = polyline_to_road_points(&contour);
    let height_points_world = match height_points_world.as_deref() {
        Some(points_world) => Some(
            align_height_points_to_contour(&points_xz, points_world).ok_or(
                NodeRailGenerationError::InvalidHeightCarrier {
                    kind,
                    mouth_order_index,
                    band_index,
                    reason: "height_points_do_not_match_contour",
                },
            )?,
        ),
        None => None,
    };
    contours.push(NodeGeneratedContour {
        kind,
        purpose,
        source_mouth_order_index: mouth_order_index,
        source_band_index: band_index,
        owner,
        claim_priority,
        points_xz: points_xz.clone(),
        height_points_world,
        backend_polyline: contour,
    });
    push_constraint(
        constraints,
        constraint_kind,
        mouth_order_index,
        band_index,
        None,
        owner,
        None,
        points_xz,
    )
}

fn default_generated_contour_purpose(
    kind: NodeGeneratedContourKind,
) -> NodeGeneratedContourPurpose {
    match kind {
        NodeGeneratedContourKind::FullRoadbed => NodeGeneratedContourPurpose::FullRoadbedCorridor,
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::Carriageway,
        } => NodeGeneratedContourPurpose::CarriagewayCorridor,
        NodeGeneratedContourKind::Band { .. } => NodeGeneratedContourPurpose::NonRoadBand,
    }
}

fn push_path_band_contour(
    kind: NodeGeneratedContourKind,
    purpose: NodeGeneratedContourPurpose,
    mouth_order_index: usize,
    band_index: Option<usize>,
    owner: Option<NodeBandOwner>,
    claim_priority: NodeGeneratedContourClaimPriority,
    constraint_kind: NodeRailConstraintKind,
    start_path_world: &[RoadVec3],
    end_path_world: &[RoadVec3],
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    validate_paired_path_band_height_carrier(
        kind,
        mouth_order_index,
        band_index,
        start_path_world,
        end_path_world,
    )?;
    let mut points_world = Vec::with_capacity(start_path_world.len() + end_path_world.len());
    append_world_path_points(&mut points_world, start_path_world.iter());
    append_world_path_points(&mut points_world, end_path_world.iter().rev());
    remove_closing_world_path_duplicate(&mut points_world);
    let points = points_world.iter().copied().map(xz).collect::<Vec<_>>();
    push_generated_contour_with_purpose(
        kind,
        purpose,
        mouth_order_index,
        band_index,
        owner,
        claim_priority,
        constraint_kind,
        points,
        Some(points_world),
        contours,
        constraints,
    )
}

fn path_strip_contours_world(
    start_path_world: &[RoadVec3],
    end_path_world: &[RoadVec3],
) -> Vec<Vec<RoadVec3>> {
    let point_count = start_path_world.len();
    if point_count < 2 {
        return Vec::new();
    }
    let mut strips = Vec::with_capacity(point_count - 1);
    for index in 0..point_count - 1 {
        let mut points = Vec::with_capacity(4);
        push_world_path_point(&mut points, start_path_world[index]);
        push_world_path_point(&mut points, end_path_world[index]);
        push_world_path_point(&mut points, end_path_world[index + 1]);
        push_world_path_point(&mut points, start_path_world[index + 1]);
        remove_closing_world_path_duplicate(&mut points);
        strips.push(points);
    }
    strips
}

fn validate_paired_path_band_height_carrier(
    kind: NodeGeneratedContourKind,
    mouth_order_index: usize,
    band_index: Option<usize>,
    start_path_world: &[RoadVec3],
    end_path_world: &[RoadVec3],
) -> Result<(), NodeRailGenerationError> {
    if start_path_world.len() != end_path_world.len() {
        return Err(NodeRailGenerationError::InvalidHeightCarrier {
            kind,
            mouth_order_index,
            band_index,
            reason: "mismatched_path_height_carrier_lengths",
        });
    }
    if start_path_world.len() < 2 {
        return Err(NodeRailGenerationError::InvalidHeightCarrier {
            kind,
            mouth_order_index,
            band_index,
            reason: "too_few_path_height_carrier_points",
        });
    }
    Ok(())
}

fn subdivided_world_chord(start: RoadVec3, end: RoadVec3, point_count: usize) -> Vec<RoadVec3> {
    if point_count < 2 {
        return vec![start, end];
    }
    (0..point_count)
        .map(|index| {
            let t = index as f64 / (point_count - 1) as f64;
            start + (end - start) * t
        })
        .collect()
}

fn push_generated_band_constraint(
    constraints: &mut Vec<NodeRailConstraint>,
    kind: NodeRailConstraintKind,
    source_mouth_order_index: usize,
    source_band_index: usize,
    owner: NodeBandOwner,
    opposite_owner: Option<NodeBandOwner>,
    start: RoadVec2,
    end: RoadVec2,
) -> Result<(), NodeRailGenerationError> {
    if road_point_key(start) == road_point_key(end) {
        return Ok(());
    }
    push_owned_rail_constraint(
        constraints,
        kind,
        source_mouth_order_index,
        Some(source_band_index),
        None,
        Some(owner),
        opposite_owner,
        vec![start, end],
    )
}

fn push_generated_band_path_constraint(
    constraints: &mut Vec<NodeRailConstraint>,
    kind: NodeRailConstraintKind,
    source_mouth_order_index: usize,
    source_band_index: usize,
    owner: NodeBandOwner,
    opposite_owner: Option<NodeBandOwner>,
    points: Vec<RoadVec2>,
) -> Result<(), NodeRailGenerationError> {
    let Some(points) = clean_generated_constraint_path(points) else {
        return Ok(());
    };
    push_owned_rail_constraint(
        constraints,
        kind,
        source_mouth_order_index,
        Some(source_band_index),
        None,
        Some(owner),
        opposite_owner,
        points,
    )
}

fn clean_generated_constraint_path(points: Vec<RoadVec2>) -> Option<Vec<RoadVec2>> {
    let mut cleaned = Vec::with_capacity(points.len());
    for point in points {
        push_road_path_point(&mut cleaned, point);
    }
    if cleaned
        .windows(2)
        .any(|segment| road_point_key(segment[0]) != road_point_key(segment[1]))
    {
        let raw = road_points_to_polyline(cleaned, false);
        let rail = RoadPolyline::create_from_remove_repeat(&raw, RAIL_CONTOUR_POINT_EQUAL_EPS_M);
        (rail.vertex_count() >= 2 && rail.path_length() > RAIL_CONTOUR_POINT_EQUAL_EPS_M)
            .then(|| polyline_to_road_points(&rail))
    } else {
        None
    }
}

fn open_world_path_xz(
    path_world: &[RoadVec3],
    mouth_world: RoadVec3,
    endpoint_world: RoadVec3,
) -> Vec<RoadVec2> {
    let mut points = Vec::new();
    push_road_path_point(&mut points, xz(mouth_world));
    append_world_path_xz(&mut points, path_world.iter());
    push_road_path_point(&mut points, xz(endpoint_world));
    points
}

fn append_world_path_xz<'a>(
    points: &mut Vec<RoadVec2>,
    path_world: impl IntoIterator<Item = &'a RoadVec3>,
) {
    for point in path_world {
        push_road_path_point(points, xz(*point));
    }
}

fn append_world_path_points<'a>(
    points: &mut Vec<RoadVec3>,
    path_world: impl IntoIterator<Item = &'a RoadVec3>,
) {
    for point in path_world {
        push_world_path_point(points, *point);
    }
}

fn push_road_path_point(points: &mut Vec<RoadVec2>, point: RoadVec2) {
    if points
        .last()
        .is_none_or(|last| road_point_key(*last) != road_point_key(point))
    {
        points.push(point);
    }
}

fn push_world_path_point(points: &mut Vec<RoadVec3>, point: RoadVec3) {
    if points
        .last()
        .is_none_or(|last| road_point_key(xz(*last)) != road_point_key(xz(point)))
    {
        points.push(point);
    }
}

fn remove_closing_road_path_duplicate(points: &mut Vec<RoadVec2>) {
    if points.len() > 1
        && road_point_key(points[0]) == road_point_key(*points.last().expect("len checked"))
    {
        points.pop();
    }
}

fn remove_closing_world_path_duplicate(points: &mut Vec<RoadVec3>) {
    if points.len() > 1
        && road_point_key(xz(points[0])) == road_point_key(xz(*points.last().expect("len checked")))
    {
        points.pop();
    }
}

fn align_height_points_to_contour(
    contour_points_xz: &[RoadVec2],
    source_points_world: &[RoadVec3],
) -> Option<Vec<RoadVec3>> {
    let mut height_by_key = BTreeMap::<NodeRailPointKey, f64>::new();
    for point in source_points_world {
        let key = road_point_key(xz(*point));
        if let Some(existing_height_m) = height_by_key.get(&key)
            && (*existing_height_m - point.y).abs() > f64::EPSILON
        {
            return None;
        }
        height_by_key.insert(key, point.y);
    }
    contour_points_xz
        .iter()
        .copied()
        .map(|point_xz| {
            height_by_key
                .get(&road_point_key(point_xz))
                .copied()
                .map(|height_m| RoadVec3::new(point_xz.x, height_m, point_xz.y))
        })
        .collect()
}

fn align_height_points_to_source_contours(
    contour_points_xz: &[RoadVec2],
    source_contours_world: &[&[RoadVec3]],
) -> Option<Vec<RoadVec3>> {
    contour_points_xz
        .iter()
        .copied()
        .map(|point_xz| {
            height_on_source_contours(point_xz, source_contours_world)
                .map(|height_m| RoadVec3::new(point_xz.x, height_m, point_xz.y))
        })
        .collect()
}

fn height_on_source_contours(
    point_xz: RoadVec2,
    source_contours_world: &[&[RoadVec3]],
) -> Option<f64> {
    let key = road_point_key(point_xz);
    let mut height_m: Option<f64> = None;
    for source_contour_world in source_contours_world {
        if let Some(candidate_height_m) = height_on_source_contour_edge(key, source_contour_world) {
            if let Some(existing_height_m) = height_m
                && (existing_height_m - candidate_height_m).abs() > f64::EPSILON
            {
                return None;
            }
            height_m = Some(candidate_height_m);
        }
    }
    height_m
}

fn height_on_source_contour_edge(
    key: NodeRailPointKey,
    source_points_world: &[RoadVec3],
) -> Option<f64> {
    if source_points_world.is_empty() {
        return None;
    }
    for point in source_points_world {
        if road_point_key(xz(*point)) == key {
            return Some(point.y);
        }
    }
    for index in 0..source_points_world.len() {
        let next = (index + 1) % source_points_world.len();
        let start = road_point_key(xz(source_points_world[index]));
        let end = road_point_key(xz(source_points_world[next]));
        if start == end || !generated_point_key_lies_on_segment(key, start, end) {
            continue;
        }
        if let Some(height_m) = height_for_key_on_generated_edge(
            key,
            start,
            end,
            source_points_world[index].y,
            source_points_world[next].y,
        ) {
            return Some(height_m);
        }
    }
    None
}

fn push_boundary_constraint(
    mouth: &NodeInputMouth,
    boundary_index: usize,
    role: NodeInputBoundaryRailRole,
    owner: Option<NodeBandOwner>,
    opposite_owner: Option<NodeBandOwner>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let rail = &mouth.boundary_rails[boundary_index];
    push_owned_rail_constraint(
        constraints,
        boundary_constraint_kind(role),
        mouth.order_index,
        None,
        Some(boundary_index),
        owner,
        opposite_owner,
        open_world_path_xz(&rail.path_world, rail.mouth_world, rail.endpoint_world),
    )
}

fn push_span_handoff_constraint(
    mouth: &NodeInputMouth,
    profile_rail: &NodeInputProfileRail,
    owner: NodeBandOwner,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    push_constraint(
        constraints,
        NodeRailConstraintKind::SpanHandoff {
            kind: profile_rail.band_kind,
        },
        mouth.order_index,
        Some(profile_rail.band_index),
        None,
        Some(owner),
        None,
        vec![xz(profile_rail.start_world), xz(profile_rail.end_world)],
    )
}

fn push_owned_rail_constraint(
    constraints: &mut Vec<NodeRailConstraint>,
    kind: NodeRailConstraintKind,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
    source_boundary_index: Option<usize>,
    owner: Option<NodeBandOwner>,
    opposite_owner: Option<NodeBandOwner>,
    points: Vec<RoadVec2>,
) -> Result<(), NodeRailGenerationError> {
    if kind == NodeRailConstraintKind::RaisedStepContact {
        let (Some(owner), Some(opposite_owner)) = (owner, opposite_owner) else {
            return Ok(());
        };
        let Some(pair) = GeneratedRaisedStepOwnerPair::new(owner, opposite_owner) else {
            return Ok(());
        };
        return push_constraint(
            constraints,
            kind,
            source_mouth_order_index,
            source_band_index,
            source_boundary_index,
            Some(pair.owner),
            Some(pair.opposite_owner),
            points,
        );
    }
    push_constraint(
        constraints,
        kind,
        source_mouth_order_index,
        source_band_index,
        source_boundary_index,
        owner,
        opposite_owner,
        points,
    )
}

fn push_constraint(
    constraints: &mut Vec<NodeRailConstraint>,
    kind: NodeRailConstraintKind,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
    source_boundary_index: Option<usize>,
    owner: Option<NodeBandOwner>,
    opposite_owner: Option<NodeBandOwner>,
    points: Vec<RoadVec2>,
) -> Result<(), NodeRailGenerationError> {
    let polyline = cleaned_open_rail(
        kind,
        source_mouth_order_index,
        source_band_index,
        source_boundary_index,
        points,
    )?;
    let constraint_index = constraints.len();
    constraints.push(NodeRailConstraint {
        constraint_index,
        kind,
        source_mouth_order_index,
        source_band_index,
        source_boundary_index,
        owner,
        opposite_owner,
        points_xz: polyline_to_road_points(&polyline),
    });
    Ok(())
}

fn height_for_key_on_generated_edge(
    point: NodeRailPointKey,
    start: NodeRailPointKey,
    end: NodeRailPointKey,
    start_height_m: f64,
    end_height_m: f64,
) -> Option<f64> {
    if start == end {
        return None;
    }
    let dx = end.0 - start.0;
    let dz = end.1 - start.1;
    let denominator = if dx.abs() >= dz.abs() { dx } else { dz };
    if denominator == 0 {
        return None;
    }
    let numerator = if dx.abs() >= dz.abs() {
        point.0 - start.0
    } else {
        point.1 - start.1
    };
    let t = numerator as f64 / denominator as f64;
    Some(start_height_m + (end_height_m - start_height_m) * t)
}

fn set_generated_contour_from_keys(
    contour: &mut NodeGeneratedContour,
    constraints: &mut [NodeRailConstraint],
    keys: Vec<NodeRailPointKey>,
) -> Result<(), NodeRailGenerationError> {
    let points = keys
        .into_iter()
        .map(road_point_from_key)
        .collect::<Vec<_>>();
    let polyline = cleaned_closed_contour(
        contour.kind,
        contour.source_mouth_order_index,
        contour.source_band_index,
        points,
    )?;
    contour.points_xz = polyline_to_road_points(&polyline);
    contour.backend_polyline = polyline;
    if let Some(height_points_world) = contour.height_points_world.as_deref() {
        contour.height_points_world =
            align_height_points_to_contour(&contour.points_xz, height_points_world);
    }
    update_generated_band_contour_constraint(contour, constraints);
    Ok(())
}

fn update_generated_band_contour_constraint(
    contour: &NodeGeneratedContour,
    constraints: &mut [NodeRailConstraint],
) {
    let Some(kind) = generated_contour_band_kind(contour) else {
        return;
    };
    for constraint in constraints {
        if matches!(
            constraint.kind,
            NodeRailConstraintKind::BandContour { kind: constraint_kind }
                if constraint_kind == kind
        ) && constraint.source_mouth_order_index == contour.source_mouth_order_index
            && constraint.source_band_index == contour.source_band_index
            && constraint.owner == contour.owner
        {
            constraint.points_xz = contour.points_xz.clone();
        }
    }
}

fn shared_generated_contour_edges(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
) -> Vec<GeneratedContourEdgeKey> {
    let mut left_edges = generated_contour_edges(left);
    let mut right_edges = generated_contour_edges(right);
    left_edges.sort_unstable();
    left_edges.dedup();
    right_edges.sort_unstable();
    right_edges.dedup();
    left_edges
        .into_iter()
        .filter(|edge| right_edges.binary_search(edge).is_ok())
        .collect()
}

fn shared_generated_contour_points(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
) -> Vec<NodeRailPointKey> {
    let mut left_points = generated_contour_keys(left);
    let mut right_points = generated_contour_keys(right);
    left_points.sort_unstable();
    left_points.dedup();
    right_points.sort_unstable();
    right_points.dedup();
    left_points
        .into_iter()
        .filter(|point| right_points.binary_search(point).is_ok())
        .collect()
}

fn generated_contour_edges(contour: &NodeGeneratedContour) -> Vec<GeneratedContourEdgeKey> {
    let keys = generated_contour_keys(contour);
    let mut edges = Vec::new();
    for index in 0..keys.len() {
        let start = keys[index];
        let end = keys[(index + 1) % keys.len()];
        if start != end {
            edges.push(GeneratedContourEdgeKey::new(start, end));
        }
    }
    edges
}

fn generated_contour_directed_edges(
    contour: &NodeGeneratedContour,
) -> Vec<GeneratedContourDirectedEdge> {
    let keys = generated_contour_keys(contour);
    let mut edges = Vec::new();
    for index in 0..keys.len() {
        let start = keys[index];
        let end = keys[(index + 1) % keys.len()];
        if start != end {
            edges.push(GeneratedContourDirectedEdge { start, end });
        }
    }
    edges
}

fn generated_contour_keys(contour: &NodeGeneratedContour) -> Vec<NodeRailPointKey> {
    contour
        .points_xz
        .iter()
        .copied()
        .map(road_point_key)
        .collect()
}

fn generated_same_band_boundary_role_at_contour_vertex(
    contour: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    key: NodeRailPointKey,
) -> Option<GeneratedSameBandBoundaryRole> {
    let Some(kind) = generated_contour_band_kind(contour) else {
        return None;
    };
    if !generated_contour_supports_same_band_role(kind) {
        return None;
    }
    let keys = generated_contour_keys(contour);
    if keys.len() < 2 {
        return None;
    }
    let mut roles = Vec::new();
    for index in 0..keys.len() {
        if keys[index] != key {
            continue;
        }
        let previous_key = keys[if index == 0 {
            keys.len() - 1
        } else {
            index - 1
        }];
        let next_key = keys[(index + 1) % keys.len()];
        collect_generated_same_band_role_on_segment(
            contour,
            constraints,
            previous_key,
            key,
            &mut roles,
        );
        collect_generated_same_band_role_on_segment(
            contour,
            constraints,
            key,
            next_key,
            &mut roles,
        );
    }
    roles.sort_unstable();
    roles.dedup();
    if roles.len() == 1 {
        return roles.first().copied();
    }
    generated_same_band_boundary_role_at_key(contour, constraints, key)
}

fn generated_contour_supports_same_band_role(kind: RoadSurfaceBandKind) -> bool {
    matches!(
        kind,
        RoadSurfaceBandKind::CurbOrShoulder | RoadSurfaceBandKind::Sidewalk
    )
}

fn collect_generated_same_band_role_on_segment(
    contour: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    start: NodeRailPointKey,
    end: NodeRailPointKey,
    roles: &mut Vec<GeneratedSameBandBoundaryRole>,
) {
    if start == end {
        return;
    }
    let Some(owner) = contour.owner else {
        return;
    };
    let Some(kind) = generated_contour_band_kind(contour) else {
        return;
    };
    for constraint in constraints
        .iter()
        .filter(|constraint| generated_constraint_applies_to_owner(constraint, owner))
        .filter(|constraint| generated_constraint_contains_key_segment(constraint, start, end))
    {
        if let Some(role) = generated_boundary_role_from_constraint(kind, owner, constraint) {
            roles.push(role);
        }
    }
}

fn generated_same_band_boundary_role_at_key(
    contour: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    key: NodeRailPointKey,
) -> Option<GeneratedSameBandBoundaryRole> {
    let Some(kind) = generated_contour_band_kind(contour) else {
        return None;
    };
    if !generated_contour_supports_same_band_role(kind) {
        return None;
    }
    let Some(owner) = contour.owner else {
        return None;
    };
    let mut has_lower_side = false;
    let mut has_raised_side = false;
    for constraint in constraints
        .iter()
        .filter(|constraint| generated_constraint_applies_to_owner(constraint, owner))
        .filter(|constraint| generated_constraint_touches_key(constraint, key))
    {
        match generated_boundary_role_from_constraint(kind, owner, constraint) {
            Some(GeneratedSameBandBoundaryRole::LowerSide) => has_lower_side = true,
            Some(GeneratedSameBandBoundaryRole::RaisedSide) => has_raised_side = true,
            None => {}
        }
    }
    if has_raised_side {
        Some(GeneratedSameBandBoundaryRole::RaisedSide)
    } else if has_lower_side {
        Some(GeneratedSameBandBoundaryRole::LowerSide)
    } else {
        None
    }
}

fn generated_boundary_role_from_constraint(
    contour_kind: RoadSurfaceBandKind,
    owner: NodeBandOwner,
    constraint: &NodeRailConstraint,
) -> Option<GeneratedSameBandBoundaryRole> {
    if contour_kind != owner.kind() {
        return None;
    }
    match constraint.kind {
        NodeRailConstraintKind::RaisedStepContact => {
            let opposite_owner = generated_constraint_opposite_owner(constraint, owner)?;
            generated_raised_step_boundary_role_for_owner(owner, opposite_owner)
        }
        NodeRailConstraintKind::FootprintSeam {
            adjacent_kind: RoadSurfaceBandKind::Sidewalk,
        } if contour_kind == RoadSurfaceBandKind::Sidewalk => {
            Some(GeneratedSameBandBoundaryRole::RaisedSide)
        }
        NodeRailConstraintKind::FullRoadbedContour
        | NodeRailConstraintKind::BandContour { .. }
        | NodeRailConstraintKind::SpanHandoff { .. }
        | NodeRailConstraintKind::AsphaltBoundary { .. }
        | NodeRailConstraintKind::FootprintSeam { .. }
        | NodeRailConstraintKind::BandBoundary { .. } => None,
    }
}

fn generated_constraint_opposite_owner(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
) -> Option<NodeBandOwner> {
    match (constraint.owner, constraint.opposite_owner) {
        (Some(left), Some(right)) if left == owner => Some(right),
        (Some(left), Some(right)) if right == owner => Some(left),
        _ => None,
    }
}

fn generated_constraint_applies_to_owner(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
) -> bool {
    if constraint.owner.is_some() || constraint.opposite_owner.is_some() {
        return constraint.owner == Some(owner) || constraint.opposite_owner == Some(owner);
    }
    match constraint.kind {
        NodeRailConstraintKind::FullRoadbedContour => true,
        NodeRailConstraintKind::BandContour { kind }
        | NodeRailConstraintKind::SpanHandoff { kind }
        | NodeRailConstraintKind::FootprintSeam {
            adjacent_kind: kind,
        } => kind == owner.kind(),
        NodeRailConstraintKind::AsphaltBoundary { adjacent_kind } => {
            is_carriageway(owner.kind()) || adjacent_kind == owner.kind()
        }
        NodeRailConstraintKind::RaisedStepContact => {
            constraint.owner == Some(owner) || constraint.opposite_owner == Some(owner)
        }
        NodeRailConstraintKind::BandBoundary {
            left_kind,
            right_kind,
        } => left_kind == owner.kind() || right_kind == owner.kind(),
    }
}

fn generated_constraint_contains_key_segment(
    constraint: &NodeRailConstraint,
    start: NodeRailPointKey,
    end: NodeRailPointKey,
) -> bool {
    let Some(first) = constraint.points_xz.first().copied() else {
        return false;
    };
    let mut previous_key = road_point_key(first);
    for point in constraint.points_xz.iter().copied().skip(1) {
        let next_key = road_point_key(point);
        if generated_point_key_lies_on_segment(start, previous_key, next_key)
            && generated_point_key_lies_on_segment(end, previous_key, next_key)
        {
            return true;
        }
        previous_key = next_key;
    }
    false
}

fn generated_constraint_touches_key(
    constraint: &NodeRailConstraint,
    key: NodeRailPointKey,
) -> bool {
    constraint.points_xz.windows(2).any(|segment| {
        generated_point_key_lies_on_segment(
            key,
            road_point_key(segment[0]),
            road_point_key(segment[1]),
        )
    })
}

fn generated_constraint_directed_edges(
    constraint: &NodeRailConstraint,
) -> Vec<GeneratedContourDirectedEdge> {
    constraint
        .points_xz
        .windows(2)
        .filter_map(|segment| {
            let start = road_point_key(segment[0]);
            let end = road_point_key(segment[1]);
            (start != end).then_some(GeneratedContourDirectedEdge { start, end })
        })
        .collect()
}

fn owners_match_unordered(
    owner: Option<NodeBandOwner>,
    opposite_owner: Option<NodeBandOwner>,
    left: NodeBandOwner,
    right: NodeBandOwner,
) -> bool {
    (owner == Some(left) && opposite_owner == Some(right))
        || (owner == Some(right) && opposite_owner == Some(left))
}

fn remove_generated_contour_spikes(keys: &mut Vec<NodeRailPointKey>) {
    keys.dedup();
    loop {
        if keys.len() < 3 {
            return;
        }
        let mut removed = false;
        for index in 0..keys.len() {
            let previous = if index == 0 {
                keys.len() - 1
            } else {
                index - 1
            };
            let next = if index + 1 == keys.len() {
                0
            } else {
                index + 1
            };
            if keys[previous] == keys[next] {
                keys.remove(index);
                removed = true;
                break;
            }
        }
        if !removed {
            return;
        }
    }
}

fn generated_triangle_double_area(
    a: NodeRailPointKey,
    b: NodeRailPointKey,
    c: NodeRailPointKey,
) -> i128 {
    SurfaceXzKey::raw_tuple_triangle_area2(a, b, c)
}

fn quantized_proper_segment_intersection(
    a: NodeRailPointKey,
    b: NodeRailPointKey,
    c: NodeRailPointKey,
    d: NodeRailPointKey,
) -> Option<NodeRailPointKey> {
    if a == b || c == d {
        return None;
    }
    let ab_c = generated_triangle_double_area(a, b, c);
    let ab_d = generated_triangle_double_area(a, b, d);
    let cd_a = generated_triangle_double_area(c, d, a);
    let cd_b = generated_triangle_double_area(c, d, b);
    if ab_c == 0 || ab_d == 0 || cd_a == 0 || cd_b == 0 {
        return None;
    }
    if (ab_c > 0) == (ab_d > 0) || (cd_a > 0) == (cd_b > 0) {
        return None;
    }

    let r_x = i128::from(b.0 - a.0);
    let r_z = i128::from(b.1 - a.1);
    let s_x = i128::from(d.0 - c.0);
    let s_z = i128::from(d.1 - c.1);
    let offset_x = i128::from(c.0 - a.0);
    let offset_z = i128::from(c.1 - a.1);
    let denominator = r_x * s_z - r_z * s_x;
    if denominator == 0 {
        return None;
    }
    let numerator = offset_x * s_z - offset_z * s_x;
    let x_num = i128::from(a.0) * denominator + r_x * numerator;
    let z_num = i128::from(a.1) * denominator + r_z * numerator;
    let intersection = (
        div_round_nearest_i128(x_num, denominator)?,
        div_round_nearest_i128(z_num, denominator)?,
    );
    if intersection == a || intersection == b || intersection == c || intersection == d {
        None
    } else {
        Some(intersection)
    }
}

fn div_round_nearest_i128(numerator: i128, denominator: i128) -> Option<i64> {
    if denominator == 0 {
        return None;
    }
    let (numerator, denominator) = if denominator < 0 {
        (-numerator, -denominator)
    } else {
        (numerator, denominator)
    };
    let rounded = if numerator >= 0 {
        (numerator + denominator / 2) / denominator
    } else {
        (numerator - denominator / 2) / denominator
    };
    i64::try_from(rounded).ok()
}

fn generated_contour_band_kind(contour: &NodeGeneratedContour) -> Option<RoadSurfaceBandKind> {
    match contour.kind {
        NodeGeneratedContourKind::Band { kind } => Some(kind),
        NodeGeneratedContourKind::FullRoadbed => None,
    }
}

fn road_point_key(point: RoadVec2) -> NodeRailPointKey {
    let key = SurfaceXzKey::from_road_xz(point);
    (key.x_key(), key.z_key())
}

fn road_point_from_key(point: NodeRailPointKey) -> RoadVec2 {
    SurfaceXzKey::from_raw_keys(point.0, point.1).to_road_xz()
}

fn cleaned_closed_contour(
    kind: NodeGeneratedContourKind,
    mouth_order_index: usize,
    band_index: Option<usize>,
    points: Vec<RoadVec2>,
) -> Result<RoadPolyline, NodeRailGenerationError> {
    let raw = road_points_to_polyline(points, true);
    let mut contour = RoadPolyline::create_from_remove_repeat(&raw, RAIL_CONTOUR_POINT_EQUAL_EPS_M);
    if contour.area() < 0.0 {
        contour.invert_direction_mut();
    }
    let area_m2 = contour.area().abs();
    if contour.vertex_count() < 3 || area_m2 <= f64::from(NODE_OVERLAY_MIN_AREA_M2) {
        return Err(NodeRailGenerationError::DegenerateContour {
            kind,
            mouth_order_index,
            band_index,
            area_m2,
            vertex_count: contour.vertex_count(),
        });
    }
    Ok(contour)
}

fn cleaned_open_rail(
    kind: NodeRailConstraintKind,
    mouth_order_index: usize,
    band_index: Option<usize>,
    boundary_index: Option<usize>,
    points: Vec<RoadVec2>,
) -> Result<RoadPolyline, NodeRailGenerationError> {
    let raw = road_points_to_polyline(points, false);
    let rail = RoadPolyline::create_from_remove_repeat(&raw, RAIL_CONTOUR_POINT_EQUAL_EPS_M);
    let path_length_m = rail.path_length();
    if rail.vertex_count() < 2 || path_length_m <= RAIL_CONTOUR_POINT_EQUAL_EPS_M {
        return Err(NodeRailGenerationError::DegenerateConstraint {
            kind,
            mouth_order_index,
            band_index,
            boundary_index,
            path_length_m,
            vertex_count: rail.vertex_count(),
        });
    }
    Ok(rail)
}

fn owners_by_mouth(
    input: &NodeArrangementInput,
    terminal_cap_bands_by_mouth: &[Vec<NodeTerminalCapBand>],
    side_join_bands_by_mouth: &[Vec<NodeInputSideJoinBand>],
) -> Vec<MouthOwners> {
    let mut next_owner_index = 0usize;
    input
        .mouths
        .iter()
        .enumerate()
        .map(|(mouth_index, mouth)| {
            let band_owners: Vec<NodeBandOwner> = mouth
                .band_intervals
                .iter()
                .map(|interval| {
                    let owner = NodeBandOwner::new(interval.band_kind, next_owner_index);
                    next_owner_index += 1;
                    owner
                })
                .collect();
            let mut terminal_owner_by_source =
                BTreeMap::<(RoadSurfaceBandKind, usize), NodeBandOwner>::new();
            for (interval, owner) in mouth.band_intervals.iter().zip(&band_owners) {
                terminal_owner_by_source.insert((interval.band_kind, interval.band_index), *owner);
            }
            let side_join_bands = side_join_bands_by_mouth
                .get(mouth_index)
                .map_or(&[] as &[NodeInputSideJoinBand], Vec::as_slice);
            let terminal_cap_bands = terminal_cap_bands_by_mouth
                .get(mouth_index)
                .map_or(&[] as &[NodeTerminalCapBand], Vec::as_slice);
            let terminal_cap_band_owners = terminal_cap_bands
                .iter()
                .map(|cap_band| {
                    let key = (cap_band.band_kind, cap_band.source_band_index);
                    if let Some(owner) = terminal_owner_by_source.get(&key).copied() {
                        owner
                    } else {
                        let owner = NodeBandOwner::new(cap_band.band_kind, next_owner_index);
                        next_owner_index += 1;
                        terminal_owner_by_source.insert(key, owner);
                        owner
                    }
                })
                .collect();
            let side_join_band_owners = side_join_bands
                .iter()
                .map(|side_join_band| {
                    let key = (side_join_band.band_kind, side_join_band.source_band_index);
                    if let Some(owner) = terminal_owner_by_source.get(&key).copied() {
                        owner
                    } else {
                        let owner = NodeBandOwner::new(side_join_band.band_kind, next_owner_index);
                        next_owner_index += 1;
                        terminal_owner_by_source.insert(key, owner);
                        owner
                    }
                })
                .collect();
            MouthOwners {
                band_owners,
                terminal_cap_band_owners,
                side_join_band_owners,
            }
        })
        .collect()
}

fn boundary_owners(
    boundary_index: usize,
    band_owners: &[NodeBandOwner],
) -> (Option<NodeBandOwner>, Option<NodeBandOwner>) {
    let left_owner = boundary_index
        .checked_sub(1)
        .and_then(|index| band_owners.get(index))
        .copied();
    let right_owner = band_owners.get(boundary_index).copied();
    match (left_owner, right_owner) {
        (Some(left_owner), Some(right_owner)) => (Some(left_owner), Some(right_owner)),
        (Some(owner), None) | (None, Some(owner)) => (Some(owner), None),
        (None, None) => (None, None),
    }
}

fn boundary_constraint_kind(role: NodeInputBoundaryRailRole) -> NodeRailConstraintKind {
    match role {
        NodeInputBoundaryRailRole::OuterFootprint { adjacent_kind } => {
            NodeRailConstraintKind::FootprintSeam { adjacent_kind }
        }
        NodeInputBoundaryRailRole::InteriorBandBoundary {
            left_kind,
            right_kind,
        } => {
            if raised_step_band_kinds_can_contact(left_kind, right_kind) {
                NodeRailConstraintKind::RaisedStepContact
            } else if is_carriageway(left_kind) || is_carriageway(right_kind) {
                let adjacent_kind = if is_carriageway(left_kind) {
                    right_kind
                } else {
                    left_kind
                };
                NodeRailConstraintKind::AsphaltBoundary { adjacent_kind }
            } else {
                NodeRailConstraintKind::BandBoundary {
                    left_kind,
                    right_kind,
                }
            }
        }
    }
}

fn is_carriageway(kind: RoadSurfaceBandKind) -> bool {
    kind == RoadSurfaceBandKind::Carriageway
}

#[cfg(test)]
mod tests;
