// SPDX-License-Identifier: GPL-3.0-or-later

//! Freight yards (`PARK-20`) on the minimal straight road: a carrier bound for a building with
//! a published dock detaches at the gate, is granted the dock in the ordered phase, drives the
//! dock's route and stands in the bay, where its shipment's check delivers; with every dock held
//! it drives from the gate to the door. The dock rows and the gate are made as the site
//! reconcile makes them (`SimCore::publish_docks`), with no gameplay constructor.

use super::support::{create_test_building, register_test_asset, setup_straight_road};
use super::*;
use crate::simulation::buildings::allocator::FreightGate;
use crate::simulation::economy::definitions::{
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::economy::logistics::{
    CarrierClass, Shipment, ShipmentEndpoint, ShipmentStatus, ShipmentSystem,
};
use crate::simulation::parking::{Bay, ParkingSystem, ParkingTrip, ParkingTripPhase};

// The source's and the yard's centres on the road's south side.
const SOURCE_X: f32 = 45.0;
const YARD_X: f32 = 300.0;
// The gate's road connection: 20 m east of the yard's door, so the gate's lane point and the
// door's differ.
const GATE_X: f32 = 320.0;
const PACKAGED_FOOD: &str = "packaged_food";

struct Yard {
    agents: AgentSystem,
    parking: ParkingSystem,
    allocator: BuildingAllocator,
    network: TransitNetwork,
    graph: RegionGraph,
    shipments: ShipmentSystem,
}

// A food processor at `SOURCE_X` and a grocery at `YARD_X` with `docks` loading bays behind
// one gate, in anchor order from west to east.
fn yard(docks: u32) -> Yard {
    let (network, graph, _, _) = setup_straight_road();
    let mut allocator = BuildingAllocator::new();
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let source = register_test_asset(&mut allocator, "yard", "source", ZoneClass::Industrial);
    let grocery = register_test_asset(&mut allocator, "yard", "grocery", ZoneClass::Commercial);
    for (x, asset, zone, profile) in [
        (SOURCE_X, source, ZoneType::Industrial, "food_processor_basic"),
        (YARD_X, grocery, ZoneType::Commercial, "grocery_basic"),
    ] {
        let mut b = create_test_building(0, -1);
        b.center_x = x;
        b.center_y = -12.0;
        b.facing_dir = Vector2::new(0.0, -1.0);
        b.asset_id = asset;
        b.zone_type = zone;
        b.economy_profile_runtime_id = catalog.profile_for_id(profile).unwrap().runtime_id;
        allocator.buildings.push(b);
    }
    allocator
        .building_handles
        .register_appended(allocator.buildings.len());
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut parking = ParkingSystem::default();
    if docks > 0 {
        publish_docks(&mut parking, &mut allocator, &network, &graph, docks);
    }
    Yard {
        agents: AgentSystem::new(),
        parking,
        allocator,
        network,
        graph,
        shipments: ShipmentSystem::new(),
    }
}

// The yard's dock rows, their access and its gate, as `SimCore::publish_docks` and
// `rebuild_freight_gate` make them: each route runs from the gate's road connection, 4.25 m in
// (the approach), across to the aisle and into the bay 6 m behind it.
fn publish_docks(
    parking: &mut ParkingSystem,
    allocator: &mut BuildingAllocator,
    network: &TransitNetwork,
    graph: &RegionGraph,
    docks: u32,
) {
    let handle = allocator.building_handles.handle(1).to_bits();
    let entrance = allocator.entrances[1].clone();
    let kerb = entrance.curb_pos.y;
    let rows: Vec<_> = (0..docks)
        .map(|anchor| {
            let x = GATE_X + 4.0 * anchor as f32;
            let stop = kerb - 20.0;
            let bay = Bay {
                identity: [handle, u64::from(anchor)],
                position: [x, 0.0, stop],
                path: [
                    [GATE_X, kerb],
                    [GATE_X, kerb - 4.25],
                    [x, stop + 6.0],
                    [x, stop],
                ],
                heading: 0.0,
                width: 3.5,
                length: 8.0,
            };
            (anchor, bay)
        })
        .collect();
    parking.replace_docks(handle, &rows);
    parking.refresh_dock_access(handle, allocator, network, graph);
    let b = &allocator.buildings[1];
    let gate = BuildingAllocator::parking_access_at(
        entrance.door_pos,
        Vector2::new(GATE_X, kerb),
        b.edge_idx,
        b.side,
        graph,
        &network.lane_system,
    );
    allocator.freight_gates.insert(
        handle,
        FreightGate {
            arrive: gate.clone(),
            leave: gate,
        },
    );
}

impl Yard {
    // Loads and dispatches shipment `id` of packaged food from the processor to the grocery.
    fn dispatch(&mut self, id: u64) -> usize {
        let catalog = load_runtime_economy_catalog().unwrap();
        self.shipments.shipments.push(Shipment {
            id,
            resource_runtime_id: catalog.resource_runtime_id_for_id(PACKAGED_FOOD).unwrap(),
            amount: 40.0,
            source: ShipmentEndpoint::Building(0),
            destination: ShipmentEndpoint::Building(1),
            carrier_class: CarrierClass::Truck,
            status: ShipmentStatus::InTransit,
            carrier: AgentHandle::NONE,
            total_cost: 600.0,
            freight_cost: 50.0,
            eta_hours: 1,
            leg_started_hour: 0,
            trip_event_slot: 0,
            cargo_loaded: true,
        });
        let mut treasury = 0.0;
        assert!(self.shipments.dispatch_shipment(
            id,
            &mut self.allocator,
            &mut self.agents,
            &self.network,
            &self.graph,
            &mut treasury,
        ));
        let idx = self.shipments.shipment_index(id).unwrap();
        self.agents
            .resolve_agent(self.shipments.shipments[idx].carrier)
            .unwrap()
    }

    fn tick(&mut self) {
        self.agents.tick_with_parking(
            &self.allocator,
            &mut self.network,
            &mut self.graph,
            0.1,
            &test_clock(0, 0),
            &mut self.parking,
        );
    }

    // Ticks until `done` holds, at most 6,000 ticks (10 minutes).
    fn tick_until(&mut self, mut done: impl FnMut(&Self) -> bool) -> usize {
        for tick in 0..6_000 {
            if done(self) {
                return tick;
            }
            self.tick();
        }
        panic!("not reached in 6,000 ticks");
    }

    // One `FreightCheck` of shipment `id`.
    fn check(&mut self, id: u64) -> bool {
        let mut treasury = 0.0;
        self.shipments.check_shipment(
            id,
            &mut self.allocator,
            &mut self.agents,
            &self.network,
            &self.graph,
            &mut treasury,
            &mut |_, _| {},
        )
    }

    fn status(&self, id: u64) -> ShipmentStatus {
        let idx = self.shipments.shipment_index(id).unwrap();
        self.shipments.shipments[idx].status
    }

    fn grocery_food(&self) -> f32 {
        let catalog = load_runtime_economy_catalog().unwrap();
        self.allocator.buildings[1]
            .inventory_units(catalog.resource_runtime_id_for_id(PACKAGED_FOOD).unwrap())
    }

    fn inside_grocery(&self, i: usize) -> bool {
        self.agents.agents.transit[i] == TRANSIT_IN_BUILDING
            && self.agents.agents.current_building[i] == 1
    }

    fn handle(&mut self, i: usize) -> AgentHandle {
        self.agents.agent_handle(i)
    }

    fn dock(&self, anchor: u32) -> crate::simulation::core::handles::Handle<crate::simulation::parking::Place> {
        let handle = self.allocator.building_handles.handle(1).to_bits();
        self.parking.building_docks(handle).nth(anchor as usize).unwrap()
    }
}

// Whether `trip` is the row of an agent with no trip: never written, or its yard trip ended.
fn idle(trip: ParkingTrip) -> bool {
    format!("{trip:?}") == format!("{:?}", ParkingTrip::default())
}

// The lane point a carrier plans to leave the road at.
fn detach_point(yard: &Yard, i: usize) -> Vector2 {
    let a = &yard.agents.agents;
    let lane = &yard.network.lane_system.lanes[a.planned_detach_lane_id[i] as usize];
    BuildingAllocator::sample_pos_on_lane(lane, a.planned_detach_lane_d[i])
}

#[test]
fn a_carrier_drives_into_the_dock_stands_in_the_bay_and_delivers_there() {
    let mut yard = yard(1);
    let carrier = yard.dispatch(0);
    let handle = yard.allocator.building_handles.handle(1).to_bits();
    let trip = yard.agents.agents.parking_trip[carrier];
    assert!(trip.freight_gate && trip.target == handle && trip.phase == ParkingTripPhase::None);
    // Its plan ends at the gate's lane point, not the door's.
    let gate = detach_point(&yard, carrier);
    assert!((gate.x - GATE_X).abs() < 0.5, "planned to leave the road at {gate:?}");

    // At the gate the ordered phase grants the dock: the first tick in access ingress.
    yard.tick_until(|y| y.agents.agents.parking_trip[carrier].phase != ParkingTripPhase::None);
    let dock = yard.dock(0);
    let trip = yard.agents.agents.parking_trip[carrier];
    assert_eq!(trip.phase, ParkingTripPhase::PullIn);
    assert_eq!(trip.destination, dock);
    assert_eq!(yard.parking.dock_holder(dock), yard.handle(carrier));
    assert_eq!(
        yard.agents.agents.transit[carrier],
        crate::simulation::economy::agents::TRANSIT_ACCESS_INGRESS
    );

    // The route's end stands it in the bay, drawn as an off-lane car.
    yard.tick_until(|y| y.agents.agents.parking_trip[carrier].phase == ParkingTripPhase::AtDock);
    let stop = yard.parking.dock_route(dock).unwrap()[3];
    let at = |y: &Yard| (y.agents.agents.pos_x[carrier], y.agents.agents.pos_y[carrier]);
    assert_eq!(at(&yard), (stop[0], stop[1]));
    assert!(crate::simulation::economy::agents::transit_is_visible(
        yard.agents.agents.transit[carrier]
    ));
    // It stands: no tick moves it and none notes it for the parking phase.
    for _ in 0..50 {
        yard.tick();
        assert_eq!(at(&yard), (stop[0], stop[1]));
        assert_eq!(
            yard.agents.agents.parking_trip[carrier].phase,
            ParkingTripPhase::AtDock
        );
        assert_eq!(yard.agents.parking_work.last_candidates, 0);
    }

    // The next check delivers where it stands and sends it back.
    let before = yard.grocery_food();
    assert!(yard.check(0), "the carrier turns back");
    assert_eq!(yard.status(0), ShipmentStatus::Returning);
    assert_eq!(yard.grocery_food(), before + 40.0);
    // Round 1: the return starts at the door (`PARK-20` round 2 reverses out through the gate),
    // and the dock is free for the next carrier.
    let returning = yard.agents.resolve_agent(yard.shipments.shipments[0].carrier).unwrap();
    assert!(idle(yard.agents.agents.parking_trip[returning]));
}

// Two carriers, one dock: the first through the gate takes it, the second goes on to the door
// from the gate and delivers there. Every product is the same at 1 and 8 workers.
#[test]
fn a_second_carrier_finds_the_dock_held_and_delivers_at_the_door_on_any_worker_count() {
    let run = |workers: usize| {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            let mut yard = yard(1);
            let first = yard.dispatch(0);
            let second = yard.dispatch(1);
            let dock = yard.dock(0);
            let mut trace = Vec::new();
            let ticks = yard.tick_until(|y| {
                y.agents.agents.parking_trip[first].phase == ParkingTripPhase::AtDock
                    && y.inside_grocery(second)
            });
            let held_by_first = yard.parking.dock_holder(dock) == yard.handle(first);
            let second_idle = idle(yard.agents.agents.parking_trip[second]);
            for i in [first, second] {
                let a = &yard.agents.agents;
                trace.push((
                    a.pos_x[i].to_bits(),
                    a.pos_y[i].to_bits(),
                    a.transit[i],
                    format!("{:?}", a.parking_trip[i]),
                ));
            }
            let before = yard.grocery_food();
            assert!(yard.check(0) && yard.check(1));
            let delivered = yard.grocery_food() - before;
            let statuses = [yard.status(0), yard.status(1)];
            (ticks, held_by_first, second_idle, trace, delivered, statuses)
        })
    };
    let serial = run(1);
    let (_, held_by_first, second_idle, _, delivered, statuses) = &serial;
    assert!(*held_by_first, "the first carrier holds the dock");
    // The second went through the gate and on to the door: its yard trip ended there.
    assert!(*second_idle);
    assert_eq!(*delivered, 80.0);
    assert_eq!(*statuses, [ShipmentStatus::Returning; 2]);
    assert_eq!(run(8), serial);
}

