//! Agent simulation: data layout, activity states, and lifecycle management.

mod building_refs;
mod daily;
pub mod data;
mod determinism;
mod lifecycle;
mod remap;
#[cfg(test)]
mod test_departure_side;
#[cfg(test)]
mod tests;
pub mod tick;

pub use data::{Agent, AgentSystem, AgentVec};

use determinism::stable_index;

/// Child resident: consumes household resources but does not work or shop.
pub const AGE_CHILD: u8 = 0;
/// Adult resident: can work, shop, and anchor a household.
pub const AGE_ADULT: u8 = 1;
/// Elder resident: can shop and anchor a household, but does not work.
pub const AGE_ELDER: u8 = 2;

pub(crate) const MAX_ADULTS_PER_HOUSEHOLD: u16 = 2;
pub(crate) const MAX_ELDERS_PER_HOUSEHOLD: u16 = 2;
const SECOND_ADULT_PERCENT: usize = 55;
const ONE_ELDER_PERCENT: usize = 17;
const TWO_ELDER_PERCENT: usize = 8;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct HouseholdAgeComposition {
    pub(crate) child_count: u16,
    pub(crate) adult_count: u16,
    pub(crate) elder_count: u16,
}

/// Agent is at home or travelling to a home stop.
pub const ACTIVITY_HOME: u8 = 0;
/// Agent is at work or travelling to a work stop.
pub const ACTIVITY_WORK: u8 = 1;
/// Agent is shopping or at another non-home, non-work stop.
pub const ACTIVITY_SHOPPING: u8 = 2;

/// Agent is inside a building and hidden until the next trip trigger fires.
pub const TRANSIT_IN_BUILDING: u8 = 0;
/// Agent is traversing the short local segment from the building entry point to the network.
pub const TRANSIT_ACCESS_EGRESS: u8 = 1;
/// Agent is traversing the live lane/path network.
pub const TRANSIT_NETWORK: u8 = 2;
/// Agent is traversing the short local segment from the network into the destination building.
pub const TRANSIT_ACCESS_INGRESS: u8 = 3;
/// Border-spawn transport state used by household arrival carriers and exceptional/manual arrivals.
pub const TRANSIT_IMMIGRATING: u8 = 4;
/// Agent is traversing a bezier curve through a road intersection (lane-change phase).
pub const TRANSIT_INTERSECTION: u8 = 5;

/// Returns whether an agent in `transit` should be rendered in the live world.
pub(crate) fn transit_is_visible(transit: u8) -> bool {
    matches!(
        transit,
        TRANSIT_ACCESS_EGRESS
            | TRANSIT_NETWORK
            | TRANSIT_ACCESS_INGRESS
            | TRANSIT_IMMIGRATING
            | TRANSIT_INTERSECTION
    )
}

/// Trip-plan bit: the `planned_*` scalars contain a valid authoritative access/network plan.
pub const ACCESS_PLAN_VALID: u8 = 0x01;
/// Trip-plan bit: the node-path portion is zero-hop because attach and detach nodes match.
pub const ACCESS_ZERO_HOP_NODE_PATH: u8 = 0x02;
/// Trip-plan bit: the current path came from a validated flow-field fast path.
pub const ACCESS_PATH_FROM_FLOW_FIELD: u8 = 0x04;
/// Trip-plan bit: the trip originated from a border-node immigration spawn, not a building egress.
pub const ACCESS_IMMIGRATION_ORIGIN: u8 = 0x08;

// Transit Modes
/// Agent is walking on foot (sidewalks/crosswalks).
pub const MODE_WALK: u8 = 0;
/// Agent is driving a private car (road edges).
pub const MODE_CAR: u8 = 1;
/// Agent is cycling (sidewalks or road edges).
pub const MODE_BIKE: u8 = 2;
/// Agent is a passenger on a bus.
pub const MODE_BUS_PASSENGER: u8 = 3;
/// Agent is a passenger on a train/metro.
pub const MODE_TRAIN_PASSENGER: u8 = 4;
/// Agent is a passenger in a taxi.
pub const MODE_TAXI_PASSENGER: u8 = 5;
/// Agent is a passenger on a ship/ferry.
pub const MODE_SHIP_PASSENGER: u8 = 6;

// Vehicle Types (Civilians)
/// Default civilian sedan.
pub const VEHICLE_SEDAN: u8 = 0;
/// Faster/Sportier civilian sedan.
pub const VEHICLE_SPORTS: u8 = 1;
/// Basic civilian SUV.
pub const VEHICLE_SUV: u8 = 2;
/// Premium civilian SUV.
pub const VEHICLE_LUXURY: u8 = 3;

