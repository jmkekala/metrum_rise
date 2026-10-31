// SPDX-License-Identifier: GPL-2.0-only

//! Reservation wage dynamics, labor supply elasticity, and sector shortage tracking.
//!
//! Models the feedback loop between the Citizen's Dividend (UBI), statutory transfers,
//! household savings reserves, and the citizen **Reservation Wage** (the lowest effective net
//! daily wage a worker is willing to accept).

use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, RuntimeEconomyCatalog, RuntimeEconomyTuning,
};
use crate::simulation::economy::fiscal::CityFiscalPolicy;
use crate::simulation::economy::households::{
    Household, consumer_basket_cost, household_daily_utility_allowance, household_reserve_days,
};
use crate::simulation::zoning::ZoneType;

/// Number of distinct wage sectors tracked by [`LaborMarket`].
pub const WAGE_SECTOR_COUNT: usize = 4;

/// Wage tier and sector classification for private and public workplaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum WageSector {
    /// Low-margin retail, service stores, basic agriculture, and entry-level commercial sites.
    LowWageCommercial = 0,
    /// Standard industrial manufacturing, extraction, and logistics workplaces.
    Industrial = 1,
    /// High-value commercial/industrial workplaces (`level >= 3` or high base wage).
    SkilledPrivate = 2,
    /// City-owned utilities and municipal services.
    PublicSector = 3,
}

impl WageSector {
    /// Returns the zero-based array index for this sector.
    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// Classifies a workplace building into its [`WageSector`]. O(1).
#[inline]
pub(crate) fn classify_workplace_sector(
    building: &Building,
    profile: &EconomyProfileRuntime,
    is_city_service: bool,
) -> WageSector {
    if is_city_service
        || matches!(
            profile.kind,
            EconomyProfileRuntimeKind::UtilityProducer
                | EconomyProfileRuntimeKind::UtilityProcessor
        )
    {
        return WageSector::PublicSector;
    }
    if building.level >= 3 || profile.average_daily_wage() >= 110.0 {
        return WageSector::SkilledPrivate;
    }
    if building.zone_type == ZoneType::Industrial
        || matches!(
            profile.kind,
            EconomyProfileRuntimeKind::Producer
                | EconomyProfileRuntimeKind::Processor
                | EconomyProfileRuntimeKind::Extractor
        )
    {
        return WageSector::Industrial;
    }
    WageSector::LowWageCommercial
}

/// Citywide labor market state, reservation wage parameters, and sector shortage diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LaborMarket {
    /// Minimum daily net subsistence cost per adult resident (food basket + basic utility allowance).
    pub base_subsistence_wage: f32,
    /// Current citywide baseline reservation wage (lowest daily net wage an average adult accepts).
    pub reservation_wage: f32,
    /// Elasticity coefficient governing how strongly `daily_ubi_per_adult` raises `reservation_wage`.
    pub ubi_reservation_elasticity: f32,
    /// Multiplier applied to public-sector wages (`0.50..=3.00`, synced from `CityFiscalPolicy`).
    pub public_sector_wage_multiplier: f32,
    /// Capital-for-labor automation adoption factor (`0.0..=0.50`), boosting per-worker throughput.
    pub automation_level: f32,
    /// Latest daily Citizen's Dividend per adult active in the labor market.
    pub active_daily_ubi_per_adult: f32,
    /// Latest income tax rate active in the labor market.
    pub active_income_tax_rate: f32,
    /// Latest unemployment benefit per adult per day active in the labor market.
    pub active_unemployment_benefit: f32,
    /// Latest unemployment benefit max days active in the labor market.
    pub active_unemployment_max_days: u32,
    /// Open funded job offers rejected by workers because offered net wage fell below their
    /// reservation wage, indexed by [`WageSector`].
    pub reservation_wage_refusals_by_sector: [u32; WAGE_SECTOR_COUNT],
    /// Total active funded job capacities per sector.
    pub funded_jobs_by_sector: [u32; WAGE_SECTOR_COUNT],
    /// Total filled jobs per sector.
    pub filled_jobs_by_sector: [u32; WAGE_SECTOR_COUNT],
    /// Unfilled funded job vacancies per sector.
    pub open_vacancies_by_sector: [u32; WAGE_SECTOR_COUNT],
    /// Computed labor shortage ratio (`0.0..=1.0`) per sector (`open_vacancies / funded_jobs`).
    pub sector_shortage_ratio: [f32; WAGE_SECTOR_COUNT],
}

impl Default for LaborMarket {
    fn default() -> Self {
        Self {
            base_subsistence_wage: 25.0,
            reservation_wage: 25.0,
            ubi_reservation_elasticity: 0.90,
            public_sector_wage_multiplier: 1.0,
            automation_level: 0.0,
            active_daily_ubi_per_adult: 0.0,
            active_income_tax_rate: 0.10,
            active_unemployment_benefit: 12.0,
            active_unemployment_max_days: 14,
            reservation_wage_refusals_by_sector: [0; WAGE_SECTOR_COUNT],
            funded_jobs_by_sector: [0; WAGE_SECTOR_COUNT],
            filled_jobs_by_sector: [0; WAGE_SECTOR_COUNT],
            open_vacancies_by_sector: [0; WAGE_SECTOR_COUNT],
            sector_shortage_ratio: [0.0; WAGE_SECTOR_COUNT],
        }
    }
}


