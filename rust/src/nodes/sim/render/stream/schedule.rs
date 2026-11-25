// SPDX-License-Identifier: GPL-3.0-or-later

//! The install scheduler (`RENDER-32-E`, `docs/terrain.md` "Streaming core" 5.4): one
//! streaming budget a frame for every layer, one order, and a cost predicted from fixed
//! constants.
//!
//! A frame's installs run as units (a layer's chunk product, atomic). A unit starts only if its
//! predicted cost fits what is left of the budget, the first unit of a frame excepted, so work
//! always progresses. What is spent is the larger of the units' measured time (a monotonic
//! clock read around every unit) and their predicted cost, so both stay within the budget save
//! for the first unit. The prediction is `calls x CALL_US + bytes x BYTE_US`, constants measured
//! by the `PLAN-STREAM` install microbenchmark (pooled objects, buffer written: 1.5-2.3 us for a
//! 64-instance batch, 71 us for 4,000) with a margin; never adapted at run time, so which units
//! start in a frame depends on the inputs and the clock alone. Units are taken in [`Class`]
//! order, then nearest first by exact squared distance, then by key. `O(1)` per unit.

/// The streaming budget of one frame, microseconds (`PLAN-STREAM` decision 6).
pub(crate) const STREAM_BUDGET_US: f64 = 2000.0;
/// Predicted cost of one backend call, microseconds.
pub(crate) const CALL_US: f64 = 0.5;
/// Predicted cost of one byte written to a buffer, microseconds (0.37 ns measured).
pub(crate) const BYTE_US: f64 = 0.0005;

/// Predicted cost of a unit of `calls` backend calls writing `bytes`.
pub(crate) fn predicted_us(calls: usize, bytes: usize) -> f64 {
    calls as f64 * CALL_US + bytes as f64 * BYTE_US
}

/// Install classes, most urgent first. The frustum changes a chunk's class, never whether it is
/// wanted. (An edit's atomic batch comes first of all once a layer has edits, `G2c`.)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Class {
    /// Wanted, drawing nothing, in the view.
    HoleInView,
    /// Wanted, drawing nothing, within the near ring.
    HoleNear,
    /// Wanted, drawing nothing.
    Hole,
    /// Replaces a drawn version, in the view.
    UpgradeInView,
    /// Replaces a drawn version.
    Upgrade,
}

/// Another owner's units in the patch layers' order (`VEG-35` R1, [`frame_layers`]): the
/// vegetation's waiting products in its own order (class, squared ground distance from the eye,
/// key), its holes among the patch layers' holes and its replacements of drawn patches among their
/// level changes. `O(1)` a call.
///
/// [`frame_layers`]: super::layers::terrain::frame_layers
pub(crate) trait UnitSource {
    /// The next waiting unit's class and squared ground distance from the eye; `None` when none
    /// waits or the budget closed the source for the pass.
    fn next_unit(&self) -> Option<(Class, f64)>;
    /// Installs the next waiting unit within `budget`: a hole as any unit (the frame's first
    /// always), a replacement as an upgrade leaving `reserve_us` for holes
    /// ([`FrameBudget::admits_upgrade`]), whatever the budget with `None` (the frame's first in-view
    /// replacement, as a patch layer's first in-view level change). False when the budget refused
    /// it, which closes the source for the pass.
    fn install_next(&mut self, budget: &mut FrameBudget, clock: &dyn Clock, reserve_us: Option<f64>) -> bool;
}

/// Whether a unit at `a` (class, squared ground distance from the eye) runs before one at `b`:
/// class, then nearest. On a tie the patch layers' unit runs first (owner order water, terrain,
/// vegetation; keys never tie across owners).
pub(crate) fn orders_before(a: (Class, f64), b: (Class, f64)) -> bool {
    a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).is_lt()
}

/// A source of monotonic microseconds.
pub(crate) trait Clock {
    /// Microseconds since an arbitrary fixed point.
    fn now_us(&self) -> f64;
}

/// The process's monotonic clock.
pub(crate) struct MonotonicClock(std::time::Instant);

impl Default for MonotonicClock {
    fn default() -> Self {
        Self(std::time::Instant::now())
    }
}

impl Clock for MonotonicClock {
    fn now_us(&self) -> f64 {
        self.0.elapsed().as_secs_f64() * 1e6
    }
}

/// One frame's account of the streaming budget.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct FrameBudget {
    budget_us: f64,
    /// Units started this frame.
    pub(crate) units: u32,
    /// Their predicted cost, microseconds.
    pub(crate) predicted_us: f64,
    /// Their measured time, microseconds.
    pub(crate) measured_us: f64,
    /// The longest unit, microseconds.
    pub(crate) max_unit_us: f64,
    /// The largest amount by which a unit's measured time exceeded its prediction.
    pub(crate) max_overrun_us: f64,
}

