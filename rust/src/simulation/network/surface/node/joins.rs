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
const SIDE_JOIN_ARC_SPLIT_DEPTH: usize = 2;
const SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M: f64 = 1.0e-6;
const SIDE_JOIN_ENDPOINT_PLANE_HEIGHT_DUST_MM: i64 = 1;

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
pub(crate) struct NodeInputSideJoinBand {
    pub(crate) source_band_index: usize,
    pub(crate) band_kind: RoadSurfaceBandKind,
    pub(crate) boundary_mode: NodeInputSideJoinBandBoundaryMode,
    pub(crate) inner_path_world: Vec<RoadVec3>,
    pub(crate) outer_path_world: Vec<RoadVec3>,
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
            add_bend_side_join_bands(&input.mouths, &mut bands_by_mouth)?;
        }
        RoadSurfaceVisualNodePieceKind::JunctionN => {
            add_junction_side_join_bands(&input.mouths, &mut bands_by_mouth)?;
        }
        RoadSurfaceVisualNodePieceKind::Terminal => {}
    }
    Ok(bands_by_mouth)
}

fn add_bend_side_join_bands(
    mouths: &[NodeInputMouth],
    bands_by_mouth: &mut [Vec<NodeInputSideJoinBand>],
) -> Result<(), SideJoinGenerationError> {
    if mouths.len() != 2 {
        return Ok(());
    }

    append_adjacent_side_join_bands(mouths, bands_by_mouth, 0, 1, SideJoinPathMode::BendArc)?;
    append_adjacent_side_join_bands(mouths, bands_by_mouth, 1, 0, SideJoinPathMode::BendArc)?;
    Ok(())
}

fn add_junction_side_join_bands(
    mouths: &[NodeInputMouth],
    bands_by_mouth: &mut [Vec<NodeInputSideJoinBand>],
) -> Result<(), SideJoinGenerationError> {
    if mouths.len() < 2 {
        return Ok(());
    }

    for from_index in 0..mouths.len() {
        let to_index = if from_index + 1 == mouths.len() {
            0
        } else {
            from_index + 1
        };
        append_adjacent_side_join_bands(
            mouths,
            bands_by_mouth,
            from_index,
            to_index,
            SideJoinPathMode::JunctionNonRoad,
        )?;
    }
    Ok(())
}

fn append_adjacent_side_join_bands(
    mouths: &[NodeInputMouth],
    bands_by_mouth: &mut [Vec<NodeInputSideJoinBand>],
    from_index: usize,
    to_index: usize,
    path_mode: SideJoinPathMode,
) -> Result<(), SideJoinGenerationError> {
    let from_mouth = &mouths[from_index];
    let to_mouth = &mouths[to_index];
    let from_layers = side_join_layers(from_mouth, SideJoinProfileSide::End);
    let to_layers = side_join_layers(to_mouth, SideJoinProfileSide::Start);
    if from_layers.is_empty() || to_layers.is_empty() {
        return Ok(());
    }

    let mut join_bands = side_join_bands(
        mouths,
        from_mouth,
        &from_layers,
        to_mouth,
        &to_layers,
        path_mode,
    )?;
    canonicalize_side_join_bands(&mut join_bands);
    bands_by_mouth[from_index].extend(join_bands);
    Ok(())
}

fn canonicalize_side_join_bands(join_bands: &mut Vec<NodeInputSideJoinBand>) {
    for join_band in join_bands.iter_mut() {
        quantize_side_join_band_xz(join_band);
    }
    join_bands.retain(side_join_band_has_quantized_area);
}

fn side_join_layers(mouth: &NodeInputMouth, side: SideJoinProfileSide) -> Vec<SideJoinLayer> {
    let Some(first_carriageway) = mouth
        .band_intervals
        .iter()
        .position(|band| band.band_kind == RoadSurfaceBandKind::Carriageway)
    else {
        return Vec::new();
    };
    let Some(last_carriageway) = mouth
        .band_intervals
        .iter()
        .rposition(|band| band.band_kind == RoadSurfaceBandKind::Carriageway)
    else {
        return Vec::new();
    };

    match side {
        SideJoinProfileSide::Start => (0..=first_carriageway)
            .rev()
            .filter_map(|band_index| {
                mouth
                    .band_intervals
                    .get(band_index)
                    .map(|band| SideJoinLayer {
                        band_index,
                        band_kind: band.band_kind,
                        inner_boundary_index: band_index + 1,
                        outer_boundary_index: band_index,
                    })
            })
            .collect(),
        SideJoinProfileSide::End => (last_carriageway..mouth.band_intervals.len())
            .filter_map(|band_index| {
                mouth
                    .band_intervals
                    .get(band_index)
                    .map(|band| SideJoinLayer {
                        band_index,
                        band_kind: band.band_kind,
                        inner_boundary_index: band_index,
                        outer_boundary_index: band_index + 1,
                    })
            })
            .collect(),
    }
}

