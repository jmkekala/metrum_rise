//! Library-backed rail and contour generation for canonical node arrangements.

#![allow(dead_code)]

use super::arrangement::NodeBandOwner;
use super::backend::{
    ROAD_OVERLAY_COORDINATE_SCALE, RoadPolyline, RoadVec2, RoadVec3, polyline_to_road_points,
    road_points_to_polyline,
};
use super::input::{
    NodeArrangementInput, NodeInputBandInterval, NodeInputBoundaryRailRole, NodeInputMouth,
    NodeInputProfileRail, NodeInputTerminalEndBand, NodeInputTerminalEndBandBoundaryMode,
};
use super::{
    NODE_OVERLAY_MIN_AREA_M2, RoadSurfaceBandKind, RoadSurfaceSystem,
    RoadSurfaceVisualNodePieceKind,
};
use cavalier_contours::polyline::{PlineCreation, PlineSource, PlineSourceMut};
use std::collections::{BTreeMap, BTreeSet};

const RAIL_CONTOUR_POINT_EQUAL_EPS_M: f64 = 1.0e-6;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) enum NodeGeneratedContourKind {
    FullRoadbed,
    Band { kind: RoadSurfaceBandKind },
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
    AsphaltCurbContact,
    CurbSidewalkContact,
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
}

#[derive(Clone, Debug)]
pub(crate) struct NodeGeneratedContour {
    pub(crate) kind: NodeGeneratedContourKind,
    pub(crate) source_mouth_order_index: usize,
    pub(crate) source_band_index: Option<usize>,
    pub(crate) owner: Option<NodeBandOwner>,
    pub(crate) claim_priority: NodeGeneratedContourClaimPriority,
    pub(crate) points_xz: Vec<RoadVec2>,
    pub(crate) backend_polyline: RoadPolyline,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) enum NodeGeneratedContourClaimPriority {
    JoinOrCap,
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
}

struct MouthOwners {
    band_owners: Vec<NodeBandOwner>,
    terminal_end_band_owners: Vec<NodeBandOwner>,
}

#[derive(Clone, Copy)]
struct GeneratedSameBandRoleJoinRewrite {
    donor_contour_index: usize,
    receiver_contour_index: usize,
    equal_key: NodeRailPointKey,
    conflict_key: NodeRailPointKey,
    candidate_key: NodeRailPointKey,
}

#[derive(Clone, Copy)]
struct GeneratedSameBandRolePointRewrite {
    donor_contour_index: usize,
    previous_key: NodeRailPointKey,
    conflict_key: NodeRailPointKey,
    next_key: NodeRailPointKey,
}

