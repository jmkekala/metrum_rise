//! Source-authorized raised-step contact collection.

use super::super::geometry::{
    generated_contour_boundary_contains_key, generated_directed_edge_segments_inside_shape_edges,
    generated_shape_boundary_segments_on_source_edge,
};
use super::super::{
    GeneratedContourDirectedEdge, GeneratedRaisedStepOwnerPair, NodeGeneratedContour,
    NodeGeneratedContourClaimPriority, NodeRailConstraint, NodeRailConstraintKind,
    NodeRailPointKey, RoadSurfaceVisualNodePieceKind, generated_constraint_directed_edges,
    generated_constraint_touches_key, generated_point_key_lies_on_segment,
    quantized_proper_segment_intersection, road_point_key,
};
use super::target_groups::{
    collect_source_authorized_exact_group_overlap_contacts, source_authorized_contact_segments,
    source_authorized_raised_step_target_pairs, source_authorized_target_claim_priority,
    source_authorized_target_groups,
};
use super::types::{
    GeneratedRaisedStepEndpointSource, GeneratedSameBandContactConstraint,
    RaisedStepSourceAuthority, RaisedStepSourceConstraint,
};
use std::collections::{BTreeMap, BTreeSet};

pub(in crate::simulation::network::surface::node::rails::contacts) fn collect_source_authorized_raised_step_contacts(
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
        collect_junctionn_source_authorized_mouth_band_endpoint_handoffs(
            piece_kind,
            contours,
            source_constraint,
            &target_groups,
            contacts,
        );
    }

    for (point, sources) in source_authority.sources_by_contact_point() {
        for left_index in 0..sources.len() {
            for right_index in left_index + 1..sources.len() {
                let (source_mouth_order_index, source_band_index) =
                    deterministic_contact_source_name(sources[left_index], sources[right_index]);
                for left_owner in sources[left_index].owners {
                    for right_owner in sources[right_index].owners {
                        let Some(pair) = GeneratedRaisedStepOwnerPair::new(left_owner, right_owner)
                        else {
                            continue;
                        };
                        contacts.insert(GeneratedSameBandContactConstraint {
                            kind: NodeRailConstraintKind::RaisedStepContact,
                            owner: pair.owner,
                            opposite_owner: pair.opposite_owner,
                            start: point,
                            end: point,
                            source_mouth_order_index,
                            source_band_index,
                        });
                    }
                }
            }
        }
    }
}

fn collect_junctionn_source_authorized_mouth_band_endpoint_handoffs(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    contours: &[NodeGeneratedContour],
    source_constraint: &RaisedStepSourceConstraint<'_>,
    target_groups: &[super::types::SourceAuthorizedTargetGroup],
    contacts: &mut BTreeSet<GeneratedSameBandContactConstraint>,
) {
    if piece_kind != RoadSurfaceVisualNodePieceKind::JunctionN {
        return;
    }
    for point in generated_constraint_endpoint_keys(source_constraint.constraint) {
        for replaced_owner_index in 0..source_constraint.source.owners.len() {
            let replaced_owner = source_constraint.source.owners[replaced_owner_index];
            let retained_owner = source_constraint.source.owners[1 - replaced_owner_index];
            for target_group in target_groups {
                let target_owner = target_group.key.owner;
                if target_owner == replaced_owner
                    || source_constraint.source.owners.contains(&target_owner)
                    || target_owner.kind() != replaced_owner.kind()
                    || target_group.key.claim_priority
                        != NodeGeneratedContourClaimPriority::MouthBand
                    || Some(target_group.key.claim_priority)
                        != source_authorized_target_claim_priority(contours, target_owner)
                    || !target_group_contains_boundary_key(contours, target_group, point)
                {
                    continue;
                }
                let Some(pair) = GeneratedRaisedStepOwnerPair::new(retained_owner, target_owner)
                else {
                    continue;
                };
                contacts.insert(GeneratedSameBandContactConstraint {
                    kind: NodeRailConstraintKind::RaisedStepContact,
                    owner: pair.owner,
                    opposite_owner: pair.opposite_owner,
                    start: point,
                    end: point,
                    source_mouth_order_index: source_constraint.source.source_mouth_order_index,
                    source_band_index: source_constraint.source.source_band_index,
                });
            }
        }
    }
}

fn target_group_contains_boundary_key(
    contours: &[NodeGeneratedContour],
    target_group: &super::types::SourceAuthorizedTargetGroup,
    point: NodeRailPointKey,
) -> bool {
    target_group.contour_indices.iter().any(|contour_index| {
        contours
            .get(*contour_index)
            .is_some_and(|contour| generated_contour_boundary_contains_key(contour, point))
    })
}

fn deterministic_contact_source_name(
    left: GeneratedRaisedStepEndpointSource,
    right: GeneratedRaisedStepEndpointSource,
) -> (usize, Option<usize>) {
    // The generated contact already has exact endpoint authority from both sources.
    // NodeRailConstraint carries one source name, so use a deterministic label only.
    let source = left.min(right);
    (source.source_mouth_order_index, source.source_band_index)
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
