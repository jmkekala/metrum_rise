// =========================================================================
//  MANIFEST
// =========================================================================
//  script_name: frontage.rs
//  script_path: rust/src/simulation/buildings/frontage.rs
//  module_name: frontage
//  version: 0.1.0
//  description: Frontage roles, which are what make an alley an alley. A
//           building held exactly one edge_idx, so it had exactly one
//           street, and every consequence the genre gets wrong about alleys
//           followed from that one field. A role says what a frontage is
//           FOR rather than how wide it is, because an alley is not a narrow
//           road: it is a second frontage on the far side of the parcel
//           serving a different purpose. The rule that stops houses fronting
//           onto alleys lives here.
//  kind: module
//  spec: none
//  internal_dependencies: [simulation/network/types.rs]
//  external_dependencies: []
//  features: [frontage-roles, service-access, water-frontage, allocator-rules]
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-24
// =========================================================================

//! What a frontage is for, and which edges may carry which kind.

// =========================================================================
// WHY A ROLE AND NOT A WIDTH
// =========================================================================
// Model an alley as a narrow road and the allocator will place houses
// fronting onto it, because to the allocator it is a road like any other.
// That result is wrong, it is what makes alleys look like a mistake, and it
// is why the genre cuts them.
//
// An alley is defined by what it SERVES: the back of a block, deliveries,
// waste, utility access. That is a relationship, not a dimension. So the
// frontage carries a role, an edge carries the roles it will accept, and the
// allocator refuses the combination that produces the wrong city.

/// What a frontage is for.
///
/// Ordered by how public it is: the address first, then the service side,
/// then the water. A building has exactly one `Primary` and may have one of
/// each other kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FrontageRole {
    /// The address. Pedestrians, residents, visitors, the front door.
    ///
    /// Exactly one per building. Everything that worked before this type
    /// existed is a `Primary` frontage, so a building that declares nothing
    /// else behaves exactly as it always did.
    #[default]
    Primary,

    /// The service side: deliveries, waste collection, and utility access.
    ///
    /// This is the alley. A building may have one, and it is where a truck
    /// docks rather than the street the residents use.
    Service,

    /// A navigable edge, for the canal case.
    ///
    /// A house with a boat behind it. Whether anyone uses the boat is a
    /// routing question the trip planner already answers by comparing costs;
    /// this only says the water is reachable from here.
    Water,
}

impl FrontageRole {
    /// A stable identifier for save files and for the Godot boundary.
    pub fn as_ordinal(self) -> u8 {
        match self {
            Self::Primary => 0,
            Self::Service => 1,
            Self::Water => 2,
        }
    }

    /// Recover a role from its ordinal, defaulting to the address.
    ///
    /// An unknown value reads as `Primary` because that is the behaviour
    /// every building had before roles existed, so an old or forward-dated
    /// save degrades to the working case rather than to nothing.
    pub fn from_ordinal(value: u8) -> Self {
        match value {
            1 => Self::Service,
            2 => Self::Water,
            _ => Self::Primary,
        }
    }

    /// The name a player sees.
    pub fn label(self) -> &'static str {
        match self {
            Self::Primary => "Address",
            Self::Service => "Service",
            Self::Water => "Water",
        }
    }
}

/// Which frontage roles an edge will accept.
///
/// This is the classification that keeps a city the right shape. A service
/// edge never accepts an address, so no matter how the allocator scores a
/// site, it cannot put a front door on an alley.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum EdgeFrontageClass {
    /// An ordinary street. Accepts an address, and a service frontage too,
    /// because a building on a street with no alley still takes deliveries
    /// from the kerb.
    #[default]
    Street,

    /// A service way: an alley, a service road, a loading court.
    ///
    /// Accepts service frontage only. **This is the rule that makes alleys
    /// work.** Without it an alley is a thin street and the allocator fills
    /// it with houses facing the wrong way.
    ServiceWay,

    /// A navigable waterway. Accepts water frontage only.
    Waterway,
}

impl EdgeFrontageClass {
    /// Whether an edge of this class will carry a frontage of that role.
    pub fn accepts(self, role: FrontageRole) -> bool {
        match self {
            Self::Street => matches!(role, FrontageRole::Primary | FrontageRole::Service),
            Self::ServiceWay => matches!(role, FrontageRole::Service),
            Self::Waterway => matches!(role, FrontageRole::Water),
        }
    }

    /// Whether a building may take its address from an edge of this class.
    pub fn can_address(self) -> bool {
        self.accepts(FrontageRole::Primary)
    }

