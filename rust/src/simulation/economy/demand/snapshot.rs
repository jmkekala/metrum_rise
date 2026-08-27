//! Settled economy snapshots consumed by demand pressure and planning.

use super::config::DemandConfig;
use super::credits::clamp01;
use super::spawn_need::{
    OutputAbsorptionContext, add_resource_amount, resource_amount, resource_is_commercial_input,
};
use super::types::EPSILON;
use crate::assets::ZoneClass;
use crate::debug_log;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::economy::agents::{HouseholdAgeComposition, household_age_composition};
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, ResourceRuntimeId, RuntimeEconomyCatalog,
    RuntimeEconomyTuning,
};
#[cfg(test)]
use crate::simulation::economy::definitions::{
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::economy::fiscal::{CityFiscalPolicy, tax_amount};
use crate::simulation::economy::households::{
    Household, HouseholdSystem, active_worker_capacity_equivalent_for_profile_with_floor_scale,
    active_worker_capacity_for_profile_with_floor_scale,
    building_operation_factors_with_floor_scale, candidate_immigrant_household_size_from_flat_size,
    commercial_activity_signal_for_city, household_reserve_days, service_funded_worker_capacity,
};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::{NodeType, TransitFlags, TransitType};
use crate::simulation::work_area::profile_kind_uses_explicit_work_area;
use crate::simulation::zoning::ZoneType;
use rayon::prelude::*;
use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Clone, Debug)]
pub(super) struct ResidentialOccupantSnapshot {
    pub(super) household_count_by_building: Vec<u32>,
    pub(super) min_reserve_days_by_building: Vec<f32>,
}

#[derive(Default)]
/// Reusable atomics for the parallel residential occupant reduction.
pub(super) struct ResidentialOccupantScratch {
    /// Per-building live household counts during one snapshot reduction.
    household_count_by_building: Vec<AtomicU32>,
    /// Per-building minimum reserve-days value encoded as `f32::to_bits()`.
    min_reserve_days_by_building: Vec<AtomicU32>,
}

impl ResidentialOccupantSnapshot {
    #[cfg(test)]
    pub(super) fn from_runtime(
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
    ) -> Self {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let mut scratch = ResidentialOccupantScratch::default();
        Self::from_runtime_with_catalog(
            allocator,
            households,
            catalog.as_ref(),
            tuning.as_ref(),
            &mut scratch,
        )
    }

    pub(super) fn from_runtime_with_catalog(
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        scratch: &mut ResidentialOccupantScratch,
    ) -> Self {
        scratch.reset(allocator.buildings.len());
        let household_count_by_building = &scratch.household_count_by_building;
        let min_reserve_days_by_building = &scratch.min_reserve_days_by_building;

        households.households.par_iter().for_each(|household| {
            if household.member_count == 0
                || household.adult_count.saturating_add(household.elder_count) == 0
            {
                return;
            }
            let home_building_id = household.home_building_id;
            if home_building_id >= allocator.buildings.len()
                || allocator.buildings[home_building_id].broken
                || allocator.buildings[home_building_id].economy_broken
                || allocator.buildings[home_building_id].is_deserted
                || allocator.buildings[home_building_id].is_under_construction()
            {
                return;
            }
            household_count_by_building[home_building_id].fetch_add(1, Ordering::Relaxed);
            atomic_min_f32(
                &min_reserve_days_by_building[home_building_id],
                household_reserve_days(catalog, tuning, household),
            );
        });

        Self {
            household_count_by_building: household_count_by_building
                .iter()
                .map(|count| count.load(Ordering::Relaxed))
                .collect(),
            min_reserve_days_by_building: min_reserve_days_by_building
                .iter()
                .map(|reserve| f32::from_bits(reserve.load(Ordering::Relaxed)))
                .collect(),
        }
    }
}

impl ResidentialOccupantScratch {
    fn reset(&mut self, building_count: usize) {
        resize_atomic_u32_scratch(&mut self.household_count_by_building, building_count, 0);
        resize_atomic_u32_scratch(
            &mut self.min_reserve_days_by_building,
            building_count,
            f32::INFINITY.to_bits(),
        );
    }
}

fn atomic_min_f32(target: &AtomicU32, value: f32) {
    let value = if value.is_finite() {
        value.max(0.0)
    } else {
        f32::INFINITY
    };
    let value_bits = value.to_bits();
    let mut current_bits = target.load(Ordering::Relaxed);
    loop {
        let current = f32::from_bits(current_bits);
        if value >= current {
            return;
        }
        match target.compare_exchange_weak(
            current_bits,
            value_bits,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return,
            Err(next_bits) => current_bits = next_bits,
        }
    }
}

fn resize_atomic_u32_scratch(scratch: &mut Vec<AtomicU32>, len: usize, reset_value: u32) {
    if scratch.len() > len {
        scratch.truncate(len);
    }
    while scratch.len() < len {
        scratch.push(AtomicU32::new(reset_value));
    }
    for slot in scratch {
        slot.store(reset_value, Ordering::Relaxed);
    }
}

pub(super) struct DailyDemandSnapshot {
    pub(super) vacant_household_slots: u32,
    pub(super) total_household_count: u32,
    pub(super) housed_household_count: u32,
    pub(super) unhoused_household_count: u32,
    pub(super) zero_budget_household_count: u32,
    pub(super) persistent_exit_eligible_household_count: u32,
    pub(super) unhoused_household_ratio: f32,
    pub(super) zero_budget_household_ratio: f32,
    pub(super) housing_availability: f32,
    pub(super) incoming_household_need: f32,
    pub(super) open_job_household_pull: f32,
    pub(super) marginal_commercial_job_household_pull: f32,
    pub(super) regional_growth_household_pull: f32,
    pub(super) household_affordability: f32,
    pub(super) household_stock_stability: f32,
    pub(super) commercial_capacity_deficit: f32,
    pub(super) under_construction_household_slots: u32,
    #[cfg(test)]
    pub(super) unmet_commercial_consumer_demand: f32,
    pub(super) committed_unmet_commercial_consumer_demand: f32,
    pub(super) committed_unmet_commercial_consumer_demand_by_resource:
        Vec<(ResourceRuntimeId, f32)>,
    pub(super) industrial_input_capacity_deficit: f32,
    #[cfg(test)]
    pub(super) commercial_input_need_value: f32,
    #[cfg(test)]
    pub(super) local_industrial_input_capacity_value: f32,
    #[cfg(test)]
    pub(super) industrial_missing_input_value: f32,
    pub(super) committed_industrial_missing_input_value: f32,
    pub(super) external_connection_available: f32,
    pub(super) connected_border_count: u32,
    pub(super) city_treasury_balance: f32,
    pub(super) candidate_household_size: f32,
    pub(super) candidate_child_count: u16,
    pub(super) candidate_adult_count: u16,
    pub(super) candidate_elder_count: u16,
    pub(super) immigrant_starter_savings_per_household: f32,
    pub(super) candidate_daily_essential_cost: f32,
    pub(super) unemployment_daily_benefit_per_adult: f32,
    pub(super) pension_daily_benefit_per_elder: f32,
    pub(super) child_support_daily_benefit_per_child: f32,
    pub(super) existing_unemployed_member_count: u32,
    pub(super) existing_child_count: u32,
    pub(super) existing_elder_count: u32,
    pub(super) open_job_slots: u32,
    pub(super) marginal_commercial_job_slots: u32,
    pub(super) marginal_commercial_job_equivalent_slots: f32,
    pub(super) move_in_job_slots: u32,
    pub(super) move_in_job_equivalent_slots: f32,
    pub(super) average_move_in_job_wage_per_day: f32,
    pub(super) physical_worker_capacity: u32,
    pub(super) funded_worker_capacity: u32,
    pub(super) open_jobs_unfunded: u32,
    pub(super) output_absorption: OutputAbsorptionContext,
    // Fraction of commercial input value sourced from OWA rather than local industrial.
    pub(super) commercial_owa_dependency: f32,
    #[cfg(test)]
    pub(super) commercial_owa_input_value: f32,
}

