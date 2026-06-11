//! Serialization for terrain, water, buildings, and zoning systems.

use crate::simulation::buildings::allocator::resolve_building_economy_profile_binding;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::core::config::WorldConfig;
use crate::simulation::economy::definitions::load_runtime_economy_catalog;
use crate::simulation::economy::demand::DemandSystem;
use crate::simulation::economy::households::{Household, HouseholdSystem};
use crate::simulation::economy::logistics::{
    CarrierClass, FreightRequestFailure, FreightRequestKey, Shipment, ShipmentEndpoint,
    ShipmentStatus, ShipmentSystem,
};
use crate::simulation::grid::data_grid::DataGrid;
use crate::simulation::grid::noise::NoiseSystem;
use crate::simulation::grid::pollution::PollutionSystem;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::water::WaterSystem;
use crate::simulation::zoning::ZoningSystem;
use godot::prelude::Vector2;
use rusqlite::{Connection, Transaction, params};

use super::schema::*;
use super::{SaveLoadError, SaveLoadResult, SnapshotMaps};
use super::{
    db_to_optional_usize, i64_to_i8, i64_to_u8, i64_to_u16, i64_to_u32, i64_to_usize,
    optional_building_to_db, pack_f32_slice, pack_flux_slice, u32_to_i64, u64_to_i64,
    unpack_f32_blob, unpack_flux_blob, usize_to_i64,
};

const SHIPMENT_ENDPOINT_BUILDING: i64 = 0;
const SHIPMENT_ENDPOINT_OWA_BORDER: i64 = 1;

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

