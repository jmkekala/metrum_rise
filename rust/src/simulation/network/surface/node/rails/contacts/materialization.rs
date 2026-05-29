//! Materialization of source-authorized generated rail contacts.

use super::geometry::{
    generated_contact_edges_from_overlay_intersection, generated_contact_edges_inside_contour,
    generated_contact_points_from_contour_intersections, generated_contour_contains_key,
};
use super::source_authority::{
    GeneratedSameBandContactConstraint, collect_source_authorized_raised_step_contacts,
    generated_raised_step_contact_kind_for_owners, generated_same_band_contact_constraint_key,
};
use super::{
    GeneratedContourEdgeKey, GeneratedRaisedStepOwnerPair, NodeBandOwner, NodeGeneratedContour,
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
    min_x: i64,
    min_z: i64,
    max_x: i64,
    max_z: i64,
}

impl GeneratedContactContourSummary {
    pub(in crate::simulation::network::surface::node::rails::contacts::materialization) fn from_contour(
        contour: &NodeGeneratedContour,
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
        Self {
            owner: contour.owner,
            kind: generated_contour_band_kind(contour),
            keys,
            edges,
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
    append_generated_material_point_contact_constraints,
    append_generated_same_band_contact_constraints,
    append_source_authorized_raised_step_point_contacts,
};
