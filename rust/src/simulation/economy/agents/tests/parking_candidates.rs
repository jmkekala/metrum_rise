// SPDX-License-Identifier: GPL-2.0-only

//! Which agents the ordered parking phase visits (`PARK-02`, B1). Debug test builds also check
//! after every tick of every test that the phase missed no agent due in it.

use super::parking::fixture;
use super::*;
use crate::simulation::core::handles::Handle;
use crate::simulation::parking::{Bay, ParkingSystem, ParkingTripPhase, PlaceSource, Vehicle};
use crate::simulation::zoning::profiles::ZoneDensity;

struct World {
    agents: AgentSystem,
    parking: ParkingSystem,
    allocator: BuildingAllocator,
    network: TransitNetwork,
    graph: RegionGraph,
}

impl World {
    fn new() -> Self {
        let (agents, parking, allocator, network, graph) = fixture();
        Self { agents, parking, allocator, network, graph }
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

    fn candidates(&self) -> usize {
        self.agents.parking_work.last_candidates
    }

    // Ticks agent 0's trip up to `phase`, which only its first step can change.
    fn tick_until(&mut self, phase: ParkingTripPhase) {
        for _ in 0..8000 {
            if self.agents.parking_trip[0].phase == phase {
                return;
            }
            self.tick();
        }
        panic!("never reached {phase:?}");
    }

    // What a parking retry's event does; see `SimCore::run_trip_event`.
    fn release_retry(&mut self, agent: usize) {
        self.agents.agents.parking_trip[agent].retry_pending = false;
        self.agents.agents.next_replan_time[agent] = 0.0;
        let handle = self.agents.agent_handle(agent);
        self.agents.wake_parking_agent(handle);
    }

    fn car(&self) -> Handle<Vehicle> {
        self.agents.parking_trip[0].vehicle
    }
}

#[test]
fn an_idle_city_visits_no_agent_and_a_due_departure_visits_only_that_one() {
    let mut world = World::new();
    world.agents.agents.planned_target_building.fill(usize::MAX);
    for _ in 0..3 {
        world.tick();
        assert_eq!(world.candidates(), 0);
    }
    world.agents.agents.planned_target_building[1] = 1;
    world.agents.agents.next_replan_time[1] = world.agents.sim_time + 0.55;
    for _ in 0..5 {
        world.tick();
        assert_eq!(world.candidates(), 0, "a departure that is not due yet is no candidate");
    }
    world.tick();
    assert_eq!(world.candidates(), 1);
    assert_eq!(world.agents.parking_trip[1].phase, ParkingTripPhase::WalkToCar);
    world.tick();
    assert_eq!(world.candidates(), 0, "a leg under way is no candidate");
}

// Two residents leave in one tick for a destination with one bay. Their movement shards push in
// shard order: with 8 workers agent `LATE`'s chunk comes back to the first shard, so unsorted its
// note would come first and the car would go to the wrong resident.
#[test]
fn departures_commit_in_agent_order_on_any_worker_count() {
    const EARLY: usize = 300;
    const LATE: usize = 2090;
    let outcome = |workers: usize| {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(workers).build().unwrap();
        pool.install(|| {
            let mut world = World::new();
            world.agents.agents.planned_target_building.fill(usize::MAX);
            for _ in world.agents.len()..LATE + 10 {
                world.agents.spawn_housed_agent(0, 45.0, -12.0);
            }
            let home = world.allocator.building_handles.handle(0).to_bits();
            let (_, bay) = world.parking.source_bays(PlaceSource::Building { building: home, anchor: 0 })[0];
            let shifted = |x: f32| Bay {
                position: [bay.position[0] + x, bay.position[1], bay.position[2]],
                path: bay.path.map(|[px, pz]| [px + x, pz]),
                ..bay
            };
            world.parking.add_place(PlaceSource::Building { building: home, anchor: 1 }, shifted(3.0));
            world.parking.rebuild_candidates(&[0, 1], &world.allocator, &world.network, &world.graph, None);
            let second = world.parking.assign_household(1, ZoneDensity::Low, [45.0, -12.0]);
            world.parking.park_new_car(second, home);
            world.parking.seed_fixture_car(second, home);
            let first = world.car();
            for (agent, car) in [(EARLY, first), (LATE, second)] {
                world.agents.agents.parking_trip[agent].vehicle = car;
                world.agents.agents.planned_target_building[agent] = 1;
                world.agents.agents.planned_activity[agent] = 1;
            }
            world.tick();
            (
                world.agents.parking_trip[EARLY].phase,
                world.agents.parking_trip[LATE].phase,
                world.candidates(),
            )
        })
    };
    let serial = outcome(1);
    assert_eq!(serial, (ParkingTripPhase::WalkToCar, ParkingTripPhase::None, 2));
    assert_eq!(outcome(8), serial);
}

#[test]
fn a_retry_is_a_candidate_when_it_comes_due_and_not_before() {
    let mut world = World::new();
    world.agents.agents.planned_target_building[1] = usize::MAX;
    world.tick_until(ParkingTripPhase::PullOut);
    world.agents.agents.parking_trip[0].retry_pending = true;
    world.tick_until(ParkingTripPhase::Drive);
    world.tick();
    assert_eq!(world.candidates(), 0, "a pending retry is not polled");
    let car = world.car();
    assert!(world.parking.vehicle_place(car).is_some(), "the commit that releases the bay waits");
    world.release_retry(0);
    world.tick();
    assert_eq!(world.candidates(), 1);
    assert!(world.parking.vehicle_place(car).is_none());
}

#[test]
fn a_bay_a_replan_kept_from_releasing_is_released_on_the_next_tick() {
    let mut world = World::new();
    world.agents.agents.planned_target_building[1] = usize::MAX;
    world.tick_until(ParkingTripPhase::PullOut);
    world.agents.agents.parking_trip[0].retry_pending = true;
    world.tick_until(ParkingTripPhase::Drive);
    world.agents.agents.parking_trip[0].ready = true;
    world.agents.agents.parking_trip[0].replan_needed = true;
    world.release_retry(0);
    world.tick();
    let car = world.car();
    assert!(!world.agents.parking_trip[0].ready, "the replan ran");
    assert!(world.parking.vehicle_place(car).is_some(), "the replan goes before the release");
    world.tick();
    assert!(world.parking.vehicle_place(car).is_none());
}

#[test]
fn a_frozen_agent_is_a_candidate_until_its_commit_finishes() {
    let mut world = World::new();
    world.agents.agents.planned_target_building[1] = usize::MAX;
    world.tick_until(ParkingTripPhase::PullIn);
    // Another car takes the bay reserved for this one's arrival, so its commit cannot occupy.
    let bay = world.agents.parking_trip[0].destination;
    world.parking.cancel_reservation(world.car());
    let other = world.parking.assign_household(9, ZoneDensity::Low, [45.0, -12.0]);
    assert!(world.parking.occupy(other, bay));
    world.agents.agents.parking_trip[0].ready = true;
    for _ in 0..3 {
        world.tick();
        assert!(world.agents.parking_trip[0].retry_pending || world.candidates() == 1);
    }
}

#[test]
fn a_reservation_lost_on_the_way_flags_the_driver_at_the_next_tick() {
    let mut world = World::new();
    world.agents.agents.planned_target_building[1] = usize::MAX;
    world.tick_until(ParkingTripPhase::Drive);
    world.tick();
    world.parking.cancel_reservation(world.car());
    assert!(!world.agents.parking_trip[0].replan_needed);
    world.tick();
    assert_eq!(world.candidates(), 1);
}

#[test]
fn replaced_rows_are_rebuilt_at_the_next_tick() {
    let mut world = World::new();
    world.agents.agents.planned_target_building[1] = usize::MAX;
    world.tick_until(ParkingTripPhase::Drive);
    world.tick();
    // A row put back with no event told: the bay held again, the reservation gone, `has_car` old.
    let car = world.car();
    world.parking.occupy(car, world.agents.parking_trip[0].destination);
    world.parking.lost_reservations.clear();
    world.agents.agents.has_car[0] = true;
    world.agents.rebuild_parking_work();
    world.tick();
    assert!(!world.agents.has_car[0]);
    assert_eq!(world.candidates(), 1, "the driver's replan and its bay are found from the row");
}
