// SPDX-License-Identifier: GPL-2.0-only

//! `TRAFFIC-14` saturation fixture as a whole city: one block as a closed ring of four one-way
//! roads with no way out, full of trucks on looping routes and more waiting at every corner,
//! run through the scheduler until the gridlock check reports the ring's junctions. The same
//! city saved to a file and loaded between two checks continues as the unsaved one, and an
//! ignored recorder writes it as the locked fixture the Godot gridlock notice test loads.
//! Minimal setup: no buildings, zones or economy beyond what `SimCore::new` holds.

use super::*;
use crate::nodes::sim::core::frame::FRAME_DT_S;
use crate::simulation::core::time::SIMULATION_SPEED_STEPS;
use crate::simulation::economy::agents::{
    ACCESS_FREIGHT_BORDER_DESTINATION, ACCESS_PLAN_VALID, MODE_CAR, TRANSIT_NETWORK,
};
use crate::simulation::pathing::cch::CchGraph;

// Block corners, counter-clockwise; edge k runs from corner k to corner k + 1.
const CORNERS: [(f32, f32); 4] = [(0.0, 0.0), (400.0, 0.0), (400.0, 50.0), (0.0, 50.0)];
// Junction mouths: every road lane stops this far short of both of its nodes.
const MOUTH_M: f32 = 10.0;
// Trucks along every road lane, and waiting at every corner to start there.
const SPACING_M: f32 = 12.0;
const WAITING: usize = 120;
// Laps in every route: far more than any run drives.
const LAPS: usize = 40;
// Frames the ring is given to lock and be reported.
const MAX_FRAMES: usize = 20_000;

fn corner(k: usize) -> Vector3 {
    let (x, z) = CORNERS[k % 4];
    Vector3::new(x, 0.0, z)
}

// The locked ring's city before its first frame: nodes 0..4 at the corners, edge k from
// corner k to k + 1 with one forward lane.
fn ring_core() -> SimCore {
    // A 2 km world keeps the saved fixture small.
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
            bkw_lanes: 0,
            speed_limit: 13.89,
            base_cost: length / 13.89,
            physical_length: length,
            start_clip: MOUTH_M,
            end_clip: MOUTH_M,
            geometry: vec![a, b],
            physical_geometry: vec![a, b],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
        });
    }
    core.transit_network
        .lane_system
        .rebuild(&mut core.region_graph);
    core.transit_network.cch_graph = CchGraph::build(&core.region_graph);
    let truck = |core: &mut SimCore, next: usize| {
        let agents = &mut core.agents;
        let i = agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
        agents.transit_mode[i] = MODE_CAR;
        agents.transit[i] = TRANSIT_NETWORK;
        agents.current_path[i] = (0..LAPS * 4)
            .map(|step| ((next + step) % 4) as u32)
            .collect();
        agents.current_path_index[i] = 0;
        agents.access_flags[i] = ACCESS_PLAN_VALID | ACCESS_FREIGHT_BORDER_DESTINATION;
        agents.freight_target_border_node[i] = *agents.current_path[i].last().unwrap();
        agents.freight_shipment_id[i] = 1_000 + i as u64;
        i
    };
    for k in 0..4 {
        let lane = core.transit_network.lane_system.edge_lanes[&k][0];
        let length = core.transit_network.lane_system.lanes[lane].length;
        let mut d = 1.0;
        while d < length {
            let i = truck(&mut core, (k + 1) % 4);
            core.agents.current_lane_id[i] = lane;
            core.agents.current_edge[i] = k;
            core.agents.current_node[i] = k as u32;
            core.agents.lane_distance[i] = d;
            d += SPACING_M;
        }
    }
    for k in 0..4 {
        for _ in 0..WAITING {
            let i = truck(&mut core, (k + 1) % 4);
            core.agents.current_node[i] = k as u32;
        }
    }
    // The fastest game speed: about half a second of driving per frame, as the ring fixture's
    // ticks drive.
    core.time.speed_multiplier = *SIMULATION_SPEED_STEPS.last().unwrap();
    core
}

