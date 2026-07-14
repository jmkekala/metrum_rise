//! SQLite save/load infrastructure for simulation snapshots.

use crate::nodes::sim::core::{CityTreasury, PendingDemandSpawnAction};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::core::time::TimeSystem;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::demand::DemandSystem;
use crate::simulation::economy::households::HouseholdSystem;
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::grid::desirability::DesirabilitySystem;
use crate::simulation::grid::noise::NoiseSystem;
use crate::simulation::grid::pollution::PollutionSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::pathing::cch::CchGraph;
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::water::WaterSystem;
use crate::simulation::zoning::ZoningSystem;
use chrono::Utc;
use rusqlite::{Connection, params};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fmt::{Display, Formatter};
use std::fs;
use std::path::Path;

pub mod agents;
pub mod network;
pub mod schema;
#[cfg(test)]
pub mod tests;
pub mod world;

use schema::*;

/// View of the live simulation state for serialization.
pub(crate) struct SaveGameView<'a> {
    pub config: &'a WorldConfig,
    pub time: &'a TimeSystem,
    pub terrain: &'a TerrainSystem,
    pub water: &'a WaterSystem,
    pub graph: &'a RegionGraph,
    pub zoning: &'a ZoningSystem,
    pub pollution: &'a PollutionSystem,
    pub noise: &'a NoiseSystem,
    pub demand: &'a DemandSystem,
    pub pending_demand_spawns: &'a VecDeque<PendingDemandSpawnAction>,
    pub allocator: &'a BuildingAllocator,
    pub households: &'a HouseholdSystem,
    pub logistics: &'a ShipmentSystem,
    pub agents: &'a AgentSystem,
    pub network: &'a TransitNetwork,
    pub treasury: &'a CityTreasury,
}

/// Fully hydrated simulation state after loading from disk.
pub(crate) struct LoadedSimulation {
    pub config: WorldConfig,
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
    pub pending_demand_spawns: VecDeque<PendingDemandSpawnAction>,
    pub allocator: BuildingAllocator,
    pub households: HouseholdSystem,
    pub logistics: ShipmentSystem,
    pub agents: AgentSystem,
    pub treasury: CityTreasury,
}

struct DemandStateRow {
    residential: f32,
    commercial: f32,
    industrial: f32,
    households_to_admit_today: i64,
    households_to_remove_today: i64,
    admission_action_credit: f32,
    removal_action_credit: f32,
    persistent_exit_action_credit: f32,
    spawn_action_credit: [f32; 3],
    upgrade_action_credit: [f32; 3],
    downgrade_action_credit: [f32; 3],
    despawn_action_credit: [f32; 3],
    spawn_hysteresis_active: [bool; 3],
    upgrade_hysteresis_active: [bool; 3],
    downgrade_hysteresis_active: [bool; 3],
    despawn_hysteresis_active: [bool; 3],
    recent_household_failure_pressure: f32,
    cheat_max_demands_enabled: bool,
}

fn demand_bool_triplet(row: &rusqlite::Row<'_>, start: usize) -> rusqlite::Result<[bool; 3]> {
    Ok([
        row.get::<_, i64>(start)? != 0,
        row.get::<_, i64>(start + 1)? != 0,
        row.get::<_, i64>(start + 2)? != 0,
    ])
}

#[derive(Debug)]
pub(crate) struct SaveLoadError(String);
pub(crate) type SaveLoadResult<T> = Result<T, SaveLoadError>;

