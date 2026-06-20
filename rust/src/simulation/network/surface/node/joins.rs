//! Contour-adapter boundary for node side-join ownership candidates.

use super::backend::{
    RoadPolylineVertex, RoadVec2, RoadVec3, polyline_to_road_points,
    quantize_road_vec3_path_xz_to_overlay_grid, road_vec3_xz as xz_from_road_vec3,
};
use super::input::{NodeArrangementInput, NodeInputMouth};
use super::keys::{SurfaceHeightMmKey, SurfaceXzKey};
use super::paths::{
    PathHeightResolutionError, cleaned_open_road_polyline, cleaned_open_world_path_polyline,
    closed_world_contour_has_area, reheight_road_points_from_world_path,
    remove_repeated_road_vec3_xz_points,
};
use super::{NODE_OVERLAY_MIN_AREA_M2, RoadSurfaceBandKind, RoadSurfaceVisualNodePieceKind};
use cavalier_contours::core::math::{
    LineLineIntr, Vector2 as CavalierVec2, bulge_from_angle, line_line_intr,
};
use cavalier_contours::polyline::{PlineSource, seg_midpoint, seg_split_at_point};
use std::f64::consts::{PI, TAU};

#[derive(Clone, Copy)]
enum SideJoinProfileSide {
    Start,
    End,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum SideJoinPathMode {
    BendArc,
    JunctionNonRoad,
}

const SIDE_JOIN_ARC_RADIUS_EPS_M: f64 = 0.001;
const SIDE_JOIN_ARC_SPLIT_DEPTH: usize = 3;
const SIDE_JOIN_FILLET_MIN_TANGENT_M: f64 = 0.25;
const SIDE_JOIN_FILLET_TANGENT_FRACTION: f64 = 0.25;
const SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M: f64 = 1.0e-6;
const SIDE_JOIN_ENDPOINT_PLANE_HEIGHT_DUST_MM: i64 = 1;

mod generation;
mod heights;
mod paths;

#[cfg(test)]
use generation::side_join_band_has_quantized_area;

#[derive(Clone, Copy)]
struct SideJoinLayer {
    band_index: usize,
    band_kind: RoadSurfaceBandKind,
    inner_boundary_index: usize,
    outer_boundary_index: usize,
}

#[derive(Clone, Copy)]
struct SideJoinHeightPlane {
    origin: RoadVec3,
    grade_x: f64,
    grade_z: f64,
}

#[derive(Clone, Debug, PartialEq)]
struct SideJoinBoundaryPath {
    rounded_world: Vec<RoadVec3>,
    miter_world: Vec<RoadVec3>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeInputSideJoinBand {
    pub(crate) source_band_index: usize,
    pub(crate) band_kind: RoadSurfaceBandKind,
    pub(crate) boundary_mode: NodeInputSideJoinBandBoundaryMode,
    pub(crate) inner_path_world: Vec<RoadVec3>,
    pub(crate) outer_path_world: Vec<RoadVec3>,
    pub(crate) outer_footprint_trim_world: Vec<RoadVec3>,
    pub(crate) trims_outer_footprint: bool,
    pub(crate) contour_world: Vec<RoadVec3>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NodeInputSideJoinBandBoundaryMode {
    MaterialBand,
    MaterialBandWithSameOwnerOuterCap,
    SameOwnerOuterCap,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SideJoinGenerationError {
    pub(crate) reason: &'static str,
    pub(crate) point_x_key: i64,
    pub(crate) point_z_key: i64,
    pub(crate) existing_height_mm: i64,
    pub(crate) incoming_height_mm: i64,
}

impl SideJoinGenerationError {
    pub(crate) fn from_path_height_error(error: PathHeightResolutionError) -> Self {
        Self {
            reason: error.diagnostic_reason(),
            point_x_key: error.point_x_key,
            point_z_key: error.point_z_key,
            existing_height_mm: error.existing_height_mm,
            incoming_height_mm: error.incoming_height_mm,
        }
    }
}

pub(crate) fn side_join_bands_by_mouth(
    input: &NodeArrangementInput,
) -> Result<Vec<Vec<NodeInputSideJoinBand>>, SideJoinGenerationError> {
    let mut bands_by_mouth = vec![Vec::new(); input.mouths.len()];
    match input.piece_kind {
        RoadSurfaceVisualNodePieceKind::Bend => {
            generation::add_bend_side_join_bands(&input.mouths, &mut bands_by_mouth)?;
        }
        RoadSurfaceVisualNodePieceKind::JunctionN => {
            generation::add_junction_side_join_bands(&input.mouths, &mut bands_by_mouth)?;
        }
        RoadSurfaceVisualNodePieceKind::Terminal => {}
    }
    Ok(bands_by_mouth)
}

#[cfg(test)]
mod tests {
    use super::super::{
        IncidentEdgeSide, IncidentMouthBand, IncidentMouthProfile, OrderedIncidentPieceMouth,
    };
    use super::*;
    use crate::simulation::network::surface::backend::{RoadVec2, RoadVec3};

    fn band(kind: RoadSurfaceBandKind, start: RoadVec3, end: RoadVec3) -> IncidentMouthBand {
        IncidentMouthBand {
            kind,
            start_point_world: start,
            end_point_world: end,
        }
    }

    fn profile_x(x: f64, inward_direction_xz: RoadVec2) -> IncidentMouthProfile {
        let boundary_points_world = vec![
            RoadVec3::new(x, 4.0, -4.0),
            RoadVec3::new(x, 4.1, -3.0),
            RoadVec3::new(x, 4.2, -1.0),
            RoadVec3::new(x, 4.0, 0.0),
            RoadVec3::new(x, 4.2, 1.0),
            RoadVec3::new(x, 4.1, 3.0),
            RoadVec3::new(x, 4.0, 4.0),
        ];
        let bands = symmetric_road_bands(&boundary_points_world);
        IncidentMouthProfile {
            inward_direction_xz,
            boundary_points_world,
            bands,
        }
    }

    fn profile_z(z: f64, inward_direction_xz: RoadVec2) -> IncidentMouthProfile {
        let boundary_points_world = vec![
            RoadVec3::new(4.0, 4.0, z),
            RoadVec3::new(3.0, 4.1, z),
            RoadVec3::new(1.0, 4.2, z),
            RoadVec3::new(0.0, 4.0, z),
            RoadVec3::new(-1.0, 4.2, z),
            RoadVec3::new(-3.0, 4.1, z),
            RoadVec3::new(-4.0, 4.0, z),
        ];
        let bands = symmetric_road_bands(&boundary_points_world);
        IncidentMouthProfile {
            inward_direction_xz,
            boundary_points_world,
            bands,
        }
    }

    fn symmetric_road_bands(boundary_points_world: &[RoadVec3]) -> Vec<IncidentMouthBand> {
        vec![
            band(
                RoadSurfaceBandKind::Sidewalk,
                boundary_points_world[0],
                boundary_points_world[1],
            ),
            band(
                RoadSurfaceBandKind::CurbOrShoulder,
                boundary_points_world[1],
                boundary_points_world[2],
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
                boundary_points_world[4],
                boundary_points_world[5],
            ),
            band(
                RoadSurfaceBandKind::Sidewalk,
                boundary_points_world[5],
                boundary_points_world[6],
            ),
        ]
    }

    fn ordered_mouth(
        profile: IncidentMouthProfile,
        endpoint_profile: IncidentMouthProfile,
        direction_angle_ccw: f32,
        direction_xz: RoadVec2,
        edge_idx: usize,
    ) -> OrderedIncidentPieceMouth {
        OrderedIncidentPieceMouth {
            profile,
            endpoint_profile,
            boundary_paths_world: Vec::new(),
            band_start_paths_world: Vec::new(),
            band_end_paths_world: Vec::new(),
            uses_explicit_band_domain_paths: false,
            direction_angle_ccw,
            direction_xz,
            edge_idx,
            side: IncidentEdgeSide::Start,
        }
    }

    fn junction_input() -> NodeArrangementInput {
        let mouths = [
            ordered_mouth(
                profile_x(10.0, RoadVec2::X),
                profile_x(0.0, RoadVec2::X),
                0.0,
                RoadVec2::X,
                1,
            ),
            ordered_mouth(
                profile_z(12.0, RoadVec2::Y),
                profile_z(2.0, RoadVec2::Y),
                std::f32::consts::FRAC_PI_2,
                RoadVec2::Y,
                2,
            ),
            ordered_mouth(
                profile_x(-10.0, RoadVec2::NEG_X),
                profile_x(0.0, RoadVec2::NEG_X),
                std::f32::consts::PI,
                RoadVec2::NEG_X,
                3,
            ),
        ];
        NodeArrangementInput::from_ordered_mouths(
            42,
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &mouths,
        )
        .expect("test junction mouths should produce canonical input")
    }

    fn junction_input_with_shared_endpoint_center() -> NodeArrangementInput {
        let mouths = [
            ordered_mouth(
                profile_x(10.0, RoadVec2::X),
                profile_x(0.0, RoadVec2::X),
                0.0,
                RoadVec2::X,
                1,
            ),
            ordered_mouth(
                profile_z(12.0, RoadVec2::Y),
                profile_z(0.0, RoadVec2::Y),
                std::f32::consts::FRAC_PI_2,
                RoadVec2::Y,
                2,
            ),
            ordered_mouth(
                profile_x(-10.0, RoadVec2::NEG_X),
                profile_x(0.0, RoadVec2::NEG_X),
                std::f32::consts::PI,
                RoadVec2::NEG_X,
                3,
            ),
        ];
        NodeArrangementInput::from_ordered_mouths(
            42,
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &mouths,
        )
        .expect("test junction mouths should produce canonical input")
    }

    fn bend_input() -> NodeArrangementInput {
        let mouths = [
            ordered_mouth(
                profile_x(10.0, RoadVec2::X),
                profile_x(0.0, RoadVec2::X),
                0.0,
                RoadVec2::X,
                1,
            ),
            ordered_mouth(
                profile_z(12.0, RoadVec2::Y),
                profile_z(2.0, RoadVec2::Y),
                std::f32::consts::FRAC_PI_2,
                RoadVec2::Y,
                2,
            ),
        ];
        NodeArrangementInput::from_ordered_mouths(42, RoadSurfaceVisualNodePieceKind::Bend, &mouths)
            .expect("test bend mouths should produce canonical input")
    }

    fn bend_input_with_shared_endpoint_center() -> NodeArrangementInput {
        let mouths = [
            ordered_mouth(
                profile_x(10.0, RoadVec2::X),
                profile_x(0.0, RoadVec2::X),
                0.0,
                RoadVec2::X,
                1,
            ),
            ordered_mouth(
                profile_z(12.0, RoadVec2::Y),
                profile_z(0.0, RoadVec2::Y),
                std::f32::consts::FRAC_PI_2,
                RoadVec2::Y,
                2,
            ),
        ];
        NodeArrangementInput::from_ordered_mouths(42, RoadSurfaceVisualNodePieceKind::Bend, &mouths)
            .expect("test bend mouths should produce canonical input")
    }

    #[test]
    fn junction_side_join_bands_are_backend_cleaned_surface_carriers() {
        let bands_by_mouth = side_join_bands_by_mouth(&junction_input())
            .expect("test junction side joins should not have height conflicts");
        let bands = bands_by_mouth.iter().flatten().collect::<Vec<_>>();

        assert!(
            !bands.is_empty(),
            "junction side-join adapter should emit ownership carriers"
        );
        assert!(
            bands
                .iter()
                .any(|band| band.band_kind == RoadSurfaceBandKind::Carriageway),
            "JunctionN side joins should fill visible asphalt between adjacent mouths"
        );
        assert!(
            bands
                .iter()
                .any(|band| band.band_kind != RoadSurfaceBandKind::Carriageway),
            "JunctionN side joins should keep non-road ownership carriers"
        );
        assert!(
            bands
                .iter()
                .any(|band| band.inner_path_world.len() >= 3 || band.outer_path_world.len() >= 3),
            "at least one adjacent-mouth join should keep its backend join point"
        );
        for band in bands {
            assert!(side_join_band_has_quantized_area(band));
        }
    }

    #[test]
    fn bend_side_join_bands_continue_after_degenerate_carriageway_slice() {
        let bands_by_mouth = side_join_bands_by_mouth(&bend_input())
            .expect("test bend side joins should not have height conflicts");

        assert_rounded_non_road_gap(&bands_by_mouth[0]);
        assert_rounded_non_road_gap(&bands_by_mouth[1]);
    }

    #[test]
    fn junction_side_join_bands_continue_after_degenerate_carriageway_slice() {
        let bands_by_mouth = side_join_bands_by_mouth(&junction_input())
            .expect("test junction side joins should not have height conflicts");

        let bands = bands_by_mouth.iter().flatten().collect::<Vec<_>>();
        assert_rounded_non_road_gap_iter(bands.iter().copied());
    }

    #[test]
    fn bend_side_join_bands_round_shared_endpoint_centerline() {
        let input = bend_input_with_shared_endpoint_center();
        let bands_by_mouth = side_join_bands_by_mouth(&input)
            .expect("test bend side joins should not have height conflicts");
        let graph_center = RoadVec3::new(0.0, 4.0, 0.0);

        assert_no_visible_side_join_path_uses_point(&bands_by_mouth, graph_center);
        assert_rounded_non_road_gap(&bands_by_mouth[0]);
        assert_rounded_non_road_gap(&bands_by_mouth[1]);
    }

    #[test]
    fn junction_side_join_bands_round_shared_endpoint_centerline() {
        let input = junction_input_with_shared_endpoint_center();
        let bands_by_mouth = side_join_bands_by_mouth(&input)
            .expect("test junction side joins should not have height conflicts");
        let graph_center = RoadVec3::new(0.0, 4.0, 0.0);
        let bands = bands_by_mouth.iter().flatten().collect::<Vec<_>>();

        assert_no_visible_side_join_path_uses_point(&bands_by_mouth, graph_center);
        assert_rounded_non_road_gap_iter(bands.iter().copied());
    }

    fn assert_rounded_non_road_gap(bands: &[NodeInputSideJoinBand]) {
        assert_rounded_non_road_gap_iter(bands.iter());
    }

    fn assert_rounded_non_road_gap_iter<'a>(
        bands: impl IntoIterator<Item = &'a NodeInputSideJoinBand>,
    ) {
        let bands = bands.into_iter().collect::<Vec<_>>();
        assert!(
            bands.iter().any(|band| {
                band.band_kind == RoadSurfaceBandKind::CurbOrShoulder
                    && band.inner_path_world.len() > 3
            }),
            "adjacent gap must keep a rounded asphalt-to-curb boundary: {bands:?}"
        );
        assert!(
            bands.iter().any(|band| {
                band.band_kind == RoadSurfaceBandKind::Sidewalk && band.outer_path_world.len() > 3
            }),
            "adjacent gap must keep a rounded sidewalk-to-terrain boundary: {bands:?}"
        );
    }

    fn assert_no_visible_side_join_path_uses_point(
        bands_by_mouth: &[Vec<NodeInputSideJoinBand>],
        forbidden: RoadVec3,
    ) {
        let forbidden_key = SurfaceXzKey::from_road_xz(xz_from_road_vec3(forbidden));
        for band in bands_by_mouth.iter().flatten() {
            if band.band_kind == RoadSurfaceBandKind::Carriageway {
                continue;
            }
            assert!(
                band.inner_path_world
                    .iter()
                    .chain(&band.outer_path_world)
                    .all(
                        |point| SurfaceXzKey::from_road_xz(xz_from_road_vec3(*point))
                            != forbidden_key
                    ),
                "rounded Bend/JunctionN curb/sidewalk side joins must not route visible paths through the shared graph endpoint: {band:?}"
            );
        }
    }
}
