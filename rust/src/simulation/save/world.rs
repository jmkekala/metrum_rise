//! Serialization for terrain, water, buildings, and zoning systems.

use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::core::config::MapConfig;
use crate::simulation::economy::demand::DemandSystem;
use crate::simulation::economy::households::{Household, HouseholdSystem};
use crate::simulation::economy::logistics::{Shipment, ShipmentSystem};
use crate::simulation::grid::data_grid::DataGrid;
use crate::simulation::grid::noise::NoiseSystem;
use crate::simulation::grid::pollution::PollutionSystem;
use crate::simulation::grid::zoning::ZoningSystem;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::water::WaterSystem;
use godot::prelude::Vector2;
use rusqlite::{Connection, Transaction, params};

use super::{SaveLoadError, SaveLoadResult, SnapshotMaps};
use super::schema::*;
use super::{db_to_optional_usize, i64_to_u8, i64_to_u32, i64_to_usize, optional_building_to_db, pack_f32_slice, pack_flux_slice, u32_to_i64, unpack_f32_blob, unpack_flux_blob, usize_to_i64};

pub(super) fn save_world(tx: &Transaction, terrain: &TerrainSystem, water: &WaterSystem, zoning: &ZoningSystem, buildings: &BuildingAllocator, households: &HouseholdSystem, logistics: &ShipmentSystem, demand: &DemandSystem, pollution: &PollutionSystem, noise: &NoiseSystem, maps: &SnapshotMaps) -> SaveLoadResult<()> {
    // Terrain
    tx.execute("INSERT INTO terrain_state(width, height, height_blob_f32_le) VALUES (?1, ?2, ?3)", params![usize_to_i64(terrain.width)?, usize_to_i64(terrain.height)?, pack_f32_slice(&terrain.source_data)])?;

    // Water
    tx.execute("INSERT INTO water_state(width, height, depth_blob_f32_le, velocity_blob_f32_le, flux_blob_f32x4_le) VALUES (?1, ?2, ?3, ?4, ?5)", params![usize_to_i64(water.width)?, usize_to_i64(water.height)?, pack_f32_slice(&water.depth), pack_f32_slice(&water.velocity), pack_flux_slice(&water.flux)])?;
    let mut ws_stmt = tx.prepare("INSERT INTO water_sources(grid_x, grid_y, rate_m_per_tick) VALUES (?1, ?2, ?3)")?;
    for &(gx, gy, r) in &water.sources { ws_stmt.execute(params![usize_to_i64(gx)?, usize_to_i64(gy)?, r])?; }

    // Demand
    tx.execute("INSERT INTO demand_state(residential, commercial, industrial) VALUES (?1, ?2, ?3)", params![demand.residential, demand.commercial, demand.industrial])?;

    // Grids
    tx.execute("INSERT INTO pollution_state(width, height, grid_blob_f32_le) VALUES (?1, ?2, ?3)", params![usize_to_i64(pollution.grid.width)?, usize_to_i64(pollution.grid.height)?, pack_f32_slice(&pollution.grid.data)])?;
    tx.execute("INSERT INTO noise_state(width, height, grid_blob_f32_le) VALUES (?1, ?2, ?3)", params![usize_to_i64(noise.grid.width)?, usize_to_i64(noise.grid.height)?, pack_f32_slice(&noise.grid.data)])?;

    // Zoning — serialize the flat world-grid as a single BLOB
    {
        let data: Vec<u8> = zoning.grid.data.iter().map(|&z| z as u8).collect();
        tx.execute(
            "INSERT INTO zoning_world_grid(width, height, data) VALUES (?1, ?2, ?3)",
            params![usize_to_i64(zoning.grid.width)?, usize_to_i64(zoning.grid.height)?, data],
        )?;
    }

    // Buildings
    let mut bld_stmt = tx.prepare("INSERT INTO buildings(building_id, edge_id, frontage_t, side, cell_x, cell_y, zone_type, occupancy, worker_count, stock, revenue, operating_budget, utility_service_available, shipment_cooldown_days, width, depth, asset_id, level, broken) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)")?;
    for (old_bid, b) in buildings.buildings.iter().enumerate() {
        let saved_bid = maps.building_old_to_new.get(&old_bid).copied().ok_or_else(|| SaveLoadError::custom("missing building mapping"))?;
        let saved_eid = maps.edge_old_to_new.get(&b.edge_idx).copied().ok_or_else(|| SaveLoadError::custom("missing building edge mapping"))?;
        bld_stmt.execute(params![
            usize_to_i64(saved_bid)?,
            usize_to_i64(saved_eid)?,
            b.frontage_t,
            i64::from(b.side),
            usize_to_i64(b.cell_x)?,
            usize_to_i64(b.cell_y as usize)?,
            zone_type_to_i64(b.zone_type),
            u32_to_i64(b.occupancy)?,
            u32_to_i64(b.worker_count)?,
            b.stock,
            b.revenue,
            b.operating_budget,
            i64::from(if b.utility_service_available { 1 } else { 0 }),
            i64::from(b.shipment_cooldown_days),
            usize_to_i64(b.width_cells as usize)?,
            usize_to_i64(b.depth_cells as usize)?,
            &b.asset_id,
            i64::from(b.level),
            i64::from(if b.broken { 1 } else { 0 })
        ])?;
    }
    tx.execute(
        "INSERT INTO founding_state(bootstrap_consumed) VALUES (?1)",
        params![i64::from(if buildings.founding_bootstrap_consumed { 1 } else { 0 })],
    )?;

    let mut household_stmt = tx.prepare("INSERT INTO households(household_id, home_building, budget, stock, member_count, consumption_rate, stock_days, replenishment_state, cooldown_days, reserved_store_building_id, reserved_amount, reserved_total_cost, pickup_eta_days) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)")?;
    for (hid, household) in households.households.iter().enumerate() {
        household_stmt.execute(params![
            usize_to_i64(hid)?,
            optional_building_to_db(household.home_building_id, maps)?,
            household.budget,
            household.stock,
            i64::from(household.member_count),
            household.consumption_rate,
            household.stock_days,
            i64::from(household.replenishment_state),
            i64::from(household.cooldown_days),
            optional_building_to_db(household.reserved_store_building_id, maps)?,
            household.reserved_amount,
            household.reserved_total_cost,
            i64::from(household.pickup_eta_days),
        ])?;
    }

    let mut shipment_stmt = tx.prepare("INSERT INTO shipments(shipment_id, resource_type, amount, source_kind, source_building_id, source_border_node, destination_building_id, carrier_class, status, total_cost, eta_days) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)")?;
    for (shipment_id, shipment) in logistics.shipments.iter().enumerate() {
        shipment_stmt.execute(params![
            usize_to_i64(shipment_id)?,
            i64::from(shipment.resource_type),
            shipment.amount,
            i64::from(shipment.source_kind),
            optional_building_to_db(shipment.source_building_id, maps)?,
            if shipment.source_border_node == u32::MAX {
                NONE_REF
            } else {
                let saved_node = maps
                    .node_old_to_new
                    .get(&shipment.source_border_node)
                    .copied()
                    .ok_or_else(|| SaveLoadError::custom("missing shipment border-node mapping"))?;
                usize_to_i64(saved_node as usize)?
            },
            optional_building_to_db(shipment.destination_building_id, maps)?,
            i64::from(shipment.carrier_class),
            i64::from(shipment.status),
            shipment.total_cost,
            i64::from(shipment.eta_days),
        ])?;
    }

    Ok(())
}

