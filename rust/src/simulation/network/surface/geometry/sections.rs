// SPDX-License-Identifier: GPL-2.0-only

//! Section-boundary world coordinate projection.

use super::*;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn section_boundary_world_point(
        &self,
        section: &RoadSurfaceSection,
        lateral_offset_m: f32,
        height_m: f32,
    ) -> RoadVec3 {
        Self::section_boundary_world_point_static(section, lateral_offset_m, height_m)
    }

    pub(in crate::simulation::network::surface) fn section_boundary_world_point_static(
        section: &RoadSurfaceSection,
        lateral_offset_m: f32,
        height_m: f32,
    ) -> RoadVec3 {
        RoadVec3::new(
            section.center_xz.x + section.lateral_xz.x * f64::from(lateral_offset_m),
            f64::from(height_m),
            section.center_xz.y + section.lateral_xz.y * f64::from(lateral_offset_m),
        )
    }
}
