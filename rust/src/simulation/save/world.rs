// SPDX-License-Identifier: GPL-2.0-only

//! Serialization for terrain, water, buildings, and zoning systems.

use crate::nodes::sim::core::PendingDemandSpawnAction;
use crate::simulation::agriculture::{AgricultureSystem, FieldSite, validate_field_polygon_world};
use crate::simulation::buildings::allocator::resolve_building_economy_profile_binding;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator, SavedVacancyOrder};
use crate::simulation::core::config::WorldConfig;
use crate::simulation::core::request_queue::RequestQueue;
use crate::simulation::core::scheduler::{LEVELER_MINUTES, MinuteLeveler};
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::definitions::{
    QuantityUnit, load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::economy::demand::{
    ConstructionPermit, DemandBuildingActionKey, DemandLevelChangeAction, DemandSpawnAction,
    DemandSystem, SpawnPermit,
};
use crate::simulation::economy::households::{
    DestinationRoll, Household, HouseholdSystem, JobBoardRoll, PendingHouseholdArrivals,
    SavedDestination, SavedDestinationRoll, SavedDestinationSweep, SavedJobBoard,
    SavedJobBoardSweep,
};
use crate::simulation::economy::logistics::{
    CarrierClass, FreightRequest, FreightRequestFailure, FreightRequestKey, FreightRequestKind,
    SavedSupplierRoll, SavedSupplierSweep, Shipment, ShipmentEndpoint, ShipmentStatus,
    ShipmentSystem, SupplierRoll,
};
use crate::simulation::extraction::{
    ExtractorSite, ResourceExtractionSystem, validate_extractor_polygon_world,
    validate_extractor_reserve,
};
use crate::simulation::grid::data_grid::DataGrid;
use crate::simulation::grid::noise::NoiseSystem;
use crate::simulation::grid::pollution::PollutionSystem;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::resources::{
    COAL_RESOURCE_ID, RESOURCE_RICHNESS_MAX, ResourceDepositSystem,
};
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::water::WaterSystem;
use crate::simulation::work_area::initial_work_area_scale;
use crate::simulation::zoning::{ZoneType, ZoningSystem};
use godot::prelude::Vector2;
use rusqlite::{Connection, Transaction, params};

use super::schema::*;
use super::{SaveLoadError, SaveLoadResult, SnapshotMaps, handle_from_db, handle_to_db};
use super::{
    db_to_optional_usize, i64_to_i8, i64_to_u8, i64_to_u16, i64_to_u32, i64_to_u64, i64_to_usize,
    optional_building_to_db, pack_f32_slice, pack_u16_slice, u64_to_i64, unpack_f32_blob,
    unpack_u16_blob, usize_to_i64,
};
use std::collections::{HashSet, VecDeque};

const SHIPMENT_ENDPOINT_BUILDING: i64 = 0;
const SHIPMENT_ENDPOINT_OWA_BORDER: i64 = 1;
const RESOURCE_DEPOSIT_CHUNK_SIZE_SAMPLES: usize = 64;

fn shipment_endpoint_to_db(
    endpoint: ShipmentEndpoint,
    maps: &SnapshotMaps,
) -> SaveLoadResult<(i64, i64, i64)> {
    match endpoint {
        ShipmentEndpoint::Building(building_id) => {
            let saved_building = optional_building_to_db(building_id, maps)?;
            if saved_building == NONE_REF {
                return Err(SaveLoadError::custom("missing shipment building endpoint"));
            }
            Ok((SHIPMENT_ENDPOINT_BUILDING, saved_building, NONE_REF))
        }
        ShipmentEndpoint::OwaBorder(border_node) => {
            let saved_node = maps
                .node_old_to_new
                .get(&border_node)
                .copied()
                .ok_or_else(|| SaveLoadError::custom("missing shipment border-node mapping"))?;
            Ok((
                SHIPMENT_ENDPOINT_OWA_BORDER,
                NONE_REF,
                usize_to_i64(saved_node as usize)?,
            ))
        }
    }
}

fn shipment_endpoint_from_db(
    endpoint_kind: i64,
    building_id: i64,
    border_node: i64,
) -> SaveLoadResult<ShipmentEndpoint> {
    match endpoint_kind {
        SHIPMENT_ENDPOINT_BUILDING => {
            if border_node != NONE_REF {
                return Err(SaveLoadError::custom(
                    "building shipment endpoint has border node",
                ));
            }
            let building_id = db_to_optional_usize(building_id)?;
            if building_id == usize::MAX {
                return Err(SaveLoadError::custom(
                    "building shipment endpoint missing building id",
                ));
            }
            Ok(ShipmentEndpoint::Building(building_id))
        }
        SHIPMENT_ENDPOINT_OWA_BORDER => {
            if building_id != NONE_REF {
                return Err(SaveLoadError::custom(
                    "border shipment endpoint has building id",
                ));
            }
            if border_node == NONE_REF {
                return Err(SaveLoadError::custom(
                    "border shipment endpoint missing node id",
                ));
            }
            Ok(ShipmentEndpoint::OwaBorder(i64_to_u32(border_node)?))
        }
        _ => Err(SaveLoadError::custom("invalid shipment endpoint kind")),
    }
}

fn shipment_status_from_db(code: i64) -> SaveLoadResult<ShipmentStatus> {
    ShipmentStatus::from_code(code).ok_or_else(|| SaveLoadError::custom("invalid shipment status"))
}

fn carrier_class_from_db(code: i64) -> SaveLoadResult<CarrierClass> {
    CarrierClass::from_code(code).ok_or_else(|| SaveLoadError::custom("invalid carrier class"))
}

fn demand_spawn_zone_to_db(zone_type: ZoneType) -> SaveLoadResult<i64> {
    match zone_type {
        ZoneType::Residential => Ok(1),
        ZoneType::Commercial => Ok(2),
        ZoneType::Industrial => Ok(3),
        _ => Err(SaveLoadError::custom("invalid pending demand spawn zone")),
    }
}

fn demand_spawn_zone_from_db(code: i64) -> SaveLoadResult<ZoneType> {
    match code {
        1 => Ok(ZoneType::Residential),
        2 => Ok(ZoneType::Commercial),
        3 => Ok(ZoneType::Industrial),
        _ => Err(SaveLoadError::custom("invalid pending demand spawn zone")),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn save_world(
    tx: &Transaction,
    terrain: &TerrainSystem,
    water: &WaterSystem,
    resource_deposits: &ResourceDepositSystem,
    zoning: &ZoningSystem,
    buildings: &BuildingAllocator,
    households: &HouseholdSystem,
    logistics: &ShipmentSystem,
    resource_extraction: &ResourceExtractionSystem,
    agriculture: &AgricultureSystem,
    demand: &DemandSystem,
    pending_demand_spawns: &VecDeque<PendingDemandSpawnAction>,
    pollution: &PollutionSystem,
    noise: &NoiseSystem,
    maps: &SnapshotMaps,
) -> SaveLoadResult<()> {
    // Terrain
    tx.execute(
        "INSERT INTO terrain_state(width, height, height_blob_f32_le) VALUES (?1, ?2, ?3)",
        params![
            usize_to_i64(terrain.width)?,
            usize_to_i64(terrain.height)?,
            pack_f32_slice(&terrain.clone_source_dense())
        ],
    )?;

    // Water
    tx.execute(
        "INSERT INTO water_state(width, height, baseline_depth_blob_f32_le) VALUES (?1, ?2, ?3)",
        params![
            usize_to_i64(water.width)?,
            usize_to_i64(water.height)?,
            pack_f32_slice(&water.clone_baseline_depth_dense()),
        ],
    )?;

    save_resource_deposits(tx, resource_deposits)?;

    // Demand
    let upgrade_action_credit = demand.upgrade_action_credit.as_array();
    let downgrade_action_credit = demand.downgrade_action_credit.as_array();
    let despawn_action_credit = demand.despawn_action_credit.as_array();
    tx.execute(
        "INSERT INTO demand_state(households_to_admit_today, households_to_remove_today, persistent_exit_action_credit, upgrade_action_credit_residential, upgrade_action_credit_commercial, upgrade_action_credit_industrial, downgrade_action_credit_residential, downgrade_action_credit_commercial, downgrade_action_credit_industrial, despawn_action_credit_residential, despawn_action_credit_commercial, despawn_action_credit_industrial, cheat_max_demands_enabled, exit_eligible_today) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            i64::from(demand.households_to_admit_today),
            i64::from(demand.households_to_remove_today),
            demand.persistent_exit_action_credit,
            upgrade_action_credit[0],
            upgrade_action_credit[1],
            upgrade_action_credit[2],
            downgrade_action_credit[0],
            downgrade_action_credit[1],
            downgrade_action_credit[2],
            despawn_action_credit[0],
            despawn_action_credit[1],
            despawn_action_credit[2],
            bool_to_db(demand.cheat_max_demands_enabled),
            bool_to_db(demand.exit_eligible_today),
        ],
    )?;
    let [admission, removal] = demand.saved_household_diagnostics();
    tx.execute(
        "INSERT INTO demand_diagnostics(singleton, admission, removal) VALUES (1, ?1, ?2)",
        params![admission, removal],
    )?;
    let mut pending_spawn_stmt = tx.prepare(
        "INSERT INTO pending_demand_spawns(sequence, due_minute, zone_type, parcel_id, asset_id, planned_day_index, planned_minute_of_day, spawn_id, permit, counted) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    )?;
    for (sequence, pending) in pending_demand_spawns.iter().enumerate() {
        pending_spawn_stmt.execute(params![
            usize_to_i64(sequence)?,
            u64_to_i64(pending.due_minute)?,
            demand_spawn_zone_to_db(pending.zone_type)?,
            u64_to_i64(pending.action.parcel_id)?,
            &pending.action.asset_id,
            i64::from(pending.planned_day_index),
            i64::from(pending.planned_minute_of_day),
            u64_to_i64(pending.id)?,
            pack_spawn_permit(pending.permit),
            i64::from(pending.counted),
        ])?;
    }

    save_demand_hour_plan(tx, demand, maps)?;
    // A planning slot's setup held for its plan, `PLANNING_SETUP_LEAD` slots later
    // (`SIM-02-K`); no row without one.
    if let Some(setup) = demand.saved_planning_setup() {
        tx.execute(
            "INSERT INTO demand_planning_setup(singleton, setup) VALUES (1, ?1)",
            params![setup],
        )?;
    }

    // Grids: the front every reader uses and the back of the rolling sweep (`SIM-02-H`).
    tx.execute(
        "INSERT INTO pollution_state(width, height, grid_blob_f32_le, back_blob_f32_le) VALUES (?1, ?2, ?3, ?4)",
        params![
            usize_to_i64(pollution.grid.width)?,
            usize_to_i64(pollution.grid.height)?,
            pack_f32_slice(&pollution.grid.data),
            pack_f32_slice(&pollution.back.data)
        ],
    )?;
    tx.execute(
        "INSERT INTO noise_state(width, height, grid_blob_f32_le, back_blob_f32_le) VALUES (?1, ?2, ?3, ?4)",
        params![
            usize_to_i64(noise.grid.width)?,
            usize_to_i64(noise.grid.height)?,
            pack_f32_slice(&noise.grid.data),
            pack_f32_slice(&noise.back.data)
        ],
    )?;

    // Zoning parcels
    {
        let mut parcel_stmt = tx.prepare("INSERT INTO zoning_parcels(parcel_id, edge_id, side, frontage_t, frontage_m, depth_m, profile_runtime_id, build_generation) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)")?;
        for parcel in zoning.parcels() {
            let saved_eid = maps
                .edge_old_to_new
                .get(&parcel.edge_idx())
                .copied()
                .ok_or_else(|| SaveLoadError::custom("missing parcel edge mapping"))?;
            parcel_stmt.execute(params![
                i64::try_from(parcel.id().raw())
                    .map_err(|_| SaveLoadError::custom("parcel id overflow"))?,
                usize_to_i64(saved_eid)?,
                i64::from(parcel.side()),
                parcel.frontage_center_t(),
                parcel.frontage_m(),
                parcel.depth_m(),
                i64::from(parcel.zone_profile_runtime_id()),
                i64::from(parcel.build_generation()),
            ])?;
        }
    }

    super::cell_zoning::save(tx, zoning, maps)?;

    // Buildings
    let mut bld_stmt = tx.prepare("INSERT INTO buildings(building_id, parcel_id, edge_id, frontage_t, side, cell_x, cell_y, profile_runtime_id, occupancy, worker_count, service_funding_override, revenue, operating_budget, profit_tax_budget_baseline, last_day_profit, shipment_cooldown_until_slot, width, depth, asset_id, level, construction_total_hours, construction_complete_slot, broken, pending_redevelopment, rezone_grace_days_remaining, is_deserted, budget_distress, daily_household_sales_value, daily_power_service_units, daily_power_served_units, recent_power_service_units, recent_power_served_units, recent_household_sales_value, support_height_m, build_generation, electricity_usage, hourly_economy_activity, commercial_activity_floor_scale, paid_workers_in_window, funded_job_capacity, unpaid_cash_wages_in_window, service_sales_capacity, service_sales_profile_runtime_id, funded_activity, applicant_seed) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35, ?36, ?37, ?38, ?39, ?40, ?41, ?42, ?43, ?44, ?45)")?;
    let mut inventory_stmt = tx.prepare(
        "INSERT INTO building_inventories(building_id, resource_runtime_id, amount) VALUES (?1, ?2, ?3)",
    )?;
    let mut detached_stmt =
        tx.prepare("INSERT INTO building_detached_poses VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)")?;
    for (saved_bid, b) in buildings.buildings.iter().enumerate() {
        if !b.daily_city_funded_input_cost.is_finite() {
            return Err(SaveLoadError::custom("invalid city input cost"));
        }
        for usage in [
            b.daily_electricity.workplace,
            b.daily_electricity.residential,
            b.recent_electricity.workplace,
            b.recent_electricity.residential,
        ] {
            validate_electricity_usage(usage)?;
        }
        let saved_bid_db = usize_to_i64(saved_bid)?;
        let saved_eid = maps
            .edge_old_to_new
            .get(&b.edge_idx)
            .copied()
            .map(usize_to_i64)
            .transpose()?;
        if saved_eid.is_none() {
            if b.parcel_id != 0
                || b.zone_type != ZoneType::None
                || ![
                    b.center_x,
                    b.center_y,
                    b.facing_dir.x,
                    b.facing_dir.y,
                    b.side_offset,
                    b.front_gap_m[0],
                    b.front_gap_m[1],
                    b.front_gap_m[2],
                ]
                .into_iter()
                .all(f32::is_finite)
                || (b.facing_dir.length_squared() - 1.0).abs() > 0.001
            {
                return Err(SaveLoadError::custom(
                    "invalid detached explicit building pose",
                ));
            }
            detached_stmt.execute(params![
                saved_bid_db,
                b.center_x,
                b.center_y,
                b.facing_dir.x,
                b.facing_dir.y,
                b.side_offset,
                b.front_gap_m[0],
                b.front_gap_m[1],
                b.front_gap_m[2]
            ])?;
        }
        bld_stmt.execute(params![
            saved_bid_db,
            i64::try_from(b.parcel_id)
                .map_err(|_| SaveLoadError::custom("building parcel id overflow"))?,
            saved_eid.unwrap_or(NONE_REF),
            b.frontage_t,
            i64::from(b.side),
            usize_to_i64(b.cell_x)?,
            usize_to_i64(b.cell_y as usize)?,
            i64::from(b.zone_profile_runtime_id),
            i64::from(b.occupancy),
            i64::from(b.worker_count),
            b.service_funding_override,
            b.revenue,
            b.operating_budget,
            b.profit_tax_budget_baseline,
            b.last_day_profit,
            u64_to_i64(b.shipment_cooldown_until_slot)?,
            usize_to_i64(b.width_cells as usize)?,
            usize_to_i64(b.depth_cells as usize)?,
            &b.asset_id,
            i64::from(b.level),
            i64::from(b.construction_total_hours),
            u64_to_i64(b.construction_complete_slot)?,
            i64::from(if b.broken { 1 } else { 0 }),
            i64::from(if b.pending_redevelopment { 1 } else { 0 }),
            i64::from(b.rezone_grace_days_remaining),
            i64::from(if b.is_deserted { 1 } else { 0 }),
            i64::from(if b.budget_distress { 1 } else { 0 }),
            b.daily_household_sales_value,
            b.daily_power_service_units,
            b.daily_power_served_units,
            b.recent_power_service_units,
            b.recent_power_served_units,
            b.recent_household_sales_value,
            b.support_height_m,
            i64::from(b.build_generation),
            serde_json::to_string(&(
                b.daily_electricity,
                b.recent_electricity,
                b.daily_city_funded_input_cost
            ))
            .map_err(|err| SaveLoadError::custom(err.to_string()))?,
            b.hourly_economy_activity,
            b.commercial_activity_floor_scale,
            i64::from(b.paid_workers_in_window),
            i64::from(b.funded_job_capacity),
            i64::from(b.unpaid_cash_wages_in_window),
            b.service_sales_capacity,
            i64::from(b.service_sales_profile_runtime_id),
            b.funded_activity,
            u64_to_i64(b.applicant_seed)?,
        ])?;
        for (slot, amount) in b.resource_inventory.iter().enumerate() {
            if *amount <= 0.0 {
                continue;
            }
            inventory_stmt.execute(params![saved_bid_db, i64::from(slot as u16 + 1), *amount])?;
        }
    }
    tx.execute(
        "INSERT INTO building_seed_state(singleton, next_applicant_seed) VALUES (1, ?1)",
        [u64_to_i64(buildings.next_applicant_seed)?],
    )?;
    save_vacancy_order(tx, buildings)?;
    save_resource_extraction(tx, resource_extraction, maps)?;
    save_agriculture(tx, agriculture, maps)?;
    if !households.removed_electricity.is_valid() {
        return Err(SaveLoadError::custom(
            "invalid removed-building electricity account",
        ));
    }
    tx.execute(
        "INSERT INTO electricity_day_state(singleton, removed_buildings, day_totals, last_day_totals, coal_inventory_kg, city_input_day) VALUES (1, ?1, ?2, ?3, ?4, ?5)",
        params![
            to_json(&households.removed_electricity)?,
            to_json(&households.day_totals)?,
            to_json(&households.last_day_totals)?,
            households.coal_inventory_kg,
            to_json(&buildings.city_input_day)?,
        ],
    )?;
    // The hour in progress spans a save: the hourly phases read the last finished hour's totals
    // and fold their own (`SIM-02-I`).
    if !households.electricity_hour.is_valid() || !households.service_sales.is_valid() {
        return Err(SaveLoadError::custom("invalid city hour state"));
    }
    // REAL columns keep every float bit for bit.
    let electricity = &households.electricity_hour;
    let sales = &households.service_sales;
    tx.execute(
        "INSERT INTO city_hour_state(singleton, electricity_dispatch, electricity_coverage, electricity_demand_kwh, electricity_capacity_kwh, electricity_coal_burned_kg, electricity_coal_stock_kg, service_residents, service_gross_revenue, service_hour_residents, service_hour_collected) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            electricity.dispatch,
            electricity.coverage,
            electricity.demand_kwh,
            electricity.capacity_kwh,
            electricity.coal_burned_kg,
            electricity.coal_stock_kg,
            i64::from(sales.residents),
            sales.gross_revenue,
            i64::from(sales.hour_residents),
            sales.hour_collected,
        ],
    )?;
    let mut sales_stmt = tx.prepare(
        "INSERT INTO service_sales_resources(resource_slot, gross, capacity, drawable, undrawn, hour_capacity) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for slot in 0..sales.gross_by_resource.len() {
        sales_stmt.execute(params![
            usize_to_i64(slot)?,
            sales.gross_by_resource[slot],
            sales.capacity_by_resource[slot],
            sales.drawable_by_resource[slot],
            sales.undrawn_by_resource[slot],
            sales.hour_capacity_by_resource[slot],
        ])?;
    }
    // Requests the households' phases queued, in order: kinds 0 housing reviews, 2 rehousings,
    // 3 restocks and 4 visible visits with their queue slots (`SIM-02-J`), 1 claimed departures
    // for the `departures` queue (`SIM-02-G`, `SIM-02-J`).
    let mut request_stmt = tx.prepare(
        "INSERT INTO household_requests(kind, sequence, slot, household_slot, household_generation) VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    for (kind, queue) in [
        (0_i64, &households.housing_reviews),
        (2, &households.rehousing),
        (3, &households.restock_requests),
        (4, &households.visit_requests),
    ] {
        for (sequence, (queued_slot, handle)) in queue.iter().enumerate() {
            let (slot, generation) = handle_to_db(handle);
            request_stmt.execute(params![
                kind,
                sequence as i64,
                u64_to_i64(queued_slot)?,
                slot,
                generation
            ])?;
        }
    }
    for (sequence, &handle) in households.exits.iter().enumerate() {
        let (slot, generation) = handle_to_db(handle);
        request_stmt.execute(params![
            1_i64,
            sequence as i64,
            Option::<i64>::None,
            slot,
            generation
        ])?;
    }
    let mut household_stmt = tx.prepare("INSERT INTO households(household_id, home_building, budget, stock, member_count, child_count, adult_count, elder_count, consumption_rate, stock_days, replenishment_state, cooldown_hours, replenishment_failure_count, reserved_store_building_id, reserved_amount, reserved_total_cost, shopping_agent_slot, shopping_agent_generation, shopping_timeout_hours_remaining, replenishment_search_cursor, stay_failure_days, unhoused_days_elapsed, replenishment_offset_hours, unemployment_days_elapsed, power_consumption_cost, water_consumption_cost, sewage_consumption_cost, electricity_account, gross_daily_wages, employed_adults, paid_workers_in_window, shopping_has_car) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32)")?;
    for (hid, household) in households.households.iter().enumerate() {
        let ledger = households
            .daily_ledgers()
            .get(hid)
            .copied()
            .unwrap_or_default();
        validate_electricity_usage(ledger.electricity)?;
        if !household.gross_daily_wages.is_finite() || household.gross_daily_wages < 0.0 {
            return Err(SaveLoadError::custom("invalid household wage forecast"));
        }
        if [
            household.power_arrears.local,
            household.power_arrears.imported,
            ledger.power_unpaid_cost,
            ledger.power_local_payment,
        ]
        .iter()
        .any(|v| !v.is_finite() || *v < 0.0)
        {
            return Err(SaveLoadError::custom(
                "invalid household electricity account",
            ));
        }
        household_stmt.execute(params![
            usize_to_i64(hid)?,
            optional_building_to_db(household.home_building_id, maps)?,
            household.budget,
            household.stock,
            i64::from(household.member_count),
            i64::from(household.child_count),
            i64::from(household.adult_count),
            i64::from(household.elder_count),
            household.consumption_rate,
            household.stock_days,
            i64::from(household.replenishment_state),
            i64::from(household.cooldown_hours),
            i64::from(household.replenishment_failure_count),
            optional_building_to_db(household.reserved_store_building_id, maps)?,
            household.reserved_amount,
            household.reserved_total_cost,
            handle_to_db(household.shopping_agent).0,
            handle_to_db(household.shopping_agent).1,
            i64::from(household.shopping_timeout_hours_remaining),
            i64::from(household.replenishment_search_cursor),
            i64::from(household.stay_failure_days),
            i64::from(household.unhoused_days_elapsed),
            i64::from(household.replenishment_offset_hours),
            i64::from(household.unemployment_days_elapsed),
            ledger.power_consumption_cost,
            ledger.water_consumption_cost,
            ledger.sewage_consumption_cost,
            serde_json::to_string(&(
                ledger.electricity,
                household.power_arrears,
                ledger.power_unpaid_cost,
                ledger.power_local_payment
            ))
            .map_err(|err| SaveLoadError::custom(err.to_string()))?,
            household.gross_daily_wages,
            i64::from(household.employed_adults),
            i64::from(ledger.paid_workers),
            i64::from(household.shopping_has_car),
        ])?;
    }

    let mut shipment_stmt = tx.prepare("INSERT INTO shipments(shipment_id, resource_runtime_id, amount, source_endpoint_kind, source_building_id, source_border_node, destination_endpoint_kind, destination_building_id, destination_border_node, carrier_class, status, carrier_slot, total_cost, eta_hours, leg_started_hour, freight_cost, carrier_generation, cargo_loaded, trip_event_slot) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)")?;
    for shipment in &logistics.shipments {
        let (source_kind, source_building_id, source_border_node) =
            shipment_endpoint_to_db(shipment.source, maps)?;
        let (destination_kind, destination_building_id, destination_border_node) =
            shipment_endpoint_to_db(shipment.destination, maps)?;
        shipment_stmt.execute(params![
            u64_to_i64(shipment.id)?,
            i64::from(shipment.resource_runtime_id),
            shipment.amount,
            source_kind,
            source_building_id,
            source_border_node,
            destination_kind,
            destination_building_id,
            destination_border_node,
            shipment.carrier_class.code(),
            shipment.status.code(),
            handle_to_db(shipment.carrier).0,
            shipment.total_cost,
            i64::from(shipment.eta_hours),
            i64::from(shipment.leg_started_hour),
            shipment.freight_cost,
            handle_to_db(shipment.carrier).1,
            i64::from(shipment.cargo_loaded),
            u64_to_i64(shipment.trip_event_slot)?,
        ])?;
    }
    let mut request_failure_stmt = tx.prepare("INSERT INTO freight_request_failures(destination_building_id, resource_runtime_id, failures, terminal) VALUES (?1, ?2, ?3, ?4)")?;
    for (key, failure) in &logistics.request_failures {
        request_failure_stmt.execute(params![
            optional_building_to_db(key.destination_building_id, maps)?,
            i64::from(key.resource_runtime_id),
            i64::from(failure.failures),
            if failure.terminal { 1_i64 } else { 0_i64 },
        ])?;
    }
    // Requests the buildings' phases queued and the queue has not served (`SIM-02-J`).
    let mut request_stmt = tx.prepare("INSERT INTO freight_requests(position, slot, kind, building_slot, building_generation) VALUES (?1, ?2, ?3, ?4, ?5)")?;
    for (position, request) in logistics.freight_requests.iter().enumerate() {
        let (slot, generation) = handle_to_db(request.building);
        request_stmt.execute(params![
            usize_to_i64(position)?,
            u64_to_i64(request.slot)?,
            request.kind.code(),
            slot,
            generation,
        ])?;
    }

    Ok(())
}

// The visit credit, the shopper departure leveler, the household destination roll, the job
// board and the queued job searches (`SIM-02-J`), for a game whose road components are of CCH
// build `layout`, `None` while a road change waits for the next rebuild. The destination front's
// entries are buffer 0 and the sweep's buffer 1, each kind (0 stores, 1 service stores) in its
// own order.
pub(super) fn save_household_queue_state(
    tx: &Transaction,
    households: &HouseholdSystem,
    layout: Option<u32>,
) -> SaveLoadResult<()> {
    let roll = households.destinations.saved_for(layout);
    let (sweep_handles, sweep_hour, sweep_next_chunk) = match &roll.sweep {
        Some(sweep) => (
            u64_to_i64(sweep.building_handles)?,
            u64_to_i64(sweep.hour)?,
            u64_to_i64(sweep.next_chunk)?,
        ),
        None => (NONE_REF, 0, 0),
    };
    let front_hour = match &roll.front {
        Some((hour, _)) => u64_to_i64(*hour)?,
        None => NONE_REF,
    };
    let (first_minute, counts) = households.shopper_departures.saved();
    let board = households.job_board.saved();
    let job_board_hour = match &board.front {
        Some((hour, _)) => u64_to_i64(*hour)?,
        None => NONE_REF,
    };
    let (job_sweep_handles, job_sweep_hour, job_sweep_next_chunk) = match &board.sweep {
        Some(sweep) => (
            u64_to_i64(sweep.building_handles)?,
            u64_to_i64(sweep.hour)?,
            u64_to_i64(sweep.next_chunk)?,
        ),
        None => (NONE_REF, 0, 0),
    };
    tx.execute(
        "INSERT INTO household_queue_state(singleton, visit_credit, shopper_departure_first_minute, shopper_departure_counts, destination_front_hour, destination_sweep_building_handles, destination_sweep_hour, destination_sweep_next_chunk, destination_front_current, destination_sweep_current, job_board_hour, job_sweep_building_handles, job_sweep_hour, job_sweep_next_chunk, admission_credit, admission_rate, pending_households, pending_adults, pending_elders, pending_children) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
        params![
            i64::from(households.visit_credit),
            u64_to_i64(first_minute)?,
            pack_u16_slice(counts),
            front_hour,
            sweep_handles,
            sweep_hour,
            sweep_next_chunk,
            bool_to_db(roll.current[0]),
            bool_to_db(roll.current[1]),
            job_board_hour,
            job_sweep_handles,
            job_sweep_hour,
            job_sweep_next_chunk,
            i64::from(households.admission_credit),
            i64::from(households.admission_rate),
            i64::from(households.pending_arrivals.households),
            i64::from(households.pending_arrivals.adults),
            i64::from(households.pending_arrivals.elders),
            i64::from(households.pending_arrivals.children),
        ],
    )?;
    // The job board's members (buffer 0) and the sweep's (buffer 1), and the queued job
    // searches, each in order (`SIM-02-J`).
    let mut stmt = tx.prepare(
        "INSERT INTO job_board_members(buffer, position, building_slot, building_generation) VALUES (?1, ?2, ?3, ?4)",
    )?;
    let front = board.front.as_ref().map(|(_, members)| members.as_slice());
    let back = board.sweep.as_ref().map(|sweep| sweep.members.as_slice());
    for (buffer, members) in [(0_i64, front), (1, back)] {
        for (position, &building) in members.unwrap_or_default().iter().enumerate() {
            let (slot, generation) = handle_to_db(building);
            stmt.execute(params![buffer, usize_to_i64(position)?, slot, generation])?;
        }
    }
    let mut stmt = tx.prepare(
        "INSERT INTO job_requests(sequence, slot, agent_slot, agent_generation) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (sequence, (queued_slot, agent)) in households.job_requests.iter().enumerate() {
        let (slot, generation) = handle_to_db(agent);
        stmt.execute(params![
            usize_to_i64(sequence)?,
            u64_to_i64(queued_slot)?,
            slot,
            generation
        ])?;
    }
    let mut stmt = tx.prepare("INSERT INTO household_destinations(buffer, kind, position, building_slot, building_generation, chunk_x, chunk_y, foot_count, foot_a, foot_b, car_count, car_a, car_b, parking_components) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)")?;
    let front = roll.front.as_ref().map(|(_, entries)| entries);
    let back = roll.sweep.as_ref().map(|sweep| &sweep.entries);
    for (buffer, entries) in [(0_i64, front), (1, back)] {
        let Some(entries) = entries else {
            continue;
        };
        for (kind, entries) in entries.iter().enumerate() {
            for (position, (building, x, y, (foot, foot_count), (car, car_count), parking_car)) in
                entries.iter().enumerate()
            {
                stmt.execute(params![
                    buffer,
                    usize_to_i64(kind)?,
                    usize_to_i64(position)?,
                    i64::from((*building >> 32) as u32),
                    i64::from(*building as u32),
                    i64::from(*x),
                    i64::from(*y),
                    usize_to_i64(*foot_count)?,
                    i64::from(foot[0]),
                    i64::from(foot[1]),
                    usize_to_i64(*car_count)?,
                    i64::from(car[0]),
                    i64::from(car[1]),
                    serde_json::to_vec(parking_car).map_err(|err| SaveLoadError::custom(err.to_string()))?,
                ])?;
            }
        }
    }
    Ok(())
}

// `household_queue_state`: the visit credit and shopper leveler, the destination roll's header,
// the job board's hour, the admission credit (`SIM-02-J`) and the pending arrivals the last
// operational hour counted, which demand planning reads until the next count.
type QueueStateRow = (
    (i64, i64, Vec<u8>),
    (i64, i64, i64, i64),
    (i64, i64, i64),
    (i64, i64, i64),
    (i64, i64),
    (i64, i64, i64, i64),
);

fn load_household_queue_state(
    conn: &Connection,
    households: &mut HouseholdSystem,
) -> SaveLoadResult<()> {
    let (
        (credit, first_minute, counts),
        (front_hour, sweep_handles, sweep_hour, sweep_next_chunk),
        (front_current, sweep_current, job_board_hour),
        (job_sweep_handles, job_sweep_hour, job_sweep_next_chunk),
        (admission_credit, admission_rate),
        (pending_households, pending_adults, pending_elders, pending_children),
    ): QueueStateRow = conn.query_row(
        "SELECT visit_credit, shopper_departure_first_minute, shopper_departure_counts, destination_front_hour, destination_sweep_building_handles, destination_sweep_hour, destination_sweep_next_chunk, destination_front_current, destination_sweep_current, job_board_hour, job_sweep_building_handles, job_sweep_hour, job_sweep_next_chunk, admission_credit, admission_rate, pending_households, pending_adults, pending_elders, pending_children FROM household_queue_state WHERE singleton = 1",
        [],
        |row| {
            Ok((
                (row.get(0)?, row.get(1)?, row.get(2)?),
                (row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?),
                (row.get(7)?, row.get(8)?, row.get(9)?),
                (row.get(10)?, row.get(11)?, row.get(12)?),
                (row.get(13)?, row.get(14)?),
                (row.get(15)?, row.get(16)?, row.get(17)?, row.get(18)?),
            ))
        },
    )?;
    households.visit_credit = i64_to_u32(credit)?;
    households.admission_credit = i64_to_u32(admission_credit)?;
    households.admission_rate = i64_to_u32(admission_rate)?;
    // The count stays as the last operational hour took it, plus the hour's admissions, until
    // the next one recounts; a recount at load would drop the carriers that arrived since.
    households.pending_arrivals = PendingHouseholdArrivals {
        households: i64_to_u32(pending_households)?,
        adults: i64_to_u32(pending_adults)?,
        elders: i64_to_u32(pending_elders)?,
        children: i64_to_u32(pending_children)?,
    };
    let mut board_buffers = [Vec::new(), Vec::new()];
    let mut stmt = conn.prepare(
        "SELECT buffer, building_slot, building_generation FROM job_board_members ORDER BY buffer, position",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let members = usize::try_from(row.get::<_, i64>(0)?)
            .ok()
            .and_then(|buffer| board_buffers.get_mut(buffer))
            .ok_or_else(|| SaveLoadError::custom("invalid job board buffer"))?;
        members.push(handle_from_db(row.get(1)?, row.get(2)?)?);
    }
    let [board_front, board_back] = board_buffers;
    if (job_board_hour == NONE_REF && !board_front.is_empty())
        || (job_sweep_handles == NONE_REF && !board_back.is_empty())
    {
        return Err(SaveLoadError::custom(
            "job board members without their buffer",
        ));
    }
    let board = SavedJobBoard {
        front: match job_board_hour {
            NONE_REF => None,
            hour => Some((i64_to_u64(hour)?, board_front)),
        },
        sweep: match job_sweep_handles {
            NONE_REF => None,
            handles => Some(SavedJobBoardSweep {
                building_handles: i64_to_u64(handles)?,
                hour: i64_to_u64(job_sweep_hour)?,
                next_chunk: i64_to_u64(job_sweep_next_chunk)?,
                members: board_back,
            }),
        },
    };
    households.job_board = JobBoardRoll::restore(board).map_err(SaveLoadError::custom)?;
    let mut stmt = conn
        .prepare("SELECT slot, agent_slot, agent_generation FROM job_requests ORDER BY sequence")?;
    let requests = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .map(|row| {
            let (queued_slot, slot, generation): (i64, i64, i64) = row?;
            Ok((i64_to_u64(queued_slot)?, handle_from_db(slot, generation)?))
        })
        .collect::<SaveLoadResult<Vec<_>>>()?;
    households.job_requests = RequestQueue::restore(requests);
    let counts = unpack_u16_blob(&counts, LEVELER_MINUTES)?;
    households.shopper_departures = MinuteLeveler::restore(i64_to_u64(first_minute)?, &counts)
        .ok_or_else(|| SaveLoadError::custom("invalid shopper departure counts"))?;
    let mut buffers: [[Vec<SavedDestination>; 2]; 2] = Default::default();
    let mut stmt = conn.prepare("SELECT buffer, kind, building_slot, building_generation, chunk_x, chunk_y, foot_count, foot_a, foot_b, car_count, car_a, car_b, parking_components FROM household_destinations ORDER BY buffer, kind, position")?;
    let mut rows = stmt.query([])?;
    let chunk = |value: i64| {
        i32::try_from(value).map_err(|_| SaveLoadError::custom("destination chunk out of range"))
    };
    while let Some(row) = rows.next()? {
        let entries = usize::try_from(row.get::<_, i64>(0)?)
            .ok()
            .zip(usize::try_from(row.get::<_, i64>(1)?).ok())
            .and_then(|(buffer, kind)| buffers.get_mut(buffer)?.get_mut(kind))
            .ok_or_else(|| SaveLoadError::custom("invalid household destination buffer"))?;
        entries.push((
            u64::from(i64_to_u32(row.get(2)?)?) << 32 | u64::from(i64_to_u32(row.get(3)?)?),
            chunk(row.get(4)?)?,
            chunk(row.get(5)?)?,
            (
                [i64_to_u32(row.get(7)?)?, i64_to_u32(row.get(8)?)?],
                i64_to_usize(row.get(6)?)?,
            ),
            (
                [i64_to_u32(row.get(10)?)?, i64_to_u32(row.get(11)?)?],
                i64_to_usize(row.get(9)?)?,
            ),
            serde_json::from_slice(&row.get::<_, Vec<u8>>(12)?).map_err(|err| SaveLoadError::custom(err.to_string()))?,
        ));
    }
    let [front, back] = buffers;
    let has_entries = |entries: &[Vec<SavedDestination>; 2]| entries.iter().any(|e| !e.is_empty());
    if (front_hour == NONE_REF && has_entries(&front))
        || (sweep_handles == NONE_REF && has_entries(&back))
    {
        return Err(SaveLoadError::custom(
            "household destinations without their buffer",
        ));
    }
    let saved = SavedDestinationRoll {
        current: [front_current != 0, sweep_current != 0],
        front: match front_hour {
            NONE_REF => None,
            hour => Some((i64_to_u64(hour)?, front)),
        },
        sweep: match sweep_handles {
            NONE_REF => None,
            handles => Some(SavedDestinationSweep {
                building_handles: i64_to_u64(handles)?,
                hour: i64_to_u64(sweep_hour)?,
                next_chunk: i64_to_u64(sweep_next_chunk)?,
                entries: back,
            }),
        },
    };
    households.destinations = DestinationRoll::restore(saved).map_err(SaveLoadError::custom)?;
    Ok(())
}

// The front's entries are buffer 0 and the sweep's buffer 1, each in its own order; building
// handles split into slot and generation as other saved handles do. The roll comes from
// `ShipmentSystem::saved_supplier_roll`, which knows whether its buffers are current.
pub(super) fn save_supplier_roll(tx: &Transaction, roll: &SavedSupplierRoll) -> SaveLoadResult<()> {
    let (sweep_handles, sweep_hour, sweep_next_chunk) = match &roll.sweep {
        Some(sweep) => (
            u64_to_i64(sweep.building_handles)?,
            i64::from(sweep.hour),
            u64_to_i64(sweep.next_chunk)?,
        ),
        None => (NONE_REF, 0, 0),
    };
    tx.execute(
        "INSERT INTO freight_supplier_sweep(singleton, front_hour, sweep_building_handles, sweep_hour, sweep_next_chunk, front_current, sweep_current) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            roll.front
                .as_ref()
                .map_or(NONE_REF, |&(hour, _)| i64::from(hour)),
            sweep_handles,
            sweep_hour,
            sweep_next_chunk,
            i64::from(roll.current[0]),
            i64::from(roll.current[1]),
        ],
    )?;
    let mut stmt = tx.prepare("INSERT INTO freight_supplier_entries(buffer, position, resource_runtime_id, component, chunk_x, chunk_y, building_slot, building_generation) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)")?;
    let front = roll.front.as_ref().map(|(_, entries)| entries.as_slice());
    let back = roll.sweep.as_ref().map(|sweep| sweep.entries.as_slice());
    for (buffer, entries) in [(0_i64, front), (1, back)] {
        for (position, &(resource, component, x, y, building)) in
            entries.unwrap_or_default().iter().enumerate()
        {
            stmt.execute(params![
                buffer,
                usize_to_i64(position)?,
                i64::from(resource),
                i64::from(component),
                i64::from(x),
                i64::from(y),
                i64::from((building >> 32) as u32),
                i64::from(building as u32),
            ])?;
        }
    }
    Ok(())
}

fn load_supplier_roll(conn: &Connection) -> SaveLoadResult<SupplierRoll> {
    let ((front_hour, sweep_handles, sweep_hour, sweep_next_chunk), current): (
        (i64, i64, i64, i64),
        [bool; 2],
    ) = conn.query_row(
        "SELECT front_hour, sweep_building_handles, sweep_hour, sweep_next_chunk, front_current, sweep_current FROM freight_supplier_sweep WHERE singleton = 1",
        [],
        |row| {
            Ok((
                (row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?),
                [row.get::<_, i64>(4)? != 0, row.get::<_, i64>(5)? != 0],
            ))
        },
    )?;
    let mut buffers = [Vec::new(), Vec::new()];
    let mut stmt = conn.prepare("SELECT buffer, resource_runtime_id, component, chunk_x, chunk_y, building_slot, building_generation FROM freight_supplier_entries ORDER BY buffer, position")?;
    let mut rows = stmt.query([])?;
    let chunk = |value: i64| {
        i32::try_from(value).map_err(|_| SaveLoadError::custom("supplier chunk out of range"))
    };
    while let Some(row) = rows.next()? {
        let entries = usize::try_from(row.get::<_, i64>(0)?)
            .ok()
            .and_then(|buffer| buffers.get_mut(buffer))
            .ok_or_else(|| SaveLoadError::custom("invalid supplier index buffer"))?;
        entries.push((
            i64_to_u16(row.get(1)?)?,
            i64_to_u32(row.get(2)?)?,
            chunk(row.get(3)?)?,
            chunk(row.get(4)?)?,
            u64::from(i64_to_u32(row.get(5)?)?) << 32 | u64::from(i64_to_u32(row.get(6)?)?),
        ));
    }
    let [front, back] = buffers;
    if (front_hour == NONE_REF && !front.is_empty())
        || (sweep_handles == NONE_REF && !back.is_empty())
    {
        return Err(SaveLoadError::custom(
            "supplier index entries without their buffer",
        ));
    }
    let saved = SavedSupplierRoll {
        current,
        front: match front_hour {
            NONE_REF => None,
            hour => Some((i64_to_u32(hour)?, front)),
        },
        sweep: match sweep_handles {
            NONE_REF => None,
            handles => Some(SavedSupplierSweep {
                building_handles: i64_to_u64(handles)?,
                hour: i64_to_u32(sweep_hour)?,
                next_chunk: i64_to_u64(sweep_next_chunk)?,
                entries: back,
            }),
        },
    };
    SupplierRoll::restore(saved).map_err(SaveLoadError::custom)
}

fn save_resource_deposits(
    tx: &Transaction,
    resource_deposits: &ResourceDepositSystem,
) -> SaveLoadResult<()> {
    let coal_dense = resource_deposits.clone_coal_richness_dense();
    let (resource_w, resource_h) = resource_deposits.grid_dimensions();
    let mut resource_stmt = tx.prepare(
        "INSERT INTO resource_deposit_chunks(resource_id, chunk_x, chunk_z, width_samples, height_samples, richness_blob_u16_le) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    let chunk_cols = resource_w.div_ceil(RESOURCE_DEPOSIT_CHUNK_SIZE_SAMPLES);
    let chunk_rows = resource_h.div_ceil(RESOURCE_DEPOSIT_CHUNK_SIZE_SAMPLES);
    for chunk_z in 0..chunk_rows {
        for chunk_x in 0..chunk_cols {
            let origin_x = chunk_x * RESOURCE_DEPOSIT_CHUNK_SIZE_SAMPLES;
            let origin_z = chunk_z * RESOURCE_DEPOSIT_CHUNK_SIZE_SAMPLES;
            let width_samples = (resource_w - origin_x).min(RESOURCE_DEPOSIT_CHUNK_SIZE_SAMPLES);
            let height_samples = (resource_h - origin_z).min(RESOURCE_DEPOSIT_CHUNK_SIZE_SAMPLES);
            let mut payload = Vec::with_capacity(width_samples * height_samples);
            let mut touched = false;

            for local_z in 0..height_samples {
                let row_start = (origin_z + local_z) * resource_w + origin_x;
                let row_end = row_start + width_samples;
                let row = &coal_dense[row_start..row_end];
                if !touched && row.iter().any(|value| *value != 0) {
                    touched = true;
                }
                payload.extend_from_slice(row);
            }

            if !touched {
                continue;
            }

            resource_stmt.execute(params![
                COAL_RESOURCE_ID,
                chunk_x as i64,
                chunk_z as i64,
                usize_to_i64(width_samples)?,
                usize_to_i64(height_samples)?,
                pack_u16_slice(&payload),
            ])?;
        }
    }
    Ok(())
}

fn save_resource_extraction(
    tx: &Transaction,
    resource_extraction: &ResourceExtractionSystem,
    maps: &SnapshotMaps,
) -> SaveLoadResult<()> {
    let mut site_stmt = tx.prepare(
        "INSERT INTO resource_extractor_sites(site_id, building_id, resource_id, total_reserve_units, extracted_units, reserve_density_kg_per_m2) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    let mut point_stmt = tx.prepare(
        "INSERT INTO resource_extractor_site_points(site_id, point_index, x, z) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (site_id, site) in resource_extraction.sites().iter().enumerate() {
        validate_reserve_density(site.reserve_density_kg_per_m2)?;
        validate_extractor_reserve(site.total_reserve_units, site.extracted_units)
            .map_err(SaveLoadError::custom)?;
        let saved_building_id = optional_building_to_db(site.building_idx, maps)?;
        if saved_building_id == NONE_REF {
            return Err(SaveLoadError::custom(
                "resource extractor site references missing building",
            ));
        }
        let site_id_db = usize_to_i64(site_id)?;
        site_stmt.execute(params![
            site_id_db,
            saved_building_id,
            site.resource_id.as_str(),
            site.total_reserve_units,
            site.extracted_units,
            site.reserve_density_kg_per_m2,
        ])?;
        for (point_index, point) in site.polygon_world.iter().enumerate() {
            point_stmt.execute(params![
                site_id_db,
                usize_to_i64(point_index)?,
                point.x,
                point.y,
            ])?;
        }
    }
    Ok(())
}

fn save_agriculture(
    tx: &Transaction,
    agriculture: &AgricultureSystem,
    maps: &SnapshotMaps,
) -> SaveLoadResult<()> {
    let mut site_stmt = tx.prepare(
        "INSERT INTO agriculture_field_sites(site_id, building_id, resource_id, area_m2) VALUES (?1, ?2, ?3, ?4)",
    )?;
    let mut point_stmt = tx.prepare(
        "INSERT INTO agriculture_field_site_points(site_id, point_index, x, z) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (site_id, site) in agriculture.sites().iter().enumerate() {
        let saved_building_id = optional_building_to_db(site.building_idx, maps)?;
        if saved_building_id == NONE_REF {
            return Err(SaveLoadError::custom(
                "agriculture field site references missing building",
            ));
        }
        let site_id_db = usize_to_i64(site_id)?;
        site_stmt.execute(params![
            site_id_db,
            saved_building_id,
            site.resource_id.as_str(),
            site.area_m2,
        ])?;
        for (point_index, point) in site.polygon_world.iter().enumerate() {
            point_stmt.execute(params![
                site_id_db,
                usize_to_i64(point_index)?,
                point.x,
                point.y,
            ])?;
        }
    }
    Ok(())
}

pub(super) fn load_terrain(
    conn: &Connection,
    config: &WorldConfig,
) -> SaveLoadResult<TerrainSystem> {
    let (w_raw, h_raw, blob): (i64, i64, Vec<u8>) = conn.query_row(
        "SELECT width, height, height_blob_f32_le FROM terrain_state LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let (w, h) = (i64_to_usize(w_raw)?, i64_to_usize(h_raw)?);
    if w != config.terrain_grid_width() || h != config.terrain_grid_height() {
        return Err(SaveLoadError::custom("terrain size mismatch"));
    }
    let mut t = TerrainSystem::from_world_config(config);
    let source_dense = unpack_f32_blob(&blob, w * h)?;
    t.replace_source_from_dense(&source_dense)
        .map_err(SaveLoadError::custom)?;
    Ok(t)
}

pub(super) fn load_water(
    conn: &Connection,
    config: &WorldConfig,
    ew: usize,
    eh: usize,
) -> SaveLoadResult<WaterSystem> {
    let (w_raw, h_raw, baseline_db): (i64, i64, Vec<u8>) = conn.query_row(
        "SELECT width, height, baseline_depth_blob_f32_le FROM water_state LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let (w, h) = (i64_to_usize(w_raw)?, i64_to_usize(h_raw)?);
    if w != ew || h != eh {
        return Err(SaveLoadError::custom("water size mismatch"));
    }
    let mut water = WaterSystem::from_world_config(config);
    water
        .replace_baseline_depth_from_dense(&unpack_f32_blob(&baseline_db, w * h)?)
        .map_err(SaveLoadError::custom)?;
    Ok(water)
}

pub(super) fn load_resource_deposits(
    conn: &Connection,
    config: &WorldConfig,
) -> SaveLoadResult<ResourceDepositSystem> {
    let mut resource_deposits = ResourceDepositSystem::from_world_config(config);
    let (resource_w, resource_h) = resource_deposits.grid_dimensions();
    let mut stmt = conn.prepare(
        "SELECT resource_id, chunk_x, chunk_z, width_samples, height_samples, richness_blob_u16_le FROM resource_deposit_chunks ORDER BY resource_id, chunk_z, chunk_x",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let resource_id: String = row.get(0)?;
        if resource_id != COAL_RESOURCE_ID {
            return Err(SaveLoadError::custom(format!(
                "unsupported resource deposit id '{resource_id}'"
            )));
        }
        let chunk_x = i64_to_usize(row.get(1)?)?;
        let chunk_z = i64_to_usize(row.get(2)?)?;
        let width_samples = i64_to_usize(row.get(3)?)?;
        let height_samples = i64_to_usize(row.get(4)?)?;
        let blob: Vec<u8> = row.get(5)?;

        if width_samples == 0 || height_samples == 0 {
            return Err(SaveLoadError::custom(
                "resource deposit chunk has zero-sized payload",
            ));
        }
        if width_samples > RESOURCE_DEPOSIT_CHUNK_SIZE_SAMPLES
            || height_samples > RESOURCE_DEPOSIT_CHUNK_SIZE_SAMPLES
        {
            return Err(SaveLoadError::custom(
                "resource deposit chunk exceeds canonical chunk sample span",
            ));
        }

        let origin_x = chunk_x
            .checked_mul(RESOURCE_DEPOSIT_CHUNK_SIZE_SAMPLES)
            .ok_or_else(|| SaveLoadError::custom("resource deposit chunk_x overflow"))?;
        let origin_z = chunk_z
            .checked_mul(RESOURCE_DEPOSIT_CHUNK_SIZE_SAMPLES)
            .ok_or_else(|| SaveLoadError::custom("resource deposit chunk_z overflow"))?;
        if origin_x + width_samples > resource_w || origin_z + height_samples > resource_h {
            return Err(SaveLoadError::custom(
                "resource deposit chunk falls outside world bounds",
            ));
        }

        let payload = unpack_u16_blob(&blob, width_samples * height_samples)?;
        for local_z in 0..height_samples {
            let row_start = local_z * width_samples;
            let row_end = row_start + width_samples;
            let payload_row = &payload[row_start..row_end];
            for (local_x, value) in payload_row.iter().enumerate() {
                if *value == 0 {
                    continue;
                }
                if *value > RESOURCE_RICHNESS_MAX {
                    return Err(SaveLoadError::custom(
                        "resource deposit richness exceeds canonical range",
                    ));
                }
                resource_deposits.set_coal_richness_at(
                    origin_x + local_x,
                    origin_z + local_z,
                    *value,
                );
            }
        }
    }
    Ok(resource_deposits)
}

pub(super) fn load_pending_demand_spawns(
    conn: &Connection,
) -> SaveLoadResult<VecDeque<PendingDemandSpawnAction>> {
    let mut stmt = conn.prepare(
        "SELECT due_minute, zone_type, parcel_id, asset_id, planned_day_index, planned_minute_of_day, spawn_id, permit, counted FROM pending_demand_spawns ORDER BY sequence ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            (
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
            ),
            row.get::<_, i64>(4)?,
            row.get::<_, i64>(5)?,
            row.get::<_, i64>(6)?,
            row.get::<_, Option<Vec<u8>>>(7)?,
            row.get::<_, i64>(8)? != 0,
        ))
    })?;
    let mut pending = VecDeque::new();
    for row in rows {
        let (
            (due_minute, zone_type, parcel_id, asset_id),
            planned_day,
            planned_minute,
            id,
            permit,
            counted,
        ) = row?;
        pending.push_back(PendingDemandSpawnAction {
            id: i64_to_u64(id)?,
            due_minute: i64_to_u64(due_minute)?,
            zone_type: demand_spawn_zone_from_db(zone_type)?,
            action: DemandSpawnAction {
                parcel_id: i64_to_u64(parcel_id)?,
                asset_id,
            },
            planned_day_index: i64_to_u32(planned_day)?,
            planned_minute_of_day: i64_to_u16(planned_minute)?,
            permit: unpack_spawn_permit(permit.as_deref())?,
            counted,
        });
    }
    Ok(pending)
}

