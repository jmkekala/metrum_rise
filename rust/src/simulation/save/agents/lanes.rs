// SPDX-License-Identifier: GPL-2.0-only

//! Snapshot lane identities and smoothed traffic state against the rebuilt lane graph.

use super::*;
use crate::simulation::economy::agents::lane_state::lane_for_mode;
use crate::simulation::network::lanes::{LaneSystem, LaneType};

#[cfg(test)]
mod tests;

/// Writes directed road identities and connector endpoints once per live lane.
pub(super) fn save_lane_references(
    tx: &Transaction,
    graph: &RegionGraph,
    network: &TransitNetwork,
    maps: &SnapshotMaps,
) -> SaveLoadResult<()> {
    tx.execute(
        "INSERT INTO network_runtime_state(singleton, frontage_delay_elapsed_s) VALUES (1, ?1)",
        [network.frontage_delay_elapsed_s],
    )?;
    let mut stmt = tx.prepare("INSERT INTO saved_lanes(lane_id, edge_id, is_fwd, lane_idx, node_id, from_lane_id, to_lane_id, frontage_delay_penalty_s, live_frontage_delay_s) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)")?;
    // Visit only live indexed lanes; incremental rebuilds leave tombstones in storage.
    // Each connector has one incoming road lane and one outgoing road lane.
    for edge_id in 0..graph.edge_count() {
        let Some(ids) = network.lane_system.edge_lanes.get(&edge_id) else {
            continue;
        };
        for &id in ids {
            let lane = &network.lane_system.lanes[id];
            stmt.execute(params![
                usize_to_i64(id)?,
                optional_edge_to_db(edge_id, maps)?,
                lane.is_fwd,
                lane.lane_idx,
                -1_i64,
                -1_i64,
                -1_i64,
                lane.frontage_delay_penalty_s,
                lane.live_frontage_delay_s
            ])?;
            for &conn_id in &lane.next_lanes {
                let conn = &network.lane_system.lanes[conn_id];
                if conn.edge_id != usize::MAX {
                    continue;
                }
                let Some(&to_id) = conn.next_lanes.first() else {
                    return Err(SaveLoadError::custom(
                        "cannot save connector without an outgoing lane",
                    ));
                };
                stmt.execute(params![
                    usize_to_i64(conn_id)?,
                    -1_i64,
                    conn.is_fwd,
                    conn.lane_idx,
                    optional_node_to_db(graph, conn.node_id as u32, maps)?,
                    usize_to_i64(id)?,
                    usize_to_i64(to_id)?,
                    conn.frontage_delay_penalty_s,
                    conn.live_frontage_delay_s
                ])?;
            }
        }
    }
    Ok(())
}

