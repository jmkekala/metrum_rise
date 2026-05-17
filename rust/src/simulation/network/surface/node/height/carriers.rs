//! Height-carrier construction from source bands and generated contours.

use super::model::*;
use super::source_edges::*;
use super::triangles::*;
use super::vertices::validate_canonical_height_vertices;
use super::*;

pub(super) fn interval_height_carrier(
    id: NodeBandHeightFieldId,
    interval: &NodeInputBandInterval,
    source_support_points: Option<&[RoadVec3]>,
) -> Result<(Vec<NodeBandHeightTriangle>, Vec<NodeBandHeightEdge>), NodeHeightFieldError> {
    if interval.start_path_world.is_empty() && interval.end_path_world.is_empty() {
        let points = [
            interval.mouth_start_world,
            interval.mouth_end_world,
            interval.endpoint_end_world,
            interval.endpoint_start_world,
        ];
        return Ok((
            height_triangles_from_vertices(&points).map_err(|error| {
                invalid_source_band_height_carrier_error(
                    id,
                    interval.band_kind,
                    error.diagnostic_reason(),
                )
            })?,
            height_edges_from_vertices(&points).map_err(|error| {
                invalid_source_band_height_carrier_error(
                    id,
                    interval.band_kind,
                    error.diagnostic_reason(),
                )
            })?,
        ));
    }
    let (start_path_world, end_path_world) =
        explicit_source_band_height_paths(id, interval, source_support_points)?;
    validate_canonical_height_vertices(&start_path_world).map_err(|error| {
        invalid_source_band_height_carrier_error(id, interval.band_kind, error.diagnostic_reason())
    })?;
    validate_canonical_height_vertices(&end_path_world).map_err(|error| {
        invalid_source_band_height_carrier_error(id, interval.band_kind, error.diagnostic_reason())
    })?;
    if start_path_world.len() < 2 {
        return Err(invalid_source_band_height_carrier_error(
            id,
            interval.band_kind,
            "too_few_source_band_path_points",
        ));
    }
    let triangles =
        path_band_height_triangles(&start_path_world, &end_path_world).ok_or_else(|| {
            invalid_source_band_height_carrier_error(
                id,
                interval.band_kind,
                "degenerate_source_band_height_triangles",
            )
        })?;
    let contour_edges = path_band_height_edges(&start_path_world, &end_path_world)
        .map_err(|error| {
            invalid_source_band_height_carrier_error(
                id,
                interval.band_kind,
                error.diagnostic_reason(),
            )
        })?
        .ok_or_else(|| {
            invalid_source_band_height_carrier_error(
                id,
                interval.band_kind,
                "degenerate_source_band_height_edges",
            )
        })?;
    Ok((triangles, contour_edges))
}

pub(super) fn interval_height_carrier_vertices(
    id: NodeBandHeightFieldId,
    interval: &NodeInputBandInterval,
    source_support_points: Option<&[RoadVec3]>,
) -> Result<Vec<RoadVec3>, NodeHeightFieldError> {
    if interval.start_path_world.is_empty() && interval.end_path_world.is_empty() {
        return Ok(vec![
            interval.mouth_start_world,
            interval.mouth_end_world,
            interval.endpoint_end_world,
            interval.endpoint_start_world,
        ]);
    }
    let (start_path_world, end_path_world) =
        explicit_source_band_height_paths(id, interval, source_support_points)?;
    Ok(start_path_world.into_iter().chain(end_path_world).collect())
}

pub(super) fn explicit_source_band_height_paths(
    id: NodeBandHeightFieldId,
    interval: &NodeInputBandInterval,
    source_support_points: Option<&[RoadVec3]>,
) -> Result<(Vec<RoadVec3>, Vec<RoadVec3>), NodeHeightFieldError> {
    if interval.start_path_world.len() == interval.end_path_world.len() {
        return Ok((
            interval.start_path_world.clone(),
            interval.end_path_world.clone(),
        ));
    }
    if interval.start_path_world.len() > 2
        && source_height_path_is_endpoint_chord(
            &interval.end_path_world,
            interval.mouth_end_world,
            interval.endpoint_end_world,
        )
    {
        return Ok((
            interval.start_path_world.clone(),
            materialized_height_chord(
                id,
                interval.band_kind,
                interval.mouth_end_world,
                interval.endpoint_end_world,
                interval.start_path_world.len(),
                source_support_points,
            )?,
        ));
    }
    if interval.end_path_world.len() > 2
        && source_height_path_is_endpoint_chord(
            &interval.start_path_world,
            interval.mouth_start_world,
            interval.endpoint_start_world,
        )
    {
        return Ok((
            materialized_height_chord(
                id,
                interval.band_kind,
                interval.mouth_start_world,
                interval.endpoint_start_world,
                interval.end_path_world.len(),
                source_support_points,
            )?,
            interval.end_path_world.clone(),
        ));
    }
    Err(invalid_source_band_height_carrier_error(
        id,
        interval.band_kind,
        "mismatched_source_band_path_lengths",
    ))
}

