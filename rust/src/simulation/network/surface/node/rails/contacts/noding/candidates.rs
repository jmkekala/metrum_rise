//! Contact contour noding candidate discovery.

use super::super::super::NodeGeneratedContourKind;
use super::super::materialization::{
    GeneratedContactAuthorityIndex, generated_contact_point_has_explicit_roles,
};
use super::super::source_authority::generated_raised_step_contact_kind_for_owners;
use super::super::{
    GeneratedContourDirectedEdge, NodeGeneratedContour, NodeGeneratedContourClaimPriority,
    NodeGeneratedContourPurpose, NodeRailConstraint, NodeRailConstraintKind, NodeRailPointKey,
    RoadSurfaceBandKind, generated_contour_band_kind, generated_contour_directed_edges,
    generated_contour_keys, generated_point_key_lies_on_segment,
    quantized_proper_segment_intersection, road_point_key,
};
use super::ContactNodingCandidate;
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

const CONTACT_NODING_BOUNDS_MARGIN_KEYS: i64 = 4096;
const CONTACT_NODING_CANDIDATE_TILE_KEYS: i64 = 8_000_000;
const CONTACT_NODING_PARALLEL_PAIR_THRESHOLD: usize = 32;

/// Immutable pair-local candidate outputs reusable across contact-noding passes and generations.
#[derive(Clone, Debug, Default)]
pub(in crate::simulation::network::surface::node::rails) struct NodeContactNodingPairCache {
    entries: BTreeMap<ContactNodingPairKey, Arc<[ContactNodingPairCandidate]>>,
    active_pair_keys: BTreeSet<ContactNodingPairKey>,
    pub(super) component_entries:
        BTreeMap<ContactNodingComponentKey, Arc<ContactNodingComponentOutput>>,
}

/// Exact candidate- and final-component-cache activity for one noding call.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::simulation::network::surface::node::rails) struct NodeContactNodingReuseStats {
    pub(in crate::simulation::network::surface::node::rails) pair_cache_hits: usize,
    pub(in crate::simulation::network::surface::node::rails) pair_cache_misses: usize,
    pub(in crate::simulation::network::surface::node::rails) component_cache_hits: usize,
    pub(in crate::simulation::network::surface::node::rails) component_cache_misses: usize,
}

