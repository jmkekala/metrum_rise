// SPDX-License-Identifier: GPL-2.0-only

//! Materialization of source-authorized generated rail contacts.

use super::geometry::{
    GeneratedContactOverlayScratch, GeneratedOverlayShapeKeys, PreparedGeneratedContourEdge,
    PreparedGeneratedPointLocationContour, append_generated_contact_edges_inside_prepared_contour,
    generated_contact_edges_from_overlay_intersection,
    generated_contact_edges_from_overlay_shape_intersection,
    generated_contact_edges_from_overlay_shape_key_intersection,
    generated_contact_edges_from_source_edges_inside_shape_key_intersection,
    generated_contour_overlay_shapes, generated_overlay_shape_keys_directed_edges,
    generated_overlay_shapes_keys,
};
use super::source_authority::{
    GeneratedSameBandContactConstraint, NodeSourceAuthorizedContactCache,
    SourceAuthorizedContactReuseStats, collect_source_authorized_raised_step_contacts_with_reuse,
    generated_raised_step_contact_kind_for_owners, generated_same_band_contact_constraint_key,
};
use super::{
    GeneratedContourDirectedEdge, GeneratedContourEdgeKey, GeneratedRaisedStepOwnerPair,
    NodeBandOwner, NodeGeneratedContour, NodeGeneratedContourClaimPriority, NodeOverlayShapes,
    NodeRailConstraint, NodeRailConstraintKind, NodeRailPointKey, RoadSurfaceBandKind,
    RoadSurfaceVisualNodePieceKind, append_quantized_segment_contact_points,
    generated_contour_band_kind, generated_contour_keys, generated_contour_supports_same_band_role,
    generated_same_band_boundary_role_at_contour_vertex, owners_match_unordered,
    road_point_from_key, road_point_key,
};
use std::collections::BTreeMap;
mod authority;
mod emission;

#[derive(Clone, Debug, Default)]
pub(in crate::simulation::network::surface::node::rails) struct GeneratedContactEmissionStats {
    pub(in crate::simulation::network::surface::node::rails) pair_tests: usize,
    pub(in crate::simulation::network::surface::node::rails) aabb_rejected: usize,
    pub(in crate::simulation::network::surface::node::rails) kind_rejected: usize,
    pub(in crate::simulation::network::surface::node::rails) processed_pairs: usize,
    pub(in crate::simulation::network::surface::node::rails) overlay_calls: usize,
    pub(in crate::simulation::network::surface::node::rails) emitted_constraints: usize,
    pub(in crate::simulation::network::surface::node::rails) candidate_pairs: usize,
    pub(in crate::simulation::network::surface::node::rails) same_material_candidate_pairs: usize,
    pub(in crate::simulation::network::surface::node::rails) raised_step_candidate_pairs: usize,
    pub(in crate::simulation::network::surface::node::rails) authority_rejected: usize,
    pub(in crate::simulation::network::surface::node::rails) same_authority_skipped: usize,
    pub(in crate::simulation::network::surface::node::rails) same_material_overlay_calls: usize,
    pub(in crate::simulation::network::surface::node::rails) same_material_pair_cache_hits: usize,
    pub(in crate::simulation::network::surface::node::rails) raised_step_pair_cache_previous_hits:
        usize,
    pub(in crate::simulation::network::surface::node::rails) raised_step_pair_cache_misses: usize,
    pub(in crate::simulation::network::surface::node::rails) source_target_group_cache_hits: usize,
    pub(in crate::simulation::network::surface::node::rails) source_contact_cache_hits: usize,
    pub(in crate::simulation::network::surface::node::rails) source_contact_cache_misses: usize,
    pub(in crate::simulation::network::surface::node::rails) source_pair_cache_hits: usize,
    pub(in crate::simulation::network::surface::node::rails) source_pair_cache_misses: usize,
    pub(in crate::simulation::network::surface::node::rails) same_material_height_split_candidates:
        usize,
    pub(in crate::simulation::network::surface::node::rails) same_material_height_split_appended:
        usize,
    pub(in crate::simulation::network::surface::node::rails) same_material_height_split_duplicates:
        usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(in crate::simulation::network::surface::node::rails::contacts::materialization) struct GeneratedContactAuthorityKey
{
    pub(in crate::simulation::network::surface::node::rails::contacts::materialization) owner:
        NodeBandOwner,
    pub(in crate::simulation::network::surface::node::rails::contacts::materialization) kind:
        RoadSurfaceBandKind,
    pub(in crate::simulation::network::surface::node::rails::contacts::materialization) source_mouth_order_index:
        usize,
    pub(in crate::simulation::network::surface::node::rails::contacts::materialization) source_band_index:
        Option<usize>,
    pub(in crate::simulation::network::surface::node::rails::contacts::materialization) claim_priority:
        NodeGeneratedContourClaimPriority,
    pub(in crate::simulation::network::surface::node::rails::contacts::materialization) has_height_carrier:
        bool,
}

#[derive(Clone, Debug)]
pub(in crate::simulation::network::surface::node::rails::contacts::materialization) struct GeneratedContactContourSummary
{
    pub(in crate::simulation::network::surface::node::rails::contacts::materialization) owner:
        Option<NodeBandOwner>,
    pub(in crate::simulation::network::surface::node::rails::contacts::materialization) kind:
        Option<RoadSurfaceBandKind>,
    pub(in crate::simulation::network::surface::node::rails::contacts::materialization) keys:
        Vec<NodeRailPointKey>,
    point_location: PreparedGeneratedPointLocationContour,
    pub(in crate::simulation::network::surface::node::rails::contacts::materialization) edges:
        Vec<GeneratedContourEdgeKey>,
    edges_by_min_x: Vec<PreparedGeneratedContourEdge>,
    edges_by_min_z: Vec<PreparedGeneratedContourEdge>,
    pub(in crate::simulation::network::surface::node::rails::contacts::materialization) overlay_shapes:
        Option<NodeOverlayShapes>,
    pub(in crate::simulation::network::surface::node::rails::contacts::materialization) overlay_shape_edges:
        Vec<GeneratedContourDirectedEdge>,
    pub(in crate::simulation::network::surface::node::rails::contacts::materialization) overlay_shape_keys:
        GeneratedOverlayShapeKeys,
    pub(in crate::simulation::network::surface::node::rails::contacts::materialization) authority_key:
        Option<GeneratedContactAuthorityKey>,
    min_x: i64,
    min_z: i64,
    max_x: i64,
    max_z: i64,
}

impl GeneratedContactContourSummary {
    pub(in crate::simulation::network::surface::node::rails::contacts::materialization) fn from_contour(
        contour: &NodeGeneratedContour,
    ) -> Self {
        Self::from_contour_with_overlay(contour, false)
    }