impl FrameBudget {
    /// A frame's account of `budget_us`.
    pub(crate) fn new(budget_us: f64) -> Self {
        Self {
            budget_us,
            ..Self::default()
        }
    }

    /// Starts a new frame.
    pub(crate) fn begin_frame(&mut self) {
        *self = Self::new(self.budget_us);
    }

    /// What the frame has spent: the larger of the measured and the predicted time.
    pub(crate) fn spent_us(&self) -> f64 {
        self.measured_us.max(self.predicted_us)
    }

    /// Whether a unit predicted at `predicted_us` may start now: always as the frame's first.
    pub(crate) fn admits(&self, predicted_us: f64) -> bool {
        self.units == 0 || self.spent_us() + predicted_us <= self.budget_us
    }

    /// Whether a unit predicted at `predicted_us` fits the room left, never as an exempt first
    /// unit (a release, `TERRAIN-41` B1: it never takes a frame the budget has no room in).
    pub(crate) fn fits(&self, predicted_us: f64) -> bool {
        self.spent_us() + predicted_us <= self.budget_us
    }

    /// This owner's account inside the frame's account `frame`: the frame's spend so far, its
    /// own `units` (so its first unit of the frame still runs).
    pub(crate) fn within(frame: &FrameBudget, units: u32) -> Self {
        Self { units, ..*frame }
    }

    /// Whether an upgrade predicted at `predicted_us` may start with `reserve_us` kept for holes
    /// (`RENDER-32-Q`): within the budget less the reserve, or as the owner's first unit.
    pub(crate) fn admits_upgrade(&self, predicted_us: f64, reserve_us: f64) -> bool {
        self.units == 0 || self.spent_us() + predicted_us <= self.budget_us - reserve_us
    }

    /// Accounts measured time that is not a unit (frees): it narrows the room, never a unit's
    /// right to run first.
    pub(crate) fn add_spent(&mut self, measured_us: f64) {
        self.measured_us += measured_us;
    }

    /// Accounts a unit that ran.
    pub(crate) fn record(&mut self, predicted_us: f64, measured_us: f64) {
        self.units += 1;
        self.predicted_us += predicted_us;
        self.measured_us += measured_us;
        self.max_unit_us = self.max_unit_us.max(measured_us);
        self.max_overrun_us = self.max_overrun_us.max(measured_us - predicted_us);
    }
}

/// The streaming node's reserve against upgrades installed ahead of it in the frame, while its
/// last frame left holes waiting (`RENDER-32-Q`): the classes' order across owners. The node's
/// holes are the water's, the terrain's and the vegetation's; since `VEG-35` R1 they spend the
/// reserve in one order (class, nearest) ahead of the patches' level changes, so the vegetation's
/// waiting holes, which hold the reserve on, can reach it.
pub(crate) const HOLE_RESERVE_US: f64 = 1000.0;

/// The frame's install owners (`RENDER-32-Q`), each charging the one ledger. The process order
/// of a frame is theirs too: the terrain's dirty-patch commit and the graded tiles
/// (`terrain.gd`), the water's mesh pipeline (`water.gd`), the building batches and sites
/// (`buildings.gd`), then the streaming node (patch units, vegetation) and its frees, and just
/// before the draw the agents' shadow casters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Owner {
    /// The terrain's dirty patches, staged a patch a unit (`RENDER-40`).
    TerrainCommit,
    /// The graded tiles' poll, cell set builds and the cell and wall rewrites they cause.
    Graded,
    /// The water's mesh request, submit, poll and apply (`WATER-01`).
    Water,
    /// The building LOD batches' publication.
    BuildingPublication,
    /// The plots' and construction sites' transforms (`RENDER-30`).
    BuildingSites,
    /// The streaming node's units: terrain and water patches, vegetation.
    Stream,
    /// The streaming node's frees (the plan's `free` class, AUDIT-stream finding 6).
    Free,
    /// The moving agents' shadow-only twins, kept just before the frame draws (`RENDER-15`,
    /// R47): charged, never deferred, since the shadows must follow the agents drawn.
    AgentCasters,
}

/// Owners in [`Owner`] order.
pub(crate) const OWNERS: usize = 8;

