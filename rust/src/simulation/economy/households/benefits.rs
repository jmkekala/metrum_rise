// SPDX-License-Identifier: GPL-2.0-only

//! Household transfer payment and treasury interaction.

use super::HouseholdSystem;
use crate::debug_log;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::fiscal::CityFiscalPolicy;

impl HouseholdSystem {
    #[cfg(test)]
    pub(crate) fn pay_unemployment_benefits(
        &mut self,
        allocator: &BuildingAllocator,
        treasury_balance: &mut f64,
    ) {
        let policy = CityFiscalPolicy {
            pension_per_elder_per_day: 0.0,
            child_support_per_child_per_day: 0.0,
            ..Default::default()
        };
        self.pay_household_transfers(allocator, treasury_balance, &policy);
    }

    /// Every household's transfers at once, as their daily phases would pay them.
    #[cfg(test)]
    pub(crate) fn pay_household_transfers(
        &mut self,
        allocator: &BuildingAllocator,
        treasury_balance: &mut f64,
        policy: &CityFiscalPolicy,
    ) {
        self.ensure_daily_ledger_len();
        for household_id in 0..self.households.len() {
            self.pay_household_transfer(household_id, allocator, treasury_balance, policy);
        }
    }

    /// Pays household `household_id`'s daily transfers at its daily phase (`SIM-02-G`):
    /// unemployment benefit for its adults not paid a wage in its window, pensions and child
    /// support, while the treasury holds cash. When the treasury runs out, slot order
    /// decides who is paid. O(1).
    pub(super) fn pay_household_transfer(
        &mut self,
        household_id: usize,
        allocator: &BuildingAllocator,
        treasury_balance: &mut f64,
        policy: &CityFiscalPolicy,
    ) {
        let household = &mut self.households[household_id];
        if household.member_count == 0 {
            return;
        }
        let ledger = &mut self.daily_ledgers[household_id];
        // Each member's wage falls once in any 24-hour window, so the adults not paid in this
        // one are the household's unemployed, including workers an insolvent employer could
        // not pay before it closed (`SIM-02-G`).
        let unemployed = household.adult_count.saturating_sub(ledger.paid_workers);
        ledger.unemployed_adults = unemployed;
        let unemployment_claim = if allocator.household_capacity(household.home_building_id) == 0 {
            0.0
        } else if unemployed == 0 {
            household.unemployment_days_elapsed = 0;
            0.0
        } else if household.unemployment_days_elapsed >= policy.unemployment_max_days {
            // Benefit exhausted; household is emigration-eligible via removal pressure.
            0.0
        } else {
            household.unemployment_days_elapsed =
                household.unemployment_days_elapsed.saturating_add(1);
            unemployed as f32 * policy.unemployment_benefit_per_adult_per_day.max(0.0)
        };
        let pension_claim =
            household.elder_count as f32 * policy.pension_per_elder_per_day.max(0.0);
        let child_support_claim =
            household.child_count as f32 * policy.child_support_per_child_per_day.max(0.0);
        ledger.transfer_claim += unemployment_claim + pension_claim + child_support_claim;
        let unemployment_paid = pay_transfer_claim(treasury_balance, unemployment_claim);
        let pension_paid = pay_transfer_claim(treasury_balance, pension_claim);
        let child_support_paid = pay_transfer_claim(treasury_balance, child_support_claim);
        let paid = unemployment_paid + pension_paid + child_support_paid;
        if paid <= 0.0 {
            return;
        }
        household.budget += paid;
        ledger.unemployment_benefit_income += unemployment_paid;
        ledger.pension_income += pension_paid;
        ledger.child_support_income += child_support_paid;
        let totals = &mut self.day_totals;
        totals.unemployment_benefits += f64::from(unemployment_paid);
        totals.pensions += f64::from(pension_paid);
        totals.child_support += f64::from(child_support_paid);
        debug_log!(
            "economy",
            "household_transfer_recipient: household_id={} unemployed_adults={} elders={} children={} paid={:.1} unemployment={:.1} pension={:.1} child_support={:.1} unemployment_days_elapsed={}",
            household_id,
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
}

pub(super) fn pay_transfer_claim(treasury_balance: &mut f64, claim: f32) -> f32 {
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
