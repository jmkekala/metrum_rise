// SPDX-License-Identifier: GPL-2.0-only

//! Economy snapshots consumed by project planning and household diagnostics.

use super::config::DemandConfig;
use super::credits::clamp01;
use super::spawn_need::OutputAbsorptionContext;
use super::types::EPSILON;
use crate::debug_log;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, ResourceRuntimeId, RuntimeEconomyCatalog,
    RuntimeEconomyTuning,
};
#[cfg(test)]
use crate::simulation::economy::definitions::{
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::economy::fiscal::{CityFiscalPolicy, tax_amount};
use crate::simulation::economy::hiring::JobFunding;
use crate::simulation::economy::households::{
    Household, HouseholdSystem, active_worker_capacity_for_profile_with_floor_scale,
    affordable_consumer_residents, commercial_activity_signal_for_city, household_is_housed,
    household_reserve_days, output_area_scale_for_building,
};
use crate::simulation::economy::reduction::ordered_fold;
use crate::simulation::economy::resource_totals::{
    add_resource_amount, merge_resource_amounts, resource_amount,
};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::{NodeType, TransitFlags, TransitType};
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
            if household.member_count == 0 || !household_is_housed(household, allocator) {
                return;
            }
            let home_building_id = household.home_building_id;
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

#[derive(Clone, Copy)]
/// A displaced household's eligibility and available labour, in household priority order.
pub(super) struct UnhousedWorkforce {
    /// Current relocation affordability, evaluated by the ordinary household contract.
    pub(super) reserve_days: f32,
    /// Adults with no current workplace assignment; already employed adults are excluded.
    pub(super) workers: u32,
}

pub(super) struct DailyDemandSnapshot {
    /// Existing residents who must claim eligible future homes before prospective arrivals.
    pub(super) unhoused_workforce: Vec<UnhousedWorkforce>,
    pub(super) vacant_household_slots: u32,
    pub(super) total_household_count: u32,
    pub(super) housed_household_count: u32,
    pub(super) unhoused_household_count: u32,
    pub(super) zero_budget_household_count: u32,
    pub(super) persistent_exit_eligible_household_count: u32,
    pub(super) unhoused_household_ratio: f32,
    pub(super) zero_budget_household_ratio: f32,
    pub(super) under_construction_household_slots: u32,
    pub(super) constructing_by_use: [u32; 3],
    pub(super) queued_by_use: [u32; 3],
    pub(super) committed_unmet_commercial_consumer_demand_by_resource:
        Vec<(ResourceRuntimeId, f32)>,
    pub(super) connected_border_count: u32,
    pub(super) city_treasury_balance: f32,
    /// Recurring consumption and variable imported utility cost per resident.
    pub(super) daily_essential_cost_per_resident: f32,
    /// Full home base electricity allowance, charged once per prospective household.
    pub(super) daily_home_electricity_cost: f32,
    pub(super) unemployment_daily_benefit_per_adult: f32,
    pub(super) pension_daily_benefit_per_elder: f32,
    pub(super) child_support_daily_benefit_per_child: f32,
    pub(super) existing_unemployed_member_count: u32,
    pub(super) existing_child_count: u32,
    pub(super) existing_elder_count: u32,
    pub(super) open_job_slots: u32,
    pub(super) move_in_job_slots: u32,
    pub(super) move_in_job_equivalent_slots: f32,
    pub(super) average_move_in_job_wage_per_day: f32,
    pub(super) physical_worker_capacity: u32,
    pub(super) funded_worker_capacity: u32,
    pub(super) open_jobs_unfunded: u32,
    pub(super) output_absorption: OutputAbsorptionContext,
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
    ) -> Self {
        Self::from_runtime_with_funding(
            allocator,
            households,
            graph,
            config,
            catalog,
            tuning,
            fiscal_policy,
            treasury_balance,
            &households.job_funding,
        )
    }

    /// Builds jobs and business purchases from the same freshly validated operation book.
    pub(super) fn from_runtime_with_funding(
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
        graph: &RegionGraph,
        config: &DemandConfig,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        fiscal_policy: &CityFiscalPolicy,
        treasury_balance: f64,
        job_funding: &JobFunding,
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
        // Admission must remain affordable with imported utilities, before local coverage exists.
        let daily_home_electricity_cost =
            crate::simulation::economy::households::household_daily_utility_allowance(
                catalog, tuning, 0.0,
            ) * tuning.owa_import_price_multiplier;
        let daily_essential_cost_per_resident = daily_supply_cost_per_resident
            * (1.0 + fiscal_policy.household_vat_rate)
            + crate::simulation::economy::households::household_daily_utility_allowance(
                catalog, tuning, 1.0,
            ) * tuning.owa_import_price_multiplier
            - daily_home_electricity_cost;
        let commercial_activity_signal =
            commercial_activity_signal_for_city(catalog, &households.households, allocator);
        let household_accumulator =
            collect_household_snapshot_accumulator(allocator, households, catalog, tuning, config);
        let housed_resident_count = household_accumulator.housed_resident_count;
        let building_accumulator = collect_building_snapshot_accumulator(
            allocator,
            catalog,
            fiscal_policy.income_tax_rate,
            &demand_sink_rates_by_resource,
            job_funding,
        );
        let total_household_slots = building_accumulator.total_household_slots;
        let occupied_household_slots = building_accumulator.occupied_household_slots;
        let under_construction_household_slots =
            building_accumulator.under_construction_household_slots;
        let open_job_slots = building_accumulator.open_job_slots;
        let open_job_wage_sum = building_accumulator.open_job_wage_sum;
        let physical_worker_capacity = building_accumulator.physical_worker_capacity;
        let funded_worker_capacity = building_accumulator.funded_worker_capacity;
        let open_jobs_unfunded = building_accumulator.open_jobs_unfunded;
        let committed_commercial_output_capacity_by_resource =
            building_accumulator.committed_commercial_output_capacity_by_resource;
        let business_input_need_by_resource = building_accumulator.business_input_need_by_resource;
        let committed_output_capacity_by_resource =
            building_accumulator.committed_output_capacity_by_resource;
        let existing_unemployed_member_count = household_accumulator
            .housed_unemployed_count
            .saturating_add(households.pending_arrivals.adults);

        let vacant_household_slots = total_household_slots.saturating_sub(occupied_household_slots);
        let live_child_count = household_accumulator
            .live_child_count
            .saturating_add(households.pending_arrivals.children);
        let live_elder_count = household_accumulator
            .live_elder_count
            .saturating_add(households.pending_arrivals.elders);
        let housed_household_count = household_accumulator.housed_household_count;
        let unhoused_household_count = household_accumulator.unhoused_household_count;
        let zero_budget_household_count = household_accumulator.zero_budget_household_count;
        let persistent_exit_eligible_household_count =
            household_accumulator.persistent_exit_eligible_household_count;
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
        let mut output_absorption = OutputAbsorptionContext::from_resource_amounts(
            catalog.resource_count(),
            &committed_output_capacity_by_resource,
            &demand_sink_rates_by_resource,
            0,
            &business_input_need_by_resource,
        );
        let affordable_residents = affordable_consumer_residents(
            &households.households,
            allocator,
            catalog,
            tuning,
            fiscal_policy,
            treasury_balance,
        );
        for &(resource, rate) in &demand_sink_rates_by_resource {
            output_absorption
                .add_demand_units(resource, (f64::from(rate) * affordable_residents) as f32);
        }

        let export_expansion_available =
            connected_border_count > 0 && tuning.owa_export_price_multiplier > EPSILON;
        output_absorption.set_export_expansion_available(export_expansion_available);

        let total_household_count = housed_household_count
            .saturating_add(unhoused_household_count)
            .saturating_add(households.pending_arrivals.households);
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
            let committed_capacity = resource_amount(
                &committed_commercial_output_capacity_by_resource,
                resource_runtime_id,
            );
            let committed_gap = (consumer_demand - committed_capacity).max(0.0);
            add_resource_amount(
                &mut committed_unmet_commercial_consumer_demand_by_resource,
                resource_runtime_id,
                committed_gap,
            );
        }
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
        let net_open_job_slots = open_job_slots.saturating_sub(existing_unemployed_member_count);
        let net_open_job_wage_sum = if open_job_slots == 0 {
            0.0
        } else {
            open_job_wage_sum * net_open_job_slots as f32 / open_job_slots as f32
        };
        // A household cannot finance its arrival with jobs its own future spending might
        // create. Only existing open jobs enter admission's finite recurring-income budget.
        // Housing need converts them into homes with admission's applicant stream.
        let move_in_job_slots = net_open_job_slots;
        let move_in_job_equivalent_slots = net_open_job_slots as f32;
        let average_move_in_job_wage_per_day = if move_in_job_equivalent_slots <= EPSILON {
            0.0
        } else {
            net_open_job_wage_sum / move_in_job_equivalent_slots
        };

        debug_log!(
            "spawn",
            "daily_snapshot: border_nodes={} homes={} \
             live_jobs={} available_jobs={} pending_homes={}",
            connected_border_count,
            vacant_household_slots,
            open_job_slots,
            move_in_job_slots,
            under_construction_household_slots,
        );

        Self {
            // Indexed parallel collection preserves household priority. Only displaced
            // households need completion-time matching; the ordinary housed case is empty.
            unhoused_workforce: households
                .households
                .par_iter()
                .filter_map(|household| {
                    (household.member_count > 0
                        && (household.adult_count > 0 || household.elder_count > 0)
                        && !household_is_housed(household, allocator))
                    .then(|| UnhousedWorkforce {
                        reserve_days: household_reserve_days(catalog, tuning, household),
                        workers: u32::from(
                            household
                                .adult_count
                                .saturating_sub(household.employed_adults),
                        ),
                    })
                })
                .collect(),
            vacant_household_slots,
            total_household_count,
            housed_household_count,
            unhoused_household_count,
            zero_budget_household_count,
            persistent_exit_eligible_household_count,
            unhoused_household_ratio,
            zero_budget_household_ratio,
            committed_unmet_commercial_consumer_demand_by_resource,
            under_construction_household_slots,
            constructing_by_use: building_accumulator.constructing_by_use,
            queued_by_use: [0; 3],
            connected_border_count,
            city_treasury_balance: treasury_balance as f32,
            daily_essential_cost_per_resident,
            daily_home_electricity_cost,
            unemployment_daily_benefit_per_adult: fiscal_policy
                .unemployment_benefit_per_adult_per_day,
            pension_daily_benefit_per_elder: fiscal_policy.pension_per_elder_per_day,
            child_support_daily_benefit_per_child: fiscal_policy.child_support_per_child_per_day,
            existing_unemployed_member_count,
            existing_child_count: live_child_count,
            existing_elder_count: live_elder_count,
            open_job_slots,
            move_in_job_slots,
            move_in_job_equivalent_slots,
            average_move_in_job_wage_per_day,
            physical_worker_capacity,
            funded_worker_capacity,
            open_jobs_unfunded,
            output_absorption,
        }
    }
}

