// SPDX-License-Identifier: GPL-2.0-only

//! Household record storage and building-reference maintenance.

use rayon::prelude::*;
use std::sync::atomic::{AtomicU32, AtomicUsize};

use super::metrics::household_supply_resource_runtime_id;
use super::replenishment::{
    REPLENISHMENT_SHOPPING_RETURNING, REPLENISHMENT_SHOPPING_TO_STORE, clear_replenishment_request,
    register_replenishment_failure,
};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::handles::{Handle, HandleTable};
use crate::simulation::economy::agents::AgentHandle;
use crate::simulation::economy::definitions::{
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};

/// Daily household accounting, including collected utility payments awaiting settlement.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DailyHouseholdLedger {
    /// Household budget at the start of the current debug ledger window.
    pub(crate) budget_before: f32,
    /// Household budget after daily settlement.
    pub(crate) budget_after: f32,
    /// Wage income received during daily settlement.
    pub(crate) wage_income: f32,
    /// Unemployment benefit received during daily settlement.
    pub(crate) unemployment_benefit_income: f32,
    /// Pension transfer received during daily settlement.
    pub(crate) pension_income: f32,
    /// Child-support transfer received during daily settlement.
    pub(crate) child_support_income: f32,
    /// Eligible transfers before treasury cash limits, across all benefit categories.
    pub(crate) transfer_claim: f32,
    /// Residential property tax paid during daily settlement.
    pub(crate) property_tax_paid: f32,
    /// Net household budget reserved or spent on shopping during the ledger window.
    pub(crate) shopping_spend: f32,
    /// Value of household stock consumed from the home buffer.
    pub(crate) household_supply_consumption_cost: f32,
    /// Residential electricity charge paid from the household budget.
    pub(crate) power_consumption_cost: f32,
    /// Delivered electricity independent of cash; billed at daily settlement.
    pub(crate) electricity: crate::simulation::economy::electricity::ElectricityUsage,
    /// Unpaid electricity after settlement, included in household distress reporting.
    pub(crate) power_unpaid_cost: f32,
    /// Collected electricity payments attributed to local generation, excluding unpaid bills.
    pub(crate) power_local_payment: f32,
    /// Residential water-service charge paid from the household budget.
    pub(crate) water_consumption_cost: f32,
    /// Residential sewage-service charge paid from the household budget.
    pub(crate) sewage_consumption_cost: f32,
    /// Unemployed adult count observed by the daily benefit pass.
    pub(crate) unemployed_adults: u16,
    /// Members paid a wage in this window; the household's other adults draw unemployment
    /// benefit at its daily phase (`SIM-02-G`).
    pub(crate) paid_workers: u16,
    /// Shopper tasks completed during the ledger window.
    pub(crate) shopper_trips_completed: u16,
    /// Shopper tasks failed during the ledger window.
    pub(crate) shopper_trips_failed: u16,
}

impl DailyHouseholdLedger {
    /// Total city transfer income received during the current ledger window.
    pub(crate) fn transfer_income(self) -> f32 {
        self.unemployment_benefit_income + self.pension_income + self.child_support_income
    }
}

/// Completed daily electricity settlement summary kept for city budget reporting.
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct DailyPowerSettlementSummary {
    /// Total electricity demand considered by the daily utility settlement.
    pub(crate) demand_units: f32,
    /// Total local electricity units available from active providers.
    pub(crate) supply_units: f32,
    /// Electricity demand served from local providers.
    pub(crate) served_units: f32,
    /// Local electricity coverage ratio in `0.0..=1.0`.
    pub(crate) coverage: f32,
    /// Household payments routed to local city providers.
    pub(crate) household_local_revenue: f32,
    /// Private business payments routed to local city providers.
    pub(crate) private_local_revenue: f32,
    /// City-service payments routed to local city providers.
    pub(crate) city_service_local_cost: f32,
    /// Local utility revenue across all settled services.
    pub(crate) utility_local_revenue: f32,
    /// City-service payments routed to local providers across all settled services.
    pub(crate) city_service_utility_local_cost: f32,
    /// City-service payments routed to OWA fallback across all settled services.
    pub(crate) city_service_utility_owa_cost: f32,
    /// Plant auxiliary electricity fees that are internal transfers, not external sales.
    pub(crate) power_auxiliary_local_cost: f32,
    /// Imported auxiliary electricity and plant water/sewage upkeep paid outside the power service.
    pub(crate) power_upkeep_utility_cost: f32,
}

