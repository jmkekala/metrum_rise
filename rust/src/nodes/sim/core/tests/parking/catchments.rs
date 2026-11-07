// SPDX-License-Identifier: GPL-2.0-only

//! Door catchments stay what a load would rebuild: after every site edit, and once the frame
//! that adopts a road edit has run. `audit_every_parking_catchment` is that rebuild.

use super::*;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::graph::RoadParking;
use crate::simulation::parking::{Bay, PlaceSource};

const BOTH: RoadParking = RoadParking {
    left: true,
    right: true,
};

// [`street_with_lot`] and a second lot, whose door is one no publication of the first rebuilds.
fn street_with_two_lots() -> (SimCore, String) {
    let (mut c, id) = street_with_lot();
    c.place_service_building_internal(&id, 60.0, -30.0).unwrap();
    c.publish_pending_building_site_changes();
    (c, id)
}

// A street and a cross street with kerbside bays, and a lot on the street.
fn street_with_lot() -> (SimCore, String) {
    let mut c = core_with(BOTH);
    commit_with(&mut c, (0.0, 0.0), (0.0, 120.0), BOTH);
    settle(&mut c);
    let id = register_lot(&mut c);
    c.place_service_building_internal(&id, -60.0, -30.0)
        .unwrap();
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

// The kerb with the most bays, and those bays.
fn busiest_kerb(c: &SimCore) -> (PlaceSource, Vec<Bay>) {
    (0..c.region_graph.edge_count() as u32)
        .flat_map(|edge| [false, true].map(|right| PlaceSource::Kerb { edge, right }))
        .map(|source| {
            let bays = c.parking.source_bays(source);
            (
                source,
                bays.into_iter().map(|(_, bay)| bay).collect::<Vec<_>>(),
            )
        })
        .max_by_key(|(_, bays)| bays.len())
        .expect("a road")
}

// Publishes a site edit of the lot that changes none of its own bays.
fn publish_lot(c: &mut SimCore) {
    let lot = c.allocator.building_handles.handle(0);
    c.allocator.parking_site_changes.push(lot);
    c.publish_parking_sites();
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
fn a_bay_on_a_new_path_moves_in_the_lists() {
    let (mut c, _) = street_with_two_lots();
    let (source, mut bays) = busiest_kerb(&c);
    // The same pose, reached from three metres along the kerb.
    bays[0].path[0][0] += 3.0;
    c.parking.replace_source(source, &bays);
    publish_lot(&mut c);
    c.audit_every_parking_catchment();
}

#[test]
fn a_bay_follows_its_hosts_door_across_the_road() {
    let (mut c, _) = street_with_two_lots();
    let lanes = &c.transit_network.lane_system;
    let door = &mut c.allocator.entrances[0];
    door.side = -door.side;
    BuildingAllocator::refresh_entrance_lanes(door, &c.region_graph, lanes);
    publish_lot(&mut c);
    c.audit_every_parking_catchment();
}

#[test]
fn a_walking_tie_falls_to_the_driveway_time() {
    // Either bay behind the longer drive: a freed handle may make the newer one the lower, and
    // only a drive against the handles' order tells the driveway time from the place.
    for newer_goes_round in [false, true] {
        let (mut c, _) = street_with_two_lots();
        let (source, mut bays) = busiest_kerb(&c);
        // Two bays either side of one kerb point. A quarter metre holds the two distances to the
        // kerb exact, so the walks tie to the bit.
        let [x, z] = bays[0].path[0].map(|v| (v * 4.0).round() / 4.0);
        let straight = |end: f32| [[x, z], [x, z], [x, z], [x, end]];
        let round = |end: f32| [[x, z], [x + 4.0, z], [x, z], [x, end]];
        let mut twin = bays[0];
        bays[0].position = [x, bays[0].position[1], z + 2.0];
        twin.position = [x, bays[0].position[1], z - 2.0];
        bays[0].path = if newer_goes_round {
            straight(z + 2.0)
        } else {
            round(z + 2.0)
        };
        twin.path = if newer_goes_round {
            round(z - 2.0)
        } else {
            straight(z - 2.0)
        };
        c.parking.replace_source(source, &bays);
        publish_lot(&mut c);
        bays.push(twin);
        c.parking.replace_source(source, &bays);
        publish_lot(&mut c);
        c.audit_every_parking_catchment();
        let door = c.allocator.building_handles.handle(1).to_bits();
        let list = c.parking.candidate_products(door);
        let walk = |bay: &Bay| {
            let position = bay.position.map(f32::to_bits);
            list.iter()
                .find(|row| row.0 == position)
                .expect("in the list")
                .1
        };
        assert_eq!(walk(&twin), walk(&bays[0]), "the walks tie");
    }
}

#[test]
fn a_door_whose_list_changes_wakes_its_own_chunk() {
    // The lots stand in different chunks, either side of x = 0.
    let (mut c, _) = street_with_two_lots();
    let door = c.allocator.entrances[1].door_pos;
    let size = crate::simulation::network::graph::RegionGraph::CHUNK_SIZE;
    let chunk = (
        (door.x / size).floor() as i32,
        (door.y / size).floor() as i32,
    );
    for key in std::mem::take(&mut c.parking.release_events) {
        c.parking.finish_supply_event(key);
    }
    assert!(c.bulldoze_building(0));
    assert!(c.parking.release_events.contains(&chunk));
}

#[test]
fn a_site_edit_rebinds_the_bay_a_driving_car_keeps() {
    use crate::simulation::parking::ParkingTripPhase;
    // Two lots far enough apart to drive between, and a commuter on its way to the far one.
    let mut c = SimCore::new(
        WorldConfig::new(2048.0, 1024.0, 16.0, 10.0)
            .with_chunking(128.0, 0.0)
            .with_terrain_resolution(2.0),
    );
    c.treasury.balance = 10_000_000.0;
    commit_with(&mut c, (-600.0, 0.0), (600.0, 0.0), BOTH);
    let id = register_lot(&mut c);
    let mut manifest = c.allocator.registry.get(&id).unwrap().manifest.clone();
    manifest.building.as_mut().unwrap().household_capacity = Some(4);
    c.allocator
        .registry
        .register("parking", manifest, String::new());
    c.place_service_building_internal(&id, -450.0, -30.0)
        .unwrap();
    c.place_service_building_internal(&id, 450.0, -30.0)
        .unwrap();
    c.publish_pending_building_site_changes();
    let (home, target) = if c.allocator.entrances[0].side == -1 {
        (0, 1)
    } else {
        (1, 0)
    };
    let agent = c.debug_add_parking_commuter(home, target).unwrap();
    let handle = c.agents.agent_handle(agent);
    c.agents.release_departure(agent);
    c.time.speed_multiplier = 8.0;
    let driving = |c: &SimCore| {
        c.agents
            .resolve_agent(handle)
            .is_some_and(|a| c.agents.parking_trip[a].phase == ParkingTripPhase::Drive)
    };
    for _ in 0..6_000 {
        if driving(&c) {
            break;
        }
        c.step_frame(1.0 / 60.0);
    }
    assert!(driving(&c), "the commuter is on its way");
    // The audit behind the publication finds every kept bay rebound.
    assert!(c.bulldoze_building(target));
    assert!(c.parking.vehicle_breakdown().moving > 0);
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
    c.place_service_building_internal(&id, -60.0, -30.0)
        .unwrap();
    c.place_service_building_internal(&id, 30.0, 60.0).unwrap();
    c.publish_pending_building_site_changes();
    let other = c.allocator.building_handles.handle(1).to_bits();
    let ends = c
        .parking
        .economy_ends(other)
        .expect("a list")
        .targets
        .to_vec();
    assert!(c.bulldoze_building(0));
    c.audit_every_parking_catchment();
    let left = c
        .parking
        .economy_ends(other)
        .expect("a list")
        .targets
        .to_vec();
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