fn side_join_bands(
    mouths: &[NodeInputMouth],
    from_mouth: &NodeInputMouth,
    from_layers: &[SideJoinLayer],
    to_mouth: &NodeInputMouth,
    to_layers: &[SideJoinLayer],
    path_mode: SideJoinPathMode,
) -> Result<Vec<NodeInputSideJoinBand>, SideJoinGenerationError> {
    let mut join_bands = Vec::new();
    let mut inner_path_world = None;
    for (from_layer, to_layer) in from_layers.iter().zip(to_layers) {
        if from_layer.band_kind != to_layer.band_kind {
            break;
        }
        if path_mode == SideJoinPathMode::JunctionNonRoad
            && from_layer.band_kind == RoadSurfaceBandKind::Carriageway
        {
            inner_path_world = None;
            continue;
        }
        let height_plane = if path_mode == SideJoinPathMode::JunctionNonRoad {
            endpoint_height_plane_for_band_kind(mouths, from_layer.band_kind)?
        } else {
            None
        };

        let Some(band_inner_path_world) = side_join_band_inner_path(
            from_mouth,
            from_layer,
            to_mouth,
            to_layer,
            inner_path_world,
            path_mode,
            height_plane,
        )?
        else {
            break;
        };
        let Some(outer_start_world) = endpoint_layer_outer_world(from_mouth, from_layer) else {
            break;
        };
        let Some(outer_end_world) = endpoint_layer_outer_world(to_mouth, to_layer) else {
            break;
        };
        let Some(band_outer_path_world) = side_join_boundary_path_world(
            from_mouth,
            outer_start_world,
            to_mouth,
            outer_end_world,
            path_mode,
            height_plane,
        )?
        else {
            break;
        };

        let boundary_mode = match from_layer.band_kind {
            RoadSurfaceBandKind::Carriageway
            | RoadSurfaceBandKind::CurbOrShoulder
            | RoadSurfaceBandKind::Sidewalk => NodeInputSideJoinBandBoundaryMode::MaterialBand,
            _ => NodeInputSideJoinBandBoundaryMode::MaterialBandWithSameOwnerOuterCap,
        };
        push_side_join_band(
            &mut join_bands,
            from_layer.band_index,
            from_layer.band_kind,
            boundary_mode,
            band_inner_path_world,
            band_outer_path_world.clone(),
        )?;
        inner_path_world = Some(band_outer_path_world);
    }
    Ok(join_bands)
}

fn side_join_band_inner_path(
    from_mouth: &NodeInputMouth,
    from_layer: &SideJoinLayer,
    to_mouth: &NodeInputMouth,
    to_layer: &SideJoinLayer,
    previous_outer_path_world: Option<Vec<RoadVec3>>,
    path_mode: SideJoinPathMode,
    height_plane: Option<SideJoinHeightPlane>,
) -> Result<Option<Vec<RoadVec3>>, SideJoinGenerationError> {
    if let Some(path_world) = previous_outer_path_world {
        if path_mode != SideJoinPathMode::BendArc {
            return Ok(Some(path_world));
        }
        let Some(inner_start_world) = endpoint_layer_inner_world(from_mouth, from_layer) else {
            return Ok(None);
        };
        let Some(inner_end_world) = endpoint_layer_inner_world(to_mouth, to_layer) else {
            return Ok(None);
        };
        return reheight_side_join_path_world(path_world, inner_start_world.y, inner_end_world.y);
    }
    let Some(inner_start_world) = endpoint_layer_inner_world(from_mouth, from_layer) else {
        return Ok(None);
    };
    let Some(inner_end_world) = endpoint_layer_inner_world(to_mouth, to_layer) else {
        return Ok(None);
    };
    side_join_boundary_path_world(
        from_mouth,
        inner_start_world,
        to_mouth,
        inner_end_world,
        path_mode,
        height_plane,
    )
}