/// Actual city-funded payroll settled during the latest fiscal day.
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct CityServiceWages {
    /// Wages paid by the treasury across all city services.
    pub(crate) total: f32,
    /// Portion paid to power-plant employees, used by the electricity ledger.
    pub(crate) power: f32,
}

/// City-wide flows of one calendar day, posted as each entity settles at its own daily phase
/// (`SIM-02-G`, `docs/simulation.md#phases`). The calendar close keeps the finished day for the
/// budget report and the next day's pooled reads, then starts an empty one. O(1) per posting.
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CityDayTotals {
    /// Utility and electricity bills: local receipts, city-service costs and delivered units.
    pub(crate) power: DailyPowerSettlementSummary,
    /// Wages the treasury paid to city-service workers.
    pub(crate) city_service_wages: CityServiceWages,
    /// Gross wages paid by private employers.
    pub(crate) private_payroll: f64,
    /// Unemployment benefits paid from the treasury.
    pub(crate) unemployment_benefits: f64,
    /// Pensions paid from the treasury.
    pub(crate) pensions: f64,
    /// Child support paid from the treasury.
    pub(crate) child_support: f64,
    /// Coal burned by power plants in the day's hourly dispatches.
    pub(crate) coal_consumed_kg: f64,
    /// Operational providers that settled, by utility service; the next day's water and sewage
    /// coverage reads it.
    pub(crate) utility_providers: [u32; 3],
    /// Local water and sewage revenue by service, shared among the next day's providers.
    pub(crate) utility_local_revenue: [f32; 3],
}

impl CityDayTotals {
    /// Treasury transfers paid to households during the day.
    pub(crate) fn transfers(&self) -> f64 {
        self.unemployment_benefits + self.pensions + self.child_support
    }
}

/// Explicit household runtime record anchored to a residential building.
#[derive(Clone, Debug)]
pub struct Household {
    /// Residential building currently anchoring the household.
    pub home_building_id: usize,
    /// Unpaid local/OWA electricity bills carried into the next settlement.
    pub(crate) power_arrears: crate::simulation::economy::electricity::ElectricityBill,
    /// Shared household budget used for essentials in the first-pass loop.
    pub budget: f32,
    /// Current household stock buffer in `household_supplies`.
    pub stock: f32,
    /// Cached linked population count. Rebuilt from resident agents every economy pass.
    pub member_count: u16,
    /// Cached number of child residents. Rebuilt from resident agents every economy pass.
    pub child_count: u16,
    /// Cached number of adult residents. Rebuilt from resident agents every economy pass.
    pub adult_count: u16,
    /// Assigned adults at the latest membership pass; distinct from funded vacancies.
    pub(crate) employed_adults: u16,
    /// Expected gross daily wages at that pass; unpaid or inactive jobs contribute nothing.
    pub(crate) gross_daily_wages: f32,
    /// Whether an adult/elder member can shop by car; rebuilt from saved resident agents.
    pub(crate) shopping_has_car: bool,
    /// Cached number of elder residents. Rebuilt from resident agents every economy pass.
    pub elder_count: u16,
    /// Baseline daily consumption in `household_supplies / day / resident`.
    pub consumption_rate: f32,
    /// Cached derived stock horizon in days at the current consumption rate.
    pub stock_days: f32,
    /// Current replenishment state for diagnostics and cooldown handling.
    pub replenishment_state: u8,
    /// Remaining operational-hour cooldown steps before another replenishment retry.
    pub cooldown_hours: u16,
    /// Consecutive failed replenishment attempts for the current shortage.
    pub replenishment_failure_count: u16,
    /// Reserved source building for the current replenishment request, if any.
    pub reserved_store_building_id: usize,
    /// Reserved amount carried by the active household shopper.
    pub reserved_amount: f32,
    /// Reserved budget waiting to be transferred to the supplying store.
    pub reserved_total_cost: f32,
    /// Selected household member carrying the active shopping request; stale once it leaves.
    pub(crate) shopping_agent: AgentHandle,
    /// Remaining operational-hour budget for the current shopping leg.
    pub shopping_timeout_hours_remaining: u16,
    /// Deterministic store-search continuation cursor used after failed replenishment attempts.
    pub replenishment_search_cursor: u32,
    /// Consecutive daily stay-rule failures for the current home.
    pub stay_failure_days: u32,
    /// Consecutive settled days with no valid home after the daily rehousing attempt.
    pub unhoused_days_elapsed: u32,
    /// Stable authored cadence offset used for periodic replenishment checks.
    pub replenishment_offset_hours: u16,
    /// Days elapsed with at least one unemployed adult. Resets to 0 when all adult members are
    /// employed. Incremented each daily tick while the household is benefit-eligible. Once
    /// this reaches `unemployment_max_days`, the household becomes emigration-eligible and
    /// benefit payments stop.
    pub unemployment_days_elapsed: u32,
}

