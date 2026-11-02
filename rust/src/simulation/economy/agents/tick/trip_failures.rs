// SPDX-License-Identifier: GPL-2.0-only

//! Trip failures for the `events` log (`DEBUG-06`).
//!
//! A failure site in the movement pass records what went wrong from its own data: the agent's
//! position and destination before any recovery moves it, the cause (a missing turn, the
//! planner's rejection) and what happens next. Records go into one reused buffer behind a
//! mutex that is only locked when something fails, so a pass without failures pays one branch
//! at each failure site. After the pass, [`report`] sorts them by agent and logs them, so the
//! log does not depend on the worker schedule.

mod report;
mod text;

use super::super::{ACCESS_IMMIGRATION_ORIGIN, TRANSIT_IMMIGRATING};
use super::planning::ReplanFailure;
use super::slices::MovementSlices;
use godot::prelude::Vector2;
use std::sync::{Mutex, MutexGuard};

pub(crate) use report::TripFailureLog;

/// What the failing agent is doing in the city.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AgentKind {
    Resident,
    FreightCarrier,
    MovingIn,
}

/// Why a trip failed, from the failure site's own data.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum FailureCause {
    /// The route's next node has no road from `node`.
    NoRoad { node: u32, next_node: u32 },
    /// No connector from `from_lane` onto `to_edge` at `node`.
    NoTurnToEdge {
        node: u32,
        from_lane: usize,
        from_edge: usize,
        to_edge: usize,
    },
    /// No connector from `from_lane` into the planned exit lane `to_lane` at `node`.
    NoTurnToLane {
        node: u32,
        from_lane: usize,
        to_lane: usize,
    },
    /// The planner found no route to the destination building.
    NoRoute(ReplanFailure),
    /// The planned exit from the road (`lane` at `lane_d`) is no longer legal; `replan` is
    /// why the new plan failed, `None` when the agent had no position to plan from.
    StaleExit {
        lane: usize,
        lane_d: f32,
        replan: Option<ReplanFailure>,
    },
    /// The planned way from the building onto the road (`lane` at `lane_d`) is no longer
    /// legal.
    StaleEntry { lane: usize, lane_d: f32 },
    /// No road path from `from` to the freight carrier's border node.
    NoBorderRoute { from: u32, border: u32 },
    /// No route from the border node the household arrives at to its new home.
    NoImmigrationRoute { border: u32, building: usize },
    /// No route from the building the agent is in to its next destination.
    NoDeparture { has_car: bool },
    /// The agent's place on the road is unknown, so it cannot plan from there.
    NoPosition,
    /// The agent travels with no plan to reach its destination.
    NoPlan,
}

/// What happens to the agent after the failure.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum FailureOutcome {
    /// Drops its route and plans a new one at once.
    Replans,
    /// A walker gives up its trip and walks home to `building` (`ROAD-74`).
    WalksHome { building: usize },
    /// A walker with no way home either waits at the end of its lane and plans home again
    /// after `after_s`, then on its backoff (`ROAD-74`).
    Stranded { after_s: f32 },
    /// A walker off every sidewalk walks `metres` across the ground to the nearest one, then plans
    /// from there (`TRAFFIC-19`).
    WalksToSidewalk { metres: f32 },
    /// Plans again after `after_s`; this was failure `failure` of `of`.
    Retry { after_s: f32, failure: u8, of: u8 },
    /// Plans its departure again after `after_s`.
    RetryDeparture { after_s: f32 },
    /// The watchdog moved it home to `building`, at `pos`, after `failures` failures.
    Home {
        building: usize,
        pos: Vector2,
        failures: u8,
    },
    /// The watchdog moved it to border node `node`, at `pos`, after `failures` failures;
    /// `building` is the destination it drives in to again, if any.
    Border {
        node: u32,
        pos: Vector2,
        building: usize,
        failures: u8,
    },
    /// The watchdog found no border node to move it to and retries after `after_s`.
    NoBorder { after_s: f32 },
}

