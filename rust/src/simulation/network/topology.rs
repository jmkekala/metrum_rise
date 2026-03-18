use std::collections::HashMap;
use crate::config;
use super::TransitNetwork;
use super::graph::Edge;
use super::types::NodeType;
use super::interaction;

pub fn process_intersections(network: &mut TransitNetwork, edge_id: usize) {
    let mut all_splits: HashMap<usize, Vec<(f32, u32)>> = HashMap::new();
    
    // 1. Find all intersections (crossing AND touching/snapping)
    let edge_count = network.graph.edges.len();
    for other_id in 0..edge_count {
        let (edge1_geo, edge2_geo) = {
            (network.graph.edges[edge_id].geometry.clone(), network.graph.edges[other_id].geometry.clone())
        };

        // Check crossing segments
        for i in 0..edge1_geo.len() - 1 {
            let d1 = edge1_geo[i+1] - edge1_geo[i];
            let l1 = d1.length();
            if l1 < 0.001 { continue; }
            let v1 = d1 / l1;
            
            for j in 0..edge2_geo.len() - 1 {
                // Ignore adjacent segments AND segments very close in sequence (jitter protection)
                if edge_id == other_id && (i as i32 - j as i32).abs() < 30 { continue; }

                let d2 = edge2_geo[j+1] - edge2_geo[j];
                let l2 = d2.length();
                if l2 < 0.001 { continue; }
                let v2 = d2 / l2;
                
                let dot = v1.dot(v2).abs();
                if dot > 0.98 { continue; } // Near parallel: Ignore for crossing check to avoid 'ghost' junctions

                if let Some((t, u)) = interaction::find_intersection_2d(
                    edge1_geo[i], edge1_geo[i+1],
                    edge2_geo[j], edge2_geo[j+1]
                ) {
                    let factor_t = i as f32 + t;
                    let factor_u = j as f32 + u;
                    let pos = edge1_geo[i].lerp(edge1_geo[i+1], t);
                    
                    // Unified Node Capture
                    let junction_id = network.graph.find_or_add_node(pos, config::INTERSECTION_TOLERANCE, NodeType::Junction);

                    all_splits.entry(edge_id).or_default().push((factor_t, junction_id));
                    all_splits.entry(other_id).or_default().push((factor_u, junction_id));
                }
            }
        }

        // Explicitly check endpoints of the new road against segments of other roads (Snapping/Touching)
        let endpoints = [edge1_geo[0], edge1_geo[edge1_geo.len()-1]];
        for (idx, &p) in endpoints.iter().enumerate() {
            let factor_t = if idx == 0 { 0.0 } else { (edge1_geo.len() - 1) as f32 };
            
            for j in 0..edge2_geo.len() - 1 {
                if edge_id == other_id { continue; }
                let p1 = edge2_geo[j];
                let p2 = edge2_geo[j+1];
                let closest = interaction::get_closest_point_on_segment(p, p1, p2);
                
                if p.distance_to(closest) < config::INTERSECTION_TOLERANCE { 
                    let factor_u = j as f32 + (closest - p1).length() / (p2 - p1).length().max(0.001);
                    
                    // Unified Node Capture
                    let junction_id = network.graph.find_or_add_node(closest, config::INTERSECTION_TOLERANCE, NodeType::Junction);

                    all_splits.entry(edge_id).or_default().push((factor_t, junction_id));
                    all_splits.entry(other_id).or_default().push((factor_u, junction_id));
                }
            }
        }
    }

    // 2. Process splits for each edge
    for (eid, mut splits) in all_splits {
        splits.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        splits.dedup_by(|a, b| a.1 == b.1 || (a.0 - b.0).abs() < 5.0);

        for (factor, junction_id) in splits {
            let seg_idx = factor.floor() as usize;
            let sub_t = factor.fract();
            
            let geo_len = network.graph.edges[eid].geometry.len();
            if factor < 0.01 {
                let start_node = network.graph.edges[eid].start_node;
                network.graph.unite_nodes(junction_id, start_node);
                continue;
            }
            if factor > (geo_len - 1) as f32 - 0.01 {
                let end_node = network.graph.edges[eid].end_node;
                network.graph.unite_nodes(junction_id, end_node);
                continue;
            }

            split_edge(network, eid, seg_idx, sub_t, junction_id);
        }
    }
}

pub fn split_edge(network: &mut TransitNetwork, edge_id: usize, segment_idx: usize, _t: f32, junction_node_id: u32) {
    let old_edge = &network.graph.edges[edge_id];
    let end_node = old_edge.end_node;
    let split_pos = network.graph.nodes[junction_node_id as usize].pos;

    let mut part2_geo = vec![split_pos];
    part2_geo.extend_from_slice(&old_edge.geometry[segment_idx+1..]);
    
    let mut part1_geo = old_edge.geometry[..=segment_idx].to_vec();
    part1_geo.push(split_pos);
    
    let primary_type = old_edge.primary_type;
    let allowed_types = old_edge.allowed_types;
    let width = old_edge.width;
    let fwd_lanes = old_edge.fwd_lanes;
    let bkw_lanes = old_edge.bkw_lanes;
    let speed_limit = old_edge.speed_limit;
    let current_congestion = old_edge.current_congestion;

    network.graph.edges[edge_id].end_node = junction_node_id;
    network.graph.edges[edge_id].geometry = part1_geo;
    network.graph.edges[edge_id].base_cost = crate::simulation::pathing::cost::CostCalculator::calculate_base_cost(&network.graph.edges[edge_id]);

    let mut new_edge = Edge {
        start_node: junction_node_id,
        end_node,
        primary_type,
        allowed_types,
        width,
        fwd_lanes,
        bkw_lanes,
        speed_limit,
        base_cost: 0.0,
        current_congestion,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: part2_geo.clone(),
        physical_geometry: part2_geo,
    };
    new_edge.base_cost = crate::simulation::pathing::cost::CostCalculator::calculate_base_cost(&new_edge);
    network.graph.add_edge(new_edge);
}