impl LaborMarket {
    /// Synchronizes labor market parameters with the current fiscal policy, daily UBI per adult,
    /// and runtime economy catalog. O(profiles).
    pub(crate) fn sync_policy_and_ubi(
        &mut self,
        fiscal: &CityFiscalPolicy,
        daily_ubi_per_adult: f32,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
    ) {
        let basket = consumer_basket_cost(catalog, fiscal.household_vat_rate) as f32;
        let utility = household_daily_utility_allowance(catalog, tuning, 1.0);
        self.base_subsistence_wage = (basket + utility).max(1.0);
        self.public_sector_wage_multiplier = fiscal.public_sector_wage_multiplier.clamp(0.50, 3.00);
        self.automation_level = fiscal.automation_level.clamp(0.0, 0.50);
        self.active_daily_ubi_per_adult = daily_ubi_per_adult.max(0.0);
        self.active_income_tax_rate = fiscal.income_tax_rate.clamp(0.0, 0.75);
        self.active_unemployment_benefit = fiscal.unemployment_benefit_per_adult_per_day.max(0.0);
        self.active_unemployment_max_days = fiscal.unemployment_max_days;

        let ubi_term = self.ubi_reservation_elasticity * self.active_daily_ubi_per_adult;
        let unemp_term = if self.active_unemployment_max_days > 0 {
            0.35 * self.active_unemployment_benefit
        } else {
            0.0
        };
        self.reservation_wage = self.base_subsistence_wage + ubi_term + unemp_term;
    }

    /// Computes the binding reservation wage for a specific household during job search. O(1).
    ///
    /// When `active_daily_ubi_per_adult <= 0.0` (either default non-UBI policy or during a
    /// fiscal deficit crisis that triggered the `CityBudget` UBI circuit-breaker), returns `0.0`
    /// so unemployed citizens immediately accept any positive funded wage offer.
    #[inline]
    pub(crate) fn effective_household_reservation_wage(
        &self,
        household: &Household,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
    ) -> f32 {
        if self.active_daily_ubi_per_adult <= 0.0 {
            return 0.0;
        }
        let unemp_fallback =
            if household.unemployment_days_elapsed < self.active_unemployment_max_days {
                0.35 * self.active_unemployment_benefit
            } else {
                0.0
            };
        let reserve_days = household_reserve_days(catalog, tuning, household);
        let savings_cushion = (reserve_days - 3.0).clamp(0.0, 20.0) * 1.25;
        self.base_subsistence_wage
            + self.ubi_reservation_elasticity * self.active_daily_ubi_per_adult
            + unemp_fallback
            + savings_cushion
    }

    /// Returns the gross daily wage offered by a workplace, applying `public_sector_wage_multiplier`
    /// to city-owned service/utility workplaces. O(1).
    #[inline]
    pub(crate) fn workplace_gross_daily_wage(
        &self,
        profile: &EconomyProfileRuntime,
        is_city_service: bool,
    ) -> f32 {
        let base = profile.average_daily_wage();
        if is_city_service {
            base * self.public_sector_wage_multiplier.clamp(0.50, 3.00)
        } else {
            base
        }
    }

    /// Returns the effective net daily wage of a job offer after income tax and commute disutility.
    /// O(1).
    #[inline]
    pub(crate) fn workplace_effective_net_wage(
        &self,
        gross_daily_wage: f32,
        commute_penalty: f32,
    ) -> f32 {
        let net_after_tax =
            gross_daily_wage.max(0.0) * (1.0 - self.active_income_tax_rate.clamp(0.0, 0.75));
        net_after_tax * (1.0 - 0.15 * commute_penalty.clamp(0.0, 1.0))
    }

