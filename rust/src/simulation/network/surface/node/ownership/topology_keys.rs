//! Canonical topology key adapters for node boolean ownership.

use super::super::NodeOverlayPoint;
use super::super::backend::RoadVec2;
use super::super::keys::{SurfaceXzKey, SurfaceXzSegmentKey};
use super::super::segments::{
    key_collinear_with_overlay_grid_segment, key_collinear_with_segment, raw_tuple_key,
    raw_tuple_key_lies_exactly_on_segment, raw_tuple_key_lies_on_segment,
    raw_tuple_segment_parameter_key,
};
use super::NodeOwnedRegionArrangementKey;

pub(crate) type NodeOwnershipPointKey = (i64, i64);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct OwnedRegionEdgeKey {
    pub(super) start: NodeOwnershipPointKey,
    pub(super) end: NodeOwnershipPointKey,
}

impl OwnedRegionEdgeKey {
    pub(super) fn new(a: NodeOwnershipPointKey, b: NodeOwnershipPointKey) -> Self {
        let segment = SurfaceXzSegmentKey::new(
            SurfaceXzKey::from_raw_tuple(a),
            SurfaceXzKey::from_raw_tuple(b),
        );
        Self {
            start: segment.start().raw_tuple(),
            end: segment.end().raw_tuple(),
        }
    }
}

impl NodeOwnedRegionArrangementKey {
    #[cfg(test)]
    pub(crate) fn from_point(point: RoadVec2) -> Self {
        Self::from_ownership_key(ownership_key_from_road_point(point))
    }

    pub(crate) fn from_ownership_key(point: NodeOwnershipPointKey) -> Self {
        Self {
            x_key: point.0,
            z_key: point.1,
        }
    }

    pub(crate) fn x_mm(self) -> i64 {
        ownership_coordinate_key_to_mm(self.x_key)
    }

    pub(crate) fn z_mm(self) -> i64 {
        ownership_coordinate_key_to_mm(self.z_key)
    }

    pub(crate) fn raw_tuple(self) -> NodeOwnershipPointKey {
        (self.x_key, self.z_key)
    }
}

pub(super) fn canonical_source_indices(sources: impl IntoIterator<Item = usize>) -> Vec<usize> {
    let mut sources = sources.into_iter().collect::<Vec<_>>();
    sources.sort_unstable();
    sources.dedup();
    sources
}

pub(super) fn segment_parameter_key(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    point: NodeOwnershipPointKey,
) -> i128 {
    raw_tuple_segment_parameter_key(start, end, point)
}

pub(super) fn road_point_from_key(point: NodeOwnershipPointKey) -> RoadVec2 {
    SurfaceXzKey::from_raw_tuple(point).to_road_xz()
}

pub(super) fn overlay_point_from_key(point: NodeOwnershipPointKey) -> NodeOverlayPoint {
    let point = SurfaceXzKey::from_raw_tuple(point).to_road_xz();
    [point.x, point.y]
}

pub(super) fn ownership_mm_key(point: NodeOwnershipPointKey) -> NodeOwnershipPointKey {
    (
        ownership_coordinate_key_to_mm(point.0),
        ownership_coordinate_key_to_mm(point.1),
    )
}

pub(super) fn point_key_collinear_with_edge(
    point: NodeOwnershipPointKey,
    edge_start: NodeOwnershipPointKey,
    edge_end: NodeOwnershipPointKey,
) -> bool {
    key_collinear_with_segment(
        raw_tuple_key(point),
        raw_tuple_key(edge_start),
        raw_tuple_key(edge_end),
    )
}

pub(super) fn point_key_collinear_with_edge_on_overlay_grid(
    point: NodeOwnershipPointKey,
    edge_start: NodeOwnershipPointKey,
    edge_end: NodeOwnershipPointKey,
) -> bool {
    key_collinear_with_overlay_grid_segment(
        raw_tuple_key(point),
        raw_tuple_key(edge_start),
        raw_tuple_key(edge_end),
    )
}

pub(super) fn point_key_lies_on_segment(
    point: NodeOwnershipPointKey,
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
) -> bool {
    raw_tuple_key_lies_on_segment(point, start, end)
}

pub(super) fn point_key_lies_exactly_on_segment(
    point: NodeOwnershipPointKey,
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
) -> bool {
    raw_tuple_key_lies_exactly_on_segment(point, start, end)
}

fn ownership_coordinate_key_to_mm(value: i64) -> i64 {
    SurfaceXzKey::coordinate_key_to_mm(value)
}

pub(super) fn ownership_key_from_overlay_point(point: NodeOverlayPoint) -> NodeOwnershipPointKey {
    SurfaceXzKey::from_overlay_point(point).raw_tuple()
}

pub(super) fn ownership_key_from_road_point(point: RoadVec2) -> NodeOwnershipPointKey {
    SurfaceXzKey::from_road_xz(point).raw_tuple()
}
