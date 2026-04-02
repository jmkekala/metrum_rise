//! SQLite save/load for the live simulation world.
//!
//! The live simulation keeps soft-deleted edges, derived caches, and runtime indices in memory.
//! Save/load does not mutate that running state. Instead it writes a compact canonical snapshot to
//! SQLite and rebuilds derived caches when loading.

use crate::config::ZONING_DEPTH;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::core::config::MapConfig;
use crate::simulation::core::time::TimeSystem;
use crate::simulation::economy::agents::{
    Agent, AgentSystem, MODE_CAR, TRANSIT_IDLE, TRANSIT_IMMIGRATING, TRANSIT_INTERSECTION,
    TRANSIT_ON_ROAD,
};
use crate::simulation::economy::demand::DemandSystem;
use crate::simulation::grid::data_grid::DataGrid;
use crate::simulation::grid::desirability::DesirabilitySystem;
use crate::simulation::grid::noise::NoiseSystem;
use crate::simulation::grid::pollution::PollutionSystem;
use crate::simulation::grid::zoning::{EdgeZoning, ZoneType, ZoningSystem};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{EdgeClass, NodeType, TransitType};
use crate::simulation::pathing::cch::CchGraph;
use crate::simulation::pathing::cost::CostCalculator;

use crate::simulation::terrain::TerrainSystem;
use crate::simulation::water::WaterSystem;
use chrono::Utc;
use godot::prelude::{Vector2, Vector3};
use rusqlite::{Connection, params};
use std::collections::{BTreeSet, HashMap};
use std::fmt::{Display, Formatter};
use std::fs;
use std::path::Path;

const SAVE_VERSION: i64 = 3;
const NONE_REF: i64 = -1;
const SCHEMA: &str = r#"
CREATE TABLE save_meta(
    version INTEGER NOT NULL,
    saved_at_unix INTEGER NOT NULL,
    game_build TEXT NOT NULL
);
CREATE TABLE map_config(
    width_m REAL NOT NULL,
    height_m REAL NOT NULL,
    env_cell_m REAL NOT NULL,
    zone_cell_m REAL NOT NULL
);
CREATE TABLE time_state(
    time_elapsed REAL NOT NULL,
    speed_multiplier REAL NOT NULL,
    current_day INTEGER NOT NULL,
    seconds_per_day REAL NOT NULL,
    agent_sim_time REAL NOT NULL
);
CREATE TABLE terrain_state(
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    height_blob_f32_le BLOB NOT NULL
);
CREATE TABLE water_state(
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    depth_blob_f32_le BLOB NOT NULL,
    velocity_blob_f32_le BLOB NOT NULL,
    flux_blob_f32x4_le BLOB NOT NULL
);
CREATE TABLE water_sources(
    grid_x INTEGER NOT NULL,
    grid_y INTEGER NOT NULL,
    rate_m_per_tick REAL NOT NULL
);
CREATE TABLE demand_state(
    residential REAL NOT NULL,
    commercial REAL NOT NULL,
    industrial REAL NOT NULL
);
CREATE TABLE pollution_state(
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    grid_blob_f32_le BLOB NOT NULL
);
CREATE TABLE noise_state(
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    grid_blob_f32_le BLOB NOT NULL
);
CREATE TABLE network_nodes(
    node_id INTEGER PRIMARY KEY,
    x REAL NOT NULL,
    y REAL NOT NULL,
    z REAL NOT NULL,
    node_type INTEGER NOT NULL
);
CREATE TABLE network_edges(
    edge_id INTEGER PRIMARY KEY,
    start_node INTEGER NOT NULL,
    end_node INTEGER NOT NULL,
    primary_type INTEGER NOT NULL,
    allowed_types INTEGER NOT NULL,
    class INTEGER NOT NULL,
    width REAL NOT NULL,
    fwd_lanes INTEGER NOT NULL,
    bkw_lanes INTEGER NOT NULL,
    speed_limit REAL NOT NULL,
    base_cost REAL NOT NULL,
    physical_length REAL NOT NULL,
    current_congestion REAL NOT NULL,
    start_clip REAL NOT NULL,
    end_clip REAL NOT NULL
);
CREATE TABLE network_edge_geometry(
    edge_id INTEGER NOT NULL,
    point_index INTEGER NOT NULL,
    x REAL NOT NULL,
    y REAL NOT NULL,
    z REAL NOT NULL,
    physical INTEGER NOT NULL,
    PRIMARY KEY(edge_id, physical, point_index)
);
CREATE TABLE lane_connections(
    node_id INTEGER NOT NULL,
    from_edge INTEGER NOT NULL,
    from_lane INTEGER NOT NULL,
    to_edge INTEGER NOT NULL,
    to_lane INTEGER NOT NULL
);
CREATE TABLE zoning_grids(
    edge_id INTEGER PRIMARY KEY,
    cells_long INTEGER NOT NULL,
    left_zone_blob_u8 BLOB NOT NULL,
    right_zone_blob_u8 BLOB NOT NULL
);
CREATE TABLE buildings(
    building_id INTEGER PRIMARY KEY,
    edge_id INTEGER NOT NULL,
    frontage_t REAL NOT NULL,
    side INTEGER NOT NULL,
    cell_x INTEGER NOT NULL,
    cell_y INTEGER NOT NULL,
    zone_type INTEGER NOT NULL,
    occupancy INTEGER NOT NULL,
    width INTEGER NOT NULL,
    depth INTEGER NOT NULL,
    frontage_node INTEGER NOT NULL,
    variant INTEGER NOT NULL
);
CREATE TABLE agents(
    agent_id INTEGER PRIMARY KEY,
    home_building INTEGER NOT NULL,
    work_building INTEGER NOT NULL,
    current_building INTEGER NOT NULL,
    target_building INTEGER NOT NULL,
    current_node INTEGER NOT NULL,
    target_node INTEGER NOT NULL,
    current_edge INTEGER NOT NULL,
    current_lane_id INTEGER NOT NULL,
    lane_distance REAL NOT NULL,
    pos_x REAL NOT NULL,
    pos_y REAL NOT NULL,
    is_visible INTEGER NOT NULL,
    activity INTEGER NOT NULL,
    transit INTEGER NOT NULL,
    transit_mode INTEGER NOT NULL,
    pedestrian_side INTEGER NOT NULL,
    happiness REAL NOT NULL,
    money REAL NOT NULL,
    journey_start_time REAL NOT NULL,
    has_car INTEGER NOT NULL,
    vehicle_type INTEGER NOT NULL,
    current_path_index INTEGER NOT NULL
);
CREATE TABLE agent_path_nodes(
    agent_id INTEGER NOT NULL,
    step_index INTEGER NOT NULL,
    node_id INTEGER NOT NULL,
    PRIMARY KEY(agent_id, step_index)
);
CREATE TABLE agent_ped_steps(
    agent_id INTEGER NOT NULL,
    step_index INTEGER NOT NULL,
    edge_id INTEGER NOT NULL,
    forward INTEGER NOT NULL,
    side INTEGER NOT NULL,
    PRIMARY KEY(agent_id, step_index)
);
"#;

/// Read-only view of the current simulation state for SQLite serialization.
pub(crate) struct SaveGameView<'a> {
    /// Current map configuration.
    pub config: &'a MapConfig,
    /// Time subsystem state.
    pub time: &'a TimeSystem,
    /// Terrain heightmap.
    pub terrain: &'a TerrainSystem,
    /// Water simulation state.
    pub water: &'a WaterSystem,
    /// Compactable road graph.
    pub graph: &'a RegionGraph,
    /// Zoning intent state.
    pub zoning: &'a ZoningSystem,
    /// Pollution grid.
    pub pollution: &'a PollutionSystem,
    /// Noise grid.
    pub noise: &'a NoiseSystem,
    /// Demand state.
    pub demand: &'a DemandSystem,
    /// Building allocator.
    pub allocator: &'a BuildingAllocator,
    /// Agent system.
    pub agents: &'a AgentSystem,
    /// Transit network for lane mapping.
    pub network: &'a TransitNetwork,
}

