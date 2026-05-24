//! Same-band and contact-edge constraint emission for generated rails.

use super::super::authority::{
    GeneratedMaterialPointContactAuthority, generated_contact_edge_source_authority,
    generated_contact_point_has_explicit_roles,
    generated_exact_owner_pair_contact_authority_at_point,
};
use super::super::*;
use std::collections::BTreeSet;

type SameMaterialHeightSplitConstraint = (
    NodeBandOwner,
    NodeBandOwner,
    NodeRailPointKey,
    NodeRailPointKey,
    usize,
    Option<usize>,
);

pub(in crate::simulation::network::surface::node::rails) fn append_generated_same_band_contact_constraints(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    contours: &[NodeGeneratedContour],
    source_constraint_count: usize,
    constraints: &mut Vec<NodeRailConstraint>,
) {
    let mut contact_edges = BTreeSet::<GeneratedSameBandContactConstraint>::new();
    let mut same_material_height_splits = BTreeSet::<SameMaterialHeightSplitConstraint>::new();
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
            if kind == right_kind {
                collect_same_material_height_splits(
                    left,
                    right,
                    left_owner,
                    right_owner,
                    &mut same_material_height_splits,
                );
                continue;
            }
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
    let source_constraints = super::source_authority_constraints_for_generated_contacts(
        constraints,
        source_constraint_count,
    );
    collect_source_authorized_raised_step_contacts(
        piece_kind,
        contours,
        &source_constraints,
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
    append_same_material_height_split_constraints(constraints, same_material_height_splits);
}

fn collect_same_material_height_splits(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    left_owner: NodeBandOwner,
    right_owner: NodeBandOwner,
    contacts: &mut BTreeSet<SameMaterialHeightSplitConstraint>,
) {
    let mut edges = BTreeSet::new();
    for edge in shared_generated_contour_edges(left, right) {
        insert_same_material_height_split(
            contacts,
            left_owner,
            right_owner,
            edge.start,
            edge.end,
            left.source_mouth_order_index,
            left.source_band_index,
        );
        edges.insert(edge);
    }
    for edge in generated_contact_edges_inside_contour(left, right) {
        insert_same_material_height_split(
            contacts,
            left_owner,
            right_owner,
            edge.start,
            edge.end,
            left.source_mouth_order_index,
            left.source_band_index,
        );
        edges.insert(edge);
    }
    for edge in generated_contact_edges_inside_contour(right, left) {
        insert_same_material_height_split(
            contacts,
            left_owner,
            right_owner,
            edge.start,
            edge.end,
            right.source_mouth_order_index,
            right.source_band_index,
        );
        edges.insert(edge);
    }
    for edge in generated_contact_edges_from_overlay_intersection(left, right) {
        let (source_mouth_order_index, source_band_index) =
            same_material_height_split_source_name(left, right, left_owner, right_owner);
        insert_same_material_height_split(
            contacts,
            left_owner,
            right_owner,
            edge.start,
            edge.end,
            source_mouth_order_index,
            source_band_index,
        );
        edges.insert(edge);
    }
    let shared_edge_points = edges
        .iter()
        .flat_map(|edge| [edge.start, edge.end])
        .collect::<BTreeSet<_>>();
    let mut points = shared_generated_contour_points(left, right);
    points.extend(generated_contact_points_from_contour_intersections(
        left, right,
    ));
    points.sort_unstable();
    points.dedup();
    for point in points {
        if shared_edge_points.contains(&point) {
            continue;
        }
        let (source_mouth_order_index, source_band_index) =
            same_material_height_split_source_name(left, right, left_owner, right_owner);
        insert_same_material_height_split(
            contacts,
            left_owner,
            right_owner,
            point,
            point,
            source_mouth_order_index,
            source_band_index,
        );
    }
}

fn same_material_height_split_source_name(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    left_owner: NodeBandOwner,
    right_owner: NodeBandOwner,
) -> (usize, Option<usize>) {
    if left_owner <= right_owner {
        (left.source_mouth_order_index, left.source_band_index)
    } else {
        (right.source_mouth_order_index, right.source_band_index)
    }
}

fn insert_same_material_height_split(
    contacts: &mut BTreeSet<SameMaterialHeightSplitConstraint>,
    left_owner: NodeBandOwner,
    right_owner: NodeBandOwner,
    start: NodeRailPointKey,
    end: NodeRailPointKey,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
) {
    let (owner, opposite_owner) = if left_owner <= right_owner {
        (left_owner, right_owner)
    } else {
        (right_owner, left_owner)
    };
    let (start, end) = if end < start {
        (end, start)
    } else {
        (start, end)
    };
    contacts.insert((
        owner,
        opposite_owner,
        start,
        end,
        source_mouth_order_index,
        source_band_index,
    ));
}

fn append_same_material_height_split_constraints(
    constraints: &mut Vec<NodeRailConstraint>,
    contacts: BTreeSet<SameMaterialHeightSplitConstraint>,
) {
    for (owner, opposite_owner, start, end, source_mouth_order_index, source_band_index) in contacts
    {
        if constraints.iter().any(|constraint| {
            constraint.kind == NodeRailConstraintKind::RaisedStepContact
                && owners_match_unordered(
                    constraint.owner,
                    constraint.opposite_owner,
                    owner,
                    opposite_owner,
                )
                && constraint.points_xz.len() == 2
                && GeneratedContourEdgeKey::new(
                    road_point_key(constraint.points_xz[0]),
                    road_point_key(constraint.points_xz[1]),
                ) == GeneratedContourEdgeKey::new(start, end)
        }) {
            continue;
        }
        constraints.push(NodeRailConstraint {
            constraint_index: constraints.len(),
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index,
            source_band_index,
            source_boundary_index: None,
            owner: Some(owner),
            opposite_owner: Some(opposite_owner),
            points_xz: vec![road_point_from_key(start), road_point_from_key(end)],
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
