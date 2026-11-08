// SPDX-License-Identifier: GPL-2.0-only

//! A driver whose trip is taken from it on the way (`TRAFFIC-29`): each case starts from the real
//! event, a job ending or a building going, with the car on the road.

use super::*;
use crate::simulation::core::handles::Handle;
use crate::simulation::economy::agents::{
    ACCESS_PLAN_VALID, ACTIVITY_WORK, MODE_CAR, MODE_WALK, TRANSIT_ACCESS_EGRESS,
    TRANSIT_ACCESS_INGRESS, TRANSIT_IN_BUILDING,
};
use crate::simulation::parking::{ParkingTripPhase, Vehicle};

// A home and a workplace at the two ends of a road, each a lot of its own, and a commuter with its
// car. The road continues past both, so a car that overshoots has somewhere to go on to.
struct Commute {
    c: SimCore,
    agent: usize,
    home: usize,
    work: usize,
}

impl Commute {
    fn new() -> Self {
        let mut c = SimCore::new(
            WorldConfig::new(2048.0, 1024.0, 16.0, 10.0)
                .with_chunking(128.0, 0.0)
                .with_terrain_resolution(2.0),
        );
        c.treasury.balance = 1_000_000.0;
        commit(&mut c, (-600.0, 0.0), (600.0, 0.0));
        let id = register_lot(&mut c);
        let mut manifest = c.allocator.registry.get(&id).unwrap().manifest.clone();
        manifest.building.as_mut().unwrap().household_capacity = Some(64);
        c.allocator
            .registry
            .register("parking", manifest, String::new());
        c.place_service_building_internal(&id, -450.0, -30.0)
            .unwrap();
        c.place_service_building_internal(&id, 450.0, -30.0)
            .unwrap();
        c.publish_pending_building_site_changes();
        let (home, work) = if c.allocator.entrances[0].side == -1 {
            (0, 1)
        } else {
            (1, 0)
        };
        let agent = c.debug_add_parking_commuter(home, work).unwrap();
        c.agents.assign_work_building(agent, work, 0);
        c.agents.release_departure(agent);
        c.agents.agents.planned_activity[agent] = ACTIVITY_WORK;
        Self {
            c,
            agent,
            home,
            work,
        }
    }

    fn tick(&mut self) {
        let c = &mut self.c;
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
        // A retry waits for its slot; ticks alone do not move the wheel.
        if c.agents.parking_trip[self.agent].retry_pending {
            c.run_slots_through_for_test(c.time.absolute_slot() + 1);
        }
    }

    // Ticks until `done`, at most 40 sim minutes.
    fn tick_until(&mut self, what: &str, done: impl Fn(&SimCore, usize) -> bool) {
        for _ in 0..24_000 {
            if done(&self.c, self.agent) {
                return;
            }
            self.tick();
        }
        let a = &self.c.agents;
        let i = self.agent;
        panic!(
            "never {what}: trip {:?} transit {} mode {} lane {} d {} target {} building {}",
            a.parking_trip[i],
            a.transit[i],
            a.transit_mode[i],
            a.current_lane_id[i],
            a.lane_distance[i],
            a.target_building[i],
            a.current_building[i],
        );
    }

    fn driving(&mut self) {
        self.tick_until("drove", |c, i| {
            c.agents.parking_trip[i].phase == ParkingTripPhase::Drive
                && c.agents.current_lane_id[i] != usize::MAX
        });
    }

    fn on_the_road(&mut self) {
        self.driving();
        assert_eq!(self.c.agents.target_building[self.agent], self.work);
        assert_eq!(self.c.agents.transit_mode[self.agent], MODE_CAR);
    }

    fn inside(&mut self, building: usize) {
        self.tick_until("arrived", |c, i| {
            c.agents.transit[i] == TRANSIT_IN_BUILDING
                && c.agents.parking_trip[i].phase == ParkingTripPhase::None
                && !c.agents.parking_trip[i].ready
        });
        let a = &self.c.agents;
        assert_eq!(a.current_building[self.agent], building);
    }

    fn car(&self) -> Handle<Vehicle> {
        self.c.agents.parking_trip[self.agent].vehicle
    }

    fn building_handle(&self, building: usize) -> u64 {
        self.c.allocator.building_handles.handle(building).to_bits()
    }

    fn end_job(&mut self) {
        self.c.agents.assign_work_building(self.agent, usize::MAX, 0);
    }

    fn turned_for(&mut self, building: usize) {
        self.tick_until("turned", |c, i| {
            c.agents.target_building[i] == building
                && c.agents.access_flags[i] & ACCESS_PLAN_VALID != 0
                && !c.agents.parking_trip[i].ready
        });
        let handle = self.building_handle(building);
        assert_eq!(self.c.agents.parking_trip[self.agent].target, handle);
    }

