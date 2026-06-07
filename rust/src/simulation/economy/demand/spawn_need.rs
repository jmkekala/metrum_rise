//! Demand spawn-need accounting and hard spawn gates.

use super::actions::DemandSpawnCandidate;
use super::snapshot::DailyDemandSnapshot;
use super::types::{DemandUse, EPSILON, RESIDENTIAL_SPAWN_VACANT_SLOT_RESERVE_RATIO};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, ResourceRuntimeId, RuntimeEconomyCatalog,
};

pub(super) fn add_resource_amount(
    amounts: &mut Vec<(ResourceRuntimeId, f32)>,
    resource_runtime_id: ResourceRuntimeId,
    amount: f32,
) {
    if amount <= 0.0 {
        return;
    }
    if let Some((_, existing)) = amounts
        .iter_mut()
        .find(|(resource, _)| *resource == resource_runtime_id)
    {
        *existing += amount;
    } else {
        amounts.push((resource_runtime_id, amount));
    }
}

pub(super) fn resource_amount(
    amounts: &[(ResourceRuntimeId, f32)],
    resource_runtime_id: ResourceRuntimeId,
) -> f32 {
    amounts
        .iter()
        .find_map(|(resource, amount)| (*resource == resource_runtime_id).then_some(*amount))
        .unwrap_or(0.0)
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
    let missing_household_slots =
        (desired_vacant_slots - snapshot.vacant_household_slots as f32).max(0.0);
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
    if snapshot.unmet_commercial_consumer_demand <= EPSILON {
        return 0.0;
    }
    let average_output_units =
        average_candidate_output_units_for_household_demand(allocator, catalog, candidates);
    if average_output_units <= EPSILON {
        0.0
    } else {
        (snapshot.unmet_commercial_consumer_demand / average_output_units).ceil()
    }
}

pub(super) fn industrial_spawn_need_buildings(
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    snapshot: &DailyDemandSnapshot,
    candidates: &[DemandSpawnCandidate],
) -> f32 {
    if snapshot.industrial_missing_input_value <= EPSILON {
        return 0.0;
    }
    let average_output_value =
        average_candidate_output_value_for_commercial_inputs(allocator, catalog, candidates);
    if average_output_value <= EPSILON {
        0.0
    } else {
        (snapshot.industrial_missing_input_value / average_output_value).ceil()
    }
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

fn average_candidate_output_units_for_household_demand(
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    candidates: &[DemandSpawnCandidate],
) -> f32 {
    let mut total_output_units = 0.0_f32;
    let mut candidate_count = 0_u32;
    for candidate in candidates {
        let Some(profile) =
            candidate_economy_profile(allocator, catalog, &candidate.action.asset_id)
        else {
            continue;
        };
        let output_units = profile
            .outputs
            .iter()
            .filter(|port| resource_has_household_demand(catalog, port.resource_runtime_id))
            .map(|port| port.units_per_day.max(0.0))
            .sum::<f32>();
        if output_units <= EPSILON {
            continue;
        }
        total_output_units += output_units;
        candidate_count = candidate_count.saturating_add(1);
    }
    if candidate_count == 0 {
        0.0
    } else {
        total_output_units / candidate_count as f32
    }
}

fn average_candidate_output_value_for_commercial_inputs(
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    candidates: &[DemandSpawnCandidate],
) -> f32 {
    let mut total_output_value = 0.0_f32;
    let mut candidate_count = 0_u32;
    for candidate in candidates {
        let Some(profile) =
            candidate_economy_profile(allocator, catalog, &candidate.action.asset_id)
        else {
            continue;
        };
        let output_value = profile
            .outputs
            .iter()
            .filter(|port| resource_is_commercial_input(catalog, port.resource_runtime_id))
            .map(|port| {
                let unit_price = catalog
                    .unit_price_for_resource(port.resource_runtime_id)
                    .unwrap_or(0.0);
                port.units_per_day.max(0.0) * unit_price.max(0.0)
            })
            .sum::<f32>();
        if output_value <= EPSILON {
            continue;
        }
        total_output_value += output_value;
        candidate_count = candidate_count.saturating_add(1);
    }
    if candidate_count == 0 {
        0.0
    } else {
        total_output_value / candidate_count as f32
    }
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

pub(super) fn resource_is_commercial_input(
    catalog: &RuntimeEconomyCatalog,
    resource_runtime_id: ResourceRuntimeId,
) -> bool {
    catalog.all_profiles().iter().any(|profile| {
        profile.kind == EconomyProfileRuntimeKind::Store
            && profile.input_port(resource_runtime_id).is_some()
    })
}

pub(super) fn nonresidential_passes_absorption_gate(
    allocator: &BuildingAllocator,
    catalog: &crate::simulation::economy::definitions::RuntimeEconomyCatalog,
    asset_id: &str,
    housed_resident_count: u32,
) -> bool {
    use crate::simulation::economy::definitions::EconomyProfileRuntimeKind;
    // Resolve the candidate profile from the asset registry.
    let Some(profile_id) = allocator.registry.economy_profile(asset_id) else {
        // No economy profile binding → no capacity limit, pass.
        return true;
    };
    let Some(candidate_profile) = catalog.profile_for_id(profile_id) else {
        return true;
    };
    // Buildings with no declared outputs are not capacity-limited.
    if candidate_profile.outputs.is_empty() {
        return true;
    }
    let candidate_output_resource_ids: Vec<_> = candidate_profile
        .outputs
        .iter()
        .map(|p| p.resource_runtime_id)
        .collect();

    // Sum output capacity (units/day) already placed for matching resource types.
    // Deserted buildings are excluded: they produce nothing and must not block a replacement spawn.
    let placed_capacity: f32 = allocator
        .buildings
        .iter()
        .filter(|b| !b.broken && !b.economy_broken && !b.is_deserted)
        .filter_map(|b| {
            let p = catalog.profile_by_runtime_id(b.economy_profile_runtime_id)?;
            let overlaps = p
                .outputs
                .iter()
                .any(|port| candidate_output_resource_ids.contains(&port.resource_runtime_id));
            if overlaps {
                Some(p.outputs.iter().map(|port| port.units_per_day).sum::<f32>())
            } else {
                None
            }
        })
        .sum();

    // Derive consumer demand from housed residents and demand-sink consumption rates.
    let consumer_demand: f32 = catalog
        .all_profiles()
        .iter()
        .filter(|p| p.kind == EconomyProfileRuntimeKind::DemandSink)
        .filter(|p| {
            p.inputs
                .iter()
                .any(|port| candidate_output_resource_ids.contains(&port.resource_runtime_id))
        })
        .map(|p| p.consumption_rate_per_resident * housed_resident_count as f32)
        .sum();

    // If no demand-sink found for this resource, gate is not applicable → pass.
    if consumer_demand == 0.0 {
        return true;
    }
    placed_capacity < consumer_demand
}
