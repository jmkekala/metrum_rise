// SPDX-License-Identifier: GPL-3.0-or-later

//! `RENDER-66` signals on hand-built lanes: a road east along `+X` (whose driver's left is
//! `-Z`), a parallel lane on its left, a left and a right turn of radius 10 m, a straight
//! connector, and cars set onto them.

use super::*;
use crate::config::CAR_LENGTH;
use crate::nodes::sim::render::vehicle_catalogue::Axles;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::economy::agents::{MODE_CAR, TRANSIT_INTERSECTION, TRANSIT_NETWORK};
use crate::simulation::network::lanes::{Lane, LaneType};

const ROAD: usize = 0;
const LEFT_TURN: usize = 1;
const RIGHT_TURN: usize = 2;
const LEFT_LANE: usize = 3;
const STRAIGHT: usize = 4;
const OTHER_ROAD: usize = 5;
const RADIUS: f32 = 10.0;
// The pilot estate's axles.
const ESTATE: Axles = Axles {
    front_m: 1.3,
    rear_m: -1.35,
};

fn lane(points: Vec<Vector3>, edge_id: usize) -> Lane {
    let mut cum_dist = vec![0.0];
    for pair in points.windows(2) {
        cum_dist.push(cum_dist.last().unwrap() + pair[0].distance_to(pair[1]));
    }
    Lane {
        length: *cum_dist.last().unwrap(),
        geometry: points,
        cum_dist,
        lane_type: LaneType::Vehicle,
        edge_id,
        node_id: if edge_id == usize::MAX { 0 } else { usize::MAX },
        ..Default::default()
    }
}

// A quarter turn from (100, 0, 0) heading `+X`, to the left (`-Z`) for `side` -1, to the right
// for +1, every 5 degrees.
fn quarter_turn(side: f32, radius: f32) -> Lane {
    let points = (0..=18)
        .map(|k| {
            let a = (k as f32 * 5.0).to_radians();
            Vector3::new(
                100.0 + radius * a.sin(),
                0.0,
                side * radius * (1.0 - a.cos()),
            )
        })
        .collect();
    lane(points, usize::MAX)
}

fn core() -> SimCore {
    let mut core = SimCore {
        benchmark_mode: true,
        ..SimCore::new(WorldConfig::default())
    };
    let east = |z: f32| vec![Vector3::new(0.0, 0.0, z), Vector3::new(100.0, 0.0, z)];
    core.transit_network.lane_system.lanes = vec![
        lane(east(0.0), 0),
        quarter_turn(-1.0, RADIUS),
        quarter_turn(1.0, RADIUS),
        lane(east(-3.5), 0),
        lane(
            vec![Vector3::new(100.0, 0.0, 0.0), Vector3::new(115.0, 0.0, 0.5)],
            usize::MAX,
        ),
        lane(east(-20.0), 0),
    ];
    core
}

// A car on `lane` at `d` metres with `speed`.
fn car(core: &mut SimCore, lane: usize, d: f32, speed: f32) -> usize {
    let id = core.agents.spawn_housed_agent(0, 0.0, 0.0);
    let agents = &mut core.agents;
    agents.transit[id] = if core.transit_network.lane_system.lanes[lane].edge_id == usize::MAX {
        TRANSIT_INTERSECTION
    } else {
        TRANSIT_NETWORK
    };
    agents.transit_mode[id] = MODE_CAR;
    agents.current_lane_id[id] = lane;
    agents.lane_distance[id] = d;
    agents.speed[id] = speed;
    id
}

// The lane buckets the tick would have left: each lane's cars by distance.
fn fill_buckets(core: &mut SimCore) {
    let lanes = core.transit_network.lane_system.lanes.len();
    let mut buckets = vec![Vec::new(); lanes];
    for i in 0..core.agents.len() {
        buckets[core.agents.current_lane_id[i]].push((core.agents.lane_distance[i], i));
    }
    for bucket in &mut buckets {
        bucket.sort_by(|a, b| a.0.total_cmp(&b.0));
    }
    core.agents.lane_buckets = buckets;
}

fn signals(core: &SimCore, i: usize, changing: bool) -> (u32, f32) {
    core.car_signals(i, Some(ESTATE), 0, None, CarPath::Lane { changing, gate: None })
}

fn lamps(core: &SimCore, i: usize) -> u32 {
    signals(core, i, false).0
}

