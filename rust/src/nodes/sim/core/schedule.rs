// SPDX-License-Identifier: GPL-2.0-only

//! The simulation's system declarations and per-slot dispatch (`SIM-02`,
//! `docs/simulation.md#scheduler`).
//!
//! [`SimCore::step_frame`] reports the slots the clock crossed; each runs here in order. The timing
//! wheel fires first, then the fixed-slot systems: the funding census's publication, what is left
//! of the operational hour, the hour's four demand planning slots, each with its setup half a
//! minute before it, and the calendar close. The
//! entities of the slot's hourly and daily phases follow, then the cell-lot, freight, housing,
//! restock, visit and job request queues, the level-change, building-removal, departure and
//! admission queues (`SIM-02-J`; removals and departures only in the hour's removal window, and
//! no queue but the building removals in the hour's queue-free slots, [`QUEUE_FREE_SLOTS`]), one
//! chunk each of the rolling road length, environment sweep, freight supplier index, demand
//! snapshot, household destinations and job board, at an hour's last slot the hour's city totals,
//! one chunk of the funding census's input copy over the hour's first half and the census's share
//! of its steps over the second half.

use std::time::Instant;

use super::demand_requests::{BUILDING_REMOVAL_QUOTA, DEPARTURE_QUOTA, LEVEL_CHANGE_QUOTA};
use super::frame::{EconomyObserver, FrameStepReport, record_crash_phase_for_core, run_sim_phase};
use super::state::SimCore;
use super::trips::TripEvent;
use crate::simulation::buildings::allocator::Building;
use crate::simulation::core::handles::Handle;
use crate::simulation::core::scheduler::{
    Cost, Declaration, Period, SLOTS_PER_DAY, SLOTS_PER_HOUR, SLOTS_PER_MINUTE, Schedule,
    SystemKind, Tier, state,
};
use crate::simulation::core::timing_wheel::TimedEvent;
use crate::simulation::economy::demand::PlanningSlot;
use crate::simulation::economy::hiring::{
    PLANNING_PREPARATION_REST_SLOT, PLANNING_PREPARATION_ROWS_SLOT, PLANNING_PREPARATION_SLOT,
    REMOVAL_WINDOW_START, removals_allowed,
};

// Removals and level changes wait until the hour's planning slots have read the census its
// first slot published (`SIM-02-J`).
const _: () = assert!(REMOVAL_WINDOW_START > PlanningSlot::Actions as u64);

/// Slots of the hour in which the request queues serve nothing (`SIM-02-K`): the hour's four
/// planning slots' setups and plans, the removal window's first slot, which serves the hour's
/// first building removals, and the census book's three preparation slots. Each holds 1.2–2.5 ms
/// of its own work at 100k residents, so at 1× a queue's share there would put the frame over
/// its budget. The slot after one serves the skipped slot's share first, then its own, so every
/// queue serves the same quota and accrues the same credit per hour. The removal window's first
/// slot keeps its building removals, the slot's own work. Trip events and construction starts
/// due in one fire in the next slot ([`event_slot`]).
pub(crate) const QUEUE_FREE_SLOTS: [u64; 12] = [
    PlanningSlot::Residential.setup_slot(),
    PlanningSlot::Residential as u64,
    PlanningSlot::Commercial.setup_slot(),
    PlanningSlot::Commercial as u64,
    PlanningSlot::Industrial.setup_slot(),
    PlanningSlot::Industrial as u64,
    PlanningSlot::Actions.setup_slot(),
    PlanningSlot::Actions as u64,
    REMOVAL_WINDOW_START,
    PLANNING_PREPARATION_ROWS_SLOT,
    PLANNING_PREPARATION_SLOT,
    PLANNING_PREPARATION_REST_SLOT,
];

// No two queue-free slots are adjacent, so a skipped share always lands in a slot that serves.
const _: () = {
    let mut i = 0;
    while i < QUEUE_FREE_SLOTS.len() {
        let mut j = 0;
        while j < QUEUE_FREE_SLOTS.len() {
            assert!((QUEUE_FREE_SLOTS[i] + 1) % SLOTS_PER_HOUR != QUEUE_FREE_SLOTS[j]);
            j += 1;
        }
        i += 1;
    }
};

