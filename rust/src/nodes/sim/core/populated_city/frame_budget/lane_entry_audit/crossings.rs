// SPDX-License-Identifier: GPL-3.0-or-later

//! `PARK-32` crossing figures of the lane-entry audit, one `PARK32_CROSSINGS` line a day: the
//! driven distance and time of each car and truck trip that ended at an access (from its lane
//! entry to its detach), the day's vehicle-kilometres, how many trips crossed the road entering
//! and leaving, the give-ups, and the waits to cross. Sampled once a frame from the agent columns:
//! a crossing lasts over a second of game time (several frames at 32x), a wait shorter than a
//! frame reads as none. Read-only: nothing here touches the simulation.

use crate::nodes::sim::core::state::SimCore;
use crate::simulation::economy::agents::tick::CROSSING_GAVE_UP;
use crate::simulation::economy::agents::{
    AgentVec, MODE_CAR, TRANSIT_ACCESS_EGRESS, TRANSIT_ACCESS_INGRESS, TRANSIT_INTERSECTION,
    TRANSIT_NETWORK, VEHICLE_FREIGHT_DELIVERY,
};

// Vehicle classes, in report order.
const CLASSES: [&str; 2] = ["car", "truck"];

// Agent `i`'s wait to cross and whether it holds a crossed lane now.
fn crossing_state(agents: &AgentVec, i: usize) -> (f32, bool) {
    (
        agents.crossing_wait_s[i],
        agents.crossing_lane[i] != u32::MAX,
    )
}

#[derive(Clone, Copy, Default)]
struct Row {
    id: u64,
    transit: u8,
    wait: f32,
    // This trip's driven metres and seconds on the road so far.
    metres: f32,
    seconds: f32,
    // Its leaving leg crossed the road.
    left_across: bool,
}

#[derive(Default)]
struct Class {
    trips: u64,
    metres: f64,
    seconds: f64,
    driven_m: f64,
    entered: u64,
    entered_across: u64,
    left: u64,
    left_across: u64,
}

#[derive(Default)]
pub(super) struct CrossingBook {
    rows: Vec<Row>,
    classes: [Class; 2],
    give_ups: u64,
    // Cars that gave up waiting at their mouth and left on the near lane.
    leave_give_ups: u64,
    enter_waits: Vec<f32>,
    leave_waits: Vec<f32>,
    last_time: Option<f32>,
    // `METRUM_AUDIT_LEAVE_TRACE=1`: one `PARK32_LONG_LEAVE` line per car whose wait to cross
    // when leaving passes each of `LONG_LEAVE_S`.
    trace_long: bool,
    // Per agent index: render id, where it stood still since when, and whether it was traced.
    still: Vec<(u64, f32, f32, f32, bool)>,
}

// The leaving waits a trace line is printed at.
const LONG_LEAVE_S: [f32; 2] = [300.0, 3600.0];

// What a car waiting `wait` s at its access mouth to cross faces: every body within 80 m of the
// crossing on each vehicle lane of its road, with its offset along that lane from the crossing
// (negative: upstream), speed, transit and own crossing wait.
fn trace_long_leave(core: &SimCore, i: usize, wait: f32) {
    use crate::simulation::buildings::allocator::BuildingAllocator;
    let agents = &core.agents.agents;
    let lanes = &core.transit_network.lane_system;
    let lane_id = agents.planned_attach_lane_id[i] as usize;
    let Some(lane) = lanes.lanes.get(lane_id) else {
        return;
    };
    let point = BuildingAllocator::sample_pos_on_lane(lane, agents.planned_attach_lane_d[i]);
    let mut rows = Vec::new();
    for &id in lanes.edge_lanes.get(&lane.edge_id).into_iter().flatten() {
        let other = &lanes.lanes[id];
        if other.lane_type != crate::simulation::network::lanes::LaneType::Vehicle {
            continue;
        }
        let d0 = BuildingAllocator::project_point_to_polyline_s(&other.geometry, point);
        let bodies: Vec<_> = core
            .agents
            .lane_buckets
            .get(id)
            .into_iter()
            .flatten()
            .filter(|entry| (entry.0 - d0).abs() <= 80.0)
            .map(|&(d, j, _)| {
                serde_json::json!([
                    (d - d0).round(), agents.speed[j], agents.transit[j],
                    agents.crossing_wait_s[j], agents.vehicle_type[j]
                ])
            })
            .collect();
        rows.push(serde_json::json!({
            "lane": id, "attach": id == lane_id, "idx": other.lane_idx, "d": d0.round(),
            "length": other.length.round(), "bodies": bodies,
        }));
    }
    eprintln!(
        "PARK32_LONG_LEAVE {}",
        serde_json::json!({
            "day": core.time.day_index, "agent": agents.render_id[i], "wait_s": wait,
            "truck": agents.vehicle_type[i] == VEHICLE_FREIGHT_DELIVERY, "edge": lane.edge_id,
            "lanes": rows,
        })
    );
}

impl CrossingBook {
    pub(super) fn new() -> Self {
        Self {
            trace_long: std::env::var("METRUM_AUDIT_LEAVE_TRACE").is_ok_and(|v| v == "1"),
            ..Self::default()
        }
    }

