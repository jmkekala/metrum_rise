//! SQLite save/load infrastructure for simulation snapshots.

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::config::MapConfig;
use crate::simulation::core::time::TimeSystem;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::demand::DemandSystem;
use crate::simulation::grid::desirability::DesirabilitySystem;
use crate::simulation::grid::noise::NoiseSystem;
use crate::simulation::grid::pollution::PollutionSystem;
use crate::simulation::grid::zoning::{ZoneType, ZoningSystem};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::pathing::cch::CchGraph;
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::water::WaterSystem;
use chrono::Utc;
use rusqlite::{Connection, params};
use std::collections::{BTreeSet, HashMap};
use std::fmt::{Display, Formatter};
use std::fs;
use std::path::Path;

pub mod schema;
pub mod network;
pub mod agents;
pub mod world;
#[cfg(test)]
pub mod tests;

use schema::*;

/// View of the live simulation state for serialization.
pub(crate) struct SaveGameView<'a> {
    pub config: &'a MapConfig,
    pub time: &'a TimeSystem,
    pub terrain: &'a TerrainSystem,
    pub water: &'a WaterSystem,
    pub graph: &'a RegionGraph,
    pub zoning: &'a ZoningSystem,
    pub pollution: &'a PollutionSystem,
    pub noise: &'a NoiseSystem,
    pub demand: &'a DemandSystem,
    pub allocator: &'a BuildingAllocator,
    pub agents: &'a AgentSystem,
    pub network: &'a TransitNetwork,
}

/// Fully hydrated simulation state after loading from disk.
pub(crate) struct LoadedSimulation {
    pub config: MapConfig,
    pub time: TimeSystem,
    pub terrain: TerrainSystem,
    pub water: WaterSystem,
    pub graph: RegionGraph,
    pub transit_network: TransitNetwork,
    pub zoning: ZoningSystem,
    pub pollution: PollutionSystem,
    pub noise: NoiseSystem,
    pub desirability: DesirabilitySystem,
    pub demand: DemandSystem,
    pub allocator: BuildingAllocator,
    pub agents: AgentSystem,
}

#[derive(Debug)]
pub(crate) struct SaveLoadError(String);
pub(crate) type SaveLoadResult<T> = Result<T, SaveLoadError>;

impl SaveLoadError {
    pub fn custom(message: impl Into<String>) -> Self { Self(message.into()) }
}

impl Display for SaveLoadError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result { f.write_str(&self.0) }
}
impl std::error::Error for SaveLoadError {}
impl From<rusqlite::Error> for SaveLoadError { fn from(v: rusqlite::Error) -> Self { Self(v.to_string()) } }
impl From<std::io::Error> for SaveLoadError { fn from(v: std::io::Error) -> Self { Self(v.to_string()) } }
impl From<String> for SaveLoadError { fn from(v: String) -> Self { Self(v) } }

pub(super) struct SnapshotMaps {
    pub node_old_to_new: HashMap<u32, u32>,
    pub edge_old_to_new: HashMap<usize, usize>,
    pub building_old_to_new: HashMap<usize, usize>,
}

/// Entry point to save the live simulation state to a SQLite database.
pub(crate) fn save_to_sqlite(path: &Path, view: SaveGameView<'_>) -> SaveLoadResult<()> {
    if let Some(p) = path.parent() { if !p.as_os_str().is_empty() { fs::create_dir_all(p)?; } }
    if path.exists() { fs::remove_file(path)?; }
    let mut conn = Connection::open(path)?;
    conn.execute_batch(SCHEMA)?;
    let maps = build_snapshot_maps(view.graph, view.allocator, view.agents)?;
    let tx = conn.transaction()?;
    tx.execute("INSERT INTO save_meta(version, saved_at_unix, game_build) VALUES (?1, ?2, ?3)", params![SAVE_VERSION, Utc::now().timestamp(), env!("CARGO_PKG_VERSION")])?;
    tx.execute("INSERT INTO map_config(width_m, height_m, env_cell_m, zone_cell_m) VALUES (?1, ?2, ?3, ?4)", params![view.config.width_m, view.config.height_m, view.config.env_cell_m, view.config.zone_cell_m])?;
    tx.execute("INSERT INTO time_state(time_elapsed, speed_multiplier, current_day, seconds_per_day, agent_sim_time) VALUES (?1, ?2, ?3, ?4, ?5)", params![view.time.time_elapsed, view.time.speed_multiplier, i64::from(view.time.current_day), view.time.seconds_per_day, view.agents.sim_time])?;

    world::save_world(&tx, view.terrain, view.water, view.zoning, view.allocator, view.demand, view.pollution, view.noise, &maps)?;
    network::save_network(&tx, view.graph, &maps)?;
    agents::save_agents(&tx, view.agents, view.graph, view.network, &maps)?;

    tx.commit()?;
    Ok(())
}