// The hour's plan between its planning slots (`SIM-02-I`): each zone's upgrades (kind 0),
// despawns (kind 1) and spawns (kind 3), then the residential slot's optional homes (kind 2),
// in plan order. Building keys save their edge through the save's edge renumbering; a key whose
// edge the save drops names a building that resolves in neither game and is not saved, which
// keeps the loaded upgrade list sorted as slot 3's search needs. Spawns and optional homes
// save only their parcel and asset.
fn save_demand_hour_plan(
    tx: &Transaction,
    demand: &DemandSystem,
    maps: &SnapshotMaps,
) -> SaveLoadResult<()> {
    let mut stmt = tx.prepare(
        "INSERT INTO demand_hour_plan(sequence, kind, zone_type, parcel_id, edge_id, side, cell_x, width, depth, level, asset_id, target_asset_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
    )?;
    let mut sequence = 0_i64;
    let mut row = |kind: i64,
                   zone: ZoneType,
                   key: &DemandBuildingActionKey,
                   target: &str|
     -> SaveLoadResult<()> {
        let edge = match maps.edge_old_to_new.get(&key.edge_idx) {
            Some(&edge) => usize_to_i64(edge)?,
            None if key.edge_idx == usize::MAX => NONE_REF,
            None => return Ok(()),
        };
        stmt.execute(params![
            sequence,
            kind,
            demand_spawn_zone_to_db(zone)?,
            u64_to_i64(key.parcel_id)?,
            edge,
            i64::from(key.side),
            usize_to_i64(key.cell_x)?,
            i64::from(key.width_cells),
            i64::from(key.depth_cells),
            i64::from(key.level),
            &key.asset_id,
            target,
        ])?;
        sequence += 1;
        Ok(())
    };
    let plan = &demand.building_actions;
    for (zone, use_plan) in [
        (ZoneType::Residential, &plan.residential),
        (ZoneType::Commercial, &plan.commercial),
        (ZoneType::Industrial, &plan.industrial),
    ] {
        for upgrade in &use_plan.upgrades {
            row(0, zone, &upgrade.building, &upgrade.target_asset_id)?;
        }
        for despawn in &use_plan.despawns {
            row(1, zone, despawn, "")?;
        }
        for spawn in &use_plan.spawns {
            row(3, zone, &spawn_key(spawn), "")?;
        }
    }
    for home in &demand.optional_homes {
        row(2, ZoneType::Residential, &spawn_key(home), "")?;
    }
    Ok(())
}

