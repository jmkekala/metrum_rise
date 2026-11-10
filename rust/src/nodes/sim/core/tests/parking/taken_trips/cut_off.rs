// SPDX-License-Identifier: GPL-2.0-only

//! A driver whose road route to its bay is cut on the way (`TRAFFIC-31`, `TRAFFIC-32`,
//! `TRAFFIC-34`): it parks in the nearest bay it can reach and walks on, or drives home, or, with
//! home cut off too, waits at a lane's end and retries home on the stranded walkers' backoff; a
//! driver whose walk from its bay is cut drives home or stays parked. Each failure logs one
//! `events` line.

use super::*;
use crate::simulation::economy::agents::ACCESS_LANE_ATTACH_PENDING;

// The commute's road in 100 m pieces with a side road at x = 0: the junction the car crosses on a
// connector.
fn pieces() -> Vec<Road> {
    let mut roads = vec![((-600.0, 0.0), (-400.0, 0.0))];
    roads.extend((-4..4).map(|k| ((k as f32 * 100.0, 0.0), ((k + 1) as f32 * 100.0, 0.0))));
    roads.extend([((400.0, 0.0), (600.0, 0.0)), ((0.0, 0.0), (0.0, 300.0))]);
    roads
}

fn node_at(c: &SimCore, x: f32) -> u32 {
    let graph = &c.region_graph;
    (0..graph.nodes().len() as u32)
        .find(|&n| {
            let pos = graph.nodes()[n as usize].pos;
            (pos.x - x).abs() < 0.5 && pos.z.abs() < 0.5 && graph.node_has_live_incident_edge(n)
        })
        .unwrap()
}

impl Commute {
    // +1 when the workplace is east of home, -1 when west: the pieces below are measured towards
    // the workplace.
    fn side(&self) -> f32 {
        self.c.allocator.buildings[self.work].center_x.signum()
    }

    // The piece of road between `a` and `b` metres from the junction towards the workplace goes.
    fn cut(&mut self, a: f32, b: f32) {
        let side = self.side();
        let edge = self
            .c
            .region_graph
            .get_edge_between_nodes(node_at(&self.c, side * a), node_at(&self.c, side * b))
            .unwrap();
        assert!(self.c.bulldoze_road_edge_for_test(edge));
        // The frames after an edit hold until its routing is adopted; these ticks skip the frame.
        self.c.transit_network.settle_cch();
    }

    fn rebuild(&mut self, a: f32, b: f32) {
        let side = self.side();
        commit(&mut self.c, (side * a, 0.0), (side * b, 0.0));
        self.c.transit_network.settle_cch();
    }

    // The piece between `a` and `b` becomes a walkway: no car passes, people walk on.
    fn pedestrianise(&mut self, a: f32, b: f32) {
        self.cut(a, b);
        let side = self.side();
        core_commit_walkway(&mut self.c, (side * a, 0.0), (side * b, 0.0));
        self.c.transit_network.settle_cch();
    }

    // The `events` trip-failure lines about the commuter.
    fn failure_lines(&self) -> Vec<String> {
        let row = format!("[row {}]", self.agent);
        self.c
            .agents
            .trip_failures
            .lines
            .iter()
            .filter(|line| line.contains(&row))
            .cloned()
            .collect()
    }

    // A tick that notes the parking retries the commuter asks for, in seconds.
    fn tick_noting_retries(&mut self, retries: &mut Vec<u16>) {
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
        let me = c.agents.agent_handle(self.agent);
        retries.extend(
            c.agents
                .parking_retries
                .iter()
                .filter(|(agent, _)| *agent == me)
                .map(|&(_, seconds)| seconds),
        );
        c.schedule_parking_retries();
        if c.agents.parking_trip[self.agent].retry_pending {
            c.run_slots_through_for_test(c.time.absolute_slot() + 1);
        }
    }

