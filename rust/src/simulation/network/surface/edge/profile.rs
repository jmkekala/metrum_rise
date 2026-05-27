//! Lateral roadbed profile bands and section boundary reconstruction.

use super::super::{RoadSurfaceBand, RoadSurfaceBandKind, RoadSurfaceSection, RoadSurfaceSystem};
use crate::config;
use crate::simulation::network::graph::Edge;
use crate::simulation::network::graph::rebuild::JunctionEndpointProfilePlane;
use crate::simulation::network::types::{TransitFlags, TransitType};
use godot::prelude::{Vector2, Vector3};

// Standard roadbed lateral shaping.
const CURB_BAND_WIDTH_M: f32 = 0.15;
const SHOULDER_BAND_WIDTH_M: f32 = 0.75;
pub(in crate::simulation::network::surface::edge) const CURB_STEP_HEIGHT_M: f32 = 0.12;

#[derive(Clone, Copy)]
struct EdgeLateralBandSpec {
    kind: RoadSurfaceBandKind,
    lateral_start_m: f32,
    lateral_end_m: f32,
    height_offset_start_m: f32,
    height_offset_end_m: f32,
}

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

        let mut bands = Vec::new();
        Self::for_each_lateral_band_spec(edge, |spec| {
            bands.push(RoadSurfaceBand {
                kind: spec.kind,
                lateral_start_m: spec.lateral_start_m,
                lateral_end_m: spec.lateral_end_m,
                height_start_m: boundary_height_m(spec.lateral_start_m, spec.height_offset_start_m),
                height_end_m: boundary_height_m(spec.lateral_end_m, spec.height_offset_end_m),
            });
        });
        bands
    }

    pub(in crate::simulation::network::surface) fn visual_profile_half_widths_for_edge(
        edge: &Edge,
    ) -> (f32, f32) {
        let mut roadbed_half_width_m = 0.0_f32;
        let mut carriageway_half_width_m = 0.0_f32;
        Self::for_each_lateral_band_spec(edge, |spec| {
            let band_half_width_m = spec.lateral_start_m.abs().max(spec.lateral_end_m.abs());
            roadbed_half_width_m = roadbed_half_width_m.max(band_half_width_m);
            if spec.kind == RoadSurfaceBandKind::Carriageway {
                carriageway_half_width_m = carriageway_half_width_m.max(band_half_width_m);
            }
        });
        (roadbed_half_width_m, carriageway_half_width_m)
    }

    fn for_each_lateral_band_spec(edge: &Edge, mut emit: impl FnMut(EdgeLateralBandSpec)) {
        if edge.primary_type == TransitType::Foot || (edge.allowed_types & TransitFlags::CAR) == 0 {
            let half_width = edge.width.max(2.0) * 0.5;
            emit(EdgeLateralBandSpec {
                kind: RoadSurfaceBandKind::Footpath,
                lateral_start_m: -half_width,
                lateral_end_m: half_width,
                height_offset_start_m: 0.0,
                height_offset_end_m: 0.0,
            });
            return;
        }

        let half_carriageway = edge.width.max(config::LANE_WIDTH) * 0.5;
        let has_sidewalk = edge.allowed_types & TransitFlags::FOOT != 0;
        let sidewalk_total = if has_sidewalk {
            config::SIDEWALK_WIDTH
        } else {
            0.0
        };
        let curb_width = if has_sidewalk {
            CURB_BAND_WIDTH_M.min(sidewalk_total)
        } else {
            SHOULDER_BAND_WIDTH_M
        };
        let sidewalk_width = (sidewalk_total - curb_width).max(0.0);
        let raised_offset_m = if curb_width > 0.0 {
            CURB_STEP_HEIGHT_M
        } else {
            0.0
        };
        if sidewalk_width > 0.0 {
            emit(EdgeLateralBandSpec {
                kind: RoadSurfaceBandKind::Sidewalk,
                lateral_start_m: -(half_carriageway + curb_width + sidewalk_width),
                lateral_end_m: -(half_carriageway + curb_width),
                height_offset_start_m: raised_offset_m,
                height_offset_end_m: raised_offset_m,
            });
        }

        emit(EdgeLateralBandSpec {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
            lateral_start_m: -(half_carriageway + curb_width),
            lateral_end_m: -half_carriageway,
            height_offset_start_m: raised_offset_m,
            height_offset_end_m: raised_offset_m,
        });
        emit(EdgeLateralBandSpec {
            kind: RoadSurfaceBandKind::Carriageway,
            lateral_start_m: -half_carriageway,
            lateral_end_m: 0.0,
            height_offset_start_m: 0.0,
            height_offset_end_m: 0.0,
        });
        emit(EdgeLateralBandSpec {
            kind: RoadSurfaceBandKind::Carriageway,
            lateral_start_m: 0.0,
            lateral_end_m: half_carriageway,
            height_offset_start_m: 0.0,
            height_offset_end_m: 0.0,
        });
        emit(EdgeLateralBandSpec {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
            lateral_start_m: half_carriageway,
            lateral_end_m: half_carriageway + curb_width,
            height_offset_start_m: raised_offset_m,
            height_offset_end_m: raised_offset_m,
        });

        if sidewalk_width > 0.0 {
            emit(EdgeLateralBandSpec {
                kind: RoadSurfaceBandKind::Sidewalk,
                lateral_start_m: half_carriageway + curb_width,
                lateral_end_m: half_carriageway + curb_width + sidewalk_width,
                height_offset_start_m: raised_offset_m,
                height_offset_end_m: raised_offset_m,
            });
        }
    }

    pub(in crate::simulation::network::surface) fn section_profile_world_points(
        &self,
        section: &RoadSurfaceSection,
    ) -> Vec<Vector3> {
        let Some(first_band) = section.bands.first() else {
            return Vec::new();
        };

        let mut points = Vec::with_capacity(section.bands.len() + 1);
        let first_point = self.section_boundary_world_point(
            section,
            first_band.lateral_start_m,
            first_band.height_start_m,
        );
        points.push(first_point);

        for band in &section.bands {
            let point =
                self.section_boundary_world_point(section, band.lateral_end_m, band.height_end_m);
            points.push(point);
        }

        points
    }
}