// Every queue-free slot's next slot is in the same game minute, so an event moved there keeps
// its minute.
const _: () = {
    let mut i = 0;
    while i < QUEUE_FREE_SLOTS.len() {
        let slot = QUEUE_FREE_SLOTS[i];
        assert!((slot + 1) / SLOTS_PER_MINUTE == slot / SLOTS_PER_MINUTE);
        i += 1;
    }
};

/// The slot a trip event or construction start due at absolute `slot` fires in (`SIM-02-K`):
/// one due in a queue-free slot fires in the next, in the same game minute, so that slot holds
/// its own system alone ([`QUEUE_FREE_SLOTS`]). Construction completions keep their deadlines.
/// O(1).
pub(crate) fn event_slot(slot: u64) -> u64 {
    slot + u64::from(QUEUE_FREE_SLOTS.contains(&(slot % SLOTS_PER_HOUR)))
}

/// Shares of queue work absolute `slot` serves (`QUEUE_FREE_SLOTS`): none in a queue-free slot,
/// two in the slot after one (the skipped slot's, then its own), else one. O(1).
pub(crate) fn queue_shares(slot: u64) -> usize {
    let of_hour = slot % SLOTS_PER_HOUR;
    let previous = (of_hour + SLOTS_PER_HOUR - 1) % SLOTS_PER_HOUR;
    if QUEUE_FREE_SLOTS.contains(&of_hour) {
        0
    } else if QUEUE_FREE_SLOTS.contains(&previous) {
        2
    } else {
        1
    }
}

/// One system the dispatcher can run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SimSystem {
    /// Fires the slot's timed events.
    TimedEvents,
    /// Publishes the funding census the previous hour's second half built (`SIM-02-I`).
    CensusPublish,
    /// What is left of the operational hour: the demand snapshot's new entities, pending
    /// arrivals, the logistics clock and the commute-quote expiry (`SIM-02-K`).
    OperationalHour,
    /// A planning slot's setup at its minute's first slot: the live workforce and the plan's
    /// scans of lots and existing buildings, held for the plan (`SIM-02-K`).
    PlanningSetup(PlanningSlot),
    /// The hour's residential demand planning slot (`SIM-02-I`).
    ResidentialPlanning,
    /// The hour's commercial demand planning slot (`SIM-02-I`).
    CommercialPlanning,
    /// The hour's industrial demand planning slot (`SIM-02-I`).
    IndustrialPlanning,
    /// The hour's downgrades, admission and demand actions: permits, level changes and
    /// despawns (`SIM-02-I`).
    DemandActions,
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
    /// Plans the oldest queued freight requests, `FREIGHT_REQUEST_QUOTA` per slot (`SIM-02-J`).
    FreightRequests,
    /// Serves the oldest queued rehousings and housing reviews, `HOUSING_REQUEST_QUOTA` per
    /// slot (`SIM-02-J`).
    HousingRequests,
    /// Plans the oldest queued restocks, `RESTOCK_REQUEST_QUOTA` per slot (`SIM-02-J`).
    RestockRequests,
    /// Plans the oldest queued visible service visits as the visit credit allows (`SIM-02-J`).
    VisitRequests,
    /// Runs the oldest queued job searches, `JOB_REQUEST_QUOTA` per slot (`SIM-02-J`).
    JobRequests,
    /// Applies the oldest proved level changes, `LEVEL_CHANGE_QUOTA` per slot outside the
    /// hour's planning slots (`SIM-02-J`).
    LevelChanges,
    /// Serves the oldest queued building removals, `BUILDING_REMOVAL_QUOTA` per slot of the
    /// hour's removal window (`SIM-02-J`).
    BuildingRemovals,
    /// Removes the oldest households that claimed a departure, `DEPARTURE_QUOTA` per slot of
    /// the hour's removal window (`SIM-02-J`).
    Departures,
    /// Creates the hour's admitted households as the admission credit allows (`SIM-02-J`).
    Admissions,
    /// One chunk of the rolling road length that daily upkeep reads.
    RoadLength,
    /// One chunk of the day-long sweep of pollution, noise and desirability (`SIM-02-H`).
    EnvironmentGrids,
    /// One chunk of the hour-long sweep of the freight supplier index (`SIM-02-I`).
    FreightSuppliers,
    /// One chunk of the hour-long sweep of the demand snapshot (`SIM-02-I`).
    DemandSnapshot,
    /// One chunk of the hour-long sweep of the stores and service stores restock and visit
    /// requests read (`SIM-02-J`).
    HouseholdDestinations,
    /// One chunk of the hour-long sweep of the open workplaces the next hour's job searches
    /// read (`SIM-02-J`).
    JobBoard,
    /// The hour's city totals, which the hourly phases fold: at its last slot they become the
    /// next hour's electricity dispatch and coverage and service-sales prices and pool
    /// (`SIM-02-I`).
    CityHour,
    /// One chunk of the copy of the buildings and households the hour's funding census reads,
    /// filled over the hour's first half (`SIM-02-I`).
    CensusInputs,
    /// The funding census's share of its steps over the hour's second half (`SIM-02-I`).
    Census,
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