/// Entry point to load a simulation state from a SQLite database.
pub(crate) fn load_from_sqlite(path: &Path) -> SaveLoadResult<LoadedSimulation> {
    let conn = Connection::open(path)?;
    let version: i64 = conn.query_row("SELECT version FROM save_meta LIMIT 1", [], |row| row.get(0))?;
    if version != SAVE_VERSION { return Err(SaveLoadError::custom("version mismatch")); }
    let config = conn.query_row("SELECT width_m, height_m, env_cell_m, zone_cell_m FROM map_config LIMIT 1", [], |r| Ok(MapConfig::new(r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
    let time_r: (f64, f32, i64, f64, f32) = conn.query_row("SELECT time_elapsed, speed_multiplier, current_day, seconds_per_day, agent_sim_time FROM time_state LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?;
    let time = TimeSystem { time_elapsed: time_r.0, speed_multiplier: time_r.1, current_day: i64_to_u32(time_r.2)?, seconds_per_day: time_r.3 };

    let mut terrain = world::load_terrain(&conn, &config)?;
    let water = world::load_water(&conn, terrain.width, terrain.height)?;
    let demand = conn.query_row("SELECT residential, commercial, industrial FROM demand_state LIMIT 1", [], |r| Ok(DemandSystem { residential: r.get(0)?, commercial: r.get(1)?, industrial: r.get(2)? }))?;
    let pollution = world::load_grid_system::<PollutionSystem>(&conn, &config, "pollution_state")?;
    let noise = world::load_grid_system::<NoiseSystem>(&conn, &config, "noise_state")?;

    let mut graph = network::load_graph(&conn)?;
    let mut zoning = world::load_zoning(&conn, &config)?;
    let mut allocator = world::load_buildings(&conn)?;
    let mut agents = agents::load_agents(&conn, time_r.4)?;

    let mut transit_network = TransitNetwork::new();
    network::rebuild_loaded_graph_runtime(&mut graph, &mut transit_network, &mut terrain);
    transit_network.lane_system.rebuild(&mut graph);

    for i in 0..agents.len() {
        let eid = agents.current_edge[i];
        let lidx = agents.current_lane_id[i];
        if eid != usize::MAX && lidx != usize::MAX {
            agents.current_lane_id[i] = transit_network.lane_system.get_lane_id(eid, lidx).unwrap_or(usize::MAX);
        }
    }

    allocator.recompute_derived_transforms(&graph, &zoning)?;
    world::repaint_building_occupancy(&mut zoning, &allocator)?;
    allocator.rebuild_zone_index();
    allocator.dirty = true;
    world::rebuild_distance_to_road(&mut zoning, &graph);
    transit_network.cch_graph = CchGraph::build(&graph);
    agents::validate_loaded_agents(&mut agents, &graph, &allocator)?;

    let mut desirability = DesirabilitySystem::new(&config);
    desirability.tick(&zoning, &pollution, &noise);

    Ok(LoadedSimulation { config, time, terrain, water, graph, transit_network, zoning, pollution, noise, desirability, demand, allocator, agents })
}

fn build_snapshot_maps(graph: &RegionGraph, allocator: &BuildingAllocator, agents: &AgentSystem) -> SaveLoadResult<SnapshotMaps> {
    let mut edge_old_to_new = HashMap::new();
    for (old, edge) in graph.edges().iter().enumerate() { if !edge.deleted { let nid = edge_old_to_new.len(); edge_old_to_new.insert(old, nid); } }
    let mut saved_nodes = BTreeSet::new();
    for edge in graph.edges() { if edge.deleted { continue; } saved_nodes.insert(network::canonical_existing_node(graph, edge.start_node)?); saved_nodes.insert(network::canonical_existing_node(graph, edge.end_node)?); }
    for i in 0..agents.len() { saved_nodes.insert(network::canonical_existing_node(graph, agents.current_node[i])?); saved_nodes.insert(network::canonical_existing_node(graph, agents.target_node[i])?); for &nid in &agents.current_path[i] { saved_nodes.insert(network::canonical_existing_node(graph, nid)?); } }
    let node_old_to_new = saved_nodes.into_iter().enumerate().map(|(n, o)| (o, n as u32)).collect();
    let mut building_old_to_new = HashMap::new();
    for (old, b) in allocator.buildings.iter().enumerate() { if !edge_old_to_new.contains_key(&b.edge_idx) { return Err(SaveLoadError::custom("bld edge missing")); } building_old_to_new.insert(old, building_old_to_new.len()); }
    Ok(SnapshotMaps { node_old_to_new, edge_old_to_new, building_old_to_new })
}

pub(super) fn pack_f32_slice(v: &[f32]) -> Vec<u8> { let mut b = Vec::with_capacity(v.len() * 4); for &x in v { b.extend_from_slice(&x.to_le_bytes()); } b }
pub(super) fn unpack_f32_blob(b: &[u8], len: usize) -> SaveLoadResult<Vec<f32>> { if b.len() != len * 4 { return Err(SaveLoadError::custom("f32 blob size mismatch")); } let mut v = Vec::with_capacity(len); for c in b.chunks_exact(4) { v.push(f32::from_le_bytes([c[0], c[1], c[2], c[3]])); } Ok(v) }
pub(super) fn pack_flux_slice(v: &[[f32; 4]]) -> Vec<u8> { let mut b = Vec::with_capacity(v.len() * 16); for q in v { for &x in q { b.extend_from_slice(&x.to_le_bytes()); } } b }
pub(super) fn unpack_flux_blob(b: &[u8], len: usize) -> SaveLoadResult<Vec<[f32; 4]>> { if b.len() != len * 16 { return Err(SaveLoadError::custom("flux blob mismatch")); } let mut v = Vec::with_capacity(len); for c in b.chunks_exact(16) { v.push([f32::from_le_bytes([c[0], c[1], c[2], c[3]]), f32::from_le_bytes([c[4], c[5], c[6], c[7]]), f32::from_le_bytes([c[8], c[9], c[10], c[11]]), f32::from_le_bytes([c[12], c[13], c[14], c[15]])]); } Ok(v) }
pub(super) fn pack_zone_slice(v: &[ZoneType]) -> Vec<u8> { v.iter().map(|&z| z as u8).collect() }
pub(super) fn unpack_zone_blob(b: &[u8], len: usize) -> SaveLoadResult<Vec<ZoneType>> { if b.len() != len { return Err(SaveLoadError::custom("zone blob mismatch")); } b.iter().map(|&v| zone_type_from_i64(i64::from(v))).collect() }

pub(super) fn optional_building_to_db(v: usize, m: &SnapshotMaps) -> SaveLoadResult<i64> { if v == usize::MAX { Ok(NONE_REF) } else { usize_to_i64(m.building_old_to_new.get(&v).copied().ok_or_else(|| SaveLoadError::custom("missing bld map"))?) } }
pub(super) fn optional_edge_to_db(v: usize, m: &SnapshotMaps) -> SaveLoadResult<i64> { if v == usize::MAX { Ok(NONE_REF) } else { usize_to_i64(m.edge_old_to_new.get(&v).copied().ok_or_else(|| SaveLoadError::custom("missing edge map"))?) } }
pub(super) fn db_to_optional_usize(v: i64) -> SaveLoadResult<usize> { if v == NONE_REF { Ok(usize::MAX) } else { i64_to_usize(v) } }
pub(super) fn usize_to_i64(v: usize) -> SaveLoadResult<i64> { i64::try_from(v).map_err(|_| SaveLoadError::custom("usize overflow")) }
pub(super) fn u32_to_i64(v: u32) -> SaveLoadResult<i64> { Ok(i64::from(v)) }
pub(super) fn u8_to_i64(v: u8) -> SaveLoadResult<i64> { Ok(i64::from(v)) }
pub(super) fn i64_to_usize(v: i64) -> SaveLoadResult<usize> { usize::try_from(v).map_err(|_| SaveLoadError::custom("usize underflow")) }
pub(super) fn i64_to_u32(v: i64) -> SaveLoadResult<u32> { u32::try_from(v).map_err(|_| SaveLoadError::custom("u32 overflow")) }
pub(super) fn i64_to_u8(v: i64) -> SaveLoadResult<u8> { u8::try_from(v).map_err(|_| SaveLoadError::custom("u8 overflow")) }
pub(super) fn i64_to_i8(v: i64) -> SaveLoadResult<i8> { i8::try_from(v).map_err(|_| SaveLoadError::custom("i8 overflow")) }