/// Fully hydrated simulation state returned by SQLite load.
pub(crate) struct LoadedSimulation {
    /// Loaded map configuration.
    pub config: MapConfig,
    /// Loaded time subsystem.
    pub time: TimeSystem,
    /// Loaded terrain.
    pub terrain: TerrainSystem,
    /// Loaded water state.
    pub water: WaterSystem,
    /// Loaded compact road graph.
    pub graph: RegionGraph,
    /// Rebuilt transit network runtime state.
    pub transit_network: TransitNetwork,
    /// Loaded zoning intent with rebuilt occupancy/blocked caches.
    pub zoning: ZoningSystem,
    /// Loaded pollution state.
    pub pollution: PollutionSystem,
    /// Loaded noise state.
    pub noise: NoiseSystem,
    /// Rebuilt desirability.
    pub desirability: DesirabilitySystem,
    /// Loaded demand state.
    pub demand: DemandSystem,
    /// Loaded buildings with derived transforms rebuilt.
    pub allocator: BuildingAllocator,
    /// Loaded agents with travel-state validation applied.
    pub agents: AgentSystem,
}

#[derive(Debug)]
pub(crate) struct SaveLoadError(String);

type SaveLoadResult<T> = Result<T, SaveLoadError>;

impl SaveLoadError {
    fn custom(message: impl Into<String>) -> Self {
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
    fn from(value: rusqlite::Error) -> Self {
        Self(value.to_string())
    }
}

impl From<std::io::Error> for SaveLoadError {
    fn from(value: std::io::Error) -> Self {
        Self(value.to_string())
    }
}

impl From<String> for SaveLoadError {
    fn from(value: String) -> Self {
        Self(value)
    }
}

struct SnapshotMaps {
    node_old_to_new: HashMap<u32, u32>,
    edge_old_to_new: HashMap<usize, usize>,
    building_old_to_new: HashMap<usize, usize>,
}

pub(crate) fn save_to_sqlite(path: &Path, view: SaveGameView<'_>) -> SaveLoadResult<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
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
        "INSERT INTO map_config(width_m, height_m, env_cell_m, zone_cell_m) VALUES (?1, ?2, ?3, ?4)",
        params![
            view.config.width_m,
            view.config.height_m,
            view.config.env_cell_m,
            view.config.zone_cell_m
        ],
    )?;
    tx.execute(
        "INSERT INTO time_state(time_elapsed, speed_multiplier, current_day, seconds_per_day, agent_sim_time)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            view.time.time_elapsed,
            view.time.speed_multiplier,
            i64::from(view.time.current_day),
            view.time.seconds_per_day,
            view.agents.sim_time
        ],
    )?;
    tx.execute(
        "INSERT INTO terrain_state(width, height, height_blob_f32_le) VALUES (?1, ?2, ?3)",
        params![
            usize_to_i64(view.terrain.width)?,
            usize_to_i64(view.terrain.height)?,
            pack_f32_slice(&view.terrain.source_data)
        ],
    )?;
    tx.execute(
        "INSERT INTO water_state(width, height, depth_blob_f32_le, velocity_blob_f32_le, flux_blob_f32x4_le)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            usize_to_i64(view.water.width)?,
            usize_to_i64(view.water.height)?,
            pack_f32_slice(&view.water.depth),
            pack_f32_slice(&view.water.velocity),
            pack_flux_slice(&view.water.flux)
        ],
    )?;
    tx.execute(
        "INSERT INTO demand_state(residential, commercial, industrial) VALUES (?1, ?2, ?3)",
        params![
            view.demand.residential,
            view.demand.commercial,
            view.demand.industrial
        ],
    )?;
    tx.execute(
        "INSERT INTO pollution_state(width, height, grid_blob_f32_le) VALUES (?1, ?2, ?3)",
        params![
            usize_to_i64(view.pollution.grid.width)?,
            usize_to_i64(view.pollution.grid.height)?,
            pack_f32_slice(&view.pollution.grid.data)
        ],
    )?;
    tx.execute(
        "INSERT INTO noise_state(width, height, grid_blob_f32_le) VALUES (?1, ?2, ?3)",
        params![
            usize_to_i64(view.noise.grid.width)?,
            usize_to_i64(view.noise.grid.height)?,
            pack_f32_slice(&view.noise.grid.data)
        ],
    )?;

    {
        let mut stmt = tx.prepare(
            "INSERT INTO water_sources(grid_x, grid_y, rate_m_per_tick) VALUES (?1, ?2, ?3)",
        )?;
        for &(grid_x, grid_y, rate) in &view.water.sources {
            stmt.execute(params![usize_to_i64(grid_x)?, usize_to_i64(grid_y)?, rate])?;
        }
    }

    {
        let mut node_rows: Vec<(u32, u32)> = maps
            .node_old_to_new
            .iter()
            .map(|(&old_id, &new_id)| (new_id, old_id))
            .collect();
        node_rows.sort_by_key(|&(new_id, _)| new_id);

        let mut stmt = tx.prepare(
            "INSERT INTO network_nodes(node_id, x, y, z, node_type) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for (saved_node_id, old_node_id) in node_rows {
            let node = &view.graph.nodes[old_node_id as usize];
            stmt.execute(params![
                i64::from(saved_node_id),
                node.pos.x,
                node.pos.y,
                node.pos.z,
                node_type_to_i64(node.node_type)
            ])?;
        }
    }

    {
        let mut stmt = tx.prepare(
            "INSERT INTO network_edges(
                edge_id, start_node, end_node, primary_type, allowed_types, class, width, fwd_lanes,
                bkw_lanes, speed_limit, base_cost, physical_length, current_congestion, start_clip,
                end_clip
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        )?;

        for (old_edge_id, edge) in view.graph.edges.iter().enumerate() {
            let Some(&saved_edge_id) = maps.edge_old_to_new.get(&old_edge_id) else {
                continue;
            };
            let start_node = canonical_existing_node(view.graph, edge.start_node)?;
            let end_node = canonical_existing_node(view.graph, edge.end_node)?;
            let saved_start = maps
                .node_old_to_new
                .get(&start_node)
                .copied()
                .ok_or_else(|| SaveLoadError::custom("missing saved start node mapping"))?;
            let saved_end = maps
                .node_old_to_new
                .get(&end_node)
                .copied()
                .ok_or_else(|| SaveLoadError::custom("missing saved end node mapping"))?;

            stmt.execute(params![
                usize_to_i64(saved_edge_id)?,
                i64::from(saved_start),
                i64::from(saved_end),
                transit_type_to_i64(edge.primary_type),
                i64::from(edge.allowed_types),
                edge_class_to_i64(edge.class),
                edge.width,
                i64::from(edge.fwd_lanes),
                i64::from(edge.bkw_lanes),
                edge.speed_limit,
                edge.base_cost,
                edge.physical_length,
                edge.current_congestion,
                edge.start_clip,
                edge.end_clip
            ])?;
        }
    }

    {
        let mut stmt = tx.prepare(
            "INSERT INTO network_edge_geometry(edge_id, point_index, x, y, z, physical)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;

        for (old_edge_id, edge) in view.graph.edges.iter().enumerate() {
            let Some(&saved_edge_id) = maps.edge_old_to_new.get(&old_edge_id) else {
                continue;
            };

            for (point_index, point) in edge.geometry.iter().enumerate() {
                stmt.execute(params![
                    usize_to_i64(saved_edge_id)?,
                    usize_to_i64(point_index)?,
                    point.x,
                    point.y,
                    point.z,
                    false
                ])?;
            }
            for (point_index, point) in edge.physical_geometry.iter().enumerate() {
                stmt.execute(params![
                    usize_to_i64(saved_edge_id)?,
                    usize_to_i64(point_index)?,
                    point.x,
                    point.y,
                    point.z,
                    true
                ])?;
            }
        }
    }

    {
        let mut stmt = tx.prepare(
            "INSERT INTO lane_connections(node_id, from_edge, from_lane, to_edge, to_lane)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;

        for (&old_node_id, &saved_node_id) in &maps.node_old_to_new {
            let node = &view.graph.nodes[old_node_id as usize];
            for (&(from_edge, from_lane), targets) in &node.lane_connections {
                let Some(&saved_from_edge) = maps.edge_old_to_new.get(&from_edge) else {
                    continue;
                };
                for &(to_edge, to_lane) in targets {
                    let Some(&saved_to_edge) = maps.edge_old_to_new.get(&to_edge) else {
                        continue;
                    };
                    stmt.execute(params![
                        i64::from(saved_node_id),
                        usize_to_i64(saved_from_edge)?,
                        i64::from(from_lane),
                        usize_to_i64(saved_to_edge)?,
                        i64::from(to_lane)
                    ])?;
                }
            }
        }
    }

    {
        let mut stmt = tx.prepare(
            "INSERT INTO zoning_grids(edge_id, cells_long, left_zone_blob_u8, right_zone_blob_u8)
             VALUES (?1, ?2, ?3, ?4)",
        )?;
        for (&old_edge_id, grid) in &view.zoning.edge_grids {
            let Some(&saved_edge_id) = maps.edge_old_to_new.get(&old_edge_id) else {
                continue;
            };
            stmt.execute(params![
                usize_to_i64(saved_edge_id)?,
                usize_to_i64(grid.cells_long)?,
                pack_zone_slice(&grid.left_side),
                pack_zone_slice(&grid.right_side)
            ])?;
        }
    }

    {
        let mut stmt = tx.prepare(
            "INSERT INTO buildings(
                building_id, edge_id, frontage_t, side, cell_x, cell_y, zone_type, occupancy, width, depth,
                frontage_node, variant
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        )?;

        for (old_building_id, building) in view.allocator.buildings.iter().enumerate() {
            let saved_building_id = maps
                .building_old_to_new
                .get(&old_building_id)
                .copied()
                .ok_or_else(|| SaveLoadError::custom("missing saved building mapping"))?;
            let saved_edge_id = maps
                .edge_old_to_new
                .get(&building.edge_idx)
                .copied()
                .ok_or_else(|| SaveLoadError::custom("building edge missing from saved graph"))?;
            let saved_frontage_node = maps
                .node_old_to_new
                .get(&building.frontage_node)
                .copied()
                .ok_or_else(|| SaveLoadError::custom("building frontage_node missing from saved graph"))?;

            stmt.execute(params![
                usize_to_i64(saved_building_id)?,
                usize_to_i64(saved_edge_id)?,
                building.frontage_t,
                i64::from(building.side),
                usize_to_i64(building.cell_x)?,
                u8_to_i64(building.cell_y)?,
                zone_type_to_i64(building.zone_type),
                u32_to_i64(building.occupancy)?,
                u8_to_i64(building.width_cells)?,
                u8_to_i64(building.depth_cells)?,
                u32_to_i64(saved_frontage_node)?,
                u8_to_i64(building.variant)?
            ])?;
        }
    }

    {
        let mut agent_stmt = tx.prepare(
            "INSERT INTO agents(
                agent_id, home_building, work_building, current_building, target_building, current_node,
                target_node, current_edge, current_lane_id, lane_distance, pos_x, pos_y, is_visible,
                activity, transit, transit_mode, pedestrian_side, happiness, money, journey_start_time,
                has_car, vehicle_type, current_path_index
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19,
                ?20, ?21, ?22, ?23
             )",
        )?;
        let mut car_path_stmt = tx.prepare(
            "INSERT INTO agent_path_nodes(agent_id, step_index, node_id) VALUES (?1, ?2, ?3)",
        )?;

        for agent_id in 0..view.agents.len() {
            let current_node =
                canonical_existing_node(view.graph, view.agents.current_node[agent_id])?;
            let target_node =
                canonical_existing_node(view.graph, view.agents.target_node[agent_id])?;
            let saved_current_node = maps
                .node_old_to_new
                .get(&current_node)
                .copied()
                .ok_or_else(|| SaveLoadError::custom("missing saved current node mapping"))?;
            let saved_target_node = maps
                .node_old_to_new
                .get(&target_node)
                .copied()
                .ok_or_else(|| SaveLoadError::custom("missing saved target node mapping"))?;

            agent_stmt.execute(params![
                usize_to_i64(agent_id)?,
                optional_building_to_db(view.agents.home_building[agent_id], &maps)?,
                optional_building_to_db(view.agents.work_building[agent_id], &maps)?,
                optional_building_to_db(view.agents.current_building[agent_id], &maps)?,
                optional_building_to_db(view.agents.target_building[agent_id], &maps)?,
                i64::from(saved_current_node),
                i64::from(saved_target_node),
                optional_edge_to_db(view.agents.current_edge[agent_id], &maps)?,
                if view.agents.current_lane_id[agent_id] != usize::MAX {
                    let lane_idx = view.network.lane_system.lanes[view.agents.current_lane_id[agent_id]].lane_idx;
                    Some(lane_idx as i64)
                } else {
                    Some(-1)
                },
                view.agents.lane_distance[agent_id],
                view.agents.pos_x[agent_id],
                view.agents.pos_y[agent_id],
                view.agents.is_visible[agent_id],
                i64::from(view.agents.activity[agent_id]),
                i64::from(view.agents.transit[agent_id]),
                i64::from(view.agents.transit_mode[agent_id]),
                0_i64, // Deprecated pedestrian_side
                view.agents.happiness[agent_id],
                view.agents.money[agent_id],
                view.agents.journey_start_time[agent_id],
                view.agents.has_car[agent_id],
                i64::from(view.agents.vehicle_type[agent_id]),
                usize_to_i64(view.agents.current_path_index[agent_id])?
            ])?;

            for (step_index, &node_id) in view.agents.current_path[agent_id].iter().enumerate() {
                let canonical = canonical_existing_node(view.graph, node_id)?;
                let saved_node_id = maps
                    .node_old_to_new
                    .get(&canonical)
                    .copied()
                    .ok_or_else(|| SaveLoadError::custom("missing saved route node mapping"))?;
                car_path_stmt.execute(params![
                    usize_to_i64(agent_id)?,
                    usize_to_i64(step_index)?,
                    i64::from(saved_node_id)
                ])?;
            }

        }
    }

    tx.commit()?;
    Ok(())
}