/// Freight requests the `freight requests` queue plans per slot (`SIM-02-J`): 7,680 an hour.
/// The performance-contract cities queue about 75, 314 and 1,041 an hour at 10k, 30k and 100k
/// residents (peaks 130, 416 and 1,209), about one a slot at 100k; their busiest slots held 3, 6
/// and 8, and no slot left a request for the next, so requests are planned in the slot that
/// queued them with room for a city several times larger.
pub(super) const FREIGHT_REQUEST_QUOTA: u32 = 8;

/// Rehousings and housing reviews the `housing requests` queue serves per slot (`SIM-02-J`):
/// 61,440 an hour. Daily phases queue each household's review once a day; the envelope cities
/// queue about 217, 628 and 2,119 an hour at 10k, 30k and 100k residents, with busiest slots of
/// 4, 6 and 10 and nothing carried to the next slot. A million residents queue about 22 a slot.
pub(super) const HOUSING_REQUEST_QUOTA: u32 = 64;

/// Restocks the `restock requests` queue plans per slot (`SIM-02-J`): 61,440 an hour. A
/// household waiting for a shopper, or out of stock, asks again at every hourly phase; the
/// envelope cities queue about 1,080, 3,340 and 17,000 an hour at 10k, 30k and 100k (peaks
/// 1,750, 5,170 and 20,000; most wait for a shopper, which costs only a walk of the members).
/// The 100k city's busiest slot held 51 and no slot carried a request to the next.
pub(super) const RESTOCK_REQUEST_QUOTA: u32 = 64;

