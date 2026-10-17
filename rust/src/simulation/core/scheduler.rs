// SPDX-License-Identifier: GPL-2.0-only

//! Slot clock and system declarations (`SIM-02`, `docs/simulation.md#scheduler`).
//!
//! Operational time is cut into slots, [`SLOTS_PER_MINUTE`] per authored minute. Every
//! simulation system registers a [`Declaration`]; the owner of the state runs the declarations
//! due in each crossed slot in one fixed order, so no system tests the minute, hour or day.

/// Slots per authored minute. It may only grow; systems never assume its value.
pub(crate) const SLOTS_PER_MINUTE: u64 = 16;
/// Slots per operational hour.
pub(crate) const SLOTS_PER_HOUR: u64 = 60 * SLOTS_PER_MINUTE;
/// Slots per operational day.
pub(crate) const SLOTS_PER_DAY: u64 = 24 * SLOTS_PER_HOUR;

/// Absolute slot counted from day 1, 00:00.
pub(crate) fn absolute_slot(day_index: u32, minute_of_day: u16, slot_in_minute: u64) -> u64 {
    u64::from(day_index.saturating_sub(1)) * SLOTS_PER_DAY
        + u64::from(minute_of_day) * SLOTS_PER_MINUTE
        + slot_in_minute
}

/// Shortest wait before checking a trip in progress again: one authored minute.
pub(crate) const TRIP_CHECK_SLOTS: u64 = SLOTS_PER_MINUTE;
/// Longest wait between two checks of one trip: eight authored minutes.
pub(crate) const TRIP_CHECK_MAX_SLOTS: u64 = 8 * SLOTS_PER_MINUTE;

/// Departure slot of the `index`-th of `count` trips released together at `slot`: evenly
/// spaced over the following hour, so the busiest minute holds at most `count / 60 + 2` of them
/// (`docs/simulation.md#behaviour-envelope-sim-02-a`). O(1).
pub(crate) fn paced_slot(slot: u64, index: usize, count: usize) -> u64 {
    debug_assert!(index < count);
    slot + 1 + (index as u64 * (SLOTS_PER_HOUR - 1)) / count as u64
}

/// Calendar period of a periodic declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Period {
    Hour,
    Day,
}

impl Period {
    pub(crate) fn slots(self) -> u64 {
        match self {
            Self::Hour => SLOTS_PER_HOUR,
            Self::Day => SLOTS_PER_DAY,
        }
    }
}

/// How a system is scheduled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SystemKind {
    /// Consumes the slot's timed events.
    Events,
    /// One call at slot `offset` of every `period`. Only for work with a stated O(1), O(zones)
    /// or O(dirty) bound, and for passes re-hosted unchanged during the migration.
    FixedSlot { period: Period, offset: u64 },
    /// One call per slot for the entities whose `period` phase falls in it, so each entity is
    /// visited once per period (`docs/simulation.md#phases`).
    EntityPhase { period: Period },
    /// One call per slot that serves up to `quota` pending requests, oldest first
    /// (`docs/simulation.md#request-queues`).
    Queue { quota: u32 },
    /// One chunk per slot of a sum or grid sweep over the whole city that finishes once per
    /// `period` (`docs/simulation.md#snapshots`).
    Rolling { period: Period },
}

impl SystemKind {
    // Per-slot order: events, fixed-slot systems, hourly before daily entity phases, queue
    // matchers, then rolling chunks; declaration order within a kind.
    fn order(self) -> u8 {
        match self {
            Self::Events => 0,
            Self::FixedSlot { .. } => 1,
            Self::EntityPhase {
                period: Period::Hour,
            } => 2,
            Self::EntityPhase {
                period: Period::Day,
            } => 3,
            Self::Queue { .. } => 4,
            Self::Rolling { .. } => 5,
        }
    }

    /// Whether a declaration of this kind runs in `slot`.
    pub(crate) fn due(self, slot: u64) -> bool {
        match self {
            Self::Events | Self::EntityPhase { .. } | Self::Queue { .. } | Self::Rolling { .. } => {
                true
            }
            Self::FixedSlot { period, offset } => slot % period.slots() == offset,
        }
    }
}

/// Cost bound of one call, per slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cost {
    /// Proportional to the events due in the slot.
    Events,
    /// A pass over the whole city re-hosted unchanged; removed by `SIM-02-K`.
    LegacyBoundary,
    /// Proportional to the entities whose phase falls in the slot: O(N / slots per period).
    Phase,
    /// One chunk of a rolling sum or sweep: O(N / slots per period).
    Chunk,
    /// At most the queue's quota of requests, each bounded by its own size; O(1) when nothing
    /// is queued.
    Quota,
    /// Independent of the city's size: O(1) or O(zones).
    Constant,
}

