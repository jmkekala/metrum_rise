//! Explicit source-authority support for generated rail contacts.

use super::geometry::{
    generated_directed_edge_segments_inside_shape_edges, generated_overlay_contour,
    generated_overlay_shapes_directed_edges, generated_shape_boundary_segments_on_source_edge,
};
use super::{
    GeneratedContourDirectedEdge, GeneratedContourEdgeKey, GeneratedRaisedStepOwnerPair,
    GeneratedSameBandBoundaryRole, NodeBandOwner, NodeGeneratedContour,
    NodeGeneratedContourClaimPriority, NodeOverlayShapes, NodeRailConstraint,
    NodeRailConstraintKind, NodeRailPointKey, RoadSurfaceBandKind, RoadSurfaceSystem,
    RoadSurfaceVisualNodePieceKind, generated_constraint_contains_key_segment,
    generated_constraint_directed_edges, generated_constraint_touches_key,
    generated_contour_band_kind, generated_contour_directed_edges,
    generated_point_key_lies_on_segment, quantized_proper_segment_intersection,
    raised_step_band_rank, raised_step_kinds_can_contact, road_point_key,
};
use i_overlay::core::overlay_rule::OverlayRule;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct GeneratedSameBandContactConstraint {
    pub(super) kind: NodeRailConstraintKind,
    pub(super) owner: NodeBandOwner,
    pub(super) opposite_owner: NodeBandOwner,
    pub(super) start: NodeRailPointKey,
    pub(super) end: NodeRailPointKey,
    pub(super) source_mouth_order_index: usize,
    pub(super) source_band_index: Option<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct GeneratedSameBandContactConstraintKey {
    kind: NodeRailConstraintKind,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    start: NodeRailPointKey,
    end: NodeRailPointKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct GeneratedRaisedStepEndpointSource {
    constraint_index: usize,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
    owners: [NodeBandOwner; 2],
}

#[derive(Clone, Copy)]
struct RaisedStepSourceConstraint<'a> {
    source: GeneratedRaisedStepEndpointSource,
    constraint: &'a NodeRailConstraint,
}

struct RaisedStepSourceAuthority<'a> {
    constraints: Vec<RaisedStepSourceConstraint<'a>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct SourceAuthorizedTargetGroupKey {
    owner: NodeBandOwner,
    kind: RoadSurfaceBandKind,
    claim_priority: NodeGeneratedContourClaimPriority,
}

#[derive(Clone, Debug)]
struct SourceAuthorizedTargetGroup {
    key: SourceAuthorizedTargetGroupKey,
    contour_indices: Vec<usize>,
    shapes: NodeOverlayShapes,
    shape_edges: Vec<GeneratedContourDirectedEdge>,
}

impl GeneratedSameBandContactConstraint {
    pub(super) fn key(self) -> GeneratedSameBandContactConstraintKey {
        let edge = GeneratedContourEdgeKey::new(self.start, self.end);
        GeneratedSameBandContactConstraintKey {
            kind: self.kind,
            owner: self.owner,
            opposite_owner: self.opposite_owner,
            start: edge.start,
            end: edge.end,
        }
    }
}

