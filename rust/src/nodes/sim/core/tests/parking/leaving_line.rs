// SPDX-License-Identifier: GPL-3.0-or-later

//! `PARK-32` leaving line: cars leaving one lot together wait one behind another on its way out,
//! one at a time at the mouth, while an eastbound stream passes the lot.

use super::*;
use crate::config::CAR_BODY_LENGTH_M;
use crate::simulation::economy::agents::{
    ACCESS_LANE_ATTACH_PENDING, ACTIVITY_WORK, MODE_CAR, TRANSIT_ACCESS_EGRESS,
    TRANSIT_IN_BUILDING, TRANSIT_NETWORK,
};
use crate::simulation::parking::ParkingTripPhase;

// Lots on a street from x -300 to 400: `home` north at x 250, whose cars leave west across the
// eastbound lane the stream from `source` (north, x -150) to `east` (north, x 350) drives; `south`
// at x 150, whose cars leave west on their near lane, the westbound one, which the stream from
// `south_source` (x 350) to `west` (south, x -250) drives.
struct Street {
    c: SimCore,
    home: usize,
    west: usize,
    east: usize,
    source: usize,
    south: usize,
    south_source: usize,
}

// Ticks the agents and the parking retries as `crossing_stills` does.
fn tick(c: &mut SimCore) {
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
}

fn street() -> Street {
    let mut c = SimCore::new(
        WorldConfig::new(1024.0, 1024.0, 16.0, 10.0)
            .with_chunking(128.0, 0.0)
            .with_terrain_resolution(2.0),
    );
    c.treasury.balance = 10_000_000.0;
    commit(&mut c, (-300.0, 0.0), (400.0, 0.0));
    let id = register_lot(&mut c);
    let mut manifest = c.allocator.registry.get(&id).unwrap().manifest.clone();
    manifest.building.as_mut().unwrap().household_capacity = Some(64);
    c.allocator
        .registry
        .register("parking", manifest, String::new());
    let mut lot = |x: f32, z: f32| c.place_service_building_internal(&id, x, z).unwrap();
    let (source, home) = (lot(-150.0, 30.0), lot(250.0, 30.0));
    let (west, east) = (lot(-250.0, -30.0), lot(350.0, 30.0));
    let (south, south_source) = (lot(150.0, -30.0), lot(350.0, -30.0));
    c.publish_pending_building_site_changes();
    Street {
        c,
        home,
        west,
        east,
        source,
        south,
        south_source,
    }
}

impl Street {
    // `count` commuters from `home` to `work`, held until released.
    fn commuters(&mut self, home: usize, work: usize, count: usize) -> Vec<usize> {
        (0..count)
            .filter_map(|_| {
                let agent = self.c.debug_add_parking_commuter(home, work)?;
                self.c.agents.assign_work_building(agent, work, 0);
                self.c.agents.agents.planned_activity[agent] = ACTIVITY_WORK;
                Some(agent)
            })
            .collect()
    }

    fn release(&mut self, agents: &[usize]) {
        for &agent in agents {
            self.c.agents.release_departure(agent);
        }
    }
}

// Off the road and out of its place: pulling out and moved from where it stood, or held at its
// lane point after its pull-out.
fn under_way(c: &SimCore, i: usize, start: Option<(f32, f32)>) -> bool {
    let a = &c.agents.agents;
    let trip = a.parking_trip[i];
    let pulling_out = a.transit[i] == TRANSIT_ACCESS_EGRESS
        && trip.phase == ParkingTripPhase::PullOut
        && start.is_some_and(|s| (s.0 - a.pos_x[i]).hypot(s.1 - a.pos_y[i]) > 0.01);
    let held = a.transit[i] == TRANSIT_NETWORK
        && a.current_lane_id[i] == usize::MAX
        && a.access_flags[i] & ACCESS_LANE_ATTACH_PENDING != 0;
    pulling_out || held
}

// What a leaving run saw: the order the cars left their places and reached their lanes, the most
// under way at once, the longest wait at the mouth.
#[derive(Debug, Default)]
struct Seen {
    left_place: Vec<usize>,
    on_lane: Vec<usize>,
    most_under_way: usize,
    under_way_now: usize,
    longest_mouth_wait: f32,
}

// The stream's cars: each lot has 12 places, and those left free near `west` and `east` are where
// the leaving cars park.
const STREAM: usize = 8;

