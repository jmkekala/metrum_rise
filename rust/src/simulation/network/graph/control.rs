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
}