/// Job searches the `job requests` queue runs per slot (`SIM-02-J`): 61,440 an hour. An
/// unemployed adult searches every hour and a worker once its lock of five to nine days passes;
/// the searches of the homes a slot holds share their options. The envelope cities search about
/// 160, 1,400 and 1,700 times an hour at 10k, 30k and 100k, at about 0.1 ms a search at 100k;
/// only the 100k city's first hours, when every arrival looks for work, fill the quota, and
/// their searches then wait up to a few hours.
pub(super) const JOB_REQUEST_QUOTA: u32 = 64;

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
        // Before the operational hour, so the census and all its readers that hour (planning,
        // restock and visits, job search) share one slot no save separates. The swap is O(1);
        // copying funded jobs and activity onto the buildings is an O(B/p) parallel pass, and a
        // census the window left missing, unfinished or stale (a world's first hour, a clock
        // set directly, an edit or policy change between the window's last slot and this one)
        // runs whole here, O(census), as every census did before `SIM-02-I`. Both grow with the
        // city: `CityScan`, `SIM-02-K` (K5) debt.
        Declaration {
            system: SimSystem::CensusPublish,
            name: "census publish",
            kind: SystemKind::FixedSlot {
                period: Period::Hour,
                offset: 0,
            },
            reads: state::BUILDINGS
                | state::HOUSEHOLDS
                | state::LOGISTICS
                | state::NETWORK
                | state::TREASURY
                | state::CLOCK,
            writes: state::HOUSEHOLDS | state::BUILDINGS | state::NETWORK,
            cost: Cost::CityScan,
            tier: Tier::Full,
        },
        // The demand snapshot front folds the entities created in new handle slots since its
        // sweep began, O(created); the household hour recounts pending arrivals from the queued
        // admissions and the arrival carriers in flight and registers members appended since the
        // last call; the logistics clock advances, O(1); and commute quotes added a day of hours
        // ago expire, O(expired). None of these grows with the city's size. A debug build adds
        // an O(agents/p) reference check.
        Declaration {
            system: SimSystem::OperationalHour,
            name: "operational hour",
            kind: SystemKind::FixedSlot {
                period: Period::Hour,
                offset: 0,
            },
            reads: state::CLOCK
                | state::BUILDINGS
                | state::HOUSEHOLDS
                | state::AGENTS
                | state::LOGISTICS
                | state::TREASURY,
            writes: state::DEMAND | state::HOUSEHOLDS | state::AGENTS | state::LOGISTICS,
            cost: Cost::Dirty,
            tier: Tier::Full,
        },
        // The setups of the hour's four planning slots, at their minutes' first slots
        // (`SIM-02-K`), residential's after the operational hour: the live workforce,
        // O((H + B)/p) in parallel, and the plan's scans, held for the plan
        // `PLANNING_SETUP_LEAD` slots later. A zone's setup scans its free lots, O(P/p) in
        // parallel plus its zone's free lots' assets, and its existing buildings, O(B/p); the
        // actions slot's setup every zone's downgrade candidates, O(B/p). O(B + H + P) a call,
        // the `CityScan` bound, `SIM-02-K` (K5) debt.
        planning_setup_declaration(PlanningSlot::Residential, "residential setup"),
        planning_setup_declaration(PlanningSlot::Commercial, "commercial setup"),
        planning_setup_declaration(PlanningSlot::Industrial, "industrial setup"),
        planning_setup_declaration(PlanningSlot::Actions, "actions setup"),
        // The hour's demand planning, one zone per slot, on the census the hour's first slot
        // published; the actions slot queues the hour's spawns and executes its plan
        // (`SIM-02-I`). The slots are a game minute apart (`PlanningSlot`), so at 32× each runs
        // in a frame of its own, each `PLANNING_SETUP_LEAD` slots after its setup (`SIM-02-K`).
        // Planning lends the census's books and restores them, quotes freight and prepares site
        // indices. Each slot pays the books' preparation again: the project market, O(B)
        // serial; the queue's replay, O(Q log H); and the earlier slots' plans proved again,
        // O(plan); without a held setup (a world's first hour, a clock set directly) it counts
        // the workforce and scans itself. Hence each call is O(B + H + P + Q log H), the
        // `CityScan` bound, `SIM-02-K` (K5) debt.
        Declaration {
            system: SimSystem::ResidentialPlanning,
            name: "residential planning",
            kind: SystemKind::FixedSlot {
                period: Period::Hour,
                offset: PlanningSlot::Residential as u64,
            },
            reads: state::ALL,
            writes: state::ALL,
            cost: Cost::CityScan,
            tier: Tier::Full,
        },
        Declaration {
            system: SimSystem::CommercialPlanning,
            name: "commercial planning",
            kind: SystemKind::FixedSlot {
                period: Period::Hour,
                offset: PlanningSlot::Commercial as u64,
            },
            reads: state::ALL,
            writes: state::ALL,
            cost: Cost::CityScan,
            tier: Tier::Full,
        },
        Declaration {
            system: SimSystem::IndustrialPlanning,
            name: "industrial planning",
            kind: SystemKind::FixedSlot {
                period: Period::Hour,
                offset: PlanningSlot::Industrial as u64,
            },
            reads: state::ALL,
            writes: state::ALL,
            cost: Cost::CityScan,
            tier: Tier::Full,
        },
        // The actions slot pays the same setup and every zone's downgrade candidates, O(B/p)
        // each, then admits a bounded applicant batch, queues the hour's spawns, opens the
        // construction epoch, O(B + Q log H), and runs the level changes and despawns:
        // O(B + H + Q log H), within the `CityScan` bound.
        Declaration {
            system: SimSystem::DemandActions,
            name: "demand actions",
            kind: SystemKind::FixedSlot {
                period: Period::Hour,
                offset: PlanningSlot::Actions as u64,
            },
            reads: state::ALL,
            writes: state::ALL,
            cost: Cost::CityScan,
            tier: Tier::Full,
        },
        // The city's day totals become the day's budget report, O(1), and daily road upkeep
        // reads the rolling road length, which measures the network once, O(edges), only before
        // its first finished sweep (a world's first day). Debug reports scan the city only with
        // the economy channel on.
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
            system: SimSystem::FreightRequests,
            name: "freight requests",
            kind: SystemKind::Queue {
                quota: FREIGHT_REQUEST_QUOTA,
            },
            reads: state::BUILDINGS | state::LOGISTICS | state::NETWORK | state::TREASURY,
            writes: state::BUILDINGS | state::LOGISTICS | state::TREASURY | state::EVENTS,
            cost: Cost::Quota,
            tier: Tier::Full,
        },
        Declaration {
            system: SimSystem::HousingRequests,
            name: "housing requests",
            kind: SystemKind::Queue {
                quota: HOUSING_REQUEST_QUOTA,
            },
            reads: state::HOUSEHOLDS | state::AGENTS | state::BUILDINGS,
            writes: state::HOUSEHOLDS | state::AGENTS | state::BUILDINGS,
            cost: Cost::Quota,
            tier: Tier::Full,
        },
        Declaration {
            system: SimSystem::RestockRequests,
            name: "restock requests",
            kind: SystemKind::Queue {
                quota: RESTOCK_REQUEST_QUOTA,
            },
            reads: state::HOUSEHOLDS | state::AGENTS | state::BUILDINGS | state::NETWORK,
            writes: state::HOUSEHOLDS | state::AGENTS | state::BUILDINGS | state::EVENTS,
            cost: Cost::Quota,
            tier: Tier::Full,
        },
        // One visit per whole credit: at most one a slot.
        Declaration {
            system: SimSystem::VisitRequests,
            name: "visit requests",
            kind: SystemKind::Queue { quota: 1 },
            reads: state::HOUSEHOLDS | state::AGENTS | state::BUILDINGS | state::NETWORK,
            writes: state::HOUSEHOLDS | state::AGENTS | state::EVENTS,
            cost: Cost::Quota,
            tier: Tier::Full,
        },
        Declaration {
            system: SimSystem::JobRequests,
            name: "job requests",
            kind: SystemKind::Queue {
                quota: JOB_REQUEST_QUOTA,
            },
            reads: state::HOUSEHOLDS | state::AGENTS | state::BUILDINGS | state::NETWORK,
            writes: state::HOUSEHOLDS | state::AGENTS | state::BUILDINGS,
            cost: Cost::Quota,
            tier: Tier::Full,
        },
        // The demand queues (`SIM-02-J`) carry out what the hour's actions slot decided. A
        // queue that may not serve in a slot (planning slots, outside the removal window)
        // costs one comparison there.
        Declaration {
            system: SimSystem::LevelChanges,
            name: "level changes",
            kind: SystemKind::Queue {
                quota: LEVEL_CHANGE_QUOTA,
            },
            reads: state::BUILDINGS | state::DEMAND | state::NETWORK | state::ZONING,
            writes: state::BUILDINGS | state::DEMAND | state::ZONING | state::GRIDS,
            cost: Cost::Quota,
            tier: Tier::Full,
        },
        Declaration {
            system: SimSystem::BuildingRemovals,
            name: "building removals",
            kind: SystemKind::Queue {
                quota: BUILDING_REMOVAL_QUOTA,
            },
            reads: state::ALL,
            writes: state::ALL,
            cost: Cost::Quota,
            tier: Tier::Full,
        },
        Declaration {
            system: SimSystem::Departures,
            name: "departures",
            kind: SystemKind::Queue {
                quota: DEPARTURE_QUOTA,
            },
            reads: state::HOUSEHOLDS | state::AGENTS | state::BUILDINGS | state::DEMAND,
            writes: state::HOUSEHOLDS | state::AGENTS | state::BUILDINGS | state::DEMAND,
            cost: Cost::Quota,
            tier: Tier::Full,
        },
        // One admission per whole credit; the credit spreads the hour's batch over the hour.
        Declaration {
            system: SimSystem::Admissions,
            name: "admissions",
            kind: SystemKind::Queue { quota: 1 },
            reads: state::HOUSEHOLDS | state::BUILDINGS | state::NETWORK,
            writes: state::HOUSEHOLDS | state::AGENTS | state::BUILDINGS | state::EVENTS,
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
        Declaration {
            system: SimSystem::FreightSuppliers,
            name: "freight suppliers",
            kind: SystemKind::Rolling {
                period: Period::Hour,
            },
            reads: state::BUILDINGS | state::LOGISTICS | state::NETWORK,
            writes: state::LOGISTICS,
            cost: Cost::Chunk,
            tier: Tier::Full,
        },
        Declaration {
            system: SimSystem::DemandSnapshot,
            name: "demand snapshot",
            kind: SystemKind::Rolling {
                period: Period::Hour,
            },
            reads: state::BUILDINGS | state::HOUSEHOLDS | state::TREASURY | state::DEMAND,
            writes: state::DEMAND,
            cost: Cost::Chunk,
            tier: Tier::Full,
        },
        Declaration {
            system: SimSystem::HouseholdDestinations,
            name: "household destinations",
            kind: SystemKind::Rolling {
                period: Period::Hour,
            },
            reads: state::BUILDINGS | state::NETWORK,
            writes: state::HOUSEHOLDS,
            cost: Cost::Chunk,
            tier: Tier::Full,
        },
        Declaration {
            system: SimSystem::JobBoard,
            name: "job board",
            kind: SystemKind::Rolling {
                period: Period::Hour,
            },
            reads: state::BUILDINGS,
            writes: state::HOUSEHOLDS,
            cost: Cost::Chunk,
            tier: Tier::Full,
        },
        Declaration {
            system: SimSystem::CityHour,
            name: "city hour",
            kind: SystemKind::Rolling {
                period: Period::Hour,
            },
            reads: state::HOUSEHOLDS,
            writes: state::HOUSEHOLDS,
            cost: Cost::Chunk,
            tier: Tier::Full,
        },
        Declaration {
            system: SimSystem::CensusInputs,
            name: "census inputs",
            kind: SystemKind::Rolling {
                period: Period::Hour,
            },
            reads: state::CLOCK
                | state::BUILDINGS
                | state::HOUSEHOLDS
                | state::LOGISTICS
                | state::TREASURY,
            writes: state::HOUSEHOLDS | state::BUILDINGS,
            cost: Cost::Chunk,
            tier: Tier::Full,
        },
        // A rolling chunk, not a queue: one resumable computation over a period, not requests
        // served oldest first. Each second-half slot runs ceil(R / S) rows of the census's
        // steps, R the rows left assuming as many closure passes as the previous census took
        // and S the slots left; a step that is not sliced runs whole in the slot it starts in,
        // and the hour's last slot finishes what remains (`hiring/rolling.rs`). First-half
        // slots return at once. The census start publishes the frontage prices first.
        Declaration {
            system: SimSystem::Census,
            name: "census",
            kind: SystemKind::Rolling {
                period: Period::Hour,
            },
            reads: state::CLOCK
                | state::BUILDINGS
                | state::HOUSEHOLDS
                | state::LOGISTICS
                | state::NETWORK
                | state::TREASURY,
            writes: state::HOUSEHOLDS | state::BUILDINGS | state::NETWORK,
            cost: Cost::Chunk,
            tier: Tier::Full,
        },
    ])
}

