// SPDX-License-Identifier: GPL-2.0-only

//! The simulation's system declarations and per-slot dispatch (`SIM-02`,
//! `docs/simulation.md#scheduler`).
//!
//! [`SimCore::step_frame`] reports the slots the clock crossed; each runs here in order. The
//! timing wheel fires first, then the fixed-slot systems: until `SIM-02-K` the re-hosted
//! operational hour, and the calendar close. The entities of the slot's hourly and daily phases
//! follow, then the cell-lot queue and one chunk each of the rolling road length and
//! environment sweep.

use std::time::Instant;

use super::frame::{EconomyObserver, FrameStepReport, record_crash_phase_for_core, run_sim_phase};
use super::state::SimCore;
use crate::simulation::buildings::allocator::Building;
use crate::simulation::core::handles::Handle;
use crate::simulation::core::scheduler::{
    Cost, Declaration, Period, SLOTS_PER_DAY, SLOTS_PER_MINUTE, Schedule, SystemKind, Tier, state,
};
use crate::simulation::core::timing_wheel::TimedEvent;
use super::trips::TripEvent;

/// One system the dispatcher can run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SimSystem {
    /// Fires the slot's timed events.
    TimedEvents,
    /// Today's operational hour, with the hourly demand pass.
    OperationalHour,
    /// The calendar close: the city's day totals become the day's budget report.
    CalendarClose,
    /// Production of the buildings in the slot's hourly phase.
    BuildingHours,
    /// Consumption, utilities and leg timeouts of the households in the slot's hourly phase.
    HouseholdHours,
    /// Happiness of the agents in the slot's hourly phase.
    AgentHours,
    /// Daily settlement of the buildings in the slot's daily phase.
    BuildingDays,
    /// Daily settlement of the households in the slot's daily phase.
    HouseholdDays,
    /// Wages and daily happiness of the agents in the slot's daily phase.
    AgentDays,
    /// Regenerates the oldest pending dirty cell-lot regions, `CELL_LOT_QUOTA` per slot.
    CellLots,
    /// One chunk of the rolling road length that daily upkeep reads.
    RoadLength,
    /// One chunk of the day-long sweep of pollution, noise and desirability (`SIM-02-H`).
    EnvironmentGrids,
}

/// Something that must happen at a given slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SimEvent {
    /// Starts the queued demand spawn `id` (`PendingDemandSpawnAction::id`).
    ConstructionStart { id: u64 },
    /// Finishes `building`'s construction; a building that left or already finished is skipped.
    ConstructionComplete { building: Handle<Building> },
    /// A step of one trip in progress.
    Trip(TripEvent),
}

impl SimEvent {
    /// Saved `(kind, a)` columns; a handle saves as [`Handle::to_bits`].
    pub(crate) fn to_saved(self) -> (u16, u64) {
        match self {
            Self::ConstructionStart { id } => (0, id),
            Self::ConstructionComplete { building } => (1, building.to_bits()),
            Self::Trip(trip) => trip.to_saved(),
        }
    }

    /// Inverse of [`Self::to_saved`]; `None` for an unknown kind.
    pub(crate) fn from_saved(kind: u16, a: u64) -> Option<Self> {
        match kind {
            0 => Some(Self::ConstructionStart { id: a }),
            1 => Some(Self::ConstructionComplete {
                building: Handle::from_bits(a),
            }),
            _ => TripEvent::from_saved(kind, a).map(Self::Trip),
        }
    }
}

