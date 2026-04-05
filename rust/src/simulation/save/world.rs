//! Serialization for terrain, water, buildings, and zoning systems.

use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::core::config::MapConfig;
use crate::simulation::economy::demand::DemandSystem;
use crate::simulation::grid::data_grid::DataGrid;
use crate::simulation::grid::noise::NoiseSystem;
use crate::simulation::grid::pollution::PollutionSystem;
use crate::simulation::grid::zoning::{EdgeZoning, ZoningSystem};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::water::WaterSystem;
use godot::prelude::Vector2;
use rusqlite::{Connection, Transaction, params};

use super::{SaveLoadError, SaveLoadResult, SnapshotMaps};
use super::schema::*;
use super::{i64_to_u32, i64_to_usize, pack_f32_slice, pack_flux_slice, pack_zone_slice, u32_to_i64, unpack_f32_blob, unpack_flux_blob, unpack_zone_blob, usize_to_i64};

pub(super) fn save_world(tx: &Transaction, terrain: &TerrainSystem, water: &WaterSystem, zoning: &ZoningSystem, buildings: &BuildingAllocator, demand: &DemandSystem, pollution: &PollutionSystem, noise: &NoiseSystem, maps: &SnapshotMaps) -> SaveLoadResult<()> {
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

    // Zoning
    let mut zone_stmt = tx.prepare("INSERT INTO zoning_grids(edge_id, cells_long, left_depth, right_depth, left_zone_blob_u8, right_zone_blob_u8) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")?;
    for (&old_eid, grid) in &zoning.edge_grids {
        let Some(&saved_eid) = maps.edge_old_to_new.get(&old_eid) else { continue; };
        zone_stmt.execute(params![usize_to_i64(saved_eid)?, usize_to_i64(grid.cells_long)?, usize_to_i64(grid.left_depth)?, usize_to_i64(grid.right_depth)?, pack_zone_slice(&grid.left_side), pack_zone_slice(&grid.right_side)])?;
    }

    // Buildings
    let mut bld_stmt = tx.prepare("INSERT INTO buildings(building_id, edge_id, frontage_t, side, cell_x, cell_y, zone_type, occupancy, width, depth, asset_id, level) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)")?;
    for (old_bid, b) in buildings.buildings.iter().enumerate() {
        let saved_bid = maps.building_old_to_new.get(&old_bid).copied().ok_or_else(|| SaveLoadError::custom("missing building mapping"))?;
        let saved_eid = maps.edge_old_to_new.get(&b.edge_idx).copied().ok_or_else(|| SaveLoadError::custom("missing building edge mapping"))?;
        bld_stmt.execute(params![usize_to_i64(saved_bid)?, usize_to_i64(saved_eid)?, b.frontage_t, i64::from(b.side), usize_to_i64(b.cell_x)?, usize_to_i64(b.cell_y as usize)?, zone_type_to_i64(b.zone_type), u32_to_i64(b.occupancy)?, usize_to_i64(b.width_cells as usize)?, usize_to_i64(b.depth_cells as usize)?, &b.asset_id, i64::from(b.level)])?;
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
    let mut zoning = ZoningSystem::new(config);
    let mut stmt = conn.prepare("SELECT edge_id, cells_long, left_depth, right_depth, left_zone_blob_u8, right_zone_blob_u8 FROM zoning_grids ORDER BY edge_id")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let eid = i64_to_usize(row.get(0)?)?;
        let cl  = i64_to_usize(row.get(1)?)?;
        let ld  = i64_to_usize(row.get(2)?)?;
        let rd  = i64_to_usize(row.get(3)?)?;
        let lcc = cl * ld;
        let rcc = cl * rd;
        zoning.edge_grids.insert(eid, EdgeZoning {
            left_side:      unpack_zone_blob(&row.get::<_, Vec<u8>>(4)?, lcc)?,
            right_side:     unpack_zone_blob(&row.get::<_, Vec<u8>>(5)?, rcc)?,
            left_occupied:  vec![false; lcc],
            right_occupied: vec![false; rcc],
            left_blocked:   vec![false; lcc],
            right_blocked:  vec![false; rcc],
            left_block_depth:  vec![0; cl],
            right_block_depth: vec![0; cl],
            left_block_id:  vec![0; cl],
            right_block_id: vec![0; cl],
            cells_long: cl,
            left_depth: ld,
            right_depth: rd,
        });
    }
    Ok(zoning)
}

pub(super) fn load_buildings(conn: &Connection) -> SaveLoadResult<BuildingAllocator> {
    let mut allocator = BuildingAllocator::new();
    let mut stmt = conn.prepare("SELECT building_id, edge_id, frontage_t, side, cell_x, cell_y, zone_type, occupancy, width, depth, asset_id, level FROM buildings ORDER BY building_id")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let bid = i64_to_usize(row.get(0)?)?;
        if bid != allocator.buildings.len() { return Err(SaveLoadError::custom("non-contiguous building ids")); }
        allocator.buildings.push(Building {
            center_x: 0.0, center_y: 0.0,
            width_cells: i64_to_usize(row.get(8)?)? as u16,
            depth_cells: i64_to_usize(row.get(9)?)? as u16,
            zone_type: zone_type_from_i64(row.get(6)?)?, facing_dir: Vector2::ZERO, frontage_t: row.get(2)?,
            side_offset: 0.0, abandoned_timer: 0,
            edge_idx: i64_to_usize(row.get(1)?)?, side: (row.get::<_, i64>(3)?) as i8,
            cell_x: i64_to_usize(row.get(4)?)?,
            cell_y: i64_to_usize(row.get(5)?)? as u16,
            occupancy: i64_to_u32(row.get(7)?)?,
            asset_id: row.get(10)?,
            level: row.get::<_, i64>(11)?.clamp(1, 255) as u8,
        });
    }
    Ok(allocator)
}

pub(super) fn repaint_building_occupancy(zoning: &mut ZoningSystem, allocator: &BuildingAllocator) -> SaveLoadResult<()> {
    for grid in zoning.edge_grids.values_mut() { grid.left_occupied.fill(false); grid.right_occupied.fill(false); }
    for b in &allocator.buildings {
        for dx in 0..b.width_cells as usize {
            for dy in 0..b.depth_cells as usize {
                zoning.set_occupied(b.edge_idx, b.side, b.cell_x + dx, b.cell_y as usize + dy, true);
            }
        }
    }
    Ok(())
}

pub(super) fn rebuild_zoning_obstructions(zoning: &mut ZoningSystem, graph: &RegionGraph) {
    let eids: Vec<usize> = zoning.edge_grids.keys().copied().collect();
    for eid in eids { if eid < graph.edge_count() && !graph.edge(eid).deleted { zoning.recalculate_obstructions(eid, graph); } }
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
