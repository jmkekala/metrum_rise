//! Canonical XZ and height quantization keys shared by surface stages.

use super::NodeOverlayPoint;
use super::backend::{ROAD_OVERLAY_COORDINATE_SCALE, RoadVec2, RoadVec3};
use godot::prelude::Vector3;

pub(crate) const SURFACE_XZ_KEY_SCALE: f64 = ROAD_OVERLAY_COORDINATE_SCALE;
pub(crate) const SURFACE_MM_PER_M: f64 = 1000.0;
pub(crate) const SURFACE_CANONICAL_HEIGHT_EPS_M: f64 = 0.01;
pub(crate) const SURFACE_POLYLINE_POINT_EQUAL_EPS_M: f64 = 1.0e-6;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct SurfaceXzKey {
    x_key: i64,
    z_key: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct SurfaceHeightMmKey(i64);

impl SurfaceXzKey {
    pub(crate) fn from_raw_keys(x_key: i64, z_key: i64) -> Self {
        Self { x_key, z_key }
    }

    pub(crate) fn from_road_xz(point: RoadVec2) -> Self {
        Self {
            x_key: Self::coordinate_key(point.x),
            z_key: Self::coordinate_key(point.y),
        }
    }

    pub(crate) fn from_world_xz(point: RoadVec3) -> Self {
        Self {
            x_key: Self::coordinate_key(point.x),
            z_key: Self::coordinate_key(point.z),
        }
    }

    pub(crate) fn from_godot_world_xz(point: Vector3) -> Self {
        Self {
            x_key: Self::coordinate_key(f64::from(point.x)),
            z_key: Self::coordinate_key(f64::from(point.z)),
        }
    }

    pub(crate) fn from_overlay_point(point: NodeOverlayPoint) -> Self {
        Self {
            x_key: Self::coordinate_key(point[0]),
            z_key: Self::coordinate_key(point[1]),
        }
    }

    pub(crate) fn coordinate_key(value_m: f64) -> i64 {
        (value_m * SURFACE_XZ_KEY_SCALE).round() as i64
    }

    pub(crate) fn coordinate_key_to_mm(value: i64) -> i64 {
        ((value as f64 / SURFACE_XZ_KEY_SCALE) * SURFACE_MM_PER_M).round() as i64
    }

    pub(crate) fn x_key(self) -> i64 {
        self.x_key
    }

    pub(crate) fn z_key(self) -> i64 {
        self.z_key
    }

    pub(crate) fn x_mm(self) -> i64 {
        Self::coordinate_key_to_mm(self.x_key)
    }

    pub(crate) fn z_mm(self) -> i64 {
        Self::coordinate_key_to_mm(self.z_key)
    }

    pub(crate) fn to_road_xz(self) -> RoadVec2 {
        RoadVec2::new(
            self.x_key as f64 / SURFACE_XZ_KEY_SCALE,
            self.z_key as f64 / SURFACE_XZ_KEY_SCALE,
        )
    }
}

impl SurfaceHeightMmKey {
    pub(crate) fn from_m_f64(value_m: f64) -> Self {
        Self((value_m * SURFACE_MM_PER_M).round() as i64)
    }

    pub(crate) fn from_m_f32(value_m: f32) -> Self {
        Self::from_m_f64(f64::from(value_m))
    }

    pub(crate) fn as_i64(self) -> i64 {
        self.0
    }
}
