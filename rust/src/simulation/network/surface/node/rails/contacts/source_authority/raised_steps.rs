//! Source-authorized raised-step contact collection.

use super::super::geometry::{
    generated_contour_boundary_contains_key, generated_directed_edge_segments_inside_shape_edges,
    generated_shape_boundary_segments_on_source_edge,
};
use super::super::{
    GeneratedContourDirectedEdge, GeneratedRaisedStepOwnerPair, NodeBandOwner,
    NodeGeneratedContour, NodeGeneratedContourClaimPriority, NodeRailConstraint,
    NodeRailConstraintKind, NodeRailPointKey, RoadSurfaceVisualNodePieceKind,
    generated_constraint_directed_edges, generated_constraint_touches_key,
    generated_contour_band_kind, generated_contour_keys, generated_point_key_lies_on_segment,
    quantized_proper_segment_intersection, road_point_key,
};
use super::target_groups::{
    SourceAuthorizedTargetGroupView, collect_source_authorized_exact_group_pair_overlap_contacts,
    source_authorized_contact_segments, source_authorized_raised_step_target_pairs,
    source_authorized_target_claim_priorities, source_authorized_target_group,
};
use super::types::{
    GeneratedRaisedStepEndpointSource, GeneratedSameBandContactConstraint,
    RaisedStepSourceAuthority, RaisedStepSourceConstraint, SOURCE_AUTHORITY_BOUNDS_MARGIN_KEYS,
    SourceAuthorizedTargetGroup, SourceAuthorizedTargetGroupKey,
};
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

