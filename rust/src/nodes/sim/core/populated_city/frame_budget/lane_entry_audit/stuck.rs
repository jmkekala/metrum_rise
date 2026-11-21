// SPDX-License-Identifier: GPL-3.0-or-later

//! `TRAFFIC-38` flow check of the lane-entry audit: does traffic flow, or do vehicles stay stuck
//! for good? Every game minute it records where each agent on a trip stands; an agent that has
//! not moved more than [`MOVE_M`] from where it stopped is stationary since then. Every game hour
//! it closes the lanes' throughput: a lane occupied at every sample of the hour that no vehicle
//! left is a zero-throughput lane-hour. At each day end it prints `TRAFFIC38_FLOW_DAY`: the
//! stationary agents by threshold and kind, what the stationary vehicles wait for and where the
//! blocker chains of the long waits end, how many long waits were already long at the previous
//! day end, the full lanes' heads, trip ages and the agents in no known trip state; and
//! `TRAFFIC38_ECON_DAY`, the day's economy beside it. Read-only diagnostics over the agent
//! columns; nothing here touches the simulation. O(agents) per game minute, serial.

use super::waits::{Occupancy, Outcome, Step, resolve};
use crate::config::IDM_S_MIN;
use crate::nodes::sim::core::state::SimCore;
use crate::simulation::economy::agents::tick::lane_half_length;
use crate::simulation::economy::agents::{
    ACCESS_LANE_ATTACH_PENDING, ACCESS_PLAN_VALID, AgentVec, MODE_WALK, TRANSIT_ACCESS_EGRESS,
    TRANSIT_ACCESS_INGRESS, TRANSIT_INTERSECTION, TRANSIT_NETWORK, VEHICLE_FREIGHT_DELIVERY,
};
use std::collections::{BTreeMap, HashMap, HashSet};

// Stationary thresholds in game minutes (one game minute is one simulated second).
const THRESHOLDS_MIN: [u64; 5] = [10, 60, 240, 720, 1440];
// An agent that moved less than this from where it stopped has not moved.
const MOVE_M: f32 = 0.5;
// Long waits whose blocker chain is followed and sampled at a day end.
const CHAIN_MIN: u64 = 240;
const SAMPLES: usize = 12;

// Half the length an agent's body takes in a lane.
fn body_half(agents: &AgentVec, i: usize) -> f32 {
    lane_half_length(agents.transit_mode[i], agents.vehicle_type[i])
}

fn on_trip(transit: u8) -> bool {
    matches!(
        transit,
        TRANSIT_NETWORK | TRANSIT_INTERSECTION | TRANSIT_ACCESS_EGRESS | TRANSIT_ACCESS_INGRESS
    )
}

fn kind(agents: &AgentVec, i: usize) -> &'static str {
    if agents.transit_mode[i] == MODE_WALK {
        "walker"
    } else if agents.vehicle_type[i] == VEHICLE_FREIGHT_DELIVERY {
        "truck"
    } else {
        "car"
    }
}

struct Still {
    // Where the agent stopped, and since when (game minute).
    x: f32,
    y: f32,
    since: u64,
    // First minute the audit saw this trip (agents on the road at the start count from there).
    trip_since: u64,
    seen: u64,
}

#[derive(Default)]
pub(super) struct FlowBook {
    still: HashMap<u64, Still>,
    // Vehicle id -> (lane at the last sample, minute of that sample).
    lane_of: HashMap<u64, (usize, u64)>,
    // This hour's samples, the samples each lane was occupied in, its exits and a dedup stamp.
    hour: u64,
    samples: u16,
    occupied: Vec<u16>,
    exits: Vec<u32>,
    stamp: Vec<u64>,
    // Today's zero-throughput hours per lane, and the hourly peaks of stationary vehicles.
    zero_hours: BTreeMap<usize, u32>,
    hourly: Vec<[u32; THRESHOLDS_MIN.len()]>,
    // The vehicle ids stationary over 60 and 240 minutes at the previous day end.
    previous_60: HashSet<u64>,
    previous_240: HashSet<u64>,
    // Work counters and treasury at the previous day end.
    day_work: Option<[u64; crate::debug::work::WORK_COUNT]>,
    day_treasury: f64,
}

