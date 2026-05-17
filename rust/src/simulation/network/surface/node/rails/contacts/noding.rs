//! Canonical contact-point insertion for generated rail contacts.

use super::super::*;
use super::geometry::*;
use super::materialization::generated_contact_point_has_explicit_roles;
use super::source_authority::{
    GeneratedSameBandContactConstraint, generated_contact_kind_from_constraint,
    generated_raised_step_contact_kind_for_owners, generated_same_band_contact_constraint,
};

pub(in crate::simulation::network::surface::node::rails) fn node_generated_contact_contours(
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

pub(super) fn generated_contact_contour_noding_candidates(
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

pub(super) fn insert_contact_noding_candidates(
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

pub(super) fn insert_keys_on_generated_contour_edges(
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

pub(super) fn generated_contact_source_constraint_noding_candidates(
    contours: &[NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
) -> BTreeMap<usize, BTreeMap<GeneratedContourDirectedEdge, BTreeSet<NodeRailPointKey>>> {
    let mut candidates =
        BTreeMap::<usize, BTreeMap<GeneratedContourDirectedEdge, BTreeSet<NodeRailPointKey>>>::new(
        );
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

pub(super) fn generated_contact_source_constraint_can_node_with_contour(
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
    let mut insertions =
        BTreeMap::<usize, BTreeMap<GeneratedContourDirectedEdge, BTreeSet<NodeRailPointKey>>>::new(
        );
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

pub(super) fn generated_contact_point_has_source_contour_authority(
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

pub(super) fn insert_keys_on_generated_source_constraints(
    constraints: &mut [NodeRailConstraint],
    insertions_by_constraint: BTreeMap<
        usize,
        BTreeMap<GeneratedContourDirectedEdge, BTreeSet<NodeRailPointKey>>,
    >,
) -> bool {
    let mut inserted_any = false;
    for constraint in constraints {
        let Some(insertions_by_edge) = insertions_by_constraint.get(&constraint.constraint_index)
        else {
            continue;
        };
        let keys = constraint
            .points_xz
            .iter()
            .copied()
            .map(road_point_key)
            .collect::<Vec<_>>();
        if keys.len() < 2 {
            continue;
        }
        let mut new_keys = Vec::with_capacity(keys.len());
        for segment in keys.windows(2) {
            let start = segment[0];
            let end = segment[1];
            new_keys.push(start);
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
            if !insertions.is_empty() {
                inserted_any = true;
            }
            new_keys.extend(insertions);
        }
        if let Some(last) = keys.last().copied() {
            new_keys.push(last);
        }
        new_keys.dedup();
        if new_keys != keys {
            constraint.points_xz = new_keys.into_iter().map(road_point_from_key).collect();
        }
    }
    inserted_any
}

pub(super) fn generated_contours_support_contact_noding(
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

pub(super) fn generated_contact_point_on_edge_noding_candidates(
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

pub(super) fn generated_contact_edge_intersection_noding_candidates(
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

pub(super) fn generated_contact_noding_point_has_explicit_roles(
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
