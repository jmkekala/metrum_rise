use godot::prelude::*;
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum ZoneType {
    None = 0,
    Residential = 1,
    Commercial = 2,
    Industrial = 3,
    Office = 4,
    Mixed = 5,
}

#[derive(Clone, Debug, Default)]
pub struct EdgeZoning {
    pub left_side: Vec<ZoneType>,  // 4 rows deep, N columns long
    pub right_side: Vec<ZoneType>, // 4 rows deep, N columns long
    pub left_occupied: Vec<bool>,
    pub right_occupied: Vec<bool>,
    pub cells_long: usize,
}

#[derive(Clone)]
pub struct ZoningSystem {
    pub edge_grids: HashMap<usize, EdgeZoning>,
    pub grid_cell_size: f32, // 8.0f32
}

impl ZoningSystem {
    pub fn new() -> Self {
        Self {
            edge_grids: HashMap::new(),
            grid_cell_size: 8.0,
        }
    }

    pub fn clear(&mut self) {
        self.edge_grids.clear();
    }

    pub fn update_edge_grid_size(&mut self, edge_idx: usize, length: f32) {
        let cells_long = (length / self.grid_cell_size).floor() as usize;
        let entry = self.edge_grids.entry(edge_idx).or_insert_with(|| EdgeZoning {
            left_side: vec![ZoneType::None; cells_long * 4],
            right_side: vec![ZoneType::None; cells_long * 4],
            left_occupied: vec![false; cells_long * 4],
            right_occupied: vec![false; cells_long * 4],
            cells_long,
        });

        if entry.cells_long != cells_long {
            entry.left_side.resize(cells_long * 4, ZoneType::None);
            entry.right_side.resize(cells_long * 4, ZoneType::None);
            entry.left_occupied.resize(cells_long * 4, false);
            entry.right_occupied.resize(cells_long * 4, false);
            entry.cells_long = cells_long;
        }
    }

    pub fn set_cell(&mut self, edge_idx: usize, side: i8, x: usize, y: usize, zone_type: ZoneType) {
        if let Some(grid) = self.edge_grids.get_mut(&edge_idx) {
            if x < grid.cells_long && y < 4 {
                let idx = x * 4 + y;
                if side > 0 {
                    grid.left_side[idx] = zone_type;
                } else {
                    grid.right_side[idx] = zone_type;
                }
            }
        }
    }

    pub fn get_cell(&self, edge_idx: usize, side: i8, x: usize, y: usize) -> ZoneType {
        if let Some(grid) = self.edge_grids.get(&edge_idx) {
            if x < grid.cells_long && y < 4 {
                let idx = x * 4 + y;
                if side > 0 {
                    return grid.left_side[idx];
                } else {
                    return grid.right_side[idx];
                }
            }
        }
        ZoneType::None
    }

    pub fn is_occupied(&self, edge_idx: usize, side: i8, x: usize, y: usize) -> bool {
        if let Some(grid) = self.edge_grids.get(&edge_idx) {
            if x < grid.cells_long && y < 4 {
                let idx = x * 4 + y;
                if side > 0 {
                    return grid.left_occupied[idx];
                } else {
                    return grid.right_occupied[idx];
                }
            }
        }
        true // Assume occupied if out of bounds or grid missing
    }

    pub fn set_occupied(&mut self, edge_idx: usize, side: i8, x: usize, y: usize, occupied: bool) {
        if let Some(grid) = self.edge_grids.get_mut(&edge_idx) {
            if x < grid.cells_long && y < 4 {
                let idx = x * 4 + y;
                if side > 0 {
                    grid.left_occupied[idx] = occupied;
                } else {
                    grid.right_occupied[idx] = occupied;
                }
            }
        }
    }

