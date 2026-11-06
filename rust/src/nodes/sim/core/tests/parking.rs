// SPDX-License-Identifier: GPL-2.0-only

//! PARK-01 real edit, towing, lot and SQLite integration regressions.

use super::*;
use crate::nodes::sim::benchmark::road_edit::RoadEditMetrics;
use crate::nodes::sim::core::RoadCommitRequest;
use std::time::Instant;

mod kerb;
mod poses;

fn core() -> SimCore {
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
    core.precompute_road_mesh_data();
    core.allocator
        .prepare_building_site_query_index(core.config.zone_cell_m);
    let outcome = core.commit_road(
        RoadCommitRequest {
            points: vec![Vector3::new(a.0, 0.0, a.1), Vector3::new(b.0, 0.0, b.1)],
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
            preview: None,
            tunnel: false,
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

fn register_lot(core: &mut SimCore) -> String {
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
            "markings" => [[-8.65, 0.0, -10.15], [8.65, 0.12, 10.0]],
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
                .map(|(_, chunk)| chunk.identities.values().map(Vec::len).sum::<usize>())
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
                    .map(|chunk| chunk.identities.values().map(Vec::len).sum::<usize>())
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
