// SPDX-License-Identifier: GPL-2.0-only

//! Road graph and lane connection serialization.

use crate::config::HIGH_SPEED_ROAD_THRESHOLD_MS;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::validate::{
    RoadGeometryLimits, RoadGeometryTier, RoadGeometryViolation, grade_violation,
};
use godot::prelude::Vector3;
use rusqlite::{Connection, Transaction, params};
use std::collections::HashMap;

use super::schema::*;
use super::{SaveLoadError, SaveLoadResult, SnapshotMaps};
use super::{i64_to_i8, i64_to_u8, i64_to_u32, i64_to_usize, usize_to_i64};

pub(super) fn save_network(
    tx: &Transaction,
    graph: &RegionGraph,
    maps: &SnapshotMaps,
) -> SaveLoadResult<()> {
    // 1. Nodes
    {
        let mut node_rows: Vec<(u32, u32)> = maps
            .node_old_to_new
            .iter()
            .map(|(&old, &new)| (new, old))
            .collect();
        node_rows.sort_by_key(|&(new, _)| new);
        let mut stmt = tx.prepare(
            "INSERT INTO network_nodes(node_id, x, y, z, node_type) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for (saved_id, old_id) in node_rows {
            let node = graph.node(old_id);
            stmt.execute(params![
                i64::from(saved_id),
                node.pos.x,
                node.pos.y,
                node.pos.z,
                node_type_to_i64(node.node_type)
            ])?;
        }
    }

    // 2. Edges
    {
        let mut stmt = tx.prepare("INSERT INTO network_edges(edge_id, start_node, end_node, primary_type, allowed_types, class, width, fwd_lanes, bkw_lanes, speed_limit, base_cost, physical_length, start_clip, end_clip, no_building_spawn, vehicle_frontage_access) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)")?;
        for (old_id, edge) in graph.edges().iter().enumerate() {
            let Some(&saved_id) = maps.edge_old_to_new.get(&old_id) else {
                continue;
            };
            let start = maps
                .node_old_to_new
                .get(&canonical_existing_node(graph, edge.start_node)?)
                .copied()
                .ok_or_else(|| SaveLoadError::custom("missing saved start node"))?;
            let end = maps
                .node_old_to_new
                .get(&canonical_existing_node(graph, edge.end_node)?)
                .copied()
                .ok_or_else(|| SaveLoadError::custom("missing saved end node"))?;
            stmt.execute(params![
                usize_to_i64(saved_id)?,
                i64::from(start),
                i64::from(end),
                transit_type_to_i64(edge.primary_type),
                i64::from(edge.allowed_types),
                edge_class_to_i64(edge.class),
                edge.width,
                i64::from(edge.fwd_lanes),
                i64::from(edge.bkw_lanes),
                edge.speed_limit,
                edge.base_cost,
                edge.physical_length,
                edge.start_clip,
                edge.end_clip,
                i64::from(edge.no_building_spawn),
                vehicle_frontage_access_to_i64(edge.vehicle_frontage_access)
            ])?;
        }
    }

    // 3. Geometry
    {
        let mut stmt = tx.prepare("INSERT INTO network_edge_geometry(edge_id, point_index, x, y, z, physical) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")?;
        for (old_id, edge) in graph.edges().iter().enumerate() {
            let Some(&saved_id) = maps.edge_old_to_new.get(&old_id) else {
                continue;
            };
            for (idx, p) in edge.geometry.iter().enumerate() {
                stmt.execute(params![
                    usize_to_i64(saved_id)?,
                    usize_to_i64(idx)?,
                    p.x,
                    p.y,
                    p.z,
                    false
                ])?;
            }
            for (idx, p) in edge.physical_geometry.iter().enumerate() {
                stmt.execute(params![
                    usize_to_i64(saved_id)?,
                    usize_to_i64(idx)?,
                    p.x,
                    p.y,
                    p.z,
                    true
                ])?;
            }
        }
    }

    // 4. Lane Connections, in the saved key order the load reads them in: the node map and
    // each node's connection map iterate in hash order, which differs between processes and
    // after a load, so rows written in it made the same city save different bytes.
    {
        let mut rows = Vec::new();
        for (&old_node, &saved_node) in &maps.node_old_to_new {
            let node = graph.node(old_node);
            for (&(from_e, from_l), targets) in &node.lane_connections {
                let Some(&saved_from) = maps.edge_old_to_new.get(&from_e) else {
                    continue;
                };
                for &(to_e, to_l) in targets {
                    let Some(&saved_to) = maps.edge_old_to_new.get(&to_e) else {
                        continue;
                    };
                    rows.push((saved_node, saved_from, from_l, saved_to, to_l));
                }
            }
        }
        rows.sort_unstable();
        let mut stmt = tx.prepare("INSERT INTO lane_connections(node_id, from_edge, from_lane, to_edge, to_lane) VALUES (?1, ?2, ?3, ?4, ?5)")?;
        for (saved_node, saved_from, from_l, saved_to, to_l) in rows {
            stmt.execute(params![
                i64::from(saved_node),
                usize_to_i64(saved_from)?,
                i64::from(from_l),
                usize_to_i64(saved_to)?,
                i64::from(to_l)
            ])?;
        }
    }
    Ok(())
}

