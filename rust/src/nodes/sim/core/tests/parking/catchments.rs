// SPDX-License-Identifier: GPL-2.0-only

//! Door catchments stay what a load would rebuild: after every site edit, and once the frame
//! that adopts a road edit has run. `audit_every_parking_catchment` is that rebuild.

use super::*;
use crate::simulation::network::graph::RoadParking;

const BOTH: RoadParking = RoadParking {
    left: true,
    right: true,
};

// A street and a cross street with kerbside bays, and a lot on the street.
fn street_with_lot() -> (SimCore, String) {
    let mut c = core_with(BOTH);
    commit_with(&mut c, (0.0, 0.0), (0.0, 120.0), BOTH);
    settle(&mut c);
    let id = register_lot(&mut c);
    c.place_service_building_internal(&id, -60.0, -30.0).unwrap();
    c.publish_pending_building_site_changes();
    (c, id)
}

fn core_with(parking: RoadParking) -> SimCore {
    let mut core = SimCore::new(
        WorldConfig::new(512.0, 512.0, 16.0, 10.0)
            .with_chunking(128.0, 0.0)
            .with_terrain_resolution(2.0),
    );
    core.treasury.balance = 1_000_000.0;
    commit_with(&mut core, (-120.0, 0.0), (120.0, 0.0), parking);
    core
}

// Runs the frames a commit's routing hold takes, paused: nothing else moves.
fn settle(c: &mut SimCore) {
    c.time.speed_multiplier = 0.0;
    for _ in 0..3 {
        c.step_frame(1.0 / 60.0);
    }
}

#[test]
fn every_site_edit_leaves_the_lists_a_rebuild_gives() {
    let (mut c, id) = street_with_lot();
    c.audit_every_parking_catchment();
    // A second lot: its bays enter the first lot's list and its driveway takes kerbside bays.
    c.place_service_building_internal(&id, 60.0, -30.0).unwrap();
    c.publish_pending_building_site_changes();
    c.audit_every_parking_catchment();
    let bays = c.parking.counts().0;
    assert!(c.bulldoze_building(0));
    assert!(c.parking.counts().0 < bays);
    c.audit_every_parking_catchment();
    assert!(c.undo_action_internal());
    c.precompute_road_mesh_data();
    c.publish_road_tool_view();
    c.publish_pending_building_site_changes();
    settle(&mut c);
    assert_eq!(c.parking.counts().0, bays);
    c.audit_every_parking_catchment();
}

#[test]
fn a_door_loses_the_ends_of_the_last_bays_on_a_street() {
    // Lots on two streets without kerbside bays: each door lists the other lot's bays, which
    // alone give it that street's end.
    let mut c = core_with(RoadParking::NONE);
    commit(&mut c, (0.0, 0.0), (0.0, 120.0));
    settle(&mut c);
    let id = register_lot(&mut c);
    c.place_service_building_internal(&id, -60.0, -30.0).unwrap();
    c.place_service_building_internal(&id, 30.0, 60.0).unwrap();
    c.publish_pending_building_site_changes();
    let other = c.allocator.building_handles.handle(1).to_bits();
    let ends = c.parking.economy_ends(other).expect("a list").targets.to_vec();
    assert!(c.bulldoze_building(0));
    c.audit_every_parking_catchment();
    let left = c.parking.economy_ends(other).expect("a list").targets.to_vec();
    assert!(left.len() < ends.len(), "{left:?} of {ends:?}");
}

#[test]
fn a_road_commit_leaves_its_doors_to_the_frame_that_adopts_it() {
    let (mut c, _) = street_with_lot();
    let revision = c.parking.candidate_revision();
    commit_before_its_frame(&mut c, (-120.0, 60.0), (120.0, 60.0), BOTH);
    assert!(c.parking.frontages_deferred());
    assert_eq!(c.parking.candidate_revision(), revision);
    settle(&mut c);
    assert!(!c.parking.frontages_deferred());
    c.audit_every_parking_catchment();
}

#[test]
fn a_site_edit_in_the_hold_publishes_the_road_edit_first() {
    let (mut c, id) = street_with_lot();
    commit_before_its_frame(&mut c, (-120.0, 60.0), (120.0, 60.0), BOTH);
    c.place_service_building_internal(&id, 60.0, -30.0).unwrap();
    c.publish_pending_building_site_changes();
    assert!(!c.parking.frontages_deferred());
    c.audit_every_parking_catchment();
}

#[test]
fn a_save_in_the_hold_publishes_the_road_edit_first() {
    let (mut c, _) = street_with_lot();
    commit_before_its_frame(&mut c, (-120.0, 60.0), (120.0, 60.0), BOTH);
    let path = std::env::temp_dir().join(format!("catchments_{}.sqlite", std::process::id()));
    c.save_game_settled(path.to_str().unwrap(), None).unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(!c.parking.frontages_deferred());
    c.audit_every_parking_catchment();
}
