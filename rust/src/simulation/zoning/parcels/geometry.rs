//! Parcel rectangle geometry, overlap tests, and road-corridor conflict checks.

mod bounds;
mod overlap;
mod polyline;
mod road_overlap;
mod spatial;

pub(crate) use bounds::{geometry_for_parcel, geometry_from_attachment, geometry_inside_world};
pub(crate) use overlap::{
    geometries_have_overlap, geometries_overlap, point_inside_parcel, rectangles_overlap_geometry,
    segment_touches_parcel,
};
pub(super) use polyline::sample_pos_on_polyline;
pub(super) use spatial::{chunk_key, chunks_for_aabb};

pub(crate) use road_overlap::{any_geometry_overlaps_road, geometry_overlaps_road};