// The declaration of planning slot `slot`'s setup, at its minute's first slot.
fn planning_setup_declaration(slot: PlanningSlot, name: &'static str) -> Declaration<SimSystem> {
    Declaration {
        system: SimSystem::PlanningSetup(slot),
        name,
        kind: SystemKind::FixedSlot {
            period: Period::Hour,
            offset: slot.setup_slot(),
        },
        reads: state::ALL,
        writes: state::DEMAND | state::BUILDINGS | state::NETWORK,
        cost: Cost::CityScan,
        tier: Tier::Full,
    }
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
        let queue_shares = queue_shares(slot);
        for index in 0..self.schedule.declarations().len() {
            let declaration = self.schedule.declarations()[index];
            if !declaration.kind.due(slot) {
                continue;
            }
            match declaration.system {
                SimSystem::TimedEvents => self.fire_timed_events(slot, report, observer),
                SimSystem::CensusPublish => {
                    let publish_start = Instant::now();
                    record_crash_phase_for_core(self, "census publish");
                    run_sim_phase("census publish", || self.publish_census(slot));
                    report.hourly_ms += publish_start.elapsed().as_secs_f64() * 1000.0;
                }
                SimSystem::OperationalHour => {
                    let hourly_start = Instant::now();
                    record_crash_phase_for_core(self, "operational hour tick");
                    run_sim_phase("operational hour tick", || {
                        self.run_operational_hour(day_index, minute_of_day, observer)
                    });
                    report.hourly_ms += hourly_start.elapsed().as_secs_f64() * 1000.0;
                    report.hourly_ticks += 1;
                }
                SimSystem::PlanningSetup(planning) => {
                    let setup_start = Instant::now();
                    record_crash_phase_for_core(self, declaration.name);
                    run_sim_phase(declaration.name, || {
                        self.execute_demand_planning_setup(planning, slot, day_index, minute_of_day)
                    });
                    report.hourly_ms += setup_start.elapsed().as_secs_f64() * 1000.0;
                }
                SimSystem::ResidentialPlanning => {
                    let planning_start = Instant::now();
                    record_crash_phase_for_core(self, declaration.name);
                    run_sim_phase(declaration.name, || {
                        self.execute_demand_planning_slot(
                            PlanningSlot::Residential,
                            day_index,
                            minute_of_day,
                        )
                    });
                    observer.buildings_changed(self);
                    report.hourly_ms += planning_start.elapsed().as_secs_f64() * 1000.0;
                    // At a day's first slot the calendar close prints the summary.
                    if minute_of_day != 0 && crate::debug::Channel::Sim.enabled() {
                        self.print_sim_console_summary(day_index, minute_of_day);
                    }
                }
                SimSystem::CommercialPlanning
                | SimSystem::IndustrialPlanning
                | SimSystem::DemandActions => {
                    let planning_start = Instant::now();
                    let slot_kind = match declaration.system {
                        SimSystem::CommercialPlanning => PlanningSlot::Commercial,
                        SimSystem::IndustrialPlanning => PlanningSlot::Industrial,
                        _ => PlanningSlot::Actions,
                    };
                    record_crash_phase_for_core(self, declaration.name);
                    let changed = run_sim_phase(declaration.name, || {
                        self.execute_demand_planning_slot(slot_kind, day_index, minute_of_day)
                    });
                    if changed {
                        observer.buildings_changed(self);
                    }
                    report.hourly_ms += planning_start.elapsed().as_secs_f64() * 1000.0;
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
                SimSystem::CellLots
                | SimSystem::FreightRequests
                | SimSystem::HousingRequests
                | SimSystem::RestockRequests
                | SimSystem::VisitRequests
                | SimSystem::JobRequests
                | SimSystem::LevelChanges
                | SimSystem::Departures
                | SimSystem::Admissions => {
                    for _ in 0..queue_shares {
                        self.serve_queue_share(declaration.system, slot, observer);
                    }
                }
                SimSystem::BuildingRemovals => {
                    if removals_allowed(slot) {
                        self.serve_building_removal_queue(
                            BUILDING_REMOVAL_QUOTA as usize,
                            observer,
                        );
                    }
                    // One deferred site box's terrain a slot that serves a single queue share,
                    // so neither a queue-free slot nor the one after it holds them (`SIM-02-K`).
                    if queue_shares == 1 {
                        self.publish_deferred_site_terrain_box();
                    }
                }
                SimSystem::RoadLength => self.road_length.advance(slot, &self.region_graph),
                SimSystem::EnvironmentGrids => self.advance_environment_sweep(slot),
                SimSystem::FreightSuppliers => self.advance_freight_supplier_sweep(slot),
                SimSystem::DemandSnapshot => self.advance_demand_snapshot(slot),
                SimSystem::HouseholdDestinations => self.advance_household_destinations(slot),
                SimSystem::JobBoard => self.advance_job_board(slot),
                SimSystem::CityHour => self.advance_city_hour(slot),
                SimSystem::CensusInputs => self.advance_census_inputs(slot),
                SimSystem::Census => self.advance_census(slot),
            }
        }
        self.schedule_construction_completions();
    }

    // One share of queue `system`'s work in `slot`: its quota of requests, or one slot of its
    // credit (`QUEUE_FREE_SLOTS`). Level changes and departures serve only from the removal
    // window's start, departures only within it.
    fn serve_queue_share(
        &mut self,
        system: SimSystem,
        slot: u64,
        observer: &mut impl EconomyObserver,
    ) {
        match system {
            SimSystem::CellLots => {
                self.serve_cell_lot_queue(CELL_LOT_QUOTA as usize);
            }
            SimSystem::FreightRequests => {
                self.serve_freight_request_queue(slot, FREIGHT_REQUEST_QUOTA as usize);
            }
            SimSystem::HousingRequests => {
                self.serve_housing_request_queue(HOUSING_REQUEST_QUOTA as usize);
            }
            SimSystem::RestockRequests => {
                self.serve_restock_request_queue(slot, RESTOCK_REQUEST_QUOTA as usize);
            }
            SimSystem::VisitRequests => self.serve_visit_request_queue(slot),
            SimSystem::JobRequests => {
                self.serve_job_request_queue(slot, JOB_REQUEST_QUOTA as usize);
            }
            SimSystem::LevelChanges => {
                if slot % SLOTS_PER_HOUR >= REMOVAL_WINDOW_START {
                    self.serve_level_change_queue(LEVEL_CHANGE_QUOTA as usize, observer);
                }
            }
            SimSystem::Departures => {
                if removals_allowed(slot) {
                    self.serve_departure_queue(DEPARTURE_QUOTA as usize);
                }
            }
            SimSystem::Admissions => self.serve_admission_queue(slot),
            _ => unreachable!("{system:?} is not a request queue"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_free_slots_pass_their_share_to_the_next_slot() {
        let hour = 5 * SLOTS_PER_HOUR;
        let shares: Vec<usize> = (hour..hour + SLOTS_PER_HOUR).map(queue_shares).collect();
        // Every hour serves one share a slot in all.
        assert_eq!(shares.iter().sum::<usize>(), SLOTS_PER_HOUR as usize);
        for offset in QUEUE_FREE_SLOTS {
            assert_eq!(shares[offset as usize], 0);
            assert_eq!(shares[(offset as usize + 1) % shares.len()], 2);
        }
        assert_eq!(
            shares.iter().filter(|&&share| share == 1).count(),
            SLOTS_PER_HOUR as usize - 2 * QUEUE_FREE_SLOTS.len()
        );
        // The day's first slot is queue-free too.
        assert_eq!(queue_shares(0), 0);
        assert_eq!(queue_shares(1), 2);
        // Trips and construction starts due in a queue-free slot fire in the next one.
        for slot in hour..hour + SLOTS_PER_HOUR {
            let fires = event_slot(slot);
            assert_eq!(fires, slot + u64::from(shares[(slot - hour) as usize] == 0));
            assert_ne!(shares[(fires - hour) as usize % shares.len()], 0);
            assert_eq!(fires / SLOTS_PER_MINUTE, slot / SLOTS_PER_MINUTE);
        }
    }
}
