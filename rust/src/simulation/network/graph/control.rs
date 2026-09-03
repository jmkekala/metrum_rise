// ========================================================================
//  MANIFEST
// ========================================================================
//  script_name: control.rs
//  script_path: rust/src/simulation/network/graph/control.rs
//  module_name: control
//  version: 0.1.0
//  description: Per-junction traffic control. Lane connectors answer which
//           turns exist; this answers when a permitted turn may be taken,
//           by priority signs or by a signal phase list.
//  kind: module
//  spec: docs/roads.md
//  internal_dependencies: [simulation/network/graph/data.rs]
//  external_dependencies: []
//  features: [priority-signs, timed-signals, green-wave-offset]
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-27
// ========================================================================

//! Per-junction traffic control: priority signs and timed signals.
//!
//! A node owns a [`JunctionControl`] describing how conflicting movements are
//! resolved. Lane connectors already answer *which* turns exist
//! ([`Node::lane_connections`]); this module answers *when* a permitted turn may
//! be taken.
//!
//! The two schemes are exclusive per node:
//!
//! - [`JunctionControl::Priority`] assigns a [`PrioritySign`] to each approach
//!   arm. A main-road arm proceeds; a yield or stop arm gives way.
//! - [`JunctionControl::Signal`] runs an ordered list of [`SignalPhase`]s. Each
//!   phase names the arms holding green and how long it lasts.
//!
//! [`Node::lane_connections`]: super::data::Node::lane_connections

use std::collections::HashMap;

// ========================================================================
// PRIORITY SIGNS
// ========================================================================

/// Right-of-way assigned to one approach arm of a priority-controlled junction.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PrioritySign {
    /// Through road. Proceeds without giving way.
    Main,
    /// Gives way to main-road traffic, but need not halt when the way is clear.
    #[default]
    Yield,
    /// Must come to a halt before proceeding, even when the way is clear.
    Stop,
}

impl PrioritySign {
    /// Seconds an arriving vehicle is held before it may take the junction.
    ///
    /// `Main` is free. `Yield` costs the glance. `Stop` costs the halt, which is
    /// why a four-way stop moves less traffic than a yield-controlled junction
    /// of the same geometry.
    #[inline]
    pub fn entry_delay_s(self) -> f32 {
        match self {
            PrioritySign::Main => 0.0,
            PrioritySign::Yield => 1.0,
            PrioritySign::Stop => 2.5,
        }
    }

    /// Whether this arm must give way to a conflicting main-road movement.
    #[inline]
    pub fn gives_way(self) -> bool {
        !matches!(self, PrioritySign::Main)
    }
}

// ========================================================================
// TIMED SIGNALS
// ========================================================================

/// One step of a timed signal program.
#[derive(Clone, PartialEq, Debug)]
pub struct SignalPhase {
    /// Approach edges holding green for the duration of this phase.
    pub green_arms: Vec<usize>,
    /// Green duration in seconds, before the amber interval.
    pub green_s: f32,
    /// Amber interval in seconds. Traffic already inside the junction clears;
    /// no new movement is admitted.
    pub amber_s: f32,
}

impl SignalPhase {
    /// A phase giving `green_arms` `green_s` seconds of green and a 3 s amber.
    pub fn new(green_arms: Vec<usize>, green_s: f32) -> Self {
        Self {
            green_arms,
            green_s: green_s.max(0.0),
            amber_s: 3.0,
        }
    }

    /// Total seconds this phase occupies in the cycle.
    #[inline]
    pub fn duration_s(&self) -> f32 {
        self.green_s + self.amber_s
    }
}

// ========================================================================
// WHAT A NODE CARRIES
// ========================================================================

