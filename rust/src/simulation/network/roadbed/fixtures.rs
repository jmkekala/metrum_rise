// SPDX-License-Identifier: GPL-2.0-only

//! Minimal road graphs for roadbed tests: no lanes, zoning or terrain, only edges, nodes and the
//! graph clips that place the span mouths.

use crate::config;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{
    EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
};
use godot::prelude::Vector3;

/// Grade of the sloped fixtures along +X.
pub(crate) const SLOPE: f32 = 0.025;

/// A two-way road edge (car and foot) through `points`, used for geometry and physical profile.
pub(crate) fn road_edge(start_node: u32, end_node: u32, points: Vec<Vector3>, width: f32) -> Edge {
    let length = points
        .windows(2)
        .map(|segment| segment[0].distance_to(segment[1]))
        .sum();
    Edge {
        start_node,
        end_node,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width,
        fwd_lanes: ((width / config::LANE_WIDTH).round() as u8).max(1),
        bkw_lanes: 0,
        speed_limit: 50.0,
        base_cost: 0.0,
        physical_length: length,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: points.clone(),
        physical_geometry: points,
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access: VehicleFrontageAccess::BothSides,
    }
}

/// Point on the sloped plane `y = SLOPE * x`, or on the flat plane.
pub(crate) fn point(x: f32, z: f32, sloped: bool) -> Vector3 {
    Vector3::new(x, if sloped { SLOPE * x } else { 0.0 }, z)
}

/// Points from `a` to `b` with a knot every `step_m` metres.
pub(crate) fn polyline(a: (f32, f32), b: (f32, f32), step_m: f32, sloped: bool) -> Vec<Vector3> {
    let length = (b.0 - a.0).hypot(b.1 - a.1);
    let count = (length / step_m).ceil().max(1.0) as usize;
    (0..=count)
        .map(|i| {
            let t = i as f32 / count as f32;
            point(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, sloped)
        })
        .collect()
}

/// Adds an edge between two existing nodes along a knotted polyline.
pub(crate) fn connect(graph: &mut RegionGraph, a: u32, b: u32, width: f32, sloped: bool) -> usize {
    let (pa, pb) = (graph.node(a).pos, graph.node(b).pos);
    let points = polyline((pa.x, pa.z), (pb.x, pb.z), 20.0, sloped);
    graph.add_edge(road_edge(a, b, points, width))
}

/// A T junction at the origin: arms west, east and south of 60 m, then the graph clips.
/// Edges: 0 west to centre, 1 centre to east, 2 south to centre.
pub(crate) fn t_junction(sloped: bool) -> RegionGraph {
    let mut graph = RegionGraph::new();
    let center = graph.add_node(point(0.0, 0.0, sloped), NodeType::Junction);
    let west = graph.add_node(point(-60.0, 0.0, sloped), NodeType::Junction);
    let east = graph.add_node(point(60.0, 0.0, sloped), NodeType::Junction);
    let south = graph.add_node(point(0.0, 60.0, sloped), NodeType::Junction);
    connect(&mut graph, west, center, 7.0, sloped);
    connect(&mut graph, center, east, 7.0, sloped);
    connect(&mut graph, south, center, 7.0, sloped);
    graph.rebuild_intersection_clips();
    graph
}

/// The T junction plus one edge of every other profile, apart from it: a road without
/// sidewalks, a footpath, a four-lane road and a bridge.
pub(crate) fn mixed_profiles(sloped: bool) -> RegionGraph {
    let mut graph = t_junction(sloped);
    let mut add = |a: (f32, f32), b: (f32, f32), edge: fn(Edge) -> Edge| {
        let start = graph.add_node(point(a.0, a.1, sloped), NodeType::Junction);
        let end = graph.add_node(point(b.0, b.1, sloped), NodeType::Junction);
        let points = polyline(a, b, 20.0, sloped);
        graph.add_edge(edge(road_edge(start, end, points, 7.0)))
    };
    add((-80.0, -60.0), (80.0, -50.0), |mut edge| {
        edge.allowed_types = TransitFlags::CAR;
        edge
    });
    add((-80.0, -100.0), (40.0, -140.0), |mut edge| {
        edge.primary_type = TransitType::Foot;
        edge.allowed_types = TransitFlags::FOOT;
        edge.width = 2.5;
        edge
    });
    add((100.0, -100.0), (100.0, 100.0), |mut edge| {
        edge.width = 14.0;
        edge.fwd_lanes = 2;
        edge.bkw_lanes = 2;
        edge
    });
    add((-100.0, 100.0), (-160.0, 20.0), |mut edge| {
        edge.class = EdgeClass::Bridge;
        edge
    });
    graph.rebuild_intersection_clips();
    graph
}