// Steps `core` until a gridlock check reports saturation, and returns the frames stepped.
fn run_until_reported(core: &mut SimCore) -> usize {
    for frame in 1..=MAX_FRAMES {
        core.step_frame(FRAME_DT_S);
        if core.agents.gridlock.report().saturated > 0 {
            return frame;
        }
    }
    panic!("the ring did not lock: {:?}", core.agents.gridlock.report());
}

// Steps `core` until the next gridlock check has run.
fn run_to_next_check(core: &mut SimCore) {
    let checks = core.agents.gridlock.report().checks;
    while core.agents.gridlock.report().checks == checks {
        core.step_frame(FRAME_DT_S);
    }
}

// Every car's lane, distance and speed bits, in agent order.
fn cars(core: &SimCore) -> Vec<(usize, u32, u32)> {
    let agents = &core.agents;
    (0..agents.len())
        .map(|i| {
            (
                agents.current_lane_id[i],
                agents.lane_distance[i].to_bits(),
                agents.speed[i].to_bits(),
            )
        })
        .collect()
}

#[test]
fn traffic14_saturated_ring_continues_across_a_save_file() {
    // The ring locks and its four corners are reported. Saved between two checks to a file
    // and loaded (save format 109 carries the checker), the city continues as the unsaved one:
    // same reports at the next checks, same cars, nobody removed.
    let mut continuous = ring_core();
    let n = continuous.agents.len();
    run_until_reported(&mut continuous);
    let report = continuous.agents.gridlock.report().clone();
    assert_eq!(report.junctions, [0, 1, 2, 3], "{report:?}");
    assert_eq!(report.detours, 0, "{report:?}");
    // One frame past a check and short of the next: the save falls between two checks.
    run_to_next_check(&mut continuous);
    let checks = continuous.agents.gridlock.report().checks;
    continuous.step_frame(FRAME_DT_S);
    assert_eq!(continuous.agents.gridlock.report().checks, checks);
    let path = temp_save_path("gridlock_ring");
    continuous
        .save_game_settled_at(path.to_str().unwrap(), None, 0)
        .unwrap();
    let mut loaded = test_core();
    loaded.load_game_internal(path.to_str().unwrap()).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(
        loaded.agents.gridlock.report(),
        continuous.agents.gridlock.report()
    );
    loaded.time.speed_multiplier = continuous.time.speed_multiplier;
    for _ in 0..3 {
        run_to_next_check(&mut continuous);
        run_to_next_check(&mut loaded);
        assert_eq!(
            loaded.agents.gridlock.report(),
            continuous.agents.gridlock.report()
        );
        assert_eq!(cars(&loaded), cars(&continuous));
    }
    assert_eq!(loaded.agents.len(), n, "no car is removed");
    let last = loaded.agents.gridlock.report();
    assert!(
        last.saturation_reports > report.saturation_reports,
        "{last:?}"
    );
}

// Writes the locked ring as the Godot gridlock notice test's fixture: saved at the frame its
// first saturation report lands, so a load shows the notice before its first check.
#[test]
#[ignore = "fixture recorder: writes the locked ring city to METRUM_GRIDLOCK_FIXTURE"]
fn record_gridlock_saturation_fixture() {
    let path = std::env::var("METRUM_GRIDLOCK_FIXTURE")
        .expect("METRUM_GRIDLOCK_FIXTURE: path of the fixture save to write");
    let mut core = ring_core();
    let frames = run_until_reported(&mut core);
    let report = core.agents.gridlock.report().clone();
    // Over the block's middle, looking down at the whole ring.
    let camera = crate::simulation::save::SavedCameraState {
        pivot: [200.0, 0.0, 25.0],
        yaw: 0.0,
        pitch: -0.9,
        distance: 420.0,
        orthogonal: false,
    };
    core.save_game_settled_at(&path, Some(camera), 0).unwrap();
    eprintln!(
        "GRIDLOCK_FIXTURE {}",
        serde_json::json!({"path": path, "frames": frames, "agents": core.agents.len(),
            "junctions": report.junctions, "checks": report.checks,
            "saturation_reports": report.saturation_reports})
    );
}