impl FlowBook {
    /// Prints the fixture's short lanes against the body lengths once.
    pub(super) fn short_lanes(core: &SimCore) {
        let lanes = &core.transit_network.lane_system.lanes;
        let truck = 2.0 * lane_half_length(1, VEHICLE_FREIGHT_DELIVERY) + IDM_S_MIN;
        let car = 2.0 * lane_half_length(1, 0) + IDM_S_MIN;
        let mut rows = BTreeMap::<&str, [u32; 4]>::new();
        let mut shortest = BTreeMap::<&str, f32>::new();
        for lane in lanes {
            if lane.lane_type != crate::simulation::network::lanes::LaneType::Vehicle {
                continue;
            }
            let class = match (lane.edge_id == usize::MAX, lane.ring_entry.is_some()) {
                (true, true) => "ring_entry_connector",
                (true, false) => "connector",
                (false, _) => "road_lane",
            };
            let row = rows.entry(class).or_default();
            row[0] += 1;
            row[1] += u32::from(lane.length < car);
            row[2] += u32::from(lane.length < truck);
            row[3] += u32::from(lane.length < 2.0 * truck);
            let low = shortest.entry(class).or_insert(f32::MAX);
            *low = low.min(lane.length);
        }
        eprintln!(
            "TRAFFIC38_SHORT_LANES {}",
            serde_json::json!({
                "car_spacing_m": car, "truck_spacing_m": truck,
                "columns": ["lanes", "under_car_spacing", "under_truck_spacing", "under_two_trucks"],
                "rows": rows, "shortest_m": shortest,
            })
        );
    }

    /// Samples every agent on a trip once per game minute. O(agents + vehicles on lanes).
    pub(super) fn sample(&mut self, core: &SimCore, now: u64) {
        let agents = &core.agents.agents;
        let lane_count = core.transit_network.lane_system.lanes.len();
        if self.occupied.len() != lane_count {
            self.occupied = vec![0; lane_count];
            self.exits = vec![0; lane_count];
            self.stamp = vec![u64::MAX; lane_count];
            self.lane_of.clear();
        }
        if now / 60 != self.hour {
            self.close_hour(now);
        }
        self.samples += 1;
        for i in 0..agents.len() {
            let transit = agents.transit[i];
            if !on_trip(transit) {
                continue;
            }
            let id = agents.render_id[i];
            let (x, y) = (agents.pos_x[i], agents.pos_y[i]);
            let still = self.still.entry(id).or_insert(Still {
                x,
                y,
                since: now,
                trip_since: now,
                seen: now,
            });
            if (x - still.x).hypot(y - still.y) > MOVE_M {
                (still.x, still.y, still.since) = (x, y, now);
            }
            still.seen = now;
            let lane = agents.current_lane_id[i];
            if agents.transit_mode[i] != MODE_WALK
                && (transit == TRANSIT_NETWORK || transit == TRANSIT_INTERSECTION)
                && lane < lane_count
            {
                if self.stamp[lane] != now {
                    self.stamp[lane] = now;
                    self.occupied[lane] += 1;
                }
                if let Some((previous, _)) = self.lane_of.insert(id, (lane, now))
                    && previous != lane
                    && previous < lane_count
                {
                    self.exits[previous] += 1;
                }
            }
        }
        self.still.retain(|_, still| still.seen == now);
        // Vehicles no longer on a lane left the lane they were on.
        let exits = &mut self.exits;
        self.lane_of.retain(|_, &mut (lane, seen)| {
            if seen != now && lane < exits.len() {
                exits[lane] += 1;
            }
            seen == now
        });
    }

    fn close_hour(&mut self, now: u64) {
        if self.samples > 0 {
            for lane in 0..self.occupied.len() {
                if self.occupied[lane] == self.samples && self.exits[lane] == 0 {
                    *self.zero_hours.entry(lane).or_default() += 1;
                }
            }
            // Stationary agents now, walkers included, by threshold.
            let mut counts = [0_u32; THRESHOLDS_MIN.len()];
            for still in self.still.values() {
                let waited = now.saturating_sub(still.since);
                for (count, &threshold) in counts.iter_mut().zip(&THRESHOLDS_MIN) {
                    *count += u32::from(waited >= threshold);
                }
            }
            self.hourly.push(counts);
        }
        self.occupied.iter_mut().for_each(|count| *count = 0);
        self.exits.iter_mut().for_each(|count| *count = 0);
        self.samples = 0;
        self.hour = now / 60;
    }