    /// A stable identifier for save files and for the Godot boundary.
    pub fn as_ordinal(self) -> u8 {
        match self {
            Self::Street => 0,
            Self::ServiceWay => 1,
            Self::Waterway => 2,
        }
    }

    /// Recover a class from its ordinal, defaulting to an ordinary street.
    ///
    /// Every edge in an existing save is a street, so an unknown value
    /// reading as `Street` is what makes this change invisible to old saves.
    pub fn from_ordinal(value: u8) -> Self {
        match value {
            1 => Self::ServiceWay,
            2 => Self::Waterway,
            _ => Self::Street,
        }
    }

    /// The name a player sees.
    pub fn label(self) -> &'static str {
        match self {
            Self::Street => "Street",
            Self::ServiceWay => "Service Way",
            Self::Waterway => "Waterway",
        }
    }
}

/// The most frontages one building can hold.
///
/// One address, one service side, one water edge. A fixed array of this size
/// rather than a `Vec`, because the allocator rebuilds every entrance on a
/// dirty flag and their performance rule makes a per-building allocation in
/// that path a correctness issue rather than a style one.
pub const MAX_FRONTAGES: usize = 3;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_service_way_refuses_an_address() {
        // The single most important line in the alley design: an alley that
        // accepts a front door is just a narrow street, and the allocator
        // will fill it with houses facing the wrong way.
        assert!(!EdgeFrontageClass::ServiceWay.can_address());
        assert!(!EdgeFrontageClass::ServiceWay.accepts(FrontageRole::Primary));
        assert!(EdgeFrontageClass::ServiceWay.accepts(FrontageRole::Service));
    }

    #[test]
    fn an_ordinary_street_takes_both_an_address_and_deliveries() {
        // A building on a street with no alley still receives deliveries, at
        // the kerb, so a street has to accept both roles.
        let street = EdgeFrontageClass::Street;
        assert!(street.accepts(FrontageRole::Primary));
        assert!(street.accepts(FrontageRole::Service));
        assert!(!street.accepts(FrontageRole::Water));
    }

    #[test]
    fn a_waterway_takes_only_water() {
        let water = EdgeFrontageClass::Waterway;
        assert!(water.accepts(FrontageRole::Water));
        assert!(!water.accepts(FrontageRole::Primary));
        assert!(!water.accepts(FrontageRole::Service));
    }

    #[test]
    fn the_defaults_reproduce_the_behaviour_before_roles_existed() {
        // Every building had one street and one front door. If the defaults
        // did anything else, this change would alter every existing city.
        assert_eq!(FrontageRole::default(), FrontageRole::Primary);
        assert_eq!(EdgeFrontageClass::default(), EdgeFrontageClass::Street);
        assert!(EdgeFrontageClass::default().can_address());
    }

    #[test]
    fn ordinals_round_trip_and_unknown_values_degrade_to_the_working_case() {
        for role in [
            FrontageRole::Primary,
            FrontageRole::Service,
            FrontageRole::Water,
        ] {
            assert_eq!(FrontageRole::from_ordinal(role.as_ordinal()), role);
        }
        for class in [
            EdgeFrontageClass::Street,
            EdgeFrontageClass::ServiceWay,
            EdgeFrontageClass::Waterway,
        ] {
            assert_eq!(EdgeFrontageClass::from_ordinal(class.as_ordinal()), class);
        }
        // A save from a newer build must not break an older one, and the
        // fallback has to be the case that works rather than the case that
        // silently drops a building off the road network.
        assert_eq!(FrontageRole::from_ordinal(200), FrontageRole::Primary);
        assert_eq!(
            EdgeFrontageClass::from_ordinal(200),
            EdgeFrontageClass::Street
        );
    }

    #[test]
    fn every_role_is_accepted_by_exactly_one_or_two_edge_classes() {
        // A role no edge accepts is unreachable; a role every edge accepts
        // enforces nothing. Both are design errors, so this pins the shape.
        let classes = [
            EdgeFrontageClass::Street,
            EdgeFrontageClass::ServiceWay,
            EdgeFrontageClass::Waterway,
        ];
        for role in [
            FrontageRole::Primary,
            FrontageRole::Service,
            FrontageRole::Water,
        ] {
            let n = classes.iter().filter(|c| c.accepts(role)).count();
            assert!(
                n >= 1 && n < classes.len(),
                "{role:?} accepted by {n} classes"
            );
        }
    }
}