pub(super) fn load_graph(conn: &Connection) -> SaveLoadResult<RegionGraph> {
    let mut graph = RegionGraph::new();
    {
        let mut stmt =
            conn.prepare("SELECT node_id, x, y, z, node_type FROM network_nodes ORDER BY node_id")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let node_id = i64_to_u32(row.get(0)?)?;
            if node_id != graph.node_count() as u32 {
                return Err(SaveLoadError::custom(format!(
                    "network node ids must be contiguous; found {} after {} nodes",
                    node_id,
                    graph.node_count()
                )));
            }
            graph.add_node(
                Vector3::new(row.get(1)?, row.get(2)?, row.get(3)?),
                node_type_from_i64(row.get(4)?)?,
            );
        }
    }

    let (mut geometry, mut physical_geometry) = (HashMap::new(), HashMap::new());
    {
        let mut stmt = conn.prepare("SELECT edge_id, point_index, x, y, z, physical FROM network_edge_geometry ORDER BY edge_id, physical, point_index")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let eid = i64_to_usize(row.get(0)?)?;
            let p = Vector3::new(row.get(2)?, row.get(3)?, row.get(4)?);
            let points = if row.get::<_, bool>(5)? {
                physical_geometry.entry(eid).or_insert_with(Vec::new)
            } else {
                geometry.entry(eid).or_insert_with(Vec::new)
            };
            if i64_to_usize(row.get(1)?)? != points.len() {
                return Err(SaveLoadError::custom("non-contiguous edge geometry points"));
            }
            points.push(p);
        }
    }

    {
        let mut stmt = conn.prepare("SELECT edge_id, start_node, end_node, primary_type, allowed_types, class, width, fwd_lanes, bkw_lanes, speed_limit, base_cost, physical_length, start_clip, end_clip, no_building_spawn, vehicle_frontage_access FROM network_edges ORDER BY edge_id")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let eid = i64_to_usize(row.get(0)?)?;
            if eid != graph.edge_count() {
                return Err(SaveLoadError::custom(format!(
                    "network edge ids must be contiguous; found {} after {} edges",
                    eid,
                    graph.edge_count()
                )));
            }
            let speed_limit = row.get(9)?;
            let no_building_spawn =
                row.get::<_, i64>(14)? != 0 || speed_limit >= HIGH_SPEED_ROAD_THRESHOLD_MS;
            let start_node = i64_to_u32(row.get(1)?)?;
            let end_node = i64_to_u32(row.get(2)?)?;
            if start_node as usize >= graph.node_count() || end_node as usize >= graph.node_count()
            {
                return Err(SaveLoadError::custom("edge references out-of-range node"));
            }
            graph.add_edge(Edge {
                start_node,
                end_node,
                primary_type: transit_type_from_i64(row.get(3)?)?,
                allowed_types: i64_to_u8(row.get(4)?)?,
                class: edge_class_from_i64(row.get(5)?)?,
                width: row.get(6)?,
                fwd_lanes: i64_to_u8(row.get(7)?)?,
                bkw_lanes: i64_to_u8(row.get(8)?)?,
                speed_limit,
                base_cost: row.get(10)?,
                physical_length: row.get(11)?,
                start_clip: row.get(12)?,
                end_clip: row.get(13)?,
                geometry: geometry.remove(&eid).unwrap_or_default(),
                physical_geometry: physical_geometry.remove(&eid).unwrap_or_default(),
                deleted: false,
                no_building_spawn,
                vehicle_frontage_access: vehicle_frontage_access_from_i64(row.get(15)?)?,
            });
        }
    }

    {
        let mut stmt = conn.prepare("SELECT node_id, from_edge, from_lane, to_edge, to_lane FROM lane_connections ORDER BY node_id, from_edge, from_lane, to_edge, to_lane")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let nid = i64_to_u32(row.get(0)?)?;
            let fe = i64_to_usize(row.get(1)?)?;
            let fl = i64_to_i8(row.get(2)?)?;
            let te = i64_to_usize(row.get(3)?)?;
            let tl = i64_to_i8(row.get(4)?)?;
            if nid as usize >= graph.node_count()
                || fe >= graph.edge_count()
                || te >= graph.edge_count()
            {
                return Err(SaveLoadError::custom(
                    "lane connection references out-of-range node or edge",
                ));
            }
            graph.add_lane_connection(nid, fe, fl, te, tl);
        }
    }
    if !geometry.is_empty() || !physical_geometry.is_empty() {
        return Err(SaveLoadError::custom("orphan edge geometry"));
    }
    // add_node/add_edge already maintained adjacency and both spatial indices.
    Ok(graph)
}

