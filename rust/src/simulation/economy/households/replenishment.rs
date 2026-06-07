//! Household stock consumption, store reservations, pickup, and replenishment state.

use std::sync::atomic::{AtomicBool, Ordering};

use super::data::{Household, HouseholdSystem};
use super::metrics::{
    OPERATIONAL_HOURS_PER_DAY, economy_profile_for_building, household_demand_profile,
    household_supply_resource_runtime_id, stock_days,
};
use crate::debug_log;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::definitions::{
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::zoning::ZoneType;
use rayon::prelude::*;

const GROCERY_SEARCH_MAX_RING: i32 = 6;
const GROCERY_SEARCH_CANDIDATES: usize = 24;

/// Household stock is healthy and no replenishment is pending.
pub const REPLENISHMENT_STABLE: u8 = 0;
/// Household stock fell below the trigger and needs a restock attempt.
pub const REPLENISHMENT_NEEDS: u8 = 1;
/// Household stock was replenished on this economy pass.
pub const REPLENISHMENT_RESERVED: u8 = 2;
/// Household has a reserved supply source and is waiting for pickup-side fulfillment.
pub const REPLENISHMENT_PICKUP_PENDING: u8 = 3;
/// Household stock was replenished on this economy pass.
pub const REPLENISHMENT_FULFILLED: u8 = 4;
/// Household is waiting before retrying another replenishment attempt.
pub const REPLENISHMENT_COOLDOWN: u8 = 5;

#[derive(Default)]
struct ReplenishmentDiagnostics {
    attempts: u32,
    successes: u32,
    failed_no_store_candidates: u32,
    failed_no_sale: u32,
    urgent_cooldown_skips: u32,
    candidate_count: u32,
    rejected_empty: u32,
    rejected_invalid_store: u32,
    rejected_missing_profile: u32,
    rejected_not_output: u32,
    rejected_unaffordable: u32,
    rejected_zero_desired: u32,
    rejected_zero_amount: u32,
    reserved_amount: f32,
    reserved_cost: f32,
}

impl ReplenishmentDiagnostics {
    fn has_signal(&self) -> bool {
        self.attempts > 0 || self.urgent_cooldown_skips > 0
    }
}

impl HouseholdSystem {
    pub(super) fn consume_household_stock(&mut self, agents: &mut AgentSystem) {
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let any_zero_stock = AtomicBool::new(false);
        self.households.par_iter_mut().for_each(|household| {
            if household.member_count == 0 {
                return;
            }
            let hourly_consumption = household.member_count as f32 * household.consumption_rate
                / OPERATIONAL_HOURS_PER_DAY;
            household.stock = (household.stock - hourly_consumption).max(0.0);
            let hourly_utility_cost = household.member_count as f32
                * tuning.households.utility_cost_per_member_per_day
                / OPERATIONAL_HOURS_PER_DAY;
            household.budget = (household.budget - hourly_utility_cost).max(0.0);
            household.stock_days = stock_days(
                household.stock,
                household.member_count,
                household.consumption_rate,
            );
            if matches!(
                household.replenishment_state,
                REPLENISHMENT_RESERVED | REPLENISHMENT_PICKUP_PENDING
            ) {
                return;
            } else if household.replenishment_state == REPLENISHMENT_FULFILLED {
                if household.cooldown_hours > 0 {
                    household.cooldown_hours -= 1;
                }
                household.replenishment_state = REPLENISHMENT_COOLDOWN;
            } else if household.cooldown_hours > 0 {
                household.cooldown_hours -= 1;
                household.replenishment_state = REPLENISHMENT_COOLDOWN;
            } else {
                household.replenishment_state = REPLENISHMENT_STABLE;
            }

            if household.stock_days == 0.0 {
                any_zero_stock.store(true, Ordering::Relaxed);
            }
        });

        if any_zero_stock.load(Ordering::Relaxed) {
            for i in 0..agents.len() {
                let hid = agents.household_id[i];
                if hid < self.households.len() && self.households[hid].stock_days == 0.0 {
                    agents.happiness[i] = (agents.happiness[i] - 4.0).clamp(0.0, 100.0);
                }
            }
        }
    }

    pub(super) fn run_household_replenishment(
        &mut self,
        allocator: &mut BuildingAllocator,
        absolute_hour: u32,
    ) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let check_interval = u32::from(
            tuning
                .operational_clock
                .household_replenishment_check_interval_hours,
        );
        let household_supply_resource = household_supply_resource_runtime_id(&catalog);
        let retry_cooldown_hours = tuning
            .operational_clock
            .household_replenishment_retry_cooldown_hours;

        for hid in 0..self.households.len() {
            self.progress_household_replenishment(
                hid,
                allocator,
                household_supply_resource,
                retry_cooldown_hours,
            );
        }

        let profile = household_demand_profile(&catalog);
        let target_days = profile.stock_target_days;
        let trigger_days = profile.reorder_threshold_days;
        let urgent_restock_candidate_exists = self.households.iter().any(|household| {
            household.member_count > 0
                && household.stock_days == 0.0
                && household.home_building_id < allocator.buildings.len()
                && !matches!(
                    household.replenishment_state,
                    REPLENISHMENT_RESERVED | REPLENISHMENT_PICKUP_PENDING
                )
        });
        let stock_critical_purchase_available = urgent_restock_candidate_exists
            && allocator.buildings.iter().any(|store| {
                !store.broken
                    && !store.economy_broken
                    && !store.is_deserted
                    && matches!(store.zone_type, ZoneType::Commercial)
                    && store.inventory_units(household_supply_resource) > 0.0
                    && economy_profile_for_building(&catalog, store).is_some_and(|profile| {
                        profile.output_port(household_supply_resource).is_some()
                    })
            });
        let mut diagnostics = ReplenishmentDiagnostics::default();
        let mut candidates = Vec::with_capacity(GROCERY_SEARCH_CANDIDATES);

        for hid in 0..self.households.len() {
            let household = &self.households[hid];
            let check_offset_matches = absolute_hour % check_interval
                == u32::from(household.replenishment_offset_hours % check_interval as u16);
            let stock_critical_urgent =
                household.stock_days == 0.0 && stock_critical_purchase_available;
            if stock_critical_urgent && household.cooldown_hours > 0 {
                diagnostics.urgent_cooldown_skips += 1;
            }
            if household.member_count == 0
                || household.home_building_id == usize::MAX
                || household.home_building_id >= allocator.buildings.len()
                || household.replenishment_state == REPLENISHMENT_RESERVED
                || household.replenishment_state == REPLENISHMENT_PICKUP_PENDING
                || household.cooldown_hours > 0
                || household.stock_days >= trigger_days
                || (!stock_critical_urgent && !check_offset_matches)
            {
                continue;
            }

            let home = &allocator.buildings[household.home_building_id];
            allocator.fill_nearby_buildings_by_zones(
                home.center_x,
                home.center_y,
                &[ZoneType::Commercial],
                GROCERY_SEARCH_MAX_RING,
                GROCERY_SEARCH_CANDIDATES,
                &mut candidates,
            );
            diagnostics.attempts += 1;
            diagnostics.candidate_count += candidates.len() as u32;
            if candidates.is_empty() {
                diagnostics.failed_no_store_candidates += 1;
            }

            let daily_consumption = household.member_count as f32 * household.consumption_rate;
            let target_stock = target_days * daily_consumption;
            let mut desired_amount = (target_stock - household.stock).max(0.0);
            let mut found_sale = None;

            for &candidate in &candidates {
                let store = &allocator.buildings[candidate];
                // A store can sell from existing inventory even when utility
                // service is temporarily unavailable. Broken, economy-broken,
                // and deserted stores are excluded.
                if store.inventory_units(household_supply_resource) <= 0.0
                    || store.broken
                    || store.economy_broken
                    || store.is_deserted
                {
                    if store.broken || store.economy_broken || store.is_deserted {
                        diagnostics.rejected_invalid_store += 1;
                    } else {
                        diagnostics.rejected_empty += 1;
                    }
                    continue;
                }
                let Some(store_profile) = economy_profile_for_building(&catalog, store) else {
                    diagnostics.rejected_missing_profile += 1;
                    continue;
                };
                if store_profile
                    .output_port(household_supply_resource)
                    .is_none()
                {
                    diagnostics.rejected_not_output += 1;
                    continue;
                }
                let available_stock = store.inventory_units(household_supply_resource);
                let max_affordable_amount = if store_profile.unit_price_currency > 0.0 {
                    household.budget / store_profile.unit_price_currency
                } else {
                    f32::MAX
                };
                let amount = desired_amount
                    .min(available_stock)
                    .min(max_affordable_amount);
                let total_cost = amount * store_profile.unit_price_currency;
                if amount > 0.0 && household.budget >= total_cost {
                    found_sale = Some((candidate, amount, total_cost));
                    break;
                }
                if desired_amount <= 0.0 {
                    diagnostics.rejected_zero_desired += 1;
                } else if max_affordable_amount <= 0.0 || household.budget < total_cost {
                    diagnostics.rejected_unaffordable += 1;
                } else {
                    diagnostics.rejected_zero_amount += 1;
                }
                desired_amount = desired_amount.min(available_stock);
            }

            let household = &mut self.households[hid];
            if let Some((store_idx, amount, total_cost)) = found_sale {
                let store = &mut allocator.buildings[store_idx];
                store.remove_inventory_units(household_supply_resource, amount);
                household.budget -= total_cost;
                household.reserved_store_building_id = store_idx;
                household.reserved_amount = amount;
                household.reserved_total_cost = total_cost;
                household.pickup_eta_hours = tuning.operational_clock.household_pickup_eta_hours;
                household.replenishment_state = REPLENISHMENT_RESERVED;
                diagnostics.successes += 1;
                diagnostics.reserved_amount += amount;
                diagnostics.reserved_cost += total_cost;
            } else {
                household.replenishment_state = REPLENISHMENT_COOLDOWN;
                household.cooldown_hours = tuning
                    .operational_clock
                    .household_replenishment_retry_cooldown_hours;
                diagnostics.failed_no_sale += 1;
            }
        }

        if diagnostics.has_signal() {
            debug_log!(
                "economy",
                "household replenishment diagnostics: hour={} attempts={} success={} failed={} \
                 urgent_cooldown_skips={} candidates={} no_store_candidates={} \
                 rejected_empty={} rejected_invalid_store={} rejected_missing_profile={} \
                 rejected_not_output={} rejected_unaffordable={} rejected_zero_desired={} \
                 rejected_zero_amount={} reserved_amount={:.1} reserved_cost={:.1}",
                absolute_hour,
                diagnostics.attempts,
                diagnostics.successes,
                diagnostics.failed_no_sale,
                diagnostics.urgent_cooldown_skips,
                diagnostics.candidate_count,
                diagnostics.failed_no_store_candidates,
                diagnostics.rejected_empty,
                diagnostics.rejected_invalid_store,
                diagnostics.rejected_missing_profile,
                diagnostics.rejected_not_output,
                diagnostics.rejected_unaffordable,
                diagnostics.rejected_zero_desired,
                diagnostics.rejected_zero_amount,
                diagnostics.reserved_amount,
                diagnostics.reserved_cost
            );
        }
    }

    fn progress_household_replenishment(
        &mut self,
        hid: usize,
        allocator: &mut BuildingAllocator,
        household_supply_resource: u16,
        retry_cooldown_hours: u16,
    ) {
        let Some(household) = self.households.get_mut(hid) else {
            return;
        };
        match household.replenishment_state {
            REPLENISHMENT_RESERVED => {
                if household.pickup_eta_hours > 0 {
                    household.pickup_eta_hours -= 1;
                }
                if household.pickup_eta_hours == 0 {
                    household.replenishment_state = REPLENISHMENT_PICKUP_PENDING;
                }
            }
            REPLENISHMENT_PICKUP_PENDING => {
                let store_idx = household.reserved_store_building_id;
                if store_idx == usize::MAX || store_idx >= allocator.buildings.len() {
                    cancel_replenishment_pickup(household, retry_cooldown_hours);
                    return;
                }

                let store = &mut allocator.buildings[store_idx];
                if store.broken || store.economy_broken || store.is_deserted {
                    store.add_inventory_units(household_supply_resource, household.reserved_amount);
                    cancel_replenishment_pickup(household, retry_cooldown_hours);
                    return;
                }
                store.revenue += household.reserved_total_cost;
                store.operating_budget += household.reserved_total_cost;
                household.stock += household.reserved_amount;
                household.stock_days = stock_days(
                    household.stock,
                    household.member_count,
                    household.consumption_rate,
                );
                household.replenishment_state = REPLENISHMENT_FULFILLED;
                household.cooldown_hours = 1;
                household.reserved_store_building_id = usize::MAX;
                household.reserved_amount = 0.0;
                household.reserved_total_cost = 0.0;
                household.pickup_eta_hours = 0;
            }
            _ => {}
        }
    }
}

pub(super) fn clear_replenishment_request(household: &mut Household) {
    household.replenishment_state = REPLENISHMENT_STABLE;
    household.cooldown_hours = 0;
    household.reserved_store_building_id = usize::MAX;
    household.reserved_amount = 0.0;
    household.reserved_total_cost = 0.0;
    household.pickup_eta_hours = 0;
}

fn cancel_replenishment_pickup(household: &mut Household, retry_cooldown_hours: u16) {
    household.budget += household.reserved_total_cost;
    clear_replenishment_request(household);
    household.replenishment_state = REPLENISHMENT_COOLDOWN;
    household.cooldown_hours = retry_cooldown_hours;
}

pub(super) fn stable_replenishment_offset_hours(
    home_building_id: usize,
    household_seed: u32,
) -> u16 {
    let mixed = (home_building_id as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(u64::from(household_seed).wrapping_mul(0xBF58_476D_1CE4_E5B9));
    (mixed % OPERATIONAL_HOURS_PER_DAY as u64) as u16
}
