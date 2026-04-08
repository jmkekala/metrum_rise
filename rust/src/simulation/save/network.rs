//! Road graph and lane connection serialization.

use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::pathing::cost::CostCalculator;
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::{Vector2, Vector3};
use rusqlite::{Connection, Transaction, params};
use std::collections::HashMap;

use super::schema::*;
use super::{SaveLoadError, SaveLoadResult, SnapshotMaps};
use super::{i64_to_i8, i64_to_u32, i64_to_usize, usize_to_i64};

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
        let mut stmt = tx.prepare("INSERT INTO network_edges(edge_id, start_node, end_node, primary_type, allowed_types, class, width, fwd_lanes, bkw_lanes, speed_limit, base_cost, physical_length, current_congestion, start_clip, end_clip, no_building_spawn, vehicle_frontage_access) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)")?;
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
                edge.current_congestion,
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

    // 4. Lane Connections
    {
        let mut stmt = tx.prepare("INSERT INTO lane_connections(node_id, from_edge, from_lane, to_edge, to_lane) VALUES (?1, ?2, ?3, ?4, ?5)")?;
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
                    stmt.execute(params![
                        i64::from(saved_node),
                        usize_to_i64(saved_from)?,
                        i64::from(from_l),
                        usize_to_i64(saved_to)?,
                        i64::from(to_l)
                    ])?;
                }
            }
        }
    }
    Ok(())
}

pub(super) fn load_graph(conn: &Connection) -> SaveLoadResult<RegionGraph> {
    // Forward-compatible migration: add no_building_spawn if the column is absent in older saves.
    let _ = conn.execute(
        "ALTER TABLE network_edges ADD COLUMN no_building_spawn INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE network_edges ADD COLUMN vehicle_frontage_access INTEGER NOT NULL DEFAULT 1",
        [],
    );
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
            if row.get::<_, bool>(5)? {
                physical_geometry
                    .entry(eid)
                    .or_insert_with(Vec::new)
                    .push(p);
            } else {
                geometry.entry(eid).or_insert_with(Vec::new).push(p);
            }
        }
    }

    {
        let mut stmt = conn.prepare("SELECT edge_id, start_node, end_node, primary_type, allowed_types, class, width, fwd_lanes, bkw_lanes, speed_limit, base_cost, physical_length, current_congestion, start_clip, end_clip, no_building_spawn, vehicle_frontage_access FROM network_edges ORDER BY edge_id")?;
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
            graph.add_edge(Edge {
                start_node: i64_to_u32(row.get(1)?)?,
                end_node: i64_to_u32(row.get(2)?)?,
                primary_type: transit_type_from_i64(row.get(3)?)?,
                allowed_types: (row.get::<_, i64>(4)?) as u8,
                class: edge_class_from_i64(row.get(5)?)?,
                width: row.get(6)?,
                fwd_lanes: (row.get::<_, i64>(7)?) as u8,
                bkw_lanes: (row.get::<_, i64>(8)?) as u8,
                speed_limit: row.get(9)?,
                base_cost: row.get(10)?,
                physical_length: row.get(11)?,
                current_congestion: row.get(12)?,
                start_clip: row.get(13)?,
                end_clip: row.get(14)?,
                geometry: geometry.remove(&eid).unwrap_or_default(),
                physical_geometry: physical_geometry.remove(&eid).unwrap_or_default(),
                deleted: false,
                no_building_spawn: row.get::<_, i64>(15)? != 0,
                vehicle_frontage_access: vehicle_frontage_access_from_i64(row.get(16)?)?,
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
    graph.rebuild_all_indices();
    Ok(graph)
}

pub(super) fn rebuild_loaded_graph_runtime(
    graph: &mut RegionGraph,
    transit_network: &mut TransitNetwork,
    terrain: &mut TerrainSystem,
) {
    graph.rebuild_all_indices();
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
    for edge in graph.edges_iter_mut() {
        if edge.deleted {
            continue;
        }
        let (base_cost, len) = CostCalculator::calculate_costs(edge);
        edge.base_cost = base_cost;
        edge.physical_length = len;
    }
    graph.rebuild_intersection_clips();
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
