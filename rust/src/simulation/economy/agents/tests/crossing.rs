// SPDX-License-Identifier: GPL-3.0-or-later

//! Crossing the road to a far-side access (`PARK-32`) on a 1 km two-way road between dead ends
//! 0 (west) and 1 (east): a car on the eastbound lane bound for a door beyond the westbound lane
//! waits in its lane for a gap there, crosses holding the westbound lane, and gives up after
//! `CROSSING_GIVE_UP_S` to come back on the near side.

use super::support::*;
use super::*;
use crate::simulation::economy::agents::tick::{CROSSING_GAVE_UP, CROSSING_GIVE_UP_S};

struct Street {
    agents: AgentSystem,
    allocator: BuildingAllocator,
    network: TransitNetwork,
    graph: RegionGraph,
    eastbound: usize,
    westbound: usize,
    // The crossing's distance on each lane: the door's station projected on it.
    east_d: f32,
    west_d: f32,
}

// The road, and a building at 500 m on the westbound lane's side (side 1, whose own lane is the
// backward one), its door 6 m beyond that lane.
fn street() -> Street {
    let mut graph = RegionGraph::new();
    graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    graph.add_node(Vector3::new(1_000.0, 0.0, 0.0), NodeType::Junction);
    let road = [Vector3::new(0.0, 0.0, 0.0), Vector3::new(1_000.0, 0.0, 0.0)];
    graph.add_edge(Edge {
        physical_length: 1_000.0,
        base_cost: 1_000.0 / 14.0,
        speed_limit: 14.0,
        geometry: road.to_vec(),
        physical_geometry: road.to_vec(),
        ..create_test_edge(0, 1)
    });
    graph.rebuild_adjacency_list();
    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);
    let eastbound = fwd_vehicle_lanes(&network, 0)[0];
    let westbound = bkw_vehicle_lanes(&network, 0)[0];
    let at = |lane: usize| {
        let lane = &network.lane_system.lanes[lane];
        let d = BuildingAllocator::project_point_to_polyline_s(&lane.geometry, Vector2::new(500.0, 0.0));
        (d, BuildingAllocator::sample_pos_on_lane(lane, d))
    };
    let ((east_d, east), (west_d, west)) = (at(eastbound), at(westbound));
    let door = west + (west - east).normalized() * 6.0;

    let mut allocator = BuildingAllocator::new();
    let asset_id = register_test_asset(&mut allocator, "test", "p32_shop", ZoneClass::Commercial);
    let mut shop = create_test_building(0, 1);
    shop.asset_id = asset_id;
    allocator.buildings.push(shop);
    allocator.entrances.push(BuildingEntrance {
        edge_idx: 0,
        side: 1,
        entrance_s_m: 500.0,
        door_pos: door,
        curb_pos: door,
        car_lane_fwd: eastbound,
        car_lane_bkw: westbound,
        ..BuildingEntrance::default()
    });
    Street {
        agents: AgentSystem::new(),
        allocator,
        network,
        graph,
        eastbound,
        westbound,
        east_d,
        west_d,
    }
}

impl Street {
    // A car on the eastbound lane at `d`, bound for the shop's door from that lane: the far side.
    fn shopper(&mut self, d: f32) -> usize {
        let a = &mut self.agents;
        let idx = a.spawn_border_arrival_agent(usize::MAX, 0, 0.0, 0.0);
        a.transit_mode[idx] = MODE_CAR;
        a.agents.enter_lane(idx, &self.network.lane_system, self.eastbound, d, 0);
        a.speed[idx] = 10.0;
        a.target_building[idx] = 0;
        a.current_path[idx] = vec![0, 1];
        a.current_path_index[idx] = 1;
        a.access_flags[idx] = ACCESS_PLAN_VALID;
        a.planned_detach_node[idx] = 0;
        a.planned_detach_lane_id[idx] = self.eastbound as u32;
        a.planned_detach_lane_d[idx] = self.east_d;
        idx
    }

    // A car on the westbound lane at `d` driving at the limit to the western dead end.
    fn through(&mut self, d: f32) -> usize {
        let a = &mut self.agents;
        let idx = a.spawn_border_arrival_agent(usize::MAX, 1, 0.0, 0.0);
        a.transit_mode[idx] = MODE_CAR;
        a.agents.enter_lane(idx, &self.network.lane_system, self.westbound, d, 1);
        a.speed[idx] = 14.0;
        a.current_path[idx] = vec![1, 0];
        a.current_path_index[idx] = 1;
        a.access_flags[idx] = ACCESS_PLAN_VALID | ACCESS_FREIGHT_BORDER_DESTINATION;
        a.freight_target_border_node[idx] = 0;
        a.planned_detach_node[idx] = 0;
        idx
    }