impl FailureOutcome {
    // The watchdog's recovery is a streak of its own, so it is logged after the quiet retries.
    fn streak_class(self) -> u8 {
        match self {
            Self::Home { .. } | Self::Border { .. } | Self::NoBorder { .. } => 1,
            _ => 0,
        }
    }
}

/// One trip failure, with the agent's state as the failure site saw it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct TripFailure {
    pub(super) agent: usize,
    pub(super) kind: AgentKind,
    pub(super) mode: u8,
    pub(super) node: u32,
    pub(super) edge: usize,
    pub(super) lane: usize,
    pub(super) lane_d: f32,
    /// The building the agent is in or leaves; `usize::MAX` on the road.
    pub(super) origin: usize,
    /// The destination building; `usize::MAX` for none.
    pub(super) destination: usize,
    /// A freight carrier's border node; `u32::MAX` for none.
    pub(super) border: u32,
    pub(super) cause: FailureCause,
    pub(super) outcome: FailureOutcome,
}

impl TripFailure {
    /// Agent `i`'s failure as it stands now, before the site changes its state.
    ///
    /// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
    pub(super) unsafe fn capture(
        i: usize,
        cause: FailureCause,
        outcome: FailureOutcome,
        slices: &MovementSlices,
    ) -> Self {
        unsafe {
            let moving_in = *slices.pending_household_size.get(i) > 0
                || *slices.transit.get(i) == TRANSIT_IMMIGRATING
                || *slices.access_flags.get(i) & ACCESS_IMMIGRATION_ORIGIN != 0;
            let kind = if *slices.freight_shipment_id.get(i) != u64::MAX {
                AgentKind::FreightCarrier
            } else if moving_in {
                AgentKind::MovingIn
            } else {
                AgentKind::Resident
            };
            Self {
                agent: i,
                kind,
                mode: *slices.tmode.get(i),
                node: *slices.cur_n.get(i),
                edge: *slices.cur_e.get(i),
                lane: *slices.lane_id.get(i),
                lane_d: *slices.lane_d.get(i),
                origin: *slices.cur_b.get(i),
                destination: *slices.tgt_b.get(i),
                border: *slices.freight_target_border_node.get(i),
                cause,
                outcome,
            }
        }
    }
}

/// Failures recorded during one movement pass, in no particular order.
#[derive(Default)]
pub(super) struct FailureSink(Mutex<Vec<TripFailure>>);

impl FailureSink {
    /// Collects into `buffer`, which keeps its capacity from earlier passes.
    pub(super) fn new(buffer: Vec<TripFailure>) -> Self {
        Self(Mutex::new(buffer))
    }

    /// Records one failure. Locks only on a failure, never on the common path.
    #[cold]
    pub(super) fn push(&self, failure: TripFailure) {
        #[cfg(test)]
        probe_tally::add(
            probe_tally::RECORD,
            &failure,
            text::CAUSE_LABELS[usize::from(text::cause_index(&failure.cause))],
        );
        self.lock().push(failure);
    }

