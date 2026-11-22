// SPDX-License-Identifier: GPL-3.0-or-later

//! `TRAFFIC-38` trip times of the lane-entry audit: how long the day's finished trips took, on one
//! `TRAFFIC38_TRIPS` line at the audit's end, with the parking departure outcomes beside them. A
//! resident's trip ends when it enters a building, timed from its `journey_start_time`; it drove
//! when it arrives with `has_car`, and it is a commute when the building is its workplace. A
//! resident evicted at home settles with no building, which is no trip (`TRAFFIC-42`). A
//! freight carrier's trip ends when it leaves the store (its delivery removes it), timed from the
//! first frame it was seen on the road. Read-only: nothing here touches the simulation.
//!
//! `has_car` says whether the resident may use its household's car, not whether this trip used
//! it, so the same trips are also filed by how they were made (`TRAFFIC-41`): by car when the
//! resident was a car on the road at any frame of the trip, else on foot, and by where they ended
//! (work, home, elsewhere), with the trips over 4, 12 and 24 game hours counted.

use crate::nodes::sim::core::state::SimCore;
use crate::simulation::economy::agents::{
    MODE_CAR, TRANSIT_IMMIGRATING, TRANSIT_IN_BUILDING, VEHICLE_FREIGHT_DELIVERY,
};
use crate::simulation::parking::stats::DepartureOutcome;
use std::collections::{BTreeMap, HashMap};

// Trip kinds, in report order.
const KINDS: [&str; 6] = [
    "drove",
    "walked",
    "commute_drove",
    "commute_walked",
    "household_arrival",
    "freight",
];

// Trips by how they were made and where they ended, in report order.
const MADE: [&str; 6] = [
    "foot_work",
    "foot_home",
    "foot_other",
    "car_work",
    "car_home",
    "car_other",
];
// Long trips sampled a day.
const LONG_TRIP_SAMPLES: usize = 8;

pub(super) struct TripBook {
    // Per agent index: its render id and transit at the previous frame.
    previous: Vec<(u64, u8)>,
    // Per agent index: whether it was a car on the road at any frame of its trip under way.
    drove: Vec<bool>,
    // Finished trips' seconds by `MADE` kind.
    made: BTreeMap<&'static str, Vec<f32>>,
    // Unhoused residents settled with no building since the last report: no trip.
    settled_unhoused: u32,
    // Trips over a game day that ended since the last report.
    long_trips: Vec<serde_json::Value>,
    // Freight carriers out of their building: render id to the time first seen.
    freight: HashMap<u64, f32>,
    freight_now: HashMap<u64, f32>,
    durations: BTreeMap<&'static str, Vec<f32>>,
}

impl TripBook {
    pub(super) fn new() -> Self {
        Self {
            previous: Vec::new(),
            drove: Vec::new(),
            made: MADE.iter().map(|&kind| (kind, Vec::new())).collect(),
            settled_unhoused: 0,
            long_trips: Vec::new(),
            freight: HashMap::new(),
            freight_now: HashMap::new(),
            durations: KINDS.iter().map(|&kind| (kind, Vec::new())).collect(),
        }
    }