/// Incoming households counted during the existing arrival-carrier pass.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct PendingHouseholdArrivals {
    /// Homes already claimed by carriers which have not arrived.
    pub(crate) households: u32,
    /// Incoming adults which share the labor pool with existing unemployed residents.
    pub(crate) adults: u32,
    /// Incoming pension claimants.
    pub(crate) elders: u32,
    /// Incoming child-support claimants.
    pub(crate) children: u32,
}

/// Collection of explicit household records for the live simulation.
pub struct HouseholdSystem {
    /// All known households. Agents reference these by index.
    pub households: Vec<Household>,
    /// Stable handles of the dense households; saved with the store.
    pub(crate) handles: HandleTable<Household>,
    /// Derived before each hourly demand pass; rebuilt from persisted arrival carriers.
    pub(crate) pending_arrivals: PendingHouseholdArrivals,
    /// Households whose shopper was reserved and held since SimCore last paced departures.
    pub(crate) held_shoppers: Vec<Handle<Household>>,
    /// Service visitors held since SimCore last paced departures.
    pub(crate) held_visitors: Vec<AgentHandle>,
    pub(crate) job_funding: crate::simulation::economy::hiring::JobFunding,
    pub(super) member_count_scratch: Vec<AtomicU32>,
    pub(super) child_count_scratch: Vec<AtomicU32>,
    pub(super) adult_count_scratch: Vec<AtomicU32>,
    pub(super) elder_count_scratch: Vec<AtomicU32>,
    pub(super) shopping_car_scratch: Vec<AtomicU32>,
    pub(super) worker_count_scratch: Vec<AtomicU32>,
    pub(super) household_member_heads_scratch: Vec<usize>,
    pub(super) household_member_next_scratch: Vec<usize>,
    pub(super) has_unhoused_households: bool,
    pub(super) removal_candidates_scratch: Vec<(usize, f32, f32)>,
    pub(super) removal_selected_flags_scratch: Vec<bool>,
    pub(super) removal_agent_indices_scratch: Vec<usize>,
    pub(super) shopper_candidate_scratch: Vec<AtomicUsize>,
    pub(super) workplace_route_cache: super::employment::WorkplaceRouteCache,
    // The quotes in insertion order with the workplace pass that added each; a quote expires
    // a day of passes later, so the cache holds about one day's searches (`SIM-02-G`).
    pub(super) workplace_route_cache_order:
        std::collections::VecDeque<(u32, super::employment::WorkplaceRouteCacheKey)>,
    pub(super) workplace_route_cache_pass: u32,
    pub(super) workplace_route_cache_building_revision: u64,
    pub(super) workplace_route_cache_entrance_revision: u64,
    pub(super) workplace_route_cache_cch_generation: u32,
    // Network legs of workplace commutes; exact across hours while the CCH revision holds.
    pub(super) workplace_network_quotes:
        crate::simulation::economy::agents::tick::SharedNetworkQuotes,
    // Sorted workplace keys keep the same scratch owner regardless of Rayon scheduling.
    pub(super) workplace_query_shards: Vec<super::employment::WorkplaceQueryShard>,
    pub(super) daily_ledgers: Vec<DailyHouseholdLedger>,
    /// Flows of the calendar day so far (`SIM-02-G`).
    pub(crate) day_totals: CityDayTotals,
    /// Flows of the last closed calendar day, removed buildings included.
    pub(crate) last_day_totals: CityDayTotals,
    /// Coal stocked at power plants at the latest hourly dispatch, for the budget report.
    pub(crate) coal_inventory_kg: f64,
    /// Households whose daily phase asked for a housing review, served by the next hour's pass
    /// with fresh member links (`SIM-02-G`; a per-slot queue in `SIM-02-J`). Saved.
    pub(crate) housing_reviews: Vec<Handle<Household>>,
    /// Households that chose to leave at their daily phase, removed by the next hour's pass.
    /// Saved.
    pub(crate) exits: Vec<Handle<Household>>,
    pub(super) housing_review_scratch: Vec<usize>,
    pub(super) electricity_residents_scratch: Vec<AtomicU32>,
    pub(super) electricity_hour_scratch: Vec<super::electricity::HourlyBuildingElectricity>,
    pub(super) electricity_totals_scratch: Vec<[f64; 4]>,
    /// Closed building accounts retained until the current fiscal day is reported.
    pub(crate) removed_electricity: super::electricity::RemovedBuildingElectricity,
}

