// ========================================================================
//  MANIFEST
// ========================================================================
//  script_name: report.rs
//  script_path: rust/src/simulation/economy/agents/tick/traffic/report.rs
//  module_name: report
//  version: 0.1.0
//  description: Why a car was held, counted per junction and per cause, so
//           a delay can be explained rather than only located. A heatmap
//           says where traffic is bad; this says what held it there.
//  kind: module
//  spec: none
//  internal_dependencies: []
//  external_dependencies: []
//  features: [hold-causes, per-junction tallies, lock-free accumulation]
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-28
// ========================================================================

//! Per-junction hold accounting: what stopped a car, and where.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

// ========================================================================
// WHY A CAR WAS HELD
// ========================================================================

/// The reason a movement was refused entry to a junction.
///
/// Every hold site already names its reason in a debug log. Those strings are
/// diagnostic and there are fourteen of them, several distinguishing code paths
/// rather than causes: a zero-hop connector being occupied is the same thing
/// happening to the driver as an ordinary connector being occupied. These are
/// the causes a person would name, and each maps from one or more log reasons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HoldCause {
    /// A red or amber signal.
    SignalRed = 0,
    /// A yield or stop sign's arrival delay.
    PrioritySign = 1,
    /// Gave way to a movement with the better claim.
    ///
    /// Distinct from `Conflict`: this car could have gone if it outranked the
    /// other, so the fix is a protected phase or a turn lane, not more capacity.
    Yielded = 2,
    /// A crossing movement held the box.
    Conflict = 3,
    /// The connector itself was occupied or claimed by another car.
    ConnectorBusy = 4,
    /// The exit lane was jammed to its mouth, so there was nowhere to land.
    ExitJammed = 5,
}

impl HoldCause {
    /// Every cause, in order, for iteration and for sizing a tally.
    pub const ALL: [HoldCause; 6] = [
        HoldCause::SignalRed,
        HoldCause::PrioritySign,
        HoldCause::Yielded,
        HoldCause::Conflict,
        HoldCause::ConnectorBusy,
        HoldCause::ExitJammed,
    ];

    /// The name a report shows.
    pub fn label(self) -> &'static str {
        match self {
            HoldCause::SignalRed => "signal",
            HoldCause::PrioritySign => "priority sign",
            HoldCause::Yielded => "gave way",
            HoldCause::Conflict => "crossing traffic",
            HoldCause::ConnectorBusy => "connector busy",
            HoldCause::ExitJammed => "exit jammed",
        }
    }
}

// ========================================================================
// THE TALLY
// ========================================================================

/// What rerouting decided at one junction over the current window.
///
/// The router already prices the remaining route and the best alternative at
/// every junction a car passes, compares them, and discards both numbers. The
/// rejected case is the one worth keeping: a car that stayed on a slow route
/// because the alternative was not enough better is exactly the delay a player
/// wants explained, and it leaves no trace anywhere else.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct JunctionReroutes {
    /// Cars that switched to a cheaper route here.
    pub taken: u32,
    /// Cars that priced an alternative and stayed, because it did not beat the
    /// current route by the required margin.
    pub rejected: u32,
    /// Summed cost of the routes cars were already on, over rejected decisions.
    current_cost_sum: f32,
    /// Summed cost of the alternatives they declined.
    candidate_cost_sum: f32,
}

impl JunctionReroutes {
    /// Mean cost of the route cars stayed on, or `None` with no rejections.
    pub fn mean_current_cost(&self) -> Option<f32> {
        if self.rejected == 0 {
            return None;
        }
        Some(self.current_cost_sum / self.rejected as f32)
    }

    /// Mean cost of the alternative they declined, or `None` with none.
    ///
    /// Read against `mean_current_cost`: the gap is what the detour would have
    /// cost, and a small gap is why nobody took it.
    pub fn mean_candidate_cost(&self) -> Option<f32> {
        if self.rejected == 0 {
            return None;
        }
        Some(self.candidate_cost_sum / self.rejected as f32)
    }

    /// Total decisions priced here.
    #[inline]
    pub fn total(&self) -> u32 {
        self.taken + self.rejected
    }
}

/// Hold counts for one junction over the current window.
///
/// Plain counters rather than atomics: a junction's tally is merged from the
/// per-worker accumulators after the parallel pass, so nothing writes to it
/// concurrently.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JunctionHolds {
    counts: [u32; 6],
}

impl JunctionHolds {
    /// Holds recorded for one cause.
    #[inline]
    pub fn count(&self, cause: HoldCause) -> u32 {
        self.counts[cause as usize]
    }