pub(crate) fn load_from_sqlite(path: &Path) -> SaveLoadResult<LoadedSimulation> {
    let conn = Connection::open(path)?;

    let version: i64 = conn.query_row("SELECT version FROM save_meta LIMIT 1", [], |row| {
        row.get(0)
    })?;
    if version != SAVE_VERSION {
        return Err(SaveLoadError::custom(format!(
            "unsupported save version {} (expected {})",
            version, SAVE_VERSION
        )));
    }

    let config = conn.query_row(
        "SELECT width_m, height_m, env_cell_m, zone_cell_m FROM map_config LIMIT 1",
        [],
        |row| {
            Ok(MapConfig::new(
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
            ))
        },
    )?;

    let (time_elapsed, speed_multiplier, current_day, seconds_per_day, agent_sim_time): (
        f64,
        f32,
        i64,
        f64,
        f32,
    ) = conn.query_row(
        "SELECT time_elapsed, speed_multiplier, current_day, seconds_per_day, agent_sim_time FROM time_state LIMIT 1",
        [],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )?;
    let time = TimeSystem {
        time_elapsed,
        speed_multiplier,
        current_day: i64_to_u32(current_day)?,
        seconds_per_day,
    };

    let terrain = load_terrain(&conn, &config)?;
    let water = load_water(&conn, terrain.width, terrain.height)?;
    let demand = conn.query_row(
        "SELECT residential, commercial, industrial FROM demand_state LIMIT 1",
        [],
        |row| {
            Ok(DemandSystem {
                residential: row.get(0)?,
                commercial: row.get(1)?,
                industrial: row.get(2)?,
            })
        },
    )?;
    let pollution = load_grid_system::<PollutionSystem>(&conn, &config, "pollution_state")?;
    let noise = load_grid_system::<NoiseSystem>(&conn, &config, "noise_state")?;

    let mut graph = load_graph(&conn)?;
    let mut zoning = load_zoning(&conn, &config)?;
    let mut allocator = load_buildings(&conn)?;
    let mut agents = load_agents(&conn, agent_sim_time)?;

    let mut terrain = terrain;
    let mut transit_network = TransitNetwork::new();
    rebuild_loaded_graph_runtime(&mut graph, &mut transit_network, &mut terrain);
    transit_network.lane_system.rebuild(&mut graph);

    // Remap the safely-stored `lane_idx` back into the dynamic `lane_id`
    for i in 0..agents.len() {
        let edge_id = agents.current_edge[i];
        let lane_idx = agents.current_lane_id[i]; // Temporarily stores `lane_idx` across load
        if edge_id != usize::MAX && lane_idx != usize::MAX {
            agents.current_lane_id[i] = transit_network.lane_system.get_lane_id(edge_id, lane_idx).unwrap_or(usize::MAX);
        }
    }

    allocator.recompute_derived_transforms(&graph, &zoning)?;
    repaint_building_occupancy(&mut zoning, &allocator)?;
    allocator.rebuild_zone_index();
    allocator.dirty = true;
    rebuild_zoning_obstructions(&mut zoning, &graph);

    transit_network.cch_graph = CchGraph::build(&graph);
    validate_loaded_agents(&mut agents, &graph, &allocator)?;

    let mut desirability = DesirabilitySystem::new(&config);
    desirability.tick(&zoning, &pollution, &noise);

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
        allocator,
        agents,
    })
}

