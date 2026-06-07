//! Credit accrual and pressure normalization helpers.

use super::types::EPSILON;

pub(super) fn clamp01(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

pub(super) fn advance_household_action_credit(
    credit: &mut f32,
    pressure: f32,
    threshold: f32,
    max_households_per_day: u32,
    max_actionable_households: u32,
    cadence_fraction: f32,
) -> u32 {
    let normalized_action_pressure = if threshold >= 1.0 {
        0.0
    } else {
        clamp01((pressure - threshold) / (1.0 - threshold))
    };
    if normalized_action_pressure <= EPSILON
        || max_households_per_day == 0
        || max_actionable_households == 0
    {
        *credit = 0.0;
        return 0;
    }
    *credit += normalized_action_pressure * max_households_per_day as f32 * cadence_fraction;
    let households_to_act = (*credit).floor().max(0.0) as u32;
    let households_to_act = households_to_act
        .min(max_households_per_day)
        .min(max_actionable_households);
    *credit -= households_to_act as f32;
    households_to_act
}

pub(super) fn advance_persistent_exit_credit(
    credit: &mut f32,
    eligible_households: u32,
    daily_fraction: f32,
    max_actionable_households: u32,
) -> u32 {
    if eligible_households == 0 {
        *credit = 0.0;
        return 0;
    }
    *credit += eligible_households as f32 * daily_fraction.max(0.0);
    let households_to_act = (*credit).floor().max(0.0) as u32;
    let households_to_act = households_to_act
        .min(eligible_households)
        .min(max_actionable_households);
    *credit -= households_to_act as f32;
    households_to_act
}

pub(super) fn advance_building_action_credit(
    credit: &mut f32,
    budget_units: f32,
    max_actionable_buildings: usize,
    cadence_fraction: f32,
) -> usize {
    if budget_units <= EPSILON || max_actionable_buildings == 0 {
        *credit = 0.0;
        return 0;
    }
    *credit += budget_units.max(0.0) * cadence_fraction;
    let buildings_to_act = (*credit).floor().max(0.0) as usize;
    let buildings_to_act = buildings_to_act.min(max_actionable_buildings);
    *credit -= buildings_to_act as f32;
    buildings_to_act
}

pub(super) fn advance_spawn_need_credit(
    credit: &mut f32,
    need_buildings: f32,
    max_actionable_buildings: usize,
) -> usize {
    if need_buildings <= EPSILON || max_actionable_buildings == 0 {
        *credit = 0.0;
        return 0;
    }
    *credit += need_buildings.max(0.0);
    let buildings_to_act = (*credit).floor().max(0.0) as usize;
    let selected = buildings_to_act.min(max_actionable_buildings);
    *credit -= selected as f32;
    selected
}

pub(super) fn normalized_positive_pressure(pressure: f32, threshold: f32) -> f32 {
    if threshold >= 1.0 {
        0.0
    } else {
        clamp01((pressure - threshold) / (1.0 - threshold))
    }
}

pub(super) fn normalized_negative_pressure(pressure: f32, threshold: f32) -> f32 {
    if threshold <= 0.0 {
        0.0
    } else {
        clamp01((threshold - pressure) / threshold.max(EPSILON))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn household_action_credit_debits_selected_count() {
        let mut credit = 2.75;

        let selected = advance_household_action_credit(&mut credit, 1.0, 0.0, 2, 1, 1.0);

        assert_eq!(selected, 1);
        assert!((credit - 3.75).abs() <= EPSILON);
    }

    #[test]
    fn building_action_credit_debits_selected_count() {
        let mut credit = 3.75;

        let selected = advance_building_action_credit(&mut credit, 0.25, 1, 1.0);

        assert_eq!(selected, 1);
        assert!((credit - 3.0).abs() <= EPSILON);
    }
}