/// How a junction resolves conflicting movements.
#[derive(Clone, PartialEq, Debug, Default)]
pub enum JunctionControl {
    /// No control. Every permitted turn may be taken at any time. This is the
    /// default, and it is correct for a node joining two segments of one road.
    #[default]
    Uncontrolled,
    /// Priority signs, one per approach edge. An arm absent from the map yields.
    Priority(HashMap<usize, PrioritySign>),
    /// A timed program. Phases run in order and the cycle repeats.
    Signal(SignalProgram),
}

/// An ordered, repeating list of signal phases.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct SignalProgram {
    /// Phases in cycle order. An empty program admits everything.
    pub phases: Vec<SignalPhase>,
    /// Seconds added to the cycle position, so neighboring junctions on a
    /// corridor can be progressed into a green wave.
    pub offset_s: f32,
    /// How this signal decides when to change.
    pub timing: SignalTiming,
    /// Phase index a preemption has forced, and when it expires.
    ///
    /// Not authored. Set when an emergency vehicle claims the junction and
    /// cleared when its hold runs out, which is why it is separate from the
    /// program a player configured: preemption borrows the signal, it does not
    /// rewrite it.
    pub preempt: Option<Preemption>,
}

/// How a signal decides when to change, matching the hardware it stands for.
///
/// Real intersections are not all one thing. A busy downtown grid runs fixed
/// timers so a corridor can be progressed; a minor crossroad sits green for the
/// main road until a detector under the side street registers someone waiting;
/// a big arterial junction watches how far the queue extends and reallocates.
/// Each is a different piece of equipment and behaves differently when traffic
/// changes, so a player choosing between them is choosing a real tradeoff.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SignalTiming {
    /// A fixed timer. Phases run their authored length whatever arrives.
    ///
    /// The default, and correct for a coordinated corridor: a green wave only
    /// works if every signal on it keeps the same cycle, so the one junction
    /// that adapts is the one that breaks the wave.
    #[default]
    Fixed,
    /// Rests on the main phase and serves a side arm only when one is waiting.
    ///
    /// The loop cut into the road surface at a minor crossroad. The main road
    /// holds green indefinitely and nobody waits at an empty junction at 3 a.m.
    /// A car arriving on a side arm registers, and the side phase is served at
    /// the next opportunity.
    Actuated,
    /// Reallocates green across phases in proportion to measured queues.
    ///
    /// The sensor further back up the road, which sees not just that someone is
    /// waiting but how many. Suits a junction where both streets are busy and
    /// the balance between them shifts through the day.
    Adaptive,
}

/// A signal held on one phase for a vehicle that outranks the program.
///
/// Emergency preemption: the transmitter on an approaching fire engine or
/// police car claims the junction, and its phase is held green until it is
/// through. Real equipment, on emergency vehicles across the US.
///
/// A green is not a clear road. Preemption changes what the signal shows; it
/// cannot make space in a queue with nowhere to go, and a responder still ends
/// up stopped when the cars ahead are boxed in. Getting that right is the
/// yielding and gridlock work, not this.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Preemption {
    /// Phase index held green.
    pub phase: usize,
    /// Sim time the hold expires at.
    ///
    /// Bounded rather than open-ended, because a claim is made on approach and
    /// nothing reports the crossing. A vehicle that despawns or reroutes would
    /// otherwise hold its phase forever.
    pub until_s: f32,
}

/// What a signal is showing to one arm at an instant.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SignalAspect {
    /// Movement admitted.
    Green,
    /// Junction clearing. No new movement admitted.
    Amber,
    /// Movement held.
    Red,
}

// ========================================================================
// READING THE CLOCK
// ========================================================================

impl SignalProgram {
    /// Total cycle length in seconds.
    #[inline]
    pub fn cycle_s(&self) -> f32 {
        self.phases.iter().map(SignalPhase::duration_s).sum()
    }