fn load_terrain(conn: &Connection, config: &MapConfig) -> SaveLoadResult<TerrainSystem> {
    let (width_raw, height_raw, blob): (i64, i64, Vec<u8>) = conn.query_row(
        "SELECT width, height, height_blob_f32_le FROM terrain_state LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let width = i64_to_usize(width_raw)?;
    let height = i64_to_usize(height_raw)?;

    if width != config.zone_grid_width() || height != config.zone_grid_height() {
        return Err(SaveLoadError::custom(format!(
            "terrain dimensions {}x{} do not match config-derived size {}x{}",
            width,
            height,
            config.zone_grid_width(),
            config.zone_grid_height()
        )));
    }

    let heights = unpack_f32_blob(&blob, width * height)?;
    let mut terrain = TerrainSystem::new(width, height);
    terrain.source_data = heights;
    terrain.reset_visuals_from_source();
    Ok(terrain)
}

fn load_water(
    conn: &Connection,
    expected_width: usize,
    expected_height: usize,
) -> SaveLoadResult<WaterSystem> {
    let (width_raw, height_raw, depth_blob, velocity_blob, flux_blob): (
        i64,
        i64,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
    ) = conn.query_row(
        "SELECT width, height, depth_blob_f32_le, velocity_blob_f32_le, flux_blob_f32x4_le FROM water_state LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    )?;
    let width = i64_to_usize(width_raw)?;
    let height = i64_to_usize(height_raw)?;

    if width != expected_width || height != expected_height {
        return Err(SaveLoadError::custom(format!(
            "water dimensions {}x{} do not match terrain size {}x{}",
            width, height, expected_width, expected_height
        )));
    }

    let mut water = WaterSystem::new(width, height);
    water.depth = unpack_f32_blob(&depth_blob, width * height)?;
    water.velocity = unpack_f32_blob(&velocity_blob, width * height)?;
    water.flux = unpack_flux_blob(&flux_blob, width * height)?;

    let mut stmt =
        conn.prepare("SELECT grid_x, grid_y, rate_m_per_tick FROM water_sources ORDER BY rowid")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        water.sources.push((
            i64_to_usize(row.get(0)?)?,
            i64_to_usize(row.get(1)?)?,
            row.get(2)?,
        ));
    }

    Ok(water)
}

trait GridSystemLoader: Sized {
    fn new_with_config(config: &MapConfig) -> Self;
    fn grid_mut(&mut self) -> &mut DataGrid<f32>;
}

impl GridSystemLoader for PollutionSystem {
    fn new_with_config(config: &MapConfig) -> Self {
        Self::new(config)
    }

    fn grid_mut(&mut self) -> &mut DataGrid<f32> {
        &mut self.grid
    }
}

impl GridSystemLoader for NoiseSystem {
    fn new_with_config(config: &MapConfig) -> Self {
        Self::new(config)
    }

    fn grid_mut(&mut self) -> &mut DataGrid<f32> {
        &mut self.grid
    }
}

fn load_grid_system<T: GridSystemLoader>(
    conn: &Connection,
    config: &MapConfig,
    table: &str,
) -> SaveLoadResult<T> {
    let sql = format!("SELECT width, height, grid_blob_f32_le FROM {table} LIMIT 1");
    let (width_raw, height_raw, blob): (i64, i64, Vec<u8>) =
        conn.query_row(&sql, [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
    let width = i64_to_usize(width_raw)?;
    let height = i64_to_usize(height_raw)?;

    if width != config.env_grid_width() || height != config.env_grid_height() {
        return Err(SaveLoadError::custom(format!(
            "{table} dimensions {}x{} do not match config-derived env size {}x{}",
            width,
            height,
            config.env_grid_width(),
            config.env_grid_height()
        )));
    }

    let mut system = T::new_with_config(config);
    system.grid_mut().data = unpack_f32_blob(&blob, width * height)?;
    Ok(system)
}

fn load_graph(conn: &Connection) -> SaveLoadResult<RegionGraph> {
    let mut graph = RegionGraph::new();

    {
        let mut stmt =
            conn.prepare("SELECT node_id, x, y, z, node_type FROM network_nodes ORDER BY node_id")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let node_id = i64_to_u32(row.get(0)?)?;
            if node_id != graph.nodes.len() as u32 {
                return Err(SaveLoadError::custom(format!(
                    "network node ids must be contiguous from 0; found {} after {} nodes",
                    node_id,
                    graph.nodes.len()
                )));
            }
            graph.add_node(
                Vector3::new(row.get(1)?, row.get(2)?, row.get(3)?),
                node_type_from_i64(row.get(4)?)?,
            );
        }
    }

    let mut geometry: HashMap<usize, Vec<Vector3>> = HashMap::new();
    let mut physical_geometry: HashMap<usize, Vec<Vector3>> = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT edge_id, point_index, x, y, z, physical
             FROM network_edge_geometry
             ORDER BY edge_id, physical, point_index",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let edge_id = i64_to_usize(row.get(0)?)?;
            let _point_index = i64_to_usize(row.get(1)?)?;
            let point = Vector3::new(row.get(2)?, row.get(3)?, row.get(4)?);
            if row.get::<_, bool>(5)? {
                physical_geometry.entry(edge_id).or_default().push(point);
            } else {
                geometry.entry(edge_id).or_default().push(point);
            }
        }
    }

    {
        let mut stmt = conn.prepare(
            "SELECT
                edge_id, start_node, end_node, primary_type, allowed_types, class, width,
                fwd_lanes, bkw_lanes, speed_limit, base_cost, physical_length, current_congestion,
                start_clip, end_clip
             FROM network_edges
             ORDER BY edge_id",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let edge_id = i64_to_usize(row.get(0)?)?;
            if edge_id != graph.edges.len() {
                return Err(SaveLoadError::custom(format!(
                    "network edge ids must be contiguous from 0; found {} after {} edges",
                    edge_id,
                    graph.edges.len()
                )));
            }

            let edge = Edge {
                start_node: i64_to_u32(row.get(1)?)?,
                end_node: i64_to_u32(row.get(2)?)?,
                primary_type: transit_type_from_i64(row.get(3)?)?,
                allowed_types: i64_to_u8(row.get(4)?)?,
                class: edge_class_from_i64(row.get(5)?)?,
                width: row.get(6)?,
                fwd_lanes: i64_to_u8(row.get(7)?)?,
                bkw_lanes: i64_to_u8(row.get(8)?)?,
                speed_limit: row.get(9)?,
                base_cost: row.get(10)?,
                physical_length: row.get(11)?,
                current_congestion: row.get(12)?,
                start_clip: row.get(13)?,
                end_clip: row.get(14)?,
                geometry: geometry.remove(&edge_id).unwrap_or_default(),
                physical_geometry: physical_geometry.remove(&edge_id).unwrap_or_default(),
                deleted: false,
            };
            graph.add_edge(edge);
        }
    }

    {
        let mut stmt = conn.prepare(
            "SELECT node_id, from_edge, from_lane, to_edge, to_lane
             FROM lane_connections
             ORDER BY node_id, from_edge, from_lane, to_edge, to_lane",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let node_id = i64_to_u32(row.get(0)?)?;
            let from_edge = i64_to_usize(row.get(1)?)?;
            let from_lane = i64_to_i8(row.get(2)?)?;
            let to_edge = i64_to_usize(row.get(3)?)?;
            let to_lane = i64_to_i8(row.get(4)?)?;

            if node_id as usize >= graph.nodes.len()
                || from_edge >= graph.edges.len()
                || to_edge >= graph.edges.len()
            {
                return Err(SaveLoadError::custom(
                    "lane connection references out-of-range node or edge",
                ));
            }

            graph.nodes[node_id as usize]
                .lane_connections
                .entry((from_edge, from_lane))
                .or_default()
                .push((to_edge, to_lane));
        }
    }

    rebuild_graph_indices(&mut graph);
    Ok(graph)
}

