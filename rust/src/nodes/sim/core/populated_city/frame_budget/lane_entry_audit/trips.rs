// SPDX-License-Identifier: GPL-3.0-or-later

//! `TRAFFIC-38` trip times of the lane-entry audit: how long the day's finished trips took, on one
//! `TRAFFIC38_TRIPS` line at the audit's end, with the parking departure outcomes beside them. A
//! resident's trip ends when it enters a building, timed from its `journey_start_time`; it drove
//! when it arrives with `has_car`, and it is a commute when the building is its workplace. A
//! freight carrier's trip ends when it leaves the store (its delivery removes it), timed from the
//! first frame it was seen on the road. Read-only: nothing here touches the simulation.

use crate::nodes::sim::core::state::SimCore;
use crate::simulation::economy::agents::{
    TRANSIT_IMMIGRATING, TRANSIT_IN_BUILDING, VEHICLE_FREIGHT_DELIVERY,
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

pub(super) struct TripBook {
    // Per agent index: its render id and transit at the previous frame.
    previous: Vec<(u64, u8)>,
    // Freight carriers out of their building: render id to the time first seen.
    freight: HashMap<u64, f32>,
    freight_now: HashMap<u64, f32>,
    durations: BTreeMap<&'static str, Vec<f32>>,
}

impl TripBook {
    pub(super) fn new() -> Self {
        Self {
            previous: Vec::new(),
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
                self.durations.entry(kind).or_default().push(took);
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
                "freight_on_road_at_end": self.freight.len(),
                "parking_departures": departures,
                "parking_vehicles": {"parked": vehicles.parked, "moving": vehicles.moving,
                    "borrowed": vehicles.borrowed, "reserved": vehicles.reserved},
            })
        );
    }
}