    pub fn get_cell_center(&self, edge_idx: usize, side: i8, x: usize, y: usize, graph: &crate::simulation::network::graph::TransitGraph) -> Vector2 {
        if edge_idx >= graph.edges.len() { return Vector2::new(0.0, 0.0); }
        let edge = &graph.edges[edge_idx];
        let geom = &edge.physical_geometry;
        if geom.len() < 2 { return Vector2::new(0.0, 0.0); }

        let total_l = edge.physical_length;
        if total_l < 0.1 { return Vector2::new(0.0, 0.0); }

        let t = (x as f32 + 0.5) * self.grid_cell_size / total_l;
        if t > 1.0 { return Vector2::new(0.0, 0.0); }

        // Find position and tangent at T
        let mut curr_l = 0.0;
        let mut pos = Vector2::new(0.0, 0.0);
        let mut tangent = Vector2::new(1.0, 0.0);
        let target_l = t * total_l;

        for i in 0..geom.len() - 1 {
            let p1 = Vector2::new(geom[i].x, geom[i].z);
            let p2 = Vector2::new(geom[i+1].x, geom[i+1].z);
            let d = (p2 - p1).length();
            if curr_l + d >= target_l {
                let local_t = (target_l - curr_l) / d;
                pos = p1 + (p2 - p1) * local_t;
                tangent = (p2 - p1).normalized();
                break;
            }
            curr_l += d;
            if i == geom.len() - 2 {
                pos = p2;
                tangent = (p2 - p1).normalized();
            }
        }

        let normal = Vector2::new(-tangent.y, tangent.x) * (side as f32);
        let depth = (y as f32 + 0.5) * self.grid_cell_size;
        let half_width = graph.edges[edge_idx].width * 0.5;
        
        pos + normal * (half_width + depth)
    }

    pub fn is_cell_obstructed(&self, edge_idx: usize, side: i8, x: usize, y: usize, graph: &crate::simulation::network::graph::TransitGraph) -> bool {
        let center = self.get_cell_center(edge_idx, side, x, y, graph);
        if center.x == 0.0 && center.y == 0.0 { return true; }

        let size = self.grid_cell_size;
        let edge = &graph.edges[edge_idx];
        let hw = edge.width * 0.5;

        // We check 5 points: 4 corners + center
        // All points must "belong" to our edge and not be on any other road.
        let mut check_points = Vec::new();
        check_points.push(center);
        
        for dx in [-0.5, 0.5] {
            for dy in [-0.5, 0.5] {
                let local_x = (x as f32 + 0.5 + dx) * size;
                let local_y = (y as f32 + 0.5 + dy) * size;
                
                let t = (local_x / edge.physical_length).clamp(0.0, 1.0);
                let (pos_on_edge, tangent) = self.get_edge_pos_and_tangent_static(edge_idx, t, graph);
                let normal = Vector2::new(-tangent.y, tangent.x) * (side as f32);
                check_points.push(pos_on_edge + normal * (hw + local_y));
            }
        }

        for pt in check_points {
            // 1. Check "Closest Edge Ownership" for this point
            let mut min_dist_sq = f32::MAX;
            let mut closest_edge = -1;

            for (i, other_edge) in graph.edges.iter().enumerate() {
                if i == edge_idx { continue; } // Self check handled differently

                // SKIP checks for "smoothly connected" edges (collinear segments)
                // This allows the grids to touch and surfaces to overlap perfectly at the nodes.
                if other_edge.start_node == edge.start_node || other_edge.start_node == edge.end_node ||
                   other_edge.end_node == edge.start_node || other_edge.end_node == edge.end_node {
                    
                    let v1 = (graph.nodes[edge.end_node as usize].pos - graph.nodes[edge.start_node as usize].pos).normalized();
                    let v2 = (graph.nodes[other_edge.end_node as usize].pos - graph.nodes[other_edge.start_node as usize].pos).normalized();
                    if v1.dot(v2).abs() > 0.98 {
                        continue; // Same "logical" road path
                    }
                }

                let pts = &other_edge.physical_geometry;
                if pts.len() < 2 { continue; }
                for j in 0..pts.len() - 1 {
                    let p1 = pts[j]; let p2 = pts[j+1];
                    let p1_2d = Vector2::new(p1.x, p1.z);
                    let p2_2d = Vector2::new(p2.x, p2.z);
                    
                    let l2 = (p2_2d - p1_2d).length_squared();
                    let mut dist_sq = if l2 == 0.0 {
                        (pt - p1_2d).length_squared()
                    } else {
                        let mut t_val = ((pt.x - p1_2d.x) * (p2_2d.x - p1_2d.x) + (pt.y - p1_2d.y) * (p2_2d.y - p1_2d.y)) / l2;
                        t_val = t_val.clamp(0.0, 1.0);
                        let proj = p1_2d + (p2_2d - p1_2d) * t_val;
                        (pt - proj).length_squared()
                    };

                    // Block if the point is on another road's surface (ONLY for non-collinear roads)
                    if dist_sq < (other_edge.width * 0.5 + 0.1).powi(2) {
                        return true;
                    }

                    if dist_sq < min_dist_sq {
                        min_dist_sq = dist_sq;
                        closest_edge = i as i32;
                    }
                }
            }
            
            // Re-check our own edge for min_dist with priority
            let pts = &edge.physical_geometry;
            for j in 0..pts.len() - 1 {
                let p1 = pts[j]; let p2 = pts[j+1];
                let p1_2d = Vector2::new(p1.x, p1.z);
                let p2_2d = Vector2::new(p2.x, p2.z);
                let l2 = (p2_2d - p1_2d).length_squared();
                let mut dist_sq = if l2 == 0.0 { (pt - p1_2d).length_squared() } else {
                    let mut t_val = ((pt.x - p1_2d.x) * (p2_2d.x - p1_2d.x) + (pt.y - p1_2d.y) * (p2_2d.y - p1_2d.y)) / l2;
                    t_val = t_val.clamp(0.0, 1.0);
                    let proj = p1_2d + (p2_2d - p1_2d) * t_val;
                    (pt - proj).length_squared()
                };
                dist_sq *= 0.99; // Priority boost relative to others
                if dist_sq < min_dist_sq {
                    min_dist_sq = dist_sq;
                    closest_edge = edge_idx as i32;
                }
            }

            if closest_edge != -1 && (closest_edge as usize) != edge_idx {
                return true; // Another road owns this corner of the cell
            }
        }

        false
    }