    /// Recomputes sector-level job capacities, filled jobs, open vacancies, and shortage ratios.
    /// O(buildings).
    pub(crate) fn refresh_sector_diagnostics(
        &mut self,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
    ) {
        let mut funded = [0_u32; WAGE_SECTOR_COUNT];
        let mut filled = [0_u32; WAGE_SECTOR_COUNT];
        for building in &allocator.buildings {
            if building.broken
                || building.economy_broken
                || building.is_deserted
                || building.is_under_construction()
            {
                continue;
            }
            let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            else {
                continue;
            };
            if profile.worker_capacity == 0 {
                continue;
            }
            let is_service = allocator.is_city_service_building(building);
            let sector = classify_workplace_sector(building, profile, is_service).index();
            let cap = building.funded_job_capacity;
            let workers = building.worker_count.min(cap);
            funded[sector] = funded[sector].saturating_add(cap);
            filled[sector] = filled[sector].saturating_add(workers);
        }
        self.funded_jobs_by_sector = funded;
        self.filled_jobs_by_sector = filled;
        for i in 0..WAGE_SECTOR_COUNT {
            let open = funded[i].saturating_sub(filled[i]);
            self.open_vacancies_by_sector[i] = open;
            self.sector_shortage_ratio[i] = if funded[i] > 0 {
                (open as f32 / funded[i] as f32).clamp(0.0, 1.0)
            } else {
                0.0
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::buildings::allocator::indexed_test_building;
    use crate::simulation::economy::definitions::{
        load_runtime_economy_catalog, load_runtime_economy_tuning,
    };
    use crate::simulation::economy::households::building_operation_factors_with_automation;

    #[test]
    fn high_ubi_raises_reservation_wage_and_deficit_resets_binding_floor() {
        let catalog = load_runtime_economy_catalog().unwrap();
        let tuning = load_runtime_economy_tuning().unwrap();
        let mut policy = CityFiscalPolicy::from_runtime_tuning(&tuning);
        policy.ubi_surplus_share = 0.80;

        let mut lm = LaborMarket::default();
        lm.sync_policy_and_ubi(&policy, 0.0, &catalog, &tuning);
        let base_res_wage = lm.reservation_wage;

        let household = Household {
            home_building_id: 0,
            power_arrears: Default::default(),
            budget: 500.0,
            stock: 10.0,
            member_count: 2,
            child_count: 0,
            adult_count: 2,
            employed_adults: 0,
            elder_count: 0,
            gross_daily_wages: 0.0,
            consumption_rate: 1.0,
            stock_days: 5.0,
            replenishment_state: 0,
            reserved_store_building_id: usize::MAX,
            reserved_amount: 0.0,
            reserved_total_cost: 0.0,
            shopping_agent: Default::default(),
            shopping_has_car: true,
            shopping_timeout_hours_remaining: 0,
            cooldown_hours: 0,
            replenishment_search_cursor: 0,
            replenishment_failure_count: 0,
            stay_failure_days: 0,
            unhoused_days_elapsed: 0,
            replenishment_offset_hours: 0,
            unemployment_days_elapsed: 0,
        };

        // When UBI is 0 (default or fiscal deficit crisis), binding floor is 0.0.
        assert_eq!(
            lm.effective_household_reservation_wage(&household, &catalog, &tuning),
            0.0
        );

        // When UBI is 80/day per adult, reservation wage rises sharply.
        lm.sync_policy_and_ubi(&policy, 80.0, &catalog, &tuning);
        assert!(lm.reservation_wage > base_res_wage + 60.0);
        let active_floor = lm.effective_household_reservation_wage(&household, &catalog, &tuning);
        assert!(active_floor > 90.0);

        // Crisis circuit-breaker drops UBI back to 0.0 -> floor immediately resets.
        lm.sync_policy_and_ubi(&policy, 0.0, &catalog, &tuning);
        assert_eq!(
            lm.effective_household_reservation_wage(&household, &catalog, &tuning),
            0.0
        );
    }

    #[test]
    fn public_wage_multiplier_and_automation_level_scale_wages_and_throughput() {
        let catalog = load_runtime_economy_catalog().unwrap();
        let tuning = load_runtime_economy_tuning().unwrap();
        let mut policy = CityFiscalPolicy::from_runtime_tuning(&tuning);
        policy.public_sector_wage_multiplier = 2.0;
        policy.automation_level = 0.50;

        let mut lm = LaborMarket::default();
        lm.sync_policy_and_ubi(&policy, 50.0, &catalog, &tuning);

        let power_profile = catalog.profile_for_id("power_plant_basic").unwrap();
        let base_wage = power_profile.average_daily_wage();
        let boosted_wage = lm.workplace_gross_daily_wage(power_profile, true);
        assert!((boosted_wage - base_wage * 2.0).abs() < 1e-4);

        // Verify automation_level = 0.50 allows half the physical workforce to achieve full
        // staffing factor (1.0).
        let ind_profile = catalog
            .all_profiles()
            .iter()
            .find(|p| p.kind == EconomyProfileRuntimeKind::Processor && p.worker_capacity >= 2)
            .expect("processor profile");
        let mut building = indexed_test_building(String::new(), ZoneType::Industrial, 0);
        building.economy_profile_runtime_id = ind_profile.runtime_id;
        building.worker_count = (ind_profile.worker_capacity / 2).max(1);
        for input in &ind_profile.inputs {
            building.set_inventory_units(input.resource_runtime_id, 1_000.0);
        }

        let unautomated =
            building_operation_factors_with_automation(&catalog, &building, ind_profile, 0.0);
        let automated =
            building_operation_factors_with_automation(&catalog, &building, ind_profile, 0.50);
        assert!(
            automated.throughput_factor > unautomated.throughput_factor,
            "automation should increase throughput with partial staffing ({} vs {})",
            automated.throughput_factor,
            unautomated.throughput_factor
        );
    }
}
