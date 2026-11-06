// SPDX-License-Identifier: GPL-2.0-only

//! `PARK-02` parked poses: the per-place cache against a fresh ground solve, bit for bit, across
//! every way the ground under a bay moves. Most fixtures use a place no reconcile refreshes, so
//! only the cache's own invalidation can keep it true; the kerb and lot cases are real supply.

use super::super::road_tunnels::{THROUGH_HILL, cone_hill_core};
use super::*;
use crate::nodes::sim::render::vehicle_ground::VehicleGroundSupport;
use crate::simulation::network::graph::RoadParking;
use crate::simulation::network::types::EdgeClass;
use crate::simulation::parking::{Bay, PlaceSource};
use godot::prelude::Vector2;

type Pose = [u32; 12];

// A bay at `(x, z)` that no road or site owns, with `heading` along the slope's fall line.
fn add_bay(core: &mut SimCore, n: u32, x: f32, z: f32, heading: f32) {
    let y = core.get_world_surface_height_internal(Vector2::new(x, z));
    core.parking.add_place(
        PlaceSource::Kerb {
            edge: u32::MAX - n,
            right: false,
        },
        Bay {
            identity: [u64::from(n), 0],
            position: [x, y, z],
            path: [[x, z]; 4],
            heading,
            width: 2.5,
            length: 6.0,
        },
    );
}

// Publishes as the sim thread does, then checks every parked car against a fresh solve. Returns
// the poses in publication order.
fn fresh_poses(core: &mut SimCore, stage: &str) -> Vec<Pose> {
    core.build_snapshot();
    core.parking
        .published_poses()
        .into_iter()
        .map(|(model, source, position, heading, pose)| {
            let fresh = core.parked_ground_pose(model, source, position, heading);
            let bits = pose.map(f32::to_bits);
            assert_eq!(
                bits,
                fresh.map(f32::to_bits),
                "{stage}: stale pose at {position:?} model {model}: {pose:?}, fresh {fresh:?}"
            );
            bits
        })
        .collect()
}

fn moved(before: &[Pose], after: &[Pose]) -> usize {
    before.iter().zip(after).filter(|(a, b)| a != b).count()
}

// Four cars on a hillside clear of any road, so each stands across the slope on graded ground.
fn hillside() -> SimCore {
    let mut core = cone_hill_core(120.0, 0.25);
    // One 512 m chunk, so a custody change rebuilds all four.
    for (n, (x, z)) in [(40.0, 40.0), (80.0, 40.0), (40.0, 80.0), (80.0, 80.0)]
        .into_iter()
        .enumerate()
    {
        add_bay(&mut core, n as u32, x, z, 0.6);
    }
    core.parking.fill_test_fleet();
    core
}

#[test]
fn cached_poses_equal_a_fresh_solve_on_a_slope_and_stay_put_when_only_custody_changes() {
    let mut core = hillside();
    let before = fresh_poses(&mut core, "first publication");
    assert_eq!(before.len(), 4);
    let flat = f32::to_bits(1.0);
    assert!(
        before.iter().all(|pose| pose[5] != flat),
        "every car stands on the slope"
    );
    // A rebuild that solved instead of copying would overwrite the poison.
    core.parking.poison_cached_poses_for_test();
    let leaver = core.parking.first_parked_car_for_test();
    core.parking.set_moving(leaver, true);
    core.build_snapshot();
    let poses = core.parking.published_poses();
    assert_eq!(poses.len(), 3);
    assert!(
        poses.iter().all(|pose| pose.4[0] == 1234.0),
        "a custody rebuild copies, it does not solve"
    );
}

#[test]
fn a_terrain_edit_and_its_undo_refresh_the_poses_on_it() {
    let mut core = hillside();
    let before = fresh_poses(&mut core, "before");
    core.sculpt_terrain_internal(Vector2::new(40.0, 40.0), 12.0, 1.5);
    let after = fresh_poses(&mut core, "sculpt");
    assert_eq!(moved(&before, &after), 1, "only the bay under the brush");
    assert!(core.undo_action_internal());
    let restored = fresh_poses(&mut core, "undo");
    assert_eq!(restored, before);
}

// A brush under a road: the sync re-grades the road, and the road's own change and the base edit
// each cover the bays on it.
#[test]
fn a_regraded_road_refreshes_the_poses_on_it() {
    let mut core = cone_hill_core(120.0, 0.1);
    core.benchmark_mode = false;
    commit(&mut core, (-100.0, 40.0), (100.0, 40.0));
    add_bay(&mut core, 0, 0.0, 40.0, std::f32::consts::FRAC_PI_2);
    add_bay(&mut core, 1, 30.0, 40.0, std::f32::consts::FRAC_PI_2);
    core.parking.fill_test_fleet();
    let before = fresh_poses(&mut core, "before");
    core.sculpt_terrain_internal(Vector2::new(0.0, 40.0), 24.0, 0.5);
    core.precompute_road_mesh_data();
    let after = fresh_poses(&mut core, "regrade");
    assert!(moved(&before, &after) > 0, "the road moved under the bays");
    assert!(core.undo_action_internal());
    core.precompute_road_mesh_data();
    assert_eq!(fresh_poses(&mut core, "undo"), before);
}

