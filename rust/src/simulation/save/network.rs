// SPDX-License-Identifier: GPL-3.0-or-later

//! Road graph and lane connection serialization.

use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::{
    Edge, MedianStyle, RegionGraph, RoadMedian, RoadParking, RoadPocket, RoundaboutRing,
    SidewalkSides,
};
use crate::simulation::network::roundabouts::validate_ring_edge;
use crate::simulation::network::validate::{
    RoadGeometryLimits, RoadGeometryTier, RoadGeometryViolation, grade_violation,
};
use godot::prelude::{Vector2, Vector3};
use rusqlite::{Connection, Transaction, params};
use std::collections::HashMap;

use super::schema::*;
use super::{SaveLoadError, SaveLoadResult, SnapshotMaps};
use super::{i64_to_i8, i64_to_u8, i64_to_u32, i64_to_usize, usize_to_i64};

pub(super) fn save_network(
    tx: &Transaction,
    graph: &RegionGraph,
    maps: &SnapshotMaps,
    lane_count: usize,
) -> SaveLoadResult<()> {
    // 0. Id counts and merge aliases (`SAVE-11`, format 121): ids are saved state, so a load
    // recreates the holes, deleted edges, merged nodes and retired lanes, at their ids.
    tx.execute(
        "INSERT INTO network_id_counts(singleton, node_count, edge_count, lane_count) VALUES (1, ?1, ?2, ?3)",
        params![
            usize_to_i64(graph.node_count())?,
            usize_to_i64(graph.edge_count())?,
            usize_to_i64(lane_count)?
        ],
    )?;
    {
        let mut stmt =
            tx.prepare("INSERT INTO network_node_aliases(node_id, canonical) VALUES (?1, ?2)")?;
        for (alias, canonical) in graph.node_alias_rows() {
            stmt.execute(params![i64::from(alias), i64::from(canonical)])?;
        }
    }

    // 1. Nodes: every canonical node at its id, including one whose roads were all removed (a
    // new road still snaps to it).
    {
        let mut stmt = tx.prepare(
            "INSERT INTO network_nodes(node_id, x, y, z, node_type) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for id in (0..graph.node_count() as u32).filter(|&id| maps.node(id).is_some()) {
            let node = graph.node(id);
            stmt.execute(params![
                i64::from(id),
                node.pos.x,
                node.pos.y,
                node.pos.z,
                node_type_to_i64(node.node_type)
            ])?;
        }
    }

    // 2. Edges: every live edge at its id.
    {
        let mut stmt = tx.prepare("INSERT INTO network_edges(edge_id, start_node, end_node, primary_type, allowed_types, class, width, fwd_lanes, bkw_lanes, speed_limit, base_cost, physical_length, start_clip, end_clip, no_building_spawn, vehicle_frontage_access, median_style, median_width, no_street_lights, pocket_end, pocket_roles, sidewalks, parking) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)")?;
        for (saved_id, edge) in graph.edges().iter().enumerate() {
            if edge.deleted {
                continue;
            }
            let row_node = |node: u32| {
                maps.node(node)
                    .ok_or_else(|| SaveLoadError::custom("edge names a merged or retired node"))
            };
            let start = row_node(edge.start_node)?;
            let end = row_node(edge.end_node)?;
            let (pocket_end, pocket_roles) = RoadPocket::codes(edge.pocket);
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
                vehicle_frontage_access_to_i64(edge.vehicle_frontage_access),
                i64::from(RoadMedian::style_code(edge.median)),
                edge.median.map_or(0.0, |median| median.width_m),
                i64::from(edge.no_street_lights),
                pocket_end,
                pocket_roles,
                i64::from(edge.sidewalks.bits()),
                edge.parking.code()
            ])?;
        }
    }

    // Roundabout ring edges (`ROAD-62`), a sparse table.
    {
        let mut stmt = tx.prepare(
            "INSERT INTO roundabout_edges(edge_id, centre_x, centre_z, radius_m) VALUES (?1, ?2, ?3, ?4)",
        )?;
        for (old_id, edge) in graph.edges().iter().enumerate() {
            let (Some(ring), Some(saved_id)) = (edge.roundabout, maps.edge(old_id)) else {
                continue;
            };
            stmt.execute(params![
                usize_to_i64(saved_id)?,
                ring.centre.x,
                ring.centre.y,
                ring.radius_m
            ])?;
        }
    }

    // 3. Geometry
    {
        let mut stmt = tx.prepare("INSERT INTO network_edge_geometry(edge_id, point_index, x, y, z, physical) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")?;
        for (old_id, edge) in graph.edges().iter().enumerate() {
            let Some(saved_id) = maps.edge(old_id) else {
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
        for saved_node in (0..graph.node_count() as u32).filter_map(|id| maps.node(id)) {
            let node = graph.node(saved_node);
            for (&(from_e, from_l), targets) in &node.lane_connections {
                let Some(saved_from) = maps.edge(from_e) else {
                    continue;
                };
                for &(to_e, to_l) in targets {
                    let Some(saved_to) = maps.edge(to_e) else {
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
    save_crosswalks(tx, graph, maps)
}

// 5. Crossing overrides (`ROAD-74`, save format 110): one row per authored arm that is still an
// arm of its node, in saved ids and key order. Overrides on arms an edit removed from the node
// are not saved. O(saved nodes x degree).
fn save_crosswalks(tx: &Transaction, graph: &RegionGraph, maps: &SnapshotMaps) -> SaveLoadResult<()> {
    let mut rows = Vec::new();
    for saved_node in (0..graph.node_count() as u32).filter_map(|id| maps.node(id)) {
        for (&edge, &marked) in &graph.node(saved_node).crosswalk_overrides {
            let Some(saved_edge) = maps.edge(edge) else {
                continue;
            };
            let arm = graph.edge(edge);
            let incident = [arm.start_node, arm.end_node]
                .into_iter()
                .any(|end| graph.get_valid_node(end) == saved_node);
            if incident {
                rows.push((saved_node, saved_edge, marked));
            }
        }
    }
    rows.sort_unstable();
    let mut stmt =
        tx.prepare("INSERT INTO road_crosswalks(node_id, edge_id, marked) VALUES (?1, ?2, ?3)")?;
    for (node, edge, marked) in rows {
        stmt.execute(params![i64::from(node), usize_to_i64(edge)?, marked])?;
    }
    Ok(())
}

/// Loads the road graph. With `saved_ids` (format 121, `SAVE-11`) the rows keep their ids and
/// the holes between them, merged nodes and deleted edges, are recreated at theirs from the
/// saved counts and alias table, so the loaded graph has the saved game's ids; an older save's
/// ids are contiguous. With `retired_holes` (format 122, `ROAD-124`) a node hole without an
/// alias is a retired node; format 121 refuses one. O(saved nodes + edges + aliases + geometry
/// points).
pub(super) fn load_graph(
    conn: &Connection,
    saved_ids: bool,
    retired_holes: bool,
) -> SaveLoadResult<RegionGraph> {
    let mut graph = RegionGraph::new();
    let counts = if saved_ids {
        let (nodes, edges): (i64, i64) = conn.query_row(
            "SELECT node_count, edge_count FROM network_id_counts WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Some((i64_to_usize(nodes)?, i64_to_usize(edges)?))
    } else {
        None
    };
    // Whether each node slot is a saved row rather than a hole.
    let mut row_nodes = Vec::new();
    {
        let mut stmt =
            conn.prepare("SELECT node_id, x, y, z, node_type FROM network_nodes ORDER BY node_id")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let node_id = i64_to_u32(row.get(0)?)?;
            if let Some((node_count, _)) = counts
                && (node_id as usize) < node_count
            {
                while graph.node_count() < node_id as usize {
                    graph.push_node_hole();
                    row_nodes.push(false);
                }
            }
            if node_id != graph.node_count() as u32 {
                return Err(SaveLoadError::custom(format!(
                    "network node ids must be ascending and within the saved count; found {} after {} nodes",
                    node_id,
                    graph.node_count()
                )));
            }
            graph.add_node(
                Vector3::new(row.get(1)?, row.get(2)?, row.get(3)?),
                node_type_from_i64(row.get(4)?)?,
            );
            row_nodes.push(true);
        }
    }
    if let Some((node_count, _)) = counts {
        while graph.node_count() < node_count {
            graph.push_node_hole();
            row_nodes.push(false);
        }
        load_node_aliases(conn, &mut graph, &row_nodes, retired_holes)?;
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
        let mut stmt = conn.prepare("SELECT edge_id, start_node, end_node, primary_type, allowed_types, class, width, fwd_lanes, bkw_lanes, speed_limit, base_cost, physical_length, start_clip, end_clip, no_building_spawn, vehicle_frontage_access, median_style, median_width, no_street_lights, pocket_end, pocket_roles, sidewalks, parking FROM network_edges ORDER BY edge_id")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let eid = i64_to_usize(row.get(0)?)?;
            if let Some((_, edge_count)) = counts
                && eid < edge_count
            {
                while graph.edge_count() < eid {
                    graph.push_edge_hole();
                }
            }
            if eid != graph.edge_count() {
                return Err(SaveLoadError::custom(format!(
                    "network edge ids must be ascending and within the saved count; found {} after {} edges",
                    eid,
                    graph.edge_count()
                )));
            }
            let speed_limit = row.get(9)?;
            // The saved flag is the road's own (`ROAD-101`: a fast road may zone).
            let no_building_spawn = row.get::<_, i64>(14)? != 0;
            let start_node = i64_to_u32(row.get(1)?)?;
            let end_node = i64_to_u32(row.get(2)?)?;
            let allowed_types = i64_to_u8(row.get(4)?)?;
            let fwd_lanes = i64_to_u8(row.get(7)?)?;
            let bkw_lanes = i64_to_u8(row.get(8)?)?;
            let invalid_median = |reason| {
                SaveLoadError::custom(format!("edge {eid} has an invalid median ({reason})"))
            };
            // Read as stored: a left pocket's edge carries its base road's median a lane
            // narrower (`ROAD-95`), so the rules check the base road's below.
            let median = match row.get::<_, i32>(16)? {
                0 => None,
                code => Some(RoadMedian {
                    style: MedianStyle::from_code(code)
                        .ok_or_else(|| invalid_median("unknown_median_style"))?,
                    width_m: row.get(17)?,
                }),
            };
            let pocket = RoadPocket::from_codes(row.get(19)?, row.get(20)?).map_err(|reason| {
                SaveLoadError::custom(format!("edge {eid} has an invalid pocket ({reason})"))
            })?;
            let sidewalks = SidewalkSides::from_bits(row.get(21)?).ok_or_else(|| {
                SaveLoadError::custom(format!("edge {eid} has an invalid sidewalk code"))
            })?;
            let parking = RoadParking::from_code(row.get(22)?)
                .ok_or("parking_code_out_of_range")
                .and_then(|parking| {
                    Edge::validate_parking(allowed_types, fwd_lanes, bkw_lanes, parking, sidewalks)
                        .map(|()| parking)
                })
                .map_err(|reason| {
                    SaveLoadError::custom(format!("edge {eid} has invalid parking ({reason})"))
                })?;
            if !(row_nodes.get(start_node as usize) == Some(&true)
                && row_nodes.get(end_node as usize) == Some(&true))
            {
                return Err(SaveLoadError::custom(
                    "edge references an out-of-range or merged node",
                ));
            }
            let edge = Edge {
                start_node,
                end_node,
                primary_type: transit_type_from_i64(row.get(3)?)?,
                allowed_types,
                class: edge_class_from_i64(row.get(5)?)?,
                width: row.get(6)?,
                fwd_lanes,
                bkw_lanes,
                speed_limit,
                base_cost: row.get(10)?,
                physical_length: row.get(11)?,
                start_clip: row.get(12)?,
                end_clip: row.get(13)?,
                geometry: geometry.remove(&eid).unwrap_or_default(),
                physical_geometry: physical_geometry.remove(&eid).unwrap_or_default(),
                deleted: false,
                no_building_spawn,
                no_street_lights: row.get::<_, i64>(18)? != 0,
                vehicle_frontage_access: vehicle_frontage_access_from_i64(row.get(15)?)?,
                median,
                parking,
                pocket,
                sidewalks,
                roundabout: None,
            };
            // A median loads only as a stroke or an edit could have built it (`ROAD-80`), a
            // pocket only as a pocket edit could have (`ROAD-95`), and sidewalks only on a car
            // road, which is walked exactly when it has one (`ROAD-107`).
            Edge::validate_median(allowed_types, fwd_lanes, bkw_lanes, edge.base_median())
                .map_err(invalid_median)?;
            edge.validate_pocket().map_err(|reason| {
                SaveLoadError::custom(format!("edge {eid} has an invalid pocket ({reason})"))
            })?;
            edge.validate_sidewalks().map_err(|reason| {
                SaveLoadError::custom(format!("edge {eid} has invalid sidewalks ({reason})"))
            })?;
            graph.add_edge(edge);
        }
        if let Some((_, edge_count)) = counts {
            while graph.edge_count() < edge_count {
                graph.push_edge_hole();
            }
        }
    }

    // A ring edge loads only as a roundabout edit could have built it (`ROAD-62`), once every
    // edge of its ring has its circle.
    {
        let mut stmt = conn.prepare(
            "SELECT edge_id, centre_x, centre_z, radius_m FROM roundabout_edges ORDER BY edge_id",
        )?;
        let mut rows = stmt.query([])?;
        let mut ring_edges = Vec::new();
        while let Some(row) = rows.next()? {
            let eid = i64_to_usize(row.get(0)?)?;
            if graph.get_edge(eid).is_none_or(|edge| edge.deleted) {
                return Err(SaveLoadError::custom("ring row names no edge"));
            }
            graph.edge_mut(eid).roundabout = Some(RoundaboutRing {
                centre: Vector2::new(row.get(1)?, row.get(2)?),
                radius_m: row.get(3)?,
            });
            ring_edges.push(eid);
        }
        for eid in ring_edges {
            validate_ring_edge(&graph, eid).map_err(|reason| {
                SaveLoadError::custom(format!("edge {eid} is an invalid ring edge ({reason})"))
            })?;
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
            if row_nodes.get(nid as usize) != Some(&true)
                || graph.get_edge(fe).is_none_or(|edge| edge.deleted)
                || graph.get_edge(te).is_none_or(|edge| edge.deleted)
            {
                return Err(SaveLoadError::custom(
                    "lane connection references out-of-range node or edge",
                ));
            }
            graph.add_lane_connection(nid, fe, fl, te, tl);
        }
    }
    {
        let mut stmt = conn.prepare(
            "SELECT node_id, edge_id, marked FROM road_crosswalks ORDER BY node_id, edge_id",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let node = i64_to_u32(row.get(0)?)?;
            let edge = i64_to_usize(row.get(1)?)?;
            let marked: bool = row.get(2)?;
            if row_nodes.get(node as usize) != Some(&true)
                || !graph.get_edge(edge).is_some_and(|arm| {
                    !arm.deleted && (arm.start_node == node || arm.end_node == node)
                })
            {
                return Err(SaveLoadError::custom(
                    "crosswalk override names an edge that is not an arm of its node",
                ));
            }
            graph.set_crosswalk_override(node, edge, marked);
        }
    }
    if !geometry.is_empty() || !physical_geometry.is_empty() {
        return Err(SaveLoadError::custom("orphan edge geometry"));
    }
    // add_node/add_edge already maintained adjacency and both spatial indices.
    Ok(graph)
}

// The saved merge aliases (`SAVE-11`): every alias is a hole, and each chain ends at a saved
// node. A hole without an alias is a retired node (`ROAD-124`) with `retired_holes`, else
// refused. O(holes + aliases x chain length), chains bounded by the node count.
fn load_node_aliases(
    conn: &Connection,
    graph: &mut RegionGraph,
    row_nodes: &[bool],
    retired_holes: bool,
) -> SaveLoadResult<()> {
    let mut stmt =
        conn.prepare("SELECT node_id, canonical FROM network_node_aliases ORDER BY node_id")?;
    let mut rows = stmt.query([])?;
    let mut aliases = HashMap::new();
    while let Some(row) = rows.next()? {
        let alias = i64_to_u32(row.get(0)?)?;
        let canonical = i64_to_u32(row.get(1)?)?;
        if row_nodes.get(alias as usize) != Some(&false)
            || (canonical as usize) >= row_nodes.len()
            || alias == canonical
        {
            return Err(SaveLoadError::custom(format!(
                "node alias {alias} -> {canonical} names no merged node"
            )));
        }
        aliases.insert(alias, canonical);
    }
    if !retired_holes && aliases.len() != row_nodes.iter().filter(|&&row| !row).count() {
        return Err(SaveLoadError::custom("a merged node has no saved alias"));
    }
    for (&alias, &canonical) in &aliases {
        let mut node = canonical;
        for _ in 0..row_nodes.len() {
            match aliases.get(&node) {
                Some(&next) => node = next,
                None => break,
            }
        }
        if !row_nodes[node as usize] {
            return Err(SaveLoadError::custom(format!(
                "node alias {alias} does not end at a saved node"
            )));
        }
        graph.restore_node_alias(alias, canonical);
    }
    if retired_holes {
        for (node, _) in row_nodes
            .iter()
            .enumerate()
            .filter(|&(node, &row)| !row && !aliases.contains_key(&(node as u32)))
        {
            graph.retire_node(node as u32);
        }
    }
    Ok(())
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