const BUILDING_SNAPSHOT_CHUNK_SIZE: usize = 1024;
const HOUSEHOLD_SNAPSHOT_CHUNK_SIZE: usize = 2048;
#[derive(Default)]
struct BuildingSnapshotAccumulator {
    total_household_slots: u32,
    occupied_household_slots: u32,
    under_construction_household_slots: u32,
    constructing_by_use: [u32; 3],
    open_job_slots: u32,
    open_job_wage_sum: f32,
    physical_worker_capacity: u32,
    funded_worker_capacity: u32,
    open_jobs_unfunded: u32,
    committed_output_capacity_by_resource: Vec<(ResourceRuntimeId, f32)>,
    committed_commercial_output_capacity_by_resource: Vec<(ResourceRuntimeId, f32)>,
    business_input_need_by_resource: Vec<(ResourceRuntimeId, f32)>,
}

impl BuildingSnapshotAccumulator {
    fn absorb_building(
        &mut self,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        income_tax_rate: f32,
        demand_sink_rates_by_resource: &[(ResourceRuntimeId, f32)],
        job_funding: &JobFunding,
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
        let active_profile = catalog.profile_by_runtime_id(building.economy_profile_runtime_id);

        if building.is_under_construction() {
            if is_private_building {
                if let Some(index) = super::actions::private_zone_index(building.zone_type) {
                    self.constructing_by_use[index] += 1;
                }
            }
            self.under_construction_household_slots = self
                .under_construction_household_slots
                .saturating_add(allocator.registry.household_capacity(&building.asset_id));
            if let Some(profile) = active_profile {
                let scale = output_area_scale_for_building(building, profile);
                for output_port in &profile.outputs {
                    let net_output = profile.net_output_units_per_day(output_port) * scale;
                    add_resource_amount(
                        &mut self.committed_output_capacity_by_resource,
                        output_port.resource_runtime_id,
                        net_output,
                    );
                    if building.zone_type == ZoneType::Commercial
                        && resource_amount(
                            demand_sink_rates_by_resource,
                            output_port.resource_runtime_id,
                        ) > 0.0
                    {
                        add_resource_amount(
                            &mut self.committed_commercial_output_capacity_by_resource,
                            output_port.resource_runtime_id,
                            net_output,
                        );
                    }
                }
            }
            return;
        }

        let household_capacity = allocator.household_capacity(idx);
        if household_capacity > 0 {
            self.total_household_slots = self
                .total_household_slots
                .saturating_add(household_capacity);
            let occupied = building.occupancy.min(household_capacity);
            self.occupied_household_slots = self.occupied_household_slots.saturating_add(occupied);
        }

        if let Some(profile) =
            active_profile.filter(|profile| profile_offers_work(building, profile))
        {
            // A funding change does not end an employment assignment. Releasing these
            // workers early lets another project reserve them while their employer still
            // reserves its customers (e.g. before the daily bankruptcy/layoff boundary).
            let filled_workers = building.worker_count;
            let physical_worker_capacity = active_worker_capacity_for_profile_with_floor_scale(
                catalog,
                building,
                profile,
                // Stores and explicit work areas cache their activity; a store's is its census.
                building.commercial_activity_floor_scale,
            );
            let funded_worker_capacity =
                physical_worker_capacity.min(job_funding.capacities.get(idx).copied().unwrap_or(0));
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
                if average_daily_wage > 0.1 {
                    let open_slots = funded_worker_capacity.saturating_sub(filled_workers);
                    let net_daily_wage =
                        average_daily_wage - tax_amount(average_daily_wage, income_tax_rate);
                    self.open_job_slots = self.open_job_slots.saturating_add(open_slots);
                    self.open_job_wage_sum += open_slots as f32 * net_daily_wage.max(0.0);
                }
            }
        }

