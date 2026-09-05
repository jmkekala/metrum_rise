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
const SAME_BAND_PARALLEL_CONTOUR_THRESHOLD: usize = 32;

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
    let mut contact_edges = Vec::<GeneratedSameBandContactConstraint>::new();
    let mut same_material_height_splits = Vec::<SameMaterialHeightSplitConstraint>::new();
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
    contact_edges.sort_unstable();
    contact_edges.dedup();
    same_material_height_splits.sort_unstable();
    same_material_height_splits.dedup();
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

    super::retain_new_sorted_generated_contacts(&mut contact_edges, constraints);
    for contact in contact_edges {
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
    let mut required_indices = vec![false; summaries.len()];
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
                            .is_some_and(|previous| previous.raised_step_entries.contains_key(key))
                    })
            };
        if pair_is_cached {
            continue;
        }
        required_indices[left_index] = true;
        required_indices[right_index] = true;
    }
    let required_indices = required_indices
        .into_iter()
        .enumerate()
        .filter_map(|(index, required)| required.then_some(index))
        .collect::<Vec<_>>();
    let build_summary =
        |&index: &usize| (index, generated_contour_overlay_shapes(&contours[index]));
    let overlay_summaries = if required_indices.len() >= SAME_BAND_PARALLEL_CONTOUR_THRESHOLD {
        required_indices
            .par_iter()
            .map(build_summary)
            .collect::<Vec<_>>()
    } else {
        required_indices
            .iter()
            .map(build_summary)
            .collect::<Vec<_>>()
    };
    for (index, overlay_shapes) in overlay_summaries {
        summaries[index].replace_overlay_shapes(overlay_shapes);
    }
}

#[derive(Default)]
struct SameBandCandidatePairIndex {
    stats: GeneratedContactEmissionStats,
    pair_indices: Vec<(usize, usize)>,
}