pub(super) fn save_world(
    tx: &Transaction,
    terrain: &TerrainSystem,
    water: &WaterSystem,
    zoning: &ZoningSystem,
    buildings: &BuildingAllocator,
    households: &HouseholdSystem,
    logistics: &ShipmentSystem,
    demand: &DemandSystem,
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
        "INSERT INTO water_state(width, height, baseline_depth_blob_f32_le, dynamic_depth_blob_f32_le, velocity_blob_f32_le, flux_blob_f32x4_le) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            usize_to_i64(water.width)?,
            usize_to_i64(water.height)?,
            pack_f32_slice(&water.clone_baseline_depth_dense()),
            pack_f32_slice(&water.clone_dynamic_depth_dense()),
            pack_f32_slice(&water.clone_velocity_dense()),
            pack_flux_slice(&water.clone_flux_dense())
        ],
    )?;
    let mut ws_stmt = tx.prepare(
        "INSERT INTO water_sources(grid_x, grid_y, rate_m_per_tick) VALUES (?1, ?2, ?3)",
    )?;
    for (gx, gy, r) in water.clone_sources() {
        ws_stmt.execute(params![usize_to_i64(gx)?, usize_to_i64(gy)?, r])?;
    }

    // Demand
    let spawn_action_credit = demand.spawn_action_credit.as_array();
    let upgrade_action_credit = demand.upgrade_action_credit.as_array();
    let downgrade_action_credit = demand.downgrade_action_credit.as_array();
    let despawn_action_credit = demand.despawn_action_credit.as_array();
    let spawn_hysteresis_active = demand.spawn_hysteresis_active.as_array();
    let upgrade_hysteresis_active = demand.upgrade_hysteresis_active.as_array();
    let downgrade_hysteresis_active = demand.downgrade_hysteresis_active.as_array();
    let despawn_hysteresis_active = demand.despawn_hysteresis_active.as_array();
    tx.execute(
        "INSERT INTO demand_state(residential, commercial, industrial, households_to_admit_today, households_to_remove_today, admission_action_credit, removal_action_credit, persistent_exit_action_credit, spawn_action_credit_residential, spawn_action_credit_commercial, spawn_action_credit_industrial, upgrade_action_credit_residential, upgrade_action_credit_commercial, upgrade_action_credit_industrial, downgrade_action_credit_residential, downgrade_action_credit_commercial, downgrade_action_credit_industrial, despawn_action_credit_residential, despawn_action_credit_commercial, despawn_action_credit_industrial, spawn_hysteresis_active_residential, spawn_hysteresis_active_commercial, spawn_hysteresis_active_industrial, upgrade_hysteresis_active_residential, upgrade_hysteresis_active_commercial, upgrade_hysteresis_active_industrial, downgrade_hysteresis_active_residential, downgrade_hysteresis_active_commercial, downgrade_hysteresis_active_industrial, despawn_hysteresis_active_residential, despawn_hysteresis_active_commercial, despawn_hysteresis_active_industrial, recent_household_failure_pressure) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33)",
        params![
            demand.residential,
            demand.commercial,
            demand.industrial,
            i64::from(demand.households_to_admit_today),
            i64::from(demand.households_to_remove_today),
            demand.admission_action_credit,
            demand.removal_action_credit,
            demand.persistent_exit_action_credit,
            spawn_action_credit[0],
            spawn_action_credit[1],
            spawn_action_credit[2],
            upgrade_action_credit[0],
            upgrade_action_credit[1],
            upgrade_action_credit[2],
            downgrade_action_credit[0],
            downgrade_action_credit[1],
            downgrade_action_credit[2],
            despawn_action_credit[0],
            despawn_action_credit[1],
            despawn_action_credit[2],
            bool_to_db(spawn_hysteresis_active[0]),
            bool_to_db(spawn_hysteresis_active[1]),
            bool_to_db(spawn_hysteresis_active[2]),
            bool_to_db(upgrade_hysteresis_active[0]),
            bool_to_db(upgrade_hysteresis_active[1]),
            bool_to_db(upgrade_hysteresis_active[2]),
            bool_to_db(downgrade_hysteresis_active[0]),
            bool_to_db(downgrade_hysteresis_active[1]),
            bool_to_db(downgrade_hysteresis_active[2]),
            bool_to_db(despawn_hysteresis_active[0]),
            bool_to_db(despawn_hysteresis_active[1]),
            bool_to_db(despawn_hysteresis_active[2]),
            demand.recent_household_failure_pressure,
        ],
    )?;

    // Grids
    tx.execute(
        "INSERT INTO pollution_state(width, height, grid_blob_f32_le) VALUES (?1, ?2, ?3)",
        params![
            usize_to_i64(pollution.grid.width)?,
            usize_to_i64(pollution.grid.height)?,
            pack_f32_slice(&pollution.grid.data)
        ],
    )?;
    tx.execute(
        "INSERT INTO noise_state(width, height, grid_blob_f32_le) VALUES (?1, ?2, ?3)",
        params![
            usize_to_i64(noise.grid.width)?,
            usize_to_i64(noise.grid.height)?,
            pack_f32_slice(&noise.grid.data)
        ],
    )?;

    // Zoning parcels
    {
        let mut parcel_stmt = tx.prepare("INSERT INTO zoning_parcels(parcel_id, edge_id, side, frontage_t, frontage_m, depth_m, profile_runtime_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)")?;
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
            ])?;
        }
    }

    // Buildings
    let mut bld_stmt = tx.prepare("INSERT INTO buildings(building_id, parcel_id, edge_id, frontage_t, side, cell_x, cell_y, profile_runtime_id, occupancy, worker_count, revenue, operating_budget, profit_tax_budget_baseline, shipment_cooldown_hours, width, depth, asset_id, level, construction_total_hours, construction_remaining_hours, broken, pending_redevelopment, rezone_grace_days_remaining, is_deserted, budget_distress) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25)")?;
    let mut inventory_stmt = tx.prepare(
        "INSERT INTO building_inventories(building_id, resource_runtime_id, amount) VALUES (?1, ?2, ?3)",
    )?;
    for (old_bid, b) in buildings.buildings.iter().enumerate() {
        let saved_bid = maps
            .building_old_to_new
            .get(&old_bid)
            .copied()
            .ok_or_else(|| SaveLoadError::custom("missing building mapping"))?;
        let saved_bid_db = usize_to_i64(saved_bid)?;
        let saved_eid = maps
            .edge_old_to_new
            .get(&b.edge_idx)
            .copied()
            .ok_or_else(|| SaveLoadError::custom("missing building edge mapping"))?;
        bld_stmt.execute(params![
            saved_bid_db,
            i64::try_from(b.parcel_id)
                .map_err(|_| SaveLoadError::custom("building parcel id overflow"))?,
            usize_to_i64(saved_eid)?,
            b.frontage_t,
            i64::from(b.side),
            usize_to_i64(b.cell_x)?,
            usize_to_i64(b.cell_y as usize)?,
            i64::from(b.zone_profile_runtime_id),
            u32_to_i64(b.occupancy)?,
            u32_to_i64(b.worker_count)?,
            b.revenue,
            b.operating_budget,
            b.profit_tax_budget_baseline,
            i64::from(b.shipment_cooldown_hours),
            usize_to_i64(b.width_cells as usize)?,
            usize_to_i64(b.depth_cells as usize)?,
            &b.asset_id,
            i64::from(b.level),
            i64::from(b.construction_total_hours),
            i64::from(b.construction_remaining_hours),
            i64::from(if b.broken { 1 } else { 0 }),
            i64::from(if b.pending_redevelopment { 1 } else { 0 }),
            i64::from(b.rezone_grace_days_remaining),
            i64::from(if b.is_deserted { 1 } else { 0 }),
            i64::from(if b.budget_distress { 1 } else { 0 })
        ])?;
        for (slot, amount) in b.resource_inventory.iter().enumerate() {
            if *amount <= 0.0 {
                continue;
            }
            inventory_stmt.execute(params![saved_bid_db, i64::from(slot as u16 + 1), *amount])?;
        }
    }
    let mut household_stmt = tx.prepare("INSERT INTO households(household_id, home_building, budget, stock, member_count, child_count, adult_count, elder_count, consumption_rate, stock_days, replenishment_state, cooldown_hours, replenishment_failure_count, reserved_store_building_id, reserved_amount, reserved_total_cost, shopping_agent_id, shopping_agent_schedule_seed, shopping_timeout_hours_remaining, replenishment_search_cursor, stay_failure_days, unhoused_days_elapsed, replenishment_offset_hours, unemployment_days_elapsed) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)")?;
    for (hid, household) in households.households.iter().enumerate() {
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
            if household.shopping_agent_id == usize::MAX {
                -1_i64
            } else {
                usize_to_i64(household.shopping_agent_id)?
            },
            u32_to_i64(household.shopping_agent_schedule_seed)?,
            i64::from(household.shopping_timeout_hours_remaining),
            u32_to_i64(household.replenishment_search_cursor)?,
            i64::from(household.stay_failure_days),
            i64::from(household.unhoused_days_elapsed),
            i64::from(household.replenishment_offset_hours),
            i64::from(household.unemployment_days_elapsed),
        ])?;
    }

    let mut shipment_stmt = tx.prepare("INSERT INTO shipments(shipment_id, resource_runtime_id, amount, source_endpoint_kind, source_building_id, source_border_node, destination_endpoint_kind, destination_building_id, destination_border_node, carrier_class, status, carrier_agent_id, total_cost, tax_cost, eta_hours, queued_hours) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)")?;
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
            if shipment.carrier_agent_id == usize::MAX {
                -1_i64
            } else {
                usize_to_i64(shipment.carrier_agent_id)?
            },
            shipment.total_cost,
            shipment.tax_cost,
            i64::from(shipment.eta_hours),
            i64::from(shipment.queued_hours),
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
    t.reset_visuals_from_source();
    Ok(t)
}

