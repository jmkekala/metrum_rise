//! Contact contour noding candidate discovery.

use super::super::materialization::generated_contact_point_has_explicit_roles;
use super::super::source_authority::generated_raised_step_contact_kind_for_owners;
use super::super::{
    GeneratedContourDirectedEdge, NodeGeneratedContour, NodeRailConstraint, NodeRailPointKey,
    generated_contour_band_kind, generated_contour_directed_edges, generated_contour_keys,
    generated_point_key_lies_on_segment, quantized_proper_segment_intersection,
};
use super::ContactNodingCandidate;

pub(super) fn generated_contact_contour_noding_candidates(
    contours: &[NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
) -> Vec<ContactNodingCandidate> {
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