impl SaveLoadError {
    pub fn custom(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl Display for SaveLoadError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for SaveLoadError {}
impl From<rusqlite::Error> for SaveLoadError {
    fn from(v: rusqlite::Error) -> Self {
        Self(v.to_string())
    }
}
impl From<std::io::Error> for SaveLoadError {
    fn from(v: std::io::Error) -> Self {
        Self(v.to_string())
    }
}
impl From<String> for SaveLoadError {
    fn from(v: String) -> Self {
        Self(v)
    }
}

pub(super) struct SnapshotMaps {
    pub node_old_to_new: HashMap<u32, u32>,
    pub edge_old_to_new: HashMap<usize, usize>,
    pub building_old_to_new: HashMap<usize, usize>,
}

/// Entry point to save the live simulation state to a SQLite database.
pub(crate) fn save_to_sqlite(path: &Path, view: SaveGameView<'_>) -> SaveLoadResult<()> {
    if let Some(p) = path.parent() {
        if !p.as_os_str().is_empty() {
            fs::create_dir_all(p)?;
        }
    }
    if path.exists() {
        fs::remove_file(path)?;
    }
    let mut conn = Connection::open(path)?;
    conn.execute_batch(SCHEMA)?;
    let maps = build_snapshot_maps(view.graph, view.allocator, view.agents)?;
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO save_meta(version, saved_at_unix, game_build) VALUES (?1, ?2, ?3)",
        params![
            SAVE_VERSION,
            Utc::now().timestamp(),
            env!("CARGO_PKG_VERSION")
        ],
    )?;
    tx.execute(
        "INSERT INTO world_config(width_m, height_m, terrain_cell_m, terrain_chunk_m, terrain_base_elevation_m, env_cell_m, zone_cell_m) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            view.config.width_m,
            view.config.height_m,
            view.config.terrain_cell_m,
            view.config.terrain_chunk_m,
            view.config.terrain_base_elevation_m,
            view.config.env_cell_m,
            view.config.zone_cell_m
        ],
    )?;
    tx.execute("INSERT INTO time_state(time_elapsed, speed_multiplier, day_index, minute_of_day, seconds_per_day, agent_sim_time) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", params![view.time.time_elapsed, view.time.speed_multiplier, i64::from(view.time.day_index), i64::from(view.time.minute_of_day), view.time.seconds_per_day, view.agents.sim_time])?;
    tx.execute(
        "INSERT INTO city_treasury(balance, lifetime_build_cost, lifetime_tax_revenue, last_daily_upkeep, last_daily_income_tax, last_daily_household_vat, last_daily_business_purchase_tax, last_daily_business_profit_tax, last_daily_property_tax, pending_income_tax, pending_household_vat, pending_business_purchase_tax, pending_business_profit_tax, pending_property_tax) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            view.treasury.balance,
            view.treasury.lifetime_build_cost,
            view.treasury.lifetime_tax_revenue,
            view.treasury.last_daily_upkeep,
            view.treasury.last_daily_income_tax,
            view.treasury.last_daily_household_vat,
            view.treasury.last_daily_business_purchase_tax,
            view.treasury.last_daily_business_profit_tax,
            view.treasury.last_daily_property_tax,
            view.treasury.pending_income_tax,
            view.treasury.pending_household_vat,
            view.treasury.pending_business_purchase_tax,
            view.treasury.pending_business_profit_tax,
            view.treasury.pending_property_tax,
        ],
    )?;

    world::save_world(
        &tx,
        view.terrain,
        view.water,
        view.zoning,
        view.allocator,
        view.households,
        view.logistics,
        view.demand,
        view.pending_demand_spawns,
        view.pollution,
        view.noise,
        &maps,
    )?;
    network::save_network(&tx, view.graph, &maps)?;
    agents::save_agents(&tx, view.agents, view.graph, view.network, &maps)?;

    tx.commit()?;
    Ok(())
}