pub(super) fn load_water(
    conn: &Connection,
    config: &WorldConfig,
    ew: usize,
    eh: usize,
) -> SaveLoadResult<WaterSystem> {
    let (w_raw, h_raw, baseline_db, dynamic_db, vb, fb): (i64, i64, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) = conn.query_row(
        "SELECT width, height, baseline_depth_blob_f32_le, dynamic_depth_blob_f32_le, velocity_blob_f32_le, flux_blob_f32x4_le FROM water_state LIMIT 1",
        [],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        },
    )?;
    let (w, h) = (i64_to_usize(w_raw)?, i64_to_usize(h_raw)?);
    if w != ew || h != eh {
        return Err(SaveLoadError::custom("water size mismatch"));
    }
    let mut water = WaterSystem::from_world_config(config);
    water
        .replace_baseline_depth_from_dense(&unpack_f32_blob(&baseline_db, w * h)?)
        .map_err(SaveLoadError::custom)?;
    water
        .replace_dynamic_depth_from_dense(&unpack_f32_blob(&dynamic_db, w * h)?)
        .map_err(SaveLoadError::custom)?;
    water
        .replace_velocity_from_dense(&unpack_f32_blob(&vb, w * h)?)
        .map_err(SaveLoadError::custom)?;
    water
        .replace_flux_from_dense(&unpack_flux_blob(&fb, w * h)?)
        .map_err(SaveLoadError::custom)?;
    let mut stmt =
        conn.prepare("SELECT grid_x, grid_y, rate_m_per_tick FROM water_sources ORDER BY rowid")?;
    let mut rows = stmt.query([])?;
    let mut sources = Vec::new();
    while let Some(row) = rows.next()? {
        sources.push((
            i64_to_usize(row.get(0)?)?,
            i64_to_usize(row.get(1)?)?,
            row.get(2)?,
        ));
    }
    water.replace_sources(sources);
    Ok(water)
}

