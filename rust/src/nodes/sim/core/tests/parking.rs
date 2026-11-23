// SPDX-License-Identifier: GPL-3.0-or-later

//! PARK-01 real edit, towing, lot and SQLite integration regressions.

use super::*;
use crate::nodes::sim::benchmark::road_edit::RoadEditMetrics;
use crate::nodes::sim::core::RoadCommitRequest;
use std::time::Instant;

mod catchments;
mod crossing_stills;
mod docks;
mod drop_off;
mod family;
mod drive_lines;
mod gate_turn;
mod kerb;
mod leaving_line;
mod poses;
mod published;
mod strand;
mod taken_trips;

pub(in crate::nodes::sim::core::tests) fn core() -> SimCore {
    let mut core = SimCore::new(
        WorldConfig::new(512.0, 512.0, 16.0, 10.0)
            .with_chunking(128.0, 0.0)
            .with_terrain_resolution(2.0),
    );
    core.treasury.balance = 1_000_000.0;
    commit(&mut core, (-120.0, 0.0), (120.0, 0.0));
    core
}
fn commit(core: &mut SimCore, a: (f32, f32), b: (f32, f32)) {
    commit_with(core, a, b, crate::simulation::network::graph::RoadParking::NONE);
}
fn commit_with(
    core: &mut SimCore,
    a: (f32, f32),
    b: (f32, f32),
    parking: crate::simulation::network::graph::RoadParking,
) {
    commit_before_its_frame(core, a, b, parking);
    core.publish_deferred_parking_frontages();
}

// The commit alone: the frame after it publishes its parking.
fn commit_before_its_frame(
    core: &mut SimCore,
    a: (f32, f32),
    b: (f32, f32),
    parking: crate::simulation::network::graph::RoadParking,
) {
    let points = vec![Vector3::new(a.0, 0.0, a.1), Vector3::new(b.0, 0.0, b.1)];
    commit_points_before_its_frame(core, points, parking);
}

// [`commit_before_its_frame`] of a road through `points`.
fn commit_points_before_its_frame(
    core: &mut SimCore,
    points: Vec<Vector3>,
    parking: crate::simulation::network::graph::RoadParking,
) {
    core.precompute_road_mesh_data();
    core.allocator
        .prepare_building_site_query_index(core.config.zone_cell_m);
    let outcome = core.commit_road(
        RoadCommitRequest {
            points,
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
            preview: None,
            profile: crate::simulation::network::road_edit::RoadProfile::FollowGround,
            median: None,
            parking,
            attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
        },
        Instant::now(),
        RoadEditMetrics::default(),
    );
    assert!(outcome.committed, "{}", outcome.rejection);
    core.adopt_pending_cell_lots();
    core.transit_network.settle_cch();
}
// Bays in the bundled small lot.
const LOT_BAYS: usize = 12;

pub(in crate::nodes::sim::core::tests) fn register_lot(core: &mut SimCore) -> String {
    register_lot_toml(
        core,
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../godot/bootstrap/mods/parking/assets/building.parking.small_lot/asset.toml"
        )),
    )
}

fn register_lot_toml(core: &mut SimCore, toml: &str) -> String {
    let mut manifest = AssetManifest::from_str(toml).unwrap();
    // The bounds the game imports from the lot's glb files.
    for part in &mut manifest.mesh_parts {
        part.imported_bounds = Some(match part.name.as_str() {
            "kerbs" => [[-8.65, 0.0, -10.15], [8.65, 0.12, 10.0]],
            "p_sign" => [[-0.3, 0.0, -0.03], [0.3, 2.6, 0.07]],
            _ => [[-0.12, 0.0, -0.1], [0.12, 1.36, 0.1]],
        });
    }
    let id = manifest.qualified_id("parking");
    core.allocator
        .registry
        .register("parking", manifest, String::new());
    id
}

#[test]
fn parking_lot_frontage_split_and_undo_reconcile_live_cars() {
    use crate::simulation::parking::PlaceSource;
    let mut c = core();
    let id = register_lot(&mut c);
    c.place_service_building_internal(&id, -60.0, -30.0)
        .unwrap();
    c.publish_pending_building_site_changes();
    c.parking.fill_test_fleet();
    let owner = c.allocator.building_handles.handle(0).to_bits();
    let product = |c: &SimCore| {
        (2..2 + LOT_BAYS as u32)
            .flat_map(|anchor| {
                c.parking.source_bays(PlaceSource::Building {
                    building: owner,
                    anchor,
                })
            })
            .collect::<Vec<_>>()
    };
    let before = product(&c);
    assert_eq!(before.len(), LOT_BAYS);
    commit(&mut c, (30.0, 0.0), (30.0, 80.0));
    c.precompute_road_mesh_data();
    c.publish_road_tool_view();
    c.publish_pending_building_site_changes();
    assert_eq!(product(&c), before);
    assert_eq!(c.parking.counts(), (LOT_BAYS, LOT_BAYS, 0));
    assert!(c.undo_action_internal());
    c.precompute_road_mesh_data();
    c.publish_road_tool_view();
    c.publish_pending_building_site_changes();
    assert_eq!(product(&c), before);
    assert_eq!(c.parking.counts(), (LOT_BAYS, LOT_BAYS, 0));
}

#[test]
fn parking_lot_is_public_neutral_and_round_trips_with_cars() {
    let mut c = core();
    let id = register_lot(&mut c);
    let before = c.parking.counts().0;
    let result = c.place_service_building_internal(&id, 0.0, -30.0);
    assert!(result.is_ok(), "{result:?}");
    c.publish_pending_building_site_changes();
    assert!(
        c.parking.diagnostics.is_empty(),
        "{:?}",
        c.parking.diagnostics
    );
    assert!(c.parking.counts().0 >= before + 3);
    assert_eq!(c.allocator.household_capacity(0), 0);
    assert_eq!(c.allocator.registry.worker_capacity(&id), 0);
    c.parking.fill_test_fleet();
    let count = c.parking.counts().1;
    let path = temp_save_path("parking");
    c.save_game_settled_at(path.to_str().unwrap(), None, 0)
        .unwrap();
    c.load_game_internal(path.to_str().unwrap()).unwrap();
    assert_eq!(c.parking.counts().1, count);
    assert!(c.bulldoze_building(0));
    c.publish_pending_building_site_changes();
    assert_eq!(c.parking.counts(), (0, count, count));
    assert!(c.undo_action_internal());
    c.publish_pending_building_site_changes();
    assert_eq!(c.parking.counts(), (LOT_BAYS, count, count));
    std::fs::remove_file(path).unwrap();
}

