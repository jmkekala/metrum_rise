//! Contour-adapter boundary for node side-join ownership candidates.

use super::backend::{
    RoadPolyline, RoadPolylineVertex, RoadVec2, RoadVec3, polyline_to_road_points,
    quantize_road_vec3_xz_to_overlay_grid, road_points_to_polyline,
    road_vec3_xz as xz_from_road_vec3,
};
use super::input::{NodeArrangementInput, NodeInputMouth};
use super::keys::SurfaceXzKey;
use super::{NODE_OVERLAY_MIN_AREA_M2, RoadSurfaceBandKind, RoadSurfaceVisualNodePieceKind};
use cavalier_contours::core::math::{
    LineLineIntr, Vector2 as CavalierVec2, bulge_from_angle, line_line_intr,
};
use cavalier_contours::polyline::{PlineCreation, PlineSource, seg_midpoint, seg_split_at_point};
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

const SIDE_JOIN_HEIGHT_EDGE_EPS_M: f64 = 0.001;
const SIDE_JOIN_ARC_SPLIT_DEPTH: usize = 2;
const SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M: f64 = 1.0e-6;

#[derive(Clone, Copy)]
struct SideJoinLayer {
    band_index: usize,
    band_kind: RoadSurfaceBandKind,
    inner_boundary_index: usize,
    outer_boundary_index: usize,
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

pub(crate) fn side_join_bands_by_mouth(
    input: &NodeArrangementInput,
) -> Vec<Vec<NodeInputSideJoinBand>> {
    let mut bands_by_mouth = vec![Vec::new(); input.mouths.len()];
    match input.piece_kind {
        RoadSurfaceVisualNodePieceKind::Bend => {
            add_bend_side_join_bands(&input.mouths, &mut bands_by_mouth);
        }
        RoadSurfaceVisualNodePieceKind::JunctionN => {
            add_junction_side_join_bands(&input.mouths, &mut bands_by_mouth);
        }
        RoadSurfaceVisualNodePieceKind::Terminal => {}
    }
    bands_by_mouth
}

fn add_bend_side_join_bands(
    mouths: &[NodeInputMouth],
    bands_by_mouth: &mut [Vec<NodeInputSideJoinBand>],
) {
    if mouths.len() != 2 {
        return;
    }

    append_adjacent_side_join_bands(mouths, bands_by_mouth, 0, 1, SideJoinPathMode::BendArc);
    append_adjacent_side_join_bands(mouths, bands_by_mouth, 1, 0, SideJoinPathMode::BendArc);
}

fn add_junction_side_join_bands(
    mouths: &[NodeInputMouth],
    bands_by_mouth: &mut [Vec<NodeInputSideJoinBand>],
) {
    if mouths.len() < 2 {
        return;
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
        );
    }
}

fn append_adjacent_side_join_bands(
    mouths: &[NodeInputMouth],
    bands_by_mouth: &mut [Vec<NodeInputSideJoinBand>],
    from_index: usize,
    to_index: usize,
    path_mode: SideJoinPathMode,
) {
    let from_mouth = &mouths[from_index];
    let to_mouth = &mouths[to_index];
    let from_layers = side_join_layers(from_mouth, SideJoinProfileSide::End);
    let to_layers = side_join_layers(to_mouth, SideJoinProfileSide::Start);
    if from_layers.is_empty() || to_layers.is_empty() {
        return;
    }

    let mut join_bands = side_join_bands(from_mouth, &from_layers, to_mouth, &to_layers, path_mode);
    canonicalize_side_join_bands(&mut join_bands);
    bands_by_mouth[from_index].extend(join_bands);
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
    from_mouth: &NodeInputMouth,
    from_layers: &[SideJoinLayer],
    to_mouth: &NodeInputMouth,
    to_layers: &[SideJoinLayer],
    path_mode: SideJoinPathMode,
) -> Vec<NodeInputSideJoinBand> {
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

        let Some(band_inner_path_world) = side_join_band_inner_path(
            from_mouth,
            from_layer,
            to_mouth,
            to_layer,
            inner_path_world,
            path_mode,
        ) else {
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
        ) else {
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
        );
        inner_path_world = Some(band_outer_path_world);
    }
    join_bands
}

