// SPDX-License-Identifier: GPL-2.0-only

//! Generic generated contour, constraint, and path helpers for node rails.

use super::super::arrangement::NodeBandOwner;
use super::super::backend::{
    RoadPolyline, RoadVec2, RoadVec3, polyline_to_road_points, road_points_to_polyline,
    road_vec3_xz as xz,
};
use super::super::input::{NodeInputBoundaryRailRole, NodeInputMouth, NodeInputProfileRail};
use super::super::keys::SurfaceHeightMmKey;
use super::super::segments::raw_tuple_key_lies_on_segment as generated_point_key_lies_on_segment;
use super::super::{NODE_OVERLAY_MIN_AREA_M2, RoadSurfaceBandKind};
use super::constraints::{GeneratedRaisedStepOwnerPair, boundary_constraint_kind};
use super::geometry::road_point_key;
use super::topology::NodeRailPointKey;
use super::{
    NodeGeneratedContour, NodeGeneratedContourClaimPriority, NodeGeneratedContourKind,
    NodeGeneratedContourPurpose, NodeRailConstraint, NodeRailConstraintKind,
    NodeRailGenerationError, RAIL_CONTOUR_POINT_EQUAL_EPS_M,
};
use cavalier_contours::polyline::{PlineCreation, PlineSource, PlineSourceMut};
use std::collections::BTreeMap;

mod cleaning;
mod constraints;
mod emit;
mod height_points;
mod paths;

pub(super) use cleaning::cleaned_closed_contour;
pub(super) use constraints::{
    push_boundary_constraint, push_constraint, push_generated_band_constraint,
    push_generated_band_path_constraint, push_span_handoff_constraint,
};
pub(super) use emit::{
    default_generated_contour_purpose, push_generated_contour, push_generated_contour_with_purpose,
    push_path_band_contour, push_path_strip_contours,
};
pub(super) use height_points::{
    align_height_points_to_source_contours, height_for_key_on_generated_edge,
};
pub(super) use paths::{
    append_world_path_points, append_world_path_xz, clean_generated_constraint_path,
    push_road_path_point, push_world_path_point, remove_closing_road_path_duplicate,
    remove_closing_world_path_duplicate, subdivided_world_chord,
};
