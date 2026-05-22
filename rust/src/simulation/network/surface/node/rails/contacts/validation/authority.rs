//! Exact generated-contact source authority index.

use super::super::source_authority::generated_contact_kind_from_constraint;
use super::super::{
    GeneratedContourEdgeKey, GeneratedRaisedStepOwnerPair, NodeBandOwner, NodeRailConstraintKind,
    NodeRailPointKey,
};
use std::collections::{BTreeMap, BTreeSet};

mod build;
mod query;

type ExactContactSourceKey = (
    NodeRailConstraintKind,
    NodeBandOwner,
    NodeBandOwner,
    usize,
    Option<usize>,
);

pub(super) struct ExactGeneratedSourceAuthority {
    pub(super) keys_by_owner: BTreeMap<NodeBandOwner, BTreeSet<NodeRailPointKey>>,
    pub(super) segments_by_owner: BTreeMap<NodeBandOwner, BTreeSet<GeneratedContourEdgeKey>>,
    pub(super) keys_by_source: BTreeMap<(NodeBandOwner, usize, usize), BTreeSet<NodeRailPointKey>>,
    pub(super) segments_by_contact_source:
        BTreeMap<ExactContactSourceKey, BTreeSet<GeneratedContourEdgeKey>>,
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