impl NodeContactNodingReuseStats {
    pub(super) fn merge(&mut self, other: Self) {
        self.pair_cache_hits += other.pair_cache_hits;
        self.pair_cache_misses += other.pair_cache_misses;
        self.component_cache_hits += other.component_cache_hits;
        self.component_cache_misses += other.component_cache_misses;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum ContactNodingPairSide {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct ContactNodingPairCandidate {
    side: ContactNodingPairSide,
    edge: GeneratedContourDirectedEdge,
    insert_key: NodeRailPointKey,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct ContactNodingContourContributorKey {
    kind: NodeGeneratedContourKind,
    purpose: NodeGeneratedContourPurpose,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
    owner: Option<super::super::NodeBandOwner>,
    claim_priority: NodeGeneratedContourClaimPriority,
    has_height_carrier: bool,
    ordered_points_xz: Arc<[NodeRailPointKey]>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct ContactNodingConstraintFingerprint {
    kind: NodeRailConstraintKind,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
    source_boundary_index: Option<usize>,
    owner: Option<super::super::NodeBandOwner>,
    opposite_owner: Option<super::super::NodeBandOwner>,
    ordered_points_xz: Arc<[NodeRailPointKey]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct ContactNodingBandConstraintSelector {
    kind: RoadSurfaceBandKind,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
    owner: Option<super::super::NodeBandOwner>,
}

#[derive(Clone, Debug)]
pub(super) struct ContactNodingBandConstraintOutput {
    pub(super) selector: ContactNodingBandConstraintSelector,
    pub(super) ordered_points_xz: Arc<[NodeRailPointKey]>,
}

#[derive(Clone, Debug)]
pub(super) struct ContactNodingComponentOutput {
    pub(super) contour_keys: Arc<[Arc<[NodeRailPointKey]>]>,
    pub(super) band_constraints: Arc<[ContactNodingBandConstraintOutput]>,
    pair_entries: Arc<[ContactNodingPairCacheEntry]>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct ContactNodingPairKey {
    left: Arc<ContactNodingContourContributorKey>,
    right: Arc<ContactNodingContourContributorKey>,
    role_constraints: Arc<[ContactNodingConstraintFingerprint]>,
}

#[derive(Clone, Debug)]
struct ContactNodingPairCacheEntry {
    key: ContactNodingPairKey,
    candidates: Arc<[ContactNodingPairCandidate]>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct ContactNodingComponentRoleFingerprint {
    scope: ContactNodingConstraintScope,
    constraints: Arc<[ContactNodingConstraintFingerprint]>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct ContactNodingComponentKey {
    members: Arc<[Arc<ContactNodingContourContributorKey>]>,
    role_constraints: Arc<[ContactNodingComponentRoleFingerprint]>,
}

pub(super) struct ContactNodingComponentPlan {
    pub(super) key: ContactNodingComponentKey,
    pub(super) contour_indices: Arc<[usize]>,
    pub(super) band_constraint_selectors: Arc<[ContactNodingBandConstraintSelector]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum ContactNodingConstraintScope {
    SameBand {
        kind: RoadSurfaceBandKind,
        lower_owner: super::super::NodeBandOwner,
        upper_owner: super::super::NodeBandOwner,
    },
    RaisedStep {
        lower_owner: super::super::NodeBandOwner,
        upper_owner: super::super::NodeBandOwner,
    },
}

struct ContactNodingPairWork {
    left_index: usize,
    right_index: usize,
    key: ContactNodingPairKey,
}

impl NodeContactNodingPairCache {
    pub(super) fn begin_noding_call(&mut self) {
        self.active_pair_keys.clear();
    }

    pub(super) fn promote_component_pair_entries(&mut self, output: &ContactNodingComponentOutput) {
        for entry in output.pair_entries.iter() {
            self.active_pair_keys.insert(entry.key.clone());
            self.entries
                .entry(entry.key.clone())
                .or_insert_with(|| Arc::clone(&entry.candidates));
        }
    }

    fn active_pair_entries_for_component(
        &self,
        selectors: &[ContactNodingBandConstraintSelector],
    ) -> Arc<[ContactNodingPairCacheEntry]> {
        self.active_pair_keys
            .iter()
            .filter(|key| {
                key.left
                    .band_constraint_selector()
                    .is_some_and(|selector| selectors.binary_search(&selector).is_ok())
                    && key
                        .right
                        .band_constraint_selector()
                        .is_some_and(|selector| selectors.binary_search(&selector).is_ok())
            })
            .filter_map(|key| {
                self.entries
                    .get(key)
                    .map(|candidates| ContactNodingPairCacheEntry {
                        key: key.clone(),
                        candidates: Arc::clone(candidates),
                    })
            })
            .collect()
    }

    pub(super) fn component_output(
        &self,
        selectors: &[ContactNodingBandConstraintSelector],
        contour_keys: Arc<[Arc<[NodeRailPointKey]>]>,
        band_constraints: Arc<[ContactNodingBandConstraintOutput]>,
    ) -> Arc<ContactNodingComponentOutput> {
        Arc::new(ContactNodingComponentOutput {
            contour_keys,
            band_constraints,
            pair_entries: self.active_pair_entries_for_component(selectors),
        })
    }
}

impl ContactNodingContourContributorKey {
    fn band_constraint_selector(&self) -> Option<ContactNodingBandConstraintSelector> {
        let NodeGeneratedContourKind::Band { kind } = self.kind else {
            return None;
        };
        Some(ContactNodingBandConstraintSelector {
            kind,
            source_mouth_order_index: self.source_mouth_order_index,
            source_band_index: self.source_band_index,
            owner: self.owner,
        })
    }
}

impl ContactNodingBandConstraintSelector {
    pub(super) fn matches_constraint(&self, constraint: &NodeRailConstraint) -> bool {
        matches!(
            constraint.kind,
            NodeRailConstraintKind::BandContour { kind } if kind == self.kind
        ) && constraint.source_mouth_order_index == self.source_mouth_order_index
            && constraint.source_band_index == self.source_band_index
            && constraint.owner == self.owner
    }
}

impl ContactNodingComponentPlan {
    pub(super) fn affected_band_constraint_selectors(
        &self,
        contours: &[NodeGeneratedContour],
    ) -> Vec<ContactNodingBandConstraintSelector> {
        let mut selectors = self
            .contour_indices
            .iter()
            .zip(self.key.members.iter())
            .filter_map(|(&contour_index, input)| {
                let contour = contours.get(contour_index)?;
                (generated_contour_keys(contour).as_slice() != input.ordered_points_xz.as_ref())
                    .then(|| input.band_constraint_selector())
                    .flatten()
            })
            .collect::<Vec<_>>();
        selectors.sort_unstable();
        selectors.dedup();
        selectors
    }
}

pub(super) fn generated_contact_noding_component_plans(
    contours: &[NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
) -> Vec<ContactNodingComponentPlan> {
    let summaries = contours
        .iter()
        .map(ContactNodingContourSummary::from_contour)
        .collect::<Vec<_>>();
    let pair_indices = contact_noding_candidate_pair_indices(&summaries);
    if pair_indices.is_empty() {
        return Vec::new();
    }

    let mut parents = (0..summaries.len()).collect::<Vec<_>>();
    for &(left_index, right_index) in &pair_indices {
        union_component_members(&mut parents, left_index, right_index);
    }
    let mut first_index_by_selector = BTreeMap::<ContactNodingBandConstraintSelector, usize>::new();
    for (contour_index, summary) in summaries.iter().enumerate() {
        let Some(selector) = summary.band_constraint_selector() else {
            continue;
        };
        if let Some(first_index) = first_index_by_selector.insert(selector, contour_index) {
            union_component_members(&mut parents, first_index, contour_index);
        }
    }

    let mut indices_by_root = BTreeMap::<usize, Vec<usize>>::new();
    let mut pairs_by_scope_by_root =
        BTreeMap::<usize, BTreeMap<ContactNodingConstraintScope, Vec<(usize, usize)>>>::new();
    for &(left_index, right_index) in &pair_indices {
        let root = component_root(&mut parents, left_index);
        let Some(scope) =
            contact_noding_constraint_scope(&summaries[left_index], &summaries[right_index])
        else {
            continue;
        };
        pairs_by_scope_by_root
            .entry(root)
            .or_default()
            .entry(scope)
            .or_default()
            .push((left_index, right_index));
    }
    for contour_index in 0..summaries.len() {
        let root = component_root(&mut parents, contour_index);
        if pairs_by_scope_by_root.contains_key(&root) {
            indices_by_root.entry(root).or_default().push(contour_index);
        }
    }

    let role_constraint_index = ContactNodingRoleConstraintIndex::new(constraints);
    let mut plans = indices_by_root
        .into_iter()
        .map(|(root, mut contour_indices)| {
            contour_indices.sort_by(|left, right| {
                summaries[*left]
                    .contributor_key
                    .cmp(&summaries[*right].contributor_key)
                    .then_with(|| left.cmp(right))
            });
            let members = contour_indices
                .iter()
                .map(|index| Arc::clone(&summaries[*index].contributor_key))
                .collect::<Vec<_>>();
            let role_constraints = pairs_by_scope_by_root
                .remove(&root)
                .unwrap_or_default()
                .into_iter()
                .map(
                    |(scope, pair_indices)| ContactNodingComponentRoleFingerprint {
                        scope,
                        constraints: role_constraint_index.fingerprint_for_pairs(
                            scope,
                            pair_indices
                                .iter()
                                .map(|&(left, right)| (&summaries[left], &summaries[right])),
                        ),
                    },
                )
                .collect::<Vec<_>>();
            let mut band_constraint_selectors = contour_indices
                .iter()
                .filter_map(|index| summaries[*index].band_constraint_selector())
                .collect::<Vec<_>>();
            band_constraint_selectors.sort_unstable();
            band_constraint_selectors.dedup();
            ContactNodingComponentPlan {
                key: ContactNodingComponentKey {
                    members: Arc::from(members),
                    role_constraints: Arc::from(role_constraints),
                },
                contour_indices: Arc::from(contour_indices),
                band_constraint_selectors: Arc::from(band_constraint_selectors),
            }
        })
        .collect::<Vec<_>>();
    plans.sort_by(|left, right| left.key.cmp(&right.key));
    plans
}

fn component_root(parents: &mut [usize], index: usize) -> usize {
    let parent = parents[index];
    if parent == index {
        return index;
    }
    let root = component_root(parents, parent);
    parents[index] = root;
    root
}

fn union_component_members(parents: &mut [usize], left: usize, right: usize) {
    let left_root = component_root(parents, left);
    let right_root = component_root(parents, right);
    if left_root == right_root {
        return;
    }
    let (lower, upper) = if left_root <= right_root {
        (left_root, right_root)
    } else {
        (right_root, left_root)
    };
    parents[upper] = lower;
}

pub(super) fn generated_contact_contour_noding_candidates_with_reuse(
    contours: &[NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
    previous_cache: Option<&NodeContactNodingPairCache>,
    current_cache: &mut NodeContactNodingPairCache,
) -> (Vec<ContactNodingCandidate>, NodeContactNodingReuseStats) {
    let summaries = contours
        .iter()
        .map(ContactNodingContourSummary::from_contour)
        .collect::<Vec<_>>();
    let role_constraint_index = ContactNodingRoleConstraintIndex::new(constraints);
    let pair_work = contact_noding_candidate_pair_indices(&summaries)
        .into_iter()
        .map(|(left_index, right_index)| ContactNodingPairWork {
            left_index,
            right_index,
            key: ContactNodingPairKey {
                left: Arc::clone(&summaries[left_index].contributor_key),
                right: Arc::clone(&summaries[right_index].contributor_key),
                role_constraints: contact_noding_role_constraint_fingerprint(
                    &summaries[left_index],
                    &summaries[right_index],
                    &role_constraint_index,
                ),
            },
        })
        .collect::<Vec<_>>();

    let mut stats = NodeContactNodingReuseStats::default();
    let mut pair_candidates = vec![None; pair_work.len()];
    let mut miss_indices = Vec::new();
    for (pair_index, work) in pair_work.iter().enumerate() {
        current_cache.active_pair_keys.insert(work.key.clone());
        let cached = current_cache.entries.get(&work.key).cloned().or_else(|| {
            previous_cache
                .and_then(|cache| cache.entries.get(&work.key))
                .cloned()
        });
        if let Some(cached) = cached {
            stats.pair_cache_hits += 1;
            current_cache
                .entries
                .entry(work.key.clone())
                .or_insert_with(|| Arc::clone(&cached));
            pair_candidates[pair_index] = Some(cached);
        } else {
            stats.pair_cache_misses += 1;
            miss_indices.push(pair_index);
        }
    }

    if !miss_indices.is_empty() {
        let authority_index = GeneratedContactAuthorityIndex::new(constraints);
        let compute_miss = |pair_index: &usize| {
            let work = &pair_work[*pair_index];
            (
                *pair_index,
                generated_contact_noding_candidates_for_pair(
                    &contours[work.left_index],
                    &summaries[work.left_index],
                    &contours[work.right_index],
                    &summaries[work.right_index],
                    constraints,
                    &authority_index,
                ),
            )
        };
        let computed = if miss_indices.len() >= CONTACT_NODING_PARALLEL_PAIR_THRESHOLD {
            miss_indices
                .par_iter()
                .map(compute_miss)
                .collect::<Vec<_>>()
        } else {
            miss_indices.iter().map(compute_miss).collect::<Vec<_>>()
        };
        for (pair_index, candidates) in computed {
            current_cache
                .entries
                .insert(pair_work[pair_index].key.clone(), Arc::clone(&candidates));
            pair_candidates[pair_index] = Some(candidates);
        }
    }

    let mut candidates = Vec::new();
    for (work, pair_candidates) in pair_work.iter().zip(pair_candidates) {
        let Some(pair_candidates) = pair_candidates else {
            continue;
        };
        candidates.extend(pair_candidates.iter().map(|candidate| {
            let contour_index = match candidate.side {
                ContactNodingPairSide::Left => work.left_index,
                ContactNodingPairSide::Right => work.right_index,
            };
            (contour_index, candidate.edge, candidate.insert_key)
        }));
    }
    candidates.sort_unstable();
    candidates.dedup();
    (candidates, stats)
}

fn generated_contact_noding_candidates_for_pair(
    left: &NodeGeneratedContour,
    left_summary: &ContactNodingContourSummary,
    right: &NodeGeneratedContour,
    right_summary: &ContactNodingContourSummary,
    constraints: &[NodeRailConstraint],
    authority_index: &GeneratedContactAuthorityIndex<'_>,
) -> Arc<[ContactNodingPairCandidate]> {
    let mut candidates = Vec::new();
    candidates.extend(
        generated_contact_point_on_edge_noding_candidates(
            left,
            &left_summary.keys,
            right,
            &right_summary.keys,
            constraints,
            authority_index,
        )
        .into_iter()
        .map(|(edge, insert_key)| ContactNodingPairCandidate {
            side: ContactNodingPairSide::Left,
            edge,
            insert_key,
        }),
    );
    candidates.extend(
        generated_contact_point_on_edge_noding_candidates(
            right,
            &right_summary.keys,
            left,
            &left_summary.keys,
            constraints,
            authority_index,
        )
        .into_iter()
        .map(|(edge, insert_key)| ContactNodingPairCandidate {
            side: ContactNodingPairSide::Right,
            edge,
            insert_key,
        }),
    );
    candidates.extend(
        generated_contact_edge_intersection_noding_candidates(
            left,
            &left_summary.keys,
            right,
            &right_summary.keys,
            constraints,
            authority_index,
        )
        .into_iter()
        .flat_map(|(left_edge, right_edge, insert_key)| {
            [
                ContactNodingPairCandidate {
                    side: ContactNodingPairSide::Left,
                    edge: left_edge,
                    insert_key,
                },
                ContactNodingPairCandidate {
                    side: ContactNodingPairSide::Right,
                    edge: right_edge,
                    insert_key,
                },
            ]
        }),
    );
    candidates.sort_unstable();
    candidates.dedup();
    Arc::from(candidates)
}

fn contact_noding_role_constraint_fingerprint(
    left: &ContactNodingContourSummary,
    right: &ContactNodingContourSummary,
    constraint_index: &ContactNodingRoleConstraintIndex,
) -> Arc<[ContactNodingConstraintFingerprint]> {
    let Some(scope) = contact_noding_constraint_scope(left, right) else {
        return Arc::from([]);
    };
    constraint_index.fingerprint_for_pairs(scope, std::iter::once((left, right)))
}

fn contact_noding_constraint_scope(
    left: &ContactNodingContourSummary,
    right: &ContactNodingContourSummary,
) -> Option<ContactNodingConstraintScope> {
    let (left_owner, right_owner, left_kind, right_kind) =
        (left.owner?, right.owner?, left.kind?, right.kind?);
    let (lower_owner, upper_owner) = if left_owner <= right_owner {
        (left_owner, right_owner)
    } else {
        (right_owner, left_owner)
    };
    Some(if left_kind == right_kind {
        ContactNodingConstraintScope::SameBand {
            kind: left_kind,
            lower_owner,
            upper_owner,
        }
    } else {
        ContactNodingConstraintScope::RaisedStep {
            lower_owner,
            upper_owner,
        }
    })
}

struct ContactNodingRoleConstraintIndex {
    summaries: Vec<ContactNodingConstraintSummary>,
    raised_by_owner_pair:
        BTreeMap<(super::super::NodeBandOwner, super::super::NodeBandOwner), Vec<usize>>,
    raised_by_owner: BTreeMap<super::super::NodeBandOwner, Vec<usize>>,
    sidewalk_seams_global: Vec<usize>,
    sidewalk_seams_by_owner: BTreeMap<super::super::NodeBandOwner, Vec<usize>>,
}

struct ContactNodingConstraintSummary {
    fingerprint: ContactNodingConstraintFingerprint,
    min_x: i64,
    min_z: i64,
    max_x: i64,
    max_z: i64,
}

impl ContactNodingRoleConstraintIndex {
    fn new(constraints: &[NodeRailConstraint]) -> Self {
        let mut index = Self {
            summaries: Vec::new(),
            raised_by_owner_pair: BTreeMap::new(),
            raised_by_owner: BTreeMap::new(),
            sidewalk_seams_global: Vec::new(),
            sidewalk_seams_by_owner: BTreeMap::new(),
        };
        for constraint in constraints {
            let is_relevant_kind = constraint.kind == NodeRailConstraintKind::RaisedStepContact
                || constraint.kind
                    == (NodeRailConstraintKind::FootprintSeam {
                        adjacent_kind: RoadSurfaceBandKind::Sidewalk,
                    });
            if !is_relevant_kind {
                continue;
            }
            let summary_index = index.summaries.len();
            index
                .summaries
                .push(ContactNodingConstraintSummary::from_constraint(constraint));
            match constraint.kind {
                NodeRailConstraintKind::RaisedStepContact => {
                    let (Some(owner), Some(opposite_owner)) =
                        (constraint.owner, constraint.opposite_owner)
                    else {
                        continue;
                    };
                    if owner == opposite_owner {
                        continue;
                    }
                    let pair = ordered_owner_pair(owner, opposite_owner);
                    index
                        .raised_by_owner_pair
                        .entry(pair)
                        .or_default()
                        .push(summary_index);
                    index
                        .raised_by_owner
                        .entry(owner)
                        .or_default()
                        .push(summary_index);
                    index
                        .raised_by_owner
                        .entry(opposite_owner)
                        .or_default()
                        .push(summary_index);
                }
                NodeRailConstraintKind::FootprintSeam {
                    adjacent_kind: RoadSurfaceBandKind::Sidewalk,
                } => {
                    let mut owners = [constraint.owner, constraint.opposite_owner]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>();
                    owners.sort_unstable();
                    owners.dedup();
                    if owners.is_empty() {
                        index.sidewalk_seams_global.push(summary_index);
                    } else {
                        for owner in owners {
                            index
                                .sidewalk_seams_by_owner
                                .entry(owner)
                                .or_default()
                                .push(summary_index);
                        }
                    }
                }
                _ => {}
            }
        }
        index
    }

    fn fingerprint_for_pairs<'a>(
        &self,
        scope: ContactNodingConstraintScope,
        pairs: impl IntoIterator<
            Item = (
                &'a ContactNodingContourSummary,
                &'a ContactNodingContourSummary,
            ),
        >,
    ) -> Arc<[ContactNodingConstraintFingerprint]> {
        let pairs = pairs.into_iter().collect::<Vec<_>>();
        let mut summary_indices = match scope {
            ContactNodingConstraintScope::RaisedStep {
                lower_owner,
                upper_owner,
            } => self
                .raised_by_owner_pair
                .get(&ordered_owner_pair(lower_owner, upper_owner))
                .cloned()
                .unwrap_or_default(),
            ContactNodingConstraintScope::SameBand {
                kind,
                lower_owner,
                upper_owner,
            } => {
                let mut indices = Vec::new();
                if let Some(owner_indices) = self.raised_by_owner.get(&lower_owner) {
                    indices.extend_from_slice(owner_indices);
                }
                if let Some(owner_indices) = self.raised_by_owner.get(&upper_owner) {
                    indices.extend_from_slice(owner_indices);
                }
                if kind == RoadSurfaceBandKind::Sidewalk {
                    indices.extend_from_slice(&self.sidewalk_seams_global);
                    if let Some(owner_indices) = self.sidewalk_seams_by_owner.get(&lower_owner) {
                        indices.extend_from_slice(owner_indices);
                    }
                    if let Some(owner_indices) = self.sidewalk_seams_by_owner.get(&upper_owner) {
                        indices.extend_from_slice(owner_indices);
                    }
                }
                indices
            }
        };
        summary_indices.sort_unstable();
        summary_indices.dedup();

        let mut fingerprint = summary_indices
            .into_iter()
            .filter_map(|index| self.summaries.get(index))
            .filter(|summary| {
                pairs
                    .iter()
                    .any(|(left, right)| summary.bounds_touch_pair(left, right))
            })
            .map(|summary| summary.fingerprint.clone())
            .collect::<Vec<_>>();
        fingerprint.sort_unstable();
        fingerprint.dedup();
        Arc::from(fingerprint)
    }
}

impl ContactNodingConstraintSummary {
    fn from_constraint(constraint: &NodeRailConstraint) -> Self {
        let mut keys = constraint
            .points_xz
            .iter()
            .copied()
            .map(road_point_key)
            .collect::<Vec<_>>();
        let (mut min_x, mut min_z) = (i64::MAX, i64::MAX);
        let (mut max_x, mut max_z) = (i64::MIN, i64::MIN);
        for key in &keys {
            min_x = min_x.min(key.0);
            min_z = min_z.min(key.1);
            max_x = max_x.max(key.0);
            max_z = max_z.max(key.1);
        }
        if keys.is_empty() {
            min_x = 1;
            min_z = 1;
            max_x = 0;
            max_z = 0;
        }
        let fingerprint = ContactNodingConstraintFingerprint {
            kind: constraint.kind,
            source_mouth_order_index: constraint.source_mouth_order_index,
            source_band_index: constraint.source_band_index,
            source_boundary_index: constraint.source_boundary_index,
            owner: constraint.owner,
            opposite_owner: constraint.opposite_owner,
            ordered_points_xz: Arc::from(std::mem::take(&mut keys)),
        };
        Self {
            fingerprint,
            min_x,
            min_z,
            max_x,
            max_z,
        }
    }

    fn bounds_touch_pair(
        &self,
        left: &ContactNodingContourSummary,
        right: &ContactNodingContourSummary,
    ) -> bool {
        self.bounds_valid() && self.bounds_touch_summary(left) && self.bounds_touch_summary(right)
    }

    fn bounds_valid(&self) -> bool {
        self.min_x <= self.max_x && self.min_z <= self.max_z
    }

    fn bounds_touch_summary(&self, summary: &ContactNodingContourSummary) -> bool {
        summary.bounds_valid()
            && self.min_x - CONTACT_NODING_BOUNDS_MARGIN_KEYS <= summary.max_x
            && summary.min_x <= self.max_x + CONTACT_NODING_BOUNDS_MARGIN_KEYS
            && self.min_z - CONTACT_NODING_BOUNDS_MARGIN_KEYS <= summary.max_z
            && summary.min_z <= self.max_z + CONTACT_NODING_BOUNDS_MARGIN_KEYS
    }
}

fn ordered_owner_pair(
    left: super::super::NodeBandOwner,
    right: super::super::NodeBandOwner,
) -> (super::super::NodeBandOwner, super::super::NodeBandOwner) {
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
}

fn contact_noding_candidate_pair_indices(
    summaries: &[ContactNodingContourSummary],
) -> Vec<(usize, usize)> {
    let mut indices_by_tile = BTreeMap::<ContactNodingCandidateTile, Vec<usize>>::new();
    for (summary_index, summary) in summaries.iter().enumerate() {
        if summary.owner.is_none() || !summary.bounds_valid() {
            continue;
        }
        for tile in ContactNodingCandidateTile::tiles_for_summary(summary) {
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

    tile_pairs
        .into_iter()
        .filter(|(left_index, right_index)| {
            contact_noding_summaries_can_contact(&summaries[*left_index], &summaries[*right_index])
        })
        .collect()
}

struct ContactNodingContourSummary {
    owner: Option<super::super::NodeBandOwner>,
    kind: Option<RoadSurfaceBandKind>,
    contributor_key: Arc<ContactNodingContourContributorKey>,
    keys: Vec<NodeRailPointKey>,
    min_x: i64,
    min_z: i64,
    max_x: i64,
    max_z: i64,
}

impl ContactNodingContourSummary {
    fn from_contour(contour: &NodeGeneratedContour) -> Self {
        let ordered_keys = generated_contour_keys(contour);
        let mut keys = ordered_keys.clone();
        keys.sort_unstable();
        keys.dedup();
        let (mut min_x, mut min_z) = (i64::MAX, i64::MAX);
        let (mut max_x, mut max_z) = (i64::MIN, i64::MIN);
        for key in &keys {
            min_x = min_x.min(key.0);
            min_z = min_z.min(key.1);
            max_x = max_x.max(key.0);
            max_z = max_z.max(key.1);
        }
        if keys.is_empty() {
            min_x = 1;
            min_z = 1;
            max_x = 0;
            max_z = 0;
        }
        Self {
            owner: contour.owner,
            kind: generated_contour_band_kind(contour),
            contributor_key: Arc::new(ContactNodingContourContributorKey {
                kind: contour.kind,
                purpose: contour.purpose,
                source_mouth_order_index: contour.source_mouth_order_index,
                source_band_index: contour.source_band_index,
                owner: contour.owner,
                claim_priority: contour.claim_priority,
                has_height_carrier: contour
                    .height_points_world
                    .as_ref()
                    .is_some_and(|points| !points.is_empty()),
                ordered_points_xz: Arc::from(ordered_keys),
            }),
            keys,
            min_x,
            min_z,
            max_x,
            max_z,
        }
    }

    fn bounds_valid(&self) -> bool {
        self.min_x <= self.max_x && self.min_z <= self.max_z
    }

    fn band_constraint_selector(&self) -> Option<ContactNodingBandConstraintSelector> {
        self.contributor_key.band_constraint_selector()
    }

    fn bounds_disjoint(&self, other: &Self) -> bool {
        self.max_x + CONTACT_NODING_BOUNDS_MARGIN_KEYS < other.min_x
            || other.max_x + CONTACT_NODING_BOUNDS_MARGIN_KEYS < self.min_x
            || self.max_z + CONTACT_NODING_BOUNDS_MARGIN_KEYS < other.min_z
            || other.max_z + CONTACT_NODING_BOUNDS_MARGIN_KEYS < self.min_z
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct ContactNodingCandidateTile {
    x: i64,
    z: i64,
}

impl ContactNodingCandidateTile {
    fn tiles_for_summary(summary: &ContactNodingContourSummary) -> Vec<Self> {
        let min_tile_x = (summary.min_x - CONTACT_NODING_BOUNDS_MARGIN_KEYS)
            .div_euclid(CONTACT_NODING_CANDIDATE_TILE_KEYS);
        let max_tile_x = (summary.max_x + CONTACT_NODING_BOUNDS_MARGIN_KEYS)
            .div_euclid(CONTACT_NODING_CANDIDATE_TILE_KEYS);
        let min_tile_z = (summary.min_z - CONTACT_NODING_BOUNDS_MARGIN_KEYS)
            .div_euclid(CONTACT_NODING_CANDIDATE_TILE_KEYS);
        let max_tile_z = (summary.max_z + CONTACT_NODING_BOUNDS_MARGIN_KEYS)
            .div_euclid(CONTACT_NODING_CANDIDATE_TILE_KEYS);
        let mut tiles = Vec::new();
        for x in min_tile_x..=max_tile_x {
            for z in min_tile_z..=max_tile_z {
                tiles.push(Self { x, z });
            }
        }
        tiles
    }
}

fn contact_noding_summaries_can_contact(
    left: &ContactNodingContourSummary,
    right: &ContactNodingContourSummary,
) -> bool {
    let Some(left_owner) = left.owner else {
        return false;
    };
    let Some(right_owner) = right.owner else {
        return false;
    };
    if left_owner == right_owner || left.bounds_disjoint(right) {
        return false;
    }
    generated_raised_step_contact_kind_for_owners(left_owner, right_owner).is_some()
}

fn generated_contact_point_on_edge_noding_candidates(
    edge_contour: &NodeGeneratedContour,
    edge_keys: &[NodeRailPointKey],
    point_contour: &NodeGeneratedContour,
    point_keys: &[NodeRailPointKey],
    constraints: &[NodeRailConstraint],
    authority_index: &GeneratedContactAuthorityIndex<'_>,
) -> Vec<(GeneratedContourDirectedEdge, NodeRailPointKey)> {
    let mut candidates = Vec::new();
    for edge in generated_contour_directed_edges(edge_contour) {
        for point_key in point_keys.iter().copied() {
            if edge_keys.binary_search(&point_key).is_ok()
                || !generated_point_key_lies_on_segment(point_key, edge.start, edge.end)
                || !generated_contact_noding_point_has_explicit_roles(
                    edge_contour,
                    point_contour,
                    constraints,
                    authority_index,
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
    left_keys: &[NodeRailPointKey],
    right: &NodeGeneratedContour,
    right_keys: &[NodeRailPointKey],
    constraints: &[NodeRailConstraint],
    authority_index: &GeneratedContactAuthorityIndex<'_>,
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
            if left_keys.binary_search(&intersection).is_ok()
                && right_keys.binary_search(&intersection).is_ok()
            {
                continue;
            }
            if !generated_contact_noding_point_has_explicit_roles(
                left,
                right,
                constraints,
                authority_index,
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
    authority_index: &GeneratedContactAuthorityIndex<'_>,
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
        authority_index,
        point,
        contact_kind,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::surface::node::arrangement::NodeBandOwner;
    use crate::simulation::network::surface::node::backend::{RoadVec2, RoadVec3};
    use crate::simulation::network::surface::node::rails::contours::push_generated_contour;

    #[test]
    fn pair_cache_reuses_exact_candidates_and_invalidates_semantic_inputs() {
        let (contours, constraints) = crossing_raised_step_contours();
        let mut first_generation = NodeContactNodingPairCache::default();
        let (cold_candidates, cold_stats) = generated_contact_contour_noding_candidates_with_reuse(
            &contours,
            &constraints,
            None,
            &mut first_generation,
        );
        assert!(!cold_candidates.is_empty());
        assert_eq!(cold_stats.pair_cache_hits, 0);
        assert_eq!(cold_stats.pair_cache_misses, 1);
        let cached_sides = first_generation
            .entries
            .values()
            .flat_map(|candidates| candidates.iter().map(|candidate| candidate.side))
            .collect::<BTreeSet<_>>();
        assert_eq!(
            cached_sides,
            BTreeSet::from([ContactNodingPairSide::Left, ContactNodingPairSide::Right,])
        );

        let mut reused_generation = NodeContactNodingPairCache::default();
        let (reused_candidates, reused_stats) =
            generated_contact_contour_noding_candidates_with_reuse(
                &contours,
                &constraints,
                Some(&first_generation),
                &mut reused_generation,
            );
        assert_eq!(reused_candidates, cold_candidates);
        assert_eq!(reused_stats.pair_cache_hits, 1);
        assert_eq!(reused_stats.pair_cache_misses, 0);

        let mut unrelated_constraints = constraints.clone();
        unrelated_constraints.push(NodeRailConstraint {
            constraint_index: unrelated_constraints.len(),
            kind: NodeRailConstraintKind::FullRoadbedContour,
            source_mouth_order_index: 99,
            source_band_index: None,
            source_boundary_index: None,
            owner: None,
            opposite_owner: None,
            points_xz: vec![RoadVec2::new(20.0, 20.0), RoadVec2::new(21.0, 20.0)],
        });
        unrelated_constraints.push(NodeRailConstraint {
            constraint_index: unrelated_constraints.len(),
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index: 99,
            source_band_index: Some(99),
            source_boundary_index: Some(99),
            owner: constraints
                .iter()
                .find_map(|constraint| constraint.owner)
                .or(contours[0].owner),
            opposite_owner: constraints
                .iter()
                .find_map(|constraint| constraint.opposite_owner)
                .or(contours[1].owner),
            points_xz: vec![RoadVec2::new(20.0, 20.0), RoadVec2::new(21.0, 20.0)],
        });
        let mut unrelated_generation = NodeContactNodingPairCache::default();
        let (unrelated_candidates, unrelated_stats) =
            generated_contact_contour_noding_candidates_with_reuse(
                &contours,
                &unrelated_constraints,
                Some(&first_generation),
                &mut unrelated_generation,
            );
        assert_eq!(unrelated_candidates, cold_candidates);
        assert_eq!(unrelated_stats.pair_cache_hits, 1);
        assert_eq!(unrelated_stats.pair_cache_misses, 0);

        let mut changed_constraints = constraints.clone();
        changed_constraints
            .iter_mut()
            .find(|constraint| constraint.kind == NodeRailConstraintKind::RaisedStepContact)
            .expect("raised-step authority")
            .points_xz = vec![RoadVec2::new(1.0, 1.0), RoadVec2::new(3.0, 1.0)];
        let mut changed_constraint_generation = NodeContactNodingPairCache::default();
        let (_, changed_constraint_stats) = generated_contact_contour_noding_candidates_with_reuse(
            &contours,
            &changed_constraints,
            Some(&first_generation),
            &mut changed_constraint_generation,
        );
        assert_eq!(changed_constraint_stats.pair_cache_hits, 0);
        assert_eq!(changed_constraint_stats.pair_cache_misses, 1);

        let mut changed_contours = contours.clone();
        changed_contours[0].points_xz.push(RoadVec2::new(0.0, 2.0));
        let mut changed_contour_generation = NodeContactNodingPairCache::default();
        let (_, changed_contour_stats) = generated_contact_contour_noding_candidates_with_reuse(
            &changed_contours,
            &constraints,
            Some(&first_generation),
            &mut changed_contour_generation,
        );
        assert_eq!(changed_contour_stats.pair_cache_hits, 0);
        assert_eq!(changed_contour_stats.pair_cache_misses, 1);
    }

    #[test]
    fn component_cache_replays_final_noded_output_and_invalidates_role_changes() {
        let (base_contours, base_constraints) = crossing_raised_step_contours();
        let mut cold_contours = base_contours.clone();
        let mut cold_constraints = base_constraints.clone();
        let mut first_generation = NodeContactNodingPairCache::default();
        let cold_stats = super::super::node_generated_contact_contours_with_reuse(
            &mut cold_contours,
            &mut cold_constraints,
            None,
            &mut first_generation,
        )
        .expect("cold contact noding");
        let cold_keys = cold_contours
            .iter()
            .map(generated_contour_keys)
            .collect::<Vec<_>>();
        assert_eq!(cold_stats.component_cache_hits, 0);
        assert_eq!(cold_stats.component_cache_misses, 1);
        assert!(cold_keys[0].len() > generated_contour_keys(&base_contours[0]).len());

        let mut reused_contours = base_contours.clone();
        for point in reused_contours[0]
            .height_points_world
            .as_mut()
            .expect("height carrier")
        {
            point.y += 100.0;
        }
        let mut reused_constraints = base_constraints.clone();
        let mut reused_generation = NodeContactNodingPairCache::default();
        let reused_stats = super::super::node_generated_contact_contours_with_reuse(
            &mut reused_contours,
            &mut reused_constraints,
            Some(&first_generation),
            &mut reused_generation,
        )
        .expect("reused contact noding");
        assert_eq!(reused_stats.component_cache_hits, 1);
        assert_eq!(reused_stats.component_cache_misses, 0);
        assert_eq!(reused_stats.pair_cache_hits, 0);
        assert_eq!(reused_stats.pair_cache_misses, 0);
        assert_eq!(
            reused_contours
                .iter()
                .map(generated_contour_keys)
                .collect::<Vec<_>>(),
            cold_keys
        );
        let reused_heights = reused_contours[0]
            .height_points_world
            .as_ref()
            .expect("replayed height carrier");
        assert_eq!(reused_heights.len(), cold_keys[0].len());
        assert!(reused_heights.iter().all(|point| point.y >= 100.0));

        let mut unrelated_constraints = base_constraints.clone();
        unrelated_constraints.push(NodeRailConstraint {
            constraint_index: unrelated_constraints.len(),
            kind: NodeRailConstraintKind::FullRoadbedContour,
            source_mouth_order_index: 99,
            source_band_index: None,
            source_boundary_index: None,
            owner: None,
            opposite_owner: None,
            points_xz: vec![RoadVec2::new(20.0, 20.0), RoadVec2::new(21.0, 20.0)],
        });
        unrelated_constraints.push(NodeRailConstraint {
            constraint_index: unrelated_constraints.len(),
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index: 99,
            source_band_index: Some(99),
            source_boundary_index: Some(99),
            owner: base_constraints
                .iter()
                .find_map(|constraint| constraint.owner)
                .or(base_contours[0].owner),
            opposite_owner: base_constraints
                .iter()
                .find_map(|constraint| constraint.opposite_owner)
                .or(base_contours[1].owner),
            points_xz: vec![RoadVec2::new(20.0, 20.0), RoadVec2::new(21.0, 20.0)],
        });
        let mut unrelated_contours = base_contours.clone();
        let mut unrelated_generation = NodeContactNodingPairCache::default();
        let unrelated_stats = super::super::node_generated_contact_contours_with_reuse(
            &mut unrelated_contours,
            &mut unrelated_constraints,
            Some(&first_generation),
            &mut unrelated_generation,
        )
        .expect("unrelated constraint reuse");
        assert_eq!(unrelated_stats.component_cache_hits, 1);
        assert_eq!(
            unrelated_contours
                .iter()
                .map(generated_contour_keys)
                .collect::<Vec<_>>(),
            cold_keys
        );

        let mut changed_constraints = base_constraints.clone();
        changed_constraints
            .iter_mut()
            .find(|constraint| constraint.kind == NodeRailConstraintKind::RaisedStepContact)
            .expect("raised-step authority")
            .points_xz = vec![RoadVec2::new(1.0, 1.0), RoadVec2::new(3.0, 1.0)];
        let mut changed_contours = base_contours.clone();
        let mut changed_generation = NodeContactNodingPairCache::default();
        let changed_stats = super::super::node_generated_contact_contours_with_reuse(
            &mut changed_contours,
            &mut changed_constraints,
            Some(&first_generation),
            &mut changed_generation,
        )
        .expect("changed role constraint fallback");
        assert_eq!(changed_stats.component_cache_hits, 0);
        assert_eq!(changed_stats.component_cache_misses, 1);

        let mut changed_cold_contours = base_contours.clone();
        let mut changed_cold_constraints = base_constraints.clone();
        changed_cold_constraints
            .iter_mut()
            .find(|constraint| constraint.kind == NodeRailConstraintKind::RaisedStepContact)
            .expect("raised-step authority")
            .points_xz = vec![RoadVec2::new(1.0, 1.0), RoadVec2::new(3.0, 1.0)];
        let mut changed_cold_generation = NodeContactNodingPairCache::default();
        super::super::node_generated_contact_contours_with_reuse(
            &mut changed_cold_contours,
            &mut changed_cold_constraints,
            None,
            &mut changed_cold_generation,
        )
        .expect("changed cold noding");
        assert_eq!(
            changed_contours
                .iter()
                .map(generated_contour_keys)
                .collect::<Vec<_>>(),
            changed_cold_contours
                .iter()
                .map(generated_contour_keys)
                .collect::<Vec<_>>()
        );

        let mut removed_contours = vec![base_contours[0].clone()];
        let removed_keys = generated_contour_keys(&removed_contours[0]);
        let mut removed_constraints = base_constraints.clone();
        let mut removed_generation = NodeContactNodingPairCache::default();
        let removed_stats = super::super::node_generated_contact_contours_with_reuse(
            &mut removed_contours,
            &mut removed_constraints,
            Some(&first_generation),
            &mut removed_generation,
        )
        .expect("removed contributor noding");
        assert_eq!(removed_stats.component_cache_hits, 0);
        assert_eq!(removed_stats.component_cache_misses, 0);
        assert_eq!(generated_contour_keys(&removed_contours[0]), removed_keys);
    }

    #[test]
    fn component_cache_replays_shared_band_selector_constraints_exactly() {
        let (base_contours, base_constraints) = shared_selector_contact_contours();
        let mut cold_contours = base_contours.clone();
        let mut cold_constraints = base_constraints.clone();
        let mut first_generation = NodeContactNodingPairCache::default();
        super::super::node_generated_contact_contours_with_reuse(
            &mut cold_contours,
            &mut cold_constraints,
            None,
            &mut first_generation,
        )
        .expect("cold shared-selector contact noding");

        let shared_owner = base_contours[0].owner;
        let shared_constraints = cold_constraints
            .iter()
            .filter(|constraint| {
                matches!(
                    constraint.kind,
                    NodeRailConstraintKind::BandContour {
                        kind: RoadSurfaceBandKind::Carriageway,
                    }
                ) && constraint.source_mouth_order_index == 0
                    && constraint.source_band_index == Some(0)
                    && constraint.owner == shared_owner
            })
            .collect::<Vec<_>>();
        assert_eq!(shared_constraints.len(), 2);
        assert_eq!(
            constraint_keys(shared_constraints[0]),
            constraint_keys(shared_constraints[1])
        );
        assert_eq!(
            constraint_keys(shared_constraints[0]),
            generated_contour_keys(&cold_contours[2])
        );

        let mut warm_contours = base_contours;
        let mut warm_constraints = base_constraints;
        let mut warm_generation = NodeContactNodingPairCache::default();
        let warm_stats = super::super::node_generated_contact_contours_with_reuse(
            &mut warm_contours,
            &mut warm_constraints,
            Some(&first_generation),
            &mut warm_generation,
        )
        .expect("warm shared-selector contact noding");
        assert_eq!(warm_stats.component_cache_hits, 1);
        assert_eq!(
            warm_contours
                .iter()
                .map(generated_contour_keys)
                .collect::<Vec<_>>(),
            cold_contours
                .iter()
                .map(generated_contour_keys)
                .collect::<Vec<_>>()
        );
        assert_constraints_equal(&warm_constraints, &cold_constraints);
    }

    #[test]
    fn full_component_hit_promotes_unchanged_pairs_for_partial_generation() {
        let (base_contours, base_constraints) = shared_selector_contact_contours();
        let mut cold_contours = base_contours.clone();
        let mut cold_constraints = base_constraints.clone();
        let mut first_generation = NodeContactNodingPairCache::default();
        super::super::node_generated_contact_contours_with_reuse(
            &mut cold_contours,
            &mut cold_constraints,
            None,
            &mut first_generation,
        )
        .expect("cold promoted-pair source");

        let mut warm_contours = base_contours.clone();
        let mut warm_constraints = base_constraints.clone();
        let mut promoted_generation = NodeContactNodingPairCache::default();
        let warm_stats = super::super::node_generated_contact_contours_with_reuse(
            &mut warm_contours,
            &mut warm_constraints,
            Some(&first_generation),
            &mut promoted_generation,
        )
        .expect("full component hit");
        assert_eq!(warm_stats.component_cache_hits, 1);
        assert!(!promoted_generation.entries.is_empty());

        let mut changed_contours = base_contours;
        changed_contours[2].points_xz[0].x -= 0.5;
        changed_contours[2].points_xz[3].x -= 0.5;
        changed_contours[2].backend_polyline =
            crate::simulation::network::surface::node::rails::contours::cleaned_closed_contour(
                changed_contours[2].kind,
                changed_contours[2].source_mouth_order_index,
                changed_contours[2].source_band_index,
                changed_contours[2].points_xz.clone(),
            )
            .expect("changed partial contour");
        let mut changed_constraints = base_constraints;
        let mut changed_generation = NodeContactNodingPairCache::default();
        let changed_stats = super::super::node_generated_contact_contours_with_reuse(
            &mut changed_contours,
            &mut changed_constraints,
            Some(&promoted_generation),
            &mut changed_generation,
        )
        .expect("partial generation");
        assert_eq!(changed_stats.component_cache_hits, 0);
        assert_eq!(changed_stats.component_cache_misses, 1);
        assert!(changed_stats.pair_cache_hits >= 1);
        assert!(changed_stats.pair_cache_misses >= 1);
    }

    fn assert_constraints_equal(left: &[NodeRailConstraint], right: &[NodeRailConstraint]) {
        assert_eq!(left.len(), right.len());
        for (left, right) in left.iter().zip(right) {
            assert_eq!(left.constraint_index, right.constraint_index);
            assert_eq!(left.kind, right.kind);
            assert_eq!(
                left.source_mouth_order_index,
                right.source_mouth_order_index
            );
            assert_eq!(left.source_band_index, right.source_band_index);
            assert_eq!(left.source_boundary_index, right.source_boundary_index);
            assert_eq!(left.owner, right.owner);
            assert_eq!(left.opposite_owner, right.opposite_owner);
            assert_eq!(constraint_keys(left), constraint_keys(right));
        }
    }

    fn constraint_keys(constraint: &NodeRailConstraint) -> Vec<NodeRailPointKey> {
        constraint
            .points_xz
            .iter()
            .copied()
            .map(road_point_key)
            .collect()
    }

    fn shared_selector_contact_contours() -> (Vec<NodeGeneratedContour>, Vec<NodeRailConstraint>) {
        fn push_carriageway(
            owner: NodeBandOwner,
            points: Vec<RoadVec2>,
            contours: &mut Vec<NodeGeneratedContour>,
            constraints: &mut Vec<NodeRailConstraint>,
        ) {
            push_generated_contour(
                NodeGeneratedContourKind::Band {
                    kind: RoadSurfaceBandKind::Carriageway,
                },
                0,
                Some(0),
                Some(owner),
                NodeGeneratedContourClaimPriority::MouthBand,
                NodeRailConstraintKind::BandContour {
                    kind: RoadSurfaceBandKind::Carriageway,
                },
                points,
                None,
                contours,
                constraints,
            )
            .expect("shared-selector carriageway contour");
        }

        let carriageway_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let mut contours = Vec::new();
        let mut constraints = Vec::new();
        push_carriageway(
            carriageway_owner,
            vec![
                RoadVec2::new(10.0, 0.0),
                RoadVec2::new(14.0, 0.0),
                RoadVec2::new(14.0, 4.0),
                RoadVec2::new(10.0, 4.0),
            ],
            &mut contours,
            &mut constraints,
        );
        push_generated_contour(
            NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::CurbOrShoulder,
            },
            1,
            Some(0),
            Some(curb_owner),
            NodeGeneratedContourClaimPriority::MouthBand,
            NodeRailConstraintKind::BandContour {
                kind: RoadSurfaceBandKind::CurbOrShoulder,
            },
            vec![
                RoadVec2::new(2.0, -2.0),
                RoadVec2::new(12.0, -2.0),
                RoadVec2::new(12.0, 2.0),
                RoadVec2::new(2.0, 2.0),
            ],
            None,
            &mut contours,
            &mut constraints,
        )
        .expect("shared-selector curb contour");
        push_carriageway(
            carriageway_owner,
            vec![
                RoadVec2::new(0.0, 0.0),
                RoadVec2::new(4.0, 0.0),
                RoadVec2::new(4.0, 4.0),
                RoadVec2::new(0.0, 4.0),
            ],
            &mut contours,
            &mut constraints,
        );
        constraints.push(NodeRailConstraint {
            constraint_index: constraints.len(),
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index: 0,
            source_band_index: Some(0),
            source_boundary_index: Some(0),
            owner: Some(carriageway_owner),
            opposite_owner: Some(curb_owner),
            points_xz: vec![RoadVec2::new(1.0, 0.0), RoadVec2::new(13.0, 0.0)],
        });
        (contours, constraints)
    }

    fn crossing_raised_step_contours() -> (Vec<NodeGeneratedContour>, Vec<NodeRailConstraint>) {
        let carriageway_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let mut contours = Vec::new();
        let mut constraints = Vec::new();
        push_generated_contour(
            NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::Carriageway,
            },
            0,
            Some(0),
            Some(carriageway_owner),
            NodeGeneratedContourClaimPriority::MouthBand,
            NodeRailConstraintKind::BandContour {
                kind: RoadSurfaceBandKind::Carriageway,
            },
            vec![
                RoadVec2::new(0.0, 0.0),
                RoadVec2::new(4.0, 0.0),
                RoadVec2::new(4.0, 4.0),
                RoadVec2::new(0.0, 4.0),
            ],
            Some(vec![
                RoadVec3::new(0.0, 0.0, 0.0),
                RoadVec3::new(4.0, 4.0, 0.0),
                RoadVec3::new(4.0, 8.0, 4.0),
                RoadVec3::new(0.0, 4.0, 4.0),
            ]),
            &mut contours,
            &mut constraints,
        )
        .expect("carriageway contour");
        push_generated_contour(
            NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::CurbOrShoulder,
            },
            1,
            Some(0),
            Some(curb_owner),
            NodeGeneratedContourClaimPriority::MouthBand,
            NodeRailConstraintKind::BandContour {
                kind: RoadSurfaceBandKind::CurbOrShoulder,
            },
            vec![
                RoadVec2::new(2.0, -2.0),
                RoadVec2::new(6.0, -2.0),
                RoadVec2::new(6.0, 2.0),
                RoadVec2::new(2.0, 2.0),
            ],
            None,
            &mut contours,
            &mut constraints,
        )
        .expect("curb contour");
        constraints.push(NodeRailConstraint {
            constraint_index: constraints.len(),
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index: 0,
            source_band_index: Some(0),
            source_boundary_index: Some(0),
            owner: Some(carriageway_owner),
            opposite_owner: Some(curb_owner),
            points_xz: vec![RoadVec2::new(1.0, 0.0), RoadVec2::new(3.0, 0.0)],
        });
        (contours, constraints)
    }
}