#[test]
fn brake_lamps_light_for_a_braking_or_standing_car_and_not_for_a_following_one() {
    let mut core = core();
    let leader = car(&mut core, ROAD, 60.0, 0.0);
    // 5.4 m behind a standing car at 12 m/s: the IDM brakes at about 5 m/s².
    let braking = car(&mut core, ROAD, 60.0 - CAR_LENGTH - 5.4, 12.0);
    let alone = car(&mut core, LEFT_LANE, 20.0, 10.0);
    // Following at 10 m/s with the gap whose IDM acceleration is -0.5 m/s²: steady following
    // hovers about zero, so the lamp stays dark there.
    let v_max = crate::config::DEFAULT_URBAN_ROAD_SPEED_MS;
    let free = (10.0 / v_max).powi(4);
    let gap = (0.1 + 10.0 * 0.5) / (1.0 - free + 0.5 / 6.0).sqrt();
    let follower = car(&mut core, OTHER_ROAD, 20.0, 10.0);
    car(&mut core, OTHER_ROAD, 20.0 + CAR_LENGTH + gap, 10.0);
    fill_buckets(&mut core);
    let queued = car(&mut core, ROAD, 60.0 - CAR_LENGTH - 0.5, 0.0);
    fill_buckets(&mut core);
    let accel = |i: usize| {
        let outlook = core
            .agents
            .speed_outlook(i, &core.transit_network, &core.region_graph)
            .unwrap();
        idm_acceleration(core.agents.speed[i], outlook.v_max, outlook.gap)
    };
    assert!(
        accel(braking) < -BRAKE_DECEL_MS2 * 4.0,
        "{}",
        accel(braking)
    );
    assert!((accel(follower) + 0.5).abs() < 0.05, "{}", accel(follower));
    for (i, braking_lamp) in [
        (leader, true),
        (braking, true),
        (queued, true),
        (alone, false),
        (follower, false),
    ] {
        assert_eq!(lamps(&core, i) & LAMP_BRAKE != 0, braking_lamp, "car {i}");
        assert_eq!(lamps(&core, i) & LAMP_LIGHTS_ON, LAMP_LIGHTS_ON, "car {i}");
    }
    // An off-lane car waiting for its attach slot brakes too.
    let waiting = car(&mut core, ROAD, 10.0, 0.0);
    core.agents.current_lane_id[waiting] = usize::MAX;
    core.agents.access_flags[waiting] = ACCESS_LANE_ATTACH_PENDING;
    let off_lane = CarPath::OffLane {
        facing: Some(Vector3::RIGHT),
        curvature: 0.0,
    };
    assert_ne!(
        core.car_signals(waiting, None, 0, None, off_lane).0 & LAMP_BRAKE,
        0
    );
}

#[test]
fn a_car_signals_the_side_its_connector_or_lane_change_turns_to() {
    let mut core = core();
    let left = car(&mut core, LEFT_TURN, 5.0, 5.0);
    let right = car(&mut core, RIGHT_TURN, 5.0, 5.0);
    let straight = car(&mut core, STRAIGHT, 5.0, 5.0);
    // Changing into the lane on its left (`-Z`), and back.
    let into_left = car(&mut core, LEFT_LANE, 40.0, 12.0);
    core.agents.lane_change_from_lane_id[into_left] = ROAD as u32;
    let into_right = car(&mut core, ROAD, 40.0, 12.0);
    core.agents.lane_change_from_lane_id[into_right] = LEFT_LANE as u32;
    fill_buckets(&mut core);
    let indicators =
        |i: usize, changing: bool| signals(&core, i, changing).0 & (LAMP_LEFT | LAMP_RIGHT);
    assert_eq!(indicators(left, false), LAMP_LEFT);
    assert_eq!(indicators(right, false), LAMP_RIGHT);
    assert_eq!(indicators(straight, false), 0);
    assert_eq!(indicators(into_left, true), LAMP_LEFT);
    assert_eq!(indicators(into_right, true), LAMP_RIGHT);
    // Beyond the signalling range, and on the far tier, no indicator is worked out.
    let far = SIGNAL_RANGE_M * SIGNAL_RANGE_M * 1.01;
    let path = CarPath::Lane { changing: false, gate: None };
    assert_eq!(
        core.car_signals(left, Some(ESTATE), 1, Some(far), path).0 & LAMP_LEFT,
        0
    );
    assert_eq!(
        core.car_signals(left, Some(ESTATE), 3, None, path),
        (LAMP_LIGHTS_ON, 0.0)
    );
}

