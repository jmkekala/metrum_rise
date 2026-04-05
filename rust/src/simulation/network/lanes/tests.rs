use godot::prelude::*;
use std::collections::HashSet;
use super::super::graph::{Edge, RegionGraph};
use super::super::types::{NodeType, TransitFlags, TransitType, EdgeClass};
use super::{LaneSystem, LaneType};

#[test]
fn test_lane_geometry_and_length() {
    let mut graph = RegionGraph::new();
    let n1 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n2 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);

    graph.add_edge(Edge {
        start_node: n1,
        end_node: n2,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 50.0,
        base_cost: 2.0,
        physical_length: 100.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        deleted: false,
    });
    graph.rebuild_adjacency_list();

    let mut lanes = LaneSystem::new();
    lanes.rebuild(&mut graph);

    let physical: Vec<_> = lanes.lanes.iter().filter(|l| l.edge_id != usize::MAX).collect();
    assert_eq!(physical.len(), 6, "Expected 2 vehicle and 4 foot physical lanes");

    let l_fwd = &lanes.lanes[0];
    assert!(l_fwd.is_fwd, "Lane 0 should be forward");
    assert!((l_fwd.length - 100.0).abs() < 1.0, "FWD lane length should be roughly 100m, was {}", l_fwd.length);

    let l_bkw = &lanes.lanes[1];
    assert!(!l_bkw.is_fwd, "Lane 1 should be backward");
    assert!((l_bkw.length - 100.0).abs() < 1.0, "BKW lane length should be roughly 100m, was {}", l_bkw.length);
}

#[test]
fn test_highway_no_sidewalks() {
    let mut graph = RegionGraph::new();
    let n1 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n2 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);

    graph.add_edge(Edge {
        start_node: n1,
        end_node: n2,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR, // No FOOT
        class: EdgeClass::Standard,
        width: 14.0,
        fwd_lanes: 2,
        bkw_lanes: 2,
        speed_limit: 100.0,
        base_cost: 1.0,
        physical_length: 100.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        deleted: false,
    });
    graph.rebuild_adjacency_list();

    let mut lanes = LaneSystem::new();
    lanes.rebuild(&mut graph);

    let physical: Vec<_> = lanes.lanes.iter().filter(|l| l.edge_id != usize::MAX).collect();
    assert_eq!(physical.len(), 4, "Highways should have no foot lanes");
    for lane in &physical {
        assert_eq!(lane.lane_type, LaneType::Vehicle);
    }
}

#[test]
fn test_dedicated_footpath_centering() {
    let mut graph = RegionGraph::new();
    let n1 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n2 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);

    graph.add_edge(Edge {
        start_node: n1,
        end_node: n2,
        primary_type: TransitType::Foot,
        allowed_types: TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 3.0,
        fwd_lanes: 0,
        bkw_lanes: 0,
        speed_limit: 5.0,
        base_cost: 20.0,
        physical_length: 100.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        deleted: false,
    });
    graph.rebuild_adjacency_list();

    let mut lanes = LaneSystem::new();
    lanes.rebuild(&mut graph);

    assert_eq!(lanes.lanes.len(), 2, "Dedicated footpaths should have 2 bidirectional lanes");
    for lane in &lanes.lanes {
        assert_eq!(lane.lane_type, LaneType::Foot);
        assert_eq!(lane.lane_idx, 0);
        assert!((lane.geometry[0].z - 0.0).abs() < 0.1);
    }
}

#[test]
fn test_junction_pedestrian_connectivity() {
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(-100.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n2 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
    let n3 = graph.add_node(Vector3::new(0.0, 0.0, 100.0), NodeType::Junction);

    let roads = [
        (n0, n1), (n1, n2), (n1, n3)
    ];

    for (s, e) in roads {
        graph.add_edge(Edge {
            start_node: s,
            end_node: e,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: 1.0,
            physical_length: 100.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![graph.nodes[s as usize].pos, graph.nodes[e as usize].pos],
            physical_geometry: vec![graph.nodes[s as usize].pos, graph.nodes[e as usize].pos],
            deleted: false,
        });
    }
    graph.rebuild_adjacency_list();

    let mut lanes = LaneSystem::new();
    lanes.rebuild(&mut graph);

    let l_id = *lanes.edge_lanes[&0].iter().find(|&&id| {
        let l = &lanes.lanes[id];
        l.lane_idx == 100 && l.is_fwd
    }).expect("Edge 0 should have a forward left sidewalk");

    let next = &lanes.lanes[l_id].next_lanes;
    assert!(!next.is_empty(), "Sidewalk should have connections at junction");
    let has_crosswalk = next.iter().any(|&nid| lanes.lanes[nid].edge_id == usize::MAX);
    assert!(has_crosswalk, "Should have a crosswalk connection at junction");
}

#[test]
fn test_rht_lane_offsets() {
    let mut graph = RegionGraph::new();
    let n1 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n2 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);

    graph.add_edge(Edge {
        start_node: n1,
        end_node: n2,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 50.0,
        base_cost: 1.0,
        physical_length: 100.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        deleted: false,
    });
    graph.rebuild_adjacency_list();

    let mut lanes = LaneSystem::new();
    lanes.rebuild(&mut graph);

    let l_fwd = &lanes.lanes[0];
    assert!(l_fwd.is_fwd);
    assert!(l_fwd.geometry[0].z > 0.0, "Forward lane should be at positive Z (Right)");

    let l_bkw = &lanes.lanes[1];
    assert!(!l_bkw.is_fwd);
    assert!(l_bkw.geometry[0].z < 0.0);
}

