// SPDX-License-Identifier: GPL-3.0-or-later

//! `TRAFFIC-41`: a census of the walkers' leg cycle against their speed on a prepared city, by
//! where they walk: a road's sidewalk, a crossing, an access path (a door's, a bay's, a family
//! stepping out of its car) or the ground off every lane. A walker's legs match its body when its
//! walk phase advances the same number of cycles for every metre of its size wherever it walks,
//! and no step is cut by the per-tick cap. Read-only: nothing here touches the simulation.

use super::super::super::frame::FRAME_DT_S;
use super::super::contract::{cache_dir, residents, timing_mode};
use crate::simulation::economy::agents::walker_gait::walker_size;
use crate::simulation::economy::agents::{
    MODE_CAR, TRANSIT_ACCESS_EGRESS, TRANSIT_ACCESS_INGRESS, TRANSIT_IN_BUILDING,
};

// Where a walker walks, in report order.
const PLACES: [&str; 4] = ["sidewalk", "crossing", "access_path", "off_lane"];
// A walker that moved less than this in a frame, metres, stands: it has no gait to read.
const MOVED_M: f32 = 1.0e-4;

#[derive(Clone, Copy, Default)]
struct Gait {
    samples: u64,
    metres: f64,
    cycles: f64,
    // Cycles a metre of the walker's size: the least and the most seen.
    least: f32,
    most: f32,
}

/// Steps the prepared `METRUM_PERF_RESIDENTS` city to 07:30 at 32x, then `METRUM_GAIT_FRAMES`
/// frames (default 1,800, 30 s) at `METRUM_GAIT_SPEED` (default 1) and prints, by place, the
/// walkers' mean speed, leg cycles a second and cycles a metre of their size.
#[test]
#[ignore = "walkers' leg cycle against their speed on a prepared city; release"]
fn walker_gait_on_a_prepared_city() {
    timing_mode();
    let number = |name: &str, default: f32| {
        std::env::var(name).map_or(default, |value| value.parse().expect("a number"))
    };
    let frames = number("METRUM_GAIT_FRAMES", 1_800.0) as usize;
    let speed = number("METRUM_GAIT_SPEED", 1.0);
    let mut city = super::super::snapshot::load_snapshot(residents(), &cache_dir());
    let core = &mut city.core;
    core.time.speed_multiplier = 32.0;
    while core.time.minute_of_day != 450 {
        core.step_frame(FRAME_DT_S);
    }
    core.time.speed_multiplier = speed;
    let dt = FRAME_DT_S as f32 * speed;
    let mut gaits = [Gait {
        least: f32::MAX,
        ..Default::default()
    }; PLACES.len()];
    // Per agent index: render id, position, walk phase and whether it walked, last frame.
    let mut before: Vec<(u64, f32, f32, f32, bool)> = Vec::new();
    for _ in 0..frames {
        core.step_frame(FRAME_DT_S);
        let agents = &core.agents.agents;
        let lanes = &core.transit_network.lane_system.lanes;
        before.resize(agents.len(), (u64::MAX, 0.0, 0.0, 0.0, false));
        for i in 0..agents.len() {
            let transit = agents.transit[i];
            let walks = agents.transit_mode[i] != MODE_CAR && transit != TRANSIT_IN_BUILDING;
            let now = (
                agents.render_id[i],
                agents.pos_x[i],
                agents.pos_y[i],
                agents.walk_phase[i],
                walks,
            );
            let (id, x, z, phase, walked) = std::mem::replace(&mut before[i], now);
            let moved = (now.1 - x).hypot(now.2 - z);
            if !walks || !walked || id != now.0 || moved < MOVED_M {
                continue;
            }
            let place = if matches!(transit, TRANSIT_ACCESS_EGRESS | TRANSIT_ACCESS_INGRESS) {
                2
            } else {
                match lanes.get(agents.current_lane_id[i]) {
                    Some(lane) if lane.edge_id != usize::MAX => 0,
                    Some(_) => 1,
                    None => 3,
                }
            };
            let cycles = (now.3 - phase).rem_euclid(1.0);
            let per_metre = cycles * walker_size(agents.schedule_seed[i]) / moved;
            let gait = &mut gaits[place];
            gait.samples += 1;
            gait.metres += f64::from(moved);
            gait.cycles += f64::from(cycles);
            gait.least = gait.least.min(per_metre);
            gait.most = gait.most.max(per_metre);
        }
    }
    let places: serde_json::Map<String, serde_json::Value> = PLACES
        .iter()
        .zip(gaits)
        .map(|(&place, gait)| {
            let seconds = gait.samples as f64 * f64::from(dt);
            let value = if gait.samples == 0 {
                serde_json::json!({ "samples": 0 })
            } else {
                serde_json::json!({
                    "samples": gait.samples,
                    "speed_ms": gait.metres / seconds,
                    "cycles_per_s": gait.cycles / seconds,
                    "cycles_per_m": gait.cycles / gait.metres,
                    "least_cycles_per_size_m": gait.least,
                    "most_cycles_per_size_m": gait.most,
                })
            };
            (place.to_string(), value)
        })
        .collect();
    eprintln!(
        "WALKER_GAIT {}",
        serde_json::json!({
            "residents": residents(), "frames": frames, "speed": speed, "places": places,
        })
    );
}