// A lot fronting a walkway has no way in for its cars: placement refuses it with the reason,
// preview and commit alike. A placed lot whose street loses its cars publishes no bay and says why.
#[test]
fn parking_lot_needs_a_frontage_cars_can_use() {
    use crate::simulation::buildings::allocator::ExplicitServicePlacementRejection;
    use crate::simulation::network::types::TransitFlags;
    let mut c = core();
    core_commit_walkway(&mut c, (-120.0, 80.0), (120.0, 80.0));
    let id = register_lot(&mut c);
    let preview = c
        .get_service_building_placement_preview_internal(&id, 0.0, 110.0)
        .expect("a resolved pose");
    assert_eq!(
        preview.preview.rejection,
        Some(ExplicitServicePlacementRejection::NoCarAccess)
    );
    let count = c.allocator.buildings.len();
    assert_eq!(
        c.place_service_building_internal(&id, 0.0, 110.0),
        Err(ExplicitServicePlacementRejection::NoCarAccess)
    );
    assert_eq!(c.allocator.buildings.len(), count, "nothing is built");

    let index = c.place_service_building_internal(&id, 0.0, -30.0).unwrap();
    c.publish_pending_building_site_changes();
    let edge = c.allocator.buildings[index].edge_idx;
    c.region_graph.edge_mut(edge).allowed_types &= !TransitFlags::CAR;
    let (bays, issues) = crate::simulation::parking::lots::lot_bays(
        &c.allocator,
        index,
        &c.region_graph,
        &c.transit_network.roadbed,
    );
    assert!(bays.is_empty(), "{bays:?}");
    assert!(issues[0].ends_with("not a walkway"), "{issues:?}");
}

fn core_commit_walkway(core: &mut SimCore, a: (f32, f32), b: (f32, f32)) {
    core.precompute_road_mesh_data();
    let outcome = core.commit_road(
        RoadCommitRequest {
            points: vec![Vector3::new(a.0, 0.0, a.1), Vector3::new(b.0, 0.0, b.1)],
            fwd_lanes: 0,
            bkw_lanes: 0,
            snap_to_existing_roads: true,
            preview: None,
            profile: crate::simulation::network::road_edit::RoadProfile::FollowGround,
            median: None,
            parking: crate::simulation::network::graph::RoadParking::NONE,
            attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
        },
        Instant::now(),
        RoadEditMetrics::default(),
    );
    assert!(outcome.committed, "{}", outcome.rejection);
    core.adopt_pending_cell_lots();
    core.publish_deferred_parking_frontages();
}

#[test]
fn parking_invalid_anchor_sweep_reports_diagnostic_and_tows_on_reload() {
    let mut c = core();
    let id = register_lot(&mut c);
    c.place_service_building_internal(&id, 0.0, -30.0).unwrap();
    c.publish_pending_building_site_changes();
    c.parking.fill_test_fleet();
    let cars = c.parking.counts().1;
    let mut manifest = c.allocator.registry.get(&id).unwrap().manifest.clone();
    for anchor in &mut manifest.anchors {
        if anchor.anchor_type == AnchorType::Parking {
            anchor.position = [0.0, 0.0, -8.0];
        }
    }
    c.replace_asset_registry(vec![("parking".to_owned(), manifest, String::new())]);
    assert!(!c.parking.diagnostics.is_empty());
    assert_eq!(c.parking.counts().1, cars);
    let json = c.parking.save_json(|s| s).unwrap();
    crate::simulation::parking::ParkingSystem::load_json(&json).unwrap();
}

// `PARK-17`: the lot validation diagnostics are derived and unsaved, and a load rebuilt none, so
// the inspector and the parking panel showed nothing for a rejected lot until an edit revisited
// it. A lot with one anchor its sweep rejects stands beside a whole lot; after a save and a load
// both views list the unbroken game's reasons, validated on demand.
#[test]
fn a_loaded_game_shows_the_rejected_lot_reasons_of_the_unbroken_one() {
    let register = |core: &mut SimCore| {
        let whole = register_lot(core);
        let mut manifest = core.allocator.registry.get(&whole).unwrap().manifest.clone();
        manifest.asset_id = "building.parking.test_rejecting_lot".into();
        let anchor = manifest
            .anchors
            .iter_mut()
            .find(|a| a.anchor_type == AnchorType::Parking)
            .unwrap();
        anchor.position = [0.0, 0.0, -8.0];
        let rejecting = manifest.qualified_id("parking");
        core.allocator
            .registry
            .register("parking", manifest, String::new());
        (whole, rejecting)
    };
    let mut c = core();
    let (whole, rejecting) = register(&mut c);
    let whole = c.place_service_building_internal(&whole, -60.0, -30.0).unwrap();
    let rejecting = c
        .place_service_building_internal(&rejecting, 60.0, -30.0)
        .unwrap();
    c.publish_pending_building_site_changes();
    let views = |c: &SimCore| {
        let lines = c.parking_rejection_lines();
        let reasons = |index| c.parking_rejections(index).into_owned();
        (reasons(whole), reasons(rejecting), lines)
    };
    let live = views(&c);
    assert!(live.0.is_empty(), "{live:?}");
    assert!(!live.1.is_empty() && live.2.len() == live.1.len(), "{live:?}");
    let handle = c.allocator.building_handles.handle(rejecting).to_bits();
    let bays = c.parking.building_place_count(handle);
    assert!(bays > 0 && bays < LOT_BAYS, "{bays} of the rejecting lot's bays published");
    let path = temp_save_path("parking-lot-diagnostics-load");
    c.save_game_settled_at(path.to_str().unwrap(), None, 0)
        .unwrap();
    let mut l = core();
    register(&mut l);
    l.load_game_internal(path.to_str().unwrap()).unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(l.parking.diagnostics.is_empty(), "the load keeps no reasons");
    assert_eq!(views(&l), live);
}

