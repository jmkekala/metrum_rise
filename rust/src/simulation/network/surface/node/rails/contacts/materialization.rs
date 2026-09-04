//! Materialization of source-authorized generated rail contacts.

use super::geometry::{
    GeneratedOverlayShapeKeys, generated_contact_edges_from_overlay_intersection,
    generated_contact_edges_from_overlay_shape_intersection,
    generated_contact_edges_from_source_edges_inside_shape_key_intersection,
    generated_contact_edges_inside_contour, generated_contact_points_from_contour_intersections,
    generated_contour_contains_key, generated_contour_overlay_shapes,
    generated_overlay_shape_keys_directed_edges, generated_overlay_shapes_keys,
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
    RoadSurfaceVisualNodePieceKind, generated_constraint_contains_key_segment,
    generated_constraint_directed_edges, generated_constraint_touches_key,
    generated_contour_band_kind, generated_contour_directed_edges, generated_contour_keys,
    generated_contour_supports_same_band_role, generated_point_key_lies_on_segment,
    generated_same_band_boundary_role_at_contour_vertex, owners_match_unordered,
    quantized_proper_segment_intersection, road_point_from_key, road_point_key,
    shared_generated_contour_points,
};
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
    pub(in crate::simulation::network::surface::node::rails::contacts::materialization) edges:
        Vec<GeneratedContourEdgeKey>,
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
        let mut keys = generated_contour_keys(contour);
        keys.sort_unstable();
        keys.dedup();
        let mut edges = Vec::new();
        if keys.len() >= 2 {
            let contour_keys = generated_contour_keys(contour);
            for index in 0..contour_keys.len() {
                let start = contour_keys[index];
                let end = contour_keys[(index + 1) % contour_keys.len()];
                if start != end {
                    edges.push(GeneratedContourEdgeKey::new(start, end));
                }
            }
            edges.sort_unstable();
            edges.dedup();
        }
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
            edges,
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
}

pub(in crate::simulation::network::surface::node::rails::contacts::materialization) fn generated_contact_contour_summaries(
    contours: &[NodeGeneratedContour],
) -> Vec<GeneratedContactContourSummary> {
    contours
        .iter()
        .map(GeneratedContactContourSummary::from_contour)
        .collect()
}

pub(in crate::simulation::network::surface::node::rails::contacts::materialization) fn shared_sorted_keys(
    left: &[NodeRailPointKey],
    right: &[NodeRailPointKey],
) -> Vec<NodeRailPointKey> {
    left.iter()
        .copied()
        .filter(|key| right.binary_search(key).is_ok())
        .collect()
}

pub(in crate::simulation::network::surface::node::rails::contacts::materialization) fn shared_sorted_edges(
    left: &[GeneratedContourEdgeKey],
    right: &[GeneratedContourEdgeKey],
) -> Vec<GeneratedContourEdgeKey> {
    left.iter()
        .copied()
        .filter(|edge| right.binary_search(edge).is_ok())
        .collect()
}

pub(in crate::simulation::network::surface::node::rails::contacts) use authority::{
    GeneratedContactAuthorityIndex, generated_contact_point_has_explicit_roles,
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