// The gate commit's refusal on its own: the second carrier, held at the gate, drives the path
// from the gate's lane point to the door, never waiting for the dock.
#[test]
fn a_refused_carrier_drives_from_the_gate_to_the_door() {
    let mut yard = yard(1);
    let first = yard.dispatch(0);
    yard.tick_until(|y| y.agents.agents.parking_trip[first].phase == ParkingTripPhase::AtDock);
    let second = yard.dispatch(1);
    let gate = detach_point(&yard, second);
    assert!((gate.x - GATE_X).abs() < 0.5);
    yard.tick_until(|y| y.agents.agents.parking_trip[second].phase != ParkingTripPhase::None);
    assert_eq!(
        yard.agents.agents.parking_trip[second].phase,
        ParkingTripPhase::GateToDoor
    );
    let door = yard.allocator.entrances[1].door_pos;
    yard.tick_until(|y| y.inside_grocery(second));
    assert_eq!(
        (yard.agents.agents.pos_x[second], yard.agents.agents.pos_y[second]),
        (door.x, door.y)
    );
    assert!(idle(yard.agents.agents.parking_trip[second]));
    // The dock stays with the first.
    let dock = yard.dock(0);
    assert_eq!(yard.parking.dock_holder(dock), yard.handle(first));
}

// A carrier that left (here, delivered and turned back) holds the dock no more: the next one
// through the gate takes it, though the row still names the first.
#[test]
fn a_dock_its_carrier_left_is_free_for_the_next() {
    let mut yard = yard(1);
    let first = yard.dispatch(0);
    yard.tick_until(|y| y.agents.agents.parking_trip[first].phase == ParkingTripPhase::AtDock);
    assert!(yard.check(0));
    let dock = yard.dock(0);
    assert_eq!(yard.parking.dock_holder(dock), yard.handle(first));
    let second = yard.dispatch(1);
    yard.tick_until(|y| y.agents.agents.parking_trip[second].phase != ParkingTripPhase::None);
    assert_eq!(
        yard.agents.agents.parking_trip[second].phase,
        ParkingTripPhase::PullIn
    );
    assert_eq!(yard.parking.dock_holder(dock), yard.handle(second));
}

