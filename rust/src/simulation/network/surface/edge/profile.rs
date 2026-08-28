// ========================================================================
//  MANIFEST
// ========================================================================
//  script_name: profile.rs
//  script_path: rust/src/simulation/network/surface/edge/profile.rs
//  module_name: profile
//  version: 0.1.0
//  description: Lateral roadbed profile bands and section boundary
//           reconstruction.
//  kind: module
//  spec: none
//  internal_dependencies: []
//  external_dependencies: []
//  features: []
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-27
// ========================================================================

//! Lateral roadbed profile bands and section boundary reconstruction.

use super::super::backend::{RoadVec2, RoadVec3};
use super::super::{RoadSurfaceBand, RoadSurfaceBandKind, RoadSurfaceSection, RoadSurfaceSystem};
use crate::config;
use crate::simulation::network::graph::Edge;
use crate::simulation::network::graph::rebuild::JunctionEndpointProfilePlane;
use crate::simulation::network::graph::{LaneKind, LaneSpec};
use crate::simulation::network::types::{TransitFlags, TransitType};

// ========================================================================
// BAND HEIGHT
// ========================================================================

// Standard roadbed lateral shaping.
/// How far a band stands above the carriageway.
///
/// A built median is kerbed and planted, so it sits at kerb height and a
/// vehicle cannot cross it. A painted median is a line on the road surface and
/// sits flush, which is the whole difference between the two. Every other band
/// is level with the carriageway it belongs to.
fn median_lift_m(lane: &LaneSpec) -> f32 {
    match lane.kind {
        // A built median is curbed and planted; a painted one is a line on the
        // road and sits flush. That is the whole difference between them.
        LaneKind::Median if lane.blocks_turns_across() => CURB_STEP_HEIGHT_M,
        // A verge is always planted ground, so it always stands proud.
        LaneKind::Verge => CURB_STEP_HEIGHT_M,
        _ => 0.0,
    }
}

// ========================================================================
// BAND WIDTHS
// ========================================================================

const CURB_BAND_WIDTH_M: f32 = 0.15;
const SHOULDER_BAND_WIDTH_M: f32 = 0.75;
pub(in crate::simulation::network::surface::edge) const CURB_STEP_HEIGHT_M: f32 = 0.12;

#[derive(Clone, Copy)]
pub(in crate::simulation::network::surface::edge) struct EdgeProfilePlaneBlend {
    plane: JunctionEndpointProfilePlane,
    pub(in crate::simulation::network::surface::edge) weight: f32,
}

// ========================================================================
// BLENDING INTO A JUNCTION
// ========================================================================

impl EdgeProfilePlaneBlend {
    pub(in crate::simulation::network::surface::edge) fn new(
        plane: JunctionEndpointProfilePlane,
        weight: f32,
    ) -> Option<Self> {
        (weight > 0.0).then_some(Self {
            plane,
            weight: weight.clamp(0.0, 1.0),
        })
    }

    pub(in crate::simulation::network::surface::edge) fn height_at_xz(
        &self,
        x: f32,
        z: f32,
        fallback_height_m: f32,
    ) -> f32 {
        let plane_height_m = self.plane.height_at_xz(x, z);
        fallback_height_m * (1.0 - self.weight) + plane_height_m * self.weight
    }
}

#[derive(Clone, Copy)]
struct EdgeLateralBandSpec {
    kind: RoadSurfaceBandKind,
    lateral_start_m: f32,
    lateral_end_m: f32,
    height_offset_start_m: f32,
    height_offset_end_m: f32,
}

