// =========================================================================
//  MANIFEST
// =========================================================================
//  script_name: border.rs
//  script_path: rust/src/simulation/network/border.rs
//  module_name: border
//  version: 0.1.0
//  description: The four border states and what each one looks like on the
//           ground. Border openness is a continuous policy because migration
//           multiplies by it, but a player cannot read a float off a
//           landscape, so the same number also resolves to one of exactly
//           four named states that decide what gets built at the crossing.
//           The state is derived from the policy rather than stored beside
//           it, so the two can never disagree.
//  kind: module
//  spec: none
//  internal_dependencies: [simulation/economy/fiscal.rs]
//  external_dependencies: []
//  features: [border-states, checkpoint-dressing, crossing-line, derived-presentation]
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-24
// =========================================================================

//! Border presentation: the line, the checkpoint, and what stands beside it.

// =========================================================================
// WHY FOUR STATES AND NOT A GRADIENT
// =========================================================================
// The simulation wants a continuous number: migration multiplies by border
// openness, and a policy that jumps in steps would make the population jump
// in steps too.
//
// A player wants something they can look at. Nobody reads 0.62 off a
// landscape. So the same number resolves to one of four states, and the
// state is what decides which props stand at the crossing. Openness moves
// smoothly; the crossing changes appearance four times across its range.
//
// The state is DERIVED, never stored. A stored copy would drift from the
// policy the first time something set one without the other.

/// How a border crossing presents itself, derived from border openness.
///
/// Four states, in order from sealed to open. Each names what a player sees
/// when they look at the crossing, not what the simulation does with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BorderState {
    /// Sealed. Nobody crosses.
    ///
    /// A wall across the line, the checkpoint shuttered, the far side's road
    /// running up to it and stopping. Whatever pressure was on the other side
    /// is still there and now has nowhere to go, so this is where the signs of
    /// overpopulation stack up: a camp, queued vehicles, lights burning at
    /// night against a dark strip of no-man's-land.
    Sealed,

    /// Restricted. A few cross, slowly.
    ///
    /// The wall becomes a fence, the checkpoint is manned and open, and the
    /// queue is long. Inspection bays, a holding area, a barrier arm. The far
    /// side still shows strain, because a trickle does not relieve it.
    Restricted,

    /// Controlled. The ordinary case for a working border.
    ///
    /// A staffed checkpoint with booths, no wall, a short queue that moves.
    /// Freight and people both pass. The far side looks like a place doing
    /// business rather than a place under pressure: depots, a fuel stop, the
    /// commerce that gathers wherever traffic has to slow down.
    #[default]
    Controlled,

    /// Open. The crossing is a formality.
    ///
    /// A sign, a line on the road, and the booths standing empty. Traffic does
    /// not slow. Industry and housing grow right up to the line on both sides,
    /// because a border nobody stops at stops being an edge.
    Open,
}

/// Openness at or below this is [`BorderState::Sealed`].
pub const SEALED_MAX: f32 = 0.05;
/// Openness at or below this is [`BorderState::Restricted`].
pub const RESTRICTED_MAX: f32 = 0.35;
/// Openness at or below this is [`BorderState::Controlled`]; above it, open.
pub const CONTROLLED_MAX: f32 = 0.85;

impl BorderState {
    /// Resolve the state a given openness presents as.
    ///
    /// The bands are uneven on purpose. Sealed is a narrow band at the bottom
    /// because a border that admits almost nobody should still look sealed,
    /// and open is a narrow band at the top for the same reason in reverse.
    /// Most of the range is the two middle states, which is where a player
    /// actually governs.
    pub fn from_openness(openness: f32) -> Self {
        if !openness.is_finite() || openness <= SEALED_MAX {
            Self::Sealed
        } else if openness <= RESTRICTED_MAX {
            Self::Restricted
        } else if openness <= CONTROLLED_MAX {
            Self::Controlled
        } else {
            Self::Open
        }
    }

    /// A stable identifier for the renderer and for save files.
    ///
    /// Ordinal rather than name, because it crosses the Rust to Godot boundary
    /// as a number and a renaming should not invalidate a save.
    pub fn as_ordinal(self) -> u8 {
        match self {
            Self::Sealed => 0,
            Self::Restricted => 1,
            Self::Controlled => 2,
            Self::Open => 3,
        }
    }

    /// Recover a state from its ordinal, defaulting to the ordinary case.
    pub fn from_ordinal(value: u8) -> Self {
        match value {
            0 => Self::Sealed,
            1 => Self::Restricted,
            3 => Self::Open,
            _ => Self::Controlled,
        }
    }