// A lot's pad levels the ground it sits on: a car beside or under it changes without any bay
// being re-added, and bulldozing the lot and undoing it take the ground there and back.
#[test]
fn a_lot_on_graded_ground_refreshes_the_poses_it_moves() {
    let mut core = cone_hill_core(120.0, 0.1);
    core.benchmark_mode = false;
    commit(&mut core, (-100.0, 60.0), (100.0, 60.0));
    let id = register_lot(&mut core);
    let (x, z) = (10.0, 30.0);
    add_bay(&mut core, 0, x + 2.0, z, 0.6);
    add_bay(&mut core, 1, -90.0, -90.0, 0.6);
    core.parking.fill_test_fleet();
    let before = fresh_poses(&mut core, "before");
    let placed = core.place_service_building_internal(&id, x, z);
    assert!(placed.is_ok(), "{placed:?}");
    core.publish_pending_building_site_changes();
    let after = fresh_poses(&mut core, "lot");
    assert_eq!(
        moved(&before, &after),
        1,
        "the bay on the pad, not the far one"
    );
    assert!(core.bulldoze_building(0));
    core.publish_pending_building_site_changes();
    assert_eq!(fresh_poses(&mut core, "bulldoze"), before);
    assert!(core.undo_action_internal());
    core.publish_pending_building_site_changes();
    assert_eq!(fresh_poses(&mut core, "undo"), after);
}

// `ROAD-99`: a deck over a street. A car stands on its own street, so neither the deck nor its
// undo moves the kerb cars under the bridge, however the cache is refreshed; the bays stay.
#[test]
fn a_bridge_over_kerb_bays_leaves_their_cars_on_the_street_and_its_undo_changes_nothing() {
    let mut core = cone_hill_core(0.0, 0.25);
    core.benchmark_mode = false;
    commit_with(
        &mut core,
        (0.0, -60.0),
        (0.0, 60.0),
        RoadParking {
            left: true,
            right: true,
        },
    );
    core.parking.fill_test_fleet();
    let before = fresh_poses(&mut core, "before");
    let places = core.parking.counts().0;
    assert!(places >= 8);
    let outcome = core.commit_road(
        RoadCommitRequest {
            points: vec![Vector3::new(-30.0, 6.0, 0.0), Vector3::new(30.0, 6.0, 0.0)],
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
            preview: None,
            tunnel: false,
            median: None,
            parking: RoadParking::NONE,
            attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
        },
        Instant::now(),
        RoadEditMetrics::default(),
    );
    assert!(outcome.committed, "{}", outcome.rejection);
    core.adopt_pending_cell_lots();
    core.transit_network.settle_cch();
    assert_eq!(core.parking.counts().0, places, "the bays are not re-added");
    let after = fresh_poses(&mut core, "bridge");
    assert_eq!(
        moved(&before, &after),
        0,
        "a car stands on its street, not the deck"
    );
    assert!(core.undo_action_internal());
    core.precompute_road_mesh_data();
    assert_eq!(fresh_poses(&mut core, "undo"), before);
}

// The bay's own level on both sides of the rule: a car parked on a bridge stands on the deck, and
// one on a lot under a deck stands on the ground with no road beneath it.
#[test]
fn a_car_stands_on_the_level_of_its_own_bay() {
    let mut core = cone_hill_core(0.0, 0.25);
    core.benchmark_mode = false;
    commit_with(&mut core, (-60.0, -80.0), (60.0, -80.0), RoadParking::NONE);
    // A bay no road owns on the ground the deck will pass over, off its road.
    add_bay(&mut core, 0, 0.0, 0.0, 0.0);
    core.parking.fill_test_fleet();
    let ground = fresh_poses(&mut core, "ground");
    let outcome = core.commit_road(
        RoadCommitRequest {
            points: vec![Vector3::new(-40.0, 8.0, 0.0), Vector3::new(40.0, 8.0, 0.0)],
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
            preview: None,
            tunnel: false,
            median: None,
            parking: RoadParking {
                left: true,
                right: true,
            },
            attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
        },
        Instant::now(),
        RoadEditMetrics::default(),
    );
    assert!(outcome.committed, "{}", outcome.rejection);
    core.adopt_pending_cell_lots();
    core.transit_network.settle_cch();
    assert!(core.parking.counts().0 > 1, "the bridge has its own bays");
    core.parking.fill_test_fleet();
    let deck_edge = core
        .region_graph
        .edges()
        .iter()
        .position(|edge| !edge.deleted && edge.class == EdgeClass::Bridge)
        .expect("a bridge");
    core.build_snapshot();
    let poses = core.parking.published_poses();
    let mut on_deck = 0;
    for (model, source, position, heading, pose) in poses {
        let bits = pose.map(f32::to_bits);
        assert_eq!(
            bits,
            core.parked_ground_pose(model, source, position, heading)
                .map(f32::to_bits)
        );
        match source {
            PlaceSource::Kerb { edge, .. } if edge as usize == deck_edge => {
                assert!(
                    (pose[7] - position[1]).abs() < 0.5,
                    "a bridge's car on its deck: {} at {position:?}",
                    pose[7],
                );
                on_deck += 1;
            }
            _ => assert_eq!(bits, ground[0], "the lot's car stays on the ground"),
        }
    }
    assert!(on_deck > 0, "cars on the deck");
}

