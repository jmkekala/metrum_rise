// SPDX-License-Identifier: GPL-2.0-only

//! The simulation's system declarations and per-slot dispatch (`SIM-02`,
//! `docs/simulation.md#scheduler`).
//!
//! [`SimCore::step_frame`] reports the slots the clock crossed; each runs here in order. The
//! timing wheel fires first, then the fixed-slot systems. Until `SIM-02-K`, the operational
//! hour and the daily settlement are re-hosted unchanged as fixed-slot systems at the slots
//! where they ran before.

use std::time::Instant;

use super::frame::{EconomyObserver, FrameStepReport, record_crash_phase_for_core, run_sim_phase};
use super::state::SimCore;
use crate::simulation::core::scheduler::{
    Cost, Declaration, Period, SLOTS_PER_DAY, SLOTS_PER_MINUTE, Schedule, SystemKind, Tier, state,
};
use crate::simulation::core::timing_wheel::TimedEvent;

/// One system the dispatcher can run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SimSystem {
    /// Fires the slot's timed events.
    TimedEvents,
    /// Today's operational hour, with the hourly demand pass except at midnight.
    OperationalHour,
    /// Today's daily settlement, with the midnight demand pass.
    DailySettlement,
}

/// Something that must happen at a given slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SimEvent {
    /// Starts the queued demand spawn `id` (`PendingDemandSpawnAction::id`).
    ConstructionStart { id: u64 },
}

impl SimEvent {
    /// Saved `(kind, a)` columns.
    pub(crate) fn to_saved(self) -> (u16, u64) {
        match self {
            Self::ConstructionStart { id } => (0, id),
        }
    }

    /// Inverse of [`Self::to_saved`]; `None` for an unknown kind.
    pub(crate) fn from_saved(kind: u16, a: u64) -> Option<Self> {
        match kind {
            0 => Some(Self::ConstructionStart { id: a }),
            _ => None,
        }
    }
}

/// The fixed declaration list, in registration order.
pub(crate) fn simulation_schedule() -> Schedule<SimSystem> {
    Schedule::new(vec![
        Declaration {
            system: SimSystem::TimedEvents,
            name: "timed events",
            kind: SystemKind::Events,
            reads: state::EVENTS
                | state::CLOCK
                | state::BUILDINGS
                | state::HOUSEHOLDS
                | state::ZONING
                | state::DEMAND
                | state::NETWORK
                | state::TREASURY,
            writes: state::EVENTS
                | state::BUILDINGS
                | state::ZONING
                | state::DEMAND
                | state::LOGISTICS
                | state::GRIDS,
            cost: Cost::Events,
            tier: Tier::Full,
        },
        Declaration {
            system: SimSystem::OperationalHour,
            name: "operational hour",
            kind: SystemKind::FixedSlot {
                period: Period::Hour,
                offset: 0,
            },
            reads: state::ALL,
            writes: state::ALL,
            cost: Cost::LegacyBoundary,
            tier: Tier::Full,
        },
        Declaration {
            system: SimSystem::DailySettlement,
            name: "daily settlement",
            kind: SystemKind::FixedSlot {
                period: Period::Day,
                offset: 0,
            },
            reads: state::ALL,
            writes: state::ALL,
            cost: Cost::LegacyBoundary,
            tier: Tier::Full,
        },
    ])
}

/// Day index and authored minute of absolute `slot`.
pub(crate) fn slot_calendar(slot: u64) -> (u32, u16) {
    let day_index = (slot / SLOTS_PER_DAY + 1) as u32;
    let minute_of_day = ((slot % SLOTS_PER_DAY) / SLOTS_PER_MINUTE) as u16;
    (day_index, minute_of_day)
}

impl SimCore {
    /// Runs every declaration due in `slot`, in dispatch order.
    pub(super) fn step_slot(
        &mut self,
        slot: u64,
        report: &mut FrameStepReport,
        observer: &mut impl EconomyObserver,
    ) {
        let (day_index, minute_of_day) = slot_calendar(slot);
        for index in 0..self.schedule.declarations().len() {
            let declaration = self.schedule.declarations()[index];
            if !declaration.kind.due(slot) {
                continue;
            }
            match declaration.system {
                SimSystem::TimedEvents => self.fire_timed_events(slot, report, observer),
                SimSystem::OperationalHour => {
                    let hourly_start = Instant::now();
                    record_crash_phase_for_core(self, "operational hour tick");
                    run_sim_phase("operational hour tick", || {
                        self.simulate_operational_hour_observed(day_index, minute_of_day, observer)
                    });
                    report.hourly_ms += hourly_start.elapsed().as_secs_f64() * 1000.0;
                    report.hourly_ticks += 1;
                    if minute_of_day != 0 && crate::debug::Channel::Sim.enabled() {
                        self.print_sim_console_summary(day_index, minute_of_day);
                    }
                }
                SimSystem::DailySettlement => {
                    let daily_start = Instant::now();
                    record_crash_phase_for_core(self, "daily tick");
                    run_sim_phase("daily tick", || {
                        self.simulate_tick_observed(day_index, observer)
                    });
                    report.daily_ms += daily_start.elapsed().as_secs_f64() * 1000.0;
                    report.daily_ticks += 1;
                    if crate::debug::Channel::Sim.enabled() {
                        self.print_sim_console_summary(day_index, minute_of_day);
                    }
                }
            }
        }
    }

    fn fire_timed_events(
        &mut self,
        slot: u64,
        report: &mut FrameStepReport,
        observer: &mut impl EconomyObserver,
    ) {
        let mut due = std::mem::take(&mut self.due_events);
        self.events.pop_slot(slot, &mut due);
        for TimedEvent { event, .. } in due.drain(..) {
            match event {
                SimEvent::ConstructionStart { id } => {
                    let start = Instant::now();
                    record_crash_phase_for_core(self, "demand spawn tick");
                    let (day_index, minute_of_day) = slot_calendar(slot);
                    let spawned = run_sim_phase("demand spawn tick", || {
                        self.start_pending_demand_spawn(id, day_index, minute_of_day)
                    });
                    if spawned {
                        report.pending_spawns_executed += 1;
                        observer.buildings_changed(self);
                    }
                    report.pending_spawn_ms += start.elapsed().as_secs_f64() * 1000.0;
                }
            }
        }
        self.due_events = due;
    }
}
