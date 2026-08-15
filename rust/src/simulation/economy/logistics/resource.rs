//! Resource profile and freight timing lookup helpers.

use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, FreightTimingProfile, ResourceRuntimeId,
    RuntimeEconomyCatalog, RuntimeEconomyTuning,
};
use crate::simulation::zoning::ZoneType;

pub(super) fn required_unit_price(
    catalog: &RuntimeEconomyCatalog,
    resource_runtime_id: ResourceRuntimeId,
    profile_id: &str,
) -> f32 {
    catalog
        .unit_price_for_resource(resource_runtime_id)
        .unwrap_or_else(|| {
            let resource_id = catalog
                .resource_id_for_runtime_id(resource_runtime_id)
                .unwrap_or("<unknown>");
            panic!("resource '{resource_id}' used by profile '{profile_id}' has no catalog price")
        })
}

pub(super) fn freight_profile_for_building<'a>(
    catalog: &RuntimeEconomyCatalog,
    tuning: &'a RuntimeEconomyTuning,
    building: &Building,
) -> Option<&'a FreightTimingProfile> {
    if let Some(profile_id) = catalog
        .profile_by_runtime_id(building.economy_profile_runtime_id)
        .and_then(|profile| profile.freight_timing_profile.as_deref())
        && let Some(profile) = tuning
            .operational_clock
            .freight_profiles
            .iter()
            .find(|profile| profile.id == profile_id)
    {
        return Some(profile);
    }
    tuning
        .operational_clock
        .freight_profile_for_zone_type(match building.zone_type {
            ZoneType::Commercial => "commercial",
            ZoneType::Industrial => "industrial",
            _ => return None,
        })
}

pub(super) fn building_accepts_input_resource(
    catalog: &RuntimeEconomyCatalog,
    building: &Building,
    resource_runtime_id: ResourceRuntimeId,
) -> bool {
    catalog
        .profile_by_runtime_id(building.economy_profile_runtime_id)
        .is_some_and(|profile| profile.input_port(resource_runtime_id).is_some())
}

pub(super) fn building_outputs_can_supply_local_inputs(
    building: &Building,
    profile: &EconomyProfileRuntime,
) -> bool {
    !profile.outputs.is_empty()
        && profile.kind != EconomyProfileRuntimeKind::ServiceStore
        && (matches!(
            building.zone_type,
            ZoneType::Commercial | ZoneType::Industrial
        ) || matches!(
            profile.kind,
            EconomyProfileRuntimeKind::Extractor | EconomyProfileRuntimeKind::FieldProducer
        ))
}

pub(super) fn building_outputs_can_export_to_owa(
    building: &Building,
    profile: &EconomyProfileRuntime,
) -> bool {
    !profile.outputs.is_empty()
        && profile.kind != EconomyProfileRuntimeKind::ServiceStore
        && (matches!(building.zone_type, ZoneType::Industrial)
            || matches!(
                profile.kind,
                EconomyProfileRuntimeKind::Extractor | EconomyProfileRuntimeKind::FieldProducer
            ))
}

pub(super) fn input_purchase_budget(allocator: &BuildingAllocator, dest_idx: usize) -> f32 {
    let Some(destination) = allocator.buildings.get(dest_idx) else {
        return 0.0;
    };
    if allocator.is_city_service_building(destination) {
        f32::MAX
    } else {
        destination.operating_budget.max(0.0)
    }
}

pub(super) fn reserve_input_payment(
    allocator: &mut BuildingAllocator,
    treasury_balance: &mut f64,
    dest_idx: usize,
    total_cost: f32,
) {
    let city_funded = allocator
        .buildings
        .get(dest_idx)
        .is_some_and(|building| allocator.is_city_service_building(building));
    if city_funded {
        if let Some(destination) = allocator.buildings.get_mut(dest_idx) {
            destination.daily_city_funded_input_cost += total_cost;
        }
        *treasury_balance -= f64::from(total_cost);
    } else if let Some(destination) = allocator.buildings.get_mut(dest_idx) {
        destination.operating_budget -= total_cost;
    }
}

pub(super) fn refund_input_payment(
    allocator: &mut BuildingAllocator,
    treasury_balance: &mut f64,
    dest_idx: usize,
    total_cost: f32,
) {
    let city_funded = allocator
        .buildings
        .get(dest_idx)
        .is_some_and(|building| allocator.is_city_service_building(building));
    if city_funded {
        if let Some(destination) = allocator.buildings.get_mut(dest_idx) {
            destination.daily_city_funded_input_cost =
                (destination.daily_city_funded_input_cost - total_cost).max(0.0);
        }
        *treasury_balance += f64::from(total_cost);
    } else if let Some(destination) = allocator.buildings.get_mut(dest_idx) {
        destination.operating_budget += total_cost;
    }
}
