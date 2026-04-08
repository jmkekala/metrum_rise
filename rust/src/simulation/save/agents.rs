//! Agent SoA and path serialization.

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::{
    ACCESS_IMMIGRATION_ORIGIN, ACCESS_PLAN_VALID, Agent, AgentSystem, MODE_CAR,
    TRANSIT_IMMIGRATING, TRANSIT_IN_BUILDING, TRANSIT_INTERSECTION, TRANSIT_NETWORK,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use rusqlite::{Connection, Transaction, params};
use std::collections::HashMap;

use super::{SaveLoadError, SaveLoadResult, SnapshotMaps};
use super::{
    db_to_optional_usize, i64_to_u8, i64_to_u32, i64_to_usize, optional_building_to_db,
    optional_edge_to_db, u32_to_i64, usize_to_i64,
};

pub(super) struct LoadedAgentRecord {
    pub home_building: usize,
    pub household_id: usize,
    pub work_building: usize,
    pub current_building: usize,
    pub target_building: usize,
    pub current_node: u32,
    pub target_node: u32,
    pub planned_attach_node: u32,
    pub planned_detach_node: u32,
    pub planned_attach_lane_id: u32,
    pub planned_detach_lane_id: u32,
    pub planned_attach_lane_d: f32,
    pub planned_detach_lane_d: f32,
    pub access_flags: u8,
    pub next_replan_time: f32,
    pub current_edge: usize,
    pub current_lane_id: i64,
    pub lane_distance: f32,
    pub pos_x: f32,
    pub pos_y: f32,
    pub is_visible: bool,
    pub activity: u8,
    pub transit: u8,
    pub transit_mode: u8,
    pub happiness: f32,
    pub money: f32,
    pub journey_start_time: f32,
    pub has_car: bool,
    pub vehicle_type: u8,
    pub current_path_index: usize,
    pub current_path: Vec<u32>,
    pub pedestrian_type: u8,
    pub walk_phase: f32,
}

pub(super) fn save_agents(
    tx: &Transaction,
    agents: &AgentSystem,
    graph: &RegionGraph,
    network: &TransitNetwork,
    maps: &SnapshotMaps,
) -> SaveLoadResult<()> {
    let mut stmt = tx.prepare("INSERT INTO agents(agent_id, home_building, household_id, work_building, current_building, target_building, current_node, target_node, planned_attach_node, planned_detach_node, planned_attach_lane_id, planned_detach_lane_id, planned_attach_lane_d, planned_detach_lane_d, access_flags, next_replan_time, current_edge, current_lane_id, lane_distance, pos_x, pos_y, is_visible, activity, transit, transit_mode, pedestrian_side, happiness, money, journey_start_time, has_car, vehicle_type, current_path_index) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32)")?;
    let mut path_stmt = tx.prepare(
        "INSERT INTO agent_path_nodes(agent_id, step_index, node_id) VALUES (?1, ?2, ?3)",
    )?;

    for i in 0..agents.len() {
        let cn = super::network::canonical_existing_node(graph, agents.current_node[i])?;
        let tn = super::network::canonical_existing_node(graph, agents.target_node[i])?;
        let scn = maps
            .node_old_to_new
            .get(&cn)
            .copied()
            .ok_or_else(|| SaveLoadError::custom("missing current node"))?;
        let stn = maps
            .node_old_to_new
            .get(&tn)
            .copied()
            .ok_or_else(|| SaveLoadError::custom("missing target node"))?;
        let span = if agents.planned_attach_node[i] != u32::MAX {
            let canon =
                super::network::canonical_existing_node(graph, agents.planned_attach_node[i])?;
            maps.node_old_to_new
                .get(&canon)
                .copied()
                .ok_or_else(|| SaveLoadError::custom("missing planned attach node"))?
        } else {
            u32::MAX
        };
        let spdn = if agents.planned_detach_node[i] != u32::MAX {
            let canon =
                super::network::canonical_existing_node(graph, agents.planned_detach_node[i])?;
            maps.node_old_to_new
                .get(&canon)
                .copied()
                .ok_or_else(|| SaveLoadError::custom("missing planned detach node"))?
        } else {
            u32::MAX
        };

        stmt.execute(params![
            usize_to_i64(i)?,
            optional_building_to_db(agents.home_building[i], maps)?,
            if agents.household_id[i] == usize::MAX {
                -1_i64
            } else {
                usize_to_i64(agents.household_id[i])?
            },
            optional_building_to_db(agents.work_building[i], maps)?,
            optional_building_to_db(agents.current_building[i], maps)?,
            optional_building_to_db(agents.target_building[i], maps)?,
            i64::from(scn),
            i64::from(stn),
            u32_to_i64(span)?,
            u32_to_i64(spdn)?,
            u32_to_i64(agents.planned_attach_lane_id[i])?,
            u32_to_i64(agents.planned_detach_lane_id[i])?,
            agents.planned_attach_lane_d[i],
            agents.planned_detach_lane_d[i],
            i64::from(agents.access_flags[i]),
            agents.next_replan_time[i],
            optional_edge_to_db(agents.current_edge[i], maps)?,
            if agents.current_lane_id[i] != usize::MAX {
                Some(network.lane_system.lanes[agents.current_lane_id[i]].lane_idx as i64)
            } else {
                Some(-1)
            },
            agents.lane_distance[i],
            agents.pos_x[i],
            agents.pos_y[i],
            agents.is_visible[i],
            i64::from(agents.activity[i]),
            i64::from(agents.transit[i]),
            i64::from(agents.transit_mode[i]),
            0_i64,
            agents.happiness[i],
            agents.money[i],
            agents.journey_start_time[i],
            agents.has_car[i],
            i64::from(agents.vehicle_type[i]),
            usize_to_i64(agents.current_path_index[i])?
        ])?;

        for (idx, &nid) in agents.current_path[i].iter().enumerate() {
            let canon = super::network::canonical_existing_node(graph, nid)?;
            let snid = maps
                .node_old_to_new
                .get(&canon)
                .copied()
                .ok_or_else(|| SaveLoadError::custom("missing path node"))?;
            path_stmt.execute(params![
                usize_to_i64(i)?,
                usize_to_i64(idx)?,
                i64::from(snid)
            ])?;
        }
    }
    Ok(())
}

pub(super) fn load_agents(conn: &Connection, sim_time: f32) -> SaveLoadResult<AgentSystem> {
    let _ = conn.execute(
        "ALTER TABLE agents ADD COLUMN planned_attach_node INTEGER NOT NULL DEFAULT 4294967295",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE agents ADD COLUMN planned_detach_node INTEGER NOT NULL DEFAULT 4294967295",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE agents ADD COLUMN planned_attach_lane_id INTEGER NOT NULL DEFAULT 4294967295",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE agents ADD COLUMN planned_detach_lane_id INTEGER NOT NULL DEFAULT 4294967295",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE agents ADD COLUMN planned_attach_lane_d REAL NOT NULL DEFAULT 0.0",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE agents ADD COLUMN planned_detach_lane_d REAL NOT NULL DEFAULT 0.0",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE agents ADD COLUMN access_flags INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE agents ADD COLUMN next_replan_time REAL NOT NULL DEFAULT 0.0",
        [],
    );

    let mut car_paths = HashMap::new();
    {
        let mut stmt = conn.prepare("SELECT agent_id, step_index, node_id FROM agent_path_nodes ORDER BY agent_id, step_index")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let aid = i64_to_usize(row.get(0)?)?;
            car_paths
                .entry(aid)
                .or_insert_with(Vec::new)
                .push(i64_to_u32(row.get(2)?)?);
        }
    }

    let mut agents = AgentSystem::new();
    agents.sim_time = sim_time;
    {
        let mut stmt = conn.prepare("SELECT agent_id, home_building, household_id, work_building, current_building, target_building, current_node, target_node, planned_attach_node, planned_detach_node, planned_attach_lane_id, planned_detach_lane_id, planned_attach_lane_d, planned_detach_lane_d, access_flags, next_replan_time, current_edge, current_lane_id, lane_distance, pos_x, pos_y, is_visible, activity, transit, transit_mode, pedestrian_side, happiness, money, journey_start_time, has_car, vehicle_type, current_path_index FROM agents ORDER BY agent_id")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let aid = i64_to_usize(row.get(0)?)?;
            if aid != agents.len() {
                return Err(SaveLoadError::custom("non-contiguous agent ids"));
            }
            push_loaded_agent(
                &mut agents,
                LoadedAgentRecord {
                    home_building: db_to_optional_usize(row.get(1)?)?,
                    household_id: db_to_optional_usize(row.get(2)?)?,
                    work_building: db_to_optional_usize(row.get(3)?)?,
                    current_building: db_to_optional_usize(row.get(4)?)?,
                    target_building: db_to_optional_usize(row.get(5)?)?,
                    current_node: i64_to_u32(row.get(6)?)?,
                    target_node: i64_to_u32(row.get(7)?)?,
                    planned_attach_node: i64_to_u32(row.get(8)?)?,
                    planned_detach_node: i64_to_u32(row.get(9)?)?,
                    planned_attach_lane_id: i64_to_u32(row.get(10)?)?,
                    planned_detach_lane_id: i64_to_u32(row.get(11)?)?,
                    planned_attach_lane_d: row.get(12)?,
                    planned_detach_lane_d: row.get(13)?,
                    access_flags: i64_to_u8(row.get(14)?)?,
                    next_replan_time: row.get(15)?,
                    current_edge: db_to_optional_usize(row.get(16)?)?,
                    current_lane_id: row.get(17)?,
                    lane_distance: row.get(18)?,
                    pos_x: row.get(19)?,
                    pos_y: row.get(20)?,
                    is_visible: row.get(21)?,
                    activity: i64_to_u8(row.get(22)?)?,
                    transit: i64_to_u8(row.get(23)?)?,
                    transit_mode: i64_to_u8(row.get(24)?)?,
                    happiness: row.get(26)?,
                    money: row.get(27)?,
                    journey_start_time: row.get(28)?,
                    has_car: row.get(29)?,
                    vehicle_type: i64_to_u8(row.get(30)?)?,
                    current_path_index: i64_to_usize(row.get(31)?)?,
                    current_path: car_paths.remove(&aid).unwrap_or_default(),
                    pedestrian_type: 0,
                    walk_phase: 0.0,
                },
            );
        }
    }
    if !car_paths.is_empty() {
        return Err(SaveLoadError::custom("orphan agent path nodes"));
    }
    Ok(agents)
}

