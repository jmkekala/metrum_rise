// SPDX-License-Identifier: GPL-2.0-only

//! Exact source-authority queries for generated contacts.

use super::super::super::{
    GeneratedContourEdgeKey, NodeBandOwner, NodeRailConstraintKind, NodeRailPointKey,
    RoadSurfaceBandKind,
};
use super::{
    ExactContactSourceBucket, ExactGeneratedSourceAuthority, exact_contact_presence_key,
    exact_generated_contact_owner_pair,
};
use std::collections::BTreeSet;

impl ExactGeneratedSourceAuthority {
    pub(in crate::simulation::network::surface::node::rails::contacts::validation) fn has_any_source(
        &self,
        owners: [Option<NodeBandOwner>; 2],
        source_mouth_order_index: usize,
        source_band_index: Option<usize>,
    ) -> bool {
        let has_source_contour = source_band_index.is_some_and(|source_band_index| {
            owners.into_iter().flatten().any(|owner| {
                self.keys_by_source.contains_key(&(
                    owner,
                    source_mouth_order_index,
                    source_band_index,
                ))
            })
        });
        if has_source_contour {
            return true;
        }
        let (Some(owner), Some(opposite_owner)) = (owners[0], owners[1]) else {
            return false;
        };
        self.contact_sources_by_presence
            .contains_key(&exact_contact_presence_key(
                owner,
                opposite_owner,
                source_mouth_order_index,
                source_band_index,
            ))
    }

    pub(in crate::simulation::network::surface::node::rails::contacts::validation) fn has_exact_point(
        &self,
        owners: [Option<NodeBandOwner>; 2],
        source_mouth_order_index: usize,
        source_band_index: Option<usize>,
        point: NodeRailPointKey,
    ) -> bool {
        let Some(source_band_index) = source_band_index else {
            return false;
        };
        owners.into_iter().flatten().any(|owner| {
            self.keys_by_source
                .get(&(owner, source_mouth_order_index, source_band_index))
                .is_some_and(|keys| keys.contains(&point))
        })
    }

    pub(in crate::simulation::network::surface::node::rails::contacts::validation) fn has_exact_source_key(
        &self,
        kind: NodeRailConstraintKind,
        owners: [Option<NodeBandOwner>; 2],
        source_mouth_order_index: usize,
        source_band_index: Option<usize>,
        point: NodeRailPointKey,
    ) -> bool {
        let (Some(owner), Some(opposite_owner)) = (owners[0], owners[1]) else {
            return false;
        };
        let Some((owner, opposite_owner)) =
            exact_generated_contact_owner_pair(kind, owner, opposite_owner)
        else {
            return false;
        };
        self.segments_by_contact_source
            .get(&(
                kind,
                owner,
                opposite_owner,
                source_mouth_order_index,
                source_band_index,
            ))
            .is_some_and(|segments| generated_segments_have_endpoint(segments, point))
    }

    pub(in crate::simulation::network::surface::node::rails::contacts::validation) fn has_exact_same_kind_source_handoff_key(
        &self,
        kind: NodeRailConstraintKind,
        owners: [Option<NodeBandOwner>; 2],
        source_mouth_order_index: usize,
        source_band_index: Option<usize>,
        point: NodeRailPointKey,
    ) -> bool {
        let (Some(left_owner), Some(right_owner)) = (owners[0], owners[1]) else {
            return false;
        };
        self.has_exact_same_kind_source_handoff_side(
            kind,
            left_owner,
            right_owner,
            source_mouth_order_index,
            source_band_index,
            point,
        ) || self.has_exact_same_kind_source_handoff_side(
            kind,
            right_owner,
            left_owner,
            source_mouth_order_index,
            source_band_index,
            point,
        )
    }

    fn has_exact_same_kind_source_handoff_side(
        &self,
        kind: NodeRailConstraintKind,
        retained_owner: NodeBandOwner,
        final_owner: NodeBandOwner,
        source_mouth_order_index: usize,
        source_band_index: Option<usize>,
        point: NodeRailPointKey,
    ) -> bool {
        if !self.owner_geometry_has_exact_key(final_owner, point) {
            return false;
        }
        self.contact_sources_for_owner_kind(kind, retained_owner, final_owner.kind())
            .iter()
            .filter(|source| {
                source.key.3 == source_mouth_order_index && source.key.4 == source_band_index
            })
            .any(|source| generated_segments_have_endpoint(&source.segments, point))
    }

    pub(in crate::simulation::network::surface::node::rails::contacts::validation) fn has_exact_cross_source_same_kind_contact_key(
        &self,
        kind: NodeRailConstraintKind,
        owners: [Option<NodeBandOwner>; 2],
        source_mouth_order_index: usize,
        source_band_index: Option<usize>,
        point: NodeRailPointKey,
    ) -> bool {
        let (Some(left_owner), Some(right_owner)) = (owners[0], owners[1]) else {
            return false;
        };
        (self.has_same_kind_source_key_for_owner(
            kind,
            left_owner,
            right_owner.kind(),
            Some(source_mouth_order_index),
            source_band_index,
            point,
        ) && self.has_same_kind_source_key_for_owner(
            kind,
            right_owner,
            left_owner.kind(),
            None,
            None,
            point,
        )) || (self.has_same_kind_source_key_for_owner(
            kind,
            left_owner,
            right_owner.kind(),
            None,
            None,
            point,
        ) && self.has_same_kind_source_key_for_owner(
            kind,
            right_owner,
            left_owner.kind(),
            Some(source_mouth_order_index),
            source_band_index,
            point,
        ))
    }

    fn has_same_kind_source_key_for_owner(
        &self,
        kind: NodeRailConstraintKind,
        owner: NodeBandOwner,
        counterpart_kind: RoadSurfaceBandKind,
        required_source_mouth_order_index: Option<usize>,
        required_source_band_index: Option<usize>,
        point: NodeRailPointKey,
    ) -> bool {
        self.contact_sources_for_owner_kind(kind, owner, counterpart_kind)
            .iter()
            .filter(|source| {
                required_source_mouth_order_index.is_none_or(|required| source.key.3 == required)
                    && required_source_band_index
                        .is_none_or(|required| source.key.4 == Some(required))
            })
            .any(|source| generated_segments_have_endpoint(&source.segments, point))
    }

    fn owner_geometry_has_exact_key(&self, owner: NodeBandOwner, point: NodeRailPointKey) -> bool {
        self.keys_by_owner
            .get(&owner)
            .is_some_and(|keys| keys.contains(&point))
            || self
                .segments_by_owner
                .get(&owner)
                .is_some_and(|segments| generated_segments_have_endpoint(segments, point))
    }

    fn contact_sources_for_owner_kind(
        &self,
        kind: NodeRailConstraintKind,
        owner: NodeBandOwner,
        counterpart_kind: RoadSurfaceBandKind,
    ) -> &[ExactContactSourceBucket] {
        self.contact_sources_by_owner_kind
            .get(&(kind, owner, counterpart_kind))
            .map_or(&[], AsRef::as_ref)
    }
}

fn generated_segments_have_endpoint(
    segments: &BTreeSet<GeneratedContourEdgeKey>,
    point: NodeRailPointKey,
) -> bool {
    segments
        .iter()
        .any(|segment| segment.start == point || segment.end == point)
}
