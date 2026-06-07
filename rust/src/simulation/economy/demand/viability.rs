//! Building level-change viability gates.

use super::credits::clamp01;
use super::snapshot::ResidentialOccupantSnapshot;
use super::types::EPSILON;
use crate::assets::asset::ZoneClass;
use crate::simulation::buildings::allocator::{
    BuildingAllocator, resolve_building_economy_profile_binding,
};
use crate::simulation::economy::definitions::{RuntimeEconomyCatalog, RuntimeEconomyTuning};
use crate::simulation::economy::households::{
    HouseholdSystem, building_operating_buffer_days, building_staffing_ratio,
    building_total_output_inventory, industrial_input_coverage_factor,
    industrial_output_headroom_factor, level_tuning_value,
};
use crate::simulation::zoning::ZoneType;

pub(super) fn building_is_viable_for_upgrade(
    allocator: &BuildingAllocator,
    households: &HouseholdSystem,
    catalog: &RuntimeEconomyCatalog,
    economy_tuning: &RuntimeEconomyTuning,
    residential_occupants: &ResidentialOccupantSnapshot,
    building_idx: usize,
    target_asset_id: &str,
) -> bool {
    let Some(building) = allocator.buildings.get(building_idx) else {
        return false;
    };
    if building.is_deserted {
        return false;
    }
    match building.zone_type {
        ZoneType::Residential => residential_upgrade_viable(
            allocator,
            households,
            economy_tuning,
            residential_occupants,
            building_idx,
            target_asset_id,
        ),
        ZoneType::Commercial => nonresidential_upgrade_viable(
            allocator,
            catalog,
            economy_tuning,
            building_idx,
            target_asset_id,
        ),
        ZoneType::Industrial => industrial_upgrade_viable(
            allocator,
            catalog,
            economy_tuning,
            building_idx,
            target_asset_id,
        ),
        _ => false,
    }
}

pub(super) fn building_is_viable_for_downgrade(
    allocator: &BuildingAllocator,
    households: &HouseholdSystem,
    catalog: &RuntimeEconomyCatalog,
    economy_tuning: &RuntimeEconomyTuning,
    residential_occupants: &ResidentialOccupantSnapshot,
    building_idx: usize,
    _target_asset_id: &str,
) -> bool {
    let Some(building) = allocator.buildings.get(building_idx) else {
        return false;
    };
    if building.is_deserted {
        return false;
    }
    match building.zone_type {
        ZoneType::Residential => residential_downgrade_viable(
            allocator,
            households,
            economy_tuning,
            residential_occupants,
            building_idx,
        ),
        ZoneType::Commercial => {
            nonresidential_downgrade_viable(allocator, catalog, economy_tuning, building_idx)
        }
        ZoneType::Industrial => {
            industrial_downgrade_viable(allocator, catalog, economy_tuning, building_idx)
        }
        _ => false,
    }
}

pub(super) fn residential_upgrade_viable(
    allocator: &BuildingAllocator,
    households: &HouseholdSystem,
    economy_tuning: &RuntimeEconomyTuning,
    residential_occupants: &ResidentialOccupantSnapshot,
    building_idx: usize,
    target_asset_id: &str,
) -> bool {
    let Some(building) = allocator.buildings.get(building_idx) else {
        return false;
    };
    let Some(target_building) = allocator
        .registry
        .get(target_asset_id)
        .and_then(|entry| entry.manifest.building.as_ref())
    else {
        return false;
    };
    let household_capacity = allocator.household_capacity(building_idx);
    if household_capacity == 0 {
        return false;
    }
    let occupancy_ratio = clamp01(building.occupancy as f32 / household_capacity as f32);
    let min_occupancy_ratio = level_tuning_value(
        &economy_tuning
            .viability
            .residential_min_occupancy_ratio_for_upgrade,
        target_building.level,
    );
    if occupancy_ratio + EPSILON < min_occupancy_ratio {
        return false;
    }
    if building.occupancy > 0
        && residential_occupants.household_count_by_building[building_idx] == 0
    {
        return false;
    }
    let required_reserve_days = level_tuning_value(
        &economy_tuning
            .households
            .residential_move_in_min_reserve_days_by_level,
        target_building.level,
    );
    let min_reserve_days = residential_occupants.min_reserve_days_by_building[building_idx];
    if building.occupancy > 0 && min_reserve_days + EPSILON < required_reserve_days {
        return false;
    }

    let _ = households;
    true
}

pub(super) fn residential_downgrade_viable(
    allocator: &BuildingAllocator,
    households: &HouseholdSystem,
    economy_tuning: &RuntimeEconomyTuning,
    residential_occupants: &ResidentialOccupantSnapshot,
    building_idx: usize,
) -> bool {
    let Some(building) = allocator.buildings.get(building_idx) else {
        return false;
    };
    let household_capacity = allocator.household_capacity(building_idx);
    if household_capacity == 0 {
        return false;
    }
    let occupancy_ratio = clamp01(building.occupancy as f32 / household_capacity as f32);
    let max_occupancy_ratio = level_tuning_value(
        &economy_tuning
            .viability
            .residential_max_occupancy_ratio_for_downgrade,
        building.level,
    );
    if occupancy_ratio > max_occupancy_ratio + EPSILON {
        return false;
    }
    if building.occupancy > 0
        && residential_occupants.household_count_by_building[building_idx] == 0
    {
        return false;
    }

    let _ = households;
    true
}

