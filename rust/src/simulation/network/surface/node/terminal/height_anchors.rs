//! Terminal-cap height anchors derived from endpoint profile rails.

use super::*;
use crate::simulation::network::surface::keys::SurfaceXzKey;

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
    let rail = mouth.boundary_rails.get(boundary_index)?;
    let source_boundary = endpoint_source_boundary_world(mouth, boundary_index)?;
    Some(RoadVec3::new(
        source_boundary.x,
        rail.endpoint_world.y,
        source_boundary.z,
    ))
}

fn endpoint_source_boundary_world(
    mouth: &NodeInputMouth,
    boundary_index: usize,
) -> Option<RoadVec3> {
    let left = boundary_index
        .checked_sub(1)
        .and_then(|index| mouth.band_intervals.get(index))
        .map(|band| band.endpoint_end_world);
    let right = mouth
        .band_intervals
        .get(boundary_index)
        .map(|band| band.endpoint_start_world);

    match (left, right) {
        (Some(left), Some(right)) => endpoint_source_boundary_pair(left, right),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => mouth
            .boundary_rails
            .get(boundary_index)
            .map(|rail| rail.endpoint_world),
    }
}

fn endpoint_source_boundary_pair(left: RoadVec3, right: RoadVec3) -> Option<RoadVec3> {
    (SurfaceXzKey::from_world_xz(left) == SurfaceXzKey::from_world_xz(right)).then_some(left)
}