// Runs the street with the stream released one every 2.5 s and `leaving` together `leave_at`
// ticks in,
// asserting at every tick that no two leaving cars under way stand within a body length.
// `check` is called each tick and may stop the run.
fn run(
    s: &mut Street,
    leaving: &[usize],
    stream: &[usize],
    leave_at: usize,
    ticks: usize,
    mut check: impl FnMut(&mut SimCore, &Seen) -> bool,
) -> Seen {
    let mut seen = Seen::default();
    let mut starts: Vec<Option<(f32, f32)>> = vec![None; leaving.len()];
    for step in 0..ticks {
        if step % 25 == 0
            && let Some(&agent) = stream.get(step / 25)
        {
            s.release(&[agent]);
        }
        if step == leave_at {
            s.release(leaving);
        }
        tick(&mut s.c);
        let c = &s.c;
        let a = &c.agents.agents;
        let mut moving = Vec::new();
        for (k, &i) in leaving.iter().enumerate() {
            let pulling_out = a.transit[i] == TRANSIT_ACCESS_EGRESS
                && a.parking_trip[i].phase == ParkingTripPhase::PullOut;
            if pulling_out && starts[k].is_none() {
                starts[k] = Some((a.pos_x[i], a.pos_y[i]));
            }
            if under_way(c, i, starts[k]) {
                if !seen.left_place.contains(&i) {
                    seen.left_place.push(i);
                }
                moving.push(i);
            }
            if a.transit[i] == TRANSIT_NETWORK
                && a.transit_mode[i] == MODE_CAR
                && a.current_lane_id[i] != usize::MAX
                && !seen.on_lane.contains(&i)
            {
                seen.on_lane.push(i);
            }
            if a.transit[i] == TRANSIT_ACCESS_EGRESS {
                seen.longest_mouth_wait = seen.longest_mouth_wait.max(a.crossing_wait_s[i]);
            }
        }
        seen.most_under_way = seen.most_under_way.max(moving.len());
        seen.under_way_now = moving.len();
        for (k, &i) in moving.iter().enumerate() {
            for &j in &moving[k + 1..] {
                let d = (a.pos_x[i] - a.pos_x[j]).hypot(a.pos_y[i] - a.pos_y[j]);
                assert!(
                    d >= CAR_BODY_LENGTH_M,
                    "step {step}: cars {i} and {j} under way {d:.2} m apart"
                );
            }
        }
        if check(&mut s.c, &seen) {
            break;
        }
    }
    seen
}

// Each car reached its lane in the order it left its place: nobody passed anybody in the line.
fn served_in_order(seen: &Seen, leaving: &[usize]) {
    assert_eq!(
        seen.on_lane.len(),
        leaving.len(),
        "every car reached its lane: {seen:?}"
    );
    assert_eq!(
        seen.on_lane, seen.left_place,
        "served in the order they left: {seen:?}"
    );
    assert!(seen.most_under_way >= 2, "a line formed: {seen:?}");
}

#[test]
fn cars_leaving_across_the_road_wait_one_behind_another() {
    let mut s = street();
    let leaving = s.commuters(s.home, s.west, 3);
    let stream = s.commuters(s.source, s.east, STREAM);
    assert_eq!((leaving.len(), stream.len()), (3, STREAM));
    let seen = run(&mut s, &leaving, &stream, 150, 12_000, |_, seen| {
        seen.on_lane.len() == 3
    });
    served_in_order(&seen, &leaving);
    assert!(
        seen.longest_mouth_wait > 0.0,
        "the head waited at the mouth: {seen:?}"
    );
}

#[test]
fn cars_leaving_to_the_near_lane_wait_one_behind_another() {
    let mut s = street();
    let leaving = s.commuters(s.south, s.west, 3);
    let stream = s.commuters(s.south_source, s.west, STREAM);
    let seen = run(&mut s, &leaving, &stream, 150, 12_000, |_, seen| {
        seen.on_lane.len() == 3
    });
    served_in_order(&seen, &leaving);
}

