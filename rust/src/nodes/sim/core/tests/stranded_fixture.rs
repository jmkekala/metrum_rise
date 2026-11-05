// SPDX-License-Identifier: GPL-2.0-only

//! `ROAD-74` stranded walkers as a whole city: one square block of two-way roads with every
//! crossing off, so its inner and outer sidewalks are two walk networks, and walkers on the
//! outer one whose exits lie on the inner one, with no destination or home to fall back on. They
//! strand, walk to the ends of their lanes and wait there on their backoff; the `gridlock check`
//! slot reports where. The same city saved to a file and loaded mid-backoff continues as the
//! unsaved one, and an ignored recorder writes it as the fixture the Godot stranded-walker
//! notice test loads. Minimal setup: no buildings, zones or economy beyond what `SimCore::new`
//! holds.

use super::*;
use crate::config::AGENT_WALK_SPEED_MS;
use crate::nodes::sim::core::frame::FRAME_DT_S;
use crate::simulation::economy::agents::{
    ACCESS_PLAN_VALID, ACCESS_WALK_STRANDED, ACCESS_WALK_TO_SIDEWALK, MODE_WALK, TRANSIT_NETWORK,
};
use crate::simulation::network::lanes::LaneType;
use crate::simulation::pathing::cch::CchGraph;

const SIDE_M: f32 = 120.0;
// Walkers on road 0 (waiting at node 1) and on road 1 (waiting at node 2).
const WALKERS: [(usize, usize); 2] = [(0, 3), (1, 2)];
const MAX_FRAMES: usize = 2_000;

fn corner(k: usize) -> Vector3 {
    let (x, z) = [(0.0, 0.0), (SIDE_M, 0.0), (SIDE_M, SIDE_M), (0.0, SIDE_M)][k % 4];
    Vector3::new(500.0 + x, 0.0, 500.0 + z)
}

// Whether foot lane `lane` runs along the block's inner side.
fn inner(core: &SimCore, lane: usize) -> bool {
    let lane = &core.transit_network.lane_system.lanes[lane];
    let mid = crate::simulation::buildings::allocator::BuildingAllocator::sample_pos_on_lane(
        lane,
        lane.length * 0.5,
    );
    let center = corner(0) + Vector3::new(SIDE_M * 0.5, 0.0, SIDE_M * 0.5);
    mid.distance_to(godot::prelude::Vector2::new(center.x, center.z)) < SIDE_M * 0.5
}

fn foot_lane(core: &SimCore, edge: usize, inner_side: bool, fwd: bool) -> usize {
    core.transit_network.lane_system.edge_lanes[&edge]
        .iter()
        .copied()
        .find(|&lane| {
            let value = &core.transit_network.lane_system.lanes[lane];
            value.lane_type == LaneType::Foot
                && value.is_fwd == fwd
                && inner(core, lane) == inner_side
        })
        .unwrap()
}

// The block before its first frame: nodes 0..4 at the corners, edge k from corner k to k + 1.
fn stranded_core() -> SimCore {
    stranded_core_with(false)
}

