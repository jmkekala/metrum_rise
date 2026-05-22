//! Same-band and contact-edge constraint emission for generated rails.

use super::super::authority::{
    GeneratedMaterialPointContactAuthority, generated_contact_edge_source_authority,
    generated_contact_point_has_explicit_roles,
    generated_exact_owner_pair_contact_authority_at_point,
};
use super::super::*;
use std::collections::BTreeSet;

pub(in crate::simulation::network::surface::node::rails) fn append_generated_same_band_contact_constraints(
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
