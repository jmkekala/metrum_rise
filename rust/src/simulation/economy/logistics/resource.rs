// SPDX-License-Identifier: GPL-2.0-only

//! Resource profile and freight timing lookup helpers.

use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, FreightTimingProfile, ResourceRuntimeId,
    RuntimeEconomyCatalog, RuntimeEconomyTuning, RuntimeResourcePort,
};
use crate::simulation::economy::households::scaled_input_inventory_targets_for_building;
use crate::simulation::zoning::ZoneType;

/// Requested stock and emergency status shared by incoming orders and local export holds.
/// No request is emitted until the reorder threshold and shipment size permit one.
pub(super) fn input_restock_request(
    catalog: &RuntimeEconomyCatalog,
    building: &Building,
    profile: &EconomyProfileRuntime,
    input: &RuntimeResourcePort,
    reserved_inbound: f32,
) -> Option<(f32, bool)> {
    let (target, reorder, critical) =
        scaled_input_inventory_targets_for_building(catalog, building, profile, input);
    let stock = building.inventory_units(input.resource_runtime_id) + reserved_inbound;
    let threshold = if reorder > 0.0 { reorder } else { target };
    if target <= 0.0 || stock >= threshold {
        return None;
    }
    let amount = (target - stock).max(0.0);
    let emergency = stock <= critical;
    (amount > 0.0 && (amount >= input.shipment_batch_kg || emergency))
        .then_some((amount, emergency))
}

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
    freight_profile_for_operation(
        tuning,
        catalog.profile_by_runtime_id(building.economy_profile_runtime_id),
        building.zone_type,
    )
}

fn freight_profile_for_operation<'a>(
    tuning: &'a RuntimeEconomyTuning,
    operation: Option<&EconomyProfileRuntime>,
    zone: ZoneType,
) -> Option<&'a FreightTimingProfile> {
    if let Some(profile_id) =
        operation.and_then(|profile| profile.freight_timing_profile.as_deref())
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
        .freight_profile_for_zone_type(match zone {
            ZoneType::Commercial => "commercial",
            ZoneType::Industrial => "industrial",
            _ => return None,
        })
}

/// Current goods-price factor shared by live and proposed input purchases. Carrier charges
/// do not receive this factor. Missing freight timing means no executable material order.
/// O(authored freight profiles + preferred windows), independent of city size, allocation-free.
pub(crate) fn freight_goods_multiplier(
    profile: &EconomyProfileRuntime,
    zone: ZoneType,
    tuning: &RuntimeEconomyTuning,
    minute_of_day: u16,
) -> f32 {
    freight_profile_for_operation(tuning, Some(profile), zone).map_or(f32::INFINITY, |freight| {
        super::timing::adjusted_unit_price(1.0, freight, minute_of_day)
    })
}

/// Prices imported goods in settlement order: catalogue price, outside tariff, then the
/// receiving window. Combining the two factors first can change the f32 invoice bits.
/// A missing window only permits valuation of internally replenished circulation stock.
pub(crate) fn imported_input_unit_price(
    catalog: &RuntimeEconomyCatalog,
    tuning: &RuntimeEconomyTuning,
    resource: ResourceRuntimeId,
    goods_multiplier: f32,
) -> f32 {
    catalog.unit_price_for_resource(resource).unwrap_or(0.0)
        * tuning.owa_import_price_multiplier
        * if goods_multiplier.is_finite() {
            goods_multiplier
        } else {
            1.0
        }
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

/// Whether this operation participates in local business deliveries.
pub(crate) fn building_outputs_can_supply_local_inputs(
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

pub(crate) fn building_outputs_can_export_to_owa(
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
        allocator.record_city_input_purchase(dest_idx, f64::from(total_cost));
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
        allocator.record_city_input_purchase(dest_idx, -f64::from(total_cost));
        if let Some(destination) = allocator.buildings.get_mut(dest_idx) {
            destination.daily_city_funded_input_cost -= total_cost;
        }
        *treasury_balance += f64::from(total_cost);
    } else if let Some(destination) = allocator.buildings.get_mut(dest_idx) {
        destination.operating_budget += total_cost;
    }
}