impl DailyDemandSnapshot {
    #[cfg(test)]
    pub(super) fn from_runtime(
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
        graph: &RegionGraph,
        config: &DemandConfig,
        treasury_balance: f64,
    ) -> Self {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let fiscal_policy = CityFiscalPolicy::from_runtime_tuning(tuning.as_ref());
        Self::from_runtime_with_catalog(
            allocator,
            households,
            graph,
            config,
            catalog.as_ref(),
            tuning.as_ref(),
            &fiscal_policy,
            treasury_balance,
            &[],
        )
    }

    pub(super) fn from_runtime_with_catalog(
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
        graph: &RegionGraph,
        config: &DemandConfig,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        fiscal_policy: &CityFiscalPolicy,
        treasury_balance: f64,
        service_funding_by_building: &[f32],
    ) -> Self {
        let mut commercial_profile_output_resources = Vec::new();
        for profile in catalog.all_profiles() {
            if !matches!(
                profile.kind,
                EconomyProfileRuntimeKind::Store | EconomyProfileRuntimeKind::ServiceStore
            ) {
                continue;
            }
            for output_port in &profile.outputs {
                add_resource_amount(
                    &mut commercial_profile_output_resources,
                    output_port.resource_runtime_id,
                    1.0,
                );
            }
        }
        let mut demand_sink_rates_by_resource = Vec::new();
        for profile in catalog.all_profiles() {
            if profile.kind != EconomyProfileRuntimeKind::DemandSink {
                continue;
            }
            for input_port in &profile.inputs {
                if resource_amount(
                    &commercial_profile_output_resources,
                    input_port.resource_runtime_id,
                ) <= 0.0
                {
                    continue;
                }
                add_resource_amount(
                    &mut demand_sink_rates_by_resource,
                    input_port.resource_runtime_id,
                    profile.consumption_rate_per_resident,
                );
            }
        }
        let mut daily_supply_cost_per_resident = 0.0_f32;
        for &(resource_runtime_id, consumption_rate_per_resident) in &demand_sink_rates_by_resource
        {
            let resource_price = catalog
                .unit_price_for_resource(resource_runtime_id)
                .unwrap_or_else(|| {
                    let resource_id = catalog
                        .resource_id_for_runtime_id(resource_runtime_id)
                        .unwrap_or("<unknown>");
                    panic!(
                        "resource '{resource_id}' used by household demand sink has no catalog price"
                    )
                });
            daily_supply_cost_per_resident +=
                consumption_rate_per_resident.max(0.0) * resource_price.max(0.0);
        }
        let daily_essential_cost_per_resident =
            daily_supply_cost_per_resident + tuning.households.utility_cost_per_member_per_day;
        let commercial_activity_signal =
            commercial_activity_signal_for_city(catalog, &households.households, allocator);
        let household_accumulator =
            collect_household_snapshot_accumulator(allocator, households, catalog, tuning, config);
        let housed_resident_count = household_accumulator.housed_resident_count;
        let service_activity_scale_by_resource = service_store_activity_scale_by_resource(
            catalog,
            allocator,
            &demand_sink_rates_by_resource,
            housed_resident_count,
        );
        let building_accumulator = collect_building_snapshot_accumulator(
            allocator,
            catalog,
            fiscal_policy.income_tax_rate,
            &demand_sink_rates_by_resource,
            commercial_activity_signal.household_supply_resource_runtime_id,
            commercial_activity_signal.activity_floor_scale,
            &service_activity_scale_by_resource,
            service_funding_by_building,
        );
        let total_household_slots = building_accumulator.total_household_slots;
        let occupied_household_slots = building_accumulator.occupied_household_slots;
        let existing_private_building_count = building_accumulator.existing_private_building_count;
        let total_commercial_owa_input = building_accumulator.total_commercial_owa_input;
        let total_commercial_local_input = building_accumulator.total_commercial_local_input;
        let total_commercial_expected_input = building_accumulator.total_commercial_expected_input;
        let under_construction_household_slots =
            building_accumulator.under_construction_household_slots;
        let filled_job_count = building_accumulator.filled_job_count;
        let open_job_slots = building_accumulator.open_job_slots;
        let open_job_wage_sum = building_accumulator.open_job_wage_sum;
        let physical_worker_capacity = building_accumulator.physical_worker_capacity;
        let funded_worker_capacity = building_accumulator.funded_worker_capacity;
        let open_jobs_unfunded = building_accumulator.open_jobs_unfunded;
        let live_commercial_output_capacity_by_resource =
            building_accumulator.live_commercial_output_capacity_by_resource;
        let committed_commercial_output_capacity_by_resource =
            building_accumulator.committed_commercial_output_capacity_by_resource;
        let commercial_input_need_by_resource =
            building_accumulator.commercial_input_need_by_resource;
        let live_local_industrial_output_capacity_by_resource =
            building_accumulator.live_local_industrial_output_capacity_by_resource;
        let committed_local_industrial_output_capacity_by_resource =
            building_accumulator.committed_local_industrial_output_capacity_by_resource;
        let committed_output_capacity_by_resource =
            building_accumulator.committed_output_capacity_by_resource;
        let sales_scaled_household_supply_output_units_per_day =
            building_accumulator.sales_scaled_household_supply_output_units_per_day;

        let housed_adult_count = household_accumulator.housed_adult_count;
        let existing_unemployed_member_count = housed_adult_count.saturating_sub(filled_job_count);
        let prefer_worker_capable_candidate = open_job_slots > existing_unemployed_member_count;

        let vacant_household_slots = total_household_slots.saturating_sub(occupied_household_slots);
        let candidate =
            candidate_household_preview(allocator, households, prefer_worker_capable_candidate);
        let candidate_household_size = candidate.household_size as f32;
        let immigrant_starter_savings_per_household =
            candidate_household_size * tuning.households.immigrant_starting_budget_per_member;
        let candidate_daily_essential_cost =
            candidate_household_size * daily_essential_cost_per_resident;
        let candidate_household_supply_demand_units = candidate_household_size
            * resource_amount(
                &demand_sink_rates_by_resource,
                commercial_activity_signal.household_supply_resource_runtime_id,
            );
        let marginal_commercial_jobs = marginal_commercial_job_forecast_for_candidate_household(
            allocator,
            catalog,
            fiscal_policy.income_tax_rate,
            commercial_activity_signal.household_supply_resource_runtime_id,
            commercial_activity_signal.demand_units_per_day,
            commercial_activity_signal.activity_floor_scale,
            sales_scaled_household_supply_output_units_per_day,
            candidate_household_supply_demand_units,
            service_funding_by_building,
        );
        let live_child_count = household_accumulator.live_child_count;
        let live_elder_count = household_accumulator.live_elder_count;
        let housed_household_count = household_accumulator.housed_household_count;
        let unhoused_household_count = household_accumulator.unhoused_household_count;
        let zero_budget_household_count = household_accumulator.zero_budget_household_count;
        let persistent_exit_eligible_household_count =
            household_accumulator.persistent_exit_eligible_household_count;
        let household_affordability_sum = household_accumulator.household_affordability_sum;
        let household_stock_stability_sum = household_accumulator.household_stock_stability_sum;
        let output_absorption = OutputAbsorptionContext::from_resource_amounts(
            catalog.resource_count(),
            &committed_output_capacity_by_resource,
            &demand_sink_rates_by_resource,
            housed_resident_count,
            &commercial_input_need_by_resource,
        );

        let total_household_count = housed_household_count.saturating_add(unhoused_household_count);
        let housing_availability = if total_household_slots == 0 {
            0.0
        } else {
            clamp01(vacant_household_slots as f32 / total_household_slots as f32)
        };
        let household_affordability = if housed_household_count == 0 {
            1.0
        } else {
            clamp01(household_affordability_sum / housed_household_count as f32)
        };
        let household_stock_stability = if housed_household_count == 0 {
            1.0
        } else {
            clamp01(household_stock_stability_sum / housed_household_count as f32)
        };
        let mut total_commercial_consumer_demand = 0.0_f32;
        let mut unmet_commercial_consumer_demand = 0.0_f32;
        let mut committed_unmet_commercial_consumer_demand = 0.0_f32;
        let mut committed_unmet_commercial_consumer_demand_by_resource = Vec::new();
        for &(resource_runtime_id, consumption_rate_per_resident) in &demand_sink_rates_by_resource
        {
            let consumer_demand = if resource_runtime_id
                == commercial_activity_signal.household_supply_resource_runtime_id
            {
                commercial_activity_signal.demand_units_per_day
            } else {
                consumption_rate_per_resident * housed_resident_count as f32
            };
            if consumer_demand <= 0.0 {
                continue;
            }
            let live_capacity = resource_amount(
                &live_commercial_output_capacity_by_resource,
                resource_runtime_id,
            );
            let committed_capacity = resource_amount(
                &committed_commercial_output_capacity_by_resource,
                resource_runtime_id,
            );
            total_commercial_consumer_demand += consumer_demand;
            unmet_commercial_consumer_demand += (consumer_demand - live_capacity).max(0.0);
            let committed_gap = (consumer_demand - committed_capacity).max(0.0);
            committed_unmet_commercial_consumer_demand += committed_gap;
            add_resource_amount(
                &mut committed_unmet_commercial_consumer_demand_by_resource,
                resource_runtime_id,
                committed_gap,
            );
        }
        let commercial_capacity_deficit = if total_commercial_consumer_demand <= 0.0 {
            0.0
        } else {
            clamp01(unmet_commercial_consumer_demand / total_commercial_consumer_demand)
        };
        let mut commercial_input_need_value = 0.0_f32;
        let mut local_industrial_input_capacity_value = 0.0_f32;
        let mut industrial_missing_input_value = 0.0_f32;
        let mut committed_industrial_missing_input_value = 0.0_f32;
        for &(resource_runtime_id, need_units) in &commercial_input_need_by_resource {
            let resource_price = catalog
                .unit_price_for_resource(resource_runtime_id)
                .unwrap_or_else(|| {
                    let resource_id = catalog
                        .resource_id_for_runtime_id(resource_runtime_id)
                        .unwrap_or("<unknown>");
                    panic!(
                        "resource '{resource_id}' used by commercial input capacity has no catalog price"
                    )
                });
            let live_local_units = resource_amount(
                &live_local_industrial_output_capacity_by_resource,
                resource_runtime_id,
            );
            let committed_local_units = resource_amount(
                &committed_local_industrial_output_capacity_by_resource,
                resource_runtime_id,
            );
            commercial_input_need_value += need_units.max(0.0) * resource_price.max(0.0);
            local_industrial_input_capacity_value +=
                live_local_units.max(0.0) * resource_price.max(0.0);
            industrial_missing_input_value +=
                (need_units - live_local_units).max(0.0) * resource_price.max(0.0);
            committed_industrial_missing_input_value +=
                (need_units - committed_local_units).max(0.0) * resource_price.max(0.0);
        }
        let industrial_input_capacity_deficit = if commercial_input_need_value <= EPSILON {
            0.0
        } else {
            clamp01(industrial_missing_input_value / commercial_input_need_value)
        };
        let connected_border_count = graph
            .nodes()
            .iter()
            .enumerate()
            .filter(|(idx, node)| {
                node.node_type == NodeType::Border
                    && graph.node_adjacency(*idx as u32).iter().any(|&edge_idx| {
                        let edge = graph.edge(edge_idx);
                        !edge.deleted
                            && edge.primary_type == TransitType::Road
                            && (edge.allowed_types & TransitFlags::CAR) != 0
                    })
            })
            .count() as u32;
        // A physical connection is still required: with no border road there is
        // no route in, and no policy can conjure one. What changed is that a
        // connection is no longer all-or-nothing. Border policy scales how much
        // of that connection admits arrivals, so a sealed border reads 0.0,
        // which is the same value the no-connection case has always produced
        // and which every consumer downstream already handles.
        let external_connection_available = if connected_border_count > 0 {
            fiscal_policy.border_openness
        } else {
            0.0
        };
        let unhoused_household_ratio = if total_household_count == 0 {
            0.0
        } else {
            clamp01(unhoused_household_count as f32 / total_household_count as f32)
        };
        let zero_budget_household_ratio = if total_household_count == 0 {
            0.0
        } else {
            clamp01(zero_budget_household_count as f32 / total_household_count as f32)
        };
        let candidate_effective_workers = f32::from(candidate.composition.adult_count);
        let net_open_job_slots = open_job_slots.saturating_sub(existing_unemployed_member_count);
        let existing_unemployed_after_open_jobs =
            existing_unemployed_member_count.saturating_sub(open_job_slots);
        let net_open_job_wage_sum = if open_job_slots == 0 {
            0.0
        } else {
            open_job_wage_sum * net_open_job_slots as f32 / open_job_slots as f32
        };
        let net_marginal_commercial_job_slots = marginal_commercial_jobs
            .open_slots
            .saturating_sub(existing_unemployed_after_open_jobs);
        let net_marginal_commercial_job_equivalent_slots = (marginal_commercial_jobs
            .job_equivalent_slots
            - existing_unemployed_after_open_jobs as f32)
            .max(0.0);
        let net_marginal_commercial_job_wage_sum =
            if marginal_commercial_jobs.job_equivalent_slots <= EPSILON {
                0.0
            } else {
                marginal_commercial_jobs.job_equivalent_net_wage_sum
                    * net_marginal_commercial_job_equivalent_slots
                    / marginal_commercial_jobs.job_equivalent_slots
            };
        let move_in_job_slots =
            net_open_job_slots.saturating_add(net_marginal_commercial_job_slots);
        let move_in_job_equivalent_slots =
            net_open_job_slots as f32 + net_marginal_commercial_job_equivalent_slots;
        let average_move_in_job_wage_per_day = if move_in_job_equivalent_slots <= EPSILON {
            0.0
        } else {
            (net_open_job_wage_sum + net_marginal_commercial_job_wage_sum)
                / move_in_job_equivalent_slots
        };
        let open_job_household_pull = if candidate_effective_workers <= EPSILON {
            0.0
        } else {
            net_open_job_slots as f32 / candidate_effective_workers
        };
        let marginal_commercial_job_household_pull = if candidate_effective_workers <= EPSILON {
            0.0
        } else {
            (net_marginal_commercial_job_equivalent_slots / candidate_effective_workers).min(1.0)
        };
        let bootstrap_household_pull = if total_household_count == 0 { 1.0 } else { 0.0 };
        let regional_growth_household_pull = regional_growth_household_pull(
            config,
            total_household_count,
            external_connection_available,
            household_affordability,
            household_stock_stability,
            unhoused_household_ratio,
            zero_budget_household_ratio,
        );
        let incoming_household_need = (open_job_household_pull
            + marginal_commercial_job_household_pull
            + regional_growth_household_pull)
            .max(bootstrap_household_pull);

        // Fraction of commercial input value sourced from OWA vs local industrial.
        // Uses expected daily input cost as a minimum denominator so a tiny OWA
        // emergency import (e.g. one unit when the building budget is briefly
        // exhausted) does not register as full OWA dependency when local supply
        // exists and normal throughput resumes the next hour.
        let commercial_owa_dependency = {
            let actual_total = total_commercial_owa_input + total_commercial_local_input;
            let denom = actual_total.max(total_commercial_expected_input);
            if denom <= 0.0 {
                0.0
            } else {
                clamp01(total_commercial_owa_input / denom)
            }
        };

        debug_log!(
            "spawn",
            "daily_snapshot: border_nodes={} ext_conn={:.0} housing_avail={:.2} \
             unhoused_ratio={:.2} zero_budget_ratio={:.2} stock_stab={:.2} afford={:.2} \
             incoming_need={:.2} job_pull={:.2} marginal_com_pull={:.2} regional_pull={:.2} \
             com_cap_def={:.2} unmet_com_units={:.1} \
             ind_cap_def={:.2} com_input_need={:.1} local_ind_capacity={:.1} \
             ind_missing={:.1} pending_home_slots={} owa_dep={:.2} owa_input_value={:.1} \
             treasury={:.0} cand_size={:.1} cand=(children:{} adults:{} elders:{}) \
             open_jobs={} marginal_com_jobs={} marginal_com_job_equiv={:.2} move_in_jobs={} move_in_job_equiv={:.2} existing_unemployed={} physical_worker_capacity={} \
             funded_worker_capacity={} open_jobs_unfunded={} \
             private_buildings={}",
            connected_border_count,
            external_connection_available,
            housing_availability,
            unhoused_household_ratio,
            zero_budget_household_ratio,
            household_stock_stability,
            household_affordability,
            incoming_household_need,
            open_job_household_pull,
            marginal_commercial_job_household_pull,
            regional_growth_household_pull,
            commercial_capacity_deficit,
            unmet_commercial_consumer_demand,
            industrial_input_capacity_deficit,
            commercial_input_need_value,
            local_industrial_input_capacity_value,
            industrial_missing_input_value,
            under_construction_household_slots,
            commercial_owa_dependency,
            total_commercial_owa_input,
            treasury_balance,
            candidate_household_size,
            candidate.composition.child_count,
            candidate.composition.adult_count,
            candidate.composition.elder_count,
            open_job_slots,
            marginal_commercial_jobs.open_slots,
            marginal_commercial_jobs.job_equivalent_slots,
            move_in_job_slots,
            move_in_job_equivalent_slots,
            existing_unemployed_member_count,
            physical_worker_capacity,
            funded_worker_capacity,
            open_jobs_unfunded,
            existing_private_building_count,
        );

        Self {
            vacant_household_slots,
            total_household_count,
            housed_household_count,
            unhoused_household_count,
            zero_budget_household_count,
            persistent_exit_eligible_household_count,
            unhoused_household_ratio,
            zero_budget_household_ratio,
            housing_availability,
            incoming_household_need,
            open_job_household_pull,
            marginal_commercial_job_household_pull,
            regional_growth_household_pull,
            household_affordability,
            household_stock_stability,
            commercial_capacity_deficit,
            #[cfg(test)]
            unmet_commercial_consumer_demand,
            committed_unmet_commercial_consumer_demand,
            committed_unmet_commercial_consumer_demand_by_resource,
            under_construction_household_slots,
            industrial_input_capacity_deficit,
            #[cfg(test)]
            commercial_input_need_value,
            #[cfg(test)]
            local_industrial_input_capacity_value,
            #[cfg(test)]
            industrial_missing_input_value,
            committed_industrial_missing_input_value,
            external_connection_available,
            connected_border_count,
            city_treasury_balance: treasury_balance as f32,
            candidate_household_size,
            candidate_child_count: candidate.composition.child_count,
            candidate_adult_count: candidate.composition.adult_count,
            candidate_elder_count: candidate.composition.elder_count,
            immigrant_starter_savings_per_household,
            candidate_daily_essential_cost,
            unemployment_daily_benefit_per_adult: fiscal_policy
                .unemployment_benefit_per_adult_per_day,
            pension_daily_benefit_per_elder: fiscal_policy.pension_per_elder_per_day,
            child_support_daily_benefit_per_child: fiscal_policy.child_support_per_child_per_day,
            existing_unemployed_member_count,
            existing_child_count: live_child_count,
            existing_elder_count: live_elder_count,
            open_job_slots,
            marginal_commercial_job_slots: marginal_commercial_jobs.open_slots,
            marginal_commercial_job_equivalent_slots: marginal_commercial_jobs.job_equivalent_slots,
            move_in_job_slots,
            move_in_job_equivalent_slots,
            average_move_in_job_wage_per_day,
            physical_worker_capacity,
            funded_worker_capacity,
            open_jobs_unfunded,
            output_absorption,
            commercial_owa_dependency,
            #[cfg(test)]
            commercial_owa_input_value: total_commercial_owa_input,
        }
    }
}

