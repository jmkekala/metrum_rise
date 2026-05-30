//! Same-band and contact-edge constraint emission for generated rails.

use super::super::authority::{
    GeneratedContactAuthorityIndex, GeneratedMaterialPointContactAuthority,
    generated_contact_edge_source_authority, generated_contact_point_has_explicit_roles,
    generated_exact_owner_pair_contact_authority_at_point,
};
use super::super::*;
use rayon::prelude::*;
use std::collections::BTreeSet;

type SameMaterialHeightSplitConstraint = (
    NodeBandOwner,
    NodeBandOwner,
    NodeRailPointKey,
    NodeRailPointKey,
    usize,
    Option<usize>,
);

const SAME_BAND_PARALLEL_PAIR_THRESHOLD: usize = 512;
const SAME_BAND_PARALLEL_PAIR_BATCH: usize = 64;

pub(in crate::simulation::network::surface::node::rails) fn append_generated_same_band_contact_constraints(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    contours: &[NodeGeneratedContour],
    source_constraint_count: usize,
    constraints: &mut Vec<NodeRailConstraint>,
) -> GeneratedContactEmissionStats {
    let before_len = constraints.len();
    let authority_index = GeneratedContactAuthorityIndex::new(constraints);
    let summaries = generated_contact_contour_summaries_with_overlay(contours);
    let mut stats = GeneratedContactEmissionStats::default();
    let mut contact_edges = BTreeSet::<GeneratedSameBandContactConstraint>::new();
    let mut same_material_height_splits = BTreeSet::<SameMaterialHeightSplitConstraint>::new();
    let pair_indices = same_band_pair_indices(contours.len());
    let pair_results = if pair_indices.len() >= SAME_BAND_PARALLEL_PAIR_THRESHOLD {
        pair_indices
            .par_chunks(SAME_BAND_PARALLEL_PAIR_BATCH)
            .map(|pair_batch| {
                collect_same_band_pair_batch_contacts(
                    contours,
                    &summaries,
                    constraints,
                    &authority_index,
                    pair_batch,
                )
            })
            .collect::<Vec<_>>()
    } else {
        vec![collect_same_band_pair_batch_contacts(
            contours,
            &summaries,
            constraints,
            &authority_index,
            &pair_indices,
        )]
    };
    for mut result in pair_results {
        merge_contact_emission_stats(&mut stats, result.stats);
        contact_edges.append(&mut result.contact_edges);
        same_material_height_splits.append(&mut result.same_material_height_splits);
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
    stats.emitted_constraints = constraints.len() - before_len;
    stats
}

fn same_band_pair_indices(contour_count: usize) -> Vec<(usize, usize)> {
    let mut pair_indices = Vec::with_capacity(contour_count.saturating_mul(contour_count) / 2);
    for left_index in 0..contour_count {
        for right_index in left_index + 1..contour_count {
            pair_indices.push((left_index, right_index));
        }
    }
    pair_indices
}

struct SameBandContactPairResult {
    stats: GeneratedContactEmissionStats,
    contact_edges: BTreeSet<GeneratedSameBandContactConstraint>,
    same_material_height_splits: BTreeSet<SameMaterialHeightSplitConstraint>,
}

impl Default for SameBandContactPairResult {
    fn default() -> Self {
        Self {
            stats: GeneratedContactEmissionStats::default(),
            contact_edges: BTreeSet::new(),
            same_material_height_splits: BTreeSet::new(),
        }
    }
}

impl SameBandContactPairResult {
    fn merge(&mut self, mut next: Self) {
        merge_contact_emission_stats(&mut self.stats, next.stats);
        self.contact_edges.append(&mut next.contact_edges);
        self.same_material_height_splits
            .append(&mut next.same_material_height_splits);
    }
}

fn collect_same_band_pair_batch_contacts(
    contours: &[NodeGeneratedContour],
    summaries: &[GeneratedContactContourSummary],
    constraints: &[NodeRailConstraint],
    authority_index: &GeneratedContactAuthorityIndex<'_>,
    pair_indices: &[(usize, usize)],
) -> SameBandContactPairResult {
    let mut batch_result = SameBandContactPairResult::default();
    for &(left_index, right_index) in pair_indices {
        batch_result.merge(collect_same_band_pair_contacts(
            contours,
            summaries,
            constraints,
            authority_index,
            left_index,
            right_index,
        ));
    }
    batch_result
}

fn collect_same_band_pair_contacts(
    contours: &[NodeGeneratedContour],
    summaries: &[GeneratedContactContourSummary],
    constraints: &[NodeRailConstraint],
    authority_index: &GeneratedContactAuthorityIndex<'_>,
    left_index: usize,
    right_index: usize,
) -> SameBandContactPairResult {
    let mut result = SameBandContactPairResult::default();
    result.stats.pair_tests = 1;
    let left = &contours[left_index];
    let right = &contours[right_index];
    let left_summary = &summaries[left_index];
    let right_summary = &summaries[right_index];
    let Some(left_owner) = left_summary.owner else {
        result.stats.kind_rejected = 1;
        return result;
    };
    let Some(right_owner) = right_summary.owner else {
        result.stats.kind_rejected = 1;
        return result;
    };
    if left_owner == right_owner {
        result.stats.kind_rejected = 1;
        return result;
    }
    let Some(kind) = left_summary.kind else {
        result.stats.kind_rejected = 1;
        return result;
    };
    let Some(right_kind) = right_summary.kind else {
        result.stats.kind_rejected = 1;
        return result;
    };
    if left_summary.aabb_disjoint(right_summary) {
        result.stats.aabb_rejected = 1;
        return result;
    }
    result.stats.processed_pairs = 1;
    if kind == right_kind {
        collect_same_material_height_splits_from_edges(
            left,
            right,
            left_summary.overlay_shapes.as_ref(),
            right_summary.overlay_shapes.as_ref(),
            &shared_sorted_edges(&left_summary.edges, &right_summary.edges),
            left_owner,
            right_owner,
            &mut result.same_material_height_splits,
        );
        return result;
    }
    let Some(contact_kind) = generated_raised_step_contact_kind_for_owners(left_owner, right_owner)
    else {
        result.stats.kind_rejected = 1;
        return result;
    };
    let Some(pair) = GeneratedRaisedStepOwnerPair::new(left_owner, right_owner) else {
        result.stats.kind_rejected = 1;
        return result;
    };
    if !authority_index.has_constraints_for(
        NodeRailConstraintKind::RaisedStepContact,
        pair.owner,
        pair.opposite_owner,
    ) {
        result.stats.kind_rejected = 1;
        return result;
    }
    let shared_edges = shared_sorted_edges(&left_summary.edges, &right_summary.edges);
    let shared_edge_points = shared_edges
        .iter()
        .flat_map(|edge| [edge.start, edge.end])
        .collect::<BTreeSet<_>>();
    for edge in shared_edges {
        if let Some(source) = generated_contact_edge_source_authority(
            pair.owner,
            pair.opposite_owner,
            authority_index,
            edge,
        ) {
            insert_generated_contact_constraint(
                &mut result.contact_edges,
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
            authority_index,
            edge,
        ) {
            insert_generated_contact_constraint(
                &mut result.contact_edges,
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
            authority_index,
            edge,
        ) {
            insert_generated_contact_constraint(
                &mut result.contact_edges,
                contact_kind,
                pair.owner,
                pair.opposite_owner,
                edge,
                source,
            );
        }
    }
    result.stats.overlay_calls = 1;
    for edge in generated_contact_edges_from_summary_overlay(
        left,
        right,
        left_summary.overlay_shapes.as_ref(),
        right_summary.overlay_shapes.as_ref(),
    ) {
        if let Some(source) = generated_contact_edge_source_authority(
            pair.owner,
            pair.opposite_owner,
            authority_index,
            edge,
        ) {
            insert_generated_contact_constraint(
                &mut result.contact_edges,
                contact_kind,
                pair.owner,
                pair.opposite_owner,
                edge,
                source,
            );
        }
    }
    for point in shared_sorted_keys(&left_summary.keys, &right_summary.keys) {
        if shared_edge_points.contains(&point) {
            continue;
        }
        if !generated_contact_point_has_explicit_roles(
            kind,
            right_kind,
            left,
            right,
            constraints,
            authority_index,
            point,
            contact_kind,
        ) {
            continue;
        }
        let Some(source) = generated_exact_owner_pair_contact_authority_at_point(
            pair.owner,
            pair.opposite_owner,
            authority_index,
            point,
        ) else {
            continue;
        };
        result
            .contact_edges
            .insert(GeneratedSameBandContactConstraint {
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
            authority_index,
            point,
            contact_kind,
        ) {
            continue;
        }
        let Some(source) = generated_exact_owner_pair_contact_authority_at_point(
            pair.owner,
            pair.opposite_owner,
            authority_index,
            point,
        ) else {
            continue;
        };
        result
            .contact_edges
            .insert(GeneratedSameBandContactConstraint {
                kind: contact_kind,
                owner: pair.owner,
                opposite_owner: pair.opposite_owner,
                start: point,
                end: point,
                source_mouth_order_index: source.source_mouth_order_index,
                source_band_index: source.source_band_index,
            });
    }
    result
}

fn merge_contact_emission_stats(
    stats: &mut GeneratedContactEmissionStats,
    next: GeneratedContactEmissionStats,
) {
    stats.pair_tests += next.pair_tests;
    stats.aabb_rejected += next.aabb_rejected;
    stats.kind_rejected += next.kind_rejected;
    stats.processed_pairs += next.processed_pairs;
    stats.overlay_calls += next.overlay_calls;
    stats.emitted_constraints += next.emitted_constraints;
}

fn collect_same_material_height_splits_from_edges(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    left_shapes: Option<&NodeOverlayShapes>,
    right_shapes: Option<&NodeOverlayShapes>,
    shared_edges: &[GeneratedContourEdgeKey],
    left_owner: NodeBandOwner,
    right_owner: NodeBandOwner,
    contacts: &mut BTreeSet<SameMaterialHeightSplitConstraint>,
) {
    let mut edges = BTreeSet::new();
    for &edge in shared_edges {
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
    for edge in generated_contact_edges_from_summary_overlay(left, right, left_shapes, right_shapes)
    {
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

fn generated_contact_edges_from_summary_overlay(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    left_shapes: Option<&NodeOverlayShapes>,
    right_shapes: Option<&NodeOverlayShapes>,
) -> Vec<GeneratedContourEdgeKey> {
    match (left_shapes, right_shapes) {
        (Some(left_shapes), Some(right_shapes)) => {
            generated_contact_edges_from_overlay_shape_intersection(left_shapes, right_shapes)
        }
        _ => generated_contact_edges_from_overlay_intersection(left, right),
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
    let mut existing = constraints
        .iter()
        .filter(|constraint| {
            constraint.kind == NodeRailConstraintKind::RaisedStepContact
                && constraint.points_xz.len() == 2
        })
        .filter_map(|constraint| {
            let owner = constraint.owner?;
            let opposite_owner = constraint.opposite_owner?;
            let (owner, opposite_owner) = ordered_owner_pair(owner, opposite_owner);
            Some((
                owner,
                opposite_owner,
                GeneratedContourEdgeKey::new(
                    road_point_key(constraint.points_xz[0]),
                    road_point_key(constraint.points_xz[1]),
                ),
            ))
        })
        .collect::<BTreeSet<_>>();
    for (owner, opposite_owner, start, end, source_mouth_order_index, source_band_index) in contacts
    {
        let edge = GeneratedContourEdgeKey::new(start, end);
        if !existing.insert((owner, opposite_owner, edge)) {
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

fn ordered_owner_pair(
    left_owner: NodeBandOwner,
    right_owner: NodeBandOwner,
) -> (NodeBandOwner, NodeBandOwner) {
    if left_owner <= right_owner {
        (left_owner, right_owner)
    } else {
        (right_owner, left_owner)
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
