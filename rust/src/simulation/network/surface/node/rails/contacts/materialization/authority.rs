//! Contact authority lookup for generated rail contact materialization.

use super::*;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct GeneratedMaterialPointContactAuthority {
    pub(super) source_mouth_order_index: usize,
    pub(super) source_band_index: Option<usize>,
    pub(super) owner: Option<NodeBandOwner>,
    pub(super) opposite_owner: Option<NodeBandOwner>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct GeneratedContactOwnerPair {
    lower: NodeBandOwner,
    upper: NodeBandOwner,
}

pub(in crate::simulation::network::surface::node::rails::contacts) struct GeneratedContactAuthorityIndex<
    'a,
> {
    constraints_by_kind_owner_pair:
        BTreeMap<(NodeRailConstraintKind, GeneratedContactOwnerPair), Vec<&'a NodeRailConstraint>>,
}

impl GeneratedContactOwnerPair {
    fn new(a: NodeBandOwner, b: NodeBandOwner) -> Self {
        if a <= b {
            Self { lower: a, upper: b }
        } else {
            Self { lower: b, upper: a }
        }
    }
}

impl<'a> GeneratedContactAuthorityIndex<'a> {
    pub(in crate::simulation::network::surface::node::rails::contacts) fn new(
        constraints: &'a [NodeRailConstraint],
    ) -> Self {
        let mut constraints_by_kind_owner_pair = BTreeMap::<
            (NodeRailConstraintKind, GeneratedContactOwnerPair),
            Vec<&'a NodeRailConstraint>,
        >::new();
        for constraint in constraints {
            let (Some(owner), Some(opposite_owner)) = (constraint.owner, constraint.opposite_owner)
            else {
                continue;
            };
            constraints_by_kind_owner_pair
                .entry((
                    constraint.kind,
                    GeneratedContactOwnerPair::new(owner, opposite_owner),
                ))
                .or_default()
                .push(constraint);
        }
        Self {
            constraints_by_kind_owner_pair,
        }
    }

    fn constraints_for(
        &self,
        kind: NodeRailConstraintKind,
        owner: NodeBandOwner,
        opposite_owner: NodeBandOwner,
    ) -> &[&'a NodeRailConstraint] {
        self.constraints_by_kind_owner_pair
            .get(&(kind, GeneratedContactOwnerPair::new(owner, opposite_owner)))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

pub(super) fn generated_material_authority_points_on_counterpart_contour(
    kind: NodeRailConstraintKind,
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    left_owner: NodeBandOwner,
    right_owner: NodeBandOwner,
    authority_index: &GeneratedContactAuthorityIndex<'_>,
) -> Vec<NodeRailPointKey> {
    let mut points = Vec::new();
    for constraint in authority_index.constraints_for(kind, left_owner, right_owner) {
        for point in constraint.points_xz.iter().copied().map(road_point_key) {
            if generated_contour_contains_key(right, point) {
                points.push(point);
            }
            if generated_contour_contains_key(left, point) {
                points.push(point);
            }
        }
        points.extend(generated_constraint_contour_contact_points(
            constraint, right,
        ));
        points.extend(generated_constraint_contour_contact_points(
            constraint, left,
        ));
    }
    points.sort_unstable();
    points.dedup();
    points
}

fn generated_constraint_contour_contact_points(
    constraint: &NodeRailConstraint,
    contour: &NodeGeneratedContour,
) -> Vec<NodeRailPointKey> {
    let mut points = Vec::new();
    for constraint_edge in generated_constraint_directed_edges(constraint) {
        for contour_edge in generated_contour_directed_edges(contour) {
            if let Some(point) = quantized_proper_segment_intersection(
                constraint_edge.start,
                constraint_edge.end,
                contour_edge.start,
                contour_edge.end,
            ) {
                points.push(point);
            }
            if generated_point_key_lies_on_segment(
                constraint_edge.start,
                contour_edge.start,
                contour_edge.end,
            ) {
                points.push(constraint_edge.start);
            }
            if generated_point_key_lies_on_segment(
                constraint_edge.end,
                contour_edge.start,
                contour_edge.end,
            ) {
                points.push(constraint_edge.end);
            }
            if generated_point_key_lies_on_segment(
                contour_edge.start,
                constraint_edge.start,
                constraint_edge.end,
            ) {
                points.push(contour_edge.start);
            }
            if generated_point_key_lies_on_segment(
                contour_edge.end,
                constraint_edge.start,
                constraint_edge.end,
            ) {
                points.push(contour_edge.end);
            }
        }
    }
    points.sort_unstable();
    points.dedup();
    points
}

pub(super) fn generated_material_point_contact_authority(
    kind: NodeRailConstraintKind,
    left_owner: NodeBandOwner,
    right_owner: NodeBandOwner,
    point: NodeRailPointKey,
    authority_index: &GeneratedContactAuthorityIndex<'_>,
) -> Option<GeneratedMaterialPointContactAuthority> {
    authority_index
        .constraints_for(kind, left_owner, right_owner)
        .iter()
        .copied()
        .filter(|constraint| generated_constraint_touches_key(constraint, point))
        .min_by_key(|constraint| constraint.constraint_index)
        .map(|constraint| GeneratedMaterialPointContactAuthority {
            source_mouth_order_index: constraint.source_mouth_order_index,
            source_band_index: constraint.source_band_index,
            owner: constraint.owner,
            opposite_owner: constraint.opposite_owner,
        })
}

fn generated_exact_owner_pair_contact_authority_for_edge(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    authority_index: &GeneratedContactAuthorityIndex<'_>,
    edge: GeneratedContourEdgeKey,
) -> Option<GeneratedMaterialPointContactAuthority> {
    authority_index
        .constraints_for(
            NodeRailConstraintKind::RaisedStepContact,
            owner,
            opposite_owner,
        )
        .iter()
        .copied()
        .filter(|constraint| {
            generated_constraint_contains_key_segment(constraint, edge.start, edge.end)
        })
        .min_by_key(|constraint| constraint.constraint_index)
        .map(|constraint| GeneratedMaterialPointContactAuthority {
            source_mouth_order_index: constraint.source_mouth_order_index,
            source_band_index: constraint.source_band_index,
            owner: constraint.owner,
            opposite_owner: constraint.opposite_owner,
        })
}

pub(super) fn generated_exact_owner_pair_contact_authority_at_point(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    authority_index: &GeneratedContactAuthorityIndex<'_>,
    point: NodeRailPointKey,
) -> Option<GeneratedMaterialPointContactAuthority> {
    generated_material_point_contact_authority(
        NodeRailConstraintKind::RaisedStepContact,
        owner,
        opposite_owner,
        point,
        authority_index,
    )
}

pub(super) fn generated_contact_edge_source_authority(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    authority_index: &GeneratedContactAuthorityIndex<'_>,
    edge: GeneratedContourEdgeKey,
) -> Option<GeneratedMaterialPointContactAuthority> {
    generated_exact_owner_pair_contact_authority_for_edge(
        owner,
        opposite_owner,
        authority_index,
        edge,
    )
}

fn generated_same_band_point_contact_has_explicit_roles(
    kind: RoadSurfaceBandKind,
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    point: NodeRailPointKey,
) -> bool {
    generated_contour_supports_same_band_role(kind)
        && generated_same_band_boundary_role_at_contour_vertex(left, constraints, point).is_some()
        && generated_same_band_boundary_role_at_contour_vertex(right, constraints, point).is_some()
}

pub(in crate::simulation::network::surface::node::rails::contacts) fn generated_contact_point_has_explicit_roles(
    left_kind: RoadSurfaceBandKind,
    right_kind: RoadSurfaceBandKind,
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    authority_index: &GeneratedContactAuthorityIndex<'_>,
    point: NodeRailPointKey,
    contact_kind: NodeRailConstraintKind,
) -> bool {
    if left_kind == right_kind {
        return generated_same_band_point_contact_has_explicit_roles(
            left_kind,
            left,
            right,
            constraints,
            point,
        );
    }
    match contact_kind {
        NodeRailConstraintKind::RaisedStepContact => {
            let Some(left_owner) = left.owner else {
                return false;
            };
            let Some(right_owner) = right.owner else {
                return false;
            };
            let Some(pair) = GeneratedRaisedStepOwnerPair::new(left_owner, right_owner) else {
                return false;
            };
            generated_exact_owner_pair_contact_authority_at_point(
                pair.owner,
                pair.opposite_owner,
                authority_index,
                point,
            )
            .is_some()
        }
        _ => true,
    }
}