pub(super) fn collect_source_authorized_raised_step_contacts(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    contours: &[NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
    contacts: &mut BTreeSet<GeneratedSameBandContactConstraint>,
) {
    let source_authority = RaisedStepSourceAuthority::from_constraints(constraints);
    let target_groups = source_authorized_target_groups(contours);
    for source_constraint in source_authority.constraints() {
        for target_group in &target_groups {
            let target_contacts = source_authorized_raised_step_target_pairs(
                piece_kind,
                contours,
                source_constraint.source,
                target_group.key,
            );
            if target_contacts.is_empty() {
                continue;
            }
            for source_edge in generated_constraint_directed_edges(source_constraint.constraint) {
                let mut source_edges = generated_directed_edge_segments_inside_shape_edges(
                    source_edge,
                    &target_group.shape_edges,
                    &target_group.shapes,
                )
                .into_iter()
                .collect::<BTreeSet<_>>();
                source_edges.extend(generated_shape_boundary_segments_on_source_edge(
                    source_edge,
                    &target_group.shape_edges,
                ));
                for edge in source_edges {
                    for (owner, opposite_owner, include_edge) in &target_contacts {
                        for (start, end) in source_authorized_contact_segments(edge, *include_edge)
                        {
                            contacts.insert(GeneratedSameBandContactConstraint {
                                kind: NodeRailConstraintKind::RaisedStepContact,
                                owner: *owner,
                                opposite_owner: *opposite_owner,
                                start,
                                end,
                                source_mouth_order_index: source_constraint
                                    .source
                                    .source_mouth_order_index,
                                source_band_index: source_constraint.source.source_band_index,
                            });
                        }
                    }
                }
            }
        }
        collect_source_authorized_exact_group_overlap_contacts(
            source_constraint,
            contours,
            &target_groups,
            contacts,
        );
    }

    for (point, sources) in source_authority.sources_by_contact_point() {
        for left_index in 0..sources.len() {
            for right_index in left_index + 1..sources.len() {
                let source = sources[left_index].min(sources[right_index]);
                for left_owner in sources[left_index].owners {
                    for right_owner in sources[right_index].owners {
                        let Some(kind) =
                            generated_raised_step_contact_kind_for_owners(left_owner, right_owner)
                        else {
                            continue;
                        };
                        let Some(pair) = GeneratedRaisedStepOwnerPair::new(left_owner, right_owner)
                        else {
                            continue;
                        };
                        contacts.insert(GeneratedSameBandContactConstraint {
                            kind,
                            owner: pair.owner,
                            opposite_owner: pair.opposite_owner,
                            start: point,
                            end: point,
                            source_mouth_order_index: source.source_mouth_order_index,
                            source_band_index: source.source_band_index,
                        });
                    }
                }
            }
        }
    }
}

impl<'a> RaisedStepSourceAuthority<'a> {
    fn from_constraints(constraints: &'a [NodeRailConstraint]) -> Self {
        Self {
            constraints: constraints
                .iter()
                .filter_map(|constraint| {
                    generated_raised_step_endpoint_source(constraint)
                        .map(|source| RaisedStepSourceConstraint { source, constraint })
                })
                .collect(),
        }
    }

    fn constraints(&self) -> &[RaisedStepSourceConstraint<'a>] {
        &self.constraints
    }

    fn sources_by_contact_point(
        &self,
    ) -> BTreeMap<NodeRailPointKey, Vec<GeneratedRaisedStepEndpointSource>> {
        generated_raised_step_source_contact_points(&self.constraints)
            .into_iter()
            .filter_map(|point| {
                let mut sources = self
                    .constraints
                    .iter()
                    .filter(|source_constraint| {
                        generated_constraint_touches_key(source_constraint.constraint, point)
                    })
                    .map(|source_constraint| source_constraint.source)
                    .collect::<Vec<_>>();
                sources.sort_unstable();
                sources.dedup();
                (!sources.is_empty()).then_some((point, sources))
            })
            .collect()
    }
}

fn collect_source_authorized_exact_group_overlap_contacts(
    source_constraint: &RaisedStepSourceConstraint<'_>,
    contours: &[NodeGeneratedContour],
    target_groups: &[SourceAuthorizedTargetGroup],
    contacts: &mut BTreeSet<GeneratedSameBandContactConstraint>,
) {
    let [left_owner, right_owner] = source_constraint.source.owners;
    let left_groups = source_authorized_exact_target_groups(target_groups, left_owner);
    let right_groups = source_authorized_exact_target_groups(target_groups, right_owner);
    for left_group in &left_groups {
        for right_group in &right_groups {
            for edge in source_authorized_group_edges_inside_group(
                source_constraint.constraint,
                left_group,
                right_group,
                contours,
            )
            .into_iter()
            .chain(source_authorized_group_edges_inside_group(
                source_constraint.constraint,
                right_group,
                left_group,
                contours,
            ))
            .chain(source_authorized_source_edges_inside_group_intersection(
                source_constraint.constraint,
                left_group,
                right_group,
            )) {
                for (start, end) in source_authorized_contact_segments(edge, true) {
                    contacts.insert(GeneratedSameBandContactConstraint {
                        kind: NodeRailConstraintKind::RaisedStepContact,
                        owner: left_owner,
                        opposite_owner: right_owner,
                        start,
                        end,
                        source_mouth_order_index: source_constraint.source.source_mouth_order_index,
                        source_band_index: source_constraint.source.source_band_index,
                    });
                }
            }
        }
    }
}

fn source_authorized_exact_target_groups(
    target_groups: &[SourceAuthorizedTargetGroup],
    owner: NodeBandOwner,
) -> Vec<&SourceAuthorizedTargetGroup> {
    target_groups
        .iter()
        .filter(|group| group.key.owner == owner)
        .collect()
}

fn source_authorized_group_edges_inside_group(
    source_constraint: &NodeRailConstraint,
    edge_group: &SourceAuthorizedTargetGroup,
    containing_group: &SourceAuthorizedTargetGroup,
    contours: &[NodeGeneratedContour],
) -> Vec<GeneratedContourEdgeKey> {
    let mut edges = BTreeSet::new();
    for contour_index in &edge_group.contour_indices {
        let Some(contour) = contours.get(*contour_index) else {
            continue;
        };
        for contour_edge in generated_contour_directed_edges(contour) {
            let mut candidate_edges = generated_directed_edge_segments_inside_shape_edges(
                contour_edge,
                &containing_group.shape_edges,
                &containing_group.shapes,
            )
            .into_iter()
            .collect::<BTreeSet<_>>();
            candidate_edges.extend(generated_shape_boundary_segments_on_source_edge(
                contour_edge,
                &containing_group.shape_edges,
            ));
            for edge in candidate_edges {
                if generated_constraint_contains_key_segment(
                    source_constraint,
                    edge.start,
                    edge.end,
                ) {
                    edges.insert(edge);
                }
            }
        }
    }
    edges.into_iter().collect()
}

fn source_authorized_source_edges_inside_group_intersection(
    source_constraint: &NodeRailConstraint,
    left_group: &SourceAuthorizedTargetGroup,
    right_group: &SourceAuthorizedTargetGroup,
) -> Vec<GeneratedContourEdgeKey> {
    let Some(intersection_shapes) = RoadSurfaceSystem::overlay_binary_shapes(
        &left_group.shapes,
        &right_group.shapes,
        OverlayRule::Intersect,
    ) else {
        return Vec::new();
    };
    let intersection_edges = generated_overlay_shapes_directed_edges(&intersection_shapes);
    let mut edges = BTreeSet::new();
    for source_edge in generated_constraint_directed_edges(source_constraint) {
        edges.extend(generated_directed_edge_segments_inside_shape_edges(
            source_edge,
            &intersection_edges,
            &intersection_shapes,
        ));
        edges.extend(generated_shape_boundary_segments_on_source_edge(
            source_edge,
            &intersection_edges,
        ));
    }
    edges.into_iter().collect()
}

fn source_authorized_raised_step_target_pairs(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    contours: &[NodeGeneratedContour],
    source: GeneratedRaisedStepEndpointSource,
    target: SourceAuthorizedTargetGroupKey,
) -> Vec<(NodeBandOwner, NodeBandOwner, bool)> {
    let target_owner = target.owner;
    if source.owners.contains(&target_owner) {
        if Some(target.claim_priority)
            == source_authorized_target_claim_priority(contours, target_owner)
        {
            let Some(pair) = GeneratedRaisedStepOwnerPair::new(source.owners[0], source.owners[1])
            else {
                return Vec::new();
            };
            return vec![(pair.owner, pair.opposite_owner, true)];
        }
        return Vec::new();
    }

    if piece_kind != RoadSurfaceVisualNodePieceKind::Bend
        || target.claim_priority != NodeGeneratedContourClaimPriority::SideJoin
    {
        return Vec::new();
    }

    let mut pairs = Vec::new();
    for source_owner_index in 0..source.owners.len() {
        let source_owner = source.owners[source_owner_index];
        let replaced_owner = source.owners[1 - source_owner_index];
        if target_owner.kind() != replaced_owner.kind()
            || generated_raised_step_contact_kind_for_owners(source_owner, target_owner).is_none()
        {
            continue;
        }
        let Some(pair) = GeneratedRaisedStepOwnerPair::new(source_owner, target_owner) else {
            continue;
        };
        pairs.push((pair.owner, pair.opposite_owner, false));
    }
    pairs.sort_unstable();
    pairs.dedup();
    pairs
}

fn source_authorized_contact_segments(
    edge: GeneratedContourEdgeKey,
    include_edge: bool,
) -> Vec<(NodeRailPointKey, NodeRailPointKey)> {
    if include_edge {
        vec![(edge.start, edge.end)]
    } else {
        vec![(edge.start, edge.start), (edge.end, edge.end)]
    }
}

fn source_authorized_target_claim_priority(
    contours: &[NodeGeneratedContour],
    owner: NodeBandOwner,
) -> Option<NodeGeneratedContourClaimPriority> {
    if contours.iter().any(|contour| {
        contour.owner == Some(owner)
            && contour.claim_priority == NodeGeneratedContourClaimPriority::MouthBand
    }) {
        return Some(NodeGeneratedContourClaimPriority::MouthBand);
    }
    contours
        .iter()
        .filter(|contour| contour.owner == Some(owner))
        .map(|contour| contour.claim_priority)
        .min()
}

fn generated_raised_step_source_contact_points(
    source_constraints: &[RaisedStepSourceConstraint<'_>],
) -> BTreeSet<NodeRailPointKey> {
    let mut points = source_constraints
        .iter()
        .flat_map(|source_constraint| {
            generated_constraint_endpoint_keys(source_constraint.constraint)
        })
        .collect::<BTreeSet<_>>();
    for left_index in 0..source_constraints.len() {
        for right_index in left_index + 1..source_constraints.len() {
            let left = source_constraints[left_index].constraint;
            let right = source_constraints[right_index].constraint;
            for left_edge in generated_constraint_directed_edges(left) {
                for right_edge in generated_constraint_directed_edges(right) {
                    points.extend(generated_source_edge_contact_points(left_edge, right_edge));
                }
            }
        }
    }
    points
}

fn generated_source_edge_contact_points(
    left: GeneratedContourDirectedEdge,
    right: GeneratedContourDirectedEdge,
) -> Vec<NodeRailPointKey> {
    let mut points = Vec::new();
    if let Some(point) =
        quantized_proper_segment_intersection(left.start, left.end, right.start, right.end)
    {
        points.push(point);
    }
    for point in [left.start, left.end] {
        if generated_point_key_lies_on_segment(point, right.start, right.end) {
            points.push(point);
        }
    }
    for point in [right.start, right.end] {
        if generated_point_key_lies_on_segment(point, left.start, left.end) {
            points.push(point);
        }
    }
    points.sort_unstable();
    points.dedup();
    points
}

fn generated_raised_step_endpoint_source(
    constraint: &NodeRailConstraint,
) -> Option<GeneratedRaisedStepEndpointSource> {
    if constraint.kind != NodeRailConstraintKind::RaisedStepContact {
        return None;
    }
    let owner = constraint.owner?;
    let opposite_owner = constraint.opposite_owner?;
    let pair = GeneratedRaisedStepOwnerPair::new(owner, opposite_owner)?;
    Some(GeneratedRaisedStepEndpointSource {
        constraint_index: constraint.constraint_index,
        source_mouth_order_index: constraint.source_mouth_order_index,
        source_band_index: constraint.source_band_index,
        owners: [pair.owner, pair.opposite_owner],
    })
}

fn generated_constraint_endpoint_keys(constraint: &NodeRailConstraint) -> Vec<NodeRailPointKey> {
    let mut points = Vec::new();
    if let Some(point) = constraint.points_xz.first().copied() {
        points.push(road_point_key(point));
    }
    if let Some(point) = constraint.points_xz.last().copied() {
        points.push(road_point_key(point));
    }
    points.sort_unstable();
    points.dedup();
    points
}

fn source_authorized_target_groups(
    contours: &[NodeGeneratedContour],
) -> Vec<SourceAuthorizedTargetGroup> {
    let mut contour_indices_by_key = BTreeMap::<SourceAuthorizedTargetGroupKey, Vec<usize>>::new();
    for (contour_index, contour) in contours.iter().enumerate() {
        let Some(owner) = contour.owner else {
            continue;
        };
        let Some(kind) = generated_contour_band_kind(contour) else {
            continue;
        };
        contour_indices_by_key
            .entry(SourceAuthorizedTargetGroupKey {
                owner,
                kind,
                claim_priority: contour.claim_priority,
            })
            .or_default()
            .push(contour_index);
    }

    contour_indices_by_key
        .into_iter()
        .filter_map(|(key, contour_indices)| {
            let overlay_contours = contour_indices
                .iter()
                .map(|index| generated_overlay_contour(&contours[*index]))
                .collect::<Vec<_>>();
            let shapes = RoadSurfaceSystem::overlay_union_contours(&overlay_contours)?;
            Some(SourceAuthorizedTargetGroup {
                key,
                contour_indices,
                shape_edges: generated_overlay_shapes_directed_edges(&shapes),
                shapes,
            })
        })
        .collect()
}

pub(super) fn generated_raised_step_contact_kind_for_owners(
    left_owner: NodeBandOwner,
    right_owner: NodeBandOwner,
) -> Option<NodeRailConstraintKind> {
    GeneratedRaisedStepOwnerPair::new(left_owner, right_owner)
        .map(|_| NodeRailConstraintKind::RaisedStepContact)
}

pub(in crate::simulation::network::surface::node::rails) fn raised_step_band_kinds_can_contact(
    left_kind: RoadSurfaceBandKind,
    right_kind: RoadSurfaceBandKind,
) -> bool {
    raised_step_kinds_can_contact(left_kind, right_kind)
}

pub(in crate::simulation::network::surface::node::rails) fn generated_raised_step_boundary_role_for_owner(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> Option<GeneratedSameBandBoundaryRole> {
    GeneratedRaisedStepOwnerPair::new(owner, opposite_owner)?;
    let owner_rank = raised_step_band_rank(owner.kind())?;
    let opposite_rank = raised_step_band_rank(opposite_owner.kind())?;
    if opposite_rank < owner_rank {
        Some(GeneratedSameBandBoundaryRole::LowerSide)
    } else if opposite_rank > owner_rank {
        Some(GeneratedSameBandBoundaryRole::RaisedSide)
    } else {
        None
    }
}

pub(super) fn generated_same_band_contact_constraint_key(
    constraint: &NodeRailConstraint,
) -> Option<GeneratedSameBandContactConstraintKey> {
    generated_same_band_contact_constraint(constraint).map(GeneratedSameBandContactConstraint::key)
}

pub(super) fn generated_same_band_contact_constraint(
    constraint: &NodeRailConstraint,
) -> Option<GeneratedSameBandContactConstraint> {
    let Some(kind) = generated_contact_kind_from_constraint(constraint.kind) else {
        return None;
    };
    let owner = constraint.owner?;
    let opposite_owner = constraint.opposite_owner?;
    if owner == opposite_owner {
        return None;
    }
    let points = constraint.points_xz.as_slice();
    if points.len() != 2 {
        return None;
    }
    let (owner, opposite_owner) = if kind == NodeRailConstraintKind::RaisedStepContact {
        let pair = GeneratedRaisedStepOwnerPair::new(owner, opposite_owner)?;
        (pair.owner, pair.opposite_owner)
    } else {
        (owner.min(opposite_owner), owner.max(opposite_owner))
    };
    Some(GeneratedSameBandContactConstraint {
        kind,
        owner,
        opposite_owner,
        start: road_point_key(points[0]),
        end: road_point_key(points[1]),
        source_mouth_order_index: constraint.source_mouth_order_index,
        source_band_index: constraint.source_band_index,
    })
}

pub(super) fn generated_contact_kind_from_constraint(
    kind: NodeRailConstraintKind,
) -> Option<NodeRailConstraintKind> {
    match kind {
        NodeRailConstraintKind::AsphaltBoundary { .. }
        | NodeRailConstraintKind::RaisedStepContact => Some(kind),
        NodeRailConstraintKind::BandBoundary {
            left_kind,
            right_kind,
        } => raised_step_band_kinds_can_contact(left_kind, right_kind).then_some(kind),
        NodeRailConstraintKind::FullRoadbedContour
        | NodeRailConstraintKind::BandContour { .. }
        | NodeRailConstraintKind::SpanHandoff { .. }
        | NodeRailConstraintKind::FootprintSeam { .. } => None,
    }
}