// A planned spawn as a building key row: its parcel and asset, no edge.
fn spawn_key(spawn: &DemandSpawnAction) -> DemandBuildingActionKey {
    DemandBuildingActionKey {
        parcel_id: spawn.parcel_id,
        edge_idx: usize::MAX,
        side: 0,
        cell_x: 0,
        width_cells: 0,
        depth_cells: 0,
        level: 0,
        asset_id: spawn.asset_id.clone(),
    }
}

/// Loads the hour's plan [`save_demand_hour_plan`] saved into `demand`.
pub(super) fn load_demand_hour_plan(
    conn: &Connection,
    demand: &mut DemandSystem,
) -> SaveLoadResult<()> {
    let mut stmt = conn.prepare(
        "SELECT kind, zone_type, parcel_id, edge_id, side, cell_x, width, depth, level, asset_id, target_asset_id FROM demand_hour_plan ORDER BY sequence ASC",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let kind: i64 = row.get(0)?;
        let zone = demand_spawn_zone_from_db(row.get(1)?)?;
        let key = DemandBuildingActionKey {
            parcel_id: i64_to_u64(row.get(2)?)?,
            edge_idx: db_to_optional_usize(row.get(3)?)?,
            side: i64_to_i8(row.get(4)?)?,
            cell_x: i64_to_usize(row.get(5)?)?,
            width_cells: i64_to_u16(row.get(6)?)?,
            depth_cells: i64_to_u16(row.get(7)?)?,
            level: i64_to_u8(row.get(8)?)?,
            asset_id: row.get(9)?,
        };
        let plan = &mut demand.building_actions;
        let use_plan = match zone {
            ZoneType::Commercial => &mut plan.commercial,
            ZoneType::Industrial => &mut plan.industrial,
            _ => &mut plan.residential,
        };
        match kind {
            0 => use_plan.upgrades.push(DemandLevelChangeAction {
                building: key,
                target_asset_id: row.get(10)?,
            }),
            1 => use_plan.despawns.push(key),
            2 => demand.optional_homes.push(DemandSpawnAction {
                parcel_id: key.parcel_id,
                asset_id: key.asset_id,
            }),
            3 => use_plan.spawns.push(DemandSpawnAction {
                parcel_id: key.parcel_id,
                asset_id: key.asset_id,
            }),
            _ => return Err(SaveLoadError::custom("invalid demand hour plan row")),
        }
    }
    Ok(())
}