    /// Prints the day's flow line and economy line. O(agents + lanes + long-wait chains).
    pub(super) fn report_day(&mut self, core: &SimCore, now: u64) {
        let agents = &core.agents.agents;
        let lanes = &core.transit_network.lane_system.lanes;
        let occ = Occupancy::new(core);
        let mut steps = HashMap::new();
        let mut memo = HashMap::new();
        // Stationary agents by threshold and kind.
        let mut by_threshold = BTreeMap::<String, BTreeMap<&str, u32>>::new();
        // What stationary vehicles (over 60 min) wait for, and the chain ends of waits over 240.
        let mut waits_for = BTreeMap::<String, u32>::new();
        let mut chain_ends = BTreeMap::<String, u32>::new();
        let mut samples = Vec::new();
        let mut over_60 = HashSet::new();
        let mut over_240 = HashSet::new();
        let mut trip_ages = BTreeMap::<String, BTreeMap<&str, u32>>::new();
        let mut longest = (0_u64, String::new());
        let mut long_trips = BTreeMap::<String, u32>::new();
        let mut long_trip_samples = Vec::new();
        let mut ids: Vec<(&u64, &Still)> = self.still.iter().collect();
        ids.sort_by_key(|(id, still)| (still.since, **id));
        for (&id, still) in ids {
            let Some(&i) = occ.index_of.get(&id) else {
                continue;
            };
            let k = kind(agents, i);
            let waited = now.saturating_sub(still.since);
            let age = now.saturating_sub(still.trip_since);
            for &threshold in &THRESHOLDS_MIN {
                if waited >= threshold {
                    *by_threshold
                        .entry(format!("{threshold:04}"))
                        .or_default()
                        .entry(k)
                        .or_default() += 1;
                }
                if age >= threshold {
                    *trip_ages
                        .entry(format!("{threshold:04}"))
                        .or_default()
                        .entry(k)
                        .or_default() += 1;
                }
            }
            if age >= 1440 {
                // Trips the audit has seen for a whole day: where they stand and whether they move.
                let state = format!(
                    "{k} transit {} {} plan {} target {}",
                    agents.transit[i],
                    place_class(&occ, i),
                    (agents.access_flags[i] & ACCESS_PLAN_VALID) != 0,
                    agents.target_building[i] != usize::MAX,
                );
                *long_trips.entry(state).or_default() += 1;
                if long_trip_samples.len() < SAMPLES {
                    long_trip_samples.push(serde_json::json!({
                        "trip_min": age, "stationary_min": waited, "agent": occ.describe(i),
                        "path": format!("{}/{}", agents.current_path_index[i], agents.current_path[i].len()),
                        "home": agents.home_building[i], "target": agents.target_building[i],
                        "access": agents.access_flags[i], "mode": agents.transit_mode[i],
                    }));
                }
            }
            if k == "walker" || waited < 60 {
                continue;
            }
            over_60.insert(id);
            let class = self.wait_class(&occ, &mut steps, i);
            *waits_for.entry(format!("{k}: {class}")).or_default() += 1;
            if waited > longest.0 {
                longest = (waited, occ.describe(i));
            }
            if waited < CHAIN_MIN {
                continue;
            }
            over_240.insert(id);
            let outcome = resolve(&occ, &mut steps, &mut memo, i);
            let end = match &outcome {
                Outcome::Moving(car) => format!("moving ({})", place_class(&occ, *car)),
                Outcome::Unexplained(_, why) => format!("unexplained ({})", strip_digits(why)),
                Outcome::Cycle(cycle) => format!("cycle of {} vehicles", cycle.len()),
            };
            *chain_ends.entry(end.clone()).or_default() += 1;
            if samples.len() < SAMPLES {
                samples.push(serde_json::json!({
                    "stationary_min": waited, "trip_min": age, "vehicle": occ.describe(i),
                    "waits_for": class, "chain_end": end,
                }));
            }
        }
        let kept_60 = over_60.intersection(&self.previous_60).count();
        let kept_240 = over_240.intersection(&self.previous_240).count();
        let turnover = serde_json::json!({
            "over_60_now": over_60.len(), "over_60_previous_day_end": self.previous_60.len(),
            "over_60_also_previous": kept_60,
            "over_240_now": over_240.len(), "over_240_previous_day_end": self.previous_240.len(),
            "over_240_also_previous": kept_240,
        });
        self.previous_60 = over_60;
        self.previous_240 = over_240;
        let mut peaks = [0_u32; THRESHOLDS_MIN.len()];
        for hour in &self.hourly {
            for (peak, &count) in peaks.iter_mut().zip(hour) {
                *peak = (*peak).max(count);
            }
        }
        let hourly_over_60: Vec<u32> = self.hourly.iter().map(|hour| hour[1]).collect();
        self.hourly.clear();
        // Zero-throughput lane-hours: road lanes and connectors, the most repeated lanes.
        let (mut road_hours, mut connector_hours) = (0_u32, 0_u32);
        for (&lane, &hours) in &self.zero_hours {
            if lanes.get(lane).is_some_and(|l| l.edge_id == usize::MAX) {
                connector_hours += hours;
            } else {
                road_hours += hours;
            }
        }
        let mut repeated: Vec<(usize, u32)> =
            self.zero_hours.iter().map(|(&l, &h)| (l, h)).collect();
        repeated.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let repeated: Vec<serde_json::Value> = repeated
            .iter()
            .take(8)
            .map(|&(lane, hours)| {
                let l = &lanes[lane];
                serde_json::json!({
                    "lane": lane, "hours": hours, "edge": l.edge_id as i64,
                    "node": if l.edge_id == usize::MAX { l.node_id as i64 } else { -1 },
                    "length": l.length, "now": occ.buckets[lane].len(),
                })
            })
            .collect();
        let zero = serde_json::json!({
            "lanes": self.zero_hours.len(), "road_lane_hours": road_hours,
            "connector_hours": connector_hours, "most_repeated": repeated,
        });
        self.zero_hours.clear();
        let full = self.full_lanes(&occ, &mut steps, now);
        let limbo = limbo(agents, lanes.len());
        let (mut cars, mut trucks, mut walkers) = (0_u32, 0_u32, 0_u32);
        for i in 0..agents.len() {
            if on_trip(agents.transit[i]) {
                match kind(agents, i) {
                    "walker" => walkers += 1,
                    "truck" => trucks += 1,
                    _ => cars += 1,
                }
            }
        }
        eprintln!(
            "TRAFFIC38_FLOW_DAY {}",
            serde_json::json!({
                "day": core.time.day_index, "minute": now,
                "en_route": {"car": cars, "truck": trucks, "walker": walkers},
                "stationary": by_threshold, "hourly_peak": peaks,
                "hourly_over_60": hourly_over_60, "trip_age": trip_ages,
                "vehicles_over_60_wait_for": waits_for, "chain_ends_over_240": chain_ends,
                "turnover": turnover, "zero_throughput": zero, "full_lanes": full,
                "limbo": limbo, "longest": {"min": longest.0, "vehicle": longest.1},
                "samples_over_240": samples,
                "trips_over_1440": long_trips, "trips_over_1440_samples": long_trip_samples,
            })
        );
        self.report_economy(core);
    }

