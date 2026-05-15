//! Canonical terminal-cap adapter for one-mouth visual node ownership.

use super::backend::{
    ROAD_OVERLAY_COORDINATE_SCALE, RoadPolyline, RoadVec2, RoadVec3, polyline_to_road_points,
    quantize_road_vec2_to_overlay_grid, road_points_to_polyline,
};
use super::input::{NodeArrangementInput, NodeInputMouth};
use super::{NODE_OVERLAY_MIN_AREA_M2, RoadSurfaceBandKind, RoadSurfaceVisualNodePieceKind};
use cavalier_contours::polyline::{PlineCreation, PlineSource};

const TERMINAL_CAP_HEIGHT_EDGE_EPS_M: f64 = 0.001;
const TERMINAL_CAP_POLYLINE_POINT_EQUAL_EPS_M: f64 = 1.0e-6;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeTerminalCapBand {
    pub(crate) source_band_index: usize,
    pub(crate) band_kind: RoadSurfaceBandKind,
    pub(crate) inner_path_world: Vec<RoadVec3>,
    pub(crate) outer_path_world: Vec<RoadVec3>,
    pub(crate) contour_world: Vec<RoadVec3>,
}

pub(crate) fn terminal_cap_bands_by_mouth(
    input: &NodeArrangementInput,
) -> Vec<Vec<NodeTerminalCapBand>> {
    let mut bands_by_mouth = vec![Vec::new(); input.mouths.len()];
    if input.piece_kind != RoadSurfaceVisualNodePieceKind::Terminal {
        return bands_by_mouth;
    }

    for (mouth_index, mouth) in input.mouths.iter().enumerate() {
        let mut bands = terminal_cap_bands(mouth);
        canonicalize_terminal_cap_bands(&mut bands);
        bands_by_mouth[mouth_index] = bands;
    }

    bands_by_mouth
}

fn terminal_cap_bands(mouth: &NodeInputMouth) -> Vec<NodeTerminalCapBand> {
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
    if first_carriageway == 0
        || last_carriageway + 1 >= mouth.band_intervals.len()
        || mouth.boundary_rails.len() != mouth.band_intervals.len() + 1
    {
        return Vec::new();
    }

    let Some(outward) = normalized_terminal_cap_direction(-mouth.direction_xz) else {
        return Vec::new();
    };
    let paired_layers = first_carriageway.min(mouth.band_intervals.len() - last_carriageway - 1);
    let mut cap_bands = Vec::new();
    let mut inner_offset_m = 0.0;
    let mut next_terminal_source_band_index = mouth.band_intervals.len();

    for layer_index in 0..paired_layers {
        let left_band_index = first_carriageway - 1 - layer_index;
        let right_band_index = last_carriageway + 1 + layer_index;
        let left_band = &mouth.band_intervals[left_band_index];
        let right_band = &mouth.band_intervals[right_band_index];
        if left_band.band_kind != right_band.band_kind
            || left_band.band_kind == RoadSurfaceBandKind::Carriageway
        {
            break;
        }

        let depth_m = band_width_m(left_band).min(band_width_m(right_band));
        if depth_m <= f64::EPSILON {
            continue;
        }
        let outer_offset_m = inner_offset_m + depth_m;
        push_terminal_paired_cap_bands(
            &mut cap_bands,
            mouth,
            outward,
            next_terminal_source_band_index,
            left_band_index,
            right_band_index,
            inner_offset_m,
            outer_offset_m,
        );
        next_terminal_source_band_index += 1;
        inner_offset_m = outer_offset_m;
    }

    cap_bands
}