    // The driver's household loses its home while it is out.
    fn lose_home(&mut self) {
        self.c.agents.evict_household_member_home(self.agent, self.home);
    }

    // Takes every free bay at home with the parked cars of new neighbours.
    fn fill_home_bays(&mut self) -> Vec<Handle<Vehicle>> {
        let handle = self.building_handle(self.home);
        let mut cars = Vec::new();
        while self.c.parking.first_free_candidate(handle).is_some() {
            let neighbour = self.c.debug_add_parking_commuter(self.home, self.work).unwrap();
            cars.push(self.c.agents.parking_trip[neighbour].vehicle);
        }
        cars
    }

    // The agent's whole row as a load must leave it.
    fn row(&self) -> impl PartialEq + std::fmt::Debug + use<> {
        let (a, i) = (&self.c.agents, self.agent);
        (
            (a.transit[i], a.transit_mode[i], a.current_lane_id[i], a.current_node[i]),
            (a.target_building[i], a.current_path_index[i], a.current_path[i].clone()),
            (
                a.lane_distance[i].to_bits(),
                a.speed[i].to_bits(),
                a.next_replan_time[i].to_bits(),
            ),
            serde_json::to_value(a.parking_trip[i]).unwrap(),
        )
    }

    // Saves here, records `ticks` ticks of the running game, loads and asserts the loaded game
    // gives the same rows.
    fn continues_exactly_across_a_save(&mut self, name: &str, ticks: usize) {
        let before = self.row();
        let path = temp_save_path(name);
        self.c
            .save_game_settled_at(path.to_str().unwrap(), None, 0)
            .unwrap();
        let mut straight = Vec::with_capacity(ticks);
        for _ in 0..ticks {
            self.tick();
            straight.push(self.row());
        }
        self.c.load_game_internal(path.to_str().unwrap()).unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(self.row() == before, "a load leaves the row as it stands");
        for (tick, row) in straight.iter().enumerate() {
            self.tick();
            assert_eq!(&self.row(), row, "tick {tick} after the load");
        }
    }
}

// The row's own case: the job ends with the car on the road, the car runs to its lane end, turns
// for home from there with the lane it has, parks at home and the driver walks in.
#[test]
fn a_driver_whose_job_ends_on_the_way_drives_home_and_parks() {
    let mut t = Commute::new();
    t.on_the_road();
    t.end_job();
    let (home, agent) = (t.home, t.agent);
    assert_eq!(t.c.agents.target_building[agent], usize::MAX);
    t.turned_for(home);
    assert_ne!(t.c.agents.current_lane_id[agent], usize::MAX, "it keeps its lane");
    assert!(
        t.c.parking.trip_reserved(t.car(), t.c.agents.parking_trip[agent].destination),
        "with a bay at home"
    );
    t.inside(home);
    assert_eq!(t.c.agents.transit_mode[agent], MODE_WALK);
    assert!(t.c.parking.vehicle_place(t.car()).is_some(), "it parked");
    assert_eq!(t.c.parking.borrowed_by(t.car()), None, "and gave the car back");
    assert!(t.c.parking.available_to(t.car(), t.c.agents.agent_handle(agent)));
}

// Saved in the frames where the car stands at its lane end asking for its recovery, and again on
// its way home.
#[test]
fn a_driver_turning_for_home_continues_exactly_across_a_save() {
    let mut t = Commute::new();
    t.on_the_road();
    t.end_job();
    t.tick_until("reached its lane end", |c, i| c.agents.parking_trip[i].ready);
    assert_ne!(t.c.agents.current_lane_id[t.agent], usize::MAX);
    t.continues_exactly_across_a_save("taken-trip-at-lane-end", 300);
    let home = t.home;
    t.turned_for(home);
    t.continues_exactly_across_a_save("taken-trip-on-the-way-home", 300);
    t.inside(home);
}

// The building and its bay go after the job, with the car still on the lane it was on and again
// with the car already turned for home.
#[test]
fn a_driver_whose_workplace_goes_after_the_job_drives_home() {
    let mut t = Commute::new();
    t.on_the_road();
    t.end_job();
    assert!(t.c.bulldoze_building(t.work));
    let home = t.c.agents.home_building[t.agent];
    t.turned_for(home);
    t.inside(home);
    assert!(t.c.parking.vehicle_place(t.car()).is_some());
    assert_eq!(t.c.parking.borrowed_by(t.car()), None);
}

