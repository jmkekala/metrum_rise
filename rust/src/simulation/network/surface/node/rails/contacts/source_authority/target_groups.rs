//! Source-authorized generated-contact target grouping.

use super::super::geometry::{
    generated_directed_edge_segments_inside_shape_edges, generated_overlay_contour,
    generated_overlay_shapes_directed_edges, generated_shape_boundary_segments_on_source_edge,
};
use super::super::{
    GeneratedContourEdgeKey, GeneratedRaisedStepOwnerPair, NodeBandOwner, NodeGeneratedContour,
    NodeGeneratedContourClaimPriority, NodeRailConstraint, NodeRailConstraintKind,
    NodeRailPointKey, RoadSurfaceSystem, RoadSurfaceVisualNodePieceKind,
    generated_constraint_contains_key_segment, generated_constraint_directed_edges,
    generated_contour_band_kind, generated_contour_directed_edges,
};
use super::types::{
    GeneratedRaisedStepEndpointSource, GeneratedSameBandContactConstraint,
    RaisedStepSourceConstraint, SourceAuthorizedTargetGroup, SourceAuthorizedTargetGroupKey,
};
use i_overlay::core::overlay_rule::OverlayRule;
use std::collections::{BTreeMap, BTreeSet};

pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) fn collect_source_authorized_exact_group_overlap_contacts(
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

pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) fn source_authorized_raised_step_target_pairs(
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
        if target_owner.kind() != replaced_owner.kind() {
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

pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) fn source_authorized_contact_segments(
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

pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) fn source_authorized_target_groups(
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
