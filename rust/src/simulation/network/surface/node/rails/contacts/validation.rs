//! Exact source validation for generated rail contacts.

use super::source_authority::generated_contact_kind_from_constraint;
use super::{
    GeneratedContourEdgeKey, GeneratedRaisedStepOwnerPair, NodeBandOwner, NodeGeneratedContour,
    NodeRailConstraint, NodeRailConstraintKind, NodeRailGenerationError, NodeRailPointKey,
    RoadSurfaceBandKind, generated_constraint_directed_edges, generated_contour_directed_edges,
    generated_contour_keys, owners_match_unordered, road_point_key,
};
use std::collections::{BTreeMap, BTreeSet};

pub(in crate::simulation::network::surface::node::rails) fn retain_source_authorized_generated_contact_constraints(
    contours: &[NodeGeneratedContour],
    authority_constraints: &[NodeRailConstraint],
    constraints: &mut Vec<NodeRailConstraint>,
    generated_constraint_start_index: usize,
) {
    let source_authority = ExactGeneratedSourceAuthority::from_sources(
        contours,
        authority_constraints,
        generated_constraint_start_index,
    );
    let mut index = 0usize;
    constraints.retain(|constraint| {
        let retain = index < generated_constraint_start_index
            || generated_contact_kind_from_constraint(constraint.kind).is_none()
            || generated_contact_constraint_has_exact_source_authority(
                constraint,
                &source_authority,
            );
        index += 1;
        retain
    });
}

fn generated_contact_constraint_has_exact_source_authority(
    constraint: &NodeRailConstraint,
    source_authority: &ExactGeneratedSourceAuthority,
) -> bool {
    let source_band_index = constraint.source_band_index;
    if constraint.owner.is_none() || constraint.opposite_owner.is_none() {
        return true;
    }
    let owners = [constraint.owner, constraint.opposite_owner];
    if !source_authority.has_any_source(
        owners,
        constraint.source_mouth_order_index,
        source_band_index,
    ) {
        return true;
    }
    constraint.points_xz.iter().copied().all(|point| {
        generated_contact_constraint_endpoint_has_exact_source_authority(
            constraint,
            source_authority,
            owners,
            source_band_index,
            road_point_key(point),
        )
    })
}

fn generated_contact_constraint_endpoint_has_exact_source_authority(
    constraint: &NodeRailConstraint,
    source_authority: &ExactGeneratedSourceAuthority,
    owners: [Option<NodeBandOwner>; 2],
    source_band_index: Option<usize>,
    key: NodeRailPointKey,
) -> bool {
    source_authority.has_exact_point(
        owners,
        constraint.source_mouth_order_index,
        source_band_index,
        key,
    ) || source_authority.has_exact_source_key(
        constraint.kind,
        owners,
        constraint.source_mouth_order_index,
        constraint.source_band_index,
        key,
    ) || source_authority.has_exact_same_kind_source_handoff_key(
        constraint.kind,
        owners,
        constraint.source_mouth_order_index,
        constraint.source_band_index,
        key,
    ) || source_authority.has_exact_cross_source_same_kind_contact_key(
        constraint.kind,
        owners,
        constraint.source_mouth_order_index,
        constraint.source_band_index,
        key,
    )
}

pub(in crate::simulation::network::surface::node::rails) fn validate_generated_contact_constraint_endpoints_from_sources(
    contours: &[NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
    generated_constraint_start_index: usize,
) -> Result<(), NodeRailGenerationError> {
    let source_authority = ExactGeneratedSourceAuthority::from_sources(
        contours,
        constraints,
        generated_constraint_start_index,
    );
    for constraint in constraints.iter().skip(generated_constraint_start_index) {
        if generated_contact_kind_from_constraint(constraint.kind).is_none() {
            continue;
        }
        let source_band_index = constraint.source_band_index;
        if constraint.owner.is_none() || constraint.opposite_owner.is_none() {
            continue;
        }
        let owners = [constraint.owner, constraint.opposite_owner];
        if !source_authority.has_any_source(
            owners,
            constraint.source_mouth_order_index,
            source_band_index,
        ) {
            continue;
        }
        for point in &constraint.points_xz {
            let key = road_point_key(*point);
            if generated_contact_constraint_endpoint_has_exact_source_authority(
                constraint,
                &source_authority,
                owners,
                source_band_index,
                key,
            ) {
                continue;
            }
            return Err(
                NodeRailGenerationError::NonCanonicalGeneratedContactEndpoint {
                    kind: constraint.kind,
                    mouth_order_index: constraint.source_mouth_order_index,
                    band_index: constraint.source_band_index,
                    owner: constraint.owner,
                    opposite_owner: constraint.opposite_owner,
                    point_x_key: key.0,
                    point_z_key: key.1,
                },
            );
        }
    }
    Ok(())
}

struct ExactGeneratedSourceAuthority {
    keys_by_owner: BTreeMap<NodeBandOwner, BTreeSet<NodeRailPointKey>>,
    segments_by_owner: BTreeMap<NodeBandOwner, BTreeSet<GeneratedContourEdgeKey>>,
    keys_by_source: BTreeMap<(NodeBandOwner, usize, usize), BTreeSet<NodeRailPointKey>>,
    segments_by_contact_source: BTreeMap<
        (
            NodeRailConstraintKind,
            NodeBandOwner,
            NodeBandOwner,
            usize,
            Option<usize>,
        ),
        BTreeSet<GeneratedContourEdgeKey>,
    >,
}

impl ExactGeneratedSourceAuthority {
    fn from_sources(
        contours: &[NodeGeneratedContour],
        constraints: &[NodeRailConstraint],
        generated_constraint_start_index: usize,
    ) -> Self {
        let mut keys_by_owner = BTreeMap::<NodeBandOwner, BTreeSet<NodeRailPointKey>>::new();
        let mut segments_by_owner =
            BTreeMap::<NodeBandOwner, BTreeSet<GeneratedContourEdgeKey>>::new();
        let mut keys_by_source =
            BTreeMap::<(NodeBandOwner, usize, usize), BTreeSet<NodeRailPointKey>>::new();
        let mut segments_by_contact_source = BTreeMap::<
            (
                NodeRailConstraintKind,
                NodeBandOwner,
                NodeBandOwner,
                usize,
                Option<usize>,
            ),
            BTreeSet<GeneratedContourEdgeKey>,
        >::new();
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

    fn has_any_source(
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

    fn has_exact_point(
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

    fn has_exact_source_key(
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

    fn has_exact_same_kind_source_handoff_key(
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

    fn has_exact_cross_source_same_kind_contact_key(
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
