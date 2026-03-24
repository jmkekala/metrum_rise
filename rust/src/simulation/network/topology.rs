use std::collections::HashMap;
use crate::config;
use godot::prelude::*;
use super::TransitNetwork;
use super::graph::Edge;
use super::types::NodeType;
use super::interaction;

pub fn process_intersections(network: &mut TransitNetwork, edge_id: usize, zoning: &mut crate::simulation::grid::zoning::ZoningSystem, allocator: &mut crate::simulation::buildings::allocator::BuildingAllocator) {
    let mut all_splits: HashMap<usize, Vec<(f32, u32)>> = HashMap::new();
    
    // 1. Find all intersections (crossing AND touching/snapping)
    let edge_count = network.graph.edges.len();
    for other_id in 0..edge_count {
        if network.graph.edges[other_id].deleted { continue; }
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
                if edge_id == other_id && (i as i32 - j as i32).abs() < 5 { continue; }

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
                
                let p2d = Vector2::new(p.x, p.z);
                let closest2d = Vector2::new(closest.x, closest.z);
                let dist = p2d.distance_to(closest2d);

                if dist < config::INTERSECTION_TOLERANCE { 
                    let factor_u = j as f32 + (closest2d - Vector2::new(p1.x, p1.z)).length() / Vector2::new(p2.x - p1.x, p2.z - p1.z).length().max(0.001);
                    
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
        // Only dedup if it's the EXACT same junction ID to avoid skipping nearby splits
        splits.dedup_by(|a, b| a.1 == b.1);

        for (factor, junction_id) in splits {
            let seg_idx = factor.floor() as usize;
            let sub_t = factor.fract();
            
            let geo_len = network.graph.edges[eid].geometry.len();
            if factor < 0.1 {
                let start_node = network.graph.edges[eid].start_node;
                network.graph.unite_nodes(junction_id, start_node);
                continue;
            }
            if factor > (geo_len - 1) as f32 - 0.1 {
                let end_node = network.graph.edges[eid].end_node;
                network.graph.unite_nodes(junction_id, end_node);
                continue;
            }
            let valid_junction_id = network.graph.get_valid_node(junction_id);
            split_edge(network, eid, seg_idx, sub_t, valid_junction_id, zoning, allocator);
        }
    }
}

pub fn split_edge(network: &mut TransitNetwork, edge_id: usize, segment_idx: usize, _t: f32, junction_node_id: u32, zoning: &mut crate::simulation::grid::zoning::ZoningSystem, allocator: &mut crate::simulation::buildings::allocator::BuildingAllocator) {
    let old_edge = &network.graph.edges[edge_id];
    let geometry = &old_edge.geometry;
    let _length = old_edge.physical_length;
    let split_pos = network.graph.nodes[junction_node_id as usize].pos;

    // Physical distance guard: Don't split if too close to either end (e.g. < 0.2m)
    let start_pos = geometry[0];
    let end_pos = *geometry.last().unwrap();
    if split_pos.distance_to(start_pos) < 0.2 || split_pos.distance_to(end_pos) < 0.2 {
        return;
    }

    let end_node = old_edge.end_node;

    let mut part2_geo = vec![split_pos];
    part2_geo.extend_from_slice(&old_edge.geometry[segment_idx+1..]);
    
    let mut part1_geo = old_edge.geometry[..=segment_idx].to_vec();
    if part1_geo.last().unwrap().distance_to(split_pos) > 0.001 {
        part1_geo.push(split_pos);
    }
    
    let primary_type = old_edge.primary_type;
    let allowed_types = old_edge.allowed_types;
    let width = old_edge.width;
    let fwd_lanes = old_edge.fwd_lanes;
    let bkw_lanes = old_edge.bkw_lanes;
    let speed_limit = old_edge.speed_limit;
    let current_congestion = old_edge.current_congestion;
    let zoning_left = old_edge.zoning_left;
    let zoning_right = old_edge.zoning_right;

    let old_end_node = network.graph.edges[edge_id].end_node;
    network.graph.edges[edge_id].end_node = junction_node_id;
    network.graph.edges[edge_id].geometry = part1_geo.clone();
    network.graph.edges[edge_id].physical_geometry = part1_geo;
    let (cost, length) = crate::simulation::pathing::cost::CostCalculator::calculate_costs(&network.graph.edges[edge_id]);
    network.graph.edges[edge_id].base_cost = cost;
    network.graph.edges[edge_id].physical_length = length;

    // RE-INDEX and UPDATE ADJACENCY for modified edge
    network.graph.remove_from_spatial_index(edge_id);
    network.graph.add_to_spatial_index(edge_id);
    
    if let Some(adj) = network.graph.adjacency.get_mut(&old_end_node) {
        adj.retain(|&i| i != edge_id);
    }
    network.graph.adjacency.entry(junction_node_id).or_default().push(edge_id);

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
        physical_length: 0.0,
        current_congestion,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: part2_geo.clone(),
        physical_geometry: part2_geo.clone(),
        parking_occupied: 0,
        zoning_left,
        zoning_right,
        deleted: false,
    };
    let (cost_new, length_new) = crate::simulation::pathing::cost::CostCalculator::calculate_costs(&new_edge);
    new_edge.base_cost = cost_new;
    new_edge.physical_length = length_new;
    
    let new_edge_id = network.graph.add_edge(new_edge);

    // --- MIGRATION LOGIC ---
    let cell_size = zoning.grid_cell_size;
    let split_x = (length / cell_size).floor() as usize;

    // 1. Migrate Zoning
    zoning.split_edge_grid(edge_id, new_edge_id, split_x);

    // 2. Migrate Buildings
    for b in &mut allocator.buildings {
        if b.edge_idx == edge_id && b.cell_x >= split_x {
            b.edge_idx = new_edge_id;
            b.cell_x -= split_x;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::grid::zoning::ZoningSystem;
    use crate::simulation::buildings::allocator::BuildingAllocator;
    use crate::simulation::network::types::TransitType;
    use godot::prelude::Vector3;

    #[test]
    fn test_topology_t_junction() {
        let mut net = TransitNetwork::new();
        let mut zoning = ZoningSystem::new();
        let mut allocator = BuildingAllocator::new(100, 100);
        // straight road
        net.add_road(vec![
            Vector3::new(-10.0, 0.0, 0.0),
            Vector3::new(10.0, 0.0, 0.0),
        ].into(), 1, 1, false, false, &mut zoning, &mut allocator);
        
        // side road connecting to the middle
        net.add_road(vec![
            Vector3::new(0.0, 0.0, 10.0),
            Vector3::new(0.0, 0.0, 0.0),
        ].into(), 1, 1, false, false, &mut zoning, &mut allocator);
        
        println!("Graph has {} edges", net.graph.edges.len());
        for (i, edge) in net.graph.edges.iter().enumerate() {
            println!("Edge {}: start node {}, end node {}, geometry len {}", i, edge.start_node, edge.end_node, edge.geometry.len());
        }
    }
}