fn push_terminal_paired_cap_bands(
    cap_bands: &mut Vec<NodeTerminalCapBand>,
    mouth: &NodeInputMouth,
    outward: RoadVec2,
    source_band_index: usize,
    left_band_index: usize,
    right_band_index: usize,
    inner_offset_m: f64,
    outer_offset_m: f64,
) {
    let band_kind = mouth.band_intervals[left_band_index].band_kind;
    push_terminal_cap_band(
        cap_bands,
        source_band_index,
        band_kind,
        terminal_offset_boundary_path(mouth, left_band_index, left_band_index + 1, outward, 0.0),
        terminal_offset_boundary_path(
            mouth,
            left_band_index,
            left_band_index + 1,
            outward,
            outer_offset_m,
        ),
    );
    let closure_heights_m =
        terminal_paired_closure_height_anchors(mouth, left_band_index, right_band_index);
    push_terminal_cap_band(
        cap_bands,
        source_band_index,
        band_kind,
        terminal_offset_boundary_path_with_linear_height(
            mouth,
            left_band_index + 1,
            right_band_index,
            outward,
            inner_offset_m,
            closure_heights_m,
        ),
        terminal_offset_boundary_path_with_linear_height(
            mouth,
            left_band_index + 1,
            right_band_index,
            outward,
            outer_offset_m,
            closure_heights_m,
        ),
    );
    push_terminal_cap_band(
        cap_bands,
        source_band_index,
        band_kind,
        terminal_offset_boundary_path(mouth, right_band_index, right_band_index + 1, outward, 0.0),
        terminal_offset_boundary_path(
            mouth,
            right_band_index,
            right_band_index + 1,
            outward,
            outer_offset_m,
        ),
    );
}

fn push_terminal_cap_band(
    cap_bands: &mut Vec<NodeTerminalCapBand>,
    source_band_index: usize,
    band_kind: RoadSurfaceBandKind,
    inner_path_world: Option<Vec<RoadVec3>>,
    outer_path_world: Option<Vec<RoadVec3>>,
) {
    let Some(inner_path_world) = inner_path_world.and_then(clean_terminal_cap_path_world) else {
        return;
    };
    let Some(outer_path_world) = outer_path_world.and_then(clean_terminal_cap_path_world) else {
        return;
    };
    let Some(contour_world) = terminal_cap_contour_world(&inner_path_world, &outer_path_world)
    else {
        return;
    };

    cap_bands.push(NodeTerminalCapBand {
        source_band_index,
        band_kind,
        inner_path_world,
        outer_path_world,
        contour_world,
    });
}

fn terminal_offset_boundary_path(
    mouth: &NodeInputMouth,
    start_boundary_index: usize,
    end_boundary_index: usize,
    outward: RoadVec2,
    offset_m: f64,
) -> Option<Vec<RoadVec3>> {
    terminal_offset_boundary_path_with_linear_height(
        mouth,
        start_boundary_index,
        end_boundary_index,
        outward,
        offset_m,
        None,
    )
}