fn reheight_side_join_path_world(
    mut path_world: Vec<RoadVec3>,
    start_height_m: f64,
    end_height_m: f64,
) -> Result<Option<Vec<RoadVec3>>, SideJoinGenerationError> {
    let total_length_m = path_world
        .windows(2)
        .map(|segment| xz_from_road_vec3(segment[0]).distance(xz_from_road_vec3(segment[1])))
        .sum::<f64>();
    let mut cumulative_length_m = 0.0;
    for index in 0..path_world.len() {
        if index > 0 {
            cumulative_length_m += xz_from_road_vec3(path_world[index - 1])
                .distance(xz_from_road_vec3(path_world[index]));
        }
        let t = if total_length_m > f64::EPSILON {
            cumulative_length_m / total_length_m
        } else {
            0.0
        };
        path_world[index].y = start_height_m + (end_height_m - start_height_m) * t;
    }
    clean_side_join_path_world(path_world).map_err(SideJoinGenerationError::from_path_height_error)
}

fn push_side_join_band(
    join_bands: &mut Vec<NodeInputSideJoinBand>,
    source_band_index: usize,
    band_kind: RoadSurfaceBandKind,
    boundary_mode: NodeInputSideJoinBandBoundaryMode,
    inner_path_world: Vec<RoadVec3>,
    outer_path_world: Vec<RoadVec3>,
) -> Result<(), SideJoinGenerationError> {
    if inner_path_world.len() < 2 || outer_path_world.len() < 2 {
        return Ok(());
    }

    let mut contour_world = inner_path_world.clone();
    contour_world.extend(outer_path_world.iter().rev().copied());
    remove_repeated_road_vec3_xz_points(&mut contour_world)
        .map_err(SideJoinGenerationError::from_path_height_error)?;
    join_bands.push(NodeInputSideJoinBand {
        source_band_index,
        band_kind,
        boundary_mode,
        inner_path_world,
        outer_path_world,
        contour_world,
    });
    Ok(())
}

fn side_join_boundary_path_world(
    from_mouth: &NodeInputMouth,
    from_world: RoadVec3,
    to_mouth: &NodeInputMouth,
    to_world: RoadVec3,
    path_mode: SideJoinPathMode,
    height_plane: Option<SideJoinHeightPlane>,
) -> Result<Option<Vec<RoadVec3>>, SideJoinGenerationError> {
    let from_xz = xz_from_road_vec3(from_world);
    let to_xz = xz_from_road_vec3(to_world);
    let join_point_xz = side_join_backend_meet_point_xz(
        from_xz,
        from_mouth.direction_xz,
        to_xz,
        to_mouth.direction_xz,
    );
    let path_xz = if let Some(join_point_xz) = join_point_xz {
        let Some(path_xz) =
            side_join_backend_join_path_xz(from_xz, join_point_xz, to_xz, path_mode)
        else {
            return Ok(None);
        };
        path_xz
    } else {
        let Some(path_xz) = cleaned_open_road_points([from_xz, to_xz]) else {
            return Ok(None);
        };
        path_xz
    };
    let path_world = path_xz
        .into_iter()
        .map(|point_xz| {
            let height_m = height_plane.map_or_else(
                || height_on_linear_height_path(point_xz, from_xz, from_world.y, to_xz, to_world.y),
                |plane| {
                    if same_surface_xz_key(point_xz, from_xz) {
                        from_world.y
                    } else if same_surface_xz_key(point_xz, to_xz) {
                        to_world.y
                    } else {
                        plane.height_at_xz(point_xz)
                    }
                },
            );
            RoadVec3::new(point_xz.x, height_m, point_xz.y)
        })
        .collect();
    clean_side_join_path_world(path_world).map_err(SideJoinGenerationError::from_path_height_error)
}

impl SideJoinHeightPlane {
    fn height_at_xz(self, point_xz: RoadVec2) -> f64 {
        self.origin.y
            + self.grade_x * (point_xz.x - self.origin.x)
            + self.grade_z * (point_xz.y - self.origin.z)
    }
}

