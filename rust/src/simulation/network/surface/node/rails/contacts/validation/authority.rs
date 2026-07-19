//! Exact generated-contact source authority index.

use super::super::source_authority::generated_contact_kind_from_constraint;
use super::super::{
    GeneratedContourEdgeKey, GeneratedRaisedStepOwnerPair, NodeBandOwner, NodeRailConstraintKind,
    NodeRailPointKey, RoadSurfaceBandKind,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

mod build;
mod query;

type ExactContactSourceKey = (
    NodeRailConstraintKind,
    NodeBandOwner,
    NodeBandOwner,
    usize,
    Option<usize>,
);

type ExactContactPresenceKey = (NodeBandOwner, NodeBandOwner, usize, Option<usize>);

type ExactContactOwnerKindKey = (NodeRailConstraintKind, NodeBandOwner, RoadSurfaceBandKind);

type AuthorityPointSet = Arc<BTreeSet<NodeRailPointKey>>;
type AuthoritySegmentSet = Arc<BTreeSet<GeneratedContourEdgeKey>>;

#[derive(Clone, Debug, Eq, PartialEq)]
struct ExactContactSourceBucket {
    key: ExactContactSourceKey,
    segments: AuthoritySegmentSet,
}

type ExactContactSourceBucketSet = Arc<[ExactContactSourceBucket]>;

/// Immutable authority buckets that can affect one generated-contact decision.
#[derive(Clone, Debug, Default)]
pub(in crate::simulation::network::surface::node::rails) struct ExactGeneratedSourceAuthorityFingerprint
{
    owner_points: Arc<[(NodeBandOwner, AuthorityPointSet)]>,
    owner_segments: Arc<[(NodeBandOwner, AuthoritySegmentSet)]>,
    source_points: Arc<[((NodeBandOwner, usize, usize), AuthorityPointSet)]>,
    contact_presence: Option<(ExactContactPresenceKey, ExactContactSourceBucketSet)>,
    contact_owner_kinds: Arc<[(ExactContactOwnerKindKey, ExactContactSourceBucketSet)]>,
}

#[derive(Clone, Debug)]
pub(in crate::simulation::network::surface::node::rails) struct ExactGeneratedSourceAuthority {
    keys_by_owner: BTreeMap<NodeBandOwner, AuthorityPointSet>,
    segments_by_owner: BTreeMap<NodeBandOwner, AuthoritySegmentSet>,
    keys_by_source: BTreeMap<(NodeBandOwner, usize, usize), AuthorityPointSet>,
    segments_by_contact_source: BTreeMap<ExactContactSourceKey, AuthoritySegmentSet>,
    contact_sources_by_presence: BTreeMap<ExactContactPresenceKey, ExactContactSourceBucketSet>,
    contact_sources_by_owner_kind: BTreeMap<ExactContactOwnerKindKey, ExactContactSourceBucketSet>,
}

fn exact_generated_contact_owner_pair(
    kind: NodeRailConstraintKind,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> Option<(NodeBandOwner, NodeBandOwner)> {
    if generated_contact_kind_from_constraint(kind).is_none() {
        return None;
    }
    if kind == NodeRailConstraintKind::RaisedStepContact {
        let pair = GeneratedRaisedStepOwnerPair::new(owner, opposite_owner)?;
        return Some((pair.owner, pair.opposite_owner));
    }
    Some((owner.min(opposite_owner), owner.max(opposite_owner)))
}

fn exact_contact_presence_key(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
) -> ExactContactPresenceKey {
    (
        owner.min(opposite_owner),
        owner.max(opposite_owner),
        source_mouth_order_index,
        source_band_index,
    )
}

impl ExactGeneratedSourceAuthority {
    /// Collects shared authority buckets relevant to one generated-contact metadata cohort.
    pub(in crate::simulation::network::surface::node::rails) fn relevant_fingerprint(
        &self,
        kind: NodeRailConstraintKind,
        owner: Option<NodeBandOwner>,
        opposite_owner: Option<NodeBandOwner>,
        source_mouth_order_index: usize,
        source_band_index: Option<usize>,
    ) -> ExactGeneratedSourceAuthorityFingerprint {
        let (Some(owner), Some(opposite_owner)) = (owner, opposite_owner) else {
            return ExactGeneratedSourceAuthorityFingerprint::default();
        };
        let mut relevant_owners = vec![owner, opposite_owner];
        relevant_owners.sort_unstable();
        relevant_owners.dedup();

        let owner_points = relevant_owners
            .iter()
            .filter_map(|owner| {
                self.keys_by_owner
                    .get(owner)
                    .map(|points| (*owner, Arc::clone(points)))
            })
            .collect::<Vec<_>>();
        let owner_segments = relevant_owners
            .iter()
            .filter_map(|owner| {
                self.segments_by_owner
                    .get(owner)
                    .map(|segments| (*owner, Arc::clone(segments)))
            })
            .collect::<Vec<_>>();
        let source_points = source_band_index.map_or_else(Vec::new, |source_band_index| {
            relevant_owners
                .iter()
                .filter_map(|owner| {
                    let key = (*owner, source_mouth_order_index, source_band_index);
                    self.keys_by_source
                        .get(&key)
                        .map(|points| (key, Arc::clone(points)))
                })
                .collect()
        });

        let presence_key = exact_contact_presence_key(
            owner,
            opposite_owner,
            source_mouth_order_index,
            source_band_index,
        );
        let contact_presence = self
            .contact_sources_by_presence
            .get(&presence_key)
            .map(|sources| (presence_key, Arc::clone(sources)));

        let mut owner_kind_keys = vec![
            (kind, owner, opposite_owner.kind()),
            (kind, opposite_owner, owner.kind()),
        ];
        owner_kind_keys.sort_unstable();
        owner_kind_keys.dedup();
        let contact_owner_kinds = owner_kind_keys
            .into_iter()
            .filter_map(|key| {
                self.contact_sources_by_owner_kind
                    .get(&key)
                    .map(|sources| (key, Arc::clone(sources)))
            })
            .collect::<Vec<_>>();

        ExactGeneratedSourceAuthorityFingerprint {
            owner_points: Arc::from(owner_points),
            owner_segments: Arc::from(owner_segments),
            source_points: Arc::from(source_points),
            contact_presence,
            contact_owner_kinds: Arc::from(contact_owner_kinds),
        }
    }
}

impl ExactGeneratedSourceAuthorityFingerprint {
    /// Returns whether both fingerprints reference the same immutable authority payloads.
    pub(in crate::simulation::network::surface::node::rails) fn matches(
        &self,
        other: &Self,
    ) -> bool {
        // Authority reconstruction interns structurally unchanged payloads against the
        // immediately previous generation. Pointer equality is therefore the exact
        // contributor identity check and avoids rescanning bucket geometry here.
        shared_arc_entries_match(&self.owner_points, &other.owner_points)
            && shared_arc_entries_match(&self.owner_segments, &other.owner_segments)
            && shared_arc_entries_match(&self.source_points, &other.source_points)
            && optional_shared_arc_entry_matches(
                self.contact_presence.as_ref(),
                other.contact_presence.as_ref(),
            )
            && shared_arc_entries_match(&self.contact_owner_kinds, &other.contact_owner_kinds)
    }
}

fn shared_arc_entries_match<K, T>(left: &[(K, Arc<T>)], right: &[(K, Arc<T>)]) -> bool
where
    K: Eq,
    T: ?Sized,
{
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|((left_key, left_value), (right_key, right_value))| {
                left_key == right_key && Arc::ptr_eq(left_value, right_value)
            })
}

fn optional_shared_arc_entry_matches<K, T>(
    left: Option<&(K, Arc<T>)>,
    right: Option<&(K, Arc<T>)>,
) -> bool
where
    K: Eq,
    T: ?Sized,
{
    match (left, right) {
        (Some((left_key, left_value)), Some((right_key, right_value))) => {
            left_key == right_key && Arc::ptr_eq(left_value, right_value)
        }
        (None, None) => true,
        (Some(_), None) | (None, Some(_)) => false,
    }
}
