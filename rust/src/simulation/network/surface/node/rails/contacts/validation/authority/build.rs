// SPDX-License-Identifier: GPL-2.0-only

//! Source-authority index construction for generated contacts.

use super::super::super::source_authority::generated_contact_kind_from_constraint;
use super::super::super::{
    GeneratedContourEdgeKey, NodeBandOwner, NodeGeneratedContour, NodeRailConstraint,
    NodeRailPointKey, generated_constraint_directed_edges, generated_contour_directed_edges,
    generated_contour_keys, road_point_key,
};
use super::{
    AuthoritySegmentSet, ExactContactOwnerKindKey, ExactContactPresenceKey,
    ExactContactSourceBucket, ExactContactSourceBucketSet, ExactContactSourceKey,
    ExactGeneratedSourceAuthority, exact_contact_presence_key, exact_generated_contact_owner_pair,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

impl ExactGeneratedSourceAuthority {
    #[cfg(test)]
    pub(in crate::simulation::network::surface::node::rails) fn from_sources(
        contours: &[NodeGeneratedContour],
        constraints: &[NodeRailConstraint],
        generated_constraint_start_index: usize,
    ) -> Self {
        Self::from_sources_with_reuse(
            contours,
            constraints,
            generated_constraint_start_index,
            None,
        )
    }

    /// Builds current authority while retaining unchanged immutable buckets from `previous`.
    pub(in crate::simulation::network::surface::node::rails) fn from_sources_with_reuse(
        contours: &[NodeGeneratedContour],
        constraints: &[NodeRailConstraint],
        generated_constraint_start_index: usize,
        previous: Option<&Self>,
    ) -> Self {
        let mut keys_by_owner = BTreeMap::<NodeBandOwner, BTreeSet<NodeRailPointKey>>::new();
        let mut segments_by_owner =
            BTreeMap::<NodeBandOwner, BTreeSet<GeneratedContourEdgeKey>>::new();
        let mut keys_by_source =
            BTreeMap::<(NodeBandOwner, usize, usize), BTreeSet<NodeRailPointKey>>::new();
        let mut segments_by_contact_source =
            BTreeMap::<ExactContactSourceKey, BTreeSet<GeneratedContourEdgeKey>>::new();
        for contour in contours {
            let Some(owner) = contour.owner else {
                continue;
            };
            let keys = generated_contour_keys(contour);
            keys_by_owner
                .entry(owner)
                .or_default()
                .extend(keys.iter().copied());
            segments_by_owner.entry(owner).or_default().extend(
                generated_contour_directed_edges(contour)
                    .into_iter()
                    .map(|edge| GeneratedContourEdgeKey::new(edge.start, edge.end)),
            );
            let Some(source_band_index) = contour.source_band_index else {
                continue;
            };
            keys_by_source
                .entry((owner, contour.source_mouth_order_index, source_band_index))
                .or_default()
                .extend(keys.into_iter());
        }
        for constraint in constraints.iter().take(generated_constraint_start_index) {
            if generated_contact_kind_from_constraint(constraint.kind).is_none() {
                continue;
            }
            if let Some(source_band_index) = constraint.source_band_index {
                let owners = [constraint.owner, constraint.opposite_owner];
                for owner in owners.into_iter().flatten() {
                    keys_by_source
                        .entry((
                            owner,
                            constraint.source_mouth_order_index,
                            source_band_index,
                        ))
                        .or_default()
                        .extend(constraint.points_xz.iter().copied().map(road_point_key));
                }
            }
            let (Some(owner), Some(opposite_owner)) = (constraint.owner, constraint.opposite_owner)
            else {
                continue;
            };
            let Some((owner, opposite_owner)) =
                exact_generated_contact_owner_pair(constraint.kind, owner, opposite_owner)
            else {
                continue;
            };
            segments_by_contact_source
                .entry((
                    constraint.kind,
                    owner,
                    opposite_owner,
                    constraint.source_mouth_order_index,
                    constraint.source_band_index,
                ))
                .or_default()
                .extend(
                    generated_constraint_directed_edges(constraint)
                        .into_iter()
                        .map(|edge| GeneratedContourEdgeKey::new(edge.start, edge.end)),
                );
        }
        let keys_by_owner = freeze_set_map(
            keys_by_owner,
            previous.map(|previous| &previous.keys_by_owner),
        );
        let segments_by_owner = freeze_set_map(
            segments_by_owner,
            previous.map(|previous| &previous.segments_by_owner),
        );
        let keys_by_source = freeze_set_map(
            keys_by_source,
            previous.map(|previous| &previous.keys_by_source),
        );
        let segments_by_contact_source = freeze_set_map(
            segments_by_contact_source,
            previous.map(|previous| &previous.segments_by_contact_source),
        );
        let (presence_keys, owner_kind_keys) =
            contact_source_reverse_index_keys(&segments_by_contact_source);
        let contact_sources_by_presence = freeze_contact_source_index(
            presence_keys,
            &segments_by_contact_source,
            previous.map(|previous| &previous.contact_sources_by_presence),
        );
        let contact_sources_by_owner_kind = freeze_contact_source_index(
            owner_kind_keys,
            &segments_by_contact_source,
            previous.map(|previous| &previous.contact_sources_by_owner_kind),
        );
        Self {
            keys_by_owner,
            segments_by_owner,
            keys_by_source,
            segments_by_contact_source,
            contact_sources_by_presence,
            contact_sources_by_owner_kind,
        }
    }
}

fn freeze_set_map<K, V>(
    values: BTreeMap<K, BTreeSet<V>>,
    previous: Option<&BTreeMap<K, Arc<BTreeSet<V>>>>,
) -> BTreeMap<K, Arc<BTreeSet<V>>>
where
    K: Ord,
    V: Ord + Eq,
{
    values
        .into_iter()
        .map(|(key, values)| {
            let values = previous
                .and_then(|previous| previous.get(&key))
                .filter(|previous| previous.as_ref() == &values)
                .map(Arc::clone)
                .unwrap_or_else(|| Arc::new(values));
            (key, values)
        })
        .collect()
}

fn contact_source_reverse_index_keys(
    sources: &BTreeMap<ExactContactSourceKey, AuthoritySegmentSet>,
) -> (
    BTreeMap<ExactContactPresenceKey, BTreeSet<ExactContactSourceKey>>,
    BTreeMap<ExactContactOwnerKindKey, BTreeSet<ExactContactSourceKey>>,
) {
    let mut by_presence =
        BTreeMap::<ExactContactPresenceKey, BTreeSet<ExactContactSourceKey>>::new();
    let mut by_owner_kind =
        BTreeMap::<ExactContactOwnerKindKey, BTreeSet<ExactContactSourceKey>>::new();
    for &key @ (kind, owner, opposite_owner, source_mouth, source_band) in sources.keys() {
        by_presence
            .entry(exact_contact_presence_key(
                owner,
                opposite_owner,
                source_mouth,
                source_band,
            ))
            .or_default()
            .insert(key);
        by_owner_kind
            .entry((kind, owner, opposite_owner.kind()))
            .or_default()
            .insert(key);
        by_owner_kind
            .entry((kind, opposite_owner, owner.kind()))
            .or_default()
            .insert(key);
    }
    (by_presence, by_owner_kind)
}

fn freeze_contact_source_index<K>(
    index_keys: BTreeMap<K, BTreeSet<ExactContactSourceKey>>,
    sources: &BTreeMap<ExactContactSourceKey, AuthoritySegmentSet>,
    previous: Option<&BTreeMap<K, ExactContactSourceBucketSet>>,
) -> BTreeMap<K, ExactContactSourceBucketSet>
where
    K: Ord,
{
    index_keys
        .into_iter()
        .map(|(key, source_keys)| {
            let buckets = source_keys
                .into_iter()
                .map(|source_key| ExactContactSourceBucket {
                    key: source_key,
                    segments: Arc::clone(
                        sources
                            .get(&source_key)
                            .expect("reverse-indexed contact source must exist"),
                    ),
                })
                .collect::<Vec<_>>();
            let buckets = previous
                .and_then(|previous| previous.get(&key))
                .filter(|previous| contact_source_buckets_share_payloads(previous, &buckets))
                .map(Arc::clone)
                .unwrap_or_else(|| Arc::from(buckets));
            (key, buckets)
        })
        .collect()
}

fn contact_source_buckets_share_payloads(
    previous: &[ExactContactSourceBucket],
    current: &[ExactContactSourceBucket],
) -> bool {
    previous.len() == current.len()
        && previous.iter().zip(current).all(|(previous, current)| {
            previous.key == current.key && Arc::ptr_eq(&previous.segments, &current.segments)
        })
}