    // O(agents) per frame.
    pub(super) fn after_frame(&mut self, core: &SimCore) {
        let agents = &core.agents.agents;
        let now = core.agents.sim_time;
        let dt = now - self.last_time.unwrap_or(now);
        self.last_time = Some(now);
        self.rows.resize(agents.len(), Row::default());
        let on_road = |transit: u8| transit == TRANSIT_NETWORK || transit == TRANSIT_INTERSECTION;
        if self.trace_long {
            self.trace_still_egress(core, now);
        }
        for i in 0..agents.len() {
            let (wait, crossing) = crossing_state(agents, i);
            let transit = agents.transit[i];
            let id = agents.render_id[i];
            let row = &mut self.rows[i];
            if row.id != id || agents.transit_mode[i] != MODE_CAR {
                // Another agent took this index, or not a vehicle: start afresh.
                *row = Row {
                    id,
                    transit,
                    wait,
                    ..Row::default()
                };
                continue;
            }
            let class =
                &mut self.classes[usize::from(agents.vehicle_type[i] == VEHICLE_FREIGHT_DELIVERY)];
            if on_road(transit) {
                let step = agents.speed[i] * dt;
                row.metres += step;
                row.seconds += dt;
                class.driven_m += f64::from(step);
                if row.wait > 0.0 && wait == CROSSING_GAVE_UP {
                    self.give_ups += 1;
                    self.enter_waits.push(row.wait);
                }
                if row.transit == TRANSIT_ACCESS_EGRESS {
                    class.left += 1;
                    class.left_across += u64::from(row.left_across);
                    row.left_across = false;
                    row.metres = 0.0;
                    row.seconds = 0.0;
                }
            } else if transit == TRANSIT_ACCESS_INGRESS && on_road(row.transit) {
                class.trips += 1;
                class.metres += f64::from(row.metres);
                class.seconds += f64::from(row.seconds);
                class.entered += 1;
                class.entered_across += u64::from(crossing);
                if row.wait > 0.0 {
                    self.enter_waits.push(row.wait);
                }
                row.metres = 0.0;
                row.seconds = 0.0;
            } else if transit == TRANSIT_ACCESS_EGRESS {
                if self.trace_long
                    && !crossing
                    && LONG_LEAVE_S.iter().any(|&s| wait >= s && row.wait < s)
                {
                    trace_long_leave(core, i, wait);
                }
                // Only a give-up empties the wait without a crossing.
                if !crossing && row.wait > 0.0 && wait == 0.0 && row.transit == TRANSIT_ACCESS_EGRESS {
                    self.leave_give_ups += 1;
                    self.leave_waits.push(row.wait);
                }
                if crossing && !row.left_across {
                    row.left_across = true;
                    if row.wait > 0.0 {
                        self.leave_waits.push(row.wait);
                    }
                }
            } else {
                row.metres = 0.0;
                row.seconds = 0.0;
                row.left_across = false;
            }
            row.transit = transit;
            row.wait = wait;
        }
    }

    // `PARK32_STILL_EGRESS`: one line per agent that has stood an hour in the access-egress
    // state, whatever its mode: what it is doing.
    fn trace_still_egress(&mut self, core: &SimCore, now: f32) {
        let agents = &core.agents.agents;
        self.still.resize(agents.len(), (u64::MAX, 0.0, 0.0, 0.0, false));
        for i in 0..agents.len() {
            let (id, x, y) = (agents.render_id[i], agents.pos_x[i], agents.pos_y[i]);
            let row = &mut self.still[i];
            if agents.transit[i] != TRANSIT_ACCESS_EGRESS {
                row.0 = u64::MAX;
                continue;
            }
            if row.0 != id || (row.1 - x).hypot(row.2 - y) > 0.5 {
                *row = (id, x, y, now, false);
                continue;
            }
            if !row.4 && now - row.3 >= 3600.0 {
                row.4 = true;
                let trip = agents.parking_trip[i];
                eprintln!(
                    "PARK32_STILL_EGRESS {}",
                    serde_json::json!({
                        "day": core.time.day_index, "agent": id, "mode": agents.transit_mode[i],
                        "vehicle_type": agents.vehicle_type[i], "freight": agents.freight_shipment_id[i] != u64::MAX,
                        "trip": format!("{trip:?}"), "crossing_wait": agents.crossing_wait_s[i],
                        "crossing_lane": agents.crossing_lane[i], "attach": [agents.planned_attach_lane_id[i], agents.planned_attach_lane_d[i]],
                        "flags": agents.access_flags[i], "path_index": agents.current_path_index[i],
                        "path_len": agents.current_path[i].len(), "building": agents.current_building[i],
                        "pos": [x, y], "next_replan": agents.next_replan_time[i], "now": now,
                    })
                );
            }
        }
    }

    // Prints the day's figures and forgets them.
    pub(super) fn report(&mut self, core: &SimCore) {
        let waits = |waits: &mut Vec<f32>| {
            let count = waits.len();
            let mean = waits.iter().map(|&w| f64::from(w)).sum::<f64>() / (count.max(1) as f64);
            let max = waits.iter().copied().fold(0.0_f32, f32::max);
            waits.clear();
            serde_json::json!({"count": count, "mean_s": mean, "max_s": max})
        };
        let mut classes = serde_json::Map::new();
        for (name, class) in CLASSES.iter().zip(&mut self.classes) {
            let trips = class.trips.max(1) as f64;
            classes.insert(
                (*name).to_string(),
                serde_json::json!({
                    "trips": class.trips, "mean_m": class.metres / trips,
                    "mean_s": class.seconds / trips, "vehicle_km": class.driven_m / 1000.0,
                    "entered": class.entered, "entered_across": class.entered_across,
                    "left": class.left, "left_across": class.left_across,
                }),
            );
            *class = Class::default();
        }
        eprintln!(
            "PARK32_CROSSINGS {}",
            serde_json::json!({
                "day": core.time.day_index, "classes": classes,
                "give_ups": std::mem::take(&mut self.give_ups),
                "leave_give_ups": std::mem::take(&mut self.leave_give_ups),
                "enter_waits": waits(&mut self.enter_waits),
                "leave_waits": waits(&mut self.leave_waits),
            })
        );
    }
}