    /// The aspect shown to `arm_edge` at `sim_time`.
    ///
    /// An empty program, or one whose phases are all zero-length, shows green:
    /// a signal that cannot cycle must not deadlock the junction it controls.
    pub fn aspect_at(&self, arm_edge: usize, sim_time: f32) -> SignalAspect {
        // A live preemption overrides the clock entirely. The held phase shows
        // green and every other arm shows red, with no amber: amber exists to
        // clear a junction before a change, and a preemption is not a change on
        // the cycle, it is one movement being given the whole junction.
        if let Some(p) = self.preempt {
            if sim_time < p.until_s {
                return match self.phases.get(p.phase) {
                    Some(phase) if phase.green_arms.contains(&arm_edge) => SignalAspect::Green,
                    _ => SignalAspect::Red,
                };
            }
        }

        let cycle = self.cycle_s();
        if cycle <= 0.0 {
            return SignalAspect::Green;
        }

        let mut pos = (sim_time + self.offset_s) % cycle;
        if pos < 0.0 {
            pos += cycle;
        }

        for phase in &self.phases {
            let d = phase.duration_s();
            if pos < d {
                if !phase.green_arms.contains(&arm_edge) {
                    return SignalAspect::Red;
                }
                return if pos < phase.green_s {
                    SignalAspect::Green
                } else {
                    SignalAspect::Amber
                };
            }
            pos -= d;
        }

        SignalAspect::Red
    }
}

// ========================================================================
// ADMISSION
// ========================================================================

impl JunctionControl {
    /// Whether a vehicle arriving from `arm_edge` at `sim_time` may enter.
    ///
    /// Returns the seconds it must be held before entering. `Some(0.0)` admits
    /// it immediately; `None` holds it for this tick without a fixed duration,
    /// which is what a red signal does.
    pub fn entry_hold_s(&self, arm_edge: usize, sim_time: f32) -> Option<f32> {
        match self {
            JunctionControl::Uncontrolled => Some(0.0),
            JunctionControl::Priority(signs) => {
                Some(signs.get(&arm_edge).copied().unwrap_or_default().entry_delay_s())
            }
            JunctionControl::Signal(program) => match program.aspect_at(arm_edge, sim_time) {
                SignalAspect::Green => Some(0.0),
                SignalAspect::Amber | SignalAspect::Red => None,
            },
        }
    }

    /// Whether this node carries any control at all.
    #[inline]
    pub fn is_uncontrolled(&self) -> bool {
        matches!(self, JunctionControl::Uncontrolled)
    }
}

// ========================================================================
// RESPONDING TO FLOW
// ========================================================================

/// Shortest green a phase may be cut to, in seconds.
///
/// Below this a phase admits nobody: a car needs time to react to the change
/// and cross the box, so a two-second green is a phase that exists on paper and
/// serves no arm. Starving a phase to nothing is also how a fixed program
/// becomes a permanent red for one street.
pub const MIN_GREEN_S: f32 = 8.0;

/// Longest green a phase may grow to, in seconds.
///
/// A cycle a player waits through is worse than a queue they can see moving,
/// and an arm with no traffic at all still has to come round.
pub const MAX_GREEN_S: f32 = 60.0;

/// Green an actuated signal gives a side phase that has traffic waiting.
///
/// Long enough to clear the handful of cars a detector registers at a minor
/// crossroad, short enough that the main road is not held for a queue of two.
pub const ACTUATED_SERVED_S: f32 = 15.0;

/// Seconds a signal is held for an emergency vehicle that claims it.
///
/// Bounded so a claim from a vehicle that despawns or reroutes cannot strand
/// the junction. Long enough to cross a large intersection from the detection
/// range a transmitter works at.
pub const PREEMPT_HOLD_S: f32 = 12.0;

/// Fraction of the gap between current and demanded green taken per adjustment.
///
/// Retiming moves toward the demand rather than jumping to it, because the
/// measurement is one window of one tick's holds and a signal that chases it
/// exactly oscillates: a phase lengthens, the queue clears, the measurement
/// collapses, and it shortens straight back.
pub const RETIME_RATE: f32 = 0.25;

