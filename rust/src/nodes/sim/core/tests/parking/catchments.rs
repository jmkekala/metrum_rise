// SPDX-License-Identifier: GPL-3.0-or-later

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

// Graded terrain and a street bending round four lots, with a cross street from its apex placed
// before the lots (`cross_first`) or not at all: on it a lane's lift onto the roadbed changes the
// lane's length (`PARK-14`).
fn graded_curve_with_lots(cross_first: bool) -> SimCore {
    let mut c = SimCore::new(
        WorldConfig::new(512.0, 512.0, 16.0, 10.0)
            .with_chunking(128.0, 0.0)
            .with_terrain_resolution(2.0),
    );
    c.treasury.balance = 1_000_000.0;
    let (half_x, half_z) = c.heightmap.half_world_extents();
    let cell = c.heightmap.cell_size_m();
    for z in 0..c.heightmap.height {
        for x in 0..c.heightmap.width {
            let (wx, wz) = (x as f32 * cell - half_x, z as f32 * cell - half_z);
            let height = 30.0 + 0.08 * wx + 3.0 * (wx / 17.0).sin() + 2.0 * (wz / 13.0).sin();
            c.heightmap
                .set_height(x, z, height);
        }
    }
    // The street bends round the lots, so its sidewalks are not its centre line's offsets.
    let curve: Vec<_> = (0..=8)
        .map(|i| {
            let x = -120.0 + 30.0 * i as f32;
            Vector3::new(x, 0.0, 40.0 * (1.0 - (x / 120.0).powi(2)))
        })
        .collect();
    commit_points_before_its_frame(&mut c, curve, BOTH);
    c.publish_deferred_parking_frontages();
    if cross_first {
        commit_with(&mut c, (0.0, 40.0), (0.0, 160.0), BOTH);
    }
    settle(&mut c);
    let id = register_lot(&mut c);
    for x in [-100.0, -40.0, 40.0, 100.0] {
        let z = 40.0 * (1.0 - (x / 120.0_f32).powi(2)) - 22.0;
        c.place_service_building_internal(&id, x, z).unwrap();
    }
    c.publish_pending_building_site_changes();
    settle(&mut c);
    assert_doors_on_the_roadbed(&mut c, "the fixture");
    c
}

// Every live lane stands on the roadbed, every entrance is what a derivation gives and every
// door's list what a rebuild gives (`audit_every_parking_catchment`).
fn assert_doors_on_the_roadbed(c: &mut SimCore, when: &str) {
    let network = &c.transit_network;
    let lanes = &network.lane_system;
    assert_eq!(
        lanes.lanes_off_surface(&c.region_graph, &network.roadbed),
        0,
        "{when}: lanes off the roadbed"
    );
    assert_entrances_derived(c, when);
    c.audit_every_parking_catchment();
}

fn assert_entrances_derived(c: &SimCore, when: &str) {
    let a = &c.allocator;
    let lanes = &c.transit_network.lane_system;
    for b in 0..a.buildings.len() {
        let derived = a.derived_entrance(b, &c.region_graph, lanes);
        assert!(
            a.entrances[b] == derived,
            "{when}: building {b}'s entrance is not a derivation's: {:?} != {derived:?}",
            a.entrances[b]
        );
    }
}

// Running frames, which lift any lane left off the roadbed, then the edit command's own
// publication: neither may move a lane a door's list or kerb was derived from.
fn assert_frames_and_publication_move_no_door(c: &mut SimCore, edit: &str) {
    c.time.speed_multiplier = 1.0;
    for frame in 0..3 {
        c.step_frame(1.0 / 60.0);
        assert_entrances_derived(c, &format!("{edit}, frame {frame}"));
        c.audit_every_parking_catchment();
    }
    c.finalize_network_render_products();
    assert_doors_on_the_roadbed(c, &format!("{edit}, publication"));
}

// The cross street of [`graded_curve_with_lots`].
fn cross_street(c: &SimCore) -> usize {
    let graph = &c.region_graph;
    (0..graph.edge_count())
        .find(|&e| {
            let edge = graph.edge(e);
            let (a, b) = (
                graph.node(edge.start_node).pos,
                graph.node(edge.end_node).pos,
            );
            !edge.deleted && a.x.abs() < 1.0 && b.x.abs() < 1.0
        })
        .expect("the cross street")
}

// A road removal lifts the lanes it rebuilds onto the roadbed before it derives the entrances
// along them and publishes their doors' lists (`PARK-14`). On a graded curve a lane's lift
// changes its length: published before it, the doors' lists went stale at the frame that lifted
// the lanes. A door's kerb is sampled along its lane too, so an entrance derived before the lift
// could differ from a later cache rebuild's by a rounding (the Kuopio save's building 252). Its
// undo lifts the lanes it restores onto the roadbed it restores before it derives the entrances
// and lists the same way (`PARK-16`).
#[test]
fn a_road_removal_on_a_graded_curve_leaves_every_door_and_list_a_rebuild_gives() {
    let mut c = graded_curve_with_lots(true);
    let cross = cross_street(&c);
    assert!(c.bulldoze_road_edge_for_test(cross));
    assert_doors_on_the_roadbed(&mut c, "the removal");
    assert_frames_and_publication_move_no_door(&mut c, "the removal");
    // The undo alone (as tests call it), then the undo command's publication.
    assert!(c.undo_action_internal());
    assert!(!c.region_graph.edge(cross).deleted);
    assert_doors_on_the_roadbed(&mut c, "the undo");
    assert_frames_and_publication_move_no_door(&mut c, "the undo");
}

