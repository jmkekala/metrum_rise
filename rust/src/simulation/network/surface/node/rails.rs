//! Library-backed rail and contour generation for canonical node arrangements.

use super::keys::SURFACE_POLYLINE_POINT_EQUAL_EPS_M;
use super::{RoadSurfaceBandKind, RoadSurfaceSystem, RoadSurfaceVisualNodePieceKind};

mod bands;
mod build;
mod caps_and_joins;
mod constraints;
mod contacts;
mod contours;
mod geometry;
mod model;
mod owners;
mod source_points;
mod topology;

pub(crate) use model::{
    NodeGeneratedContour, NodeGeneratedContourClaimPriority, NodeGeneratedContourKind,
    NodeGeneratedContourPurpose, NodeRailBuildProfile, NodeRailConstraint, NodeRailConstraintKind,
    NodeRailContourSet, NodeRailGenerationError, NodeRailHeightCarrierPaths,
};

const RAIL_CONTOUR_POINT_EQUAL_EPS_M: f64 = SURFACE_POLYLINE_POINT_EQUAL_EPS_M;

#[cfg(test)]
mod tests;
