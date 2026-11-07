// SPDX-License-Identifier: GPL-2.0-only

//! The parking commit's prepared answers (`PARK-02`). The walk takes an answer only to the
//! question it has just formed from the live state, so the answers change no outcome at any
//! worker count. Debug test builds also ask the router again for every answer taken.

use super::parking::fixture;
use super::*;
use crate::simulation::core::handles::Handle;
use crate::simulation::economy::agents::tick::{AnswerCounts, take_answer_counts};
use crate::simulation::parking::stats::DepartureOutcome;
use crate::simulation::parking::{
    Bay, ParkingSystem, ParkingTrip, ParkingTripPhase, Place, PlaceSource,
};
use crate::simulation::zoning::profiles::ZoneDensity;

const HOME_X: f32 = 45.0;
const HOME_Z: f32 = -12.0;

struct Town {
    agents: AgentSystem,
    parking: ParkingSystem,
    allocator: BuildingAllocator,
    network: TransitNetwork,
    graph: RegionGraph,
}

impl Town {
    // The fixture without its residents' trips, with a free bay at home for each of `cars`.
    fn new(cars: usize) -> Self {
        let (mut agents, mut parking, allocator, network, graph) = fixture();
        agents.agents.planned_target_building.fill(usize::MAX);
        let home = allocator.building_handles.handle(0).to_bits();
        for n in 1..=cars {
            let bay = Self::bay_beside(&parking, home, 3.0 * n as f32);
            parking.add_place(
                PlaceSource::Building {
                    building: home,
                    anchor: n as u32,
                },
                bay,
            );
        }
        let mut town = Self {
            agents,
            parking,
            allocator,
            network,
            graph,
        };
        town.publish();
        town
    }

    fn bay_beside(parking: &ParkingSystem, building: u64, x: f32) -> Bay {
        let (_, bay) = parking.source_bays(PlaceSource::Building {
            building,
            anchor: 0,
        })[0];
        Bay {
            identity: [building, x.to_bits().into()],
            position: [bay.position[0] + x, bay.position[1], bay.position[2]],
            path: bay.path.map(|[px, pz]| [px + x, pz]),
            ..bay
        }
    }

    fn publish(&mut self) {
        let all: Vec<_> = (0..self.allocator.buildings.len()).collect();
        self.parking
            .rebuild_candidates(&all, &self.allocator, &self.network, &self.graph, None);
    }

    // A building along the road with one bay of its own, at `x`. Returns its index.
    fn add_door(&mut self, x: f32) -> usize {
        let mut building = self.allocator.buildings[1].clone();
        building.center_x = x;
        self.allocator.buildings.push(building);
        self.allocator
            .building_handles
            .register_appended(self.allocator.buildings.len());
        self.allocator
            .rebuild_entrance_cache(&self.graph, &self.network.lane_system);
        let door = self.allocator.buildings.len() - 1;
        let entrance = &self.allocator.entrances[door];
        let pos = entrance.door_pos + Vector2::new(5.0, 0.0);
        let curb = entrance.curb_pos;
        let building = self.allocator.building_handles.handle(door).to_bits();
        self.parking.add_place(
            PlaceSource::Building {
                building,
                anchor: 0,
            },
            Bay {
                identity: [building, 0],
                position: [pos.x, 0.0, pos.y],
                path: [
                    [curb.x, curb.y],
                    [pos.x, curb.y],
                    [pos.x, pos.y + 3.0],
                    [pos.x, pos.y],
                ],
                heading: 0.0,
                width: 2.5,
                length: 5.0,
            },
        );
        door
    }

    // Another bay at `door`, further along than those before it.
    fn add_bay(&mut self, door: usize, anchor: u32) -> Handle<Place> {
        let building = self.allocator.building_handles.handle(door).to_bits();
        let bay = Self::bay_beside(&self.parking, building, 3.0 * anchor as f32);
        self.parking
            .add_place(PlaceSource::Building { building, anchor }, bay)
    }