#[test]
fn parking_road_demolition_also_retires_its_lot_driveway_supply() {
    let mut c = core();
    let id = register_lot(&mut c);
    c.place_service_building_internal(&id, 0.0, -30.0).unwrap();
    c.publish_pending_building_site_changes();
    c.parking.fill_test_fleet();
    let cars = c.parking.counts().1;
    assert!(c.bulldoze_road_edge_for_test(0));
    assert_eq!(c.parking.counts(), (0, cars, cars));
    let path = temp_save_path("parking_road_removed");
    c.save_game_settled_at(path.to_str().unwrap(), None, 0)
        .unwrap();
    c.load_game_internal(path.to_str().unwrap()).unwrap();
    assert_eq!(c.parking.counts(), (0, cars, cars));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn parking_resident_save_round_trip_and_handoff_at_every_leg() {
    use crate::simulation::parking::ParkingTripPhase;
    let mut c = SimCore::new(
        WorldConfig::new(2048.0, 1024.0, 16.0, 10.0)
            .with_chunking(128.0, 0.0)
            .with_terrain_resolution(2.0),
    );
    c.treasury.balance = 1_000_000.0;
    commit(&mut c, (-600.0, 0.0), (600.0, 0.0));
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
    // Travel along the lot's legal driving side; loop/end-detach regressions belong to TRAFFIC-13/15.
    let eastbound = c.allocator.entrances[0].side == -1;
    let (home, target) = if eastbound { (0, 1) } else { (1, 0) };
    let agent = c.debug_add_parking_commuter(home, target).unwrap();
    assert!(
        c.parking
            .vehicle_place(c.agents.parking_trip[agent].vehicle)
            .is_some(),
        "new resident car must be parked"
    );
    assert!(
        c.parking
            .first_free_candidate(c.allocator.building_handles.handle(target).to_bits())
            .is_some(),
        "destination catchment must be published on placement"
    );
    c.agents.release_departure(agent);
    let mut seen = [false; 5];
    let path = temp_save_path("parking-resident-mid-trip");
    let mut arrived = false;
    for _ in 0..12000 {
        crate::thread_pools::install(|| {
            c.agents.tick_with_parking(
                &c.allocator,
                &mut c.transit_network,
                &mut c.region_graph,
                0.1,
                &c.time,
                &mut c.parking,
            )
        });
        c.schedule_parking_retries();
        c.schedule_stored_parking_arrivals();
        let phase = c.agents.parking_trip[agent].phase;
        if phase == ParkingTripPhase::WalkToCar
            && c.agents.transit[agent] == crate::simulation::economy::agents::TRANSIT_ACCESS_INGRESS
        {
            let bay = c
                .parking
                .place_entrance(
                    c.agents.parking_trip[agent].source,
                    &c.allocator,
                    &c.transit_network,
                    &c.region_graph,
                )
                .unwrap();
            let target =
                crate::nodes::sim::core::snapshot::access_phase_target(&c, agent, false).unwrap();
            assert_eq!(
                (target.x, target.z),
                (bay.door_pos.x, bay.door_pos.y),
                "walking pose must aim at the actual bay"
            );
        }
        let slot = match phase {
            ParkingTripPhase::WalkToCar => Some(0),
            ParkingTripPhase::PullOut => Some(1),
            ParkingTripPhase::Drive => Some(2),
            ParkingTripPhase::PullIn => Some(3),
            ParkingTripPhase::WalkToDoor => Some(4),
            _ => None,
        };
        if let Some(slot) = slot
            && !seen[slot]
        {
            seen[slot] = true;
            let before = serde_json::to_value(c.agents.parking_trip[agent]).unwrap();
            let position = (
                c.agents.pos_x[agent].to_bits(),
                c.agents.pos_y[agent].to_bits(),
            );
            c.save_game_settled_at(path.to_str().unwrap(), None, 0)
                .unwrap();
            c.load_game_internal(path.to_str().unwrap()).unwrap();
            assert!(
                c.agents.parking_retries.capacity() >= c.agents.len(),
                "load must prepare retry storage before any movement"
            );
            assert_eq!(
                serde_json::to_value(c.agents.parking_trip[agent]).unwrap(),
                before
            );
            assert_eq!(
                (
                    c.agents.pos_x[agent].to_bits(),
                    c.agents.pos_y[agent].to_bits()
                ),
                position
            );
            let snapshot = c.build_snapshot();
            c.build_parked_chunks();
            let moving = snapshot
                .car_transforms
                .values()
                .map(|v| v.len() / 12)
                .sum::<usize>();
            let parked = c
                .parking
                .publications(true)
                .iter()
                .map(|(_, chunk)| chunk.cars())
                .sum::<usize>();
            assert_eq!(
                moving + parked,
                1,
                "handoff drew a car twice or omitted it at {phase:?}"
            );
        }
        if c.agents.current_building[agent] == target
            && c.agents.transit[agent] == crate::simulation::economy::agents::TRANSIT_IN_BUILDING
        {
            arrived = true;
            break;
        }
    }
    assert_eq!(seen, [true; 5]);
    assert!(arrived);
    std::fs::remove_file(path).unwrap();
}

// What Godot does with a snapshot: every chunk it has not installed replaces its old instances,
// and the moving cars are the snapshot's. Reading one snapshot for both is the whole guarantee:
// a car entering or leaving a bay is drawn exactly once, on every frame Godot reads, also when
// it reads only some of the sim's snapshots.
#[test]
fn a_car_entering_and_leaving_a_bay_is_drawn_once_in_every_snapshot() {
    use crate::simulation::parking::ParkingTripPhase;
    for stride in [1, 3] {
        let mut c = SimCore::new(
            WorldConfig::new(2048.0, 1024.0, 16.0, 10.0)
                .with_chunking(128.0, 0.0)
                .with_terrain_resolution(2.0),
        );
        c.treasury.balance = 1_000_000.0;
        commit(&mut c, (-600.0, 0.0), (600.0, 0.0));
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
        let eastbound = c.allocator.entrances[0].side == -1;
        let (home, target) = if eastbound { (0, 1) } else { (1, 0) };
        let agent = c.debug_add_parking_commuter(home, target).unwrap();
        c.agents.release_departure(agent);
        let mut installed = std::collections::BTreeMap::new();
        let (mut left_bay, mut entered_bay) = (false, false);
        for frame in 0..20_000 {
            c.time.speed_multiplier = 1.0;
            c.step_frame(1.0 / 60.0);
            let snapshot = c.build_snapshot();
            let phase = c.agents.parking_trip[agent].phase;
            left_bay |= phase == ParkingTripPhase::Drive;
            let parked_again = left_bay && phase == ParkingTripPhase::WalkToDoor;
            entered_bay |= parked_again;
            if frame % stride == 0 {
                for update in snapshot.parked.unseen() {
                    installed.insert(update.key, update.chunk.clone());
                }
                snapshot.parked.acknowledge(snapshot.parked.serial);
                let moving = snapshot
                    .car_render_ids
                    .values()
                    .map(Vec::len)
                    .sum::<usize>();
                let parked = installed
                    .values()
                    .map(|chunk| chunk.cars())
                    .sum::<usize>();
                assert_eq!(
                    moving + parked,
                    1,
                    "stride {stride} frame {frame} {phase:?}: {moving} moving, {parked} parked"
                );
            }
            if entered_bay
                && c.agents.current_building[agent] == target
                && c.agents.transit[agent]
                    == crate::simulation::economy::agents::TRANSIT_IN_BUILDING
            {
                break;
            }
        }
        assert!(left_bay && entered_bay, "stride {stride}");
    }
}

#[test]
fn parking_home_demolition_cannot_start_a_door_to_door_car_leg() {
    let mut c = SimCore::new(
        WorldConfig::new(2048.0, 1024.0, 16.0, 10.0)
            .with_chunking(128.0, 0.0)
            .with_terrain_resolution(2.0),
    );
    c.treasury.balance = 1_000_000.0;
    commit(&mut c, (-600.0, 0.0), (600.0, 0.0));
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
    c.agents.has_car[agent] = true;
    assert!(!c.agents.parking_trip[agent].vehicle.is_none());
    assert!(c.bulldoze_building(home));
    assert_eq!(
        c.agents.transit_mode[agent],
        crate::simulation::economy::agents::MODE_WALK
    );
}

// The economy reads `has_car` between ticks, so a custody change shows there from the next tick
// on, not from the commit that made it.
#[test]
fn has_car_follows_custody_at_the_next_tick() {
    let mut c = SimCore::new(
        WorldConfig::new(2048.0, 1024.0, 16.0, 10.0)
            .with_chunking(128.0, 0.0)
            .with_terrain_resolution(2.0),
    );
    c.treasury.balance = 10_000_000.0;
    commit_with(
        &mut c,
        (-600.0, 0.0),
        (600.0, 0.0),
        crate::simulation::network::graph::RoadParking { left: true, right: true },
    );
    let id = register_lot(&mut c);
    let mut manifest = c.allocator.registry.get(&id).unwrap().manifest.clone();
    manifest.building.as_mut().unwrap().household_capacity = Some(4);
    c.allocator.registry.register("parking", manifest, String::new());
    c.place_service_building_internal(&id, -450.0, -30.0).unwrap();
    c.place_service_building_internal(&id, 450.0, -30.0).unwrap();
    c.publish_pending_building_site_changes();
    let (home, target) = if c.allocator.entrances[0].side == -1 { (0, 1) } else { (1, 0) };
    let agent = c.debug_add_parking_commuter(home, target).unwrap();
    c.time.speed_multiplier = 8.0;
    c.step_frame(1.0 / 60.0);
    assert!(c.agents.has_car[agent], "the seeded car is available");
    c.agents.release_departure(agent);
    while c.agents.parking_trip[agent].phase == crate::simulation::parking::ParkingTripPhase::None {
        c.step_frame(1.0 / 60.0);
    }
    assert!(c.agents.has_car[agent], "the reservation shows from the next tick");
    c.step_frame(1.0 / 60.0);
    assert!(!c.agents.has_car[agent]);
}

// A member arriving with the household finds the car its carrier brought: no custody changed, so
// the publication that names it the household's car is what reports it.
#[test]
fn a_household_moving_in_with_its_car_gets_it_at_publication() {
    use crate::simulation::economy::agents::HouseholdAgeComposition;
    let mut c = SimCore::new(
        WorldConfig::new(2048.0, 1024.0, 16.0, 10.0)
            .with_chunking(128.0, 0.0)
            .with_terrain_resolution(2.0),
    );
    c.treasury.balance = 10_000_000.0;
    commit_with(
        &mut c,
        (-600.0, 0.0),
        (600.0, 0.0),
        crate::simulation::network::graph::RoadParking { left: true, right: true },
    );
    let id = register_lot(&mut c);
    let mut manifest = c.allocator.registry.get(&id).unwrap().manifest.clone();
    manifest.building.as_mut().unwrap().household_capacity = Some(4);
    c.allocator.registry.register("parking", manifest, String::new());
    c.place_service_building_internal(&id, 0.0, -30.0).unwrap();
    c.publish_pending_building_site_changes();
    let border = if c.allocator.entrances[0].side == -1 {
        c.region_graph.edge(0).start_node
    } else {
        c.region_graph.edge(0).end_node
    };
    let pos = c.region_graph.node(border).pos;
    let carrier = c.agents.spawn_household_arrival_carrier(
        0,
        2,
        HouseholdAgeComposition { adult_count: 2, ..Default::default() },
        border,
        pos.x,
        pos.z,
    );
    let handle = c.agents.agent_handle(carrier);
    assert!(c.agents.start_parking_move_in(
        carrier,
        &mut c.parking,
        &c.allocator,
        &c.transit_network,
        &c.region_graph
    ));
    let car = c.agents.parking_trip[carrier].vehicle;
    assert!(!car.is_none());
    c.time.speed_multiplier = 8.0;
    let indoors = |c: &SimCore| {
        let i = c.agents.resolve_agent(handle).unwrap();
        c.agents.transit[i] == crate::simulation::economy::agents::TRANSIT_IN_BUILDING
            && c.agents.parking_trip[i].phase == crate::simulation::parking::ParkingTripPhase::None
    };
    for _ in 0..40_000 {
        c.step_frame(1.0 / 60.0);
        if indoors(&c) {
            break;
        }
    }
    assert!(indoors(&c));
    // Past the frames of the arrival's own custody changes.
    c.step_frame(1.0 / 60.0);
    c.step_frame(1.0 / 60.0);
    // The household formed as the car parked, the second adult stepping out beside the driver
    // (`PARK-35`): the arrival check has nothing left to make.
    let i = c.agents.resolve_agent(handle).unwrap();
    assert_ne!(c.agents.household_id[i], usize::MAX);
    assert!(!c.households.materialize_arrival_carrier(i, &mut c.agents, &c.allocator));
    c.step_frame(1.0 / 60.0);
    let adults: Vec<_> = (0..c.agents.len())
        .filter(|&i| c.agents.parking_trip[i].vehicle == car)
        .collect();
    assert_eq!(adults.len(), 2, "both adults hold the household's car");
    assert!(c.parking.vehicle_place(car).is_some());
    assert!(adults.iter().all(|&i| c.agents.has_car[i]));
}

// Undo puts agent rows back without events; `has_car` must not stay as the row kept it.
#[test]
fn undoing_a_bulldoze_rewrites_has_car_from_the_cars_as_they_stand() {
    let mut c = SimCore::new(
        WorldConfig::new(2048.0, 1024.0, 16.0, 10.0)
            .with_chunking(128.0, 0.0)
            .with_terrain_resolution(2.0),
    );
    c.treasury.balance = 10_000_000.0;
    commit_with(
        &mut c,
        (-600.0, 0.0),
        (600.0, 0.0),
        crate::simulation::network::graph::RoadParking { left: true, right: true },
    );
    let id = register_lot(&mut c);
    let mut manifest = c.allocator.registry.get(&id).unwrap().manifest.clone();
    manifest.building.as_mut().unwrap().household_capacity = Some(4);
    c.allocator.registry.register("parking", manifest, String::new());
    c.place_service_building_internal(&id, -450.0, -30.0).unwrap();
    c.place_service_building_internal(&id, 450.0, -30.0).unwrap();
    c.publish_pending_building_site_changes();
    let (home, target) = if c.allocator.entrances[0].side == -1 { (0, 1) } else { (1, 0) };
    let agent = c.debug_add_parking_commuter(home, target).unwrap();
    c.time.speed_multiplier = 8.0;
    c.step_frame(1.0 / 60.0);
    assert!(c.agents.has_car[agent]);
    assert!(c.bulldoze_building(home));
    for _ in 0..3 {
        c.step_frame(1.0 / 60.0);
    }
    assert!(c.undo_action_internal());
    for _ in 0..3 {
        c.step_frame(1.0 / 60.0);
    }
}

#[test]
fn parking_move_in_save_round_trip_mid_arrival() {
    use crate::simulation::economy::agents::HouseholdAgeComposition;
    use crate::simulation::parking::ParkingTripPhase;
    let mut c = SimCore::new(
        WorldConfig::new(2048.0, 1024.0, 16.0, 10.0)
            .with_chunking(128.0, 0.0)
            .with_terrain_resolution(2.0),
    );
    c.treasury.balance = 1_000_000.0;
    commit(&mut c, (-600.0, 0.0), (600.0, 0.0));
    let id = register_lot(&mut c);
    c.place_service_building_internal(&id, 0.0, -30.0).unwrap();
    c.publish_pending_building_site_changes();
    let border = if c.allocator.entrances[0].side == -1 {
        c.region_graph.edge(0).start_node
    } else {
        c.region_graph.edge(0).end_node
    };
    let pos = c.region_graph.node(border).pos;
    let i = c.agents.spawn_household_arrival_carrier(
        0,
        1,
        HouseholdAgeComposition {
            adult_count: 1,
            ..Default::default()
        },
        border,
        pos.x,
        pos.z,
    );
    assert!(c.agents.start_parking_move_in(
        i,
        &mut c.parking,
        &c.allocator,
        &c.transit_network,
        &c.region_graph
    ));
    c.households.recount_pending_arrivals(&mut c.agents);
    assert_eq!(c.agents.parking_trip[i].phase, ParkingTripPhase::Drive);
    let car = c.agents.parking_trip[i].vehicle;
    let before = c.parking.save_json(|s| s).unwrap();
    let trip = serde_json::to_string(&c.agents.parking_trip[i]).unwrap();
    let path = temp_save_path("parking-move-in-mid-arrival");
    c.save_game_settled_at(path.to_str().unwrap(), None, 0)
        .unwrap();
    c.load_game_internal(path.to_str().unwrap()).unwrap();
    assert_eq!(c.parking.save_json(|s| s).unwrap(), before);
    assert_eq!(
        serde_json::to_string(&c.agents.parking_trip[i]).unwrap(),
        trip
    );
    assert!(
        c.parking.vehicle_place(car).is_none(),
        "loading cannot park an arriving car"
    );
    std::fs::remove_file(path).unwrap();
}

#[test]
fn parking_new_lot_fills_by_driven_arrivals_and_saves_mid_delivery() {
    use crate::assets::AnchorType;
    use crate::simulation::economy::agents::TRANSIT_IN_BUILDING;
    let mut c = core();
    let lot = register_lot(&mut c);
    let mut home = c.allocator.registry.get(&lot).unwrap().manifest.clone();
    home.asset_id = "building.parking.home_without_bays".to_owned();
    home.anchors
        .retain(|a| a.anchor_type != AnchorType::Parking);
    home.building.as_mut().unwrap().household_capacity = Some(4);
    let home_id = home.qualified_id("parking");
    c.allocator
        .registry
        .register("parking", home, String::new());
    c.place_service_building_internal(&home_id, -60.0, -30.0)
        .unwrap();
    c.publish_pending_building_site_changes();
    let resident = c.debug_add_parking_commuter(0, 0).unwrap();
    let car = c.agents.parking_trip[resident].vehicle;
    assert_eq!(c.parking.counts(), (0, 1, 1));
    let border = if c.allocator.entrances[0].side == -1 {
        c.region_graph.edge(0).start_node
    } else {
        c.region_graph.edge(0).end_node
    };
    c.set_border_connection_internal(border as i32);
    c.transit_network.settle_cch();
    c.place_service_building_internal(&lot, 60.0, -30.0)
        .unwrap();
    c.publish_pending_building_site_changes();
    assert!(
        c.parking.vehicle_place(car).is_none(),
        "placing supply cannot park a stored car"
    );
    c.logistics
        .prepare_parking_borders(&c.transit_network, &c.region_graph);
    // This is the scheduled event handler, not a direct occupancy mutation.
    c.run_stored_parking_arrivals((0, -1), c.time.absolute_slot());
    c.run_stored_parking_arrivals((-1, -1), c.time.absolute_slot());
    assert_eq!(
        c.agents.len(),
        2,
        "one delivery carrier must enter at the border"
    );
    assert!(c.parking.vehicle_place(car).is_none());
    assert!(!c.parking.available_to(car, c.agents.agent_handle(resident)));
    let path = temp_save_path("parking-stored-car-mid-delivery");
    c.save_game_settled_at(path.to_str().unwrap(), None, 0)
        .unwrap();
    c.load_game_internal(path.to_str().unwrap()).unwrap();
    assert!(
        c.parking.vehicle_place(car).is_none(),
        "loading cannot complete a driven arrival"
    );
    for _ in 0..6000 {
        c.agents.tick_with_parking(
            &c.allocator,
            &mut c.transit_network,
            &mut c.region_graph,
            0.1,
            &c.time,
            &mut c.parking,
        );
        for i in 0..c.agents.len() {
            if c.agents.parking_trip[i].delivery_only && c.agents.transit[i] == TRANSIT_IN_BUILDING
            {
                assert_eq!(
                    c.agents.current_building[i],
                    usize::MAX,
                    "delivery driver entered a building"
                );
            }
        }
        c.agents.remove_completed_parking_deliveries();
        if c.parking.vehicle_place(car).is_some() {
            break;
        }
    }
    assert!(c.parking.vehicle_place(car).is_some());
    assert_eq!(c.agents.len(), 1, "delivery carrier must retire at the bay");
    assert!(c.parking.available_to(car, c.agents.agent_handle(resident)));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn parking_place_access_follows_a_full_lane_renumbering() {
    use crate::simulation::core::handles::Handle;
    use crate::simulation::parking::PlaceSource;
    let mut c = core();
    let id = register_lot(&mut c);
    c.place_service_building_internal(&id, -90.0, -30.0)
        .unwrap();
    c.publish_pending_building_site_changes();
    let owner = c.allocator.building_handles.handle(0).to_bits();
    let places: Vec<_> = (2..8)
        .flat_map(|anchor| {
            c.parking.source_bays(PlaceSource::Building {
                building: owner,
                anchor,
            })
        })
        .map(|(bits, _)| Handle::from_bits(bits))
        .collect();
    assert_eq!(places.len(), 6);
    // A road commit appends lane ids; the terrain sync then rebuilds every lane and renumbers
    // them compactly, as a terrain edit does. Agents carry their lanes over by identity; the
    // lot's access is not republished, so its cached lane ids point past the rebuilt lanes.
    commit(&mut c, (100.0, 0.0), (100.0, 80.0));
    c.transit_network
        .sync_to_terrain(&mut c.region_graph, &c.heightmap);
    let lanes = &c.transit_network.lane_system;
    for place in places {
        let entrance = c
            .parking
            .place_entrance(place, &c.allocator, &c.transit_network, &c.region_graph)
            .expect("lot access survives a far edit");
        assert!(c.region_graph.get_edge(entrance.edge_idx).is_some());
        for lane in [
            entrance.foot_lane_fwd,
            entrance.foot_lane_bkw,
            entrance.car_lane_fwd,
            entrance.car_lane_bkw,
        ] {
            if lane != usize::MAX {
                assert_eq!(
                    lanes.lanes.get(lane).map(|l| l.edge_id),
                    Some(entrance.edge_idx),
                    "parking access lane must lie on its access edge after renumbering"
                );
            }
        }
        assert!(
            entrance.car_lane_fwd != usize::MAX || entrance.car_lane_bkw != usize::MAX,
            "lot access keeps a car lane"
        );
    }
}

// A lot on either side of a one-way road is reached by its one lane (`PARK-32`): the lane serves
// both kerbs, so nothing is crossed. Before, every lot was same-side only and the lot left of the
// lane had no car access at all.
#[test]
fn a_lot_on_either_side_of_a_one_way_road_has_car_access() {
    use crate::simulation::core::handles::Handle;
    use crate::simulation::parking::PlaceSource;
    let mut c = SimCore::new(
        WorldConfig::new(512.0, 512.0, 16.0, 10.0)
            .with_chunking(128.0, 0.0)
            .with_terrain_resolution(2.0),
    );
    c.treasury.balance = 1_000_000.0;
    c.precompute_road_mesh_data();
    c.allocator
        .prepare_building_site_query_index(c.config.zone_cell_m);
    let outcome = c.commit_road(
        RoadCommitRequest {
            points: vec![Vector3::new(-120.0, 0.0, 0.0), Vector3::new(120.0, 0.0, 0.0)],
            fwd_lanes: 1,
            bkw_lanes: 0,
            snap_to_existing_roads: true,
            preview: None,
            profile: crate::simulation::network::road_edit::RoadProfile::FollowGround,
            median: None,
            parking: crate::simulation::network::graph::RoadParking::NONE,
            attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
        },
        Instant::now(),
        RoadEditMetrics::default(),
    );
    assert!(outcome.committed, "{}", outcome.rejection);
    c.adopt_pending_cell_lots();
    c.transit_network.settle_cch();
    c.publish_deferred_parking_frontages();
    let id = register_lot(&mut c);
    let north = c.place_service_building_internal(&id, 0.0, -30.0).unwrap();
    let south = c.place_service_building_internal(&id, 0.0, 30.0).unwrap();
    c.publish_pending_building_site_changes();
    assert_ne!(
        c.allocator.entrances[north].side, c.allocator.entrances[south].side,
        "one lot each side"
    );
    for building in [north, south] {
        let owner = c.allocator.building_handles.handle(building).to_bits();
        let places: Vec<_> = (2..8)
            .flat_map(|anchor| c.parking.source_bays(PlaceSource::Building { building: owner, anchor }))
            .map(|(bits, _)| Handle::from_bits(bits))
            .collect();
        assert!(!places.is_empty(), "building {building} publishes its bays");
        for place in places {
            let entrance = c
                .parking
                .place_entrance(place, &c.allocator, &c.transit_network, &c.region_graph)
                .expect("an access");
            assert!(
                entrance.car_lane_fwd != usize::MAX,
                "side {}: the one lane reaches the lot",
                entrance.side
            );
        }
    }
}

#[test]
fn parking_lot_on_a_diagonal_road_accepts_every_bay() {
    // Off-axis seams between the yard and the road surface leave float slivers, and the
    // turning squares must follow the lot rather than the world axes.
    let mut c = SimCore::new(
        WorldConfig::new(512.0, 512.0, 16.0, 10.0)
            .with_chunking(128.0, 0.0)
            .with_terrain_resolution(2.0),
    );
    c.treasury.balance = 1_000_000.0;
    commit(&mut c, (-100.0, -100.0), (100.0, 100.0));
    let id = register_lot(&mut c);
    for (x, z) in [(-40.0, -10.0), (-10.0, 40.0)] {
        c.place_service_building_internal(&id, x, z).unwrap();
    }
    c.publish_pending_building_site_changes();
    let issues: Vec<_> = c.parking.diagnostics.values().flatten().collect();
    assert!(issues.is_empty(), "{issues:#?}");
    assert_eq!(c.parking.counts().0, 2 * LOT_BAYS);
}

#[test]
fn parking_medium_lot_accepts_all_twenty_bays() {
    let mut c = core();
    let id = register_lot_toml(
        &mut c,
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../godot/bootstrap/mods/parking/assets/building.parking.medium_lot/asset.toml"
        )),
    );
    c.place_service_building_internal(&id, -60.0, -30.0)
        .unwrap();
    c.publish_pending_building_site_changes();
    let issues: Vec<_> = c.parking.diagnostics.values().flatten().collect();
    assert!(issues.is_empty(), "{issues:#?}");
    assert_eq!(c.parking.counts().0, 20);
}