pub(super) fn nonresidential_upgrade_viable(
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    economy_tuning: &RuntimeEconomyTuning,
    building_idx: usize,
    target_asset_id: &str,
) -> bool {
    let Some(building) = allocator.buildings.get(building_idx) else {
        return false;
    };
    if building.is_deserted {
        return false;
    }
    let Some(target_level) = allocator
        .registry
        .get(target_asset_id)
        .and_then(|entry| entry.manifest.building.as_ref())
        .map(|target| target.level)
    else {
        return false;
    };
    let staffing_ratio = building_staffing_ratio(catalog, building);
    if staffing_ratio + EPSILON
        < economy_tuning
            .viability
            .nonresidential_min_staffing_ratio_for_upgrade
    {
        return false;
    }
    let min_buffer_days = level_tuning_value(
        &economy_tuning
            .viability
            .nonresidential_min_buffer_days_by_level,
        target_level,
    );
    if building_operating_buffer_days(catalog, economy_tuning, building) + EPSILON < min_buffer_days
    {
        return false;
    }
    if matches!(building.zone_type, ZoneType::Commercial)
        && building_total_output_inventory(catalog, building) <= EPSILON
    {
        return false;
    }
    true
}

pub(super) fn nonresidential_downgrade_viable(
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    economy_tuning: &RuntimeEconomyTuning,
    building_idx: usize,
) -> bool {
    let Some(building) = allocator.buildings.get(building_idx) else {
        return false;
    };
    if building.is_deserted {
        return false;
    }
    let staffing_ratio = building_staffing_ratio(catalog, building);
    let buffer_days = building_operating_buffer_days(catalog, economy_tuning, building);
    let max_buffer_days = level_tuning_value(
        &economy_tuning
            .viability
            .nonresidential_max_buffer_days_for_downgrade,
        building.level,
    );
    staffing_ratio
        <= economy_tuning
            .viability
            .nonresidential_max_staffing_ratio_for_downgrade
            + EPSILON
        || buffer_days <= max_buffer_days + EPSILON
        || matches!(building.zone_type, ZoneType::Commercial)
            && building_total_output_inventory(catalog, building) <= EPSILON
}

pub(super) fn industrial_upgrade_viable(
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    economy_tuning: &RuntimeEconomyTuning,
    building_idx: usize,
    target_asset_id: &str,
) -> bool {
    let Some(building) = allocator.buildings.get(building_idx) else {
        return false;
    };
    nonresidential_upgrade_viable(
        allocator,
        catalog,
        economy_tuning,
        building_idx,
        target_asset_id,
    ) && industrial_input_coverage_factor(catalog, building) + EPSILON
        >= economy_tuning
            .viability
            .industrial_min_input_coverage_for_upgrade
        && industrial_output_headroom_factor(catalog, building) + EPSILON
            >= economy_tuning
                .viability
                .industrial_min_output_headroom_for_upgrade
}

pub(super) fn industrial_downgrade_viable(
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    economy_tuning: &RuntimeEconomyTuning,
    building_idx: usize,
) -> bool {
    let Some(building) = allocator.buildings.get(building_idx) else {
        return false;
    };
    if building.is_deserted {
        return false;
    }
    nonresidential_downgrade_viable(allocator, catalog, economy_tuning, building_idx)
        || industrial_input_coverage_factor(catalog, building)
            <= economy_tuning
                .viability
                .industrial_max_input_coverage_for_downgrade
                + EPSILON
        || industrial_output_headroom_factor(catalog, building)
            <= economy_tuning
                .viability
                .industrial_max_output_headroom_for_downgrade
                + EPSILON
}

pub(super) fn level_change_is_compatible(
    allocator: &BuildingAllocator,
    building_idx: usize,
    target_asset_id: &str,
) -> bool {
    let Some(building) = allocator.buildings.get(building_idx) else {
        return false;
    };
    let Some(target_entry) = allocator.registry.get(target_asset_id) else {
        return false;
    };
    let Some(target_building) = target_entry.manifest.building.as_ref() else {
        return false;
    };
    if !target_building.is_zoned_private() {
        return false;
    }
    if target_building.lot_width_cells != building.width_cells
        || target_building.lot_depth_cells != building.depth_cells
    {
        return false;
    }
    if matches!(
        target_building.zone_type,
        Some(ZoneClass::Commercial | ZoneClass::Industrial)
    ) {
        let binding =
            resolve_building_economy_profile_binding(&allocator.registry, target_asset_id);
        if binding.economy_broken || binding.runtime_id == 0 {
            return false;
        }
    }
    allocator.registry.household_capacity(target_asset_id) >= building.occupancy
        && allocator.worker_capacity_for_asset(target_asset_id) >= building.worker_count
}
