use super::*;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
use crate::simulation::pathing::cch::CchGraph;
use godot::prelude::Vector3;

#[test]
fn test_slope_cost_calculation() {
    let mut edge = Edge {
        start_node: 0,
        end_node: 1,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR,
        width: 4.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 50.0,
        base_cost: 0.0,
        physical_length: 100.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        class: EdgeClass::Standard,
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access:
            crate::simulation::network::types::VehicleFrontageAccess::BothSides,
    };

    let (flat_cost, _) = cost::CostCalculator::calculate_costs(&edge);

    // Add a hill: 100m length, 50m height (50% slope)
    edge.geometry[1] = Vector3::new(100.0, 50.0, 0.0);
    edge.physical_geometry[1] = Vector3::new(100.0, 50.0, 0.0);
    let (steep_cost, _) = cost::CostCalculator::calculate_costs(&edge);

    assert!(
        steep_cost > flat_cost * 2.0,
        "Steep route cost ({}) should heavily penalize over flat route cost ({})",
        steep_cost,
        flat_cost
    );
}

#[test]
fn test_pathing_avoids_steep_slope() {
    let mut graph = RegionGraph::new();
    // A (0,0,0)
    let n_a = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    // B (100,0,0) - Goal
    let n_b = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
    // C (50, 0, 100) - Detour point
    let n_c = graph.add_node(Vector3::new(50.0, 0.0, 100.0), NodeType::Junction);

    // 1. Short but very steep road: A -> B
    // Slope = 50m height / 100m length = 50%.
    let mut edge_ab = Edge {
        start_node: n_a,
        end_node: n_b,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR,
        width: 6.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 50.0,
        base_cost: 0.0,
        physical_length: 100.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(50.0, 50.0, 0.0),
            Vector3::new(100.0, 0.0, 0.0),
        ],
        physical_geometry: vec![
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(50.0, 50.0, 0.0),
            Vector3::new(100.0, 0.0, 0.0),
        ],
        class: EdgeClass::Standard,
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access:
            crate::simulation::network::types::VehicleFrontageAccess::BothSides,
    };
    let (cost_ab, dist_ab) = cost::CostCalculator::calculate_costs(&edge_ab);
    edge_ab.base_cost = cost_ab;
    edge_ab.physical_length = dist_ab;
    graph.add_edge(edge_ab);

    // 2. Long but flat detour: A -> C -> B
    // A -> C (111.8m)
    let mut edge_ac = Edge {
        start_node: n_a,
        end_node: n_c,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR,
        width: 6.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 50.0,
        base_cost: 0.0,
        physical_length: 0.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(50.0, 0.0, 100.0)],
        physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(50.0, 0.0, 100.0)],
        class: EdgeClass::Standard,
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access:
            crate::simulation::network::types::VehicleFrontageAccess::BothSides,
    };
    let (cost_ac, dist_ac) = cost::CostCalculator::calculate_costs(&edge_ac);
    edge_ac.base_cost = cost_ac;
    edge_ac.physical_length = dist_ac;
    graph.add_edge(edge_ac);

    // C -> B (111.8m)
    let mut edge_cb = Edge {
        start_node: n_c,
        end_node: n_b,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR,
        width: 6.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 50.0,
        base_cost: 0.0,
        physical_length: 0.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![
            Vector3::new(50.0, 0.0, 100.0),
            Vector3::new(100.0, 0.0, 0.0),
        ],
        physical_geometry: vec![
            Vector3::new(50.0, 0.0, 100.0),
            Vector3::new(100.0, 0.0, 0.0),
        ],
        class: EdgeClass::Standard,
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access:
            crate::simulation::network::types::VehicleFrontageAccess::BothSides,
    };
    let (cost_cb, dist_cb) = cost::CostCalculator::calculate_costs(&edge_cb);
    edge_cb.base_cost = cost_cb;
    edge_cb.physical_length = dist_cb;
    graph.add_edge(edge_cb);

    let cch = CchGraph::build(&graph);
    let path = cch.find_path(n_a, n_b, usize::MAX, &graph, TransitFlags::CAR);

    assert!(path.is_some(), "Should find a path");
    let (_cost_found, _dist, nodes) = path.unwrap();

    assert!(
        nodes.contains(&n_c),
        "Router should have detoured through node C (cost {}) to avoid the steep slope on A-B (cost {}). Path was: {:?}",
        cost_ac + cost_cb,
        cost_ab,
        nodes
    );
    assert_eq!(nodes, vec![n_a, n_c, n_b]);
}