    /// The recorded failures.
    pub(super) fn into_inner(self) -> Vec<TripFailure> {
        self.0
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock(&self) -> MutexGuard<'_, Vec<TripFailure>> {
        // A panicking worker must not hide the failures of the others.
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Records agent `i`'s failure with its current state.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[cold]
#[inline(never)]
pub(super) unsafe fn record_failure(
    i: usize,
    cause: FailureCause,
    outcome: FailureOutcome,
    slices: &MovementSlices,
) {
    unsafe {
        slices
            .failures
            .push(TripFailure::capture(i, cause, outcome, slices));
    }
}

/// Test-only tally of every trip-failure record and every watchdog verdict, by agent kind, lane
/// state, site and outcome (the `TRAFFIC-19` probe). Touched only when something fails.
#[cfg(test)]
pub(crate) mod probe_tally {
    use super::super::super::MODE_WALK;
    use super::{AgentKind, FailureOutcome, TripFailure};
    use std::collections::BTreeMap;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// (section, kind, lane state, site, outcome) -> count.
    pub(crate) type Tally = BTreeMap<
        (
            &'static str,
            &'static str,
            &'static str,
            &'static str,
            &'static str,
        ),
        u64,
    >;
    /// A trip-failure record; its site is the cause label.
    pub(crate) const RECORD: &str = "record";
    /// A watchdog verdict; its site is the call site's reason.
    pub(crate) const WATCHDOG: &str = "watchdog";

    static TALLY: Mutex<Tally> = Mutex::new(BTreeMap::new());
    // Off unless the probe turns it on, so other tests and timing runs never take its locks.
    static ENABLED: AtomicBool = AtomicBool::new(false);

    /// Turns the tally on for the rest of the process.
    pub(crate) fn enable() {
        ENABLED.store(true, Ordering::Relaxed);
    }

    /// Counts `failure` under `section` and `site`.
    pub(in crate::simulation::economy::agents::tick) fn add(
        section: &'static str,
        failure: &TripFailure,
        site: &'static str,
    ) {
        if !ENABLED.load(Ordering::Relaxed) {
            return;
        }
        let kind = if failure.kind == AgentKind::FreightCarrier {
            "freight"
        } else if failure.mode == MODE_WALK {
            "walker"
        } else {
            "car"
        };
        let lane = if failure.lane == usize::MAX {
            "no lane"
        } else if failure.edge == usize::MAX {
            "connector"
        } else {
            "road lane"
        };
        let outcome = match failure.outcome {
            FailureOutcome::Replans => "replans",
            FailureOutcome::WalksHome { .. } => "walks home",
            FailureOutcome::Stranded { .. } => "stranded",
            FailureOutcome::WalksToSidewalk { .. } => "walks to a sidewalk",
            FailureOutcome::Retry { .. } => "retry",
            FailureOutcome::RetryDeparture { .. } => "retry departure",
            FailureOutcome::Home { .. } => "TELEPORT home",
            FailureOutcome::Border { .. } => "TELEPORT border",
            FailureOutcome::NoBorder { .. } => "no border",
        };
        *TALLY
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry((section, kind, lane, site, outcome))
            .or_insert(0) += 1;
        if kind == "walker" {
            sample(|| {
                format!(
                    "{section} walker agent {} node {} edge {} lane {} at {:.1} destination {} \
                     site {site} outcome {outcome}",
                    failure.agent,
                    failure.node,
                    failure.edge,
                    failure.lane,
                    failure.lane_d,
                    failure.destination
                )
            });
        }
    }

    static SAMPLES: Mutex<Vec<String>> = Mutex::new(Vec::new());
    const MAX_SAMPLES: usize = 40;

    /// Counts one event of `kind` that is not a failure record.
    pub(crate) fn add_event(section: &'static str, kind: &'static str, site: &'static str) {
        if !ENABLED.load(Ordering::Relaxed) {
            return;
        }
        *TALLY
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry((section, kind, "-", site, "-"))
            .or_insert(0) += 1;
    }

    /// Keeps `line()` while fewer than 40 samples are kept.
    pub(crate) fn sample(line: impl FnOnce() -> String) {
        if !ENABLED.load(Ordering::Relaxed) {
            return;
        }
        let mut samples = SAMPLES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if samples.len() < MAX_SAMPLES {
            samples.push(line());
        }
    }

    /// The tally and the samples so far, emptied.
    pub(crate) fn take() -> (Tally, Vec<String>) {
        (
            std::mem::take(
                &mut *TALLY
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()),
            ),
            std::mem::take(
                &mut *SAMPLES
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()),
            ),
        )
    }
}

#[cfg(test)]
mod tests;
