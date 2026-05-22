//! Exact source-authority queries for generated contacts.

use super::super::super::{
    GeneratedContourEdgeKey, NodeBandOwner, NodeRailConstraintKind, NodeRailPointKey,
    RoadSurfaceBandKind, owners_match_unordered,
};
use super::{ExactGeneratedSourceAuthority, exact_generated_contact_owner_pair};
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
        self.segments_by_contact_source.keys().any(
            |(_, source_owner, source_opposite_owner, source_mouth, source_band)| {
                *source_mouth == source_mouth_order_index
                    && *source_band == source_band_index
                    && owners_match_unordered(
                        Some(*source_owner),
                        Some(*source_opposite_owner),
                        owner,
                        opposite_owner,
                    )
            },
        )
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
        self.segments_by_contact_source
            .iter()
            .filter(|((source_kind, _, _, source_mouth, source_band), _)| {
                *source_kind == kind
                    && *source_mouth == source_mouth_order_index
                    && *source_band == source_band_index
            })
            .any(
                |((_, source_owner, source_opposite_owner, _, _), segments)| {
                    let same_kind_handoff = (*source_owner == retained_owner
                        && source_opposite_owner.kind() == final_owner.kind())
                        || (*source_opposite_owner == retained_owner
                            && source_owner.kind() == final_owner.kind());
                    same_kind_handoff && generated_segments_have_endpoint(segments, point)
                },
            )
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
        self.segments_by_contact_source
            .iter()
            .filter(|((source_kind, _, _, source_mouth, source_band), _)| {
                *source_kind == kind
                    && required_source_mouth_order_index
                        .is_none_or(|required| *source_mouth == required)
                    && required_source_band_index
                        .is_none_or(|required| *source_band == Some(required))
            })
            .any(
                |((_, source_owner, source_opposite_owner, _, _), segments)| {
                    let owner_matches = (*source_owner == owner
                        && source_opposite_owner.kind() == counterpart_kind)
                        || (*source_opposite_owner == owner
                            && source_owner.kind() == counterpart_kind);
                    owner_matches && generated_segments_have_endpoint(segments, point)
                },
            )
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
}

fn generated_segments_have_endpoint(
    segments: &BTreeSet<GeneratedContourEdgeKey>,
    point: NodeRailPointKey,
) -> bool {
    segments
        .iter()
        .any(|segment| segment.start == point || segment.end == point)
}