fn regional_growth_household_pull(
    config: &DemandConfig,
    total_household_count: u32,
    external_connection_available: f32,
    household_affordability: f32,
    household_stock_stability: f32,
    unhoused_household_ratio: f32,
    zero_budget_household_ratio: f32,
) -> f32 {
    let base_pull = config.household_action.regional_growth_household_pull;
    if base_pull <= EPSILON || external_connection_available <= EPSILON {
        return 0.0;
    }

    let soft_households = config
        .household_action
        .regional_growth_soft_households
        .max(1.0);
    let household_gap = clamp01(1.0 - total_household_count as f32 / soft_households);
    let stability = clamp01(household_affordability.min(household_stock_stability));
    let failure_damping = 1.0 - unhoused_household_ratio.max(zero_budget_household_ratio);

    clamp01(
        external_connection_available
            * base_pull
            * household_gap
            * stability
            * clamp01(failure_damping),
    )
}

const BUILDING_SNAPSHOT_CHUNK_SIZE: usize = 1024;
const HOUSEHOLD_SNAPSHOT_CHUNK_SIZE: usize = 2048;
const BASELINE_STARTER_CONSTRUCTION_HOUSEHOLD_SIZE: u16 = 2;

#[derive(Clone, Copy, Debug, Default)]
struct CandidateHouseholdPreview {
    household_size: u16,
    composition: HouseholdAgeComposition,
}