/// Dirty cell-lot regions the `cell lots` queue regenerates per slot (`SIM-02-H`).
pub(super) const CELL_LOT_QUOTA: u32 = 1;

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
            system: SimSystem::CalendarClose,
            name: "calendar close",
            kind: SystemKind::FixedSlot {
                period: Period::Day,
                offset: 0,
            },
            reads: state::HOUSEHOLDS | state::BUILDINGS | state::DEMAND | state::TREASURY,
            writes: state::HOUSEHOLDS
                | state::BUILDINGS
                | state::AGENTS
                | state::DEMAND
                | state::TREASURY,
            cost: Cost::Constant,
            tier: Tier::Full,
        },
        Declaration {
            system: SimSystem::BuildingHours,
            name: "building hours",
            kind: SystemKind::EntityPhase {
                period: Period::Hour,
            },
            reads: state::BUILDINGS,
            writes: state::BUILDINGS,
            cost: Cost::Phase,
            tier: Tier::Full,
        },
        Declaration {
            system: SimSystem::HouseholdHours,
            name: "household hours",
            kind: SystemKind::EntityPhase {
                period: Period::Hour,
            },
            reads: state::HOUSEHOLDS | state::AGENTS | state::BUILDINGS,
            writes: state::HOUSEHOLDS | state::AGENTS | state::BUILDINGS,
            cost: Cost::Phase,
            tier: Tier::Full,
        },
        Declaration {
            system: SimSystem::AgentHours,
            name: "agent hours",
            kind: SystemKind::EntityPhase {
                period: Period::Hour,
            },
            reads: state::HOUSEHOLDS,
            writes: state::AGENTS,
            cost: Cost::Phase,
            tier: Tier::Full,
        },
        Declaration {
            system: SimSystem::BuildingDays,
            name: "building days",
            kind: SystemKind::EntityPhase {
                period: Period::Day,
            },
            reads: state::BUILDINGS
                | state::ZONING
                | state::NETWORK
                | state::HOUSEHOLDS
                | state::TREASURY,
            writes: state::ALL,
            cost: Cost::Phase,
            tier: Tier::Full,
        },
        Declaration {
            system: SimSystem::HouseholdDays,
            name: "household days",
            kind: SystemKind::EntityPhase {
                period: Period::Day,
            },
            reads: state::HOUSEHOLDS | state::BUILDINGS | state::DEMAND | state::TREASURY,
            writes: state::HOUSEHOLDS | state::DEMAND | state::TREASURY,
            cost: Cost::Phase,
            tier: Tier::Full,
        },
        Declaration {
            system: SimSystem::AgentDays,
            name: "agent days",
            kind: SystemKind::EntityPhase {
                period: Period::Day,
            },
            reads: state::AGENTS | state::BUILDINGS | state::GRIDS | state::TREASURY,
            writes: state::AGENTS | state::BUILDINGS | state::HOUSEHOLDS | state::TREASURY,
            cost: Cost::Phase,
            tier: Tier::Full,
        },
        Declaration {
            system: SimSystem::CellLots,
            name: "cell lots",
            kind: SystemKind::Queue {
                quota: CELL_LOT_QUOTA,
            },
            reads: state::ZONING | state::BUILDINGS | state::NETWORK,
            writes: state::ZONING | state::BUILDINGS | state::NETWORK,
            cost: Cost::Quota,
            tier: Tier::Full,
        },
        Declaration {
            system: SimSystem::RoadLength,
            name: "road length",
            kind: SystemKind::Rolling {
                period: Period::Day,
            },
            reads: state::NETWORK,
            writes: state::TREASURY,
            cost: Cost::Chunk,
            tier: Tier::Full,
        },
        Declaration {
            system: SimSystem::EnvironmentGrids,
            name: "environment grids",
            kind: SystemKind::Rolling {
                period: Period::Day,
            },
            reads: state::BUILDINGS | state::NETWORK | state::GRIDS,
            writes: state::GRIDS,
            cost: Cost::Chunk,
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
        self.allocator.now_slot = slot;
        crate::debug::work::set_clock_minute(slot / SLOTS_PER_MINUTE);
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
                SimSystem::CalendarClose => {
                    let close_start = Instant::now();
                    record_crash_phase_for_core(self, "calendar close");
                    run_sim_phase("calendar close", || {
                        self.close_calendar_day(day_index, observer)
                    });
                    report.daily_ms += close_start.elapsed().as_secs_f64() * 1000.0;
                    report.daily_ticks += 1;
                    if crate::debug::Channel::Sim.enabled() {
                        self.print_sim_console_summary(day_index, minute_of_day);
                    }
                }
                SimSystem::BuildingHours => self.run_building_hours(slot),
                SimSystem::HouseholdHours => self.run_household_hours(slot),
                SimSystem::AgentHours => self.run_agent_hours(slot),
                SimSystem::BuildingDays => self.run_building_days(slot, observer),
                SimSystem::HouseholdDays => self.run_household_days(slot, observer),
                SimSystem::AgentDays => self.run_agent_days(slot, observer),
                SimSystem::CellLots => {
                    self.serve_cell_lot_queue(CELL_LOT_QUOTA as usize);
                }
                SimSystem::RoadLength => self.road_length.advance(slot, &self.region_graph),
                SimSystem::EnvironmentGrids => self.advance_environment_sweep(slot),
            }
        }
        self.schedule_construction_completions();
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
                SimEvent::ConstructionComplete { building } => {
                    if let Some(building_idx) = self.allocator.building_handles.resolve(building)
                        && self.allocator.buildings[building_idx].is_under_construction()
                    {
                        self.allocator.complete_construction(building_idx);
                        observer.buildings_changed(self);
                    }
                }
                SimEvent::Trip(trip) => self.run_trip_event(trip, slot, observer),
            }
        }
        self.due_events = due;
    }

    /// Schedules the completion of every construction site placed since the last call, in
    /// placement order. O(new sites).
    pub(crate) fn schedule_construction_completions(&mut self) {
        for (building, slot) in self.allocator.pending_construction.drain(..) {
            self.events
                .schedule(slot, SimEvent::ConstructionComplete { building });
        }
    }
}