// The block, with a car-only road from node 4, 60 m west of node 0, into node 0 when `spur`
// (edge 4; node 4 has no sidewalk).
fn stranded_core_with(spur: bool) -> SimCore {
    let mut core = SimCore {
        treasury: CityTreasury::new(0.0),
        terrain_dirty: false,
        water_dirty: false,
        benchmark_mode: true,
        ..SimCore::new(WorldConfig::new(2048.0, 2048.0, 40.0, 10.0))
    };
    for k in 0..4 {
        core.region_graph.add_node(corner(k), NodeType::Junction);
    }
    for k in 0..4 {
        let (a, b) = (corner(k), corner(k + 1));
        let length = a.distance_to(b);
        core.region_graph.add_edge(Edge {
            start_node: k as u32,
            end_node: ((k + 1) % 4) as u32,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 13.89,
            base_cost: length / 13.89,
            physical_length: length,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![a, b],
            physical_geometry: vec![a, b],
            deleted: false,
            no_building_spawn: false,
            no_street_lights: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
            median: None,
            pocket: None,
            sidewalks: crate::simulation::network::graph::SidewalkSides::BOTH,
            roundabout: None,
        });
    }
    if spur {
        let (a, b) = (corner(0) - Vector3::new(60.0, 0.0, 0.0), corner(0));
        let node = core.region_graph.add_node(a, NodeType::Junction);
        core.region_graph.add_edge(Edge {
            start_node: node,
            end_node: 0,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 13.89,
            base_cost: 60.0 / 13.89,
            physical_length: 60.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![a, b],
            physical_geometry: vec![a, b],
            deleted: false,
            no_building_spawn: false,
            no_street_lights: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
            median: None,
            pocket: None,
            sidewalks: crate::simulation::network::graph::SidewalkSides::NONE,
            roundabout: None,
        });
    }
    core.region_graph.rebuild_adjacency_list();
    for node in 0..core.region_graph.node_count() as u32 {
        for edge in core.region_graph.node_adjacency(node).to_vec() {
            core.region_graph.set_crosswalk_override(node, edge, false);
        }
    }
    core.region_graph.rebuild_intersection_clips();
    core.transit_network
        .lane_system
        .rebuild(&mut core.region_graph);
    core.transit_network.cch_graph = CchGraph::build(&core.region_graph);
    for (edge, count) in WALKERS {
        let lane = foot_lane(&core, edge, false, true);
        let exit = foot_lane(&core, (edge + 1) % 4, true, true);
        let length = core.transit_network.lane_system.lanes[lane].length;
        for k in 0..count {
            let agents = &mut core.agents;
            let i = agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
            agents.transit_mode[i] = MODE_WALK;
            agents.transit[i] = TRANSIT_NETWORK;
            agents.speed[i] = AGENT_WALK_SPEED_MS;
            agents.current_building[i] = usize::MAX;
            agents.current_edge[i] = edge;
            agents.current_node[i] = edge as u32;
            agents.current_lane_id[i] = lane;
            agents.lane_distance[i] = length * 0.5 - 4.0 * k as f32;
            // An exit across the next road at the corner, which no crossing reaches.
            agents.access_flags[i] = ACCESS_PLAN_VALID;
            agents.planned_detach_node[i] = ((edge + 1) % 4) as u32;
            agents.planned_detach_lane_id[i] = exit as u32;
            agents.planned_detach_lane_d[i] = 10.0;
        }
    }
    core.time.speed_multiplier = 1.0;
    core
}

// Steps `core` until the stranded report names every walker, and returns the frames stepped.
fn run_until_reported(core: &mut SimCore) -> usize {
    let walkers: usize = WALKERS.iter().map(|&(_, count)| count).sum();
    for frame in 1..=MAX_FRAMES {
        core.step_frame(FRAME_DT_S);
        if core.agents.stranded_report().walkers as usize == walkers {
            return frame;
        }
    }
    panic!(
        "the walkers did not strand: {:?}",
        core.agents.stranded_report()
    );
}

// Every walker's lane, distance, position, flags, backoff step and next plan time, bitwise.
fn walkers(core: &SimCore) -> Vec<(usize, u32, u32, u8, u8, u32)> {
    let agents = &core.agents;
    (0..agents.len())
        .map(|i| {
            (
                agents.current_lane_id[i],
                agents.lane_distance[i].to_bits(),
                agents.pos_x[i].to_bits(),
                agents.access_flags[i],
                agents.network_replan_failures[i],
                agents.next_replan_time[i].to_bits(),
            )
        })
        .collect()
}

#[test]
fn stranded_walkers_wait_and_continue_across_a_save_file() {
    let mut continuous = stranded_core();
    run_until_reported(&mut continuous);
    let report = continuous.agents.stranded_report().clone();
    assert_eq!(report.junctions, [(1, 3), (2, 2)], "{report:?}");
    assert!(
        (0..continuous.agents.len())
            .all(|i| continuous.agents.access_flags[i] & ACCESS_WALK_STRANDED != 0)
    );
    // Walk on until every walker is past its first retry, mid-backoff, then save.
    let retried = |core: &SimCore, step: u8| {
        (0..core.agents.len()).all(|i| core.agents.network_replan_failures[i] >= step)
    };
    let mut frames = 0;
    while !retried(&continuous, 1) {
        continuous.step_frame(FRAME_DT_S);
        frames += 1;
        assert!(frames < 20 * MAX_FRAMES, "{:?}", walkers(&continuous));
    }
    let path = temp_save_path("stranded_walkers");
    continuous
        .save_game_settled_at(path.to_str().unwrap(), None, 0)
        .unwrap();
    let mut loaded = test_core();
    loaded.load_game_internal(path.to_str().unwrap()).unwrap();
    let _ = std::fs::remove_file(&path);
    loaded.time.speed_multiplier = continuous.time.speed_multiplier;
    assert_eq!(walkers(&loaded), walkers(&continuous));
    assert_eq!(
        loaded.agents.stranded_report(),
        continuous.agents.stranded_report()
    );
    // Past two 2 min retries: the same walkers, timers and report, frame for frame.
    let mut frames = 0;
    while !retried(&continuous, 4) {
        continuous.step_frame(FRAME_DT_S);
        loaded.step_frame(FRAME_DT_S);
        assert_eq!(walkers(&loaded), walkers(&continuous));
        frames += 1;
        assert!(frames < 100 * MAX_FRAMES, "{:?}", walkers(&continuous));
    }
    assert_eq!(
        loaded.agents.stranded_report(),
        continuous.agents.stranded_report()
    );
}