fn load_zoning(conn: &Connection, config: &MapConfig) -> SaveLoadResult<ZoningSystem> {
    let mut zoning = ZoningSystem::new(config);
    let mut stmt = conn.prepare(
        "SELECT edge_id, cells_long, left_zone_blob_u8, right_zone_blob_u8
         FROM zoning_grids
         ORDER BY edge_id",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let edge_id = i64_to_usize(row.get(0)?)?;
        let cells_long = i64_to_usize(row.get(1)?)?;
        let left_blob: Vec<u8> = row.get(2)?;
        let right_blob: Vec<u8> = row.get(3)?;
        let cell_count = cells_long * ZONING_DEPTH;
        zoning.edge_grids.insert(
            edge_id,
            EdgeZoning {
                left_side: unpack_zone_blob(&left_blob, cell_count)?,
                right_side: unpack_zone_blob(&right_blob, cell_count)?,
                left_occupied: vec![false; cell_count],
                right_occupied: vec![false; cell_count],
                left_blocked: vec![false; cell_count],
                right_blocked: vec![false; cell_count],
                left_block_depth: vec![0u8; cells_long],
                right_block_depth: vec![0u8; cells_long],
                left_block_id: vec![0u16; cells_long],
                right_block_id: vec![0u16; cells_long],
                cells_long,
            },
        );
    }
    Ok(zoning)
}

fn load_buildings(conn: &Connection) -> SaveLoadResult<BuildingAllocator> {
    let mut allocator = BuildingAllocator::new();
    let mut stmt = conn.prepare(
        "SELECT
            building_id, edge_id, frontage_t, side, cell_x, cell_y, zone_type, occupancy, width, depth,
            frontage_node, variant
         FROM buildings
         ORDER BY building_id",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let building_id = i64_to_usize(row.get(0)?)?;
        if building_id != allocator.buildings.len() {
            return Err(SaveLoadError::custom(format!(
                "building ids must be contiguous from 0; found {} after {} buildings",
                building_id,
                allocator.buildings.len()
            )));
        }

        let occupancy = i64_to_u32(row.get(7)?)?;
        
        allocator.buildings.push(Building {
            center_x: 0.0,
            center_y: 0.0,
            width_cells: i64_to_u8(row.get(8)?)?,
            depth_cells: i64_to_u8(row.get(9)?)?,
            zone_type: zone_type_from_i64(row.get(6)?)?,
            facing_dir: Vector2::new(0.0, 0.0),
            frontage_t: row.get(2)?,
            frontage_node: i64_to_u32(row.get(10)?)?,
            side_offset: 0.0,
            abandoned_timer: 0,
            edge_idx: i64_to_usize(row.get(1)?)?,
            side: i64_to_i8(row.get(3)?)?,
            cell_x: i64_to_usize(row.get(4)?)?,
            cell_y: i64_to_u8(row.get(5)?)?,
            occupancy,
            variant: i64_to_u8(row.get(11)?)?,
        });
    }
    Ok(allocator)
}

fn load_agents(conn: &Connection, agent_sim_time: f32) -> SaveLoadResult<AgentSystem> {
    let mut car_paths: HashMap<usize, Vec<u32>> = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT agent_id, step_index, node_id FROM agent_path_nodes ORDER BY agent_id, step_index",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let agent_id = i64_to_usize(row.get(0)?)?;
            let _step_index = i64_to_usize(row.get(1)?)?;
            car_paths
                .entry(agent_id)
                .or_default()
                .push(i64_to_u32(row.get(2)?)?);
        }
    }


    let mut agents = AgentSystem::new();
    agents.sim_time = agent_sim_time;

    let mut stmt = conn.prepare(
        "SELECT
            agent_id, home_building, work_building, current_building, target_building, current_node,
            target_node, current_edge, current_lane_id, lane_distance, pos_x, pos_y, is_visible,
            activity, transit, transit_mode, pedestrian_side, happiness, money, journey_start_time,
            has_car, vehicle_type, current_path_index
         FROM agents
         ORDER BY agent_id",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let agent_id = i64_to_usize(row.get(0)?)?;
        if agent_id != agents.len() {
            return Err(SaveLoadError::custom(format!(
                "agent ids must be contiguous from 0; found {} after {} agents",
                agent_id, agents.len()
            )));
        }

        push_loaded_agent(
            &mut agents,
            LoadedAgentRecord {
                home_building: db_to_optional_usize(row.get(1)?)?,
                work_building: db_to_optional_usize(row.get(2)?)?,
                current_building: db_to_optional_usize(row.get(3)?)?,
                target_building: db_to_optional_usize(row.get(4)?)?,
                current_node: i64_to_u32(row.get(5)?)?,
                target_node: i64_to_u32(row.get(6)?)?,
                current_edge: db_to_optional_usize(row.get(7)?)?,
                current_lane_id: row.get(8)?, // B18: Read as i64 directly to support signed sidewalk indices (-100, 100)
                lane_distance: row.get(9)?,
                pos_x: row.get(10)?,
                pos_y: row.get(11)?,
                is_visible: row.get(12)?,
                activity: i64_to_u8(row.get(13)?)?,
                transit: i64_to_u8(row.get(14)?)?,
                transit_mode: i64_to_u8(row.get(15)?)?,
                happiness: row.get(17)?,
                money: row.get(18)?,
                journey_start_time: row.get(19)?,
                has_car: row.get(20)?,
                vehicle_type: i64_to_u8(row.get(21)?)?,
                current_path_index: i64_to_usize(row.get(22)?)?,
                current_path: car_paths.remove(&agent_id).unwrap_or_default(),
                pedestrian_type: 0,
                walk_phase: 0.0,
            },
        );
    }

    if !car_paths.is_empty() {
        return Err(SaveLoadError::custom(
            "found path rows for agent ids that were not present in agents table",
        ));
    }

    Ok(agents)
}

