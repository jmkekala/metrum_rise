//! Explicit vertical-step authority extraction from canonical arrangement edges.

use super::super::band_semantics::{ordered_raised_step_kinds, raised_step_band_rank};
use super::seams::{
    NodeRegionSeamConstraint, seam_constraint_covers_edge, seam_constraint_covers_key,
    seam_constraint_matches_owner_pair, seam_constraint_opposite_owner_for_edge_owner,
};
use super::{
    NodeArrangement, NodeArrangementEdge, NodeArrangementKey, NodeBandOwner, NodeSeamSource,
};
use std::collections::BTreeSet;

mod authority;
mod extraction;
mod segment;

pub(crate) use authority::owners_form_explicit_vertical_step_pair;
use authority::{owner_sets_have_explicit_vertical_step_endpoint_authority, owner_sets_match_step};
pub(crate) use segment::{
    NodeExplicitVerticalStepSegment, explicit_vertical_step_segments_authorize_height_side_at_key,
};