    // `row` with each lane named by its start, direction and index, and the position.
    fn row_by_road(&self) -> impl PartialEq + std::fmt::Debug + use<> {
        let (a, i) = (&self.c.agents, self.agent);
        // A load numbers edited roads and their lanes afresh: a lane is named by where it starts.
        let lanes = &self.c.transit_network.lane_system.lanes;
        let named = |lane: usize| {
            lanes.get(lane).map(|l| {
                let start = l.geometry.first().map(|p| (p.x.to_bits(), p.z.to_bits()));
                (start, l.is_fwd, l.lane_idx)
            })
        };
        // The attach point means something only while the car is held for it.
        let held = a.access_flags[i] & ACCESS_LANE_ATTACH_PENDING != 0;
        (
            (
                a.transit[i],
                a.transit_mode[i],
                named(a.current_lane_id[i]),
                a.current_node[i],
            ),
            (
                a.target_building[i],
                a.current_path_index[i],
                a.current_path[i].clone(),
            ),
            (
                a.lane_distance[i].to_bits(),
                a.speed[i].to_bits(),
                a.next_replan_time[i].to_bits(),
                a.network_replan_failures[i],
            ),
            (
                a.pos_x[i].to_bits(),
                a.pos_y[i].to_bits(),
                a.access_flags[i],
                held.then(|| named(a.planned_attach_lane_id[i] as usize)),
            ),
            serde_json::to_value(a.parking_trip[i]).unwrap(),
        )
    }

    // `continues_exactly_across_a_save` by `row_by_road`; the `events` lines logged before the
    // save are kept across the load.
    fn continues_across_a_save(&mut self, name: &str, ticks: usize) {
        let before = self.row_by_road();
        let path = temp_save_path(name);
        self.c
            .save_game_settled_at(path.to_str().unwrap(), None, 0)
            .unwrap();
        let mut straight = Vec::with_capacity(ticks);
        for _ in 0..ticks {
            self.tick();
            straight.push(self.row_by_road());
        }
        let lines = std::mem::take(&mut self.c.agents.trip_failures.lines);
        self.c.load_game_internal(path.to_str().unwrap()).unwrap();
        std::fs::remove_file(path).unwrap();
        self.c.agents.trip_failures.lines = lines;
        assert_eq!(
            self.row_by_road(),
            before,
            "a load leaves the row as it stands"
        );
        for (tick, row) in straight.iter().enumerate() {
            self.tick();
            assert_eq!(&self.row_by_road(), row, "tick {tick} after the load");
        }
    }

    fn position(&self) -> (f32, f32) {
        let a = &self.c.agents;
        (a.pos_x[self.agent], a.pos_y[self.agent])
    }

    // On the road towards the workplace, before the junction at x = 0.
    fn short_of_the_junction(&mut self) {
        let side = self.side();
        self.tick_until("drove towards the junction", |c, i| {
            c.agents.parking_trip[i].phase == ParkingTripPhase::Drive
                && c.agents.current_lane_id[i] != usize::MAX
                && c.agents.pos_x[i] * side < -150.0
        });
    }

    // Crossing the junction at x = 0 on a connector.
    fn in_the_junction(&mut self) -> u32 {
        let junction = node_at(&self.c, 0.0);
        self.tick_until("crossed the junction", |c, i| {
            c.agents.transit[i] == TRANSIT_INTERSECTION && c.agents.current_node[i] == junction
        });
        junction
    }

    // A road edit that cuts the route while the car crosses the junction drops it off its
    // connector at the junction's node with no lane (`clear_route_plan_after_road_edit`).
    fn dropped_at(&self, junction: u32) {
        let a = &self.c.agents;
        let i = self.agent;
        assert_eq!(
            (a.transit[i], a.current_node[i], a.current_lane_id[i]),
            (TRANSIT_NETWORK, junction, usize::MAX),
            "the edit leaves the car at the junction's node"
        );
    }
}

