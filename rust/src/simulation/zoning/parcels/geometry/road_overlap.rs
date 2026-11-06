// SPDX-License-Identifier: GPL-2.0-only

//! Road-corridor conflict checks for parcel placement.

use super::overlap::rectangles_overlap_on_axes;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::parcels::{
    OVERLAP_EPSILON_M, ParcelGeometry, ROAD_OVERLAP_QUERY_PAD_M,
};
use godot::prelude::{Vector2, Vector3};

pub(crate) fn geometry_overlaps_road(graph: &RegionGraph, geometry: &ParcelGeometry) -> bool {
    let min = Vector3::new(
        geometry.aabb_min.x - ROAD_OVERLAP_QUERY_PAD_M,
        0.0,
        geometry.aabb_min.y - ROAD_OVERLAP_QUERY_PAD_M,
    );
    let max = Vector3::new(
        geometry.aabb_max.x + ROAD_OVERLAP_QUERY_PAD_M,
        0.0,
        geometry.aabb_max.y + ROAD_OVERLAP_QUERY_PAD_M,
    );
    graph
        .get_edges_near_aabb(min, max)
        .into_iter()
        .any(|edge_idx| {
            edge_idx != geometry.edge_idx && geometry_overlaps_road_edge(graph, geometry, edge_idx)
        })
}

fn geometry_overlaps_road_edge(
    graph: &RegionGraph,
    geometry: &ParcelGeometry,
    edge_idx: usize,
) -> bool {
    let edge = graph.edge(edge_idx);
    if edge.deleted || edge.physical_geometry.len() < 2 {
        return false;
    }
    let half_width = edge.width * 0.5 + crate::config::SIDEWALK_WIDTH;
    edge.physical_geometry.windows(2).any(|window| {
        // A pocket edge's corridor lies about its carriageway's axis (`ROAD-95`, D17).
        let (start, end) = edge.carriageway_segment(
            Vector2::new(window[0].x, window[0].z),
            Vector2::new(window[1].x, window[1].z),
        );
        geometry_overlaps_road_corridor_segment(geometry, start, end, half_width)
    })
}

pub(crate) fn geometry_overlaps_road_corridor_segment(
    geometry: &ParcelGeometry,
    start: Vector2,
    end: Vector2,
    half_width: f32,
) -> bool {
    let segment = end - start;
    if segment.length_squared() <= OVERLAP_EPSILON_M * OVERLAP_EPSILON_M {
        return false;
    }
    let tangent = segment.normalized();
    let normal = Vector2::new(tangent.y, -tangent.x);
    let road_corners = [
        start - normal * half_width,
        end - normal * half_width,
        end + normal * half_width,
        start + normal * half_width,
    ];
    rectangles_overlap_on_axes(
        &geometry.corners,
        &road_corners,
        [geometry.tangent, geometry.normal, tangent, normal],
    )
}

// Whether a parcel touches the disc of `radius_m` at `centre`: the nearest point of its rectangle
// lies within the radius (the centre itself when it stands in the parcel), for a roundabout's
// land (`ROAD-62`). O(1).
pub(crate) fn geometry_overlaps_disc(
    geometry: &ParcelGeometry,
    centre: Vector2,
    radius_m: f32,
) -> bool {
    let corners = &geometry.corners;
    let cross = |a: Vector2, b: Vector2| {
        let (edge, to_centre) = (b - a, centre - a);
        edge.x * to_centre.y - edge.y * to_centre.x
    };
    let turns = (0..4).map(|i| cross(corners[i], corners[(i + 1) % 4]));
    let (mut any_left, mut any_right) = (false, false);
    for turn in turns {
        any_left |= turn > 0.0;
        any_right |= turn < 0.0;
    }
    if !(any_left && any_right) {
        return true;
    }
    (0..4).any(|i| {
        let (a, b) = (corners[i], corners[(i + 1) % 4]);
        let edge = b - a;
        let along = (centre - a).dot(edge) / edge.length_squared().max(OVERLAP_EPSILON_M);
        (a + edge * along.clamp(0.0, 1.0)).distance_to(centre) < radius_m
    })
}