fn candidate_household_preview(
    allocator: &BuildingAllocator,
    households: &HouseholdSystem,
    prefer_worker_capable: bool,
) -> CandidateHouseholdPreview {
    let next_household_id = households.households.len();
    if let Some((home_building_id, household_size)) = allocator
        .next_household_admission_candidate_for_household(next_household_id, prefer_worker_capable)
    {
        return CandidateHouseholdPreview {
            household_size,
            composition: household_age_composition(
                home_building_id,
                next_household_id,
                household_size,
            ),
        };
    }

    let household_size = construction_candidate_household_size_from_registry(allocator) as u16;
    if household_size == 0 {
        return CandidateHouseholdPreview::default();
    }
    CandidateHouseholdPreview {
        household_size,
        composition: household_age_composition(usize::MAX, next_household_id, household_size),
    }
}

#[derive(Default)]
struct BuildingSnapshotAccumulator {
    total_household_slots: u32,
    occupied_household_slots: u32,
    existing_private_building_count: u32,
    total_commercial_owa_input: f32,
    total_commercial_local_input: f32,
    total_commercial_expected_input: f32,
    under_construction_household_slots: u32,
    filled_job_count: u32,
    open_job_slots: u32,
    open_job_wage_sum: f32,
    physical_worker_capacity: u32,
    funded_worker_capacity: u32,
    open_jobs_unfunded: u32,
    sales_scaled_household_supply_output_units_per_day: f32,
    committed_output_capacity_by_resource: Vec<(ResourceRuntimeId, f32)>,
    live_commercial_output_capacity_by_resource: Vec<(ResourceRuntimeId, f32)>,
    committed_commercial_output_capacity_by_resource: Vec<(ResourceRuntimeId, f32)>,
    commercial_input_need_by_resource: Vec<(ResourceRuntimeId, f32)>,
    live_local_industrial_output_capacity_by_resource: Vec<(ResourceRuntimeId, f32)>,
    committed_local_industrial_output_capacity_by_resource: Vec<(ResourceRuntimeId, f32)>,
}