    // What a stationary vehicle waits for, from the audit's blocker step.
    fn wait_class(
        &self,
        occ: &Occupancy<'_>,
        steps: &mut HashMap<usize, Step>,
        i: usize,
    ) -> String {
        let agents = occ.agents();
        let transit = agents.transit[i];
        if transit == TRANSIT_ACCESS_EGRESS {
            return "driveway egress (access state)".into();
        }
        if transit == TRANSIT_ACCESS_INGRESS {
            return "parking ingress (access state)".into();
        }
        if agents.current_lane_id[i] == usize::MAX
            && (agents.access_flags[i] & ACCESS_LANE_ATTACH_PENDING) == 0
            && agents.current_path[i].is_empty()
        {
            return "network with no lane and no route (limbo)".into();
        }
        let lanes = &occ.core().transit_network.lane_system.lanes;
        let step = steps.entry(i).or_insert_with(|| occ.step(i));
        match step {
            Step::Moving => "creeping (speed over 0.5 m/s)".into(),
            Step::Unexplained(why) => format!("unexplained: {}", strip_digits(why)),
            Step::Blocked(_, via) => {
                let lane = via
                    .split_whitespace()
                    .find_map(|word| word.parse::<usize>().ok())
                    .and_then(|lane| lanes.get(lane));
                let target = match lane {
                    Some(l) if l.ring_entry.is_some() => "ring entry connector",
                    Some(l) if l.edge_id == usize::MAX => "junction connector",
                    Some(_) => "road lane",
                    None => "?",
                };
                if via.starts_with("queued behind") {
                    format!("leader on {}", place_class(occ, i))
                } else if via.starts_with("attach slot") {
                    "attach slot (driveway or kerb pull-out)".into()
                } else if via.starts_with("first-lane entry") {
                    format!("first-lane entry at a node ({target})")
                } else {
                    format!("lane-end entry ({target})")
                }
            }
        }
    }

