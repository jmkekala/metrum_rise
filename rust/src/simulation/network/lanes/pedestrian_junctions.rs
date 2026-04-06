use godot::prelude::*;
use std::collections::HashMap;
use super::super::graph::RegionGraph;
use super::super::types::{TransitFlags, TransitType};
use super::{Lane, LaneType};
use super::geometry::{build_cum_dist, road_half_width};
use crate::config;

/// A classification of a sidewalk-end at a junction, used for sorting.
pub struct SidewalkMouth {
    /// Index of the road edge this mouth belongs to.
    pub edge_idx: usize,
    /// Lane index of the sidewalk (100 or -100).
    pub lane_idx: i8,
    /// Sorting angle in radians relative to the junction center.
    pub angle: f32,
    /// Unit direction vector pointing from the junction node into the road body.
    /// Used to detect same-road corridors: two mouths are on opposite sides of the
    /// same road through the junction when their `dir` vectors are anti-parallel.
    pub dir: Vector3,
    /// ID of the inbound lane relative to the junction.
    pub in_id: usize,
    /// ID of the outbound lane relative to the junction.
    pub out_id: usize,
    /// 3-D world position at the road mouth offset to the sidewalk centreline.
    /// Used for agent routing (pedestrians walk along sidewalks).
    pub mouth_world_pos: Vector3,
    /// 3-D world position at the road mouth offset to the asphalt edge (no sidewalk).
    /// Used for crosswalk geometry so stripes stay on the car lanes only.
    pub road_edge_pos: Vector3,
}

/// Returns the 3-D point `dist` metres along `geom` measured from the start (`from_start=true`)
/// or from the end (`from_start=false`). Clamps to the endpoint if `dist` exceeds the length.
fn walk_geom_from_end(geom: &[Vector3], dist: f32, from_start: bool) -> Vector3 {
    if geom.is_empty() { return Vector3::ZERO; }
    if dist <= 0.0 {
        return if from_start { geom[0] } else { *geom.last().unwrap() };
    }
    let mut remaining = dist;
    if from_start {
        for i in 0..geom.len().saturating_sub(1) {
            let seg = geom[i].distance_to(geom[i + 1]);
            if remaining <= seg || i == geom.len() - 2 {
                let t = (remaining / seg).min(1.0);
                return geom[i].lerp(geom[i + 1], t);
            }
            remaining -= seg;
        }
        geom[0]
    } else {
        let n = geom.len();
        for i in (1..n).rev() {
            let seg = geom[i - 1].distance_to(geom[i]);
            if remaining <= seg || i == 1 {
                let t = (remaining / seg).min(1.0);
                return geom[i].lerp(geom[i - 1], t);
            }
            remaining -= seg;
        }
        *geom.last().unwrap()
    }
}

