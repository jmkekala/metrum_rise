//! Parcel rectangle construction and world-bounds checks.

use super::polyline::{sample_pos_on_polyline, sample_tangent_on_polyline};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::parcels::{ParcelGeometry, ZoningParcel};
use godot::prelude::Vector2;

pub(crate) fn geometry_from_attachment(
    graph: &RegionGraph,
    edge_idx: usize,
    side: i8,
    frontage_center_t: f32,
    frontage_m: f32,
    depth_m: f32,
) -> ParcelGeometry {
    let edge = graph.edge(edge_idx);
    let s_m = frontage_center_t.clamp(0.0, 1.0) * edge.physical_length;
    let road_pos = sample_pos_on_polyline(&edge.physical_geometry, edge.physical_length, s_m);
    let tangent = sample_tangent_on_polyline(&edge.physical_geometry, edge.physical_length, s_m);
    let normal = Vector2::new(tangent.y, -tangent.x) * side as f32;
    let front_center = road_pos + normal * (edge.width * 0.5 + crate::config::SIDEWALK_WIDTH);
    let center = front_center + normal * (depth_m * 0.5);
    let half_frontage = frontage_m * 0.5;
    let front_left = front_center - tangent * half_frontage;
    let front_right = front_center + tangent * half_frontage;
    let rear_right = front_right + normal * depth_m;
    let rear_left = front_left + normal * depth_m;
    let corners = [front_left, front_right, rear_right, rear_left];
    let (aabb_min, aabb_max) = aabb_for_corners(&corners);
    ParcelGeometry {
        edge_idx,
        side,
        frontage_center_t,
        frontage_m,
        depth_m,
        front_center,
        center,
        tangent,
        normal,
        corners,
        aabb_min,
        aabb_max,
    }
}

pub(crate) fn geometry_inside_world(
    geometry: &ParcelGeometry,
    world_width_m: f32,
    world_height_m: f32,
) -> bool {
    let half_w = world_width_m * 0.5;
    let half_h = world_height_m * 0.5;
    geometry.corners.iter().all(|corner| {
        corner.x >= -half_w && corner.x <= half_w && corner.y >= -half_h && corner.y <= half_h
    })
}

pub(crate) fn geometry_for_parcel(parcel: &ZoningParcel) -> ParcelGeometry {
    ParcelGeometry {
        edge_idx: parcel.edge_idx(),
        side: parcel.side(),
        frontage_center_t: parcel.frontage_center_t(),
        frontage_m: parcel.frontage_m(),
        depth_m: parcel.depth_m(),
        front_center: parcel.front_center(),
        center: parcel.center(),
        tangent: parcel.tangent(),
        normal: parcel.normal(),
        corners: parcel.corners(),
        aabb_min: parcel.aabb_min(),
        aabb_max: parcel.aabb_max(),
    }
}

fn aabb_for_corners(corners: &[Vector2; 4]) -> (Vector2, Vector2) {
    let mut min = Vector2::new(f32::INFINITY, f32::INFINITY);
    let mut max = Vector2::new(f32::NEG_INFINITY, f32::NEG_INFINITY);
    for corner in corners {
        min.x = min.x.min(corner.x);
        min.y = min.y.min(corner.y);
        max.x = max.x.max(corner.x);
        max.y = max.y.max(corner.y);
    }
    (min, max)
}
