// SPDX-License-Identifier: GPL-2.0-only

//! Logging trip failures after the movement pass, with flood control.
//!
//! Three rules keep the `events` ring readable when a road edit strands many agents:
//! - An agent that repeats the same failure at the same place within a simulated minute of
//!   the last one is not logged again. The watchdog's recovery counts as a different failure,
//!   so it is logged after the quiet retries before it.
//! - Each simulated minute logs at most [`DETAIL_PER_PLACE`] lines per cause and place, and
//!   [`DETAIL_PER_MINUTE`] in total.
//! - The failures past those limits are counted per cause and logged as one summary each when
//!   the minute ends.
//!
//! The inspected agent's failures are always logged in full. State is O(failing agents) and
//! is only touched when something fails, apart from one minute check per tick.

use super::super::super::data::AgentSystem;
use super::text::{CAUSE_LABELS, cause_at_building, cause_index, failure_line};
use super::{FailureCause, TripFailure};
use crate::debug_log;
use crate::simulation::buildings::allocator::BuildingAllocator;
use std::collections::HashMap;

/// Most detailed lines per simulated minute for one cause at one place.
pub(crate) const DETAIL_PER_PLACE: u32 = 20;
/// Most detailed lines per simulated minute in total.
pub(crate) const DETAIL_PER_MINUTE: u32 = 200;
const MINUTE_S: f32 = 60.0;
const CAUSES: usize = CAUSE_LABELS.len();

// A cause and where it happened: a node, or a building for failures at a door.
type Place = (u8, u32);

#[derive(Clone, Copy, Default)]
struct Folded {
    count: u32,
    first_render_id: u64,
}

/// Flood-control state and the reused collection buffer. Never saved.
#[derive(Default)]
pub(crate) struct TripFailureLog {
    pub(in crate::simulation::economy::agents::tick) buffer: Vec<TripFailure>,
    minute: i64,
    // When each agent last failed this way: (render id, place, recovery) -> simulated seconds.
    streaks: HashMap<(u64, Place, u8), f32>,
    // Failures this minute per place: (counted, folded into the summary).
    places: HashMap<Place, (u32, u32)>,
    detailed: u32,
    folded: [Folded; CAUSES],
    /// Every line logged, for tests.
    #[cfg(test)]
    pub(crate) lines: Vec<String>,
}

impl TripFailureLog {
    fn emit(&mut self, line: String) {
        debug_log!("events", "{line}");
        #[cfg(test)]
        self.lines.push(line);
    }

    // Ends the minute once simulated time has moved past it: logs its summaries and forgets
    // streaks older than a minute.
    fn roll_minute(&mut self, sim_time: f32) {
        let minute = (sim_time / MINUTE_S).floor() as i64;
        if minute == self.minute {
            return;
        }
        self.minute = minute;
        if self.places.is_empty() && self.streaks.is_empty() {
            return;
        }
        for (cause, label) in CAUSE_LABELS.iter().enumerate() {
            let Folded {
                count,
                first_render_id,
            } = self.folded[cause];
            if count == 0 {
                continue;
            }
            // The place with the most folded failures; the lowest place wins a tie.
            let (place, at_place) = self
                .places
                .iter()
                .filter(|(place, (_, folded))| usize::from(place.0) == cause && *folded > 0)
                .map(|(place, (_, folded))| (place.1, *folded))
                .max_by_key(|&(place, folded)| (folded, std::cmp::Reverse(place)))
                .unwrap_or((u32::MAX, 0));
            let unit = if cause_at_building(cause) {
                "building"
            } else {
                "node"
            };
            self.emit(format!(
                "trip failures: {count} more in the last simulated minute: {label}, \
                 {at_place} at {unit} {place} (first agent {first_render_id})"
            ));
        }
        self.places.clear();
        self.detailed = 0;
        self.folded = [Folded::default(); CAUSES];
        self.streaks.retain(|_, last| sim_time - *last < MINUTE_S);
    }
}

impl AgentSystem {
    /// Logs the failures the movement pass recorded, in agent order, and ends the simulated
    /// minute when it is over. O(failures log failures) plus one check when nothing failed.
    pub(in crate::simulation::economy::agents::tick) fn report_trip_failures(
        &mut self,
        allocator: &BuildingAllocator,
    ) {
        let sim_time = self.sim_time;
        let mut log = std::mem::take(&mut self.trip_failures);
        log.roll_minute(sim_time);
        if log.buffer.is_empty() {
            self.trip_failures = log;
            return;
        }
        let mut failures = std::mem::take(&mut log.buffer);
        crate::debug::work::add(
            crate::debug::work::Work::TripFailures,
            failures.len() as u64,
        );
        // Stable: one agent's failures stay in the order its worker found them.
        failures.sort_by_key(|failure| failure.agent);
        let inspected = self.inspected.map(|inspected| inspected.index);
        for failure in &failures {
            let render_id = self.agents.render_id[failure.agent];
            let place = place_of(failure);
            let inspected = inspected == Some(failure.agent);
            let streak = (render_id, place, failure.outcome.streak_class());
            let repeat = log
                .streaks
                .insert(streak, sim_time)
                .is_some_and(|last| sim_time - last < MINUTE_S);
            if inspected {
                self.inspected_failure = Some(failure_line(failure, render_id, allocator));
            }
            if repeat {
                continue;
            }
            let (counted, folded) = log.places.entry(place).or_default();
            *counted += 1;
            if !inspected && (*counted > DETAIL_PER_PLACE || log.detailed >= DETAIL_PER_MINUTE) {
                *folded += 1;
                let summary = &mut log.folded[usize::from(place.0)];
                if summary.count == 0 {
                    summary.first_render_id = render_id;
                }
                summary.count += 1;
                continue;
            }
            log.detailed += 1;
            let line = match &self.inspected_failure {
                Some(line) if inspected => line.clone(),
                _ => failure_line(failure, render_id, allocator),
            };
            log.emit(line);
        }
        failures.clear();
        log.buffer = failures;
        self.trip_failures = log;
    }
}

fn place_of(failure: &TripFailure) -> Place {
    let cause = cause_index(&failure.cause);
    let at = match failure.cause {
        FailureCause::NoRoad { node, .. }
        | FailureCause::NoTurnToEdge { node, .. }
        | FailureCause::NoTurnToLane { node, .. } => node,
        FailureCause::NoRoute(replan) => replan.start_node,
        FailureCause::NoBorderRoute { from, .. } => from,
        FailureCause::NoImmigrationRoute { border, .. } => border,
        _ if cause_at_building(usize::from(cause)) => failure.origin as u32,
        _ => failure.node,
    };
    (cause, at)
}