    fn tick(&mut self) {
        self.agents.tick(
            &self.allocator,
            &mut self.network,
            &mut self.graph,
            0.25,
            &test_clock(0, 0),
        );
    }
}

// The far-side door's crossing: the car waits in its lane while the westbound car comes, crosses
// once it has passed, holding the westbound lane while it crosses, and arrives. Returns the final
// state of both cars as bits.
fn wait_then_cross(workers: usize) -> Vec<u32> {
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap()
        .install(|| {
            let mut s = street();
            let car = s.shopper(s.east_d - 2.0);
            let other = s.through(s.west_d - 30.0);
            let mut waited = false;
            let mut crossed = false;
            for tick in 0..400 {
                s.tick();
                let a = &s.agents;
                if a.transit[car] == TRANSIT_NETWORK && a.crossing_wait_s[car] > 0.0 {
                    assert_eq!(a.current_lane_id[car], s.eastbound, "tick {tick}");
                    assert!((a.lane_distance[car] - s.east_d).abs() < 1e-3, "tick {tick}: waits at the detach");
                    waited = true;
                }
                if a.transit[car] == TRANSIT_ACCESS_INGRESS && a.crossing_lane[car] != u32::MAX {
                    assert_eq!(a.crossing_lane[car] as usize, s.westbound);
                    assert!(
                        a.lane_distance[other] > s.west_d,
                        "tick {tick}: crossed before the westbound car passed"
                    );
                    assert!(
                        a.lane_buckets[s.westbound].iter().any(|e| e.1 == car),
                        "tick {tick}: the crossing holds no entry in the westbound lane"
                    );
                    crossed = true;
                }
                if a.transit[car] == TRANSIT_IN_BUILDING {
                    assert!(waited, "it never waited for the westbound car");
                    assert!(crossed, "it never crossed holding the westbound lane");
                    return [car, other]
                        .iter()
                        .flat_map(|&i| [a.pos_x[i].to_bits(), a.pos_y[i].to_bits(), u32::from(a.transit[i])])
                        .collect();
                }
            }
            panic!("the car never arrived: transit {}", s.agents.transit[car]);
        })
}

#[test]
fn far_side_door_waits_for_a_gap_and_crosses() {
    assert_eq!(wait_then_cross(1), wait_then_cross(8), "worker count changed the outcome");
}

#[test]
fn far_side_crossing_gives_up_and_comes_back_on_the_near_side() {
    let mut s = street();
    let car = s.shopper(s.east_d - 2.0);
    s.through(s.west_d - 30.0);
    // Its wait all but over: the westbound car still coming makes it give up.
    s.agents.crossing_wait_s[car] = CROSSING_GIVE_UP_S - 0.1;
    s.tick();
    let a = &s.agents;
    assert_eq!(a.transit[car], TRANSIT_NETWORK);
    assert_eq!(a.current_lane_id[car], s.eastbound);
    assert_eq!(a.crossing_wait_s[car], CROSSING_GAVE_UP, "marked as given up for this leg");
    assert_eq!(
        a.planned_detach_lane_id[car] as usize, s.westbound,
        "the new plan reaches the door from its own side"
    );
    let mut came_back = false;
    for tick in 0..2_400 {
        s.tick();
        let a = &s.agents;
        // Its lane is free at once: it drives on from the next tick.
        assert!(tick > 0 || a.speed[car] > 0.0, "it still stands in its lane");
        came_back |= a.current_lane_id[car] == s.westbound;
        assert!(a.crossing_lane[car] == u32::MAX, "it crossed after giving up");
        if a.transit[car] == TRANSIT_IN_BUILDING {
            assert!(came_back, "it arrived without coming back on the westbound lane");
            return;
        }
    }
    panic!("the car never arrived after giving up");
}

// A car that gave up a crossing on its leg and is brought back to the far side by a later plan
// gives up at once, with no gap to wait for and an empty road: it never waits to cross twice.
#[test]
fn a_car_that_gave_up_never_waits_to_cross_again_on_its_leg() {
    let mut s = street();
    let car = s.shopper(s.east_d - 2.0);
    s.agents.crossing_wait_s[car] = CROSSING_GAVE_UP;
    s.tick();
    let a = &s.agents;
    assert_eq!(a.transit[car], TRANSIT_NETWORK, "it crossed though it had given up");
    assert_eq!(a.current_lane_id[car], s.eastbound);
    assert_eq!(a.planned_detach_lane_id[car] as usize, s.westbound);
    assert_eq!(a.crossing_wait_s[car], CROSSING_GAVE_UP);
}