#[test]
fn the_front_wheels_steer_toward_the_curve_by_the_wheelbase_over_the_radius() {
    let mut core = core();
    let middle = core.transit_network.lane_system.lanes[LEFT_TURN].length * 0.5;
    let left = car(&mut core, LEFT_TURN, middle, 5.0);
    let right = car(&mut core, RIGHT_TURN, middle, 5.0);
    let straight = car(&mut core, ROAD, 50.0, 10.0);
    fill_buckets(&mut core);
    let wanted = ESTATE.wheelbase_m() / RADIUS;
    let (_, left_steer) = signals(&core, left, false);
    let (_, right_steer) = signals(&core, right, false);
    // Positive to the left; the polygon's chords and the quantum cost under 2 %.
    assert!(
        (left_steer - wanted).abs() < 0.02 * wanted,
        "{left_steer} for {wanted}"
    );
    assert_eq!(right_steer.to_bits(), (-left_steer).to_bits());
    let steps = left_steer / STEER_QUANTUM_RAD;
    assert!((steps - steps.round()).abs() < 1e-3, "{steps} quanta");
    assert_eq!(
        signals(&core, straight, false).1.to_bits(),
        0.0_f32.to_bits()
    );
    // A tight turn stops at the lock; tier 2 has no wheels to steer.
    core.transit_network.lane_system.lanes[LEFT_TURN] = quarter_turn(-1.0, 1.5);
    core.agents.lane_distance[left] = 1.2;
    let (_, locked) = signals(&core, left, false);
    assert!(
        (locked - STEER_LOCK_RAD).abs() <= STEER_QUANTUM_RAD,
        "{locked}"
    );
    let path = CarPath::Lane { changing: false, gate: None };
    assert_eq!(core.car_signals(left, Some(ESTATE), 2, None, path).1, 0.0);
    // Off a lane, the drawn arc's curvature: forward toward it, backing against it.
    let off_lane = CarPath::OffLane {
        facing: Some(Vector3::RIGHT),
        curvature: 1.0 / 6.0,
    };
    let steer = core.car_steer(straight, ESTATE, off_lane, false);
    assert!((steer - (ESTATE.wheelbase_m() / 6.0).atan()).abs() < 1e-6);
    assert_eq!(core.car_steer(straight, ESTATE, off_lane, true), -steer);
}

// `PARK-31`: a lane car on its gate turn steers by the drawn arc, as off a lane, not by its lane's
// tangents: a right turn of 2.5 m on a straight lane, at the lock for the estate.
#[test]
fn a_lane_car_on_its_gate_turn_steers_by_the_arc() {
    let mut core = core();
    let i = car(&mut core, ROAD, 50.0, 3.0);
    fill_buckets(&mut core);
    let gate = CarPath::Lane {
        changing: false,
        gate: Some(-1.0 / 2.5),
    };
    let (_, steer) = core.car_signals(i, Some(ESTATE), 0, None, gate);
    // atan(2.65 / 2.5) is 46.7°: the lock.
    let wanted = -(ESTATE.wheelbase_m() / 2.5).atan().min(STEER_LOCK_RAD);
    assert!((steer - wanted).abs() <= STEER_QUANTUM_RAD, "{steer} for {wanted}");
    assert_eq!(signals(&core, i, false).1, 0.0, "straight without it");
}

#[test]
fn the_same_state_draws_the_same_bits_and_angles() {
    let build = || {
        let mut core = core();
        for (k, lane) in [ROAD, LEFT_TURN, RIGHT_TURN, LEFT_LANE, STRAIGHT, OTHER_ROAD]
            .into_iter()
            .enumerate()
        {
            for j in 0..6 {
                let length = core.transit_network.lane_system.lanes[lane].length;
                car(
                    &mut core,
                    lane,
                    length * j as f32 / 6.0,
                    (k + j) as f32 * 1.5,
                );
            }
        }
        fill_buckets(&mut core);
        core
    };
    let (a, b) = (build(), build());
    let drawn = |core: &SimCore| {
        (0..core.agents.len())
            .map(|i| {
                let (lamps, steer) = signals(core, i, false);
                let custom = core
                    .posed_car_custom(i)
                    .map(|(_, custom)| custom.map(f32::to_bits));
                (lamps, steer.to_bits(), custom)
            })
            .collect::<Vec<_>>()
    };
    let first = drawn(&a);
    assert_eq!(first, drawn(&a), "read twice");
    assert_eq!(first, drawn(&b), "a second city in the same state");
    assert!(first.iter().any(|(lamps, _, _)| lamps & LAMP_BRAKE != 0));
    assert!(first.iter().any(|(_, steer, _)| *steer != 0));
}