#[test]
fn test_bidirectional_walkway_pathing() {
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);

    // Create a bidirectional walkway (primary_type = Foot, allowed_types = 1, lanes = 0)
    graph.add_edge(Edge {
        start_node: n0,
        end_node: n1,
        primary_type: TransitType::Foot,
        allowed_types: TransitFlags::FOOT,
        width: 2.0,
        fwd_lanes: 0,
        bkw_lanes: 0,
        speed_limit: 5.0,
        base_cost: 0.0,
        physical_length: 10.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
        class: EdgeClass::Standard,
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access:
            crate::simulation::network::types::VehicleFrontageAccess::BothSides,
    });

    let cch = CchGraph::build(&graph);

    // 1. Pedestrian should be able to walk n0 -> n1
    let path_fwd = cch.find_path(n0, n1, usize::MAX, &graph, TransitFlags::FOOT);
    assert!(
        path_fwd.is_some(),
        "Pedestrian should find path n0 -> n1 on walkway"
    );

    // 2. Pedestrian should be able to walk n1 -> n0
    let path_bkw = cch.find_path(n1, n0, usize::MAX, &graph, TransitFlags::FOOT);
    assert!(
        path_bkw.is_some(),
        "Pedestrian should find path n1 -> n0 on walkway"
    );

    // 3. Car should NOT be able to use walkway
    let path_car = cch.find_path(n0, n1, usize::MAX, &graph, TransitFlags::CAR);
    assert!(path_car.is_none(), "Car should NOT find path on walkway");
}

#[test]
fn test_car_uturn_allowed() {
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);

    // Create a bidirectional road (idx 0)
    let edge_idx = graph.add_edge(Edge {
        start_node: n0,
        end_node: n1,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        width: 6.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 50.0,
        base_cost: 10.0,
        physical_length: 10.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
        class: EdgeClass::Standard,
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access:
            crate::simulation::network::types::VehicleFrontageAccess::BothSides,
    });

    let cch = CchGraph::build(&graph);

    // Agent is at n1, having come from n0 via edge_idx.
    // They want to go back to n0.
    // If U-turns are allowed on same edge, they should find path [n0] via edge_idx.
    let path = cch.find_path(n1, n0, edge_idx, &graph, TransitFlags::CAR);
    assert!(
        path.is_some(),
        "Car should be allowed to U-turn on bidirectional road"
    );
    let (_, _, nodes) = path.unwrap();
    assert_eq!(nodes, vec![n1, n0]);
}

#[test]
fn test_car_avoids_walkway_shortcut() {
    let mut graph = RegionGraph::new();
    // n0 --- (Road) --- n1 --- (Road) --- n2
    //  \--- (Walkway shortcut) ---/

    let n0 = graph.add_node(
        Vector3::new(0.0, 0.0, 0.0),
        crate::simulation::network::types::NodeType::Junction,
    );
    let n1 = graph.add_node(
        Vector3::new(100.0, 0.0, 0.0),
        crate::simulation::network::types::NodeType::Junction,
    );
    let n2 = graph.add_node(
        Vector3::new(200.0, 0.0, 0.0),
        crate::simulation::network::types::NodeType::Junction,
    );

    // Road path
    graph.add_edge(Edge {
        start_node: n0,
        end_node: n1,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        width: 6.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 50.0,
        base_cost: 10.0,
        physical_length: 100.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![],
        physical_geometry: vec![Vector3::ZERO, Vector3::RIGHT * 100.0],
        class: EdgeClass::Standard,
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access:
            crate::simulation::network::types::VehicleFrontageAccess::BothSides,
    });
    graph.add_edge(Edge {
        start_node: n1,
        end_node: n2,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        width: 6.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 50.0,
        base_cost: 10.0,
        physical_length: 100.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![],
        physical_geometry: vec![Vector3::RIGHT * 100.0, Vector3::RIGHT * 200.0],
        class: EdgeClass::Standard,
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access:
            crate::simulation::network::types::VehicleFrontageAccess::BothSides,
    });

    // Walkway shortcut n0 -> n2 directly
    graph.add_edge(Edge {
        start_node: n0,
        end_node: n2,
        primary_type: TransitType::Foot,
        allowed_types: TransitFlags::FOOT,
        width: 2.0,
        fwd_lanes: 0,
        bkw_lanes: 0,
        speed_limit: 10.0,
        base_cost: 5.0, // Cheaper than road
        physical_length: 200.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![],
        physical_geometry: vec![Vector3::ZERO, Vector3::RIGHT * 200.0],
        class: EdgeClass::Standard,
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access:
            crate::simulation::network::types::VehicleFrontageAccess::BothSides,
    });

    let cch = cch::CchGraph::build(&graph);

    // Car should take the road path (2 nodes) and ignore the shortcut
    let path_car = cch.find_path(n0, n2, usize::MAX, &graph, TransitFlags::CAR);
    assert!(path_car.is_some());
    let (_cost, _dist, nodes) = path_car.unwrap();
    assert_eq!(nodes.len(), 3, "Car should take 3 nodes (n0, n1, n2)");
    assert!(
        nodes.contains(&n1),
        "Car must travel through n1 to avoid walkway"
    );

    // Pedestrian should take the shortcut (n2 only)
    let path_ped = cch.find_path(n0, n2, usize::MAX, &graph, TransitFlags::FOOT);
    assert!(path_ped.is_some());
    let (_c, _d, nodes_ped) = path_ped.unwrap();
    assert_eq!(
        nodes_ped.len(),
        2,
        "Pedestrian should take direct walkway shortcut [n0, n2]"
    );
    assert_eq!(nodes_ped[0], n0);
    assert_eq!(nodes_ped[1], n2);
}

