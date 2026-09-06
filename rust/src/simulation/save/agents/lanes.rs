// SPDX-License-Identifier: GPL-2.0-only

//! Snapshot lane identities and restoration against the rebuilt lane graph.

use super::*;
use crate::simulation::economy::agents::MODE_WALK;
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
    let mut stmt = tx.prepare("INSERT INTO saved_lanes(lane_id, edge_id, is_fwd, lane_idx, node_id, from_lane_id, to_lane_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)")?;
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
                -1_i64
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
                    usize_to_i64(to_id)?
                ])?;
            }
        }
    }
    Ok(())
}

/// Restores current and planned IDs after rebuilding lanes and building entrances.
pub(in crate::simulation::save) fn restore_lane_references(
    conn: &Connection,
    version: i64,
    agents: &mut AgentSystem,
    graph: &RegionGraph,
    network: &TransitNetwork,
    allocator: &BuildingAllocator,
) -> SaveLoadResult<()> {
    if version == 57 {
        return restore_v57_lane_references(agents, graph, network, allocator);
    }
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
        let id = remap
            .get(&from)
            .zip(remap.get(&to))
            .and_then(|(&from, &to)| {
                lanes.lanes[from].next_lanes.iter().copied().find(|&id| {
                    let lane = &lanes.lanes[id];
                    lane.node_id == node && lane.next_lanes.first() == Some(&to)
                })
            })
            .ok_or_else(|| {
                SaveLoadError::custom(format!(
                    "saved connector {old} has no matching junction route"
                ))
            })?;
        remap.insert(old, id);
    }
    for i in 0..agents.len() {
        let old = agents.current_lane_id[i];
        if old != usize::MAX {
            let id = remap.get(&old).copied().ok_or_else(|| {
                SaveLoadError::custom(format!("agent {i} has unknown saved lane {old}"))
            })?;
            validate_current_lane(agents, i, id, graph, lanes)?;
            agents.current_lane_id[i] = id;
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
    validate_loaded_planned_lane_ids(agents, lanes.lanes.len());
    Ok(())
}

fn validate_current_lane(
    agents: &AgentSystem,
    i: usize,
    id: usize,
    graph: &RegionGraph,
    lanes: &LaneSystem,
) -> SaveLoadResult<()> {
    let lane = &lanes.lanes[id];
    let desired = if agents.transit_mode[i] == MODE_WALK {
        LaneType::Foot
    } else {
        LaneType::Vehicle
    };
    let owner_ok = if lane.edge_id == usize::MAX {
        agents.transit[i] == TRANSIT_INTERSECTION && lane.node_id == agents.current_node[i] as usize
    } else {
        lane.edge_id == agents.current_edge[i] && lane.edge_id < graph.edge_count()
    };
    if lane.lane_type != desired || !owner_ok {
        return Err(SaveLoadError::custom(format!(
            "agent {i} lane does not match its travel mode or owner"
        )));
    }
    Ok(())
}

fn restore_v57_lane_references(
    agents: &mut AgentSystem,
    graph: &RegionGraph,
    network: &TransitNetwork,
    allocator: &BuildingAllocator,
) -> SaveLoadResult<()> {
    let lanes = &network.lane_system;
    for i in 0..agents.len() {
        // v57 planned IDs referred to the pre-save (possibly incremental) lane array.
        // Reconstruct the destination from its building side and directed graph node.
        agents.planned_detach_lane_id[i] = entrance_lane(
            agents.target_building[i],
            agents.transit_mode[i],
            agents.planned_detach_node[i],
            false,
            allocator,
            graph,
            lanes,
        )
        .map(|id| id as u32)
        .unwrap_or(u32::MAX);
        agents.planned_attach_lane_id[i] = entrance_lane(
            agents.current_building[i],
            agents.transit_mode[i],
            agents.planned_attach_node[i],
            true,
            allocator,
            graph,
            lanes,
        )
        .map(|id| id as u32)
        .unwrap_or(u32::MAX);

        let saved_idx = agents.current_lane_id[i] as i64;
        if matches!(agents.transit[i], TRANSIT_NETWORK | TRANSIT_INTERSECTION) {
            if agents.current_edge[i] != usize::MAX {
                // -1 is both the old missing-lane sentinel and the backward vehicle lane.
                let ids = lanes.edge_lanes.get(&agents.current_edge[i]);
                let desired = if agents.transit_mode[i] == MODE_WALK {
                    LaneType::Foot
                } else {
                    LaneType::Vehicle
                };
                let id = ids.into_iter().flatten().copied().find(|&id| {
                    let lane = &lanes.lanes[id];
                    if lane.lane_type != desired || i64::from(lane.lane_idx) != saved_idx {
                        return false;
                    }
                    if desired == LaneType::Vehicle {
                        return true;
                    }
                    let edge = graph.edge(lane.edge_id);
                    (if lane.is_fwd {
                        edge.start_node
                    } else {
                        edge.end_node
                    }) == agents.current_node[i]
                });
                agents.current_lane_id[i] = id.unwrap_or(usize::MAX);
            } else if agents.transit[i] == TRANSIT_INTERSECTION {
                agents.current_lane_id[i] = restore_v57_connector(agents, i, graph, lanes)?;
            } else {
                agents.current_lane_id[i] = usize::MAX;
            }
        } else {
            agents.current_lane_id[i] = usize::MAX;
        }
    }
    validate_loaded_planned_lane_ids(agents, lanes.lanes.len());
    Ok(())
}

fn entrance_lane(
    building: usize,
    mode: u8,
    node: u32,
    attach: bool,
    allocator: &BuildingAllocator,
    graph: &RegionGraph,
    lanes: &LaneSystem,
) -> Option<usize> {
    let entrance = allocator.entrances.get(building)?;
    let ids = if mode == MODE_WALK {
        [entrance.foot_lane_fwd, entrance.foot_lane_bkw]
    } else {
        [entrance.car_lane_fwd, entrance.car_lane_bkw]
    };
    ids.into_iter().find(|&id| {
        lanes.lanes.get(id).is_some_and(|lane| {
            let edge = graph.edge(lane.edge_id);
            (if lane.is_fwd == attach {
                edge.end_node
            } else {
                edge.start_node
            }) == node
        })
    })
}

fn restore_v57_connector(
    agents: &AgentSystem,
    i: usize,
    graph: &RegionGraph,
    lanes: &LaneSystem,
) -> SaveLoadResult<usize> {
    let node = agents.current_node[i];
    let path = &agents.current_path[i];
    let idx = agents.current_path_index[i];
    let to_edge = path
        .get(idx)
        .and_then(|&next| graph.get_edge_between_nodes(node, next));
    let detach = agents.planned_detach_lane_id[i] as usize;
    let desired = if agents.transit_mode[i] == MODE_WALK {
        LaneType::Foot
    } else {
        LaneType::Vehicle
    };
    let id = lanes
        .node_lanes
        .get(&(node as usize))
        .into_iter()
        .flatten()
        .copied()
        .find_map(|id| {
            let lane = &lanes.lanes[id];
            if lane.lane_type != desired {
                return None;
            }
            let &to = lane.next_lanes.first()?;
            if to_edge.is_some_and(|edge| lanes.lanes[to].edge_id != edge)
                || (to_edge.is_none() && to != detach)
            {
                return None;
            }
            if saved_pose_matches(agents, i, lane, agents.lane_distance[i]) {
                return Some(id);
            }
            None
        });
    id.ok_or_else(|| {
        SaveLoadError::custom(format!(
            "cannot restore v57 junction lane for agent {i} at node {node}"
        ))
    })
}

fn saved_pose_matches(
    agents: &AgentSystem,
    i: usize,
    lane: &crate::simulation::network::lanes::Lane,
    distance: f32,
) -> bool {
    let Some(pos) = crate::simulation::network::lanes::geometry::agent_lane_position(
        lane,
        distance,
        (agents.transit_mode[i] == MODE_WALK).then_some(i),
    ) else {
        return false;
    };
    // Eight f32 ULPs at the saved world scale cover interpolation/height-sync rounding.
    let tolerance = 8.0 * f32::EPSILON * agents.pos_x[i].abs().max(agents.pos_y[i].abs()).max(1.0);
    (pos.x - agents.pos_x[i]).abs() <= tolerance && (pos.z - agents.pos_y[i]).abs() <= tolerance
}