/// The frame's one install ledger (`RENDER-32-Q`, `PLAN-STREAM` decision 6): every owner of
/// main-thread install work charges the frame's [`STREAM_BUDGET_US`] here, in the frame's process
/// order. Owners that install upgrades ahead of the streaming node in the frame (the terrain's
/// commit, the graded tiles) take only what [`Self::upgrade_room_us`] leaves, so the node's holes
/// keep [`HOLE_RESERVE_US`]; each owner's first unit of a frame always runs, so the worst frame
/// overruns by one unit an owner. The load windows (the post-load patches', the road hydration's)
/// and an edit's atomic batch are named exceptions with budgets of their own. `O(1)` a charge.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct FrameLedger {
    frame: u64,
    /// The frame's account: what every owner spent.
    pub(crate) budget: FrameBudget,
    /// Whether the streaming node's last frame left holes waiting.
    pub(crate) holes_waiting: bool,
    /// Whether the streaming node's last patch frame held a terrain or water hole back for its
    /// budget (`TERRAIN-41` L2): the graded book's builds that no shown tile waits for then wait.
    /// A hole that waits for anything else (a payload, a layer without scripts) holds nothing.
    pub(crate) holes_deferred: bool,
    /// Measured microseconds by [`Owner`] this frame.
    pub(crate) charged: [f64; OWNERS],
}

impl FrameLedger {
    /// A ledger of `budget_us` a frame.
    pub(crate) fn new(budget_us: f64) -> Self {
        Self {
            budget: FrameBudget::new(budget_us),
            ..Self::default()
        }
    }

    /// Opens process frame `frame`: a new one starts an empty account (the holes flag carries).
    pub(crate) fn open(&mut self, frame: u64) {
        if frame != self.frame {
            self.frame = frame;
            self.budget.begin_frame();
            self.charged = [0.0; OWNERS];
        }
    }

    /// What an owner installing upgrades ahead of the streaming node may still spend.
    pub(crate) fn upgrade_room_us(&self) -> f64 {
        self.budget.budget_us - self.reserve_us() - self.budget.spent_us()
    }

    /// What upgrades leave for holes this frame: [`HOLE_RESERVE_US`] while the streaming node's
    /// last frame left holes waiting.
    pub(crate) fn reserve_us(&self) -> f64 {
        if self.holes_waiting { HOLE_RESERVE_US } else { 0.0 }
    }

    /// Accounts `owner`'s unit (predicted and measured microseconds).
    pub(crate) fn charge(&mut self, owner: Owner, predicted_us: f64, measured_us: f64) {
        self.budget.record(predicted_us, measured_us);
        self.charged[owner as usize] += measured_us;
    }
}

thread_local! {
    // The main thread's ledger: every owner runs there.
    static LEDGER: std::cell::Cell<FrameLedger> = std::cell::Cell::new(FrameLedger::new(STREAM_BUDGET_US));
}

/// Runs `f` on the main thread's ledger, opened for process frame `frame`.
pub(crate) fn with_ledger<R>(frame: u64, f: impl FnOnce(&mut FrameLedger) -> R) -> R {
    LEDGER.with(|cell| {
        let mut ledger = cell.get();
        ledger.open(frame);
        let out = f(&mut ledger);
        cell.set(ledger);
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_unit_always_runs_and_the_rest_fit_the_remainder() {
        let mut frame = FrameBudget::new(100.0);
        assert!(
            frame.admits(500.0),
            "the first unit of a frame always starts"
        );
        frame.record(500.0, 20.0);
        assert!(
            !frame.admits(1.0),
            "the predicted spend counts when it exceeds the clock"
        );
        frame.begin_frame();
        frame.record(30.0, 70.0);
        assert!(
            frame.admits(30.0) && !frame.admits(31.0),
            "the clock counts when it exceeds the prediction"
        );
        assert_eq!(frame.max_overrun_us, 40.0);
    }

    #[test]
    fn classes_order_holes_before_upgrades_and_the_view_first() {
        let mut classes = [
            Class::Upgrade,
            Class::Hole,
            Class::UpgradeInView,
            Class::HoleNear,
            Class::HoleInView,
        ];
        classes.sort();
        assert_eq!(
            classes,
            [
                Class::HoleInView,
                Class::HoleNear,
                Class::Hole,
                Class::UpgradeInView,
                Class::Upgrade
            ]
        );
    }

    #[test]
    fn owners_share_one_frame_and_upgrades_leave_the_holes_their_reserve() {
        let mut ledger = FrameLedger::new(2000.0);
        ledger.open(7);
        ledger.charge(Owner::Graded, 100.0, 600.0);
        assert_eq!(ledger.upgrade_room_us(), 1400.0);
        ledger.holes_waiting = true;
        assert_eq!(ledger.upgrade_room_us(), 400.0, "the holes keep their reserve");
        ledger.charge(Owner::BuildingPublication, 0.0, 300.0);
        ledger.open(7);
        assert_eq!(ledger.budget.spent_us(), 900.0, "the same frame keeps its account");
        assert_eq!(ledger.charged[Owner::Graded as usize], 600.0);
        ledger.open(8);
        assert_eq!(ledger.budget.spent_us(), 0.0, "a new frame starts empty");
        assert_eq!(ledger.charged, [0.0; OWNERS]);
        assert!(ledger.holes_waiting, "the flag is the last frame's until the node sets it");
    }
}
