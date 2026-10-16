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
}

impl SystemKind {
    // Per-slot order: events first, then fixed-slot systems in declaration order.
    fn order(self) -> u8 {
        match self {
            Self::Events => 0,
            Self::FixedSlot { .. } => 1,
        }
    }

    /// Whether a declaration of this kind runs in `slot`.
    pub(crate) fn due(self, slot: u64) -> bool {
        match self {
            Self::Events => true,
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
    /// a fixed-slot system without a bounded cost or legacy status, or an events system whose
    /// cost is not its events.
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
                        matches!(declaration.cost, Cost::LegacyBoundary),
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

    fn declaration(system: u8, kind: SystemKind) -> Declaration<u8> {
        Declaration {
            system,
            name: "test",
            kind,
            reads: state::ALL,
            writes: state::ALL,
            cost: if kind == SystemKind::Events {
                Cost::Events
            } else {
                Cost::LegacyBoundary
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
}