// (b), and `TRAFFIC-31`'s car: a road edit cuts the driver's route beyond the junction it crosses
// and drops it there; its workplace's only bays are cut off, so it gives the trip up, releases its
// booking and drives home from the junction. One `events` line says so (`TRAFFIC-32`). Before, it
// stood in the junction for good (`TRAFFIC-31`), then until a retry found its bay again.
#[test]
fn a_car_left_in_a_junction_by_a_cut_route_releases_its_bay_and_drives_home() {
    let mut t = Commute::on_roads(&pieces());
    let (agent, home, work) = (t.agent, t.home, t.work);
    let junction = t.in_the_junction();
    let booked = t.c.agents.parking_trip[agent].destination;
    assert!(t.c.parking.trip_reserved(t.car(), booked));
    t.cut(300.0, 400.0);
    t.dropped_at(junction);
    let home_handle = t.building_handle(home);
    t.tick_until("turned for home", |c, i| {
        c.agents.parking_trip[i].target == home_handle
    });
    let a = &t.c.agents;
    assert_eq!(a.target_building[agent], home);
    assert!(!a.parking_trip[agent].retry_pending, "it drives at once");
    assert!(
        !t.c.parking.trip_reserved(t.car(), booked),
        "the booking is released"
    );
    let work_handle = t.building_handle(work);
    assert_eq!(
        t.c.parking
            .first_free_candidate(work_handle)
            .map(|c| c.place),
        Some(booked),
        "the bay is free again"
    );
    assert!(
        t.c.parking
            .trip_reserved(t.car(), a.parking_trip[agent].destination)
    );
    t.tick_until("left the junction", |c, i| {
        c.agents.current_lane_id[i] != usize::MAX
    });
    assert_eq!(
        t.c.agents.current_node[agent], junction,
        "it drives on from where it stood"
    );
    t.inside(home);
    assert!(t.c.parking.vehicle_place(t.car()).is_some(), "it parked");
    let lines = t.failure_lines();
    assert_eq!(lines.len(), 1, "{lines:#?}");
    assert!(
        lines[0].contains(&format!("at node {junction}"))
            && lines[0].contains(&format!("to building {work}"))
            && lines[0].contains(&format!("no route from node {junction} to building {work}"))
            && lines[0].ends_with(&format!("gives up and drives home to building {home}")),
        "{}",
        lines[0]
    );
}

// `TRAFFIC-32`: the failed replan of a car on a parking leg logs one `events` line with the
// planner's reason and what the leg's recovery did, as every other trip failure does. Before, the
// leg went to its recovery with no line at all.
#[test]
fn a_parking_legs_failed_replan_logs_one_events_line() {
    let mut t = Commute::on_roads(&pieces());
    let (agent, work) = (t.agent, t.work);
    t.short_of_the_junction();
    t.cut(300.0, 400.0);
    t.tick_until("logged its failure", |c, _| {
        !c.agents.trip_failures.lines.is_empty()
    });
    for _ in 0..600 {
        t.tick();
    }
    let lines = t.failure_lines();
    assert_eq!(lines.len(), 1, "{lines:#?}");
    let line = &lines[0];
    let render = t.c.agents.render_id[agent];
    assert!(
        line.starts_with(&format!(
            "trip failed: agent {render} [row {agent}], resident, car, "
        )) && line.contains(&format!("to building {work} ("))
            && line.contains(&format!(" to building {work} ("))
            && line.contains("no route from node")
            && line.contains("no road path from node"),
        "{line}"
    );
}

// (a): the workplace's own bays are cut off by road, but a public lot short of the cut can be
// reached, and the cut piece is a walkway now: the car parks in that lot's nearest free bay and
// the driver walks on to work. Its booking at work is released.
#[test]
fn a_car_whose_bay_is_cut_off_parks_nearby_and_the_driver_walks_on() {
    let mut t = Commute::with_lots_near_work(&pieces(), &[200.0]);
    let (agent, work) = (t.agent, t.work);
    let lot_x = t.side() * 250.0;
    t.short_of_the_junction();
    let booked = t.c.agents.parking_trip[agent].destination;
    t.pedestrianise(300.0, 400.0);
    t.tick_until("took another bay", |c, i| {
        c.agents.parking_trip[i].destination != booked
            && !c.agents.parking_trip[i].ready
            && c.parking.trip_reserved(
                c.agents.parking_trip[i].vehicle,
                c.agents.parking_trip[i].destination,
            )
    });
    let nearby = t.c.agents.parking_trip[agent].destination;
    let curb = t.c.parking.place_curb(nearby).unwrap();
    assert!(
        (curb.x - lot_x).abs() < 30.0,
        "a bay of the lot short of the cut: {curb:?}"
    );
    assert!(
        !t.c.parking.trip_reserved(t.car(), booked),
        "the booking at work is released"
    );
    assert_eq!(t.c.agents.target_building[agent], work, "the trip goes on");
    t.tick_until("walked from the bay", |c, i| {
        c.agents.parking_trip[i].phase == ParkingTripPhase::WalkToDoor
    });
    assert_eq!(
        t.c.parking.vehicle_place(t.car()),
        Some(nearby),
        "parked there"
    );
    assert_eq!(t.c.agents.transit_mode[agent], MODE_WALK);
    t.inside(work);
    let lines = t.failure_lines();
    assert_eq!(lines.len(), 1, "{lines:#?}");
    assert!(
        lines[0].contains(&format!("to building {work}"))
            && lines[0].contains("parks in the nearest free bay it can reach by road")
            && lines[0].ends_with("and walks on"),
        "{}",
        lines[0]
    );
}