    fn get_edge_pos_and_tangent_static(&self, edge_idx: usize, t: f32, graph: &crate::simulation::network::graph::TransitGraph) -> (Vector2, Vector2) {
        let edge = &graph.edges[edge_idx];
        let geom = &edge.physical_geometry;
        let total_l = edge.physical_length;
        let target_l = t * total_l;
        
        let mut curr_l = 0.0;
        for i in 0..geom.len() - 1 {
            let p1 = Vector2::new(geom[i].x, geom[i].z);
            let p2 = Vector2::new(geom[i+1].x, geom[i+1].z);
            let d = (p2 - p1).length();
            if curr_l + d >= target_l || i == geom.len() - 2 {
                let local_t = if d > 0.0 { ((target_l - curr_l) / d).clamp(0.0, 1.0) } else { 0.0 };
                return (p1 + (p2 - p1) * local_t, (p2 - p1).normalized());
            }
            curr_l += d;
        }
        (Vector2::new(0.0, 0.0), Vector2::new(1.0, 0.0))
    }

    // Helper for rendering all painted cells
    pub fn get_render_data(&self, graph: &crate::simulation::network::graph::TransitGraph) -> PackedFloat32Array {
        let mut data = Vec::new();
        for (&edge_idx, grid) in &self.edge_grids {
            if edge_idx >= graph.edges.len() { continue; }
            for side in [1, -1] {
                let cells = if side > 0 { &grid.left_side } else { &grid.right_side };
                for x in 0..grid.cells_long {
                    for y in 0..4 {
                        let z_type = cells[x * 4 + y];
                        if z_type == ZoneType::None { continue; }
                        if self.is_cell_obstructed(edge_idx, side, x, y, graph) { continue; }
                        
                        // Calculate world position of this cell
                        // This is a simplified version; real implementation needs tangent/normal integration
                        // but let's just pass the raw data for now and let the visual side handle some positioning?
                        // Actually, better to pass cell centers.
                        
                        data.push(edge_idx as f32);
                        data.push(side as f32);
                        data.push(x as f32);
                        data.push(y as f32);
                        data.push(z_type as u8 as f32);
                    }
                }
            }
        }
        PackedFloat32Array::from_iter(data)
    }
}