pub(super) fn load_zoning(
    conn: &Connection,
    config: &WorldConfig,
    graph: &RegionGraph,
) -> SaveLoadResult<ZoningSystem> {
    let mut zoning = ZoningSystem::new(config);
    let mut stmt = conn.prepare(
        "SELECT parcel_id, edge_id, side, frontage_t, frontage_m, depth_m, profile_runtime_id FROM zoning_parcels ORDER BY parcel_id",
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
        zoning
            .restore_parcel_from_attachment(
                parcel_id,
                edge_idx,
                side,
                frontage_t,
                frontage_m,
                depth_m,
                profile_runtime_id,
                graph,
            )
            .map_err(|err| SaveLoadError::custom(format!("invalid saved parcel: {err:?}")))?;
    }
    Ok(zoning)
}

pub(super) fn load_buildings(
    conn: &Connection,
    registry: &crate::assets::AssetRegistry,
    profiles: &crate::simulation::zoning::profiles::ZoningProfileRegistry,
) -> SaveLoadResult<BuildingAllocator> {
    let mut allocator = BuildingAllocator::new();
    let resource_count = load_runtime_economy_catalog()
        .map_err(SaveLoadError::custom)?
        .resource_count();
    // col: 0=building_id 1=parcel_id 2=edge_id 3=frontage_t 4=side 5=cell_x 6=cell_y
    //      7=profile_runtime_id 8=occupancy 9=worker_count 10=revenue 11=operating_budget
    //      12=profit_tax_budget_baseline 13=shipment_cooldown_hours 14=width 15=depth
    //      16=asset_id 17=level 18=construction_total_hours 19=construction_remaining_hours
    //      20=broken 21=pending_redevelopment 22=rezone_grace_days_remaining 23=is_deserted
    //      24=budget_distress
    let mut stmt = conn.prepare("SELECT building_id, parcel_id, edge_id, frontage_t, side, cell_x, cell_y, profile_runtime_id, occupancy, worker_count, revenue, operating_budget, profit_tax_budget_baseline, shipment_cooldown_hours, width, depth, asset_id, level, construction_total_hours, construction_remaining_hours, broken, pending_redevelopment, rezone_grace_days_remaining, is_deserted, budget_distress FROM buildings ORDER BY building_id")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let bid = i64_to_usize(row.get(0)?)?;
        if bid != allocator.buildings.len() {
            return Err(SaveLoadError::custom("non-contiguous building ids"));
        }
        let asset_id: String = row.get(16)?;
        let broken = (row.get::<_, i64>(20)? != 0) || registry.get(&asset_id).is_none();
        let economy_binding = resolve_building_economy_profile_binding(registry, &asset_id);
        let profile_runtime_id = i64_to_u16(row.get(7)?)?;
        allocator.buildings.push(Building {
            center_x: 0.0,
            center_y: 0.0,
            width_cells: i64_to_usize(row.get(14)?)? as u16,
            depth_cells: i64_to_usize(row.get(15)?)? as u16,
            zone_profile_runtime_id: profile_runtime_id,
            parcel_id: i64_to_usize(row.get(1)?)? as u64,
            zone_type: profiles.zone_type_for_runtime_id(profile_runtime_id),
            facing_dir: Vector2::ZERO,
            frontage_t: row.get(3)?,
            side_offset: 0.0,
            is_deserted: row.get::<_, i64>(23)? != 0,
            budget_distress: row.get::<_, i64>(24)? != 0,
            edge_idx: i64_to_usize(row.get(2)?)?,
            side: (row.get::<_, i64>(4)?) as i8,
            cell_x: i64_to_usize(row.get(5)?)?,
            cell_y: i64_to_usize(row.get(6)?)? as u16,
            occupancy: i64_to_u32(row.get(8)?)?,
            worker_count: i64_to_u32(row.get(9)?)?,
            asset_id,
            revenue: row.get(10)?,
            operating_budget: row.get(11)?,
            profit_tax_budget_baseline: row.get(12)?,
            shipment_cooldown_hours: i64_to_u16(row.get(13)?)?,
            level: row.get::<_, i64>(17)?.clamp(1, 255) as u8,
            construction_total_hours: i64_to_u16(row.get(18)?)?,
            construction_remaining_hours: i64_to_u16(row.get(19)?)?,
            broken,
            economy_profile_runtime_id: economy_binding.runtime_id,
            economy_broken: economy_binding.economy_broken,
            resource_inventory: vec![0.0; resource_count],
            // Transient daily accumulators — not persisted; start fresh each session.
            daily_owa_input_value: 0.0,
            daily_local_input_value: 0.0,
            pending_redevelopment: row.get::<_, i64>(21)? != 0,
            rezone_grace_days_remaining: i64_to_u8(row.get(22)?)?,
        });
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
    Ok(allocator)
}

