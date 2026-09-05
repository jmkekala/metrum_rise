// SPDX-License-Identifier: GPL-2.0-only

//! Source-authorized generated rail contact materialization.

mod geometry;
mod materialization;
mod noding;
mod shared_height;
mod source_authority;
mod validation;

use super::super::arrangement::NodeBandOwner;
use super::super::backend::{ROAD_OVERLAY_COORDINATE_SCALE, RoadVec3};
use super::super::band_semantics::{
    raised_step_band_rank, raised_step_kinds_can_contact,
    raised_step_requires_exact_constraint_span,
};
use super::super::keys::{SURFACE_MM_PER_M, SurfaceHeightMmKey, SurfaceXzKey};
use super::super::segments::{
    raw_tuple_key_lies_on_segment as generated_point_key_lies_on_segment,
    raw_tuple_segment_parameter_key as generated_segment_parameter_key,
};
use super::super::{
    NodeOverlayContour, NodeOverlayShapes, RoadSurfaceBandKind, RoadSurfaceSystem,
    RoadSurfaceVisualNodePieceKind,
};
use super::constraints::{
    GeneratedRaisedStepOwnerPair, GeneratedSameBandBoundaryRole,
    generated_constraint_contains_key_segment, generated_constraint_directed_edges,
    generated_constraint_touches_key, generated_contour_supports_same_band_role,
    generated_same_band_boundary_role_at_contour_vertex, owners_match_unordered,
};
use super::contours::height_for_key_on_generated_edge;
use super::geometry::{
    append_quantized_segment_contact_points, quantized_proper_segment_intersection,
    road_point_from_key, road_point_key,
};
use super::owners::generated_contour_band_kind;
use super::topology::{
    GeneratedContourDirectedEdge, GeneratedContourEdgeKey, NodeRailPointKey,
    generated_contour_directed_edges, generated_contour_keys, set_generated_contour_from_keys,
};
use super::{
    NodeGeneratedContour, NodeGeneratedContourClaimPriority, NodeGeneratedContourPurpose,
    NodeRailConstraint, NodeRailConstraintKind, NodeRailGenerationError,
};

pub(super) use materialization::{
    NodeSameMaterialContactPairCache, append_generated_material_point_contact_constraints,
    append_generated_same_band_contact_constraints_with_source_reuse,
    append_source_authorized_raised_step_point_contacts_with_reuse,
};
#[cfg(test)]
pub(super) use materialization::{
    append_generated_same_band_contact_constraints,
    append_generated_same_band_contact_constraints_with_reuse,
    append_source_authorized_raised_step_point_contacts,
};
pub(super) use noding::{
    NodeContactNodingPairCache, NodeContactNodingReuseStats,
    node_generated_contact_contours_with_pair_reuse, node_generated_contact_contours_with_reuse,
    node_generated_contact_source_constraints,
    node_generated_contact_sources_from_contour_backed_contacts,
};
pub(super) use shared_height::synchronize_shared_height_contact_vertices;
pub(super) use source_authority::{
    NodeSourceAuthorizedContactCache, SourceAuthorizedContactReuseStats,
    generated_raised_step_boundary_role_for_owner, raised_step_band_kinds_can_contact,
};
#[cfg(test)]
pub(super) use validation::validate_generated_contact_constraint_endpoints_from_sources;
pub(super) use validation::{
    NodeRetainedContactCache, NodeRetainedContactReuseStats,
    retain_source_authorized_generated_contact_constraint_sets_with_reuse,
    validate_generated_contact_constraint_endpoints_with_authority,
};