    pub(in crate::simulation::network::surface::node::rails::contacts::materialization) fn from_contour_with_overlay(
        contour: &NodeGeneratedContour,
        include_overlay_shapes: bool,
    ) -> Self {
        let ordered_keys = generated_contour_keys(contour);
        let mut keys = ordered_keys.clone();
        keys.sort_unstable();
        keys.dedup();
        let mut edges = Vec::new();
        if ordered_keys.len() >= 2 {
            for index in 0..ordered_keys.len() {
                let start = ordered_keys[index];
                let end = ordered_keys[(index + 1) % ordered_keys.len()];
                if start != end {
                    edges.push(GeneratedContourEdgeKey::new(start, end));
                }
            }
            edges.sort_unstable();
            edges.dedup();
        }
        let prepared_edges = edges
            .iter()
            .copied()
            .map(PreparedGeneratedContourEdge::new)
            .collect::<Vec<_>>();
        let mut edges_by_min_x = prepared_edges.clone();
        edges_by_min_x.sort_unstable_by_key(|edge| {
            (edge.min_x, edge.max_x, edge.min_z, edge.max_z, edge.edge)
        });
        let mut edges_by_min_z = prepared_edges;
        edges_by_min_z.sort_unstable_by_key(|edge| {
            (edge.min_z, edge.max_z, edge.min_x, edge.max_x, edge.edge)
        });
        let (mut min_x, mut min_z) = (i64::MAX, i64::MAX);
        let (mut max_x, mut max_z) = (i64::MIN, i64::MIN);
        for key in &keys {
            min_x = min_x.min(key.0);
            min_z = min_z.min(key.1);
            max_x = max_x.max(key.0);
            max_z = max_z.max(key.1);
        }
        if keys.is_empty() {
            min_x = 1;
            min_z = 1;
            max_x = 0;
            max_z = 0;
        }
        let point_location = PreparedGeneratedPointLocationContour::new(&ordered_keys);
        let overlay_shapes = include_overlay_shapes
            .then(|| generated_contour_overlay_shapes(contour))
            .flatten();
        let overlay_shape_keys = overlay_shapes
            .as_ref()
            .map(generated_overlay_shapes_keys)
            .unwrap_or_default();
        let overlay_shape_edges = generated_overlay_shape_keys_directed_edges(&overlay_shape_keys);
        let owner = contour.owner;
        let kind = generated_contour_band_kind(contour);
        let authority_key = owner
            .zip(kind)
            .map(|(owner, kind)| GeneratedContactAuthorityKey {
                owner,
                kind,
                source_mouth_order_index: contour.source_mouth_order_index,
                source_band_index: contour.source_band_index,
                claim_priority: contour.claim_priority,
                has_height_carrier: contour
                    .height_points_world
                    .as_ref()
                    .is_some_and(|points| !points.is_empty()),
            });
        Self {
            owner,
            kind,
            keys,
            point_location,
            edges,
            edges_by_min_x,
            edges_by_min_z,
            overlay_shapes,
            overlay_shape_edges,
            overlay_shape_keys,
            authority_key,
            min_x,
            min_z,
            max_x,
            max_z,
        }
    }

