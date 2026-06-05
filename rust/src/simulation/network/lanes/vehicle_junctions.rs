use super::super::graph::RegionGraph;
use super::geometry::build_cum_dist;
use super::{Lane, LaneType};
use godot::prelude::*;
use std::collections::HashMap;

const VEHICLE_CONNECTION_MAX_STEP_M: f32 = 1.0;
const VEHICLE_CONNECTION_MIN_STEPS: usize = 8;
const VEHICLE_CONNECTION_MAX_STEPS: usize = 64;

/// Builds vehicle intersection connection lanes at a single node.
pub fn build_vehicle_connections_at_node(
    lanes: &mut Vec<Lane>,
    lane_map: &HashMap<(usize, bool, i8), usize>,
    graph: &RegionGraph,
    node_id: usize,
    node_lanes: &mut std::collections::HashMap<usize, Vec<usize>>,
) {
    let mut inbound: Vec<(usize, i8, usize)> = Vec::new();
    let mut outbound: Vec<(usize, i8, usize)> = Vec::new();

    for &e_idx in graph.node_adjacency(node_id as u32) {
        let edge = graph.edge(e_idx);
        if edge.deleted {
            continue;
        }

        if edge.start_node as usize == node_id {
            for l in 0..edge.fwd_lanes {
                if let Some(&lid) = lane_map.get(&(e_idx, true, l as i8)) {
                    outbound.push((e_idx, l as i8, lid));
                }
            }
            for l in 0..edge.bkw_lanes {
                if let Some(&lid) = lane_map.get(&(e_idx, false, -(l as i8) - 1)) {
                    inbound.push((e_idx, -(l as i8) - 1, lid));
                }
            }
        }

        if edge.end_node as usize == node_id {
            for l in 0..edge.fwd_lanes {
                if let Some(&lid) = lane_map.get(&(e_idx, true, l as i8)) {
                    inbound.push((e_idx, l as i8, lid));
                }
            }
            for l in 0..edge.bkw_lanes {
                if let Some(&lid) = lane_map.get(&(e_idx, false, -(l as i8) - 1)) {
                    outbound.push((e_idx, -(l as i8) - 1, lid));
                }
            }
        }
    }

    let lane_conns = &graph.node(node_id as u32).lane_connections;
    let node_deg = graph.node_adjacency_count_at(node_id as u32);

    for &(in_edge_id, in_lane_idx, in_lane_id) in &inbound {
        let mut allowed: Option<Vec<(usize, i8)>> =
            lane_conns.get(&(in_edge_id, in_lane_idx)).cloned();

        // Global whitelist mode: if the node has ANY user vehicle connection (lane_idx ≠ ±100),
        // ALL unspecified turns are blocked. Only explicitly listed turns are permitted.
        // Nodes with no user connections remain fully open (allow all non-U-turns).
        if allowed.is_none() {
            let node_has_any_conn = lane_conns
                .keys()
                .any(|&(_, lane_idx)| lane_idx != 100 && lane_idx != -100);
            if !node_has_any_conn {
                // Open node: allow all non-U-turn outbound lanes.
                let mut defaults = Vec::new();
                for &(out_edge_id, out_lane_idx, _) in &outbound {
                    if out_edge_id != in_edge_id || node_deg == 1 {
                        defaults.push((out_edge_id, out_lane_idx));
                    }
                }
                if !defaults.is_empty() {
                    allowed = Some(defaults);
                }
            }
            // If node_has_any_conn but this arm has no explicit connection, allowed stays
            // None → no junction connection lanes created → turn is blocked.
        }

        let mut valid_outs = Vec::new();
        for &(out_edge_id, out_lane_idx, out_lid) in &outbound {
            if let Some(rules) = &allowed {
                if rules.contains(&(out_edge_id, out_lane_idx)) {
                    valid_outs.push(out_lid);
                }
            }
        }

        for out_lid in valid_outs {
            let p0 = *lanes[in_lane_id].geometry.last().unwrap();
            let p1_base = {
                let g = &lanes[in_lane_id].geometry;
                if g.len() >= 2 {
                    let d = g[g.len() - 1] - g[g.len() - 2];
                    if d.length_squared() > 0.00001 {
                        d.normalized()
                    } else {
                        Vector3::new(1.0, 0.0, 0.0)
                    }
                } else {
                    Vector3::new(1.0, 0.0, 0.0)
                }
            };
            let p3 = lanes[out_lid].geometry[0];
            let p2_base = {
                let g = &lanes[out_lid].geometry;
                if g.len() >= 2 {
                    let d = g[1] - g[0];
                    if d.length_squared() > 0.00001 {
                        d.normalized()
                    } else {
                        Vector3::new(1.0, 0.0, 0.0)
                    }
                } else {
                    Vector3::new(1.0, 0.0, 0.0)
                }
            };

            let dist = p0.distance_to(p3);
            let cd = dist * 0.35;
            let p1 = p0 + p1_base * cd;
            let p2 = p3 - p2_base * cd;

            let steps = vehicle_connection_steps(dist);
            let mut conn_geom = Vec::with_capacity(steps + 1);
            let mut conn_len = 0.0;
            for k in 0..=steps {
                let t = k as f32 / steps as f32;
                let mut p = (1.0 - t).powi(3) * p0
                    + 3.0 * (1.0 - t).powi(2) * t * p1
                    + 3.0 * (1.0 - t) * t.powi(2) * p2
                    + t.powi(3) * p3;
                p.y = p0.y + (p3.y - p0.y) * t;
                conn_geom.push(p);
                if k > 0 {
                    conn_len += conn_geom[k - 1].distance_to(p);
                }
            }

            let conn_cum = build_cum_dist(&conn_geom);
            let conn_id = lanes.len();
            lanes.push(Lane {
                edge_id: usize::MAX,
                is_fwd: true,
                lane_idx: 0,
                geometry: conn_geom,
                length: conn_len,
                frontage_delay_penalty_s: 0.0,
                cum_dist: conn_cum,
                lane_type: LaneType::Vehicle,
                is_crosswalk: false,
                next_lanes: vec![out_lid],
                node_id,
            });
            node_lanes.entry(node_id).or_default().push(conn_id);
            lanes[in_lane_id].next_lanes.push(conn_id);
        }
    }
}

fn vehicle_connection_steps(chord_distance_m: f32) -> usize {
    ((chord_distance_m / VEHICLE_CONNECTION_MAX_STEP_M).ceil() as usize)
        .clamp(VEHICLE_CONNECTION_MIN_STEPS, VEHICLE_CONNECTION_MAX_STEPS)
}