// `PARK-30`: a placed bundled lot draws the editor's bay paint, line for line, on its pad; a
// load rebuilds it, and a demolition clears its chunk.
#[test]
fn a_placed_lot_paints_its_bays_as_the_editor_draws_them() {
    use crate::simulation::parking::paint::STRIDE;
    let mut c = core();
    let id = register_lot(&mut c);
    c.place_service_building_internal(&id, 0.0, -30.0).unwrap();
    c.publish_pending_building_site_changes();
    c.prepare_site_queries();
    c.build_lot_paint();
    let paint = c.parking.paint_publication();
    let rows: Vec<f32> = paint
        .chunks
        .values()
        .flat_map(|chunk| chunk.rows())
        .collect();
    // Two rows of six: seven side lines and one head line each.
    assert_eq!(rows.len(), 16 * STRIDE);
    let manifest = c.allocator.registry.get(&id).unwrap().manifest.clone();
    let params = crate::simulation::parking::validation::editor_params(&manifest).unwrap();
    let bounds: Vec<f32> = manifest
        .mesh_parts
        .iter()
        .flat_map(|p| p.imported_bounds.unwrap_or_default().into_iter().flatten())
        .collect();
    let editor = crate::simulation::parking::validation::editor_lot_payload(&params, &bounds);
    assert_eq!(editor["paint_rows"].as_array().unwrap().len(), rows.len());
    // Every line lies on the lot's pad, at its support height plus the decal lift.
    let support = c.allocator.building_sites[0].support_height_m;
    for row in rows.as_chunks::<STRIDE>().0 {
        let lift = row[7] - support;
        assert!((lift - crate::config::ROAD_DECAL_RENDER_Z_BIAS_M).abs() < 1e-3, "{row:?}");
        assert!(row[4].abs() < 1e-4, "a level pad tilts no line: {row:?}");
    }
    let path = temp_save_path("parking-paint");
    c.save_game_settled_at(path.to_str().unwrap(), None, 0).unwrap();
    c.load_game_internal(path.to_str().unwrap()).unwrap();
    c.prepare_site_queries();
    c.build_lot_paint();
    let loaded = c.parking.paint_publication();
    let bits = |paint: &crate::simulation::parking::PaintPublication| {
        let rows = |chunk: &crate::simulation::parking::paint::PaintChunk| {
            chunk.rows().iter().map(|f| f.to_bits()).collect::<Vec<_>>()
        };
        paint.chunks.iter().map(|(k, v)| (*k, rows(v))).collect::<Vec<_>>()
    };
    assert_eq!(bits(&loaded), bits(&paint), "a load paints the same lines");
    std::fs::remove_file(path).unwrap();
    assert!(c.bulldoze_building(0));
    c.publish_pending_building_site_changes();
    c.prepare_site_queries();
    c.build_lot_paint();
    assert!(c.parking.paint_publication().chunks.values().all(|chunk| chunk.is_empty()));
}