    fn replace_overlay_shapes(&mut self, overlay_shapes: Option<NodeOverlayShapes>) {
        self.overlay_shape_keys = overlay_shapes
            .as_ref()
            .map(generated_overlay_shapes_keys)
            .unwrap_or_default();
        self.overlay_shape_edges =
            generated_overlay_shape_keys_directed_edges(&self.overlay_shape_keys);
        self.overlay_shapes = overlay_shapes;
    }

    pub(in crate::simulation::network::surface::node::rails::contacts::materialization) fn aabb_disjoint(
        &self,
        other: &Self,
    ) -> bool {
        self.max_x + 1 < other.min_x
            || other.max_x + 1 < self.min_x
            || self.max_z + 1 < other.min_z
            || other.max_z + 1 < self.min_z
    }

    pub(in crate::simulation::network::surface::node::rails::contacts::materialization) fn bounds_valid(
        &self,
    ) -> bool {
        self.min_x <= self.max_x && self.min_z <= self.max_z
    }

    pub(in crate::simulation::network::surface::node::rails::contacts::materialization) fn bounds(
        &self,
    ) -> Option<(i64, i64, i64, i64)> {
        self.bounds_valid()
            .then_some((self.min_x, self.min_z, self.max_x, self.max_z))
    }

    pub(in crate::simulation::network::surface::node::rails::contacts::materialization) fn bounds_contain_key(
        &self,
        point: NodeRailPointKey,
    ) -> bool {
        self.min_x <= point.0
            && point.0 <= self.max_x
            && self.min_z <= point.1
            && point.1 <= self.max_z
    }
}

pub(in crate::simulation::network::surface::node::rails::contacts::materialization) fn generated_contact_contour_summaries(
    contours: &[NodeGeneratedContour],
) -> Vec<GeneratedContactContourSummary> {
    contours
        .iter()
        .map(GeneratedContactContourSummary::from_contour)
        .collect()
}

const GENERATED_CONTACT_CANDIDATE_TILE_KEYS: i64 = 8_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct GeneratedContactCandidateTile {
    x: i64,
    z: i64,
}

pub(in crate::simulation::network::surface::node::rails::contacts::materialization) fn generated_contact_candidate_pair_indices(
    summaries: &[GeneratedContactContourSummary],
) -> Vec<(usize, usize)> {
    let mut indices_by_tile = BTreeMap::<GeneratedContactCandidateTile, Vec<usize>>::new();
    for (summary_index, summary) in summaries.iter().enumerate() {
        let Some((min_x, min_z, max_x, max_z)) = summary.bounds() else {
            continue;
        };
        let min_tile_x = min_x.div_euclid(GENERATED_CONTACT_CANDIDATE_TILE_KEYS);
        let max_tile_x = max_x.div_euclid(GENERATED_CONTACT_CANDIDATE_TILE_KEYS);
        let min_tile_z = min_z.div_euclid(GENERATED_CONTACT_CANDIDATE_TILE_KEYS);
        let max_tile_z = max_z.div_euclid(GENERATED_CONTACT_CANDIDATE_TILE_KEYS);
        for x in min_tile_x..=max_tile_x {
            for z in min_tile_z..=max_tile_z {
                indices_by_tile
                    .entry(GeneratedContactCandidateTile { x, z })
                    .or_default()
                    .push(summary_index);
            }
        }
    }

    let mut pairs = Vec::new();
    for indices in indices_by_tile.values() {
        for left_position in 0..indices.len() {
            for right_index in indices.iter().copied().skip(left_position + 1) {
                let left_index = indices[left_position];
                pairs.push((left_index.min(right_index), left_index.max(right_index)));
            }
        }
    }
    pairs.sort_unstable();
    pairs.dedup();
    pairs
}

pub(in crate::simulation::network::surface::node::rails::contacts::materialization) fn append_generated_contact_points_from_summary_intersections(
    left: &GeneratedContactContourSummary,
    right: &GeneratedContactContourSummary,
    points: &mut Vec<NodeRailPointKey>,
) {
    let (outer, inner) = if left.edges_by_min_x.len() <= right.edges_by_min_x.len() {
        (left, right)
    } else {
        (right, left)
    };
    for prepared_left_edge in &outer.edges_by_min_x {
        let left_min_x = prepared_left_edge.min_x;
        let left_max_x = prepared_left_edge.max_x;
        let left_min_z = prepared_left_edge.min_z;
        let left_max_z = prepared_left_edge.max_z;
        let left_edge = prepared_left_edge.edge;
        let x_last = inner
            .edges_by_min_x
            .partition_point(|edge| edge.min_x <= left_max_x);
        let z_last = inner
            .edges_by_min_z
            .partition_point(|edge| edge.min_z <= left_max_z);
        let right_edges = if x_last <= z_last {
            &inner.edges_by_min_x[..x_last]
        } else {
            &inner.edges_by_min_z[..z_last]
        };
        for right_edge in right_edges {
            if right_edge.max_x < left_min_x
                || left_max_x < right_edge.min_x
                || right_edge.max_z < left_min_z
                || left_max_z < right_edge.min_z
            {
                continue;
            }
            let right_edge = right_edge.edge;
            append_quantized_segment_contact_points(
                left_edge.start,
                left_edge.end,
                right_edge.start,
                right_edge.end,
                points,
            );
        }
    }
}

pub(in crate::simulation::network::surface::node::rails::contacts::materialization) fn append_generated_contact_edges_inside_summary(
    role: &GeneratedContactContourSummary,
    target: &GeneratedContactContourSummary,
    edges: &mut Vec<GeneratedContourEdgeKey>,
    split_keys: &mut Vec<NodeRailPointKey>,
) {
    append_generated_contact_edges_inside_prepared_contour(
        &role.edges_by_min_x,
        &target.edges_by_min_x,
        &target.edges_by_min_z,
        &target.point_location,
        target.bounds(),
        edges,
        split_keys,
    );
}

pub(in crate::simulation::network::surface::node::rails::contacts::materialization) fn append_shared_sorted_keys(
    left: &[NodeRailPointKey],
    right: &[NodeRailPointKey],
    shared: &mut Vec<NodeRailPointKey>,
) {
    let (mut left_index, mut right_index) = (0, 0);
    while left_index < left.len() && right_index < right.len() {
        match left[left_index].cmp(&right[right_index]) {
            std::cmp::Ordering::Less => left_index += 1,
            std::cmp::Ordering::Greater => right_index += 1,
            std::cmp::Ordering::Equal => {
                shared.push(left[left_index]);
                left_index += 1;
                right_index += 1;
            }
        }
    }
}

pub(in crate::simulation::network::surface::node::rails::contacts::materialization) fn append_shared_sorted_edges(
    left: &[GeneratedContourEdgeKey],
    right: &[GeneratedContourEdgeKey],
    shared: &mut Vec<GeneratedContourEdgeKey>,
) {
    let (mut left_index, mut right_index) = (0, 0);
    while left_index < left.len() && right_index < right.len() {
        match left[left_index].cmp(&right[right_index]) {
            std::cmp::Ordering::Less => left_index += 1,
            std::cmp::Ordering::Greater => right_index += 1,
            std::cmp::Ordering::Equal => {
                shared.push(left[left_index]);
                left_index += 1;
                right_index += 1;
            }
        }
    }
}

pub(in crate::simulation::network::surface::node::rails::contacts) use authority::{
    GeneratedContactAuthorityIndex, GeneratedContactAuthorityPointQuery,
};
pub(in crate::simulation::network::surface::node::rails) use emission::{
    NodeSameMaterialContactPairCache, append_generated_material_point_contact_constraints,
    append_generated_same_band_contact_constraints_with_source_reuse,
    append_source_authorized_raised_step_point_contacts_with_reuse,
};
#[cfg(test)]
pub(in crate::simulation::network::surface::node::rails) use emission::{
    append_generated_same_band_contact_constraints,
    append_generated_same_band_contact_constraints_with_reuse,
    append_source_authorized_raised_step_point_contacts,
};