fn side_join_band_inner_path(
    from_mouth: &NodeInputMouth,
    from_layer: &SideJoinLayer,
    to_mouth: &NodeInputMouth,
    to_layer: &SideJoinLayer,
    previous_outer_path_world: Option<Vec<RoadVec3>>,
    path_mode: SideJoinPathMode,
) -> Option<Vec<RoadVec3>> {
    if let Some(path_world) = previous_outer_path_world {
        if path_mode != SideJoinPathMode::BendArc {
            return Some(path_world);
        }
        let inner_start_world = endpoint_layer_inner_world(from_mouth, from_layer)?;
        let inner_end_world = endpoint_layer_inner_world(to_mouth, to_layer)?;
        return reheight_side_join_path_world(path_world, inner_start_world.y, inner_end_world.y);
    }
    let inner_start_world = endpoint_layer_inner_world(from_mouth, from_layer)?;
    let inner_end_world = endpoint_layer_inner_world(to_mouth, to_layer)?;
    side_join_boundary_path_world(
        from_mouth,
        inner_start_world,
        to_mouth,
        inner_end_world,
        path_mode,
    )
}

fn reheight_side_join_path_world(
    mut path_world: Vec<RoadVec3>,
    start_height_m: f64,
    end_height_m: f64,
) -> Option<Vec<RoadVec3>> {
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
    clean_side_join_path_world(path_world)
}

fn push_side_join_band(
    join_bands: &mut Vec<NodeInputSideJoinBand>,
    source_band_index: usize,
    band_kind: RoadSurfaceBandKind,
    boundary_mode: NodeInputSideJoinBandBoundaryMode,
    inner_path_world: Vec<RoadVec3>,
    outer_path_world: Vec<RoadVec3>,
) {
    if inner_path_world.len() < 2 || outer_path_world.len() < 2 {
        return;
    }

    let mut contour_world = inner_path_world.clone();
    contour_world.extend(outer_path_world.iter().rev().copied());
    remove_repeated_road_vec3_points(&mut contour_world);
    join_bands.push(NodeInputSideJoinBand {
        source_band_index,
        band_kind,
        boundary_mode,
        inner_path_world,
        outer_path_world,
        contour_world,
    });
}