// `PARK-30`: a lot's lines read the graded surface through one gather per lot, bit for bit the
// heights point queries give, on a lot cut into a slope and on a level one.
#[test]
fn a_lots_paint_reads_the_gathered_surface_as_point_queries_do() {
    use crate::simulation::parking::paint;
    use godot::prelude::Vector2;
    let mut c = SimCore::new(
        WorldConfig::new(512.0, 512.0, 16.0, 10.0)
            .with_chunking(128.0, 0.0)
            .with_terrain_resolution(2.0),
    );
    c.treasury.balance = 1_000_000.0;
    // A 1:8 rise north of the street, under one lot.
    for _ in 0..12 {
        c.slope_terrain_internal(
            Vector2::new(-60.0, 30.0),
            70.0,
            Vector2::new(-60.0, -5.0),
            0.0,
            Vector2::new(-60.0, 65.0),
            9.0,
            1.0,
        );
    }
    commit(&mut c, (-120.0, 0.0), (120.0, 0.0));
    let id = register_lot(&mut c);
    for (x, z) in [(-60.0, 30.0), (60.0, -30.0)] {
        c.place_service_building_internal(&id, x, z).unwrap();
    }
    c.publish_pending_building_site_changes();
    c.prepare_site_queries();
    let supports: Vec<f32> = c.allocator.building_sites.iter().map(|site| site.support_height_m).collect();
    assert!(supports.iter().any(|&height| height > 0.5), "a lot on the rise: {supports:?}");
    let buildings = (0..c.allocator.buildings.len())
        .map(|b| c.allocator.building_handles.handle(b).to_bits())
        .collect();
    let bits = |rows: &[f32]| rows.iter().map(|f| f.to_bits()).collect::<Vec<_>>();
    for (building, _, rows) in c.paint_buildings(buildings) {
        assert!(rows.len() >= 16 * paint::STRIDE, "two rows of six bays");
        let (lines, _) = c.parking.paint_lines(building);
        let mut point = Vec::new();
        c.with_graded_surface(|surface, scratch| {
            let height =
                |x: f32, z: f32| surface.surface_height_at(f64::from(x), f64::from(z), scratch) as f32;
            paint::paint_rows(&lines, height, &mut point);
        });
        assert_eq!(bits(&rows), bits(&point), "building {building}");
    }
}

