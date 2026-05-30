//! Source-authorized raised-step contact collection.

use super::super::geometry::{
    generated_contour_boundary_contains_key, generated_directed_edge_segments_inside_shape_edges,
    generated_shape_boundary_segments_on_source_edge,
};
use super::super::{
    GeneratedContourDirectedEdge, GeneratedRaisedStepOwnerPair, NodeBandOwner,
    NodeGeneratedContour, NodeGeneratedContourClaimPriority, NodeRailConstraint,
    NodeRailConstraintKind, NodeRailPointKey, RoadSurfaceVisualNodePieceKind,
    generated_constraint_directed_edges, generated_constraint_touches_key,
    generated_point_key_lies_on_segment, quantized_proper_segment_intersection, road_point_key,
};
use super::target_groups::{
    collect_source_authorized_exact_group_overlap_contacts, source_authorized_contact_segments,
    source_authorized_raised_step_target_pairs, source_authorized_target_claim_priorities,
    source_authorized_target_groups,
};
use super::types::{
    GeneratedRaisedStepEndpointSource, GeneratedSameBandContactConstraint,
    RaisedStepSourceAuthority, RaisedStepSourceConstraint, SourceAuthorizedTargetGroupKey,
};
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

const SOURCE_AUTHORITY_PARALLEL_SOURCE_THRESHOLD: usize = 64;

type SourceAuthorizedTargetPair = (NodeBandOwner, NodeBandOwner, bool);
type SourceAuthorizedTargetPairCache =
    BTreeMap<([NodeBandOwner; 2], SourceAuthorizedTargetGroupKey), Vec<SourceAuthorizedTargetPair>>;

pub(in crate::simulation::network::surface::node::rails::contacts) fn collect_source_authorized_raised_step_contacts(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    contours: &[NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
    contacts: &mut BTreeSet<GeneratedSameBandContactConstraint>,
) {
    let source_authority = RaisedStepSourceAuthority::from_constraints(constraints);
    let target_groups = source_authorized_target_groups(contours);
    let claim_priorities = source_authorized_target_claim_priorities(contours);
    let target_pair_cache = source_authorized_target_pair_cache(
        piece_kind,
        &claim_priorities,
        source_authority.constraints(),
        &target_groups,
    );
    let source_contacts =
        if source_authority.constraints().len() >= SOURCE_AUTHORITY_PARALLEL_SOURCE_THRESHOLD {
            source_authority
                .constraints()
                .par_iter()
                .map(|source_constraint| {
                    collect_source_authorized_contacts_for_source(
                        piece_kind,
                        contours,
                        source_constraint,
                        &target_groups,
                        &claim_priorities,
                        &target_pair_cache,
                    )
                })
                .collect::<Vec<_>>()
        } else {
            source_authority
                .constraints()
                .iter()
                .map(|source_constraint| {
                    collect_source_authorized_contacts_for_source(
                        piece_kind,
                        contours,
                        source_constraint,
                        &target_groups,
                        &claim_priorities,
                        &target_pair_cache,
                    )
                })
                .collect::<Vec<_>>()
        };
    for mut source_contacts in source_contacts {
        contacts.append(&mut source_contacts);
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

fn source_authorized_target_pair_cache(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    claim_priorities: &BTreeMap<NodeBandOwner, NodeGeneratedContourClaimPriority>,
    source_constraints: &[RaisedStepSourceConstraint<'_>],
    target_groups: &[super::types::SourceAuthorizedTargetGroup],
) -> SourceAuthorizedTargetPairCache {
    let mut target_pair_cache = SourceAuthorizedTargetPairCache::new();
    for source_constraint in source_constraints {
        for target_group in target_groups {
            target_pair_cache
                .entry((source_constraint.source.owners, target_group.key))
                .or_insert_with(|| {
                    source_authorized_raised_step_target_pairs(
                        piece_kind,
                        claim_priorities,
                        source_constraint.source,
                        target_group.key,
                    )
                });
        }
    }
    target_pair_cache
}

fn collect_source_authorized_contacts_for_source(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    contours: &[NodeGeneratedContour],
    source_constraint: &RaisedStepSourceConstraint<'_>,
    target_groups: &[super::types::SourceAuthorizedTargetGroup],
    claim_priorities: &BTreeMap<NodeBandOwner, NodeGeneratedContourClaimPriority>,
    target_pair_cache: &SourceAuthorizedTargetPairCache,
) -> BTreeSet<GeneratedSameBandContactConstraint> {
    let mut contacts = BTreeSet::new();
    for target_group in target_groups {
        if source_constraint.bounds_disjoint_group(target_group) {
            continue;
        }
        let Some(target_contacts) =
            target_pair_cache.get(&(source_constraint.source.owners, target_group.key))
        else {
            continue;
        };
        if target_contacts.is_empty() {
            continue;
        }
        for source_edge in &source_constraint.edges {
            let source_edge = *source_edge;
            if target_group.bounds_disjoint_edge(source_edge) {
                continue;
            }
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
                for (owner, opposite_owner, include_edge) in target_contacts {
                    for (start, end) in source_authorized_contact_segments(edge, *include_edge) {
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
        target_groups,
        &mut contacts,
    );
    collect_junctionn_source_authorized_mouth_band_endpoint_handoffs(
        piece_kind,
        contours,
        source_constraint,
        target_groups,
        claim_priorities,
        &mut contacts,
    );
    contacts
}

fn collect_junctionn_source_authorized_mouth_band_endpoint_handoffs(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    contours: &[NodeGeneratedContour],
    source_constraint: &RaisedStepSourceConstraint<'_>,
    target_groups: &[super::types::SourceAuthorizedTargetGroup],
    claim_priorities: &BTreeMap<NodeBandOwner, NodeGeneratedContourClaimPriority>,
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
                    || claim_priorities.get(&target_owner).copied()
                        != Some(target_group.key.claim_priority)
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
                    generated_raised_step_endpoint_source(constraint).map(|source| {
                        let edges = generated_constraint_directed_edges(constraint);
                        let (min_x, min_z, max_x, max_z) = generated_constraint_bounds(constraint);
                        RaisedStepSourceConstraint {
                            source,
                            constraint,
                            edges,
                            min_x,
                            min_z,
                            max_x,
                            max_z,
                        }
                    })
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
                        source_constraint.bounds_contains_key(point)
                            && generated_constraint_touches_key(source_constraint.constraint, point)
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
            let left = &source_constraints[left_index];
            let right = &source_constraints[right_index];
            if left.bounds_disjoint_source(right) {
                continue;
            }
            for left_edge in &left.edges {
                for right_edge in &right.edges {
                    points.extend(generated_source_edge_contact_points(left_edge, right_edge));
                }
            }
        }
    }
    points
}

fn generated_source_edge_contact_points(
    left: &GeneratedContourDirectedEdge,
    right: &GeneratedContourDirectedEdge,
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

fn generated_constraint_bounds(constraint: &NodeRailConstraint) -> (i64, i64, i64, i64) {
    let (mut min_x, mut min_z) = (i64::MAX, i64::MAX);
    let (mut max_x, mut max_z) = (i64::MIN, i64::MIN);
    for point in constraint.points_xz.iter().copied().map(road_point_key) {
        min_x = min_x.min(point.0);
        min_z = min_z.min(point.1);
        max_x = max_x.max(point.0);
        max_z = max_z.max(point.1);
    }
    if constraint.points_xz.is_empty() {
        return (1, 1, 0, 0);
    }
    (min_x, min_z, max_x, max_z)
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