pub(super) fn load_households(conn: &Connection) -> SaveLoadResult<HouseholdSystem> {
    let mut households = HouseholdSystem::new();
    let mut stmt = conn.prepare("SELECT household_id, home_building, budget, stock, member_count, child_count, adult_count, elder_count, consumption_rate, stock_days, replenishment_state, cooldown_hours, replenishment_failure_count, reserved_store_building_id, reserved_amount, reserved_total_cost, shopping_agent_id, shopping_agent_schedule_seed, shopping_timeout_hours_remaining, replenishment_search_cursor, stay_failure_days, unhoused_days_elapsed, replenishment_offset_hours, unemployment_days_elapsed FROM households ORDER BY household_id")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let hid = i64_to_usize(row.get(0)?)?;
        if hid != households.households.len() {
            return Err(SaveLoadError::custom("non-contiguous household ids"));
        }
        let member_count = i64_to_u32(row.get(4)?)? as u16;
        let child_count = i64_to_u32(row.get(5)?)? as u16;
        let adult_count = i64_to_u32(row.get(6)?)? as u16;
        let elder_count = i64_to_u32(row.get(7)?)? as u16;
        if u32::from(child_count)
            .saturating_add(u32::from(adult_count))
            .saturating_add(u32::from(elder_count))
            != u32::from(member_count)
        {
            return Err(SaveLoadError::custom(
                "household age counters do not match member count",
            ));
        }
        households.households.push(Household {
            home_building_id: db_to_optional_usize(row.get(1)?)?,
            budget: row.get(2)?,
            stock: row.get(3)?,
            member_count,
            child_count,
            adult_count,
            elder_count,
            consumption_rate: row.get(8)?,
            stock_days: row.get(9)?,
            replenishment_state: i64_to_u8(row.get(10)?)?,
            cooldown_hours: i64_to_u16(row.get(11)?)?,
            replenishment_failure_count: i64_to_u16(row.get(12)?)?,
            reserved_store_building_id: db_to_optional_usize(row.get(13)?)?,
            reserved_amount: row.get(14)?,
            reserved_total_cost: row.get(15)?,
            shopping_agent_id: db_to_optional_usize(row.get(16)?)?,
            shopping_agent_schedule_seed: i64_to_u32(row.get(17)?)?,
            shopping_timeout_hours_remaining: i64_to_u16(row.get(18)?)?,
            replenishment_search_cursor: i64_to_u32(row.get(19)?)?,
            stay_failure_days: i64_to_u32(row.get(20)?)?,
            unhoused_days_elapsed: i64_to_u32(row.get(21)?)?,
            replenishment_offset_hours: i64_to_u16(row.get(22)?)?,
            unemployment_days_elapsed: i64_to_u32(row.get(23)?)?,
        });
    }
    Ok(households)
}

