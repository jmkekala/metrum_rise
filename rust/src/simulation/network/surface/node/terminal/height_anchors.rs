//! Terminal-cap height anchors derived from endpoint profile rails.

use super::*;

pub(super) fn terminal_side_band_height_anchors(
    mouth: &NodeInputMouth,
    band_index: usize,
) -> Option<(f64, f64)> {
    let band = mouth.band_intervals.get(band_index)?;
    Some((band.endpoint_start_world.y, band.endpoint_end_world.y))
}

pub(super) fn terminal_end_band_inner_height_anchors(
    mouth: &NodeInputMouth,
    left_band_index: usize,
    right_band_index: usize,
) -> Option<(f64, f64)> {
    let left_band = mouth.band_intervals.get(left_band_index)?;
    let right_band = mouth.band_intervals.get(right_band_index)?;
    Some((
        left_band.endpoint_end_world.y,
        right_band.endpoint_start_world.y,
    ))
}

pub(super) fn terminal_end_band_outer_height_anchors(
    mouth: &NodeInputMouth,
    left_band_index: usize,
    right_band_index: usize,
) -> Option<(f64, f64)> {
    let left_band = mouth.band_intervals.get(left_band_index)?;
    let right_band = mouth.band_intervals.get(right_band_index)?;
    let left_height_m = left_band.endpoint_start_world.y;
    let right_height_m = right_band.endpoint_end_world.y;
    Some((left_height_m, right_height_m))
}

pub(super) fn endpoint_boundary_world(
    mouth: &NodeInputMouth,
    boundary_index: usize,
) -> Option<RoadVec3> {
    mouth
        .boundary_rails
        .get(boundary_index)
        .map(|rail| rail.endpoint_world)
}
