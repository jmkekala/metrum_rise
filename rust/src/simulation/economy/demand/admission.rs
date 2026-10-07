// SPDX-License-Identifier: GPL-2.0-only

//! Finite recurring-income claims for one demand-owned arrival batch.

use super::config::DemandConfig;
use super::snapshot::DailyDemandSnapshot;
use super::types::EPSILON;
use crate::simulation::economy::agents::HouseholdAgeComposition;
use crate::simulation::economy::fiscal::{CityFiscalPolicy, daily_property_tax};
use crate::simulation::zoning::ZoneType;

/// Scalar budget reused for every candidate in a bounded admission batch.
#[derive(Default)]
pub(super) struct HouseholdAdmissionBudget {
    remaining_job_equivalents: f32,
    net_wage_per_day: f32,
    essentials_per_member: f32,
    home_electricity_cost: f32,
    transfer_claim_per_day: f32,
    treasury: f32,
    transfer_coverage_days: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> CityFiscalPolicy {
        CityFiscalPolicy::from_runtime_tuning(
            &crate::simulation::economy::definitions::load_runtime_economy_tuning().unwrap(),
        )
    }

    #[test]
    fn batch_cannot_spend_the_same_wage_twice_or_fund_a_different_family() {
        let mut policy = policy();
        policy.child_support_per_child_per_day = 0.0;
        let mut budget = HouseholdAdmissionBudget {
            remaining_job_equivalents: 1.0,
            net_wage_per_day: 80.0,
            essentials_per_member: 30.0,
            treasury: 100_000.0,
            transfer_coverage_days: 30.0,
            ..Default::default()
        };
        let family = HouseholdAgeComposition {
            adult_count: 1,
            child_count: 3,
            elder_count: 0,
        };
        assert!(!budget.claim(family, 4, 1, &policy));
        let worker = HouseholdAgeComposition {
            adult_count: 1,
            ..Default::default()
        };
        assert!(
            budget.claim(worker, 1, 1, &policy),
            "a rejected family must not reserve wages"
        );
        assert!(
            !budget.claim(worker, 1, 1, &policy),
            "temporary unemployment support is not recurring pay"
        );
    }

    #[test]
    fn each_pension_household_consumes_transfer_coverage() {
        let mut policy = policy();
        policy.pension_per_elder_per_day = 50.0;
        let mut budget = HouseholdAdmissionBudget {
            essentials_per_member: 40.0,
            treasury: 100.0,
            transfer_coverage_days: 1.0,
            ..Default::default()
        };
        let elder = HouseholdAgeComposition {
            elder_count: 1,
            ..Default::default()
        };
        assert!(budget.claim(elder, 1, 1, &policy));
        assert!(budget.claim(elder, 1, 1, &policy));
        assert!(!budget.claim(elder, 1, 1, &policy));
    }

    #[test]
    fn arrival_budget_charges_home_electricity_once_per_household() {
        let policy = policy();
        let property = daily_property_tax(ZoneType::Residential, 1, &policy);
        let mut budget = HouseholdAdmissionBudget {
            remaining_job_equivalents: 2.0,
            net_wage_per_day: (60.0 + 0.25 + property) / 2.0,
            essentials_per_member: 30.0,
            home_electricity_cost: 0.25,
            treasury: 100_000.0,
            transfer_coverage_days: 30.0,
            ..Default::default()
        };
        assert!(budget.claim(
            HouseholdAgeComposition {
                adult_count: 2,
                ..Default::default()
            },
            2,
            1,
            &policy,
        ));
    }
}

impl HouseholdAdmissionBudget {
    /// Reserves existing residents and pending arrivals before considering this batch.
    pub(super) fn from_snapshot(snapshot: &DailyDemandSnapshot, config: &DemandConfig) -> Self {
        Self {
            remaining_job_equivalents: snapshot.move_in_job_equivalent_slots,
            net_wage_per_day: snapshot.average_move_in_job_wage_per_day,
            essentials_per_member: snapshot.daily_essential_cost_per_resident,
            home_electricity_cost: snapshot.daily_home_electricity_cost,
            transfer_claim_per_day: snapshot.existing_unemployed_member_count as f32
                * snapshot.unemployment_daily_benefit_per_adult
                + snapshot.existing_child_count as f32
                    * snapshot.child_support_daily_benefit_per_child
                + snapshot.existing_elder_count as f32 * snapshot.pension_daily_benefit_per_elder,
            treasury: snapshot.city_treasury_balance.max(0.0),
            transfer_coverage_days: config
                .household_action
                .move_in_benefit_treasury_coverage_days
                .max(EPSILON),
        }
    }

    /// Claims wages and transfer headroom only for an affordable, physically launchable household.
    /// The arithmetic is O(1); no building or routing lookup occurs here.
    pub(super) fn claim(
        &mut self,
        composition: HouseholdAgeComposition,
        members: u16,
        home_level: u8,
        policy: &CityFiscalPolicy,
    ) -> bool {
        let adults = f32::from(composition.adult_count);
        let employed = adults.min(self.remaining_job_equivalents.max(0.0));
        let permanent_transfers = f32::from(composition.elder_count)
            * policy.pension_per_elder_per_day
            + f32::from(composition.child_count) * policy.child_support_per_child_per_day;
        let candidate_claim = permanent_transfers
            + (adults - employed) * policy.unemployment_benefit_per_adult_per_day;
        let total_claim = self.transfer_claim_per_day + candidate_claim;
        let reliability = if total_claim <= EPSILON {
            1.0
        } else {
            (self.treasury / (total_claim * self.transfer_coverage_days)).clamp(0.0, 1.0)
        };
        let recurring_income = employed * self.net_wage_per_day + permanent_transfers * reliability;
        let cost = f32::from(members) * self.essentials_per_member
            + self.home_electricity_cost
            + daily_property_tax(ZoneType::Residential, home_level.max(1), policy);
        if recurring_income + EPSILON < cost {
            return false;
        }
        // All incoming adults have labor-market priority over a later household, including
        // an adult initially supported by another member's wage or pension.
        self.remaining_job_equivalents = (self.remaining_job_equivalents - adults).max(0.0);
        self.transfer_claim_per_day = total_claim;
        true
    }
}
