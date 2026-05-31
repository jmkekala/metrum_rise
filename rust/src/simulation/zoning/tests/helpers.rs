//! Shared zoning test fixtures.

use crate::simulation::core::config::WorldConfig;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{
    EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
};
use crate::simulation::zoning::ZoningSystem;
use godot::prelude::{Vector2, Vector3};

pub(super) fn make_straight_road() -> (RegionGraph, usize) {
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(-60.0, 0.0, 0.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(60.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(Edge {
        start_node: start,
        end_node: end,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 50.0,
        base_cost: 120.0,
        physical_length: 120.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![Vector3::new(-60.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(-60.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)],
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access: VehicleFrontageAccess::BothSides,
    });
    (graph, edge_idx)
}

pub(super) fn add_vertical_road_at_x(graph: &mut RegionGraph, x: f32) -> usize {
    let start = graph.add_node(Vector3::new(x, 0.0, -80.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(x, 0.0, 80.0), NodeType::Junction);
    graph.add_edge(Edge {
        start_node: start,
        end_node: end,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 50.0,
        base_cost: 160.0,
        physical_length: 160.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![Vector3::new(x, 0.0, -80.0), Vector3::new(x, 0.0, 80.0)],
        physical_geometry: vec![Vector3::new(x, 0.0, -80.0), Vector3::new(x, 0.0, 80.0)],
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access: VehicleFrontageAccess::BothSides,
    })
}

pub(super) fn make_quarter_arc_road(radius_m: f32) -> (RegionGraph, usize) {
    let mut graph = RegionGraph::new();
    let mut points = Vec::new();
    for step in 0..=12 {
        let t = step as f32 / 12.0;
        let angle = -std::f32::consts::FRAC_PI_2 + t * std::f32::consts::FRAC_PI_2;
        points.push(Vector3::new(
            radius_m * angle.cos(),
            0.0,
            radius_m * angle.sin(),
        ));
    }
    let length = points
        .windows(2)
        .map(|window| window[0].distance_to(window[1]))
        .sum();
    let start = graph.add_node(points[0], NodeType::Junction);
    let end = graph.add_node(*points.last().unwrap(), NodeType::Junction);
    let edge_idx = graph.add_edge(Edge {
        start_node: start,
        end_node: end,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 50.0,
        base_cost: length,
        physical_length: length,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: points.clone(),
        physical_geometry: points,
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access: VehicleFrontageAccess::BothSides,
    });
    (graph, edge_idx)
}

pub(super) fn inward_arc_point(radius_m: f32, angle: f32, offset_m: f32) -> Vector2 {
    let road = Vector2::new(radius_m * angle.cos(), radius_m * angle.sin());
    let inward = -road.normalized();
    road + inward * offset_m
}

pub(super) fn make_zoning() -> ZoningSystem {
    ZoningSystem::new(&WorldConfig::default())
}