// `TRAFFIC-19`: two walkers stand at node 3 with no lane, at the ends of its two outer
// sidewalks, and one at node 4, which has no sidewalk, all with exits no crossing reaches. The
// first two take the sidewalk each stands on and strand at its end, the third strands at node 4;
// none is moved.
// Saved mid-backoff and loaded, the city continues as the unsaved one.
#[test]
fn walkers_at_nodes_without_a_lane_strand_in_place_and_continue_across_a_save_file() {
    let mut continuous = stranded_core_with(true);
    // The ends of the two outer sidewalks into node 3: road 2's forward, road 3's backward.
    let lanes = [
        foot_lane(&continuous, 2, false, true),
        foot_lane(&continuous, 3, false, false),
    ];
    let exit = foot_lane(&continuous, 3, true, true) as u32;
    let end = |lane: usize| {
        *continuous.transit_network.lane_system.lanes[lane]
            .geometry
            .last()
            .unwrap()
    };
    let spur_node = corner(0) - Vector3::new(60.0, 0.0, 0.0);
    let mut at_nodes = Vec::new();
    for (node, pos) in [(3u32, end(lanes[0])), (3, end(lanes[1])), (4, spur_node)] {
        let agents = &mut continuous.agents;
        let i = agents.spawn_housed_agent(usize::MAX, pos.x, pos.z);
        agents.transit_mode[i] = MODE_WALK;
        agents.transit[i] = TRANSIT_NETWORK;
        agents.speed[i] = AGENT_WALK_SPEED_MS;
        agents.current_building[i] = usize::MAX;
        agents.current_edge[i] = usize::MAX;
        agents.current_node[i] = node;
        agents.current_lane_id[i] = usize::MAX;
        agents.access_flags[i] = ACCESS_PLAN_VALID;
        agents.planned_detach_node[i] = 3;
        agents.planned_detach_lane_id[i] = exit;
        agents.planned_detach_lane_d[i] = 10.0;
        at_nodes.push((i, pos));
    }
    let walkers = continuous.agents.len() as u32;
    let mut frames = 0;
    while continuous.agents.stranded_report().walkers < walkers {
        continuous.step_frame(FRAME_DT_S);
        frames += 1;
        assert!(
            frames < MAX_FRAMES,
            "{:?}",
            continuous.agents.stranded_report()
        );
    }
    assert_eq!(
        continuous.agents.stranded_report().junctions,
        [(1, 3), (2, 2), (3, 2), (4, 1)]
    );
    for &(i, pos) in &at_nodes {
        let agents = &continuous.agents;
        assert_eq!(
            (agents.pos_x[i], agents.pos_y[i]),
            (pos.x, pos.z),
            "not moved"
        );
    }
    for (k, &lane) in lanes.iter().enumerate() {
        assert_eq!(continuous.agents.current_lane_id[at_nodes[k].0], lane);
    }
    let spur = at_nodes[2].0;
    assert_eq!(continuous.agents.current_lane_id[spur], usize::MAX);
    assert_eq!(continuous.agents.current_node[spur], 4);
    // Mid-backoff, then save and load.
    let retried = |core: &SimCore, step: u8| {
        (0..core.agents.len()).all(|i| core.agents.network_replan_failures[i] >= step)
    };
    let mut frames = 0;
    while !retried(&continuous, 1) {
        continuous.step_frame(FRAME_DT_S);
        frames += 1;
        assert!(frames < 20 * MAX_FRAMES, "{:?}", walkers_state(&continuous));
    }
    let path = temp_save_path("stranded_walkers_at_nodes");
    continuous
        .save_game_settled_at(path.to_str().unwrap(), None, 0)
        .unwrap();
    let mut loaded = test_core();
    loaded.load_game_internal(path.to_str().unwrap()).unwrap();
    let _ = std::fs::remove_file(&path);
    loaded.time.speed_multiplier = continuous.time.speed_multiplier;
    assert_eq!(walkers_state(&loaded), walkers_state(&continuous));
    assert_eq!(
        loaded.agents.stranded_report(),
        continuous.agents.stranded_report()
    );
    let mut frames = 0;
    while !retried(&continuous, 3) {
        continuous.step_frame(FRAME_DT_S);
        loaded.step_frame(FRAME_DT_S);
        assert_eq!(walkers_state(&loaded), walkers_state(&continuous));
        frames += 1;
        assert!(
            frames < 100 * MAX_FRAMES,
            "{:?}",
            walkers_state(&continuous)
        );
    }
    assert_eq!(
        loaded.agents.stranded_report(),
        continuous.agents.stranded_report()
    );
    assert!(
        !continuous
            .agents
            .trip_failures
            .lines
            .iter()
            .any(|line| line.contains("sent home") || line.contains("sent to border")),
        "{:#?}",
        continuous.agents.trip_failures.lines
    );
}

