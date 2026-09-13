// SPDX-License-Identifier: GPL-2.0-only

//! Demand spawn-need accounting and hard spawn gates.

use super::actions::DemandSpawnCandidate;
use super::snapshot::DailyDemandSnapshot;
use super::types::{DemandUse, EPSILON, RESIDENTIAL_SPAWN_VACANT_SLOT_RESERVE_RATIO};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, ResourceRuntimeId, RuntimeEconomyCatalog,
};

use crate::simulation::economy::resource_totals::{add_resource_amount, resource_amount};

fn add_resource_count(
    counts: &mut Vec<(ResourceRuntimeId, u32)>,
    resource_runtime_id: ResourceRuntimeId,
) {
    if let Some((_, existing)) = counts
        .iter_mut()
        .find(|(resource, _)| *resource == resource_runtime_id)
    {
        *existing = existing.saturating_add(1);
    } else {
        counts.push((resource_runtime_id, 1));
    }
}

fn resource_count(
    counts: &[(ResourceRuntimeId, u32)],
    resource_runtime_id: ResourceRuntimeId,
) -> u32 {
    counts
        .iter()
        .find_map(|(resource, count)| (*resource == resource_runtime_id).then_some(*count))
        .unwrap_or(0)
}

pub(super) fn spawn_need_buildings_for_use(
    use_kind: DemandUse,
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    snapshot: &DailyDemandSnapshot,
    candidates: &[DemandSpawnCandidate],
) -> f32 {
    if candidates.is_empty() {
        return 0.0;
    }
    match use_kind {
        DemandUse::Residential => residential_spawn_need_buildings(allocator, snapshot, candidates),
        DemandUse::Commercial => {
            commercial_spawn_need_buildings(allocator, catalog, snapshot, candidates)
        }
        DemandUse::Industrial => {
            industrial_spawn_need_buildings(allocator, catalog, snapshot, candidates)
        }
    }
}

pub(super) fn residential_spawn_need_buildings(
    allocator: &BuildingAllocator,
    snapshot: &DailyDemandSnapshot,
    candidates: &[DemandSpawnCandidate],
) -> f32 {
    let incoming_slots = snapshot.incoming_household_need.ceil();
    let reserve_slots = if snapshot.total_household_count == 0 {
        0.0
    } else {
        (snapshot.total_household_count as f32 * RESIDENTIAL_SPAWN_VACANT_SLOT_RESERVE_RATIO)
            .ceil()
            .max(1.0)
    };
    let desired_vacant_slots = incoming_slots + reserve_slots;
    let committed_slots =
        snapshot.vacant_household_slots + snapshot.under_construction_household_slots;
    let missing_household_slots = (desired_vacant_slots - committed_slots as f32).max(0.0);
    if missing_household_slots <= EPSILON {
        return 0.0;
    }
    let average_household_slots =
        average_residential_candidate_household_slots(allocator, candidates);
    if average_household_slots <= EPSILON {
        0.0
    } else {
        (missing_household_slots / average_household_slots).ceil()
    }
}

pub(super) fn commercial_spawn_need_buildings(
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    snapshot: &DailyDemandSnapshot,
    candidates: &[DemandSpawnCandidate],
) -> f32 {
    if snapshot
        .committed_unmet_commercial_consumer_demand_by_resource
        .is_empty()
    {
        return 0.0;
    }
    let average_output_units_by_resource =
        average_candidate_output_units_by_resource_for_household_demand(
            allocator, catalog, candidates,
        );
    snapshot
        .committed_unmet_commercial_consumer_demand_by_resource
        .iter()
        .filter_map(|&(resource_runtime_id, unmet_units)| {
            let output_units =
                resource_amount(&average_output_units_by_resource, resource_runtime_id);
            (unmet_units > EPSILON && output_units > EPSILON)
                .then(|| (unmet_units / output_units).ceil())
        })
        .fold(0.0, f32::max)
}

pub(super) fn industrial_spawn_need_buildings(
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    snapshot: &DailyDemandSnapshot,
    candidates: &[DemandSpawnCandidate],
) -> f32 {
    candidates
        .iter()
        .filter_map(|candidate| {
            candidate_economy_profile(allocator, catalog, &candidate.action.asset_id)
        })
        .map(|profile| {
            profile
                .outputs
                .iter()
                .map(|port| {
                    let net_output = profile.net_output_units_per_day(port);
                    let gap = snapshot
                        .output_absorption
                        .unmet_units(port.resource_runtime_id);
                    if net_output > EPSILON {
                        (gap / net_output).ceil()
                    } else {
                        0.0
                    }
                })
                .fold(0.0, f32::max)
        })
        .fold(0.0, f32::max)
}

fn average_residential_candidate_household_slots(
    allocator: &BuildingAllocator,
    candidates: &[DemandSpawnCandidate],
) -> f32 {
    let mut total_slots = 0.0_f32;
    let mut candidate_count = 0_u32;
    for candidate in candidates {
        let capacity = allocator
            .registry
            .household_capacity(&candidate.action.asset_id);
        if capacity == 0 {
            continue;
        }
        total_slots += capacity as f32;
        candidate_count = candidate_count.saturating_add(1);
    }
    if candidate_count == 0 {
        0.0
    } else {
        total_slots / candidate_count as f32
    }
}

