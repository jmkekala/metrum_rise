// SPDX-License-Identifier: GPL-2.0-only

//! Retention filter for exact source-authorized generated contacts.

use super::super::source_authority::generated_contact_kind_from_constraint;
use super::super::{
    NodeBandOwner, NodeGeneratedContour, NodeRailConstraint, NodeRailConstraintKind,
    NodeRailPointKey, generated_contour_keys, road_point_key,
};
use super::authority::{ExactGeneratedSourceAuthority, ExactGeneratedSourceAuthorityFingerprint};
use super::endpoints::generated_contact_constraint_has_exact_source_authority;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct RetentionContourContributorKey {
    owner: NodeBandOwner,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
    ordered_points_xz: Arc<[NodeRailPointKey]>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct RetentionSourceConstraintContributorKey {
    kind: NodeRailConstraintKind,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
    owner: Option<NodeBandOwner>,
    opposite_owner: Option<NodeBandOwner>,
    ordered_points_xz: Arc<[NodeRailPointKey]>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct RetentionAuthorityContributorKey {
    contours: Arc<[RetentionContourContributorKey]>,
    source_constraints: Arc<[RetentionSourceConstraintContributorKey]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct RetentionConstraintMetadataKey {
    kind: NodeRailConstraintKind,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
    owner: Option<NodeBandOwner>,
    opposite_owner: Option<NodeBandOwner>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct RetentionConstraintBucketKey {
    metadata: RetentionConstraintMetadataKey,
    point_hash: u64,
}

#[derive(Clone, Debug)]
struct RetainedContactDecision {
    ordered_points_xz: Arc<[NodeRailPointKey]>,
    authority_fingerprint: Arc<ExactGeneratedSourceAuthorityFingerprint>,
    retained: bool,
}

#[derive(Clone, Debug)]
struct RetainedContactContext {
    key: RetentionAuthorityContributorKey,
    authority: Arc<ExactGeneratedSourceAuthority>,
}

/// Exact retained-contact decisions reusable by a later rail generation.
#[derive(Clone, Debug, Default)]
pub(in crate::simulation::network::surface::node::rails) struct NodeRetainedContactCache {
    context: Option<RetainedContactContext>,
    retained_by_constraint: BTreeMap<RetentionConstraintBucketKey, Vec<RetainedContactDecision>>,
}

/// Retained-output cache activity for one rail generation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::simulation::network::surface::node::rails) struct NodeRetainedContactReuseStats {
    pub(in crate::simulation::network::surface::node::rails) authority_cache_hits: usize,
    pub(in crate::simulation::network::surface::node::rails) authority_current_hits: usize,
    pub(in crate::simulation::network::surface::node::rails) authority_previous_hits: usize,
    pub(in crate::simulation::network::surface::node::rails) authority_cache_misses: usize,
    pub(in crate::simulation::network::surface::node::rails) decision_cache_hits: usize,
    pub(in crate::simulation::network::surface::node::rails) decision_current_hits: usize,
    pub(in crate::simulation::network::surface::node::rails) decision_previous_hits: usize,
    pub(in crate::simulation::network::surface::node::rails) decision_cache_misses: usize,
}

pub(in crate::simulation::network::surface::node::rails) fn retain_source_authorized_generated_contact_constraint_sets_with_reuse(
    contours: &[NodeGeneratedContour],
    constraints: &mut Vec<NodeRailConstraint>,
    validation_constraints: &mut Vec<NodeRailConstraint>,
    generated_constraint_start_index: usize,
    previous: Option<&NodeRetainedContactCache>,
    current: &mut NodeRetainedContactCache,
) -> (
    Arc<ExactGeneratedSourceAuthority>,
    NodeRetainedContactReuseStats,
) {
    let authority_key = RetentionAuthorityContributorKey::from_sources(
        contours,
        validation_constraints,
        generated_constraint_start_index,
    );
    let previous_authority = previous
        .and_then(|previous| previous.context.as_ref())
        .map(|context| &context.authority);
    let previous_context = previous
        .and_then(|previous| previous.context.as_ref())
        .filter(|context| context.key == authority_key);
    let current_context_exists = current
        .context
        .as_ref()
        .is_some_and(|context| context.key == authority_key);
    let mut stats = NodeRetainedContactReuseStats::default();
    if current_context_exists {
        stats.authority_cache_hits += 1;
        stats.authority_current_hits += 1;
    } else if previous_context.is_some() {
        stats.authority_cache_hits += 1;
        stats.authority_previous_hits += 1;
    } else {
        stats.authority_cache_misses += 1;
    }
    if !current_context_exists {
        let authority = previous_context
            .map(|context| Arc::clone(&context.authority))
            .unwrap_or_else(|| {
                Arc::new(ExactGeneratedSourceAuthority::from_sources_with_reuse(
                    contours,
                    validation_constraints,
                    generated_constraint_start_index,
                    previous_authority.map(AsRef::as_ref),
                ))
            });
        current.context = Some(RetainedContactContext {
            key: authority_key.clone(),
            authority,
        });
        current.retained_by_constraint.clear();
    }
    let current_authority = Arc::clone(
        &current
            .context
            .as_ref()
            .expect("current retained-contact context was inserted")
            .authority,
    );
    let previous_decisions = previous.map(|previous| &previous.retained_by_constraint);
    let mut authority_fingerprints = BTreeMap::<
        RetentionConstraintMetadataKey,
        Arc<ExactGeneratedSourceAuthorityFingerprint>,
    >::new();
    retain_source_authorized_generated_contact_constraints(
        constraints,
        generated_constraint_start_index,
        previous_decisions,
        &mut current.retained_by_constraint,
        &current_authority,
        &mut authority_fingerprints,
        &mut stats,
    );
    retain_source_authorized_generated_contact_constraints(
        validation_constraints,
        generated_constraint_start_index,
        previous_decisions,
        &mut current.retained_by_constraint,
        &current_authority,
        &mut authority_fingerprints,
        &mut stats,
    );
    (current_authority, stats)
}

impl RetentionAuthorityContributorKey {
    fn from_sources(
        contours: &[NodeGeneratedContour],
        constraints: &[NodeRailConstraint],
        generated_constraint_start_index: usize,
    ) -> Self {
        let mut contours = contours
            .iter()
            .filter_map(|contour| {
                Some(RetentionContourContributorKey {
                    owner: contour.owner?,
                    source_mouth_order_index: contour.source_mouth_order_index,
                    source_band_index: contour.source_band_index,
                    ordered_points_xz: Arc::from(generated_contour_keys(contour)),
                })
            })
            .collect::<Vec<_>>();
        contours.sort_unstable();
        let mut source_constraints = constraints
            .iter()
            .take(generated_constraint_start_index)
            .filter(|constraint| generated_contact_kind_from_constraint(constraint.kind).is_some())
            .map(RetentionSourceConstraintContributorKey::from_constraint)
            .collect::<Vec<_>>();
        source_constraints.sort_unstable();
        Self {
            contours: Arc::from(contours),
            source_constraints: Arc::from(source_constraints),
        }
    }
}

impl RetentionSourceConstraintContributorKey {
    fn from_constraint(constraint: &NodeRailConstraint) -> Self {
        Self {
            kind: constraint.kind,
            source_mouth_order_index: constraint.source_mouth_order_index,
            source_band_index: constraint.source_band_index,
            owner: constraint.owner,
            opposite_owner: constraint.opposite_owner,
            ordered_points_xz: Arc::from(
                constraint
                    .points_xz
                    .iter()
                    .copied()
                    .map(road_point_key)
                    .collect::<Vec<_>>(),
            ),
        }
    }
}

impl RetentionConstraintMetadataKey {
    fn from_constraint(constraint: &NodeRailConstraint) -> Self {
        Self {
            kind: constraint.kind,
            source_mouth_order_index: constraint.source_mouth_order_index,
            source_band_index: constraint.source_band_index,
            owner: constraint.owner,
            opposite_owner: constraint.opposite_owner,
        }
    }
}

impl RetentionConstraintBucketKey {
    fn from_constraint(constraint: &NodeRailConstraint) -> Self {
        Self {
            metadata: RetentionConstraintMetadataKey::from_constraint(constraint),
            point_hash: retention_constraint_point_hash(constraint),
        }
    }
}

fn retain_source_authorized_generated_contact_constraints(
    constraints: &mut Vec<NodeRailConstraint>,
    generated_constraint_start_index: usize,
    previous: Option<&BTreeMap<RetentionConstraintBucketKey, Vec<RetainedContactDecision>>>,
    current: &mut BTreeMap<RetentionConstraintBucketKey, Vec<RetainedContactDecision>>,
    authority: &ExactGeneratedSourceAuthority,
    authority_fingerprints: &mut BTreeMap<
        RetentionConstraintMetadataKey,
        Arc<ExactGeneratedSourceAuthorityFingerprint>,
    >,
    stats: &mut NodeRetainedContactReuseStats,
) {
    let mut index = 0usize;
    constraints.retain(|constraint| {
        let retain = if index < generated_constraint_start_index
            || generated_contact_kind_from_constraint(constraint.kind).is_none()
        {
            true
        } else {
            retained_contact_decision(
                constraint,
                previous,
                current,
                authority,
                authority_fingerprints,
                stats,
            )
        };
        index += 1;
        retain
    });
}

fn retained_contact_decision(
    constraint: &NodeRailConstraint,
    previous: Option<&BTreeMap<RetentionConstraintBucketKey, Vec<RetainedContactDecision>>>,
    current: &mut BTreeMap<RetentionConstraintBucketKey, Vec<RetainedContactDecision>>,
    authority: &ExactGeneratedSourceAuthority,
    authority_fingerprints: &mut BTreeMap<
        RetentionConstraintMetadataKey,
        Arc<ExactGeneratedSourceAuthorityFingerprint>,
    >,
    stats: &mut NodeRetainedContactReuseStats,
) -> bool {
    let key = RetentionConstraintBucketKey::from_constraint(constraint);
    let authority_fingerprint = Arc::clone(
        authority_fingerprints
            .entry(key.metadata)
            .or_insert_with(|| {
                Arc::new(authority.relevant_fingerprint(
                    key.metadata.kind,
                    key.metadata.owner,
                    key.metadata.opposite_owner,
                    key.metadata.source_mouth_order_index,
                    key.metadata.source_band_index,
                ))
            }),
    );
    if let Some(retained) = retained_contact_decision_for_constraint(
        current.get(&key),
        constraint,
        &authority_fingerprint,
    ) {
        stats.decision_cache_hits += 1;
        stats.decision_current_hits += 1;
        return retained;
    }
    if let Some(previous_decision) =
        previous
            .and_then(|previous| previous.get(&key))
            .and_then(|decisions| {
                decisions.iter().find(|decision| {
                    retained_contact_points_match(decision, constraint)
                        && authority_fingerprints_match(
                            &decision.authority_fingerprint,
                            &authority_fingerprint,
                        )
                })
            })
    {
        stats.decision_cache_hits += 1;
        stats.decision_previous_hits += 1;
        current
            .entry(key)
            .or_default()
            .push(previous_decision.clone());
        return previous_decision.retained;
    }
    stats.decision_cache_misses += 1;
    let retained = generated_contact_constraint_has_exact_source_authority(constraint, authority);
    current
        .entry(key)
        .or_default()
        .push(RetainedContactDecision {
            ordered_points_xz: Arc::from(
                constraint
                    .points_xz
                    .iter()
                    .copied()
                    .map(road_point_key)
                    .collect::<Vec<_>>(),
            ),
            authority_fingerprint,
            retained,
        });
    retained
}

fn retained_contact_decision_for_constraint(
    decisions: Option<&Vec<RetainedContactDecision>>,
    constraint: &NodeRailConstraint,
    authority_fingerprint: &Arc<ExactGeneratedSourceAuthorityFingerprint>,
) -> Option<bool> {
    decisions?
        .iter()
        .find(|decision| {
            retained_contact_points_match(decision, constraint)
                && authority_fingerprints_match(
                    &decision.authority_fingerprint,
                    authority_fingerprint,
                )
        })
        .map(|decision| decision.retained)
}

fn authority_fingerprints_match(
    cached: &Arc<ExactGeneratedSourceAuthorityFingerprint>,
    current: &Arc<ExactGeneratedSourceAuthorityFingerprint>,
) -> bool {
    Arc::ptr_eq(cached, current) || cached.matches(current)
}

fn retained_contact_points_match(
    decision: &RetainedContactDecision,
    constraint: &NodeRailConstraint,
) -> bool {
    decision.ordered_points_xz.len() == constraint.points_xz.len()
        && decision
            .ordered_points_xz
            .iter()
            .copied()
            .zip(constraint.points_xz.iter().copied().map(road_point_key))
            .all(|(cached, current)| cached == current)
}

fn retention_constraint_point_hash(constraint: &NodeRailConstraint) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for (x, z) in constraint.points_xz.iter().copied().map(road_point_key) {
        hash ^= x as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
        hash ^= z as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash ^ constraint.points_xz.len() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::surface::node::RoadSurfaceBandKind;
    use crate::simulation::network::surface::node::backend::RoadVec2;

    fn raised_step_constraint(
        constraint_index: usize,
        owner: NodeBandOwner,
        opposite_owner: NodeBandOwner,
        start: RoadVec2,
        end: RoadVec2,
    ) -> NodeRailConstraint {
        raised_step_constraint_at_source(constraint_index, 0, owner, opposite_owner, start, end)
    }

    fn raised_step_constraint_at_source(
        constraint_index: usize,
        source_mouth_order_index: usize,
        owner: NodeBandOwner,
        opposite_owner: NodeBandOwner,
        start: RoadVec2,
        end: RoadVec2,
    ) -> NodeRailConstraint {
        NodeRailConstraint {
            constraint_index,
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index,
            source_band_index: Some(1),
            source_boundary_index: Some(1),
            owner: Some(owner),
            opposite_owner: Some(opposite_owner),
            points_xz: vec![start, end],
        }
    }

    fn run_retention_generation(
        sources: &[NodeRailConstraint],
        generated: &[NodeRailConstraint],
        previous: Option<&NodeRetainedContactCache>,
    ) -> (
        Vec<NodeRailConstraint>,
        Vec<NodeRailConstraint>,
        NodeRetainedContactCache,
        NodeRetainedContactReuseStats,
    ) {
        let original = sources.iter().chain(generated).cloned().collect::<Vec<_>>();
        let mut constraints = original.clone();
        let mut validation_constraints = original;
        let mut current = NodeRetainedContactCache::default();
        let (_, stats) = retain_source_authorized_generated_contact_constraint_sets_with_reuse(
            &[],
            &mut constraints,
            &mut validation_constraints,
            sources.len(),
            previous,
            &mut current,
        );
        (constraints, validation_constraints, current, stats)
    }

    fn assert_unrelated_authority_change_reuses_decisions(
        previous_sources: &[NodeRailConstraint],
        current_sources: &[NodeRailConstraint],
    ) {
        let asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let generated = [
            raised_step_constraint(
                100,
                asphalt_owner,
                curb_owner,
                RoadVec2::new(0.0, 0.0),
                RoadVec2::new(0.0, 0.0),
            ),
            raised_step_constraint(
                101,
                asphalt_owner,
                curb_owner,
                RoadVec2::new(2.0, 0.0),
                RoadVec2::new(2.0, 0.0),
            ),
        ];
        let (_, _, previous_cache, _) =
            run_retention_generation(previous_sources, &generated, None);
        let (warm, warm_validation, warm_cache, warm_stats) =
            run_retention_generation(current_sources, &generated, Some(&previous_cache));
        let (cold, cold_validation, _, cold_stats) =
            run_retention_generation(current_sources, &generated, None);
        let previous_authority = &previous_cache
            .context
            .as_ref()
            .expect("previous authority context")
            .authority;
        let warm_authority = &warm_cache
            .context
            .as_ref()
            .expect("warm authority context")
            .authority;
        let previous_fingerprint = previous_authority.relevant_fingerprint(
            NodeRailConstraintKind::RaisedStepContact,
            Some(asphalt_owner),
            Some(curb_owner),
            0,
            Some(1),
        );
        let warm_fingerprint = warm_authority.relevant_fingerprint(
            NodeRailConstraintKind::RaisedStepContact,
            Some(asphalt_owner),
            Some(curb_owner),
            0,
            Some(1),
        );

        assert_eq!(warm, cold);
        assert_eq!(warm_validation, cold_validation);
        assert!(!Arc::ptr_eq(previous_authority, warm_authority));
        assert!(previous_fingerprint.matches(&warm_fingerprint));
        assert_eq!(warm_stats.authority_cache_hits, 0);
        assert_eq!(warm_stats.authority_previous_hits, 0);
        assert_eq!(warm_stats.authority_cache_misses, 1);
        assert_eq!(warm_stats.decision_previous_hits, 2);
        assert_eq!(warm_stats.decision_current_hits, 2);
        assert_eq!(warm_stats.decision_cache_hits, 4);
        assert_eq!(warm_stats.decision_cache_misses, 0);
        assert_eq!(cold_stats.decision_cache_misses, 2);
    }

    #[test]
    fn retained_contact_cache_reuses_exact_decisions_and_invalidates_authority() {
        let asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let source = raised_step_constraint(
            0,
            asphalt_owner,
            curb_owner,
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(1.0, 0.0),
        );
        let valid = raised_step_constraint(
            1,
            asphalt_owner,
            curb_owner,
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(0.0, 0.0),
        );
        let invalid = raised_step_constraint(
            2,
            asphalt_owner,
            curb_owner,
            RoadVec2::new(2.0, 0.0),
            RoadVec2::new(2.0, 0.0),
        );
        let original = vec![source, valid, invalid];

        let mut first = original.clone();
        let mut first_validation = original.clone();
        let mut first_cache = NodeRetainedContactCache::default();
        let (_, first_stats) =
            retain_source_authorized_generated_contact_constraint_sets_with_reuse(
                &[],
                &mut first,
                &mut first_validation,
                1,
                None,
                &mut first_cache,
            );
        assert_eq!(first.len(), 2);
        assert_eq!(first, first_validation);
        assert_eq!(first_stats.authority_cache_misses, 1);
        assert_eq!(first_stats.decision_cache_misses, 2);
        assert_eq!(first_stats.decision_cache_hits, 2);
        assert_eq!(first_stats.decision_current_hits, 2);
        assert_eq!(first_stats.decision_previous_hits, 0);

        let mut reused = original.clone();
        let mut reused_validation = original.clone();
        let mut reused_cache = NodeRetainedContactCache::default();
        let (_, reused_stats) =
            retain_source_authorized_generated_contact_constraint_sets_with_reuse(
                &[],
                &mut reused,
                &mut reused_validation,
                1,
                Some(&first_cache),
                &mut reused_cache,
            );
        assert_eq!(reused, first);
        assert_eq!(reused_validation, first_validation);
        assert_eq!(reused_stats.authority_cache_hits, 1);
        assert_eq!(reused_stats.authority_previous_hits, 1);
        assert_eq!(reused_stats.authority_current_hits, 0);
        assert_eq!(reused_stats.authority_cache_misses, 0);
        assert_eq!(reused_stats.decision_cache_misses, 0);
        assert_eq!(reused_stats.decision_cache_hits, 4);
        assert_eq!(reused_stats.decision_previous_hits, 2);
        assert_eq!(reused_stats.decision_current_hits, 2);

        let mut changed = original;
        changed[0].points_xz[0] = RoadVec2::new(0.5, 0.0);
        let mut changed_reused = changed.clone();
        let mut changed_reused_validation = changed.clone();
        let mut changed_reused_cache = NodeRetainedContactCache::default();
        let (_, changed_reused_stats) =
            retain_source_authorized_generated_contact_constraint_sets_with_reuse(
                &[],
                &mut changed_reused,
                &mut changed_reused_validation,
                1,
                Some(&first_cache),
                &mut changed_reused_cache,
            );
        let mut changed_cold = changed.clone();
        let mut changed_cold_validation = changed;
        let mut changed_cold_cache = NodeRetainedContactCache::default();
        retain_source_authorized_generated_contact_constraint_sets_with_reuse(
            &[],
            &mut changed_cold,
            &mut changed_cold_validation,
            1,
            None,
            &mut changed_cold_cache,
        );
        assert_eq!(changed_reused, changed_cold);
        assert_eq!(changed_reused_validation, changed_cold_validation);
        assert_eq!(changed_reused_stats.authority_cache_hits, 0);
        assert_eq!(changed_reused_stats.authority_cache_misses, 1);
        assert_eq!(changed_reused_stats.decision_previous_hits, 0);
        assert_eq!(changed_reused_stats.decision_cache_misses, 2);
    }

    #[test]
    fn retained_contact_cache_reuses_decisions_after_unrelated_source_add() {
        let asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let sidewalk_owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 10);
        let unrelated_curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 11);
        let source = raised_step_constraint(
            0,
            asphalt_owner,
            curb_owner,
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(1.0, 0.0),
        );
        let unrelated = raised_step_constraint_at_source(
            1,
            7,
            unrelated_curb_owner,
            sidewalk_owner,
            RoadVec2::new(10.0, 0.0),
            RoadVec2::new(11.0, 0.0),
        );

        assert_unrelated_authority_change_reuses_decisions(&[source.clone()], &[source, unrelated]);
    }

    #[test]
    fn retained_contact_cache_reuses_decisions_after_unrelated_source_remove() {
        let asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let sidewalk_owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 10);
        let unrelated_curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 11);
        let source = raised_step_constraint(
            0,
            asphalt_owner,
            curb_owner,
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(1.0, 0.0),
        );
        let unrelated = raised_step_constraint_at_source(
            1,
            7,
            unrelated_curb_owner,
            sidewalk_owner,
            RoadVec2::new(10.0, 0.0),
            RoadVec2::new(11.0, 0.0),
        );

        assert_unrelated_authority_change_reuses_decisions(&[source.clone(), unrelated], &[source]);
    }

    #[test]
    fn retained_contact_cache_reuses_decisions_after_unrelated_source_edit() {
        let asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let sidewalk_owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 10);
        let unrelated_curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 11);
        let source = raised_step_constraint(
            0,
            asphalt_owner,
            curb_owner,
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(1.0, 0.0),
        );
        let unrelated = raised_step_constraint_at_source(
            1,
            7,
            unrelated_curb_owner,
            sidewalk_owner,
            RoadVec2::new(10.0, 0.0),
            RoadVec2::new(11.0, 0.0),
        );
        let mut changed_unrelated = unrelated.clone();
        changed_unrelated.points_xz[0] = RoadVec2::new(10.5, 0.0);

        assert_unrelated_authority_change_reuses_decisions(
            &[source.clone(), unrelated],
            &[source, changed_unrelated],
        );
    }
}
