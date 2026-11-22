// SPDX-License-Identifier: GPL-3.0-or-later

//! Freight at a building without a dock (`PARK-36`): the truck stands at the kerb stop, drawn,
//! at both ends of its trip. Two copies of the yard without its lane, so no dock proves.

use super::*;
use crate::simulation::parking::ParkingTripPhase;

// A truck from a dockless yard 60 m east to a dockless yard, just dispatched: at its source's
// kerb stop, pulling out.
fn dockless_truck() -> (SimCore, usize) {
    let (mut core, building) = place_yard(false, "6");
    assert!(dock(&core, building).is_empty() && core.allocator.freight_gates.is_empty());
    let id = core.allocator.buildings[0].asset_id.clone();
    let source = core.place_service_building_internal(&id, 60.0, -40.0).unwrap();
    core.publish_pending_building_site_changes();
    let carrier = dispatch_shipment(&mut core, 0, source);
    (core, carrier)
}

// The kerb stop of building `building`.
fn kerb_stop(core: &SimCore, building: usize) -> Vector2 {
    crate::simulation::economy::agents::tick::kerb_stop_entrance(
        &core.allocator.entrances[building],
        &core.region_graph,
        crate::simulation::economy::agents::tick::FREIGHT_KERB_INSET_M,
    )
    .unwrap()
    .door_pos
}

fn pos(core: &SimCore, i: usize) -> Vector2 {
    Vector2::new(core.agents.pos_x[i], core.agents.pos_y[i])
}

// A save made while a truck pulls out of its source's kerb stop, and one made while it stands at
// its destination's, continue tick for tick as the live city does. It never enters either
// building.
#[test]
fn a_truck_at_a_kerb_stop_round_trips_a_save() {
    use crate::simulation::economy::agents::TRANSIT_IN_BUILDING;
    let (mut core, carrier) = dockless_truck();
    let source = core.agents.current_building[carrier];
    assert!(pos(&core, carrier).distance_to(kerb_stop(&core, source)) < 0.05);
    let live = save_and_continue(&mut core, "truck_leaving_kerb", 200, 0, |_| {});
    assert!(live.iter().all(|t| t.2 != TRANSIT_IN_BUILDING));

    tick_truck_until(&mut core, carrier, |core, i| phase_of(core, i) == ParkingTripPhase::AtKerb);
    assert!(pos(&core, carrier).distance_to(kerb_stop(&core, 0)) < 0.05);
    // On the building's side of the road, by its kerb.
    let curb = core.allocator.entrances[0].curb_pos;
    assert!(pos(&core, carrier).distance_to(curb) < 4.0, "{:?} {curb:?}", pos(&core, carrier));
    let live = save_and_continue(&mut core, "truck_at_kerb", 100, 0, |_| {});
    assert!(live.iter().all(|t| t == &live[0] && t.3.contains("AtKerb")), "it stands");
}

// A short delivery between two buildings without a dock, 60 m apart (`PARK-36`, the owner's "no
// truck at a small shop"): the carrier drives as a truck the whole way, on the driving lanes, to
// the kerb stop and back, and is drawn as one; before, a door-to-door plan chose to walk such a
// trip, and the carrier walked the sidewalk drawn as a pedestrian.
#[test]
fn a_short_delivery_between_buildings_without_a_dock_drives_as_a_truck() {
    use crate::simulation::economy::agents::{MODE_CAR, TRANSIT_NETWORK};
    use crate::simulation::network::lanes::LaneType;
    let (mut core, carrier) = dockless_truck();
    let on_a_car_lane = std::cell::Cell::new(false);
    tick_truck_until(&mut core, carrier, |core, i| {
        assert_eq!(core.agents.transit_mode[i], MODE_CAR, "it drives");
        if core.agents.transit[i] == TRANSIT_NETWORK
            && let Some(lane) = core.transit_network.lane_system.lanes.get(core.agents.current_lane_id[i])
        {
            on_a_car_lane.set(on_a_car_lane.get() || lane.lane_type == LaneType::Vehicle);
        }
        phase_of(core, i) == ParkingTripPhase::AtKerb
    });
    assert!(on_a_car_lane.get(), "it drove on a driving lane");
    assert!(pos(&core, carrier).distance_to(kerb_stop(&core, 0)) < 0.05);
    assert!(check_truck(&mut core));
    let source = core.agents.target_building[carrier];
    tick_truck_until(&mut core, carrier, |core, i| {
        assert_eq!(core.agents.transit_mode[i], MODE_CAR, "it drives back");
        phase_of(core, i) == ParkingTripPhase::AtKerb
    });
    assert!(pos(&core, carrier).distance_to(kerb_stop(&core, source)) < 0.05);
}