fn same_band_candidate_pair_index(
    summaries: &[GeneratedContactContourSummary],
    authority_index: &GeneratedContactAuthorityIndex,
) -> SameBandCandidatePairIndex {
    let mut index = SameBandCandidatePairIndex::default();
    index.stats.pair_tests = summaries
        .len()
        .saturating_mul(summaries.len().saturating_sub(1))
        / 2;

    let tile_pairs = generated_contact_candidate_pair_indices(summaries);
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

fn same_band_candidate_pair_can_contact(
    left_summary: &GeneratedContactContourSummary,
    right_summary: &GeneratedContactContourSummary,
    authority_index: &GeneratedContactAuthorityIndex,
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
    authority_index: &GeneratedContactAuthorityIndex,
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
                ordered_points_xz: Arc::from(constraint.ordered_keys.as_slice()),
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
    authority_index: &GeneratedContactAuthorityIndex,
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

#[derive(Default)]
struct SameBandContactPairResult {
    stats: GeneratedContactEmissionStats,
    contact_edges: Vec<GeneratedSameBandContactConstraint>,
    same_material_height_splits: Vec<SameMaterialHeightSplitConstraint>,
    same_material_pair_cache: NodeSameMaterialContactPairCache,
}

#[derive(Default)]
struct SameBandPairGeometryScratch {
    overlay: GeneratedContactOverlayScratch,
    edges: Vec<GeneratedContourEdgeKey>,
    edge_points: Vec<NodeRailPointKey>,
    points: Vec<NodeRailPointKey>,
    split_keys: Vec<NodeRailPointKey>,
}

impl SameBandContactPairResult {
    fn merge(&mut self, next: &mut Self) {
        merge_contact_emission_stats(&mut self.stats, std::mem::take(&mut next.stats));
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
    authority_index: &GeneratedContactAuthorityIndex,
    pair_indices: &[(usize, usize)],
    contour_contributors: &[Option<Arc<SameMaterialContourContributorKey>>],
    raised_step_pair_contributor_keys: &BTreeMap<(usize, usize), RaisedStepPairContributorKey>,
    previous_same_material_pairs: Option<&NodeSameMaterialContactPairCache>,
) -> SameBandContactPairResult {
    let mut batch_result = SameBandContactPairResult::default();
    let mut pair_result = SameBandContactPairResult::default();
    let mut geometry_scratch = SameBandPairGeometryScratch::default();
    for &(left_index, right_index) in pair_indices {
        collect_same_band_pair_contacts(
            contours,
            summaries,
            constraints,
            authority_index,
            left_index,
            right_index,
            contour_contributors,
            raised_step_pair_contributor_keys,
            previous_same_material_pairs,
            &mut geometry_scratch,
            &mut pair_result,
        );
        batch_result.merge(&mut pair_result);
    }
    batch_result
}

fn collect_same_band_pair_contacts(
    contours: &[NodeGeneratedContour],
    summaries: &[GeneratedContactContourSummary],
    constraints: &[NodeRailConstraint],
    authority_index: &GeneratedContactAuthorityIndex,
    left_index: usize,
    right_index: usize,
    contour_contributors: &[Option<Arc<SameMaterialContourContributorKey>>],
    raised_step_pair_contributor_keys: &BTreeMap<(usize, usize), RaisedStepPairContributorKey>,
    previous_same_material_pairs: Option<&NodeSameMaterialContactPairCache>,
    geometry_scratch: &mut SameBandPairGeometryScratch,
    result: &mut SameBandContactPairResult,
) {
    debug_assert!(result.contact_edges.is_empty());
    debug_assert!(result.same_material_height_splits.is_empty());
    debug_assert!(result.same_material_pair_cache.entries.is_empty());
    debug_assert!(
        result
            .same_material_pair_cache
            .raised_step_entries
            .is_empty()
    );
    let left = &contours[left_index];
    let right = &contours[right_index];
    let left_summary = &summaries[left_index];
    let right_summary = &summaries[right_index];
    let Some(left_owner) = left_summary.owner else {
        result.stats.kind_rejected = 1;
        return;
    };
    let Some(right_owner) = right_summary.owner else {
        result.stats.kind_rejected = 1;
        return;
    };
    if left_owner == right_owner {
        result.stats.kind_rejected = 1;
        return;
    }
    let Some(kind) = left_summary.kind else {
        result.stats.kind_rejected = 1;
        return;
    };
    let Some(right_kind) = right_summary.kind else {
        result.stats.kind_rejected = 1;
        return;
    };
    if left_summary.aabb_disjoint(right_summary) {
        result.stats.aabb_rejected = 1;
        return;
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
            return;
        }
        result.stats.same_material_overlay_calls = 1;
        collect_same_material_height_splits_from_edges(
            left,
            right,
            left_summary,
            right_summary,
            left_summary.overlay_shapes.as_ref(),
            right_summary.overlay_shapes.as_ref(),
            left_owner,
            right_owner,
            &mut result.same_material_height_splits,
            geometry_scratch,
        );
        result.same_material_height_splits.sort_unstable();
        result.same_material_height_splits.dedup();
        let contributions = Arc::<[SameMaterialHeightSplitConstraint]>::from(
            result.same_material_height_splits.clone(),
        );
        result
            .same_material_pair_cache
            .entries
            .insert(contributor_key, contributions);
        return;
    }
    let Some(contact_kind) = generated_raised_step_contact_kind_for_owners(left_owner, right_owner)
    else {
        result.stats.kind_rejected = 1;
        return;
    };
    let Some(pair) = GeneratedRaisedStepOwnerPair::new(left_owner, right_owner) else {
        result.stats.kind_rejected = 1;
        return;
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
        return;
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
    let used_pair_overlay = generated_raised_step_contact_edges_from_authority(
        left,
        right,
        left_summary,
        right_summary,
        &source_edges,
        geometry_scratch,
    );
    if used_pair_overlay {
        result.stats.overlay_calls = 1;
    }
    geometry_scratch.edge_points.clear();
    geometry_scratch.edge_points.extend(
        geometry_scratch
            .edges
            .iter()
            .flat_map(|edge| [edge.start, edge.end]),
    );
    geometry_scratch.edge_points.sort_unstable();
    geometry_scratch.edge_points.dedup();
    for &edge in &geometry_scratch.edges {
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
    geometry_scratch.points.clear();
    append_shared_sorted_keys(
        &left_summary.keys,
        &right_summary.keys,
        &mut geometry_scratch.points,
    );
    append_generated_contact_points_from_summary_intersections(
        left_summary,
        right_summary,
        &mut geometry_scratch.points,
    );
    geometry_scratch.points.sort_unstable();
    geometry_scratch.points.dedup();
    for &point in &geometry_scratch.points {
        if geometry_scratch.edge_points.binary_search(&point).is_ok() {
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
            .push(GeneratedSameBandContactConstraint {
                kind: contact_kind,
                owner: pair.owner,
                opposite_owner: pair.opposite_owner,
                start: point,
                end: point,
                source_mouth_order_index: source.source_mouth_order_index,
                source_band_index: source.source_band_index,
            });
    }
    result.contact_edges.sort_unstable();
    result.contact_edges.dedup();
    let contributions =
        Arc::<[GeneratedSameBandContactConstraint]>::from(result.contact_edges.clone());
    result
        .same_material_pair_cache
        .raised_step_entries
        .insert(contributor_key, contributions);
}

fn generated_raised_step_contact_edges_from_authority(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    left_summary: &GeneratedContactContourSummary,
    right_summary: &GeneratedContactContourSummary,
    source_edges: &[GeneratedContourDirectedEdge],
    geometry_scratch: &mut SameBandPairGeometryScratch,
) -> bool {
    geometry_scratch.edges.clear();
    if !source_edges.is_empty()
        && left_summary.overlay_shapes.is_some()
        && right_summary.overlay_shapes.is_some()
    {
        geometry_scratch.edges.extend(
            generated_contact_edges_from_source_edges_inside_shape_key_intersection(
                source_edges,
                &left_summary.overlay_shape_edges,
                &left_summary.overlay_shape_keys,
                &right_summary.overlay_shape_edges,
                &right_summary.overlay_shape_keys,
            ),
        );
        return false;
    }
    append_shared_sorted_edges(
        &left_summary.edges,
        &right_summary.edges,
        &mut geometry_scratch.edges,
    );
    append_generated_contact_edges_inside_summary(
        left_summary,
        right_summary,
        &mut geometry_scratch.edges,
        &mut geometry_scratch.split_keys,
    );
    append_generated_contact_edges_inside_summary(
        right_summary,
        left_summary,
        &mut geometry_scratch.edges,
        &mut geometry_scratch.split_keys,
    );
    geometry_scratch
        .edges
        .extend_from_slice(generated_contact_edges_from_summary_overlay(
            left,
            right,
            left_summary,
            right_summary,
            left_summary.overlay_shapes.as_ref(),
            right_summary.overlay_shapes.as_ref(),
            &mut geometry_scratch.overlay,
        ));
    geometry_scratch.edges.sort_unstable();
    geometry_scratch.edges.dedup();
    true
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
    left_summary: &GeneratedContactContourSummary,
    right_summary: &GeneratedContactContourSummary,
    left_shapes: Option<&NodeOverlayShapes>,
    right_shapes: Option<&NodeOverlayShapes>,
    left_owner: NodeBandOwner,
    right_owner: NodeBandOwner,
    contacts: &mut Vec<SameMaterialHeightSplitConstraint>,
    geometry_scratch: &mut SameBandPairGeometryScratch,
) {
    geometry_scratch.edges.clear();
    append_shared_sorted_edges(
        &left_summary.edges,
        &right_summary.edges,
        &mut geometry_scratch.edges,
    );
    for &edge in &geometry_scratch.edges {
        insert_same_material_height_split(
            contacts,
            left_owner,
            right_owner,
            edge.start,
            edge.end,
            left.source_mouth_order_index,
            left.source_band_index,
        );
    }
    let left_inside_start = geometry_scratch.edges.len();
    append_generated_contact_edges_inside_summary(
        left_summary,
        right_summary,
        &mut geometry_scratch.edges,
        &mut geometry_scratch.split_keys,
    );
    for &edge in &geometry_scratch.edges[left_inside_start..] {
        insert_same_material_height_split(
            contacts,
            left_owner,
            right_owner,
            edge.start,
            edge.end,
            left.source_mouth_order_index,
            left.source_band_index,
        );
    }
    let right_inside_start = geometry_scratch.edges.len();
    append_generated_contact_edges_inside_summary(
        right_summary,
        left_summary,
        &mut geometry_scratch.edges,
        &mut geometry_scratch.split_keys,
    );
    for &edge in &geometry_scratch.edges[right_inside_start..] {
        insert_same_material_height_split(
            contacts,
            left_owner,
            right_owner,
            edge.start,
            edge.end,
            right.source_mouth_order_index,
            right.source_band_index,
        );
    }
    for &edge in generated_contact_edges_from_summary_overlay(
        left,
        right,
        left_summary,
        right_summary,
        left_shapes,
        right_shapes,
        &mut geometry_scratch.overlay,
    ) {
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
        geometry_scratch.edges.push(edge);
    }
    geometry_scratch.edge_points.clear();
    geometry_scratch.edge_points.extend(
        geometry_scratch
            .edges
            .iter()
            .flat_map(|edge| [edge.start, edge.end]),
    );
    geometry_scratch.edge_points.sort_unstable();
    geometry_scratch.edge_points.dedup();
    geometry_scratch.points.clear();
    append_shared_sorted_keys(
        &left_summary.keys,
        &right_summary.keys,
        &mut geometry_scratch.points,
    );
    append_generated_contact_points_from_summary_intersections(
        left_summary,
        right_summary,
        &mut geometry_scratch.points,
    );
    geometry_scratch.points.sort_unstable();
    geometry_scratch.points.dedup();
    for &point in &geometry_scratch.points {
        if geometry_scratch.edge_points.binary_search(&point).is_ok() {
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

fn generated_contact_edges_from_summary_overlay<'a>(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    left_summary: &GeneratedContactContourSummary,
    right_summary: &GeneratedContactContourSummary,
    left_shapes: Option<&NodeOverlayShapes>,
    right_shapes: Option<&NodeOverlayShapes>,
    scratch: &'a mut GeneratedContactOverlayScratch,
) -> &'a [GeneratedContourEdgeKey] {
    let edges = match (left_shapes, right_shapes) {
        (Some(left_shapes), Some(right_shapes)) => {
            if generated_contact_edges_from_overlay_shape_key_intersection(
                &left_summary.overlay_shape_keys,
                &right_summary.overlay_shape_keys,
                scratch,
            ) {
                return scratch.edges();
            }
            generated_contact_edges_from_overlay_shape_intersection(left_shapes, right_shapes)
        }
        _ => generated_contact_edges_from_overlay_intersection(left, right),
    };
    scratch.replace_edges(edges);
    scratch.edges()
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
    contacts: &mut Vec<SameMaterialHeightSplitConstraint>,
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
    contacts.push((
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
    contacts: Vec<SameMaterialHeightSplitConstraint>,
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
    contact_edges: &mut Vec<GeneratedSameBandContactConstraint>,
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
        contact_edges.push(GeneratedSameBandContactConstraint {
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
