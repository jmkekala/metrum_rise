//! Same-band and contact-edge constraint emission for generated rails.

use super::super::authority::{
    GeneratedContactAuthorityIndex, GeneratedMaterialPointContactAuthority,
    generated_contact_authority_source_edges_touching_contour_pair,
    generated_contact_edge_source_authority, generated_contact_point_has_explicit_roles,
    generated_exact_owner_pair_contact_authority_at_point,
};
use super::super::*;
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

type SameMaterialHeightSplitConstraint = (
    NodeBandOwner,
    NodeBandOwner,
    NodeRailPointKey,
    NodeRailPointKey,
    usize,
    Option<usize>,
);

const SAME_BAND_PARALLEL_PAIR_THRESHOLD: usize = 64;
const SAME_BAND_PARALLEL_PAIR_BATCH: usize = 16;
const SAME_BAND_CANDIDATE_TILE_KEYS: i64 = 8_000_000;

pub(in crate::simulation::network::surface::node::rails) fn append_generated_same_band_contact_constraints(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    contours: &[NodeGeneratedContour],
    source_constraint_count: usize,
    constraints: &mut Vec<NodeRailConstraint>,
) -> GeneratedContactEmissionStats {
    let before_len = constraints.len();
    let authority_index = GeneratedContactAuthorityIndex::new(constraints);
    let summaries = generated_contact_contour_summaries_with_overlay(contours);
    let indexed_pairs = same_band_candidate_pair_index(&summaries, &authority_index);
    let mut stats = indexed_pairs.stats;
    let mut contact_edges = BTreeSet::<GeneratedSameBandContactConstraint>::new();
    let mut same_material_height_splits = BTreeSet::<SameMaterialHeightSplitConstraint>::new();
    let pair_indices = indexed_pairs.pair_indices;
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
    stats.same_material_height_split_candidates = same_material_height_splits.len();
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
    let append_stats =
        append_same_material_height_split_constraints(constraints, same_material_height_splits);
    stats.same_material_height_split_appended = append_stats.appended;
    stats.same_material_height_split_duplicates = append_stats.duplicates;
    stats.emitted_constraints = constraints.len() - before_len;
    stats
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct SameBandCandidateTile {
    x: i64,
    z: i64,
}

#[derive(Default)]
struct SameBandCandidatePairIndex {
    stats: GeneratedContactEmissionStats,
    pair_indices: Vec<(usize, usize)>,
}

fn same_band_candidate_pair_index(
    summaries: &[GeneratedContactContourSummary],
    authority_index: &GeneratedContactAuthorityIndex<'_>,
) -> SameBandCandidatePairIndex {
    let mut index = SameBandCandidatePairIndex::default();
    index.stats.pair_tests = summaries
        .len()
        .saturating_mul(summaries.len().saturating_sub(1))
        / 2;

    let mut indices_by_tile = BTreeMap::<SameBandCandidateTile, Vec<usize>>::new();
    for (summary_index, summary) in summaries.iter().enumerate() {
        if summary.owner.is_none() || summary.kind.is_none() || !summary.bounds_valid() {
            continue;
        }
        for tile in same_band_candidate_tiles(summary) {
            indices_by_tile.entry(tile).or_default().push(summary_index);
        }
    }

    let mut tile_pairs = BTreeSet::<(usize, usize)>::new();
    for indices in indices_by_tile.values() {
        for left_position in 0..indices.len() {
            for right_index in indices.iter().copied().skip(left_position + 1) {
                let left_index = indices[left_position];
                let pair = if left_index <= right_index {
                    (left_index, right_index)
                } else {
                    (right_index, left_index)
                };
                tile_pairs.insert(pair);
            }
        }
    }
    index.stats.candidate_pairs = tile_pairs.len();

    for (left_index, right_index) in tile_pairs {
        let left_summary = &summaries[left_index];
        let right_summary = &summaries[right_index];
        if !same_band_candidate_pair_can_contact(
            left_summary,
            right_summary,
            authority_index,
            &mut index.stats,
        ) {
            continue;
        }
        index.pair_indices.push((left_index, right_index));
    }
    index
}

fn same_band_candidate_tiles(
    summary: &GeneratedContactContourSummary,
) -> Vec<SameBandCandidateTile> {
    let Some((min_x, min_z, max_x, max_z)) = summary.bounds() else {
        return Vec::new();
    };
    let min_tile_x = min_x.div_euclid(SAME_BAND_CANDIDATE_TILE_KEYS);
    let max_tile_x = max_x.div_euclid(SAME_BAND_CANDIDATE_TILE_KEYS);
    let min_tile_z = min_z.div_euclid(SAME_BAND_CANDIDATE_TILE_KEYS);
    let max_tile_z = max_z.div_euclid(SAME_BAND_CANDIDATE_TILE_KEYS);
    let mut tiles = Vec::new();
    for x in min_tile_x..=max_tile_x {
        for z in min_tile_z..=max_tile_z {
            tiles.push(SameBandCandidateTile { x, z });
        }
    }
    tiles
}

fn same_band_candidate_pair_can_contact(
    left_summary: &GeneratedContactContourSummary,
    right_summary: &GeneratedContactContourSummary,
    authority_index: &GeneratedContactAuthorityIndex<'_>,
    stats: &mut GeneratedContactEmissionStats,
) -> bool {
    let Some(left_owner) = left_summary.owner else {
        stats.kind_rejected += 1;
        return false;
    };
    let Some(right_owner) = right_summary.owner else {
        stats.kind_rejected += 1;
        return false;
    };
    if left_owner == right_owner {
        stats.kind_rejected += 1;
        return false;
    }
    let Some(kind) = left_summary.kind else {
        stats.kind_rejected += 1;
        return false;
    };
    let Some(right_kind) = right_summary.kind else {
        stats.kind_rejected += 1;
        return false;
    };
    if left_summary.aabb_disjoint(right_summary) {
        stats.aabb_rejected += 1;
        return false;
    }

    if kind == right_kind {
        if same_material_pair_has_same_height_authority(left_summary, right_summary) {
            stats.same_authority_skipped += 1;
            return false;
        }
        stats.processed_pairs += 1;
        stats.same_material_candidate_pairs += 1;
        return true;
    }

    let Some(pair) = GeneratedRaisedStepOwnerPair::new(left_owner, right_owner) else {
        stats.kind_rejected += 1;
        return false;
    };
    if !authority_index.has_constraints_touching_contour_pair(
        NodeRailConstraintKind::RaisedStepContact,
        pair.owner,
        pair.opposite_owner,
        left_summary,
        right_summary,
    ) {
        stats.authority_rejected += 1;
        return false;
    }
    stats.processed_pairs += 1;
    stats.raised_step_candidate_pairs += 1;
    true
}

fn same_material_pair_has_same_height_authority(
    left_summary: &GeneratedContactContourSummary,
    right_summary: &GeneratedContactContourSummary,
) -> bool {
    let (Some(left), Some(right)) = (left_summary.authority_key, right_summary.authority_key)
    else {
        return false;
    };
    left.kind == right.kind
        && left.source_mouth_order_index == right.source_mouth_order_index
        && left.source_band_index.is_some()
        && left.source_band_index == right.source_band_index
        && left.claim_priority == right.claim_priority
        && left.has_height_carrier
        && right.has_height_carrier
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
    if kind == right_kind {
        result.stats.same_material_overlay_calls = 1;
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
    if !authority_index.has_constraints_touching_contour_pair(
        NodeRailConstraintKind::RaisedStepContact,
        pair.owner,
        pair.opposite_owner,
        left_summary,
        right_summary,
    ) {
        result.stats.kind_rejected = 1;
        return result;
    }
    let source_edges = generated_contact_authority_source_edges_touching_contour_pair(
        NodeRailConstraintKind::RaisedStepContact,
        pair.owner,
        pair.opposite_owner,
        left_summary,
        right_summary,
        authority_index,
    );
    let (contact_edges, used_pair_overlay) = generated_raised_step_contact_edges_from_authority(
        left,
        right,
        left_summary,
        right_summary,
        &source_edges,
    );
    if used_pair_overlay {
        result.stats.overlay_calls = 1;
    }
    let shared_edge_points = contact_edges
        .iter()
        .flat_map(|edge| [edge.start, edge.end])
        .collect::<BTreeSet<_>>();
    for edge in contact_edges {
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

fn generated_raised_step_contact_edges_from_authority(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    left_summary: &GeneratedContactContourSummary,
    right_summary: &GeneratedContactContourSummary,
    source_edges: &[GeneratedContourDirectedEdge],
) -> (Vec<GeneratedContourEdgeKey>, bool) {
    if !source_edges.is_empty() {
        if let (Some(left_shapes), Some(right_shapes)) = (
            left_summary.overlay_shapes.as_ref(),
            right_summary.overlay_shapes.as_ref(),
        ) {
            return (
                generated_contact_edges_from_source_edges_inside_shape_intersection(
                    source_edges,
                    &left_summary.overlay_shape_edges,
                    left_shapes,
                    &right_summary.overlay_shape_edges,
                    right_shapes,
                ),
                false,
            );
        }
    }
    let mut edges = BTreeSet::new();
    edges.extend(shared_sorted_edges(
        &left_summary.edges,
        &right_summary.edges,
    ));
    edges.extend(generated_contact_edges_inside_contour(left, right));
    edges.extend(generated_contact_edges_inside_contour(right, left));
    edges.extend(generated_contact_edges_from_summary_overlay(
        left,
        right,
        left_summary.overlay_shapes.as_ref(),
        right_summary.overlay_shapes.as_ref(),
    ));
    (edges.into_iter().collect(), true)
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
    stats.candidate_pairs += next.candidate_pairs;
    stats.same_material_candidate_pairs += next.same_material_candidate_pairs;
    stats.raised_step_candidate_pairs += next.raised_step_candidate_pairs;
    stats.authority_rejected += next.authority_rejected;
    stats.same_authority_skipped += next.same_authority_skipped;
    stats.same_material_overlay_calls += next.same_material_overlay_calls;
    stats.same_material_height_split_candidates += next.same_material_height_split_candidates;
    stats.same_material_height_split_appended += next.same_material_height_split_appended;
    stats.same_material_height_split_duplicates += next.same_material_height_split_duplicates;
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

struct SameMaterialHeightSplitAppendStats {
    appended: usize,
    duplicates: usize,
}

fn append_same_material_height_split_constraints(
    constraints: &mut Vec<NodeRailConstraint>,
    contacts: BTreeSet<SameMaterialHeightSplitConstraint>,
) -> SameMaterialHeightSplitAppendStats {
    let mut stats = SameMaterialHeightSplitAppendStats {
        appended: 0,
        duplicates: 0,
    };
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
            stats.duplicates += 1;
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
        stats.appended += 1;
    }
    stats
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
