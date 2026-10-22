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

// 2^64 / φ, odd: multiples of it modulo 2^64 are the fractional parts of k/φ, the golden-ratio
// sequence, scaled to 64 bits.
const GOLDEN_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

/// The first slot after `slot` at `key`'s offset within the hour, `floor(frac(key/φ) × S)` of
/// the hour's `S` slots: a wait of 1 to `S` slots, half an hour on average. The offsets of
/// consecutive keys follow the golden-ratio sequence, so trips keyed by consecutive ids and
/// released at random slots spread over every hour without the clusters of leaving at once.
/// Simulated over 1,000 hours of uniform releases, the busiest minute of an hour with `n` of
/// them held at most 5.0% at n = 45, 4.8% at 62, 3.6% at 100 and 2.3% at 600, within the 5%
/// burst limit (`docs/simulation.md#behaviour-envelope-sim-02-a`). O(1).
pub(crate) fn spread_slot(slot: u64, key: u64) -> u64 {
    let offset =
        ((u128::from(key.wrapping_mul(GOLDEN_GAMMA)) * u128::from(SLOTS_PER_HOUR)) >> 64) as u64;
    slot + (offset + SLOTS_PER_HOUR - slot % SLOTS_PER_HOUR - 1) % SLOTS_PER_HOUR + 1
}

/// `key`'s slot within a minute, `floor(frac(key/φ) × S)` of the minute's `S` slots: the slot
/// at which a trip due in an authored minute leaves (`SIM-02-K` part 4), so the trips of one
/// minute leave over its slots instead of all in the frame that crosses into it. O(1).
pub(crate) fn slot_in_minute_for_key(key: u64) -> u64 {
    ((u128::from(key.wrapping_mul(GOLDEN_GAMMA)) * u128::from(SLOTS_PER_MINUTE)) >> 64) as u64
}

/// Minutes a [`MinuteLeveler`] remembers: the hour a departure may wait plus its window.
pub(crate) const LEVELER_MINUTES: usize = 128;
// Minutes on either side of a target a levelled departure may move to.
const LEVELER_REACH: u64 = 15;

/// Departures already placed in each upcoming minute, so a new one can take the least busy
/// minute near its target. A target's window is the 31 minutes centred on it, moved later to
/// start no sooner than the minute after the current one; the departure takes the minute with
/// the fewest departures,
/// the closest to its target among equals, then the earliest, and keeps the target's slot
/// within the minute. Released at random slots with [`spread_slot`] targets, the busiest minute
/// of an hour with 40 to 200 departures stayed at or below 4% in simulation, against 5% for
/// the targets alone, with the wait still about half an hour
/// (`docs/simulation.md#behaviour-envelope-sim-02-a`). O(window) per departure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MinuteLeveler {
    // First minute the counts cover; minute `m` is at `m % LEVELER_MINUTES`.
    first_minute: u64,
    counts: [u16; LEVELER_MINUTES],
}

impl Default for MinuteLeveler {
    fn default() -> Self {
        Self {
            first_minute: 0,
            counts: [0; LEVELER_MINUTES],
        }
    }
}

impl MinuteLeveler {
    /// The slot of a departure targeted at `target_slot`, placed after slot `now_slot`, and
    /// counted in its minute. Targets more than an hour ahead are not levelled beyond the
    /// remembered minutes. O(window), allocation-free.
    pub(crate) fn place(&mut self, now_slot: u64, target_slot: u64) -> u64 {
        let first = now_slot / SLOTS_PER_MINUTE + 1;
        self.advance(first);
        let target_minute = target_slot / SLOTS_PER_MINUTE;
        let low = target_minute.saturating_sub(LEVELER_REACH).max(first);
        let high = (low + 2 * LEVELER_REACH).min(first + LEVELER_MINUTES as u64 - 1);
        let mut best = low;
        let mut best_key = (u16::MAX, u64::MAX);
        for minute in low..=high {
            let key = (
                self.counts[minute as usize % LEVELER_MINUTES],
                minute.abs_diff(target_minute),
            );
            if key < best_key {
                best = minute;
                best_key = key;
            }
        }
        let count = &mut self.counts[best as usize % LEVELER_MINUTES];
        *count = count.saturating_add(1);
        best * SLOTS_PER_MINUTE + target_slot % SLOTS_PER_MINUTE
    }

    // Forgets the minutes before `first`, whose entries the minutes after the window reuse.
    // Amortized O(1) per minute the clock advances.
    fn advance(&mut self, first: u64) {
        if first <= self.first_minute {
            return;
        }
        let passed = (first - self.first_minute).min(LEVELER_MINUTES as u64);
        for minute in self.first_minute..self.first_minute + passed {
            self.counts[minute as usize % LEVELER_MINUTES] = 0;
        }
        self.first_minute = first;
    }

    /// Saved form: the first minute and the counts in ring order.
    pub(crate) fn saved(&self) -> (u64, &[u16; LEVELER_MINUTES]) {
        (self.first_minute, &self.counts)
    }

