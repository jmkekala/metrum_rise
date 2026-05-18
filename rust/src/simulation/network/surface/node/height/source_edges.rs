//! Canonical source-height support keys.

use super::model::NodeHeightSourcePointKey;
use super::*;

pub(super) fn height_source_point_key(point: RoadVec2) -> NodeHeightSourcePointKey {
    SurfaceXzKey::from_road_xz(point).raw_tuple()
}
