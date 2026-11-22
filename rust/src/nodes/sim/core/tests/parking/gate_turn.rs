// SPDX-License-Identifier: GPL-3.0-or-later

//! `PARK-31` gate turns as the snapshot poses them: a resident leaving one lot and parking in
//! another is drawn joining and leaving its lane on an arc, continuous in position and heading
//! across both lane switches; held at its attach point it stands where it
//! joined, facing along the arc; and on a curved lane the lane half meets the lane pose where the
//! reach begins and the arc's middle at the lane point.

use super::*;
use crate::nodes::sim::render::lane_pose::sample_lane_pose;
use crate::simulation::parking::ParkingTripPhase;
use crate::simulation::parking::turns::{CAR_GATE_RADIUS_M, GATE_LANE_OFFSET_M};
use godot::prelude::Vector2;

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
    c.schedule_stored_parking_arrivals();
}

// Two bundled small lots on a straight street and a commuter between them.
fn commuter() -> (SimCore, usize) {
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
    (c, agent)
}

// Where car `i` is in the simulation: on its lane's centre line, or at its stored position.
fn simulated(c: &SimCore, i: usize) -> Vector2 {
    let lane = c.agents.current_lane_id[i];
    c.transit_network
        .lane_system
        .lanes
        .get(lane)
        .and_then(|lane| sample_lane_pose(lane, c.agents.lane_distance[i]))
        .map_or(
            Vector2::new(c.agents.pos_x[i], c.agents.pos_y[i]),
            |(p, _)| Vector2::new(p.x, p.z),
        )
}

// One snapshot of the car: simulated point, drawn point, nose, on a lane.
type Frame = (Vector2, Vector2, Vector2, bool);

fn frame(c: &SimCore, i: usize) -> Option<Frame> {
    let (point, nose, on_lane, _) = c.posed_car_pose(i)?;
    Some((simulated(c, i), point, nose, on_lane))
}

// Consecutive snapshots of a car near a gate never jump: the drawn point moves at most 1.6 times
// as far as the simulated one, and the drawn line turns no faster than a 1 m bend's arc would
// (the lot's shortest), plus the 5° a slight bend keeps as a corner. `cusp` allows a reversal of
// the nose where a truck changes between backing and driving forward (the line stays smooth).
fn assert_smooth(frames: &[Frame], cusp: bool) {
    for pair in frames.windows(2) {
        let ((s0, d0, n0, lane0), (s1, d1, n1, lane1)) = (pair[0], pair[1]);
        let moved = s0.distance_to(s1);
        assert!(
            d0.distance_to(d1) <= 1.6 * moved + 0.05,
            "drawn {d0:?} -> {d1:?} for {moved} m (lane {lane0} -> {lane1})"
        );
        let turn = if cusp { n0.dot(n1).abs() } else { n0.dot(n1) }
            .clamp(-1.0, 1.0)
            .acos();
        assert!(
            turn <= 1.3 * moved + 5f32.to_radians() + 0.01,
            "heading turns {}° for {moved} m at {d1:?} (lane {lane0} -> {lane1})",
            turn.to_degrees()
        );
    }
}