fn endpoint_height_plane_for_band_kind(
    mouths: &[NodeInputMouth],
    band_kind: RoadSurfaceBandKind,
) -> Result<Option<SideJoinHeightPlane>, SideJoinGenerationError> {
    let mut points = mouths
        .iter()
        .flat_map(|mouth| mouth.endpoint_rails.iter())
        .filter(|rail| rail.band_kind == band_kind)
        .flat_map(|rail| [rail.start_world, rail.end_world])
        .collect::<Vec<_>>();
    canonicalize_height_plane_points(&mut points);
    let Some(plane) = height_plane_from_points(&points) else {
        return Ok(None);
    };
    Ok(validate_height_plane(&points, plane).then_some(plane))
}

fn canonicalize_height_plane_points(points: &mut Vec<RoadVec3>) {
    points.sort_by_key(|point| {
        let key = SurfaceXzKey::from_road_xz(xz_from_road_vec3(*point));
        (
            key.x_key(),
            key.z_key(),
            SurfaceHeightMmKey::from_m_f64(point.y).as_i64(),
        )
    });
    points.dedup_by_key(|point| {
        let key = SurfaceXzKey::from_road_xz(xz_from_road_vec3(*point));
        (
            key.x_key(),
            key.z_key(),
            SurfaceHeightMmKey::from_m_f64(point.y).as_i64(),
        )
    });
}

fn height_plane_from_points(points: &[RoadVec3]) -> Option<SideJoinHeightPlane> {
    let mut selected: Option<(u128, SideJoinHeightPlane)> = None;
    for a_index in 0..points.len() {
        for b_index in a_index + 1..points.len() {
            for c_index in b_index + 1..points.len() {
                let area =
                    height_plane_triangle_area2(points[a_index], points[b_index], points[c_index]);
                if area == 0 {
                    continue;
                }
                let plane =
                    height_plane_from_triangle(points[a_index], points[b_index], points[c_index])?;
                if selected.is_none_or(|(selected_area, _)| area > selected_area) {
                    selected = Some((area, plane));
                }
            }
        }
    }
    selected.map(|(_, plane)| plane)
}

fn height_plane_triangle_area2(a: RoadVec3, b: RoadVec3, c: RoadVec3) -> u128 {
    let a = SurfaceXzKey::from_road_xz(xz_from_road_vec3(a)).raw_tuple();
    let b = SurfaceXzKey::from_road_xz(xz_from_road_vec3(b)).raw_tuple();
    let c = SurfaceXzKey::from_road_xz(xz_from_road_vec3(c)).raw_tuple();
    SurfaceXzKey::raw_tuple_triangle_area2(a, b, c).unsigned_abs()
}

fn height_plane_from_triangle(
    origin: RoadVec3,
    b: RoadVec3,
    c: RoadVec3,
) -> Option<SideJoinHeightPlane> {
    let ux = b.x - origin.x;
    let uz = b.z - origin.z;
    let uy = b.y - origin.y;
    let vx = c.x - origin.x;
    let vz = c.z - origin.z;
    let vy = c.y - origin.y;
    let denominator = ux * vz - uz * vx;
    if denominator.abs() <= f64::EPSILON {
        return None;
    }
    Some(SideJoinHeightPlane {
        origin,
        grade_x: (uy * vz - uz * vy) / denominator,
        grade_z: (ux * vy - uy * vx) / denominator,
    })
}

fn validate_height_plane(points: &[RoadVec3], plane: SideJoinHeightPlane) -> bool {
    for point in points {
        let expected_height_m = plane.height_at_xz(xz_from_road_vec3(*point));
        let expected_height_key = SurfaceHeightMmKey::from_m_f64(expected_height_m);
        let incoming_height_key = SurfaceHeightMmKey::from_m_f64(point.y);
        if (expected_height_key.as_i64() - incoming_height_key.as_i64()).abs()
            <= SIDE_JOIN_ENDPOINT_PLANE_HEIGHT_DUST_MM
        {
            continue;
        }
        return false;
    }
    true
}

fn same_surface_xz_key(a: RoadVec2, b: RoadVec2) -> bool {
    SurfaceXzKey::from_road_xz(a) == SurfaceXzKey::from_road_xz(b)
}

fn side_join_backend_join_path_xz(
    start_xz: RoadVec2,
    join_point_xz: RoadVec2,
    end_xz: RoadVec2,
    path_mode: SideJoinPathMode,
) -> Option<Vec<RoadVec2>> {
    let start_tangent = join_point_xz - start_xz;
    let end_tangent = end_xz - join_point_xz;
    match path_mode {
        SideJoinPathMode::BendArc => {
            if let Some(points) =
                side_join_backend_arc_path_xz(start_xz, start_tangent, end_xz, end_tangent)
            {
                return cleaned_open_road_points(points);
            }
        }
        SideJoinPathMode::JunctionNonRoad => {
            return side_join_backend_cavalier_join_path_xz(start_xz, join_point_xz, end_xz);
        }
    }
    cleaned_open_road_points([start_xz, join_point_xz, end_xz])
}

