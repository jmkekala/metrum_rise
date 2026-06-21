//! Terminal-cap and side-join rail contour/constraint generation.

use super::super::arrangement::NodeBandOwner;
use super::super::backend::{RoadVec2, RoadVec3, polyline_to_road_points, road_vec3_xz as xz};
use super::super::input::NodeInputMouth;
use super::super::joins::{
    NodeInputSideJoinBand, NodeInputSideJoinBandBoundaryMode, NodeInputSideJoinGapRole,
};
use super::super::terminal::{NodeTerminalCapBand, TerminalCapBandRole};
use super::super::{
    NodeOverlayContour, RoadSurfaceBandKind, RoadSurfaceSystem, RoadSurfaceVisualNodePieceKind,
};
use super::contours::{
    align_height_points_to_source_contours, clean_generated_constraint_path,
    cleaned_closed_contour, push_constraint, push_generated_band_constraint,
    push_generated_band_path_constraint,
};
use super::geometry::road_point_key;
use super::owners::{MouthOwners, is_carriageway};
use super::{
    NodeGeneratedContour, NodeGeneratedContourClaimPriority, NodeGeneratedContourKind,
    NodeGeneratedContourPurpose, NodeGeneratedCornerTrim, NodeRailConstraint,
    NodeRailConstraintKind, NodeRailGenerationError,
};
use std::collections::BTreeMap;

mod boundary_constraints;
mod generation;

pub(super) use generation::{push_side_join_band_contours, push_terminal_cap_band_contours};