/// Household records changed when one building is removed.
pub(crate) struct HouseholdBuildingUndo {
    pub(crate) records: Vec<(usize, Household, DailyHouseholdLedger)>,
    pub(crate) mutated_building_ids: Vec<usize>,
    removed_electricity: super::electricity::RemovedBuildingElectricity,
}

impl HouseholdSystem {
    /// Creates an empty household system.
    pub fn new() -> Self {
        Self {
            households: Vec::new(),
            handles: HandleTable::default(),
            pending_arrivals: PendingHouseholdArrivals::default(),
            held_shoppers: Vec::new(),
            held_visitors: Vec::new(),
            job_funding: Default::default(),
            member_count_scratch: Vec::new(),
            child_count_scratch: Vec::new(),
            adult_count_scratch: Vec::new(),
            elder_count_scratch: Vec::new(),
            shopping_car_scratch: Vec::new(),
            worker_count_scratch: Vec::new(),
            household_member_heads_scratch: Vec::new(),
            household_member_next_scratch: Vec::new(),
            has_unhoused_households: false,
            removal_candidates_scratch: Vec::new(),
            removal_selected_flags_scratch: Vec::new(),
            removal_agent_indices_scratch: Vec::new(),
            shopper_candidate_scratch: Vec::new(),
            workplace_route_cache: Default::default(),
            workplace_route_cache_order: Default::default(),
            workplace_route_cache_pass: 0,
            workplace_route_cache_building_revision: u64::MAX,
            workplace_route_cache_entrance_revision: u64::MAX,
            workplace_route_cache_cch_generation: u32::MAX,
            workplace_network_quotes: Default::default(),
            workplace_query_shards: Vec::new(),
            daily_ledgers: Vec::new(),
            day_totals: CityDayTotals::default(),
            last_day_totals: CityDayTotals::default(),
            coal_inventory_kg: 0.0,
            housing_reviews: Vec::new(),
            exits: Vec::new(),
            housing_review_scratch: Vec::new(),
            electricity_residents_scratch: Vec::new(),
            electricity_hour_scratch: Vec::new(),
            electricity_totals_scratch: Vec::new(),
            removed_electricity: Default::default(),
        }
    }