/// Entry point to load a simulation state from a SQLite database.
pub(crate) fn load_from_sqlite(
    path: &Path,
    registry: &crate::assets::AssetRegistry,
) -> SaveLoadResult<LoadedSimulation> {
    let conn = Connection::open(path)?;
    let version: i64 = conn.query_row("SELECT version FROM save_meta LIMIT 1", [], |row| {
        row.get(0)
    })?;
    if version != SAVE_VERSION {
        return Err(SaveLoadError::custom("version mismatch"));
    }
    let config = conn.query_row(
        "SELECT width_m, height_m, terrain_cell_m, terrain_chunk_m, terrain_base_elevation_m, env_cell_m, zone_cell_m FROM world_config LIMIT 1",
        [],
        |r| {
            Ok(
                WorldConfig::new(r.get(0)?, r.get(1)?, r.get(5)?, r.get(6)?)
                    .with_terrain_resolution(r.get(2)?)
                    .with_chunking(r.get(3)?, r.get(4)?),
            )
        },
    )?;
    let time_r: (f64, f32, i64, i64, f64, f32) = conn.query_row("SELECT time_elapsed, speed_multiplier, day_index, minute_of_day, seconds_per_day, agent_sim_time FROM time_state LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?;
    let time = TimeSystem {
        time_elapsed: time_r.0,
        speed_multiplier: time_r.1,
        day_index: i64_to_u32(time_r.2)?,
        minute_of_day: i64_to_u16(time_r.3)?,
        seconds_per_day: time_r.4,
    };

    let mut terrain = world::load_terrain(&conn, &config)?;
    let water = world::load_water(&conn, &config, terrain.width, terrain.height)?;
    let demand_row = conn.query_row(
        "SELECT residential, commercial, industrial, households_to_admit_today, households_to_remove_today, admission_action_credit, removal_action_credit, persistent_exit_action_credit, spawn_action_credit_residential, spawn_action_credit_commercial, spawn_action_credit_industrial, upgrade_action_credit_residential, upgrade_action_credit_commercial, upgrade_action_credit_industrial, downgrade_action_credit_residential, downgrade_action_credit_commercial, downgrade_action_credit_industrial, despawn_action_credit_residential, despawn_action_credit_commercial, despawn_action_credit_industrial, spawn_hysteresis_active_residential, spawn_hysteresis_active_commercial, spawn_hysteresis_active_industrial, upgrade_hysteresis_active_residential, upgrade_hysteresis_active_commercial, upgrade_hysteresis_active_industrial, downgrade_hysteresis_active_residential, downgrade_hysteresis_active_commercial, downgrade_hysteresis_active_industrial, despawn_hysteresis_active_residential, despawn_hysteresis_active_commercial, despawn_hysteresis_active_industrial, recent_household_failure_pressure, cheat_max_demands_enabled FROM demand_state LIMIT 1",
        [],
        |r| {
            Ok(DemandStateRow {
                residential: r.get(0)?,
                commercial: r.get(1)?,
                industrial: r.get(2)?,
                households_to_admit_today: r.get(3)?,
                households_to_remove_today: r.get(4)?,
                admission_action_credit: r.get(5)?,
                removal_action_credit: r.get(6)?,
                persistent_exit_action_credit: r.get(7)?,
                spawn_action_credit: [r.get(8)?, r.get(9)?, r.get(10)?],
                upgrade_action_credit: [r.get(11)?, r.get(12)?, r.get(13)?],
                downgrade_action_credit: [r.get(14)?, r.get(15)?, r.get(16)?],
                despawn_action_credit: [r.get(17)?, r.get(18)?, r.get(19)?],
                spawn_hysteresis_active: demand_bool_triplet(r, 20)?,
                upgrade_hysteresis_active: demand_bool_triplet(r, 23)?,
                downgrade_hysteresis_active: demand_bool_triplet(r, 26)?,
                despawn_hysteresis_active: demand_bool_triplet(r, 29)?,
                recent_household_failure_pressure: r.get(32)?,
                cheat_max_demands_enabled: r.get::<_, i64>(33)? != 0,
            })
        },
    )?;
    let demand = DemandSystem::with_persisted_state(
        demand_row.residential,
        demand_row.commercial,
        demand_row.industrial,
        i64_to_u32(demand_row.households_to_admit_today)?,
        i64_to_u32(demand_row.households_to_remove_today)?,
        demand_row.admission_action_credit,
        demand_row.removal_action_credit,
        demand_row.persistent_exit_action_credit,
        demand_row.spawn_action_credit,
        demand_row.upgrade_action_credit,
        demand_row.downgrade_action_credit,
        demand_row.despawn_action_credit,
        demand_row.spawn_hysteresis_active,
        demand_row.upgrade_hysteresis_active,
        demand_row.downgrade_hysteresis_active,
        demand_row.despawn_hysteresis_active,
        demand_row.recent_household_failure_pressure,
        demand_row.cheat_max_demands_enabled,
    );
    let pending_demand_spawns = world::load_pending_demand_spawns(&conn)?;
    let pollution = world::load_grid_system::<PollutionSystem>(&conn, &config, "pollution_state")?;
    let noise = world::load_grid_system::<NoiseSystem>(&conn, &config, "noise_state")?;

    let mut graph = network::load_graph(&conn)?;
    let mut zoning = world::load_zoning(&conn, &config, &graph)?;
    let mut allocator = world::load_buildings(&conn, registry, &zoning.profiles)?;
    let households = world::load_households(&conn)?;
    let logistics = world::load_shipments(&conn)?;
    let mut agents = agents::load_agents(&conn, time_r.5)?;

    let mut transit_network =
        TransitNetwork::new_with_world_terrain_chunk_span(config.terrain_chunk_m);
    network::rebuild_loaded_graph_runtime(&mut graph, &mut transit_network, &mut terrain);
    transit_network.lane_system.rebuild(&mut graph);

    for i in 0..agents.len() {
        let eid = agents.current_edge[i];
        let lidx = agents.current_lane_id[i];
        if eid != usize::MAX && lidx != usize::MAX {
            agents.current_lane_id[i] = transit_network
                .lane_system
                .get_lane_id(eid, lidx)
                .unwrap_or(usize::MAX);
        }
    }
    agents::validate_loaded_planned_lane_ids(&mut agents, transit_network.lane_system.lanes.len());

    allocator.recompute_derived_transforms(&graph, &zoning)?;
    allocator.rebuild_entrance_cache(&graph, &transit_network.lane_system);
    world::repaint_building_occupancy(&mut zoning, &allocator)?;
    allocator.rebuild_zone_index();
    allocator.dirty = true;
    transit_network.cch_graph = CchGraph::build(&graph);
    agents::validate_loaded_agents(&mut agents, &graph, &allocator)?;

    let mut desirability = DesirabilitySystem::new(&config);
    desirability.tick(&zoning, &pollution, &noise);

    let treasury_row: (f64, f64, f64, f64, f64, f64, f64, f64, f64, f64, f64, f64, f64, f64) = conn.query_row(
        "SELECT balance, lifetime_build_cost, lifetime_tax_revenue, last_daily_upkeep, last_daily_income_tax, last_daily_household_vat, last_daily_business_purchase_tax, last_daily_business_profit_tax, last_daily_property_tax, pending_income_tax, pending_household_vat, pending_business_purchase_tax, pending_business_profit_tax, pending_property_tax FROM city_treasury LIMIT 1",
        [],
        |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
                r.get(9)?,
                r.get(10)?,
                r.get(11)?,
                r.get(12)?,
                r.get(13)?,
            ))
        },
    )?;
    let treasury = CityTreasury {
        balance: treasury_row.0,
        lifetime_build_cost: treasury_row.1,
        lifetime_tax_revenue: treasury_row.2,
        last_daily_upkeep: treasury_row.3,
        last_daily_income_tax: treasury_row.4,
        last_daily_household_vat: treasury_row.5,
        last_daily_business_purchase_tax: treasury_row.6,
        last_daily_business_profit_tax: treasury_row.7,
        last_daily_property_tax: treasury_row.8,
        pending_income_tax: treasury_row.9,
        pending_household_vat: treasury_row.10,
        pending_business_purchase_tax: treasury_row.11,
        pending_business_profit_tax: treasury_row.12,
        pending_property_tax: treasury_row.13,
    };

    Ok(LoadedSimulation {
        config,
        time,
        terrain,
        water,
        graph,
        transit_network,
        zoning,
        pollution,
        noise,
        desirability,
        demand,
        pending_demand_spawns,
        allocator,
        households,
        logistics,
        agents,
        treasury,
    })
}