pub(super) fn load_shipments(conn: &Connection) -> SaveLoadResult<ShipmentSystem> {
    let mut logistics = ShipmentSystem::new();
    let mut stmt = conn.prepare("SELECT shipment_id, resource_runtime_id, amount, source_endpoint_kind, source_building_id, source_border_node, destination_endpoint_kind, destination_building_id, destination_border_node, carrier_class, status, carrier_agent_id, total_cost, tax_cost, eta_hours, queued_hours FROM shipments ORDER BY shipment_id")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        logistics.shipments.push(Shipment {
            id: u64::try_from(row.get::<_, i64>(0)?)
                .map_err(|_| SaveLoadError::custom("bad shipment id"))?,
            resource_runtime_id: i64_to_u16(row.get(1)?)?,
            amount: row.get(2)?,
            source: shipment_endpoint_from_db(row.get(3)?, row.get(4)?, row.get(5)?)?,
            destination: shipment_endpoint_from_db(row.get(6)?, row.get(7)?, row.get(8)?)?,
            carrier_class: carrier_class_from_db(row.get(9)?)?,
            status: shipment_status_from_db(row.get(10)?)?,
            carrier_agent_id: db_to_optional_usize(row.get(11)?)?,
            total_cost: row.get(12)?,
            tax_cost: row.get(13)?,
            eta_hours: i64_to_u16(row.get(14)?)?,
            queued_hours: i64_to_u16(row.get(15)?)?,
        });
    }
    logistics.rebuild_next_shipment_id();
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
        if !zoning.occupy_parcel(b.parcel_id, building_idx) {
            return Err(SaveLoadError::custom("building parcel occupancy mismatch"));
        }
    }
    Ok(())
}

fn bool_to_db(value: bool) -> i64 {
    if value { 1 } else { 0 }
}

pub(super) trait GridSystemLoader: Sized {
    fn new_with_config(config: &WorldConfig) -> Self;
    fn grid_mut(&mut self) -> &mut DataGrid<f32>;
}
impl GridSystemLoader for PollutionSystem {
    fn new_with_config(config: &WorldConfig) -> Self {
        Self::new(config)
    }
    fn grid_mut(&mut self) -> &mut DataGrid<f32> {
        &mut self.grid
    }
}
impl GridSystemLoader for NoiseSystem {
    fn new_with_config(config: &WorldConfig) -> Self {
        Self::new(config)
    }
    fn grid_mut(&mut self) -> &mut DataGrid<f32> {
        &mut self.grid
    }
}

pub(super) fn load_grid_system<T: GridSystemLoader>(
    conn: &Connection,
    config: &WorldConfig,
    table: &str,
) -> SaveLoadResult<T> {
    let raw: (i64, i64, Vec<u8>) = conn.query_row(
        &format!("SELECT width, height, grid_blob_f32_le FROM {table} LIMIT 1"),
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let (w, h) = (i64_to_usize(raw.0)?, i64_to_usize(raw.1)?);
    if w != config.env_grid_width() || h != config.env_grid_height() {
        return Err(SaveLoadError::custom(format!(
            "grid size mismatch in {table}"
        )));
    }
    let mut s = T::new_with_config(config);
    s.grid_mut().data = unpack_f32_blob(&raw.2, w * h)?;
    Ok(s)
}