    /// Captures only household and store records that building invalidation can mutate.
    pub(crate) fn capture_building_undo(
        &mut self,
        removed_building: usize,
    ) -> HouseholdBuildingUndo {
        self.ensure_daily_ledger_len();
        let mut records = Vec::new();
        let mut mutated_building_ids = Vec::new();
        for (household_id, household) in self.households.iter().enumerate() {
            let removed_home = household.home_building_id == removed_building;
            let removed_store = household.reserved_store_building_id == removed_building;
            if !removed_home && !removed_store {
                continue;
            }
            records.push((
                household_id,
                household.clone(),
                self.daily_ledgers[household_id],
            ));
            if household.replenishment_state == REPLENISHMENT_SHOPPING_TO_STORE
                && household.reserved_store_building_id != removed_building
            {
                mutated_building_ids.push(household.reserved_store_building_id);
            }
        }
        mutated_building_ids.sort_unstable();
        mutated_building_ids.dedup();
        HouseholdBuildingUndo {
            records,
            mutated_building_ids,
            removed_electricity: self.removed_electricity,
        }
    }

    /// Restores records captured by [`Self::capture_building_undo`].
    pub(crate) fn restore_building_undo(&mut self, undo: HouseholdBuildingUndo) {
        self.removed_electricity = undo.removed_electricity;
        self.ensure_daily_ledger_len();
        for (household_id, household, ledger) in undo.records {
            if let Some(target) = self.households.get_mut(household_id) {
                *target = household;
            }
            if let Some(target) = self.daily_ledgers.get_mut(household_id) {
                *target = ledger;
            }
        }
        self.invalidate_workplace_route_cache();
    }

    fn invalidate_workplace_route_cache(&mut self) {
        self.workplace_route_cache.clear();
        self.workplace_route_cache_order.clear();
        self.workplace_route_cache_building_revision = u64::MAX;
        self.workplace_route_cache_entrance_revision = u64::MAX;
        self.workplace_route_cache_cch_generation = u32::MAX;
    }

    /// Returns the topology revisions attached to the complete workplace quotes.
    pub(crate) fn workplace_route_cache_revisions(&self) -> (u64, u64, u32) {
        (
            self.workplace_route_cache_building_revision,
            self.workplace_route_cache_entrance_revision,
            self.workplace_route_cache_cch_generation,
        )
    }

