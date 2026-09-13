// SPDX-License-Identifier: GPL-2.0-only

//! Building departure-side selection across the shared road configuration matrix.

use super::support::*;
use super::*;
use crate::simulation::LANE_CONFIGS;
use crate::simulation::network::lanes::LaneType;

// Shared departure-side check. Spawns an agent departing from a building on the given
// `side` (1 = LEFT, -1 = RIGHT) and asserts it lands on the matching sidewalk lane.
//
// `lane_idx` for sidewalks is a fixed constant regardless of vehicle lane count:
//   LEFT sidewalk  → lane_idx =  100
//   RIGHT sidewalk → lane_idx = -100
// (The directional sidewalk lanes share the same physical side index.)
fn check_departure_side(fwd: u8, bkw: u8, building_side: i8, expected_lane_idx: i8, label: &str) {
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(Edge {
        width: (fwd as f32 + bkw as f32) * 3.5,
        fwd_lanes: fwd,
        bkw_lanes: bkw,
        ..create_test_edge(n0, n1)
    });
    graph.rebuild_adjacency_list();

    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);

    let mut allocator = BuildingAllocator::new();
    let asset_id = register_test_asset(
        &mut allocator,
        "test",
        &format!("departure_side_{label}_{building_side}"),
        ZoneClass::Residential,
    );
    let mut building = create_test_building(edge_idx, building_side);
    building.asset_id = asset_id;
    allocator.buildings.push(building);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let entrance = allocator.entrances[0].clone();
    let lane_id = [entrance.foot_lane_fwd, entrance.foot_lane_bkw]
        .into_iter()
        .find(|&lane_id| {
            lane_id != usize::MAX
                && network.lane_system.lanes[lane_id].lane_idx == expected_lane_idx
        })
        .expect("expected exact sidewalk lane");
    let planned_attach_node = if network.lane_system.lanes[lane_id].is_fwd {
        n1
    } else {
        n0
    };
    let projected_s =
        crate::simulation::buildings::allocator::BuildingAllocator::project_point_to_polyline_s(
            &network.lane_system.lanes[lane_id].geometry,
            crate::simulation::buildings::allocator::BuildingAllocator::sample_pos_on_edge(
                &graph,
                edge_idx,
                entrance.entrance_s_m / graph.edge(edge_idx).physical_length,
            ),
        );

    let mut agents = AgentSystem::new();
    let a_id = agents.spawn_border_arrival_agent(0, n0, 100.0, 0.0);
    agents.transit[a_id] = TRANSIT_ACCESS_EGRESS;
    agents.transit_mode[a_id] = MODE_WALK;
    agents.current_building[a_id] = 0;
    agents.pos_x[a_id] = entrance.door_pos.x;
    agents.pos_y[a_id] = entrance.door_pos.y;
    agents.planned_attach_node[a_id] = planned_attach_node;
    agents.planned_attach_lane_id[a_id] = lane_id as u32;
    agents.planned_attach_lane_d[a_id] = projected_s;
    agents.access_flags[a_id] = ACCESS_PLAN_VALID;

    for _ in 0..500 {
        agents.tick(
            &mut allocator,
            &mut network,
            &mut graph,
            0.1,
            &test_clock(0, 0),
        );
        if agents.transit[a_id] == TRANSIT_NETWORK {
            break;
        }
    }

    assert_eq!(
        agents.transit[a_id], TRANSIT_NETWORK,
        "[{label}] agent never reached the network"
    );

    let lane_id = agents.current_lane_id[a_id];
    assert_ne!(
        lane_id,
        usize::MAX,
        "[{label}] lane attachment stayed invalid"
    );
    let lane = &network.lane_system.lanes[lane_id];

    assert_eq!(
        lane.lane_type,
        LaneType::Foot,
        "[{label}] side={building_side}: agent should be on a Foot lane, found {:?}",
        lane.lane_type
    );
    assert_eq!(
        lane.lane_idx, expected_lane_idx,
        "[{label}] side={building_side}: expected lane_idx={expected_lane_idx} but found {}",
        lane.lane_idx
    );
}

#[test]
fn test_agent_departure_uses_correct_side_sidewalk() {
    for &(fwd, bkw, label) in LANE_CONFIGS {
        // LEFT building (side=1) → LEFT sidewalk (lane_idx=100)
        check_departure_side(fwd, bkw, 1, 100, label);
        // RIGHT building (side=-1) → RIGHT sidewalk (lane_idx=-100)
        check_departure_side(fwd, bkw, -1, -100, label);
    }
}
