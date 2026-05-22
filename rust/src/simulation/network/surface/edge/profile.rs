//! Lateral roadbed profile bands and section boundary reconstruction.

use super::super::{RoadSurfaceBand, RoadSurfaceBandKind, RoadSurfaceSection, RoadSurfaceSystem};
use crate::config;
use crate::simulation::network::graph::Edge;
use crate::simulation::network::graph::rebuild::JunctionEndpointProfilePlane;
use crate::simulation::network::types::{TransitFlags, TransitType};
use godot::prelude::{Vector2, Vector3};

// Standard roadbed lateral shaping.
const CURB_BAND_WIDTH_M: f32 = 0.15;
pub(in crate::simulation::network::surface::edge) const CURB_STEP_HEIGHT_M: f32 = 0.12;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface::edge) fn build_lateral_bands(
        &self,
        edge: &Edge,
        center: Vector3,
        lateral_xz: Vector2,
        profile_plane: Option<JunctionEndpointProfilePlane>,
    ) -> Vec<RoadSurfaceBand> {
        let boundary_height_m = |lateral_m: f32, offset_m: f32| {
            let base_height_m = profile_plane.map_or(center.y, |plane| {
                plane.height_at_xz(
                    center.x + lateral_xz.x * lateral_m,
                    center.z + lateral_xz.y * lateral_m,
                )
            });
            base_height_m + offset_m
        };
        if edge.primary_type == TransitType::Foot || (edge.allowed_types & TransitFlags::CAR) == 0 {
            let half_width = edge.width.max(2.0) * 0.5;
            return vec![RoadSurfaceBand {
                kind: RoadSurfaceBandKind::Footpath,
                lateral_start_m: -half_width,
                lateral_end_m: half_width,
                height_start_m: boundary_height_m(-half_width, 0.0),
                height_end_m: boundary_height_m(half_width, 0.0),
            }];
        }

        let half_carriageway = edge.width.max(config::LANE_WIDTH) * 0.5;
        let sidewalk_total = if edge.allowed_types & TransitFlags::FOOT != 0 {
            config::SIDEWALK_WIDTH
        } else {
            0.0
        };
        let curb_width = if sidewalk_total > 0.0 {
            CURB_BAND_WIDTH_M.min(sidewalk_total)
        } else {
            0.0
        };
        let sidewalk_width = (sidewalk_total - curb_width).max(0.0);
        let raised_offset_m = if curb_width > 0.0 {
            CURB_STEP_HEIGHT_M
        } else {
            0.0
        };
        let mut bands = Vec::new();
        if sidewalk_width > 0.0 {
            bands.push(RoadSurfaceBand {
                kind: RoadSurfaceBandKind::Sidewalk,
                lateral_start_m: -(half_carriageway + curb_width + sidewalk_width),
                lateral_end_m: -(half_carriageway + curb_width),
                height_start_m: boundary_height_m(
                    -(half_carriageway + curb_width + sidewalk_width),
                    raised_offset_m,
                ),
                height_end_m: boundary_height_m(-(half_carriageway + curb_width), raised_offset_m),
            });
        }

        bands.push(RoadSurfaceBand {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
            lateral_start_m: -(half_carriageway + curb_width),
            lateral_end_m: -half_carriageway,
            height_start_m: boundary_height_m(-(half_carriageway + curb_width), raised_offset_m),
            height_end_m: boundary_height_m(-half_carriageway, raised_offset_m),
        });
        bands.push(RoadSurfaceBand {
            kind: RoadSurfaceBandKind::Carriageway,
            lateral_start_m: -half_carriageway,
            lateral_end_m: 0.0,
            height_start_m: boundary_height_m(-half_carriageway, 0.0),
            height_end_m: boundary_height_m(0.0, 0.0),
        });
        bands.push(RoadSurfaceBand {
            kind: RoadSurfaceBandKind::Carriageway,
            lateral_start_m: 0.0,
            lateral_end_m: half_carriageway,
            height_start_m: boundary_height_m(0.0, 0.0),
            height_end_m: boundary_height_m(half_carriageway, 0.0),
        });
        bands.push(RoadSurfaceBand {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
            lateral_start_m: half_carriageway,
            lateral_end_m: half_carriageway + curb_width,
            height_start_m: boundary_height_m(half_carriageway, raised_offset_m),
            height_end_m: boundary_height_m(half_carriageway + curb_width, raised_offset_m),
        });

        if sidewalk_width > 0.0 {
            bands.push(RoadSurfaceBand {
                kind: RoadSurfaceBandKind::Sidewalk,
                lateral_start_m: half_carriageway + curb_width,
                lateral_end_m: half_carriageway + curb_width + sidewalk_width,
                height_start_m: boundary_height_m(half_carriageway + curb_width, raised_offset_m),
                height_end_m: boundary_height_m(
                    half_carriageway + curb_width + sidewalk_width,
                    raised_offset_m,
                ),
            });
        }

        bands
    }

    pub(in crate::simulation::network::surface) fn section_profile_world_points(
        &self,
        section: &RoadSurfaceSection,
        y_lift_m: f32,
    ) -> Vec<Vector3> {
        let Some(first_band) = section.bands.first() else {
            return Vec::new();
        };

        let mut points = Vec::with_capacity(section.bands.len() + 1);
        let mut first_point = self.section_boundary_world_point(
            section,
            first_band.lateral_start_m,
            first_band.height_start_m,
        );
        first_point.y += y_lift_m;
        points.push(first_point);

        for band in &section.bands {
            let mut point =
                self.section_boundary_world_point(section, band.lateral_end_m, band.height_end_m);
            point.y += y_lift_m;
            points.push(point);
        }

        points
    }
}