// The car runs on to the bay it took near work while the walkway from there is cut as well: the
// driver finds no walk on, takes the car again and drives home (the user's rule of 2026-11-10).
#[test]
fn a_driver_with_no_walk_on_from_its_bay_drives_home() {
    let mut t = Commute::with_lots_near_work(&pieces(), &[200.0]);
    let (agent, home, work) = (t.agent, t.home, t.work);
    t.short_of_the_junction();
    let booked = t.c.agents.parking_trip[agent].destination;
    t.pedestrianise(300.0, 400.0);
    t.tick_until("took another bay", |c, i| {
        c.agents.parking_trip[i].destination != booked && !c.agents.parking_trip[i].ready
    });
    let nearby = t.c.agents.parking_trip[agent].destination;
    t.cut(300.0, 400.0);
    t.tick_until("pulled out for home", |c, i| {
        c.agents.parking_trip[i].phase == ParkingTripPhase::PullOut
    });
    let trip = t.c.agents.parking_trip[agent];
    assert_eq!(trip.source, nearby, "it leaves the bay it pulled into");
    assert_eq!(t.c.agents.target_building[agent], home);
    assert_eq!(trip.target, t.building_handle(home));
    t.inside(home);
    assert!(
        t.c.parking.vehicle_place(t.car()).is_some(),
        "parked at home"
    );
    let lines = t.failure_lines();
    assert_eq!(lines.len(), 2, "{lines:#?}");
    assert!(
        lines[0].contains("parks in the nearest free bay"),
        "{}",
        lines[0]
    );
    assert!(
        lines[1].contains(&format!("no walk from its parking bay to building {work}"))
            && lines[1].ends_with(&format!("gives up and drives home to building {home}")),
        "{}",
        lines[1]
    );
}

// As above with the road home cut too: the car stays in its bay, off the road, and the drive home
// is planned again on the stranded walkers' backoff, 5 s, 30 s, then every 2 min, with one line for
// it all. Once the road home is back the next retry drives home.
#[test]
fn a_driver_with_no_walk_on_and_no_way_home_stays_parked_and_retries() {
    let mut t = Commute::with_lots_near_work(&pieces(), &[200.0]);
    let (agent, home) = (t.agent, t.home);
    let side = t.side();
    t.short_of_the_junction();
    let booked = t.c.agents.parking_trip[agent].destination;
    t.pedestrianise(300.0, 400.0);
    t.tick_until("took another bay past the junction", |c, i| {
        c.agents.parking_trip[i].destination != booked
            && !c.agents.parking_trip[i].ready
            && c.agents.pos_x[i] * side > 50.0
    });
    t.cut(300.0, 400.0);
    t.cut(-200.0, -100.0);
    t.tick_until("stayed parked", |c, i| {
        c.agents.parking_trip[i].phase == ParkingTripPhase::PullIn
            && c.agents.parking_trip[i].retry_pending
    });
    let at = t.position();
    let mut retries = Vec::new();
    while retries.len() < 4 {
        t.tick_noting_retries(&mut retries);
        let trip = t.c.agents.parking_trip[agent];
        assert_eq!(trip.phase, ParkingTripPhase::PullIn, "it stays in its bay");
        assert_eq!(
            t.c.agents.transit[agent], TRANSIT_ACCESS_INGRESS,
            "off the road"
        );
        assert_eq!(t.position(), at);
    }
    assert_eq!(retries, [30, 120, 120, 120], "after the first 5 s");
    t.continues_across_a_save("cut-off-stays-parked", 50);
    t.rebuild(-200.0, -100.0);
    t.tick_until("pulled out for home", |c, i| {
        c.agents.parking_trip[i].phase == ParkingTripPhase::PullOut
    });
    t.inside(home);
    let lines = t.failure_lines();
    assert_eq!(lines.len(), 2, "{lines:#?}");
    assert!(
        lines[1].contains("no walk from its parking bay")
            && lines[1].ends_with("stays parked in its bay, plans the drive home again in 5 s"),
        "{}",
        lines[1]
    );
}

