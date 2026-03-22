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

    pub fn split_edge_grid(&mut self, old_idx: usize, new_idx: usize, split_x: usize) {
        if let Some(old_grid) = self.edge_grids.get(&old_idx).cloned() {
            let cells_long = old_grid.cells_long;
            let part2_cells = cells_long.saturating_sub(split_x);
            
            let mut new_grid = EdgeZoning {
                left_side: vec![ZoneType::None; part2_cells * 4],
                right_side: vec![ZoneType::None; part2_cells * 4],
                left_occupied: vec![false; part2_cells * 4],
                right_occupied: vec![false; part2_cells * 4],
                cells_long: part2_cells,
            };

            // Copy data to new grid
            for x in 0..part2_cells {
                for y in 0..4 {
                    let old_x = split_x + x;
                    if old_x < cells_long {
                        let old_i = old_x * 4 + y;
                        let new_i = x * 4 + y;
                        new_grid.left_side[new_i] = old_grid.left_side[old_i];
                        new_grid.right_side[new_i] = old_grid.right_side[old_i];
                        new_grid.left_occupied[new_i] = old_grid.left_occupied[old_i];
                        new_grid.right_occupied[new_i] = old_grid.right_occupied[old_i];
                    }
                }
            }
            self.edge_grids.insert(new_idx, new_grid);

            // Truncate old grid
            if let Some(g) = self.edge_grids.get_mut(&old_idx) {
                g.left_side.truncate(split_x * 4);
                g.right_side.truncate(split_x * 4);
                g.left_occupied.truncate(split_x * 4);
                g.right_occupied.truncate(split_x * 4);
                g.cells_long = split_x;
            }
        }
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
            let mut closest_competitor_d_sq = f32::MAX;
            let mut asphalt_collision = false;

            let mut edges_to_check = std::collections::HashSet::new();
            
            // Spatial AABB Filter
            let pt_min = Vector2::new(pt.x - 20.0, pt.y - 20.0);
            let pt_max = Vector2::new(pt.x + 20.0, pt.y + 20.0);

            for (i, e) in graph.edges.iter().enumerate() {
                if i == edge_idx { continue; }
                if e.deleted { continue; }
                
                let mut e_min = Vector2::new(f32::MAX, f32::MAX);
                let mut e_max = Vector2::new(f32::MIN, f32::MIN);
                for p in &e.physical_geometry {
                    e_min.x = e_min.x.min(p.x); e_min.y = e_min.y.min(p.z);
                    e_max.x = e_max.x.max(p.x); e_max.y = e_max.y.max(p.z);
                }
                
                if pt_min.x < e_max.x && pt_max.x > e_min.x &&
                   pt_min.y < e_max.y && pt_max.y > e_min.y {
                    edges_to_check.insert(i);
                }
            }
            
            for &i in &edges_to_check {
                let other_edge = &graph.edges[i];
                let pts = &other_edge.physical_geometry;
                if pts.len() < 2 { continue; }
                
                for j in 0..pts.len() - 1 {
                    let p1 = pts[j]; let p2 = pts[j+1];
                    let p1_2d = Vector2::new(p1.x, p1.z);
                    let p2_2d = Vector2::new(p2.x, p2.z);
                    let l2 = (p2_2d - p1_2d).length_squared();
                    if l2 == 0.0 { continue; }

                    let seg_vec = p2_2d - p1_2d;
                    let t_val = (((pt.x - p1_2d.x) * seg_vec.x + (pt.y - p1_2d.y) * seg_vec.y) / l2).clamp(0.0, 1.0);
                    let proj = p1_2d + seg_vec * t_val;
                    let d_sq = (pt - proj).length_squared();
                    
                    // A. Asphalt Collision: Absolute block
                    if d_sq < (other_edge.width * 0.5 + 0.1).powi(2) {
                        asphalt_collision = true;
                        break;
                    }

                    // B. Zoning Claim: Competitor for space
                    let tangent = seg_vec.normalized();
                    let rel_pt = pt - proj;
                    let is_left = (tangent.x * rel_pt.y - tangent.y * rel_pt.x) < 0.0;
                    let other_is_claiming = if is_left { other_edge.zoning_left } else { other_edge.zoning_right };

                    if other_is_claiming {
                        if d_sq < closest_competitor_d_sq {
                            closest_competitor_d_sq = d_sq;
                        }
                    }
                }
                if asphalt_collision { break; }
            }

            if asphalt_collision { return true; }

            // 2. Voronoi Comparison: Is this point closer to its own centerline than any competitor?
            let self_d_sq = self.get_distance_to_edge_sq(edge_idx, pt, graph);
            
            // Priority bias for first few rows (within 12m)
            let mut bias = 1.0;
            if self_d_sq < (12.0f32).powi(2) {
                bias = 2.0; // Effectively make owner 2x stronger
            }
            
            if self_d_sq > (closest_competitor_d_sq * bias) + 0.1 { 
                return true; 
            }
        }

        false
    }

    fn get_distance_to_edge_sq(&self, edge_idx: usize, pt: Vector2, graph: &crate::simulation::network::graph::TransitGraph) -> f32 {
        let edge = &graph.edges[edge_idx];
        let mut min_d_sq = f32::MAX;
        for j in 0..edge.physical_geometry.len() - 1 {
            let p1 = edge.physical_geometry[j]; let p2 = edge.physical_geometry[j+1];
            let p1_2d = Vector2::new(p1.x, p1.z);
            let p2_2d = Vector2::new(p2.x, p2.z);
            let l2 = (p2_2d - p1_2d).length_squared();
            if l2 == 0.0 { continue; }
            let seg_vec = p2_2d - p1_2d;
            let t = (((pt.x-p1_2d.x)*seg_vec.x + (pt.y-p1_2d.y)*seg_vec.y)/l2).clamp(0.0, 1.0);
            let d_sq = (pt - (p1_2d + seg_vec * t)).length_squared();
            if d_sq < min_d_sq { min_d_sq = d_sq; }
        }
        min_d_sq
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
            if edge_idx >= graph.edges.len() || graph.edges[edge_idx].deleted { continue; }
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