#[test]
fn test_crosswalk_counts() {
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(-100.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n2 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
    let n3 = graph.add_node(Vector3::new(200.0, 0.0, 0.0), NodeType::Junction);
    let n4 = graph.add_node(Vector3::new(100.0, 0.0, 100.0), NodeType::Junction);
    let n5 = graph.add_node(Vector3::new(100.0, 0.0, 200.0), NodeType::Junction);
    let n6 = graph.add_node(Vector3::new(200.0, 0.0, 200.0), NodeType::Junction);
    let n7 = graph.add_node(Vector3::new(100.0, 0.0, 300.0), NodeType::Junction);
    let n8 = graph.add_node(Vector3::new(200.0, 0.0, 300.0), NodeType::Junction);
    let n9 = graph.add_node(Vector3::new(0.0, 0.0, 200.0), NodeType::Junction);

    let edges = [
        (n0, n1), (n1, n2), (n2, n3), (n2, n4), 
        (n4, n5), (n5, n6), (n5, n7), (n5, n9), (n6, n8)
    ];
    for (s, e) in edges {
        graph.add_edge(Edge {
            start_node: s,
            end_node: e,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: 1.0,
            physical_length: 100.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![graph.nodes[s as usize].pos, graph.nodes[e as usize].pos],
            physical_geometry: vec![graph.nodes[s as usize].pos, graph.nodes[e as usize].pos],
            deleted: false,
        });
    }
    graph.rebuild_adjacency_list();
    let mut lanes = LaneSystem::new();
    lanes.rebuild(&mut graph);

    fn count_crosswalks_at(lanes: &LaneSystem, graph: &RegionGraph, node_id: usize) -> usize {
        let mut inbound_lane_ids = Vec::new();
        for &e_idx in &graph.adjacency[node_id] {
            let edge = &graph.edges[e_idx];
            let is_end = edge.end_node as usize == node_id;
            let edge_lanes = &lanes.edge_lanes[&e_idx];
            for &lane_id in edge_lanes {
                let l = &lanes.lanes[lane_id];
                if l.lane_type == LaneType::Foot && l.is_fwd == is_end {
                    inbound_lane_ids.push(lane_id);
                }
            }
        }
        
        let mut crosswalk_lanes = Vec::new();
        for &lid in &inbound_lane_ids {
            for &next_id in &lanes.lanes[lid].next_lanes {
                let next_l = &lanes.lanes[next_id];
                if next_l.is_crosswalk && next_l.lane_type == LaneType::Foot {
                    if !crosswalk_lanes.contains(&next_id) {
                        crosswalk_lanes.push(next_id);
                    }
                }
            }
        }
        crosswalk_lanes.len() / 2
    }

    assert_eq!(count_crosswalks_at(&lanes, &graph, n0 as usize), 1);
    assert_eq!(count_crosswalks_at(&lanes, &graph, n1 as usize), 1);
    assert_eq!(count_crosswalks_at(&lanes, &graph, n2 as usize), 3);
    assert_eq!(count_crosswalks_at(&lanes, &graph, n5 as usize), 4);
}

#[test]
fn test_vehicle_connections() {
    let mut graph = RegionGraph::new();
    let n_center = graph.add_node(Vector3::ZERO, NodeType::Junction);
    let n_north = graph.add_node(Vector3::new(0.0, 0.0, -100.0), NodeType::Junction);
    let n_east = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
    let n_south = graph.add_node(Vector3::new(0.0, 0.0, 100.0), NodeType::Junction);
    
    for &other in &[n_north, n_east, n_south] {
        graph.add_edge(Edge {
            start_node: n_center,
            end_node: other,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: 1.0,
            physical_length: 100.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![graph.nodes[n_center as usize].pos, graph.nodes[other as usize].pos],
            physical_geometry: vec![graph.nodes[n_center as usize].pos, graph.nodes[other as usize].pos],
            deleted: false,
        });
    }
    graph.rebuild_adjacency_list();
    let mut lanes = LaneSystem::new();
    lanes.rebuild(&mut graph);
    
    let node = &graph.nodes[n_center as usize];
    assert!(!node.lane_connections.is_empty());
    for (&(e_in, l_in), targets) in &node.lane_connections {
        // Only check vehicle lanes (idx < 10; sidewalks are 100/-100)
        if l_in.abs() < 10 {
            let unique_edges: std::collections::HashSet<usize> =
                targets.iter().map(|(e, _)| *e).collect();
            assert_eq!(
                unique_edges.len(),
                2,
                "Vehicle lane on edge {} should connect to 2 other arms at T-junction",
                e_in
            );
        }
    }
}


fn add_road_edge(graph: &mut RegionGraph, s: u32, e: u32) -> usize {
    let p0 = graph.nodes[s as usize].pos;
    let p1 = graph.nodes[e as usize].pos;
    graph.add_edge(Edge {
        start_node: s,
        end_node: e,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 50.0,
        base_cost: 1.0,
        physical_length: p0.distance_to(p1),
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![p0, p1],
        physical_geometry: vec![p0, p1],
        deleted: false,
    })
}

#[test]
fn test_incremental_rebuild_new_edge_gets_lanes() {
    let mut graph = RegionGraph::new();
    let na = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let nb = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
    let nc = graph.add_node(Vector3::new(50.0, 0.0, -100.0), NodeType::Junction);
    let e0 = add_road_edge(&mut graph, na, nb);
    graph.rebuild_adjacency_list();
    let mut lanes = LaneSystem::new();
    lanes.rebuild(&mut graph);
    let e1 = add_road_edge(&mut graph, nb, nc);
    graph.rebuild_adjacency_list();
    let mut affected = HashSet::new();
    affected.insert(e1);
    lanes.rebuild_edges_incremental(&mut graph, &affected);
    assert!(lanes.edge_lanes.contains_key(&e1));
    let physical_e1 = lanes.edge_lanes[&e1].iter().filter(|&&id| lanes.lanes[id].edge_id != usize::MAX).count();
    assert_eq!(physical_e1, 6);
    assert!(lanes.edge_lanes.contains_key(&e0));
}

#[test]
fn test_incremental_rebuild_preserves_unaffected_lane_ids() {
    let mut graph = RegionGraph::new();
    let na = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let nb = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
    let nc = graph.add_node(Vector3::new(50.0, 0.0, -100.0), NodeType::Junction);
    let nd = graph.add_node(Vector3::new(0.0, 0.0, 500.0), NodeType::Junction);
    let ne = graph.add_node(Vector3::new(100.0, 0.0, 500.0), NodeType::Junction);
    let _e0 = add_road_edge(&mut graph, na, nb);
    let e_far = add_road_edge(&mut graph, nd, ne);
    graph.rebuild_adjacency_list();
    let mut lanes = LaneSystem::new();
    lanes.rebuild(&mut graph);
    let far_ids_before: Vec<usize> = lanes.edge_lanes[&e_far].clone();
    let e1 = add_road_edge(&mut graph, nb, nc);
    graph.rebuild_adjacency_list();
    let mut affected = HashSet::new();
    affected.insert(e1);
    lanes.rebuild_edges_incremental(&mut graph, &affected);
    assert_eq!(lanes.edge_lanes[&e_far], far_ids_before);
}

#[test]
fn test_incremental_rebuild_connection_lanes_exist_at_junction() {
    let mut graph = RegionGraph::new();
    let na = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let nb = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
    let nc = graph.add_node(Vector3::new(100.0, 0.0, -100.0), NodeType::Junction);
    let e0 = add_road_edge(&mut graph, na, nb);
    graph.rebuild_adjacency_list();
    let mut lanes = LaneSystem::new();
    lanes.rebuild(&mut graph);
    let e1 = add_road_edge(&mut graph, nb, nc);
    graph.rebuild_adjacency_list();
    let mut affected = HashSet::new();
    affected.insert(e1);
    lanes.rebuild_edges_incremental(&mut graph, &affected);
    let has_vehicle_conn = lanes.lanes.iter().any(|l| {
        l.edge_id == usize::MAX
            && l.lane_type == LaneType::Vehicle
            && !l.next_lanes.is_empty()
            && {
                let target_edge = lanes.lanes[l.next_lanes[0]].edge_id;
                target_edge == e0 || target_edge == e1
            }
    });
    let e0_lane_has_conns = lanes.edge_lanes[&e0].iter().any(|&lid| !lanes.lanes[lid].next_lanes.is_empty());
    assert!(has_vehicle_conn);
    assert!(e0_lane_has_conns);
}