impl BuildingSnapshotAccumulator {
    fn absorb_building(
        &mut self,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        income_tax_rate: f32,
        demand_sink_rates_by_resource: &[(ResourceRuntimeId, f32)],
        household_supply_resource_runtime_id: ResourceRuntimeId,
        commercial_activity_floor_scale: f32,
        service_activity_scale_by_resource: &[(ResourceRuntimeId, f32)],
        service_funding_by_building: &[f32],
        idx: usize,
        building: &Building,
    ) {
        if building.broken || building.economy_broken || building.is_deserted {
            return;
        }

        let is_private_building = allocator
            .registry
            .get(&building.asset_id)
            .and_then(|entry| entry.manifest.building.as_ref())
            .map(|authored| authored.is_zoned_private())
            .unwrap_or(!matches!(building.zone_type, ZoneType::None));
        if is_private_building {
            self.existing_private_building_count =
                self.existing_private_building_count.saturating_add(1);
        }

        let active_profile = catalog.profile_by_runtime_id(building.economy_profile_runtime_id);
        let profile_activity_floor_scale = active_profile
            .map(|profile| {
                profile_activity_floor_scale(
                    building,
                    profile,
                    commercial_activity_floor_scale,
                    service_activity_scale_by_resource,
                )
            })
            .unwrap_or(1.0);

        if building.is_under_construction() {
            if matches!(building.zone_type, ZoneType::Residential) {
                self.under_construction_household_slots = self
                    .under_construction_household_slots
                    .saturating_add(allocator.registry.household_capacity(&building.asset_id));
            }
            if let Some(profile) = active_profile {
                for output_port in &profile.outputs {
                    add_resource_amount(
                        &mut self.committed_output_capacity_by_resource,
                        output_port.resource_runtime_id,
                        output_port.units_per_day,
                    );
                }
                if matches!(building.zone_type, ZoneType::Commercial) {
                    for output_port in &profile.outputs {
                        if resource_amount(
                            demand_sink_rates_by_resource,
                            output_port.resource_runtime_id,
                        ) > 0.0
                        {
                            add_resource_amount(
                                &mut self.committed_commercial_output_capacity_by_resource,
                                output_port.resource_runtime_id,
                                output_port.units_per_day,
                            );
                        }
                    }
                }
                if matches!(building.zone_type, ZoneType::Industrial) {
                    for output_port in &profile.outputs {
                        if resource_is_commercial_input(catalog, output_port.resource_runtime_id) {
                            add_resource_amount(
                                &mut self.committed_local_industrial_output_capacity_by_resource,
                                output_port.resource_runtime_id,
                                output_port.units_per_day,
                            );
                        }
                    }
                }
            }
            return;
        }

        if matches!(building.zone_type, ZoneType::Residential) {
            let household_capacity = allocator.household_capacity(idx);
            self.total_household_slots = self
                .total_household_slots
                .saturating_add(household_capacity);
            let occupied = building.occupancy.min(household_capacity);
            self.occupied_household_slots = self.occupied_household_slots.saturating_add(occupied);
        }

        if let Some(profile) =
            active_profile.filter(|profile| profile_offers_work(building, profile))
        {
            let physical_worker_capacity = active_worker_capacity_for_profile_with_floor_scale(
                catalog,
                building,
                profile,
                profile_activity_floor_scale,
            );
            let funded_worker_capacity = service_funded_worker_capacity(
                physical_worker_capacity,
                profile,
                idx,
                service_funding_by_building,
            );
            if physical_worker_capacity > funded_worker_capacity {
                self.open_jobs_unfunded = self.open_jobs_unfunded.saturating_add(
                    physical_worker_capacity.saturating_sub(funded_worker_capacity),
                );
            }
            if physical_worker_capacity > 0 {
                self.physical_worker_capacity = self
                    .physical_worker_capacity
                    .saturating_add(physical_worker_capacity);
                self.funded_worker_capacity = self
                    .funded_worker_capacity
                    .saturating_add(funded_worker_capacity);
            }
            if funded_worker_capacity > 0 {
                let average_daily_wage = profile.average_daily_wage();
                let filled_workers = building.worker_count.min(funded_worker_capacity);
                self.filled_job_count = self.filled_job_count.saturating_add(filled_workers);
                if average_daily_wage > 0.1 {
                    let budget_capacity = if allocator.is_city_service_building(building) {
                        funded_worker_capacity
                    } else {
                        (building.operating_budget.max(0.0) / average_daily_wage).floor() as u32
                    };
                    let effective_capacity = funded_worker_capacity.min(budget_capacity);
                    let open_slots = effective_capacity.saturating_sub(filled_workers);
                    let net_daily_wage =
                        average_daily_wage - tax_amount(average_daily_wage, income_tax_rate);
                    self.open_job_slots = self.open_job_slots.saturating_add(open_slots);
                    self.open_job_wage_sum += open_slots as f32 * net_daily_wage.max(0.0);
                }
            }
        }

        if let Some(profile) = active_profile {
            let output_capacity_scale = profile_output_capacity_scale(
                catalog,
                building,
                profile,
                profile_activity_floor_scale,
            );
            for output_port in &profile.outputs {
                add_resource_amount(
                    &mut self.committed_output_capacity_by_resource,
                    output_port.resource_runtime_id,
                    output_port.units_per_day * output_capacity_scale,
                );
            }
        }

        if matches!(building.zone_type, ZoneType::Commercial) {
            self.total_commercial_owa_input += building.daily_owa_input_value;
            self.total_commercial_local_input += building.daily_local_input_value;
            if let Some(profile) = active_profile {
                if matches!(profile.kind, EconomyProfileRuntimeKind::Store) {
                    self.sales_scaled_household_supply_output_units_per_day += profile
                        .outputs
                        .iter()
                        .filter(|port| {
                            port.resource_runtime_id == household_supply_resource_runtime_id
                        })
                        .map(|port| port.units_per_day.max(0.0))
                        .sum::<f32>();
                }
                let commercial_capacity_scale = profile_output_capacity_scale(
                    catalog,
                    building,
                    profile,
                    profile_activity_floor_scale,
                );
                for output_port in &profile.outputs {
                    if resource_amount(
                        demand_sink_rates_by_resource,
                        output_port.resource_runtime_id,
                    ) > 0.0
                    {
                        add_resource_amount(
                            &mut self.live_commercial_output_capacity_by_resource,
                            output_port.resource_runtime_id,
                            output_port.units_per_day * commercial_capacity_scale,
                        );
                        add_resource_amount(
                            &mut self.committed_commercial_output_capacity_by_resource,
                            output_port.resource_runtime_id,
                            output_port.units_per_day * commercial_capacity_scale,
                        );
                    }
                }
                for input_port in &profile.inputs {
                    let input_units_per_day = input_port.units_per_day * commercial_capacity_scale;
                    add_resource_amount(
                        &mut self.commercial_input_need_by_resource,
                        input_port.resource_runtime_id,
                        input_units_per_day,
                    );
                    let resource_price = catalog
                        .unit_price_for_resource(input_port.resource_runtime_id)
                        .unwrap_or_else(|| {
                            let resource_id = catalog
                                .resource_id_for_runtime_id(input_port.resource_runtime_id)
                                .unwrap_or("<unknown>");
                            panic!(
                                "resource '{resource_id}' used by profile '{}' has no catalog price",
                                profile.id
                            )
                        });
                    self.total_commercial_expected_input += input_units_per_day * resource_price;
                }
            }
        }

        if matches!(building.zone_type, ZoneType::Industrial) {
            if let Some(profile) = active_profile {
                for output_port in &profile.outputs {
                    if resource_is_commercial_input(catalog, output_port.resource_runtime_id) {
                        add_resource_amount(
                            &mut self.live_local_industrial_output_capacity_by_resource,
                            output_port.resource_runtime_id,
                            output_port.units_per_day,
                        );
                        add_resource_amount(
                            &mut self.committed_local_industrial_output_capacity_by_resource,
                            output_port.resource_runtime_id,
                            output_port.units_per_day,
                        );
                    }
                }
            }
        }
    }

