//! Source-authorized generated-contact target grouping.

use super::super::geometry::{
    generated_directed_edge_segments_inside_shape_edges, generated_overlay_contour,
    generated_overlay_shapes_directed_edges, generated_shape_boundary_segments_on_source_edge,
};
use super::super::{
    GeneratedContourDirectedEdge, GeneratedContourEdgeKey, GeneratedRaisedStepOwnerPair,
    NodeBandOwner, NodeGeneratedContour, NodeGeneratedContourClaimPriority, NodeRailConstraintKind,
    NodeRailPointKey, RoadSurfaceBandKind, RoadSurfaceSystem, RoadSurfaceVisualNodePieceKind,
    generated_constraint_contains_key_segment, generated_contour_directed_edges,
};
use super::types::{
    GeneratedRaisedStepEndpointSource, GeneratedSameBandContactConstraint,
    RaisedStepSourceConstraint, SourceAuthorizedTargetGroup, SourceAuthorizedTargetGroupKey,
};
use i_overlay::core::overlay_rule::OverlayRule;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) struct SourceAuthorizedTargetGroupView
{
    pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) geometry:
        Arc<SourceAuthorizedTargetGroup>,
    pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) contour_indices:
        Arc<[usize]>,
}

pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) fn collect_source_authorized_exact_group_pair_overlap_contacts(
    source_constraint: &RaisedStepSourceConstraint<'_>,
    contours: &[NodeGeneratedContour],
    left_group: &SourceAuthorizedTargetGroupView,
    right_group: &SourceAuthorizedTargetGroupView,
    contacts: &mut BTreeSet<GeneratedSameBandContactConstraint>,
) {
    let [left_owner, right_owner] = source_constraint.source.owners;
    if left_group.geometry.key.owner != left_owner
        || right_group.geometry.key.owner != right_owner
        || source_constraint.bounds_disjoint_group(&left_group.geometry)
        || source_constraint.bounds_disjoint_group(&right_group.geometry)
    {
        return;
    }
    for edge in source_authorized_group_edges_inside_group(
        source_constraint,
        left_group,
        right_group,
        contours,
    )
    .into_iter()
    .chain(source_authorized_group_edges_inside_group(
        source_constraint,
        right_group,
        left_group,
        contours,
    ))
    .chain(source_authorized_source_edges_inside_group_intersection(
        source_constraint,
        &left_group.geometry,
        &right_group.geometry,
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

pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) fn source_authorized_target_group(
    contours: &[NodeGeneratedContour],
    key: SourceAuthorizedTargetGroupKey,
    contour_indices: &[usize],
) -> Option<SourceAuthorizedTargetGroup> {
    let overlay_contours = contour_indices
        .iter()
        .map(|index| generated_overlay_contour(&contours[*index]))
        .collect::<Vec<_>>();
    let shapes = RoadSurfaceSystem::overlay_union_contours(&overlay_contours)?;
    let shape_edges = generated_overlay_shapes_directed_edges(&shapes);
    let (min_x, min_z, max_x, max_z) = source_authorized_group_bounds(&shape_edges);
    Some(SourceAuthorizedTargetGroup {
        key,
        // Indices belong to the current contour slice and are therefore kept on
        // SourceAuthorizedTargetGroupView rather than cached with the geometry.
        contour_indices: Vec::new(),
        shape_edges,
        shapes,
        min_x,
        min_z,
        max_x,
        max_z,
    })
}

fn source_authorized_group_edges_inside_group(
    source_constraint: &RaisedStepSourceConstraint<'_>,
    edge_group: &SourceAuthorizedTargetGroupView,
    containing_group: &SourceAuthorizedTargetGroupView,
    contours: &[NodeGeneratedContour],
) -> Vec<GeneratedContourEdgeKey> {
    if source_constraint.bounds_disjoint_group(&edge_group.geometry)
        || source_constraint.bounds_disjoint_group(&containing_group.geometry)
    {
        return Vec::new();
    }
    let mut edges = BTreeSet::new();
    for contour_index in edge_group.contour_indices.iter() {
        let Some(contour) = contours.get(*contour_index) else {
            continue;
        };
        for contour_edge in generated_contour_directed_edges(contour) {
            if containing_group.geometry.bounds_disjoint_edge(contour_edge) {
                continue;
            }
            let mut candidate_edges = generated_directed_edge_segments_inside_shape_edges(
                contour_edge,
                &containing_group.geometry.shape_edges,
                &containing_group.geometry.shapes,
            )
            .into_iter()
            .collect::<BTreeSet<_>>();
            candidate_edges.extend(generated_shape_boundary_segments_on_source_edge(
                contour_edge,
                &containing_group.geometry.shape_edges,
            ));
            for edge in candidate_edges {
                if generated_constraint_contains_key_segment(
                    source_constraint.constraint,
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
    source_constraint: &RaisedStepSourceConstraint<'_>,
    left_group: &SourceAuthorizedTargetGroup,
    right_group: &SourceAuthorizedTargetGroup,
) -> Vec<GeneratedContourEdgeKey> {
    if left_group.bounds_disjoint_group(right_group)
        || source_constraint.bounds_disjoint_group(left_group)
        || source_constraint.bounds_disjoint_group(right_group)
    {
        return Vec::new();
    }
    let Some(intersection_shapes) = RoadSurfaceSystem::overlay_binary_shapes(
        &left_group.shapes,
        &right_group.shapes,
        OverlayRule::Intersect,
    ) else {
        return Vec::new();
    };
    let intersection_edges = generated_overlay_shapes_directed_edges(&intersection_shapes);
    let mut edges = BTreeSet::new();
    for source_edge in &source_constraint.edges {
        edges.extend(generated_directed_edge_segments_inside_shape_edges(
            *source_edge,
            &intersection_edges,
            &intersection_shapes,
        ));
        edges.extend(generated_shape_boundary_segments_on_source_edge(
            *source_edge,
            &intersection_edges,
        ));
    }
    edges.into_iter().collect()
}

pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) fn source_authorized_raised_step_target_pairs(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    effective_owner_priority: Option<NodeGeneratedContourClaimPriority>,
    source: GeneratedRaisedStepEndpointSource,
    target: SourceAuthorizedTargetGroupKey,
) -> Vec<(NodeBandOwner, NodeBandOwner, bool)> {
    let target_owner = target.owner;
    if source.owners.contains(&target_owner) {
        if effective_owner_priority == Some(target.claim_priority) {
            let Some(pair) = GeneratedRaisedStepOwnerPair::new(source.owners[0], source.owners[1])
            else {
                return Vec::new();
            };
            return vec![(pair.owner, pair.opposite_owner, true)];
        }
        return Vec::new();
    }

    let can_reown_target_edge = match piece_kind {
        RoadSurfaceVisualNodePieceKind::Bend => {
            target.claim_priority == NodeGeneratedContourClaimPriority::SideJoin
        }
        RoadSurfaceVisualNodePieceKind::JunctionN => {
            target_owner.kind() == RoadSurfaceBandKind::CurbOrShoulder
                && target.claim_priority == NodeGeneratedContourClaimPriority::MouthBand
                && effective_owner_priority == Some(target.claim_priority)
        }
        RoadSurfaceVisualNodePieceKind::Terminal => false,
    };
    if !can_reown_target_edge {
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
        let include_edge = piece_kind == RoadSurfaceVisualNodePieceKind::JunctionN
            && pair.owner.kind() != pair.opposite_owner.kind();
        pairs.push((pair.owner, pair.opposite_owner, include_edge));
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

pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) fn source_authorized_target_claim_priority(
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

pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) fn source_authorized_target_claim_priorities(
    contours: &[NodeGeneratedContour],
) -> BTreeMap<NodeBandOwner, NodeGeneratedContourClaimPriority> {
    let mut owners = contours
        .iter()
        .filter_map(|contour| contour.owner)
        .collect::<Vec<_>>();
    owners.sort_unstable();
    owners.dedup();
    owners
        .into_iter()
        .filter_map(|owner| {
            source_authorized_target_claim_priority(contours, owner)
                .map(|priority| (owner, priority))
        })
        .collect()
}

fn source_authorized_group_bounds(
    shape_edges: &[GeneratedContourDirectedEdge],
) -> (i64, i64, i64, i64) {
    let (mut min_x, mut min_z) = (i64::MAX, i64::MAX);
    let (mut max_x, mut max_z) = (i64::MIN, i64::MIN);
    for edge in shape_edges {
        for point in [edge.start, edge.end] {
            min_x = min_x.min(point.0);
            min_z = min_z.min(point.1);
            max_x = max_x.max(point.0);
            max_z = max_z.max(point.1);
        }
    }
    if shape_edges.is_empty() {
        return (1, 1, 0, 0);
    }
    (min_x, min_z, max_x, max_z)
}