fn rebuild_loaded_graph_runtime(
    graph: &mut RegionGraph,
    transit_network: &mut TransitNetwork,
    terrain: &mut TerrainSystem,
) {
    rebuild_graph_indices(graph);
    terrain.reset_visuals_from_source();
    let source_terrain = TerrainSystem {
        width: terrain.width,
        height: terrain.height,
        data: terrain.data.clone(),
        source_data: terrain.source_data.clone(),
    };
    transit_network.flatten_terrain(
        graph,
        &source_terrain,
        &mut terrain.data,
        Vector2::new(terrain.width as f32, terrain.height as f32),
    );
    transit_network.sync_to_terrain(graph, terrain);
    for edge in &mut graph.edges {
        if edge.deleted {
            continue;
        }
        let (base_cost, physical_length) = CostCalculator::calculate_costs(edge);
        edge.base_cost = base_cost;
        edge.physical_length = physical_length;
    }
    graph.rebuild_intersection_clips();
}

fn rebuild_graph_indices(graph: &mut RegionGraph) {
    graph.node_aliases.clear();
    graph.rebuild_adjacency_list();
    graph.spatial_edge_rt = rstar::RTree::new();
    for edge_idx in 0..graph.edges.len() {
        graph.add_to_spatial_index(edge_idx);
    }
    graph.spatial_node_grid.clear();
    for node_id in 0..graph.nodes.len() {
        graph.add_node_to_spatial_index(node_id as u32);
    }
}

fn repaint_building_occupancy(
    zoning: &mut ZoningSystem,
    allocator: &BuildingAllocator,
) -> SaveLoadResult<()> {
    for grid in zoning.edge_grids.values_mut() {
        grid.left_occupied.fill(false);
        grid.right_occupied.fill(false);
    }

    for building in &allocator.buildings {
        let width_cells = building.width_cells as usize;
        let depth_cells = building.depth_cells as usize;
        for dx in 0..width_cells {
            for dy in 0..depth_cells {
                zoning.set_occupied(
                    building.edge_idx,
                    building.side,
                    building.cell_x + dx,
                    usize::from(building.cell_y) + dy,
                    true,
                );
            }
        }
    }

    Ok(())
}

fn rebuild_zoning_obstructions(zoning: &mut ZoningSystem, graph: &RegionGraph) {
    let edge_ids: Vec<usize> = zoning.edge_grids.keys().copied().collect();
    for edge_id in edge_ids {
        if edge_id < graph.edges.len() && !graph.edges[edge_id].deleted {
            zoning.recalculate_obstructions(edge_id, graph);
        }
    }
}

fn validate_loaded_agents(
    agents: &mut AgentSystem,
    graph: &RegionGraph,
    allocator: &BuildingAllocator,
) -> SaveLoadResult<()> {
    for i in 0..agents.len() {
        if (agents.current_node[i] as usize) >= graph.nodes.len()
            || (agents.target_node[i] as usize) >= graph.nodes.len()
        {
            return Err(SaveLoadError::custom(format!(
                "agent {} references out-of-range current or target node",
                i
            )));
        }

        if agents.home_building[i] != usize::MAX
            && agents.home_building[i] >= allocator.buildings.len()
        {
            agents.home_building[i] = usize::MAX;
        }
        if agents.work_building[i] != usize::MAX
            && agents.work_building[i] >= allocator.buildings.len()
        {
            agents.work_building[i] = usize::MAX;
        }

        let mut clear_travel = false;

        if agents.current_building[i] != usize::MAX
            && agents.current_building[i] >= allocator.buildings.len()
        {
            agents.current_building[i] = usize::MAX;
            clear_travel = true;
        }
        if agents.target_building[i] != usize::MAX
            && agents.target_building[i] >= allocator.buildings.len()
        {
            agents.target_building[i] = usize::MAX;
            clear_travel = true;
        }


        if agents.current_path_index[i] > agents.current_path[i].len() {
            clear_travel = true;
        }
        if agents.current_path[i]
            .iter()
            .any(|&node_id| (node_id as usize) >= graph.nodes.len())
        {
            clear_travel = true;
        }
        if agents.lane_distance[i] < 0.0 || !agents.lane_distance[i].is_finite() {
            clear_travel = true;
        }

        if agents.current_edge[i] != usize::MAX {
            if agents.current_edge[i] >= graph.edges.len()
                || graph.edges[agents.current_edge[i]].deleted
            {
                clear_travel = true;
            } else {
                let edge = &graph.edges[agents.current_edge[i]];
                if edge.physical_geometry.is_empty() {
                    clear_travel = true;
                } else {
                    if agents.transit_mode[i] != crate::simulation::economy::agents::MODE_CAR {
                        agents.current_lane_id[i] = usize::MAX;
                    }
                }
            }
        }

        if clear_travel {
            clear_agent_travel_state(agents, i);
        }
    }

    Ok(())
}

fn clear_agent_travel_state(agents: &mut AgentSystem, agent_id: usize) {
    agents.current_path[agent_id].clear();
    agents.current_path_index[agent_id] = 0;
    agents.current_edge[agent_id] = usize::MAX;
    agents.current_lane_id[agent_id] = usize::MAX;
    agents.lane_distance[agent_id] = 0.0;

    if agents.transit[agent_id] == TRANSIT_INTERSECTION {
        agents.transit[agent_id] = TRANSIT_ON_ROAD;
    } else if agents.transit[agent_id] != TRANSIT_IDLE
        && agents.transit[agent_id] != TRANSIT_IMMIGRATING
    {
        agents.transit[agent_id] = TRANSIT_ON_ROAD;
    }
}

fn build_snapshot_maps(
    graph: &RegionGraph,
    allocator: &BuildingAllocator,
    agents: &AgentSystem,
) -> SaveLoadResult<SnapshotMaps> {
    let mut edge_old_to_new = HashMap::new();
    for (old_edge_id, edge) in graph.edges.iter().enumerate() {
        if !edge.deleted {
            let new_edge_id = edge_old_to_new.len();
            edge_old_to_new.insert(old_edge_id, new_edge_id);
        }
    }

    let mut saved_nodes = BTreeSet::new();
    for edge in &graph.edges {
        if edge.deleted {
            continue;
        }
        saved_nodes.insert(canonical_existing_node(graph, edge.start_node)?);
        saved_nodes.insert(canonical_existing_node(graph, edge.end_node)?);
    }
    for agent_id in 0..agents.len() {
        saved_nodes.insert(canonical_existing_node(
            graph,
            agents.current_node[agent_id],
        )?);
        saved_nodes.insert(canonical_existing_node(
            graph,
            agents.target_node[agent_id],
        )?);
        for &node_id in &agents.current_path[agent_id] {
            saved_nodes.insert(canonical_existing_node(graph, node_id)?);
        }
    }

    let node_old_to_new = saved_nodes
        .into_iter()
        .enumerate()
        .map(|(new_id, old_id)| (old_id, new_id as u32))
        .collect();

    let mut building_old_to_new = HashMap::new();
    for (old_building_id, building) in allocator.buildings.iter().enumerate() {
        if !edge_old_to_new.contains_key(&building.edge_idx) {
            return Err(SaveLoadError::custom(format!(
                "building {} references deleted or missing edge {}",
                old_building_id, building.edge_idx
            )));
        }
        building_old_to_new.insert(old_building_id, building_old_to_new.len());
    }

    Ok(SnapshotMaps {
        node_old_to_new,
        edge_old_to_new,
        building_old_to_new,
    })
}