// With the job still running: the demolition points the driver home and takes its bay. Before,
// recovery found its target dead and the car stood on the lane for good.
#[test]
fn a_commuter_whose_workplace_goes_on_the_way_drives_home() {
    let mut t = Commute::new();
    t.on_the_road();
    assert!(t.c.bulldoze_building(t.work));
    let home = t.c.agents.home_building[t.agent];
    assert_eq!(t.c.agents.target_building[t.agent], home);
    t.turned_for(home);
    t.inside(home);
    assert!(t.c.parking.vehicle_place(t.car()).is_some());
}

// No free bay at home, the booked one at the old job still held: it parks there and walks home.
#[test]
fn a_driver_with_no_bay_at_home_parks_where_it_booked_and_walks() {
    let mut t = Commute::new();
    t.on_the_road();
    let booked = t.c.agents.parking_trip[t.agent].destination;
    t.fill_home_bays();
    t.end_job();
    let home = t.home;
    t.turned_for(home);
    assert_eq!(t.c.agents.parking_trip[t.agent].destination, booked);
    t.tick_until("walked", |c, i| {
        c.agents.parking_trip[i].phase == ParkingTripPhase::WalkToDoor
    });
    assert_eq!(t.c.parking.vehicle_place(t.car()), Some(booked));
    assert_eq!(t.c.agents.transit_mode[t.agent], MODE_WALK);
    t.inside(home);
}

// No bay anywhere it can go: the car keeps its lane and waits on the retry, which a load leaves
// as it stands in the very slot the retry fires (`PARK-08`), and a bay coming free sends it home.
#[test]
fn a_car_with_no_bay_left_waits_on_its_lane_and_loads_as_it_stands() {
    let mut t = Commute::new();
    t.on_the_road();
    let cars = t.fill_home_bays();
    t.end_job();
    assert!(t.c.bulldoze_building(t.work));
    t.tick_until("asked for a retry", |c, i| c.agents.parking_trip[i].retry_pending);
    let agent = t.agent;
    assert_ne!(t.c.agents.current_lane_id[agent], usize::MAX, "it keeps its lane");
    let home = t.c.agents.home_building[agent];
    assert_eq!(t.c.agents.target_building[agent], home);
    while t.c.agents.parking_trip[agent].retry_pending {
        let next = t.c.time.absolute_slot() + 1;
        t.c.run_slots_through_for_test(next);
    }
    assert_eq!(
        t.c.agents.next_replan_time[agent].to_bits(),
        0,
        "the retry has fired: no retry time left on the row"
    );
    t.continues_exactly_across_a_save("taken-trip-no-bay", 50);
    t.c.parking.release(cars[0]);
    for _ in 0..4 {
        // Each retry is 30 game minutes of slots.
        while t.c.agents.parking_trip[agent].retry_pending {
            let next = t.c.time.absolute_slot() + 1;
            t.c.run_slots_through_for_test(next);
        }
        t.tick();
    }
    t.turned_for(home);
    t.inside(home);
}

// A driver with no home goes on to where it was going, which still stands: it parks in the bay it
// booked and goes in, and waits inside for its household's next home.
#[test]
fn a_driver_with_no_home_parks_where_it_was_going_and_goes_in() {
    let mut t = Commute::new();
    t.on_the_road();
    t.lose_home();
    t.end_job();
    let work = t.work;
    t.turned_for(work);
    t.inside(work);
    assert!(t.c.parking.vehicle_place(t.car()).is_some());
}

// A driver evicted on its way home, its trip home cancelled, parks at the old home and goes in.
#[test]
fn a_driver_evicted_on_its_way_home_parks_there_and_goes_in() {
    let mut t = Commute::new();
    let (home, work) = (t.home, t.work);
    t.on_the_road();
    t.inside(work);
    assert!(t.c.agents.request_parking_fixture_trip(t.agent, home));
    t.c.agents.release_departure(t.agent);
    t.driving();
    assert_eq!(t.c.agents.target_building[t.agent], home);
    t.lose_home();
    assert_eq!(t.c.agents.target_building[t.agent], usize::MAX);
    t.turned_for(home);
    t.inside(home);
    assert_eq!(t.c.agents.home_building[t.agent], usize::MAX);
    assert!(t.c.parking.vehicle_place(t.car()).is_some());
}

// A household moving on a driver's pull-in: the leg ends in its bay, the driver goes in at the old
// door and leaves for the new home from there.
#[test]
fn a_move_during_a_pull_in_waits_for_the_door() {
    let mut t = Commute::new();
    let (home, work) = (t.home, t.work);
    t.on_the_road();
    t.inside(work);
    assert!(t.c.agents.request_parking_fixture_trip(t.agent, home));
    t.c.agents.release_departure(t.agent);
    t.tick_until("pulled in", |c, i| {
        c.agents.parking_trip[i].phase == ParkingTripPhase::PullIn
            && c.agents.transit[i] == TRANSIT_ACCESS_INGRESS
    });
    t.c.agents.relocate_household_member_home(t.agent, home, work, true);
    assert_eq!(t.c.agents.target_building[t.agent], home);
    assert_eq!(t.c.agents.planned_target_building[t.agent], work);
    t.inside(home);
    t.tick_until("left for the new home", |c, i| c.agents.target_building[i] == work);
    t.inside(work);
}