fn terminal_offset_boundary_path_with_linear_height(
    mouth: &NodeInputMouth,
    start_boundary_index: usize,
    end_boundary_index: usize,
    outward: RoadVec2,
    offset_m: f64,
    endpoint_heights_m: Option<(f64, f64)>,
) -> Option<Vec<RoadVec3>> {
    if start_boundary_index >= end_boundary_index
        || end_boundary_index >= mouth.boundary_rails.len()
    {
        return None;
    }

    let start_base = xz(endpoint_boundary_world(mouth, start_boundary_index)?);
    let end_base = xz(endpoint_boundary_world(mouth, end_boundary_index)?);
    let axis = end_base - start_base;
    let axis_len2 = axis.length_squared();
    let mut points = Vec::with_capacity(end_boundary_index - start_boundary_index + 1);

    for boundary_index in start_boundary_index..=end_boundary_index {
        let base = endpoint_boundary_world(mouth, boundary_index)?;
        let base_xz = xz(base);
        let t = if axis_len2 > f64::EPSILON {
            ((base_xz - start_base).dot(axis) / axis_len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let height_m = endpoint_heights_m.map_or(base.y, |(start_height_m, end_height_m)| {
            start_height_m + (end_height_m - start_height_m) * t
        });
        points.push(RoadVec3::new(
            base.x + outward.x * offset_m,
            height_m,
            base.z + outward.y * offset_m,
        ));
    }

    Some(points)
}

fn terminal_paired_closure_height_anchors(
    mouth: &NodeInputMouth,
    left_band_index: usize,
    right_band_index: usize,
) -> Option<(f64, f64)> {
    let left_height_m = endpoint_boundary_world(mouth, left_band_index)?.y;
    let right_height_m = endpoint_boundary_world(mouth, right_band_index + 1)?.y;
    Some((left_height_m, right_height_m))
}

fn endpoint_boundary_world(mouth: &NodeInputMouth, boundary_index: usize) -> Option<RoadVec3> {
    mouth
        .boundary_rails
        .get(boundary_index)
        .map(|rail| rail.endpoint_world)
}

fn band_width_m(band: &super::input::NodeInputBandInterval) -> f64 {
    xz(band.endpoint_start_world).distance(xz(band.endpoint_end_world))
}

fn canonicalize_terminal_cap_bands(cap_bands: &mut Vec<NodeTerminalCapBand>) {
    for cap_band in cap_bands.iter_mut() {
        quantize_terminal_cap_band_xz(cap_band);
        let Some(inner_path_world) =
            clean_terminal_cap_path_world(cap_band.inner_path_world.clone())
        else {
            cap_band.contour_world.clear();
            continue;
        };
        let Some(outer_path_world) =
            clean_terminal_cap_path_world(cap_band.outer_path_world.clone())
        else {
            cap_band.contour_world.clear();
            continue;
        };
        let Some(contour_world) = terminal_cap_contour_world(&inner_path_world, &outer_path_world)
        else {
            cap_band.contour_world.clear();
            continue;
        };
        cap_band.inner_path_world = inner_path_world;
        cap_band.outer_path_world = outer_path_world;
        cap_band.contour_world = contour_world;
    }
    cap_bands.retain(terminal_cap_band_has_quantized_area);
}

fn quantize_terminal_cap_band_xz(cap_band: &mut NodeTerminalCapBand) {
    for point in &mut cap_band.inner_path_world {
        *point = quantize_road_vec3_xz(*point);
    }
    for point in &mut cap_band.outer_path_world {
        *point = quantize_road_vec3_xz(*point);
    }
    for point in &mut cap_band.contour_world {
        *point = quantize_road_vec3_xz(*point);
    }
}

fn quantize_road_vec3_xz(point: RoadVec3) -> RoadVec3 {
    let point_xz = quantize_road_vec2_to_overlay_grid(xz(point));
    RoadVec3::new(point_xz.x, point.y, point_xz.y)
}

fn terminal_cap_contour_world(
    inner_path_world: &[RoadVec3],
    outer_path_world: &[RoadVec3],
) -> Option<Vec<RoadVec3>> {
    if inner_path_world.len() < 2 || outer_path_world.len() < 2 {
        return None;
    }
    let mut contour_world = inner_path_world.to_vec();
    contour_world.extend(outer_path_world.iter().rev().copied());
    clean_terminal_cap_contour_world(contour_world)
}

fn clean_terminal_cap_path_world(path_world: Vec<RoadVec3>) -> Option<Vec<RoadVec3>> {
    if path_world.len() < 2 {
        return None;
    }
    let polyline = cleaned_open_world_path_polyline(&path_world)?;
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

fn clean_terminal_cap_contour_world(contour_world: Vec<RoadVec3>) -> Option<Vec<RoadVec3>> {
    let raw = road_points_to_polyline(contour_world.iter().copied().map(xz), true);
    let mut cleaned =
        RoadPolyline::create_from_remove_repeat(&raw, TERMINAL_CAP_POLYLINE_POINT_EQUAL_EPS_M);
    if let Some(reduced) = cleaned.remove_redundant(TERMINAL_CAP_POLYLINE_POINT_EQUAL_EPS_M) {
        cleaned = reduced;
    }
    if cleaned.vertex_count() < 3
        || cleaned.area().abs() <= f64::from(NODE_OVERLAY_MIN_AREA_M2)
        || cleaned.scan_for_self_intersect()
    {
        return None;
    }
    let mut cleaned_world = polyline_to_road_points(&cleaned)
        .into_iter()
        .map(|point_xz| {
            let height_m = height_on_world_path(point_xz, &contour_world)?;
            Some(RoadVec3::new(point_xz.x, height_m, point_xz.y))
        })
        .collect::<Option<Vec<_>>>()?;
    remove_repeated_road_vec3_points(&mut cleaned_world);
    (cleaned_world.len() >= 3).then_some(cleaned_world)
}

fn cleaned_open_world_path_polyline(path_world: &[RoadVec3]) -> Option<RoadPolyline> {
    let raw = road_points_to_polyline(path_world.iter().copied().map(xz), false);
    let cleaned =
        RoadPolyline::create_from_remove_repeat(&raw, TERMINAL_CAP_POLYLINE_POINT_EQUAL_EPS_M);
    (cleaned.vertex_count() >= 2).then_some(cleaned)
}

fn height_on_world_path(point_xz: RoadVec2, path_world: &[RoadVec3]) -> Option<f64> {
    let key = quantized_xz_key(point_xz);
    for point_world in path_world {
        if quantized_xz_key(xz(*point_world)) == key {
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
    let start_xz = xz(start_world);
    let end_xz = xz(end_world);
    let axis = end_xz - start_xz;
    let axis_len2 = axis.length_squared();
    if axis_len2 <= f64::EPSILON {
        return None;
    }
    let t = ((point_xz - start_xz).dot(axis) / axis_len2).clamp(0.0, 1.0);
    let closest = start_xz + axis * t;
    if closest.distance_squared(point_xz)
        > TERMINAL_CAP_HEIGHT_EDGE_EPS_M * TERMINAL_CAP_HEIGHT_EDGE_EPS_M
    {
        return None;
    }
    Some(start_world.y + (end_world.y - start_world.y) * t)
}

fn terminal_cap_band_has_quantized_area(cap_band: &NodeTerminalCapBand) -> bool {
    let raw = road_points_to_polyline(cap_band.contour_world.iter().copied().map(xz), true);
    let contour =
        RoadPolyline::create_from_remove_repeat(&raw, TERMINAL_CAP_POLYLINE_POINT_EQUAL_EPS_M);
    contour.vertex_count() >= 3
        && contour.area().abs() > f64::from(NODE_OVERLAY_MIN_AREA_M2)
        && !contour.scan_for_self_intersect()
}

fn remove_repeated_road_vec3_points(points: &mut Vec<RoadVec3>) {
    points.dedup_by(|a, b| quantized_xz_key(xz(*a)) == quantized_xz_key(xz(*b)));
    if points.len() > 1
        && quantized_xz_key(xz(points[0]))
            == quantized_xz_key(xz(*points.last().expect("points are non-empty")))
    {
        points.pop();
    }
}

fn normalized_terminal_cap_direction(direction: RoadVec2) -> Option<RoadVec2> {
    let length = direction.length();
    (length > TERMINAL_CAP_POLYLINE_POINT_EQUAL_EPS_M).then_some(direction / length)
}

fn quantized_xz_key(point: RoadVec2) -> (i64, i64) {
    (
        (point.x * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
        (point.y * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
    )
}

fn xz(point: RoadVec3) -> RoadVec2 {
    RoadVec2::new(point.x, point.z)
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
            Vector3::new(x, 4.12, -3.5),
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
            uses_sampled_band_domain_paths: false,
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
    fn terminal_cap_adapter_uses_paired_outer_source_heights() {
        let input = terminal_input(symmetric_profile_x(0.0, Vector2::RIGHT));
        let mouth = &input.mouths[0];
        let cap_bands_by_mouth = terminal_cap_bands_by_mouth(&input);
        let center_boundary = mouth.boundary_rails[3].endpoint_world;
        let expected_height_m = mouth.boundary_rails[1]
            .endpoint_world
            .y
            .max(mouth.boundary_rails[5].endpoint_world.y);

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
    fn car_only_terminal_emits_no_non_road_cap() {
        let input = terminal_input(car_only_profile_x(0.0, Vector2::RIGHT));
        let cap_bands_by_mouth = terminal_cap_bands_by_mouth(&input);

        assert!(cap_bands_by_mouth.iter().flatten().next().is_none());
    }
}
