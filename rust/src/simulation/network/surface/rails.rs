//! Library-backed rail and contour generation for canonical node arrangements.

#![allow(dead_code)]

use super::arrangement::NodeBandOwner;
use super::backend::{
    ROAD_OVERLAY_COORDINATE_SCALE, RoadPolyline, RoadVec2, RoadVec3, polyline_to_road_points,
    road_points_to_polyline,
};
use super::input::{
    NodeArrangementInput, NodeInputBandInterval, NodeInputBoundaryRailRole, NodeInputMouth,
    NodeInputProfileRail, NodeInputTerminalEndBand,
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
    MouthBand,
    JoinOrCap,
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
        resolve_generated_same_band_curb_transition_ownership(&mut contours, &mut constraints)?;

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
    for _ in 0..max_passes {
        let Some(rewrite) = generated_same_band_role_join_rewrite_candidate(
            contours,
            constraints,
            &resolved_transitions,
        ) else {
            return Ok(());
        };
        resolved_transitions.insert(generated_same_band_transition_key(
            rewrite.equal_key,
            rewrite.conflict_key,
            rewrite.candidate_key,
        ));
        apply_generated_same_band_role_join_rewrite(contours, constraints, rewrite)?;
    }
    Ok(())
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
            if generated_contour_band_kind(left) != Some(RoadSurfaceBandKind::CurbOrShoulder)
                || generated_contour_band_kind(right) != Some(RoadSurfaceBandKind::CurbOrShoulder)
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
    if generated_contour_band_kind(contour) != Some(RoadSurfaceBandKind::CurbOrShoulder) {
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
    for constraint in constraints
        .iter()
        .filter(|constraint| generated_constraint_applies_to_owner(constraint, owner))
        .filter(|constraint| generated_constraint_contains_key_segment(constraint, start, end))
    {
        if let Some(role) = generated_boundary_role_from_constraint_kind(constraint.kind) {
            roles.push(role);
        }
    }
}

fn generated_same_band_boundary_role_at_key(
    contour: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    key: NodeRailPointKey,
) -> Option<GeneratedSameBandBoundaryRole> {
    if generated_contour_band_kind(contour) != Some(RoadSurfaceBandKind::CurbOrShoulder) {
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
        match generated_boundary_role_from_constraint_kind(constraint.kind) {
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
    kind: NodeRailConstraintKind,
) -> Option<GeneratedSameBandBoundaryRole> {
    match kind {
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
            let band_owners = mouth
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
            "receiver contour must carry the transferred transition vertex before boolean"
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