    /// Inverse of [`Self::saved`]; `None` for a count list of another length.
    pub(crate) fn restore(first_minute: u64, counts: &[u16]) -> Option<Self> {
        Some(Self {
            first_minute,
            counts: counts.try_into().ok()?,
        })
    }
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
    /// or O(dirty) bound, and for the whole-city passes `Cost::CityScan` lists.
    FixedSlot { period: Period, offset: u64 },
    /// One call per slot for the entities whose `period` phase falls in it, so each entity is
    /// visited once per period (`docs/simulation.md#phases`).
    EntityPhase { period: Period },
    /// One call per slot that serves up to `quota` pending requests, oldest first
    /// (`docs/simulation.md#request-queues`). The simulation's dispatch leaves a few heavy slots
    /// of the hour without queue work and serves their share in the slot after
    /// (`nodes/sim/core/schedule.rs`, `SIM-02-K`).
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
    /// A fixed-slot pass that grows with the city, its terms stated at its declaration: the
    /// census publication, O(B) plus a whole census when none is ready, and the hour's four
    /// demand planning slots and their setups, each O(B + H + P + Q log H) in buildings B,
    /// households H, lots P and queued constructions Q. `SIM-02-K` (K5) replaces their scans with maintained totals
    /// and indices.
    CityScan,
    /// Proportional to the entities whose phase falls in the slot: O(N / slots per period).
    Phase,
    /// One chunk of a rolling sum or sweep: O(N / slots per period).
    Chunk,
    /// At most the queue's quota of requests, each bounded by its own size; O(1) when nothing
    /// is queued.
    Quota,
    /// Independent of the city's size: O(1) or O(zones).
    Constant,
    /// Proportional to what changed or came due since the previous call (entities created,
    /// requests waiting, entries expiring), not to the city's size.
    Dirty,
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
    /// Every domain, for passes that touch most of the city (the demand planning slots).
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
    /// a fixed-slot system without a constant, dirty or listed city-scan cost, a queue without
    /// a quota, or
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
                        matches!(
                            declaration.cost,
                            Cost::CityScan | Cost::Constant | Cost::Dirty
                        ),
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
            assert!(
                busiest <= count / 60 + 2,
                "{count}: busiest minute {busiest}"
            );
        }
    }

    #[test]
    fn spread_departures_of_consecutive_keys_leave_no_busy_minute() {
        // Keys released at random-looking slots of each hour, as freight plans are at the
        // buildings' phases, leave within the following hour at their keys' offsets.
        for per_hour in [40_u64, 62, 100, 333, 1_000] {
            let hours = 12;
            let mut departures = vec![[0_u64; 60]; hours as usize + 2];
            for key in 0..per_hour * hours {
                let hour = key / per_hour;
                let released = hour * SLOTS_PER_HOUR + key.wrapping_mul(7_919) % SLOTS_PER_HOUR;
                let slot = spread_slot(released, key);
                assert!(slot > released && slot <= released + SLOTS_PER_HOUR);
                assert_eq!(slot % SLOTS_PER_HOUR, spread_slot(0, key) % SLOTS_PER_HOUR);
                departures[(slot / SLOTS_PER_HOUR) as usize]
                    [((slot % SLOTS_PER_HOUR) / SLOTS_PER_MINUTE) as usize] += 1;
            }
            // Full hours only: the first and last are half filled.
            for minutes in &departures[1..hours as usize] {
                let total: u64 = minutes.iter().sum();
                let busiest = *minutes.iter().max().unwrap();
                assert!(
                    busiest * 20 <= total.max(40),
                    "{per_hour} an hour: busiest minute {busiest} of {total}"
                );
            }
        }
    }

    #[test]
    fn levelled_departures_keep_the_busiest_minute_within_four_percent() {
        // Releases at random-looking slots, 40 to 62 an hour, levelled around their spread
        // targets: no hour of 40 or more departures puts over 4% in one minute, and the wait
        // stays about half an hour.
        for per_hour in [40_u64, 45, 50, 62] {
            let hours = 48;
            let mut leveler = MinuteLeveler::default();
            let mut departures = vec![[0_u64; 60]; hours as usize + 3];
            let mut waited = 0;
            // Releases in clock order, as the queue plans them.
            let mut releases: Vec<u64> = (0..per_hour * hours)
                .map(|n| {
                    (n / per_hour) * SLOTS_PER_HOUR
                        + n.wrapping_mul(0x9E37_79B9).rotate_left(7) % SLOTS_PER_HOUR
                })
                .collect();
            releases.sort_unstable();
            for (key, &released) in releases.iter().enumerate() {
                let key = key as u64;
                let slot = leveler.place(released, spread_slot(released, key));
                assert!(slot > released && slot <= released + 2 * SLOTS_PER_HOUR);
                waited += slot - released;
                departures[(slot / SLOTS_PER_HOUR) as usize]
                    [((slot % SLOTS_PER_HOUR) / SLOTS_PER_MINUTE) as usize] += 1;
            }
            let mean_wait_minutes = waited / (per_hour * hours) / SLOTS_PER_MINUTE;
            assert!(
                (25..=40).contains(&mean_wait_minutes),
                "{per_hour}: {mean_wait_minutes}"
            );
            for minutes in &departures[1..hours as usize] {
                let total: u64 = minutes.iter().sum();
                let busiest = *minutes.iter().max().unwrap();
                assert!(
                    total < 40 || busiest * 25 <= total,
                    "{per_hour} an hour: busiest minute {busiest} of {total}"
                );
            }
        }
    }

    #[test]
    fn a_leveler_continues_identically_from_its_saved_form() {
        let mut running = MinuteLeveler::default();
        for key in 0..200 {
            running.place(key * 7, spread_slot(key * 7, key));
        }
        let (first, counts) = running.saved();
        let mut restored = MinuteLeveler::restore(first, counts).unwrap();
        for key in 200..400 {
            let now = key * 7;
            assert_eq!(
                running.place(now, spread_slot(now, key)),
                restored.place(now, spread_slot(now, key))
            );
        }
        assert!(MinuteLeveler::restore(0, &[0; 3]).is_none());
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
                SystemKind::FixedSlot { .. } => Cost::Constant,
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
