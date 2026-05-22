//! Earthwork skirt geometry, transition vectors, and top-surface intrusion checks.

use super::super::{
    NodeOverlayContour, NodeOverlayShapes, RoadSurfaceSystem, RoadSurfaceVisualPolygon,
    SAMPLE_EPSILON_M, backend,
};
use super::{
    EARTHWORK_CUT_SLOPE_RATE, EARTHWORK_FILL_SLOPE_RATE, EARTHWORK_MARGIN_SAMPLE_STEP_M,
    EARTHWORK_MAX_MARGIN_M, EARTHWORK_MIN_MARGIN_M, EARTHWORK_RETAINING_WALL_SLOPE_THRESHOLD,
    RoadSurfaceEarthworkBoundarySegment, RoadSurfaceEarthworkFaceKind,
    RoadSurfaceEarthworkGeometryError, RoadSurfaceEarthworkRenderFace,
};
use crate::config;
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::{Vector2, Vector3};
use i_overlay::core::overlay_rule::OverlayRule;

mod build;
mod faces;
mod intrusion;
mod orientation;
#[cfg(test)]
mod tests;
mod transitions;
