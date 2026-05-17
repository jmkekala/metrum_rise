//! Height-carrier construction from source bands and generated contours.

use super::model::*;
use super::source_edges::*;
use super::triangles::*;
use super::*;

pub(super) fn interval_height_carrier(
    id: NodeBandHeightFieldId,
    interval: &NodeInputBandInterval,
) -> Result<(Vec<NodeBandHeightTriangle>, Vec<NodeBandHeightEdge>), NodeHeightFieldError> {
    if interval.start_path_world.is_empty() && interval.end_path_world.is_empty() {
        let points = [
            interval.mouth_start_world,
            interval.mouth_end_world,
            interval.endpoint_end_world,
            interval.endpoint_start_world,
        ];
        return Ok((
            height_triangles_from_vertices(&points),
            height_edges_from_vertices(&points),
        ));
    }
    let (start_path_world, end_path_world) = explicit_source_band_height_paths(id, interval)?;
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
    let contour_edges =
        path_band_height_edges(&start_path_world, &end_path_world).ok_or_else(|| {
            invalid_source_band_height_carrier_error(
                id,
                interval.band_kind,
                "degenerate_source_band_height_edges",
            )
        })?;
    Ok((triangles, contour_edges))
}

pub(super) fn explicit_source_band_height_paths(
    id: NodeBandHeightFieldId,
    interval: &NodeInputBandInterval,
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
            subdivided_height_chord(
                interval.mouth_end_world,
                interval.endpoint_end_world,
                interval.start_path_world.len(),
            ),
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
            subdivided_height_chord(
                interval.mouth_start_world,
                interval.endpoint_start_world,
                interval.end_path_world.len(),
            ),
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

pub(super) fn subdivided_height_chord(
    start: RoadVec3,
    end: RoadVec3,
    point_count: usize,
) -> Vec<RoadVec3> {
    if point_count < 2 {
        return vec![start, end];
    }
    let denominator = (point_count - 1) as f64;
    (0..point_count)
        .map(|index| {
            let t = index as f64 / denominator;
            RoadVec3::new(
                start.x + (end.x - start.x) * t,
                start.y + (end.y - start.y) * t,
                start.z + (end.z - start.z) * t,
            )
        })
        .collect()
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