// The commuter's whole trip near its two gates: out of its home lot onto the lane on the arc,
// along the street, and off it into the other lot on the arc, with no pop at either switch.
#[test]
fn a_car_joins_and_leaves_its_lane_on_the_gate_arc() {
    let (mut c, agent) = commuter();
    let mut near_gate = Vec::new();
    let (mut joined_on_arc, mut left_on_arc, mut parked) = (0, 0, false);
    let mut gates = [None::<Vector2>; 2];
    let mut switches = 0;
    let mut last_lane = None;
    for _ in 0..20_000 {
        tick(&mut c);
        let a = &c.agents;
        let phase = a.parking_trip[agent].phase;
        if phase == ParkingTripPhase::WalkToDoor {
            parked = true;
            break;
        }
        let Some(frame) = frame(&c, agent) else {
            continue;
        };
        let lane_point = |lane: u32, d: f32| {
            c.transit_network
                .lane_system
                .lanes
                .get(lane as usize)
                .and_then(|lane| sample_lane_pose(lane, d))
                .map(|(p, _)| Vector2::new(p.x, p.z))
        };
        match phase {
            ParkingTripPhase::PullOut => {
                gates[0] = lane_point(
                    a.planned_attach_lane_id[agent],
                    a.planned_attach_lane_d[agent],
                )
            }
            ParkingTripPhase::Drive | ParkingTripPhase::PullIn => {
                gates[1] = lane_point(
                    a.planned_detach_lane_id[agent],
                    a.planned_detach_lane_d[agent],
                )
            }
            _ => {}
        }
        let near = |gate: Option<Vector2>| {
            gate.is_some_and(|gate| gate.distance_to(frame.0) < CAR_GATE_RADIUS_M + 4.0)
        };
        if !near(gates[0]) && !near(gates[1]) {
            if !near_gate.is_empty() {
                assert_smooth(&near_gate, false);
                near_gate.clear();
            }
            continue;
        }
        let (simulated, drawn, _, on_lane) = frame;
        if last_lane.is_some_and(|last| last != on_lane) {
            switches += 1;
        }
        last_lane = Some(on_lane);
        if on_lane {
            let off = simulated.distance_to(drawn);
            assert!(off <= GATE_LANE_OFFSET_M, "{off} m off its lane pose");
            if off > 0.3 {
                if near(gates[1]) && phase != ParkingTripPhase::PullOut && gates[1] != gates[0] {
                    left_on_arc += 1;
                } else {
                    joined_on_arc += 1;
                }
            }
        }
        near_gate.push(frame);
    }
    assert_smooth(&near_gate, false);
    assert!(parked, "it parked");
    assert!(switches >= 2, "{switches} switches seen");
    assert!(
        joined_on_arc > 0 && left_on_arc > 0,
        "{joined_on_arc} {left_on_arc}"
    );
}

