//! Library-backed rail and contour generation for canonical node arrangements.

use super::arrangement::NodeBandOwner;
use super::backend::{
    ROAD_OVERLAY_COORDINATE_SCALE, RoadPolyline, RoadVec2, RoadVec3, polyline_to_road_points,
    road_points_to_polyline,
};
use super::input::{
    NodeArrangementInput, NodeInputBandInterval, NodeInputBoundaryRailRole, NodeInputMouth,
    NodeInputProfileRail,
};
use super::joins::{
    NodeInputSideJoinBand, NodeInputSideJoinBandBoundaryMode, side_join_bands_by_mouth,
};
use super::terminal::{NodeTerminalCapBand, terminal_cap_bands_by_mouth};
use super::{
    NODE_OVERLAY_MIN_AREA_M2, NodeOverlayContour, NodeOverlayShapes, RoadSurfaceBandKind,
    RoadSurfaceSystem, RoadSurfaceVisualNodePieceKind,
};
use cavalier_contours::polyline::{PlineCreation, PlineSource, PlineSourceMut};
use i_overlay::core::overlay_rule::OverlayRule;
use std::collections::{BTreeMap, BTreeSet};

const RAIL_CONTOUR_POINT_EQUAL_EPS_M: f64 = 1.0e-6;

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
    NonCanonicalGeneratedContactEndpoint {
        kind: NodeRailConstraintKind,
        mouth_order_index: usize,
        band_index: Option<usize>,
        owner: Option<NodeBandOwner>,
        opposite_owner: Option<NodeBandOwner>,
        point_x_key: i64,
        point_z_key: i64,
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
    pub(super) fn build_node_rail_contours_from_input(
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

        let terminal_cap_bands_by_mouth = terminal_cap_bands_by_mouth(input);
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
        validate_generated_contact_constraint_endpoints_from_sources(
            &contours,
            &constraints,
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
    if mouth.uses_sampled_band_domain_paths
        && interval
            .start_path_world
            .len()
            .min(interval.end_path_world.len())
            > 2
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
    let height_points_world = height_points_world
        .as_deref()
        .and_then(|points_world| align_height_points_to_contour(&points_xz, points_world));
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
    let point_count = start_path_world.len().min(end_path_world.len());
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

fn subdivided_world_chord(start: RoadVec3, end: RoadVec3, point_count: usize) -> Vec<RoadVec3> {
    if point_count < 2 {
        return vec![start, end];
    }
    (0..point_count)
        .map(|index| {
            let t = index as f64 / (point_count - 1) as f64;
            start * (1.0 - t) + end * t
        })
        .collect()
}

fn push_terminal_cap_band_contours(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    mouth: &NodeInputMouth,
    cap_bands: &[NodeTerminalCapBand],
    mouth_owners: &MouthOwners,
    owners: &[NodeBandOwner],
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let owner_by_kind_and_source =
        terminal_owner_by_kind_and_source(mouth, mouth_owners, cap_bands, owners);
    if piece_kind != RoadSurfaceVisualNodePieceKind::Terminal {
        return Ok(());
    }

    let mut groups = BTreeMap::<GeneratedCapOrJoinGroupKey, TerminalCapBandGroup>::new();
    for (cap_band, owner) in cap_bands.iter().zip(owners) {
        groups
            .entry(GeneratedCapOrJoinGroupKey {
                kind: cap_band.band_kind,
                source_band_index: cap_band.source_band_index,
                owner: *owner,
                contributes_footprint: true,
            })
            .or_insert_with(|| TerminalCapBandGroup {
                contour_world: Vec::new(),
                source_contours_world: Vec::new(),
                cap_bands: Vec::new(),
            })
            .push(cap_band);
    }

    for (key, group) in groups {
        push_grouped_cap_or_join_candidate_contours(
            mouth,
            key,
            NodeGeneratedContourPurpose::TerminalCap,
            &group.contour_world,
            &group.source_contours_world,
            NodeGeneratedContourClaimPriority::JoinOrCap,
            contours,
            constraints,
        )?;
        for cap_band in group.cap_bands {
            push_terminal_cap_band_boundary_constraints(
                mouth,
                cap_band,
                key.owner,
                &owner_by_kind_and_source,
                constraints,
            )?;
        }
    }

    Ok(())
}

fn push_side_join_band_contours(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    mouth: &NodeInputMouth,
    side_join_bands: &[NodeInputSideJoinBand],
    mouth_owners: &MouthOwners,
    owners: &[NodeBandOwner],
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    if piece_kind == RoadSurfaceVisualNodePieceKind::Terminal {
        return Ok(());
    }

    let owner_by_kind_and_source =
        side_join_owner_by_kind_and_source(mouth, mouth_owners, side_join_bands, owners);
    let mut groups = BTreeMap::<GeneratedCapOrJoinGroupKey, SideJoinBandGroup>::new();
    for (side_join_band, owner) in side_join_bands.iter().zip(owners) {
        if !side_join_band_contributes_domain(side_join_band) {
            continue;
        }
        groups
            .entry(GeneratedCapOrJoinGroupKey {
                kind: side_join_band.band_kind,
                source_band_index: side_join_band.source_band_index,
                owner: *owner,
                contributes_footprint: side_join_band_contributes_footprint(
                    piece_kind,
                    side_join_band,
                ),
            })
            .or_insert_with(|| SideJoinBandGroup {
                contour_world: Vec::new(),
                source_contours_world: Vec::new(),
            })
            .push(side_join_band);
    }

    for (key, group) in groups {
        push_grouped_cap_or_join_candidate_contours(
            mouth,
            key,
            side_join_contour_purpose(piece_kind),
            &group.contour_world,
            &group.source_contours_world,
            NodeGeneratedContourClaimPriority::SideJoin,
            contours,
            constraints,
        )?;
    }
    for (side_join_band, owner) in side_join_bands.iter().zip(owners) {
        push_side_join_band_boundary_constraints(
            mouth,
            side_join_band,
            *owner,
            &owner_by_kind_and_source,
            constraints,
        )?;
    }

    Ok(())
}

fn terminal_owner_by_kind_and_source(
    mouth: &NodeInputMouth,
    mouth_owners: &MouthOwners,
    cap_bands: &[NodeTerminalCapBand],
    owners: &[NodeBandOwner],
) -> BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner> {
    let mut owner_by_kind_and_source = BTreeMap::new();
    for (interval, owner) in mouth.band_intervals.iter().zip(&mouth_owners.band_owners) {
        owner_by_kind_and_source.insert((interval.band_kind, interval.band_index), *owner);
    }
    for (cap_band, owner) in cap_bands.iter().zip(owners) {
        owner_by_kind_and_source.insert((cap_band.band_kind, cap_band.source_band_index), *owner);
    }
    owner_by_kind_and_source
}

fn side_join_owner_by_kind_and_source(
    mouth: &NodeInputMouth,
    mouth_owners: &MouthOwners,
    side_join_bands: &[NodeInputSideJoinBand],
    owners: &[NodeBandOwner],
) -> BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner> {
    let mut owner_by_kind_and_source = BTreeMap::new();
    for (interval, owner) in mouth.band_intervals.iter().zip(&mouth_owners.band_owners) {
        owner_by_kind_and_source.insert((interval.band_kind, interval.band_index), *owner);
    }
    for (side_join_band, owner) in side_join_bands.iter().zip(owners) {
        owner_by_kind_and_source.insert(
            (side_join_band.band_kind, side_join_band.source_band_index),
            *owner,
        );
    }
    owner_by_kind_and_source
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct GeneratedCapOrJoinGroupKey {
    kind: RoadSurfaceBandKind,
    source_band_index: usize,
    owner: NodeBandOwner,
    contributes_footprint: bool,
}

struct TerminalCapBandGroup<'a> {
    contour_world: Vec<NodeOverlayContour>,
    source_contours_world: Vec<&'a [RoadVec3]>,
    cap_bands: Vec<&'a NodeTerminalCapBand>,
}

impl<'a> TerminalCapBandGroup<'a> {
    fn push(&mut self, cap_band: &'a NodeTerminalCapBand) {
        let mut contour = cap_band
            .contour_world
            .iter()
            .map(|point| [point.x, point.z])
            .collect::<Vec<_>>();
        if RoadSurfaceSystem::overlay_contour_area(&contour) < 0.0 {
            contour.reverse();
        }
        self.contour_world.push(contour);
        self.source_contours_world
            .push(cap_band.contour_world.as_slice());
        self.cap_bands.push(cap_band);
    }
}

struct SideJoinBandGroup<'a> {
    contour_world: Vec<NodeOverlayContour>,
    source_contours_world: Vec<&'a [RoadVec3]>,
}

impl<'a> SideJoinBandGroup<'a> {
    fn push(&mut self, side_join_band: &'a NodeInputSideJoinBand) {
        let mut contour = side_join_band
            .contour_world
            .iter()
            .map(|point| [point.x, point.z])
            .collect::<Vec<_>>();
        if RoadSurfaceSystem::overlay_contour_area(&contour) < 0.0 {
            contour.reverse();
        }
        self.contour_world.push(contour);
        self.source_contours_world
            .push(side_join_band.contour_world.as_slice());
    }
}

fn push_grouped_cap_or_join_candidate_contours(
    mouth: &NodeInputMouth,
    key: GeneratedCapOrJoinGroupKey,
    purpose: NodeGeneratedContourPurpose,
    contour_world: &[NodeOverlayContour],
    source_contours_world: &[&[RoadVec3]],
    claim_priority: NodeGeneratedContourClaimPriority,
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let Some(mut shapes) = RoadSurfaceSystem::overlay_union_contours(contour_world) else {
        return Ok(());
    };
    RoadSurfaceSystem::sort_overlay_shapes(&mut shapes);

    for shape in shapes {
        for contour in shape {
            let points = contour
                .into_iter()
                .map(|point| RoadVec2::new(point[0], point[1]))
                .collect::<Vec<_>>();
            if key.contributes_footprint {
                let footprint = cleaned_closed_contour(
                    NodeGeneratedContourKind::FullRoadbed,
                    mouth.order_index,
                    None,
                    points.clone(),
                )?;
                let footprint_points_xz = polyline_to_road_points(&footprint);
                contours.push(NodeGeneratedContour {
                    kind: NodeGeneratedContourKind::FullRoadbed,
                    purpose,
                    source_mouth_order_index: mouth.order_index,
                    source_band_index: None,
                    owner: None,
                    claim_priority: NodeGeneratedContourClaimPriority::Footprint,
                    points_xz: footprint_points_xz.clone(),
                    height_points_world: None,
                    backend_polyline: footprint,
                });
                push_constraint(
                    constraints,
                    NodeRailConstraintKind::FullRoadbedContour,
                    mouth.order_index,
                    None,
                    None,
                    None,
                    None,
                    footprint_points_xz,
                )?;
            }

            let kind = NodeGeneratedContourKind::Band { kind: key.kind };
            let band_contour = cleaned_closed_contour(
                kind,
                mouth.order_index,
                Some(key.source_band_index),
                points,
            )?;
            let points_xz = polyline_to_road_points(&band_contour);
            let height_points_world =
                align_height_points_to_source_contours(&points_xz, source_contours_world);
            contours.push(NodeGeneratedContour {
                kind,
                purpose,
                source_mouth_order_index: mouth.order_index,
                source_band_index: Some(key.source_band_index),
                owner: Some(key.owner),
                claim_priority,
                points_xz: points_xz.clone(),
                height_points_world,
                backend_polyline: band_contour,
            });
            push_constraint(
                constraints,
                NodeRailConstraintKind::BandContour { kind: key.kind },
                mouth.order_index,
                Some(key.source_band_index),
                None,
                Some(key.owner),
                None,
                points_xz,
            )?;
        }
    }

    Ok(())
}

fn side_join_contour_purpose(
    piece_kind: RoadSurfaceVisualNodePieceKind,
) -> NodeGeneratedContourPurpose {
    match piece_kind {
        RoadSurfaceVisualNodePieceKind::Bend => NodeGeneratedContourPurpose::BendSideJoin,
        RoadSurfaceVisualNodePieceKind::JunctionN => NodeGeneratedContourPurpose::JunctionSideJoin,
        RoadSurfaceVisualNodePieceKind::Terminal => NodeGeneratedContourPurpose::TerminalCap,
    }
}

fn push_terminal_cap_band_boundary_constraints(
    mouth: &NodeInputMouth,
    cap_band: &NodeTerminalCapBand,
    owner: NodeBandOwner,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let inner_path = terminal_cap_band_inner_contour_path(cap_band);
    let outer_path = terminal_cap_band_outer_contour_path(cap_band);
    let opposite_owner =
        terminal_cap_band_material_opposite_owner(mouth, cap_band, owner_by_kind_and_source);
    match cap_band.band_kind {
        RoadSurfaceBandKind::CurbOrShoulder => {
            push_terminal_cap_inner_raised_step_contact_constraints(
                mouth,
                cap_band,
                owner,
                owner_by_kind_and_source,
                constraints,
            )?;
            if let Some(points) = outer_path.clone() {
                push_generated_band_path_constraint(
                    constraints,
                    NodeRailConstraintKind::RaisedStepContact,
                    mouth.order_index,
                    cap_band.source_band_index,
                    owner,
                    opposite_owner,
                    points,
                )?;
            }
            for (start, end) in terminal_cap_material_boundary_side_edges(cap_band) {
                push_generated_band_constraint(
                    constraints,
                    NodeRailConstraintKind::RaisedStepContact,
                    mouth.order_index,
                    cap_band.source_band_index,
                    owner,
                    opposite_owner,
                    xz(start),
                    xz(end),
                )?;
            }
            Ok(())
        }
        RoadSurfaceBandKind::Sidewalk => {
            if let Some(points) = inner_path {
                push_generated_band_path_constraint(
                    constraints,
                    NodeRailConstraintKind::RaisedStepContact,
                    mouth.order_index,
                    cap_band.source_band_index,
                    owner,
                    opposite_owner,
                    points,
                )?;
            }
            if let Some(points) = outer_path {
                push_generated_band_path_constraint(
                    constraints,
                    NodeRailConstraintKind::FootprintSeam {
                        adjacent_kind: RoadSurfaceBandKind::Sidewalk,
                    },
                    mouth.order_index,
                    cap_band.source_band_index,
                    owner,
                    None,
                    points,
                )?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn push_side_join_band_boundary_constraints(
    mouth: &NodeInputMouth,
    side_join_band: &NodeInputSideJoinBand,
    owner: NodeBandOwner,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let inner_path = side_join_band_inner_contour_path(side_join_band);
    let outer_path = side_join_band_outer_contour_path(side_join_band);
    let opposite_owner =
        side_join_band_material_opposite_owner(mouth, side_join_band, owner_by_kind_and_source);
    match side_join_band.band_kind {
        RoadSurfaceBandKind::CurbOrShoulder => {
            if side_join_band.boundary_mode != NodeInputSideJoinBandBoundaryMode::SameOwnerOuterCap
            {
                push_side_join_inner_raised_step_contact_constraints(
                    mouth,
                    side_join_band,
                    owner,
                    owner_by_kind_and_source,
                    constraints,
                )?;
            }
            if side_join_band_has_material_boundary(side_join_band)
                && let Some(points) = outer_path.clone()
            {
                push_generated_band_path_constraint(
                    constraints,
                    NodeRailConstraintKind::RaisedStepContact,
                    mouth.order_index,
                    side_join_band.source_band_index,
                    owner,
                    opposite_owner,
                    points,
                )?;
            }
            if side_join_band_has_material_boundary(side_join_band) {
                for (start, end) in side_join_material_boundary_side_edges(side_join_band) {
                    push_generated_band_constraint(
                        constraints,
                        NodeRailConstraintKind::RaisedStepContact,
                        mouth.order_index,
                        side_join_band.source_band_index,
                        owner,
                        opposite_owner,
                        xz(start),
                        xz(end),
                    )?;
                }
            }
            Ok(())
        }
        RoadSurfaceBandKind::Sidewalk => {
            if side_join_band.boundary_mode != NodeInputSideJoinBandBoundaryMode::SameOwnerOuterCap
                && let Some(points) = inner_path
            {
                push_generated_band_path_constraint(
                    constraints,
                    NodeRailConstraintKind::RaisedStepContact,
                    mouth.order_index,
                    side_join_band.source_band_index,
                    owner,
                    opposite_owner,
                    points,
                )?;
            }
            if side_join_band_has_material_boundary(side_join_band)
                && let Some(points) = outer_path
            {
                push_generated_band_path_constraint(
                    constraints,
                    NodeRailConstraintKind::FootprintSeam {
                        adjacent_kind: RoadSurfaceBandKind::Sidewalk,
                    },
                    mouth.order_index,
                    side_join_band.source_band_index,
                    owner,
                    None,
                    points,
                )?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn side_join_band_contributes_domain(side_join_band: &NodeInputSideJoinBand) -> bool {
    matches!(
        side_join_band.boundary_mode,
        NodeInputSideJoinBandBoundaryMode::MaterialBand
            | NodeInputSideJoinBandBoundaryMode::MaterialBandWithSameOwnerOuterCap
            | NodeInputSideJoinBandBoundaryMode::SameOwnerOuterCap
    )
}

fn side_join_band_contributes_footprint(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    side_join_band: &NodeInputSideJoinBand,
) -> bool {
    if piece_kind != RoadSurfaceVisualNodePieceKind::Bend {
        return false;
    }
    match side_join_band.boundary_mode {
        NodeInputSideJoinBandBoundaryMode::MaterialBand
        | NodeInputSideJoinBandBoundaryMode::MaterialBandWithSameOwnerOuterCap
        | NodeInputSideJoinBandBoundaryMode::SameOwnerOuterCap => true,
    }
}

fn terminal_cap_band_material_opposite_owner(
    mouth: &NodeInputMouth,
    cap_band: &NodeTerminalCapBand,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
) -> Option<NodeBandOwner> {
    if let Some(owner) =
        terminal_cap_generated_material_opposite_owner(cap_band, owner_by_kind_and_source)
    {
        return Some(owner);
    }
    match cap_band.band_kind {
        RoadSurfaceBandKind::CurbOrShoulder => adjacent_source_band_owner(
            mouth,
            cap_band.source_band_index,
            RoadSurfaceBandKind::Sidewalk,
            owner_by_kind_and_source,
        ),
        RoadSurfaceBandKind::Sidewalk => adjacent_source_band_owner(
            mouth,
            cap_band.source_band_index,
            RoadSurfaceBandKind::CurbOrShoulder,
            owner_by_kind_and_source,
        ),
        _ => None,
    }
}

fn terminal_cap_generated_material_opposite_owner(
    cap_band: &NodeTerminalCapBand,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
) -> Option<NodeBandOwner> {
    match cap_band.band_kind {
        RoadSurfaceBandKind::CurbOrShoulder => {
            cap_band
                .source_band_index
                .checked_add(1)
                .and_then(|source_band_index| {
                    owner_by_kind_and_source
                        .get(&(RoadSurfaceBandKind::Sidewalk, source_band_index))
                        .copied()
                })
        }
        RoadSurfaceBandKind::Sidewalk => {
            cap_band
                .source_band_index
                .checked_sub(1)
                .and_then(|source_band_index| {
                    owner_by_kind_and_source
                        .get(&(RoadSurfaceBandKind::CurbOrShoulder, source_band_index))
                        .copied()
                })
        }
        _ => None,
    }
}

fn side_join_band_material_opposite_owner(
    mouth: &NodeInputMouth,
    side_join_band: &NodeInputSideJoinBand,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
) -> Option<NodeBandOwner> {
    match side_join_band.band_kind {
        RoadSurfaceBandKind::CurbOrShoulder => adjacent_source_band_owner(
            mouth,
            side_join_band.source_band_index,
            RoadSurfaceBandKind::Sidewalk,
            owner_by_kind_and_source,
        ),
        RoadSurfaceBandKind::Sidewalk => adjacent_source_band_owner(
            mouth,
            side_join_band.source_band_index,
            RoadSurfaceBandKind::CurbOrShoulder,
            owner_by_kind_and_source,
        ),
        _ => None,
    }
}

fn push_terminal_cap_inner_raised_step_contact_constraints(
    mouth: &NodeInputMouth,
    cap_band: &NodeTerminalCapBand,
    owner: NodeBandOwner,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let Some(points) = terminal_cap_band_inner_contour_path(cap_band) else {
        return Ok(());
    };
    for segment in points.windows(2) {
        let opposite_owner = inner_raised_step_opposite_owner_for_segment(
            mouth,
            cap_band.source_band_index,
            segment[0],
            segment[1],
            owner_by_kind_and_source,
        );
        push_generated_band_constraint(
            constraints,
            NodeRailConstraintKind::RaisedStepContact,
            mouth.order_index,
            cap_band.source_band_index,
            owner,
            opposite_owner,
            segment[0],
            segment[1],
        )?;
    }
    Ok(())
}

fn push_side_join_inner_raised_step_contact_constraints(
    mouth: &NodeInputMouth,
    side_join_band: &NodeInputSideJoinBand,
    owner: NodeBandOwner,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let Some(points) = side_join_band_inner_contour_path(side_join_band) else {
        return Ok(());
    };
    for segment in points.windows(2) {
        let opposite_owner = inner_raised_step_opposite_owner_for_segment(
            mouth,
            side_join_band.source_band_index,
            segment[0],
            segment[1],
            owner_by_kind_and_source,
        );
        push_generated_band_constraint(
            constraints,
            NodeRailConstraintKind::RaisedStepContact,
            mouth.order_index,
            side_join_band.source_band_index,
            owner,
            opposite_owner,
            segment[0],
            segment[1],
        )?;
    }
    Ok(())
}

fn inner_raised_step_opposite_owner_for_segment(
    mouth: &NodeInputMouth,
    source_band_index: usize,
    start: RoadVec2,
    end: RoadVec2,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
) -> Option<NodeBandOwner> {
    if let Some(owner) =
        endpoint_raised_step_opposite_owner(mouth, start, end, owner_by_kind_and_source)
    {
        return Some(owner);
    }
    adjacent_source_band_owner(
        mouth,
        source_band_index,
        RoadSurfaceBandKind::Carriageway,
        owner_by_kind_and_source,
    )
}

fn endpoint_raised_step_opposite_owner(
    mouth: &NodeInputMouth,
    start: RoadVec2,
    end: RoadVec2,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
) -> Option<NodeBandOwner> {
    let start_boundary = endpoint_boundary_index_for_point(mouth, start)?;
    let end_boundary = endpoint_boundary_index_for_point(mouth, end)?;
    let (lower_boundary, upper_boundary) = if start_boundary <= end_boundary {
        (start_boundary, end_boundary)
    } else {
        (end_boundary, start_boundary)
    };
    if lower_boundary + 1 != upper_boundary {
        return None;
    }
    let interval = mouth.band_intervals.get(lower_boundary)?;
    if !is_carriageway(interval.band_kind) {
        return None;
    }
    owner_by_kind_and_source
        .get(&(RoadSurfaceBandKind::Carriageway, interval.band_index))
        .copied()
}

fn adjacent_source_band_owner(
    mouth: &NodeInputMouth,
    source_band_index: usize,
    adjacent_kind: RoadSurfaceBandKind,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
) -> Option<NodeBandOwner> {
    let source = mouth.band_intervals.get(source_band_index)?;
    let mut adjacent_owner = None;
    for adjacent_index in [
        source_band_index.checked_sub(1),
        source_band_index.checked_add(1),
    ]
    .into_iter()
    .flatten()
    {
        let Some(adjacent) = mouth.band_intervals.get(adjacent_index) else {
            continue;
        };
        if adjacent.band_kind != adjacent_kind {
            continue;
        }
        let owner = owner_by_kind_and_source
            .get(&(adjacent.band_kind, adjacent.band_index))
            .copied()?;
        if adjacent_owner.replace(owner).is_some() {
            return None;
        }
    }
    (source.band_kind != adjacent_kind)
        .then_some(adjacent_owner)
        .flatten()
}

fn endpoint_boundary_index_for_point(mouth: &NodeInputMouth, point: RoadVec2) -> Option<usize> {
    let key = road_point_key(point);
    mouth
        .boundary_rails
        .iter()
        .find(|rail| road_point_key(xz(rail.endpoint_world)) == key)
        .map(|rail| rail.boundary_index)
}

fn side_join_band_has_material_boundary(side_join_band: &NodeInputSideJoinBand) -> bool {
    matches!(
        side_join_band.boundary_mode,
        NodeInputSideJoinBandBoundaryMode::MaterialBand
    )
}

fn terminal_cap_material_boundary_side_edges(
    cap_band: &NodeTerminalCapBand,
) -> Vec<(RoadVec3, RoadVec3)> {
    let Some(inner_start_world) = cap_band.inner_path_world.first().copied() else {
        return Vec::new();
    };
    let Some(inner_end_world) = cap_band.inner_path_world.last().copied() else {
        return Vec::new();
    };
    let Some(outer_start_world) = cap_band.outer_path_world.first().copied() else {
        return Vec::new();
    };
    let Some(outer_end_world) = cap_band.outer_path_world.last().copied() else {
        return Vec::new();
    };
    [
        (inner_start_world, outer_start_world),
        (inner_end_world, outer_end_world),
    ]
    .into_iter()
    .filter(|(start, end)| road_point_key(xz(*start)) != road_point_key(xz(*end)))
    .collect()
}

fn side_join_material_boundary_side_edges(
    side_join_band: &NodeInputSideJoinBand,
) -> Vec<(RoadVec3, RoadVec3)> {
    let Some(inner_start_world) = side_join_band.inner_path_world.first().copied() else {
        return Vec::new();
    };
    let Some(inner_end_world) = side_join_band.inner_path_world.last().copied() else {
        return Vec::new();
    };
    let Some(outer_start_world) = side_join_band.outer_path_world.first().copied() else {
        return Vec::new();
    };
    let Some(outer_end_world) = side_join_band.outer_path_world.last().copied() else {
        return Vec::new();
    };
    [
        (inner_start_world, outer_start_world),
        (inner_end_world, outer_end_world),
    ]
    .into_iter()
    .filter(|(start, end)| road_point_key(xz(*start)) != road_point_key(xz(*end)))
    .collect()
}

fn terminal_cap_band_inner_contour_path(cap_band: &NodeTerminalCapBand) -> Option<Vec<RoadVec2>> {
    let points = cap_band
        .inner_path_world
        .iter()
        .copied()
        .map(xz)
        .collect::<Vec<_>>();
    clean_generated_constraint_path(points)
}

fn terminal_cap_band_outer_contour_path(cap_band: &NodeTerminalCapBand) -> Option<Vec<RoadVec2>> {
    let points = cap_band
        .outer_path_world
        .iter()
        .copied()
        .map(xz)
        .collect::<Vec<_>>();
    clean_generated_constraint_path(points)
}

fn side_join_band_inner_contour_path(
    side_join_band: &NodeInputSideJoinBand,
) -> Option<Vec<RoadVec2>> {
    let points = side_join_band
        .inner_path_world
        .iter()
        .copied()
        .map(xz)
        .collect::<Vec<_>>();
    clean_generated_constraint_path(points)
}

fn side_join_band_outer_contour_path(
    side_join_band: &NodeInputSideJoinBand,
) -> Option<Vec<RoadVec2>> {
    let points = side_join_band
        .outer_path_world
        .iter()
        .copied()
        .map(xz)
        .collect::<Vec<_>>();
    clean_generated_constraint_path(points)
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

fn insert_generated_material_point_constraint(
    constraints: &mut Vec<NodeRailConstraint>,
    kind: NodeRailConstraintKind,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    point: NodeRailPointKey,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
) {
    let (owner, opposite_owner) = if kind == NodeRailConstraintKind::RaisedStepContact {
        let Some(pair) = GeneratedRaisedStepOwnerPair::new(owner, opposite_owner) else {
            return;
        };
        (pair.owner, pair.opposite_owner)
    } else {
        (owner, opposite_owner)
    };
    let edge = [road_point_from_key(point), road_point_from_key(point)];
    if constraints.iter().any(|constraint| {
        constraint.kind == kind
            && owners_match_unordered(
                constraint.owner,
                constraint.opposite_owner,
                owner,
                opposite_owner,
            )
            && constraint.points_xz.len() == 2
            && road_point_key(constraint.points_xz[0]) == point
            && road_point_key(constraint.points_xz[1]) == point
    }) {
        return;
    }
    constraints.push(NodeRailConstraint {
        constraint_index: constraints.len(),
        kind,
        source_mouth_order_index,
        source_band_index,
        source_boundary_index: None,
        owner: Some(owner),
        opposite_owner: Some(opposite_owner),
        points_xz: edge.to_vec(),
    });
}

fn generated_role_edge_segments_inside_contour(
    role_edge: GeneratedContourDirectedEdge,
    target: &NodeGeneratedContour,
) -> Vec<GeneratedContourEdgeKey> {
    let mut keys = vec![role_edge.start, role_edge.end];
    for target_edge in generated_contour_directed_edges(target) {
        if let Some(point) = quantized_proper_segment_intersection(
            role_edge.start,
            role_edge.end,
            target_edge.start,
            target_edge.end,
        ) {
            keys.push(point);
        }
        for point in [target_edge.start, target_edge.end] {
            if generated_point_key_lies_on_segment(point, role_edge.start, role_edge.end) {
                keys.push(point);
            }
        }
        for point in [role_edge.start, role_edge.end] {
            if generated_point_key_lies_on_segment(point, target_edge.start, target_edge.end) {
                keys.push(point);
            }
        }
    }
    keys.sort_by_key(|point| {
        generated_segment_parameter_key(role_edge.start, role_edge.end, *point)
    });
    keys.dedup();

    let mut edges = BTreeSet::new();
    for segment in keys.windows(2) {
        let start = segment[0];
        let end = segment[1];
        if start == end {
            continue;
        }
        let point_x2 = i128::from(start.0) + i128::from(end.0);
        let point_z2 = i128::from(start.1) + i128::from(end.1);
        if doubled_point_inside_or_on_generated_contour(point_x2, point_z2, target) {
            edges.insert(GeneratedContourEdgeKey::new(start, end));
        }
    }
    edges.into_iter().collect()
}

fn append_source_authorized_raised_step_point_contacts(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    contours: &[NodeGeneratedContour],
    constraints: &mut Vec<NodeRailConstraint>,
) {
    let mut existing = constraints
        .iter()
        .filter_map(generated_same_band_contact_constraint_key)
        .collect::<BTreeSet<_>>();
    let mut contacts = BTreeSet::<GeneratedSameBandContactConstraint>::new();
    collect_source_authorized_raised_step_contacts(
        piece_kind,
        contours,
        constraints,
        &mut contacts,
    );

    for contact in contacts {
        if !existing.insert(contact.key()) {
            continue;
        }
        constraints.push(NodeRailConstraint {
            constraint_index: constraints.len(),
            kind: contact.kind,
            source_mouth_order_index: contact.source_mouth_order_index,
            source_band_index: contact.source_band_index,
            source_boundary_index: None,
            owner: Some(contact.owner),
            opposite_owner: Some(contact.opposite_owner),
            points_xz: vec![
                road_point_from_key(contact.start),
                road_point_from_key(contact.end),
            ],
        });
    }
}

fn collect_source_authorized_raised_step_contacts(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    contours: &[NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
    contacts: &mut BTreeSet<GeneratedSameBandContactConstraint>,
) {
    let source_authority = RaisedStepSourceAuthority::from_constraints(constraints);
    let target_groups = source_authorized_target_groups(contours);
    for source_constraint in source_authority.constraints() {
        for target_group in &target_groups {
            let target_contacts = source_authorized_raised_step_target_pairs(
                piece_kind,
                contours,
                source_constraint.source,
                target_group.key,
            );
            if target_contacts.is_empty() {
                continue;
            }
            for source_edge in generated_constraint_directed_edges(source_constraint.constraint) {
                let mut source_edges = generated_directed_edge_segments_inside_shape_edges(
                    source_edge,
                    &target_group.shape_edges,
                    &target_group.shapes,
                )
                .into_iter()
                .collect::<BTreeSet<_>>();
                source_edges.extend(generated_shape_boundary_segments_on_source_edge(
                    source_edge,
                    &target_group.shape_edges,
                ));
                for edge in source_edges {
                    for (owner, opposite_owner, include_edge) in &target_contacts {
                        for (start, end) in source_authorized_contact_segments(edge, *include_edge)
                        {
                            contacts.insert(GeneratedSameBandContactConstraint {
                                kind: NodeRailConstraintKind::RaisedStepContact,
                                owner: *owner,
                                opposite_owner: *opposite_owner,
                                start,
                                end,
                                source_mouth_order_index: source_constraint
                                    .source
                                    .source_mouth_order_index,
                                source_band_index: source_constraint.source.source_band_index,
                            });
                        }
                    }
                }
            }
        }
        collect_source_authorized_exact_group_overlap_contacts(
            source_constraint,
            contours,
            &target_groups,
            contacts,
        );
    }

    for (point, sources) in source_authority.sources_by_contact_point() {
        for left_index in 0..sources.len() {
            for right_index in left_index + 1..sources.len() {
                let source = sources[left_index].min(sources[right_index]);
                for left_owner in sources[left_index].owners {
                    for right_owner in sources[right_index].owners {
                        let Some(kind) =
                            generated_raised_step_contact_kind_for_owners(left_owner, right_owner)
                        else {
                            continue;
                        };
                        let Some(pair) = GeneratedRaisedStepOwnerPair::new(left_owner, right_owner)
                        else {
                            continue;
                        };
                        contacts.insert(GeneratedSameBandContactConstraint {
                            kind,
                            owner: pair.owner,
                            opposite_owner: pair.opposite_owner,
                            start: point,
                            end: point,
                            source_mouth_order_index: source.source_mouth_order_index,
                            source_band_index: source.source_band_index,
                        });
                    }
                }
            }
        }
    }
}

impl<'a> RaisedStepSourceAuthority<'a> {
    fn from_constraints(constraints: &'a [NodeRailConstraint]) -> Self {
        Self {
            constraints: constraints
                .iter()
                .filter_map(|constraint| {
                    generated_raised_step_endpoint_source(constraint)
                        .map(|source| RaisedStepSourceConstraint { source, constraint })
                })
                .collect(),
        }
    }

    fn constraints(&self) -> &[RaisedStepSourceConstraint<'a>] {
        &self.constraints
    }

    fn sources_by_contact_point(
        &self,
    ) -> BTreeMap<NodeRailPointKey, Vec<GeneratedRaisedStepEndpointSource>> {
        generated_raised_step_source_contact_points(&self.constraints)
            .into_iter()
            .filter_map(|point| {
                let mut sources = self
                    .constraints
                    .iter()
                    .filter(|source_constraint| {
                        generated_constraint_touches_key(source_constraint.constraint, point)
                    })
                    .map(|source_constraint| source_constraint.source)
                    .collect::<Vec<_>>();
                sources.sort_unstable();
                sources.dedup();
                (!sources.is_empty()).then_some((point, sources))
            })
            .collect()
    }
}

fn collect_source_authorized_exact_group_overlap_contacts(
    source_constraint: &RaisedStepSourceConstraint<'_>,
    contours: &[NodeGeneratedContour],
    target_groups: &[SourceAuthorizedTargetGroup],
    contacts: &mut BTreeSet<GeneratedSameBandContactConstraint>,
) {
    let [left_owner, right_owner] = source_constraint.source.owners;
    let left_groups = source_authorized_exact_target_groups(target_groups, left_owner);
    let right_groups = source_authorized_exact_target_groups(target_groups, right_owner);
    for left_group in &left_groups {
        for right_group in &right_groups {
            for edge in source_authorized_group_edges_inside_group(
                source_constraint.constraint,
                left_group,
                right_group,
                contours,
            )
            .into_iter()
            .chain(source_authorized_group_edges_inside_group(
                source_constraint.constraint,
                right_group,
                left_group,
                contours,
            ))
            .chain(source_authorized_source_edges_inside_group_intersection(
                source_constraint.constraint,
                left_group,
                right_group,
            )) {
                for (start, end) in source_authorized_contact_segments(edge, true) {
                    contacts.insert(GeneratedSameBandContactConstraint {
                        kind: NodeRailConstraintKind::RaisedStepContact,
                        owner: left_owner,
                        opposite_owner: right_owner,
                        start,
                        end,
                        source_mouth_order_index: source_constraint.source.source_mouth_order_index,
                        source_band_index: source_constraint.source.source_band_index,
                    });
                }
            }
        }
    }
}

fn source_authorized_exact_target_groups(
    target_groups: &[SourceAuthorizedTargetGroup],
    owner: NodeBandOwner,
) -> Vec<&SourceAuthorizedTargetGroup> {
    target_groups
        .iter()
        .filter(|group| group.key.owner == owner)
        .collect()
}

fn source_authorized_group_edges_inside_group(
    source_constraint: &NodeRailConstraint,
    edge_group: &SourceAuthorizedTargetGroup,
    containing_group: &SourceAuthorizedTargetGroup,
    contours: &[NodeGeneratedContour],
) -> Vec<GeneratedContourEdgeKey> {
    let mut edges = BTreeSet::new();
    for contour_index in &edge_group.contour_indices {
        let Some(contour) = contours.get(*contour_index) else {
            continue;
        };
        for contour_edge in generated_contour_directed_edges(contour) {
            let mut candidate_edges = generated_directed_edge_segments_inside_shape_edges(
                contour_edge,
                &containing_group.shape_edges,
                &containing_group.shapes,
            )
            .into_iter()
            .collect::<BTreeSet<_>>();
            candidate_edges.extend(generated_shape_boundary_segments_on_source_edge(
                contour_edge,
                &containing_group.shape_edges,
            ));
            for edge in candidate_edges {
                if generated_constraint_contains_key_segment(
                    source_constraint,
                    edge.start,
                    edge.end,
                ) {
                    edges.insert(edge);
                }
            }
        }
    }
    edges.into_iter().collect()
}

fn source_authorized_source_edges_inside_group_intersection(
    source_constraint: &NodeRailConstraint,
    left_group: &SourceAuthorizedTargetGroup,
    right_group: &SourceAuthorizedTargetGroup,
) -> Vec<GeneratedContourEdgeKey> {
    let Some(intersection_shapes) = RoadSurfaceSystem::overlay_binary_shapes(
        &left_group.shapes,
        &right_group.shapes,
        OverlayRule::Intersect,
    ) else {
        return Vec::new();
    };
    let intersection_edges = generated_overlay_shapes_directed_edges(&intersection_shapes);
    let mut edges = BTreeSet::new();
    for source_edge in generated_constraint_directed_edges(source_constraint) {
        edges.extend(generated_directed_edge_segments_inside_shape_edges(
            source_edge,
            &intersection_edges,
            &intersection_shapes,
        ));
        edges.extend(generated_shape_boundary_segments_on_source_edge(
            source_edge,
            &intersection_edges,
        ));
    }
    edges.into_iter().collect()
}

fn source_authorized_raised_step_target_pairs(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    contours: &[NodeGeneratedContour],
    source: GeneratedRaisedStepEndpointSource,
    target: SourceAuthorizedTargetGroupKey,
) -> Vec<(NodeBandOwner, NodeBandOwner, bool)> {
    let target_owner = target.owner;
    if source.owners.contains(&target_owner) {
        if Some(target.claim_priority)
            == source_authorized_target_claim_priority(contours, target_owner)
        {
            let Some(pair) = GeneratedRaisedStepOwnerPair::new(source.owners[0], source.owners[1])
            else {
                return Vec::new();
            };
            return vec![(pair.owner, pair.opposite_owner, true)];
        }
        return Vec::new();
    }

    if piece_kind != RoadSurfaceVisualNodePieceKind::Bend
        || target.claim_priority != NodeGeneratedContourClaimPriority::SideJoin
    {
        return Vec::new();
    }

    let mut pairs = Vec::new();
    for source_owner_index in 0..source.owners.len() {
        let source_owner = source.owners[source_owner_index];
        let replaced_owner = source.owners[1 - source_owner_index];
        if target_owner.kind() != replaced_owner.kind()
            || generated_raised_step_contact_kind_for_owners(source_owner, target_owner).is_none()
        {
            continue;
        }
        let Some(pair) = GeneratedRaisedStepOwnerPair::new(source_owner, target_owner) else {
            continue;
        };
        pairs.push((pair.owner, pair.opposite_owner, false));
    }
    pairs.sort_unstable();
    pairs.dedup();
    pairs
}

fn source_authorized_contact_segments(
    edge: GeneratedContourEdgeKey,
    include_edge: bool,
) -> Vec<(NodeRailPointKey, NodeRailPointKey)> {
    if include_edge {
        vec![(edge.start, edge.end)]
    } else {
        vec![(edge.start, edge.start), (edge.end, edge.end)]
    }
}

fn source_authorized_target_claim_priority(
    contours: &[NodeGeneratedContour],
    owner: NodeBandOwner,
) -> Option<NodeGeneratedContourClaimPriority> {
    if contours.iter().any(|contour| {
        contour.owner == Some(owner)
            && contour.claim_priority == NodeGeneratedContourClaimPriority::MouthBand
    }) {
        return Some(NodeGeneratedContourClaimPriority::MouthBand);
    }
    contours
        .iter()
        .filter(|contour| contour.owner == Some(owner))
        .map(|contour| contour.claim_priority)
        .min()
}

fn generated_raised_step_source_contact_points(
    source_constraints: &[RaisedStepSourceConstraint<'_>],
) -> BTreeSet<NodeRailPointKey> {
    let mut points = source_constraints
        .iter()
        .flat_map(|source_constraint| {
            generated_constraint_endpoint_keys(source_constraint.constraint)
        })
        .collect::<BTreeSet<_>>();
    for left_index in 0..source_constraints.len() {
        for right_index in left_index + 1..source_constraints.len() {
            let left = source_constraints[left_index].constraint;
            let right = source_constraints[right_index].constraint;
            for left_edge in generated_constraint_directed_edges(left) {
                for right_edge in generated_constraint_directed_edges(right) {
                    points.extend(generated_source_edge_contact_points(left_edge, right_edge));
                }
            }
        }
    }
    points
}

fn generated_source_edge_contact_points(
    left: GeneratedContourDirectedEdge,
    right: GeneratedContourDirectedEdge,
) -> Vec<NodeRailPointKey> {
    let mut points = Vec::new();
    if let Some(point) =
        quantized_proper_segment_intersection(left.start, left.end, right.start, right.end)
    {
        points.push(point);
    }
    for point in [left.start, left.end] {
        if generated_point_key_lies_on_segment(point, right.start, right.end) {
            points.push(point);
        }
    }
    for point in [right.start, right.end] {
        if generated_point_key_lies_on_segment(point, left.start, left.end) {
            points.push(point);
        }
    }
    points.sort_unstable();
    points.dedup();
    points
}

fn generated_raised_step_endpoint_source(
    constraint: &NodeRailConstraint,
) -> Option<GeneratedRaisedStepEndpointSource> {
    if constraint.kind != NodeRailConstraintKind::RaisedStepContact {
        return None;
    }
    let owner = constraint.owner?;
    let opposite_owner = constraint.opposite_owner?;
    let pair = GeneratedRaisedStepOwnerPair::new(owner, opposite_owner)?;
    Some(GeneratedRaisedStepEndpointSource {
        constraint_index: constraint.constraint_index,
        source_mouth_order_index: constraint.source_mouth_order_index,
        source_band_index: constraint.source_band_index,
        owners: [pair.owner, pair.opposite_owner],
    })
}

fn generated_constraint_endpoint_keys(constraint: &NodeRailConstraint) -> Vec<NodeRailPointKey> {
    let mut points = Vec::new();
    if let Some(point) = constraint.points_xz.first().copied() {
        points.push(road_point_key(point));
    }
    if let Some(point) = constraint.points_xz.last().copied() {
        points.push(road_point_key(point));
    }
    points.sort_unstable();
    points.dedup();
    points
}

fn append_generated_material_point_contact_constraints(
    contours: &[NodeGeneratedContour],
    constraints: &mut Vec<NodeRailConstraint>,
) {
    let mut contact_points = BTreeSet::<GeneratedSameBandContactConstraint>::new();
    for left_index in 0..contours.len() {
        for right_index in left_index + 1..contours.len() {
            let left = &contours[left_index];
            let right = &contours[right_index];
            let Some(left_owner) = left.owner else {
                continue;
            };
            let Some(right_owner) = right.owner else {
                continue;
            };
            if left_owner == right_owner {
                continue;
            }
            let Some(left_kind) = generated_contour_band_kind(left) else {
                continue;
            };
            let Some(right_kind) = generated_contour_band_kind(right) else {
                continue;
            };
            if left_kind == right_kind {
                continue;
            }
            let Some(contact_kind) =
                generated_raised_step_contact_kind_for_owners(left_owner, right_owner)
            else {
                continue;
            };
            let mut points = shared_generated_contour_points(left, right);
            points.extend(generated_contact_points_from_contour_intersections(
                left, right,
            ));
            points.extend(generated_material_authority_points_on_counterpart_contour(
                contact_kind,
                left,
                right,
                left_owner,
                right_owner,
                constraints,
            ));
            points.extend(generated_contour_keys(left).into_iter().filter(|point| {
                generated_material_point_contact_authority(
                    contact_kind,
                    left_owner,
                    right_owner,
                    *point,
                    constraints,
                )
                .is_some_and(|authority| {
                    authority.owner == Some(right_owner)
                        || authority.opposite_owner == Some(right_owner)
                        || owners_match_unordered(
                            authority.owner,
                            authority.opposite_owner,
                            left_owner,
                            right_owner,
                        )
                })
            }));
            points.extend(generated_contour_keys(right).into_iter().filter(|point| {
                generated_material_point_contact_authority(
                    contact_kind,
                    left_owner,
                    right_owner,
                    *point,
                    constraints,
                )
                .is_some_and(|authority| {
                    authority.owner == Some(left_owner)
                        || authority.opposite_owner == Some(left_owner)
                        || owners_match_unordered(
                            authority.owner,
                            authority.opposite_owner,
                            left_owner,
                            right_owner,
                        )
                })
            }));
            points.sort_unstable();
            points.dedup();
            for point in points {
                let Some(contact_source) = generated_material_point_contact_authority(
                    contact_kind,
                    left_owner,
                    right_owner,
                    point,
                    constraints,
                )
                .or_else(|| {
                    generated_material_point_contact_contour_authority(
                        left,
                        right,
                        point,
                        left_owner,
                        right_owner,
                    )
                })
                else {
                    continue;
                };
                let Some(pair) = GeneratedRaisedStepOwnerPair::new(left_owner, right_owner) else {
                    continue;
                };
                contact_points.insert(GeneratedSameBandContactConstraint {
                    kind: contact_kind,
                    owner: pair.owner,
                    opposite_owner: pair.opposite_owner,
                    start: point,
                    end: point,
                    source_mouth_order_index: contact_source.source_mouth_order_index,
                    source_band_index: contact_source.source_band_index,
                });
            }
        }
    }

    let mut existing = constraints
        .iter()
        .filter_map(generated_same_band_contact_constraint_key)
        .collect::<BTreeSet<_>>();
    for contact in contact_points {
        let key = contact.key();
        if !existing.insert(key) {
            continue;
        }
        insert_generated_material_point_constraint(
            constraints,
            contact.kind,
            contact.owner,
            contact.opposite_owner,
            contact.start,
            contact.source_mouth_order_index,
            contact.source_band_index,
        );
    }
}

fn generated_material_point_contact_contour_authority(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    point: NodeRailPointKey,
    left_owner: NodeBandOwner,
    right_owner: NodeBandOwner,
) -> Option<GeneratedMaterialPointContactAuthority> {
    if left_owner.kind() != RoadSurfaceBandKind::Footpath
        && right_owner.kind() != RoadSurfaceBandKind::Footpath
    {
        return None;
    }
    if !generated_contour_boundary_contains_key(left, point)
        || !generated_contour_boundary_contains_key(right, point)
    {
        return None;
    }
    let source = [left, right].into_iter().min_by_key(|contour| {
        (
            contour.claim_priority,
            contour.source_mouth_order_index,
            contour.source_band_index,
        )
    })?;
    Some(GeneratedMaterialPointContactAuthority {
        source_mouth_order_index: source.source_mouth_order_index,
        source_band_index: source.source_band_index,
        owner: source.owner,
        opposite_owner: None,
    })
}

fn generated_material_authority_points_on_counterpart_contour(
    kind: NodeRailConstraintKind,
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    left_owner: NodeBandOwner,
    right_owner: NodeBandOwner,
    constraints: &[NodeRailConstraint],
) -> Vec<NodeRailPointKey> {
    let mut points = Vec::new();
    for constraint in constraints
        .iter()
        .filter(|constraint| constraint.kind == kind)
        .filter(|constraint| {
            owners_match_unordered(
                constraint.owner,
                constraint.opposite_owner,
                left_owner,
                right_owner,
            )
        })
    {
        for point in constraint.points_xz.iter().copied().map(road_point_key) {
            if generated_contour_contains_key(right, point) {
                points.push(point);
            }
            if generated_contour_contains_key(left, point) {
                points.push(point);
            }
        }
        points.extend(generated_constraint_contour_contact_points(
            constraint, right,
        ));
        points.extend(generated_constraint_contour_contact_points(
            constraint, left,
        ));
    }
    points.sort_unstable();
    points.dedup();
    points
}

fn generated_constraint_contour_contact_points(
    constraint: &NodeRailConstraint,
    contour: &NodeGeneratedContour,
) -> Vec<NodeRailPointKey> {
    let mut points = Vec::new();
    for constraint_edge in generated_constraint_directed_edges(constraint) {
        for contour_edge in generated_contour_directed_edges(contour) {
            if let Some(point) = quantized_proper_segment_intersection(
                constraint_edge.start,
                constraint_edge.end,
                contour_edge.start,
                contour_edge.end,
            ) {
                points.push(point);
            }
            if generated_point_key_lies_on_segment(
                constraint_edge.start,
                contour_edge.start,
                contour_edge.end,
            ) {
                points.push(constraint_edge.start);
            }
            if generated_point_key_lies_on_segment(
                constraint_edge.end,
                contour_edge.start,
                contour_edge.end,
            ) {
                points.push(constraint_edge.end);
            }
            if generated_point_key_lies_on_segment(
                contour_edge.start,
                constraint_edge.start,
                constraint_edge.end,
            ) {
                points.push(contour_edge.start);
            }
            if generated_point_key_lies_on_segment(
                contour_edge.end,
                constraint_edge.start,
                constraint_edge.end,
            ) {
                points.push(contour_edge.end);
            }
        }
    }
    points.sort_unstable();
    points.dedup();
    points
}

fn generated_material_point_contact_authority(
    kind: NodeRailConstraintKind,
    left_owner: NodeBandOwner,
    right_owner: NodeBandOwner,
    point: NodeRailPointKey,
    constraints: &[NodeRailConstraint],
) -> Option<GeneratedMaterialPointContactAuthority> {
    constraints
        .iter()
        .filter(|constraint| constraint.kind == kind)
        .filter(|constraint| generated_constraint_touches_key(constraint, point))
        .filter(|constraint| {
            owners_match_unordered(
                constraint.owner,
                constraint.opposite_owner,
                left_owner,
                right_owner,
            )
        })
        .min_by_key(|constraint| constraint.constraint_index)
        .map(|constraint| GeneratedMaterialPointContactAuthority {
            source_mouth_order_index: constraint.source_mouth_order_index,
            source_band_index: constraint.source_band_index,
            owner: constraint.owner,
            opposite_owner: constraint.opposite_owner,
        })
}

fn generated_exact_owner_pair_contact_authority_for_edge(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    constraints: &[NodeRailConstraint],
    edge: GeneratedContourEdgeKey,
) -> Option<GeneratedMaterialPointContactAuthority> {
    constraints
        .iter()
        .filter(|constraint| constraint.kind == NodeRailConstraintKind::RaisedStepContact)
        .filter(|constraint| {
            owners_match_unordered(
                constraint.owner,
                constraint.opposite_owner,
                owner,
                opposite_owner,
            )
        })
        .filter(|constraint| {
            generated_constraint_contains_key_segment(constraint, edge.start, edge.end)
        })
        .min_by_key(|constraint| constraint.constraint_index)
        .map(|constraint| GeneratedMaterialPointContactAuthority {
            source_mouth_order_index: constraint.source_mouth_order_index,
            source_band_index: constraint.source_band_index,
            owner: constraint.owner,
            opposite_owner: constraint.opposite_owner,
        })
}

fn generated_exact_owner_pair_contact_authority_at_point(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    constraints: &[NodeRailConstraint],
    point: NodeRailPointKey,
) -> Option<GeneratedMaterialPointContactAuthority> {
    generated_material_point_contact_authority(
        NodeRailConstraintKind::RaisedStepContact,
        owner,
        opposite_owner,
        point,
        constraints,
    )
}

fn generated_contour_contains_key(contour: &NodeGeneratedContour, point: NodeRailPointKey) -> bool {
    doubled_point_inside_or_on_generated_contour(
        i128::from(point.0) * 2,
        i128::from(point.1) * 2,
        contour,
    )
}

fn generated_contour_boundary_contains_key(
    contour: &NodeGeneratedContour,
    point: NodeRailPointKey,
) -> bool {
    generated_contour_directed_edges(contour).into_iter().any(|edge| {
        generated_point_key_lies_on_segment(point, edge.start, edge.end)
    })
}

fn append_generated_same_band_contact_constraints(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    contours: &[NodeGeneratedContour],
    constraints: &mut Vec<NodeRailConstraint>,
) {
    let mut contact_edges = BTreeSet::<GeneratedSameBandContactConstraint>::new();
    for left_index in 0..contours.len() {
        for right_index in left_index + 1..contours.len() {
            let left = &contours[left_index];
            let right = &contours[right_index];
            let Some(left_owner) = left.owner else {
                continue;
            };
            let Some(right_owner) = right.owner else {
                continue;
            };
            if left_owner == right_owner {
                continue;
            }
            let Some(kind) = generated_contour_band_kind(left) else {
                continue;
            };
            let Some(right_kind) = generated_contour_band_kind(right) else {
                continue;
            };
            let Some(contact_kind) =
                generated_raised_step_contact_kind_for_owners(left_owner, right_owner)
            else {
                continue;
            };
            let Some(pair) = GeneratedRaisedStepOwnerPair::new(left_owner, right_owner) else {
                continue;
            };
            let shared_edges = shared_generated_contour_edges(left, right);
            let shared_edge_points = shared_edges
                .iter()
                .flat_map(|edge| [edge.start, edge.end])
                .collect::<BTreeSet<_>>();
            for edge in shared_edges {
                if let Some(source) = generated_contact_edge_source_authority(
                    pair.owner,
                    pair.opposite_owner,
                    constraints,
                    left,
                    right,
                    edge,
                ) {
                    insert_generated_contact_constraint(
                        &mut contact_edges,
                        contact_kind,
                        pair.owner,
                        pair.opposite_owner,
                        edge,
                        source,
                    );
                }
            }
            for edge in generated_contact_edges_inside_contour(left, right) {
                if let Some(source) = generated_contact_edge_source_authority(
                    pair.owner,
                    pair.opposite_owner,
                    constraints,
                    left,
                    right,
                    edge,
                ) {
                    insert_generated_contact_constraint(
                        &mut contact_edges,
                        contact_kind,
                        pair.owner,
                        pair.opposite_owner,
                        edge,
                        source,
                    );
                }
            }
            for edge in generated_contact_edges_inside_contour(right, left) {
                if let Some(source) = generated_contact_edge_source_authority(
                    pair.owner,
                    pair.opposite_owner,
                    constraints,
                    left,
                    right,
                    edge,
                ) {
                    insert_generated_contact_constraint(
                        &mut contact_edges,
                        contact_kind,
                        pair.owner,
                        pair.opposite_owner,
                        edge,
                        source,
                    );
                }
            }
            for edge in generated_contact_edges_from_overlay_intersection(left, right) {
                if let Some(source) = generated_contact_edge_source_authority(
                    pair.owner,
                    pair.opposite_owner,
                    constraints,
                    left,
                    right,
                    edge,
                ) {
                    insert_generated_contact_constraint(
                        &mut contact_edges,
                        contact_kind,
                        pair.owner,
                        pair.opposite_owner,
                        edge,
                        source,
                    );
                }
            }
            for point in shared_generated_contour_points(left, right) {
                if shared_edge_points.contains(&point) {
                    continue;
                }
                if !generated_contact_point_has_explicit_roles(
                    kind,
                    right_kind,
                    left,
                    right,
                    constraints,
                    point,
                    contact_kind,
                ) {
                    continue;
                }
                let Some(source) = generated_exact_owner_pair_contact_authority_at_point(
                    pair.owner,
                    pair.opposite_owner,
                    constraints,
                    point,
                ) else {
                    continue;
                };
                contact_edges.insert(GeneratedSameBandContactConstraint {
                    kind: contact_kind,
                    owner: pair.owner,
                    opposite_owner: pair.opposite_owner,
                    start: point,
                    end: point,
                    source_mouth_order_index: source.source_mouth_order_index,
                    source_band_index: source.source_band_index,
                });
            }
            for point in generated_contact_points_from_contour_intersections(left, right) {
                if shared_edge_points.contains(&point) {
                    continue;
                }
                if !generated_contact_point_has_explicit_roles(
                    kind,
                    right_kind,
                    left,
                    right,
                    constraints,
                    point,
                    contact_kind,
                ) {
                    continue;
                }
                let Some(source) = generated_exact_owner_pair_contact_authority_at_point(
                    pair.owner,
                    pair.opposite_owner,
                    constraints,
                    point,
                ) else {
                    continue;
                };
                contact_edges.insert(GeneratedSameBandContactConstraint {
                    kind: contact_kind,
                    owner: pair.owner,
                    opposite_owner: pair.opposite_owner,
                    start: point,
                    end: point,
                    source_mouth_order_index: source.source_mouth_order_index,
                    source_band_index: source.source_band_index,
                });
            }
        }
    }
    collect_source_authorized_raised_step_contacts(
        piece_kind,
        contours,
        constraints,
        &mut contact_edges,
    );

    let mut existing = constraints
        .iter()
        .filter_map(generated_same_band_contact_constraint_key)
        .collect::<BTreeSet<_>>();
    for contact in contact_edges {
        let key = contact.key();
        if !existing.insert(key) {
            continue;
        }
        constraints.push(NodeRailConstraint {
            constraint_index: constraints.len(),
            kind: contact.kind,
            source_mouth_order_index: contact.source_mouth_order_index,
            source_band_index: contact.source_band_index,
            source_boundary_index: None,
            owner: Some(contact.owner),
            opposite_owner: Some(contact.opposite_owner),
            points_xz: vec![
                road_point_from_key(contact.start),
                road_point_from_key(contact.end),
            ],
        });
    }
}

fn source_authorized_target_groups(
    contours: &[NodeGeneratedContour],
) -> Vec<SourceAuthorizedTargetGroup> {
    let mut contour_indices_by_key = BTreeMap::<SourceAuthorizedTargetGroupKey, Vec<usize>>::new();
    for (contour_index, contour) in contours.iter().enumerate() {
        let Some(owner) = contour.owner else {
            continue;
        };
        let Some(kind) = generated_contour_band_kind(contour) else {
            continue;
        };
        contour_indices_by_key
            .entry(SourceAuthorizedTargetGroupKey {
                owner,
                kind,
                claim_priority: contour.claim_priority,
            })
            .or_default()
            .push(contour_index);
    }

    contour_indices_by_key
        .into_iter()
        .filter_map(|(key, contour_indices)| {
            let overlay_contours = contour_indices
                .iter()
                .map(|index| generated_overlay_contour(&contours[*index]))
                .collect::<Vec<_>>();
            let shapes = RoadSurfaceSystem::overlay_union_contours(&overlay_contours)?;
            Some(SourceAuthorizedTargetGroup {
                key,
                contour_indices,
                shape_edges: generated_overlay_shapes_directed_edges(&shapes),
                shapes,
            })
        })
        .collect()
}

fn insert_generated_contact_constraint(
    contact_edges: &mut BTreeSet<GeneratedSameBandContactConstraint>,
    kind: NodeRailConstraintKind,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    edge: GeneratedContourEdgeKey,
    source: GeneratedMaterialPointContactAuthority,
) {
    let (owner, opposite_owner) = if kind == NodeRailConstraintKind::RaisedStepContact {
        let Some(pair) = GeneratedRaisedStepOwnerPair::new(owner, opposite_owner) else {
            return;
        };
        (pair.owner, pair.opposite_owner)
    } else {
        (owner, opposite_owner)
    };
    for (start, end) in [
        (edge.start, edge.end),
        (edge.start, edge.start),
        (edge.end, edge.end),
    ] {
        contact_edges.insert(GeneratedSameBandContactConstraint {
            kind,
            owner,
            opposite_owner,
            start,
            end,
            source_mouth_order_index: source.source_mouth_order_index,
            source_band_index: source.source_band_index,
        });
    }
}

fn generated_contact_edge_source_authority(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    constraints: &[NodeRailConstraint],
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    edge: GeneratedContourEdgeKey,
) -> Option<GeneratedMaterialPointContactAuthority> {
    generated_exact_owner_pair_contact_authority_for_edge(
        owner,
        opposite_owner,
        constraints,
        edge,
    )
    .or_else(|| generated_material_edge_contact_contour_authority(left, right, edge))
}

fn generated_material_edge_contact_contour_authority(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    edge: GeneratedContourEdgeKey,
) -> Option<GeneratedMaterialPointContactAuthority> {
    for point in [edge.start, edge.end] {
        if !generated_contour_boundary_contains_key(left, point)
            || !generated_contour_boundary_contains_key(right, point)
        {
            return None;
        }
    }
    let left_owner = left.owner?;
    let right_owner = right.owner?;
    generated_material_point_contact_contour_authority(
        left,
        right,
        edge.start,
        left_owner,
        right_owner,
    )
}

fn generated_contact_edges_inside_contour(
    edge_contour: &NodeGeneratedContour,
    containing_contour: &NodeGeneratedContour,
) -> Vec<GeneratedContourEdgeKey> {
    let mut edges = BTreeSet::new();
    for edge in generated_contour_directed_edges(edge_contour) {
        edges.extend(generated_role_edge_segments_inside_contour(
            edge,
            containing_contour,
        ));
    }
    edges.into_iter().collect()
}

fn generated_directed_edge_segments_inside_shape_edges(
    edge: GeneratedContourDirectedEdge,
    shape_edges: &[GeneratedContourDirectedEdge],
    containing_shapes: &NodeOverlayShapes,
) -> Vec<GeneratedContourEdgeKey> {
    let mut keys = vec![edge.start, edge.end];
    for shape_edge in shape_edges {
        if let Some(point) = quantized_proper_segment_intersection(
            edge.start,
            edge.end,
            shape_edge.start,
            shape_edge.end,
        ) {
            keys.push(point);
        }
        for point in [shape_edge.start, shape_edge.end] {
            if generated_point_key_lies_on_segment(point, edge.start, edge.end) {
                keys.push(point);
            }
        }
        for point in [edge.start, edge.end] {
            if generated_point_key_lies_on_segment(point, shape_edge.start, shape_edge.end) {
                keys.push(point);
            }
        }
    }
    keys.sort_by_key(|point| generated_segment_parameter_key(edge.start, edge.end, *point));
    keys.dedup();

    let mut edges = BTreeSet::new();
    for segment in keys.windows(2) {
        let start = segment[0];
        let end = segment[1];
        if start == end {
            continue;
        }
        let point_x2 = i128::from(start.0) + i128::from(end.0);
        let point_z2 = i128::from(start.1) + i128::from(end.1);
        if doubled_point_inside_or_on_overlay_shapes(point_x2, point_z2, containing_shapes) {
            edges.insert(GeneratedContourEdgeKey::new(start, end));
        }
    }
    edges.into_iter().collect()
}

fn generated_shape_boundary_segments_on_source_edge(
    source_edge: GeneratedContourDirectedEdge,
    shape_edges: &[GeneratedContourDirectedEdge],
) -> Vec<GeneratedContourEdgeKey> {
    let mut edges = BTreeSet::new();
    for shape_edge in shape_edges {
        let mut keys = Vec::new();
        for point in [shape_edge.start, shape_edge.end] {
            if generated_point_key_lies_on_segment(point, source_edge.start, source_edge.end) {
                keys.push(point);
            }
        }
        for point in [source_edge.start, source_edge.end] {
            if generated_point_key_lies_on_segment(point, shape_edge.start, shape_edge.end) {
                keys.push(point);
            }
        }
        keys.sort_by_key(|point| {
            generated_segment_parameter_key(source_edge.start, source_edge.end, *point)
        });
        keys.dedup();
        for segment in keys.windows(2) {
            let start = segment[0];
            let end = segment[1];
            if start != end {
                edges.insert(GeneratedContourEdgeKey::new(start, end));
            }
        }
    }
    edges.into_iter().collect()
}

fn generated_contact_edges_from_overlay_intersection(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
) -> Vec<GeneratedContourEdgeKey> {
    let Some(left_shapes) = generated_contour_overlay_shapes(left) else {
        return Vec::new();
    };
    let Some(right_shapes) = generated_contour_overlay_shapes(right) else {
        return Vec::new();
    };
    let Some(intersection) = RoadSurfaceSystem::overlay_binary_shapes(
        &left_shapes,
        &right_shapes,
        OverlayRule::Intersect,
    ) else {
        return Vec::new();
    };
    let mut edges = intersection
        .into_iter()
        .flat_map(|shape| shape.into_iter())
        .flat_map(|contour| {
            let keys = contour
                .into_iter()
                .map(|point| {
                    (
                        (point[0] * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
                        (point[1] * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
                    )
                })
                .collect::<Vec<_>>();
            let mut edges = Vec::new();
            for index in 0..keys.len() {
                let start = keys[index];
                let end = keys[(index + 1) % keys.len()];
                if start != end {
                    edges.push(GeneratedContourEdgeKey::new(start, end));
                }
            }
            edges
        })
        .collect::<Vec<_>>();
    edges.sort_unstable();
    edges.dedup();
    edges
}

fn generated_contact_points_from_contour_intersections(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
) -> Vec<NodeRailPointKey> {
    let mut points = Vec::new();
    for left_edge in generated_contour_directed_edges(left) {
        for right_edge in generated_contour_directed_edges(right) {
            if let Some(point) = quantized_proper_segment_intersection(
                left_edge.start,
                left_edge.end,
                right_edge.start,
                right_edge.end,
            ) {
                points.push(point);
            }
            if generated_point_key_lies_on_segment(
                left_edge.start,
                right_edge.start,
                right_edge.end,
            ) {
                points.push(left_edge.start);
            }
            if generated_point_key_lies_on_segment(left_edge.end, right_edge.start, right_edge.end)
            {
                points.push(left_edge.end);
            }
            if generated_point_key_lies_on_segment(right_edge.start, left_edge.start, left_edge.end)
            {
                points.push(right_edge.start);
            }
            if generated_point_key_lies_on_segment(right_edge.end, left_edge.start, left_edge.end) {
                points.push(right_edge.end);
            }
        }
    }
    points.sort_unstable();
    points.dedup();
    points
}

fn generated_contour_overlay_shapes(contour: &NodeGeneratedContour) -> Option<NodeOverlayShapes> {
    RoadSurfaceSystem::overlay_union_contours(&[generated_overlay_contour(contour)])
}

fn generated_overlay_contour(contour: &NodeGeneratedContour) -> NodeOverlayContour {
    contour
        .points_xz
        .iter()
        .map(|point| [point.x, point.y])
        .collect()
}

fn generated_overlay_shapes_directed_edges(
    shapes: &NodeOverlayShapes,
) -> Vec<GeneratedContourDirectedEdge> {
    let mut edges = Vec::new();
    for contour in shapes.iter().flat_map(|shape| shape.iter()) {
        let keys = generated_overlay_contour_keys(contour);
        for index in 0..keys.len() {
            let start = keys[index];
            let end = keys[(index + 1) % keys.len()];
            if start != end {
                edges.push(GeneratedContourDirectedEdge { start, end });
            }
        }
    }
    edges
}

fn generated_overlay_contour_keys(contour: &NodeOverlayContour) -> Vec<NodeRailPointKey> {
    contour
        .iter()
        .copied()
        .map(generated_overlay_point_key)
        .collect()
}

fn generated_overlay_point_key(point: [f64; 2]) -> NodeRailPointKey {
    (
        (point[0] * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
        (point[1] * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
    )
}

fn doubled_point_inside_or_on_generated_contour(
    point_x2: i128,
    point_z2: i128,
    contour: &NodeGeneratedContour,
) -> bool {
    let keys = generated_contour_keys(contour);
    doubled_point_inside_or_on_generated_keys(point_x2, point_z2, &keys)
}

fn doubled_point_inside_or_on_generated_keys(
    point_x2: i128,
    point_z2: i128,
    keys: &[NodeRailPointKey],
) -> bool {
    doubled_point_location_in_generated_keys(point_x2, point_z2, keys)
        != GeneratedPointContourLocation::Outside
}

fn doubled_point_inside_or_on_overlay_shapes(
    point_x2: i128,
    point_z2: i128,
    shapes: &NodeOverlayShapes,
) -> bool {
    shapes.iter().any(|shape| {
        let Some(outer) = shape.first() else {
            return false;
        };
        let outer_keys = generated_overlay_contour_keys(outer);
        match doubled_point_location_in_generated_keys(point_x2, point_z2, &outer_keys) {
            GeneratedPointContourLocation::Outside => false,
            GeneratedPointContourLocation::Boundary => true,
            GeneratedPointContourLocation::Inside => shape.iter().skip(1).all(|hole| {
                let hole_keys = generated_overlay_contour_keys(hole);
                doubled_point_location_in_generated_keys(point_x2, point_z2, &hole_keys)
                    != GeneratedPointContourLocation::Inside
            }),
        }
    })
}

fn doubled_point_location_in_generated_keys(
    point_x2: i128,
    point_z2: i128,
    keys: &[NodeRailPointKey],
) -> GeneratedPointContourLocation {
    if keys.len() < 3 {
        return GeneratedPointContourLocation::Outside;
    }
    let mut inside = false;
    for index in 0..keys.len() {
        let start = keys[index];
        let end = keys[(index + 1) % keys.len()];
        if doubled_point_lies_on_generated_segment(point_x2, point_z2, start, end) {
            return GeneratedPointContourLocation::Boundary;
        }
        let start_z2 = i128::from(start.1) * 2;
        let end_z2 = i128::from(end.1) * 2;
        if (start_z2 > point_z2) == (end_z2 > point_z2) {
            continue;
        }
        let start_x2 = i128::from(start.0) * 2;
        let end_x2 = i128::from(end.0) * 2;
        let denominator = end_z2 - start_z2;
        let lhs = (point_x2 - start_x2) * denominator;
        let rhs = (point_z2 - start_z2) * (end_x2 - start_x2);
        let crosses = if denominator > 0 {
            lhs < rhs
        } else {
            lhs > rhs
        };
        if crosses {
            inside = !inside;
        }
    }
    if inside {
        GeneratedPointContourLocation::Inside
    } else {
        GeneratedPointContourLocation::Outside
    }
}

fn doubled_point_lies_on_generated_segment(
    point_x2: i128,
    point_z2: i128,
    start: NodeRailPointKey,
    end: NodeRailPointKey,
) -> bool {
    let start_x2 = i128::from(start.0) * 2;
    let start_z2 = i128::from(start.1) * 2;
    let end_x2 = i128::from(end.0) * 2;
    let end_z2 = i128::from(end.1) * 2;
    let dx = end_x2 - start_x2;
    let dz = end_z2 - start_z2;
    let px = point_x2 - start_x2;
    let pz = point_z2 - start_z2;
    if px * dz - pz * dx != 0 {
        return false;
    }
    point_x2 >= start_x2.min(end_x2)
        && point_x2 <= start_x2.max(end_x2)
        && point_z2 >= start_z2.min(end_z2)
        && point_z2 <= start_z2.max(end_z2)
}

fn node_generated_contact_contours(
    contours: &mut [NodeGeneratedContour],
    constraints: &mut [NodeRailConstraint],
) -> Result<(), NodeRailGenerationError> {
    let max_passes = contours.len().saturating_mul(contours.len()).max(1) * 4;
    let mut previous_candidates = None;
    for _ in 0..max_passes {
        let candidates = generated_contact_contour_noding_candidates(contours, constraints);
        if candidates.is_empty() {
            return Ok(());
        };
        if previous_candidates.as_ref() == Some(&candidates) {
            return Ok(());
        }
        if !insert_contact_noding_candidates(contours, constraints, candidates.clone())? {
            return Ok(());
        }
        previous_candidates = Some(candidates);
    }
    Ok(())
}

fn generated_contact_contour_noding_candidates(
    contours: &[NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
) -> Vec<(usize, GeneratedContourDirectedEdge, NodeRailPointKey)> {
    let mut candidates = Vec::new();
    for left_index in 0..contours.len() {
        for right_index in left_index + 1..contours.len() {
            let left = &contours[left_index];
            let right = &contours[right_index];
            if !generated_contours_support_contact_noding(left, right) {
                continue;
            }
            candidates.extend(
                generated_contact_point_on_edge_noding_candidates(left, right, constraints)
                    .into_iter()
                    .map(|(edge, insert_key)| (left_index, edge, insert_key)),
            );
            candidates.extend(
                generated_contact_point_on_edge_noding_candidates(right, left, constraints)
                    .into_iter()
                    .map(|(edge, insert_key)| (right_index, edge, insert_key)),
            );
            candidates.extend(
                generated_contact_edge_intersection_noding_candidates(left, right, constraints)
                    .into_iter()
                    .flat_map(|(left_edge, right_edge, insert_key)| {
                        [
                            (left_index, left_edge, insert_key),
                            (right_index, right_edge, insert_key),
                        ]
                    }),
            );
        }
    }
    candidates.sort_unstable();
    candidates.dedup();
    candidates
}

fn insert_contact_noding_candidates(
    contours: &mut [NodeGeneratedContour],
    constraints: &mut [NodeRailConstraint],
    candidates: Vec<(usize, GeneratedContourDirectedEdge, NodeRailPointKey)>,
) -> Result<bool, NodeRailGenerationError> {
    let mut insertions_by_contour =
        BTreeMap::<usize, BTreeMap<GeneratedContourDirectedEdge, BTreeSet<NodeRailPointKey>>>::new(
        );
    for (contour_index, edge, insert_key) in candidates {
        insertions_by_contour
            .entry(contour_index)
            .or_default()
            .entry(edge)
            .or_default()
            .insert(insert_key);
    }

    let mut inserted_any = false;
    for (contour_index, insertions_by_edge) in insertions_by_contour {
        inserted_any |= insert_keys_on_generated_contour_edges(
            contours,
            constraints,
            contour_index,
            insertions_by_edge,
        )?;
    }
    Ok(inserted_any)
}

fn insert_keys_on_generated_contour_edges(
    contours: &mut [NodeGeneratedContour],
    constraints: &mut [NodeRailConstraint],
    contour_index: usize,
    insertions_by_edge: BTreeMap<GeneratedContourDirectedEdge, BTreeSet<NodeRailPointKey>>,
) -> Result<bool, NodeRailGenerationError> {
    let Some(contour) = contours.get_mut(contour_index) else {
        return Ok(false);
    };
    let keys = generated_contour_keys(contour);
    if keys.len() < 2 {
        return Ok(false);
    }

    let height_points = contour.height_points_world.clone();
    let mut new_keys = Vec::with_capacity(keys.len());
    let mut new_height_points = height_points
        .as_ref()
        .filter(|points| points.len() == keys.len())
        .map(|_| Vec::with_capacity(keys.len()));
    let mut inserted_any = false;

    for index in 0..keys.len() {
        let next = (index + 1) % keys.len();
        let start = keys[index];
        let end = keys[next];
        new_keys.push(start);
        if let (Some(height_points), Some(new_height_points)) =
            (height_points.as_ref(), new_height_points.as_mut())
        {
            new_height_points.push(height_points[index]);
        }

        let edge = GeneratedContourDirectedEdge { start, end };
        let Some(insertions) = insertions_by_edge.get(&edge) else {
            continue;
        };
        let mut insertions = insertions
            .iter()
            .copied()
            .filter(|point| *point != start && *point != end)
            .filter(|point| generated_point_key_lies_on_segment(*point, start, end))
            .collect::<Vec<_>>();
        insertions.sort_by_key(|point| generated_segment_parameter_key(start, end, *point));
        insertions.dedup();
        for insert_key in insertions {
            inserted_any = true;
            new_keys.push(insert_key);
            if let (Some(height_points), Some(new_height_points)) =
                (height_points.as_ref(), new_height_points.as_mut())
            {
                let Some(height_m) = height_for_key_on_generated_edge(
                    insert_key,
                    start,
                    end,
                    height_points[index].y,
                    height_points[next].y,
                ) else {
                    contour.height_points_world = None;
                    continue;
                };
                let point = road_point_from_key(insert_key);
                new_height_points.push(RoadVec3::new(point.x, height_m, point.y));
            }
        }
    }

    if !inserted_any {
        return Ok(false);
    }
    remove_generated_contour_spikes(&mut new_keys);
    if new_keys == keys {
        return Ok(false);
    }
    contour.height_points_world = new_height_points;
    set_generated_contour_from_keys(contour, constraints, new_keys)?;
    Ok(generated_contour_keys(contour) != keys)
}

fn validate_generated_contact_constraint_endpoints_from_sources(
    contours: &[NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
    generated_constraint_start_index: usize,
) -> Result<(), NodeRailGenerationError> {
    let source_authority = ExactGeneratedSourceAuthority::from_sources(
        contours,
        constraints,
        generated_constraint_start_index,
    );
    for constraint in constraints.iter().skip(generated_constraint_start_index) {
        if generated_contact_kind_from_constraint(constraint.kind).is_none() {
            continue;
        }
        let Some(source_band_index) = constraint.source_band_index else {
            continue;
        };
        if constraint.owner.is_none() || constraint.opposite_owner.is_none() {
            continue;
        }
        let owners = [constraint.owner, constraint.opposite_owner];
        if !source_authority.has_any_source(
            owners,
            constraint.source_mouth_order_index,
            source_band_index,
        ) {
            continue;
        }
        for point in &constraint.points_xz {
            let key = road_point_key(*point);
            if source_authority.has_exact_point(
                owners,
                constraint.source_mouth_order_index,
                source_band_index,
                key,
            ) || source_authority.has_exact_source_segment_point(
                constraint.kind,
                owners,
                constraint.source_mouth_order_index,
                source_band_index,
                key,
            ) || source_authority.has_exact_unbanded_source_segment_point(
                constraint.kind,
                owners,
                constraint.source_mouth_order_index,
                source_band_index,
                key,
            ) || source_authority.has_exact_same_kind_source_segment_handoff_point(
                constraint.kind,
                owners,
                constraint.source_mouth_order_index,
                source_band_index,
                key,
            ) || source_authority.has_exact_cross_source_same_kind_contact_point(
                constraint.kind,
                owners,
                constraint.source_mouth_order_index,
                source_band_index,
                key,
            ) {
                continue;
            }
            return Err(
                NodeRailGenerationError::NonCanonicalGeneratedContactEndpoint {
                    kind: constraint.kind,
                    mouth_order_index: constraint.source_mouth_order_index,
                    band_index: constraint.source_band_index,
                    owner: constraint.owner,
                    opposite_owner: constraint.opposite_owner,
                    point_x_key: key.0,
                    point_z_key: key.1,
                },
            );
        }
    }
    Ok(())
}

struct ExactGeneratedSourceAuthority {
    keys_by_owner: BTreeMap<NodeBandOwner, BTreeSet<NodeRailPointKey>>,
    segments_by_owner: BTreeMap<NodeBandOwner, BTreeSet<GeneratedContourEdgeKey>>,
    keys_by_source: BTreeMap<(NodeBandOwner, usize, usize), BTreeSet<NodeRailPointKey>>,
    segments_by_source: BTreeMap<(NodeBandOwner, usize, usize), BTreeSet<GeneratedContourEdgeKey>>,
    segments_by_contact_source: BTreeMap<
        (
            NodeRailConstraintKind,
            NodeBandOwner,
            NodeBandOwner,
            usize,
            Option<usize>,
        ),
        BTreeSet<GeneratedContourEdgeKey>,
    >,
}

impl ExactGeneratedSourceAuthority {
    fn from_sources(
        contours: &[NodeGeneratedContour],
        constraints: &[NodeRailConstraint],
        generated_constraint_start_index: usize,
    ) -> Self {
        let mut keys_by_owner = BTreeMap::<NodeBandOwner, BTreeSet<NodeRailPointKey>>::new();
        let mut segments_by_owner =
            BTreeMap::<NodeBandOwner, BTreeSet<GeneratedContourEdgeKey>>::new();
        let mut keys_by_source =
            BTreeMap::<(NodeBandOwner, usize, usize), BTreeSet<NodeRailPointKey>>::new();
        let mut segments_by_source =
            BTreeMap::<(NodeBandOwner, usize, usize), BTreeSet<GeneratedContourEdgeKey>>::new();
        let mut segments_by_contact_source = BTreeMap::<
            (
                NodeRailConstraintKind,
                NodeBandOwner,
                NodeBandOwner,
                usize,
                Option<usize>,
            ),
            BTreeSet<GeneratedContourEdgeKey>,
        >::new();
        for contour in contours {
            let Some(owner) = contour.owner else {
                continue;
            };
            let keys = generated_contour_keys(contour);
            keys_by_owner
                .entry(owner)
                .or_default()
                .extend(keys.iter().copied());
            segments_by_owner.entry(owner).or_default().extend(
                generated_contour_directed_edges(contour)
                    .into_iter()
                    .map(|edge| GeneratedContourEdgeKey::new(edge.start, edge.end)),
            );
            let Some(source_band_index) = contour.source_band_index else {
                continue;
            };
            keys_by_source
                .entry((owner, contour.source_mouth_order_index, source_band_index))
                .or_default()
                .extend(keys.into_iter());
            segments_by_source
                .entry((owner, contour.source_mouth_order_index, source_band_index))
                .or_default()
                .extend(
                    generated_contour_directed_edges(contour)
                        .into_iter()
                        .map(|edge| GeneratedContourEdgeKey::new(edge.start, edge.end)),
                );
        }
        for constraint in constraints.iter().take(generated_constraint_start_index) {
            if generated_contact_kind_from_constraint(constraint.kind).is_none() {
                continue;
            }
            if let Some(source_band_index) = constraint.source_band_index {
                let owners = [constraint.owner, constraint.opposite_owner];
                for owner in owners.into_iter().flatten() {
                    keys_by_source
                        .entry((
                            owner,
                            constraint.source_mouth_order_index,
                            source_band_index,
                        ))
                        .or_default()
                        .extend(constraint.points_xz.iter().copied().map(road_point_key));
                }
            }
            let (Some(owner), Some(opposite_owner)) = (constraint.owner, constraint.opposite_owner)
            else {
                continue;
            };
            let Some((owner, opposite_owner)) =
                exact_generated_contact_owner_pair(constraint.kind, owner, opposite_owner)
            else {
                continue;
            };
            segments_by_contact_source
                .entry((
                    constraint.kind,
                    owner,
                    opposite_owner,
                    constraint.source_mouth_order_index,
                    constraint.source_band_index,
                ))
                .or_default()
                .extend(
                    generated_constraint_directed_edges(constraint)
                        .into_iter()
                        .map(|edge| GeneratedContourEdgeKey::new(edge.start, edge.end)),
                );
        }
        Self {
            keys_by_owner,
            segments_by_owner,
            keys_by_source,
            segments_by_source,
            segments_by_contact_source,
        }
    }

    fn has_any_source(
        &self,
        owners: [Option<NodeBandOwner>; 2],
        source_mouth_order_index: usize,
        source_band_index: usize,
    ) -> bool {
        owners.into_iter().flatten().any(|owner| {
            self.keys_by_source
                .contains_key(&(owner, source_mouth_order_index, source_band_index))
        })
    }

    fn has_exact_point(
        &self,
        owners: [Option<NodeBandOwner>; 2],
        source_mouth_order_index: usize,
        source_band_index: usize,
        point: NodeRailPointKey,
    ) -> bool {
        owners.into_iter().flatten().any(|owner| {
            self.keys_by_source
                .get(&(owner, source_mouth_order_index, source_band_index))
                .is_some_and(|keys| keys.contains(&point))
        })
    }

    fn has_exact_source_segment_point(
        &self,
        kind: NodeRailConstraintKind,
        owners: [Option<NodeBandOwner>; 2],
        source_mouth_order_index: usize,
        source_band_index: usize,
        point: NodeRailPointKey,
    ) -> bool {
        let (Some(owner), Some(opposite_owner)) = (owners[0], owners[1]) else {
            return false;
        };
        let Some((owner, opposite_owner)) =
            exact_generated_contact_owner_pair(kind, owner, opposite_owner)
        else {
            return false;
        };
        self.segments_by_contact_source
            .get(&(
                kind,
                owner,
                opposite_owner,
                source_mouth_order_index,
                Some(source_band_index),
            ))
            .is_some_and(|segments| {
                segments.iter().any(|segment| {
                    generated_point_key_lies_on_segment(point, segment.start, segment.end)
                })
            })
    }

    fn has_exact_unbanded_source_segment_point(
        &self,
        kind: NodeRailConstraintKind,
        owners: [Option<NodeBandOwner>; 2],
        source_mouth_order_index: usize,
        source_band_index: usize,
        point: NodeRailPointKey,
    ) -> bool {
        if !self.source_geometry_contains_point(
            owners,
            source_mouth_order_index,
            source_band_index,
            point,
        ) {
            return false;
        }
        let (Some(owner), Some(opposite_owner)) = (owners[0], owners[1]) else {
            return false;
        };
        let Some((owner, opposite_owner)) =
            exact_generated_contact_owner_pair(kind, owner, opposite_owner)
        else {
            return false;
        };
        self.segments_by_contact_source
            .get(&(kind, owner, opposite_owner, source_mouth_order_index, None))
            .is_some_and(|segments| {
                segments.iter().any(|segment| {
                    generated_point_key_lies_on_segment(point, segment.start, segment.end)
                })
            })
    }

    fn has_exact_same_kind_source_segment_handoff_point(
        &self,
        kind: NodeRailConstraintKind,
        owners: [Option<NodeBandOwner>; 2],
        source_mouth_order_index: usize,
        source_band_index: usize,
        point: NodeRailPointKey,
    ) -> bool {
        let (Some(left_owner), Some(right_owner)) = (owners[0], owners[1]) else {
            return false;
        };
        self.has_exact_same_kind_source_segment_handoff_side(
            kind,
            left_owner,
            right_owner,
            source_mouth_order_index,
            source_band_index,
            point,
        ) || self.has_exact_same_kind_source_segment_handoff_side(
            kind,
            right_owner,
            left_owner,
            source_mouth_order_index,
            source_band_index,
            point,
        )
    }

    fn has_exact_same_kind_source_segment_handoff_side(
        &self,
        kind: NodeRailConstraintKind,
        retained_owner: NodeBandOwner,
        final_owner: NodeBandOwner,
        source_mouth_order_index: usize,
        source_band_index: usize,
        point: NodeRailPointKey,
    ) -> bool {
        if !self.owner_geometry_contains_point(final_owner, point) {
            return false;
        }
        self.segments_by_contact_source
            .iter()
            .filter(|((source_kind, _, _, source_mouth, source_band), _)| {
                *source_kind == kind
                    && *source_mouth == source_mouth_order_index
                    && *source_band == Some(source_band_index)
            })
            .any(
                |((_, source_owner, source_opposite_owner, _, _), segments)| {
                    let same_kind_handoff = (*source_owner == retained_owner
                        && source_opposite_owner.kind() == final_owner.kind())
                        || (*source_opposite_owner == retained_owner
                            && source_owner.kind() == final_owner.kind());
                    same_kind_handoff
                        && segments.iter().any(|segment| {
                            generated_point_key_lies_on_segment(point, segment.start, segment.end)
                        })
                },
            )
    }

    fn has_exact_cross_source_same_kind_contact_point(
        &self,
        kind: NodeRailConstraintKind,
        owners: [Option<NodeBandOwner>; 2],
        source_mouth_order_index: usize,
        source_band_index: usize,
        point: NodeRailPointKey,
    ) -> bool {
        let (Some(left_owner), Some(right_owner)) = (owners[0], owners[1]) else {
            return false;
        };
        (self.has_same_kind_source_segment_for_owner(
            kind,
            left_owner,
            right_owner.kind(),
            Some(source_mouth_order_index),
            Some(source_band_index),
            point,
        ) && self.has_same_kind_source_segment_for_owner(
            kind,
            right_owner,
            left_owner.kind(),
            None,
            None,
            point,
        )) || (self.has_same_kind_source_segment_for_owner(
            kind,
            left_owner,
            right_owner.kind(),
            None,
            None,
            point,
        ) && self.has_same_kind_source_segment_for_owner(
            kind,
            right_owner,
            left_owner.kind(),
            Some(source_mouth_order_index),
            Some(source_band_index),
            point,
        ))
    }

    fn has_same_kind_source_segment_for_owner(
        &self,
        kind: NodeRailConstraintKind,
        owner: NodeBandOwner,
        counterpart_kind: RoadSurfaceBandKind,
        required_source_mouth_order_index: Option<usize>,
        required_source_band_index: Option<usize>,
        point: NodeRailPointKey,
    ) -> bool {
        self.segments_by_contact_source
            .iter()
            .filter(|((source_kind, _, _, source_mouth, source_band), _)| {
                *source_kind == kind
                    && required_source_mouth_order_index
                        .is_none_or(|required| *source_mouth == required)
                    && required_source_band_index
                        .is_none_or(|required| *source_band == Some(required))
            })
            .any(
                |((_, source_owner, source_opposite_owner, _, _), segments)| {
                    let owner_matches = (*source_owner == owner
                        && source_opposite_owner.kind() == counterpart_kind)
                        || (*source_opposite_owner == owner
                            && source_owner.kind() == counterpart_kind);
                    owner_matches
                        && segments.iter().any(|segment| {
                            generated_point_key_lies_on_segment(point, segment.start, segment.end)
                        })
                },
            )
    }

    fn source_geometry_contains_point(
        &self,
        owners: [Option<NodeBandOwner>; 2],
        source_mouth_order_index: usize,
        source_band_index: usize,
        point: NodeRailPointKey,
    ) -> bool {
        owners.into_iter().flatten().any(|owner| {
            self.keys_by_source
                .get(&(owner, source_mouth_order_index, source_band_index))
                .is_some_and(|keys| keys.contains(&point))
                || self
                    .segments_by_source
                    .get(&(owner, source_mouth_order_index, source_band_index))
                    .is_some_and(|segments| {
                        segments.iter().any(|segment| {
                            generated_point_key_lies_on_segment(point, segment.start, segment.end)
                        })
                    })
        })
    }

    fn owner_geometry_contains_point(&self, owner: NodeBandOwner, point: NodeRailPointKey) -> bool {
        self.keys_by_owner
            .get(&owner)
            .is_some_and(|keys| keys.contains(&point))
            || self.segments_by_owner.get(&owner).is_some_and(|segments| {
                segments.iter().any(|segment| {
                    generated_point_key_lies_on_segment(point, segment.start, segment.end)
                })
            })
    }
}

fn exact_generated_contact_owner_pair(
    kind: NodeRailConstraintKind,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> Option<(NodeBandOwner, NodeBandOwner)> {
    if generated_contact_kind_from_constraint(kind).is_none() {
        return None;
    }
    if kind == NodeRailConstraintKind::RaisedStepContact {
        let pair = GeneratedRaisedStepOwnerPair::new(owner, opposite_owner)?;
        return Some((pair.owner, pair.opposite_owner));
    }
    Some((owner.min(opposite_owner), owner.max(opposite_owner)))
}

fn generated_contours_support_contact_noding(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
) -> bool {
    let Some(left_owner) = left.owner else {
        return false;
    };
    let Some(right_owner) = right.owner else {
        return false;
    };
    generated_raised_step_contact_kind_for_owners(left_owner, right_owner).is_some()
}

fn generated_contact_point_on_edge_noding_candidates(
    edge_contour: &NodeGeneratedContour,
    point_contour: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
) -> Vec<(GeneratedContourDirectedEdge, NodeRailPointKey)> {
    let mut candidates = Vec::new();
    let edge_keys = generated_contour_keys(edge_contour);
    for edge in generated_contour_directed_edges(edge_contour) {
        for point_key in generated_contour_keys(point_contour) {
            if edge_keys.contains(&point_key)
                || !generated_point_key_lies_on_segment(point_key, edge.start, edge.end)
                || !generated_contact_noding_point_has_explicit_roles(
                    edge_contour,
                    point_contour,
                    constraints,
                    point_key,
                )
            {
                continue;
            }
            candidates.push((edge, point_key));
        }
    }
    candidates
}

fn generated_contact_edge_intersection_noding_candidates(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
) -> Vec<(
    GeneratedContourDirectedEdge,
    GeneratedContourDirectedEdge,
    NodeRailPointKey,
)> {
    let mut candidates = Vec::new();
    for left_edge in generated_contour_directed_edges(left) {
        for right_edge in generated_contour_directed_edges(right) {
            let Some(intersection) = quantized_proper_segment_intersection(
                left_edge.start,
                left_edge.end,
                right_edge.start,
                right_edge.end,
            ) else {
                continue;
            };
            if !generated_contact_noding_point_has_explicit_roles(
                left,
                right,
                constraints,
                intersection,
            ) {
                continue;
            }
            candidates.push((left_edge, right_edge, intersection));
        }
    }
    candidates
}

fn generated_contact_noding_point_has_explicit_roles(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    point: NodeRailPointKey,
) -> bool {
    let Some(left_kind) = generated_contour_band_kind(left) else {
        return false;
    };
    let Some(right_kind) = generated_contour_band_kind(right) else {
        return false;
    };
    let Some(left_owner) = left.owner else {
        return false;
    };
    let Some(right_owner) = right.owner else {
        return false;
    };
    let Some(contact_kind) = generated_raised_step_contact_kind_for_owners(left_owner, right_owner)
    else {
        return false;
    };
    generated_contact_point_has_explicit_roles(
        left_kind,
        right_kind,
        left,
        right,
        constraints,
        point,
        contact_kind,
    )
}

fn generated_raised_step_contact_kind_for_owners(
    left_owner: NodeBandOwner,
    right_owner: NodeBandOwner,
) -> Option<NodeRailConstraintKind> {
    GeneratedRaisedStepOwnerPair::new(left_owner, right_owner)
        .map(|_| NodeRailConstraintKind::RaisedStepContact)
}

fn raised_step_band_kinds_can_contact(
    left_kind: RoadSurfaceBandKind,
    right_kind: RoadSurfaceBandKind,
) -> bool {
    let Some(left_rank) = raised_step_band_kind_rank(left_kind) else {
        return false;
    };
    let Some(right_rank) = raised_step_band_kind_rank(right_kind) else {
        return false;
    };
    left_rank.abs_diff(right_rank) == 1
}

fn raised_step_band_kind_rank(kind: RoadSurfaceBandKind) -> Option<u8> {
    match kind {
        RoadSurfaceBandKind::Carriageway => Some(0),
        RoadSurfaceBandKind::CurbOrShoulder | RoadSurfaceBandKind::Footpath => Some(1),
        RoadSurfaceBandKind::Sidewalk => Some(2),
        RoadSurfaceBandKind::Median
        | RoadSurfaceBandKind::Parking
        | RoadSurfaceBandKind::CycleTrack
        | RoadSurfaceBandKind::TramReservation => None,
    }
}

fn generated_raised_step_boundary_role_for_owner(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> Option<GeneratedSameBandBoundaryRole> {
    GeneratedRaisedStepOwnerPair::new(owner, opposite_owner)?;
    let owner_rank = raised_step_band_kind_rank(owner.kind())?;
    let opposite_rank = raised_step_band_kind_rank(opposite_owner.kind())?;
    if opposite_rank < owner_rank {
        Some(GeneratedSameBandBoundaryRole::LowerSide)
    } else if opposite_rank > owner_rank {
        Some(GeneratedSameBandBoundaryRole::RaisedSide)
    } else {
        None
    }
}

fn generated_same_band_point_contact_has_explicit_roles(
    kind: RoadSurfaceBandKind,
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    point: NodeRailPointKey,
) -> bool {
    generated_contour_supports_same_band_role(kind)
        && generated_same_band_boundary_role_at_contour_vertex(left, constraints, point).is_some()
        && generated_same_band_boundary_role_at_contour_vertex(right, constraints, point).is_some()
}

fn generated_contact_point_has_explicit_roles(
    left_kind: RoadSurfaceBandKind,
    right_kind: RoadSurfaceBandKind,
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    point: NodeRailPointKey,
    contact_kind: NodeRailConstraintKind,
) -> bool {
    if left_kind == right_kind {
        return generated_same_band_point_contact_has_explicit_roles(
            left_kind,
            left,
            right,
            constraints,
            point,
        );
    }
    match contact_kind {
        NodeRailConstraintKind::RaisedStepContact => {
            let Some(left_owner) = left.owner else {
                return false;
            };
            let Some(right_owner) = right.owner else {
                return false;
            };
            let Some(pair) = GeneratedRaisedStepOwnerPair::new(left_owner, right_owner) else {
                return false;
            };
            generated_exact_owner_pair_contact_authority_at_point(
                pair.owner,
                pair.opposite_owner,
                constraints,
                point,
            )
            .is_some()
        }
        _ => true,
    }
}

fn generated_same_band_contact_constraint_key(
    constraint: &NodeRailConstraint,
) -> Option<GeneratedSameBandContactConstraintKey> {
    generated_same_band_contact_constraint(constraint).map(GeneratedSameBandContactConstraint::key)
}

fn generated_same_band_contact_constraint(
    constraint: &NodeRailConstraint,
) -> Option<GeneratedSameBandContactConstraint> {
    let Some(kind) = generated_contact_kind_from_constraint(constraint.kind) else {
        return None;
    };
    let owner = constraint.owner?;
    let opposite_owner = constraint.opposite_owner?;
    if owner == opposite_owner {
        return None;
    }
    let points = constraint.points_xz.as_slice();
    if points.len() != 2 {
        return None;
    }
    let (owner, opposite_owner) = if kind == NodeRailConstraintKind::RaisedStepContact {
        let pair = GeneratedRaisedStepOwnerPair::new(owner, opposite_owner)?;
        (pair.owner, pair.opposite_owner)
    } else {
        (owner.min(opposite_owner), owner.max(opposite_owner))
    };
    Some(GeneratedSameBandContactConstraint {
        kind,
        owner,
        opposite_owner,
        start: road_point_key(points[0]),
        end: road_point_key(points[1]),
        source_mouth_order_index: constraint.source_mouth_order_index,
        source_band_index: constraint.source_band_index,
    })
}

fn generated_contact_kind_from_constraint(
    kind: NodeRailConstraintKind,
) -> Option<NodeRailConstraintKind> {
    match kind {
        NodeRailConstraintKind::AsphaltBoundary { .. }
        | NodeRailConstraintKind::RaisedStepContact => Some(kind),
        NodeRailConstraintKind::BandBoundary {
            left_kind,
            right_kind,
        } => raised_step_band_kinds_can_contact(left_kind, right_kind).then_some(kind),
        NodeRailConstraintKind::FullRoadbedContour
        | NodeRailConstraintKind::BandContour { .. }
        | NodeRailConstraintKind::SpanHandoff { .. }
        | NodeRailConstraintKind::FootprintSeam { .. } => None,
    }
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

fn generated_point_key_lies_on_segment(
    point: NodeRailPointKey,
    start: NodeRailPointKey,
    end: NodeRailPointKey,
) -> bool {
    if point == start || point == end {
        return true;
    }
    if start == end {
        return false;
    }
    let dx = i128::from(end.0 - start.0);
    let dz = i128::from(end.1 - start.1);
    let px = i128::from(point.0 - start.0);
    let pz = i128::from(point.1 - start.1);
    let cross = px * dz - pz * dx;
    if cross != 0 && cross.abs() > generated_overlay_grid_collinearity_error_bound(dx, dz) {
        return false;
    }
    let inside_x = if start.0 == end.0 {
        point.0 == start.0
    } else {
        point.0 >= start.0.min(end.0) && point.0 <= start.0.max(end.0)
    };
    let inside_z = if start.1 == end.1 {
        point.1 == start.1
    } else {
        point.1 >= start.1.min(end.1) && point.1 <= start.1.max(end.1)
    };
    inside_x && inside_z
}

fn generated_overlay_grid_collinearity_error_bound(dx: i128, dz: i128) -> i128 {
    (dx.abs() + dz.abs()) * 2
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
    let ab_x = i128::from(b.0 - a.0);
    let ab_z = i128::from(b.1 - a.1);
    let ac_x = i128::from(c.0 - a.0);
    let ac_z = i128::from(c.1 - a.1);
    ab_x * ac_z - ab_z * ac_x
}

fn generated_segment_parameter_key(
    start: NodeRailPointKey,
    end: NodeRailPointKey,
    point: NodeRailPointKey,
) -> i128 {
    let dx = i128::from(end.0 - start.0);
    let dz = i128::from(end.1 - start.1);
    let px = i128::from(point.0 - start.0);
    let pz = i128::from(point.1 - start.1);
    px * dx + pz * dz
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
    (
        (point.x * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
        (point.y * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
    )
}

fn road_point_from_key(point: NodeRailPointKey) -> RoadVec2 {
    RoadVec2::new(
        point.0 as f64 / ROAD_OVERLAY_COORDINATE_SCALE,
        point.1 as f64 / ROAD_OVERLAY_COORDINATE_SCALE,
    )
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

fn xz(point: RoadVec3) -> RoadVec2 {
    RoadVec2::new(point.x, point.z)
}

fn is_carriageway(kind: RoadSurfaceBandKind) -> bool {
    kind == RoadSurfaceBandKind::Carriageway
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::surface::{
        IncidentEdgeSide, IncidentMouthBand, IncidentMouthProfile, OrderedIncidentPieceMouth,
    };
    use godot::prelude::{Vector2, Vector3};

    fn band(kind: RoadSurfaceBandKind, start: Vector3, end: Vector3) -> IncidentMouthBand {
        IncidentMouthBand {
            kind,
            start_point_world: start,
            end_point_world: end,
        }
    }

    fn profile(x: f32) -> IncidentMouthProfile {
        let boundary_points_world = vec![
            Vector3::new(x, 4.0, -4.0),
            Vector3::new(x, 4.1, -2.0),
            Vector3::new(x, 4.2, 0.0),
            Vector3::new(x, 4.3, 2.0),
            Vector3::new(x, 4.4, 4.0),
        ];
        let bands = vec![
            band(
                RoadSurfaceBandKind::Sidewalk,
                boundary_points_world[0],
                boundary_points_world[1],
            ),
            band(
                RoadSurfaceBandKind::CurbOrShoulder,
                boundary_points_world[1],
                boundary_points_world[2],
            ),
            band(
                RoadSurfaceBandKind::Carriageway,
                boundary_points_world[2],
                boundary_points_world[3],
            ),
            band(
                RoadSurfaceBandKind::Sidewalk,
                boundary_points_world[3],
                boundary_points_world[4],
            ),
        ];
        IncidentMouthProfile {
            inward_direction_xz: Vector2::RIGHT,
            boundary_points_world,
            bands,
        }
    }

    fn terminal_profile(x: f32) -> IncidentMouthProfile {
        let boundary_points_world = vec![
            Vector3::new(x, 4.0, -4.0),
            Vector3::new(x, 4.1, -3.0),
            Vector3::new(x, 4.2, -1.0),
            Vector3::new(x, 4.0, 0.0),
            Vector3::new(x, 4.2, 1.0),
            Vector3::new(x, 4.1, 3.0),
            Vector3::new(x, 4.0, 4.0),
        ];
        let bands = vec![
            band(
                RoadSurfaceBandKind::Sidewalk,
                boundary_points_world[0],
                boundary_points_world[1],
            ),
            band(
                RoadSurfaceBandKind::CurbOrShoulder,
                boundary_points_world[1],
                boundary_points_world[2],
            ),
            band(
                RoadSurfaceBandKind::Carriageway,
                boundary_points_world[2],
                boundary_points_world[3],
            ),
            band(
                RoadSurfaceBandKind::Carriageway,
                boundary_points_world[3],
                boundary_points_world[4],
            ),
            band(
                RoadSurfaceBandKind::CurbOrShoulder,
                boundary_points_world[4],
                boundary_points_world[5],
            ),
            band(
                RoadSurfaceBandKind::Sidewalk,
                boundary_points_world[5],
                boundary_points_world[6],
            ),
        ];
        IncidentMouthProfile {
            inward_direction_xz: Vector2::RIGHT,
            boundary_points_world,
            bands,
        }
    }

    fn terminal_profile_z(z: f32) -> IncidentMouthProfile {
        let boundary_points_world = vec![
            Vector3::new(4.0, 4.0, z),
            Vector3::new(3.0, 4.1, z),
            Vector3::new(1.0, 4.2, z),
            Vector3::new(0.0, 4.0, z),
            Vector3::new(-1.0, 4.2, z),
            Vector3::new(-3.0, 4.1, z),
            Vector3::new(-4.0, 4.0, z),
        ];
        let bands = vec![
            band(
                RoadSurfaceBandKind::Sidewalk,
                boundary_points_world[0],
                boundary_points_world[1],
            ),
            band(
                RoadSurfaceBandKind::CurbOrShoulder,
                boundary_points_world[1],
                boundary_points_world[2],
            ),
            band(
                RoadSurfaceBandKind::Carriageway,
                boundary_points_world[2],
                boundary_points_world[3],
            ),
            band(
                RoadSurfaceBandKind::Carriageway,
                boundary_points_world[3],
                boundary_points_world[4],
            ),
            band(
                RoadSurfaceBandKind::CurbOrShoulder,
                boundary_points_world[4],
                boundary_points_world[5],
            ),
            band(
                RoadSurfaceBandKind::Sidewalk,
                boundary_points_world[5],
                boundary_points_world[6],
            ),
        ];
        IncidentMouthProfile {
            inward_direction_xz: Vector2::DOWN,
            boundary_points_world,
            bands,
        }
    }

    fn input_with_endpoint_x(endpoint_x: f32) -> NodeArrangementInput {
        let mouth = OrderedIncidentPieceMouth {
            profile: profile(10.0),
            endpoint_profile: profile(endpoint_x),
            boundary_paths_world: Vec::new(),
            band_start_paths_world: Vec::new(),
            band_end_paths_world: Vec::new(),
            uses_sampled_band_domain_paths: false,
            direction_angle_ccw: 0.0,
            direction_xz: Vector2::RIGHT,
            edge_idx: 7,
            side: IncidentEdgeSide::Start,
        };
        NodeArrangementInput::from_ordered_mouths(
            42,
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &[mouth],
        )
        .expect("test mouth should produce canonical input")
    }

    fn terminal_input_with_endpoint_x(endpoint_x: f32) -> NodeArrangementInput {
        let mouth = OrderedIncidentPieceMouth {
            profile: terminal_profile(10.0),
            endpoint_profile: terminal_profile(endpoint_x),
            boundary_paths_world: Vec::new(),
            band_start_paths_world: Vec::new(),
            band_end_paths_world: Vec::new(),
            uses_sampled_band_domain_paths: false,
            direction_angle_ccw: 0.0,
            direction_xz: Vector2::RIGHT,
            edge_idx: 7,
            side: IncidentEdgeSide::Start,
        };
        NodeArrangementInput::from_ordered_mouths(
            42,
            RoadSurfaceVisualNodePieceKind::Terminal,
            &[mouth],
        )
        .expect("test terminal mouth should produce canonical input")
    }

    fn side_join_input(piece_kind: RoadSurfaceVisualNodePieceKind) -> NodeArrangementInput {
        let first = OrderedIncidentPieceMouth {
            profile: terminal_profile(10.0),
            endpoint_profile: terminal_profile(0.0),
            boundary_paths_world: Vec::new(),
            band_start_paths_world: Vec::new(),
            band_end_paths_world: Vec::new(),
            uses_sampled_band_domain_paths: false,
            direction_angle_ccw: 0.0,
            direction_xz: Vector2::RIGHT,
            edge_idx: 7,
            side: IncidentEdgeSide::Start,
        };
        let second = OrderedIncidentPieceMouth {
            profile: terminal_profile_z(12.0),
            endpoint_profile: terminal_profile_z(2.0),
            boundary_paths_world: Vec::new(),
            band_start_paths_world: Vec::new(),
            band_end_paths_world: Vec::new(),
            uses_sampled_band_domain_paths: false,
            direction_angle_ccw: std::f32::consts::FRAC_PI_2,
            direction_xz: Vector2::DOWN,
            edge_idx: 8,
            side: IncidentEdgeSide::Start,
        };
        NodeArrangementInput::from_ordered_mouths(42, piece_kind, &[first, second])
            .expect("test side-join mouths should produce canonical input")
    }

    fn nonterminal_input_with_side_join_candidate() -> NodeArrangementInput {
        side_join_input(RoadSurfaceVisualNodePieceKind::JunctionN)
    }

    fn bend_input_with_curb_side_join() -> NodeArrangementInput {
        side_join_input(RoadSurfaceVisualNodePieceKind::Bend)
    }

    fn same_owner_side_join_band() -> NodeInputSideJoinBand {
        NodeInputSideJoinBand {
            source_band_index: 3,
            band_kind: RoadSurfaceBandKind::Sidewalk,
            boundary_mode: NodeInputSideJoinBandBoundaryMode::SameOwnerOuterCap,
            inner_path_world: vec![RoadVec3::new(0.0, 4.4, 4.0), RoadVec3::new(2.0, 4.4, 4.0)],
            outer_path_world: vec![RoadVec3::new(0.9, 4.4, 6.0), RoadVec3::new(1.1, 4.4, 6.0)],
            contour_world: vec![
                RoadVec3::new(0.0, 4.4, 4.0),
                RoadVec3::new(2.0, 4.4, 4.0),
                RoadVec3::new(1.0, 4.4, 6.0),
            ],
        }
    }

    #[test]
    fn generates_backend_contours_and_constraints_from_solved_mouth_input() {
        let contours =
            NodeRailContourSet::from_input(&input_with_endpoint_x(0.0)).expect("valid contours");

        assert_eq!(contours.node_id, 42);
        assert_eq!(
            contours.piece_kind,
            RoadSurfaceVisualNodePieceKind::JunctionN
        );
        assert_eq!(contours.contours.len(), 6);
        assert_eq!(contours.constraints.len(), 19);
        assert_eq!(
            contours.contours[0].kind,
            NodeGeneratedContourKind::FullRoadbed
        );
        assert_eq!(contours.contours[0].points_xz.len(), 4);
        assert!(contours.contours.iter().any(|contour| {
            contour.kind
                == NodeGeneratedContourKind::Band {
                    kind: RoadSurfaceBandKind::Carriageway,
                }
                && contour.purpose == NodeGeneratedContourPurpose::CarriagewayCorridor
                && contour.source_mouth_order_index == 0
                && contour.source_band_index.is_none()
                && contour.owner.is_none()
                && contour.contributes_to_asphalt()
                && !contour.claims_asphalt_owner_region()
        }));
        assert!(contours.contours.iter().any(|contour| contour.kind
            == NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::Carriageway
            }
            && contour.purpose == NodeGeneratedContourPurpose::CarriagewayOwnerCarrier
            && contour.source_band_index == Some(2)
            && !contour.contributes_to_asphalt()
            && contour.claims_asphalt_owner_region()));
        assert!(
            contours
                .constraints
                .iter()
                .any(|constraint| constraint.kind == NodeRailConstraintKind::RaisedStepContact)
        );
        assert!(
            contours
                .constraints
                .iter()
                .any(|constraint| constraint.kind == NodeRailConstraintKind::RaisedStepContact)
        );
        assert_eq!(
            contours.constraints[0].kind,
            NodeRailConstraintKind::FullRoadbedContour
        );
        assert_eq!(contours.constraints[0].constraint_index, 0);
    }

    #[test]
    fn nonterminal_side_join_bands_emit_canonical_ownership_candidates() {
        let input = nonterminal_input_with_side_join_candidate();
        let contours = NodeRailContourSet::from_input(&input).expect("valid contours");
        let junction_side_join_contours = contours
            .contours
            .iter()
            .filter(|contour| contour.purpose == NodeGeneratedContourPurpose::JunctionSideJoin)
            .collect::<Vec<_>>();
        let raw_full_roadbed_corridors = contours
            .contours
            .iter()
            .filter(|contour| contour.purpose == NodeGeneratedContourPurpose::FullRoadbedCorridor)
            .collect::<Vec<_>>();
        assert_eq!(raw_full_roadbed_corridors.len(), input.mouths.len());
        let raw_carriageway_corridors = contours
            .contours
            .iter()
            .filter(|contour| {
                contour.purpose == NodeGeneratedContourPurpose::CarriagewayCorridor
                    && contour.source_band_index.is_none()
            })
            .collect::<Vec<_>>();
        assert_eq!(raw_carriageway_corridors.len(), input.mouths.len());
        for mouth in &input.mouths {
            assert!(
                raw_full_roadbed_corridors
                    .iter()
                    .any(|contour| contour.source_mouth_order_index == mouth.order_index),
                "each non-terminal mouth must emit exactly one raw full-roadbed authority corridor"
            );
            assert!(
                raw_carriageway_corridors
                    .iter()
                    .any(|contour| contour.source_mouth_order_index == mouth.order_index),
                "each non-terminal mouth must emit exactly one raw carriageway authority corridor"
            );
        }
        let expected_carriageway_owner_carriers = input
            .mouths
            .iter()
            .flat_map(|mouth| mouth.band_intervals.iter())
            .filter(|interval| interval.band_kind == RoadSurfaceBandKind::Carriageway)
            .count();
        assert_eq!(
            contours
                .contours
                .iter()
                .filter(|contour| {
                    contour.purpose == NodeGeneratedContourPurpose::CarriagewayOwnerCarrier
                        && contour.claims_asphalt_owner_region()
                        && !contour.contributes_to_asphalt()
                })
                .count(),
            expected_carriageway_owner_carriers
        );

        assert!(contours.contours.iter().any(|contour| {
            contour.kind == NodeGeneratedContourKind::FullRoadbed
                && contour.claim_priority == NodeGeneratedContourClaimPriority::Footprint
                && contour.source_mouth_order_index == 0
        }));
        assert!(contours.contours.iter().any(|contour| {
            contour.kind
                == NodeGeneratedContourKind::Band {
                    kind: RoadSurfaceBandKind::Sidewalk,
                }
                && contour.purpose == NodeGeneratedContourPurpose::JunctionSideJoin
                && contour.claim_priority == NodeGeneratedContourClaimPriority::SideJoin
                && contour.source_mouth_order_index == 0
                && contour.source_band_index == Some(5)
        }));
        assert!(!junction_side_join_contours.is_empty());
        assert!(junction_side_join_contours.iter().all(
            |contour| !contour.contributes_to_footprint() && !contour.contributes_to_asphalt()
        ));
        assert!(contours.constraints.iter().any(|constraint| {
            matches!(
                constraint.kind,
                NodeRailConstraintKind::FootprintSeam {
                    adjacent_kind: RoadSurfaceBandKind::Sidewalk
                }
            ) && constraint.source_band_index == Some(5)
        }));
    }

    #[test]
    fn bend_curb_side_join_bands_contribute_canonical_footprint() {
        let contours = NodeRailContourSet::from_input(&bend_input_with_curb_side_join())
            .expect("valid contours");

        assert!(contours.contours.iter().any(|contour| {
            contour.kind == NodeGeneratedContourKind::FullRoadbed
                && contour.claim_priority == NodeGeneratedContourClaimPriority::Footprint
                && contour.source_mouth_order_index == 0
        }));
        assert!(contours.contours.iter().any(|contour| {
            contour.kind
                == NodeGeneratedContourKind::Band {
                    kind: RoadSurfaceBandKind::CurbOrShoulder,
                }
                && contour.purpose == NodeGeneratedContourPurpose::BendSideJoin
                && contour.claim_priority == NodeGeneratedContourClaimPriority::SideJoin
                && contour.source_band_index == Some(4)
        }));
    }

    #[test]
    fn bend_side_join_contacts_name_exact_adjacent_owner_pair() {
        let contours = NodeRailContourSet::from_input(&bend_input_with_curb_side_join())
            .expect("valid contours");

        let side_join_owner = contours
            .contours
            .iter()
            .find(|contour| {
                contour.purpose == NodeGeneratedContourPurpose::BendSideJoin
                    && contour.source_band_index == Some(4)
            })
            .and_then(|contour| contour.owner)
            .expect("side-join contour should have a band owner");
        assert!(contours.constraints.iter().any(|constraint| {
            constraint.kind == NodeRailConstraintKind::RaisedStepContact
                && constraint.source_band_index == Some(4)
                && (constraint.owner == Some(side_join_owner)
                    || constraint.opposite_owner == Some(side_join_owner))
                && generated_constraint_opposite_owner(constraint, side_join_owner)
                    .is_some_and(|owner| owner.kind() == RoadSurfaceBandKind::Carriageway)
        }));
        assert!(!contours.constraints.iter().any(|constraint| {
            constraint.kind == NodeRailConstraintKind::RaisedStepContact
                && constraint.source_band_index == Some(4)
                && (constraint.owner == Some(side_join_owner)
                    || constraint.opposite_owner == Some(side_join_owner))
                && constraint.opposite_owner.is_none()
        }));
    }

    #[test]
    fn generated_contact_rejects_non_exact_owner_pair_authority() {
        let asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let actual_curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let source_curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 2);
        let mut contours = Vec::new();
        let mut constraints = Vec::new();

        push_generated_contour(
            NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::Carriageway,
            },
            0,
            Some(0),
            Some(asphalt_owner),
            NodeGeneratedContourClaimPriority::MouthBand,
            NodeRailConstraintKind::BandContour {
                kind: RoadSurfaceBandKind::Carriageway,
            },
            vec![
                RoadVec2::new(0.0, 0.0),
                RoadVec2::new(2.0, 0.0),
                RoadVec2::new(2.0, 1.0),
                RoadVec2::new(0.0, 1.0),
            ],
            None,
            &mut contours,
            &mut constraints,
        )
        .expect("asphalt contour is valid");
        push_generated_contour(
            NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::CurbOrShoulder,
            },
            0,
            Some(1),
            Some(actual_curb_owner),
            NodeGeneratedContourClaimPriority::MouthBand,
            NodeRailConstraintKind::BandContour {
                kind: RoadSurfaceBandKind::CurbOrShoulder,
            },
            vec![
                RoadVec2::new(0.0, 1.0),
                RoadVec2::new(2.0, 1.0),
                RoadVec2::new(2.0, 2.0),
                RoadVec2::new(0.0, 2.0),
            ],
            None,
            &mut contours,
            &mut constraints,
        )
        .expect("curb contour is valid");
        constraints.push(NodeRailConstraint {
            constraint_index: constraints.len(),
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index: 0,
            source_band_index: None,
            source_boundary_index: Some(1),
            owner: Some(asphalt_owner),
            opposite_owner: Some(source_curb_owner),
            points_xz: vec![RoadVec2::new(0.0, 1.0), RoadVec2::new(2.0, 1.0)],
        });

        append_generated_same_band_contact_constraints(
            RoadSurfaceVisualNodePieceKind::Bend,
            &contours,
            &mut constraints,
        );

        assert!(!constraints.iter().any(|constraint| {
            let start = road_point_key(RoadVec2::new(0.0, 1.0));
            let end = road_point_key(RoadVec2::new(2.0, 1.0));
            constraint.kind == NodeRailConstraintKind::RaisedStepContact
                && owners_match_unordered(
                    constraint.owner,
                    constraint.opposite_owner,
                    asphalt_owner,
                    actual_curb_owner,
                )
                && road_point_key(constraint.points_xz[0]) == start
                && road_point_key(constraint.points_xz[1]) == end
        }));
    }

    #[test]
    fn bend_side_join_point_contact_reowns_exact_source_rail_by_band_kind() {
        let asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let actual_curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let source_curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 2);
        let mut contours = Vec::new();
        let mut constraints = Vec::new();

        push_generated_contour(
            NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::CurbOrShoulder,
            },
            0,
            Some(1),
            Some(actual_curb_owner),
            NodeGeneratedContourClaimPriority::SideJoin,
            NodeRailConstraintKind::BandContour {
                kind: RoadSurfaceBandKind::CurbOrShoulder,
            },
            vec![
                RoadVec2::new(2.0, 0.5),
                RoadVec2::new(4.0, 0.5),
                RoadVec2::new(4.0, 1.5),
                RoadVec2::new(2.0, 1.5),
            ],
            None,
            &mut contours,
            &mut constraints,
        )
        .expect("curb side join contour is valid");
        constraints.push(NodeRailConstraint {
            constraint_index: constraints.len(),
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index: 0,
            source_band_index: Some(1),
            source_boundary_index: Some(1),
            owner: Some(asphalt_owner),
            opposite_owner: Some(source_curb_owner),
            points_xz: vec![RoadVec2::new(0.0, 1.0), RoadVec2::new(4.0, 1.0)],
        });

        let mut junction_constraints = constraints.clone();
        append_source_authorized_raised_step_point_contacts(
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &contours,
            &mut junction_constraints,
        );
        assert!(!junction_constraints.iter().any(|constraint| {
            constraint.kind == NodeRailConstraintKind::RaisedStepContact
                && owners_match_unordered(
                    constraint.owner,
                    constraint.opposite_owner,
                    asphalt_owner,
                    actual_curb_owner,
                )
        }));

        append_source_authorized_raised_step_point_contacts(
            RoadSurfaceVisualNodePieceKind::Bend,
            &contours,
            &mut constraints,
        );

        let start = road_point_key(RoadVec2::new(2.0, 1.0));
        let end = road_point_key(RoadVec2::new(4.0, 1.0));
        for point in [start, end] {
            assert!(constraints.iter().any(|constraint| {
                constraint.kind == NodeRailConstraintKind::RaisedStepContact
                    && owners_match_unordered(
                        constraint.owner,
                        constraint.opposite_owner,
                        asphalt_owner,
                        actual_curb_owner,
                    )
                    && constraint.points_xz.len() == 2
                    && road_point_key(constraint.points_xz[0]) == point
                    && road_point_key(constraint.points_xz[1]) == point
            }));
        }
        assert!(!constraints.iter().any(|constraint| {
            constraint.kind == NodeRailConstraintKind::RaisedStepContact
                && owners_match_unordered(
                    constraint.owner,
                    constraint.opposite_owner,
                    asphalt_owner,
                    actual_curb_owner,
                )
                && constraint.points_xz.len() == 2
                && road_point_key(constraint.points_xz[0]) == start
                && road_point_key(constraint.points_xz[1]) == end
        }));
    }

    #[test]
    fn source_endpoint_authority_rejects_noncanonical_generated_contact_endpoint() {
        let asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let mut contours = Vec::new();
        let mut constraints = Vec::new();

        push_generated_contour(
            NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::Carriageway,
            },
            0,
            Some(0),
            Some(asphalt_owner),
            NodeGeneratedContourClaimPriority::MouthBand,
            NodeRailConstraintKind::BandContour {
                kind: RoadSurfaceBandKind::Carriageway,
            },
            vec![
                RoadVec2::new(0.0, 0.0),
                RoadVec2::new(2.0, 0.0),
                RoadVec2::new(2.0, 1.0),
                RoadVec2::new(0.0, 1.0),
            ],
            None,
            &mut contours,
            &mut constraints,
        )
        .expect("asphalt contour is valid");
        push_generated_contour(
            NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::CurbOrShoulder,
            },
            0,
            Some(1),
            Some(curb_owner),
            NodeGeneratedContourClaimPriority::MouthBand,
            NodeRailConstraintKind::BandContour {
                kind: RoadSurfaceBandKind::CurbOrShoulder,
            },
            vec![
                RoadVec2::new(0.000001, 1.0),
                RoadVec2::new(2.000001, 1.0),
                RoadVec2::new(2.000001, 2.0),
                RoadVec2::new(0.000001, 2.0),
            ],
            None,
            &mut contours,
            &mut constraints,
        )
        .expect("curb contour is valid");
        let generated_constraint_start_index = constraints.len();
        constraints.push(NodeRailConstraint {
            constraint_index: constraints.len(),
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index: 0,
            source_band_index: Some(0),
            source_boundary_index: Some(1),
            owner: Some(asphalt_owner),
            opposite_owner: Some(curb_owner),
            points_xz: vec![RoadVec2::new(0.000001, 1.0), RoadVec2::new(2.0, 1.0)],
        });

        let drifted_start = road_point_key(RoadVec2::new(0.000001, 1.0));
        let error = validate_generated_contact_constraint_endpoints_from_sources(
            &contours,
            &constraints,
            generated_constraint_start_index,
        )
        .expect_err("generated contact endpoints must be exact source keys");

        assert!(matches!(
            error,
            NodeRailGenerationError::NonCanonicalGeneratedContactEndpoint {
                kind: NodeRailConstraintKind::RaisedStepContact,
                mouth_order_index: 0,
                band_index: Some(0),
                owner: Some(owner),
                opposite_owner: Some(opposite_owner),
                point_x_key,
                point_z_key,
            } if owner == asphalt_owner
                && opposite_owner == curb_owner
                && point_x_key == drifted_start.0
                && point_z_key == drifted_start.1
        ));
    }

    #[test]
    fn generated_raised_step_owner_pair_splits_carriageway_boundary_at_overlay_contact() {
        let asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let actual_curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let mut contours = Vec::new();
        let mut constraints = Vec::new();

        push_generated_contour(
            NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::Carriageway,
            },
            0,
            Some(0),
            Some(asphalt_owner),
            NodeGeneratedContourClaimPriority::MouthBand,
            NodeRailConstraintKind::BandContour {
                kind: RoadSurfaceBandKind::Carriageway,
            },
            vec![
                RoadVec2::new(0.0, 0.0),
                RoadVec2::new(4.0, 0.0),
                RoadVec2::new(4.0, 1.0),
                RoadVec2::new(0.0, 1.0),
            ],
            None,
            &mut contours,
            &mut constraints,
        )
        .expect("asphalt contour is valid");
        push_generated_contour(
            NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::CurbOrShoulder,
            },
            0,
            Some(1),
            Some(actual_curb_owner),
            NodeGeneratedContourClaimPriority::MouthBand,
            NodeRailConstraintKind::BandContour {
                kind: RoadSurfaceBandKind::CurbOrShoulder,
            },
            vec![
                RoadVec2::new(3.0, 0.5),
                RoadVec2::new(4.0, 0.5),
                RoadVec2::new(4.0, 1.5),
                RoadVec2::new(3.0, 1.5),
            ],
            None,
            &mut contours,
            &mut constraints,
        )
        .expect("curb contour is valid");
        constraints.push(NodeRailConstraint {
            constraint_index: constraints.len(),
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index: 0,
            source_band_index: None,
            source_boundary_index: Some(1),
            owner: Some(asphalt_owner),
            opposite_owner: Some(actual_curb_owner),
            points_xz: vec![RoadVec2::new(0.0, 1.0), RoadVec2::new(4.0, 1.0)],
        });

        append_generated_same_band_contact_constraints(
            RoadSurfaceVisualNodePieceKind::Bend,
            &contours,
            &mut constraints,
        );

        let start = road_point_key(RoadVec2::new(3.0, 1.0));
        let end = road_point_key(RoadVec2::new(4.0, 1.0));
        assert!(constraints.iter().any(|constraint| {
            constraint.kind == NodeRailConstraintKind::RaisedStepContact
                && owners_match_unordered(
                    constraint.owner,
                    constraint.opposite_owner,
                    asphalt_owner,
                    actual_curb_owner,
                )
                && constraint.points_xz.len() == 2
                && road_point_key(constraint.points_xz[0]) == start
                && road_point_key(constraint.points_xz[1]) == end
        }));
    }

    #[test]
    fn generated_raised_step_owner_pair_splits_curb_sidewalk_boundary_at_overlay_contact() {
        let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 0);
        let sidewalk_owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 1);
        let mut contours = Vec::new();
        let mut constraints = Vec::new();

        push_generated_contour(
            NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::CurbOrShoulder,
            },
            0,
            Some(0),
            Some(curb_owner),
            NodeGeneratedContourClaimPriority::MouthBand,
            NodeRailConstraintKind::BandContour {
                kind: RoadSurfaceBandKind::CurbOrShoulder,
            },
            vec![
                RoadVec2::new(0.0, 0.0),
                RoadVec2::new(4.0, 0.0),
                RoadVec2::new(4.0, 1.0),
                RoadVec2::new(0.0, 1.0),
            ],
            None,
            &mut contours,
            &mut constraints,
        )
        .expect("curb contour is valid");
        push_generated_contour(
            NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::Sidewalk,
            },
            0,
            Some(1),
            Some(sidewalk_owner),
            NodeGeneratedContourClaimPriority::MouthBand,
            NodeRailConstraintKind::BandContour {
                kind: RoadSurfaceBandKind::Sidewalk,
            },
            vec![
                RoadVec2::new(3.0, 0.5),
                RoadVec2::new(4.0, 0.5),
                RoadVec2::new(4.0, 1.5),
                RoadVec2::new(3.0, 1.5),
            ],
            None,
            &mut contours,
            &mut constraints,
        )
        .expect("sidewalk contour is valid");
        constraints.push(NodeRailConstraint {
            constraint_index: constraints.len(),
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index: 0,
            source_band_index: Some(1),
            source_boundary_index: Some(1),
            owner: Some(sidewalk_owner),
            opposite_owner: Some(curb_owner),
            points_xz: vec![RoadVec2::new(0.0, 1.0), RoadVec2::new(4.0, 1.0)],
        });

        append_generated_same_band_contact_constraints(
            RoadSurfaceVisualNodePieceKind::Bend,
            &contours,
            &mut constraints,
        );

        let start = road_point_key(RoadVec2::new(3.0, 1.0));
        let end = road_point_key(RoadVec2::new(4.0, 1.0));
        assert!(constraints.iter().any(|constraint| {
            constraint.kind == NodeRailConstraintKind::RaisedStepContact
                && owners_match_unordered(
                    constraint.owner,
                    constraint.opposite_owner,
                    curb_owner,
                    sidewalk_owner,
                )
                && constraint.points_xz.len() == 2
                && road_point_key(constraint.points_xz[0]) == start
                && road_point_key(constraint.points_xz[1]) == end
        }));
    }

    #[test]
    fn generated_raised_step_owner_pair_uses_source_authority_union_for_split_domains() {
        let asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let actual_curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let mut contours = Vec::new();
        let mut constraints = Vec::new();

        push_generated_contour(
            NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::Carriageway,
            },
            0,
            Some(0),
            Some(asphalt_owner),
            NodeGeneratedContourClaimPriority::MouthBand,
            NodeRailConstraintKind::BandContour {
                kind: RoadSurfaceBandKind::Carriageway,
            },
            vec![
                RoadVec2::new(0.0, 0.0),
                RoadVec2::new(4.0, 0.0),
                RoadVec2::new(4.0, 1.0),
                RoadVec2::new(0.0, 1.0),
            ],
            None,
            &mut contours,
            &mut constraints,
        )
        .expect("asphalt contour is valid");
        for points in [
            vec![
                RoadVec2::new(2.0, 0.5),
                RoadVec2::new(3.2, 0.5),
                RoadVec2::new(3.2, 1.5),
                RoadVec2::new(2.0, 1.5),
            ],
            vec![
                RoadVec2::new(2.8, 0.5),
                RoadVec2::new(4.0, 0.5),
                RoadVec2::new(4.0, 1.5),
                RoadVec2::new(2.8, 1.5),
            ],
        ] {
            push_generated_contour(
                NodeGeneratedContourKind::Band {
                    kind: RoadSurfaceBandKind::CurbOrShoulder,
                },
                0,
                Some(1),
                Some(actual_curb_owner),
                NodeGeneratedContourClaimPriority::MouthBand,
                NodeRailConstraintKind::BandContour {
                    kind: RoadSurfaceBandKind::CurbOrShoulder,
                },
                points,
                None,
                &mut contours,
                &mut constraints,
            )
            .expect("curb contour is valid");
        }
        constraints.push(NodeRailConstraint {
            constraint_index: constraints.len(),
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index: 0,
            source_band_index: Some(1),
            source_boundary_index: Some(1),
            owner: Some(asphalt_owner),
            opposite_owner: Some(actual_curb_owner),
            points_xz: vec![RoadVec2::new(0.0, 1.0), RoadVec2::new(4.0, 1.0)],
        });

        append_generated_same_band_contact_constraints(
            RoadSurfaceVisualNodePieceKind::Bend,
            &contours,
            &mut constraints,
        );

        let start = road_point_key(RoadVec2::new(2.0, 1.0));
        let end = road_point_key(RoadVec2::new(4.0, 1.0));
        assert!(constraints.iter().any(|constraint| {
            constraint.kind == NodeRailConstraintKind::RaisedStepContact
                && owners_match_unordered(
                    constraint.owner,
                    constraint.opposite_owner,
                    asphalt_owner,
                    actual_curb_owner,
                )
                && constraint.points_xz.len() == 2
                && road_point_key(constraint.points_xz[0]) == start
                && road_point_key(constraint.points_xz[1]) == end
        }));
    }

    #[test]
    fn nonterminal_same_owner_caps_emit_canonical_side_join_fill() {
        let input = input_with_endpoint_x(0.0);
        let side_join_bands = vec![same_owner_side_join_band()];
        let owners_by_mouth = owners_by_mouth(&input, &[], std::slice::from_ref(&side_join_bands));
        let mut contours = Vec::new();
        let mut constraints = Vec::new();
        push_side_join_band_contours(
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &input.mouths[0],
            &side_join_bands,
            &owners_by_mouth[0],
            &owners_by_mouth[0].side_join_band_owners,
            &mut contours,
            &mut constraints,
        )
        .expect("valid side-join contours");
        let cap_tip = road_point_key(RoadVec2::new(1.0, 6.0));

        assert!(
            NodeGeneratedContourClaimPriority::SideJoin
                < NodeGeneratedContourClaimPriority::MouthBand,
            "side-join candidates must stay ahead of mouth bands during non-road ownership cleanup"
        );
        assert!(!contours.iter().any(|contour| {
            contour.kind == NodeGeneratedContourKind::FullRoadbed
                && contour.claim_priority == NodeGeneratedContourClaimPriority::Footprint
                && contour
                    .points_xz
                    .iter()
                    .any(|point| road_point_key(*point) == cap_tip)
        }));
        assert!(contours.iter().any(|contour| {
            contour.kind
                == NodeGeneratedContourKind::Band {
                    kind: RoadSurfaceBandKind::Sidewalk,
                }
                && contour.purpose == NodeGeneratedContourPurpose::JunctionSideJoin
                && contour.claim_priority == NodeGeneratedContourClaimPriority::SideJoin
                && contour.source_band_index == Some(3)
                && contour
                    .points_xz
                    .iter()
                    .any(|point| road_point_key(*point) == cap_tip)
        }));
    }

    #[test]
    fn terminal_cap_contacts_name_adjacent_owner_pairs() {
        let input = terminal_input_with_endpoint_x(0.0);
        let terminal_curb_source = input.mouths[0].band_intervals.len();
        let contours = NodeRailContourSet::from_input(&input).expect("valid terminal contours");
        let terminal_curb_owner = contours
            .contours
            .iter()
            .find(|contour| {
                contour.purpose == NodeGeneratedContourPurpose::TerminalCap
                    && contour.source_band_index == Some(terminal_curb_source)
            })
            .and_then(|contour| contour.owner)
            .expect("terminal cap should have an owner");
        let left_carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
        let right_carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 3);
        let left_segment = GeneratedContourEdgeKey::new(
            road_point_key(RoadVec2::new(0.0, -1.0)),
            road_point_key(RoadVec2::new(0.0, 0.0)),
        );
        let right_segment = GeneratedContourEdgeKey::new(
            road_point_key(RoadVec2::new(0.0, 0.0)),
            road_point_key(RoadVec2::new(0.0, 1.0)),
        );
        let contacts = contours
            .constraints
            .iter()
            .filter(|constraint| {
                constraint.kind == NodeRailConstraintKind::RaisedStepContact
                    && constraint.source_band_index == Some(terminal_curb_source)
                    && (constraint.owner == Some(terminal_curb_owner)
                        || constraint.opposite_owner == Some(terminal_curb_owner))
            })
            .filter_map(|constraint| {
                let opposite_owner =
                    generated_constraint_opposite_owner(constraint, terminal_curb_owner)?;
                Some((
                    GeneratedContourEdgeKey::new(
                        road_point_key(constraint.points_xz[0]),
                        road_point_key(constraint.points_xz[1]),
                    ),
                    opposite_owner,
                ))
            })
            .collect::<BTreeSet<_>>();

        assert!(contacts.contains(&(left_segment, left_carriageway)));
        assert!(contacts.contains(&(right_segment, right_carriageway)));
    }

    #[test]
    fn rejects_degenerate_backend_contours() {
        let error = NodeRailContourSet::from_input(&input_with_endpoint_x(10.0))
            .expect_err("zero-depth mouth should collapse its contours");

        assert!(matches!(
            error,
            NodeRailGenerationError::DegenerateContour {
                kind: NodeGeneratedContourKind::FullRoadbed,
                mouth_order_index: 0,
                band_index: None,
                ..
            }
        ));
    }
}