    /// The name a player sees.
    pub fn label(self) -> &'static str {
        match self {
            Self::Sealed => "Sealed",
            Self::Restricted => "Restricted",
            Self::Controlled => "Controlled",
            Self::Open => "Open",
        }
    }

    /// Whether a physical barrier stands across the line.
    ///
    /// A wall when sealed, a fence when restricted, nothing above that. The
    /// renderer picks the mesh; this decides whether one exists.
    pub fn barrier(self) -> BorderBarrier {
        match self {
            Self::Sealed => BorderBarrier::Wall,
            Self::Restricted => BorderBarrier::Fence,
            Self::Controlled | Self::Open => BorderBarrier::None,
        }
    }

    /// Whether the checkpoint is staffed and processing traffic.
    ///
    /// A sealed crossing has a checkpoint standing there shuttered, which is
    /// different from having no checkpoint: the building remains, because
    /// somebody built it when the border was open.
    pub fn checkpoint_staffed(self) -> bool {
        matches!(self, Self::Restricted | Self::Controlled)
    }

    /// How much strain shows on the far side, from 0.0 to 1.0.
    ///
    /// Pressure that cannot cross has to sit somewhere, so the tighter the
    /// border the more the other side looks crowded: queues, camps, lights.
    /// This drives density of that dressing rather than naming the props.
    pub fn far_side_strain(self) -> f32 {
        match self {
            Self::Sealed => 1.0,
            Self::Restricted => 0.6,
            Self::Controlled => 0.15,
            Self::Open => 0.0,
        }
    }

    /// How much ordinary development reaches the line, from 0.0 to 1.0.
    ///
    /// The inverse case: a crossing nobody stops at is not an edge, so
    /// industry and housing grow right up to it. A sealed border has a dead
    /// strip either side and nothing is built there.
    pub fn development_reach(self) -> f32 {
        match self {
            Self::Sealed => 0.0,
            Self::Restricted => 0.25,
            Self::Controlled => 0.7,
            Self::Open => 1.0,
        }
    }
}

/// What stands across the line, if anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BorderBarrier {
    /// Nothing blocks the road.
    #[default]
    None,
    /// A fence: a deterrent rather than a stop.
    Fence,
    /// A wall: the road ends here.
    Wall,
}

impl BorderBarrier {
    /// A stable identifier for the renderer.
    pub fn as_ordinal(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Fence => 1,
            Self::Wall => 2,
        }
    }
}

/// Everything the renderer needs to dress one border crossing.
///
/// Built from the policy each time it is asked for, so it cannot go stale.
#[derive(Clone, Copy, Debug)]
pub struct BorderPresentation {
    /// Which of the four states this crossing is in.
    pub state: BorderState,
    /// What stands across the line.
    pub barrier: BorderBarrier,
    /// Whether the checkpoint is manned.
    pub staffed: bool,
    /// Crowding on the far side, 0.0 to 1.0.
    pub far_side_strain: f32,
    /// How close ordinary building comes to the line, 0.0 to 1.0.
    pub development_reach: f32,
}

impl BorderPresentation {
    /// Resolve the whole presentation from one openness value.
    pub fn from_openness(openness: f32) -> Self {
        let state = BorderState::from_openness(openness);
        Self {
            state,
            barrier: state.barrier(),
            staffed: state.checkpoint_staffed(),
            far_side_strain: state.far_side_strain(),
            development_reach: state.development_reach(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openness_resolves_to_exactly_four_states() {
        let mut seen = Vec::new();
        // Walk the whole range and collect every distinct state produced.
        for step in 0..=100 {
            let state = BorderState::from_openness(step as f32 / 100.0);
            if !seen.contains(&state) {
                seen.push(state);
            }
        }
        assert_eq!(seen.len(), 4, "expected four states, saw {seen:?}");
    }

    #[test]
    fn the_ends_of_the_range_are_sealed_and_open() {
        assert_eq!(BorderState::from_openness(0.0), BorderState::Sealed);
        assert_eq!(BorderState::from_openness(1.0), BorderState::Open);
    }

    #[test]
    fn a_default_border_is_controlled() {
        // The ordinary case, so a crossing nobody has touched looks like a
        // working border rather than a wall or a formality.
        assert_eq!(BorderState::default(), BorderState::Controlled);
    }

    #[test]
    fn a_sealed_border_walls_the_road_and_keeps_its_checkpoint() {
        let p = BorderPresentation::from_openness(0.0);
        assert_eq!(p.barrier, BorderBarrier::Wall);
        // The building stays: somebody built it when the border was open, and
        // sealing it does not demolish it.
        assert!(!p.staffed);
        assert!((p.far_side_strain - 1.0).abs() < f32::EPSILON);
        assert!((p.development_reach - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn an_open_border_has_no_barrier_and_lets_building_reach_the_line() {
        let p = BorderPresentation::from_openness(1.0);
        assert_eq!(p.barrier, BorderBarrier::None);
        assert!(!p.staffed);
        assert!((p.far_side_strain - 0.0).abs() < f32::EPSILON);
        assert!((p.development_reach - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn strain_falls_and_development_rises_as_the_border_opens() {
        // The two dressings move in opposite directions across the whole
        // range, which is what makes the four states read as one spectrum
        // rather than four unrelated looks.
        let states = [
            BorderState::Sealed,
            BorderState::Restricted,
            BorderState::Controlled,
            BorderState::Open,
        ];
        for pair in states.windows(2) {
            assert!(
                pair[0].far_side_strain() > pair[1].far_side_strain(),
                "strain did not fall from {:?} to {:?}",
                pair[0],
                pair[1]
            );
            assert!(
                pair[0].development_reach() < pair[1].development_reach(),
                "development did not rise from {:?} to {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn ordinals_round_trip_and_an_unknown_one_is_the_ordinary_case() {
        for state in [
            BorderState::Sealed,
            BorderState::Restricted,
            BorderState::Controlled,
            BorderState::Open,
        ] {
            assert_eq!(BorderState::from_ordinal(state.as_ordinal()), state);
        }
        // A save written by a newer build must not panic an older one.
        assert_eq!(BorderState::from_ordinal(200), BorderState::Controlled);
    }

    #[test]
    fn a_non_finite_openness_reads_as_sealed() {
        // Fail closed: a corrupt value should not open a border.
        assert_eq!(BorderState::from_openness(f32::NAN), BorderState::Sealed);
    }
}