// A queued spawn's permit as saved: NULL when denied, a single 0 byte when voided, otherwise
// the door digest, profile and level, little-endian.
fn pack_spawn_permit(permit: SpawnPermit) -> Option<Vec<u8>> {
    match permit {
        SpawnPermit::Denied => None,
        SpawnPermit::Voided => Some(vec![0]),
        SpawnPermit::Granted(permit) => {
            let mut bytes = Vec::with_capacity(11);
            bytes.extend_from_slice(&permit.door.to_le_bytes());
            bytes.extend_from_slice(&permit.profile.to_le_bytes());
            bytes.push(permit.level);
            Some(bytes)
        }
    }
}

fn unpack_spawn_permit(bytes: Option<&[u8]>) -> SaveLoadResult<SpawnPermit> {
    match bytes {
        None => Ok(SpawnPermit::Denied),
        Some([0]) => Ok(SpawnPermit::Voided),
        Some(bytes) if bytes.len() == 11 => Ok(SpawnPermit::Granted(ConstructionPermit {
            door: u64::from_le_bytes(bytes[0..8].try_into().unwrap()),
            profile: u16::from_le_bytes(bytes[8..10].try_into().unwrap()),
            level: bytes[10],
        })),
        Some(_) => Err(SaveLoadError::custom("malformed construction permit")),
    }
}