    /// Every hold at this junction, whatever the cause.
    #[inline]
    pub fn total(&self) -> u32 {
        self.counts.iter().sum()
    }

    /// The cause that held the most cars here, and its count.
    ///
    /// This is the answer a report leads with: not that a junction is slow, but
    /// what is making it slow. `None` when nothing was held.
    pub fn dominant(&self) -> Option<(HoldCause, u32)> {
        HoldCause::ALL
            .iter()
            .map(|&c| (c, self.count(c)))
            .filter(|&(_, n)| n > 0)
            .max_by_key(|&(c, n)| (n, std::cmp::Reverse(c)))
    }

    #[inline]
    fn add(&mut self, cause: HoldCause, n: u32) {
        self.counts[cause as usize] = self.counts[cause as usize].saturating_add(n);
    }
}

// ========================================================================
// ACCUMULATION
// ========================================================================

/// Lock-free hold accumulation across the parallel movement pass.
///
/// The movement pass visits each agent on one of several worker threads, and a
/// hold can happen at any junction from any worker, so the counters have to
/// tolerate concurrent increment. Atomics keyed by junction would need a lock to
/// grow the map, so the map is fixed at tick start: one slot per node, sized
/// when the graph is known. A node id past the end is dropped rather than
/// growing the table mid-pass, which costs a lost count on a graph that grew
/// this tick and never a torn read.
#[derive(Debug, Default)]
pub struct HoldAccumulator {
    slots: Vec<[AtomicU32; 6]>,
    /// Cars a signal held, per approach arm, indexed by edge id.
    ///
    /// The per-node tally says a junction is holding traffic; it cannot say
    /// which approach is starving, and that is the number a signal has to
    /// respond to. Indexed by edge rather than by (node, edge) because an edge
    /// meets a node at each of its two ends and a car is held on the approach,
    /// so the edge alone identifies the arm the queue is standing on.
    ///
    /// Sized with the slots at tick start, so recording stays one relaxed
    /// increment and no worker ever takes a lock in the movement path.
    arm_holds: Vec<AtomicU32>,
    /// Per node: taken, rejected, and the two cost sums.
    ///
    /// Costs accumulate in hundredths as integers rather than as floats,
    /// because a float sum built from several threads in nondeterministic order
    /// is not reproducible, and this simulation's first rule is determinism. A
    /// route cost is seconds, so hundredths carry more precision than a report
    /// showing whole seconds can use.
    reroutes: Vec<[AtomicU32; 4]>,
}

/// Cost-to-fixed-point scale for the reroute sums.
const COST_SCALE: f32 = 100.0;

impl HoldAccumulator {
    /// Sizes the accumulator for a graph with `node_count` nodes.
    ///
    /// Called at tick start, before the parallel pass, so the allocation never
    /// happens while workers are writing.
    pub fn resize(&mut self, node_count: usize, edge_count: usize) {
        if self.slots.len() == node_count && self.arm_holds.len() == edge_count {
            self.clear();
            return;
        }
        self.slots = (0..node_count).map(|_| Default::default()).collect();
        self.reroutes = (0..node_count).map(|_| Default::default()).collect();
        self.arm_holds = (0..edge_count).map(|_| AtomicU32::new(0)).collect();
    }

    /// Zeroes every count without reallocating.
    pub fn clear(&self) {
        for slot in &self.slots {
            for c in slot {
                c.store(0, Ordering::Relaxed);
            }
        }
        for slot in &self.reroutes {
            for c in slot {
                c.store(0, Ordering::Relaxed);
            }
        }
    }