    // Lanes whose bodies need more than their length, with their heads' stationary times and
    // what the long-stationary heads wait for.
    fn full_lanes(
        &self,
        occ: &Occupancy<'_>,
        steps: &mut HashMap<usize, Step>,
        now: u64,
    ) -> serde_json::Value {
        let agents = occ.agents();
        let lanes = &occ.core().transit_network.lane_system.lanes;
        let (mut full, mut head_over) = (0_u32, [0_u32; THRESHOLDS_MIN.len()]);
        let mut head_waits = BTreeMap::<String, u32>::new();
        for (lane, bucket) in occ.buckets.iter().enumerate() {
            if bucket.len() < 2 {
                continue;
            }
            let need: f32 = bucket
                .iter()
                .map(|entry| 2.0 * body_half(agents, entry.1) + IDM_S_MIN)
                .sum();
            if need <= lanes[lane].length {
                continue;
            }
            full += 1;
            let head = bucket[bucket.len() - 1].1;
            let waited = self
                .still
                .get(&agents.render_id[head])
                .map_or(0, |still| now.saturating_sub(still.since));
            for (count, &threshold) in head_over.iter_mut().zip(&THRESHOLDS_MIN) {
                *count += u32::from(waited >= threshold);
            }
            if waited >= 60 {
                let class = self.wait_class(occ, steps, head);
                *head_waits
                    .entry(format!("{}: {class}", kind(agents, head)))
                    .or_default() += 1;
            }
        }
        serde_json::json!({
            "lanes": full, "head_stationary_over": head_over, "heads_over_60_wait_for": head_waits,
        })
    }

    fn report_economy(&mut self, core: &SimCore) {
        use crate::debug::work::{Work, snapshot};
        let work = snapshot();
        let start = self.day_work.unwrap_or(work);
        let delta = |counter: Work| work[counter as usize] - start[counter as usize];
        let city = core.daily_city_flow_diagnostics();
        let treasury = core.treasury.balance;
        eprintln!(
            "TRAFFIC38_ECON_DAY {}",
            serde_json::json!({
                "day": core.time.day_index, "employed": city.employed_agents,
                "jobs": city.commercial_job_capacity + city.industrial_job_capacity,
                "residents": city.resident_agents,
                "net_income": if self.day_work.is_some() { treasury - self.day_treasury } else { 0.0 },
                "freight_deliveries": delta(Work::FreightDeliveries),
                "trip_failures": delta(Work::TripFailures),
                "bankruptcies": delta(Work::Desertions),
            })
        );
        self.day_work = Some(work);
        self.day_treasury = treasury;
    }

    /// Starts the economy counters at the audit's start.
    pub(super) fn start(&mut self, core: &SimCore) {
        self.day_work = Some(crate::debug::work::snapshot());
        self.day_treasury = core.treasury.balance;
    }
}

// Road lane or connector, at its end or not.
fn place_class(occ: &Occupancy<'_>, i: usize) -> &'static str {
    let agents = occ.agents();
    let lanes = &occ.core().transit_network.lane_system.lanes;
    match lanes.get(agents.current_lane_id[i]) {
        Some(l) if l.edge_id == usize::MAX => "connector",
        Some(_) => "road lane",
        None => "no lane (held)",
    }
}

// The text with each number replaced by `#`, so reasons group by kind.
fn strip_digits(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_number = false;
    for c in text.chars() {
        let digit = c.is_ascii_digit() || (in_number && c == '.');
        if digit && !in_number {
            out.push('#');
        } else if !digit {
            out.push(c);
        }
        in_number = digit;
    }
    out
}

// Agents on a trip in no state a tick moves: on the network with no lane, no pending attach and
// no route (a vehicle or walker the movement pass has nothing to do with).
fn limbo(agents: &AgentVec, lane_count: usize) -> serde_json::Value {
    let mut counts = BTreeMap::<&str, u32>::new();
    for i in 0..agents.len() {
        let transit = agents.transit[i];
        if (transit == TRANSIT_NETWORK || transit == TRANSIT_INTERSECTION)
            && agents.current_lane_id[i] >= lane_count
            && (agents.access_flags[i] & ACCESS_LANE_ATTACH_PENDING) == 0
            && agents.current_path[i].is_empty()
        {
            *counts.entry(kind(agents, i)).or_default() += 1;
        }
    }
    serde_json::json!(counts)
}
