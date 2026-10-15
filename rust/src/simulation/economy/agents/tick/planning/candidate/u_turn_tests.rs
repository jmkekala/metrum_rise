// SPDX-License-Identifier: GPL-2.0-only

//! `NET-07` planner regressions: chosen car plans are drivable through the lanes' own
//! connectors, which allow a U-turn only at a dead end.

use super::*;
use crate::simulation::network::graph::Edge;
use crate::simulation::network::types::{NodeType, VehicleFrontageAccess};
use crate::simulation::pathing::cch::CchGraph;
use godot::prelude::{Vector2, Vector3};

// Straight roads between the given points; costs follow length.
fn town(points: &[(f32, f32)], roads: &[(u32, u32)]) -> (RegionGraph, TransitNetwork) {
    let mut graph = RegionGraph::new();
    for &(x, z) in points {
        graph.add_node(Vector3::new(x, 0.0, z), NodeType::Junction);
    }
    for &(a, b) in roads {
        let (from, to) = (graph.node(a).pos, graph.node(b).pos);
        let length = from.distance_to(to);
        graph.add_edge(Edge {
            base_cost: length / 14.0,
            physical_length: length,
            geometry: vec![from, to],
            physical_geometry: vec![from, to],
            ..super::tests::test_edge(a, b, 0.0, 1.0)
        });
    }
    graph.rebuild_adjacency_list();
    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);
    (graph, network)
}

// A door `s_m` metres along `edge` on `side`. Same-side-only access keeps the car lane on the
// door's own side (`side` -1: the forward lane, 1: the backward lane).
fn door(
    graph: &RegionGraph,
    network: &TransitNetwork,
    edge: usize,
    side: i8,
    s_m: f32,
    access: VehicleFrontageAccess,
) -> BuildingEntrance {
    let lane = |forward: bool| {
        network.lane_system.edge_lanes[&edge]
            .iter()
            .copied()
            .find(|&id| {
                let lane = &network.lane_system.lanes[id];
                lane.lane_type == LaneType::Vehicle && lane.is_fwd == forward
            })
            .unwrap()
    };
    let value = graph.edge(edge);
    let (from, to) = (
        graph.node(value.start_node).pos,
        graph.node(value.end_node).pos,
    );
    let along = (to - from).normalized();
    let point = from + along * s_m;
    let normal = Vector2::new(-along.z, along.x) * f32::from(side);
    let same_side = access == VehicleFrontageAccess::SameSideOnly;
    BuildingEntrance {
        edge_idx: edge,
        side,
        vehicle_frontage_access: access,
        entrance_s_m: s_m,
        door_pos: Vector2::new(point.x, point.z) + normal * 12.0,
        curb_pos: Vector2::new(point.x, point.z) + normal * 5.0,
        car_lane_fwd: if same_side && side == 1 {
            usize::MAX
        } else {
            lane(true)
        },
        car_lane_bkw: if same_side && side == -1 {
            usize::MAX
        } else {
            lane(false)
        },
        ..Default::default()
    }
}

// The best car candidate and its stored route, exactly as a departing agent receives them.
fn car_plan(
    origin: &BuildingEntrance,
    destination: &BuildingEntrance,
    graph: &RegionGraph,
    network: &TransitNetwork,
) -> (PlannedTripCandidate, Vec<u32>) {
    let count = AtomicU32::new(0);
    let mut candidate =
        best_trip_candidate_for_mode(MODE_CAR, origin, destination, network, graph, &count)
            .expect("a car trip exists");
    let (path, _) = build_exact_path_for_candidate(
        &mut candidate,
        0,
        ZoneType::Residential,
        network,
        graph,
        &count,
    )
    .expect("the candidate has a route");
    (candidate, path)
}

// Whether a car can follow `path` from `attach` and leave on `detach` using only the lanes'
// direct continuations and junction connectors, changing lanes within one road direction.
fn drivable(
    path: &[u32],
    attach: usize,
    detach: usize,
    network: &TransitNetwork,
    graph: &RegionGraph,
) -> bool {
    let lanes = &network.lane_system.lanes;
    let siblings = |lane: usize| -> Vec<usize> {
        network.lane_system.edge_lanes[&lanes[lane].edge_id]
            .iter()
            .copied()
            .filter(|&id| {
                lanes[id].lane_type == LaneType::Vehicle && lanes[id].is_fwd == lanes[lane].is_fwd
            })
            .collect()
    };
    let successors = |from: &[usize]| -> Vec<usize> {
        from.iter()
            .flat_map(|&lane| &lanes[lane].next_lanes)
            .flat_map(|&next| {
                if lanes[next].edge_id == usize::MAX {
                    lanes[next].next_lanes.iter().copied().collect()
                } else {
                    vec![next]
                }
            })
            .collect()
    };
    if path.is_empty() && attach == detach {
        return true;
    }
    let mut current = siblings(attach);
    for step in path.windows(2) {
        let edge = graph.get_edge_between_nodes(step[0], step[1]).unwrap();
        let Some(next) = successors(&current).into_iter().find(|&lane| {
            lanes[lane].edge_id == edge && lanes[lane].lane_type == LaneType::Vehicle
        }) else {
            return false;
        };
        current = siblings(next);
    }
    successors(&current).contains(&detach)
}