    fn merge(&mut self, other: Self) {
        self.total_household_slots = self
            .total_household_slots
            .saturating_add(other.total_household_slots);
        self.occupied_household_slots = self
            .occupied_household_slots
            .saturating_add(other.occupied_household_slots);
        self.existing_private_building_count = self
            .existing_private_building_count
            .saturating_add(other.existing_private_building_count);
        self.total_commercial_owa_input += other.total_commercial_owa_input;
        self.total_commercial_local_input += other.total_commercial_local_input;
        self.total_commercial_expected_input += other.total_commercial_expected_input;
        self.under_construction_household_slots = self
            .under_construction_household_slots
            .saturating_add(other.under_construction_household_slots);
        self.filled_job_count = self.filled_job_count.saturating_add(other.filled_job_count);
        self.open_job_slots = self.open_job_slots.saturating_add(other.open_job_slots);
        self.open_job_wage_sum += other.open_job_wage_sum;
        self.physical_worker_capacity = self
            .physical_worker_capacity
            .saturating_add(other.physical_worker_capacity);
        self.funded_worker_capacity = self
            .funded_worker_capacity
            .saturating_add(other.funded_worker_capacity);
        self.open_jobs_unfunded = self
            .open_jobs_unfunded
            .saturating_add(other.open_jobs_unfunded);
        self.sales_scaled_household_supply_output_units_per_day +=
            other.sales_scaled_household_supply_output_units_per_day;
        merge_resource_amounts(
            &mut self.committed_output_capacity_by_resource,
            other.committed_output_capacity_by_resource,
        );
        merge_resource_amounts(
            &mut self.live_commercial_output_capacity_by_resource,
            other.live_commercial_output_capacity_by_resource,
        );
        merge_resource_amounts(
            &mut self.committed_commercial_output_capacity_by_resource,
            other.committed_commercial_output_capacity_by_resource,
        );
        merge_resource_amounts(
            &mut self.commercial_input_need_by_resource,
            other.commercial_input_need_by_resource,
        );
        merge_resource_amounts(
            &mut self.committed_local_industrial_output_capacity_by_resource,
            other.committed_local_industrial_output_capacity_by_resource,
        );
        merge_resource_amounts(
            &mut self.live_local_industrial_output_capacity_by_resource,
            other.live_local_industrial_output_capacity_by_resource,
        );
    }
}

fn profile_offers_work(building: &Building, profile: &EconomyProfileRuntime) -> bool {
    matches!(
        building.zone_type,
        ZoneType::Commercial | ZoneType::Industrial
    ) || matches!(
        profile.kind,
        EconomyProfileRuntimeKind::Extractor
            | EconomyProfileRuntimeKind::FieldProducer
            | EconomyProfileRuntimeKind::UtilityProducer
            | EconomyProfileRuntimeKind::UtilityProcessor
    )
}

fn service_store_activity_scale_by_resource(
    catalog: &RuntimeEconomyCatalog,
    allocator: &BuildingAllocator,
    demand_sink_rates_by_resource: &[(ResourceRuntimeId, f32)],
    housed_resident_count: u32,
) -> Vec<(ResourceRuntimeId, f32)> {
    if housed_resident_count == 0 {
        return Vec::new();
    }
    let live_output_units_by_resource =
        service_store_live_output_units_by_resource(catalog, allocator);
    if live_output_units_by_resource.is_empty() {
        return Vec::new();
    }

    let mut activity_scale_by_resource = Vec::new();
    for &(resource_runtime_id, consumption_rate_per_resident) in demand_sink_rates_by_resource {
        let live_output_units =
            resource_amount(&live_output_units_by_resource, resource_runtime_id);
        if live_output_units <= EPSILON {
            continue;
        }
        let demand_units = housed_resident_count as f32 * consumption_rate_per_resident.max(0.0);
        add_resource_amount(
            &mut activity_scale_by_resource,
            resource_runtime_id,
            (demand_units / live_output_units).clamp(0.0, 1.0),
        );
    }
    activity_scale_by_resource
}

fn service_store_live_output_units_by_resource(
    catalog: &RuntimeEconomyCatalog,
    allocator: &BuildingAllocator,
) -> Vec<(ResourceRuntimeId, f32)> {
    allocator
        .buildings
        .par_iter()
        .filter_map(|building| {
            if building.broken
                || building.economy_broken
                || building.is_deserted
                || building.is_under_construction()
                || building.edge_idx == usize::MAX
                || !matches!(building.zone_type, ZoneType::Commercial)
            {
                return None;
            }
            let profile = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)?;
            (profile.kind == EconomyProfileRuntimeKind::ServiceStore)
                .then_some(profile.outputs.as_slice())
        })
        .fold(Vec::new, |mut local, outputs| {
            for output in outputs {
                add_resource_amount(
                    &mut local,
                    output.resource_runtime_id,
                    output.units_per_day.max(0.0),
                );
            }
            local
        })
        .reduce(Vec::new, |mut left, right| {
            merge_resource_amounts(&mut left, right);
            left
        })
}