fn build_snapshot_maps(
    graph: &RegionGraph,
    allocator: &BuildingAllocator,
    agents: &AgentSystem,
) -> SaveLoadResult<SnapshotMaps> {
    let mut edge_old_to_new = HashMap::new();
    for (old, edge) in graph.edges().iter().enumerate() {
        if !edge.deleted {
            let nid = edge_old_to_new.len();
            edge_old_to_new.insert(old, nid);
        }
    }
    let mut saved_nodes = BTreeSet::new();
    for edge in graph.edges() {
        if edge.deleted {
            continue;
        }
        saved_nodes.insert(network::canonical_existing_node(graph, edge.start_node)?);
        saved_nodes.insert(network::canonical_existing_node(graph, edge.end_node)?);
    }
    for i in 0..agents.len() {
        if agents.current_node[i] != u32::MAX {
            saved_nodes.insert(network::canonical_existing_node(
                graph,
                agents.current_node[i],
            )?);
        }
        if agents.planned_attach_node[i] != u32::MAX {
            saved_nodes.insert(network::canonical_existing_node(
                graph,
                agents.planned_attach_node[i],
            )?);
        }
        if agents.planned_detach_node[i] != u32::MAX {
            saved_nodes.insert(network::canonical_existing_node(
                graph,
                agents.planned_detach_node[i],
            )?);
        }
        for &nid in &agents.current_path[i] {
            saved_nodes.insert(network::canonical_existing_node(graph, nid)?);
        }
    }
    let node_old_to_new = saved_nodes
        .into_iter()
        .enumerate()
        .map(|(n, o)| (o, n as u32))
        .collect();
    let mut building_old_to_new = HashMap::new();
    for (old, b) in allocator.buildings.iter().enumerate() {
        if !edge_old_to_new.contains_key(&b.edge_idx) {
            return Err(SaveLoadError::custom("bld edge missing"));
        }
        building_old_to_new.insert(old, building_old_to_new.len());
    }
    Ok(SnapshotMaps {
        node_old_to_new,
        edge_old_to_new,
        building_old_to_new,
    })
}

