//! Source-authorized generated rail contact materialization.

mod geometry;
mod materialization;
mod noding;
mod source_authority;
mod validation;

use super::super::arrangement::NodeBandOwner;
use super::super::backend::{ROAD_OVERLAY_COORDINATE_SCALE, RoadVec3};
use super::super::band_semantics::{raised_step_band_rank, raised_step_kinds_can_contact};
use super::super::keys::SurfaceXzKey;
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
use super::geometry::{quantized_proper_segment_intersection, road_point_from_key, road_point_key};
use super::owners::generated_contour_band_kind;
use super::topology::{
    GeneratedContourDirectedEdge, GeneratedContourEdgeKey, NodeRailPointKey,
    generated_contour_directed_edges, generated_contour_keys, set_generated_contour_from_keys,
    shared_generated_contour_points,
};
use super::{
    NodeGeneratedContour, NodeGeneratedContourClaimPriority, NodeRailConstraint,
    NodeRailConstraintKind, NodeRailGenerationError,
};

pub(super) use materialization::{
    append_generated_material_point_contact_constraints,
    append_generated_same_band_contact_constraints,
    append_source_authorized_raised_step_point_contacts,
};
pub(super) use noding::{
    node_generated_contact_contours, node_generated_contact_source_constraints,
    node_generated_contact_sources_from_contour_backed_contacts,
};
pub(super) use source_authority::{
    generated_raised_step_boundary_role_for_owner, raised_step_band_kinds_can_contact,
};
pub(super) use validation::{
    retain_source_authorized_generated_contact_constraints,
    validate_generated_contact_constraint_endpoints_from_sources,
};