    /// Records a signal holding one car on `edge_id`'s approach.
    #[inline]
    pub fn record_arm_hold(&self, edge_id: usize) {
        if let Some(c) = self.arm_holds.get(edge_id) {
            c.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Zeroes the per-arm demand without touching the per-tick tallies.
    ///
    /// The hold and reroute counts answer "what is happening now" and are
    /// rebuilt every tick. Arm demand answers "which approach is starving",
    /// which a signal reads on a much slower cadence: a cycle is tens of
    /// seconds, so one tick of holds is a sixtieth of a second of evidence and
    /// retiming against it would be chasing noise. It therefore accumulates
    /// across ticks and is cleared by whoever consumes it.
    pub fn clear_arm_demand(&self) {
        for c in &self.arm_holds {
            c.store(0, Ordering::Relaxed);
        }
    }

    /// Cars a signal held on `edge_id`'s approach this window.
    ///
    /// The demand signal a timed program reallocates green against: an arm
    /// holding many cars is starving, one holding none is being given green it
    /// does not need.
    #[inline]
    pub fn arm_hold_count(&self, edge_id: usize) -> u32 {
        self.arm_holds
            .get(edge_id)
            .map(|c| c.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    /// Records a car that switched routes here.
    #[inline]
    pub fn record_reroute_taken(&self, node_id: usize) {
        if let Some(slot) = self.reroutes.get(node_id) {
            slot[0].fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Records a car that priced an alternative here and stayed on its route.
    ///
    /// Both costs are kept so the report can say what the detour would have
    /// cost, which is the question a delay raises and the one the comparison
    /// answers before discarding it.
    #[inline]
    pub fn record_reroute_rejected(&self, node_id: usize, current_cost: f32, candidate_cost: f32) {
        let Some(slot) = self.reroutes.get(node_id) else {
            return;
        };
        if !current_cost.is_finite() || !candidate_cost.is_finite() {
            return;
        }
        slot[1].fetch_add(1, Ordering::Relaxed);
        slot[2].fetch_add((current_cost.max(0.0) * COST_SCALE) as u32, Ordering::Relaxed);
        slot[3].fetch_add(
            (candidate_cost.max(0.0) * COST_SCALE) as u32,
            Ordering::Relaxed,
        );
    }

    /// Reroute decisions recorded at one junction.
    pub fn reroutes_at(&self, node_id: usize) -> JunctionReroutes {
        let Some(slot) = self.reroutes.get(node_id) else {
            return JunctionReroutes::default();
        };
        JunctionReroutes {
            taken: slot[0].load(Ordering::Relaxed),
            rejected: slot[1].load(Ordering::Relaxed),
            current_cost_sum: slot[2].load(Ordering::Relaxed) as f32 / COST_SCALE,
            candidate_cost_sum: slot[3].load(Ordering::Relaxed) as f32 / COST_SCALE,
        }
    }

    /// Records one hold. Safe to call from any worker.
    ///
    /// `Relaxed` because these are counters read after the pass joins: no other
    /// memory is published through them, so nothing needs ordering.
    #[inline]
    pub fn record(&self, node_id: usize, cause: HoldCause) {
        if let Some(slot) = self.slots.get(node_id) {
            slot[cause as usize].fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Collects the junctions that held anyone, heaviest first.
    ///
    /// Called after the pass joins. Junctions with no holds are omitted, so a
    /// quiet city produces an empty report rather than a page of zeroes.
    pub fn collect(&self) -> Vec<(usize, JunctionHolds)> {
        let mut out: Vec<(usize, JunctionHolds)> = Vec::new();
        for (node_id, slot) in self.slots.iter().enumerate() {
            let mut holds = JunctionHolds::default();
            for (i, c) in slot.iter().enumerate() {
                let n = c.load(Ordering::Relaxed);
                if n > 0 {
                    holds.add(HoldCause::ALL[i], n);
                }
            }
            if holds.total() > 0 {
                out.push((node_id, holds));
            }
        }
        out.sort_unstable_by_key(|&(node_id, h)| (std::cmp::Reverse(h.total()), node_id));
        out
    }

    /// Per-junction totals as a map, for a caller that wants to look one up.
    pub fn by_node(&self) -> HashMap<usize, JunctionHolds> {
        self.collect().into_iter().collect()
    }
}

// ========================================================================
// THE ACTIVE TALLY
// ========================================================================

// Where a hold site records, without threading a reference to it through the
// movement pass.
//
// Every hold happens five calls deep inside `process_agent_movement`, and the
// accumulator would have to reach them as a parameter on five signatures or as
// a borrow on `MovementSlices`, which puts a lifetime on a struct named at 68
// sites for the sake of a diagnostic counter. Neither is worth it.
//
// A raw pointer parked for the duration of the pass costs nothing at the call
// site and stays sound because the accumulator outlives the pass: it is owned by
// the `AgentSystem` the pass is running on, `record` only touches atomics, and
// `ACTIVE` is cleared before the borrow ends.
thread_local! {
    static ACTIVE: std::cell::Cell<*const HoldAccumulator> =
        const { std::cell::Cell::new(std::ptr::null()) };
}

/// Publishes `acc` as the tally for the current thread, for the duration of `f`.
///
/// Every worker thread that runs agent movement must be inside one of these for
/// its holds to be counted. Restores the previous value rather than clearing, so
/// nesting cannot silently drop an outer tally.
pub fn with_accumulator<R>(acc: &HoldAccumulator, f: impl FnOnce() -> R) -> R {
    let prev = ACTIVE.with(|a| a.replace(acc as *const _));
    let out = f();
    ACTIVE.with(|a| a.set(prev));
    out
}

/// Records one hold against the current thread's tally.
///
/// Does nothing when no tally is published, which is the case for every caller
/// outside the movement pass, including tests that drive a single agent.
/// Records a signal hold on one approach arm, against the current tally.
#[inline]
pub fn record_arm_hold(edge_id: usize) {
    ACTIVE.with(|a| {
        let p = a.get();
        if !p.is_null() {
            // Safety: as `record_hold`.
            unsafe { (*p).record_arm_hold(edge_id) };
        }
    });
}

/// Records a reroute the car took, against the current thread's tally.
#[inline]
pub fn record_reroute_taken(node_id: usize) {
    ACTIVE.with(|a| {
        let p = a.get();
        if !p.is_null() {
            // Safety: as `record_hold`.
            unsafe { (*p).record_reroute_taken(node_id) };
        }
    });
}

/// Records a reroute the car declined, with both prices it compared.
#[inline]
pub fn record_reroute_rejected(node_id: usize, current_cost: f32, candidate_cost: f32) {
    ACTIVE.with(|a| {
        let p = a.get();
        if !p.is_null() {
            // Safety: as `record_hold`.
            unsafe { (*p).record_reroute_rejected(node_id, current_cost, candidate_cost) };
        }
    });
}

#[inline]
pub fn record_hold(node_id: usize, cause: HoldCause) {
    ACTIVE.with(|a| {
        let p = a.get();
        if !p.is_null() {
            // Safety: `with_accumulator` published a reference that outlives
            // this call, and `record` touches only atomics.
            unsafe { (*p).record(node_id, cause) };
        }
    });
}

// ========================================================================
// TESTS
// ========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quiet_junction_reports_nothing() {
        let mut acc = HoldAccumulator::default();
        acc.resize(4, 8);
        assert!(acc.collect().is_empty());
    }

    #[test]
    fn holds_tally_per_cause_and_per_junction() {
        let mut acc = HoldAccumulator::default();
        acc.resize(3, 8);
        acc.record(1, HoldCause::SignalRed);
        acc.record(1, HoldCause::SignalRed);
        acc.record(1, HoldCause::Yielded);
        acc.record(2, HoldCause::ExitJammed);

        let by = acc.by_node();
        assert_eq!(by[&1].count(HoldCause::SignalRed), 2);
        assert_eq!(by[&1].count(HoldCause::Yielded), 1);
        assert_eq!(by[&1].total(), 3);
        assert_eq!(by[&2].total(), 1);
        assert!(!by.contains_key(&0), "a junction with no holds is omitted");
    }

    #[test]
    fn the_report_leads_with_what_held_the_most_cars() {
        // The question a heatmap cannot answer. This junction is busy, and the
        // reason is the signal rather than the geometry.
        let mut acc = HoldAccumulator::default();
        acc.resize(2, 8);
        for _ in 0..9 {
            acc.record(0, HoldCause::SignalRed);
        }
        acc.record(0, HoldCause::Conflict);
        let (cause, n) = acc.by_node()[&0].dominant().expect("holds were recorded");
        assert_eq!(cause, HoldCause::SignalRed);
        assert_eq!(n, 9);
    }

    #[test]
    fn junctions_sort_heaviest_first() {
        let mut acc = HoldAccumulator::default();
        acc.resize(3, 8);
        acc.record(0, HoldCause::Conflict);
        for _ in 0..5 {
            acc.record(2, HoldCause::Conflict);
        }
        let ranked = acc.collect();
        assert_eq!(ranked[0].0, 2, "the worst junction comes first");
        assert_eq!(ranked[1].0, 0);
    }

    #[test]
    fn a_node_id_past_the_end_is_dropped_rather_than_panicking() {
        // The graph can grow between the resize and the pass. Losing a count is
        // acceptable; a panic inside the parallel movement pass is not.
        let mut acc = HoldAccumulator::default();
        acc.resize(1, 8);
        acc.record(99, HoldCause::SignalRed);
        assert!(acc.collect().is_empty());
    }

    #[test]
    fn a_hold_recorded_outside_the_pass_is_dropped() {
        // No tally is published, so nothing is counted and nothing panics. Any
        // caller driving an agent outside the movement pass hits this, and a
        // diagnostic counter must never be the reason such a call fails.
        record_hold(0, HoldCause::SignalRed);
    }

    #[test]
    fn a_published_tally_receives_the_holds_recorded_under_it() {
        let mut acc = HoldAccumulator::default();
        acc.resize(2, 8);
        with_accumulator(&acc, || {
            record_hold(1, HoldCause::Yielded);
            record_hold(1, HoldCause::Yielded);
        });
        assert_eq!(acc.by_node()[&1].count(HoldCause::Yielded), 2);

        // The publication ends with the scope, so a later hold is dropped
        // rather than landing in last tick's tally.
        record_hold(1, HoldCause::Yielded);
        assert_eq!(acc.by_node()[&1].count(HoldCause::Yielded), 2);
    }

    #[test]
    fn arm_demand_survives_the_per_tick_reset() {
        // The bug this exists to catch. Holds and reroutes answer "what is
        // happening now" and are rebuilt every tick; arm demand answers "which
        // approach is starving" and is read on a much slower cadence. If the
        // tick reset wiped it, a signal retiming once a minute would be reading
        // one tick of evidence, which is a sixtieth of a second of traffic.
        let mut acc = HoldAccumulator::default();
        acc.resize(2, 8);
        acc.record_arm_hold(3);
        acc.record_arm_hold(3);

        // A tick boundary: same graph, so the counts are reset in place.
        acc.resize(2, 8);
        assert_eq!(
            acc.arm_hold_count(3),
            2,
            "arm demand accumulates across ticks"
        );
        assert!(acc.collect().is_empty(), "holds still reset every tick");

        // The consumer clears it when it has used it.
        acc.clear_arm_demand();
        assert_eq!(acc.arm_hold_count(3), 0);
    }

    #[test]
    fn a_graph_edit_resets_arm_demand() {
        // Edge ids shift when the graph changes, so demand measured against the
        // old ids is meaningless and must not carry over.
        let mut acc = HoldAccumulator::default();
        acc.resize(2, 8);
        acc.record_arm_hold(3);
        acc.resize(2, 9);
        assert_eq!(acc.arm_hold_count(3), 0);
    }

    #[test]
    fn a_declined_reroute_keeps_both_prices_it_compared() {
        // The third clause of the report's requirement: what the alternative
        // would have cost. The router computes both numbers and throws them
        // away, and the declined case is the one a delay raises.
        let mut acc = HoldAccumulator::default();
        acc.resize(2, 8);
        acc.record_reroute_rejected(1, 120.0, 110.0);
        acc.record_reroute_rejected(1, 100.0, 90.0);

        let rr = acc.reroutes_at(1);
        assert_eq!(rr.rejected, 2);
        assert_eq!(rr.taken, 0);
        assert_eq!(rr.mean_current_cost(), Some(110.0));
        assert_eq!(rr.mean_candidate_cost(), Some(100.0));
    }

    #[test]
    fn a_junction_nobody_repriced_reports_no_costs() {
        let mut acc = HoldAccumulator::default();
        acc.resize(1, 8);
        let rr = acc.reroutes_at(0);
        assert_eq!(rr.total(), 0);
        assert_eq!(rr.mean_current_cost(), None);
        assert_eq!(rr.mean_candidate_cost(), None);
    }

    #[test]
    fn taken_and_declined_reroutes_are_counted_apart() {
        // A junction cars route around and one they are stuck at look the same
        // in a total; they are different findings.
        let mut acc = HoldAccumulator::default();
        acc.resize(1, 8);
        acc.record_reroute_taken(0);
        acc.record_reroute_rejected(0, 50.0, 48.0);
        let rr = acc.reroutes_at(0);
        assert_eq!(rr.taken, 1);
        assert_eq!(rr.rejected, 1);
        assert_eq!(rr.total(), 2);
    }

    #[test]
    fn a_non_finite_cost_is_dropped_rather_than_poisoning_the_mean() {
        // An unreachable route prices as infinite. One of those in the sum
        // would make every mean at that junction meaningless.
        let mut acc = HoldAccumulator::default();
        acc.resize(1, 8);
        acc.record_reroute_rejected(0, f32::INFINITY, 10.0);
        acc.record_reroute_rejected(0, 20.0, 18.0);
        let rr = acc.reroutes_at(0);
        assert_eq!(rr.rejected, 1);
        assert_eq!(rr.mean_current_cost(), Some(20.0));
    }

    #[test]
    fn clearing_keeps_the_slots_and_zeroes_the_counts() {
        let mut acc = HoldAccumulator::default();
        acc.resize(2, 8);
        acc.record(0, HoldCause::SignalRed);
        acc.record_reroute_rejected(0, 10.0, 9.0);
        acc.clear();
        assert!(acc.collect().is_empty());
        assert_eq!(acc.reroutes_at(0).total(), 0, "reroutes clear with the holds");
        // Still sized, so the next tick records without reallocating.
        acc.record(1, HoldCause::Conflict);
        assert_eq!(acc.collect().len(), 1);
    }
}
