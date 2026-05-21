//! Canonical terminal-cap adapter for one-mouth visual node ownership.

use super::backend::{
    RoadPolyline, RoadVec2, RoadVec3, polyline_to_road_points,
    quantize_road_vec3_path_xz_to_overlay_grid, road_points_to_polyline, road_vec3_xz as xz,
};
use super::input::{NodeArrangementInput, NodeInputMouth};
use super::paths::{
    PathHeightResolutionError, cleaned_open_world_path_polyline, closed_world_contour_has_area,
    reheight_road_points_from_world_path,
};
use super::{NODE_OVERLAY_MIN_AREA_M2, RoadSurfaceBandKind, RoadSurfaceVisualNodePieceKind};
use cavalier_contours::polyline::{PlineCreation, PlineSource};

const TERMINAL_CAP_POLYLINE_POINT_EQUAL_EPS_M: f64 = 1.0e-6;
const TERMINAL_CAP_WIDTH_EPS_M: f64 = 0.001;

mod generation;
mod height_anchors;
mod paths;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalCapBandRole {
    LeftSide,
    LeftCorner,
    EndBand,
    RightCorner,
    RightSide,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerminalCapBandProvenance {
    pub(crate) layer_index: usize,
    pub(crate) role: TerminalCapBandRole,
    pub(crate) left_source_band_index: usize,
    pub(crate) right_source_band_index: usize,
    pub(crate) source_boundary_start_index: usize,
    pub(crate) source_boundary_end_index: usize,
    pub(crate) inner_offset_m: f64,
    pub(crate) outer_offset_m: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalCapFailureReason {
    MissingBoundaryRails,
    MissingOutwardDirection,
    MismatchedPairedBandKind,
    MismatchedPairedBandWidth,
    DegenerateBandWidth,
    DegeneratePath,
    DegenerateContour,
    InvalidCapArea,
    ConflictingPathHeight,
}

impl TerminalCapFailureReason {
    pub(crate) fn diagnostic_reason(self) -> &'static str {
        match self {
            Self::MissingBoundaryRails => "terminal_cap_missing_boundary_rails",
            Self::MissingOutwardDirection => "terminal_cap_missing_outward_direction",
            Self::MismatchedPairedBandKind => "terminal_cap_mismatched_paired_band_kind",
            Self::MismatchedPairedBandWidth => "terminal_cap_mismatched_paired_band_width",
            Self::DegenerateBandWidth => "terminal_cap_degenerate_band_width",
            Self::DegeneratePath => "terminal_cap_degenerate_path",
            Self::DegenerateContour => "terminal_cap_degenerate_contour",
            Self::InvalidCapArea => "terminal_cap_invalid_area",
            Self::ConflictingPathHeight => "terminal_cap_conflicting_path_height",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TerminalCapGenerationError {
    pub(crate) mouth_order_index: usize,
    pub(crate) layer_index: Option<usize>,
    pub(crate) source_band_index: Option<usize>,
    pub(crate) left_source_band_index: Option<usize>,
    pub(crate) right_source_band_index: Option<usize>,
    pub(crate) band_kind: Option<RoadSurfaceBandKind>,
    pub(crate) reason: TerminalCapFailureReason,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeTerminalCapBand {
    pub(crate) source_band_index: usize,
    pub(crate) band_kind: RoadSurfaceBandKind,
    pub(crate) provenance: TerminalCapBandProvenance,
    pub(crate) inner_path_world: Vec<RoadVec3>,
    pub(crate) outer_path_world: Vec<RoadVec3>,
    pub(crate) contour_world: Vec<RoadVec3>,
}

impl TerminalCapGenerationError {
    fn for_mouth(mouth: &NodeInputMouth, reason: TerminalCapFailureReason) -> Self {
        Self {
            mouth_order_index: mouth.order_index,
            layer_index: None,
            source_band_index: None,
            left_source_band_index: None,
            right_source_band_index: None,
            band_kind: None,
            reason,
        }
    }

    fn for_layer(
        mouth: &NodeInputMouth,
        layer_index: usize,
        source_band_index: usize,
        left_source_band_index: usize,
        right_source_band_index: usize,
        band_kind: RoadSurfaceBandKind,
        reason: TerminalCapFailureReason,
    ) -> Self {
        Self {
            mouth_order_index: mouth.order_index,
            layer_index: Some(layer_index),
            source_band_index: Some(source_band_index),
            left_source_band_index: Some(left_source_band_index),
            right_source_band_index: Some(right_source_band_index),
            band_kind: Some(band_kind),
            reason,
        }
    }

    fn for_cap(
        mouth: &NodeInputMouth,
        source_band_index: usize,
        band_kind: RoadSurfaceBandKind,
        provenance: TerminalCapBandProvenance,
        reason: TerminalCapFailureReason,
    ) -> Self {
        Self::for_layer(
            mouth,
            provenance.layer_index,
            source_band_index,
            provenance.left_source_band_index,
            provenance.right_source_band_index,
            band_kind,
            reason,
        )
    }
}

pub(crate) fn terminal_cap_bands_by_mouth(
    input: &NodeArrangementInput,
) -> Result<Vec<Vec<NodeTerminalCapBand>>, TerminalCapGenerationError> {
    let mut bands_by_mouth = vec![Vec::new(); input.mouths.len()];
    if input.piece_kind != RoadSurfaceVisualNodePieceKind::Terminal {
        return Ok(bands_by_mouth);
    }

    for (mouth_index, mouth) in input.mouths.iter().enumerate() {
        let mut bands = generation::terminal_cap_bands(mouth)?;
        generation::canonicalize_terminal_cap_bands(mouth, &mut bands)?;
        bands_by_mouth[mouth_index] = bands;
    }

    Ok(bands_by_mouth)
}

#[cfg(test)]
mod tests {
    use super::super::{
        IncidentEdgeSide, IncidentMouthBand, IncidentMouthProfile, OrderedIncidentPieceMouth,
    };
    use super::*;
    use godot::prelude::{Vector2, Vector3};

    fn band(kind: RoadSurfaceBandKind, start: Vector3, end: Vector3) -> IncidentMouthBand {
        IncidentMouthBand {
            kind,
            start_point_world: start,
            end_point_world: end,
        }
    }

    fn symmetric_profile_x(x: f32, inward_direction_xz: Vector2) -> IncidentMouthProfile {
        let boundary_points_world = vec![
            Vector3::new(x, 4.12, -5.0),
            Vector3::new(x, 4.12, -3.65),
            Vector3::new(x, 4.0, -3.5),
            Vector3::new(x, 4.0, 0.0),
            Vector3::new(x, 4.0, 3.5),
            Vector3::new(x, 4.12, 3.65),
            Vector3::new(x, 4.12, 5.0),
        ];
        let bands = vec![
            band(
                RoadSurfaceBandKind::Sidewalk,
                boundary_points_world[0],
                boundary_points_world[1],
            ),
            band(
                RoadSurfaceBandKind::CurbOrShoulder,
                boundary_points_world[1],
                Vector3::new(boundary_points_world[2].x, 4.12, boundary_points_world[2].z),
            ),
            band(
                RoadSurfaceBandKind::Carriageway,
                boundary_points_world[2],
                boundary_points_world[3],
            ),
            band(
                RoadSurfaceBandKind::Carriageway,
                boundary_points_world[3],
                boundary_points_world[4],
            ),
            band(
                RoadSurfaceBandKind::CurbOrShoulder,
                Vector3::new(boundary_points_world[4].x, 4.12, boundary_points_world[4].z),
                boundary_points_world[5],
            ),
            band(
                RoadSurfaceBandKind::Sidewalk,
                boundary_points_world[5],
                boundary_points_world[6],
            ),
        ];
        IncidentMouthProfile {
            inward_direction_xz,
            boundary_points_world,
            bands,
        }
    }

    fn car_only_profile_x(x: f32, inward_direction_xz: Vector2) -> IncidentMouthProfile {
        let boundary_points_world = vec![
            Vector3::new(x, 4.0, -3.5),
            Vector3::new(x, 4.0, 0.0),
            Vector3::new(x, 4.0, 3.5),
        ];
        let bands = vec![
            band(
                RoadSurfaceBandKind::Carriageway,
                boundary_points_world[0],
                boundary_points_world[1],
            ),
            band(
                RoadSurfaceBandKind::Carriageway,
                boundary_points_world[1],
                boundary_points_world[2],
            ),
        ];
        IncidentMouthProfile {
            inward_direction_xz,
            boundary_points_world,
            bands,
        }
    }

    fn asymmetric_sidewalk_profile_x(x: f32, inward_direction_xz: Vector2) -> IncidentMouthProfile {
        let mut profile = symmetric_profile_x(x, inward_direction_xz);
        profile.boundary_points_world[6] = Vector3::new(x, 4.12, 5.5);
        profile.bands[5].end_point_world = profile.boundary_points_world[6];
        profile
    }

    fn terminal_input(profile: IncidentMouthProfile) -> NodeArrangementInput {
        let endpoint_profile = profile.clone();
        let mouth_profile = IncidentMouthProfile {
            inward_direction_xz: profile.inward_direction_xz,
            boundary_points_world: profile
                .boundary_points_world
                .iter()
                .map(|point| Vector3::new(point.x + 10.0, point.y, point.z))
                .collect(),
            bands: profile
                .bands
                .iter()
                .map(|band| {
                    let start = band.start_point_world;
                    let end = band.end_point_world;
                    IncidentMouthBand {
                        kind: band.kind,
                        start_point_world: Vector3::new(start.x + 10.0, start.y, start.z),
                        end_point_world: Vector3::new(end.x + 10.0, end.y, end.z),
                    }
                })
                .collect(),
        };
        let mouth = OrderedIncidentPieceMouth {
            profile: mouth_profile,
            endpoint_profile,
            boundary_paths_world: Vec::new(),
            band_start_paths_world: Vec::new(),
            band_end_paths_world: Vec::new(),
            uses_explicit_band_domain_paths: false,
            direction_angle_ccw: 0.0,
            direction_xz: Vector2::RIGHT,
            edge_idx: 8,
            side: IncidentEdgeSide::Start,
        };

        NodeArrangementInput::from_ordered_mouths(
            42,
            RoadSurfaceVisualNodePieceKind::Terminal,
            &[mouth],
        )
        .expect("valid terminal profile should produce canonical input")
    }

    #[test]
    fn terminal_cap_adapter_uses_source_band_interval_heights() {
        let input = terminal_input(symmetric_profile_x(0.0, Vector2::RIGHT));
        let mouth = &input.mouths[0];
        let cap_bands_by_mouth =
            terminal_cap_bands_by_mouth(&input).expect("symmetric terminal cap is valid");
        let center_boundary = mouth.boundary_rails[3].endpoint_world;
        let expected_height_m = mouth.band_intervals[1]
            .endpoint_end_world
            .y
            .max(mouth.band_intervals[4].endpoint_start_world.y);

        assert!(cap_bands_by_mouth[0].iter().any(|cap_band| {
            cap_band.band_kind == RoadSurfaceBandKind::CurbOrShoulder
                && cap_band.inner_path_world.iter().any(|point| {
                    (point.x - center_boundary.x).abs() <= 0.001
                        && (point.z - center_boundary.z).abs() <= 0.001
                        && (point.y - expected_height_m).abs() <= 0.001
                })
        }));
    }

    #[test]
    fn terminal_cap_adapter_records_cap_source_provenance() {
        let input = terminal_input(symmetric_profile_x(0.0, Vector2::RIGHT));
        let mouth = &input.mouths[0];
        let cap_bands_by_mouth =
            terminal_cap_bands_by_mouth(&input).expect("symmetric terminal cap is valid");
        let first_terminal_source_band = mouth.band_intervals.len();
        let end_band = cap_bands_by_mouth[0]
            .iter()
            .find(|cap_band| {
                cap_band.source_band_index == first_terminal_source_band
                    && cap_band.provenance.role == TerminalCapBandRole::EndBand
            })
            .expect("curb terminal cap should include an endpoint span");

        assert_eq!(end_band.band_kind, RoadSurfaceBandKind::CurbOrShoulder);
        assert_eq!(end_band.provenance.layer_index, 0);
        assert_eq!(end_band.provenance.left_source_band_index, 1);
        assert_eq!(end_band.provenance.right_source_band_index, 4);
        assert_eq!(end_band.provenance.source_boundary_start_index, 2);
        assert_eq!(end_band.provenance.source_boundary_end_index, 4);
    }

    #[test]
    fn terminal_cap_adapter_emits_side_corner_closures_from_source_rails() {
        let input = terminal_input(symmetric_profile_x(0.0, Vector2::RIGHT));
        let mouth = &input.mouths[0];
        let cap_bands_by_mouth =
            terminal_cap_bands_by_mouth(&input).expect("symmetric terminal cap is valid");
        let sidewalk_terminal_source_band = mouth.band_intervals.len() + 1;
        let left_corner = cap_bands_by_mouth[0]
            .iter()
            .find(|cap_band| {
                cap_band.source_band_index == sidewalk_terminal_source_band
                    && cap_band.provenance.role == TerminalCapBandRole::LeftCorner
            })
            .expect("sidewalk terminal cap must close the left endpoint-to-cap corner");
        let right_corner = cap_bands_by_mouth[0]
            .iter()
            .find(|cap_band| {
                cap_band.source_band_index == sidewalk_terminal_source_band
                    && cap_band.provenance.role == TerminalCapBandRole::RightCorner
            })
            .expect("sidewalk terminal cap must close the right endpoint-to-cap corner");

        assert_eq!(left_corner.band_kind, RoadSurfaceBandKind::Sidewalk);
        assert_eq!(left_corner.provenance.source_boundary_start_index, 0);
        assert_eq!(left_corner.provenance.source_boundary_end_index, 1);
        assert!((left_corner.provenance.inner_offset_m - 0.0).abs() <= 0.001);
        assert!((left_corner.provenance.outer_offset_m - 0.15).abs() <= 0.001);
        assert_eq!(right_corner.band_kind, RoadSurfaceBandKind::Sidewalk);
        assert_eq!(right_corner.provenance.source_boundary_start_index, 5);
        assert_eq!(right_corner.provenance.source_boundary_end_index, 6);
        assert!((right_corner.provenance.inner_offset_m - 0.0).abs() <= 0.001);
        assert!((right_corner.provenance.outer_offset_m - 0.15).abs() <= 0.001);

        let left_endpoint_outer = mouth.boundary_rails[0].endpoint_world;
        assert!(left_corner.inner_path_world.iter().any(|point| {
            (point.x - left_endpoint_outer.x).abs() <= 0.001
                && (point.z - left_endpoint_outer.z).abs() <= 0.001
                && (point.y - left_endpoint_outer.y).abs() <= 0.001
        }));
        assert!(left_corner.outer_path_world.iter().any(|point| {
            (point.x - (left_endpoint_outer.x - 0.15)).abs() <= 0.001
                && (point.z - left_endpoint_outer.z).abs() <= 0.001
                && (point.y - left_endpoint_outer.y).abs() <= 0.001
        }));
    }

    #[test]
    fn terminal_cap_adapter_rejects_asymmetric_paired_band_widths() {
        let input = terminal_input(asymmetric_sidewalk_profile_x(0.0, Vector2::RIGHT));
        let error = terminal_cap_bands_by_mouth(&input)
            .expect_err("paired terminal caps must not silently truncate asymmetric widths");

        assert_eq!(
            error.reason,
            TerminalCapFailureReason::MismatchedPairedBandWidth
        );
        assert_eq!(error.layer_index, Some(1));
        assert_eq!(error.band_kind, Some(RoadSurfaceBandKind::Sidewalk));
    }

    #[test]
    fn car_only_terminal_emits_no_non_road_cap() {
        let input = terminal_input(car_only_profile_x(0.0, Vector2::RIGHT));
        let cap_bands_by_mouth =
            terminal_cap_bands_by_mouth(&input).expect("car-only terminal has no cap bands");

        assert!(cap_bands_by_mouth.iter().flatten().next().is_none());
    }
}
