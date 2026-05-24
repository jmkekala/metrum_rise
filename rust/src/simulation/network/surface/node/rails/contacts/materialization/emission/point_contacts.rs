//! Point-contact constraint emission for generated rail contact materialization.

use super::super::authority::{
    generated_material_authority_points_on_counterpart_contour,
    generated_material_point_contact_authority,
};
use super::super::*;
use std::collections::BTreeSet;

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

pub(in crate::simulation::network::surface::node::rails) fn append_source_authorized_raised_step_point_contacts(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    contours: &[NodeGeneratedContour],
    source_constraint_count: usize,
    constraints: &mut Vec<NodeRailConstraint>,
) {
    let mut existing = constraints
        .iter()
        .filter_map(generated_same_band_contact_constraint_key)
        .collect::<BTreeSet<_>>();
    let mut contacts = BTreeSet::<GeneratedSameBandContactConstraint>::new();
    let source_constraints = super::source_authority_constraints_for_generated_contacts(
        constraints,
        source_constraint_count,
    );
    collect_source_authorized_raised_step_contacts(
        piece_kind,
        contours,
        &source_constraints,
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

pub(in crate::simulation::network::surface::node::rails) fn append_generated_material_point_contact_constraints(
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
                ) else {
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
