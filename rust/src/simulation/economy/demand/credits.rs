// SPDX-License-Identifier: GPL-2.0-only

//! Explicit action pacing and bounded diagnostic ratios.

use super::types::EPSILON;

pub(super) fn clamp01(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

#[cfg(test)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn building_action_credit_debits_selected_count() {
        let mut credit = 3.75;

        let selected = advance_building_action_credit(&mut credit, 0.25, 1, 1.0);

        assert_eq!(selected, 1);
        assert!((credit - 3.0).abs() <= EPSILON);
    }
}
