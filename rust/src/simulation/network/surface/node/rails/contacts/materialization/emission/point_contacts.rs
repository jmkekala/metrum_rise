//! Point-contact constraint emission for generated rail contact materialization.

use super::super::authority::{
    GeneratedContactAuthorityIndex,
    append_generated_material_authority_points_on_counterpart_contour,
    generated_material_point_contact_authority,
};
use super::super::*;

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

#[cfg(test)]
pub(in crate::simulation::network::surface::node::rails) fn append_source_authorized_raised_step_point_contacts(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    contours: &[NodeGeneratedContour],
    source_constraint_count: usize,
    constraints: &mut Vec<NodeRailConstraint>,
) -> usize {
    let mut current = NodeSourceAuthorizedContactCache::default();
    append_source_authorized_raised_step_point_contacts_with_reuse(
        piece_kind,
        contours,
        source_constraint_count,
        constraints,
        None,
        &mut current,
    )
    .0
}

pub(in crate::simulation::network::surface::node::rails) fn append_source_authorized_raised_step_point_contacts_with_reuse(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    contours: &[NodeGeneratedContour],
    source_constraint_count: usize,
    constraints: &mut Vec<NodeRailConstraint>,
    previous: Option<&NodeSourceAuthorizedContactCache>,
    current: &mut NodeSourceAuthorizedContactCache,
) -> (usize, SourceAuthorizedContactReuseStats) {
    let before_len = constraints.len();
    let mut contacts = Vec::<GeneratedSameBandContactConstraint>::new();
    let source_constraints = super::source_authority_constraints_for_generated_contacts(
        constraints,
        source_constraint_count,
    );
    let stats = collect_source_authorized_raised_step_contacts_with_reuse(
        piece_kind,
        contours,
        &source_constraints,
        &mut contacts,
        previous,
        current,
    );

    super::retain_new_sorted_generated_contacts(&mut contacts, constraints);
    for contact in contacts {
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
    (constraints.len() - before_len, stats)
}

pub(in crate::simulation::network::surface::node::rails) fn append_generated_material_point_contact_constraints(
    contours: &[NodeGeneratedContour],
    constraints: &mut Vec<NodeRailConstraint>,
) -> GeneratedContactEmissionStats {
    let before_len = constraints.len();
    let authority_index = GeneratedContactAuthorityIndex::new(constraints);
    let summaries = generated_contact_contour_summaries(contours);
    let mut stats = GeneratedContactEmissionStats::default();
    stats.pair_tests = summaries
        .len()
        .saturating_mul(summaries.len().saturating_sub(1))
        / 2;
    let candidate_pairs = generated_contact_candidate_pair_indices(&summaries);
    stats.candidate_pairs = candidate_pairs.len();
    let mut contact_points = Vec::<GeneratedSameBandContactConstraint>::new();
    let mut points = Vec::new();
    for (left_index, right_index) in candidate_pairs {
        let left_summary = &summaries[left_index];
        let right_summary = &summaries[right_index];
        let Some(left_owner) = left_summary.owner else {
            stats.kind_rejected += 1;
            continue;
        };
        let Some(right_owner) = right_summary.owner else {
            stats.kind_rejected += 1;
            continue;
        };
        if left_owner == right_owner {
            stats.kind_rejected += 1;
            continue;
        }
        let Some(left_kind) = left_summary.kind else {
            stats.kind_rejected += 1;
            continue;
        };
        let Some(right_kind) = right_summary.kind else {
            stats.kind_rejected += 1;
            continue;
        };
        if left_kind == right_kind {
            stats.kind_rejected += 1;
            continue;
        }
        let Some(contact_kind) =
            generated_raised_step_contact_kind_for_owners(left_owner, right_owner)
        else {
            stats.kind_rejected += 1;
            continue;
        };
        let Some(pair) = GeneratedRaisedStepOwnerPair::new(left_owner, right_owner) else {
            stats.kind_rejected += 1;
            continue;
        };
        if left_summary.aabb_disjoint(right_summary) {
            stats.aabb_rejected += 1;
            continue;
        }
        stats.processed_pairs += 1;
        points.clear();
        append_shared_sorted_keys(&left_summary.keys, &right_summary.keys, &mut points);
        append_generated_contact_points_from_summary_intersections(
            left_summary,
            right_summary,
            &mut points,
        );
        append_generated_material_authority_points_on_counterpart_contour(
            contact_kind,
            left_summary,
            right_summary,
            left_owner,
            right_owner,
            &authority_index,
            &mut points,
        );
        points.sort_unstable();
        points.dedup();
        for &point in &points {
            let Some(contact_source) = generated_material_point_contact_authority(
                contact_kind,
                left_owner,
                right_owner,
                point,
                &authority_index,
            ) else {
                continue;
            };
            contact_points.push(GeneratedSameBandContactConstraint {
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

    super::retain_new_generated_contacts(&mut contact_points, constraints);
    for contact in contact_points {
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
    stats.emitted_constraints = constraints.len() - before_len;
    stats
}