/// Simulation tier a declaration serves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tier {
    /// Per-agent state machines in the active area.
    Full,
}

/// State a system touches, as a bit set over [`state`] domains.
pub(crate) type StateSet = u32;

/// State domains named by declarations, so ordering conflicts are visible.
pub(crate) mod state {
    use super::StateSet;

    pub(crate) const CLOCK: StateSet = 1 << 0;
    pub(crate) const BUILDINGS: StateSet = 1 << 1;
    pub(crate) const HOUSEHOLDS: StateSet = 1 << 2;
    pub(crate) const AGENTS: StateSet = 1 << 3;
    pub(crate) const LOGISTICS: StateSet = 1 << 4;
    pub(crate) const TREASURY: StateSet = 1 << 5;
    pub(crate) const DEMAND: StateSet = 1 << 6;
    pub(crate) const ZONING: StateSet = 1 << 7;
    pub(crate) const NETWORK: StateSet = 1 << 8;
    pub(crate) const GRIDS: StateSet = 1 << 9;
    pub(crate) const EVENTS: StateSet = 1 << 10;
    /// Every domain, for re-hosted legacy passes.
    pub(crate) const ALL: StateSet = (1 << 11) - 1;

    /// Domain names in bit order, for descriptions.
    pub(crate) const NAMES: [(StateSet, &str); 11] = [
        (CLOCK, "clock"),
        (BUILDINGS, "buildings"),
        (HOUSEHOLDS, "households"),
        (AGENTS, "agents"),
        (LOGISTICS, "logistics"),
        (TREASURY, "treasury"),
        (DEMAND, "demand"),
        (ZONING, "zoning"),
        (NETWORK, "network"),
        (GRIDS, "grids"),
        (EVENTS, "events"),
    ];

    /// `a|b|c` names of the domains in `set`.
    pub(crate) fn describe(set: StateSet) -> String {
        if set == ALL {
            return "all".into();
        }
        NAMES
            .iter()
            .filter(|(bit, _)| set & bit != 0)
            .map(|(_, name)| *name)
            .collect::<Vec<_>>()
            .join("|")
    }
}

/// One registered system: `system` names it to the dispatcher.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Declaration<S> {
    pub(crate) system: S,
    pub(crate) name: &'static str,
    pub(crate) kind: SystemKind,
    pub(crate) reads: StateSet,
    pub(crate) writes: StateSet,
    pub(crate) cost: Cost,
    pub(crate) tier: Tier,
}

/// Declarations in dispatch order.
#[derive(Clone, Debug)]
pub(crate) struct Schedule<S> {
    declarations: Vec<Declaration<S>>,
}

impl<S: Copy> Schedule<S> {
    /// Orders `declarations` by kind, keeping declaration order within a kind. Panics on a
    /// declaration that breaks the scheduling contract: a fixed-slot offset outside its period,
    /// a fixed-slot system without a bounded cost or legacy status, a queue without a quota, or
    /// an events, entity-phase, queue or rolling system whose cost is not its events, its
    /// phase's entities, its quota or one chunk.
    pub(crate) fn new(mut declarations: Vec<Declaration<S>>) -> Self {
        for declaration in &declarations {
            match declaration.kind {
                SystemKind::FixedSlot { period, offset } => {
                    assert!(
                        offset < period.slots(),
                        "{} offset {offset} outside its period",
                        declaration.name
                    );
                    assert!(
                        matches!(declaration.cost, Cost::LegacyBoundary | Cost::Constant),
                        "{} runs at a fixed slot without a bounded cost",
                        declaration.name
                    );
                }
                SystemKind::Events => assert_eq!(
                    declaration.cost,
                    Cost::Events,
                    "{} must cost its events",
                    declaration.name
                ),
                SystemKind::EntityPhase { .. } => assert_eq!(
                    declaration.cost,
                    Cost::Phase,
                    "{} must cost its phase's entities",
                    declaration.name
                ),
                SystemKind::Queue { quota } => {
                    assert!(quota > 0, "{} serves no requests", declaration.name);
                    assert_eq!(
                        declaration.cost,
                        Cost::Quota,
                        "{} must cost its quota of requests",
                        declaration.name
                    );
                }
                SystemKind::Rolling { .. } => assert_eq!(
                    declaration.cost,
                    Cost::Chunk,
                    "{} must cost one chunk",
                    declaration.name
                ),
            }
            assert!(
                (declaration.reads | declaration.writes) & !state::ALL == 0,
                "{} names an unknown state domain",
                declaration.name
            );
        }
        declarations.sort_by_key(|declaration| declaration.kind.order());
        Self { declarations }
    }

