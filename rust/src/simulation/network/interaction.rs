use godot::prelude::*;
use super::graph::TransitGraph;

pub fn get_closest_point(graph: &TransitGraph, world_pos: Vector3, max_dist: f32) -> Option<Vector3> {
    let mut closest_pos = None;
    let mut min_score = f32::MAX;

    // 1. Check nodes first (Higher priority/Sticky)
    let node_snap_dist = max_dist * 2.5; 
    for node in &graph.nodes {
        let d = node.pos.distance_to(world_pos);
        if d < node_snap_dist {
            let score = d * 0.4; // Nodes are 2.5x more "attractive" than segments
            if score < min_score {
                min_score = score;
                closest_pos = Some(node.pos);
            }
        }
    }

    // 2. Check edges (Considering width)
    for edge in &graph.edges {
        let half_width = edge.width * 0.5;
        let edge_snap_dist = f32::max(max_dist, half_width + 1.0);
        
        for i in 0..edge.geometry.len() - 1 {
            let p0 = edge.geometry[i];
            let p1 = edge.geometry[i+1];
            
            let pos = get_closest_point_on_segment(world_pos, p0, p1);
            let d = pos.distance_to(world_pos);
            
            if d < edge_snap_dist {
                let score = d; 
                if score < min_score {
                    min_score = score;
                    closest_pos = Some(pos);
                }
            }
        }
    }
    closest_pos
}

pub fn get_closest_node(graph: &TransitGraph, world_pos: Vector3, max_dist: f32) -> Option<u32> {
    let mut closest_node = None;
    let mut min_dist_sq = max_dist * max_dist;

    for (i, node) in graph.nodes.iter().enumerate() {
        let d_sq = node.pos.distance_squared_to(world_pos);
        if d_sq < min_dist_sq {
            min_dist_sq = d_sq;
            closest_node = Some(i as u32);
        }
    }
    closest_node
}

pub fn get_closest_point_on_segment(p: Vector3, a: Vector3, b: Vector3) -> Vector3 {
    let ab = b - a;
    let t = (p - a).dot(ab) / ab.length_squared();
    if t <= 0.0 { return a; }
    if t >= 1.0 { return b; }
    a + ab * t
}

/// Finds the intersection point of two 2D segments (XZ plane)
/// Returns (t_a, t_b) if they intersect, where t is the factor along the segment [0, 1]
pub fn find_intersection_2d(p1: Vector3, p2: Vector3, p3: Vector3, p4: Vector3) -> Option<(f32, f32)> {
    let x1 = p1.x; let z1 = p1.z;
    let x2 = p2.x; let z2 = p2.z;
    let x3 = p3.x; let z3 = p3.z;
    let x4 = p4.x; let z4 = p4.z;

    let denom = (x1 - x2) * (z3 - z4) - (z1 - z2) * (x3 - x4);
    if denom.abs() < 0.0001 { return None; }

    let t = ((x1 - x3) * (z3 - z4) - (z1 - z3) * (x3 - x4)) / denom;
    let u = ((x1 - x2) * (z1 - z3) - (z1 - z2) * (x1 - x3)) / denom;

    if t >= 0.0 && t <= 1.0 && u >= 0.0 && u <= 1.0 {
        Some((t, u))
    } else {
        None
    }
}
