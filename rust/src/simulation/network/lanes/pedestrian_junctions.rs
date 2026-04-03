use godot::prelude::*;
use std::collections::HashMap;
use super::super::graph::RegionGraph;
use super::super::types::{NodeType, TransitFlags, TransitType};
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
    /// ID of the inbound lane relative to the junction.
    pub in_id: usize,
    /// ID of the outbound lane relative to the junction.
    pub out_id: usize,
}

/// Builds pedestrian sidewalk connection lanes (crosswalks) at a single node.
pub fn build_pedestrian_connections_at_node(
    lanes: &mut Vec<Lane>,
    lane_map: &HashMap<(usize, bool, i8), usize>,
    graph: &mut RegionGraph,
    node_id: usize,
) {
    let node_pos = graph.nodes[node_id].pos;
    let adj: Vec<usize> = graph.adjacency[node_id].clone();
    let mut mouths: Vec<SidewalkMouth> = Vec::new();

    for &e_idx in &adj {
        let edge = &graph.edges[e_idx];
        if edge.deleted || (edge.allowed_types & TransitFlags::FOOT) == 0
            || edge.primary_type != TransitType::Road { continue; }

        let is_start = edge.start_node as usize == node_id;
        let other_p = if is_start { edge.geometry[1] } else { edge.geometry[edge.geometry.len()-2] };
        let diff = other_p - node_pos;
        let dist = other_p.distance_to(node_pos);
        let dir = if dist > 1e-4 { diff / dist } else { Vector3::ZERO };
        let side_vec = Vector3::new(-dir.z, 0.0, dir.x);

        for &l_idx in &[-100_i8, 100_i8] {
            let side = (l_idx as f32) / 100.0;
            let offset = -(road_half_width(edge) + config::SIDEWALK_WIDTH * 0.5) * side;
            let mouth_pos = node_pos + dir * 5.0 + side_vec * offset;
            let mouth_angle = (mouth_pos.x - node_pos.x).atan2(mouth_pos.z - node_pos.z);
            let (inbound, outbound) = if is_start {
                (lane_map.get(&(e_idx, false, l_idx)).copied(), lane_map.get(&(e_idx, true, l_idx)).copied())
            } else {
                (lane_map.get(&(e_idx, true, l_idx)).copied(), lane_map.get(&(e_idx, false, l_idx)).copied())
            };
            if let (Some(in_id), Some(out_id)) = (inbound, outbound) {
                mouths.push(SidewalkMouth { edge_idx: e_idx, lane_idx: l_idx, angle: mouth_angle, in_id, out_id });
            }
        }
    }

    mouths.sort_by(|a, b| a.angle.partial_cmp(&b.angle).unwrap());
    let num_mouths = mouths.len();
    graph.nodes[node_id].lane_connections.clear();
    if num_mouths < 2 { return; }

    let mut crosswalks_added = 0;
    for i in 0..num_mouths {
        for j in 0..num_mouths {
            if i == j { continue; }
            let diff_cw = if j > i { j - i } else { j + num_mouths - i };
            let diff_ccw = num_mouths - diff_cw;
            let use_cw = diff_cw <= diff_ccw;
            let num_steps = if use_cw { diff_cw } else { diff_ccw };
            if num_steps > 1 { continue; }

            let mut steps = Vec::new();
            let mut current = i;
            for _ in 0..num_steps {
                let next = if use_cw { (current+1)%num_mouths } else { (current+num_mouths-1)%num_mouths };
                let p0 = *lanes[mouths[current].in_id].geometry.last().unwrap();
                let p1 = lanes[mouths[next].out_id].geometry[0];
                if steps.is_empty() { steps.push(p0); }
                steps.push(p1);
                current = next;
            }

            let is_same_edge = mouths[i].edge_idx == mouths[j].edge_idx;
            let deg = graph.adjacency[node_id].len();
            let node_type = graph.nodes[node_id].node_type;

            if node_type == NodeType::Frontage && (is_same_edge || mouths[i].lane_idx != mouths[j].lane_idx) {
                continue;
            }

            let skip_visual = is_same_edge && deg <= 2 && crosswalks_added >= 2;
            let is_crosswalk = is_same_edge && num_steps == 1 && !skip_visual;
            if is_crosswalk { crosswalks_added += 1; }

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
                is_crosswalk,
                next_lanes: vec![m_end_out_id],
            });
            lanes[m_start_in_id].next_lanes.push(conn_id);

            let key = (mouths[i].edge_idx, mouths[i].lane_idx);
            let val = (mouths[j].edge_idx, mouths[j].lane_idx);
            graph.nodes[node_id].lane_connections.entry(key).or_default().push(val);
        }
    }
}