#[derive(Clone, Copy)]
struct GeneratedSameBandRoleChordRewrite {
    donor_contour_index: usize,
    receiver_contour_index: usize,
    lower_key: NodeRailPointKey,
    raised_key: NodeRailPointKey,
    conflict_key: NodeRailPointKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct GeneratedSameBandRoleJoinRewriteOrder {
    removed_role_priority: u8,
    donor_contour_index: usize,
    receiver_contour_index: usize,
    removed_key: NodeRailPointKey,
    kept_key: NodeRailPointKey,
    candidate_key: NodeRailPointKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct GeneratedSameBandRolePointRewriteOrder {
    removed_role_priority: u8,
    donor_contour_index: usize,
    conflict_key: NodeRailPointKey,
    previous_key: NodeRailPointKey,
    next_key: NodeRailPointKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct GeneratedSameBandTransitionKey {
    kept_key: NodeRailPointKey,
    side_a: NodeRailPointKey,
    side_b: NodeRailPointKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum GeneratedSameBandBoundaryRole {
    LowerSide,
    RaisedSide,
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

#[derive(Clone, Copy)]
struct GeneratedSameBandContourIntersection {
    left_contour_index: usize,
    right_contour_index: usize,
    left_edge: GeneratedContourDirectedEdge,
    right_edge: GeneratedContourDirectedEdge,
    intersection_key: NodeRailPointKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct GeneratedSameBandContactConstraint {
    kind: RoadSurfaceBandKind,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    start: NodeRailPointKey,
    end: NodeRailPointKey,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct GeneratedSameBandContactConstraintKey {
    kind: RoadSurfaceBandKind,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    start: NodeRailPointKey,
    end: NodeRailPointKey,
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

        let owners_by_mouth = owners_by_mouth(input);
        let mut contours = Vec::new();
        let mut constraints = Vec::new();

        for (mouth, mouth_owners) in input.mouths.iter().zip(&owners_by_mouth) {
            push_full_roadbed_contour(mouth, &mut contours, &mut constraints)?;

            for (band_index, interval) in mouth.band_intervals.iter().enumerate() {
                let owner = mouth_owners.band_owners[band_index];
                push_band_contour(mouth, interval, owner, &mut contours, &mut constraints)?;
            }

            for (end_band, owner) in mouth
                .terminal_end_bands
                .iter()
                .zip(&mouth_owners.terminal_end_band_owners)
            {
                push_terminal_end_band_contour(
                    mouth,
                    end_band,
                    *owner,
                    &mut contours,
                    &mut constraints,
                )?;
            }

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
        if input.piece_kind != RoadSurfaceVisualNodePieceKind::Terminal {
            resolve_generated_same_band_curb_transition_ownership(&mut contours, &mut constraints)?;
        }
        append_generated_same_band_contact_constraints(&contours, &mut constraints);

        Ok(Self {
            node_id: input.node_id,
            piece_kind: input.piece_kind,
            contours,
            constraints,
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
    let points = vec![
        xz(first.mouth_world),
        xz(last.mouth_world),
        xz(last.endpoint_world),
        xz(first.endpoint_world),
    ];
    let contour = cleaned_closed_contour(
        NodeGeneratedContourKind::FullRoadbed,
        mouth.order_index,
        None,
        points,
    )?;
    let points_xz = polyline_to_road_points(&contour);
    contours.push(NodeGeneratedContour {
        kind: NodeGeneratedContourKind::FullRoadbed,
        source_mouth_order_index: mouth.order_index,
        source_band_index: None,
        owner: None,
        claim_priority: NodeGeneratedContourClaimPriority::Footprint,
        points_xz: points_xz.clone(),
        backend_polyline: contour,
    });
    push_constraint(
        constraints,
        NodeRailConstraintKind::FullRoadbedContour,
        mouth.order_index,
        None,
        None,
        None,
        None,
        points_xz,
    )
}

fn push_band_contour(
    mouth: &NodeInputMouth,
    interval: &NodeInputBandInterval,
    owner: NodeBandOwner,
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let kind = NodeGeneratedContourKind::Band {
        kind: interval.band_kind,
    };
    let points = vec![
        xz(interval.mouth_start_world),
        xz(interval.mouth_end_world),
        xz(interval.endpoint_end_world),
        xz(interval.endpoint_start_world),
    ];
    let contour =
        cleaned_closed_contour(kind, mouth.order_index, Some(interval.band_index), points)?;
    let points_xz = polyline_to_road_points(&contour);
    contours.push(NodeGeneratedContour {
        kind,
        source_mouth_order_index: mouth.order_index,
        source_band_index: Some(interval.band_index),
        owner: Some(owner),
        claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
        points_xz: points_xz.clone(),
        backend_polyline: contour,
    });
    push_constraint(
        constraints,
        NodeRailConstraintKind::BandContour {
            kind: interval.band_kind,
        },
        mouth.order_index,
        Some(interval.band_index),
        None,
        Some(owner),
        None,
        points_xz,
    )
}

fn push_terminal_end_band_contour(
    mouth: &NodeInputMouth,
    end_band: &NodeInputTerminalEndBand,
    owner: NodeBandOwner,
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let points = end_band
        .contour_world
        .iter()
        .copied()
        .map(xz)
        .collect::<Vec<_>>();
    let footprint = cleaned_closed_contour(
        NodeGeneratedContourKind::FullRoadbed,
        mouth.order_index,
        None,
        points.clone(),
    )?;
    let footprint_points_xz = polyline_to_road_points(&footprint);
    contours.push(NodeGeneratedContour {
        kind: NodeGeneratedContourKind::FullRoadbed,
        source_mouth_order_index: mouth.order_index,
        source_band_index: None,
        owner: None,
        claim_priority: NodeGeneratedContourClaimPriority::Footprint,
        points_xz: footprint_points_xz.clone(),
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

    let kind = NodeGeneratedContourKind::Band {
        kind: end_band.band_kind,
    };
    let contour = cleaned_closed_contour(
        kind,
        mouth.order_index,
        Some(end_band.source_band_index),
        points,
    )?;
    let points_xz = polyline_to_road_points(&contour);
    contours.push(NodeGeneratedContour {
        kind,
        source_mouth_order_index: mouth.order_index,
        source_band_index: Some(end_band.source_band_index),
        owner: Some(owner),
        claim_priority: NodeGeneratedContourClaimPriority::JoinOrCap,
        points_xz: points_xz.clone(),
        backend_polyline: contour,
    });
    push_constraint(
        constraints,
        NodeRailConstraintKind::BandContour {
            kind: end_band.band_kind,
        },
        mouth.order_index,
        Some(end_band.source_band_index),
        None,
        Some(owner),
        None,
        points_xz,
    )?;
    push_terminal_end_band_boundary_constraints(mouth, end_band, owner, constraints)
}

fn push_terminal_end_band_boundary_constraints(
    mouth: &NodeInputMouth,
    end_band: &NodeInputTerminalEndBand,
    owner: NodeBandOwner,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let inner_edge = terminal_end_band_inner_contour_edge(end_band);
    let outer_path = terminal_end_band_outer_contour_path(end_band);
    let outer_cap_path = terminal_end_band_outer_cap_contour_path(end_band);
    match end_band.band_kind {
        RoadSurfaceBandKind::CurbOrShoulder => {
            if end_band.boundary_mode != NodeInputTerminalEndBandBoundaryMode::SameOwnerOuterCap
                && let Some((start, end)) = inner_edge
            {
                push_terminal_end_band_constraint(
                    constraints,
                    NodeRailConstraintKind::AsphaltCurbContact,
                    mouth.order_index,
                    end_band.source_band_index,
                    owner,
                    start,
                    end,
                )?;
            }
            if end_band.boundary_mode == NodeInputTerminalEndBandBoundaryMode::MaterialBand
                && let Some(points) = outer_path.clone()
            {
                push_terminal_end_band_path_constraint(
                    constraints,
                    NodeRailConstraintKind::CurbSidewalkContact,
                    mouth.order_index,
                    end_band.source_band_index,
                    owner,
                    points,
                )?;
            }
            Ok(())
        }
        RoadSurfaceBandKind::Sidewalk => {
            if end_band.boundary_mode != NodeInputTerminalEndBandBoundaryMode::SameOwnerOuterCap
                && let Some((start, end)) = inner_edge
            {
                push_terminal_end_band_constraint(
                    constraints,
                    NodeRailConstraintKind::CurbSidewalkContact,
                    mouth.order_index,
                    end_band.source_band_index,
                    owner,
                    start,
                    end,
                )?;
            }
            if end_band.boundary_mode == NodeInputTerminalEndBandBoundaryMode::MaterialBand
                && let Some(points) = outer_path
            {
                push_terminal_end_band_path_constraint(
                    constraints,
                    NodeRailConstraintKind::FootprintSeam {
                        adjacent_kind: RoadSurfaceBandKind::Sidewalk,
                    },
                    mouth.order_index,
                    end_band.source_band_index,
                    owner,
                    points,
                )?;
            }
            if end_band.boundary_mode == NodeInputTerminalEndBandBoundaryMode::SameOwnerOuterCap
                && let Some(points) = outer_cap_path
            {
                push_terminal_end_band_path_constraint(
                    constraints,
                    NodeRailConstraintKind::FootprintSeam {
                        adjacent_kind: RoadSurfaceBandKind::Sidewalk,
                    },
                    mouth.order_index,
                    end_band.source_band_index,
                    owner,
                    points,
                )?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn terminal_end_band_inner_contour_edge(
    end_band: &NodeInputTerminalEndBand,
) -> Option<(RoadVec2, RoadVec2)> {
    if end_band.contour_world.len() < 4 {
        return None;
    }
    Some((xz(end_band.contour_world[0]), xz(end_band.contour_world[1])))
}

fn terminal_end_band_outer_contour_path(
    end_band: &NodeInputTerminalEndBand,
) -> Option<Vec<RoadVec2>> {
    if end_band.contour_world.len() < 3 {
        return None;
    }
    let points = end_band
        .contour_world
        .iter()
        .copied()
        .skip(2)
        .rev()
        .map(xz)
        .collect::<Vec<_>>();
    if points.len() < 2
        || !points
            .windows(2)
            .any(|segment| road_point_key(segment[0]) != road_point_key(segment[1]))
    {
        None
    } else {
        Some(points)
    }
}

fn terminal_end_band_outer_cap_contour_path(
    end_band: &NodeInputTerminalEndBand,
) -> Option<Vec<RoadVec2>> {
    if end_band.contour_world.len() < 3 {
        return None;
    }
    let mut points = end_band
        .contour_world
        .iter()
        .copied()
        .skip(1)
        .map(xz)
        .collect::<Vec<_>>();
    points.push(xz(end_band.contour_world[0]));
    if points
        .windows(2)
        .any(|segment| road_point_key(segment[0]) != road_point_key(segment[1]))
    {
        Some(points)
    } else {
        None
    }
}

fn push_terminal_end_band_constraint(
    constraints: &mut Vec<NodeRailConstraint>,
    kind: NodeRailConstraintKind,
    source_mouth_order_index: usize,
    source_band_index: usize,
    owner: NodeBandOwner,
    start: RoadVec2,
    end: RoadVec2,
) -> Result<(), NodeRailGenerationError> {
    if road_point_key(start) == road_point_key(end) {
        return Ok(());
    }
    push_constraint(
        constraints,
        kind,
        source_mouth_order_index,
        Some(source_band_index),
        None,
        Some(owner),
        None,
        vec![start, end],
    )
}

fn push_terminal_end_band_path_constraint(
    constraints: &mut Vec<NodeRailConstraint>,
    kind: NodeRailConstraintKind,
    source_mouth_order_index: usize,
    source_band_index: usize,
    owner: NodeBandOwner,
    points: Vec<RoadVec2>,
) -> Result<(), NodeRailGenerationError> {
    push_constraint(
        constraints,
        kind,
        source_mouth_order_index,
        Some(source_band_index),
        None,
        Some(owner),
        None,
        points,
    )
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
    push_constraint(
        constraints,
        boundary_constraint_kind(role),
        mouth.order_index,
        None,
        Some(boundary_index),
        owner,
        opposite_owner,
        vec![xz(rail.mouth_world), xz(rail.endpoint_world)],
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

fn resolve_generated_same_band_curb_transition_ownership(
    contours: &mut [NodeGeneratedContour],
    constraints: &mut [NodeRailConstraint],
) -> Result<(), NodeRailGenerationError> {
    let max_passes = contours.len().saturating_mul(contours.len()).max(1);
    let mut resolved_transitions = BTreeSet::new();
    let mut resolved_chord_pairs = BTreeSet::new();
    for _ in 0..max_passes {
        if let Some(intersection) =
            generated_same_band_contour_intersection_candidate(contours)
        {
            apply_generated_same_band_contour_intersection_node(
                contours,
                constraints,
                intersection,
            )?;
            continue;
        }
        if let Some(touch) = generated_same_band_contour_touch_candidate(contours) {
            apply_generated_same_band_contour_intersection_node(contours, constraints, touch)?;
            continue;
        }
        if let Some(rewrite) = generated_same_band_role_chord_rewrite_candidate(
            contours,
            constraints,
            &resolved_chord_pairs,
        )
        {
            resolved_chord_pairs.insert(generated_same_band_contour_pair_key(
                rewrite.donor_contour_index,
                rewrite.receiver_contour_index,
            ));
            apply_generated_same_band_role_chord_rewrite(contours, constraints, rewrite)?;
            continue;
        }
        if let Some(rewrite) = generated_same_band_role_crossing_join_rewrite_candidate(
            contours,
            constraints,
            &resolved_transitions,
        ) {
            resolved_transitions.insert(generated_same_band_transition_key(
                rewrite.equal_key,
                rewrite.conflict_key,
                rewrite.candidate_key,
            ));
            apply_generated_same_band_role_join_rewrite(contours, constraints, rewrite)?;
            continue;
        }
        if let Some(rewrite) = generated_same_band_role_join_rewrite_candidate(
            contours,
            constraints,
            &resolved_transitions,
        ) {
            resolved_transitions.insert(generated_same_band_transition_key(
                rewrite.equal_key,
                rewrite.conflict_key,
                rewrite.candidate_key,
            ));
            apply_generated_same_band_role_join_rewrite(contours, constraints, rewrite)?;
            continue;
        }
        if let Some(rewrite) =
            generated_same_band_role_point_rewrite_candidate(contours, constraints)
        {
            apply_generated_same_band_role_point_rewrite(contours, constraints, rewrite)?;
            continue;
        }
        return Ok(());
    }
    Ok(())
}

fn append_generated_same_band_contact_constraints(
    contours: &[NodeGeneratedContour],
    constraints: &mut Vec<NodeRailConstraint>,
) {
    let mut contact_edges = BTreeSet::<GeneratedSameBandContactConstraint>::new();
    for left_index in 0..contours.len() {
        for right_index in left_index + 1..contours.len() {
            let left = &contours[left_index];
            let right = &contours[right_index];
            let Some(kind) = generated_contour_band_kind(left) else {
                continue;
            };
            if generated_contour_band_kind(right) != Some(kind)
                || !generated_contour_supports_same_band_contact(kind)
            {
                continue;
            }
            let Some(left_owner) = left.owner else {
                continue;
            };
            let Some(right_owner) = right.owner else {
                continue;
            };
            if left_owner == right_owner {
                continue;
            }
            let (owner, opposite_owner, source_contour) = if left_owner <= right_owner {
                (left_owner, right_owner, left)
            } else {
                (right_owner, left_owner, right)
            };
            for edge in shared_generated_contour_edges(left, right) {
                contact_edges.insert(GeneratedSameBandContactConstraint {
                    kind,
                    owner,
                    opposite_owner,
                    start: edge.start,
                    end: edge.end,
                    source_mouth_order_index: source_contour.source_mouth_order_index,
                    source_band_index: source_contour.source_band_index,
                });
            }
        }
    }

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
            kind: NodeRailConstraintKind::BandBoundary {
                left_kind: contact.kind,
                right_kind: contact.kind,
            },
            source_mouth_order_index: contact.source_mouth_order_index,
            source_band_index: contact.source_band_index,
            source_boundary_index: None,
            owner: Some(contact.owner),
            opposite_owner: Some(contact.opposite_owner),
            points_xz: vec![road_point_from_key(contact.start), road_point_from_key(contact.end)],
        });
    }
}

fn generated_same_band_contact_constraint_key(
    constraint: &NodeRailConstraint,
) -> Option<GeneratedSameBandContactConstraintKey> {
    let NodeRailConstraintKind::BandBoundary {
        left_kind,
        right_kind,
    } = constraint.kind
    else {
        return None;
    };
    if left_kind != right_kind || !generated_contour_supports_same_band_contact(left_kind) {
        return None;
    }
    let owner = constraint.owner?;
    let opposite_owner = constraint.opposite_owner?;
    if owner == opposite_owner {
        return None;
    }
    let points = constraint.points_xz.as_slice();
    if points.len() != 2 {
        return None;
    }
    Some(GeneratedSameBandContactConstraint {
        kind: left_kind,
        owner: owner.min(opposite_owner),
        opposite_owner: owner.max(opposite_owner),
        start: road_point_key(points[0]),
        end: road_point_key(points[1]),
        source_mouth_order_index: constraint.source_mouth_order_index,
        source_band_index: constraint.source_band_index,
    }
    .key())
}

fn generated_same_band_contour_intersection_candidate(
    contours: &[NodeGeneratedContour],
) -> Option<GeneratedSameBandContourIntersection> {
    let mut candidates = Vec::new();
    for left_index in 0..contours.len() {
        for right_index in left_index + 1..contours.len() {
            let left = &contours[left_index];
            let right = &contours[right_index];
            let Some(kind) = generated_contour_band_kind(left) else {
                continue;
            };
            if generated_contour_band_kind(right) != Some(kind)
                || !generated_contour_supports_same_band_noding(kind)
                || left.owner.is_none()
                || right.owner.is_none()
                || left.owner == right.owner
            {
                continue;
            }
            for left_edge in generated_contour_directed_edges(left) {
                for right_edge in generated_contour_directed_edges(right) {
                    if left_edge.start == right_edge.start
                        || left_edge.start == right_edge.end
                        || left_edge.end == right_edge.start
                        || left_edge.end == right_edge.end
                    {
                        continue;
                    }
                    let Some(intersection_key) = quantized_proper_segment_intersection(
                        left_edge.start,
                        left_edge.end,
                        right_edge.start,
                        right_edge.end,
                    ) else {
                        continue;
                    };
                    candidates.push((
                        (
                            left_index,
                            right_index,
                            intersection_key,
                            left_edge,
                            right_edge,
                        ),
                        GeneratedSameBandContourIntersection {
                            left_contour_index: left_index,
                            right_contour_index: right_index,
                            left_edge,
                            right_edge,
                            intersection_key,
                        },
                    ));
                }
            }
        }
    }
    candidates.sort_by_key(|(order, _)| *order);
    candidates.into_iter().map(|(_, candidate)| candidate).next()
}

fn generated_same_band_contour_touch_candidate(
    contours: &[NodeGeneratedContour],
) -> Option<GeneratedSameBandContourIntersection> {
    let mut candidates = Vec::new();
    for left_index in 0..contours.len() {
        for right_index in left_index + 1..contours.len() {
            let left = &contours[left_index];
            let right = &contours[right_index];
            let Some(kind) = generated_contour_band_kind(left) else {
                continue;
            };
            if generated_contour_band_kind(right) != Some(kind)
                || !generated_contour_supports_same_band_noding(kind)
                || left.owner.is_none()
                || right.owner.is_none()
                || left.owner == right.owner
            {
                continue;
            }
            collect_generated_same_band_contour_touch_candidates(
                &mut candidates,
                left_index,
                right_index,
                left,
                right,
            );
            collect_generated_same_band_contour_touch_candidates(
                &mut candidates,
                right_index,
                left_index,
                right,
                left,
            );
        }
    }
    candidates.sort_by_key(|(order, _)| *order);
    candidates.into_iter().map(|(_, candidate)| candidate).next()
}

fn collect_generated_same_band_contour_touch_candidates(
    candidates: &mut Vec<(
        (
            usize,
            usize,
            NodeRailPointKey,
            GeneratedContourDirectedEdge,
            GeneratedContourDirectedEdge,
        ),
        GeneratedSameBandContourIntersection,
    )>,
    edge_contour_index: usize,
    point_contour_index: usize,
    edge_contour: &NodeGeneratedContour,
    point_contour: &NodeGeneratedContour,
) {
    let edge_keys = generated_contour_keys(edge_contour);
    for edge in generated_contour_directed_edges(edge_contour) {
        for point_key in generated_contour_keys(point_contour) {
            if edge_keys.contains(&point_key)
                || !generated_point_key_lies_on_segment(point_key, edge.start, edge.end)
            {
                continue;
            }
            candidates.push((
                (
                    edge_contour_index,
                    point_contour_index,
                    point_key,
                    edge,
                    GeneratedContourDirectedEdge {
                        start: point_key,
                        end: point_key,
                    },
                ),
                GeneratedSameBandContourIntersection {
                    left_contour_index: edge_contour_index,
                    right_contour_index: point_contour_index,
                    left_edge: edge,
                    right_edge: GeneratedContourDirectedEdge {
                        start: point_key,
                        end: point_key,
                    },
                    intersection_key: point_key,
                },
            ));
        }
    }
}

fn apply_generated_same_band_contour_intersection_node(
    contours: &mut [NodeGeneratedContour],
    constraints: &mut [NodeRailConstraint],
    intersection: GeneratedSameBandContourIntersection,
) -> Result<(), NodeRailGenerationError> {
    let Some(left_owner) = contours
        .get(intersection.left_contour_index)
        .and_then(|contour| contour.owner)
    else {
        return Ok(());
    };
    let Some(right_owner) = contours
        .get(intersection.right_contour_index)
        .and_then(|contour| contour.owner)
    else {
        return Ok(());
    };
    insert_key_on_generated_constraints(
        constraints,
        left_owner,
        intersection.left_edge,
        intersection.intersection_key,
    );
    insert_key_on_generated_constraints(
        constraints,
        right_owner,
        intersection.right_edge,
        intersection.intersection_key,
    );
    insert_key_on_generated_contour(
        contours,
        constraints,
        intersection.left_contour_index,
        intersection.left_edge,
        intersection.intersection_key,
    )?;
    insert_key_on_generated_contour(
        contours,
        constraints,
        intersection.right_contour_index,
        intersection.right_edge,
        intersection.intersection_key,
    )?;
    Ok(())
}

fn generated_same_band_role_chord_rewrite_candidate(
    contours: &[NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
    resolved_pairs: &BTreeSet<(usize, usize)>,
) -> Option<GeneratedSameBandRoleChordRewrite> {
    let mut candidates = Vec::new();
    for donor_index in 0..contours.len() {
        for receiver_index in 0..contours.len() {
            if donor_index == receiver_index {
                continue;
            }
            if resolved_pairs.contains(&generated_same_band_contour_pair_key(
                donor_index,
                receiver_index,
            )) {
                continue;
            }
            let donor = &contours[donor_index];
            let receiver = &contours[receiver_index];
            let Some(kind) = generated_contour_band_kind(donor) else {
                continue;
            };
            if generated_contour_band_kind(receiver) != Some(kind)
                || kind != RoadSurfaceBandKind::CurbOrShoulder
                || donor.owner.is_none()
                || receiver.owner.is_none()
                || donor.owner == receiver.owner
            {
                continue;
            }
            for donor_edge in generated_contour_directed_edges(donor) {
                collect_generated_same_band_role_chord_candidates_for_edge(
                    &mut candidates,
                    donor_index,
                    receiver_index,
                    donor,
                    receiver,
                    constraints,
                    donor_edge.start,
                    donor_edge.end,
                );
                collect_generated_same_band_role_chord_candidates_for_edge(
                    &mut candidates,
                    donor_index,
                    receiver_index,
                    donor,
                    receiver,
                    constraints,
                    donor_edge.end,
                    donor_edge.start,
                );
            }
        }
    }
    candidates.sort_by_key(|(order, _)| *order);
    candidates.dedup_by_key(|(_, rewrite)| {
        (
            rewrite.donor_contour_index,
            rewrite.receiver_contour_index,
            rewrite.lower_key,
            rewrite.raised_key,
            rewrite.conflict_key,
        )
    });
    candidates.into_iter().map(|(_, rewrite)| rewrite).next()
}

fn generated_same_band_contour_pair_key(left: usize, right: usize) -> (usize, usize) {
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
}

fn collect_generated_same_band_role_chord_candidates_for_edge(
    candidates: &mut Vec<(
        (
            usize,
            usize,
            NodeRailPointKey,
            NodeRailPointKey,
            NodeRailPointKey,
        ),
        GeneratedSameBandRoleChordRewrite,
    )>,
    donor_contour_index: usize,
    receiver_contour_index: usize,
    donor: &NodeGeneratedContour,
    receiver: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    raised_key: NodeRailPointKey,
    conflict_key: NodeRailPointKey,
) {
    if generated_same_band_boundary_role_at_key(donor, constraints, raised_key)
        != Some(GeneratedSameBandBoundaryRole::RaisedSide)
        || generated_same_band_boundary_role_at_key(receiver, constraints, raised_key)
            != Some(GeneratedSameBandBoundaryRole::RaisedSide)
        || generated_same_band_boundary_role_at_key(donor, constraints, conflict_key)
            != Some(GeneratedSameBandBoundaryRole::RaisedSide)
        || generated_same_band_boundary_role_at_key(receiver, constraints, conflict_key)
            != Some(GeneratedSameBandBoundaryRole::LowerSide)
    {
        return;
    }

    for lower_key in generated_contour_neighbors(receiver, conflict_key) {
        if lower_key == raised_key
            || generated_triangle_double_area(lower_key, raised_key, conflict_key) == 0
            || generated_same_band_boundary_role_at_key(donor, constraints, lower_key)
                != Some(GeneratedSameBandBoundaryRole::LowerSide)
            || generated_same_band_boundary_role_at_key(receiver, constraints, lower_key)
                != Some(GeneratedSameBandBoundaryRole::LowerSide)
        {
            continue;
        }
        if !generated_contour_arc_contains_key(donor, raised_key, lower_key, conflict_key)
            || generated_contour_arc_contains_key(receiver, lower_key, raised_key, conflict_key)
        {
            continue;
        }
        candidates.push((
            (
                donor_contour_index,
                receiver_contour_index,
                lower_key,
                raised_key,
                conflict_key,
            ),
            GeneratedSameBandRoleChordRewrite {
                donor_contour_index,
                receiver_contour_index,
                lower_key,
                raised_key,
                conflict_key,
            },
        ));
    }
}

fn apply_generated_same_band_role_chord_rewrite(
    contours: &mut [NodeGeneratedContour],
    constraints: &mut [NodeRailConstraint],
    rewrite: GeneratedSameBandRoleChordRewrite,
) -> Result<(), NodeRailGenerationError> {
    if rewrite.donor_contour_index == rewrite.receiver_contour_index {
        return Ok(());
    }
    if rewrite.donor_contour_index < rewrite.receiver_contour_index {
        let (left, right) = contours.split_at_mut(rewrite.receiver_contour_index);
        apply_generated_same_band_role_chord_rewrite_to_contours(
            &mut left[rewrite.donor_contour_index],
            &mut right[0],
            constraints,
            rewrite,
        )
    } else {
        let (left, right) = contours.split_at_mut(rewrite.donor_contour_index);
        apply_generated_same_band_role_chord_rewrite_to_contours(
            &mut right[0],
            &mut left[rewrite.receiver_contour_index],
            constraints,
            rewrite,
        )
    }
}

fn apply_generated_same_band_role_chord_rewrite_to_contours(
    donor: &mut NodeGeneratedContour,
    receiver: &mut NodeGeneratedContour,
    constraints: &mut [NodeRailConstraint],
    rewrite: GeneratedSameBandRoleChordRewrite,
) -> Result<(), NodeRailGenerationError> {
    let mut donor_keys = generated_contour_keys(donor);
    if replace_generated_contour_arc_with_edge(
        &mut donor_keys,
        rewrite.raised_key,
        rewrite.lower_key,
        rewrite.conflict_key,
        true,
    ) {
        set_generated_contour_from_keys(donor, constraints, donor_keys)?;
    }

    let _ = receiver;
    Ok(())
}

fn insert_key_on_generated_contour(
    contours: &mut [NodeGeneratedContour],
    constraints: &mut [NodeRailConstraint],
    contour_index: usize,
    edge: GeneratedContourDirectedEdge,
    insert_key: NodeRailPointKey,
) -> Result<(), NodeRailGenerationError> {
    let Some(contour) = contours.get_mut(contour_index) else {
        return Ok(());
    };
    let mut keys = generated_contour_keys(contour);
    if insert_key_on_generated_contour_edge(&mut keys, edge.start, edge.end, insert_key) {
        set_generated_contour_from_keys(contour, constraints, keys)?;
    }
    Ok(())
}

fn insert_key_on_generated_constraints(
    constraints: &mut [NodeRailConstraint],
    owner: NodeBandOwner,
    edge: GeneratedContourDirectedEdge,
    insert_key: NodeRailPointKey,
) {
    for constraint in constraints {
        if !generated_constraint_applies_to_owner(constraint, owner) {
            continue;
        }
        insert_key_on_generated_constraint_segment(
            &mut constraint.points_xz,
            edge.start,
            edge.end,
            insert_key,
        );
    }
}

fn insert_key_on_generated_constraint_segment(
    points: &mut Vec<RoadVec2>,
    start_key: NodeRailPointKey,
    end_key: NodeRailPointKey,
    insert_key: NodeRailPointKey,
) -> bool {
    if points.len() < 2
        || insert_key == start_key
        || insert_key == end_key
        || points
            .iter()
            .copied()
            .any(|point| road_point_key(point) == insert_key)
    {
        return false;
    }
    for index in 0..points.len() - 1 {
        let current_key = road_point_key(points[index]);
        let next_key = road_point_key(points[index + 1]);
        if current_key == start_key && next_key == end_key {
            points.insert(index + 1, road_point_from_key(insert_key));
            return true;
        }
        if current_key == end_key && next_key == start_key {
            points.insert(index + 1, road_point_from_key(insert_key));
            return true;
        }
    }
    false
}

fn generated_same_band_role_crossing_join_rewrite_candidate(
    contours: &[NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
    resolved_transitions: &BTreeSet<GeneratedSameBandTransitionKey>,
) -> Option<GeneratedSameBandRoleJoinRewrite> {
    let mut candidates = Vec::new();
    for left_index in 0..contours.len() {
        for right_index in left_index + 1..contours.len() {
            let left = &contours[left_index];
            let right = &contours[right_index];
            let Some(kind) = generated_contour_band_kind(left) else {
                continue;
            };
            if generated_contour_band_kind(right) != Some(kind)
                || !generated_contour_supports_same_band_role(kind)
                || left.owner.is_none()
                || right.owner.is_none()
                || left.owner == right.owner
            {
                continue;
            }
            for edge in shared_generated_contour_edges(left, right) {
                let start_roles_match =
                    generated_same_band_endpoint_roles_match(left, right, constraints, edge.start);
                let end_roles_match =
                    generated_same_band_endpoint_roles_match(left, right, constraints, edge.end);
                let Some((equal_key, conflict_key)) = generated_one_matching_role_endpoint(
                    edge.start,
                    edge.end,
                    start_roles_match,
                    end_roles_match,
                ) else {
                    continue;
                };
                collect_generated_same_band_role_crossing_join_rewrite_candidates(
                    &mut candidates,
                    left_index,
                    right_index,
                    left,
                    right,
                    equal_key,
                    conflict_key,
                    constraints,
                    resolved_transitions,
                );
                collect_generated_same_band_role_crossing_join_rewrite_candidates(
                    &mut candidates,
                    right_index,
                    left_index,
                    right,
                    left,
                    equal_key,
                    conflict_key,
                    constraints,
                    resolved_transitions,
                );
            }
        }
    }

    candidates.sort_by_key(|(order, _)| *order);
    candidates.dedup_by_key(|(_, rewrite)| {
        (
            rewrite.donor_contour_index,
            rewrite.receiver_contour_index,
            rewrite.equal_key,
            rewrite.conflict_key,
            rewrite.candidate_key,
        )
    });
    candidates.into_iter().map(|(_, rewrite)| rewrite).next()
}

fn collect_generated_same_band_role_crossing_join_rewrite_candidates(
    candidates: &mut Vec<(
        GeneratedSameBandRoleJoinRewriteOrder,
        GeneratedSameBandRoleJoinRewrite,
    )>,
    donor_contour_index: usize,
    receiver_contour_index: usize,
    donor: &NodeGeneratedContour,
    receiver: &NodeGeneratedContour,
    equal_key: NodeRailPointKey,
    conflict_key: NodeRailPointKey,
    constraints: &[NodeRailConstraint],
    resolved_transitions: &BTreeSet<GeneratedSameBandTransitionKey>,
) {
    let Some((candidate_key, candidate_role)) = generated_role_matching_join_candidate(
        donor,
        receiver,
        constraints,
        equal_key,
        conflict_key,
    ) else {
        return;
    };
    if resolved_transitions.contains(&generated_same_band_transition_key(
        equal_key,
        conflict_key,
        candidate_key,
    )) {
        return;
    }
    candidates.push((
        GeneratedSameBandRoleJoinRewriteOrder {
            removed_role_priority: generated_removed_endpoint_priority(candidate_role),
            donor_contour_index,
            receiver_contour_index,
            removed_key: conflict_key,
            kept_key: equal_key,
            candidate_key,
        },
        GeneratedSameBandRoleJoinRewrite {
            donor_contour_index,
            receiver_contour_index,
            equal_key,
            conflict_key,
            candidate_key,
        },
    ));
}

fn generated_same_band_role_join_rewrite_candidate(
    contours: &[NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
    resolved_transitions: &BTreeSet<GeneratedSameBandTransitionKey>,
) -> Option<GeneratedSameBandRoleJoinRewrite> {
    let mut candidates = Vec::new();
    for left_index in 0..contours.len() {
        for right_index in left_index + 1..contours.len() {
            let left = &contours[left_index];
            let right = &contours[right_index];
            let Some(kind) = generated_contour_band_kind(left) else {
                continue;
            };
            if generated_contour_band_kind(right) != Some(kind)
                || !generated_contour_supports_same_band_role(kind)
                || left.owner.is_none()
                || right.owner.is_none()
                || left.owner == right.owner
            {
                continue;
            }
            for edge in shared_generated_contour_edges(left, right) {
                let Some(left_start_role) = generated_same_band_boundary_role_at_contour_vertex(
                    left,
                    constraints,
                    edge.start,
                ) else {
                    continue;
                };
                let Some(right_start_role) = generated_same_band_boundary_role_at_contour_vertex(
                    right,
                    constraints,
                    edge.start,
                ) else {
                    continue;
                };
                let Some(left_end_role) = generated_same_band_boundary_role_at_contour_vertex(
                    left,
                    constraints,
                    edge.end,
                ) else {
                    continue;
                };
                let Some(right_end_role) = generated_same_band_boundary_role_at_contour_vertex(
                    right,
                    constraints,
                    edge.end,
                ) else {
                    continue;
                };
                if left_start_role != right_start_role
                    || left_end_role != right_end_role
                    || left_start_role == left_end_role
                {
                    continue;
                }
                collect_generated_same_band_role_join_rewrite_candidates(
                    &mut candidates,
                    left_index,
                    right_index,
                    left,
                    right,
                    (edge.start, left_start_role),
                    (edge.end, left_end_role),
                    constraints,
                    resolved_transitions,
                );
                collect_generated_same_band_role_join_rewrite_candidates(
                    &mut candidates,
                    right_index,
                    left_index,
                    right,
                    left,
                    (edge.start, right_start_role),
                    (edge.end, right_end_role),
                    constraints,
                    resolved_transitions,
                );
            }
        }
    }

    candidates.sort_by_key(|(order, _)| *order);
    candidates.dedup_by_key(|(_, rewrite)| {
        (
            rewrite.donor_contour_index,
            rewrite.receiver_contour_index,
            rewrite.equal_key,
            rewrite.conflict_key,
            rewrite.candidate_key,
        )
    });
    candidates.into_iter().map(|(_, rewrite)| rewrite).next()
}

fn collect_generated_same_band_role_join_rewrite_candidates(
    candidates: &mut Vec<(
        GeneratedSameBandRoleJoinRewriteOrder,
        GeneratedSameBandRoleJoinRewrite,
    )>,
    donor_contour_index: usize,
    receiver_contour_index: usize,
    donor: &NodeGeneratedContour,
    receiver: &NodeGeneratedContour,
    start: (NodeRailPointKey, GeneratedSameBandBoundaryRole),
    end: (NodeRailPointKey, GeneratedSameBandBoundaryRole),
    constraints: &[NodeRailConstraint],
    resolved_transitions: &BTreeSet<GeneratedSameBandTransitionKey>,
) {
    for (removed, kept) in [(start, end), (end, start)] {
        let Some(candidate_key) = generated_same_role_endpoint_removal_candidate(
            donor,
            constraints,
            removed.0,
            kept.0,
            removed.1,
        ) else {
            continue;
        };
        if generated_same_band_boundary_role_at_key(receiver, constraints, candidate_key)
            != Some(removed.1)
        {
            continue;
        }
        if resolved_transitions.contains(&generated_same_band_transition_key(
            kept.0,
            removed.0,
            candidate_key,
        )) {
            continue;
        }
        candidates.push((
            GeneratedSameBandRoleJoinRewriteOrder {
                removed_role_priority: generated_removed_endpoint_priority(removed.1),
                donor_contour_index,
                receiver_contour_index,
                removed_key: removed.0,
                kept_key: kept.0,
                candidate_key,
            },
            GeneratedSameBandRoleJoinRewrite {
                donor_contour_index,
                receiver_contour_index,
                equal_key: kept.0,
                conflict_key: removed.0,
                candidate_key,
            },
        ));
    }
}

fn generated_same_band_transition_key(
    kept_key: NodeRailPointKey,
    side_a: NodeRailPointKey,
    side_b: NodeRailPointKey,
) -> GeneratedSameBandTransitionKey {
    if side_a <= side_b {
        GeneratedSameBandTransitionKey {
            kept_key,
            side_a,
            side_b,
        }
    } else {
        GeneratedSameBandTransitionKey {
            kept_key,
            side_a: side_b,
            side_b: side_a,
        }
    }
}

fn generated_same_band_role_point_rewrite_candidate(
    contours: &[NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
) -> Option<GeneratedSameBandRolePointRewrite> {
    let mut candidates = Vec::new();
    for left_index in 0..contours.len() {
        for right_index in left_index + 1..contours.len() {
            let left = &contours[left_index];
            let right = &contours[right_index];
            let Some(kind) = generated_contour_band_kind(left) else {
                continue;
            };
            if generated_contour_band_kind(right) != Some(kind)
                || !generated_contour_supports_same_band_role(kind)
                || left.owner.is_none()
                || right.owner.is_none()
                || left.owner == right.owner
            {
                continue;
            }
            let shared_edges = shared_generated_contour_edges(left, right);
            for key in shared_generated_contour_points(left, right) {
                if shared_edges
                    .iter()
                    .any(|edge| edge.start == key || edge.end == key)
                {
                    continue;
                }
                let Some(left_role) =
                    generated_same_band_boundary_role_at_contour_vertex(left, constraints, key)
                else {
                    continue;
                };
                let Some(right_role) =
                    generated_same_band_boundary_role_at_contour_vertex(right, constraints, key)
                else {
                    continue;
                };
                if left_role == right_role {
                    continue;
                }
                if left_role == GeneratedSameBandBoundaryRole::RaisedSide {
                    collect_generated_same_band_role_point_rewrite_candidate(
                        &mut candidates,
                        contours,
                        left_index,
                        left,
                        constraints,
                        key,
                        right_role,
                    );
                }
                if right_role == GeneratedSameBandBoundaryRole::RaisedSide {
                    collect_generated_same_band_role_point_rewrite_candidate(
                        &mut candidates,
                        contours,
                        right_index,
                        right,
                        constraints,
                        key,
                        left_role,
                    );
                }
            }
        }
    }

    candidates.sort_by_key(|(order, _)| *order);
    candidates.dedup_by_key(|(_, rewrite)| {
        (
            rewrite.donor_contour_index,
            rewrite.previous_key,
            rewrite.conflict_key,
            rewrite.next_key,
        )
    });
    candidates.into_iter().map(|(_, rewrite)| rewrite).next()
}

fn collect_generated_same_band_role_point_rewrite_candidate(
    candidates: &mut Vec<(
        GeneratedSameBandRolePointRewriteOrder,
        GeneratedSameBandRolePointRewrite,
    )>,
    contours: &[NodeGeneratedContour],
    donor_contour_index: usize,
    donor: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    conflict_key: NodeRailPointKey,
    opposite_role: GeneratedSameBandBoundaryRole,
) {
    let Some((previous_key, next_key)) =
        generated_removable_role_crossing_point(
            donor,
            constraints,
            conflict_key,
            opposite_role,
        )
    else {
        return;
    };
    if !generated_point_removal_preserves_same_band_topology(
        contours,
        donor_contour_index,
        previous_key,
        conflict_key,
        next_key,
    ) {
        return;
    }
    candidates.push((
        GeneratedSameBandRolePointRewriteOrder {
            removed_role_priority: generated_removed_endpoint_priority(
                GeneratedSameBandBoundaryRole::RaisedSide,
            ),
            donor_contour_index,
            conflict_key,
            previous_key,
            next_key,
        },
        GeneratedSameBandRolePointRewrite {
            donor_contour_index,
            previous_key,
            conflict_key,
            next_key,
        },
    ));
}

fn generated_point_removal_preserves_same_band_topology(
    contours: &[NodeGeneratedContour],
    donor_contour_index: usize,
    previous_key: NodeRailPointKey,
    removed_key: NodeRailPointKey,
    next_key: NodeRailPointKey,
) -> bool {
    if previous_key == next_key {
        return false;
    }
    let Some(donor) = contours.get(donor_contour_index) else {
        return false;
    };
    let Some(kind) = generated_contour_band_kind(donor) else {
        return false;
    };
    for (contour_index, contour) in contours.iter().enumerate() {
        if contour_index == donor_contour_index
            || generated_contour_band_kind(contour) != Some(kind)
        {
            continue;
        }
        for key in generated_contour_keys(contour) {
            if key != previous_key
                && key != removed_key
                && key != next_key
                && generated_point_key_lies_on_segment(key, previous_key, next_key)
            {
                return false;
            }
        }
        for edge in generated_contour_directed_edges(contour) {
            if edge.start == previous_key
                || edge.start == removed_key
                || edge.start == next_key
                || edge.end == previous_key
                || edge.end == removed_key
                || edge.end == next_key
            {
                continue;
            }
            if quantized_proper_segment_intersection(
                previous_key,
                next_key,
                edge.start,
                edge.end,
            )
            .is_some()
            {
                return false;
            }
        }
    }
    true
}

fn generated_removable_role_crossing_point(
    contour: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    conflict_key: NodeRailPointKey,
    opposite_role: GeneratedSameBandBoundaryRole,
) -> Option<(NodeRailPointKey, NodeRailPointKey)> {
    let keys = generated_contour_keys(contour);
    if keys.len() < 4 {
        return None;
    }
    let mut candidates = Vec::new();
    for index in 0..keys.len() {
        if keys[index] != conflict_key {
            continue;
        }
        let previous_key = keys[if index == 0 {
            keys.len() - 1
        } else {
            index - 1
        }];
        let next_key = keys[(index + 1) % keys.len()];
        if previous_key == next_key
            || generated_triangle_double_area(previous_key, conflict_key, next_key) == 0
        {
            continue;
        }
        let previous_role =
            generated_same_band_boundary_role_at_contour_vertex(contour, constraints, previous_key);
        let next_role =
            generated_same_band_boundary_role_at_contour_vertex(contour, constraints, next_key);
        if previous_role == Some(opposite_role) || next_role == Some(opposite_role) {
            candidates.push((previous_key, next_key));
        }
    }
    candidates.sort_unstable();
    candidates.dedup();
    candidates.into_iter().next()
}

fn generated_same_role_endpoint_removal_candidate(
    contour: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    removed_key: NodeRailPointKey,
    kept_key: NodeRailPointKey,
    expected_role: GeneratedSameBandBoundaryRole,
) -> Option<NodeRailPointKey> {
    if generated_same_band_boundary_role_at_contour_vertex(contour, constraints, removed_key)
        != Some(expected_role)
    {
        return None;
    }
    let keys = generated_contour_keys(contour);
    if keys.len() < 4 {
        return None;
    }
    let mut candidates = Vec::new();
    for index in 0..keys.len() {
        if keys[index] != removed_key {
            continue;
        }
        let previous_key = keys[if index == 0 {
            keys.len() - 1
        } else {
            index - 1
        }];
        let next_key = keys[(index + 1) % keys.len()];
        let candidate_key = if previous_key == kept_key {
            next_key
        } else if next_key == kept_key {
            previous_key
        } else {
            continue;
        };
        if candidate_key == removed_key
            || candidate_key == kept_key
            || generated_triangle_double_area(kept_key, removed_key, candidate_key) == 0
        {
            continue;
        }
        if generated_same_band_boundary_role_at_contour_vertex(contour, constraints, candidate_key)
            == Some(expected_role)
        {
            candidates.push(candidate_key);
        }
    }
    candidates.sort_unstable();
    candidates.dedup();
    candidates.into_iter().next()
}

fn apply_generated_same_band_role_join_rewrite(
    contours: &mut [NodeGeneratedContour],
    constraints: &mut [NodeRailConstraint],
    rewrite: GeneratedSameBandRoleJoinRewrite,
) -> Result<(), NodeRailGenerationError> {
    if rewrite.donor_contour_index == rewrite.receiver_contour_index {
        return Ok(());
    }
    if rewrite.donor_contour_index < rewrite.receiver_contour_index {
        let (left, right) = contours.split_at_mut(rewrite.receiver_contour_index);
        apply_generated_same_band_role_join_rewrite_to_contours(
            &mut left[rewrite.donor_contour_index],
            &mut right[0],
            constraints,
            rewrite,
        )
    } else {
        let (left, right) = contours.split_at_mut(rewrite.donor_contour_index);
        apply_generated_same_band_role_join_rewrite_to_contours(
            &mut right[0],
            &mut left[rewrite.receiver_contour_index],
            constraints,
            rewrite,
        )
    }
}

fn apply_generated_same_band_role_join_rewrite_to_contours(
    donor: &mut NodeGeneratedContour,
    receiver: &mut NodeGeneratedContour,
    constraints: &mut [NodeRailConstraint],
    rewrite: GeneratedSameBandRoleJoinRewrite,
) -> Result<(), NodeRailGenerationError> {
    let mut donor_keys = generated_contour_keys(donor);
    if remove_middle_key_from_generated_contour(
        &mut donor_keys,
        rewrite.equal_key,
        rewrite.conflict_key,
        rewrite.candidate_key,
    ) {
        set_generated_contour_from_keys(donor, constraints, donor_keys)?;
    }

    let mut receiver_keys = generated_contour_keys(receiver);
    if insert_key_on_generated_contour_edge(
        &mut receiver_keys,
        rewrite.conflict_key,
        rewrite.equal_key,
        rewrite.candidate_key,
    ) || insert_key_on_generated_contour_edge(
        &mut receiver_keys,
        rewrite.equal_key,
        rewrite.conflict_key,
        rewrite.candidate_key,
    ) {
        set_generated_contour_from_keys(receiver, constraints, receiver_keys)?;
    }
    Ok(())
}

fn apply_generated_same_band_role_point_rewrite(
    contours: &mut [NodeGeneratedContour],
    constraints: &mut [NodeRailConstraint],
    rewrite: GeneratedSameBandRolePointRewrite,
) -> Result<(), NodeRailGenerationError> {
    let Some(donor) = contours.get_mut(rewrite.donor_contour_index) else {
        return Ok(());
    };
    let owner = donor.owner;
    let mut donor_keys = generated_contour_keys(donor);
    if remove_middle_key_from_generated_contour(
        &mut donor_keys,
        rewrite.previous_key,
        rewrite.conflict_key,
        rewrite.next_key,
    ) {
        if let Some(owner) = owner {
            remove_key_from_generated_constraints(
                constraints,
                owner,
                rewrite.previous_key,
                rewrite.conflict_key,
                rewrite.next_key,
            );
        }
        set_generated_contour_from_keys(donor, constraints, donor_keys)?;
    }
    Ok(())
}

fn remove_key_from_generated_constraints(
    constraints: &mut [NodeRailConstraint],
    owner: NodeBandOwner,
    previous_key: NodeRailPointKey,
    removed_key: NodeRailPointKey,
    next_key: NodeRailPointKey,
) {
    for constraint in constraints {
        if generated_constraint_applies_to_owner(constraint, owner) {
            remove_middle_key_from_generated_constraint(
                &mut constraint.points_xz,
                previous_key,
                removed_key,
                next_key,
            );
        }
    }
}

fn remove_middle_key_from_generated_constraint(
    points: &mut Vec<RoadVec2>,
    previous_key: NodeRailPointKey,
    removed_key: NodeRailPointKey,
    next_key: NodeRailPointKey,
) -> bool {
    if points.len() < 3 {
        return false;
    }
    for index in 1..points.len() - 1 {
        if road_point_key(points[index]) != removed_key {
            continue;
        }
        let prev = road_point_key(points[index - 1]);
        let next = road_point_key(points[index + 1]);
        if (prev == previous_key && next == next_key)
            || (prev == next_key && next == previous_key)
        {
            points.remove(index);
            return true;
        }
    }
    false
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

fn generated_contour_neighbors(
    contour: &NodeGeneratedContour,
    key: NodeRailPointKey,
) -> Vec<NodeRailPointKey> {
    let keys = generated_contour_keys(contour);
    if keys.len() < 2 {
        return Vec::new();
    }
    let mut neighbors = Vec::new();
    for index in 0..keys.len() {
        if keys[index] != key {
            continue;
        }
        neighbors.push(keys[if index == 0 {
            keys.len() - 1
        } else {
            index - 1
        }]);
        neighbors.push(keys[(index + 1) % keys.len()]);
    }
    neighbors.sort_unstable();
    neighbors.dedup();
    neighbors
}

fn generated_contour_arc_contains_key(
    contour: &NodeGeneratedContour,
    start_key: NodeRailPointKey,
    end_key: NodeRailPointKey,
    contained_key: NodeRailPointKey,
) -> bool {
    let keys = generated_contour_keys(contour);
    generated_key_arc(&keys, start_key, end_key)
        .is_some_and(|arc| arc.iter().copied().any(|key| key == contained_key))
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

fn generated_contour_supports_same_band_noding(kind: RoadSurfaceBandKind) -> bool {
    generated_contour_supports_same_band_contact(kind)
}

fn generated_contour_supports_same_band_contact(kind: RoadSurfaceBandKind) -> bool {
    matches!(
        kind,
        RoadSurfaceBandKind::CurbOrShoulder | RoadSurfaceBandKind::Sidewalk
    )
}

fn generated_same_band_endpoint_roles_match(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    key: NodeRailPointKey,
) -> Option<bool> {
    let left_role = generated_same_band_boundary_role_at_contour_vertex(left, constraints, key);
    let right_role = generated_same_band_boundary_role_at_contour_vertex(right, constraints, key);
    match (left_role, right_role) {
        (Some(left_role), Some(right_role)) => Some(left_role == right_role),
        (Some(_), None) | (None, Some(_)) => Some(false),
        (None, None) => None,
    }
}

fn generated_one_matching_role_endpoint(
    start: NodeRailPointKey,
    end: NodeRailPointKey,
    start_matches: Option<bool>,
    end_matches: Option<bool>,
) -> Option<(NodeRailPointKey, NodeRailPointKey)> {
    match (start_matches, end_matches) {
        (Some(true), Some(false)) => Some((start, end)),
        (Some(false), Some(true)) => Some((end, start)),
        _ => None,
    }
}

fn generated_role_matching_join_candidate(
    donor: &NodeGeneratedContour,
    receiver: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    equal_key: NodeRailPointKey,
    conflict_key: NodeRailPointKey,
) -> Option<(NodeRailPointKey, GeneratedSameBandBoundaryRole)> {
    let receiver_conflict_role =
        generated_same_band_boundary_role_at_key(receiver, constraints, conflict_key)?;
    let keys = generated_contour_keys(donor);
    if !generated_edge_exists_in_keys(&keys, equal_key, conflict_key) {
        return None;
    }
    let mut candidates = Vec::new();
    for index in 0..keys.len() {
        if keys[index] != conflict_key {
            continue;
        }
        let previous_key = keys[if index == 0 {
            keys.len() - 1
        } else {
            index - 1
        }];
        let next_key = keys[(index + 1) % keys.len()];
        for candidate_key in [previous_key, next_key] {
            if candidate_key == equal_key
                || candidate_key == conflict_key
                || generated_triangle_double_area(equal_key, conflict_key, candidate_key) == 0
            {
                continue;
            }
            if generated_same_band_boundary_role_at_contour_vertex(
                donor,
                constraints,
                candidate_key,
            ) == Some(receiver_conflict_role)
            {
                candidates.push(candidate_key);
            }
        }
    }
    candidates.sort_unstable();
    candidates.dedup();
    candidates
        .into_iter()
        .next()
        .map(|candidate_key| (candidate_key, receiver_conflict_role))
}

fn generated_edge_exists_in_keys(
    keys: &[NodeRailPointKey],
    start: NodeRailPointKey,
    end: NodeRailPointKey,
) -> bool {
    keys.iter().enumerate().any(|(index, key)| {
        let next = keys[(index + 1) % keys.len()];
        (*key == start && next == end) || (*key == end && next == start)
    })
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
        if let Some(role) = generated_boundary_role_from_constraint_kind(kind, constraint.kind) {
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
        match generated_boundary_role_from_constraint_kind(kind, constraint.kind) {
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

fn generated_boundary_role_from_constraint_kind(
    contour_kind: RoadSurfaceBandKind,
    kind: NodeRailConstraintKind,
) -> Option<GeneratedSameBandBoundaryRole> {
    match contour_kind {
        RoadSurfaceBandKind::CurbOrShoulder => match kind {
            NodeRailConstraintKind::AsphaltCurbContact
            | NodeRailConstraintKind::AsphaltBoundary { .. } => {
                Some(GeneratedSameBandBoundaryRole::LowerSide)
            }
            NodeRailConstraintKind::CurbSidewalkContact => {
                Some(GeneratedSameBandBoundaryRole::RaisedSide)
            }
            NodeRailConstraintKind::FullRoadbedContour
            | NodeRailConstraintKind::BandContour { .. }
            | NodeRailConstraintKind::SpanHandoff { .. }
            | NodeRailConstraintKind::FootprintSeam { .. }
            | NodeRailConstraintKind::BandBoundary { .. } => None,
        },
        RoadSurfaceBandKind::Sidewalk => match kind {
            NodeRailConstraintKind::CurbSidewalkContact => {
                Some(GeneratedSameBandBoundaryRole::LowerSide)
            }
            NodeRailConstraintKind::FootprintSeam {
                adjacent_kind: RoadSurfaceBandKind::Sidewalk,
            } => Some(GeneratedSameBandBoundaryRole::RaisedSide),
            NodeRailConstraintKind::FullRoadbedContour
            | NodeRailConstraintKind::BandContour { .. }
            | NodeRailConstraintKind::SpanHandoff { .. }
            | NodeRailConstraintKind::AsphaltBoundary { .. }
            | NodeRailConstraintKind::AsphaltCurbContact
            | NodeRailConstraintKind::FootprintSeam { .. }
            | NodeRailConstraintKind::BandBoundary { .. } => None,
        },
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
        NodeRailConstraintKind::AsphaltCurbContact => {
            is_carriageway(owner.kind()) || is_curb_or_shoulder(owner.kind())
        }
        NodeRailConstraintKind::CurbSidewalkContact => {
            is_curb_or_shoulder(owner.kind()) || is_sidewalk(owner.kind())
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
    if px * dz - pz * dx != 0 {
        return false;
    }
    let inside_x = if start.0 == end.0 {
        point.0 == start.0
    } else {
        point.0 > start.0.min(end.0) && point.0 < start.0.max(end.0)
    };
    let inside_z = if start.1 == end.1 {
        point.1 == start.1
    } else {
        point.1 > start.1.min(end.1) && point.1 < start.1.max(end.1)
    };
    inside_x && inside_z
}

fn remove_middle_key_from_generated_contour(
    keys: &mut Vec<NodeRailPointKey>,
    first_key: NodeRailPointKey,
    middle_key: NodeRailPointKey,
    third_key: NodeRailPointKey,
) -> bool {
    if keys.len() < 3 {
        return false;
    }
    for index in 0..keys.len() {
        if keys[index] != middle_key {
            continue;
        }
        let previous_key = keys[if index == 0 {
            keys.len() - 1
        } else {
            index - 1
        }];
        let next_key = keys[(index + 1) % keys.len()];
        if (previous_key == first_key && next_key == third_key)
            || (previous_key == third_key && next_key == first_key)
        {
            keys.remove(index);
            remove_generated_contour_spikes(keys);
            return true;
        }
    }
    false
}

fn replace_generated_contour_arc_with_edge(
    keys: &mut Vec<NodeRailPointKey>,
    start_key: NodeRailPointKey,
    end_key: NodeRailPointKey,
    contained_key: NodeRailPointKey,
    remove_containing_arc: bool,
) -> bool {
    if keys.len() < 4 || start_key == end_key {
        return false;
    }
    let Some(forward_arc) = generated_key_arc(keys, start_key, end_key) else {
        return false;
    };
    let Some(reverse_arc) = generated_key_arc(keys, end_key, start_key) else {
        return false;
    };
    let forward_contains = forward_arc.iter().copied().any(|key| key == contained_key);
    let reverse_contains = reverse_arc.iter().copied().any(|key| key == contained_key);
    if forward_contains == reverse_contains {
        return false;
    }
    let remove_forward = if remove_containing_arc {
        forward_contains
    } else {
        !forward_contains
    };
    let kept_arc = if remove_forward {
        let mut kept = reverse_arc;
        kept.reverse();
        kept
    } else {
        forward_arc
    };
    if kept_arc.len() < 3 {
        return false;
    }
    *keys = kept_arc;
    remove_generated_contour_spikes(keys);
    true
}

fn generated_key_arc(
    keys: &[NodeRailPointKey],
    start_key: NodeRailPointKey,
    end_key: NodeRailPointKey,
) -> Option<Vec<NodeRailPointKey>> {
    if keys.len() < 2 {
        return None;
    }
    let start_index = keys.iter().position(|key| *key == start_key)?;
    let mut arc = Vec::new();
    let mut index = start_index;
    for _ in 0..=keys.len() {
        let key = keys[index];
        arc.push(key);
        if key == end_key && index != start_index {
            return Some(arc);
        }
        index = (index + 1) % keys.len();
    }
    None
}

fn insert_key_on_generated_contour_edge(
    keys: &mut Vec<NodeRailPointKey>,
    start_key: NodeRailPointKey,
    end_key: NodeRailPointKey,
    insert_key: NodeRailPointKey,
) -> bool {
    if keys.len() < 2 {
        return false;
    }
    for index in 0..keys.len() {
        let next = if index + 1 == keys.len() {
            0
        } else {
            index + 1
        };
        if keys[index] == start_key && keys[next] == end_key {
            keys.insert(next, insert_key);
            remove_generated_contour_spikes(keys);
            return true;
        }
    }
    false
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

fn generated_removed_endpoint_priority(role: GeneratedSameBandBoundaryRole) -> u8 {
    match role {
        GeneratedSameBandBoundaryRole::LowerSide => 0,
        GeneratedSameBandBoundaryRole::RaisedSide => 1,
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

fn owners_by_mouth(input: &NodeArrangementInput) -> Vec<MouthOwners> {
    let mut next_owner_index = 0usize;
    input
        .mouths
        .iter()
        .map(|mouth| {
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
            let terminal_end_band_owners = mouth
                .terminal_end_bands
                .iter()
                .map(|end_band| {
                    let key = (end_band.band_kind, end_band.source_band_index);
                    if let Some(owner) = terminal_owner_by_source.get(&key).copied() {
                        owner
                    } else {
                        let owner = NodeBandOwner::new(end_band.band_kind, next_owner_index);
                        next_owner_index += 1;
                        terminal_owner_by_source.insert(key, owner);
                        owner
                    }
                })
                .collect();
            MouthOwners {
                band_owners,
                terminal_end_band_owners,
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
            if is_carriageway(left_kind) && is_curb_or_shoulder(right_kind)
                || is_curb_or_shoulder(left_kind) && is_carriageway(right_kind)
            {
                NodeRailConstraintKind::AsphaltCurbContact
            } else if is_carriageway(left_kind) || is_carriageway(right_kind) {
                let adjacent_kind = if is_carriageway(left_kind) {
                    right_kind
                } else {
                    left_kind
                };
                NodeRailConstraintKind::AsphaltBoundary { adjacent_kind }
            } else if is_curb_or_shoulder(left_kind) && is_sidewalk(right_kind)
                || is_sidewalk(left_kind) && is_curb_or_shoulder(right_kind)
            {
                NodeRailConstraintKind::CurbSidewalkContact
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

fn is_curb_or_shoulder(kind: RoadSurfaceBandKind) -> bool {
    kind == RoadSurfaceBandKind::CurbOrShoulder
}

fn is_sidewalk(kind: RoadSurfaceBandKind) -> bool {
    kind == RoadSurfaceBandKind::Sidewalk
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

    fn input_with_endpoint_x(endpoint_x: f32) -> NodeArrangementInput {
        let mouth = OrderedIncidentPieceMouth {
            profile: profile(10.0),
            endpoint_profile: profile(endpoint_x),
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

    #[test]
    fn generates_backend_contours_and_constraints_from_solved_mouth_input() {
        let contours =
            NodeRailContourSet::from_input(&input_with_endpoint_x(0.0)).expect("valid contours");

        assert_eq!(contours.node_id, 42);
        assert_eq!(
            contours.piece_kind,
            RoadSurfaceVisualNodePieceKind::JunctionN
        );
        assert_eq!(contours.contours.len(), 5);
        assert_eq!(contours.constraints.len(), 14);
        assert_eq!(
            contours.contours[0].kind,
            NodeGeneratedContourKind::FullRoadbed
        );
        assert_eq!(contours.contours[0].points_xz.len(), 4);
        assert!(contours.contours.iter().any(|contour| contour.kind
            == NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::Carriageway
            }));
        assert!(
            contours
                .constraints
                .iter()
                .any(|constraint| constraint.kind == NodeRailConstraintKind::AsphaltCurbContact)
        );
        assert!(
            contours
                .constraints
                .iter()
                .any(|constraint| constraint.kind == NodeRailConstraintKind::CurbSidewalkContact)
        );
        assert_eq!(
            contours.constraints[0].kind,
            NodeRailConstraintKind::FullRoadbedContour
        );
        assert_eq!(contours.constraints[0].constraint_index, 0);
    }

    #[test]
    fn generated_curb_transition_ownership_is_resolved_before_boolean() {
        let donor_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 0);
        let receiver_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let raised_shared = RoadVec2::new(0.0, 1.0);
        let lower_shared = RoadVec2::new(0.0, 0.0);
        let donor_lower = RoadVec2::new(1.0, 0.0);
        let donor_raised = RoadVec2::new(1.0, 1.0);
        let receiver_raised = RoadVec2::new(-1.0, 1.0);
        let receiver_lower = RoadVec2::new(-1.0, 0.0);
        let mut contours = vec![
            test_generated_band_contour(
                0,
                donor_owner,
                vec![raised_shared, lower_shared, donor_lower, donor_raised],
            ),
            test_generated_band_contour(
                1,
                receiver_owner,
                vec![lower_shared, raised_shared, receiver_raised, receiver_lower],
            ),
        ];
        let mut constraints = vec![
            test_role_constraint(
                0,
                NodeRailConstraintKind::BandContour {
                    kind: RoadSurfaceBandKind::CurbOrShoulder,
                },
                donor_owner,
                vec![raised_shared, lower_shared, donor_lower, donor_raised],
            ),
            test_role_constraint(
                1,
                NodeRailConstraintKind::BandContour {
                    kind: RoadSurfaceBandKind::CurbOrShoulder,
                },
                receiver_owner,
                vec![lower_shared, raised_shared, receiver_raised, receiver_lower],
            ),
            test_role_constraint(
                2,
                NodeRailConstraintKind::AsphaltCurbContact,
                donor_owner,
                vec![lower_shared, donor_lower],
            ),
            test_role_constraint(
                3,
                NodeRailConstraintKind::CurbSidewalkContact,
                donor_owner,
                vec![donor_raised, raised_shared],
            ),
            test_role_constraint(
                4,
                NodeRailConstraintKind::AsphaltCurbContact,
                receiver_owner,
                vec![receiver_lower, lower_shared],
            ),
            test_role_constraint(
                5,
                NodeRailConstraintKind::CurbSidewalkContact,
                receiver_owner,
                vec![raised_shared, receiver_raised],
            ),
            test_role_constraint(
                6,
                NodeRailConstraintKind::AsphaltCurbContact,
                receiver_owner,
                vec![lower_shared, donor_lower],
            ),
        ];

        let rewrite = generated_same_band_role_join_rewrite_candidate(
            &contours,
            &constraints,
            &BTreeSet::new(),
        )
        .expect("generated same-band rewrite should be available before boolean");
        assert_eq!(rewrite.donor_contour_index, 0);
        assert_eq!(rewrite.receiver_contour_index, 1);
        resolve_generated_same_band_curb_transition_ownership(&mut contours, &mut constraints)
            .expect("same-role generated curb transition should resolve before boolean ownership");

        let raised_key = road_point_key(raised_shared);
        let lower_key = road_point_key(lower_shared);
        let donor_lower_key = road_point_key(donor_lower);
        assert!(
            !generated_contour_keys(&contours[0]).contains(&lower_key),
            "donor contour must not carry the receiver-owned lower transition endpoint into boolean"
        );
        assert!(
            generated_contour_keys(&contours[1]).contains(&donor_lower_key),
            "receiver contour must carry the inserted transition vertex before boolean"
        );
        let old_edge = GeneratedContourEdgeKey::new(lower_key, raised_key);
        let new_edge = GeneratedContourEdgeKey::new(donor_lower_key, raised_key);
        let shared_edges = shared_generated_contour_edges(&contours[0], &contours[1]);
        assert!(
            !shared_edges.contains(&old_edge),
            "the generated contour graph must not retain the old lower-to-raised shared edge"
        );
        assert!(
            shared_edges.contains(&new_edge),
            "the generated contour graph must expose the new same-band transition edge"
        );
        assert_eq!(
            constraints[0].points_xz, contours[0].points_xz,
            "donor band contour constraint must be updated with the canonicalized contour"
        );
        assert_eq!(
            constraints[1].points_xz, contours[1].points_xz,
            "receiver band contour constraint must be updated with the canonicalized contour"
        );
    }

    fn test_generated_band_contour(
        source_band_index: usize,
        owner: NodeBandOwner,
        points: Vec<RoadVec2>,
    ) -> NodeGeneratedContour {
        let kind = NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        };
        let backend_polyline = cleaned_closed_contour(kind, 0, Some(source_band_index), points)
            .expect("test contour should be non-degenerate");
        let points_xz = polyline_to_road_points(&backend_polyline);
        NodeGeneratedContour {
            kind,
            source_mouth_order_index: 0,
            source_band_index: Some(source_band_index),
            owner: Some(owner),
            claim_priority: NodeGeneratedContourClaimPriority::JoinOrCap,
            points_xz,
            backend_polyline,
        }
    }

    fn test_role_constraint(
        constraint_index: usize,
        kind: NodeRailConstraintKind,
        owner: NodeBandOwner,
        points_xz: Vec<RoadVec2>,
    ) -> NodeRailConstraint {
        NodeRailConstraint {
            constraint_index,
            kind,
            source_mouth_order_index: 0,
            source_band_index: Some(owner.owner_index()),
            source_boundary_index: None,
            owner: Some(owner),
            opposite_owner: None,
            points_xz,
        }
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