    // O(agents) per frame.
    pub(super) fn after_frame(&mut self, core: &SimCore) {
        let agents = &core.agents.agents;
        let now = core.agents.sim_time;
        self.previous
            .resize(agents.len(), (u64::MAX, TRANSIT_IN_BUILDING));
        self.drove.resize(agents.len(), false);
        let day_s = core.time.seconds_per_day as f32;
        self.freight_now.clear();
        for i in 0..agents.len() {
            let id = agents.render_id[i];
            let transit = agents.transit[i];
            if agents.vehicle_type[i] == VEHICLE_FREIGHT_DELIVERY {
                if transit != TRANSIT_IN_BUILDING {
                    let since = self.freight.get(&id).copied().unwrap_or(now);
                    self.freight_now.insert(id, since);
                }
            } else if transit == TRANSIT_IN_BUILDING
                && self.previous[i].0 == id
                && self.previous[i].1 != TRANSIT_IN_BUILDING
            {
                let commute = agents.current_building[i] == agents.work_building[i];
                let kind = match (
                    self.previous[i].1 == TRANSIT_IMMIGRATING,
                    agents.has_car[i],
                    commute,
                ) {
                    (true, _, _) => "household_arrival",
                    (false, true, false) => "drove",
                    (false, false, false) => "walked",
                    (false, true, true) => "commute_drove",
                    (false, false, true) => "commute_walked",
                };
                let took = now - agents.journey_start_time[i];
                let building = agents.current_building[i];
                if building == usize::MAX {
                    // Evicted at home and settled with no building: it went nowhere.
                    self.settled_unhoused += 1;
                } else {
                    self.durations.entry(kind).or_default().push(took);
                }
                if building != usize::MAX && self.previous[i].1 != TRANSIT_IMMIGRATING {
                    let end = if building == agents.work_building[i] {
                        0
                    } else if building == agents.home_building[i] {
                        1
                    } else {
                        2
                    };
                    let made = MADE[usize::from(self.drove[i]) * 3 + end];
                    self.made.entry(made).or_default().push(took);
                    if took > day_s && self.long_trips.len() < LONG_TRIP_SAMPLES {
                        self.long_trips.push(serde_json::json!({
                            "agent": i, "made": made, "trip_s": took,
                            "household": agents.household_id[i], "home": agents.home_building[i],
                            "work": agents.work_building[i], "building": building,
                            "age_group": agents.age_group[i], "has_car": agents.has_car[i],
                            "activity": agents.activity[i],
                        }));
                    }
                }
                self.drove[i] = false;
            } else if self.previous[i].0 != id {
                // Another agent took this index.
                self.drove[i] = false;
            } else if transit != TRANSIT_IN_BUILDING && agents.transit_mode[i] == MODE_CAR {
                self.drove[i] = true;
            }
            self.previous[i] = (id, transit);
        }
        // Carriers gone from the road since the last frame delivered (or were removed).
        for (id, since) in &self.freight {
            if !self.freight_now.contains_key(id) {
                self.durations
                    .entry("freight")
                    .or_default()
                    .push(now - since);
            }
        }
        std::mem::swap(&mut self.freight, &mut self.freight_now);
    }

    /// Prints the trips finished since the previous report and forgets them.
    pub(super) fn report(&mut self, core: &SimCore) {
        let mut trips = serde_json::Map::new();
        for (kind, durations) in &mut self.durations {
            durations.sort_unstable_by(f32::total_cmp);
            let at = |share: f32| {
                durations
                    .get(((durations.len() as f32 - 1.0) * share).round().max(0.0) as usize)
                    .copied()
                    .unwrap_or(0.0)
            };
            let mean = durations.iter().map(|&d| f64::from(d)).sum::<f64>()
                / (durations.len().max(1) as f64);
            trips.insert(
                (*kind).to_string(),
                serde_json::json!({"count": durations.len(), "mean_s": mean, "p50_s": at(0.5),
                    "p95_s": at(0.95), "max_s": at(1.0)}),
            );
            durations.clear();
        }
        // The same trips by how they were made, with the long ones counted in game hours.
        let hour_s = core.time.seconds_per_day as f32 / 24.0;
        let mut made = serde_json::Map::new();
        for (kind, durations) in &mut self.made {
            durations.sort_unstable_by(f32::total_cmp);
            let at = |share: f32| {
                durations
                    .get(((durations.len() as f32 - 1.0) * share).round().max(0.0) as usize)
                    .copied()
                    .unwrap_or(0.0)
            };
            let over = |hours: f32| durations.iter().filter(|&&d| d > hours * hour_s).count();
            let mean = durations.iter().map(|&d| f64::from(d)).sum::<f64>()
                / (durations.len().max(1) as f64);
            made.insert(
                (*kind).to_string(),
                serde_json::json!({"count": durations.len(), "mean_s": mean, "p50_s": at(0.5),
                    "p95_s": at(0.95), "max_s": at(1.0), "over_4h": over(4.0),
                    "over_12h": over(12.0), "over_24h": over(24.0)}),
            );
            durations.clear();
        }
        let settled_unhoused = std::mem::take(&mut self.settled_unhoused);
        let long_trips = std::mem::take(&mut self.long_trips);
        let parking = &core.parking;
        let departures: BTreeMap<&str, u64> = DepartureOutcome::ALL
            .iter()
            .map(|&outcome| (outcome.label(), parking.departures.count(outcome)))
            .collect();
        let vehicles = parking.vehicle_breakdown();
        eprintln!(
            "TRAFFIC38_TRIPS {}",
            serde_json::json!({
                "day": core.time.day_index, "trips": trips,
                "made": made, "game_hour_s": hour_s, "settled_unhoused": settled_unhoused,
                "long_trips": long_trips,
                "freight_on_road_at_end": self.freight.len(),
                "parking_departures": departures,
                "parking_vehicles": {"parked": vehicles.parked, "moving": vehicles.moving,
                    "borrowed": vehicles.borrowed, "reserved": vehicles.reserved},
            })
        );
    }
}