/// Restores current and planned IDs after rebuilding lanes and building entrances. Returns the
/// map from saved lane ids to rebuilt ones, for the other saved lane references.
pub(in crate::simulation::save) fn restore_lane_references(
    conn: &Connection,
    agents: &mut AgentSystem,
    graph: &RegionGraph,
    network: &mut TransitNetwork,
) -> SaveLoadResult<HashMap<usize, usize>> {
    let lanes = &network.lane_system;
    let mut remap = HashMap::new();
    let mut roads = conn.prepare("SELECT lane_id, edge_id, is_fwd, lane_idx FROM saved_lanes WHERE edge_id != -1 ORDER BY lane_id")?;
    let mut rows = roads.query([])?;
    while let Some(row) = rows.next()? {
        let old = i64_to_usize(row.get(0)?)?;
        let edge = i64_to_usize(row.get(1)?)?;
        let fwd: bool = row.get(2)?;
        let idx: i8 = row.get(3)?;
        let id = lanes
            .edge_lanes
            .get(&edge)
            .and_then(|ids| {
                ids.iter().copied().find(|&id| {
                    let lane = &lanes.lanes[id];
                    lane.is_fwd == fwd && lane.lane_idx == idx
                })
            })
            .ok_or_else(|| {
                SaveLoadError::custom(format!("saved lane {old} has no matching road lane"))
            })?;
        remap.insert(old, id);
    }
    let mut connectors = conn.prepare("SELECT lane_id, node_id, from_lane_id, to_lane_id FROM saved_lanes WHERE edge_id = -1 ORDER BY lane_id")?;
    let mut rows = connectors.query([])?;
    while let Some(row) = rows.next()? {
        let old = i64_to_usize(row.get(0)?)?;
        let node = i64_to_usize(row.get(1)?)?;
        let from = i64_to_usize(row.get(2)?)?;
        let to = i64_to_usize(row.get(3)?)?;
        // Saves from before footpath arms stopped counting as vehicle junction arms hold a
        // connector where the road now links straight through. Agents on it replan below.
        if let (Some(&from), Some(&to)) = (remap.get(&from), remap.get(&to))
            && lanes.lanes[from].next_lanes.contains(&to)
        {
            continue;
        }
        let ends = remap.get(&from).zip(remap.get(&to));
        let id = ends.and_then(|(&from, &to)| {
            lanes.lanes[from].next_lanes.iter().copied().find(|&id| {
                let lane = &lanes.lanes[id];
                lane.node_id == node && lane.next_lanes.first() == Some(&to)
            })
        });
        match (id, ends) {
            (Some(id), _) => {
                remap.insert(old, id);
            }
            // Pedestrian routes follow the junction layouts since `ROAD-68-C`: saves from before
            // hold crossings the layout has not got (road ends, which now wrap their caps) and
            // routes through them, as can a save from a build whose crossing rule differed.
            // Such a connector is omitted; agents on it replan below.
            (None, Some((&from, _))) if lanes.lanes[from].lane_type == LaneType::Foot => {}
            (None, _) => {
                return Err(SaveLoadError::custom(format!(
                    "saved connector {old} has no matching junction route"
                )));
            }
        }
    }
    // Saves written before ROAD-65 can hold lane ids a terrain edit or undo had renumbered:
    // a lane the save no longer lists, or one of the agent's mode owned by another edge or
    // junction. Such an agent is taken off the network to replan rather than refusing the
    // whole save. A lane of the wrong mode still refuses it (TRAFFIC-02).
    let mut repaired = 0;
    for i in 0..agents.len() {
        let old = agents.current_lane_id[i];
        if old != usize::MAX {
            let mode = agents.transit_mode[i];
            match remap.get(&old).copied() {
                Some(id) if validate_current_lane(agents, i, id, graph, lanes).is_ok() => {
                    agents.current_lane_id[i] = id;
                }
                Some(id) if lane_for_mode(lanes, id, mode).is_err() => {
                    validate_current_lane(agents, i, id, graph, lanes)?;
                }
                _ => {
                    clear_agent_travel_state(agents, i);
                    repaired += 1;
                }
            }
        }
        let changing_from = agents.lane_change_from_lane_id[i];
        if changing_from != u32::MAX {
            agents.lane_change_from_lane_id[i] = *remap
                .get(&(changing_from as usize))
                .ok_or_else(|| SaveLoadError::custom("unknown saved lane-change origin"))?
                as u32;
        }
        agents.planned_attach_lane_id[i] = remap
            .get(&(agents.planned_attach_lane_id[i] as usize))
            .copied()
            .map(|id| id as u32)
            .unwrap_or(u32::MAX);
        agents.planned_detach_lane_id[i] = remap
            .get(&(agents.planned_detach_lane_id[i] as usize))
            .copied()
            .map(|id| id as u32)
            .unwrap_or(u32::MAX);
    }
    if repaired > 0 {
        crate::debug_log!(
            "save",
            "loaded_agent_lane_repair agents={repaired}: stale lane ids sent to replan"
        );
    }
    validate_loaded_planned_lane_ids(agents, lanes.lanes.len());
    network.frontage_delay_elapsed_s = conn.query_row(
        "SELECT frontage_delay_elapsed_s FROM network_runtime_state WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;
    let mut delays = conn.prepare(
        "SELECT lane_id, frontage_delay_penalty_s, live_frontage_delay_s FROM saved_lanes ORDER BY lane_id",
    )?;
    let mut rows = delays.query([])?;
    while let Some(row) = rows.next()? {
        let old = i64_to_usize(row.get(0)?)?;
        let price: f32 = row.get(1)?;
        let live: f32 = row.get(2)?;
        if [price, live]
            .iter()
            .any(|delay| !delay.is_finite() || *delay < 0.0)
        {
            return Err(SaveLoadError::custom("invalid saved frontage delay"));
        }
        let Some(&id) = remap.get(&old) else {
            // A connector deliberately omitted above has no live lane whose delay can be
            // restored. Any agent that referenced it has already been moved to replanning.
            continue;
        };
        let lane = &mut network.lane_system.lanes[id];
        lane.frontage_delay_penalty_s = price;
        lane.live_frontage_delay_s = live;
    }
    if !network.frontage_delay_elapsed_s.is_finite() || network.frontage_delay_elapsed_s < 0.0 {
        return Err(SaveLoadError::custom("invalid saved frontage delay clock"));
    }
    Ok(remap)
}

/// Checks the shared mode/ownership contract for a snapshot lane.
pub(super) fn validate_current_lane(
    agents: &AgentSystem,
    i: usize,
    id: usize,
    graph: &RegionGraph,
    lanes: &LaneSystem,
) -> SaveLoadResult<()> {
    use crate::simulation::economy::agents::lane_state::{lane_for_mode, validate_lane_owner};
    let check = || {
        let lane = lane_for_mode(lanes, id, agents.transit_mode[i])?;
        if lane.edge_id != usize::MAX && lane.edge_id >= graph.edge_count() {
            return Err("lane road owner is out of bounds");
        }
        validate_lane_owner(
            lane,
            agents.current_edge[i],
            agents.current_node[i],
            agents.transit[i],
        )
    };
    check().map_err(|reason| {
        SaveLoadError::custom(format!(
            "agent {i} lane {id} mode={} edge={} node={} transit={}: {reason}",
            agents.transit_mode[i],
            agents.current_edge[i],
            agents.current_node[i],
            agents.transit[i]
        ))
    })
}