fn average_candidate_output_units_by_resource_for_household_demand(
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    candidates: &[DemandSpawnCandidate],
) -> Vec<(ResourceRuntimeId, f32)> {
    let mut total_output_units_by_resource = Vec::new();
    let mut output_candidate_count_by_resource = Vec::new();
    for candidate in candidates {
        let Some(profile) =
            candidate_economy_profile(allocator, catalog, &candidate.action.asset_id)
        else {
            continue;
        };
        for port in &profile.outputs {
            if !resource_has_household_demand(catalog, port.resource_runtime_id) {
                continue;
            }
            let units = port.units_per_day.max(0.0);
            if units <= EPSILON {
                continue;
            }
            add_resource_amount(
                &mut total_output_units_by_resource,
                port.resource_runtime_id,
                units,
            );
            add_resource_count(
                &mut output_candidate_count_by_resource,
                port.resource_runtime_id,
            );
        }
    }
    if output_candidate_count_by_resource.is_empty() {
        return Vec::new();
    }
    for (resource_runtime_id, units) in &mut total_output_units_by_resource {
        let divisor = resource_count(&output_candidate_count_by_resource, *resource_runtime_id);
        if divisor > 0 {
            *units /= divisor as f32;
        }
    }
    total_output_units_by_resource
}

pub(super) fn candidate_economy_profile<'a>(
    allocator: &BuildingAllocator,
    catalog: &'a RuntimeEconomyCatalog,
    asset_id: &str,
) -> Option<&'a EconomyProfileRuntime> {
    let profile_id = allocator.registry.economy_profile(asset_id)?;
    catalog.profile_for_id(profile_id)
}

pub(super) fn resource_has_household_demand(
    catalog: &RuntimeEconomyCatalog,
    resource_runtime_id: ResourceRuntimeId,
) -> bool {
    catalog.all_profiles().iter().any(|profile| {
        profile.kind == EconomyProfileRuntimeKind::DemandSink
            && profile.consumption_rate_per_resident > EPSILON
            && profile.input_port(resource_runtime_id).is_some()
    })
}

#[derive(Clone, Debug)]
pub(super) struct OutputAbsorptionContext {
    placed_output_units_by_resource: Vec<f32>,
    demand_units_by_resource: Vec<f32>,
}

impl OutputAbsorptionContext {
    pub(super) fn empty(resource_count: usize) -> Self {
        Self {
            placed_output_units_by_resource: vec![0.0; resource_count + 1],
            demand_units_by_resource: vec![0.0; resource_count + 1],
        }
    }

    pub(super) fn from_resource_amounts(
        resource_count: usize,
        placed_output_capacity_by_resource: &[(ResourceRuntimeId, f32)],
        resident_demand_rates_by_resource: &[(ResourceRuntimeId, f32)],
        housed_resident_count: u32,
        business_input_need_by_resource: &[(ResourceRuntimeId, f32)],
    ) -> Self {
        let mut context = Self::empty(resource_count);

        for &(resource_runtime_id, capacity) in placed_output_capacity_by_resource {
            context.add_placed_capacity(resource_runtime_id, capacity.max(0.0));
        }

        for &(resource_runtime_id, rate_per_resident) in resident_demand_rates_by_resource {
            context.add_demand_units(
                resource_runtime_id,
                rate_per_resident.max(0.0) * housed_resident_count as f32,
            );
        }

        for &(resource_runtime_id, need_units) in business_input_need_by_resource {
            context.add_demand_units(resource_runtime_id, need_units.max(0.0));
        }

        context
    }

    /// Daily demand remaining after existing and selected output capacity is reserved.
    pub(super) fn unmet_units(&self, resource: ResourceRuntimeId) -> f32 {
        (self.consumer_demand(resource) - self.placed_capacity(resource)).max(0.0)
    }

    /// Reserve selected output immediately; a new factory cannot justify its own customers.
    pub(super) fn reserve_candidate(&mut self, profile: &EconomyProfileRuntime) {
        for port in &profile.outputs {
            self.add_placed_capacity(
                port.resource_runtime_id,
                profile.net_output_units_per_day(port),
            );
        }
    }

    fn add_demand_units(&mut self, resource_runtime_id: ResourceRuntimeId, units: f32) {
        if let Some(slot) = self
            .demand_units_by_resource
            .get_mut(resource_runtime_id as usize)
        {
            *slot += units;
        }
    }

    fn add_placed_capacity(&mut self, resource_runtime_id: ResourceRuntimeId, capacity: f32) {
        if let Some(slot) = self
            .placed_output_units_by_resource
            .get_mut(resource_runtime_id as usize)
        {
            *slot += capacity;
        }
    }

    fn placed_capacity(&self, resource_runtime_id: ResourceRuntimeId) -> f32 {
        self.placed_output_units_by_resource
            .get(resource_runtime_id as usize)
            .copied()
            .unwrap_or(0.0)
    }

    fn consumer_demand(&self, resource_runtime_id: ResourceRuntimeId) -> f32 {
        self.demand_units_by_resource
            .get(resource_runtime_id as usize)
            .copied()
            .unwrap_or(0.0)
    }
}

pub(super) fn nonresidential_passes_absorption_gate(
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    absorption: &OutputAbsorptionContext,
    asset_id: &str,
) -> bool {
    let Some(profile_id) = allocator.registry.economy_profile(asset_id) else {
        return false;
    };
    let Some(candidate_profile) = catalog.profile_for_id(profile_id) else {
        return false;
    };
    candidate_profile.outputs.iter().any(|port| {
        candidate_profile.net_output_units_per_day(port) > EPSILON
            && absorption.unmet_units(port.resource_runtime_id) > EPSILON
    })
}