/// Build a loop road network:
///
///   east_border ──(e_ew_e)──► jct ──(e_ew_w)──► west_border
///                              │
///                           (e_n)↑
///                              │
///                           n_north
///
/// Also: east_border ──(e_south)──► south_node ──(e_sw)──► west_border
///                                      │
///                                   (e_sn)↑
///                                      │
///                                   n_north  (shared destination)
///
/// The direct right-turn  east_border → jct → n_north  is blocked.
/// The only way to reach n_north is  east_border → south_node → n_north.
fn build_loop_with_restriction() -> (RegionGraph, u32, u32, u32, u32, u32, usize, usize, usize) {
    let mut g = RegionGraph::new();
    let n_east = g.add_node(Vector3::new(-200.0, 0.0, 0.0), NodeType::Junction);
    let n_jct = g.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n_west = g.add_node(Vector3::new(200.0, 0.0, 0.0), NodeType::Junction);
    let n_north = g.add_node(Vector3::new(0.0, 0.0, -200.0), NodeType::Junction);
    let n_south = g.add_node(Vector3::new(0.0, 0.0, 200.0), NodeType::Junction);

    let mk = |s: u32, e: u32, g: &mut RegionGraph| -> usize {
        let p0 = g.nodes()[s as usize].pos;
        let p1 = g.nodes()[e as usize].pos;
        let len = p0.distance_to(p1);
        g.add_edge(Edge {
            start_node: s,
            end_node: e,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: len / (50.0 / 3.6),
            physical_length: len,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![p0, p1],
            physical_geometry: vec![p0, p1],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access:
                crate::simulation::network::types::VehicleFrontageAccess::BothSides,
        })
    };

    let e_ew_e = mk(n_east, n_jct, &mut g); // east → junction
    let e_ew_w = mk(n_jct, n_west, &mut g); // junction → west
    let e_n = mk(n_jct, n_north, &mut g); // junction → north (blocked right turn)
    let _e_es = mk(n_east, n_south, &mut g); // east → south
    let _e_sw = mk(n_south, n_west, &mut g); // south → west
    let _e_sn = mk(n_south, n_north, &mut g); // south → north (alternate route)
    g.rebuild_adjacency_list();
    (
        g, n_east, n_jct, n_west, n_north, n_south, e_ew_e, e_ew_w, e_n,
    )
}

/// Without any restrictions the direct path east → jct → north should be found.
#[test]
fn test_cch_finds_direct_path_without_restriction() {
    let (mut g, n_east, _n_jct, _n_west, n_north, _n_south, _, _, _) =
        build_loop_with_restriction();
    let cch = CchGraph::build(&mut g);
    let result = cch.find_path(n_east, n_north, usize::MAX, &g, TransitFlags::CAR);
    assert!(result.is_some(), "Should find a path from east to north");
    let (_, _, nodes) = result.unwrap();
    assert_eq!(nodes.first(), Some(&n_east));
    assert_eq!(nodes.last(), Some(&n_north));
}

/// After blocking the right-turn at jct (east_arm → north_arm), CCH must reroute
/// through the southern loop.  The path must NOT contain the junction node as an
/// intermediate step between east_border and north.
#[test]
fn test_cch_reroutes_around_turn_restriction() {
    let (mut g, n_east, n_jct, _n_west, n_north, n_south, e_ew_e, _e_ew_w, _e_n) =
        build_loop_with_restriction();

    // User blocks the right turn: east arm can only go west (no east→north connection).
    // Setting only e_ew_e → e_ew_w puts the node in whitelist mode; e_ew_e → e_n is blocked.
    g.add_lane_connection(n_jct, e_ew_e, 0, _e_ew_w, 0);

    let cch = CchGraph::build(&mut g);
    let result = cch.find_path(n_east, n_north, usize::MAX, &g, TransitFlags::CAR);

    assert!(
        result.is_some(),
        "Should still find a path via southern loop"
    );
    let (_, _, nodes) = result.unwrap();

    assert_eq!(nodes.first(), Some(&n_east), "path must start at east");
    assert_eq!(nodes.last(), Some(&n_north), "path must end at north");

    // The path must NOT route east → jct → north (blocked right turn).
    // Either n_jct is absent from the path, or n_south is the detour point.
    let jct_idx = nodes.iter().position(|&n| n == n_jct);
    let _north_idx = nodes.iter().position(|&n| n == n_north).unwrap();
    if let Some(ji) = jct_idx {
        // If jct appears, it must NOT be immediately followed by north.
        assert!(
            ji + 1 < nodes.len() && nodes[ji + 1] != n_north,
            "CCH must not route east → jct → north (right turn is restricted). Path: {:?}",
            nodes
        );
    }
    assert!(
        nodes.contains(&n_south),
        "Alternate route must pass through south node. Path: {:?}",
        nodes
    );
}