pub(super) fn load_terrain(conn: &Connection, config: &MapConfig) -> SaveLoadResult<TerrainSystem> {
    let (w_raw, h_raw, blob): (i64, i64, Vec<u8>) = conn.query_row("SELECT width, height, height_blob_f32_le FROM terrain_state LIMIT 1", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
    let (w, h) = (i64_to_usize(w_raw)?, i64_to_usize(h_raw)?);
    if w != config.zone_grid_width() || h != config.zone_grid_height() { return Err(SaveLoadError::custom("terrain size mismatch")); }
    let mut t = TerrainSystem::new(w, h);
    t.source_data = unpack_f32_blob(&blob, w * h)?;
    t.reset_visuals_from_source();
    Ok(t)
}

pub(super) fn load_water(conn: &Connection, ew: usize, eh: usize) -> SaveLoadResult<WaterSystem> {
    let (w_raw, h_raw, db, vb, fb): (i64, i64, Vec<u8>, Vec<u8>, Vec<u8>) = conn.query_row("SELECT width, height, depth_blob_f32_le, velocity_blob_f32_le, flux_blob_f32x4_le FROM water_state LIMIT 1", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)))?;
    let (w, h) = (i64_to_usize(w_raw)?, i64_to_usize(h_raw)?);
    if w != ew || h != eh { return Err(SaveLoadError::custom("water size mismatch")); }
    let mut water = WaterSystem::new(w, h);
    water.depth = unpack_f32_blob(&db, w * h)?;
    water.velocity = unpack_f32_blob(&vb, w * h)?;
    water.flux = unpack_flux_blob(&fb, w * h)?;
    let mut stmt = conn.prepare("SELECT grid_x, grid_y, rate_m_per_tick FROM water_sources ORDER BY rowid")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? { water.sources.push((i64_to_usize(row.get(0)?)?, i64_to_usize(row.get(1)?)?, row.get(2)?)); }
    Ok(water)
}