fn side_join_boundary_path_world(
    from_mouth: &NodeInputMouth,
    from_world: RoadVec3,
    to_mouth: &NodeInputMouth,
    to_world: RoadVec3,
    path_mode: SideJoinPathMode,
) -> Option<Vec<RoadVec3>> {
    let from_xz = xz_from_road_vec3(from_world);
    let to_xz = xz_from_road_vec3(to_world);
    let join_point_xz = side_join_backend_meet_point_xz(
        from_xz,
        from_mouth.direction_xz,
        to_xz,
        to_mouth.direction_xz,
    );
    let path_xz = if let Some(join_point_xz) = join_point_xz {
        side_join_backend_join_path_xz(from_xz, join_point_xz, to_xz, path_mode)?
    } else {
        cleaned_open_road_points([from_xz, to_xz])?
    };
    let path_world = path_xz
        .into_iter()
        .map(|point_xz| {
            let height_m =
                height_on_linear_height_path(point_xz, from_xz, from_world.y, to_xz, to_world.y);
            RoadVec3::new(point_xz.x, height_m, point_xz.y)
        })
        .collect();
    clean_side_join_path_world(path_world)
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
    let mut polyline = cleaned_open_road_polyline([start_xz, join_point_xz, end_xz])?;
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
    polyline = cleaned_open_road_polyline(points)?;
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
        || (start_radius_m - end_radius_m).abs() > SIDE_JOIN_HEIGHT_EDGE_EPS_M
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

fn cleaned_open_world_path_polyline(path_world: &[RoadVec3]) -> Option<RoadPolyline> {
    cleaned_open_road_polyline(path_world.iter().copied().map(xz_from_road_vec3))
}

fn cleaned_open_road_points(
    points_xz: impl IntoIterator<Item = RoadVec2>,
) -> Option<Vec<RoadVec2>> {
    cleaned_open_road_polyline(points_xz).map(|polyline| polyline_to_road_points(&polyline))
}

fn cleaned_open_road_polyline(
    points_xz: impl IntoIterator<Item = RoadVec2>,
) -> Option<RoadPolyline> {
    let raw = road_points_to_polyline(points_xz, false);
    let cleaned =
        RoadPolyline::create_from_remove_repeat(&raw, SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M);
    let cleaned = cleaned
        .remove_redundant(SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M)
        .unwrap_or(cleaned);
    (cleaned.vertex_count() >= 2).then_some(cleaned)
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

fn clean_side_join_path_world(path_world: Vec<RoadVec3>) -> Option<Vec<RoadVec3>> {
    if path_world.len() < 2 {
        return None;
    }
    let mut polyline = cleaned_open_world_path_polyline(&path_world)?;
    if let Some(cleaned) = polyline.remove_redundant(SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M) {
        polyline = cleaned;
    }
    if polyline.vertex_count() < 2 {
        return None;
    }
    let points_xz = polyline_to_road_points(&polyline);
    let mut cleaned_world = points_xz
        .into_iter()
        .map(|point_xz| {
            let height_m = height_on_world_path(point_xz, &path_world)?;
            Some(RoadVec3::new(point_xz.x, height_m, point_xz.y))
        })
        .collect::<Option<Vec<_>>>()?;
    remove_repeated_road_vec3_points(&mut cleaned_world);
    (cleaned_world.len() >= 2).then_some(cleaned_world)
}

fn height_on_world_path(point_xz: RoadVec2, path_world: &[RoadVec3]) -> Option<f64> {
    let key = SurfaceXzKey::from_road_xz(point_xz);
    for point_world in path_world {
        if SurfaceXzKey::from_road_xz(xz_from_road_vec3(*point_world)) == key {
            return Some(point_world.y);
        }
    }
    for segment in path_world.windows(2) {
        if let Some(height_m) = height_on_world_segment(point_xz, segment[0], segment[1]) {
            return Some(height_m);
        }
    }
    None
}

fn height_on_world_segment(
    point_xz: RoadVec2,
    start_world: RoadVec3,
    end_world: RoadVec3,
) -> Option<f64> {
    let start_xz = xz_from_road_vec3(start_world);
    let end_xz = xz_from_road_vec3(end_world);
    let axis = end_xz - start_xz;
    let axis_len2 = axis.length_squared();
    if axis_len2 <= f64::EPSILON {
        return None;
    }
    let t = ((point_xz - start_xz).dot(axis) / axis_len2).clamp(0.0, 1.0);
    let closest = start_xz + axis * t;
    if closest.distance_squared(point_xz)
        > SIDE_JOIN_HEIGHT_EDGE_EPS_M * SIDE_JOIN_HEIGHT_EDGE_EPS_M
    {
        return None;
    }
    Some(start_world.y + (end_world.y - start_world.y) * t)
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
    for point in &mut join_band.inner_path_world {
        *point = quantize_road_vec3_xz_to_overlay_grid(*point);
    }
    for point in &mut join_band.outer_path_world {
        *point = quantize_road_vec3_xz_to_overlay_grid(*point);
    }
    for point in &mut join_band.contour_world {
        *point = quantize_road_vec3_xz_to_overlay_grid(*point);
    }
}

fn side_join_band_has_quantized_area(join_band: &NodeInputSideJoinBand) -> bool {
    let raw = road_points_to_polyline(
        join_band
            .contour_world
            .iter()
            .copied()
            .map(xz_from_road_vec3),
        true,
    );
    let contour =
        RoadPolyline::create_from_remove_repeat(&raw, SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M);
    contour.vertex_count() >= 3
        && contour.area().abs() > f64::from(NODE_OVERLAY_MIN_AREA_M2)
        && !contour.scan_for_self_intersect()
}

fn remove_repeated_road_vec3_points(points: &mut Vec<RoadVec3>) {
    points.dedup_by(|a, b| {
        SurfaceXzKey::from_road_xz(xz_from_road_vec3(*a))
            == SurfaceXzKey::from_road_xz(xz_from_road_vec3(*b))
    });
    if points.len() > 1
        && SurfaceXzKey::from_road_xz(xz_from_road_vec3(points[0]))
            == SurfaceXzKey::from_road_xz(xz_from_road_vec3(
                *points.last().expect("points are non-empty"),
            ))
    {
        points.pop();
    }
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
            uses_sampled_band_domain_paths: false,
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
        let bands_by_mouth = side_join_bands_by_mouth(&junction_input());
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
