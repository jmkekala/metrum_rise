//! Resource profile and freight timing lookup helpers.

use crate::simulation::buildings::allocator::Building;
use crate::simulation::economy::definitions::{
    FreightTimingProfile, ResourceRuntimeId, RuntimeEconomyCatalog, RuntimeEconomyTuning,
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