// A commit lifts the lanes it rebuilds before it derives the entrances along them
// (`PARK-16`): the new street splits the cross street, so the lots on its halves re-derive
// theirs. Both forms: the lot plan deferred to the next frame, and planned in the commit (a
// batch's earlier strokes), whose entrances are derived before the commit's tail.
#[test]
fn a_road_commit_on_a_graded_curve_derives_its_doors_on_lifted_lanes() {
    for in_commit in [false, true] {
        let mut c = graded_curve_with_lots(true);
        let id = register_lot(&mut c);
        for (x, z) in [(-22.0, 90.0), (22.0, 90.0), (-22.0, 140.0), (22.0, 140.0)] {
            c.place_service_building_internal(&id, x, z).unwrap();
        }
        c.publish_pending_building_site_changes();
        settle(&mut c);
        assert_doors_on_the_roadbed(&mut c, "the lots");
        c.cell_lot_hooks.in_commit = in_commit;
        commit_before_its_frame(&mut c, (-80.0, 115.0), (80.0, 115.0), BOTH);
        assert_entrances_derived(&c, &format!("the commit (in_commit {in_commit})"));
        c.publish_deferred_parking_frontages();
        assert_doors_on_the_roadbed(&mut c, "the commit's parking");
        assert_frames_and_publication_move_no_door(&mut c, "the commit");
    }
}

// An in-place edit's publication lifts the lanes the edit rebuilt before it derives the
// entrances (`PARK-16`); its undo, whose record keeps no roadbed, publishes the restored
// roadbed and lifts the lanes before its doors and lists.
#[test]
fn a_cross_section_edit_on_a_graded_curve_and_its_undo_leave_every_door_a_rebuild_gives() {
    let mut c = graded_curve_with_lots(true);
    let graph = &c.region_graph;
    let curve = (0..graph.edge_count())
        .find(|&e| {
            let edge = graph.edge(e);
            !edge.deleted && graph.node(edge.start_node).pos.x < -100.0
        })
        .expect("the curve's west half");
    let attributes = crate::simulation::network::road_types::RoadAttributes::DEFAULT;
    assert!(c.set_edge_cross_section_internal(
        curve as i32,
        1,
        1,
        None,
        RoadParking::NONE,
        attributes
    ));
    c.finalize_network_render_products();
    assert_doors_on_the_roadbed(&mut c, "the edit");
    assert_frames_and_publication_move_no_door(&mut c, "the edit");
    assert!(c.undo_action_internal());
    assert_doors_on_the_roadbed(&mut c, "the undo");
    assert_frames_and_publication_move_no_door(&mut c, "the undo");
}

// Every door's place, kerb and frontage, without the lane ids a local rebuild renumbers.
fn door_places(c: &SimCore) -> Vec<String> {
    c.allocator
        .entrances
        .iter()
        .map(|e| {
            format!(
                "{} {} {:?} {} {:?} {:?} {}",
                e.edge_idx,
                e.side,
                e.vehicle_frontage_access,
                e.entrance_s_m.to_bits(),
                e.door_pos,
                e.curb_pos,
                e.flags
            )
        })
        .collect()
}

// A border connection rebuilds the lanes of its lengthened road's closure (the curve's east end
// meets it): it lifts them before it derives the entrances along them and publishes the doors'
// lists (`PARK-16`, `ROAD-122`). Its undo restores the road, and with it every door's place and
// list as they were.
#[test]
fn a_border_connection_on_graded_ground_lists_doors_on_lifted_lanes() {
    let mut c = graded_curve_with_lots(true);
    let (half_x, _) = c.heightmap.half_world_extents();
    commit_with(&mut c, (120.0, 0.0), (half_x - 1.0, 0.0), BOTH);
    settle(&mut c);
    let border = (0..c.region_graph.node_count() as u32)
        .find(|&node| c.region_graph.node(node).pos.x > half_x - 3.0)
        .expect("the road ends at the world edge");
    let pos = c.region_graph.node(border).pos;
    assert_eq!(c.check_border_candidate_internal(pos), i64::from(border));
    c.finalize_network_render_products();
    let doors = door_places(&c);
    c.set_border_connection_internal(border as i32);
    assert_doors_on_the_roadbed(&mut c, "the border connection");
    assert_frames_and_publication_move_no_door(&mut c, "the border connection");
    assert!(c.undo_action_internal(), "the connection is undone");
    assert_eq!(c.region_graph.node(border).pos, pos);
    assert_doors_on_the_roadbed(&mut c, "the undo");
    assert_frames_and_publication_move_no_door(&mut c, "the undo");
    assert_eq!(door_places(&c), doors, "every door is back in its place");
}

// A terrain stroke's undo restores the dense terrain and rebuilds every lane, and leaves the
// roadbed to the undo command's publication, which lifts the lanes before it derives the
// entrances and publishes the lists (`PARK-16`).
#[test]
fn a_terrain_stroke_undo_on_a_graded_curve_publishes_doors_on_lifted_lanes() {
    let mut c = graded_curve_with_lots(true);
    c.benchmark_mode = false;
    c.start_terrain_stroke_internal();
    c.sculpt_terrain_stroke_step_internal(
        godot::prelude::Vector2::new(0.0, 60.0),
        80.0,
        2.0,
    );
    assert!(c.end_terrain_stroke_internal(), "the stroke is accepted");
    c.finalize_network_render_products();
    assert_doors_on_the_roadbed(&mut c, "the stroke");
    assert!(c.undo_action_internal());
    c.finalize_network_render_products();
    assert_doors_on_the_roadbed(&mut c, "the undo");
    assert_frames_and_publication_move_no_door(&mut c, "the undo");
}