pub(super) fn load_zoning(
    conn: &Connection,
    config: &WorldConfig,
    graph: &RegionGraph,
) -> SaveLoadResult<(ZoningSystem, Vec<u64>)> {
    let mut zoning = ZoningSystem::new(config);
    let mut cell_lots = super::cell_zoning::load(conn, &mut zoning, graph)?;
    let mut quarantined_parcels = Vec::new();
    let mut road_overlap_count = 0usize;
    let mut existing_overlap_count = 0usize;
    let mut frontage_out_of_bounds_count = 0usize;
    let mut stmt = conn.prepare(
        "SELECT parcel_id, edge_id, side, frontage_t, frontage_m, depth_m, profile_runtime_id, build_generation FROM zoning_parcels ORDER BY parcel_id",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let parcel_id = i64_to_usize(row.get(0)?)? as u64;
        let edge_idx = i64_to_usize(row.get(1)?)?;
        let side = i64_to_i8(row.get(2)?)?;
        let frontage_t: f32 = row.get(3)?;
        let frontage_m: f32 = row.get(4)?;
        let depth_m: f32 = row.get(5)?;
        let profile_runtime_id = i64_to_u16(row.get(6)?)?;
        let build_generation = i64_to_u32(row.get(7)?)?;
        let restored = if let Some(lot) = cell_lots.remove(&parcel_id) {
            zoning
                .restore_saved_cell_lot(
                    crate::simulation::zoning::ParcelId::from_raw(parcel_id),
                    lot,
                    edge_idx,
                    side,
                    frontage_t,
                    frontage_m,
                    depth_m,
                    profile_runtime_id,
                    graph,
                )
                .map(|()| (false, false))
        } else {
            zoning.restore_saved_parcel_from_attachment(
                parcel_id,
                edge_idx,
                side,
                frontage_t,
                frontage_m,
                depth_m,
                profile_runtime_id,
                graph,
            )
        };
        let (overlaps_road, overlaps_existing) = match restored {
            Ok(overlaps) => overlaps,
            Err(crate::simulation::zoning::ParcelPlacementError::FrontageOutOfBounds) => {
                // Older curved-run spacing could advance beyond an edge endpoint. Use the
                // existing quarantine lifecycle so no building retains an illegal attachment.
                quarantined_parcels.push(parcel_id);
                frontage_out_of_bounds_count += 1;
                continue;
            }
            Err(err) => {
                return Err(SaveLoadError::custom(format!(
                    "invalid saved parcel parcel_id={parcel_id} edge_id={edge_idx} side={side} frontage_t={frontage_t:.6}: {err:?}"
                )));
            }
        };
        zoning.restore_parcel_build_generation(parcel_id, build_generation);
        if overlaps_road || overlaps_existing {
            quarantined_parcels.push(parcel_id);
        }
        if overlaps_road {
            road_overlap_count += 1;
        }
        existing_overlap_count += usize::from(overlaps_existing);
    }
    if !cell_lots.is_empty() {
        return Err(SaveLoadError::custom("cell lot references missing parcel"));
    }
    super::cell_zoning::load_lot_queue(conn, &mut zoning)?;
    if !quarantined_parcels.is_empty() {
        crate::debug_log!(
            "save",
            "legacy_saved_parcel_quarantine road_overlaps={} existing_overlaps={} frontage_out_of_bounds={}",
            road_overlap_count,
            existing_overlap_count,
            frontage_out_of_bounds_count
        );
    }
    Ok((zoning, quarantined_parcels))
}