pub(super) fn rebuild_loaded_graph_runtime(
    graph: &RegionGraph,
    transit_network: &mut TransitNetwork,
) -> SaveLoadResult<()> {
    // Node positions, grades, physical lengths and junction clips are saved authority, loaded
    // as saved or refused, never repaired: save compatibility is not required (`ROAD-68` open
    // question 37, decided 2026-10-26). Terrain synchronization is an edit: running it here
    // moves roads away from their restored parcels and changes the boundary geometry consumed
    // by the renderer.
    refuse_steep_road(graph)?;
    transit_network.rebuild_roadbed(graph);
    // Every node is checked as a commit checks it (`ROAD-68-E`): its piece builds, and a bend
    // or junction has a profile plane.
    if let Some(&node) = transit_network.roadbed.failed_nodes().first() {
        return Err(SaveLoadError::custom(format!(
            "road surface load failed: node {node} piece did not build ({:?})",
            transit_network.roadbed.node_error(node)
        )));
    }
    if let Some(&node) = transit_network.roadbed.unpinned_nodes(graph).first() {
        return Err(SaveLoadError::custom(format!(
            "road surface load failed: node {node} has no profile plane (NoPlane)"
        )));
    }
    Ok(())
}

// A road steeper than the 1:3 maximum fails the load with `road_too_steep`, the rule an edit
// meets (`ROAD-68` open question 37): the validator's grade check on every saved profile, the
// first such edge in id order named. O(saved profile points), no allocation.
fn refuse_steep_road(graph: &RegionGraph) -> SaveLoadResult<()> {
    let limits = RoadGeometryLimits::default();
    for edge in 0..graph.edge_count() {
        if let Some(RoadGeometryViolation::Grade {
            station_m,
            measured,
            limit,
            tier: RoadGeometryTier::Illegal,
            ..
        }) = grade_violation(graph, edge, &limits)
        {
            return Err(SaveLoadError::custom(format!(
                "road surface load failed: edge {edge} climbs at {:.1}% at {station_m:.1} m, \
                 steeper than the {:.0}% (1:3) maximum grade (road_too_steep)",
                measured * 100.0,
                limit * 100.0
            )));
        }
    }
    Ok(())
}

pub(super) fn canonical_existing_node(graph: &RegionGraph, node_id: u32) -> SaveLoadResult<u32> {
    if (node_id as usize) >= graph.node_count() {
        return Err(SaveLoadError::custom(format!(
            "node {} out of bounds",
            node_id
        )));
    }
    let c = graph.get_valid_node(node_id);
    if (c as usize) >= graph.node_count() {
        return Err(SaveLoadError::custom(format!(
            "canonical node {} out of bounds",
            c
        )));
    }
    Ok(c)
}
