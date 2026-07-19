//! Rails stage contract tests.

use super::super::arrangement::NodeBandOwner;
use super::super::backend::{RoadVec2, RoadVec3};
use super::super::input::NodeArrangementInput;
use super::super::joins::{
    NodeInputSideJoinBand, NodeInputSideJoinBandBoundaryMode, NodeInputSideJoinGap,
    NodeInputSideJoinGapRole,
};
use super::super::keys::SurfaceHeightMmKey;
use super::super::{RoadSurfaceBandKind, RoadSurfaceVisualNodePieceKind};
use super::caps_and_joins::push_side_join_band_contours;
use super::constraints::owners_match_unordered;
use super::contacts::{
    NodeSourceAuthorizedContactCache, append_generated_same_band_contact_constraints,
    append_generated_same_band_contact_constraints_with_reuse,
    append_source_authorized_raised_step_point_contacts,
    append_source_authorized_raised_step_point_contacts_with_reuse,
    node_generated_contact_source_constraints, synchronize_shared_height_contact_vertices,
    validate_generated_contact_constraint_endpoints_from_sources,
};
use super::contours::{
    height_for_key_on_generated_edge, push_generated_contour, push_generated_contour_with_purpose,
};
use super::geometry::road_point_key;
use super::owners::owners_by_mouth;
use super::topology::GeneratedContourEdgeKey;
use super::{
    NodeGeneratedContour, NodeGeneratedContourClaimPriority, NodeGeneratedContourKind,
    NodeGeneratedContourPurpose, NodeRailConstraint, NodeRailConstraintKind, NodeRailContourSet,
    NodeRailGenerationError,
};
use crate::simulation::network::surface::{
    IncidentEdgeSide, IncidentMouthBand, IncidentMouthProfile, OrderedIncidentPieceMouth,
};
use std::collections::BTreeSet;

mod caps_and_joins;
mod contacts;
mod contours;
mod generated_steps;
mod source_authority;
mod support;

use support::*;