pub(super) fn load_buildings(
    conn: &Connection,
    registry: &crate::assets::AssetRegistry,
    profiles: &crate::simulation::zoning::profiles::ZoningProfileRegistry,
) -> SaveLoadResult<BuildingAllocator> {
    let mut allocator = BuildingAllocator::new();
    let catalog = load_runtime_economy_catalog().map_err(SaveLoadError::custom)?;
    let resource_count = catalog.resource_count();
    // col: 0=building_id 1=parcel_id 2=edge_id 3=frontage_t 4=side 5=cell_x 6=cell_y
    //      7=profile_runtime_id 8=occupancy 9=worker_count 10=service_funding_override
    //      11=revenue 12=operating_budget 13=profit_tax_budget_baseline 14=last_day_profit
    //      15=shipment_cooldown_until_slot 16=width 17=depth 18=asset_id 19=level
    //      20=construction_total_hours 21=construction_complete_slot 22=broken
    //      23=pending_redevelopment 24=rezone_grace_days_remaining 25=is_deserted
    //      26=budget_distress 27=daily_household_sales_value 28=daily_power_service_units
    //      29=daily_power_served_units 30=recent_power_service_units
    //      31=recent_power_served_units 32=recent_household_sales_value 33=support_height_m
    //      34=build_generation 35=electricity_usage 36=hourly_economy_activity
    //      37=commercial_activity_floor_scale 38=paid_workers_in_window 39=funded_job_capacity
    //      40=unpaid_cash_wages_in_window 41=service_sales_capacity
    //      42=service_sales_profile_runtime_id 43=funded_activity 44=applicant_seed
    allocator.next_applicant_seed = i64_to_u64(conn.query_row(
        "SELECT next_applicant_seed FROM building_seed_state WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?)?;
    let mut stmt = conn.prepare("SELECT building_id, parcel_id, edge_id, frontage_t, side, cell_x, cell_y, profile_runtime_id, occupancy, worker_count, service_funding_override, revenue, operating_budget, profit_tax_budget_baseline, last_day_profit, shipment_cooldown_until_slot, width, depth, asset_id, level, construction_total_hours, construction_complete_slot, broken, pending_redevelopment, rezone_grace_days_remaining, is_deserted, budget_distress, daily_household_sales_value, daily_power_service_units, daily_power_served_units, recent_power_service_units, recent_power_served_units, recent_household_sales_value, support_height_m, build_generation, electricity_usage, hourly_economy_activity, commercial_activity_floor_scale, paid_workers_in_window, funded_job_capacity, unpaid_cash_wages_in_window, service_sales_capacity, service_sales_profile_runtime_id, funded_activity, applicant_seed FROM buildings ORDER BY building_id")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let bid = i64_to_usize(row.get(0)?)?;
        if bid != allocator.buildings.len() {
            return Err(SaveLoadError::custom("non-contiguous building ids"));
        }
        let asset_id: String = row.get(18)?;
        let broken = (row.get::<_, i64>(22)? != 0) || registry.get(&asset_id).is_none();
        let economy_binding = resolve_building_economy_profile_binding(registry, &asset_id);
        let profile_runtime_id = i64_to_u16(row.get(7)?)?;
        let zone_type = profiles.zone_type_for_runtime_id(profile_runtime_id);
        let profile_kind = catalog
            .profile_by_runtime_id(economy_binding.runtime_id)
            .map(|profile| profile.kind);
        let electricity_usage: (
            crate::simulation::economy::electricity::BuildingElectricityUsage,
            crate::simulation::economy::electricity::BuildingElectricityUsage,
            f32,
        ) = serde_json::from_str(&row.get::<_, String>(35)?)
            .map_err(|err| SaveLoadError::custom(err.to_string()))?;
        if !electricity_usage.2.is_finite() {
            return Err(SaveLoadError::custom("invalid city input cost"));
        }
        for usage in [
            electricity_usage.0.workplace,
            electricity_usage.0.residential,
            electricity_usage.1.workplace,
            electricity_usage.1.residential,
        ] {
            validate_electricity_usage(usage)?;
        }
        let service_sales_capacity: f32 = row.get(41)?;
        let funded_activity: f32 = row.get(43)?;
        if ![service_sales_capacity, funded_activity]
            .iter()
            .all(|v| v.is_finite() && *v >= 0.0)
        {
            return Err(SaveLoadError::custom(
                "invalid hourly sales or census activity",
            ));
        }
        allocator.buildings.push(Building {
            build_generation: i64_to_u32(row.get(34)?)?,
            center_x: 0.0,
            center_y: 0.0,
            support_height_m: row.get(33)?,
            width_cells: i64_to_u16(row.get(16)?)?,
            depth_cells: i64_to_u16(row.get(17)?)?,
            zone_profile_runtime_id: profile_runtime_id,
            parcel_id: i64_to_u64(row.get(1)?)?,
            applicant_seed: i64_to_u64(row.get(44)?)?,
            zone_type,
            facing_dir: Vector2::ZERO,
            frontage_t: row.get(3)?,
            side_offset: 0.0,
            front_gap_m: [0.0; 3],
            is_deserted: row.get::<_, i64>(25)? != 0,
            budget_distress: row.get::<_, i64>(26)? != 0,
            edge_idx: db_to_optional_usize(row.get(2)?)?,
            side: i64_to_i8(row.get(4)?)?,
            cell_x: i64_to_usize(row.get(5)?)?,
            cell_y: i64_to_u16(row.get(6)?)?,
            occupancy: i64_to_u32(row.get(8)?)?,
            worker_count: i64_to_u32(row.get(9)?)?,
            service_funding_override: row.get::<_, f32>(10)?.clamp(-1.0, 1.0),
            asset_id,
            revenue: row.get(11)?,
            operating_budget: row.get(12)?,
            profit_tax_budget_baseline: row.get(13)?,
            last_day_profit: row.get(14)?,
            shipment_cooldown_until_slot: i64_to_u64(row.get(15)?)?,
            level: row.get::<_, i64>(19)?.clamp(1, 255) as u8,
            construction_total_hours: i64_to_u16(row.get(20)?)?,
            construction_complete_slot: i64_to_u64(row.get(21)?)?,
            broken,
            economy_profile_runtime_id: economy_binding.runtime_id,
            economy_broken: economy_binding.economy_broken,
            resource_inventory: vec![0.0; resource_count],
            // Input dependency telemetry is daily-only and starts fresh each session.
            daily_owa_input_value: 0.0,
            daily_local_input_value: 0.0,
            daily_city_funded_input_cost: electricity_usage.2,
            daily_household_sales_value: row.get(27)?,
            daily_power_service_units: row.get::<_, f32>(28)?,
            daily_electricity: electricity_usage.0,
            recent_electricity: electricity_usage.1,
            // Production sets it and electricity resets it in the same building phase
            // (`SIM-02-I`), so a save between phases holds zero.
            hourly_economy_activity: row.get(36)?,
            daily_power_served_units: row.get::<_, f32>(29)?,
            recent_power_service_units: row.get::<_, f32>(30)?,
            recent_power_served_units: row.get::<_, f32>(31)?,
            recent_household_sales_value: row.get(32)?,
            // Set at each building's phase and read until its next one.
            commercial_activity_floor_scale: row.get(37)?,
            // A service store draws its sales at its next phase by this capacity (`SIM-02-I`).
            service_sales_capacity,
            service_sales_profile_runtime_id: i64_to_u16(row.get(42)?)?,
            // Daily settlement windows span saves (`SIM-02-G`).
            paid_workers_in_window: i64_to_u32(row.get(38)?)?,
            funded_job_capacity: i64_to_u32(row.get(39)?)?,
            funded_activity,
            unpaid_cash_wages_in_window: i64_to_u32(row.get(40)?)?,
            work_area_scale: initial_work_area_scale(zone_type, profile_kind),
            pending_redevelopment: row.get::<_, i64>(23)? != 0,
            rezone_grace_days_remaining: i64_to_u8(row.get(24)?)?,
        });
    }
    {
        let mut stmt = conn.prepare("SELECT building_id,center_x,center_z,facing_x,facing_z,side_offset,gap_left,gap_center,gap_right FROM building_detached_poses ORDER BY building_id")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let index = i64_to_usize(row.get(0)?)?;
            let b = allocator
                .buildings
                .get_mut(index)
                .ok_or_else(|| SaveLoadError::custom("orphan detached building pose"))?;
            let values: [f32; 8] = [
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
            ];
            if b.edge_idx != usize::MAX
                || b.parcel_id != 0
                || b.zone_type != ZoneType::None
                || !values.into_iter().all(f32::is_finite)
                || (values[2] * values[2] + values[3] * values[3] - 1.0).abs() > 0.001
            {
                return Err(SaveLoadError::custom(
                    "invalid detached explicit building pose",
                ));
            }
            b.center_x = values[0];
            b.center_y = values[1];
            b.facing_dir = Vector2::new(values[2], values[3]);
            b.side_offset = values[4];
            b.front_gap_m = [values[5], values[6], values[7]];
        }
        if allocator
            .buildings
            .iter()
            .any(|b| b.edge_idx == usize::MAX && b.facing_dir.length_squared() < 0.99)
        {
            return Err(SaveLoadError::custom("detached building has no saved pose"));
        }
    }
    let mut stmt = conn.prepare(
        "SELECT building_id, resource_runtime_id, amount FROM building_inventories ORDER BY building_id, resource_runtime_id",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let building_id = i64_to_usize(row.get(0)?)?;
        let resource_runtime_id = i64_to_u16(row.get(1)?)?;
        let amount: f32 = row.get(2)?;
        let Some(building) = allocator.buildings.get_mut(building_id) else {
            return Err(SaveLoadError::custom(
                "building inventory references missing building",
            ));
        };
        building.set_inventory_units(resource_runtime_id, amount);
    }
    let city_input_day: String = conn.query_row(
        "SELECT city_input_day FROM electricity_day_state WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;
    allocator.city_input_day = from_json(&city_input_day)?;
    Ok(allocator)
}

pub(super) fn load_resource_extraction(
    conn: &Connection,
    building_count: usize,
) -> SaveLoadResult<ResourceExtractionSystem> {
    let mut stmt = conn.prepare(
        "SELECT site_id, building_id, resource_id, total_reserve_units, extracted_units, reserve_density_kg_per_m2 FROM resource_extractor_sites ORDER BY site_id",
    )?;
    let mut rows = stmt.query([])?;
    let mut sites = Vec::new();
    while let Some(row) = rows.next()? {
        let site_id = i64_to_usize(row.get(0)?)?;
        if site_id != sites.len() {
            return Err(SaveLoadError::custom("non-contiguous extractor site ids"));
        }
        let building_idx = i64_to_usize(row.get(1)?)?;
        if building_idx >= building_count {
            return Err(SaveLoadError::custom(
                "extractor site references missing building",
            ));
        }
        let resource_id: String = row.get(2)?;
        let total_reserve_units: f32 = row.get(3)?;
        let extracted_units: f32 = row.get(4)?;
        let reserve_density_kg_per_m2 = row.get(5)?;
        validate_reserve_density(reserve_density_kg_per_m2)?;
        validate_extractor_reserve(total_reserve_units, extracted_units)
            .map_err(SaveLoadError::custom)?;
        sites.push(ExtractorSite {
            building_idx,
            resource_id,
            polygon_world: Vec::new(),
            area_m2: 0.0,
            total_reserve_units,
            reserve_density_kg_per_m2,
            extracted_units,
        });
    }

    let mut point_stmt = conn.prepare(
        "SELECT site_id, point_index, x, z FROM resource_extractor_site_points ORDER BY site_id, point_index",
    )?;
    let mut point_rows = point_stmt.query([])?;
    while let Some(row) = point_rows.next()? {
        let site_id = i64_to_usize(row.get(0)?)?;
        if site_id >= sites.len() {
            return Err(SaveLoadError::custom(
                "extractor polygon point references missing site",
            ));
        }
        let point_index = i64_to_usize(row.get(1)?)?;
        if point_index != sites[site_id].polygon_world.len() {
            return Err(SaveLoadError::custom(
                "non-contiguous extractor polygon point ids",
            ));
        }
        sites[site_id]
            .polygon_world
            .push(Vector2::new(row.get(2)?, row.get(3)?));
    }
    for site in &mut sites {
        site.area_m2 = validate_extractor_polygon_world(&site.polygon_world).map_err(|err| {
            SaveLoadError::custom(format!("invalid extractor site polygon: {err}"))
        })?;
    }
    Ok(ResourceExtractionSystem::from_sites(sites))
}

pub(super) fn load_agriculture(
    conn: &Connection,
    building_count: usize,
) -> SaveLoadResult<AgricultureSystem> {
    let mut stmt = conn.prepare(
        "SELECT site_id, building_id, resource_id, area_m2 FROM agriculture_field_sites ORDER BY site_id",
    )?;
    let mut rows = stmt.query([])?;
    let mut sites = Vec::new();
    while let Some(row) = rows.next()? {
        let site_id = i64_to_usize(row.get(0)?)?;
        if site_id != sites.len() {
            return Err(SaveLoadError::custom("non-contiguous field site ids"));
        }
        let building_idx = i64_to_usize(row.get(1)?)?;
        if building_idx >= building_count {
            return Err(SaveLoadError::custom(
                "field site references missing building",
            ));
        }
        let area_m2: f32 = row.get(3)?;
        if !area_m2.is_finite() {
            return Err(SaveLoadError::custom(
                "field site has non-finite saved area",
            ));
        }
        sites.push(FieldSite {
            building_idx,
            resource_id: row.get(2)?,
            polygon_world: Vec::new(),
            area_m2,
        });
    }

    let mut point_stmt = conn.prepare(
        "SELECT site_id, point_index, x, z FROM agriculture_field_site_points ORDER BY site_id, point_index",
    )?;
    let mut point_rows = point_stmt.query([])?;
    while let Some(row) = point_rows.next()? {
        let site_id = i64_to_usize(row.get(0)?)?;
        if site_id >= sites.len() {
            return Err(SaveLoadError::custom(
                "field polygon point references missing site",
            ));
        }
        let point_index = i64_to_usize(row.get(1)?)?;
        if point_index != sites[site_id].polygon_world.len() {
            return Err(SaveLoadError::custom(
                "non-contiguous field polygon point ids",
            ));
        }
        sites[site_id]
            .polygon_world
            .push(Vector2::new(row.get(2)?, row.get(3)?));
    }
    for site in &mut sites {
        let computed_area_m2 = validate_field_polygon_world(&site.polygon_world)
            .map_err(|err| SaveLoadError::custom(format!("invalid field site polygon: {err}")))?;
        if (site.area_m2 - computed_area_m2).abs() > 0.1 {
            return Err(SaveLoadError::custom(
                "field site saved area does not match polygon",
            ));
        }
        site.area_m2 = computed_area_m2;
    }
    Ok(AgricultureSystem::from_sites(sites))
}

// The hour in progress: the last finished hour's electricity and service-sales totals, this
// hour's so far and the sales pool (`SIM-02-I`).
fn load_city_hour(conn: &Connection, households: &mut HouseholdSystem) -> SaveLoadResult<()> {
    let electricity = &mut households.electricity_hour;
    let sales = &mut households.service_sales;
    let (residents, hour_residents): (i64, i64) = conn.query_row(
        "SELECT electricity_dispatch, electricity_coverage, electricity_demand_kwh, electricity_capacity_kwh, electricity_coal_burned_kg, electricity_coal_stock_kg, service_residents, service_gross_revenue, service_hour_residents, service_hour_collected FROM city_hour_state WHERE singleton = 1",
        [],
        |row| {
            electricity.dispatch = row.get(0)?;
            electricity.coverage = row.get(1)?;
            electricity.demand_kwh = row.get(2)?;
            electricity.capacity_kwh = row.get(3)?;
            electricity.coal_burned_kg = row.get(4)?;
            electricity.coal_stock_kg = row.get(5)?;
            sales.gross_revenue = row.get(7)?;
            sales.hour_collected = row.get(9)?;
            Ok((row.get(6)?, row.get(8)?))
        },
    )?;
    sales.residents = i64_to_u32(residents)?;
    sales.hour_residents = i64_to_u32(hour_residents)?;
    let mut stmt = conn.prepare(
        "SELECT resource_slot, gross, capacity, drawable, undrawn, hour_capacity FROM service_sales_resources ORDER BY resource_slot",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        if i64_to_usize(row.get(0)?)? != sales.gross_by_resource.len() {
            return Err(SaveLoadError::custom(
                "non-contiguous service sales resources",
            ));
        }
        sales.gross_by_resource.push(row.get(1)?);
        sales.capacity_by_resource.push(row.get(2)?);
        sales.drawable_by_resource.push(row.get(3)?);
        sales.undrawn_by_resource.push(row.get(4)?);
        sales.hour_capacity_by_resource.push(row.get(5)?);
    }
    if !electricity.is_valid() || !sales.is_valid() {
        return Err(SaveLoadError::custom("invalid city hour state"));
    }
    Ok(())
}

// The vacancy lists' order, which claims and releases leave out of building order (`SIM-02`).
// Buildings save in dense order, so a building's saved id is its index. A stale index keeps no
// order: its next reader rebuilds it.
pub(super) fn save_vacancy_order(
    tx: &Transaction,
    buildings: &BuildingAllocator,
) -> SaveLoadResult<()> {
    let stale = buildings.vacancy_index_stale();
    tx.execute(
        "INSERT INTO building_vacancy_state(singleton, stale) VALUES (1, ?1)",
        [i64::from(stale)],
    )?;
    if stale {
        return Ok(());
    }
    let mut stmt = tx.prepare(
        "INSERT INTO building_vacancies(zone_slot, position, building_id) VALUES (?1, ?2, ?3)",
    )?;
    for (slot, list) in buildings.vacancy_index.iter().enumerate() {
        for (position, &building_idx) in list.iter().enumerate() {
            stmt.execute(params![
                usize_to_i64(slot)?,
                usize_to_i64(position)?,
                usize_to_i64(building_idx)?,
            ])?;
        }
    }
    Ok(())
}

/// Loads the saved vacancy order; the caller restores it over the index rebuilt with the live
/// asset registry (`BuildingAllocator::restore_vacancy_order`).
pub(super) fn load_vacancy_order(
    conn: &Connection,
    building_count: usize,
) -> SaveLoadResult<SavedVacancyOrder> {
    let mut saved = SavedVacancyOrder {
        stale: conn.query_row(
            "SELECT stale FROM building_vacancy_state WHERE singleton = 1",
            [],
            |row| row.get::<_, i64>(0),
        )? != 0,
        ..Default::default()
    };
    let mut stmt = conn.prepare(
        "SELECT zone_slot, position, building_id FROM building_vacancies ORDER BY zone_slot, position",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let slot = i64_to_usize(row.get(0)?)?;
        let position = i64_to_usize(row.get(1)?)?;
        let building_idx = i64_to_usize(row.get(2)?)?;
        let Some(list) = saved.lists.get_mut(slot) else {
            return Err(SaveLoadError::custom("invalid vacancy zone slot"));
        };
        if saved.stale || position != list.len() || building_idx >= building_count {
            return Err(SaveLoadError::custom("invalid vacancy order"));
        }
        list.push(building_idx);
    }
    Ok(saved)
}

pub(super) fn load_households(conn: &Connection) -> SaveLoadResult<HouseholdSystem> {
    let mut households = HouseholdSystem::new();
    let (removed, day, last_day, coal): (String, String, String, f64) = conn.query_row(
        "SELECT removed_buildings, day_totals, last_day_totals, coal_inventory_kg FROM electricity_day_state WHERE singleton = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    households.removed_electricity = from_json(&removed)?;
    households.day_totals = from_json(&day)?;
    households.last_day_totals = from_json(&last_day)?;
    households.coal_inventory_kg = coal;
    if !households.removed_electricity.is_valid() {
        return Err(SaveLoadError::custom(
            "invalid removed-building electricity account",
        ));
    }
    load_city_hour(conn, &mut households)?;
    let mut stmt = conn.prepare("SELECT household_id, home_building, budget, stock, member_count, child_count, adult_count, elder_count, consumption_rate, stock_days, replenishment_state, cooldown_hours, replenishment_failure_count, reserved_store_building_id, reserved_amount, reserved_total_cost, shopping_agent_slot, shopping_agent_generation, shopping_timeout_hours_remaining, replenishment_search_cursor, stay_failure_days, unhoused_days_elapsed, replenishment_offset_hours, unemployment_days_elapsed, power_consumption_cost, water_consumption_cost, sewage_consumption_cost, electricity_account, gross_daily_wages, employed_adults, paid_workers_in_window, shopping_has_car FROM households ORDER BY household_id")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let hid = i64_to_usize(row.get(0)?)?;
        if hid != households.households.len() {
            return Err(SaveLoadError::custom("non-contiguous household ids"));
        }
        let member_count = i64_to_u16(row.get(4)?)?;
        let child_count = i64_to_u16(row.get(5)?)?;
        let adult_count = i64_to_u16(row.get(6)?)?;
        let elder_count = i64_to_u16(row.get(7)?)?;
        if u32::from(child_count)
            .saturating_add(u32::from(adult_count))
            .saturating_add(u32::from(elder_count))
            != u32::from(member_count)
        {
            return Err(SaveLoadError::custom(
                "household age counters do not match member count",
            ));
        }
        let home_building_id = db_to_optional_usize(row.get(1)?)?;
        households.households.push(Household {
            home_building_id,
            power_arrears: Default::default(),
            budget: row.get(2)?,
            stock: row.get(3)?,
            member_count,
            child_count,
            adult_count,
            elder_count,
            gross_daily_wages: row.get(28)?,
            // Refreshed only at the household's hourly recount, so saved (`PARK-02`).
            shopping_has_car: row.get::<_, i64>(31)? != 0,
            employed_adults: i64_to_u16(row.get(29)?)?,
            consumption_rate: row.get(8)?,
            stock_days: row.get(9)?,
            replenishment_state: i64_to_u8(row.get(10)?)?,
            cooldown_hours: i64_to_u16(row.get(11)?)?,
            replenishment_failure_count: i64_to_u16(row.get(12)?)?,
            reserved_store_building_id: db_to_optional_usize(row.get(13)?)?,
            reserved_amount: row.get(14)?,
            reserved_total_cost: row.get(15)?,
            shopping_agent: handle_from_db(row.get(16)?, row.get(17)?)?,
            shopping_timeout_hours_remaining: i64_to_u16(row.get(18)?)?,
            replenishment_search_cursor: i64_to_u32(row.get(19)?)?,
            stay_failure_days: i64_to_u32(row.get(20)?)?,
            unhoused_days_elapsed: i64_to_u32(row.get(21)?)?,
            replenishment_offset_hours: i64_to_u16(row.get(22)?)?,
            unemployment_days_elapsed: i64_to_u32(row.get(23)?)?,
        });
        if !households.households[hid].gross_daily_wages.is_finite()
            || households.households[hid].gross_daily_wages < 0.0
        {
            return Err(SaveLoadError::custom("invalid household wage forecast"));
        }
        let paid: [f32; 3] = [row.get(24)?, row.get(25)?, row.get(26)?];
        if paid
            .iter()
            .any(|amount| !amount.is_finite() || *amount < 0.0)
        {
            return Err(SaveLoadError::custom("invalid household utility payment"));
        }
        households.restore_utility_payments(hid, paid);
        households.restore_paid_workers(hid, i64_to_u16(row.get(30)?)?);
        {
            let json: String = row.get(27)?;
            let (usage, arrears, unpaid, local): (
                crate::simulation::economy::electricity::ElectricityUsage,
                crate::simulation::economy::electricity::ElectricityBill,
                f32,
                f32,
            ) = serde_json::from_str(&json)
                .map_err(|err| SaveLoadError::custom(err.to_string()))?;
            validate_electricity_usage(usage)?;
            if [arrears.local, arrears.imported, unpaid, local]
                .iter()
                .any(|v| !v.is_finite() || *v < 0.0)
            {
                return Err(SaveLoadError::custom(
                    "invalid household electricity account",
                ));
            }
            households.households[hid].power_arrears = arrears;
            households.restore_electricity_account(hid, usage, unpaid, local);
        }
    }
    let mut stmt = conn.prepare(
        "SELECT kind, slot, household_slot, household_generation FROM household_requests ORDER BY kind, sequence",
    )?;
    let mut rows = stmt.query([])?;
    let mut queues = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    while let Some(row) = rows.next()? {
        let handle = handle_from_db(row.get(2)?, row.get(3)?)?;
        let kind = row.get::<_, i64>(0)?;
        if kind == 1 {
            households.exits.push_back(handle);
            continue;
        }
        let queued_slot = row
            .get::<_, Option<i64>>(1)?
            .ok_or_else(|| SaveLoadError::custom("household request without its queue slot"))?;
        let queue = match kind {
            0 => 0,
            2..=4 => kind as usize - 1,
            _ => return Err(SaveLoadError::custom("unknown household request kind")),
        };
        queues[queue].push((i64_to_u64(queued_slot)?, handle));
    }
    let [reviews, rehousing, restocks, visits] = queues;
    households.housing_reviews = RequestQueue::restore(reviews);
    households.rehousing = RequestQueue::restore(rehousing);
    households.restock_requests = RequestQueue::restore(restocks);
    households.visit_requests = RequestQueue::restore(visits);
    load_household_queue_state(conn, &mut households)?;
    Ok(households)
}

pub(super) fn load_shipments(conn: &Connection) -> SaveLoadResult<ShipmentSystem> {
    let mut logistics = ShipmentSystem::new();
    let catalog = load_runtime_economy_catalog().map_err(SaveLoadError::custom)?;
    let payload = load_runtime_economy_tuning()
        .map_err(SaveLoadError::custom)?
        .logistics
        .truck_payload_kg;
    let mut stmt = conn.prepare("SELECT shipment_id, resource_runtime_id, amount, source_endpoint_kind, source_building_id, source_border_node, destination_endpoint_kind, destination_building_id, destination_border_node, carrier_class, status, carrier_slot, total_cost, eta_hours, leg_started_hour, freight_cost, carrier_generation, cargo_loaded, trip_event_slot FROM shipments ORDER BY shipment_id")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let resource = i64_to_u16(row.get(1)?)?;
        let amount: f32 = row.get(2)?;
        if !amount.is_finite()
            || amount <= 0.0
            || amount > payload
            || !catalog
                .resource_measure(resource)
                .is_some_and(|measure| measure.unit == QuantityUnit::Kilogram)
        {
            return Err(SaveLoadError::custom(
                "saved shipment cargo must be physical kg within one vehicle's payload; oversized legacy shipments cannot be loaded",
            ));
        }
        let freight_cost: f32 = row.get(15)?;
        let total_cost: f32 = row.get(12)?;
        if !freight_cost.is_finite()
            || freight_cost < 0.0
            || !total_cost.is_finite()
            || total_cost < freight_cost
        {
            return Err(SaveLoadError::custom(
                "invalid saved freight contract price",
            ));
        }
        logistics.shipments.push(Shipment {
            id: u64::try_from(row.get::<_, i64>(0)?)
                .map_err(|_| SaveLoadError::custom("bad shipment id"))?,
            resource_runtime_id: resource,
            amount,
            source: shipment_endpoint_from_db(row.get(3)?, row.get(4)?, row.get(5)?)?,
            destination: shipment_endpoint_from_db(row.get(6)?, row.get(7)?, row.get(8)?)?,
            carrier_class: carrier_class_from_db(row.get(9)?)?,
            status: shipment_status_from_db(row.get(10)?)?,
            carrier: handle_from_db(row.get(11)?, row.get(16)?)?,
            total_cost,
            freight_cost,
            eta_hours: i64_to_u16(row.get(13)?)?,
            leg_started_hour: i64_to_u32(row.get(14)?)?,
            cargo_loaded: row.get::<_, i64>(17)? != 0,
            trip_event_slot: i64_to_u64(row.get(18)?)?,
        });
    }
    let mut failure_stmt = conn.prepare("SELECT destination_building_id, resource_runtime_id, failures, terminal FROM freight_request_failures ORDER BY destination_building_id, resource_runtime_id")?;
    let mut failure_rows = failure_stmt.query([])?;
    while let Some(row) = failure_rows.next()? {
        let destination_building_id = db_to_optional_usize(row.get(0)?)?;
        if destination_building_id == usize::MAX {
            return Err(SaveLoadError::custom(
                "freight request failure missing building id",
            ));
        }
        let key = FreightRequestKey {
            destination_building_id,
            resource_runtime_id: i64_to_u16(row.get(1)?)?,
        };
        logistics.request_failures.insert(
            key,
            FreightRequestFailure {
                failures: i64_to_u16(row.get(2)?)?,
                terminal: row.get::<_, i64>(3)? != 0,
            },
        );
    }
    let mut request_stmt = conn.prepare("SELECT slot, kind, building_slot, building_generation FROM freight_requests ORDER BY position")?;
    let mut request_rows = request_stmt.query([])?;
    while let Some(row) = request_rows.next()? {
        logistics.freight_requests.push_back(FreightRequest {
            slot: i64_to_u64(row.get(0)?)?,
            kind: FreightRequestKind::from_code(row.get(1)?)
                .ok_or_else(|| SaveLoadError::custom("unknown freight request kind"))?,
            building: handle_from_db(row.get(2)?, row.get(3)?)?,
        });
    }
    logistics.rebuild_request_pending();
    logistics.supplier_roll = load_supplier_roll(conn)?;
    Ok(logistics)
}