// No destination, no home and nothing left of where it was going: the car goes off the map, the
// driver settles for its household's next home, and rehousing takes it there on foot.
#[test]
fn a_driver_with_nothing_left_leaves_the_car_and_settles() {
    let mut t = Commute::new();
    t.on_the_road();
    t.lose_home();
    t.end_job();
    let home = t.building_handle(t.home);
    assert!(t.c.bulldoze_building(t.work));
    t.tick_until("settled", |c, i| {
        c.agents.parking_trip[i].phase == ParkingTripPhase::None
            && c.agents.transit[i] == TRANSIT_IN_BUILDING
    });
    let agent = t.agent;
    let home = t.c.allocator.building_handles.resolve(Handle::from_bits(home)).unwrap();
    let a = &t.c.agents;
    assert_eq!(a.current_building[agent], usize::MAX);
    assert_eq!(a.current_lane_id[agent], usize::MAX);
    assert_eq!(a.transit_mode[agent], MODE_WALK);
    assert_eq!(t.c.parking.borrowed_by(t.car()), None);
    assert_eq!(t.c.parking.vehicle_place(t.car()), None);
    assert!(t.c.parking.stored_car_home(t.car()).is_some(), "the car waits off the map");
    t.continues_exactly_across_a_save("taken-trip-settled", 20);
    t.c.agents.relocate_household_member_home(agent, usize::MAX, home, false);
    t.inside(home);
}

// On an access path the leg finishes: a pull-in ends in its own bay, a walk at its own door, and
// the ordinary schedule then sends the driver home.
#[test]
fn a_job_ending_on_an_access_leg_lets_the_leg_finish() {
    for (phase, transit) in [
        (ParkingTripPhase::WalkToCar, TRANSIT_ACCESS_EGRESS),
        (ParkingTripPhase::PullOut, TRANSIT_ACCESS_EGRESS),
        (ParkingTripPhase::PullIn, TRANSIT_ACCESS_INGRESS),
        (ParkingTripPhase::WalkToDoor, TRANSIT_ACCESS_INGRESS),
    ] {
        let mut t = Commute::new();
        t.tick_until("reached the leg", |c, i| {
            c.agents.parking_trip[i].phase == phase && c.agents.transit[i] == transit
        });
        t.end_job();
        assert_eq!(t.c.agents.target_building[t.agent], t.work, "{phase:?}");
        let work = t.work;
        t.inside(work);
        assert!(t.c.parking.vehicle_place(t.car()).is_some(), "{phase:?}");
        let home = t.home;
        t.c.agents.agents.activity[t.agent] = ACTIVITY_WORK;
        t.tick_until("left for home", |c, i| c.agents.target_building[i] == home);
        t.inside(home);
        assert_eq!(t.c.parking.borrowed_by(t.car()), None, "{phase:?}");
    }
}

// A road edit under a car pulling into its bay drops the car's access plan (`PARK-11`): the
// pull-in still ends in its bay, never at the door, and the driver walks in from there.
#[test]
fn a_road_edit_under_a_pull_in_ends_it_in_its_bay() {
    let mut t = Commute::new();
    t.tick_until("pulled in", |c, i| {
        c.agents.parking_trip[i].phase == ParkingTripPhase::PullIn
            && c.agents.transit[i] == TRANSIT_ACCESS_INGRESS
    });
    // A side road joins the commute's road before the workplace and splits the edge the car
    // left for its bay.
    commit(&mut t.c, (300.0, 0.0), (300.0, 300.0));
    let agent = t.agent;
    assert_eq!(
        t.c.agents.access_flags[agent] & ACCESS_PLAN_VALID,
        0,
        "the edit drops the plan"
    );
    assert_eq!(t.c.agents.transit[agent], TRANSIT_ACCESS_INGRESS);
    t.tick_until("walked from the bay", |c, i| {
        assert!(
            c.agents.transit[i] != TRANSIT_IN_BUILDING
                || c.agents.parking_trip[i].phase != ParkingTripPhase::PullIn,
            "the driver went in at the door with its car on the driveway"
        );
        c.agents.parking_trip[i].phase == ParkingTripPhase::WalkToDoor
    });
    let work = t.work;
    t.inside(work);
    assert_eq!(t.c.agents.transit_mode[agent], MODE_WALK);
    assert!(t.c.parking.vehicle_place(t.car()).is_some(), "it parked");
}