    // A resident with a car of its own parked at home, due to leave for `target`.
    fn add_driver(&mut self, target: usize) -> usize {
        let home = self.allocator.building_handles.handle(0).to_bits();
        let agent = self.agents.spawn_housed_agent(0, HOME_X, HOME_Z);
        let car =
            self.parking
                .assign_household(agent as u64 + 1, ZoneDensity::Low, [HOME_X, HOME_Z]);
        self.parking.park_new_car(car, home);
        self.parking.seed_fixture_car(car, home);
        assert!(self.parking.vehicle_place(car).is_some());
        self.agents.agents.parking_trip[agent].vehicle = car;
        self.agents.agents.planned_target_building[agent] = target;
        self.agents.agents.planned_activity[agent] = 1;
        agent
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

    // Every field the commit writes on a row, exactly.
    fn rows(&self) -> Vec<String> {
        let a = &self.agents.agents;
        (0..a.len())
            .map(|i| {
                format!(
                    "{:?} {:?} {} {} {} {} {} {} {:x} {:x} {:x} {:x}",
                    a.parking_trip[i],
                    a.current_path[i],
                    a.transit[i],
                    a.transit_mode[i],
                    a.access_flags[i],
                    a.planned_attach_node[i],
                    a.planned_detach_node[i],
                    a.planned_attach_lane_id[i],
                    a.planned_detach_lane_id[i],
                    a.planned_attach_lane_d[i].to_bits(),
                    a.planned_detach_lane_d[i].to_bits(),
                    a.pos_x[i].to_bits(),
                )
            })
            .collect()
    }

    fn destination(&self, agent: usize) -> Handle<Place> {
        self.agents.parking_trip[agent].destination
    }
}

// Runs `play` on `workers` workers with the movement shards the parallel pass needs, its answers
// on or off, and returns the rows and how the commits were answered.
fn played(
    workers: usize,
    answers: bool,
    build: impl Fn() -> Town + Send + Sync,
    play: impl Fn(&mut Town) + Send + Sync,
) -> (Vec<String>, AnswerCounts) {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap();
    pool.install(|| {
        let mut town = build();
        town.agents.parking_work.answers_off = !answers;
        town.agents
            .prepare_movement_scratch(&town.network, &town.graph);
        take_answer_counts();
        play(&mut town);
        (town.rows(), take_answer_counts())
    })
}

// Twelve residents leave at once, each for a door of its own: no visit touches another's bays,
// so each must take its own answers, whichever worker found them. A commit reading the answers
// out of position would ask the router here, and the counts would show it.
#[test]
fn each_visit_takes_its_own_answers_on_any_worker_count() {
    const RESIDENTS: usize = 12;
    let build = || {
        let mut town = Town::new(RESIDENTS);
        let doors: Vec<_> = (0..RESIDENTS)
            .map(|n| town.add_door(150.0 + 25.0 * n as f32))
            .collect();
        town.publish();
        for door in doors {
            town.add_driver(door);
        }
        town
    };
    let (reference, none) = played(1, false, build, Town::tick);
    assert_eq!(none, AnswerCounts::default());
    let expected = AnswerCounts {
        legs: RESIDENTS,
        trips: RESIDENTS,
        routes: RESIDENTS,
        requoted_trips: 0,
    };
    for workers in [1, 3, 8] {
        let (rows, counts) = played(workers, true, build, Town::tick);
        assert_eq!(rows, reference, "{workers} workers");
        assert_eq!(counts, expected, "{workers} workers");
    }
}

// Twelve drivers want the same first free bay: the pass quotes them all to it from the phase's
// start, and each visit after the first finds it taken, moves to the next and quotes again.
#[test]
fn a_bay_taken_earlier_in_the_tick_sends_the_next_trip_to_the_next_bay() {
    const RESIDENTS: usize = 12;
    let build = || {
        let mut town = Town::new(RESIDENTS);
        let bays: Vec<_> = (1..RESIDENTS as u32)
            .map(|anchor| town.add_bay(1, anchor))
            .collect();
        assert_eq!(bays.len(), RESIDENTS - 1);
        town.publish();
        for _ in 0..RESIDENTS {
            town.add_driver(1);
        }
        town
    };
    let (reference, _) = played(1, false, build, Town::tick);
    for workers in [1, 3, 8] {
        let (rows, counts) = played(workers, true, build, Town::tick);
        assert_eq!(rows, reference, "{workers} workers");
        assert_eq!(counts.trips, 1, "only the first visit's bay is still free");
        assert_eq!(counts.requoted_trips, RESIDENTS - 1);
    }
}

// A driver's visit frees the nearest bay at the door a later resident is leaving for. At the
// phase's start that bay was taken, so the pass quoted the bay after it.
#[test]
fn a_bay_freed_earlier_in_the_tick_is_the_next_trips_bay() {
    let mut town = Town::new(2);
    let further = town.add_bay(1, 1);
    town.publish();
    let leaving = town.add_driver(usize::MAX);
    let car = town.agents.parking_trip[leaving].vehicle;
    let nearest = town
        .parking
        .first_free_candidate(town.allocator.building_handles.handle(1).to_bits())
        .unwrap()
        .place;
    assert_ne!(nearest, further);
    let at_home = town.parking.vehicle_place(car).unwrap();
    town.parking.release(car);
    assert!(town.parking.occupy(car, nearest));
    assert!(town.parking.reserve(car, at_home));
    town.agents.agents.parking_trip[leaving] = ParkingTrip {
        vehicle: car,
        phase: ParkingTripPhase::Drive,
        destination: at_home,
        ..Default::default()
    };
    let late = town.add_driver(1);
    assert!(leaving < late);
    take_answer_counts();
    town.tick();
    assert_eq!(town.destination(late), nearest);
    assert_eq!(take_answer_counts().requoted_trips, 1);
}

// Two members of a household share one car and leave together: the first borrows it, and the
// second, quoted a trip from the phase's start, must find it gone and walk.
#[test]
fn the_second_member_to_leave_finds_the_car_gone() {
    let (agents, parking, allocator, network, graph) = fixture();
    let mut town = Town {
        agents,
        parking,
        allocator,
        network,
        graph,
    };
    take_answer_counts();
    town.tick();
    assert_eq!(
        town.agents.parking_trip[0].phase,
        ParkingTripPhase::WalkToCar
    );
    assert_eq!(town.agents.parking_trip[1].phase, ParkingTripPhase::None);
    assert_eq!(town.parking.departures.count(DepartureOutcome::Drove), 1);
    assert_eq!(town.parking.departures.count(DepartureOutcome::CarInUse), 1);
    assert_eq!(
        town.parking
            .departures
            .count(DepartureOutcome::ReservationLost),
        0
    );
    assert_eq!(take_answer_counts().trips, 1);
}

// Whole trips, every transition and arrival, the same with the answers as without.
#[test]
fn whole_trips_are_the_same_with_answers_on_any_worker_count() {
    let build = || {
        let mut town = Town::new(9);
        for anchor in 1..9 {
            town.add_bay(1, anchor);
        }
        town.publish();
        for _ in 0..9 {
            town.add_driver(1);
        }
        town
    };
    let play = |town: &mut Town| {
        for _ in 0..4000 {
            town.tick();
        }
    };
    let (reference, _) = played(1, false, build, play);
    let (rows, counts) = played(3, true, build, play);
    assert_eq!(rows, reference);
    assert!(counts.legs > 18, "{counts:?}");
    assert!(counts.routes > 18, "{counts:?}");
}