    /// Every declaration, in dispatch order.
    pub(crate) fn declarations(&self) -> &[Declaration<S>] {
        &self.declarations
    }

    /// One line per declaration: name, kind, cost, tier, reads and writes.
    pub(crate) fn describe(&self) -> String {
        self.declarations
            .iter()
            .map(|d| {
                format!(
                    "{} {:?} cost={:?} tier={:?} reads={} writes={}",
                    d.name,
                    d.kind,
                    d.cost,
                    d.tier,
                    state::describe(d.reads),
                    state::describe(d.writes)
                )
            })
            .collect::<Vec<_>>()
            .join("; ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paced_departures_fill_the_next_hour_without_a_busy_minute() {
        for count in [1, 7, 40, 59, 60, 61, 333, 959, 960, 4_000] {
            let mut per_minute = [0_usize; 60];
            for index in 0..count {
                let slot = paced_slot(5 * SLOTS_PER_HOUR, index, count);
                assert!(slot > 5 * SLOTS_PER_HOUR && slot < 6 * SLOTS_PER_HOUR);
                per_minute[((slot % SLOTS_PER_HOUR) / SLOTS_PER_MINUTE) as usize] += 1;
            }
            let busiest = *per_minute.iter().max().unwrap();
            assert!(busiest <= count / 60 + 2, "{count}: busiest minute {busiest}");
        }
    }

    fn declaration(system: u8, kind: SystemKind) -> Declaration<u8> {
        Declaration {
            system,
            name: "test",
            kind,
            reads: state::ALL,
            writes: state::ALL,
            cost: match kind {
                SystemKind::Events => Cost::Events,
                SystemKind::FixedSlot { .. } => Cost::LegacyBoundary,
                SystemKind::EntityPhase { .. } => Cost::Phase,
                SystemKind::Queue { .. } => Cost::Quota,
                SystemKind::Rolling { .. } => Cost::Chunk,
            },
            tier: Tier::Full,
        }
    }

    #[test]
    fn events_run_first_and_fixed_slots_only_at_their_offset() {
        let schedule = Schedule::new(vec![
            declaration(
                1,
                SystemKind::FixedSlot {
                    period: Period::Hour,
                    offset: 0,
                },
            ),
            declaration(
                2,
                SystemKind::FixedSlot {
                    period: Period::Day,
                    offset: 0,
                },
            ),
            declaration(0, SystemKind::Events),
        ]);
        let due = |slot| {
            schedule
                .declarations()
                .iter()
                .filter(|d| d.kind.due(slot))
                .map(|d| d.system)
                .collect::<Vec<_>>()
        };
        assert_eq!(due(0), [0, 1, 2]);
        assert_eq!(due(SLOTS_PER_HOUR), [0, 1]);
        assert_eq!(due(SLOTS_PER_HOUR + 1), [0]);
        assert_eq!(due(3 * SLOTS_PER_DAY), [0, 1, 2]);
        assert_eq!(absolute_slot(2, 60, 3), SLOTS_PER_DAY + SLOTS_PER_HOUR + 3);
    }

    #[test]
    fn queues_run_every_slot_after_entity_phases_and_before_rolling_chunks() {
        let schedule = Schedule::new(vec![
            declaration(
                4,
                SystemKind::Rolling {
                    period: Period::Day,
                },
            ),
            declaration(3, SystemKind::Queue { quota: 1 }),
            declaration(
                2,
                SystemKind::EntityPhase {
                    period: Period::Day,
                },
            ),
            declaration(
                1,
                SystemKind::EntityPhase {
                    period: Period::Hour,
                },
            ),
            declaration(
                0,
                SystemKind::FixedSlot {
                    period: Period::Hour,
                    offset: 0,
                },
            ),
        ]);
        let order: Vec<_> = schedule.declarations().iter().map(|d| d.system).collect();
        assert_eq!(order, [0, 1, 2, 3, 4]);
        assert!(SystemKind::Queue { quota: 1 }.due(SLOTS_PER_HOUR + 7));
    }
}