// Frontage road A (0) – B (1); each has two short dead-end stubs, so neither junction may
// host a U-turn while every stub end may.
fn three_arm_town() -> (RegionGraph, TransitNetwork) {
    town(
        &[
            (0.0, 0.0),
            (100.0, 0.0),
            (-30.0, 0.0),
            (0.0, -30.0),
            (130.0, 0.0),
            (100.0, 30.0),
        ],
        &[(0, 1), (2, 0), (3, 0), (1, 4), (1, 5)],
    )
}

#[test]
fn across_the_road_at_a_three_arm_junction_turns_at_a_dead_end() {
    // The dump's building 651: the door across the road serves only westbound cars and lies
    // just short of junction B. Before NET-07 the plan was a free zero hop at B (eastbound
    // lane into the westbound exit lane), which the lanes cannot drive.
    let (graph, network) = three_arm_town();
    let origin = door(
        &graph,
        &network,
        0,
        -1,
        90.0,
        VehicleFrontageAccess::BothSides,
    );
    let destination = door(
        &graph,
        &network,
        0,
        1,
        95.0,
        VehicleFrontageAccess::SameSideOnly,
    );
    let (candidate, path) = car_plan(&origin, &destination, &graph, &network);
    assert!(
        drivable(
            &path,
            candidate.planned_attach_lane_id,
            candidate.planned_detach_lane_id,
            &network,
            &graph
        ),
        "{path:?}"
    );
    // Departure towards B, a turn at one of B's dead ends, and back to the door.
    assert_eq!(candidate.planned_attach_node, 1);
    assert_eq!(candidate.planned_detach_node, 1);
    assert!(path.len() == 3 && [4, 5].contains(&path[1]), "{path:?}");
}

#[test]
fn bend_never_hosts_a_u_turn() {
    // B (1) is a two-arm bend towards a 40 m dead-end stub C (2). The lanes build no U-turn
    // at the bend, so the car must turn at C.
    let (graph, network) = town(
        &[
            (0.0, 0.0),
            (100.0, 0.0),
            (130.0, 30.0),
            (-30.0, 0.0),
            (0.0, -30.0),
        ],
        &[(0, 1), (1, 2), (3, 0), (4, 0)],
    );
    let origin = door(
        &graph,
        &network,
        0,
        -1,
        90.0,
        VehicleFrontageAccess::BothSides,
    );
    let destination = door(
        &graph,
        &network,
        0,
        1,
        95.0,
        VehicleFrontageAccess::SameSideOnly,
    );
    let (candidate, path) = car_plan(&origin, &destination, &graph, &network);
    assert!(
        drivable(
            &path,
            candidate.planned_attach_lane_id,
            candidate.planned_detach_lane_id,
            &network,
            &graph
        ),
        "{path:?}"
    );
    assert_eq!(path, [1, 2, 1]);
}

#[test]
fn departure_direction_flips_when_the_cheap_side_needs_a_u_turn() {
    // Frontage road A (0) – B (1) continues straight to junction C (2), whose third arm is a
    // 2 km dead end. The door on B–C is served only by the C→B lane. Leaving east and turning
    // back at C looks shortest (about 200 m) but needs a U-turn at a three-arm junction; before
    // NET-07 the exit turn was unchecked and the car left east. The legal choice leaves west
    // and goes around the block A–G–H–C (about 500 m) instead of the stub loop (about 4.2 km).
    let (graph, network) = town(
        &[
            (0.0, 0.0),
            (100.0, 0.0),
            (200.0, 0.0),
            (200.0, 100.0),
            (0.0, 100.0),
            (2200.0, 0.0),
        ],
        &[(0, 1), (1, 2), (2, 3), (4, 3), (0, 4), (2, 5)],
    );
    let origin = door(
        &graph,
        &network,
        0,
        -1,
        50.0,
        VehicleFrontageAccess::BothSides,
    );
    let destination = door(
        &graph,
        &network,
        1,
        1,
        50.0,
        VehicleFrontageAccess::SameSideOnly,
    );
    let (candidate, path) = car_plan(&origin, &destination, &graph, &network);
    assert!(
        drivable(
            &path,
            candidate.planned_attach_lane_id,
            candidate.planned_detach_lane_id,
            &network,
            &graph
        ),
        "{path:?}"
    );
    assert_eq!(candidate.planned_attach_node, 0, "{path:?}");
    assert_eq!(path, [0, 4, 3, 2]);
    // Every junction-side alternative must also be legal; a car never departs into one that
    // needs B's U-turn.
    for (origin_rank, destination_rank) in NODE_RANK_PAIRS {
        let count = AtomicU32::new(0);
        let (Some(from), Some(to)) = (
            TripEnd::new(MODE_CAR, origin_rank, &origin, true, &network, &graph),
            TripEnd::new(
                MODE_CAR,
                destination_rank,
                &destination,
                false,
                &network,
                &graph,
            ),
        ) else {
            continue;
        };
        let Some(mut other) = combine_trip_ends(
            MODE_CAR,
            origin_rank,
            destination_rank,
            &from,
            &to,
            &network,
            &graph,
            &count,
        ) else {
            continue;
        };
        let (path, _) = build_exact_path_for_candidate(
            &mut other,
            0,
            ZoneType::Residential,
            &network,
            &graph,
            &count,
        )
        .unwrap();
        assert!(
            drivable(
                &path,
                other.planned_attach_lane_id,
                other.planned_detach_lane_id,
                &network,
                &graph
            ),
            "ranks {origin_rank}/{destination_rank}: {path:?}"
        );
    }
}
