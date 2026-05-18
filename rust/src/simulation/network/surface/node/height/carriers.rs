//! Height-carrier construction from source bands and generated contours.

use super::model::*;
use super::source_edges::*;
use super::triangles::*;
use super::vertices::canonical_height_vertices;
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
) -> Result<Vec<RoadVec3>, NodeHeightFieldError> {
    if interval.start_path_world.is_empty() && interval.end_path_world.is_empty() {
        return Ok(vec![
            interval.mouth_start_world,
            interval.mouth_end_world,
            interval.endpoint_end_world,
            interval.endpoint_start_world,
        ]);
    }
    let (start_path_world, end_path_world) = explicit_source_band_height_paths(id, interval)?;
    Ok(start_path_world.into_iter().chain(end_path_world).collect())
}

pub(super) fn explicit_source_band_height_paths(
    id: NodeBandHeightFieldId,
    interval: &NodeInputBandInterval,
) -> Result<(Vec<RoadVec3>, Vec<RoadVec3>), NodeHeightFieldError> {
    if interval.start_path_world.len() != interval.end_path_world.len() {
        return Err(invalid_source_band_height_carrier_error(
            id,
            interval.band_kind,
            "mismatched_source_band_path_lengths",
        ));
    }
    let start_canonical_len =
        validate_source_band_height_path(id, interval, &interval.start_path_world)?;
    let end_canonical_len =
        validate_source_band_height_path(id, interval, &interval.end_path_world)?;
    if start_canonical_len != end_canonical_len {
        return Err(invalid_source_band_height_carrier_error(
            id,
            interval.band_kind,
            "mismatched_source_band_canonical_path_lengths",
        ));
    }
    Ok((
        interval.start_path_world.clone(),
        interval.end_path_world.clone(),
    ))
}

fn validate_source_band_height_path(
    id: NodeBandHeightFieldId,
    interval: &NodeInputBandInterval,
    path_world: &[RoadVec3],
) -> Result<usize, NodeHeightFieldError> {
    canonical_height_vertices(path_world)
        .map_err(|error| {
            invalid_source_band_height_carrier_error(
                id,
                interval.band_kind,
                error.diagnostic_reason(),
            )
        })
        .map(|vertices| vertices.len())
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
