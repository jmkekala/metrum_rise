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
use std::sync::Arc;

type SameMaterialHeightSplitConstraint = (
    NodeBandOwner,
    NodeBandOwner,
    NodeRailPointKey,
    NodeRailPointKey,
    usize,
    Option<usize>,
);

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct SameMaterialContourContributorKey {
    kind: RoadSurfaceBandKind,
    // Positional publication fields intentionally remain exact cache inputs.
    // Replaying old owners or source labels would be incorrect until this
    // cache has semantic contributor rebinding.
    owner: NodeBandOwner,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
    claim_priority: NodeGeneratedContourClaimPriority,
    has_height_carrier: bool,
    ordered_points_xz: Vec<NodeRailPointKey>,
}

impl SameMaterialContourContributorKey {
    fn from_contour(
        contour: &NodeGeneratedContour,
        summary: &GeneratedContactContourSummary,
    ) -> Option<Self> {
        Some(Self {
            kind: summary.kind?,
            owner: summary.owner?,
            source_mouth_order_index: contour.source_mouth_order_index,
            source_band_index: contour.source_band_index,
            claim_priority: contour.claim_priority,
            has_height_carrier: contour
                .height_points_world
                .as_ref()
                .is_some_and(|points| !points.is_empty()),
            ordered_points_xz: generated_contour_keys(contour),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct SameMaterialPairContributorKey {
    left: Arc<SameMaterialContourContributorKey>,
    right: Arc<SameMaterialContourContributorKey>,
}

impl SameMaterialPairContributorKey {
    fn from_indices(
        contour_contributors: &[Option<Arc<SameMaterialContourContributorKey>>],
        left_index: usize,
        right_index: usize,
    ) -> Option<Self> {
        let left = Arc::clone(contour_contributors.get(left_index)?.as_ref()?);
        let right = Arc::clone(contour_contributors.get(right_index)?.as_ref()?);
        Some(Self::new(left, right))
    }

    fn new(
        left: Arc<SameMaterialContourContributorKey>,
        right: Arc<SameMaterialContourContributorKey>,
    ) -> Self {
        if left <= right {
            Self { left, right }
        } else {
            Self {
                left: right,
                right: left,
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct RaisedStepAuthorityConstraintContributorKey {
    constraint_index: usize,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    ordered_points_xz: Arc<[NodeRailPointKey]>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct RaisedStepPairContributorKey {
    contours: SameMaterialPairContributorKey,
    relevant_authority: Arc<[RaisedStepAuthorityConstraintContributorKey]>,
}

/// Immutable same-band pair contributions reusable by a later node-rail generation.
#[derive(Clone, Debug, Default)]
pub(in crate::simulation::network::surface::node::rails) struct NodeSameMaterialContactPairCache {
    entries: BTreeMap<SameMaterialPairContributorKey, Arc<[SameMaterialHeightSplitConstraint]>>,
    raised_step_entries:
        BTreeMap<RaisedStepPairContributorKey, Arc<[GeneratedSameBandContactConstraint]>>,
}

const SAME_BAND_PARALLEL_PAIR_THRESHOLD: usize = 64;
const SAME_BAND_PARALLEL_PAIR_BATCH: usize = 16;
const SAME_BAND_CANDIDATE_TILE_KEYS: i64 = 8_000_000;

#[cfg(test)]
pub(in crate::simulation::network::surface::node::rails) fn append_generated_same_band_contact_constraints(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    contours: &[NodeGeneratedContour],
    source_constraint_count: usize,
    constraints: &mut Vec<NodeRailConstraint>,
) -> GeneratedContactEmissionStats {
    append_generated_same_band_contact_constraints_with_reuse(
        piece_kind,
        contours,
        source_constraint_count,
        constraints,
        None,
    )
    .0
}

/// Emits same-band constraints while retaining exact same-material pair contributions.
#[cfg(test)]
pub(in crate::simulation::network::surface::node::rails) fn append_generated_same_band_contact_constraints_with_reuse(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    contours: &[NodeGeneratedContour],
    source_constraint_count: usize,
    constraints: &mut Vec<NodeRailConstraint>,
    previous_same_material_pairs: Option<&NodeSameMaterialContactPairCache>,
) -> (
    GeneratedContactEmissionStats,
    NodeSameMaterialContactPairCache,
) {
    let mut current_source_contacts = NodeSourceAuthorizedContactCache::default();
    append_generated_same_band_contact_constraints_with_source_reuse(
        piece_kind,
        contours,
        source_constraint_count,
        constraints,
        previous_same_material_pairs,
        None,
        &mut current_source_contacts,
    )
}

/// Emits same-band constraints while reusing exact material and source contributors.
pub(in crate::simulation::network::surface::node::rails) fn append_generated_same_band_contact_constraints_with_source_reuse(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    contours: &[NodeGeneratedContour],
    source_constraint_count: usize,
    constraints: &mut Vec<NodeRailConstraint>,
    previous_same_material_pairs: Option<&NodeSameMaterialContactPairCache>,
    previous_source_contacts: Option<&NodeSourceAuthorizedContactCache>,
    current_source_contacts: &mut NodeSourceAuthorizedContactCache,
) -> (
    GeneratedContactEmissionStats,
    NodeSameMaterialContactPairCache,
) {
    let before_len = constraints.len();
    let authority_index = GeneratedContactAuthorityIndex::new(constraints);
    let mut summaries = generated_contact_contour_summaries(contours);
    let contour_contributors = contours
        .iter()
        .zip(&summaries)
        .map(|(contour, summary)| {
            SameMaterialContourContributorKey::from_contour(contour, summary).map(Arc::new)
        })
        .collect::<Vec<_>>();
    let indexed_pairs = same_band_candidate_pair_index(&summaries, &authority_index);
    let mut stats = indexed_pairs.stats;
    let mut contact_edges = BTreeSet::<GeneratedSameBandContactConstraint>::new();
    let mut same_material_height_splits = BTreeSet::<SameMaterialHeightSplitConstraint>::new();
    let pair_indices = indexed_pairs.pair_indices;
    let raised_step_pair_contributor_keys = raised_step_pair_contributor_keys(
        &authority_index,
        &summaries,
        &pair_indices,
        &contour_contributors,
    );
    populate_required_pair_overlay_summaries(
        contours,
        &mut summaries,
        &pair_indices,
        &contour_contributors,
        &raised_step_pair_contributor_keys,
        previous_same_material_pairs,
    );
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
                    &contour_contributors,
                    &raised_step_pair_contributor_keys,
                    previous_same_material_pairs,
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
            &contour_contributors,
            &raised_step_pair_contributor_keys,
            previous_same_material_pairs,
        )]
    };
    let mut same_material_pair_cache = NodeSameMaterialContactPairCache::default();
    for mut result in pair_results {
        merge_contact_emission_stats(&mut stats, result.stats);
        contact_edges.append(&mut result.contact_edges);
        same_material_height_splits.append(&mut result.same_material_height_splits);
        same_material_pair_cache
            .entries
            .append(&mut result.same_material_pair_cache.entries);
        same_material_pair_cache
            .raised_step_entries
            .append(&mut result.same_material_pair_cache.raised_step_entries);
    }
    stats.same_material_height_split_candidates = same_material_height_splits.len();
    let source_constraints = super::source_authority_constraints_for_generated_contacts(
        constraints,
        source_constraint_count,
    );
    let source_contact_reuse = collect_source_authorized_raised_step_contacts_with_reuse(
        piece_kind,
        contours,
        &source_constraints,
        &mut contact_edges,
        previous_source_contacts,
        current_source_contacts,
    );
    include_source_authorized_contact_reuse_stats(&mut stats, source_contact_reuse);

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
    (stats, same_material_pair_cache)
}

fn include_source_authorized_contact_reuse_stats(
    stats: &mut GeneratedContactEmissionStats,
    source: SourceAuthorizedContactReuseStats,
) {
    stats.source_target_group_cache_hits += source.target_group_cache_hits;
    stats.source_contact_cache_hits += source.source_cache_hits;
    stats.source_contact_cache_misses += source.source_cache_misses;
    stats.source_pair_cache_hits += source.source_pair_cache_hits;
    stats.source_pair_cache_misses += source.source_pair_cache_misses;
}

fn populate_required_pair_overlay_summaries(
    contours: &[NodeGeneratedContour],
    summaries: &mut [GeneratedContactContourSummary],
    pair_indices: &[(usize, usize)],
    contour_contributors: &[Option<Arc<SameMaterialContourContributorKey>>],
    raised_step_pair_contributor_keys: &BTreeMap<(usize, usize), RaisedStepPairContributorKey>,
    previous_same_material_pairs: Option<&NodeSameMaterialContactPairCache>,
) {
    let mut required_indices = BTreeSet::new();
    for &(left_index, right_index) in pair_indices {
        let left_summary = &summaries[left_index];
        let right_summary = &summaries[right_index];
        let pair_is_cached =
            if left_summary.kind.is_some() && left_summary.kind == right_summary.kind {
                SameMaterialPairContributorKey::from_indices(
                    contour_contributors,
                    left_index,
                    right_index,
                )
                .is_some_and(|key| {
                    previous_same_material_pairs
                        .is_some_and(|previous| previous.entries.contains_key(&key))
                })
            } else {
                raised_step_pair_contributor_keys
                    .get(&(left_index, right_index))
                    .is_some_and(|key| {
                        previous_same_material_pairs
                            .is_some_and(|previous| previous.raised_step_entries.contains_key(&key))
                    })
            };
        if pair_is_cached {
            continue;
        }
        required_indices.insert(left_index);
        required_indices.insert(right_index);
    }
    let required_indices = required_indices.into_iter().collect::<Vec<_>>();
    let overlay_summaries = required_indices
        .par_iter()
        .map(|&index| {
            (
                index,
                GeneratedContactContourSummary::from_contour_with_overlay(&contours[index], true),
            )
        })
        .collect::<Vec<_>>();
    for (index, summary) in overlay_summaries {
        summaries[index] = summary;
    }
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

fn raised_step_pair_contributor_key(
    authority_index: &GeneratedContactAuthorityIndex<'_>,
    left_summary: &GeneratedContactContourSummary,
    right_summary: &GeneratedContactContourSummary,
    contour_contributors: &[Option<Arc<SameMaterialContourContributorKey>>],
    left_index: usize,
    right_index: usize,
) -> Option<RaisedStepPairContributorKey> {
    let contours = SameMaterialPairContributorKey::from_indices(
        contour_contributors,
        left_index,
        right_index,
    )?;
    let pair = GeneratedRaisedStepOwnerPair::new(left_summary.owner?, right_summary.owner?)?;
    let mut relevant_authority = Vec::new();
    authority_index.visit_constraints_touching_contour_pair(
        NodeRailConstraintKind::RaisedStepContact,
        pair.owner,
        pair.opposite_owner,
        left_summary,
        right_summary,
        |constraint| {
            debug_assert_eq!(
                constraint.kind,
                NodeRailConstraintKind::RaisedStepContact,
                "authority visitor must stay within its requested kind bucket"
            );
            debug_assert!(
                owners_match_unordered(
                    constraint.owner,
                    constraint.opposite_owner,
                    pair.owner,
                    pair.opposite_owner,
                ),
                "authority visitor must stay within its requested owner-pair bucket"
            );
            relevant_authority.push(RaisedStepAuthorityConstraintContributorKey {
                // Exact-authority selection uses the lowest current constraint
                // index. Retaining it here prevents a cached source label from
                // surviving a positional rebind.
                constraint_index: constraint.constraint_index,
                source_mouth_order_index: constraint.source_mouth_order_index,
                source_band_index: constraint.source_band_index,
                owner: pair.owner,
                opposite_owner: pair.opposite_owner,
                ordered_points_xz: Arc::from(
                    constraint
                        .points_xz
                        .iter()
                        .copied()
                        .map(road_point_key)
                        .collect::<Vec<_>>(),
                ),
            });
        },
    );
    debug_assert!(
        !relevant_authority.is_empty(),
        "raised-step candidate pairs must retain at least one touching authority"
    );
    Some(RaisedStepPairContributorKey {
        contours,
        // Preserve relevant constraint order: exact-authority selection uses
        // the first lowest-index constraint when malformed inputs tie.
        relevant_authority: Arc::from(relevant_authority),
    })
}

fn raised_step_pair_contributor_keys(
    authority_index: &GeneratedContactAuthorityIndex<'_>,
    summaries: &[GeneratedContactContourSummary],
    pair_indices: &[(usize, usize)],
    contour_contributors: &[Option<Arc<SameMaterialContourContributorKey>>],
) -> BTreeMap<(usize, usize), RaisedStepPairContributorKey> {
    pair_indices
        .iter()
        .copied()
        .filter(|(left_index, right_index)| {
            summaries[*left_index].kind != summaries[*right_index].kind
        })
        .filter_map(|(left_index, right_index)| {
            raised_step_pair_contributor_key(
                authority_index,
                &summaries[left_index],
                &summaries[right_index],
                contour_contributors,
                left_index,
                right_index,
            )
            .map(|key| ((left_index, right_index), key))
        })
        .collect()
}

struct SameBandContactPairResult {
    stats: GeneratedContactEmissionStats,
    contact_edges: BTreeSet<GeneratedSameBandContactConstraint>,
    same_material_height_splits: BTreeSet<SameMaterialHeightSplitConstraint>,
    same_material_pair_cache: NodeSameMaterialContactPairCache,
}

impl Default for SameBandContactPairResult {
    fn default() -> Self {
        Self {
            stats: GeneratedContactEmissionStats::default(),
            contact_edges: BTreeSet::new(),
            same_material_height_splits: BTreeSet::new(),
            same_material_pair_cache: NodeSameMaterialContactPairCache::default(),
        }
    }
}

impl SameBandContactPairResult {
    fn merge(&mut self, mut next: Self) {
        merge_contact_emission_stats(&mut self.stats, next.stats);
        self.contact_edges.append(&mut next.contact_edges);
        self.same_material_height_splits
            .append(&mut next.same_material_height_splits);
        self.same_material_pair_cache
            .entries
            .append(&mut next.same_material_pair_cache.entries);
        self.same_material_pair_cache
            .raised_step_entries
            .append(&mut next.same_material_pair_cache.raised_step_entries);
    }
}

fn collect_same_band_pair_batch_contacts(
    contours: &[NodeGeneratedContour],
    summaries: &[GeneratedContactContourSummary],
    constraints: &[NodeRailConstraint],
    authority_index: &GeneratedContactAuthorityIndex<'_>,
    pair_indices: &[(usize, usize)],
    contour_contributors: &[Option<Arc<SameMaterialContourContributorKey>>],
    raised_step_pair_contributor_keys: &BTreeMap<(usize, usize), RaisedStepPairContributorKey>,
    previous_same_material_pairs: Option<&NodeSameMaterialContactPairCache>,
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
            contour_contributors,
            raised_step_pair_contributor_keys,
            previous_same_material_pairs,
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
    contour_contributors: &[Option<Arc<SameMaterialContourContributorKey>>],
    raised_step_pair_contributor_keys: &BTreeMap<(usize, usize), RaisedStepPairContributorKey>,
    previous_same_material_pairs: Option<&NodeSameMaterialContactPairCache>,
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
        let contributor_key = SameMaterialPairContributorKey::from_indices(
            contour_contributors,
            left_index,
            right_index,
        )
        .expect("same-material candidate pair has band kinds and owners");
        if let Some(cached) =
            previous_same_material_pairs.and_then(|previous| previous.entries.get(&contributor_key))
        {
            result.stats.same_material_pair_cache_hits = 1;
            result
                .same_material_height_splits
                .extend(cached.iter().copied());
            result
                .same_material_pair_cache
                .entries
                .insert(contributor_key, Arc::clone(cached));
            return result;
        }
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
        let contributions = Arc::<[SameMaterialHeightSplitConstraint]>::from(
            result
                .same_material_height_splits
                .iter()
                .copied()
                .collect::<Vec<_>>(),
        );
        result
            .same_material_pair_cache
            .entries
            .insert(contributor_key, contributions);
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
    let contributor_key = raised_step_pair_contributor_keys
        .get(&(left_index, right_index))
        .cloned()
        .expect("raised-step candidate pair has contours, owners, and authority");
    if let Some(cached) = previous_same_material_pairs
        .and_then(|previous| previous.raised_step_entries.get(&contributor_key))
    {
        result.stats.raised_step_pair_cache_previous_hits = 1;
        result.contact_edges.extend(cached.iter().copied());
        result
            .same_material_pair_cache
            .raised_step_entries
            .insert(contributor_key, Arc::clone(cached));
        return result;
    }
    result.stats.raised_step_pair_cache_misses = 1;
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
    let contributions = Arc::<[GeneratedSameBandContactConstraint]>::from(
        result.contact_edges.iter().copied().collect::<Vec<_>>(),
    );
    result
        .same_material_pair_cache
        .raised_step_entries
        .insert(contributor_key, contributions);
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
        if left_summary.overlay_shapes.is_some() && right_summary.overlay_shapes.is_some() {
            return (
                generated_contact_edges_from_source_edges_inside_shape_key_intersection(
                    source_edges,
                    &left_summary.overlay_shape_edges,
                    &left_summary.overlay_shape_keys,
                    &right_summary.overlay_shape_edges,
                    &right_summary.overlay_shape_keys,
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
    stats.same_material_pair_cache_hits += next.same_material_pair_cache_hits;
    stats.raised_step_pair_cache_previous_hits += next.raised_step_pair_cache_previous_hits;
    stats.raised_step_pair_cache_misses += next.raised_step_pair_cache_misses;
    stats.source_target_group_cache_hits += next.source_target_group_cache_hits;
    stats.source_contact_cache_hits += next.source_contact_cache_hits;
    stats.source_contact_cache_misses += next.source_contact_cache_misses;
    stats.source_pair_cache_hits += next.source_pair_cache_hits;
    stats.source_pair_cache_misses += next.source_pair_cache_misses;
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