// `TRAFFIC-19` (round T19b): a walker in the middle of the block, its road gone from under it, is
// off every sidewalk by more than a road edit's reattach reach. It walks across the ground to the
// nearest one. Saved mid-walk and loaded, the city continues as the unsaved one, frame for frame,
// until it is on that sidewalk.
#[test]
fn a_walker_off_every_sidewalk_walks_to_one_and_continues_across_a_save_file() {
    let mut continuous = stranded_core();
    let exit = foot_lane(&continuous, 3, true, true) as u32;
    let centre = (corner(0) + corner(2)) * 0.5;
    let agents = &mut continuous.agents;
    let walker = agents.spawn_housed_agent(usize::MAX, centre.x, centre.z);
    agents.transit_mode[walker] = MODE_WALK;
    agents.transit[walker] = TRANSIT_NETWORK;
    agents.speed[walker] = AGENT_WALK_SPEED_MS;
    agents.current_building[walker] = usize::MAX;
    agents.current_edge[walker] = usize::MAX;
    agents.current_node[walker] = 3;
    agents.current_lane_id[walker] = usize::MAX;
    agents.access_flags[walker] = ACCESS_PLAN_VALID;
    agents.planned_detach_node[walker] = 3;
    agents.planned_detach_lane_id[walker] = exit;
    agents.planned_detach_lane_d[walker] = 10.0;
    let walking = |core: &SimCore| core.agents.access_flags[walker] & ACCESS_WALK_TO_SIDEWALK != 0;
    let mut frames = 0;
    while !walking(&continuous) || frames < 600 {
        continuous.step_frame(FRAME_DT_S);
        frames += 1;
        assert!(frames < MAX_FRAMES, "{:?}", walkers_state(&continuous));
    }
    let moved = godot::prelude::Vector2::new(
        continuous.agents.pos_x[walker],
        continuous.agents.pos_y[walker],
    )
    .distance_to(godot::prelude::Vector2::new(centre.x, centre.z));
    assert!(moved > 5.0, "it is on its way: {moved} m");
    let path = temp_save_path("walker_off_every_sidewalk");
    continuous
        .save_game_settled_at(path.to_str().unwrap(), None, 0)
        .unwrap();
    let mut loaded = test_core();
    loaded.load_game_internal(path.to_str().unwrap()).unwrap();
    let _ = std::fs::remove_file(&path);
    loaded.time.speed_multiplier = continuous.time.speed_multiplier;
    assert_eq!(walkers_state(&loaded), walkers_state(&continuous));
    assert!(walking(&loaded), "the walk survives the save");
    let mut frames = 0;
    while continuous.agents.current_lane_id[walker] == usize::MAX {
        continuous.step_frame(FRAME_DT_S);
        loaded.step_frame(FRAME_DT_S);
        assert_eq!(walkers_state(&loaded), walkers_state(&continuous));
        frames += 1;
        assert!(frames < 20 * MAX_FRAMES, "{:?}", walkers_state(&continuous));
    }
    let lane = continuous.agents.current_lane_id[walker];
    assert_eq!(
        continuous.transit_network.lane_system.lanes[lane].lane_type,
        LaneType::Foot
    );
    let lines = &continuous.agents.trip_failures.lines;
    assert!(
        lines
            .iter()
            .any(|line| line.contains(&format!("agent {walker} "))
                && line.contains("m to the nearest one and plans from there")),
        "{lines:#?}"
    );
}