pub(super) fn load_zoning(conn: &Connection, config: &MapConfig) -> SaveLoadResult<ZoningSystem> {
    use crate::simulation::grid::zoning::ZoneType;
    let mut zoning = ZoningSystem::new(config);
    // Try new world-grid format first; fall back to empty grid if the table is absent (old saves).
    let result: rusqlite::Result<(i64, i64, Vec<u8>)> = conn.query_row(
        "SELECT width, height, data FROM zoning_world_grid LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    );
    if let Ok((w_raw, h_raw, blob)) = result {
        let w = i64_to_usize(w_raw)?;
        let h = i64_to_usize(h_raw)?;
        let expected = w * h;
        if blob.len() == expected {
            zoning.grid.data = blob.into_iter().map(ZoneType::from_u8).collect();
        }
    }
    // Old saves using zoning_grids are silently ignored — the player repaints zones.
    Ok(zoning)
}

pub(super) fn load_buildings(conn: &Connection, registry: &crate::assets::AssetRegistry) -> SaveLoadResult<BuildingAllocator> {
    let mut allocator = BuildingAllocator::new();
    let mut stmt = conn.prepare("SELECT building_id, edge_id, frontage_t, side, cell_x, cell_y, zone_type, occupancy, worker_count, stock, revenue, operating_budget, utility_service_available, shipment_cooldown_days, width, depth, asset_id, level, broken FROM buildings ORDER BY building_id")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let bid = i64_to_usize(row.get(0)?)?;
        if bid != allocator.buildings.len() { return Err(SaveLoadError::custom("non-contiguous building ids")); }
        let asset_id: String = row.get(16)?;
        let broken = (row.get::<_, i64>(18)? != 0) || registry.get(&asset_id).is_none();
        allocator.buildings.push(Building {
            center_x: 0.0, center_y: 0.0,
            width_cells: i64_to_usize(row.get(14)?)? as u16,
            depth_cells: i64_to_usize(row.get(15)?)? as u16,
            zone_type: zone_type_from_i64(row.get(6)?)?, facing_dir: Vector2::ZERO, frontage_t: row.get(2)?,
            side_offset: 0.0, abandoned_timer: 0,
            edge_idx: i64_to_usize(row.get(1)?)?, side: (row.get::<_, i64>(3)?) as i8,
            cell_x: i64_to_usize(row.get(4)?)?,
            cell_y: i64_to_usize(row.get(5)?)? as u16,
            occupancy: i64_to_u32(row.get(7)?)?,
            worker_count: i64_to_u32(row.get(8)?)?,
            asset_id,
            stock: row.get(9)?,
            revenue: row.get(10)?,
            operating_budget: row.get(11)?,
            utility_service_available: row.get::<_, i64>(12)? != 0,
            shipment_cooldown_days: i64_to_u8(row.get(13)?)?,
            level: row.get::<_, i64>(17)?.clamp(1, 255) as u8,
            broken,
        });
    }
    allocator.founding_bootstrap_consumed = conn.query_row(
        "SELECT bootstrap_consumed FROM founding_state LIMIT 1",
        [],
        |row| row.get::<_, i64>(0),
    )? != 0;
    Ok(allocator)
}