// A building without a dock: the carrier's plan, path and arrival are the door's, and its trip
// row is never written.
#[test]
fn a_carrier_for_a_building_without_a_dock_goes_to_the_door_as_before() {
    let mut yard = yard(0);
    let carrier = yard.dispatch(0);
    let door_plan = crate::simulation::economy::agents::tick::plan_building_origin_trip(
        0,
        1,
        2,
        true,
        &yard.allocator,
        &yard.network,
        &yard.graph,
        &std::sync::atomic::AtomicU32::new(0),
    )
    .unwrap();
    let a = &yard.agents.agents;
    assert_eq!(a.planned_detach_lane_id[carrier] as usize, door_plan.planned_detach_lane_id);
    assert_eq!(
        a.planned_detach_lane_d[carrier].to_bits(),
        door_plan.planned_detach_lane_d.to_bits()
    );
    assert_eq!(a.current_path[carrier], door_plan.current_path);
    yard.tick_until(|y| {
        assert!(idle(y.agents.agents.parking_trip[carrier]));
        y.inside_grocery(carrier)
    });
    let door = yard.allocator.entrances[1].door_pos;
    assert_eq!(
        (yard.agents.agents.pos_x[carrier], yard.agents.agents.pos_y[carrier]),
        (door.x, door.y)
    );
    assert!(yard.check(0));
    assert_eq!(yard.status(0), ShipmentStatus::Returning);
}

