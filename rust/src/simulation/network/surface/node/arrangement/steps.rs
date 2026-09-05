// SPDX-License-Identifier: GPL-2.0-only

//! Explicit vertical-step authority extraction from canonical arrangement edges.

use super::super::band_semantics::{
    ordered_raised_step_kinds, raised_step_band_rank, raised_step_kinds_can_contact,
};
use super::super::keys::{SurfaceSegmentParameter, SurfaceXzKey};
use super::super::segments::{
    exact_line_parameter, interpolate_height_i64, interpolate_key,
    key_collinear_with_overlay_grid_segment, overlay_segment_parameter, segment_parameter_key,
};
use super::seams::{
    NodeRegionSeamConstraint, owners_for_material_seam_constraint, seam_constraint_covers_edge,
    seam_constraint_matches_owner_pair, seam_constraint_opposite_owner_for_edge_owner,
};
use super::{
    NodeArrangement, NodeArrangementEdge, NodeArrangementKey, NodeBandOwner, NodeOwnedRegionId,
    NodeSeamSource,
};
use std::collections::{BTreeMap, BTreeSet};

mod authority;
mod extraction;
mod reuse;
mod segment;

use authority::owner_sets_match_step;
pub(crate) use authority::owners_form_explicit_vertical_step_pair;
pub(crate) use reuse::NodeFinalExplicitStepTopologyCache;
pub(crate) use segment::{
    NodeExplicitVerticalStepSegment, explicit_vertical_step_segments_authorize_height_side_at_key,
};