pub(super) fn pack_f32_slice(v: &[f32]) -> Vec<u8> {
    let mut b = Vec::with_capacity(v.len() * 4);
    for &x in v {
        b.extend_from_slice(&x.to_le_bytes());
    }
    b
}
pub(super) fn unpack_f32_blob(b: &[u8], len: usize) -> SaveLoadResult<Vec<f32>> {
    if b.len() != len * 4 {
        return Err(SaveLoadError::custom("f32 blob size mismatch"));
    }
    let mut v = Vec::with_capacity(len);
    for c in b.chunks_exact(4) {
        v.push(f32::from_le_bytes([c[0], c[1], c[2], c[3]]));
    }
    Ok(v)
}
pub(super) fn optional_building_to_db(v: usize, m: &SnapshotMaps) -> SaveLoadResult<i64> {
    if v == usize::MAX {
        Ok(NONE_REF)
    } else {
        usize_to_i64(
            m.building_old_to_new
                .get(&v)
                .copied()
                .ok_or_else(|| SaveLoadError::custom("missing bld map"))?,
        )
    }
}
pub(super) fn optional_edge_to_db(v: usize, m: &SnapshotMaps) -> SaveLoadResult<i64> {
    if v == usize::MAX {
        Ok(NONE_REF)
    } else {
        usize_to_i64(
            m.edge_old_to_new
                .get(&v)
                .copied()
                .ok_or_else(|| SaveLoadError::custom("missing edge map"))?,
        )
    }
}
pub(super) fn optional_node_to_db(
    graph: &RegionGraph,
    v: u32,
    m: &SnapshotMaps,
) -> SaveLoadResult<i64> {
    if v == u32::MAX {
        Ok(NONE_REF)
    } else {
        Ok(i64::from(
            m.node_old_to_new
                .get(&network::canonical_existing_node(graph, v)?)
                .copied()
                .ok_or_else(|| SaveLoadError::custom("missing node map"))?,
        ))
    }
}
pub(super) fn db_to_optional_usize(v: i64) -> SaveLoadResult<usize> {
    if v == NONE_REF {
        Ok(usize::MAX)
    } else {
        i64_to_usize(v)
    }
}
pub(super) fn db_to_optional_u32(v: i64) -> SaveLoadResult<u32> {
    if v == NONE_REF {
        Ok(u32::MAX)
    } else {
        i64_to_u32(v)
    }
}
pub(super) fn optional_u64_to_db(v: u64) -> SaveLoadResult<i64> {
    if v == u64::MAX {
        Ok(NONE_REF)
    } else {
        u64_to_i64(v)
    }
}
pub(super) fn db_to_optional_u64(v: i64) -> SaveLoadResult<u64> {
    if v == NONE_REF {
        Ok(u64::MAX)
    } else {
        u64::try_from(v).map_err(|_| SaveLoadError::custom("u64 underflow"))
    }
}
pub(super) fn usize_to_i64(v: usize) -> SaveLoadResult<i64> {
    i64::try_from(v).map_err(|_| SaveLoadError::custom("usize overflow"))
}
pub(super) fn u64_to_i64(v: u64) -> SaveLoadResult<i64> {
    i64::try_from(v).map_err(|_| SaveLoadError::custom("u64 overflow"))
}
pub(super) fn i64_to_u64(v: i64) -> SaveLoadResult<u64> {
    u64::try_from(v).map_err(|_| SaveLoadError::custom("u64 underflow"))
}
pub(super) fn u32_to_i64(v: u32) -> SaveLoadResult<i64> {
    Ok(i64::from(v))
}
pub(super) fn i64_to_usize(v: i64) -> SaveLoadResult<usize> {
    usize::try_from(v).map_err(|_| SaveLoadError::custom("usize underflow"))
}
pub(super) fn i64_to_u32(v: i64) -> SaveLoadResult<u32> {
    u32::try_from(v).map_err(|_| SaveLoadError::custom("u32 overflow"))
}
pub(super) fn i64_to_u16(v: i64) -> SaveLoadResult<u16> {
    u16::try_from(v).map_err(|_| SaveLoadError::custom("u16 overflow"))
}
pub(super) fn i64_to_u8(v: i64) -> SaveLoadResult<u8> {
    u8::try_from(v).map_err(|_| SaveLoadError::custom("u8 overflow"))
}
pub(super) fn i64_to_i8(v: i64) -> SaveLoadResult<i8> {
    i8::try_from(v).map_err(|_| SaveLoadError::custom("i8 overflow"))
}