// The shipment's check that comes before the truck has stood `KERB_STAND_FLOOR_MINUTES` at the
// kerb stop is put off to the minute the floor ends, and none after it.
#[test]
fn the_check_of_a_truck_at_the_kerb_waits_for_the_floor() {
    use crate::simulation::core::scheduler::SLOTS_PER_MINUTE;
    let (mut core, carrier) = dockless_truck();
    tick_truck_until(&mut core, carrier, |core, i| phase_of(core, i) == ParkingTripPhase::AtKerb);
    let slot = core.time.absolute_slot();
    let floor = (crate::simulation::parking::KERB_STAND_FLOOR_MINUTES * SLOTS_PER_MINUTE as f64) as u64;
    let later = core.kerb_stand_floor_slot(0, slot).expect("put off");
    assert!(later > slot && later <= slot + floor, "{later} from {slot}");
    let spm = core.time.seconds_per_minute() as f32;
    core.agents.sim_time += 2.0 * spm;
    assert_eq!(core.kerb_stand_floor_slot(0, slot), None);
}

// A format 128 trip row of a truck sent on from a full yard to the door (`GateToDoor`) loads as
// one on its way to the kerb stop (`ToKerb`, `PARK-36`).
#[test]
fn a_format_128_gate_to_door_row_loads_as_bound_for_the_kerb() {
    let phase: ParkingTripPhase = serde_json::from_str("\"GateToDoor\"").unwrap();
    assert_eq!(phase, ParkingTripPhase::ToKerb);
    assert_eq!(serde_json::to_string(&ParkingTripPhase::AtKerb).unwrap(), "\"AtKerb\"");
}

// A format 128 save holding a truck inside a building without a dock, as the game before
// `PARK-36` parked one there for its wait, loads with the truck inside; its return then leaves
// from the kerb stop, never the door.
#[test]
fn a_format_128_save_with_a_truck_inside_loads_and_the_truck_leaves_from_the_kerb() {
    use crate::simulation::economy::agents::{TRANSIT_ACCESS_EGRESS, TRANSIT_IN_BUILDING};
    use crate::simulation::save::schema::PRE_KERB_FREIGHT_SAVE_VERSION;
    let (mut core, carrier) = dockless_truck();
    let a = &mut core.agents;
    a.transit[carrier] = TRANSIT_IN_BUILDING;
    a.current_building[carrier] = 0;
    a.current_lane_id[carrier] = usize::MAX;
    a.current_path[carrier].clear();
    a.parking_trip[carrier] = Default::default();
    let path = std::env::temp_dir().join(format!("metrum_kerb_freight_128_{}.sqlite", std::process::id()));
    let text = path.to_str().unwrap();
    core.save_game_settled_at(text, None, 0).unwrap();
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute("UPDATE save_meta SET version = ?1", [PRE_KERB_FREIGHT_SAVE_VERSION])
        .unwrap();
    core.load_game_internal(text).unwrap();
    let _ = std::fs::remove_file(&path);
    let i = core.agents.resolve_agent(core.logistics.shipments[0].carrier).unwrap();
    assert_eq!(core.agents.transit[i], TRANSIT_IN_BUILDING);
    assert!(check_truck(&mut core));
    assert_eq!(core.agents.transit[i], TRANSIT_ACCESS_EGRESS);
    assert!(pos(&core, i).distance_to(kerb_stop(&core, 0)) < 0.05);
}
