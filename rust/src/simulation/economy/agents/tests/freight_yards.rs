// SPDX-License-Identifier: GPL-3.0-or-later

//! Freight yards (`PARK-20`) on the minimal straight road: a carrier bound for a building with
//! a published dock detaches at the gate, is granted the dock in the ordered phase, drives the
//! dock's route and stands in the bay, where its shipment's check delivers; with every dock held
//! it waits at the building's kerb stop for the next free dock in arrival order (`PARK-36`). A
//! building without a dock is served at its kerb stop, never through its door. The dock rows and the gate are made as the site
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
    yard_with_gate(source_docks, docks, GATE_OFFSET)
}

// [`yard_with`] with each gate `gate_offset` metres east of its building's door.
fn yard_with_gate(source_docks: u32, docks: u32, gate_offset: f32) -> Yard {
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
            publish_docks(&mut parking, &mut allocator, &network, &graph, building, count, gate_offset);
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

// Building `building`'s dock rows, their access and its gate `gate_offset` metres east of its
// door, as `SimCore::publish_docks` and `rebuild_freight_gate` make them: each route runs from the
// gate's road connection, 4.25 m in (the approach), across to the aisle and into the bay 6 m
// behind it.
fn publish_docks(
    parking: &mut ParkingSystem,
    allocator: &mut BuildingAllocator,
    network: &TransitNetwork,
    graph: &RegionGraph,
    building: usize,
    docks: u32,
    gate_offset: f32,
) {
    let handle = allocator.building_handles.handle(building).to_bits();
    let entrance = allocator.entrances[building].clone();
    let kerb = entrance.curb_pos.y;
    let gate_x = allocator.buildings[building].center_x + gate_offset;
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
    parking.replace_docks(handle, &rows, &[], &[]);
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
            elsewhere: Vec::new(),
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

    // Whether truck `i` stands at the kerb stop of `building` (`PARK-36`), where it delivers or,
    // back at its source, finishes.
    fn at_kerb(&self, i: usize, building: usize) -> bool {
        let trip = self.agents.agents.parking_trip[i];
        trip.phase == ParkingTripPhase::AtKerb
            && trip.target == self.allocator.building_handles.handle(building).to_bits()
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

// The kerb stop of building `building` a truck stands at (`PARK-36`).
fn kerb_stop(yard: &Yard, building: usize) -> Vector2 {
    crate::simulation::economy::agents::tick::kerb_stop_entrance(
        &yard.allocator.entrances[building],
        &yard.graph,
        crate::simulation::economy::agents::tick::FREIGHT_KERB_INSET_M,
    )
    .unwrap()
    .door_pos
}

// Place `slot` of the line of trucks waiting before building `building`'s gate (`PARK-36`): at
// the kerb beside the gate's near lane, `slot + 1` places back from the gate's lane point.
fn line_place(yard: &Yard, building: usize, slot: usize) -> Vector2 {
    let (gate, lane, gate_d) = gate_lane(yard, building);
    let lane = &yard.network.lane_system.lanes[lane];
    let d = crate::simulation::economy::agents::tick::gate_line_d(gate_d, slot);
    let point = BuildingAllocator::sample_pos_on_lane(lane, d);
    crate::simulation::economy::agents::tick::kerb_stop_beside(&gate, &yard.graph, point)
        .unwrap()
        .door_pos
}

// Building `building`'s gate, its near lane (the forward lane: the buildings are on side -1) and
// the lane distance of its lane point.
fn gate_lane(yard: &Yard, building: usize) -> (crate::simulation::buildings::allocator::BuildingEntrance, usize, f32) {
    let handle = yard.allocator.building_handles.handle(building).to_bits();
    let gate = yard
        .allocator
        .freight_gate_entrance(handle, &yard.graph, &yard.network.lane_system)
        .unwrap();
    let lane = gate.car_lane_fwd;
    let length = yard.graph.edge(gate.edge_idx).physical_length;
    let point =
        BuildingAllocator::sample_pos_on_edge(&yard.graph, gate.edge_idx, gate.entrance_s_m / length);
    let geometry = &yard.network.lane_system.lanes[lane].geometry;
    (gate, lane, BuildingAllocator::project_point_to_polyline_s(geometry, point))
}

fn at(yard: &Yard, i: usize) -> Vector2 {
    Vector2::new(yard.agents.agents.pos_x[i], yard.agents.agents.pos_y[i])
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

// Two carriers, one dock: the first through the gate takes it; the second finds it held at the
// end of the line before the gate and waits at the kerb there (`PARK-36`), never at the door;
// once the first has delivered and left, the dock is granted to the second, which pulls forward
// into the gate, docks and delivers. Every product is the same at 1 and 8 workers.
#[test]
fn a_second_carrier_waits_at_the_kerb_and_docks_after_the_first_on_any_worker_count() {
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
            let mut track = |y: &Yard| {
                assert!(!y.inside_grocery(first) && !y.inside_grocery(second), "a door");
                let a = &y.agents.agents;
                trace.push([first, second].map(|i| {
                    (a.pos_x[i].to_bits(), a.pos_y[i].to_bits(), a.transit[i], format!("{:?}", a.parking_trip[i]))
                }));
            };
            yard.tick_until(|y| {
                track(y);
                y.phase(first) == ParkingTripPhase::AtDock
                    && y.phase(second) == ParkingTripPhase::AtKerb
                    && at(y, second).distance_to(line_place(y, 1, 0)) < 0.05
            });
            let waiting = (at(&yard, second), yard.agents.agents.parking_trip[second].ready);
            let held_by_first = yard.parking.dock_holder(dock) == yard.handle(first);
            // The first delivers and backs out; the dock is the second's once it is free.
            assert!(yard.check(0));
            yard.tick_until(|y| {
                track(y);
                y.phase(second) == ParkingTripPhase::AtDock
            });
            let before = yard.grocery_food();
            assert!(yard.check(1));
            let delivered = yard.grocery_food() - before;
            (trace, waiting, held_by_first, delivered, [yard.status(0), yard.status(1)])
        })
    };
    let serial = run(1);
    let yard = yard(1);
    let (_, (waiting_at, ready), held_by_first, delivered, statuses) = &serial;
    assert!(*held_by_first, "the first carrier holds the dock");
    assert!(waiting_at.distance_to(line_place(&yard, 1, 0)) < 0.05, "waits before the gate");
    assert!(*ready, "waiting for the parking phase's grant");
    assert_eq!(*delivered, 40.0);
    assert_eq!(*statuses, [ShipmentStatus::Returning; 2]);
    assert_eq!(run(8), serial);
}

// The commit's refusal on its own (`PARK-36`): the second carrier stops on its lane at the end of
// the line before the gate, a place short of the gate's lane point, finds the dock held, leaves
// the lane there and pulls over to the kerb beside it, where it stands drawn, out of the lane,
// waiting; the dock stays with the first. It never reaches the gate or enters the building.
#[test]
fn a_refused_carrier_waits_at_the_kerb_before_the_gate() {
    let mut yard = yard(1);
    let first = yard.dispatch(0);
    yard.tick_until(|y| y.agents.agents.parking_trip[first].phase == ParkingTripPhase::AtDock);
    let second = yard.dispatch(1);
    let gate = detach_point(&yard, second);
    assert!((gate.x - GATE_X).abs() < 0.5);
    yard.tick_until(|y| y.agents.agents.parking_trip[second].phase != ParkingTripPhase::None);
    assert_eq!(yard.phase(second), ParkingTripPhase::AtKerb);
    let (_, lane, gate_d) = gate_lane(&yard, 1);
    let a = &yard.agents.agents;
    // It left the lane at the end of the line, a place before the gate's lane point.
    assert_eq!(a.planned_detach_lane_id[second] as usize, lane);
    let left_at = a.planned_detach_lane_d[second];
    assert!(left_at <= gate_d - crate::config::TRUCK_BODY_LENGTH_M, "{left_at} {gate_d}");
    yard.tick_until(|y| {
        assert!(!y.inside_grocery(second));
        assert!(at(y, second).x < GATE_X - 4.0, "short of the gate: {:?}", at(y, second));
        at(y, second).distance_to(line_place(y, 1, 0)) < 0.05
    });
    let a = &yard.agents.agents;
    assert!(crate::simulation::economy::agents::transit_is_visible(a.transit[second]));
    assert_eq!(a.current_lane_id[second], usize::MAX, "out of the driving lane");
    let trip = a.parking_trip[second];
    assert!(trip.freight_gate && trip.ready && trip.destination.is_none());
    // It waits: the shipment's check finds it not arrived.
    for _ in 0..50 {
        yard.tick();
        assert_eq!(yard.phase(second), ParkingTripPhase::AtKerb);
    }
    assert!(yard.check(1));
    assert_eq!(yard.status(1), ShipmentStatus::InTransit);
    let dock = yard.dock(0);
    assert_eq!(yard.parking.dock_holder(dock), yard.handle(first));
}

// Three trucks for a one-dock yard: the docks go in the order the trucks reached the gate, not
// in agent order. The third is dispatched first, so it is the lowest agent, yet it arrives last.
#[test]
fn trucks_waiting_at_the_kerb_take_the_dock_in_arrival_order() {
    let mut yard = yard(1);
    let first = yard.dispatch(0);
    yard.tick_until(|y| y.phase(first) == ParkingTripPhase::AtDock);
    let second = yard.dispatch(1);
    yard.tick_until(|y| y.phase(second) == ParkingTripPhase::AtKerb);
    let third = yard.dispatch(2);
    yard.tick_until(|y| {
        y.phase(third) == ParkingTripPhase::AtKerb
            && at(y, third).distance_to(line_place(y, 1, 1)) < 0.05
    });
    let stamp = |y: &Yard, i: usize| y.agents.agents.journey_start_time[i];
    assert!(stamp(&yard, second) < stamp(&yard, third));
    // The third stands behind the second, one place back along the kerb, before the gate.
    assert!(at(&yard, second).distance_to(line_place(&yard, 1, 0)) < 0.05);
    let mut order = Vec::new();
    for (id, i) in [(0, first), (1, second), (2, third)] {
        yard.tick_until(|y| {
            for &j in &[first, second, third] {
                assert!(!y.inside_grocery(j), "no truck takes the door");
                if y.phase(j) == ParkingTripPhase::PullIn && !order.contains(&j) {
                    order.push(j);
                }
            }
            y.phase(i) == ParkingTripPhase::AtDock
        });
        assert!(yard.check(id));
    }
    assert_eq!(order, [second, third]);
}

// A truck waiting at the kerb whose shipment's leg runs out leaves the queue: the check expires
// the shipment, refunds the buyer and removes the truck where it stands; the dock was never its.
#[test]
fn a_truck_waiting_at_the_kerb_times_out_and_is_refunded() {
    let mut yard = yard(1);
    let first = yard.dispatch(0);
    yard.tick_until(|y| y.phase(first) == ParkingTripPhase::AtDock);
    let second = yard.dispatch(1);
    yard.tick_until(|y| y.phase(second) == ParkingTripPhase::AtKerb);
    let budget = yard.allocator.buildings[1].operating_budget;
    yard.shipments.hour_ticks = 1_000;
    assert!(!yard.check(1));
    assert_eq!(yard.status(1), ShipmentStatus::Expired);
    assert_eq!(yard.allocator.buildings[1].operating_budget, budget + 600.0, "the buyer is refunded");
    assert!(yard.agents.resolve_agent(yard.shipments.shipments[1].carrier).is_none());
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

// A building without a dock (`PARK-36`): the carrier's plan is a car's to the door, but it starts standing
// at its source's kerb stop and pulls out, never from the door; at the grocery it pulls over to the
// kerb stop and stands there, drawn, out of the driving lane, never inside; its check delivers
// there and turns it back from the kerb, and back at its source it stands at the kerb stop again,
// where the check finishes the shipment and removes it.
#[test]
fn a_carrier_for_a_building_without_a_dock_stands_at_the_kerb_and_never_enters() {
    let mut yard = yard(0);
    let carrier = yard.dispatch(0);
    let door_plan = crate::simulation::economy::agents::tick::plan_car_trip_between_entrances(
        &yard.allocator.entrances[0],
        1,
        2,
        &yard.allocator.entrances[1],
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
    // At its source's kerb stop, leaving: drawn.
    assert!(at(&yard, carrier).distance_to(kerb_stop(&yard, 0)) < 0.05);
    assert_eq!(a.transit[carrier], crate::simulation::economy::agents::TRANSIT_ACCESS_EGRESS);
    yard.tick_until(|y| {
        assert!(!y.inside(carrier, 0) && !y.inside_grocery(carrier));
        y.phase(carrier) == ParkingTripPhase::AtKerb
    });
    let a = &yard.agents.agents;
    assert!(at(&yard, carrier).distance_to(kerb_stop(&yard, 1)) < 0.05);
    assert!(crate::simulation::economy::agents::transit_is_visible(a.transit[carrier]));
    assert_eq!(a.current_lane_id[carrier], usize::MAX, "out of the driving lane");
    let trip = a.parking_trip[carrier];
    assert!(!trip.freight_gate && !trip.ready);
    assert_eq!(trip.target, yard.allocator.building_handles.handle(1).to_bits());
    // The stand's clock starts at the stop (the floor's start, `KERB_STAND_FLOOR_MINUTES`).
    assert_eq!(a.journey_start_time[carrier], yard.agents.sim_time);
    let stood = at(&yard, carrier);
    for _ in 0..50 {
        yard.tick();
        assert_eq!(at(&yard, carrier), stood, "it stands");
    }
    let before = yard.grocery_food();
    assert!(yard.check(0));
    assert_eq!(yard.status(0), ShipmentStatus::Returning);
    assert_eq!(yard.grocery_food(), before + 40.0);
    // Back from the kerb stop where it stood.
    let a = &yard.agents.agents;
    assert_eq!(a.transit[carrier], crate::simulation::economy::agents::TRANSIT_ACCESS_EGRESS);
    assert!(at(&yard, carrier).distance_to(kerb_stop(&yard, 1)) < 0.05);
    assert!(idle(a.parking_trip[carrier]));
    yard.tick_until(|y| {
        assert!(!y.inside(carrier, 0) && !y.inside_grocery(carrier));
        y.phase(carrier) == ParkingTripPhase::AtKerb
    });
    assert!(at(&yard, carrier).distance_to(kerb_stop(&yard, 0)) < 0.05);
    assert!(!yard.check(0));
    assert_eq!(yard.status(0), ShipmentStatus::Fulfilled);
    assert!(yard.agents.agents.is_empty(), "removed at its home's kerb stop");
}

// A truck inside a building without a dock, as a save from before `PARK-36` holds one: its check
// takes it as arrived and delivers, and it leaves from the kerb stop, never the door.
#[test]
fn a_truck_inside_a_building_without_a_dock_delivers_and_leaves_from_the_kerb() {
    let mut yard = yard(0);
    let carrier = yard.dispatch(0);
    let a = &mut yard.agents.agents;
    a.transit[carrier] = TRANSIT_IN_BUILDING;
    a.current_building[carrier] = 1;
    a.current_lane_id[carrier] = usize::MAX;
    a.current_path[carrier].clear();
    let before = yard.grocery_food();
    assert!(yard.check(0));
    assert_eq!(yard.status(0), ShipmentStatus::Returning);
    assert_eq!(yard.grocery_food(), before + 40.0);
    let a = &yard.agents.agents;
    assert_eq!(a.transit[carrier], crate::simulation::economy::agents::TRANSIT_ACCESS_EGRESS);
    assert!(at(&yard, carrier).distance_to(kerb_stop(&yard, 1)) < 0.05);
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
// and backs along the route towards the gate at the yard speed; returns the ticks it took.
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
        // the lane point, never faster than the yard speed.
        assert!(at.distance_to(last) <= 0.1 * crate::config::FREIGHT_YARD_SPEED_MS + 1e-4);
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
    yard.tick_until(|y| y.at_kerb(carrier, 0));
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

// With its source's only dock held, a dispatch starts at the kerb stop exactly as a dispatch from
// a building without a dock (`PARK-36`): the same plan and the same place.
#[test]
fn with_no_free_dock_a_dispatch_starts_at_the_kerb_stop_as_without_a_dock() {
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
    assert!(at(&docked, second).distance_to(kerb_stop(&docked, 0)) < 0.05);
    assert!(docked.holds(first, docked.dock_of(0, 0)));
}

// Two-way traffic through one dock: a truck arriving while the delivered one is still backing
// out finds the dock held at the end of the line and waits at the kerb there; the dock is granted
// to it once the first has left, and it delivers in the dock. Every product is the same at 1 and 8 workers.
#[test]
fn an_arrival_meeting_a_truck_backing_out_waits_at_the_kerb_on_any_worker_count() {
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
            yard.tick_until(|y| {
                assert!(!y.inside_grocery(second));
                y.phase(second) == ParkingTripPhase::AtDock
            });
            let before = yard.grocery_food();
            assert!(yard.check(1));
            (trace, refused, first_backing, yard.grocery_food() - before, yard.status(1))
        })
    };
    let serial = run(1);
    let (_, refused, first_backing, delivered, status) = &serial;
    assert!(*first_backing, "the first still backs out");
    assert_eq!(*refused, ParkingTripPhase::AtKerb);
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
    yard.tick_until(|y| y.at_kerb(carrier, 0));
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

// A carrier on its way from the gate to the kerb stop that a stale lane puts back on the road
// plans to the kerb stop again (`PARK-36`), with the gate or without it.
#[test]
fn a_carrier_on_its_way_to_the_kerb_stop_replans_to_the_kerb_stop() {
    let mut yard = yard(1);
    let handle = yard.allocator.building_handles.handle(1).to_bits();
    let trip = ParkingTrip {
        target: handle,
        phase: ParkingTripPhase::ToKerb,
        freight_gate: true,
        ..ParkingTrip::default()
    };
    let entrance = |y: &Yard| {
        y.parking
            .replan_entrance(trip, &y.allocator, &y.network, &y.graph)
            .map(|e| e.door_pos)
    };
    let kerb = kerb_stop(&yard, 1);
    assert_eq!(entrance(&yard), Some(kerb));
    yard.allocator.freight_gates.clear();
    assert_eq!(entrance(&yard), Some(kerb));
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

mod drive_through;
mod gate_line;