// (c): the workplace's bays and home are both cut off from the junction the car is dropped in. It
// takes the nearest road lane out of the junction, drives to its end and stands there, retrying
// home on the stranded walkers' backoff with one line for it all; it is never moved and never
// stands in the junction. Once the road home is back the next retry drives home. Saved and loaded
// on the way to the lane's end and while it waits there.
#[test]
fn a_car_with_no_bay_in_reach_and_no_way_home_waits_at_a_lane_end_and_retries() {
    let mut t = Commute::on_roads(&pieces());
    let (agent, home) = (t.agent, t.home);
    let junction = t.in_the_junction();
    t.cut(300.0, 400.0);
    t.cut(-400.0, -300.0);
    t.dropped_at(junction);
    let mut retries = Vec::new();
    let mut last = t.position();
    let mut jump: f32 = 0.0;
    let mut left_the_junction = false;
    let mut saved_on_the_way = false;
    // The lane it leaves the junction by, as numbered before any load.
    let mut first_lane = usize::MAX;
    while retries.len() < 4 {
        t.tick_noting_retries(&mut retries);
        let a = &t.c.agents;
        let here = t.position();
        jump = jump.max((here.0 - last.0).hypot(here.1 - last.1));
        last = here;
        let on_lane = a.current_lane_id[agent] != usize::MAX;
        if on_lane && !left_the_junction {
            first_lane = a.current_lane_id[agent];
        }
        left_the_junction |= on_lane;
        if left_the_junction {
            assert!(on_lane, "back in the junction");
            assert_eq!(a.transit[agent], TRANSIT_NETWORK);
        }
        if on_lane && !saved_on_the_way && a.lane_distance[agent] > 20.0 {
            saved_on_the_way = true;
            t.continues_across_a_save("cut-off-to-lane-end", 20);
            last = t.position();
        }
    }
    assert!(saved_on_the_way);
    assert!(jump < 8.0, "moved {jump} m in one tick");
    assert_eq!(retries, [5, 30, 120, 120]);
    let a = &t.c.agents;
    let waited_on = a.current_lane_id[agent];
    let length = t.c.transit_network.lane_system.lanes[waited_on].length;
    assert_eq!(
        a.lane_distance[agent], length,
        "it stands at its lane's end"
    );
    assert_eq!(a.speed[agent], 0.0);
    assert_eq!(a.target_building[agent], home, "it retries home");
    assert!(
        t.c.parking
            .trip_reserved(t.car(), a.parking_trip[agent].destination)
    );
    t.continues_across_a_save("cut-off-at-lane-end", 50);
    t.rebuild(-400.0, -300.0);
    t.tick_until("drove off for home", |c, i| {
        c.agents.access_flags[i] & ACCESS_PLAN_VALID != 0 && !c.agents.parking_trip[i].ready
    });
    t.inside(home);
    assert!(t.c.parking.vehicle_place(t.car()).is_some(), "it parked");
    let lines = t.failure_lines();
    assert_eq!(lines.len(), 1, "{lines:#?}");
    assert!(
        lines[0].contains(&format!("at node {junction}"))
            && lines[0].ends_with(&format!(
                "no free bay in reach and no way home: stops at the end of lane {first_lane}, plans \
                 home again in 5 s"
            )),
        "{}",
        lines[0]
    );
}
