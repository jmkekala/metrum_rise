// SPDX-License-Identifier: GPL-2.0-only

//! Height-carrier construction from source bands and generated contours.

use super::model::*;
use super::seams::quantize_source_height_m;
use super::triangles::*;
use super::vertices::{
    canonical_height_vertices, closed_height_contour_edges_from_vertices,
    open_height_contour_edges_from_vertices, push_height_contour_edge,
};
use super::*;

pub(super) fn interval_height_carrier(
    id: NodeBandHeightFieldId,
    interval: &NodeInputBandInterval,
) -> Result<Vec<NodeBandHeightTriangle>, NodeHeightFieldError> {
    if interval.start_path_world.is_empty() && interval.end_path_world.is_empty() {
        let points = [
            interval.mouth_start_world,
            interval.mouth_end_world,
            interval.endpoint_end_world,
            interval.endpoint_start_world,
        ];
        return height_triangles_from_vertices(&points).map_err(|error| {
            invalid_source_band_height_carrier_error(
                id,
                interval.band_kind,
                error.diagnostic_reason(),
            )
        });
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
    Ok(triangles)
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

pub(super) fn interval_height_contour_edges(
    id: NodeBandHeightFieldId,
    interval: &NodeInputBandInterval,
) -> Result<Vec<NodeBandHeightContourEdge>, NodeHeightFieldError> {
    if interval.start_path_world.is_empty() && interval.end_path_world.is_empty() {
        let points = [
            interval.mouth_start_world,
            interval.mouth_end_world,
            interval.endpoint_end_world,
            interval.endpoint_start_world,
        ];
        return closed_height_contour_edges_from_vertices(&points).map_err(|error| {
            invalid_source_band_height_carrier_error(
                id,
                interval.band_kind,
                error.diagnostic_reason(),
            )
        });
    }

    let (start_path_world, end_path_world) = explicit_source_band_height_paths(id, interval)?;
    let mut edges =
        open_height_contour_edges_from_vertices(&start_path_world).map_err(|error| {
            invalid_source_band_height_carrier_error(
                id,
                interval.band_kind,
                error.diagnostic_reason(),
            )
        })?;
    edges.extend(
        open_height_contour_edges_from_vertices(&end_path_world).map_err(|error| {
            invalid_source_band_height_carrier_error(
                id,
                interval.band_kind,
                error.diagnostic_reason(),
            )
        })?,
    );
    if let (Some(start), Some(end)) = (
        start_path_world.first().copied(),
        end_path_world.first().copied(),
    ) {
        push_height_contour_edge(
            &mut edges,
            (
                quantize_road_vec2_to_overlay_grid(xz(start)),
                quantize_source_height_m(start.y),
            ),
            (
                quantize_road_vec2_to_overlay_grid(xz(end)),
                quantize_source_height_m(end.y),
            ),
        );
    }
    if let (Some(start), Some(end)) = (
        start_path_world.last().copied(),
        end_path_world.last().copied(),
    ) {
        push_height_contour_edge(
            &mut edges,
            (
                quantize_road_vec2_to_overlay_grid(xz(start)),
                quantize_source_height_m(start.y),
            ),
            (
                quantize_road_vec2_to_overlay_grid(xz(end)),
                quantize_source_height_m(end.y),
            ),
        );
    }
    Ok(edges)
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
