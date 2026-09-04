//! Source-authorized generated-contact target grouping.

use super::super::geometry::{
    GeneratedOverlayShapeKeys, generated_directed_edge_segments_inside_shape_keys,
    generated_overlay_contour, generated_overlay_shape_keys_directed_edges,
    generated_overlay_shapes_keys, generated_shape_boundary_segments_on_source_edge,
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

#[derive(Clone, Debug, Default)]
pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) struct SourceAuthorizedTargetGroupPairGeometry
{
    group_overlap_edges: Vec<GeneratedContourEdgeKey>,
    intersection_shape_edges: Vec<GeneratedContourDirectedEdge>,
    intersection_shape_keys: GeneratedOverlayShapeKeys,
}

pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) fn collect_source_authorized_exact_group_pair_overlap_contacts(
    source_constraint: &RaisedStepSourceConstraint<'_>,
    geometry: &SourceAuthorizedTargetGroupPairGeometry,
    contacts: &mut BTreeSet<GeneratedSameBandContactConstraint>,
) {
    let [left_owner, right_owner] = source_constraint.source.owners;
    for edge in geometry
        .group_overlap_edges
        .iter()
        .copied()
        .filter(|edge| {
            generated_constraint_contains_key_segment(
                source_constraint.constraint,
                edge.start,
                edge.end,
            )
        })
        .chain(source_authorized_source_edges_inside_group_intersection(
            source_constraint,
            geometry,
        ))
    {
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
    let shape_keys = generated_overlay_shapes_keys(&shapes);
    let shape_edges = generated_overlay_shape_keys_directed_edges(&shape_keys);
    let mut contour_edges = contour_indices
        .iter()
        .flat_map(|index| generated_contour_directed_edges(&contours[*index]))
        .collect::<Vec<_>>();
    contour_edges.sort_unstable();
    contour_edges.dedup();
    let (min_x, min_z, max_x, max_z) = source_authorized_group_bounds(&shape_edges);
    Some(SourceAuthorizedTargetGroup {
        key,
        // Indices belong to the current contour slice and are therefore kept on
        // SourceAuthorizedTargetGroupView rather than cached with the geometry.
        contour_indices: Vec::new(),
        shape_edges,
        shape_keys,
        contour_edges,
        shapes,
        min_x,
        min_z,
        max_x,
        max_z,
    })
}

fn source_authorized_group_edges_inside_group(
    edge_group: &SourceAuthorizedTargetGroupView,
    containing_group: &SourceAuthorizedTargetGroupView,
) -> Vec<GeneratedContourEdgeKey> {
    if edge_group
        .geometry
        .bounds_disjoint_group(&containing_group.geometry)
    {
        return Vec::new();
    }
    let mut edges = Vec::new();
    for &contour_edge in &edge_group.geometry.contour_edges {
        if containing_group.geometry.bounds_disjoint_edge(contour_edge) {
            continue;
        }
        edges.extend(generated_directed_edge_segments_inside_shape_keys(
            contour_edge,
            &containing_group.geometry.shape_edges,
            &containing_group.geometry.shape_keys,
        ));
        edges.extend(generated_shape_boundary_segments_on_source_edge(
            contour_edge,
            &containing_group.geometry.shape_edges,
        ));
    }
    edges.sort_unstable();
    edges.dedup();
    edges
}

fn source_authorized_source_edges_inside_group_intersection(
    source_constraint: &RaisedStepSourceConstraint<'_>,
    geometry: &SourceAuthorizedTargetGroupPairGeometry,
) -> Vec<GeneratedContourEdgeKey> {
    if geometry.intersection_shape_keys.is_empty() {
        return Vec::new();
    }
    let mut edges = Vec::new();
    for source_edge in &source_constraint.edges {
        edges.extend(generated_directed_edge_segments_inside_shape_keys(
            *source_edge,
            &geometry.intersection_shape_edges,
            &geometry.intersection_shape_keys,
        ));
        edges.extend(generated_shape_boundary_segments_on_source_edge(
            *source_edge,
            &geometry.intersection_shape_edges,
        ));
    }
    edges.sort_unstable();
    edges.dedup();
    edges
}

pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) fn source_authorized_target_group_pair_geometry(
    left_group: &SourceAuthorizedTargetGroupView,
    right_group: &SourceAuthorizedTargetGroupView,
) -> SourceAuthorizedTargetGroupPairGeometry {
    if left_group
        .geometry
        .bounds_disjoint_group(&right_group.geometry)
    {
        return SourceAuthorizedTargetGroupPairGeometry::default();
    }
    let mut group_overlap_edges =
        source_authorized_group_edges_inside_group(left_group, right_group);
    group_overlap_edges.extend(source_authorized_group_edges_inside_group(
        right_group,
        left_group,
    ));
    group_overlap_edges.sort_unstable();
    group_overlap_edges.dedup();

    let intersection_shape_keys = RoadSurfaceSystem::overlay_binary_shapes(
        &left_group.geometry.shapes,
        &right_group.geometry.shapes,
        OverlayRule::Intersect,
    )
    .as_ref()
    .map(generated_overlay_shapes_keys)
    .unwrap_or_default();
    let intersection_shape_edges =
        generated_overlay_shape_keys_directed_edges(&intersection_shape_keys);
    SourceAuthorizedTargetGroupPairGeometry {
        group_overlap_edges,
        intersection_shape_edges,
        intersection_shape_keys,
    }
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