// `PERF-13`: two walkers whose trip was taken from them on the way (no plan and no destination,
// as when a job ends mid-walk) walk to the end of their sidewalk on road 2. The first keeps its
// lane there with its cursor one past its empty path (`TRAFFIC-19`), the second waits behind it.
// Saved and loaded, the city continues as the unsaved one, frame for frame: the loader took the
// first walker off its lane as a stale route, so the second walked on to the end.
#[test]
fn walkers_kept_at_their_path_end_continue_across_a_save_file() {
    let mut continuous = stranded_core();
    let lane = foot_lane(&continuous, 2, false, true);
    let length = continuous.transit_network.lane_system.lanes[lane].length;
    let mut kept = [0; 2];
    for (k, walker) in kept.iter_mut().enumerate() {
        let agents = &mut continuous.agents;
        let i = agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
        agents.transit_mode[i] = MODE_WALK;
        agents.transit[i] = TRANSIT_NETWORK;
        agents.speed[i] = AGENT_WALK_SPEED_MS;
        agents.current_building[i] = usize::MAX;
        agents.current_edge[i] = 2;
        agents.current_node[i] = 2;
        agents.current_lane_id[i] = lane;
        agents.lane_distance[i] = length - 20.0 - 4.0 * k as f32;
        *walker = i;
    }
    // Every agent's walker state with its road and route cursor.
    let state = |core: &SimCore| {
        let agents = &core.agents;
        let cursors: Vec<_> = (0..agents.len())
            .map(|i| (agents.current_edge[i], agents.current_path_index[i]))
            .collect();
        (walkers_state(core), cursors)
    };
    for _ in 0..600 {
        continuous.step_frame(FRAME_DT_S);
    }
    let agents = &continuous.agents;
    assert_eq!(
        (
            agents.current_lane_id[kept[0]],
            agents.lane_distance[kept[0]],
            agents.current_path_index[kept[0]],
            agents.current_path[kept[0]].len(),
        ),
        (lane, length, 1, 0),
        "the first walker waits at its lane's end"
    );
    assert_eq!(agents.current_lane_id[kept[1]], lane);
    assert!(
        agents.lane_distance[kept[1]] < length,
        "the second walker waits behind it"
    );
    let path = temp_save_path("walkers_kept_at_path_end");
    continuous
        .save_game_settled_at(path.to_str().unwrap(), None, 0)
        .unwrap();
    let mut loaded = test_core();
    loaded.load_game_internal(path.to_str().unwrap()).unwrap();
    let _ = std::fs::remove_file(&path);
    loaded.time.speed_multiplier = continuous.time.speed_multiplier;
    assert_eq!(state(&loaded), state(&continuous));
    for _ in 0..600 {
        continuous.step_frame(FRAME_DT_S);
        loaded.step_frame(FRAME_DT_S);
        assert_eq!(state(&loaded), state(&continuous));
    }
}

// A walker's lane, node, distance, position, flags, backoff step and next plan time, bitwise.
type WalkerState = (usize, u32, u32, u32, u32, u8, u8, u32);

fn walkers_state(core: &SimCore) -> Vec<WalkerState> {
    let agents = &core.agents;
    (0..agents.len())
        .map(|i| {
            (
                agents.current_lane_id[i],
                agents.current_node[i],
                agents.lane_distance[i].to_bits(),
                agents.pos_x[i].to_bits(),
                agents.pos_y[i].to_bits(),
                agents.access_flags[i],
                agents.network_replan_failures[i],
                agents.next_replan_time[i].to_bits(),
            )
        })
        .collect()
}

// Writes the stranded block as the Godot stranded-walker notice test's fixture, saved once the
// report names every walker.
#[test]
#[ignore = "fixture recorder: writes the stranded block city to METRUM_STRANDED_FIXTURE"]
fn record_stranded_walkers_fixture() {
    let path = std::env::var("METRUM_STRANDED_FIXTURE")
        .expect("METRUM_STRANDED_FIXTURE: path of the fixture save to write");
    let mut core = stranded_core();
    let frames = run_until_reported(&mut core);
    let report = core.agents.stranded_report().clone();
    // Over the block's middle, looking down at it.
    let center = corner(0) + Vector3::new(SIDE_M * 0.5, 0.0, SIDE_M * 0.5);
    let camera = crate::simulation::save::SavedCameraState {
        pivot: [center.x, 0.0, center.z],
        yaw: 0.0,
        pitch: -1.1,
        distance: 260.0,
        orthogonal: false,
    };
    core.save_game_settled_at(&path, Some(camera), 0).unwrap();
    eprintln!(
        "STRANDED_FIXTURE {}",
        serde_json::json!({"path": path, "frames": frames, "agents": core.agents.len(),
            "junctions": report.junctions, "walkers": report.walkers})
    );
}