/// Builds pedestrian sidewalk connection lanes (crosswalks) at a single node.
pub fn build_pedestrian_connections_at_node(
    lanes: &mut Vec<Lane>,
    lane_map: &HashMap<(usize, bool, i8), usize>,
    graph: &mut RegionGraph,
    node_id: usize,
    node_lanes: &mut HashMap<usize, Vec<usize>>,
) {
    let node_pos = graph.node(node_id as u32).pos;
    let adj: Vec<usize> = graph.node_adjacency(node_id as u32).to_vec();
    let mut mouths: Vec<SidewalkMouth> = Vec::new();

    for &e_idx in &adj {
        let edge = graph.edge(e_idx);
        if edge.deleted || (edge.allowed_types & TransitFlags::FOOT) == 0
            || edge.primary_type != TransitType::Road { continue; }

        let is_start = edge.start_node as usize == node_id;

        // Direction from node into the road body (used for angular sorting and offset).
        let other_p = if is_start { edge.geometry[1] } else { edge.geometry[edge.geometry.len()-2] };
        let diff = other_p - node_pos;
        let dist = other_p.distance_to(node_pos);
        let dir = if dist > 1e-4 { diff / dist } else { Vector3::ZERO };
        let side_vec = Vector3::new(-dir.z, 0.0, dir.x);

        // Position at the junction mouth — clip distance from the node along the road geometry.
        // This is where the road mesh visually starts/ends, and where crosswalks must sit.
        let clip = (if is_start { edge.start_clip } else { edge.end_clip }) + config::CROSSWALK_INSET;
        let mouth_center = walk_geom_from_end(&edge.geometry, clip, is_start);

        for &l_idx in &[-100_i8, 100_i8] {
            let side = (l_idx as f32) / 100.0;
            let offset = -(road_half_width(edge) + config::SIDEWALK_WIDTH * 0.5) * side;

            // Sorting position: 5 m into the road at the lateral offset (unchanged).
            let sort_pos = node_pos + dir * 5.0 + side_vec * offset;
            let mouth_angle = (sort_pos.x - node_pos.x).atan2(sort_pos.z - node_pos.z);

            // Sidewalk-centreline position: used for agent routing geometry.
            let mouth_world_pos = Vector3::new(
                mouth_center.x + side_vec.x * offset,
                mouth_center.y,
                mouth_center.z + side_vec.z * offset,
            );
            // Asphalt-edge position: used for crosswalk stripe geometry so stripes
            // stay on the car lanes and do not extend onto the sidewalk.
            let road_edge_offset = -road_half_width(edge) * side;
            let road_edge_pos = Vector3::new(
                mouth_center.x + side_vec.x * road_edge_offset,
                mouth_center.y,
                mouth_center.z + side_vec.z * road_edge_offset,
            );

            let (inbound, outbound) = if is_start {
                (lane_map.get(&(e_idx, false, l_idx)).copied(), lane_map.get(&(e_idx, true, l_idx)).copied())
            } else {
                (lane_map.get(&(e_idx, true, l_idx)).copied(), lane_map.get(&(e_idx, false, l_idx)).copied())
            };
            if let (Some(in_id), Some(out_id)) = (inbound, outbound) {
                mouths.push(SidewalkMouth {
                    edge_idx: e_idx,
                    lane_idx: l_idx,
                    angle: mouth_angle,
                    dir,
                    in_id,
                    out_id,
                    mouth_world_pos,
                    road_edge_pos,
                });
            }
        }
    }

    mouths.sort_by(|a, b| a.angle.partial_cmp(&b.angle).unwrap());
    let num_mouths = mouths.len();
    graph.nodes[node_id].lane_connections.clear();
    if num_mouths < 2 { return; }

    let deg = graph.node_adjacency(node_id as u32).len();

    // Pass 1 — crosswalks: one per road arm regardless of angular adjacency.
    // Same-arm mouths share an identical `dir` (derived from the same edge), so dot ≈ +1.
    // We process each unordered pair once (i < j) to avoid duplicate geometry.
    for i in 0..num_mouths {
        for j in (i + 1)..num_mouths {
            let dot = mouths[i].dir.dot(mouths[j].dir);
            if dot <= 0.99 { continue; }
            // Bends and pass-through nodes (deg ≤ 2) are not intersections; no crossings.
            if deg <= 2 { continue; }
            let steps = vec![mouths[i].road_edge_pos, mouths[j].road_edge_pos];
            let mut step_len = 0.0;
            for k in 0..steps.len().saturating_sub(1) { step_len += steps[k].distance_to(steps[k+1]); }
            let steps_cum = build_cum_dist(&steps);
            let conn_id = lanes.len();
            let m_start_in_id = mouths[i].in_id;
            let m_end_out_id = mouths[j].out_id;
            lanes.push(Lane {
                edge_id: usize::MAX,
                is_fwd: true,
                lane_idx: 0,
                geometry: steps,
                length: step_len,
                cum_dist: steps_cum,
                lane_type: LaneType::Foot,
                is_crosswalk: true,
                next_lanes: vec![m_end_out_id],
                node_id,
            });
            node_lanes.entry(node_id).or_default().push(conn_id);
            lanes[m_start_in_id].next_lanes.push(conn_id);
            let key = (mouths[i].edge_idx, mouths[i].lane_idx);
            let val = (mouths[j].edge_idx, mouths[j].lane_idx);
            graph.nodes[node_id].lane_connections.entry(key).or_default().push(val);
            // Also add reverse direction so agents can traverse from j→i.
            let conn_id2 = lanes.len();
            let steps2 = vec![mouths[j].mouth_world_pos, mouths[i].mouth_world_pos];
            let steps_cum2 = build_cum_dist(&steps2);
            lanes.push(Lane {
                edge_id: usize::MAX,
                is_fwd: true,
                lane_idx: 0,
                geometry: steps2,
                length: step_len,
                cum_dist: steps_cum2,
                lane_type: LaneType::Foot,
                is_crosswalk: false, // reverse not rendered, only forward stripe shown
                next_lanes: vec![mouths[i].out_id],
                node_id,
            });
            node_lanes.entry(node_id).or_default().push(conn_id2);
            lanes[mouths[j].in_id].next_lanes.push(conn_id2);
            let key2 = (mouths[j].edge_idx, mouths[j].lane_idx);
            let val2 = (mouths[i].edge_idx, mouths[i].lane_idx);
            graph.nodes[node_id].lane_connections.entry(key2).or_default().push(val2);
        }
    }

    // Pass 2 — routing connections between angularly adjacent mouths of different arms.
    for i in 0..num_mouths {
        for j in 0..num_mouths {
            if i == j { continue; }
            let dot = mouths[i].dir.dot(mouths[j].dir);
            if dot > 0.99 { continue; } // already handled as crosswalk
            let diff_cw = if j > i { j - i } else { j + num_mouths - i };
            let diff_ccw = num_mouths - diff_cw;
            let use_cw = diff_cw <= diff_ccw;
            let num_steps = if use_cw { diff_cw } else { diff_ccw };
            if num_steps > 1 { continue; }

            // Routing connection: agents walk from i's inbound end to j's outbound start.
            let mut s: Vec<Vector3> = Vec::new();
            let mut current = i;
            for _ in 0..num_steps {
                let next = if use_cw {
                    (current + 1) % num_mouths
                } else {
                    (current + num_mouths - 1) % num_mouths
                };
                let p0 = *lanes[mouths[current].in_id].geometry.last().unwrap();
                let p1 = lanes[mouths[next].out_id].geometry[0];
                if s.is_empty() { s.push(p0); }
                s.push(p1);
                current = next;
            }
            let mut step_len = 0.0;
            for k in 0..s.len().saturating_sub(1) { step_len += s[k].distance_to(s[k+1]); }
            let steps_cum = build_cum_dist(&s);
            let conn_id = lanes.len();
            let m_start_in_id = mouths[i].in_id;
            let m_end_out_id = mouths[j].out_id;
            lanes.push(Lane {
                edge_id: usize::MAX,
                is_fwd: true,
                lane_idx: 0,
                geometry: s,
                length: step_len,
                cum_dist: steps_cum,
                lane_type: LaneType::Foot,
                is_crosswalk: false,
                next_lanes: vec![m_end_out_id],
                node_id,
            });
            node_lanes.entry(node_id).or_default().push(conn_id);
            lanes[m_start_in_id].next_lanes.push(conn_id);
            let key = (mouths[i].edge_idx, mouths[i].lane_idx);
            let val = (mouths[j].edge_idx, mouths[j].lane_idx);
            graph.nodes[node_id].lane_connections.entry(key).or_default().push(val);
        }
    }
}