// Held at its attach point (the slot busy) the car stands where the lane half begins, at the
// arc's middle facing along it, not at the lane point facing a fixed direction; the next tick it
// joins its lane there with no jump.
#[test]
fn a_car_held_at_its_attach_point_faces_along_its_arc() {
    let (mut c, agent) = commuter();
    for _ in 0..20_000 {
        tick(&mut c);
        let a = &c.agents;
        if a.parking_trip[agent].phase == ParkingTripPhase::Drive
            && a.current_lane_id[agent] == a.planned_attach_lane_id[agent] as usize
        {
            break;
        }
    }
    let a = &c.agents;
    let (lane, d) = (
        a.planned_attach_lane_id[agent] as usize,
        a.planned_attach_lane_d[agent],
    );
    assert_eq!(a.lane_distance[agent], d, "just joined");
    let joined = c.posed_car_pose(agent).unwrap();
    let node = c.agents.current_node[agent];
    c.agents
        .hold_for_lane_attach(agent, &c.transit_network.lane_system, lane, d, node);
    assert!(c.agents.pending_lane_attach(agent).is_some());
    let held = c.posed_car_pose(agent).unwrap();
    assert!(!held.2, "held off its lane");
    assert!(
        held.0.distance_to(joined.0) < 1e-4,
        "{:?} {:?}",
        held.0,
        joined.0
    );
    assert!(
        held.1.dot(joined.1) > 0.99999,
        "{:?} {:?}",
        held.1,
        joined.1
    );
    let (_, tangent) = sample_lane_pose(&c.transit_network.lane_system.lanes[lane], d).unwrap();
    let along = Vector2::new(tangent.x, tangent.z).normalized();
    assert!(
        (held.1.dot(along) - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-4,
        "45° to its lane"
    );
}

// A car near the lane point of a curved lane: the lane half starts exactly at the lane pose,
// bends with the lane, and ends at the arc's middle in the lane point's own frame, where the
// off-lane half starts (`GateLane::at_lane_point`); a car just past the reach is posed on its lane
// bit for bit. Both ways, for a truck's radius.
#[test]
fn on_a_curved_lane_the_lane_half_meets_both_neighbours() {
    use crate::simulation::economy::agents::{
        ACCESS_PLAN_VALID, MODE_CAR, TRANSIT_NETWORK, VEHICLE_FREIGHT_DELIVERY,
    };
    use crate::simulation::network::lanes::{Lane, LaneType};
    use crate::simulation::parking::turns::{FREIGHT_GATE_RADIUS_M, GateLane, LOT_HAND};
    let mut c = SimCore {
        benchmark_mode: true,
        ..SimCore::new(WorldConfig::default())
    };
    // A 40 m radius curve to the left over 90°, every 2°.
    let points: Vec<_> = (0..=45)
        .map(|k| {
            let a = (k as f32 * 2.0).to_radians();
            Vector3::new(100.0 + 40.0 * a.sin(), 0.0, -40.0 * (1.0 - a.cos()))
        })
        .collect();
    let mut cum_dist = vec![0.0];
    for pair in points.windows(2) {
        cum_dist.push(cum_dist.last().unwrap() + pair[0].distance_to(pair[1]));
    }
    c.transit_network.lane_system.lanes = vec![Lane {
        length: *cum_dist.last().unwrap(),
        geometry: points,
        cum_dist,
        lane_type: LaneType::Vehicle,
        edge_id: 0,
        node_id: usize::MAX,
        ..Default::default()
    }];
    let point_d = 30.0;
    for egress in [false, true] {
        let i = c.agents.spawn_housed_agent(0, 0.0, 0.0);
        let a = &mut c.agents;
        a.transit[i] = TRANSIT_NETWORK;
        a.transit_mode[i] = MODE_CAR;
        a.vehicle_type[i] = VEHICLE_FREIGHT_DELIVERY;
        a.current_lane_id[i] = 0;
        a.access_flags[i] = ACCESS_PLAN_VALID;
        a.parking_trip[i].freight_gate = !egress;
        if egress {
            a.planned_attach_lane_id[i] = 0;
            a.planned_attach_lane_d[i] = point_d;
        } else {
            a.planned_detach_lane_id[i] = 0;
            a.planned_detach_lane_d[i] = point_d;
        }
        let lane = c.transit_network.lane_system.lanes[0].clone();
        let pose_at = |c: &mut SimCore, d: f32| {
            c.agents.lane_distance[i] = d;
            c.posed_car_pose(i).unwrap()
        };
        let sign = if egress { 1.0 } else { -1.0 };
        let radius = FREIGHT_GATE_RADIUS_M;
        // Just beyond the reach: the lane pose, as before.
        let (p, t) = sample_lane_pose(&lane, point_d + sign * (radius + 0.01)).unwrap();
        let beyond = pose_at(&mut c, point_d + sign * (radius + 0.01));
        assert_eq!(beyond.0, Vector2::new(p.x, p.z));
        assert!(beyond.1.dot(Vector2::new(t.x, t.z).normalized()) > 0.99999);
        // Where the reach begins.
        let start = pose_at(&mut c, point_d + sign * radius);
        let (p, t) = sample_lane_pose(&lane, point_d + sign * radius).unwrap();
        assert!(start.0.distance_to(Vector2::new(p.x, p.z)) < 1e-4);
        assert!(start.1.dot(Vector2::new(t.x, t.z).normalized()) > 0.99999);
        // At the lane point: the off-lane half's start.
        let (p, t) = sample_lane_pose(&lane, point_d).unwrap();
        let gate = GateLane {
            point: Vector2::new(p.x, p.z),
            along: Vector2::new(t.x, t.z).normalized(),
            hand: LOT_HAND,
            radius,
            egress,
        };
        let (middle, travel, _) = gate.at_lane_point();
        let at_point = pose_at(&mut c, point_d);
        assert!(
            at_point.0.distance_to(middle) < 1e-5,
            "{:?} {middle:?}",
            at_point.0
        );
        assert!(at_point.1.dot(travel) > 0.99999);
        // Smooth in between, and within the lane offset bound.
        let mut previous = start;
        for k in 1..=600 {
            let d = point_d + sign * radius * (1.0 - k as f32 / 600.0);
            let pose = pose_at(&mut c, d);
            let (p, _) = sample_lane_pose(&lane, d).unwrap();
            assert!(pose.0.distance_to(Vector2::new(p.x, p.z)) <= GATE_LANE_OFFSET_M);
            assert!(pose.0.distance_to(previous.0) < 0.02, "{d}");
            assert!(pose.1.dot(previous.1) > 0.2f32.to_radians().cos(), "{d}");
            previous = pose;
        }
        c.agents.current_lane_id[i] = usize::MAX;
        c.agents.transit[i] = crate::simulation::economy::agents::TRANSIT_IN_BUILDING;
    }
}