fn profile_activity_floor_scale(
    building: &Building,
    profile: &EconomyProfileRuntime,
    commercial_activity_floor_scale: f32,
    service_activity_scale_by_resource: &[(ResourceRuntimeId, f32)],
) -> f32 {
    if profile.kind == EconomyProfileRuntimeKind::ServiceStore {
        return profile
            .outputs
            .iter()
            .map(|output| {
                resource_amount(
                    service_activity_scale_by_resource,
                    output.resource_runtime_id,
                )
            })
            .fold(0.0, f32::max)
            .clamp(0.0, 1.0);
    }
    if profile_kind_uses_explicit_work_area(profile.kind) {
        return building.commercial_activity_floor_scale.clamp(0.0, 1.0);
    }
    if matches!(building.zone_type, ZoneType::Commercial)
        && profile.kind == EconomyProfileRuntimeKind::Store
    {
        return commercial_activity_floor_scale.clamp(0.0, 1.0);
    }
    1.0
}

fn collect_building_snapshot_accumulator(
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    income_tax_rate: f32,
    demand_sink_rates_by_resource: &[(ResourceRuntimeId, f32)],
    household_supply_resource_runtime_id: ResourceRuntimeId,
    commercial_activity_floor_scale: f32,
    service_activity_scale_by_resource: &[(ResourceRuntimeId, f32)],
    service_funding_by_building: &[f32],
) -> BuildingSnapshotAccumulator {
    let mut chunks: Vec<_> = allocator
        .buildings
        .par_chunks(BUILDING_SNAPSHOT_CHUNK_SIZE)
        .enumerate()
        .map(|(chunk_idx, buildings)| {
            let mut accumulator = BuildingSnapshotAccumulator::default();
            let start_idx = chunk_idx * BUILDING_SNAPSHOT_CHUNK_SIZE;
            for (local_idx, building) in buildings.iter().enumerate() {
                accumulator.absorb_building(
                    allocator,
                    catalog,
                    income_tax_rate,
                    demand_sink_rates_by_resource,
                    household_supply_resource_runtime_id,
                    commercial_activity_floor_scale,
                    service_activity_scale_by_resource,
                    service_funding_by_building,
                    start_idx + local_idx,
                    building,
                );
            }
            (chunk_idx, accumulator)
        })
        .collect();
    chunks.sort_unstable_by_key(|(chunk_idx, _)| *chunk_idx);

    let mut merged = BuildingSnapshotAccumulator::default();
    for (_, accumulator) in chunks {
        merged.merge(accumulator);
    }
    merged
}

#[derive(Default)]
struct MarginalCommercialJobForecast {
    open_slots: u32,
    job_equivalent_slots: f32,
    job_equivalent_net_wage_sum: f32,
}

fn marginal_commercial_job_forecast_for_candidate_household(
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    income_tax_rate: f32,
    household_supply_resource_runtime_id: ResourceRuntimeId,
    current_household_supply_demand_units_per_day: f32,
    current_commercial_activity_floor_scale: f32,
    full_household_supply_output_units_per_day: f32,
    candidate_household_supply_demand_units: f32,
    service_funding_by_building: &[f32],
) -> MarginalCommercialJobForecast {
    if candidate_household_supply_demand_units <= EPSILON
        || full_household_supply_output_units_per_day <= EPSILON
    {
        return MarginalCommercialJobForecast::default();
    }
    let marginal_floor_scale = ((current_household_supply_demand_units_per_day
        + candidate_household_supply_demand_units)
        / full_household_supply_output_units_per_day)
        .clamp(0.0, 1.0);
    if marginal_floor_scale <= current_commercial_activity_floor_scale + EPSILON {
        return MarginalCommercialJobForecast::default();
    }

    allocator
        .buildings
        .par_iter()
        .enumerate()
        .filter_map(|(idx, building)| {
            if building.broken
                || building.economy_broken
                || building.is_deserted
                || building.is_under_construction()
                || !matches!(building.zone_type, ZoneType::Commercial)
            {
                return None;
            }
            let profile = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)?;
            let household_supply_output = profile
                .output_port(household_supply_resource_runtime_id)
                .map(|port| port.units_per_day.max(0.0))
                .unwrap_or(0.0);
            if !matches!(profile.kind, EconomyProfileRuntimeKind::Store)
                || household_supply_output <= EPSILON
                || !profile_offers_work(building, profile)
            {
                return None;
            }
            let average_daily_wage = profile.average_daily_wage();
            if average_daily_wage <= 0.1 {
                return None;
            }

            let current_physical_capacity = active_worker_capacity_for_profile_with_floor_scale(
                catalog,
                building,
                profile,
                current_commercial_activity_floor_scale,
            );
            let current_funded_capacity = service_funded_worker_capacity(
                current_physical_capacity,
                profile,
                idx,
                service_funding_by_building,
            );
            let marginal_physical_capacity = active_worker_capacity_for_profile_with_floor_scale(
                catalog,
                building,
                profile,
                marginal_floor_scale,
            );
            let marginal_funded_capacity = service_funded_worker_capacity(
                marginal_physical_capacity,
                profile,
                idx,
                service_funding_by_building,
            );

            let budget_capacity =
                (building.operating_budget.max(0.0) / average_daily_wage).floor() as u32;
            let filled_workers = building.worker_count;
            let current_open_slots = current_funded_capacity
                .min(budget_capacity)
                .saturating_sub(filled_workers.min(current_funded_capacity));
            let marginal_open_slots = marginal_funded_capacity
                .min(budget_capacity)
                .saturating_sub(filled_workers.min(marginal_funded_capacity));
            let added_open_slots = marginal_open_slots.saturating_sub(current_open_slots);
            let current_worker_equivalent =
                active_worker_capacity_equivalent_for_profile_with_floor_scale(
                    catalog,
                    building,
                    profile,
                    current_commercial_activity_floor_scale,
                );
            let marginal_worker_equivalent =
                active_worker_capacity_equivalent_for_profile_with_floor_scale(
                    catalog,
                    building,
                    profile,
                    marginal_floor_scale,
                );
            let worker_equivalent_ceiling = budget_capacity.min(service_funded_worker_capacity(
                profile.worker_capacity,
                profile,
                idx,
                service_funding_by_building,
            )) as f32;
            let added_worker_equivalent = (marginal_worker_equivalent
                .min(worker_equivalent_ceiling)
                - current_worker_equivalent.min(worker_equivalent_ceiling))
            .max(0.0);
            if added_open_slots == 0 && added_worker_equivalent <= EPSILON {
                return None;
            }

            let net_daily_wage =
                (average_daily_wage - tax_amount(average_daily_wage, income_tax_rate)).max(0.0);
            let job_equivalent_slots = (added_open_slots as f32).max(added_worker_equivalent);
            Some(MarginalCommercialJobForecast {
                open_slots: added_open_slots,
                job_equivalent_slots,
                job_equivalent_net_wage_sum: job_equivalent_slots * net_daily_wage,
            })
        })
        .reduce(MarginalCommercialJobForecast::default, |left, right| {
            MarginalCommercialJobForecast {
                open_slots: left.open_slots.saturating_add(right.open_slots),
                job_equivalent_slots: left.job_equivalent_slots + right.job_equivalent_slots,
                job_equivalent_net_wage_sum: left.job_equivalent_net_wage_sum
                    + right.job_equivalent_net_wage_sum,
            }
        })
}