pub(super) fn source_height_path_is_endpoint_chord(
    path_world: &[RoadVec3],
    mouth_world: RoadVec3,
    endpoint_world: RoadVec3,
) -> bool {
    path_world.len() == 2
        && source_height_points_match(path_world[0], mouth_world)
        && source_height_points_match(path_world[1], endpoint_world)
}

pub(super) fn source_height_points_match(a: RoadVec3, b: RoadVec3) -> bool {
    SurfaceXzKey::from_world_xz(a) == SurfaceXzKey::from_world_xz(b)
        && SurfaceHeightMmKey::from_m_f64(a.y) == SurfaceHeightMmKey::from_m_f64(b.y)
}

pub(super) fn materialized_height_chord(
    id: NodeBandHeightFieldId,
    source_kind: RoadSurfaceBandKind,
    start: RoadVec3,
    end: RoadVec3,
    point_count: usize,
    source_support_points: Option<&[RoadVec3]>,
) -> Result<Vec<RoadVec3>, NodeHeightFieldError> {
    let Some(source_support_points) = source_support_points else {
        return Err(invalid_source_band_height_carrier_error(
            id,
            source_kind,
            "missing_materialized_source_chord_points",
        ));
    };
    if point_count < 2 {
        return Ok(vec![start, end]);
    }
    let start_key = SurfaceXzKey::from_world_xz(start).raw_tuple();
    let end_key = SurfaceXzKey::from_world_xz(end).raw_tuple();
    let Some(denominator) = chord_parameter_numerator(end_key, start_key, end_key) else {
        return Err(invalid_source_band_height_carrier_error(
            id,
            source_kind,
            "invalid_materialized_source_chord_point",
        ));
    };
    let mut points = BTreeMap::<i128, RoadVec3>::new();
    for point in source_support_points.iter().copied() {
        let point_xz = xz(point);
        let point_key = SurfaceXzKey::from_road_xz(point_xz).raw_tuple();
        if !raw_tuple_key_lies_exactly_on_segment(point_key, start_key, end_key) {
            continue;
        }
        let Some(parameter) = chord_parameter_numerator(point_key, start_key, end_key) else {
            continue;
        };
        let ordered_parameter = if denominator < 0 {
            -parameter
        } else {
            parameter
        };
        let expected_height_m =
            height_on_materialized_chord(point_key, start_key, end_key, start.y, end.y)
                .ok_or_else(|| {
                    invalid_source_band_height_carrier_error(
                        id,
                        source_kind,
                        "invalid_materialized_source_chord_point",
                    )
                })?;
        if SurfaceHeightMmKey::from_m_f64(point.y)
            != SurfaceHeightMmKey::from_m_f64(expected_height_m)
        {
            return Err(invalid_source_band_height_carrier_error(
                id,
                source_kind,
                "conflicting_materialized_source_chord_height",
            ));
        }
        points.insert(
            ordered_parameter,
            RoadVec3::new(point_xz.x, point.y, point_xz.y),
        );
    }
    if points.len() != point_count {
        return Err(invalid_source_band_height_carrier_error(
            id,
            source_kind,
            "missing_materialized_source_chord_points",
        ));
    }
    Ok(points.into_values().collect())
}

fn chord_parameter_numerator(
    point: NodeHeightSourcePointKey,
    start: NodeHeightSourcePointKey,
    end: NodeHeightSourcePointKey,
) -> Option<i128> {
    let dx = end.0 - start.0;
    let dz = end.1 - start.1;
    if dx == 0 && dz == 0 {
        return None;
    }
    Some(if dx.abs() >= dz.abs() {
        i128::from(point.0 - start.0)
    } else {
        i128::from(point.1 - start.1)
    })
}

fn height_on_materialized_chord(
    point: NodeHeightSourcePointKey,
    start: NodeHeightSourcePointKey,
    end: NodeHeightSourcePointKey,
    start_height_m: f64,
    end_height_m: f64,
) -> Option<f64> {
    let denominator = chord_parameter_numerator(end, start, end)?;
    let numerator = chord_parameter_numerator(point, start, end)?;
    if denominator == 0 {
        return None;
    }
    let t = numerator as f64 / denominator as f64;
    if !(0.0..=1.0).contains(&t) {
        return None;
    }
    Some(start_height_m + (end_height_m - start_height_m) * t)
}

pub(super) fn invalid_source_band_height_carrier_error(
    id: NodeBandHeightFieldId,
    source_kind: RoadSurfaceBandKind,
    reason: &'static str,
) -> NodeHeightFieldError {
    NodeHeightFieldError::InvalidSourceBandHeightCarrier {
        mouth_order_index: id.mouth_order_index(),
        band_index: id.band_index(),
        source_kind,
        height_field_id: id,
        reason,
    }
}