fn side_join_backend_cavalier_join_path_xz(
    start_xz: RoadVec2,
    join_point_xz: RoadVec2,
    end_xz: RoadVec2,
) -> Option<Vec<RoadVec2>> {
    let mut polyline = cleaned_open_road_polyline(
        [start_xz, join_point_xz, end_xz],
        SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M,
        true,
    )?;
    if polyline.vertex_count() < 2 || polyline.scan_for_self_intersect() {
        return None;
    }
    let mut points = polyline_to_road_points(&polyline);
    if points.len() < 2 {
        return None;
    }
    let last_index = points.len() - 1;
    if SurfaceXzKey::from_road_xz(points[0]) != SurfaceXzKey::from_road_xz(start_xz)
        || SurfaceXzKey::from_road_xz(points[last_index]) != SurfaceXzKey::from_road_xz(end_xz)
    {
        return None;
    }
    points[0] = start_xz;
    points[last_index] = end_xz;
    polyline = cleaned_open_road_polyline(points, SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M, true)?;
    if polyline.vertex_count() < 2 || polyline.scan_for_self_intersect() {
        return None;
    }
    Some(polyline_to_road_points(&polyline))
}

fn side_join_backend_arc_path_xz(
    start_xz: RoadVec2,
    start_tangent: RoadVec2,
    end_xz: RoadVec2,
    end_tangent: RoadVec2,
) -> Option<Vec<RoadVec2>> {
    let start_tangent = normalized_side_join_direction(start_tangent)?;
    let end_tangent = normalized_side_join_direction(end_tangent)?;
    let center_xz = side_join_arc_center_xz(start_xz, start_tangent, end_xz, end_tangent)?;
    let start_radius_m = center_xz.distance(start_xz);
    let end_radius_m = center_xz.distance(end_xz);
    if start_radius_m <= SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M
        || (start_radius_m - end_radius_m).abs() > SIDE_JOIN_ARC_RADIUS_EPS_M
    {
        return None;
    }

    let ccw_start_tangent = side_join_arc_tangent_xz(center_xz, start_xz, true)?;
    let cw_start_tangent = side_join_arc_tangent_xz(center_xz, start_xz, false)?;
    let is_ccw = ccw_start_tangent.dot(start_tangent) >= cw_start_tangent.dot(start_tangent);
    let end_arc_tangent = side_join_arc_tangent_xz(center_xz, end_xz, is_ccw)?;
    if end_arc_tangent.dot(end_tangent) <= 0.0 {
        return None;
    }

    let sweep_angle = side_join_arc_sweep_angle(center_xz, start_xz, end_xz, is_ccw);
    if sweep_angle.abs() <= SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M || sweep_angle.abs() > PI {
        return None;
    }

    let start_vertex =
        RoadPolylineVertex::new(start_xz.x, start_xz.y, bulge_from_angle(sweep_angle));
    let end_vertex = RoadPolylineVertex::new(end_xz.x, end_xz.y, 0.0);
    let mut points = Vec::with_capacity((1 << SIDE_JOIN_ARC_SPLIT_DEPTH) + 1);
    append_side_join_backend_arc_samples(
        &mut points,
        start_vertex,
        end_vertex,
        SIDE_JOIN_ARC_SPLIT_DEPTH,
    );
    points.push(end_xz);
    Some(points)
}

fn side_join_arc_center_xz(
    start_xz: RoadVec2,
    start_tangent: RoadVec2,
    end_xz: RoadVec2,
    end_tangent: RoadVec2,
) -> Option<RoadVec2> {
    let start_normal = left_perp(start_tangent);
    let end_normal = left_perp(end_tangent);
    side_join_backend_meet_point_xz(start_xz, start_normal, end_xz, end_normal)
}

fn side_join_arc_tangent_xz(
    center_xz: RoadVec2,
    point_xz: RoadVec2,
    is_ccw: bool,
) -> Option<RoadVec2> {
    let radius = point_xz - center_xz;
    let tangent = if is_ccw {
        left_perp(radius)
    } else {
        -left_perp(radius)
    };
    normalized_side_join_direction(tangent)
}

