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
        let mut check_pts_with_dist = Vec::new();
        
        for dx in [-0.5, 0.5] {
            for dy in [-0.5, 0.5] {
                let local_x = (x as f32 + 0.5 + dx) * size;
                let local_y = (y as f32 + 0.5 + dy) * size;
                
                let t = (local_x / edge.physical_length).clamp(0.0, 1.0);
                let (pos_on_edge, tangent) = self.get_edge_pos_and_tangent_static(edge_idx, t, graph);
                let normal = Vector2::new(-tangent.y, tangent.x) * (side as f32);
                let intended_d = hw + local_y;
                check_pts_with_dist.push((pos_on_edge + normal * intended_d, intended_d));
            }
        }
        
        // Also check the center
        let center_intended = hw + ((y as f32 + 0.5) * size);
        check_pts_with_dist.push((center, center_intended));

        for (pt, intended_d) in check_pts_with_dist {
            let mut closest_d_sq = f32::MAX;

            // PERFORMANCE OPTIMIZATION: 
            // In a huge city, scanning every road segment is O(N).
            // For now, we search all segments of our own edge and its immediate neighbors (Nodes).
            // A truly global search should eventually use a Spatial Hash.
            let mut edges_to_check = std::collections::HashSet::new();
            edges_to_check.insert(edge_idx);
            
            // Spatial AABB Filter: Check all roads whose bounding box is near our point.
            // Even with 10,000 roads, checking AABBs is extrêmement rapide.
            let pt_min = Vector2::new(pt.x - 20.0, pt.y - 20.0);
            let pt_max = Vector2::new(pt.x + 20.0, pt.y + 20.0);

            for (i, e) in graph.edges.iter().enumerate() {
                if i == edge_idx { continue; }
                
                // Fast AABB check (using physical geometry)
                let mut e_min = Vector2::new(f32::MAX, f32::MAX);
                let mut e_max = Vector2::new(f32::MIN, f32::MIN);
                for p in &e.physical_geometry {
                    e_min.x = e_min.x.min(p.x); e_min.y = e_min.y.min(p.z);
                    e_max.x = e_max.x.max(p.x); e_max.y = e_max.y.max(p.z);
                }
                
                // Expand by a small buffer
                if pt_min.x < e_max.x && pt_max.x > e_min.x &&
                   pt_min.y < e_max.y && pt_max.y > e_min.y {
                    edges_to_check.insert(i);
                }
            }
            
            // 1. Scan relevant road centerlines
            for &i in &edges_to_check {
                let other_edge = &graph.edges[i];
                let is_self = i == edge_idx;
                let pts = &other_edge.physical_geometry;
                if pts.len() < 2 { continue; }
                
                for j in 0..pts.len() - 1 {
                    let p1 = pts[j]; let p2 = pts[j+1];
                    let p1_2d = Vector2::new(p1.x, p1.z);
                    let p2_2d = Vector2::new(p2.x, p2.z);
                    let l2 = (p2_2d - p1_2d).length_squared();
                    if l2 == 0.0 { continue; }

                    let t_val = (((pt.x - p1_2d.x) * (p2_2d.x - p1_2d.x) + (pt.y - p1_2d.y) * (p2_2d.y - p1_2d.y)) / l2).clamp(0.0, 1.0);
                    let proj = p1_2d + (p2_2d - p1_2d) * t_val;
                    let mut d_sq = (pt - proj).length_squared();
                    
                    // Boost priority for self-ownership so we don't reject our own straight roads due to epsilon noise
                    if is_self { d_sq *= 0.99; }

                    if d_sq < closest_d_sq {
                        closest_d_sq = d_sq;
                    }
                }
            }

            // 2. Universal Distance Guard:
            // If ANY part of the road network centerline is CLOSER to the point than its own intended anchor point,
            // then it has overlapped a different territory (either another road or an inner-lap curve).
            // Margin of 0.5m allowed for curvature gaps.
            if closest_d_sq < (intended_d - 0.5).powi(2) {
                return true;
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
