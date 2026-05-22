//! Source-constraint noding for generated rail contacts.

use super::super::geometry::generated_contour_boundary_contains_key;
use super::super::source_authority::{
    GeneratedSameBandContactConstraint, generated_contact_kind_from_constraint,
    generated_same_band_contact_constraint,
};
use super::super::{
    NodeGeneratedContour, NodeGeneratedContourClaimPriority, NodeRailConstraint, NodeRailPointKey,
    generated_constraint_directed_edges, generated_contour_directed_edges, generated_contour_keys,
    generated_point_key_lies_on_segment, owners_match_unordered,
    quantized_proper_segment_intersection,
};
use super::ContactInsertionsByIndex;
use super::insertion::insert_keys_on_generated_source_constraints;

pub(in crate::simulation::network::surface::node::rails) fn node_generated_contact_source_constraints(
    contours: &[NodeGeneratedContour],
    constraints: &mut [NodeRailConstraint],
    source_constraint_count: usize,
) {
    let source_constraint_count = source_constraint_count.min(constraints.len());
    if source_constraint_count == 0 {
        return;
    }
    let insertions = generated_contact_source_constraint_noding_candidates(
        contours,
        &constraints[..source_constraint_count],
    );
    insert_keys_on_generated_source_constraints(
        &mut constraints[..source_constraint_count],
        insertions,
    );
}

fn generated_contact_source_constraint_noding_candidates(
    contours: &[NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
) -> ContactInsertionsByIndex {
    let mut candidates = ContactInsertionsByIndex::new();
    for constraint in constraints {
        if generated_contact_kind_from_constraint(constraint.kind).is_none()
            || constraint.owner.is_none()
            || constraint.opposite_owner.is_none()
        {
            continue;
        }
        for source_edge in generated_constraint_directed_edges(constraint) {
            for contour in contours {
                if !generated_contact_source_constraint_can_node_with_contour(constraint, contour) {
                    continue;
                }
                for point in generated_contour_keys(contour) {
                    if generated_point_key_lies_on_segment(
                        point,
                        source_edge.start,
                        source_edge.end,
                    ) {
                        candidates
                            .entry(constraint.constraint_index)
                            .or_default()
                            .entry(source_edge)
                            .or_default()
                            .insert(point);
                    }
                }
                for contour_edge in generated_contour_directed_edges(contour) {
                    if let Some(point) = quantized_proper_segment_intersection(
                        source_edge.start,
                        source_edge.end,
                        contour_edge.start,
                        contour_edge.end,
                    ) {
                        candidates
                            .entry(constraint.constraint_index)
                            .or_default()
                            .entry(source_edge)
                            .or_default()
                            .insert(point);
                    }
                }
            }
        }
    }
    candidates
}

fn generated_contact_source_constraint_can_node_with_contour(
    constraint: &NodeRailConstraint,
    contour: &NodeGeneratedContour,
) -> bool {
    let Some(contour_owner) = contour.owner else {
        return false;
    };
    let (Some(owner), Some(opposite_owner)) = (constraint.owner, constraint.opposite_owner) else {
        return false;
    };
    contour_owner == owner
        || contour_owner == opposite_owner
        || contour_owner.kind() == owner.kind()
        || contour_owner.kind() == opposite_owner.kind()
}

pub(in crate::simulation::network::surface::node::rails) fn node_generated_contact_sources_from_contour_backed_contacts(
    contours: &[NodeGeneratedContour],
    constraints: &mut [NodeRailConstraint],
    generated_constraint_start_index: usize,
) {
    let generated_constraint_start_index = generated_constraint_start_index.min(constraints.len());
    let mut insertions = ContactInsertionsByIndex::new();
    for constraint in constraints.iter().skip(generated_constraint_start_index) {
        let Some(contact) = generated_same_band_contact_constraint(constraint) else {
            continue;
        };
        for point in [contact.start, contact.end] {
            if !generated_contact_point_has_source_contour_authority(contours, contact, point) {
                continue;
            }
            for source_constraint in constraints.iter().take(generated_constraint_start_index) {
                if source_constraint.kind != contact.kind
                    || source_constraint.source_mouth_order_index
                        != contact.source_mouth_order_index
                    || source_constraint.source_band_index != contact.source_band_index
                    || !owners_match_unordered(
                        source_constraint.owner,
                        source_constraint.opposite_owner,
                        contact.owner,
                        contact.opposite_owner,
                    )
                {
                    continue;
                }
                for edge in generated_constraint_directed_edges(source_constraint) {
                    if generated_point_key_lies_on_segment(point, edge.start, edge.end) {
                        insertions
                            .entry(source_constraint.constraint_index)
                            .or_default()
                            .entry(edge)
                            .or_default()
                            .insert(point);
                    }
                }
            }
        }
    }
    insert_keys_on_generated_source_constraints(
        &mut constraints[..generated_constraint_start_index],
        insertions,
    );
}

fn generated_contact_point_has_source_contour_authority(
    contours: &[NodeGeneratedContour],
    contact: GeneratedSameBandContactConstraint,
    point: NodeRailPointKey,
) -> bool {
    let Some(source_band_index) = contact.source_band_index else {
        return false;
    };
    contours.iter().any(|contour| {
        contour.source_mouth_order_index == contact.source_mouth_order_index
            && contour.source_band_index == Some(source_band_index)
            && contour.claim_priority == NodeGeneratedContourClaimPriority::MouthBand
            && (contour.owner == Some(contact.owner)
                || contour.owner == Some(contact.opposite_owner))
            && generated_contour_boundary_contains_key(contour, point)
    })
}
