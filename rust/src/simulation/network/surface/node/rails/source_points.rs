//! Source height-carrier point collection for generated node rails.

use super::*;

pub(super) fn interval_height_carrier_points(
    interval: &NodeInputBandInterval,
) -> impl Iterator<Item = RoadVec3> + '_ {
    [
        interval.endpoint_start_world,
        interval.endpoint_end_world,
        interval.mouth_end_world,
        interval.mouth_start_world,
    ]
    .into_iter()
    .chain(interval.start_path_world.iter().copied())
    .chain(interval.end_path_world.iter().copied())
}

pub(super) fn push_band_height_carrier_points(
    points_by_source: &mut BTreeMap<(RoadSurfaceBandKind, usize, usize), Vec<RoadVec2>>,
    mouth_order_index: usize,
    source_band_index: usize,
    kind: RoadSurfaceBandKind,
    points_world: impl IntoIterator<Item = RoadVec3>,
) {
    let points = points_by_source
        .entry((kind, mouth_order_index, source_band_index))
        .or_default();
    for point in points_world {
        push_road_path_point(points, xz(point));
    }
}