// A tunnel's approach cuts the hill: the ground beside its portal drops with no bay involved.
#[test]
fn a_tunnel_portal_refreshes_the_poses_beside_it_and_its_undo_restores_them() {
    let mut core = cone_hill_core(80.0, 0.25);
    core.benchmark_mode = false;
    for (n, x) in [
        -76.0_f32, -70.0, -64.0, -58.0, -52.0, 52.0, 58.0, 64.0, 70.0, 76.0,
    ]
    .into_iter()
    .enumerate()
    {
        add_bay(&mut core, n as u32, x, 6.0, 0.0);
    }
    core.parking.fill_test_fleet();
    let before = fresh_poses(&mut core, "before");
    let outcome = core.commit_road(
        RoadCommitRequest {
            points: THROUGH_HILL.to_vec(),
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
            preview: None,
            tunnel: true,
            median: None,
            parking: RoadParking::NONE,
            attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
        },
        Instant::now(),
        RoadEditMetrics::default(),
    );
    assert!(outcome.committed, "{}", outcome.rejection);
    core.adopt_pending_cell_lots();
    core.transit_network.settle_cch();
    let after = fresh_poses(&mut core, "tunnel");
    assert!(moved(&before, &after) > 0, "the cut moved the ground");
    assert!(core.undo_action_internal());
    core.precompute_road_mesh_data();
    assert_eq!(fresh_poses(&mut core, "undo"), before);
}

// The cache is not saved: a load starts with no pose, rebuilds every one from the loaded ground
// at the first snapshot, and arrives at the same bits as before the save.
#[test]
fn a_load_rebuilds_every_pose_from_the_loaded_ground() {
    let mut core = SimCore::new(
        WorldConfig::new(512.0, 512.0, 16.0, 10.0)
            .with_chunking(128.0, 0.0)
            .with_terrain_resolution(2.0),
    );
    core.treasury.balance = 1_000_000.0;
    core.sculpt_terrain_internal(Vector2::new(0.0, 0.0), 200.0, 2.0);
    commit_with(
        &mut core,
        (-100.0, 40.0),
        (100.0, 40.0),
        RoadParking {
            left: true,
            right: true,
        },
    );
    core.parking.fill_test_fleet();
    let mut before = fresh_poses(&mut core, "before");
    let flat = f32::to_bits(1.0);
    assert!(
        before.iter().any(|pose| pose[5] != flat),
        "the street climbs the bump"
    );
    let path = temp_save_path("parked-poses");
    core.save_game_settled_at(path.to_str().unwrap(), None, 0)
        .unwrap();
    core.load_game_internal(path.to_str().unwrap()).unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(
        core.parking.solved_poses_for_test(),
        0,
        "nothing came from the save"
    );
    let mut after = fresh_poses(&mut core, "load");
    assert_eq!(core.parking.solved_poses_for_test(), before.len());
    before.sort_unstable();
    after.sort_unstable();
    assert_eq!(after, before);
}

// A loaded mesh replaces a model's contacts, and with them its pose on every bay.
#[test]
fn a_new_car_profile_refreshes_that_models_poses() {
    let mut core = hillside();
    let before = fresh_poses(&mut core, "before");
    let profile = VehicleGroundSupport::new([-1.4, -3.0, 1.4, 3.0], &[]).unwrap();
    for model in 0..4 {
        assert!(core.set_vehicle_ground_support(model, profile.clone()));
    }
    assert!(!core.set_vehicle_ground_support(9, profile));
    let after = fresh_poses(&mut core, "profile");
    assert_eq!(moved(&before, &after), before.len());
}
