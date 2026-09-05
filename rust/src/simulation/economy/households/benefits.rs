// SPDX-License-Identifier: GPL-2.0-only

//! Household transfer payment and treasury interaction.

use std::sync::atomic::Ordering;

use super::HouseholdSystem;
use crate::debug_log;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::{AgentSystem, age_group_can_work};
use crate::simulation::economy::fiscal::CityFiscalPolicy;
use crate::simulation::zoning::ZoneType;
use rayon::prelude::*;

impl HouseholdSystem {
    #[cfg(test)]
    pub(crate) fn pay_unemployment_benefits(
        &mut self,
        agents: &AgentSystem,
        allocator: &BuildingAllocator,
        treasury_balance: &mut f64,
    ) {
        let mut policy = CityFiscalPolicy::default();
        policy.pension_per_elder_per_day = 0.0;
        policy.child_support_per_child_per_day = 0.0;
        self.pay_household_transfers(agents, allocator, treasury_balance, &policy);
    }

    pub(crate) fn pay_household_transfers(
        &mut self,
        agents: &AgentSystem,
        allocator: &BuildingAllocator,
        treasury_balance: &mut f64,
        policy: &CityFiscalPolicy,
    ) {
        let household_count = self.households.len();
        self.reset_member_count_scratch();
        {
            let unemployed_per_household = &self.member_count_scratch;
            agents
                .household_id
                .par_iter()
                .zip(agents.work_building.par_iter())
                .zip(agents.age_group.par_iter())
                .for_each(|((&hid, &work), &age_group)| {
                    if hid < household_count && age_group_can_work(age_group) && work == usize::MAX
                    {
                        unemployed_per_household[hid].fetch_add(1, Ordering::Relaxed);
                    }
                });
        }

        let mut total_disbursed = 0.0f32;
        let mut total_unemployment_disbursed = 0.0f32;
        let mut total_pension_disbursed = 0.0f32;
        let mut total_child_support_disbursed = 0.0f32;
        let mut households_paid = 0u32;
        let mut households_exhausted = 0u32;
        self.ensure_daily_ledger_len();

        let benefit_per_adult = policy.unemployment_benefit_per_adult_per_day.max(0.0);
        let max_days = policy.unemployment_max_days;
        let pension_per_elder = policy.pension_per_elder_per_day.max(0.0);
        let child_support_per_child = policy.child_support_per_child_per_day.max(0.0);
        let households = &mut self.households;
        let daily_ledgers = &mut self.daily_ledgers;
        for hid in 0..households.len() {
            let household = &mut households[hid];
            if household.member_count == 0 {
                continue;
            }
            let unemployed = self.member_count_scratch[hid]
                .load(Ordering::Relaxed)
                .min(u32::from(u16::MAX)) as u16;
            daily_ledgers[hid].unemployed_adults = unemployed;
            let unemployment_claim =
                if !valid_unemployment_benefit_home(allocator, household.home_building_id) {
                    0.0
                } else if unemployed == 0 {
                    household.unemployment_days_elapsed = 0;
                    0.0
                } else if household.unemployment_days_elapsed >= max_days {
                    // Benefit exhausted; household is emigration-eligible via removal pressure.
                    households_exhausted += 1;
                    0.0
                } else {
                    household.unemployment_days_elapsed =
                        household.unemployment_days_elapsed.saturating_add(1);
                    unemployed as f32 * benefit_per_adult
                };

            let pension_claim = household.elder_count as f32 * pension_per_elder;
            let child_support_claim = household.child_count as f32 * child_support_per_child;
            let unemployment_paid = pay_transfer_claim(treasury_balance, unemployment_claim);
            let pension_paid = pay_transfer_claim(treasury_balance, pension_claim);
            let child_support_paid = pay_transfer_claim(treasury_balance, child_support_claim);
            let paid = unemployment_paid + pension_paid + child_support_paid;
            if paid <= 0.0 {
                continue;
            }
            household.budget += paid;
            daily_ledgers[hid].unemployment_benefit_income += unemployment_paid;
            daily_ledgers[hid].pension_income += pension_paid;
            daily_ledgers[hid].child_support_income += child_support_paid;
            total_disbursed += paid;
            total_unemployment_disbursed += unemployment_paid;
            total_pension_disbursed += pension_paid;
            total_child_support_disbursed += child_support_paid;
            households_paid += 1;
            debug_log!(
                "economy",
                "household_transfer_recipient: household_id={} unemployed_adults={} elders={} children={} paid={:.1} unemployment={:.1} pension={:.1} child_support={:.1} unemployment_days_elapsed={}",
                hid,
                unemployed,
                household.elder_count,
                household.child_count,
                paid,
                unemployment_paid,
                pension_paid,
                child_support_paid,
                household.unemployment_days_elapsed
            );
        }

        debug_log!(
            "economy",
            "household_transfers: paid={:.1} unemployment={:.1} pension={:.1} child_support={:.1} households={} unemployment_exhausted={} treasury={:.0}",
            total_disbursed,
            total_unemployment_disbursed,
            total_pension_disbursed,
            total_child_support_disbursed,
            households_paid,
            households_exhausted,
            *treasury_balance,
        );
    }
}

fn pay_transfer_claim(treasury_balance: &mut f64, claim: f32) -> f32 {
    if claim <= 0.0 || *treasury_balance <= 0.0 {
        return 0.0;
    }
    if *treasury_balance >= claim as f64 {
        *treasury_balance -= claim as f64;
        claim
    } else {
        let paid = (*treasury_balance).max(0.0) as f32;
        *treasury_balance = 0.0;
        paid
    }
}

fn valid_unemployment_benefit_home(allocator: &BuildingAllocator, home_building_id: usize) -> bool {
    allocator
        .buildings
        .get(home_building_id)
        .is_some_and(|building| {
            building.zone_type == ZoneType::Residential
                && !building.broken
                && !building.economy_broken
                && !building.is_deserted
                && !building.is_under_construction()
        })
}
