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
// A gate's road connection: 20 m east of its building's door, so the gate's lane point and the
// door's differ.
const GATE_OFFSET: f32 = 20.0;
const GATE_X: f32 = YARD_X + GATE_OFFSET;
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
    yard_with(0, docks)
}

// [`yard`] with `source_docks` loading bays behind a gate of the processor as well (`PARK-20`:
// trucks load at the source's dock).
fn yard_with(source_docks: u32, docks: u32) -> Yard {
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
    for (building, count) in [(0, source_docks), (1, docks)] {
        if count > 0 {
            publish_docks(&mut parking, &mut allocator, &network, &graph, building, count);
        }
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

// Building `building`'s dock rows, their access and its gate, as `SimCore::publish_docks` and
// `rebuild_freight_gate` make them: each route runs from the gate's road connection, 4.25 m in
// (the approach), across to the aisle and into the bay 6 m behind it.
fn publish_docks(
    parking: &mut ParkingSystem,
    allocator: &mut BuildingAllocator,
    network: &TransitNetwork,
    graph: &RegionGraph,
    building: usize,
    docks: u32,
) {
    let handle = allocator.building_handles.handle(building).to_bits();
    let entrance = allocator.entrances[building].clone();
    let kerb = entrance.curb_pos.y;
    let gate_x = allocator.buildings[building].center_x + GATE_OFFSET;
    let rows: Vec<_> = (0..docks)
        .map(|anchor| {
            let x = gate_x + 4.0 * anchor as f32;
            let stop = kerb - 20.0;
            let bay = Bay {
                identity: [handle, u64::from(anchor)],
                position: [x, 0.0, stop],
                path: [
                    [gate_x, kerb],
                    [gate_x, kerb - 4.25],
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
    parking.replace_docks(handle, &rows, &[]);
    parking.refresh_dock_access(handle, allocator, network, graph);
    let b = &allocator.buildings[building];
    let gate = BuildingAllocator::parking_access_at(
        entrance.door_pos,
        Vector2::new(gate_x, kerb),
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
            &mut self.parking,
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
        self.dock_of(1, anchor)
    }

    fn dock_of(
        &self,
        building: usize,
        anchor: u32,
    ) -> crate::simulation::core::handles::Handle<crate::simulation::parking::Place> {
        let handle = self.allocator.building_handles.handle(building).to_bits();
        self.parking.building_docks(handle).nth(anchor as usize).unwrap()
    }

    fn inside(&self, i: usize, building: usize) -> bool {
        self.agents.agents.transit[i] == TRANSIT_IN_BUILDING
            && self.agents.agents.current_building[i] == building
    }

    fn phase(&self, i: usize) -> ParkingTripPhase {
        self.agents.agents.parking_trip[i].phase
    }

    fn holds(&self, i: usize, dock: crate::simulation::core::handles::Handle<crate::simulation::parking::Place>) -> bool {
        self.parking.dock_holder(dock) == self.agents.handles.handle(i)
            && self.agents.holds_freight_dock(self.agents.handles.handle(i), dock)
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

    // The next check delivers where it stands and sends it back, out of the dock it keeps.
    let before = yard.grocery_food();
    assert!(yard.check(0), "the carrier turns back");
    assert_eq!(yard.status(0), ShipmentStatus::Returning);
    assert_eq!(yard.grocery_food(), before + 40.0);
    let returning = yard.agents.resolve_agent(yard.shipments.shipments[0].carrier).unwrap();
    assert_eq!(yard.phase(returning), ParkingTripPhase::PullOut);
    assert!(yard.holds(returning, dock));
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

// A carrier that left (here, delivered, turned back and pulled out to the lane) holds the dock
// no more: the next one through the gate takes it, though the row still names the first.
#[test]
fn a_dock_its_carrier_left_is_free_for_the_next() {
    let mut yard = yard(1);
    let first = yard.dispatch(0);
    yard.tick_until(|y| y.agents.agents.parking_trip[first].phase == ParkingTripPhase::AtDock);
    assert!(yard.check(0));
    yard.tick_until(|y| y.phase(first) != ParkingTripPhase::PullOut);
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

// The lane point a carrier plans to join the road at.
fn attach_point(yard: &Yard, i: usize) -> Vector2 {
    let a = &yard.agents.agents;
    let lane = &yard.network.lane_system.lanes[a.planned_attach_lane_id[i] as usize];
    BuildingAllocator::sample_pos_on_lane(lane, a.planned_attach_lane_d[i])
}

// Ticks carrier `i` through its pull-out of `dock`, asserting each tick that it holds the dock
// and backs along the route towards the gate at driveway speed; returns the ticks it took.
fn pull_out(yard: &mut Yard, i: usize, dock: crate::simulation::core::handles::Handle<crate::simulation::parking::Place>) -> usize {
    let route = yard.parking.dock_route(dock).unwrap();
    let gate = attach_point(yard, i);
    let a = &yard.agents.agents;
    let mut last = Vector2::new(a.pos_x[i], a.pos_y[i]);
    assert_eq!(last, Vector2::new(route[3][0], route[3][1]), "it starts at the stop");
    for tick in 1..6_000 {
        yard.tick();
        let a = &yard.agents.agents;
        let at = Vector2::new(a.pos_x[i], a.pos_y[i]);
        if yard.phase(i) != ParkingTripPhase::PullOut {
            // At the gate's lane point: the yard trip is over and the dock free.
            assert!(at.distance_to(gate) < 1e-3, "left the yard at {at:?}, not {gate:?}");
            assert!(!yard.agents.holds_freight_dock(yard.agents.handles.handle(i), dock));
            return tick;
        }
        assert_eq!(a.transit[i], crate::simulation::economy::agents::TRANSIT_ACCESS_EGRESS);
        assert!(yard.holds(i, dock), "the dock is its own until the lane");
        // Straight back up the route (every route here is straight at the gate's x) and on to
        // the lane point, never faster than the driveway speed.
        assert!(at.distance_to(last) <= 0.1 * crate::config::AGENT_DRIVEWAY_SPEED_MS + 1e-4);
        assert!(at.y >= last.y - 1e-4, "it moves towards the street: {last:?} -> {at:?}");
        assert!((at.x - route[0][0]).abs() < 0.5, "off the route at {at:?}");
        last = at;
    }
    panic!("never reached the lane");
}

// After delivering, a truck backs out of its dock along the route reversed (owner decision 2),
// to the gate's lane point, where it joins the lane: the dock is its own until then and free
// from there on. Its return trip then runs to its source's door, which has no dock, as before.
#[test]
fn a_delivered_truck_backs_out_through_the_gate_and_frees_the_dock_at_the_lane() {
    let mut yard = yard(1);
    let carrier = yard.dispatch(0);
    yard.tick_until(|y| y.phase(carrier) == ParkingTripPhase::AtDock);
    assert!(yard.check(0));
    let dock = yard.dock(0);
    let trip = yard.agents.agents.parking_trip[carrier];
    assert!(trip.freight_gate && trip.source == dock);
    // Its plan ends at the source's door: nothing of the yard trip outlasts the pull-out.
    assert!(idle(trip.after_freight_pull_out()));
    assert!((attach_point(&yard, carrier).x - GATE_X).abs() < 0.5, "it leaves by the gate");
    let ticks = pull_out(&mut yard, carrier, dock);
    // 20 m of route at 3 m/s and the step to the lane.
    assert!((60..120).contains(&ticks), "{ticks} ticks");
    assert!(idle(yard.agents.agents.parking_trip[carrier]));
    assert_ne!(
        yard.agents.agents.transit[carrier],
        crate::simulation::economy::agents::TRANSIT_ACCESS_EGRESS
    );
    yard.tick_until(|y| y.inside(carrier, 0));
    assert!(!yard.check(0), "back at its source it finishes");
    assert_eq!(yard.status(0), ShipmentStatus::Fulfilled);
}

// Owner decision 10: a dispatch from a building with a free dock spawns its truck standing in
// the dock, which it takes, and it leaves at once through the gate. Bound for the grocery's
// yard, it keeps its yard trip past the lane, stands in the grocery's dock, backs out of it and
// returns to its source's dock, where the last check removes it.
#[test]
fn a_truck_loads_at_its_sources_dock_and_comes_back_to_it() {
    let mut yard = yard_with(1, 1);
    let carrier = yard.dispatch(0);
    let (home, away) = (yard.dock_of(0, 0), yard.dock(0));
    let handle = |y: &Yard, b: usize| y.allocator.building_handles.handle(b).to_bits();
    let trip = yard.agents.agents.parking_trip[carrier];
    assert_eq!(trip.phase, ParkingTripPhase::PullOut);
    assert_eq!(trip.source, home);
    assert_eq!(yard.agents.agents.current_building[carrier], 0);
    assert!(yard.holds(carrier, home));
    assert!((attach_point(&yard, carrier).x - (SOURCE_X + GATE_OFFSET)).abs() < 0.5);
    assert!((detach_point(&yard, carrier).x - GATE_X).abs() < 0.5, "bound for the yard's gate");
    pull_out(&mut yard, carrier, home);
    let trip = yard.agents.agents.parking_trip[carrier];
    assert!(trip.freight_gate && trip.phase == ParkingTripPhase::None);
    assert_eq!(trip.target, handle(&yard, 1));

    yard.tick_until(|y| y.phase(carrier) == ParkingTripPhase::AtDock);
    assert_eq!(yard.agents.agents.parking_trip[carrier].destination, away);
    assert!(yard.check(0));
    assert_eq!(yard.status(0), ShipmentStatus::Returning);
    let trip = yard.agents.agents.parking_trip[carrier];
    assert_eq!((trip.phase, trip.source), (ParkingTripPhase::PullOut, away));
    assert!(
        (detach_point(&yard, carrier).x - (SOURCE_X + GATE_OFFSET)).abs() < 0.5,
        "bound for its source's gate"
    );
    pull_out(&mut yard, carrier, away);
    assert_eq!(yard.agents.agents.parking_trip[carrier].target, handle(&yard, 0));
    // It asks its source for a dock as any arrival does, and stands in it.
    yard.tick_until(|y| y.phase(carrier) == ParkingTripPhase::AtDock);
    assert_eq!(yard.agents.agents.parking_trip[carrier].destination, home);
    let removed = yard.handle(carrier);
    assert!(!yard.check(0));
    assert_eq!(yard.status(0), ShipmentStatus::Fulfilled);
    assert!(yard.agents.resolve_agent(removed).is_none());
    assert!(!yard.agents.holds_freight_dock(yard.parking.dock_holder(home), home));
}

// With its source's only dock held, a dispatch starts at the door exactly as a dispatch from a
// building without a dock: the same plan and the same place.
#[test]
fn with_no_free_dock_a_dispatch_starts_at_the_door_as_before() {
    let mut docked = yard_with(1, 1);
    let first = docked.dispatch(0);
    assert_eq!(docked.phase(first), ParkingTripPhase::PullOut);
    let second = docked.dispatch(1);
    let mut plain = yard_with(0, 1);
    let reference = plain.dispatch(1);
    let row = |y: &Yard, i: usize| {
        let a = &y.agents.agents;
        (
            a.pos_x[i].to_bits(),
            a.pos_y[i].to_bits(),
            a.transit[i],
            a.planned_attach_lane_id[i],
            a.planned_attach_lane_d[i].to_bits(),
            a.planned_detach_lane_id[i],
            a.planned_detach_lane_d[i].to_bits(),
            a.current_path[i].clone(),
            format!("{:?}", a.parking_trip[i]),
        )
    };
    assert_eq!(row(&docked, second), row(&plain, reference));
    let door = docked.allocator.entrances[0].door_pos;
    let a = &docked.agents.agents;
    assert_eq!((a.pos_x[second], a.pos_y[second]), (door.x, door.y));
    assert!(docked.holds(first, docked.dock_of(0, 0)));
}

// Two-way traffic through one dock: a truck arriving while the delivered one is still backing
// out finds the dock held and goes on to the door. Every product is the same at 1 and 8 workers.
#[test]
fn an_arrival_meeting_a_truck_backing_out_goes_to_the_door_on_any_worker_count() {
    let run = |workers: usize| {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            let mut yard = yard(1);
            let first = yard.dispatch(0);
            yard.tick_until(|y| y.phase(first) == ParkingTripPhase::AtDock);
            let second = yard.dispatch(1);
            let gate = detach_point(&yard, second);
            // Turn the first back just before the second reaches the gate.
            yard.tick_until(|y| {
                let a = &y.agents.agents;
                a.transit[second] == crate::simulation::economy::agents::TRANSIT_NETWORK
                    && Vector2::new(a.pos_x[second], a.pos_y[second]).distance_to(gate) < 12.0
            });
            assert!(yard.check(0));
            let mut trace = Vec::new();
            yard.tick_until(|y| {
                let a = &y.agents.agents;
                trace.push([first, second].map(|i| {
                    (a.pos_x[i].to_bits(), a.pos_y[i].to_bits(), a.transit[i], format!("{:?}", a.parking_trip[i]))
                }));
                y.phase(second) != ParkingTripPhase::None
            });
            let refused = yard.phase(second);
            let first_backing = yard.phase(first) == ParkingTripPhase::PullOut;
            yard.tick_until(|y| y.inside_grocery(second));
            let before = yard.grocery_food();
            assert!(yard.check(1));
            (trace, refused, first_backing, yard.grocery_food() - before, yard.status(1))
        })
    };
    let serial = run(1);
    let (_, refused, first_backing, delivered, status) = &serial;
    assert!(*first_backing, "the first still backs out");
    assert_eq!(*refused, ParkingTripPhase::GateToDoor);
    assert_eq!(*delivered, 40.0);
    assert_eq!(*status, ShipmentStatus::Returning);
    assert_eq!(run(8), serial);
}

// Ticks `yard` until truck `i` has backed out of `dock` to the lane: the dock is free from there.
fn leave_the_yard(yard: &mut Yard, i: usize, dock: crate::simulation::core::handles::Handle<crate::simulation::parking::Place>) {
    yard.tick_until(|y| y.phase(i) != ParkingTripPhase::PullOut);
    let a = &yard.agents.agents;
    assert_ne!(a.transit[i], TRANSIT_IN_BUILDING, "it left by the road");
    assert!(!yard.agents.holds_freight_dock(yard.agents.handles.handle(i), dock));
}

// A pull-out whose plan a road edit drops (`clear_route_plan_after_road_edit` clears the access
// plan) stops where it is on its route, keeps its dock, and the parking phase plans its way out
// again in the same tick; it never goes back inside the building. Both for a truck spawned in its
// source's dock and for one delivered in the yard's.
#[test]
fn a_pull_out_whose_plan_goes_stands_in_the_yard_and_plans_again() {
    for spawned in [false, true] {
        let mut yard = yard_with(1, 1);
        let carrier = yard.dispatch(0);
        let dock = if spawned {
            yard.dock_of(0, 0)
        } else {
            yard.tick_until(|y| y.phase(carrier) == ParkingTripPhase::AtDock);
            assert!(yard.check(0));
            yard.dock(0)
        };
        for _ in 0..10 {
            yard.tick();
        }
        assert_eq!(yard.phase(carrier), ParkingTripPhase::PullOut);
        let at = |y: &Yard| (y.agents.agents.pos_x[carrier], y.agents.agents.pos_y[carrier]);
        let stood = at(&yard);
        let attach = (
            yard.agents.agents.planned_attach_lane_id[carrier],
            yard.agents.agents.planned_attach_lane_d[carrier].to_bits(),
        );
        yard.agents.agents.access_flags[carrier] = 0;
        yard.tick();
        assert_eq!(at(&yard), stood, "it stopped where it was");
        assert_eq!(yard.phase(carrier), ParkingTripPhase::PullOut);
        assert!(yard.holds(carrier, dock));
        let a = &yard.agents.agents;
        let trip = a.parking_trip[carrier];
        assert!(!trip.ready && !trip.replan_needed && !trip.retry_pending, "planned at once");
        assert_eq!(
            (a.planned_attach_lane_id[carrier], a.planned_attach_lane_d[carrier].to_bits()),
            attach,
            "the same way out of the same gate"
        );
        leave_the_yard(&mut yard, carrier, dock);
        assert!(yard.agents.trip_failures.lines.is_empty(), "{:?}", yard.agents.trip_failures.lines);
    }
}

// With no road from its gate to where it goes, a truck backing out stops in the yard, holding its
// dock, and waits for the parking retry wheel: it is reported once, never moved, never sent home
// or to a border by the watchdog, and its wheel retries back off (5 s, then 30 s). Once a road is
// back, the next retry plans its way out and it leaves. The shipment's leg timeout bounds the wait.
#[test]
fn a_pull_out_with_no_way_out_waits_in_the_yard_on_the_retry_wheel() {
    let mut yard = yard(1);
    let carrier = yard.dispatch(0);
    yard.tick_until(|y| y.phase(carrier) == ParkingTripPhase::AtDock);
    assert!(yard.check(0));
    let dock = yard.dock(0);
    for _ in 0..10 {
        yard.tick();
    }
    let handle = yard.handle(carrier);
    let at = |y: &Yard| (y.agents.agents.pos_x[carrier], y.agents.agents.pos_y[carrier]);
    let stood = at(&yard);
    // Its source (the return's destination) cut off: no plan reaches building 0's door.
    let door = yard.allocator.entrances[0].clone();
    yard.allocator.entrances[0].edge_idx = usize::MAX;
    yard.agents.agents.access_flags[carrier] = 0;
    yard.tick();
    let wheel = std::mem::take(&mut yard.agents.parking_retries);
    assert_eq!(wheel, [(handle, 5)]);
    let wake = |y: &mut Yard| {
        let i = y.agents.resolve_agent(handle).unwrap();
        y.agents.agents.parking_trip[i].retry_pending = false;
        y.agents.agents.next_replan_time[i] = 0.0;
        y.agents.wake_parking_agent(handle);
    };
    for round in 0..2 {
        for _ in 0..300 {
            yard.tick();
            let a = &yard.agents.agents;
            assert_eq!(at(&yard), stood, "it waits where it stopped");
            assert_eq!(a.transit[carrier], crate::simulation::economy::agents::TRANSIT_ACCESS_EGRESS);
            assert_eq!(yard.phase(carrier), ParkingTripPhase::PullOut);
            assert!(yard.holds(carrier, dock));
        }
        assert!(yard.agents.parking_retries.is_empty(), "nothing retries before the wheel");
        wake(&mut yard);
        yard.tick();
        let wheel = std::mem::take(&mut yard.agents.parking_retries);
        assert_eq!(wheel, [(handle, if round == 0 { 30 } else { 120 })]);
    }
    let lines = &yard.agents.trip_failures.lines;
    assert_eq!(lines.len(), 1, "reported once: {lines:#?}");
    assert!(lines[0].contains("no road from its loading bay's gate to building 0"), "{lines:#?}");
    assert!(lines[0].contains("stops in the yard, keeps its loading bay"), "{lines:#?}");
    assert!(
        !lines.iter().any(|l| l.contains("sent home") || l.contains("sent to border")),
        "{lines:#?}"
    );
    yard.allocator.entrances[0] = door;
    wake(&mut yard);
    leave_the_yard(&mut yard, carrier, dock);
    yard.tick_until(|y| y.inside(carrier, 0));
    assert!(!yard.check(0));
    assert_eq!(yard.status(0), ShipmentStatus::Fulfilled);
}

// A pull-in whose way in from the gate goes stale ends in its bay, as one whose plan was dropped
// does: it is never put back on the road (where the watchdog could move it).
#[test]
fn a_stale_pull_in_ends_in_its_bay() {
    let mut yard = yard(1);
    let carrier = yard.dispatch(0);
    yard.tick_until(|y| y.phase(carrier) == ParkingTripPhase::PullIn);
    yard.tick();
    let dock = yard.dock(0);
    yard.agents.agents.planned_detach_lane_id[carrier] = u32::MAX;
    yard.tick_until(|y| y.phase(carrier) != ParkingTripPhase::PullIn);
    assert_eq!(yard.phase(carrier), ParkingTripPhase::AtDock);
    let stop = yard.parking.dock_route(dock).unwrap()[3];
    let a = &yard.agents.agents;
    assert_eq!((a.pos_x[carrier], a.pos_y[carrier]), (stop[0], stop[1]));
    assert_eq!(a.transit[carrier], crate::simulation::economy::agents::TRANSIT_ACCESS_INGRESS);
    assert!(yard.holds(carrier, dock));
}

// A removed carrier holds no dock, in every phase that holds one: pulling in, standing, and
// backing out after a delivery or after a dispatch from its source's dock. Nothing calls the
// parking system: the next gate commit or dispatch finds the dock free.
#[test]
fn a_removed_carrier_frees_its_dock_in_every_phase() {
    for phase in [
        ParkingTripPhase::PullIn,
        ParkingTripPhase::AtDock,
        ParkingTripPhase::PullOut,
    ] {
        for spawned in [false, true] {
            if spawned && phase != ParkingTripPhase::PullOut {
                continue;
            }
            let mut yard = yard_with(1, 1);
            let carrier = yard.dispatch(0);
            let (building, dock) = if spawned {
                (0, yard.dock_of(0, 0))
            } else {
                let reached = if phase == ParkingTripPhase::PullOut {
                    ParkingTripPhase::AtDock
                } else {
                    phase
                };
                yard.tick_until(|y| y.phase(carrier) == reached);
                if phase == ParkingTripPhase::PullOut {
                    assert!(yard.check(0));
                    yard.tick();
                }
                (1, yard.dock(0))
            };
            assert_eq!(yard.phase(carrier), phase);
            assert!(yard.holds(carrier, dock));
            let handle = yard.allocator.building_handles.handle(building).to_bits();
            assert_eq!(yard.agents.first_free_dock(handle, &yard.parking), None);
            let removed = yard.handle(carrier);
            yard.agents.remove_freight_carrier(carrier, 0);
            assert!(yard.agents.resolve_agent(removed).is_none());
            assert_eq!(
                yard.agents.first_free_dock(handle, &yard.parking),
                Some(dock),
                "{phase:?}, spawned {spawned}"
            );
        }
    }
}

// A carrier sent on from the gate to the door that a stale lane puts back on the road plans to
// the gate again, where its access path to the door starts; with the gate gone, to the door.
#[test]
fn a_carrier_on_its_way_from_the_gate_to_the_door_replans_to_the_gate() {
    let mut yard = yard(1);
    let handle = yard.allocator.building_handles.handle(1).to_bits();
    let trip = ParkingTrip {
        target: handle,
        phase: ParkingTripPhase::GateToDoor,
        freight_gate: true,
        ..ParkingTrip::default()
    };
    let entrance = |y: &Yard| {
        y.parking
            .replan_entrance(trip, &y.allocator, &y.network, &y.graph)
            .map(|e| e.curb_pos)
    };
    let gate = yard.allocator.freight_gates[&handle].arrive.curb_pos;
    assert_eq!(entrance(&yard), Some(gate));
    yard.allocator.freight_gates.clear();
    assert_eq!(entrance(&yard), None, "to the door");
}

// The leg timeout bounds a truck's wait in the yard: the check that finds its return overdue
// finishes the shipment and removes the truck where it stands, which frees the dock. A dispatch
// that waits is expired the same way, with its refund.
#[test]
fn the_leg_timeout_ends_a_wait_in_the_yard_and_frees_the_dock() {
    let mut yard = yard(1);
    let carrier = yard.dispatch(0);
    yard.tick_until(|y| y.phase(carrier) == ParkingTripPhase::AtDock);
    assert!(yard.check(0));
    let dock = yard.dock(0);
    yard.tick();
    let handle = yard.handle(carrier);
    yard.allocator.entrances[0].edge_idx = usize::MAX;
    yard.agents.agents.access_flags[carrier] = 0;
    for _ in 0..100 {
        yard.tick();
    }
    assert!(yard.agents.agents.parking_trip[carrier].retry_pending);
    assert!(yard.check(0), "within its leg's hours it waits");
    yard.shipments.hour_ticks = 1_000;
    assert!(!yard.check(0));
    assert_eq!(yard.status(0), ShipmentStatus::Fulfilled);
    assert!(yard.agents.resolve_agent(handle).is_none());
    let building = yard.allocator.building_handles.handle(1).to_bits();
    assert_eq!(yard.agents.first_free_dock(building, &yard.parking), Some(dock));
}