// Freight prices never read the yard (owner decision 5): a dock building's quote is its quote
// without the dock, bit for bit, though its carriers now plan to the gate.
#[test]
fn the_quote_of_a_dock_building_equals_its_quote_without_the_dock() {
    let tuning = load_runtime_economy_tuning().unwrap();
    let quote = |docks: u32| {
        let mut yard = yard(docks);
        yard.shipments
            .prepare_site_carrier_quotes(&yard.allocator, &yard.network, &yard.graph, &[]);
        let entrances = &yard.allocator.entrances;
        let (from, to) = (entrances[0].clone(), entrances[1].clone());
        yard.shipments
            .customer_carrier_quote(
                &from,
                &to,
                Some(0),
                Some(1),
                &yard.allocator,
                &yard.network,
                &yard.graph,
                &tuning,
            )
            .unwrap()
            .to_bits()
    };
    assert_eq!(quote(1), quote(0));
}

// A carrier standing in its bay passes the load's custody check and its row reads back as it
// was written; a row with no yard trip writes no new key, so a city without docks saves the
// bytes it saved before.
#[test]
fn a_standing_carrier_saves_and_validates_and_an_idle_row_writes_no_new_key() {
    let mut yard = yard(1);
    let carrier = yard.dispatch(0);
    yard.tick_until(|y| y.agents.agents.parking_trip[carrier].phase == ParkingTripPhase::AtDock);
    let trip = yard.agents.agents.parking_trip[carrier];
    let text = serde_json::to_string(&trip).unwrap();
    let back: ParkingTrip = serde_json::from_str(&text).unwrap();
    assert_eq!(format!("{back:?}"), format!("{trip:?}"));
    let households = crate::simulation::economy::households::HouseholdSystem::new();
    yard.parking.validate_custody(&yard.agents, &households).unwrap();
    let idle_row = serde_json::to_string(&ParkingTrip::default()).unwrap();
    assert!(!idle_row.contains("freight_gate"), "{idle_row}");
}