fn side_join_arc_sweep_angle(
    center_xz: RoadVec2,
    start_xz: RoadVec2,
    end_xz: RoadVec2,
    is_ccw: bool,
) -> f64 {
    let start = start_xz - center_xz;
    let end = end_xz - center_xz;
    let start_angle = start.y.atan2(start.x);
    let end_angle = end.y.atan2(end.x);
    if is_ccw {
        (end_angle - start_angle).rem_euclid(TAU)
    } else {
        -((start_angle - end_angle).rem_euclid(TAU))
    }
}

fn append_side_join_backend_arc_samples(
    points: &mut Vec<RoadVec2>,
    start_vertex: RoadPolylineVertex,
    end_vertex: RoadPolylineVertex,
    split_depth: usize,
) {
    if split_depth == 0 {
        points.push(RoadVec2::new(start_vertex.x, start_vertex.y));
        return;
    }
    let midpoint = seg_midpoint(start_vertex, end_vertex);
    let split = seg_split_at_point(
        start_vertex,
        end_vertex,
        midpoint,
        SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M,
    );
    append_side_join_backend_arc_samples(
        points,
        split.updated_start,
        split.split_vertex,
        split_depth - 1,
    );
    append_side_join_backend_arc_samples(points, split.split_vertex, end_vertex, split_depth - 1);
}

fn normalized_side_join_direction(direction: RoadVec2) -> Option<RoadVec2> {
    let length = direction.length();
    (length > SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M).then_some(direction / length)
}

fn left_perp(direction: RoadVec2) -> RoadVec2 {
    RoadVec2::new(-direction.y, direction.x)
}

fn side_join_backend_meet_point_xz(
    start_a: RoadVec2,
    direction_a: RoadVec2,
    start_b: RoadVec2,
    direction_b: RoadVec2,
) -> Option<RoadVec2> {
    let a0 = cavalier_vec2(start_a);
    let a1 = cavalier_vec2(start_a + direction_a);
    let b0 = cavalier_vec2(start_b);
    let b1 = cavalier_vec2(start_b + direction_b);
    match line_line_intr(a0, a1, b0, b1, SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M) {
        LineLineIntr::TrueIntersect { seg1_t, .. }
        | LineLineIntr::FalseIntersect { seg1_t, .. } => Some(start_a + direction_a * seg1_t),
        LineLineIntr::NoIntersect | LineLineIntr::Overlapping { .. } => None,
    }
}

fn cavalier_vec2(point: RoadVec2) -> CavalierVec2<f64> {
    CavalierVec2::new(point.x, point.y)
}

fn cleaned_open_road_points(
    points_xz: impl IntoIterator<Item = RoadVec2>,
) -> Option<Vec<RoadVec2>> {
    cleaned_open_road_polyline(points_xz, SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M, true)
        .map(|polyline| polyline_to_road_points(&polyline))
}