pub(super) fn load_households(conn: &Connection) -> SaveLoadResult<HouseholdSystem> {
    let mut households = HouseholdSystem::new();
    let mut stmt = conn.prepare("SELECT household_id, home_building, budget, stock, member_count, consumption_rate, stock_days, replenishment_state, cooldown_days, reserved_store_building_id, reserved_amount, reserved_total_cost, pickup_eta_days FROM households ORDER BY household_id")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let hid = i64_to_usize(row.get(0)?)?;
        if hid != households.households.len() {
            return Err(SaveLoadError::custom("non-contiguous household ids"));
        }
        households.households.push(Household {
            home_building_id: db_to_optional_usize(row.get(1)?)?,
            budget: row.get(2)?,
            stock: row.get(3)?,
            member_count: i64_to_u32(row.get(4)?)? as u16,
            consumption_rate: row.get(5)?,
            stock_days: row.get(6)?,
            replenishment_state: i64_to_u8(row.get(7)?)?,
            cooldown_days: i64_to_u8(row.get(8)?)?,
            reserved_store_building_id: db_to_optional_usize(row.get(9)?)?,
            reserved_amount: row.get(10)?,
            reserved_total_cost: row.get(11)?,
            pickup_eta_days: i64_to_u8(row.get(12)?)?,
        });
    }
    Ok(households)
}

pub(super) fn load_shipments(conn: &Connection) -> SaveLoadResult<ShipmentSystem> {
    let mut logistics = ShipmentSystem::new();
    let mut stmt = conn.prepare("SELECT shipment_id, resource_type, amount, source_kind, source_building_id, source_border_node, destination_building_id, carrier_class, status, total_cost, eta_days FROM shipments ORDER BY shipment_id")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let shipment_id = i64_to_usize(row.get(0)?)?;
        if shipment_id != logistics.shipments.len() {
            return Err(SaveLoadError::custom("non-contiguous shipment ids"));
        }
        let raw_border: i64 = row.get(5)?;
        logistics.shipments.push(Shipment {
            resource_type: i64_to_u8(row.get(1)?)?,
            amount: row.get(2)?,
            source_kind: i64_to_u8(row.get(3)?)?,
            source_building_id: db_to_optional_usize(row.get(4)?)?,
            source_border_node: if raw_border == NONE_REF {
                u32::MAX
            } else {
                i64_to_u32(raw_border)?
            },
            destination_building_id: db_to_optional_usize(row.get(6)?)?,
            carrier_class: i64_to_u8(row.get(7)?)?,
            status: i64_to_u8(row.get(8)?)?,
            total_cost: row.get(9)?,
            eta_days: i64_to_u8(row.get(10)?)?,
        });
    }
    Ok(logistics)
}

pub(super) fn repaint_building_occupancy(zoning: &mut ZoningSystem, allocator: &BuildingAllocator) -> SaveLoadResult<()> {
    zoning.occupied.data.fill(false);
    for b in &allocator.buildings {
        let cell_m = zoning.config.zone_cell_m;
        zoning.mark_occupied_rect(
            b.center_x,
            b.center_y,
            b.facing_dir,
            b.width_cells as f32 * cell_m,
            b.depth_cells as f32 * cell_m,
            true,
        );
    }
    Ok(())
}

pub(super) fn rebuild_distance_to_road(zoning: &mut ZoningSystem, graph: &RegionGraph) {
    zoning.update_distance_to_road(graph);
}

pub(super) trait GridSystemLoader: Sized {
    fn new_with_config(config: &MapConfig) -> Self;
    fn grid_mut(&mut self) -> &mut DataGrid<f32>;
}
impl GridSystemLoader for PollutionSystem {
    fn new_with_config(config: &MapConfig) -> Self { Self::new(config) }
    fn grid_mut(&mut self) -> &mut DataGrid<f32> { &mut self.grid }
}
impl GridSystemLoader for NoiseSystem {
    fn new_with_config(config: &MapConfig) -> Self { Self::new(config) }
    fn grid_mut(&mut self) -> &mut DataGrid<f32> { &mut self.grid }
}

pub(super) fn load_grid_system<T: GridSystemLoader>(conn: &Connection, config: &MapConfig, table: &str) -> SaveLoadResult<T> {
    let raw: (i64, i64, Vec<u8>) = conn.query_row(&format!("SELECT width, height, grid_blob_f32_le FROM {table} LIMIT 1"), [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
    let (w, h) = (i64_to_usize(raw.0)?, i64_to_usize(raw.1)?);
    if w != config.env_grid_width() || h != config.env_grid_height() { return Err(SaveLoadError::custom(format!("grid size mismatch in {table}"))); }
    let mut s = T::new_with_config(config);
    s.grid_mut().data = unpack_f32_blob(&raw.2, w * h)?;
    Ok(s)
}