// ========================================================================
// BUILDING THE BANDS
// ========================================================================

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface::edge) fn build_lateral_bands(
        &self,
        edge: &Edge,
        center: RoadVec3,
        lateral_xz: RoadVec2,
        profile_blend: Option<EdgeProfilePlaneBlend>,
        carriageway_half_width_override_m: Option<f32>,
    ) -> Vec<RoadSurfaceBand> {
        let boundary_height_m = |lateral_m: f32, offset_m: f32| {
            let flat_height_m = center.y as f32;
            let base_height_m = profile_blend.map_or(flat_height_m, |blend| {
                blend.height_at_xz(
                    (center.x + lateral_xz.x * f64::from(lateral_m)) as f32,
                    (center.z + lateral_xz.y * f64::from(lateral_m)) as f32,
                    flat_height_m,
                )
            });
            base_height_m + offset_m
        };

        let mut bands = Vec::new();
        Self::for_each_lateral_band_spec(edge, carriageway_half_width_override_m, |spec| {
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
        Self::visual_profile_half_widths_for_edge_with_carriageway_override(edge, None)
    }

    pub(in crate::simulation::network::surface::edge) fn visual_profile_half_widths_for_edge_with_carriageway_override(
        edge: &Edge,
        carriageway_half_width_override_m: Option<f32>,
    ) -> (f32, f32) {
        let mut roadbed_half_width_m = 0.0_f32;
        let mut carriageway_half_width_m = 0.0_f32;
        Self::for_each_lateral_band_spec(edge, carriageway_half_width_override_m, |spec| {
            let band_half_width_m = spec.lateral_start_m.abs().max(spec.lateral_end_m.abs());
            roadbed_half_width_m = roadbed_half_width_m.max(band_half_width_m);
            if spec.kind == RoadSurfaceBandKind::Carriageway {
                carriageway_half_width_m = carriageway_half_width_m.max(band_half_width_m);
            }
        });
        (roadbed_half_width_m, carriageway_half_width_m)
    }

    fn for_each_lateral_band_spec(
        edge: &Edge,
        carriageway_half_width_override_m: Option<f32>,
        mut emit: impl FnMut(EdgeLateralBandSpec),
    ) {
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

        let half_carriageway = carriageway_half_width_override_m
            .unwrap_or_else(|| edge.width.max(config::LANE_WIDTH) * 0.5)
            .max(config::LANE_WIDTH * 0.5);
        let has_sidewalk = edge.allowed_types & TransitFlags::FOOT != 0;
        // Authored per layout, falling back to the project default, so a high
        // street and a residential lane are not forced to the same width.
        let sidewalk_total = if has_sidewalk {
            edge.lane_layout().sidewalk_width()
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
        // The carriageway is the ordered bands of the lane layout, not one
        // slab split at the centreline. A median, a parking lane, and a cycle
        // track are each their own band with their own surface, which is what
        // `roads.md` asks for: explicit ordered bands rather than special-case
        // render offsets.
        //
        // Bands must tile the carriageway left to right with no gap, because
        // the section profile walks them in order and takes each band's end as
        // the next band's start.
        let layout = edge.lane_layout();
        let emitted_from_layout = if carriageway_half_width_override_m.is_none()
            && !layout.is_empty()
            && layout.asphalt_width() > 0.0
        {
            let scale = (half_carriageway * 2.0) / layout.asphalt_width();
            let mut cursor = -half_carriageway;
            for lane in layout.lanes() {
                let end = cursor + lane.width_m * scale;
                emit(EdgeLateralBandSpec {
                    kind: match lane.kind {
                        LaneKind::Median => RoadSurfaceBandKind::Median,
                        LaneKind::Parking => RoadSurfaceBandKind::Parking,
                        LaneKind::CycleTrack => RoadSurfaceBandKind::CycleTrack,
                        LaneKind::Verge => RoadSurfaceBandKind::Verge,
                        // A reversible lane is ordinary asphalt. What makes it
                        // reversible is its markings and who may enter it,
                        // not the surface it is built from.
                        LaneKind::Travel | LaneKind::Shoulder | LaneKind::Reversible => {
                            RoadSurfaceBandKind::Carriageway
                        }
                    },
                    lateral_start_m: cursor,
                    lateral_end_m: end,
                    // A built median stands proud of the road at kerb height;
                    // a painted one is flush, and so is everything else.
                    height_offset_start_m: median_lift_m(lane),
                    height_offset_end_m: median_lift_m(lane),
                });
                cursor = end;
            }
            true
        } else {
            false
        };

        if !emitted_from_layout {
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
        }
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
    ) -> Vec<RoadVec3> {
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