fn pack_f32_slice(values: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for &value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn unpack_f32_blob(blob: &[u8], expected_len: usize) -> SaveLoadResult<Vec<f32>> {
    if blob.len() != expected_len * 4 {
        return Err(SaveLoadError::custom(format!(
            "expected {} f32 values ({} bytes), found {} bytes",
            expected_len,
            expected_len * 4,
            blob.len()
        )));
    }
    let mut values = Vec::with_capacity(expected_len);
    for chunk in blob.chunks_exact(4) {
        values.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    Ok(values)
}

fn pack_flux_slice(values: &[[f32; 4]]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(values.len() * 16);
    for quad in values {
        for &value in quad {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    bytes
}

fn unpack_flux_blob(blob: &[u8], expected_len: usize) -> SaveLoadResult<Vec<[f32; 4]>> {
    if blob.len() != expected_len * 16 {
        return Err(SaveLoadError::custom(format!(
            "expected {} flux quads ({} bytes), found {} bytes",
            expected_len,
            expected_len * 16,
            blob.len()
        )));
    }
    let mut values = Vec::with_capacity(expected_len);
    for chunk in blob.chunks_exact(16) {
        values.push([
            f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]),
            f32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]),
            f32::from_le_bytes([chunk[8], chunk[9], chunk[10], chunk[11]]),
            f32::from_le_bytes([chunk[12], chunk[13], chunk[14], chunk[15]]),
        ]);
    }
    Ok(values)
}

fn pack_zone_slice(values: &[ZoneType]) -> Vec<u8> {
    values.iter().map(|zone| *zone as u8).collect()
}

fn unpack_zone_blob(blob: &[u8], expected_len: usize) -> SaveLoadResult<Vec<ZoneType>> {
    if blob.len() != expected_len {
        return Err(SaveLoadError::custom(format!(
            "expected {} zone bytes, found {} bytes",
            expected_len,
            blob.len()
        )));
    }

    blob.iter()
        .map(|&value| zone_type_from_i64(i64::from(value)))
        .collect()
}

fn canonical_existing_node(graph: &RegionGraph, node_id: u32) -> SaveLoadResult<u32> {
    if (node_id as usize) >= graph.nodes.len() {
        return Err(SaveLoadError::custom(format!(
            "node {} out of bounds for {} graph nodes",
            node_id,
            graph.nodes.len()
        )));
    }
    let canonical = graph.get_valid_node(node_id);
    if (canonical as usize) >= graph.nodes.len() {
        return Err(SaveLoadError::custom(format!(
            "canonical node {} out of bounds for {} graph nodes",
            canonical,
            graph.nodes.len()
        )));
    }
    Ok(canonical)
}

fn optional_building_to_db(value: usize, maps: &SnapshotMaps) -> SaveLoadResult<i64> {
    if value == usize::MAX {
        Ok(NONE_REF)
    } else {
        let saved = maps
            .building_old_to_new
            .get(&value)
            .copied()
            .ok_or_else(|| {
                SaveLoadError::custom(format!("missing saved building mapping for {}", value))
            })?;
        usize_to_i64(saved)
    }
}

fn optional_edge_to_db(value: usize, maps: &SnapshotMaps) -> SaveLoadResult<i64> {
    if value == usize::MAX {
        Ok(NONE_REF)
    } else {
        let saved = maps.edge_old_to_new.get(&value).copied().ok_or_else(|| {
            SaveLoadError::custom(format!("missing saved edge mapping for {}", value))
        })?;
        usize_to_i64(saved)
    }
}

fn db_to_optional_usize(value: i64) -> SaveLoadResult<usize> {
    if value == NONE_REF {
        Ok(usize::MAX)
    } else {
        i64_to_usize(value)
    }
}

fn usize_to_i64(value: usize) -> SaveLoadResult<i64> {
    i64::try_from(value).map_err(|_| SaveLoadError::custom("usize value does not fit i64"))
}

fn u32_to_i64(value: u32) -> SaveLoadResult<i64> {
    i64::try_from(value).map_err(|_| SaveLoadError::custom("u32 value does not fit i64"))
}

fn u8_to_i64(value: u8) -> SaveLoadResult<i64> {
    Ok(i64::from(value))
}

fn i64_to_usize(value: i64) -> SaveLoadResult<usize> {
    usize::try_from(value)
        .map_err(|_| SaveLoadError::custom(format!("{} does not fit usize", value)))
}

fn i64_to_u32(value: i64) -> SaveLoadResult<u32> {
    u32::try_from(value).map_err(|_| SaveLoadError::custom(format!("{} does not fit u32", value)))
}

fn i64_to_u8(value: i64) -> SaveLoadResult<u8> {
    u8::try_from(value).map_err(|_| SaveLoadError::custom(format!("{} does not fit u8", value)))
}

fn i64_to_i8(value: i64) -> SaveLoadResult<i8> {
    i8::try_from(value).map_err(|_| SaveLoadError::custom(format!("{} does not fit i8", value)))
}

fn transit_type_to_i64(value: TransitType) -> i64 {
    match value {
        TransitType::Road => 0,
        TransitType::Rail => 1,
        TransitType::Ship => 2,
        TransitType::Air => 3,
        TransitType::Foot => 4,
    }
}

fn transit_type_from_i64(value: i64) -> SaveLoadResult<TransitType> {
    match value {
        0 => Ok(TransitType::Road),
        1 => Ok(TransitType::Rail),
        2 => Ok(TransitType::Ship),
        3 => Ok(TransitType::Air),
        4 => Ok(TransitType::Foot),
        _ => Err(SaveLoadError::custom(format!(
            "unknown TransitType value {}",
            value
        ))),
    }
}

fn node_type_to_i64(value: NodeType) -> i64 {
    match value {
        NodeType::Junction => 0,
        NodeType::Station => 1,
        NodeType::Harbor => 2,
        NodeType::Airport => 3,
        NodeType::Transfer => 4,
        NodeType::Border => 5,
        NodeType::Frontage => 6,
    }
}

fn node_type_from_i64(value: i64) -> SaveLoadResult<NodeType> {
    match value {
        0 => Ok(NodeType::Junction),
        1 => Ok(NodeType::Station),
        2 => Ok(NodeType::Harbor),
        3 => Ok(NodeType::Airport),
        4 => Ok(NodeType::Transfer),
        5 => Ok(NodeType::Border),
        6 => Ok(NodeType::Frontage),
        _ => Err(SaveLoadError::custom(format!(
            "unknown NodeType value {}",
            value
        ))),
    }
}

fn edge_class_to_i64(value: EdgeClass) -> i64 {
    match value {
        EdgeClass::Standard => 0,
        EdgeClass::Bridge => 1,
        EdgeClass::Tunnel => 2,
    }
}

fn edge_class_from_i64(value: i64) -> SaveLoadResult<EdgeClass> {
    match value {
        0 => Ok(EdgeClass::Standard),
        1 => Ok(EdgeClass::Bridge),
        2 => Ok(EdgeClass::Tunnel),
        _ => Err(SaveLoadError::custom(format!(
            "unknown EdgeClass value {}",
            value
        ))),
    }
}

fn zone_type_to_i64(value: ZoneType) -> i64 {
    i64::from(value as u8)
}

fn zone_type_from_i64(value: i64) -> SaveLoadResult<ZoneType> {
    match value {
        0 => Ok(ZoneType::None),
        1 => Ok(ZoneType::Residential),
        2 => Ok(ZoneType::Commercial),
        3 => Ok(ZoneType::Industrial),
        4 => Ok(ZoneType::Office),
        5 => Ok(ZoneType::Mixed),
        _ => Err(SaveLoadError::custom(format!(
            "unknown ZoneType value {}",
            value
        ))),
    }
}