        if let Some(profile) = active_profile {
            let commercial = building.zone_type == ZoneType::Commercial;
            for output_port in &profile.outputs {
                // Completion, staff searches and restocking do not release this firm's
                // customers to new construction. Hiring uses the same physical net capacity.
                let committed_units = profile.net_output_units_per_day(output_port)
                    * output_area_scale_for_building(building, profile);
                add_resource_amount(
                    &mut self.committed_output_capacity_by_resource,
                    output_port.resource_runtime_id,
                    committed_units,
                );
                if commercial {
                    if resource_amount(
                        demand_sink_rates_by_resource,
                        output_port.resource_runtime_id,
                    ) > 0.0
                    {
                        add_resource_amount(
                            &mut self.committed_commercial_output_capacity_by_resource,
                            output_port.resource_runtime_id,
                            committed_units,
                        );
                    }
                }
            }
            let input_capacity_scale = job_funding.input_demand_scale(
                idx,
                building,
                profile,
                allocator.is_city_service_building(building),
            );
            for input_port in &profile.inputs {
                // Maintenance supplied by this recipe is already excluded from net output.
                // It cannot also claim another supplier's customers.
                let internal = profile
                    .output_port(input_port.resource_runtime_id)
                    .map_or(0.0, |port| port.units_per_day);
                let input_units_per_day =
                    (input_port.units_per_day - internal).max(0.0) * input_capacity_scale;
                add_resource_amount(
                    &mut self.business_input_need_by_resource,
                    input_port.resource_runtime_id,
                    input_units_per_day,
                );
            }
        }
    }

    fn merge(&mut self, other: Self) {
        for (left, right) in self
            .constructing_by_use
            .iter_mut()
            .zip(other.constructing_by_use)
        {
            *left += right;
        }
        self.total_household_slots = self
            .total_household_slots
            .saturating_add(other.total_household_slots);
        self.occupied_household_slots = self
            .occupied_household_slots
            .saturating_add(other.occupied_household_slots);
        self.under_construction_household_slots = self
            .under_construction_household_slots
            .saturating_add(other.under_construction_household_slots);
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
        merge_resource_amounts(
            &mut self.committed_output_capacity_by_resource,
            other.committed_output_capacity_by_resource,
        );
        merge_resource_amounts(
            &mut self.committed_commercial_output_capacity_by_resource,
            other.committed_commercial_output_capacity_by_resource,
        );
        merge_resource_amounts(
            &mut self.business_input_need_by_resource,
            other.business_input_need_by_resource,
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

fn collect_building_snapshot_accumulator(
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    income_tax_rate: f32,
    demand_sink_rates_by_resource: &[(ResourceRuntimeId, f32)],
    job_funding: &JobFunding,
) -> BuildingSnapshotAccumulator {
    ordered_fold(
        &allocator.buildings,
        BUILDING_SNAPSHOT_CHUNK_SIZE,
        BuildingSnapshotAccumulator::default,
        |accumulator, idx, building| {
            accumulator.absorb_building(
                allocator,
                catalog,
                income_tax_rate,
                demand_sink_rates_by_resource,
                job_funding,
                idx,
                building,
            );
        },
        |mut merged, accumulator| {
            merged.merge(accumulator);
            merged
        },
    )
}

#[derive(Default)]
struct HouseholdSnapshotAccumulator {
    housed_resident_count: u32,
    housed_unemployed_count: u32,
    live_child_count: u32,
    live_elder_count: u32,
    housed_household_count: u32,
    unhoused_household_count: u32,
    zero_budget_household_count: u32,
    persistent_exit_eligible_household_count: u32,
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
        if household_is_housed(household, allocator) {
            self.housed_household_count = self.housed_household_count.saturating_add(1);
            self.housed_resident_count = self
                .housed_resident_count
                .saturating_add(household.member_count as u32);
            self.housed_unemployed_count = self.housed_unemployed_count.saturating_add(u32::from(
                household
                    .adult_count
                    .saturating_sub(household.employed_adults),
            ));
        } else {
            self.unhoused_household_count = self.unhoused_household_count.saturating_add(1);
            if config
                .household_action
                .exit_policy
                .is_eligible(household, allocator, catalog, tuning)
            {
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
        self.housed_unemployed_count = self
            .housed_unemployed_count
            .saturating_add(other.housed_unemployed_count);
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
    }
}

fn collect_household_snapshot_accumulator(
    allocator: &BuildingAllocator,
    households: &HouseholdSystem,
    catalog: &RuntimeEconomyCatalog,
    tuning: &RuntimeEconomyTuning,
    config: &DemandConfig,
) -> HouseholdSnapshotAccumulator {
    ordered_fold(
        &households.households,
        HOUSEHOLD_SNAPSHOT_CHUNK_SIZE,
        HouseholdSnapshotAccumulator::default,
        |accumulator, _, household| {
            accumulator.absorb_household(allocator, catalog, tuning, config, household);
        },
        |mut merged, accumulator| {
            merged.merge(accumulator);
            merged
        },
    )
}

/// Reuses the household census for daily exits without evaluating unrelated business markets.
pub(super) fn collect_household_removal_diagnostics(
    allocator: &BuildingAllocator,
    households: &HouseholdSystem,
    catalog: &RuntimeEconomyCatalog,
    tuning: &RuntimeEconomyTuning,
    config: &DemandConfig,
) -> super::diagnostics::HouseholdRemovalDiagnostics {
    let counts =
        collect_household_snapshot_accumulator(allocator, households, catalog, tuning, config);
    let total = counts
        .housed_household_count
        .saturating_add(counts.unhoused_household_count)
        .saturating_add(households.pending_arrivals.households);
    let ratio = |count| {
        if total == 0 {
            0.0
        } else {
            clamp01(count as f32 / total as f32)
        }
    };
    super::diagnostics::HouseholdRemovalDiagnostics {
        total_household_count: total,
        housed_household_count: counts.housed_household_count,
        unhoused_household_count: counts.unhoused_household_count,
        zero_budget_household_count: counts.zero_budget_household_count,
        persistent_exit_eligible_household_count: counts.persistent_exit_eligible_household_count,
        unhoused_household_ratio: ratio(counts.unhoused_household_count),
        zero_budget_household_ratio: ratio(counts.zero_budget_household_count),
        ..Default::default()
    }
}
