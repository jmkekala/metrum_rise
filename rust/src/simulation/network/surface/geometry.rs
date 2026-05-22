//! Low-level polygon, triangle, and section-boundary geometry helpers.

use super::backend::RoadVec3;
use super::{
    NODE_OVERLAY_MIN_AREA_M2, RoadSurfaceSection, RoadSurfaceSystem, RoadSurfaceVisualPolygon,
    SAMPLE_EPSILON_M, SurfaceCdt, WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2,
};
use godot::prelude::{Vector2, Vector3};
use spade::{Point2, Triangulation};

// Reject triangles that are area-positive but too skinny for stable height interpolation.
const SURFACE_MIN_TRIANGLE_DOUBLE_AREA_M2: f32 = 1.0e-8;
const SURFACE_MIN_TRIANGLE_ALTITUDE_M: f32 = 0.01;

mod loops;
mod predicates;
mod sections;
#[cfg(test)]
mod tests;
mod triangulation;
