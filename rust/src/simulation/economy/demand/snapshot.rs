//! Settled economy snapshots consumed by demand pressure and planning.

use super::config::DemandConfig;
use super::credits::clamp01;
use super::spawn_need::{add_resource_amount, resource_amount, resource_is_commercial_input};
use super::types::EPSILON;
use crate::assets::ZoneClass;
use crate::debug_log;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::economy::definitions::{
    EconomyProfileRuntimeKind, ResourceRuntimeId, RuntimeEconomyCatalog, RuntimeEconomyTuning,
};
#[cfg(test)]
use crate::simulation::economy::definitions::{
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::economy::households::{Household, HouseholdSystem, household_reserve_days};
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
        Self::from_runtime_with_catalog(allocator, households, catalog.as_ref(), tuning.as_ref())
    }

    pub(super) fn from_runtime_with_catalog(
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
    ) -> Self {
        let household_count_by_building: Vec<_> = (0..allocator.buildings.len())
            .map(|_| AtomicU32::new(0))
            .collect();
        let min_reserve_days_by_building: Vec<_> = (0..allocator.buildings.len())
            .map(|_| AtomicU32::new(f32::INFINITY.to_bits()))
            .collect();

        households.households.par_iter().for_each(|household| {
            if household.member_count == 0 {
                return;
            }
            let home_building_id = household.home_building_id;
            if home_building_id >= allocator.buildings.len()
                || allocator.buildings[home_building_id].broken
                || allocator.buildings[home_building_id].economy_broken
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
                .into_iter()
                .map(|count| count.load(Ordering::Relaxed))
                .collect(),
            min_reserve_days_by_building: min_reserve_days_by_building
                .into_iter()
                .map(|reserve| f32::from_bits(reserve.load(Ordering::Relaxed)))
                .collect(),
        }
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
    pub(super) household_affordability: f32,
    pub(super) household_stock_stability: f32,
    pub(super) commercial_capacity_deficit: f32,
    pub(super) unmet_commercial_consumer_demand: f32,
    pub(super) industrial_input_capacity_deficit: f32,
    #[cfg(test)]
    pub(super) commercial_input_need_value: f32,
    #[cfg(test)]
    pub(super) local_industrial_input_capacity_value: f32,
    pub(super) industrial_missing_input_value: f32,
    pub(super) external_connection_available: f32,
    pub(super) connected_border_count: u32,
    pub(super) city_treasury_balance: f32,
    pub(super) candidate_household_size: f32,
    pub(super) immigrant_starter_savings_per_household: f32,
    pub(super) candidate_daily_essential_cost: f32,
    pub(super) unemployment_daily_benefit_per_member: f32,
    pub(super) existing_unemployed_member_count: u32,
    pub(super) open_job_slots: u32,
    pub(super) average_open_job_wage_per_day: f32,
    #[cfg(test)]
    // Fraction of commercial input value sourced from OWA rather than local industrial.
    pub(super) commercial_owa_dependency: f32,
    #[cfg(test)]
    pub(super) commercial_owa_input_value: f32,
    // Raw counts needed for non-residential spawn gates.
    pub(super) housed_resident_count: u32,
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
        Self::from_runtime_with_catalog(
            allocator,
            households,
            graph,
            config,
            catalog.as_ref(),
            tuning.as_ref(),
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
        treasury_balance: f64,
    ) -> Self {
        let mut commercial_profile_output_resources = Vec::new();
        for profile in catalog.all_profiles() {
            if profile.kind != EconomyProfileRuntimeKind::Store {
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
        let building_accumulator = collect_building_snapshot_accumulator(
            allocator,
            catalog,
            &demand_sink_rates_by_resource,
        );
        let total_household_slots = building_accumulator.total_household_slots;
        let occupied_household_slots = building_accumulator.occupied_household_slots;
        let existing_private_building_count = building_accumulator.existing_private_building_count;
        let total_commercial_owa_input = building_accumulator.total_commercial_owa_input;
        let total_commercial_local_input = building_accumulator.total_commercial_local_input;
        let total_commercial_expected_input = building_accumulator.total_commercial_expected_input;
        let candidate_household_size_sum = building_accumulator.candidate_household_size_sum;
        let candidate_household_slot_count = building_accumulator.candidate_household_slot_count;
        let filled_job_count = building_accumulator.filled_job_count;
        let open_job_slots = building_accumulator.open_job_slots;
        let open_job_wage_sum = building_accumulator.open_job_wage_sum;
        let commercial_output_capacity_by_resource =
            building_accumulator.commercial_output_capacity_by_resource;
        let commercial_input_need_by_resource =
            building_accumulator.commercial_input_need_by_resource;
        let local_industrial_output_capacity_by_resource =
            building_accumulator.local_industrial_output_capacity_by_resource;

        let vacant_household_slots = total_household_slots.saturating_sub(occupied_household_slots);
        let candidate_household_size = if candidate_household_slot_count == 0 {
            construction_candidate_household_size_from_registry(allocator)
        } else {
            candidate_household_size_sum / candidate_household_slot_count as f32
        };
        let immigrant_starter_savings_per_household =
            candidate_household_size * tuning.households.immigrant_starting_budget_per_member;
        let candidate_daily_essential_cost =
            candidate_household_size * daily_essential_cost_per_resident;
        let average_open_job_wage_per_day = if open_job_slots == 0 {
            0.0
        } else {
            open_job_wage_sum / open_job_slots as f32
        };

        let household_accumulator =
            collect_household_snapshot_accumulator(allocator, households, catalog, tuning, config);
        let housed_resident_count = household_accumulator.housed_resident_count;
        let housed_household_count = household_accumulator.housed_household_count;
        let unhoused_household_count = household_accumulator.unhoused_household_count;
        let zero_budget_household_count = household_accumulator.zero_budget_household_count;
        let persistent_exit_eligible_household_count =
            household_accumulator.persistent_exit_eligible_household_count;
        let household_affordability_sum = household_accumulator.household_affordability_sum;
        let household_stock_stability_sum = household_accumulator.household_stock_stability_sum;

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
        for &(resource_runtime_id, consumption_rate_per_resident) in &demand_sink_rates_by_resource
        {
            let consumer_demand = consumption_rate_per_resident * housed_resident_count as f32;
            if consumer_demand <= 0.0 {
                continue;
            }
            let placed_capacity =
                resource_amount(&commercial_output_capacity_by_resource, resource_runtime_id);
            total_commercial_consumer_demand += consumer_demand;
            unmet_commercial_consumer_demand += (consumer_demand - placed_capacity).max(0.0);
        }
        let commercial_capacity_deficit = if total_commercial_consumer_demand <= 0.0 {
            0.0
        } else {
            clamp01(unmet_commercial_consumer_demand / total_commercial_consumer_demand)
        };
        let mut commercial_input_need_value = 0.0_f32;
        let mut local_industrial_input_capacity_value = 0.0_f32;
        let mut industrial_missing_input_value = 0.0_f32;
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
            let local_units = resource_amount(
                &local_industrial_output_capacity_by_resource,
                resource_runtime_id,
            );
            commercial_input_need_value += need_units.max(0.0) * resource_price.max(0.0);
            local_industrial_input_capacity_value += local_units.max(0.0) * resource_price.max(0.0);
            industrial_missing_input_value +=
                (need_units - local_units).max(0.0) * resource_price.max(0.0);
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
        let external_connection_available = if connected_border_count > 0 { 1.0 } else { 0.0 };
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
        let existing_unemployed_member_count =
            housed_resident_count.saturating_sub(filled_job_count);
        let candidate_effective_workers = candidate_household_size.max(1.0);
        let open_job_household_pull = open_job_slots as f32 / candidate_effective_workers;
        let bootstrap_household_pull = if total_household_count == 0 { 1.0 } else { 0.0 };
        let incoming_household_need = open_job_household_pull.max(bootstrap_household_pull);

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
             incoming_need={:.2} job_pull={:.2} com_cap_def={:.2} unmet_com_units={:.1} \
             ind_cap_def={:.2} com_input_need={:.1} local_ind_capacity={:.1} \
             ind_missing={:.1} owa_dep={:.2} owa_input_value={:.1} \
             treasury={:.0} cand_size={:.1} \
             open_jobs={} existing_unemployed={} \
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
            commercial_capacity_deficit,
            unmet_commercial_consumer_demand,
            industrial_input_capacity_deficit,
            commercial_input_need_value,
            local_industrial_input_capacity_value,
            industrial_missing_input_value,
            commercial_owa_dependency,
            total_commercial_owa_input,
            treasury_balance,
            candidate_household_size,
            open_job_slots,
            existing_unemployed_member_count,
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
            household_affordability,
            household_stock_stability,
            commercial_capacity_deficit,
            unmet_commercial_consumer_demand,
            industrial_input_capacity_deficit,
            #[cfg(test)]
            commercial_input_need_value,
            #[cfg(test)]
            local_industrial_input_capacity_value,
            industrial_missing_input_value,
            external_connection_available,
            connected_border_count,
            city_treasury_balance: treasury_balance as f32,
            candidate_household_size,
            immigrant_starter_savings_per_household,
            candidate_daily_essential_cost,
            unemployment_daily_benefit_per_member: tuning.unemployment_daily_benefit_per_member,
            existing_unemployed_member_count,
            open_job_slots,
            average_open_job_wage_per_day,
            #[cfg(test)]
            commercial_owa_dependency,
            #[cfg(test)]
            commercial_owa_input_value: total_commercial_owa_input,
            housed_resident_count,
        }
    }
}

const BUILDING_SNAPSHOT_CHUNK_SIZE: usize = 1024;
const HOUSEHOLD_SNAPSHOT_CHUNK_SIZE: usize = 2048;

#[derive(Default)]
struct BuildingSnapshotAccumulator {
    total_household_slots: u32,
    occupied_household_slots: u32,
    existing_private_building_count: u32,
    total_commercial_owa_input: f32,
    total_commercial_local_input: f32,
    total_commercial_expected_input: f32,
    candidate_household_size_sum: f32,
    candidate_household_slot_count: u32,
    filled_job_count: u32,
    open_job_slots: u32,
    open_job_wage_sum: f32,
    commercial_output_capacity_by_resource: Vec<(ResourceRuntimeId, f32)>,
    commercial_input_need_by_resource: Vec<(ResourceRuntimeId, f32)>,
    local_industrial_output_capacity_by_resource: Vec<(ResourceRuntimeId, f32)>,
}

impl BuildingSnapshotAccumulator {
    fn absorb_building(
        &mut self,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        demand_sink_rates_by_resource: &[(u16, f32)],
        idx: usize,
        building: &Building,
    ) {
        if building.broken || building.economy_broken {
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

        if matches!(building.zone_type, ZoneType::Residential) {
            let household_capacity = allocator.household_capacity(idx);
            self.total_household_slots = self
                .total_household_slots
                .saturating_add(household_capacity);
            let occupied = building.occupancy.min(household_capacity);
            self.occupied_household_slots = self.occupied_household_slots.saturating_add(occupied);
            let free_slots = household_capacity.saturating_sub(occupied);
            if free_slots > 0 {
                let candidate_size =
                    candidate_household_size_from_flat_size(allocator.flat_size_m2(idx));
                self.candidate_household_size_sum += candidate_size as f32 * free_slots as f32;
                self.candidate_household_slot_count = self
                    .candidate_household_slot_count
                    .saturating_add(free_slots);
            }
        }

        if !building.is_deserted
            && matches!(
                building.zone_type,
                ZoneType::Commercial | ZoneType::Industrial
            )
        {
            let worker_capacity = allocator.worker_capacity(idx);
            if worker_capacity > 0 {
                let average_daily_wage = catalog
                    .profile_by_runtime_id(building.economy_profile_runtime_id)
                    .map(|profile| profile.average_daily_wage())
                    .unwrap_or(0.0);
                let filled_workers = building.worker_count.min(worker_capacity);
                self.filled_job_count = self.filled_job_count.saturating_add(filled_workers);
                if average_daily_wage > 0.1 {
                    let budget_capacity =
                        (building.operating_budget.max(0.0) / average_daily_wage).floor() as u32;
                    let effective_capacity = worker_capacity.min(budget_capacity);
                    let open_slots = effective_capacity.saturating_sub(filled_workers);
                    self.open_job_slots = self.open_job_slots.saturating_add(open_slots);
                    self.open_job_wage_sum += open_slots as f32 * average_daily_wage.max(0.0);
                }
            }
        }

        if !building.is_deserted && matches!(building.zone_type, ZoneType::Commercial) {
            self.total_commercial_owa_input += building.daily_owa_input_value;
            self.total_commercial_local_input += building.daily_local_input_value;
            if let Some(profile) =
                catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            {
                for output_port in &profile.outputs {
                    if resource_amount(
                        demand_sink_rates_by_resource,
                        output_port.resource_runtime_id,
                    ) > 0.0
                    {
                        add_resource_amount(
                            &mut self.commercial_output_capacity_by_resource,
                            output_port.resource_runtime_id,
                            output_port.units_per_day,
                        );
                    }
                }
                for input_port in &profile.inputs {
                    add_resource_amount(
                        &mut self.commercial_input_need_by_resource,
                        input_port.resource_runtime_id,
                        input_port.units_per_day,
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
                    self.total_commercial_expected_input +=
                        input_port.units_per_day * resource_price;
                }
            }
        }

        if !building.is_deserted && matches!(building.zone_type, ZoneType::Industrial) {
            if let Some(profile) =
                catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            {
                for output_port in &profile.outputs {
                    if resource_is_commercial_input(catalog, output_port.resource_runtime_id) {
                        add_resource_amount(
                            &mut self.local_industrial_output_capacity_by_resource,
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
        self.candidate_household_size_sum += other.candidate_household_size_sum;
        self.candidate_household_slot_count = self
            .candidate_household_slot_count
            .saturating_add(other.candidate_household_slot_count);
        self.filled_job_count = self.filled_job_count.saturating_add(other.filled_job_count);
        self.open_job_slots = self.open_job_slots.saturating_add(other.open_job_slots);
        self.open_job_wage_sum += other.open_job_wage_sum;
        merge_resource_amounts(
            &mut self.commercial_output_capacity_by_resource,
            other.commercial_output_capacity_by_resource,
        );
        merge_resource_amounts(
            &mut self.commercial_input_need_by_resource,
            other.commercial_input_need_by_resource,
        );
        merge_resource_amounts(
            &mut self.local_industrial_output_capacity_by_resource,
            other.local_industrial_output_capacity_by_resource,
        );
    }
}

fn collect_building_snapshot_accumulator(
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    demand_sink_rates_by_resource: &[(ResourceRuntimeId, f32)],
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
                    demand_sink_rates_by_resource,
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
        let is_housed = household.home_building_id < allocator.buildings.len()
            && !allocator.buildings[household.home_building_id].broken
            && !allocator.buildings[household.home_building_id].economy_broken;
        if is_housed {
            self.housed_household_count = self.housed_household_count.saturating_add(1);
            self.housed_resident_count = self
                .housed_resident_count
                .saturating_add(household.member_count as u32);
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

fn candidate_household_size_from_flat_size(flat_size_m2: f32) -> u16 {
    if flat_size_m2 > 1.0 {
        ((flat_size_m2 / 40.0).ceil() as u16).clamp(1, 5)
    } else {
        2
    }
}

fn construction_candidate_household_size_from_registry(allocator: &BuildingAllocator) -> f32 {
    let mut candidate_size_sum = 0.0_f32;
    let mut candidate_count = 0_u32;
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
        candidate_size_sum +=
            candidate_household_size_from_flat_size(allocator.registry.flat_size_m2(asset_id))
                as f32;
        candidate_count = candidate_count.saturating_add(1);
    }
    if candidate_count == 0 {
        0.0
    } else {
        candidate_size_sum / candidate_count as f32
    }
}