fn height_on_linear_height_path(
    point_xz: RoadVec2,
    start_xz: RoadVec2,
    start_height_m: f64,
    end_xz: RoadVec2,
    end_height_m: f64,
) -> f64 {
    let axis = end_xz - start_xz;
    let axis_len2 = axis.length_squared();
    let t = if axis_len2 > f64::EPSILON {
        ((point_xz - start_xz).dot(axis) / axis_len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    start_height_m + (end_height_m - start_height_m) * t
}

fn clean_side_join_path_world(
    path_world: Vec<RoadVec3>,
) -> Result<Option<Vec<RoadVec3>>, PathHeightResolutionError> {
    if path_world.len() < 2 {
        return Ok(None);
    }
    let Some(polyline) =
        cleaned_open_world_path_polyline(&path_world, SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M, true)
    else {
        return Ok(None);
    };
    if polyline.vertex_count() < 2 {
        return Ok(None);
    }
    let points_xz = polyline_to_road_points(&polyline);
    let Some(cleaned_world) = reheight_road_points_from_world_path(points_xz, &path_world)? else {
        return Ok(None);
    };
    Ok((cleaned_world.len() >= 2).then_some(cleaned_world))
}

fn endpoint_boundary_world(mouth: &NodeInputMouth, boundary_index: usize) -> Option<RoadVec3> {
    mouth
        .boundary_rails
        .get(boundary_index)
        .map(|rail| rail.endpoint_world)
}

fn endpoint_layer_inner_world(mouth: &NodeInputMouth, layer: &SideJoinLayer) -> Option<RoadVec3> {
    endpoint_layer_boundary_world(mouth, layer, layer.inner_boundary_index)
}

fn endpoint_layer_outer_world(mouth: &NodeInputMouth, layer: &SideJoinLayer) -> Option<RoadVec3> {
    endpoint_layer_boundary_world(mouth, layer, layer.outer_boundary_index)
}

fn endpoint_layer_boundary_world(
    mouth: &NodeInputMouth,
    layer: &SideJoinLayer,
    boundary_index: usize,
) -> Option<RoadVec3> {
    let interval = mouth.band_intervals.get(layer.band_index)?;
    if boundary_index == layer.band_index {
        Some(interval.endpoint_start_world)
    } else if boundary_index == layer.band_index + 1 {
        Some(interval.endpoint_end_world)
    } else {
        endpoint_boundary_world(mouth, boundary_index)
    }
}

fn quantize_side_join_band_xz(join_band: &mut NodeInputSideJoinBand) {
    quantize_road_vec3_path_xz_to_overlay_grid(&mut join_band.inner_path_world);
    quantize_road_vec3_path_xz_to_overlay_grid(&mut join_band.outer_path_world);
    quantize_road_vec3_path_xz_to_overlay_grid(&mut join_band.contour_world);
}

fn side_join_band_has_quantized_area(join_band: &NodeInputSideJoinBand) -> bool {
    closed_world_contour_has_area(
        &join_band.contour_world,
        SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M,
        f64::from(NODE_OVERLAY_MIN_AREA_M2),
    )
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

    fn profile_x(x: f32, inward_direction_xz: Vector2) -> IncidentMouthProfile {
        let boundary_points_world = vec![
            Vector3::new(x, 4.0, -4.0),
            Vector3::new(x, 4.1, -3.0),
            Vector3::new(x, 4.2, -1.0),
            Vector3::new(x, 4.0, 0.0),
            Vector3::new(x, 4.2, 1.0),
            Vector3::new(x, 4.1, 3.0),
            Vector3::new(x, 4.0, 4.0),
        ];
        let bands = symmetric_road_bands(&boundary_points_world);
        IncidentMouthProfile {
            inward_direction_xz,
            boundary_points_world,
            bands,
        }
    }

    fn profile_z(z: f32, inward_direction_xz: Vector2) -> IncidentMouthProfile {
        let boundary_points_world = vec![
            Vector3::new(4.0, 4.0, z),
            Vector3::new(3.0, 4.1, z),
            Vector3::new(1.0, 4.2, z),
            Vector3::new(0.0, 4.0, z),
            Vector3::new(-1.0, 4.2, z),
            Vector3::new(-3.0, 4.1, z),
            Vector3::new(-4.0, 4.0, z),
        ];
        let bands = symmetric_road_bands(&boundary_points_world);
        IncidentMouthProfile {
            inward_direction_xz,
            boundary_points_world,
            bands,
        }
    }

    fn symmetric_road_bands(boundary_points_world: &[Vector3]) -> Vec<IncidentMouthBand> {
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
        direction_xz: Vector2,
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
                profile_x(10.0, Vector2::RIGHT),
                profile_x(0.0, Vector2::RIGHT),
                0.0,
                Vector2::RIGHT,
                1,
            ),
            ordered_mouth(
                profile_z(12.0, Vector2::DOWN),
                profile_z(2.0, Vector2::DOWN),
                std::f32::consts::FRAC_PI_2,
                Vector2::DOWN,
                2,
            ),
            ordered_mouth(
                profile_x(-10.0, Vector2::LEFT),
                profile_x(0.0, Vector2::LEFT),
                std::f32::consts::PI,
                Vector2::LEFT,
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

    #[test]
    fn junction_side_join_bands_are_backend_cleaned_non_road_carriers() {
        let bands_by_mouth = side_join_bands_by_mouth(&junction_input())
            .expect("test junction side joins should not have height conflicts");
        let bands = bands_by_mouth.iter().flatten().collect::<Vec<_>>();

        assert!(
            !bands.is_empty(),
            "junction side-join adapter should emit non-road ownership carriers"
        );
        assert!(
            bands
                .iter()
                .all(|band| band.band_kind != RoadSurfaceBandKind::Carriageway),
            "JunctionN side joins must not add carriageway bubble fill"
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
}
