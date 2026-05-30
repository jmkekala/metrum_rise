//! Contact authority lookup for generated rail contact materialization.

use super::*;
use std::collections::BTreeMap;

const GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS: i64 = 4096;

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

#[derive(Clone, Copy)]
struct GeneratedContactAuthorityConstraint<'a> {
    constraint: &'a NodeRailConstraint,
    min_x: i64,
    min_z: i64,
    max_x: i64,
    max_z: i64,
}

pub(in crate::simulation::network::surface::node::rails::contacts) struct GeneratedContactAuthorityIndex<
    'a,
> {
    constraints_by_kind_owner_pair: BTreeMap<
        (NodeRailConstraintKind, GeneratedContactOwnerPair),
        Vec<GeneratedContactAuthorityConstraint<'a>>,
    >,
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
            Vec<GeneratedContactAuthorityConstraint<'a>>,
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
                .push(GeneratedContactAuthorityConstraint::new(constraint));
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
    ) -> &[GeneratedContactAuthorityConstraint<'a>] {
        self.constraints_by_kind_owner_pair
            .get(&(kind, GeneratedContactOwnerPair::new(owner, opposite_owner)))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub(in crate::simulation::network::surface::node::rails::contacts::materialization) fn has_constraints_touching_contour_pair(
        &self,
        kind: NodeRailConstraintKind,
        owner: NodeBandOwner,
        opposite_owner: NodeBandOwner,
        left: &GeneratedContactContourSummary,
        right: &GeneratedContactContourSummary,
    ) -> bool {
        self.constraints_for(kind, owner, opposite_owner)
            .iter()
            .any(|authority_constraint| {
                authority_constraint.bounds_touch_summary(left)
                    && authority_constraint.bounds_touch_summary(right)
            })
    }
}

impl<'a> GeneratedContactAuthorityConstraint<'a> {
    fn new(constraint: &'a NodeRailConstraint) -> Self {
        let (mut min_x, mut min_z) = (i64::MAX, i64::MAX);
        let (mut max_x, mut max_z) = (i64::MIN, i64::MIN);
        for point in constraint.points_xz.iter().copied().map(road_point_key) {
            min_x = min_x.min(point.0);
            min_z = min_z.min(point.1);
            max_x = max_x.max(point.0);
            max_z = max_z.max(point.1);
        }
        if constraint.points_xz.is_empty() {
            min_x = 1;
            min_z = 1;
            max_x = 0;
            max_z = 0;
        }
        Self {
            constraint,
            min_x,
            min_z,
            max_x,
            max_z,
        }
    }

    fn bounds_touch_summary(&self, summary: &GeneratedContactContourSummary) -> bool {
        if self.min_x > self.max_x
            || self.min_z > self.max_z
            || summary.min_x > summary.max_x
            || summary.min_z > summary.max_z
        {
            return false;
        }
        self.min_x - GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS <= summary.max_x
            && summary.min_x <= self.max_x + GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS
            && self.min_z - GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS <= summary.max_z
            && summary.min_z <= self.max_z + GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS
    }

    fn bounds_touch_edge(&self, edge: GeneratedContourEdgeKey) -> bool {
        let min_x = edge.start.0.min(edge.end.0);
        let min_z = edge.start.1.min(edge.end.1);
        let max_x = edge.start.0.max(edge.end.0);
        let max_z = edge.start.1.max(edge.end.1);
        self.min_x - GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS <= max_x
            && min_x <= self.max_x + GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS
            && self.min_z - GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS <= max_z
            && min_z <= self.max_z + GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS
    }

    fn bounds_touch_point(&self, point: NodeRailPointKey) -> bool {
        self.min_x - GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS <= point.0
            && point.0 <= self.max_x + GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS
            && self.min_z - GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS <= point.1
            && point.1 <= self.max_z + GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS
    }
}

pub(super) fn generated_contact_authority_source_edges_touching_contour_pair(
    kind: NodeRailConstraintKind,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    left_summary: &GeneratedContactContourSummary,
    right_summary: &GeneratedContactContourSummary,
    authority_index: &GeneratedContactAuthorityIndex<'_>,
) -> Vec<GeneratedContourDirectedEdge> {
    let mut edges = authority_index
        .constraints_for(kind, owner, opposite_owner)
        .iter()
        .filter(|authority_constraint| {
            authority_constraint.bounds_touch_summary(left_summary)
                && authority_constraint.bounds_touch_summary(right_summary)
        })
        .flat_map(|authority_constraint| {
            generated_constraint_directed_edges(authority_constraint.constraint)
                .into_iter()
                .filter(|edge| {
                    generated_edge_bounds_touch_summary(*edge, left_summary)
                        && generated_edge_bounds_touch_summary(*edge, right_summary)
                })
        })
        .collect::<Vec<_>>();
    edges.sort_unstable();
    edges.dedup();
    edges
}

fn generated_edge_bounds_touch_summary(
    edge: GeneratedContourDirectedEdge,
    summary: &GeneratedContactContourSummary,
) -> bool {
    let min_x = edge.start.0.min(edge.end.0);
    let min_z = edge.start.1.min(edge.end.1);
    let max_x = edge.start.0.max(edge.end.0);
    let max_z = edge.start.1.max(edge.end.1);
    min_x - GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS <= summary.max_x
        && summary.min_x <= max_x + GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS
        && min_z - GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS <= summary.max_z
        && summary.min_z <= max_z + GENERATED_CONTACT_AUTHORITY_BOUNDS_MARGIN_KEYS
}

pub(super) fn generated_material_authority_points_on_counterpart_contour(
    kind: NodeRailConstraintKind,
    left: &NodeGeneratedContour,
    left_summary: &GeneratedContactContourSummary,
    right: &NodeGeneratedContour,
    right_summary: &GeneratedContactContourSummary,
    left_owner: NodeBandOwner,
    right_owner: NodeBandOwner,
    authority_index: &GeneratedContactAuthorityIndex<'_>,
) -> Vec<NodeRailPointKey> {
    let mut points = Vec::new();
    for authority_constraint in authority_index.constraints_for(kind, left_owner, right_owner) {
        if !authority_constraint.bounds_touch_summary(left_summary)
            && !authority_constraint.bounds_touch_summary(right_summary)
        {
            continue;
        }
        let constraint = authority_constraint.constraint;
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
        .filter(|authority_constraint| authority_constraint.bounds_touch_point(point))
        .map(|authority_constraint| authority_constraint.constraint)
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
        .filter(|authority_constraint| authority_constraint.bounds_touch_edge(edge))
        .map(|authority_constraint| authority_constraint.constraint)
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