// A line saved while it waits loads as it stood and goes on exactly as the running game.
#[test]
fn a_leaving_line_continues_exactly_across_a_save() {
    let mut s = street();
    let leaving = s.commuters(s.home, s.west, 3);
    let stream = s.commuters(s.source, s.east, STREAM);
    let mut saved = false;
    run(&mut s, &leaving, &stream, 150, 12_000, |c, seen| {
        if seen.under_way_now < 2 || leaving.iter().all(|&i| c.agents.crossing_wait_s[i] <= 0.0) {
            return false;
        }
        let rows = |c: &SimCore| {
            leaving
                .iter()
                .chain(&stream)
                .map(|&i| {
                    let a = &c.agents.agents;
                    (
                        a.transit[i],
                        a.current_lane_id[i],
                        a.pos_x[i].to_bits(),
                        a.pos_y[i].to_bits(),
                        a.lane_distance[i].to_bits(),
                        a.crossing_wait_s[i].to_bits(),
                    )
                })
                .collect::<Vec<_>>()
        };
        let path = temp_save_path("park32_leaving_line");
        c.save_game_settled_at(path.to_str().unwrap(), None, 0)
            .unwrap();
        let mut straight = Vec::new();
        for _ in 0..300 {
            tick(c);
            straight.push(rows(c));
        }
        c.load_game_internal(path.to_str().unwrap()).unwrap();
        std::fs::remove_file(path).unwrap();
        for (k, row) in straight.iter().enumerate() {
            tick(c);
            assert!(&rows(c) == row, "tick {k} after the load");
        }
        saved = true;
        true
    });
    assert!(saved, "the line formed and was saved");
}

// A car coming home to the lot while its leaving line waits drives in and parks: the line never
// holds an entering car.
#[test]
fn an_entering_car_is_not_held_by_the_leaving_line() {
    let mut s = street();
    let leaving = s.commuters(s.home, s.west, 3);
    let stream = s.commuters(s.source, s.east, STREAM);
    let coming = s.commuters(s.source, s.home, 2);
    s.release(&coming);
    let mut met = false;
    run(&mut s, &leaving, &stream, 330, 12_000, |c, seen| {
        let a = &c.agents.agents;
        met |= seen.under_way_now >= 2
            && coming
                .iter()
                .any(|&i| a.parking_trip[i].phase == ParkingTripPhase::PullIn);
        coming.iter().all(|&i| a.transit[i] == TRANSIT_IN_BUILDING) && seen.on_lane.len() == 3
    });
    let a = &s.c.agents.agents;
    assert!(
        coming.iter().all(|&i| a.transit[i] == TRANSIT_IN_BUILDING),
        "the entering cars parked and walked in"
    );
    assert!(met, "an entering car pulled in while a line stood");
}

// A car that has waited its give-up time at the mouth to cross leaves on the near lane, the one
// it would have crossed, with a plan from that lane's end, and gets to its lot all the same
// (`PARK-32`). The first car to wait has its wait brought to just short of the give-up.
#[test]
fn a_car_waiting_too_long_to_cross_leaves_on_the_near_lane() {
    use crate::simulation::economy::agents::tick::CROSSING_GIVE_UP_S;
    let mut s = street();
    let leaving = s.commuters(s.home, s.west, 3);
    let stream = s.commuters(s.source, s.east, STREAM);
    let (mut waiting, mut gave_up) = (None, false);
    run(&mut s, &leaving, &stream, 150, 12_000, |c, _| {
        let a = &mut c.agents.agents;
        if waiting.is_none()
            && let Some(&car) = leaving
                .iter()
                .find(|&&i| a.transit[i] == TRANSIT_ACCESS_EGRESS && a.crossing_wait_s[i] > 0.0)
        {
            waiting = Some((car, a.planned_attach_lane_id[car]));
            a.crossing_wait_s[car] = CROSSING_GIVE_UP_S - 0.05;
        }
        let Some((car, far)) = waiting else {
            return false;
        };
        gave_up |= a.planned_attach_lane_id[car] != far;
        a.transit[car] == TRANSIT_IN_BUILDING
    });
    let (car, _) = waiting.expect("a car waited at the mouth");
    let a = &s.c.agents.agents;
    assert!(gave_up, "it left on another lane");
    assert_eq!(a.crossing_wait_s[car], 0.0, "no mark carried onto the road");
    // Its new attach is legal: no stale entry sent it back into its building.
    let failures = &s.c.agents.trip_failures.lines;
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(
        a.transit[car], TRANSIT_IN_BUILDING,
        "it got to its destination"
    );
}
