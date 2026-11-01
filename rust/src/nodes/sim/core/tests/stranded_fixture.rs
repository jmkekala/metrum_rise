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
    ACCESS_PLAN_VALID, ACCESS_WALK_STRANDED, MODE_WALK, TRANSIT_NETWORK,
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
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
        });
    }
    core.region_graph.rebuild_adjacency_list();
    for node in 0..4u32 {
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
