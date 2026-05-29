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
    shared_generated_contour_edges, shared_generated_contour_points,
};
mod authority;
mod emission;

pub(in crate::simulation::network::surface::node::rails::contacts) use authority::{
    GeneratedContactAuthorityIndex, generated_contact_point_has_explicit_roles,
};
pub(in crate::simulation::network::surface::node::rails) use emission::{
    append_generated_material_point_contact_constraints,
    append_generated_same_band_contact_constraints,
    append_source_authorized_raised_step_point_contacts,
};