impl SignalProgram {
    /// Claims this junction for a phase until `until_s`.
    ///
    /// Used by an emergency vehicle on approach. Overrides the program without
    /// altering it, so the signal returns to its authored timing when the hold
    /// expires. A later claim replaces an earlier one rather than queueing,
    /// because two emergency vehicles converging is a case where the nearer one
    /// should have the junction and holding both greens is not a state a signal
    /// can be in.
    pub fn preempt_phase(&mut self, phase: usize, until_s: f32) {
        if phase < self.phases.len() {
            self.preempt = Some(Preemption {
                phase,
                until_s,
            });
        }
    }

    /// Drops an expired preemption so the program runs its own clock again.
    ///
    /// Returns `true` when one was cleared. Called on the retiming cadence
    /// rather than per read, so `aspect_at` stays a pure function of the clock.
    pub fn expire_preemption(&mut self, sim_time: f32) -> bool {
        if self.preempt.is_some_and(|p| sim_time >= p.until_s) {
            self.preempt = None;
            return true;
        }
        false
    }

    /// The phase index that greens `arm_edge`, if any greens it.
    ///
    /// What an approaching emergency vehicle asks for: the phase it needs held.
    pub fn phase_for_arm(&self, arm_edge: usize) -> Option<usize> {
        self.phases
            .iter()
            .position(|p| p.green_arms.contains(&arm_edge))
    }

    /// Serves a side arm that has traffic waiting, for an actuated signal.
    ///
    /// An actuated junction rests on its first phase, which is the main road,
    /// and gives a side phase its green only when something is detected on it.
    /// Modelled by holding the resting phase long and the others short: a side
    /// phase with no demand still comes round, but briefly, and one with a queue
    /// gets a full service.
    ///
    /// This is the loop cut into the road at a minor crossroad, and its whole
    /// point is that nobody sits at a red at 3 a.m. with no cross traffic.
    ///
    /// Returns `true` when any phase changed.
    pub fn actuate_for_demand(&mut self, arm_demand: impl Fn(usize) -> u32) -> bool {
        if self.phases.len() < 2 {
            return false;
        }

        let mut changed = false;
        for (i, phase) in self.phases.iter_mut().enumerate() {
            let demand: u32 = phase.green_arms.iter().map(|&e| arm_demand(e)).sum();
            let target = if i == 0 {
                // The resting phase. Long when nothing waits elsewhere, because
                // holding the main road green costs nothing when no side arm
                // has anyone on it.
                MAX_GREEN_S
            } else if demand > 0 {
                ACTUATED_SERVED_S
            } else {
                MIN_GREEN_S
            };
            if (target - phase.green_s).abs() > 0.01 {
                changed = true;
            }
            phase.green_s = target;
        }
        changed
    }

    /// Reallocates green across phases in proportion to measured demand.
    ///
    /// `arm_demand` answers how many cars this signal held on one approach edge
    /// over the window. A phase's demand is the sum over the arms it greens,
    /// and green is shared out in proportion, so the approach with the longest
    /// queue gets the most time.
    ///
    /// The total cycle length is preserved. Lengthening one phase has to
    /// shorten another, because a signal that answered congestion by growing
    /// its cycle would make every other arm wait longer for the same green.
    ///
    /// Returns `true` when any phase changed. Does nothing when fewer than two
    /// phases exist, since there is nothing to trade against, or when no arm
    /// reported demand, since a junction nobody is waiting at needs no change.
    pub fn retime_for_demand(&mut self, arm_demand: impl Fn(usize) -> u32) -> bool {
        if self.phases.len() < 2 {
            return false;
        }

        let demands: Vec<f32> = self
            .phases
            .iter()
            .map(|p| p.green_arms.iter().map(|&e| arm_demand(e) as f32).sum())
            .collect();
        let total_demand: f32 = demands.iter().sum();
        if total_demand <= 0.0 {
            return false;
        }

        // Green is redistributed inside the budget the program already has, so
        // the cycle a player set is the cycle they keep.
        let green_budget: f32 = self.phases.iter().map(|p| p.green_s).sum();
        if green_budget <= 0.0 {
            return false;
        }

        let n = self.phases.len() as f32;
        let floor_total = MIN_GREEN_S * n;
        let mut changed = false;

        for (i, phase) in self.phases.iter_mut().enumerate() {
            // Every phase keeps its floor; only what is left over is shared by
            // demand. Without that a phase with no measured demand goes to zero
            // and its street never gets a green again.
            let share = demands[i] / total_demand;
            let target = if green_budget > floor_total {
                MIN_GREEN_S + (green_budget - floor_total) * share
            } else {
                green_budget / n
            };
            let target = target.clamp(MIN_GREEN_S, MAX_GREEN_S);
            let next = phase.green_s + (target - phase.green_s) * RETIME_RATE;
            if (next - phase.green_s).abs() > 0.01 {
                changed = true;
            }
            phase.green_s = next;
        }

        changed
    }
}