// The rendered evidence's city (`PARK-30`): on a straight street, the bundled small and medium
// lots, a lot on a slope across the street, and the explicit copies of pack assets found in
// `METRUM_PAINT_STILL_ASSETS` (folders named by asset id, each with its `asset.toml`, registered
// in pack `kuopio`): every `building.industrial.*` placed as industry, the others as services.
// Writes `paint.sqlite` and `paint.json` (each building's asset id and centre) into
// `METRUM_PAINT_STILL_OUT`.
#[test]
#[ignore = "manual fixture for rendered bay paint stills; requires METRUM_PAINT_STILL_ASSETS and _OUT"]
fn write_the_lot_paint_still_save() {
    let var = |name| std::env::var(name).unwrap_or_else(|_| panic!("{name} is required"));
    let (assets, out) = (var("METRUM_PAINT_STILL_ASSETS"), var("METRUM_PAINT_STILL_OUT"));
    let mut core = SimCore::new(
        WorldConfig::new(512.0, 512.0, 16.0, 10.0)
            .with_chunking(128.0, 0.0)
            .with_terrain_resolution(2.0),
    );
    core.treasury.balance = 10_000_000.0;
    // A 1:8 rise north of the street, under the sloped lot.
    use godot::prelude::Vector2;
    for _ in 0..12 {
        core.slope_terrain_internal(
            Vector2::new(-160.0, 30.0), 70.0, Vector2::new(-160.0, -5.0), 0.0,
            Vector2::new(-160.0, 65.0), 9.0, 1.0,
        );
    }
    let ground = |core: &SimCore, z: f32| core.get_world_surface_height_internal(Vector2::new(-160.0, z));
    eprintln!("PAINT_STILL slope {:?}", [0.0, 10.0, 20.0, 30.0].map(|z| ground(&core, z)));
    commit(&mut core, (-220.0, 0.0), (220.0, 0.0));
    let small = register_lot(&mut core);
    let medium = register_lot_toml(
        &mut core,
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../godot/bootstrap/mods/parking/assets/building.parking.medium_lot/asset.toml"
        )),
    );
    let mut placed = vec![
        (small.clone(), core.place_service_building_internal(&small, -160.0, -30.0).unwrap()),
        (medium.clone(), core.place_service_building_internal(&medium, -120.0, -35.0).unwrap()),
        (small.clone(), core.place_service_building_internal(&small, -160.0, 30.0).unwrap()),
    ];
    let mut dirs: Vec<_> = std::fs::read_dir(&assets).unwrap().map(|e| e.unwrap().path()).collect();
    dirs.sort();
    let mut x = -80.0;
    for dir in dirs {
        let text = std::fs::read_to_string(dir.join("asset.toml")).unwrap();
        let manifest = AssetManifest::from_str(&text).unwrap();
        let id = manifest.qualified_id("kuopio");
        let cells = manifest.building.as_ref().map_or(2, |b| b.lot_width_cells) as f32;
        let industry = manifest.asset_id.starts_with("building.industrial.");
        core.allocator
            .registry
            .register("kuopio", manifest, dir.to_string_lossy().into_owned());
        x += cells * 5.0 + 6.0;
        let building = if industry {
            core.place_industry_building_internal(&id, x, -45.0)
        } else {
            core.place_service_building_internal(&id, x, -40.0)
        }
        .unwrap_or_else(|e| panic!("{id}: {e:?}"));
        x += cells * 5.0;
        placed.push((id, building));
    }
    core.publish_pending_building_site_changes();
    eprintln!("PAINT_STILL lot {:?}", [5.0, 15.0, 25.0, 30.0].map(|z| ground(&core, z)));
    let parking = &core.parking;
    let issues: Vec<_> =
        parking.diagnostics.values().chain(parking.dock_diagnostics.values()).flatten().collect();
    eprintln!("PAINT_STILL issues {issues:#?}");
    let out = std::path::Path::new(&out);
    std::fs::create_dir_all(out).unwrap();
    core.save_game_settled_at(out.join("paint.sqlite").to_str().unwrap(), None, 0)
        .expect("the still city saves");
    let buildings: Vec<_> = placed
        .iter()
        .map(|(id, b)| {
            let building = &core.allocator.buildings[*b];
            serde_json::json!({ "id": id, "x": building.center_x, "z": building.center_y,
                "support": core.allocator.building_sites[*b].support_height_m })
        })
        .collect();
    let json = serde_json::json!({ "buildings": buildings });
    std::fs::write(out.join("paint.json"), json.to_string()).unwrap();
}
