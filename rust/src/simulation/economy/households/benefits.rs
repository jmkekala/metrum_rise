//! Unemployment benefit payment and treasury interaction.

use super::HouseholdSystem;
use crate::debug_log;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::definitions::load_runtime_economy_tuning;
use crate::simulation::zoning::ZoneType;

impl HouseholdSystem {
    pub(crate) fn pay_unemployment_benefits(
        &mut self,
        agents: &AgentSystem,
        allocator: &BuildingAllocator,
        treasury_balance: &mut f64,
    ) {
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let benefit_per_member = tuning.unemployment_daily_benefit_per_member;
        let max_days = tuning.unemployment_max_days;

        // Count unemployed members per household in one agent pass.
        let mut unemployed_per_household = vec![0u16; self.households.len()];
        for i in 0..agents.len() {
            let hid = agents.household_id[i];
            if hid == usize::MAX || hid >= self.households.len() {
                continue;
            }
            if agents.work_building[i] == usize::MAX {
                unemployed_per_household[hid] = unemployed_per_household[hid].saturating_add(1);
            }
        }

        let mut total_disbursed = 0.0f32;
        let mut households_paid = 0u32;
        let mut households_exhausted = 0u32;

        for (hid, household) in self.households.iter_mut().enumerate() {
            if household.member_count == 0
                || !valid_benefit_home(allocator, household.home_building_id)
            {
                continue;
            }
            let unemployed = unemployed_per_household[hid];
            if unemployed == 0 {
                household.unemployment_days_elapsed = 0;
                continue;
            }
            if household.unemployment_days_elapsed >= max_days {
                // Benefit exhausted; household is emigration-eligible via removal pressure.
                households_exhausted += 1;
                continue;
            }
            household.unemployment_days_elapsed =
                household.unemployment_days_elapsed.saturating_add(1);
            if *treasury_balance <= 0.0 {
                continue;
            }
            let benefit = unemployed as f32 * benefit_per_member;
            let paid = if *treasury_balance >= benefit as f64 {
                household.budget += benefit;
                *treasury_balance -= benefit as f64;
                benefit
            } else {
                let remainder = *treasury_balance as f32;
                household.budget += remainder;
                *treasury_balance = 0.0;
                remainder
            };
            total_disbursed += paid;
            households_paid += 1;
        }

        debug_log!(
            "economy",
            "unemployment_benefits: paid={:.1} households={} exhausted={} treasury={:.0}",
            total_disbursed,
            households_paid,
            households_exhausted,
            *treasury_balance,
        );
    }
}

fn valid_benefit_home(allocator: &BuildingAllocator, home_building_id: usize) -> bool {
    allocator
        .buildings
        .get(home_building_id)
        .is_some_and(|building| {
            building.zone_type == ZoneType::Residential
                && !building.broken
                && !building.economy_broken
                && !building.is_deserted
        })
}
