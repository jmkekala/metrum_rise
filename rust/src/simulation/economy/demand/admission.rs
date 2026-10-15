// SPDX-License-Identifier: GPL-2.0-only

//! Finite recurring-income claims for one demand-owned arrival batch.

use super::config::DemandConfig;
use super::snapshot::DailyDemandSnapshot;
use super::types::EPSILON;
use crate::simulation::economy::agents::HouseholdAgeComposition;
use crate::simulation::economy::fiscal::{CityFiscalPolicy, daily_property_tax};
use crate::simulation::economy::households::{applicant_offer, applicant_offer_limit};
use crate::simulation::zoning::ZoneType;

/// Scalar budget reused for every candidate in a bounded admission batch.
#[derive(Clone, Copy, Default)]
pub(super) struct HouseholdAdmissionBudget {
    unreserved_homes: u32,
    worker_homes: u32,
    remaining_job_equivalents: f32,
    net_wage_per_day: f32,
    essentials_per_member: f32,
    home_electricity_cost: f32,
    transfer_claim_per_day: f32,
    treasury: f32,
    transfer_coverage_days: f32,
}

impl HouseholdAdmissionBudget {
    /// Reserves existing residents and pending arrivals before considering this batch.
    pub(super) fn from_snapshot(snapshot: &DailyDemandSnapshot, config: &DemandConfig) -> Self {
        Self {
            unreserved_homes: snapshot.vacant_household_slots,
            worker_homes: 0,
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

    /// Keeps ready homes already claimed by workplace planning available to incoming workers.
    pub(super) fn reserve_worker_homes(&mut self, unreserved_ready_homes: u32) {
        self.worker_homes = self.unreserved_homes.saturating_sub(unreserved_ready_homes);
        self.unreserved_homes -= self.worker_homes;
    }

    /// Whether this composition preserves the planner's promised labour supply.
    pub(super) fn can_house(&self, composition: HouseholdAgeComposition) -> bool {
        self.unreserved_homes > 0 || (composition.adult_count > 0 && self.worker_homes > 0)
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
        if !self.can_house(composition) {
            return false;
        }
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
        if composition.adult_count > 0 && self.worker_homes > 0 {
            self.worker_homes -= 1;
        } else {
            self.unreserved_homes -= 1;
        }
        true
    }

    /// Arriving households needed to staff this budget's open jobs (`DEM-02-C`).
    ///
    /// Walks the applicant stream admission examines this hour: offer `k` goes to vacancy
    /// position `(seed + k) mod homes` in admission's cursor order with household seed
    /// `seed + k`. Offers the income check refuses are skipped; each accepted offer is one home.
    /// The walk stops once accepted adults cover the open jobs, or after admission's offer
    /// bound for that many arrivals. Homes are not a limit here: the result is a need, not a
    /// batch. `home(position)` yields `(home seed, flat size, level)`. O(offers walked), no
    /// allocation, and no routing: admission's border-route check is not repeated.
    pub(super) fn households_to_staff(
        &self,
        policy: &CityFiscalPolicy,
        seed: usize,
        homes: usize,
        home: impl Fn(usize) -> (usize, f32, u8),
    ) -> u32 {
        if homes == 0 || self.remaining_job_equivalents <= EPSILON {
            return 0;
        }
        let mut walk = Self {
            unreserved_homes: u32::MAX,
            worker_homes: 0,
            ..*self
        };
        let jobs = self.remaining_job_equivalents.ceil() as usize;
        let mut cursor = seed;
        let mut households = 0;
        for attempt in 0..applicant_offer_limit(jobs) {
            let position = cursor % homes;
            // An accepted offer claims its place, so the next offer moves on as well.
            cursor = position + 1;
            let (home_seed, flat_size_m2, level) = home(position);
            let Some((size, composition)) =
                applicant_offer(flat_size_m2, home_seed, seed.saturating_add(attempt))
            else {
                continue;
            };
            if walk.claim(composition, size, level, policy) {
                households += 1;
                if walk.remaining_job_equivalents <= EPSILON {
                    break;
                }
            }
        }
        households
    }
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
            unreserved_homes: 2,
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
            unreserved_homes: 3,
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
    fn worker_housing_reservations_survive_rejected_and_nonworker_applicants() {
        let mut policy = policy();
        policy.child_support_per_child_per_day = 0.0;
        policy.pension_per_elder_per_day = 100.0;
        let mut budget = HouseholdAdmissionBudget {
            unreserved_homes: 3,
            remaining_job_equivalents: 2.0,
            net_wage_per_day: 80.0,
            essentials_per_member: 30.0,
            treasury: 100_000.0,
            transfer_coverage_days: 30.0,
            ..Default::default()
        };
        budget.reserve_worker_homes(1);
        let worker = HouseholdAgeComposition {
            adult_count: 1,
            ..Default::default()
        };
        let elder = HouseholdAgeComposition {
            elder_count: 1,
            ..Default::default()
        };
        assert!(!budget.claim(
            HouseholdAgeComposition {
                child_count: 3,
                ..worker
            },
            4,
            1,
            &policy,
        ));
        assert_eq!(budget.worker_homes, 2);
        assert!(budget.claim(worker, 1, 1, &policy));
        assert_eq!(
            budget.unreserved_homes, 1,
            "workers use reserved homes first"
        );
        assert!(budget.claim(elder, 1, 1, &policy));
        assert!(!budget.claim(elder, 1, 1, &policy));
        assert!(budget.claim(worker, 1, 1, &policy));
        assert!(!budget.can_house(worker));
    }

    // Homes whose applicant at offer `k` (household seed `k`) has exactly `adults` adults.
    fn homes_offering(adults: u16, flat_size_m2: f32, offers: usize) -> Vec<usize> {
        let mut seed = 0;
        (0..offers)
            .map(|offer| {
                while applicant_offer(flat_size_m2, seed, offer)
                    .is_none_or(|(_, composition)| composition.adult_count != adults)
                {
                    seed += 1;
                }
                seed
            })
            .collect()
    }

    #[test]
    fn housing_need_counts_households_not_open_jobs() {
        // DEM-02-C: admission charges every adult against the jobs, so couples staffing 40
        // open jobs need 20 homes. One home per job left 79 homes vacant in a player save.
        const FLAT_M2: f32 = 200.0;
        let mut policy = policy();
        policy.child_support_per_child_per_day = 0.0;
        policy.pension_per_elder_per_day = 0.0;
        let budget = HouseholdAdmissionBudget {
            remaining_job_equivalents: 40.0,
            net_wage_per_day: 10_000.0,
            essentials_per_member: 30.0,
            treasury: 100_000.0,
            transfer_coverage_days: 30.0,
            ..Default::default()
        };
        let couples = homes_offering(2, FLAT_M2, 64);
        let need = |homes: &[usize]| {
            budget.households_to_staff(&policy, 0, homes.len(), |position| {
                (homes[position], FLAT_M2, 1)
            })
        };
        assert_eq!(need(&couples), 20);
        assert_eq!(need(&homes_offering(1, FLAT_M2, 64)), 40);
        // Applicants the income check refuses take no homes.
        let unpaid = HouseholdAdmissionBudget {
            net_wage_per_day: 0.0,
            ..budget
        };
        assert_eq!(
            unpaid.households_to_staff(&policy, 0, couples.len(), |position| {
                (couples[position], FLAT_M2, 1)
            }),
            0
        );
    }

    #[test]
    fn arrival_budget_charges_home_electricity_once_per_household() {
        let policy = policy();
        let property = daily_property_tax(ZoneType::Residential, 1, &policy);
        let mut budget = HouseholdAdmissionBudget {
            unreserved_homes: 1,
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