    /// Iterates complete workplace quotes with the pass that added each, for deterministic
    /// save serialization.
    pub(crate) fn workplace_route_cache_entries(
        &self,
    ) -> impl Iterator<Item = ((usize, usize, bool), Option<u16>, u32)> + '_ {
        self.workplace_route_cache_order
            .iter()
            .map(|&(pass, key)| (key, self.workplace_route_cache[&key], pass))
    }

    /// The workplace pass counter quotes are stamped with.
    pub(crate) fn workplace_route_cache_pass(&self) -> u32 {
        self.workplace_route_cache_pass
    }

    /// Restores complete workplace quotes after the loaded topology has been rebuilt.
    pub(crate) fn restore_workplace_route_cache(
        &mut self,
        building_revision: u64,
        entrance_revision: u64,
        cch_generation: u32,
        mut entries: Vec<(super::employment::WorkplaceRouteCacheKey, Option<u16>, u32)>,
        pass: u32,
    ) {
        // Quotes of one pass expire together, so their order within it is immaterial.
        entries.sort_unstable_by_key(|&(key, _, stamp)| (pass.wrapping_sub(stamp), key));
        entries.reverse();
        self.workplace_route_cache.clear();
        self.workplace_route_cache_order.clear();
        self.workplace_route_cache.reserve(entries.len());
        for (key, value, stamp) in entries {
            self.workplace_route_cache.insert(key, value);
            self.workplace_route_cache_order.push_back((stamp, key));
        }
        self.workplace_route_cache_pass = pass;
        self.workplace_route_cache_building_revision = building_revision;
        self.workplace_route_cache_entrance_revision = entrance_revision;
        self.workplace_route_cache_cch_generation = cch_generation;
    }

    /// Clears all households.
    pub fn clear(&mut self) {
        self.households.clear();
        self.job_funding.clear();
        self.pending_arrivals = PendingHouseholdArrivals::default();
        self.held_shoppers.clear();
        self.held_visitors.clear();
        self.member_count_scratch.clear();
        self.child_count_scratch.clear();
        self.adult_count_scratch.clear();
        self.elder_count_scratch.clear();
        self.worker_count_scratch.clear();
        self.household_member_heads_scratch.clear();
        self.household_member_next_scratch.clear();
        self.has_unhoused_households = false;
        self.removal_candidates_scratch.clear();
        self.removal_selected_flags_scratch.clear();
        self.removal_agent_indices_scratch.clear();
        self.shopper_candidate_scratch.clear();
        self.invalidate_workplace_route_cache();
        self.workplace_network_quotes = Default::default();
        self.workplace_query_shards.clear();
        self.daily_ledgers.clear();
        self.day_totals = CityDayTotals::default();
        self.last_day_totals = CityDayTotals::default();
        self.coal_inventory_kg = 0.0;
        self.housing_reviews.clear();
        self.exits.clear();
        self.electricity_residents_scratch.clear();
        self.electricity_hour_scratch.clear();
        self.electricity_totals_scratch.clear();
        self.removed_electricity = Default::default();
    }

    /// Remaps building references after a building swap-remove.
    pub fn remap_building_indices(&mut self, mapping: &std::collections::HashMap<usize, usize>) {
        self.job_funding.remap_buildings(mapping);
        self.ensure_daily_ledger_len();
        if mapping.len() == 1 {
            let (&old, &new) = mapping.iter().next().unwrap();
            self.households.par_iter_mut().for_each(|household| {
                if household.home_building_id == old {
                    household.home_building_id = new;
                }
                if household.reserved_store_building_id == old {
                    household.reserved_store_building_id = new;
                }
            });
        } else if !mapping.is_empty() {
            self.households.par_iter_mut().for_each(|household| {
                if let Some(&new_id) = mapping.get(&household.home_building_id) {
                    household.home_building_id = new_id;
                }
                if let Some(&new_id) = mapping.get(&household.reserved_store_building_id) {
                    household.reserved_store_building_id = new_id;
                }
            });
        }
    }

    /// Invalidates references to a building that is being removed.
    pub fn invalidate_building(
        &mut self,
        removed_building: usize,
        allocator: &mut BuildingAllocator,
    ) {
        self.job_funding.invalidate_building(removed_building);
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let household_supply_resource = household_supply_resource_runtime_id(&catalog);
        let retry_cooldown_hours = tuning
            .operational_clock
            .household_replenishment_retry_cooldown_hours;
        let terminal_failure_count = tuning
            .operational_clock
            .household_replenishment_terminal_failure_count;
        self.ensure_daily_ledger_len();
        let daily_ledgers = &mut self.daily_ledgers;
        for (household_id, household) in self.households.iter_mut().enumerate() {
            let removed_home = household.home_building_id == removed_building;
            if removed_home {
                household.home_building_id = usize::MAX;
            }

            let removed_store = household.reserved_store_building_id == removed_building;
            let active_to_store = household.replenishment_state == REPLENISHMENT_SHOPPING_TO_STORE;
            let active_returning =
                household.replenishment_state == REPLENISHMENT_SHOPPING_RETURNING;

            if removed_store && active_returning && !removed_home {
                household.reserved_store_building_id = usize::MAX;
                continue;
            }

            if removed_store || removed_home {
                if active_to_store {
                    let store_idx = household.reserved_store_building_id;
                    if store_idx < allocator.buildings.len() && store_idx != removed_building {
                        allocator.buildings[store_idx].add_inventory_units(
                            household_supply_resource,
                            household.reserved_amount,
                        );
                    }
                    household.budget += household.reserved_total_cost;
                    if let Some(ledger) = daily_ledgers.get_mut(household_id) {
                        ledger.shopping_spend -= household.reserved_total_cost;
                    }
                }
                if active_to_store || active_returning {
                    if let Some(ledger) = daily_ledgers.get_mut(household_id) {
                        ledger.shopper_trips_failed = ledger.shopper_trips_failed.saturating_add(1);
                    }
                    clear_replenishment_request(household);
                    register_replenishment_failure(
                        household,
                        retry_cooldown_hours,
                        terminal_failure_count,
                    );
                }
            }
        }
    }

    /// Returns daily payments and diagnostic counters for settlement, saves and reporting.
    pub(crate) fn daily_ledgers(&self) -> &[DailyHouseholdLedger] {
        &self.daily_ledgers
    }

    /// Restores validated utility payments for an already loaded household without charging it.
    /// Payment order is power, water, sewage; all other ledger fields start a new report window.
    pub(crate) fn restore_utility_payments(&mut self, household_id: usize, paid: [f32; 3]) {
        self.ensure_daily_ledger_len();
        let ledger = &mut self.daily_ledgers[household_id];
        ledger.power_consumption_cost = paid[0];
        ledger.water_consumption_cost = paid[1];
        ledger.sewage_consumption_cost = paid[2];
    }

    /// Restores the number of members paid a wage in the household's open window.
    pub(crate) fn restore_paid_workers(&mut self, household_id: usize, paid_workers: u16) {
        self.ensure_daily_ledger_len();
        self.daily_ledgers[household_id].paid_workers = paid_workers;
    }

    /// Restores physical power meters and collected local payments without charging again.
    pub(crate) fn restore_electricity_account(
        &mut self,
        household_id: usize,
        usage: crate::simulation::economy::electricity::ElectricityUsage,
        unpaid: f32,
        local_payment: f32,
    ) {
        self.ensure_daily_ledger_len();
        let ledger = &mut self.daily_ledgers[household_id];
        ledger.electricity = usage;
        ledger.power_unpaid_cost = unpaid;
        ledger.power_local_payment = local_payment;
    }

    /// Returns the electricity and utility settlement of the last closed calendar day.
    pub(crate) fn last_power_settlement(&self) -> DailyPowerSettlementSummary {
        self.last_day_totals.power
    }

    /// Returns actual city payroll and its electricity subtotal for the last closed day.
    pub(crate) fn last_city_service_wages(&self) -> CityServiceWages {
        self.last_day_totals.city_service_wages
    }

    /// Ends the calendar day: the day's totals, with the bills of buildings removed during it,
    /// become the last day, and an empty day starts (`SIM-02-G`). O(1).
    pub(crate) fn close_calendar_day(&mut self) {
        let mut closed = std::mem::take(&mut self.day_totals);
        let removed = std::mem::take(&mut self.removed_electricity).settlement;
        let power = &mut closed.power;
        power.demand_units += removed.demand_units;
        power.supply_units += removed.supply_units;
        power.served_units += removed.served_units;
        power.household_local_revenue += removed.household_local_revenue;
        power.private_local_revenue += removed.private_local_revenue;
        power.city_service_local_cost += removed.city_service_local_cost;
        power.utility_local_revenue += removed.utility_local_revenue;
        power.city_service_utility_local_cost += removed.city_service_utility_local_cost;
        power.city_service_utility_owa_cost += removed.city_service_utility_owa_cost;
        power.power_auxiliary_local_cost += removed.power_auxiliary_local_cost;
        power.power_upkeep_utility_cost += removed.power_upkeep_utility_cost;
        power.coverage = if power.demand_units > 0.0 {
            (power.served_units / power.demand_units).clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.last_day_totals = closed;
    }

    /// Ends household `household_id`'s 24-hour ledger window at its daily phase: the budget
    /// after settlement. O(1).
    pub(super) fn finish_household_window(&mut self, household_id: usize) {
        self.daily_ledgers[household_id].budget_after = self.households[household_id].budget;
    }

    /// Starts household `household_id`'s next ledger window after its settlement was reported.
    /// O(1).
    pub(crate) fn open_household_window(&mut self, household_id: usize) {
        let budget = self.households[household_id].budget;
        self.daily_ledgers[household_id] = DailyHouseholdLedger {
            budget_before: budget,
            budget_after: budget,
            ..Default::default()
        };
    }

    /// Starts every household's next ledger window at once, as if each had settled.
    #[cfg(test)]
    pub(crate) fn reset_daily_ledgers(&mut self) {
        self.ensure_daily_ledger_len();
        for household_id in 0..self.households.len() {
            self.open_household_window(household_id);
        }
    }

    pub(super) fn ensure_daily_ledger_len(&mut self) {
        if self.daily_ledgers.len() > self.households.len() {
            self.daily_ledgers.truncate(self.households.len());
        }
        if self.daily_ledgers.len() < self.households.len() {
            let start = self.daily_ledgers.len();
            self.daily_ledgers.reserve(self.households.len() - start);
            for household in &self.households[start..] {
                let ledger = DailyHouseholdLedger {
                    budget_before: household.budget,
                    budget_after: household.budget,
                    ..Default::default()
                };
                self.daily_ledgers.push(ledger);
            }
        }
    }

    /// Returns zeroed per-household counters for deterministic parallel membership rebuilds.
    pub(super) fn reset_member_count_scratch(&mut self) -> &[AtomicU32] {
        resize_atomic_scratch(&mut self.member_count_scratch, self.households.len());
        &self.member_count_scratch
    }

    /// Returns zeroed per-household car-eligibility flags for the resident census.
    pub(super) fn reset_shopping_car_scratch(&mut self) -> &[AtomicU32] {
        resize_atomic_scratch(&mut self.shopping_car_scratch, self.households.len());
        &self.shopping_car_scratch
    }

    /// Returns zeroed per-household child counters for deterministic parallel reductions.
    pub(super) fn reset_child_count_scratch(&mut self) -> &[AtomicU32] {
        resize_atomic_scratch(&mut self.child_count_scratch, self.households.len());
        &self.child_count_scratch
    }

    /// Returns zeroed per-household adult counters for deterministic parallel reductions.
    pub(super) fn reset_adult_count_scratch(&mut self) -> &[AtomicU32] {
        resize_atomic_scratch(&mut self.adult_count_scratch, self.households.len());
        &self.adult_count_scratch
    }

    /// Returns zeroed per-household elder counters for deterministic parallel reductions.
    pub(super) fn reset_elder_count_scratch(&mut self) -> &[AtomicU32] {
        resize_atomic_scratch(&mut self.elder_count_scratch, self.households.len());
        &self.elder_count_scratch
    }

    /// Returns zeroed per-building counters for deterministic parallel worker recounts.
    pub(super) fn reset_worker_count_scratch(&mut self, building_count: usize) -> &[AtomicU32] {
        resize_atomic_scratch(&mut self.worker_count_scratch, building_count);
        &self.worker_count_scratch
    }

    /// Returns max-filled per-household shopper candidates for deterministic parallel reductions.
    pub(super) fn reset_shopper_candidate_scratch(&mut self) -> &[AtomicUsize] {
        if self.shopper_candidate_scratch.len() > self.households.len() {
            self.shopper_candidate_scratch
                .truncate(self.households.len());
        }
        while self.shopper_candidate_scratch.len() < self.households.len() {
            self.shopper_candidate_scratch
                .push(AtomicUsize::new(usize::MAX));
        }
        for slot in &self.shopper_candidate_scratch {
            slot.store(usize::MAX, std::sync::atomic::Ordering::Relaxed);
        }
        &self.shopper_candidate_scratch
    }
}

impl std::fmt::Debug for HouseholdSystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HouseholdSystem")
            .field("households", &self.households)
            .finish_non_exhaustive()
    }
}

impl Default for HouseholdSystem {
    fn default() -> Self {
        Self::new()
    }
}

fn resize_atomic_scratch(scratch: &mut Vec<AtomicU32>, len: usize) {
    if scratch.len() > len {
        scratch.truncate(len);
    }
    while scratch.len() < len {
        scratch.push(AtomicU32::new(0));
    }
    for slot in scratch {
        slot.store(0, std::sync::atomic::Ordering::Relaxed);
    }
}
