//! Source-authority index construction for generated contacts.

use super::super::super::source_authority::generated_contact_kind_from_constraint;
use super::super::super::{
    GeneratedContourEdgeKey, NodeBandOwner, NodeGeneratedContour, NodeRailConstraint,
    NodeRailPointKey, generated_constraint_directed_edges, generated_contour_directed_edges,
    generated_contour_keys, road_point_key,
};
use super::{
    ExactContactSourceKey, ExactGeneratedSourceAuthority, exact_generated_contact_owner_pair,
};
use std::collections::{BTreeMap, BTreeSet};

impl ExactGeneratedSourceAuthority {
    pub(in crate::simulation::network::surface::node::rails::contacts::validation) fn from_sources(
        contours: &[NodeGeneratedContour],
        constraints: &[NodeRailConstraint],
        generated_constraint_start_index: usize,
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
        Self {
            keys_by_owner,
            segments_by_owner,
            keys_by_source,
            segments_by_contact_source,
        }
    }
}