pub(super) fn repaint_building_occupancy(
    zoning: &mut ZoningSystem,
    allocator: &BuildingAllocator,
) -> SaveLoadResult<()> {
    zoning.clear_all_parcel_occupancy();
    for (building_idx, b) in allocator.buildings.iter().enumerate() {
        if b.parcel_id == 0 {
            continue;
        }
        if !zoning.restore_parcel_occupancy(b.parcel_id, building_idx) {
            return Err(SaveLoadError::custom("building parcel occupancy mismatch"));
        }
    }
    if zoning.parcels().iter().any(|parcel| {
        parcel.cell_lot().is_some()
            && parcel.is_available()
            && parcel.zone_profile_runtime_id() == 0
    }) {
        return Err(SaveLoadError::custom(
            "incompatible cell lot has no occupying building",
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn repair_quarantined_loaded_parcels(
    zoning: &mut ZoningSystem,
    allocator: &mut BuildingAllocator,
    households: &mut HouseholdSystem,
    logistics: &mut ShipmentSystem,
    treasury_balance: &mut f64,
    resource_extraction: &mut ResourceExtractionSystem,
    agriculture: &mut AgricultureSystem,
    agents: &mut AgentSystem,
    pending_demand_spawns: &mut VecDeque<PendingDemandSpawnAction>,
    parcel_ids: &[u64],
) -> SaveLoadResult<()> {
    if parcel_ids.is_empty() {
        return Ok(());
    }

    let invalid: HashSet<u64> = parcel_ids.iter().copied().collect();
    let pending_before = pending_demand_spawns.len();
    pending_demand_spawns.retain(|spawn| !invalid.contains(&spawn.action.parcel_id));
    let dropped_pending_spawns = pending_before - pending_demand_spawns.len();

    let mut removed_buildings = 0usize;
    while let Some(building_idx) = allocator
        .buildings
        .iter()
        .position(|building| building.parcel_id != 0 && invalid.contains(&building.parcel_id))
    {
        let last_building_idx_before_remove = allocator.buildings.len().saturating_sub(1);
        if !allocator.remove_building_for_bulldoze(
            building_idx,
            zoning,
            agents,
            households,
            logistics,
            treasury_balance,
        ) {
            return Err(SaveLoadError::custom(
                "legacy invalid parcel repair could not remove building",
            ));
        }
        resource_extraction
            .remove_building_after_swap_remove(building_idx, last_building_idx_before_remove);
        agriculture
            .remove_building_after_swap_remove(building_idx, last_building_idx_before_remove);
        removed_buildings += 1;
    }

    let removed_parcels = zoning.remove_parcels_by_raw_ids(&invalid);
    crate::debug_log!(
        "save",
        "legacy_invalid_parcel_repair removed_parcels={} removed_buildings={} dropped_pending_spawns={}",
        removed_parcels,
        removed_buildings,
        dropped_pending_spawns
    );
    Ok(())
}

fn bool_to_db(value: bool) -> i64 {
    if value { 1 } else { 0 }
}

pub(super) trait GridSystemLoader: Sized {
    fn new_with_config(config: &WorldConfig) -> Self;
    // The front and back grids.
    fn grids_mut(&mut self) -> (&mut DataGrid<f32>, &mut DataGrid<f32>);
}
impl GridSystemLoader for PollutionSystem {
    fn new_with_config(config: &WorldConfig) -> Self {
        Self::new(config)
    }
    fn grids_mut(&mut self) -> (&mut DataGrid<f32>, &mut DataGrid<f32>) {
        (&mut self.grid, &mut self.back)
    }
}
impl GridSystemLoader for NoiseSystem {
    fn new_with_config(config: &WorldConfig) -> Self {
        Self::new(config)
    }
    fn grids_mut(&mut self) -> (&mut DataGrid<f32>, &mut DataGrid<f32>) {
        (&mut self.grid, &mut self.back)
    }
}

pub(super) fn load_grid_system<T: GridSystemLoader>(
    conn: &Connection,
    config: &WorldConfig,
    table: &str,
) -> SaveLoadResult<T> {
    let raw: (i64, i64, Vec<u8>, Vec<u8>) = conn.query_row(
        &format!("SELECT width, height, grid_blob_f32_le, back_blob_f32_le FROM {table} LIMIT 1"),
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    let (w, h) = (i64_to_usize(raw.0)?, i64_to_usize(raw.1)?);
    if w != config.env_grid_width() || h != config.env_grid_height() {
        return Err(SaveLoadError::custom(format!(
            "grid size mismatch in {table}"
        )));
    }
    let mut s = T::new_with_config(config);
    let (front, back) = s.grids_mut();
    front.data = unpack_f32_blob(&raw.2, w * h)?;
    back.data = unpack_f32_blob(&raw.3, w * h)?;
    Ok(s)
}

fn validate_electricity_usage(
    usage: crate::simulation::economy::electricity::ElectricityUsage,
) -> SaveLoadResult<()> {
    if !usage.consumed_kwh.is_finite()
        || !usage.local_kwh.is_finite()
        || usage.consumed_kwh < 0.0
        || usage.local_kwh < 0.0
        || usage.local_kwh > usage.consumed_kwh + 0.001
    {
        return Err(SaveLoadError::custom("invalid electricity meter"));
    }
    Ok(())
}

fn validate_reserve_density(density: f32) -> SaveLoadResult<()> {
    if !density.is_finite() || density <= 0.0 {
        return Err(SaveLoadError::custom("invalid extractor reserve density"));
    }
    Ok(())
}

// Day-state rows are small serde records.
fn to_json<T: serde::Serialize>(value: &T) -> SaveLoadResult<String> {
    serde_json::to_string(value).map_err(|err| SaveLoadError::custom(err.to_string()))
}

fn from_json<T: serde::de::DeserializeOwned>(json: &str) -> SaveLoadResult<T> {
    serde_json::from_str(json).map_err(|err| SaveLoadError::custom(err.to_string()))
}