fn profile_output_capacity_scale(
    catalog: &RuntimeEconomyCatalog,
    building: &Building,
    profile: &EconomyProfileRuntime,
    commercial_activity_floor_scale: f32,
) -> f32 {
    if !matches!(building.zone_type, ZoneType::Commercial) || profile.worker_capacity == 0 {
        return 1.0;
    }
    if profile.kind == EconomyProfileRuntimeKind::ServiceStore {
        return building_operation_factors_with_floor_scale(
            catalog,
            building,
            profile,
            commercial_activity_floor_scale,
        )
        .throughput_factor
        .clamp(0.0, 1.0);
    }
    let active_capacity = active_worker_capacity_for_profile_with_floor_scale(
        catalog,
        building,
        profile,
        commercial_activity_floor_scale,
    );
    (active_capacity as f32 / profile.worker_capacity.max(1) as f32).clamp(0.0, 1.0)
}

fn merge_resource_amounts(
    target: &mut Vec<(ResourceRuntimeId, f32)>,
    source: Vec<(ResourceRuntimeId, f32)>,
) {
    for (resource_runtime_id, amount) in source {
        add_resource_amount(target, resource_runtime_id, amount);
    }
}

#[derive(Default)]
struct HouseholdSnapshotAccumulator {
    housed_resident_count: u32,
    housed_adult_count: u32,
    housed_child_count: u32,
    housed_elder_count: u32,
    live_child_count: u32,
    live_elder_count: u32,
    housed_household_count: u32,
    unhoused_household_count: u32,
    zero_budget_household_count: u32,
    persistent_exit_eligible_household_count: u32,
    household_affordability_sum: f32,
    household_stock_stability_sum: f32,
}

impl HouseholdSnapshotAccumulator {
    fn absorb_household(
        &mut self,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        config: &DemandConfig,
        household: &Household,
    ) {
        if household.member_count == 0 {
            return;
        }
        if household.budget <= EPSILON {
            self.zero_budget_household_count = self.zero_budget_household_count.saturating_add(1);
        }
        self.live_child_count = self
            .live_child_count
            .saturating_add(household.child_count as u32);
        self.live_elder_count = self
            .live_elder_count
            .saturating_add(household.elder_count as u32);
        let is_housed = household.adult_count.saturating_add(household.elder_count) > 0
            && household.home_building_id < allocator.buildings.len()
            && !allocator.buildings[household.home_building_id].broken
            && !allocator.buildings[household.home_building_id].economy_broken
            && !allocator.buildings[household.home_building_id].is_deserted
            && allocator.buildings[household.home_building_id].is_operational();
        if is_housed {
            self.housed_household_count = self.housed_household_count.saturating_add(1);
            self.housed_resident_count = self
                .housed_resident_count
                .saturating_add(household.member_count as u32);
            self.housed_adult_count = self
                .housed_adult_count
                .saturating_add(household.adult_count as u32);
            self.housed_child_count = self
                .housed_child_count
                .saturating_add(household.child_count as u32);
            self.housed_elder_count = self
                .housed_elder_count
                .saturating_add(household.elder_count as u32);
            self.household_affordability_sum += clamp01(
                household_reserve_days(catalog, tuning, household)
                    / config
                        .signal_normalization
                        .household_affordability_target_reserve_days,
            );
            self.household_stock_stability_sum += clamp01(
                household.stock_days
                    / config
                        .signal_normalization
                        .household_stock_stability_target_days,
            );
        } else {
            self.unhoused_household_count = self.unhoused_household_count.saturating_add(1);
            let is_destitute = household.budget <= EPSILON
                && household.stock_days
                    <= config.household_action.persistent_exit_destitute_stock_days;
            let destitute_exit_eligible = is_destitute
                && household.unhoused_days_elapsed
                    >= config
                        .household_action
                        .persistent_exit_destitute_unhoused_days;
            let max_unhoused_exit_eligible = household.unhoused_days_elapsed
                >= config.household_action.persistent_exit_max_unhoused_days;
            if destitute_exit_eligible || max_unhoused_exit_eligible {
                self.persistent_exit_eligible_household_count = self
                    .persistent_exit_eligible_household_count
                    .saturating_add(1);
            }
        }
    }

    fn merge(&mut self, other: Self) {
        self.housed_resident_count = self
            .housed_resident_count
            .saturating_add(other.housed_resident_count);
        self.housed_adult_count = self
            .housed_adult_count
            .saturating_add(other.housed_adult_count);
        self.housed_child_count = self
            .housed_child_count
            .saturating_add(other.housed_child_count);
        self.housed_elder_count = self
            .housed_elder_count
            .saturating_add(other.housed_elder_count);
        self.live_child_count = self.live_child_count.saturating_add(other.live_child_count);
        self.live_elder_count = self.live_elder_count.saturating_add(other.live_elder_count);
        self.housed_household_count = self
            .housed_household_count
            .saturating_add(other.housed_household_count);
        self.unhoused_household_count = self
            .unhoused_household_count
            .saturating_add(other.unhoused_household_count);
        self.zero_budget_household_count = self
            .zero_budget_household_count
            .saturating_add(other.zero_budget_household_count);
        self.persistent_exit_eligible_household_count = self
            .persistent_exit_eligible_household_count
            .saturating_add(other.persistent_exit_eligible_household_count);
        self.household_affordability_sum += other.household_affordability_sum;
        self.household_stock_stability_sum += other.household_stock_stability_sum;
    }
}

fn collect_household_snapshot_accumulator(
    allocator: &BuildingAllocator,
    households: &HouseholdSystem,
    catalog: &RuntimeEconomyCatalog,
    tuning: &RuntimeEconomyTuning,
    config: &DemandConfig,
) -> HouseholdSnapshotAccumulator {
    let mut chunks: Vec<_> = households
        .households
        .par_chunks(HOUSEHOLD_SNAPSHOT_CHUNK_SIZE)
        .enumerate()
        .map(|(chunk_idx, households)| {
            let mut accumulator = HouseholdSnapshotAccumulator::default();
            for household in households {
                accumulator.absorb_household(allocator, catalog, tuning, config, household);
            }
            (chunk_idx, accumulator)
        })
        .collect();
    chunks.sort_unstable_by_key(|(chunk_idx, _)| *chunk_idx);

    let mut merged = HouseholdSnapshotAccumulator::default();
    for (_, accumulator) in chunks {
        merged.merge(accumulator);
    }
    merged
}

fn construction_candidate_household_size_from_registry(allocator: &BuildingAllocator) -> f32 {
    let mut smallest_candidate_size = u16::MAX;
    for asset_id in allocator
        .registry
        .buildings_for_zone(ZoneClass::Residential)
    {
        let Some(entry) = allocator.registry.get(asset_id) else {
            continue;
        };
        let Some(building) = entry.manifest.building.as_ref() else {
            continue;
        };
        if !building.is_zoned_private()
            || building.level != 1
            || building.household_capacity.unwrap_or(0) == 0
        {
            continue;
        }
        if let Some(candidate_size) = candidate_immigrant_household_size_from_flat_size(
            allocator.registry.flat_size_m2(asset_id),
        ) {
            smallest_candidate_size = smallest_candidate_size.min(
                candidate_size
                    .min(BASELINE_STARTER_CONSTRUCTION_HOUSEHOLD_SIZE)
                    .max(1),
            );
        }
    }
    if smallest_candidate_size == u16::MAX {
        0.0
    } else {
        smallest_candidate_size as f32
    }
}