struct LoadedAgentRecord {
    home_building: usize,
    work_building: usize,
    current_building: usize,
    target_building: usize,
    current_node: u32,
    target_node: u32,
    current_edge: usize,
    current_lane_id: i64, // Temporarily stores lane_idx (signed) across load
    lane_distance: f32,
    pos_x: f32,
    pos_y: f32,
    is_visible: bool,
    activity: u8,
    transit: u8,
    transit_mode: u8,
    happiness: f32,
    money: f32,
    journey_start_time: f32,
    has_car: bool,
    vehicle_type: u8,
    current_path_index: usize,
    current_path: Vec<u32>,
    pedestrian_type: u8,
    walk_phase: f32,
}

fn push_loaded_agent(agents: &mut AgentSystem, agent: LoadedAgentRecord) {
    let a = Agent {
        home_building: agent.home_building,
        work_building: agent.work_building,
        pos_x: agent.pos_x,
        pos_y: agent.pos_y,
        is_visible: agent.is_visible,
        activity: agent.activity,
        transit: agent.transit,
        happiness: agent.happiness,
        money: agent.money,
        journey_start_time: agent.journey_start_time,
        current_building: agent.current_building,
        target_building: agent.target_building,
        current_node: agent.current_node,
        target_node: agent.target_node,
        current_edge: agent.current_edge,
        current_lane_id: agent.current_lane_id as usize,
        lane_distance: agent.lane_distance,
        speed: if agent.transit_mode == MODE_CAR { 20.0 } else { 4.0 },
        transit_mode: agent.transit_mode,
        current_path: agent.current_path,
        current_path_index: agent.current_path_index,
        has_car: agent.has_car,
        vehicle_type: agent.vehicle_type,
        pedestrian_type: agent.pedestrian_type,
        walk_phase: agent.walk_phase,
    };
    agents.agents.push(a);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::economy::agents::{MODE_CAR, MODE_WALK};
    use crate::simulation::network::types::{TransitFlags, TransitType};

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "metrum_rise_{name}_{}_{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sqlite_round_trip_preserves_authoritative_state() {
        let config = MapConfig::new(100.0, 100.0, 10.0, 10.0);
        let mut time = TimeSystem::new();
        time.speed_multiplier = 2.0;
        time.time_elapsed = 1.25;
        time.current_day = 3;
        time.seconds_per_day = 4.0;

        let mut terrain = TerrainSystem::new(config.zone_grid_width(), config.zone_grid_height());
        terrain.source_data.fill(1.0);
        terrain.reset_visuals_from_source();

        let mut water = WaterSystem::new(terrain.width, terrain.height);
        water.depth[0] = 2.5;
        water.velocity[0] = 0.75;
        water.flux[0] = [1.0, 2.0, 3.0, 4.0];
        water.sources.push((1, 2, 0.5));

        let mut graph = RegionGraph::new();
        let n0 = graph.add_node(Vector3::new(-20.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
        let edge_id = graph.add_edge(Edge {
            start_node: n0,
            end_node: n1,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: 40.0,
            physical_length: 40.0,
            current_congestion: 0.1,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(-20.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(-20.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
            deleted: false,
        });
        graph.nodes[n0 as usize]
            .lane_connections
            .insert((edge_id, 0), vec![(edge_id, 0)]);

        let mut zoning = ZoningSystem::new(&config);
        zoning.update_edge_grid_size(edge_id, 40.0);
        zoning.set_zone_range(edge_id, 1, 0.0, 1.0, 3, ZoneType::Residential, &graph);

        let mut pollution = PollutionSystem::new(&config);
        pollution.grid.data[0] = 3.0;
        let mut noise = NoiseSystem::new(&config);
        noise.grid.data[0] = 7.0;
        let mut demand = DemandSystem::new();
        demand.residential = 12.0;
        demand.commercial = 8.0;
        demand.industrial = 4.0;

        let mut allocator = BuildingAllocator::new();
        allocator.buildings.push(Building {
            center_x: 0.0,
            center_y: 0.0,
            width_cells: 3,
            depth_cells: 3,
            zone_type: ZoneType::Residential,
            facing_dir: Vector2::new(0.0, 1.0),
            frontage_t: 0.5,
            frontage_node: n1,
            side_offset: 1.0,
            abandoned_timer: 0,
            edge_idx: edge_id,
            side: 1,
            cell_x: 0,
            cell_y: 0,
            occupancy: 2,
            variant: 0,
        });
        allocator
            .recompute_derived_transforms(&graph, &zoning)
            .expect("building transforms");
        repaint_building_occupancy(&mut zoning, &allocator).expect("occupancy");
        allocator.rebuild_zone_index();

        let mut agents = AgentSystem::new();
        agents.sim_time = 42.0;
        push_loaded_agent(
            &mut agents,
            LoadedAgentRecord {
                home_building: 0,
                work_building: usize::MAX,
                current_building: usize::MAX,
                target_building: 0,
                current_node: n0,
                target_node: n1,
                current_edge: edge_id,
                current_lane_id: 0,
                lane_distance: 0.0,
                pos_x: -5.0,
                pos_y: 0.0,
                is_visible: true,
                activity: 1,
                transit: TRANSIT_ON_ROAD,
                transit_mode: MODE_CAR,
                happiness: 88.0,
                money: 123.0,
                journey_start_time: 12.5,
                has_car: true,
                vehicle_type: 0,
                current_path_index: 1,
                current_path: vec![n0, n1],
                pedestrian_type: 0,
                walk_phase: 0.0,
            },
        );
        push_loaded_agent(
            &mut agents,
            LoadedAgentRecord {
                home_building: 0,
                work_building: usize::MAX,
                current_building: usize::MAX,
                target_building: 0,
                current_node: n1,
                target_node: n0,
                current_edge: usize::MAX,
                current_lane_id: -1,
                lane_distance: 0.0,
                pos_x: 5.0,
                pos_y: 0.0,
                is_visible: true,
                activity: 0,
                transit: TRANSIT_ON_ROAD,
                transit_mode: MODE_WALK,
                happiness: 77.0,
                money: 55.0,
                journey_start_time: 6.0,
                has_car: false,
                vehicle_type: 0,
                current_path_index: 0,
                current_path: Vec::new(),
                pedestrian_type: 0,
                walk_phase: 0.0,
            },
        );

        let mut network = TransitNetwork::new();
        network.lane_system.rebuild(&mut graph);

        let path = temp_path("round_trip");
        save_to_sqlite(
            &path,
            SaveGameView {
                config: &config,
                time: &time,
                terrain: &terrain,
                water: &water,
                graph: &graph,
                zoning: &zoning,
                pollution: &pollution,
                noise: &noise,
                demand: &demand,
                allocator: &allocator,
                agents: &agents,
                network: &network,
            },
        )
        .expect("save should succeed");

        let loaded = load_from_sqlite(&path).expect("load should succeed");
        fs::remove_file(&path).ok();

        assert_eq!(loaded.config.width_m, config.width_m);
        assert_eq!(loaded.time.current_day, time.current_day);
        assert_eq!(loaded.terrain.source_data, terrain.source_data);
        assert_eq!(loaded.water.depth, water.depth);
        assert_eq!(loaded.demand.residential, demand.residential);
        assert_eq!(loaded.pollution.grid.data, pollution.grid.data);
        assert_eq!(loaded.noise.grid.data, noise.grid.data);
        assert_eq!(loaded.graph.edges.len(), 1);
        assert_eq!(loaded.zoning.edge_grids.len(), 1);
        assert_eq!(loaded.allocator.buildings.len(), 1);
        assert_eq!(loaded.agents.len(), 2);
        assert_eq!(loaded.agents.current_path[0], vec![0, 1]);

        assert_eq!(loaded.agents.sim_time, agents.sim_time);
        assert!(loaded.allocator.buildings[0].center_x.is_finite());
        // frontage_node is persisted and round-trips as the saved node id (n1 maps to 1).
        assert_eq!(loaded.allocator.buildings[0].frontage_node, 1);
        assert!(!loaded.zoning.edge_grids[&0].left_occupied.is_empty());
    }
}