/// Returns whether an age group may hold a workplace.
pub(crate) fn age_group_can_work(age_group: u8) -> bool {
    age_group == AGE_ADULT
}

/// Returns whether an age group may carry household shopping trips.
pub(crate) fn age_group_can_shop(age_group: u8) -> bool {
    matches!(age_group, AGE_ADULT | AGE_ELDER)
}

/// Deterministically chooses one member's age group for a newly admitted household.
pub(crate) fn household_member_age_group(
    home_building: usize,
    household_id: usize,
    member_index: u16,
    household_size: u16,
) -> u8 {
    let composition = household_age_composition(home_building, household_id, household_size);
    if member_index < composition.adult_count {
        AGE_ADULT
    } else if member_index
        < composition
            .adult_count
            .saturating_add(composition.elder_count)
    {
        AGE_ELDER
    } else {
        AGE_CHILD
    }
}

pub(crate) fn household_age_composition(
    home_building: usize,
    household_id: usize,
    household_size: u16,
) -> HouseholdAgeComposition {
    let size = household_size.max(1);
    let seed = (home_building as u64)
        .wrapping_mul(0xA24B_AED4_963E_E407)
        .wrapping_add((household_id as u64).wrapping_mul(0x9FB2_1C65_1E98_DF25))
        .wrapping_add(u64::from(size).wrapping_mul(0x1656_67B1_9E37_79F9));

    if size == 1 {
        return if stable_index(seed, 5) == 0 {
            HouseholdAgeComposition {
                child_count: 0,
                adult_count: 0,
                elder_count: 1,
            }
        } else {
            HouseholdAgeComposition {
                child_count: 0,
                adult_count: 1,
                elder_count: 0,
            }
        };
    }

    let mut adult_count = 1u16;
    if size > 1 && stable_index(seed ^ 0xD1B5_4A32_D192_ED03, 100) < SECOND_ADULT_PERCENT {
        adult_count = adult_count.saturating_add(1);
    }
    adult_count = adult_count.min(MAX_ADULTS_PER_HOUSEHOLD).min(size);

    let remaining_after_adults = size.saturating_sub(adult_count);
    let max_elders = remaining_after_adults.min(MAX_ELDERS_PER_HOUSEHOLD);
    let elder_roll = stable_index(seed ^ 0x94D0_49BB_1331_11EB, 100);
    let elder_count = if max_elders >= 2
        && elder_roll < TWO_ELDER_PERCENT
        && size.saturating_sub(adult_count) >= 2
    {
        2
    } else if max_elders >= 1 && elder_roll < TWO_ELDER_PERCENT + ONE_ELDER_PERCENT {
        1
    } else {
        0
    };

    HouseholdAgeComposition {
        child_count: size.saturating_sub(adult_count).saturating_sub(elder_count),
        adult_count,
        elder_count,
    }
}

#[cfg(test)]
mod age_composition_tests {
    use super::*;

    #[test]
    fn household_age_composition_respects_adult_and_elder_caps() {
        for home_building in 0..64 {
            for household_id in 0..64 {
                for size in 1..16 {
                    let composition = household_age_composition(home_building, household_id, size);
                    assert!(composition.adult_count <= MAX_ADULTS_PER_HOUSEHOLD);
                    assert!(composition.elder_count <= MAX_ELDERS_PER_HOUSEHOLD);
                    assert_eq!(
                        composition
                            .child_count
                            .saturating_add(composition.adult_count)
                            .saturating_add(composition.elder_count),
                        size
                    );
                }
            }
        }
    }

    #[test]
    fn household_age_composition_keeps_children_with_adults() {
        for home_building in 0..64 {
            for household_id in 0..64 {
                for size in 1..16 {
                    let composition = household_age_composition(home_building, household_id, size);
                    if composition.child_count > 0 {
                        assert!(composition.adult_count > 0);
                    }
                    if size == 1 {
                        assert_eq!(composition.child_count, 0);
                    }
                }
            }
        }
    }

    #[test]
    fn household_member_age_group_matches_composition() {
        for size in 1..16 {
            let composition = household_age_composition(7, 11, size);
            let mut seen = HouseholdAgeComposition::default();
            for member_index in 0..size {
                match household_member_age_group(7, 11, member_index, size) {
                    AGE_CHILD => seen.child_count = seen.child_count.saturating_add(1),
                    AGE_ADULT => seen.adult_count = seen.adult_count.saturating_add(1),
                    AGE_ELDER => seen.elder_count = seen.elder_count.saturating_add(1),
                    _ => unreachable!(),
                }
            }
            assert_eq!(seen, composition);
        }
    }
}