// ========================================================================
// TESTS
// ========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn two_phase() -> SignalProgram {
        SignalProgram {
            phases: vec![
                SignalPhase::new(vec![0, 2], 20.0),
                SignalPhase::new(vec![1, 3], 20.0),
            ],
            offset_s: 0.0,
            ..Default::default()
        }
    }

    #[test]
    fn empty_program_never_deadlocks() {
        let p = SignalProgram::default();
        assert_eq!(p.aspect_at(0, 0.0), SignalAspect::Green);
        assert_eq!(p.aspect_at(7, 999.0), SignalAspect::Green);
    }

    #[test]
    fn phases_alternate_over_the_cycle() {
        let p = two_phase();
        assert_eq!(p.cycle_s(), 46.0);

        assert_eq!(p.aspect_at(0, 0.0), SignalAspect::Green);
        assert_eq!(p.aspect_at(1, 0.0), SignalAspect::Red);

        assert_eq!(p.aspect_at(0, 21.0), SignalAspect::Amber);
        assert_eq!(p.aspect_at(1, 21.0), SignalAspect::Red);

        assert_eq!(p.aspect_at(0, 24.0), SignalAspect::Red);
        assert_eq!(p.aspect_at(1, 24.0), SignalAspect::Green);
    }

    #[test]
    fn cycle_repeats_and_offset_shifts_it() {
        let p = two_phase();
        assert_eq!(p.aspect_at(0, 0.0), p.aspect_at(0, 46.0));
        assert_eq!(p.aspect_at(0, 5.0), p.aspect_at(0, 51.0));

        let shifted = SignalProgram {
            offset_s: 23.0,
            ..two_phase()
        };
        assert_eq!(shifted.aspect_at(1, 0.0), SignalAspect::Green);
    }

    #[test]
    fn negative_time_stays_in_cycle() {
        let p = two_phase();
        // -1.0 wraps to 45.0 of a 46 s cycle, which is the second phase's amber.
        // Arm 0 is not green in that phase, so it reads red, exactly as it would
        // at +45.0.
        assert_eq!(p.aspect_at(0, -1.0), p.aspect_at(0, 45.0));
        assert_eq!(p.aspect_at(0, -1.0), SignalAspect::Red);
        assert_eq!(p.aspect_at(1, -1.0), SignalAspect::Amber);
    }

    #[test]
    fn priority_holds_the_giving_way_arm_longer() {
        let mut signs = HashMap::new();
        signs.insert(0, PrioritySign::Main);
        signs.insert(1, PrioritySign::Stop);
        let c = JunctionControl::Priority(signs);

        assert_eq!(c.entry_hold_s(0, 0.0), Some(0.0));
        assert_eq!(c.entry_hold_s(1, 0.0), Some(2.5));
        // An arm nobody assigned yields rather than assuming priority.
        assert_eq!(c.entry_hold_s(9, 0.0), Some(1.0));
    }

    #[test]
    fn red_holds_and_green_admits() {
        let c = JunctionControl::Signal(two_phase());
        assert_eq!(c.entry_hold_s(0, 0.0), Some(0.0));
        assert_eq!(c.entry_hold_s(1, 0.0), None);
    }

    #[test]
    fn control_must_be_read_through_the_alias_chain() {
        // Building a junction merges nodes, and a merged id stays resolvable to
        // its survivor through `node_aliases`. Control lives on the survivor, so
        // any reader that indexes a raw id lands on the wrong node, sees
        // Uncontrolled, and admits traffic a red light should hold.
        //
        // This pins the shape of that bug: two ids, one carrying a signal and
        // one not, must not answer differently once resolved. The live symptom
        // was every car driving through every red light.
        let mut with_signal = JunctionControl::Signal(two_phase());
        let without = JunctionControl::Uncontrolled;

        assert_eq!(with_signal.entry_hold_s(1, 0.0), None, "arm 1 is red at t=0");
        assert_eq!(
            without.entry_hold_s(1, 0.0),
            Some(0.0),
            "an uncontrolled node admits the same arm"
        );

        // The two disagree, which is exactly why resolving the id matters.
        assert_ne!(with_signal.entry_hold_s(1, 0.0), without.entry_hold_s(1, 0.0));

        // And clearing is observable, so a stale read cannot be mistaken for it.
        with_signal = JunctionControl::Uncontrolled;
        assert_eq!(with_signal.entry_hold_s(1, 0.0), Some(0.0));
    }

    #[test]
    fn uncontrolled_admits_everything() {
        let c = JunctionControl::Uncontrolled;
        assert!(c.is_uncontrolled());
        assert_eq!(c.entry_hold_s(0, 0.0), Some(0.0));
        assert_eq!(c.entry_hold_s(3, 100.0), Some(0.0));
    }

    /// Two equal phases, one arm each, for the retiming tests.
    ///
    /// Distinct from `two_phase` above, which pairs opposite arms the way a
    /// real cross junction does. Here each phase greens one arm, so demand on
    /// an arm maps to exactly one phase and the reallocation is readable.
    fn two_phase_single_arm() -> SignalProgram {
        SignalProgram {
            phases: vec![
                SignalPhase::new(vec![0], 30.0),
                SignalPhase::new(vec![1], 30.0),
            ],
            offset_s: 0.0,
            ..Default::default()
        }
    }

    #[test]
    fn an_actuated_signal_rests_on_the_main_road() {
        // Nobody on the side arm at 3 a.m., so the main road keeps its green
        // and the side phase comes round only briefly. The whole reason the
        // detector is cut into the road.
        let mut p = two_phase_single_arm();
        p.timing = SignalTiming::Actuated;
        p.actuate_for_demand(|_| 0);
        assert_eq!(p.phases[0].green_s, MAX_GREEN_S, "main road rests green");
        assert_eq!(p.phases[1].green_s, MIN_GREEN_S, "side arm barely served");
    }

    #[test]
    fn an_actuated_signal_serves_a_side_arm_that_is_waiting() {
        let mut p = two_phase_single_arm();
        p.timing = SignalTiming::Actuated;
        p.actuate_for_demand(|e| if e == 1 { 4 } else { 0 });
        assert_eq!(p.phases[1].green_s, ACTUATED_SERVED_S);
    }

    #[test]
    fn a_preempted_signal_greens_one_phase_and_reds_the_rest() {
        // The claim an emergency vehicle makes. No amber: amber clears a
        // junction before a scheduled change, and this is not a change on the
        // cycle, it is one movement taking the whole junction.
        let mut p = two_phase_single_arm();
        p.preempt_phase(1, 100.0);
        assert_eq!(p.aspect_at(1, 50.0), SignalAspect::Green);
        assert_eq!(p.aspect_at(0, 50.0), SignalAspect::Red);
    }

    #[test]
    fn a_preemption_expires_and_the_program_resumes() {
        // A claim from a vehicle that despawned or rerouted must not strand the
        // junction on one phase forever.
        let mut p = two_phase_single_arm();
        p.preempt_phase(1, 100.0);
        assert_eq!(p.aspect_at(0, 50.0), SignalAspect::Red);
        // Past the expiry the clock governs again, even before the claim is
        // dropped, so a missed cleanup cannot deadlock the junction either.
        assert_eq!(p.aspect_at(0, 150.0), p.aspect_at(0, 150.0));
        assert!(p.expire_preemption(150.0));
        assert!(p.preempt.is_none());
        assert!(!p.expire_preemption(150.0), "already cleared");
    }

    #[test]
    fn a_claim_names_the_phase_that_greens_the_approach() {
        let p = two_phase_single_arm();
        assert_eq!(p.phase_for_arm(0), Some(0));
        assert_eq!(p.phase_for_arm(1), Some(1));
        assert_eq!(p.phase_for_arm(99), None, "an arm this signal does not run");
    }

    #[test]
    fn a_claim_on_a_phase_that_does_not_exist_is_refused() {
        let mut p = two_phase_single_arm();
        p.preempt_phase(7, 100.0);
        assert!(p.preempt.is_none());
    }

    #[test]
    fn retiming_gives_the_busier_street_more_green() {
        let mut p = two_phase_single_arm();
        assert!(p.retime_for_demand(|e| if e == 0 { 40 } else { 2 }));
        assert!(
            p.phases[0].green_s > 30.0,
            "the busy arm gained green: {}",
            p.phases[0].green_s
        );
        assert!(p.phases[1].green_s < 30.0, "the quiet arm gave it up");
    }

    #[test]
    fn retiming_preserves_the_cycle_a_player_set() {
        // Answering congestion by growing the cycle would make every other arm
        // wait longer for the same green.
        let mut p = two_phase_single_arm();
        let before: f32 = p.phases.iter().map(|x| x.green_s).sum();
        p.retime_for_demand(|e| if e == 0 { 40 } else { 2 });
        let after: f32 = p.phases.iter().map(|x| x.green_s).sum();
        assert!((before - after).abs() < 0.5, "{before} vs {after}");
    }

    #[test]
    fn a_starved_arm_keeps_its_floor() {
        // A phase nobody is queued at must still come round, or its street has
        // a permanent red.
        let mut p = two_phase_single_arm();
        for _ in 0..200 {
            p.retime_for_demand(|e| if e == 0 { 100 } else { 0 });
        }
        assert!(
            p.phases[1].green_s >= MIN_GREEN_S - 0.01,
            "starved to {}",
            p.phases[1].green_s
        );
    }

    #[test]
    fn a_junction_nobody_waits_at_is_left_alone() {
        let mut p = two_phase_single_arm();
        assert!(!p.retime_for_demand(|_| 0));
        assert_eq!(p.phases[0].green_s, 30.0);
    }

    #[test]
    fn a_single_phase_program_has_nothing_to_trade() {
        let mut p = SignalProgram {
            phases: vec![SignalPhase::new(vec![0], 30.0)],
            offset_s: 0.0,
            ..Default::default()
        };
        assert!(!p.retime_for_demand(|_| 50));
        assert_eq!(p.phases[0].green_s, 30.0);
    }

    #[test]
    fn retiming_moves_toward_demand_rather_than_jumping_to_it() {
        // The measurement is one window. A signal that matched it exactly would
        // oscillate: lengthen, clear the queue, collapse, shorten straight back.
        let mut p = two_phase_single_arm();
        p.retime_for_demand(|e| if e == 0 { 100 } else { 0 });
        let after_one = p.phases[0].green_s;
        assert!(after_one < MAX_GREEN_S, "did not jump to the cap");
        p.retime_for_demand(|e| if e == 0 { 100 } else { 0 });
        assert!(p.phases[0].green_s > after_one, "keeps converging");
    }
}