const SOURCE_AUTHORITY_PARALLEL_SOURCE_THRESHOLD: usize = 64;
const SOURCE_AUTHORITY_PARALLEL_PAIR_THRESHOLD: usize = 64;
const SOURCE_AUTHORITY_CANDIDATE_TILE_KEYS: i64 = 8_000_000;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct RaisedStepTargetContourContributorKey {
    ordered_points_xz: Arc<[NodeRailPointKey]>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct RaisedStepTargetGroupContributorKey {
    key: SourceAuthorizedTargetGroupKey,
    ordered_contours: Arc<[RaisedStepTargetContourContributorKey]>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct RaisedStepSourceContributorKey {
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
    owners: [NodeBandOwner; 2],
    ordered_points_xz: Arc<[NodeRailPointKey]>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct RaisedStepSourceGroupContributorKey {
    piece_kind_sort_key: u8,
    source: Arc<RaisedStepSourceContributorKey>,
    target_group: Arc<RaisedStepTargetGroupContributorKey>,
    effective_owner_priority: Option<NodeGeneratedContourClaimPriority>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct RaisedStepSourceGroupPairContributorKey {
    source: Arc<RaisedStepSourceContributorKey>,
    left_group: Arc<RaisedStepTargetGroupContributorKey>,
    right_group: Arc<RaisedStepTargetGroupContributorKey>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct RaisedStepSourcePairContributorKey {
    left: Arc<RaisedStepSourceContributorKey>,
    right: Arc<RaisedStepSourceContributorKey>,
}

#[derive(Clone, Debug)]
struct RaisedStepTargetGroupContributor {
    key: Arc<RaisedStepTargetGroupContributorKey>,
    view: SourceAuthorizedTargetGroupView,
    effective_owner_priority: Option<NodeGeneratedContourClaimPriority>,
}

type RaisedStepSourceEntry = (Arc<RaisedStepSourceContributorKey>, usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct SourceAuthorityCandidateTile {
    x: i64,
    z: i64,
}

#[derive(Default)]
struct RaisedStepSourceSpatialIndex {
    source_indices_by_tile: BTreeMap<SourceAuthorityCandidateTile, Vec<usize>>,
}

#[derive(Default)]
struct RaisedStepTargetGroupSpatialIndex {
    group_indices_by_tile: BTreeMap<SourceAuthorityCandidateTile, Vec<usize>>,
}

#[derive(Clone, Debug, Default)]
pub(in crate::simulation::network::surface::node::rails) struct NodeSourceAuthorizedContactCache {
    target_group_geometry: BTreeMap<
        Arc<RaisedStepTargetGroupContributorKey>,
        Option<Arc<SourceAuthorizedTargetGroup>>,
    >,
    source_group_contacts:
        BTreeMap<RaisedStepSourceGroupContributorKey, Arc<[GeneratedSameBandContactConstraint]>>,
    source_group_pair_contacts: BTreeMap<
        RaisedStepSourceGroupPairContributorKey,
        Arc<[GeneratedSameBandContactConstraint]>,
    >,
    source_pair_points: BTreeMap<RaisedStepSourcePairContributorKey, Arc<[NodeRailPointKey]>>,
    materialized_sources: BTreeSet<Arc<RaisedStepSourceContributorKey>>,
}

#[derive(Clone, Copy, Debug, Default)]
pub(in crate::simulation::network::surface::node::rails) struct SourceAuthorizedContactReuseStats {
    pub(in crate::simulation::network::surface::node::rails) target_group_cache_hits: usize,
    pub(in crate::simulation::network::surface::node::rails) source_cache_hits: usize,
    pub(in crate::simulation::network::surface::node::rails) source_cache_misses: usize,
    pub(in crate::simulation::network::surface::node::rails) source_pair_cache_hits: usize,
    pub(in crate::simulation::network::surface::node::rails) source_pair_cache_misses: usize,
}

pub(in crate::simulation::network::surface::node::rails) fn collect_source_authorized_raised_step_contacts_with_reuse(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    contours: &[NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
    contacts: &mut BTreeSet<GeneratedSameBandContactConstraint>,
    previous: Option<&NodeSourceAuthorizedContactCache>,
    current: &mut NodeSourceAuthorizedContactCache,
) -> SourceAuthorizedContactReuseStats {
    let mut stats = SourceAuthorizedContactReuseStats::default();
    let source_authority = RaisedStepSourceAuthority::from_constraints(constraints);
    let claim_priorities = source_authorized_target_claim_priorities(contours);
    let materialized_target_group_keys = current
        .target_group_geometry
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let target_groups =
        target_groups_with_reuse(contours, &claim_priorities, previous, current, &mut stats);
    let unmaterialized_target_group_keys = target_groups
        .iter()
        .map(|group| Arc::clone(&group.key))
        .filter(|key| !materialized_target_group_keys.contains(key))
        .collect::<BTreeSet<_>>();
    let source_entries = deduplicated_source_entries(source_authority.constraints());
    let source_spatial_index =
        RaisedStepSourceSpatialIndex::new(source_authority.constraints(), &source_entries);
    let target_group_spatial_index = RaisedStepTargetGroupSpatialIndex::new(&target_groups);
    let unmaterialized_source_keys = source_entries
        .iter()
        .map(|(key, _)| Arc::clone(key))
        .filter(|key| !current.materialized_sources.contains(key))
        .collect::<BTreeSet<_>>();
    collect_source_group_contacts_with_reuse(
        piece_kind,
        contours,
        source_authority.constraints(),
        &source_entries,
        &unmaterialized_source_keys,
        &target_group_spatial_index,
        &target_groups,
        &unmaterialized_target_group_keys,
        previous,
        current,
        contacts,
        &mut stats,
    );

    let contact_points = generated_raised_step_source_contact_points_with_reuse(
        source_authority.constraints(),
        &source_entries,
        &source_spatial_index,
        &unmaterialized_source_keys,
        previous,
        current,
        &mut stats,
    );
    for (point, sources) in source_authority.sources_by_contact_points(
        &contact_points,
        &source_entries,
        &source_spatial_index,
    ) {
        for left_index in 0..sources.len() {
            for right_index in left_index + 1..sources.len() {
                let (source_mouth_order_index, source_band_index) =
                    deterministic_contact_source_name(sources[left_index], sources[right_index]);
                for left_owner in sources[left_index].owners {
                    for right_owner in sources[right_index].owners {
                        let Some(pair) = GeneratedRaisedStepOwnerPair::new(left_owner, right_owner)
                        else {
                            continue;
                        };
                        contacts.insert(GeneratedSameBandContactConstraint {
                            kind: NodeRailConstraintKind::RaisedStepContact,
                            owner: pair.owner,
                            opposite_owner: pair.opposite_owner,
                            start: point,
                            end: point,
                            source_mouth_order_index,
                            source_band_index,
                        });
                    }
                }
            }
        }
    }
    current
        .materialized_sources
        .extend(source_entries.into_iter().map(|(key, _)| key));
    stats
}

impl RaisedStepSourceContributorKey {
    fn from_source(source_constraint: &RaisedStepSourceConstraint<'_>) -> Self {
        Self {
            source_mouth_order_index: source_constraint.source.source_mouth_order_index,
            source_band_index: source_constraint.source.source_band_index,
            owners: source_constraint.source.owners,
            ordered_points_xz: Arc::from(
                source_constraint
                    .constraint
                    .points_xz
                    .iter()
                    .copied()
                    .map(road_point_key)
                    .collect::<Vec<_>>(),
            ),
        }
    }
}

fn deduplicated_source_entries(
    source_constraints: &[RaisedStepSourceConstraint<'_>],
) -> Vec<RaisedStepSourceEntry> {
    let mut entries = BTreeMap::<Arc<RaisedStepSourceContributorKey>, usize>::new();
    for (source_index, source_constraint) in source_constraints.iter().enumerate() {
        let key = Arc::new(RaisedStepSourceContributorKey::from_source(
            source_constraint,
        ));
        match entries.entry(key) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(source_index);
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                let retained = &source_constraints[*entry.get()];
                if source_constraint.source < retained.source {
                    entry.insert(source_index);
                }
            }
        }
    }
    entries.into_iter().collect()
}

impl RaisedStepSourceSpatialIndex {
    fn new(
        source_constraints: &[RaisedStepSourceConstraint<'_>],
        source_entries: &[RaisedStepSourceEntry],
    ) -> Self {
        let mut index = Self::default();
        for (source_entry_index, (_, source_index)) in source_entries.iter().enumerate() {
            let source = &source_constraints[*source_index];
            for_source_authority_tiles(
                source.min_x,
                source.min_z,
                source.max_x,
                source.max_z,
                SOURCE_AUTHORITY_BOUNDS_MARGIN_KEYS,
                |tile| {
                    index
                        .source_indices_by_tile
                        .entry(tile)
                        .or_default()
                        .push(source_entry_index);
                },
            );
        }
        index
    }

    fn source_indices_at_point(&self, point: NodeRailPointKey) -> &[usize] {
        self.source_indices_by_tile
            .get(&source_authority_tile(point.0, point.1))
            .map_or(&[], Vec::as_slice)
    }

    fn candidate_pairs(
        &self,
        source_constraints: &[RaisedStepSourceConstraint<'_>],
        source_entries: &[RaisedStepSourceEntry],
        unmaterialized_source_keys: &BTreeSet<Arc<RaisedStepSourceContributorKey>>,
    ) -> Vec<(usize, usize)> {
        let mut pairs = BTreeSet::new();
        for (left_entry_index, (left_key, left_source_index)) in source_entries.iter().enumerate() {
            if !unmaterialized_source_keys.contains(left_key) {
                continue;
            }
            let left = &source_constraints[*left_source_index];
            for_source_authority_tiles(left.min_x, left.min_z, left.max_x, left.max_z, 0, |tile| {
                let Some(candidate_indices) = self.source_indices_by_tile.get(&tile) else {
                    return;
                };
                for &right_entry_index in candidate_indices {
                    if left_entry_index == right_entry_index {
                        continue;
                    }
                    let pair = if left_entry_index < right_entry_index {
                        (left_entry_index, right_entry_index)
                    } else {
                        (right_entry_index, left_entry_index)
                    };
                    pairs.insert(pair);
                }
            });
        }
        pairs
            .into_iter()
            .filter(|&(left_entry_index, right_entry_index)| {
                let left = &source_constraints[source_entries[left_entry_index].1];
                let right = &source_constraints[source_entries[right_entry_index].1];
                !left.bounds_disjoint_source(right)
            })
            .collect()
    }
}

impl RaisedStepTargetGroupSpatialIndex {
    fn new(target_groups: &[RaisedStepTargetGroupContributor]) -> Self {
        let mut index = Self::default();
        for (group_index, group) in target_groups.iter().enumerate() {
            let geometry = &group.view.geometry;
            for_source_authority_tiles(
                geometry.min_x,
                geometry.min_z,
                geometry.max_x,
                geometry.max_z,
                SOURCE_AUTHORITY_BOUNDS_MARGIN_KEYS,
                |tile| {
                    index
                        .group_indices_by_tile
                        .entry(tile)
                        .or_default()
                        .push(group_index);
                },
            );
        }
        index
    }

    fn candidate_group_indices(&self, source: &RaisedStepSourceConstraint<'_>) -> Vec<usize> {
        let mut candidates = Vec::new();
        for_source_authority_tiles(
            source.min_x,
            source.min_z,
            source.max_x,
            source.max_z,
            0,
            |tile| {
                if let Some(indices) = self.group_indices_by_tile.get(&tile) {
                    candidates.extend(indices.iter().copied());
                }
            },
        );
        candidates.sort_unstable();
        candidates.dedup();
        candidates
    }
}

fn source_authority_tile(x: i64, z: i64) -> SourceAuthorityCandidateTile {
    SourceAuthorityCandidateTile {
        x: x.div_euclid(SOURCE_AUTHORITY_CANDIDATE_TILE_KEYS),
        z: z.div_euclid(SOURCE_AUTHORITY_CANDIDATE_TILE_KEYS),
    }
}

fn for_source_authority_tiles(
    min_x: i64,
    min_z: i64,
    max_x: i64,
    max_z: i64,
    margin: i64,
    mut visit: impl FnMut(SourceAuthorityCandidateTile),
) {
    if min_x > max_x || min_z > max_z {
        return;
    }
    let min_tile =
        source_authority_tile(min_x.saturating_sub(margin), min_z.saturating_sub(margin));
    let max_tile =
        source_authority_tile(max_x.saturating_add(margin), max_z.saturating_add(margin));
    for x in min_tile.x..=max_tile.x {
        for z in min_tile.z..=max_tile.z {
            visit(SourceAuthorityCandidateTile { x, z });
        }
    }
}

impl RaisedStepSourceGroupPairContributorKey {
    fn new(
        source: &Arc<RaisedStepSourceContributorKey>,
        left_group: &Arc<RaisedStepTargetGroupContributorKey>,
        right_group: &Arc<RaisedStepTargetGroupContributorKey>,
    ) -> Self {
        Self {
            source: Arc::clone(source),
            left_group: Arc::clone(left_group),
            right_group: Arc::clone(right_group),
        }
    }
}

impl RaisedStepSourcePairContributorKey {
    fn from_sources(
        left: &Arc<RaisedStepSourceContributorKey>,
        right: &Arc<RaisedStepSourceContributorKey>,
    ) -> Self {
        if left <= right {
            Self {
                left: Arc::clone(left),
                right: Arc::clone(right),
            }
        } else {
            Self {
                left: Arc::clone(right),
                right: Arc::clone(left),
            }
        }
    }
}

fn target_groups_with_reuse(
    contours: &[NodeGeneratedContour],
    claim_priorities: &BTreeMap<NodeBandOwner, NodeGeneratedContourClaimPriority>,
    previous: Option<&NodeSourceAuthorizedContactCache>,
    current: &mut NodeSourceAuthorizedContactCache,
    stats: &mut SourceAuthorizedContactReuseStats,
) -> Vec<RaisedStepTargetGroupContributor> {
    let mut contributors = BTreeMap::<
        SourceAuthorizedTargetGroupKey,
        (Vec<usize>, Vec<RaisedStepTargetContourContributorKey>),
    >::new();
    for (contour_index, contour) in contours.iter().enumerate() {
        let Some(owner) = contour.owner else {
            continue;
        };
        let Some(kind) = generated_contour_band_kind(contour) else {
            continue;
        };
        let key = SourceAuthorizedTargetGroupKey {
            owner,
            kind,
            claim_priority: contour.claim_priority,
        };
        let (indices, contour_keys) = contributors.entry(key).or_default();
        indices.push(contour_index);
        contour_keys.push(RaisedStepTargetContourContributorKey {
            ordered_points_xz: Arc::from(generated_contour_keys(contour)),
        });
    }

    contributors
        .into_iter()
        .filter_map(|(group_key, (contour_indices, contour_keys))| {
            let contributor_key = Arc::new(RaisedStepTargetGroupContributorKey {
                key: group_key,
                ordered_contours: Arc::from(contour_keys),
            });
            let cached_geometry = current
                .target_group_geometry
                .get(&contributor_key)
                .cloned()
                .or_else(|| {
                    previous.and_then(|previous| {
                        previous
                            .target_group_geometry
                            .get(&contributor_key)
                            .cloned()
                    })
                });
            let geometry = if let Some(cached_geometry) = cached_geometry {
                stats.target_group_cache_hits += 1;
                cached_geometry
            } else {
                source_authorized_target_group(contours, group_key, &contour_indices).map(Arc::new)
            };
            current
                .target_group_geometry
                .insert(Arc::clone(&contributor_key), geometry.clone());
            debug_assert!(
                geometry
                    .as_ref()
                    .is_none_or(|geometry| geometry.contour_indices.is_empty())
            );
            Some(RaisedStepTargetGroupContributor {
                key: contributor_key,
                view: SourceAuthorizedTargetGroupView {
                    geometry: geometry?,
                    contour_indices: Arc::from(contour_indices),
                },
                effective_owner_priority: claim_priorities.get(&group_key.owner).copied(),
            })
        })
        .collect()
}

fn collect_source_group_contacts_with_reuse(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    contours: &[NodeGeneratedContour],
    source_constraints: &[RaisedStepSourceConstraint<'_>],
    source_entries: &[RaisedStepSourceEntry],
    unmaterialized_source_keys: &BTreeSet<Arc<RaisedStepSourceContributorKey>>,
    target_group_spatial_index: &RaisedStepTargetGroupSpatialIndex,
    target_groups: &[RaisedStepTargetGroupContributor],
    unmaterialized_target_group_keys: &BTreeSet<Arc<RaisedStepTargetGroupContributorKey>>,
    previous: Option<&NodeSourceAuthorizedContactCache>,
    current: &mut NodeSourceAuthorizedContactCache,
    contacts: &mut BTreeSet<GeneratedSameBandContactConstraint>,
    stats: &mut SourceAuthorizedContactReuseStats,
) {
    let mut source_group_misses = Vec::new();
    for (source_key, source_index) in source_entries {
        let source_is_unmaterialized = unmaterialized_source_keys.contains(source_key);
        if !source_is_unmaterialized && unmaterialized_target_group_keys.is_empty() {
            continue;
        }
        let source_constraint = &source_constraints[*source_index];
        for group_index in target_group_spatial_index
            .candidate_group_indices(source_constraint)
            .into_iter()
            .filter(|&group_index| {
                !source_constraint.bounds_disjoint_group(&target_groups[group_index].view.geometry)
            })
        {
            let target_group = &target_groups[group_index];
            if !source_is_unmaterialized
                && !unmaterialized_target_group_keys.contains(&target_group.key)
            {
                continue;
            }
            let key = RaisedStepSourceGroupContributorKey {
                piece_kind_sort_key: piece_kind.sort_key(),
                source: Arc::clone(source_key),
                target_group: Arc::clone(&target_group.key),
                effective_owner_priority: target_group.effective_owner_priority,
            };
            if current.source_group_contacts.contains_key(&key) {
                // An equivalent source already emitted this contribution into
                // the current generation's constraints.
                stats.source_cache_hits += 1;
                continue;
            }
            if let Some(cached) = previous
                .and_then(|previous| previous.source_group_contacts.get(&key))
                .cloned()
            {
                stats.source_cache_hits += 1;
                contacts.extend(cached.iter().copied());
                current.source_group_contacts.insert(key, cached);
                continue;
            }
            stats.source_cache_misses += 1;
            source_group_misses.push((key, source_constraint, group_index));
        }
    }
    let computed_source_group_contacts =
        if source_group_misses.len() >= SOURCE_AUTHORITY_PARALLEL_SOURCE_THRESHOLD {
            source_group_misses
                .par_iter()
                .map(|(key, source_constraint, group_index)| {
                    (
                        key.clone(),
                        collect_source_authorized_contacts_for_source_group(
                            piece_kind,
                            contours,
                            source_constraint,
                            &target_groups[*group_index],
                        ),
                    )
                })
                .collect::<Vec<_>>()
        } else {
            source_group_misses
                .iter()
                .map(|(key, source_constraint, group_index)| {
                    (
                        key.clone(),
                        collect_source_authorized_contacts_for_source_group(
                            piece_kind,
                            contours,
                            source_constraint,
                            &target_groups[*group_index],
                        ),
                    )
                })
                .collect::<Vec<_>>()
        };
    for (key, computed) in computed_source_group_contacts {
        let computed = Arc::<[GeneratedSameBandContactConstraint]>::from(
            computed.into_iter().collect::<Vec<_>>(),
        );
        contacts.extend(computed.iter().copied());
        current.source_group_contacts.insert(key, computed);
    }

    let mut source_group_pair_misses = Vec::new();
    for (source_key, source_index) in source_entries {
        let source_is_unmaterialized = unmaterialized_source_keys.contains(source_key);
        if !source_is_unmaterialized && unmaterialized_target_group_keys.is_empty() {
            continue;
        }
        let source_constraint = &source_constraints[*source_index];
        let [left_owner, right_owner] = source_constraint.source.owners;
        let candidate_group_indices = target_group_spatial_index
            .candidate_group_indices(source_constraint)
            .into_iter()
            .filter(|&group_index| {
                !source_constraint.bounds_disjoint_group(&target_groups[group_index].view.geometry)
            })
            .collect::<Vec<_>>();
        for &left_index in candidate_group_indices.iter().filter(|&&group_index| {
            target_groups[group_index].view.geometry.key.owner == left_owner
        }) {
            let left_group = &target_groups[left_index];
            for &right_index in candidate_group_indices.iter().filter(|&&group_index| {
                target_groups[group_index].view.geometry.key.owner == right_owner
            }) {
                let right_group = &target_groups[right_index];
                if !source_is_unmaterialized
                    && !unmaterialized_target_group_keys.contains(&left_group.key)
                    && !unmaterialized_target_group_keys.contains(&right_group.key)
                {
                    continue;
                }
                let key = RaisedStepSourceGroupPairContributorKey::new(
                    source_key,
                    &left_group.key,
                    &right_group.key,
                );
                if current.source_group_pair_contacts.contains_key(&key) {
                    // An equivalent source already emitted this contribution
                    // into the current generation's constraints.
                    stats.source_cache_hits += 1;
                    continue;
                }
                if let Some(cached) = previous
                    .and_then(|previous| previous.source_group_pair_contacts.get(&key))
                    .cloned()
                {
                    stats.source_cache_hits += 1;
                    contacts.extend(cached.iter().copied());
                    current.source_group_pair_contacts.insert(key, cached);
                    continue;
                }
                stats.source_cache_misses += 1;
                source_group_pair_misses.push((key, source_constraint, left_index, right_index));
            }
        }
    }
    let computed_source_group_pair_contacts =
        if source_group_pair_misses.len() >= SOURCE_AUTHORITY_PARALLEL_SOURCE_THRESHOLD {
            source_group_pair_misses
                .par_iter()
                .map(|(key, source_constraint, left_index, right_index)| {
                    (
                        key.clone(),
                        collect_source_authorized_contacts_for_source_group_pair(
                            contours,
                            source_constraint,
                            &target_groups[*left_index],
                            &target_groups[*right_index],
                        ),
                    )
                })
                .collect::<Vec<_>>()
        } else {
            source_group_pair_misses
                .iter()
                .map(|(key, source_constraint, left_index, right_index)| {
                    (
                        key.clone(),
                        collect_source_authorized_contacts_for_source_group_pair(
                            contours,
                            source_constraint,
                            &target_groups[*left_index],
                            &target_groups[*right_index],
                        ),
                    )
                })
                .collect::<Vec<_>>()
        };
    for (key, computed) in computed_source_group_pair_contacts {
        let computed = Arc::<[GeneratedSameBandContactConstraint]>::from(
            computed.into_iter().collect::<Vec<_>>(),
        );
        contacts.extend(computed.iter().copied());
        current.source_group_pair_contacts.insert(key, computed);
    }
}

fn collect_source_authorized_contacts_for_source_group(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    contours: &[NodeGeneratedContour],
    source_constraint: &RaisedStepSourceConstraint<'_>,
    target_group: &RaisedStepTargetGroupContributor,
) -> BTreeSet<GeneratedSameBandContactConstraint> {
    let mut contacts = BTreeSet::new();
    let target_contacts = source_authorized_raised_step_target_pairs(
        piece_kind,
        target_group.effective_owner_priority,
        source_constraint.source,
        target_group.view.geometry.key,
    );
    if !target_contacts.is_empty()
        && !source_constraint.bounds_disjoint_group(&target_group.view.geometry)
    {
        for source_edge in &source_constraint.edges {
            let source_edge = *source_edge;
            if target_group.view.geometry.bounds_disjoint_edge(source_edge) {
                continue;
            }
            let mut source_edges = generated_directed_edge_segments_inside_shape_edges(
                source_edge,
                &target_group.view.geometry.shape_edges,
                &target_group.view.geometry.shapes,
            )
            .into_iter()
            .collect::<BTreeSet<_>>();
            source_edges.extend(generated_shape_boundary_segments_on_source_edge(
                source_edge,
                &target_group.view.geometry.shape_edges,
            ));
            for edge in source_edges {
                for (owner, opposite_owner, include_edge) in &target_contacts {
                    for (start, end) in source_authorized_contact_segments(edge, *include_edge) {
                        contacts.insert(GeneratedSameBandContactConstraint {
                            kind: NodeRailConstraintKind::RaisedStepContact,
                            owner: *owner,
                            opposite_owner: *opposite_owner,
                            start,
                            end,
                            source_mouth_order_index: source_constraint
                                .source
                                .source_mouth_order_index,
                            source_band_index: source_constraint.source.source_band_index,
                        });
                    }
                }
            }
        }
    }
    collect_junctionn_source_authorized_mouth_band_endpoint_handoffs_for_group(
        piece_kind,
        contours,
        source_constraint,
        &target_group.view,
        target_group.effective_owner_priority,
        &mut contacts,
    );
    contacts
}

fn collect_source_authorized_contacts_for_source_group_pair(
    contours: &[NodeGeneratedContour],
    source_constraint: &RaisedStepSourceConstraint<'_>,
    left_group: &RaisedStepTargetGroupContributor,
    right_group: &RaisedStepTargetGroupContributor,
) -> BTreeSet<GeneratedSameBandContactConstraint> {
    let mut contacts = BTreeSet::new();
    collect_source_authorized_exact_group_pair_overlap_contacts(
        source_constraint,
        contours,
        &left_group.view,
        &right_group.view,
        &mut contacts,
    );
    contacts
}

fn collect_junctionn_source_authorized_mouth_band_endpoint_handoffs_for_group(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    contours: &[NodeGeneratedContour],
    source_constraint: &RaisedStepSourceConstraint<'_>,
    target_group: &SourceAuthorizedTargetGroupView,
    effective_owner_priority: Option<NodeGeneratedContourClaimPriority>,
    contacts: &mut BTreeSet<GeneratedSameBandContactConstraint>,
) {
    if piece_kind != RoadSurfaceVisualNodePieceKind::JunctionN {
        return;
    }
    for point in generated_constraint_endpoint_keys(source_constraint.constraint) {
        for replaced_owner_index in 0..source_constraint.source.owners.len() {
            let replaced_owner = source_constraint.source.owners[replaced_owner_index];
            let retained_owner = source_constraint.source.owners[1 - replaced_owner_index];
            let target_owner = target_group.geometry.key.owner;
            if target_owner == replaced_owner
                || source_constraint.source.owners.contains(&target_owner)
                || target_owner.kind() != replaced_owner.kind()
                || target_group.geometry.key.claim_priority
                    != NodeGeneratedContourClaimPriority::MouthBand
                || effective_owner_priority != Some(target_group.geometry.key.claim_priority)
                || !target_group_contains_boundary_key(contours, target_group, point)
            {
                continue;
            }
            let Some(pair) = GeneratedRaisedStepOwnerPair::new(retained_owner, target_owner) else {
                continue;
            };
            contacts.insert(GeneratedSameBandContactConstraint {
                kind: NodeRailConstraintKind::RaisedStepContact,
                owner: pair.owner,
                opposite_owner: pair.opposite_owner,
                start: point,
                end: point,
                source_mouth_order_index: source_constraint.source.source_mouth_order_index,
                source_band_index: source_constraint.source.source_band_index,
            });
        }
    }
}

fn target_group_contains_boundary_key(
    contours: &[NodeGeneratedContour],
    target_group: &SourceAuthorizedTargetGroupView,
    point: NodeRailPointKey,
) -> bool {
    target_group.contour_indices.iter().any(|contour_index| {
        contours
            .get(*contour_index)
            .is_some_and(|contour| generated_contour_boundary_contains_key(contour, point))
    })
}

fn deterministic_contact_source_name(
    left: GeneratedRaisedStepEndpointSource,
    right: GeneratedRaisedStepEndpointSource,
) -> (usize, Option<usize>) {
    // The generated contact already has exact endpoint authority from both sources.
    // NodeRailConstraint carries one source name, so use a semantic label that is
    // stable when equivalent constraints are inserted at different positions.
    let source_sort_key = |source: GeneratedRaisedStepEndpointSource| {
        (
            source.source_mouth_order_index,
            source.source_band_index,
            source.owners,
            source.constraint_index,
        )
    };
    let source = if source_sort_key(left) <= source_sort_key(right) {
        left
    } else {
        right
    };
    (source.source_mouth_order_index, source.source_band_index)
}

impl<'a> RaisedStepSourceAuthority<'a> {
    fn from_constraints(constraints: &'a [NodeRailConstraint]) -> Self {
        Self {
            constraints: constraints
                .iter()
                .filter_map(|constraint| {
                    generated_raised_step_endpoint_source(constraint).map(|source| {
                        let edges = generated_constraint_directed_edges(constraint);
                        let (min_x, min_z, max_x, max_z) = generated_constraint_bounds(constraint);
                        RaisedStepSourceConstraint {
                            source,
                            constraint,
                            edges,
                            min_x,
                            min_z,
                            max_x,
                            max_z,
                        }
                    })
                })
                .collect(),
        }
    }

    fn constraints(&self) -> &[RaisedStepSourceConstraint<'a>] {
        &self.constraints
    }

    fn sources_by_contact_points(
        &self,
        points: &BTreeSet<NodeRailPointKey>,
        source_entries: &[RaisedStepSourceEntry],
        source_spatial_index: &RaisedStepSourceSpatialIndex,
    ) -> BTreeMap<NodeRailPointKey, Vec<GeneratedRaisedStepEndpointSource>> {
        points
            .iter()
            .copied()
            .filter_map(|point| {
                let mut sources = source_spatial_index
                    .source_indices_at_point(point)
                    .iter()
                    .filter_map(|&source_entry_index| {
                        let source_index = source_entries.get(source_entry_index)?.1;
                        let source_constraint = self.constraints.get(source_index)?;
                        source_constraint
                            .bounds_contains_key(point)
                            .then(|| source_constraint)
                    })
                    .filter(|source_constraint| {
                        generated_constraint_touches_key(source_constraint.constraint, point)
                    })
                    .map(|source_constraint| source_constraint.source)
                    .collect::<Vec<_>>();
                sources.sort_unstable();
                sources.dedup();
                (!sources.is_empty()).then_some((point, sources))
            })
            .collect()
    }
}

fn generated_raised_step_source_contact_points_with_reuse(
    source_constraints: &[RaisedStepSourceConstraint<'_>],
    source_entries: &[RaisedStepSourceEntry],
    source_spatial_index: &RaisedStepSourceSpatialIndex,
    unmaterialized_source_keys: &BTreeSet<Arc<RaisedStepSourceContributorKey>>,
    previous: Option<&NodeSourceAuthorizedContactCache>,
    current: &mut NodeSourceAuthorizedContactCache,
    stats: &mut SourceAuthorizedContactReuseStats,
) -> BTreeSet<NodeRailPointKey> {
    let mut points = source_entries
        .iter()
        .filter(|(key, _)| unmaterialized_source_keys.contains(key))
        .map(|(_, source_index)| &source_constraints[*source_index])
        .flat_map(|source_constraint| {
            generated_constraint_endpoint_keys(source_constraint.constraint)
        })
        .collect::<BTreeSet<_>>();
    let mut cached_points = Vec::new();
    let mut misses = Vec::new();
    for (left_entry_index, right_entry_index) in source_spatial_index.candidate_pairs(
        source_constraints,
        source_entries,
        unmaterialized_source_keys,
    ) {
        let (left_key, left_source_index) = &source_entries[left_entry_index];
        let (right_key, right_source_index) = &source_entries[right_entry_index];
        let left = &source_constraints[*left_source_index];
        let right = &source_constraints[*right_source_index];
        let key = RaisedStepSourcePairContributorKey::from_sources(left_key, right_key);
        if let Some(cached) = current
            .source_pair_points
            .get(&key)
            .or_else(|| previous.and_then(|previous| previous.source_pair_points.get(&key)))
        {
            stats.source_pair_cache_hits += 1;
            cached_points.push((key, Arc::clone(cached)));
        } else {
            stats.source_pair_cache_misses += 1;
            misses.push((key, left, right));
        }
    }
    let computed_points = if misses.len() >= SOURCE_AUTHORITY_PARALLEL_PAIR_THRESHOLD {
        misses
            .par_iter()
            .map(|(key, left, right)| {
                (
                    key.clone(),
                    Arc::<[NodeRailPointKey]>::from(generated_source_constraint_contact_points(
                        left, right,
                    )),
                )
            })
            .collect::<Vec<_>>()
    } else {
        misses
            .iter()
            .map(|(key, left, right)| {
                (
                    key.clone(),
                    Arc::<[NodeRailPointKey]>::from(generated_source_constraint_contact_points(
                        left, right,
                    )),
                )
            })
            .collect::<Vec<_>>()
    };
    cached_points.extend(computed_points);
    for (key, pair_points) in cached_points {
        points.extend(pair_points.iter().copied());
        current.source_pair_points.insert(key, pair_points);
    }
    points
}

fn generated_source_constraint_contact_points(
    left: &RaisedStepSourceConstraint<'_>,
    right: &RaisedStepSourceConstraint<'_>,
) -> Vec<NodeRailPointKey> {
    let mut points = Vec::new();
    for left_edge in &left.edges {
        for right_edge in &right.edges {
            points.extend(generated_source_edge_contact_points(left_edge, right_edge));
        }
    }
    points.sort_unstable();
    points.dedup();
    points
}

fn generated_source_edge_contact_points(
    left: &GeneratedContourDirectedEdge,
    right: &GeneratedContourDirectedEdge,
) -> Vec<NodeRailPointKey> {
    let mut points = Vec::new();
    if let Some(point) =
        quantized_proper_segment_intersection(left.start, left.end, right.start, right.end)
    {
        points.push(point);
    }
    for point in [left.start, left.end] {
        if generated_point_key_lies_on_segment(point, right.start, right.end) {
            points.push(point);
        }
    }
    for point in [right.start, right.end] {
        if generated_point_key_lies_on_segment(point, left.start, left.end) {
            points.push(point);
        }
    }
    points.sort_unstable();
    points.dedup();
    points
}

fn generated_constraint_bounds(constraint: &NodeRailConstraint) -> (i64, i64, i64, i64) {
    let (mut min_x, mut min_z) = (i64::MAX, i64::MAX);
    let (mut max_x, mut max_z) = (i64::MIN, i64::MIN);
    for point in constraint.points_xz.iter().copied().map(road_point_key) {
        min_x = min_x.min(point.0);
        min_z = min_z.min(point.1);
        max_x = max_x.max(point.0);
        max_z = max_z.max(point.1);
    }
    if constraint.points_xz.is_empty() {
        return (1, 1, 0, 0);
    }
    (min_x, min_z, max_x, max_z)
}

fn generated_raised_step_endpoint_source(
    constraint: &NodeRailConstraint,
) -> Option<GeneratedRaisedStepEndpointSource> {
    if constraint.kind != NodeRailConstraintKind::RaisedStepContact {
        return None;
    }
    let owner = constraint.owner?;
    let opposite_owner = constraint.opposite_owner?;
    let pair = GeneratedRaisedStepOwnerPair::new(owner, opposite_owner)?;
    Some(GeneratedRaisedStepEndpointSource {
        constraint_index: constraint.constraint_index,
        source_mouth_order_index: constraint.source_mouth_order_index,
        source_band_index: constraint.source_band_index,
        owners: [pair.owner, pair.opposite_owner],
    })
}

fn generated_constraint_endpoint_keys(constraint: &NodeRailConstraint) -> Vec<NodeRailPointKey> {
    let mut points = Vec::new();
    if let Some(point) = constraint.points_xz.first().copied() {
        points.push(road_point_key(point));
    }
    if let Some(point) = constraint.points_xz.last().copied() {
        points.push(road_point_key(point));
    }
    points.sort_unstable();
    points.dedup();
    points
}

#[cfg(test)]
mod tests {
    use super::super::super::super::geometry::road_point_from_key;
    use super::*;
    use crate::simulation::network::surface::RoadSurfaceBandKind;

    fn owner_pair(owner_index: usize) -> [NodeBandOwner; 2] {
        let pair = GeneratedRaisedStepOwnerPair::new(
            NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, owner_index),
            NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, owner_index + 1),
        )
        .expect("carriageway and curb form a raised-step pair");
        [pair.owner, pair.opposite_owner]
    }

    fn source_constraint(
        constraint_index: usize,
        source_mouth_order_index: usize,
        owners: [NodeBandOwner; 2],
        start: NodeRailPointKey,
        end: NodeRailPointKey,
    ) -> NodeRailConstraint {
        NodeRailConstraint {
            constraint_index,
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index,
            source_band_index: Some(0),
            source_boundary_index: None,
            owner: Some(owners[0]),
            opposite_owner: Some(owners[1]),
            points_xz: vec![road_point_from_key(start), road_point_from_key(end)],
        }
    }

    fn collect_contacts(
        constraints: &[NodeRailConstraint],
        previous: Option<&NodeSourceAuthorizedContactCache>,
    ) -> (
        BTreeSet<GeneratedSameBandContactConstraint>,
        NodeSourceAuthorizedContactCache,
        SourceAuthorizedContactReuseStats,
    ) {
        let mut contacts = BTreeSet::new();
        let mut current = NodeSourceAuthorizedContactCache::default();
        let stats = collect_source_authorized_raised_step_contacts_with_reuse(
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &[],
            constraints,
            &mut contacts,
            previous,
            &mut current,
        );
        (contacts, current, stats)
    }

    #[test]
    fn duplicate_sources_have_one_registry_entry_and_same_contacts() {
        let horizontal = source_constraint(7, 0, owner_pair(0), (-2_000_000, 0), (2_000_000, 0));
        let vertical = source_constraint(8, 1, owner_pair(2), (0, -2_000_000), (0, 2_000_000));
        let mut duplicate = horizontal.clone();
        duplicate.constraint_index = 2;

        let constraints_with_duplicate = vec![horizontal.clone(), duplicate, vertical.clone()];
        let source_authority =
            RaisedStepSourceAuthority::from_constraints(&constraints_with_duplicate);
        let entries = deduplicated_source_entries(source_authority.constraints());
        assert_eq!(source_authority.constraints().len(), 3);
        assert_eq!(entries.len(), 2);
        let retained_duplicate = entries
            .iter()
            .find_map(|(key, source_index)| {
                (key.source_mouth_order_index == 0)
                    .then_some(source_authority.constraints()[*source_index].source)
            })
            .expect("deduplicated horizontal source");
        assert_eq!(retained_duplicate.constraint_index, 2);

        let (deduplicated_contacts, _, _) = collect_contacts(&constraints_with_duplicate, None);
        let (unique_contacts, _, _) = collect_contacts(&[horizontal, vertical], None);
        assert_eq!(deduplicated_contacts, unique_contacts);
        assert!(!unique_contacts.is_empty());
    }

    #[test]
    fn duplicate_source_constraint_order_does_not_change_contact_source_label() {
        let early_horizontal =
            source_constraint(0, 5, owner_pair(0), (-2_000_000, 0), (2_000_000, 0));
        let vertical = source_constraint(5, 1, owner_pair(2), (0, -2_000_000), (0, 2_000_000));
        let mut late_horizontal = early_horizontal.clone();
        late_horizontal.constraint_index = 10;

        let (deduplicated_contacts, _, _) = collect_contacts(
            &[early_horizontal.clone(), vertical.clone(), late_horizontal],
            None,
        );
        let (unique_contacts, _, _) = collect_contacts(&[early_horizontal, vertical], None);

        assert_eq!(deduplicated_contacts, unique_contacts);
        assert!(!unique_contacts.is_empty());
        assert!(
            unique_contacts
                .iter()
                .all(|contact| contact.source_mouth_order_index == 1),
            "the semantic source label must not depend on constraint insertion order"
        );
    }

    #[test]
    fn triple_intersection_indexes_every_unique_source_once() {
        let constraints = vec![
            source_constraint(0, 0, owner_pair(0), (-2_000_000, 0), (2_000_000, 0)),
            source_constraint(1, 1, owner_pair(2), (0, -2_000_000), (0, 2_000_000)),
            source_constraint(
                2,
                2,
                owner_pair(4),
                (-2_000_000, -2_000_000),
                (2_000_000, 2_000_000),
            ),
        ];
        let source_authority = RaisedStepSourceAuthority::from_constraints(&constraints);
        let entries = deduplicated_source_entries(source_authority.constraints());
        let spatial_index =
            RaisedStepSourceSpatialIndex::new(source_authority.constraints(), &entries);
        let unmaterialized = entries
            .iter()
            .map(|(key, _)| Arc::clone(key))
            .collect::<BTreeSet<_>>();
        let mut current = NodeSourceAuthorizedContactCache::default();
        let mut stats = SourceAuthorizedContactReuseStats::default();
        let points = generated_raised_step_source_contact_points_with_reuse(
            source_authority.constraints(),
            &entries,
            &spatial_index,
            &unmaterialized,
            None,
            &mut current,
            &mut stats,
        );
        assert!(points.contains(&(0, 0)));
        assert_eq!(stats.source_pair_cache_misses, 3);

        let sources = source_authority.sources_by_contact_points(&points, &entries, &spatial_index);
        let sources_at_intersection = sources
            .get(&(0, 0))
            .expect("triple intersection has exact source authority");
        assert_eq!(sources_at_intersection.len(), 3);
        assert_eq!(
            sources_at_intersection
                .iter()
                .map(|source| source.source_mouth_order_index)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn negative_tile_boundary_keeps_exact_margin_candidate() {
        let boundary_x = -SOURCE_AUTHORITY_CANDIDATE_TILE_KEYS;
        let constraints = vec![
            source_constraint(
                0,
                0,
                owner_pair(0),
                (boundary_x - SOURCE_AUTHORITY_BOUNDS_MARGIN_KEYS / 2, -1_000),
                (boundary_x - SOURCE_AUTHORITY_BOUNDS_MARGIN_KEYS / 2, 1_000),
            ),
            source_constraint(
                1,
                1,
                owner_pair(2),
                (boundary_x + SOURCE_AUTHORITY_BOUNDS_MARGIN_KEYS / 2, -1_000),
                (boundary_x + SOURCE_AUTHORITY_BOUNDS_MARGIN_KEYS / 2, 1_000),
            ),
        ];
        let source_authority = RaisedStepSourceAuthority::from_constraints(&constraints);
        let entries = deduplicated_source_entries(source_authority.constraints());
        let spatial_index =
            RaisedStepSourceSpatialIndex::new(source_authority.constraints(), &entries);
        let unmaterialized = entries
            .iter()
            .map(|(key, _)| Arc::clone(key))
            .collect::<BTreeSet<_>>();

        let pairs = spatial_index.candidate_pairs(
            source_authority.constraints(),
            &entries,
            &unmaterialized,
        );
        assert_eq!(pairs, vec![(0, 1)]);
        assert_eq!(
            source_authority_tile(boundary_x - 1, 0).x,
            source_authority_tile(boundary_x, 0).x - 1
        );
    }

    #[test]
    fn warm_source_pair_cache_matches_cold_output() {
        let constraints = vec![
            source_constraint(0, 0, owner_pair(0), (-2_000_000, 0), (2_000_000, 0)),
            source_constraint(1, 1, owner_pair(2), (0, -2_000_000), (0, 2_000_000)),
        ];
        let (cold_contacts, cold_cache, cold_stats) = collect_contacts(&constraints, None);
        let (warm_contacts, _, warm_stats) = collect_contacts(&constraints, Some(&cold_cache));

        assert_eq!(warm_contacts, cold_contacts);
        assert!(!cold_contacts.is_empty());
        assert_eq!(cold_stats.source_pair_cache_hits, 0);
        assert_eq!(cold_stats.source_pair_cache_misses, 1);
        assert_eq!(warm_stats.source_pair_cache_hits, 1);
        assert_eq!(warm_stats.source_pair_cache_misses, 0);
    }
}