pub(super) fn push_loaded_agent(agents: &mut AgentSystem, rec: LoadedAgentRecord) {
    agents.agents.push(Agent {
        home_building: rec.home_building,
        household_id: rec.household_id,
        work_building: rec.work_building,
        pos_x: rec.pos_x,
        pos_y: rec.pos_y,
        is_visible: rec.is_visible,
        activity: rec.activity,
        transit: rec.transit,
        happiness: rec.happiness,
        money: rec.money,
        journey_start_time: rec.journey_start_time,
        current_building: rec.current_building,
        target_building: rec.target_building,
        planned_target_building: usize::MAX,
        current_node: rec.current_node,
        target_node: rec.target_node,
        planned_attach_node: rec.planned_attach_node,
        planned_detach_node: rec.planned_detach_node,
        planned_attach_lane_id: rec.planned_attach_lane_id,
        planned_detach_lane_id: rec.planned_detach_lane_id,
        planned_attach_lane_d: rec.planned_attach_lane_d,
        planned_detach_lane_d: rec.planned_detach_lane_d,
        access_flags: rec.access_flags,
        next_replan_time: rec.next_replan_time,
        current_edge: rec.current_edge,
        current_lane_id: rec.current_lane_id as usize,
        lane_distance: rec.lane_distance,
        speed: if rec.transit_mode == MODE_CAR {
            20.0
        } else {
            4.0
        },
        transit_mode: rec.transit_mode,
        planned_activity: 0,
        current_path: rec.current_path,
        current_path_index: rec.current_path_index,
        has_car: rec.has_car,
        vehicle_type: rec.vehicle_type,
        pedestrian_type: rec.pedestrian_type,
        walk_phase: rec.walk_phase,
    });
}

