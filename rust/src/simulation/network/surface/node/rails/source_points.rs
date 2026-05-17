//! Source height-carrier point collection for generated node rails.

use super::super::RoadSurfaceBandKind;
use super::super::backend::{RoadVec3, road_vec3_xz as xz};
use super::super::input::NodeInputBandInterval;
use super::super::keys::SurfaceHeightMmKey;
use super::contours::subdivided_world_chord;
use super::geometry::road_point_key;
use std::collections::BTreeMap;

pub(super) fn interval_height_carrier_points(interval: &NodeInputBandInterval) -> Vec<RoadVec3> {
    let mut points = [
        interval.endpoint_start_world,
        interval.endpoint_end_world,
        interval.mouth_end_world,
        interval.mouth_start_world,
    ]
    .into_iter()
    .chain(interval.start_path_world.iter().copied())
    .chain(interval.end_path_world.iter().copied())
    .collect::<Vec<_>>();
    if interval.start_path_world.len() > 2
        && interval.end_path_world.len() == 2
        && interval.end_path_world[0] == interval.mouth_end_world
        && interval.end_path_world[1] == interval.endpoint_end_world
    {
        points.extend(subdivided_world_chord(
            interval.mouth_end_world,
            interval.endpoint_end_world,
            interval.start_path_world.len(),
        ));
    }
    if interval.end_path_world.len() > 2
        && interval.start_path_world.len() == 2
        && interval.start_path_world[0] == interval.mouth_start_world
        && interval.start_path_world[1] == interval.endpoint_start_world
    {
        points.extend(subdivided_world_chord(
            interval.mouth_start_world,
            interval.endpoint_start_world,
            interval.end_path_world.len(),
        ));
    }
    points
}

pub(super) fn push_band_height_carrier_points(
    points_by_source: &mut BTreeMap<(RoadSurfaceBandKind, usize, usize), Vec<RoadVec3>>,
    mouth_order_index: usize,
    source_band_index: usize,
    kind: RoadSurfaceBandKind,
    points_world: impl IntoIterator<Item = RoadVec3>,
) {
    let points = points_by_source
        .entry((kind, mouth_order_index, source_band_index))
        .or_default();
    for point in points_world {
        let point_key = road_point_key(xz(point));
        if let Some(existing) = points
            .iter()
            .find(|existing| road_point_key(xz(**existing)) == point_key)
        {
            if SurfaceHeightMmKey::from_m_f64(existing.y) == SurfaceHeightMmKey::from_m_f64(point.y)
            {
                continue;
            }
        }
        points.push(point);
    }
}