pub(super) fn validate_loaded_agents(
    agents: &mut AgentSystem,
    graph: &RegionGraph,
    allocator: &BuildingAllocator,
) -> SaveLoadResult<()> {
    for i in 0..agents.len() {
        if (agents.current_node[i] as usize) >= graph.node_count()
            || (agents.target_node[i] as usize) >= graph.node_count()
        {
            return Err(SaveLoadError::custom(format!("agent {} has bad node", i)));
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
        let mut clear = false;
        if agents.current_building[i] != usize::MAX
            && agents.current_building[i] >= allocator.buildings.len()
        {
            agents.current_building[i] = usize::MAX;
            clear = true;
        }
        if agents.target_building[i] != usize::MAX
            && agents.target_building[i] >= allocator.buildings.len()
        {
            agents.target_building[i] = usize::MAX;
            clear = true;
        }
        if agents.current_path_index[i] > agents.current_path[i].len() {
            clear = true;
        }
        if agents.current_path[i]
            .iter()
            .any(|&nid| (nid as usize) >= graph.node_count())
        {
            clear = true;
        }
        if !agents.next_replan_time[i].is_finite() {
            clear_agent_access_plan(agents, i);
        }
        if !agents.planned_attach_lane_d[i].is_finite() || agents.planned_attach_lane_d[i] < 0.0 {
            clear_agent_access_plan(agents, i);
        }
        if !agents.planned_detach_lane_d[i].is_finite() || agents.planned_detach_lane_d[i] < 0.0 {
            clear_agent_access_plan(agents, i);
        }
        if agents.access_flags[i] & ACCESS_PLAN_VALID == 0 {
            clear_agent_access_plan(agents, i);
        } else {
            let attach_ok = if agents.access_flags[i] & ACCESS_IMMIGRATION_ORIGIN != 0 {
                agents.planned_attach_node[i] < graph.node_count() as u32
                    && agents.planned_attach_lane_id[i] == u32::MAX
            } else {
                agents.planned_attach_node[i] < graph.node_count() as u32
                    && agents.planned_attach_lane_id[i] != u32::MAX
            };
            let detach_ok = agents.planned_detach_node[i] < graph.node_count() as u32
                && agents.planned_detach_lane_id[i] != u32::MAX;
            if !attach_ok || !detach_ok || (agents.access_flags[i] & 0xF0) != 0 {
                clear_agent_access_plan(agents, i);
            }
        }
        if agents.lane_distance[i] < 0.0 || !agents.lane_distance[i].is_finite() {
            clear = true;
        }
        if agents.current_edge[i] != usize::MAX {
            if agents.current_edge[i] >= graph.edge_count()
                || graph.edge(agents.current_edge[i]).deleted
            {
                clear = true;
            } else {
                if agents.transit_mode[i] != MODE_CAR {
                    agents.current_lane_id[i] = usize::MAX;
                }
            }
        }
        if clear {
            clear_agent_travel_state(agents, i);
        }
    }
    Ok(())
}

fn clear_agent_travel_state(agents: &mut AgentSystem, i: usize) {
    agents.current_path[i].clear();
    agents.current_path_index[i] = 0;
    agents.current_edge[i] = usize::MAX;
    agents.current_lane_id[i] = usize::MAX;
    agents.lane_distance[i] = 0.0;
    clear_agent_access_plan(agents, i);
    if agents.transit[i] == TRANSIT_INTERSECTION {
        agents.transit[i] = TRANSIT_NETWORK;
    } else if agents.transit[i] != TRANSIT_IN_BUILDING && agents.transit[i] != TRANSIT_IMMIGRATING {
        agents.transit[i] = TRANSIT_NETWORK;
    }
}

fn clear_agent_access_plan(agents: &mut AgentSystem, i: usize) {
    agents.planned_attach_node[i] = u32::MAX;
    agents.planned_detach_node[i] = u32::MAX;
    agents.planned_attach_lane_id[i] = u32::MAX;
    agents.planned_detach_lane_id[i] = u32::MAX;
    agents.planned_attach_lane_d[i] = 0.0;
    agents.planned_detach_lane_d[i] = 0.0;
    agents.access_flags[i] = 0;
    agents.next_replan_time[i] = 0.0;
}

pub(super) fn validate_loaded_planned_lane_ids(agents: &mut AgentSystem, lane_count: usize) {
    for i in 0..agents.len() {
        if agents.access_flags[i] & ACCESS_PLAN_VALID == 0 {
            clear_agent_access_plan(agents, i);
            continue;
        }

        let attach_ok = if agents.access_flags[i] & ACCESS_IMMIGRATION_ORIGIN != 0 {
            agents.planned_attach_lane_id[i] == u32::MAX
        } else {
            agents.planned_attach_lane_id[i] != u32::MAX
                && (agents.planned_attach_lane_id[i] as usize) < lane_count
        };
        let detach_ok = agents.planned_detach_lane_id[i] != u32::MAX
            && (agents.planned_detach_lane_id[i] as usize) < lane_count;

        if !attach_ok || !detach_ok {
            clear_agent_access_plan(agents, i);
        }
    }
}
