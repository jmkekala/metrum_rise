use godot::prelude::*;
use godot::classes::{Node3D, INode3D, MultiMesh};

use crate::simulation::terrain::TerrainSystem;
use crate::simulation::water::WaterSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::interaction;
use crate::simulation::grid::zoning::{ZoningSystem, ZoneType};
use crate::simulation::grid::pollution::PollutionSystem;
use crate::simulation::grid::noise::NoiseSystem;
use crate::simulation::grid::desirability::DesirabilitySystem;
use crate::simulation::core::time::TimeSystem;
use crate::simulation::economy::demand::DemandSystem;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::AgentSystem;
use crate::config::{self, ZONING_DEPTH, GRID_CELL_SIZE};
use rayon::prelude::*;

pub struct SimulationSnapshot {
    pub terrain: Option<Vec<f32>>,
    pub water: Option<Vec<f32>>,
    pub transit: Option<crate::simulation::network::graph::TransitGraph>,
    pub zoning: Option<crate::simulation::grid::zoning::ZoningSystem>,
}

#[derive(GodotClass)]
#[class(base=Node3D)]
pub struct SimulationNode {
    time: TimeSystem,
    time_passed: f64,
    heightmap: TerrainSystem,
    watermap: WaterSystem,
    transit_network: TransitNetwork,
    zoning: ZoningSystem,
    pollution: PollutionSystem,
    noise: NoiseSystem,
    desirability: DesirabilitySystem,
    demand: DemandSystem,
    allocator: BuildingAllocator,
    agents: AgentSystem,
    undo_stack: Vec<SimulationSnapshot>,
    last_tick_duration: f64,
    benchmark_mode: bool,
    terrain_dirty: bool,
    water_dirty: bool,
    base: Base<Node3D>,
}

#[godot_api]
impl SimulationNode {
// ...
    // ...

    // Helper inside godot API block
    fn grid_to_image_data(grid: &crate::simulation::grid::data_grid::DataGrid<f32>, r: u8, g: u8, b: u8, max_val: f32) -> PackedByteArray {
        let mut pixels = Vec::with_capacity(grid.width * grid.height * 4);
        for y in 0..grid.height {
            for x in 0..grid.width {
                let val = *grid.get(x, y).unwrap_or(&0.0);
                if val <= 0.01 {
                    pixels.push(0); pixels.push(0); pixels.push(0); pixels.push(0);
                } else {
                    let alpha = ((val / max_val).clamp(0.0, 1.0) * 200.0) as u8;
                    pixels.push(r);
                    pixels.push(g);
                    pixels.push(b);
                    pixels.push(alpha);
                }
            }
        }
        PackedByteArray::from_iter(pixels)
    }

    #[func]
    pub fn get_pollution_image_data(&self) -> PackedByteArray {
        Self::grid_to_image_data(&self.pollution.grid, 255, 50, 50, 100.0) // Red
    }

    #[func]
    pub fn get_noise_image_data(&self) -> PackedByteArray {
        Self::grid_to_image_data(&self.noise.grid, 200, 200, 200, 100.0) // White/Grey
    }

    #[func]
    pub fn get_desirability_image_data(&self) -> PackedByteArray {
        Self::grid_to_image_data(&self.desirability.grid, 50, 255, 50, 100.0) // Bright Green
    }

    fn push_undo_state(&mut self, inc_terrain: bool, inc_water: bool, inc_transit: bool, inc_zoning: bool) {
        if self.undo_stack.len() >= 30 {
            self.undo_stack.remove(0); // Constant 30-size rolling window
        }
        self.undo_stack.push(SimulationSnapshot {
            terrain: if inc_terrain { Some(self.heightmap.data.clone()) } else { None },
            water: if inc_water { Some(self.watermap.depth.clone()) } else { None },
            transit: if inc_transit { Some(self.transit_network.graph.clone()) } else { None },
            zoning: if inc_zoning { Some(self.zoning.clone()) } else { None },
        });
    }

    #[func]
    pub fn undo_action(&mut self) -> bool {
        if let Some(state) = self.undo_stack.pop() {
            let mut sync_transit = false;

            if let Some(t_data) = state.terrain {
                self.heightmap.data = t_data;
                sync_transit = true; 
            }
            if let Some(w_data) = state.water {
                self.watermap.depth = w_data;
            }
            if let Some(tr_graph) = state.transit {
                self.transit_network.graph = tr_graph;
                sync_transit = true;
            }
            if let Some(z_sys) = state.zoning {
                self.zoning = z_sys;
            }

            if sync_transit {
                self.transit_network.hpa_graph = crate::simulation::pathing::hpa::HpaGraph::build(&self.transit_network.graph);
            }
            return true;
        }
        false
    }

    #[func]
    pub fn sculpt_terrain(&mut self, pos: Vector2, radius: f32, strength: f32) {
        self.push_undo_state(true, false, true, false); // Sculpt triggers transit geometry re-flow
        self.heightmap.sculpt(pos.x, pos.y, radius, strength);
        self.terrain_dirty = true;
        
        // STICKY ROADS: Sync network and re-flatten
        self.transit_network.sync_to_terrain(&self.heightmap);
        self.flatten_terrain_for_roads();
    }

    #[func]
    pub fn add_water(&mut self, pos: Vector2, amount: f32) {
        self.push_undo_state(false, true, false, false);
        self.watermap.add_water(pos.x as usize, pos.y as usize, amount);
    }

    #[func]
    pub fn add_water_source(&mut self, pos: Vector2, rate_add: f32) {
        self.watermap.update_source(pos.x as usize, pos.y as usize, rate_add);
        self.water_dirty = true;
    }

    #[func]
    pub fn is_terrain_dirty(&self) -> bool { self.terrain_dirty }
    
    #[func]
    pub fn is_water_dirty(&self) -> bool { self.water_dirty }

    #[func]
    pub fn clear_terrain_dirty(&mut self) { self.terrain_dirty = false; }
    
    #[func]
    pub fn clear_water_dirty(&mut self) { self.water_dirty = false; }

    #[func]
    pub fn get_heightmap_data(&self) -> PackedFloat32Array {
        PackedFloat32Array::from_iter(self.heightmap.data.iter().cloned())
    }

    #[func]
    pub fn get_water_data(&self) -> PackedFloat32Array {
        PackedFloat32Array::from_iter(self.watermap.depth.iter().cloned())
    }

    #[func]
    pub fn get_water_velocity_data(&self) -> PackedFloat32Array {
        PackedFloat32Array::from_iter(self.watermap.velocity.iter().cloned())
    }

    #[func]
    pub fn get_heightmap_size(&self) -> Vector2 {
        Vector2::new(self.heightmap.width as f32, self.heightmap.height as f32)
    }

    #[func]
    pub fn set_zoning_cell(&mut self, edge_idx: i32, side: i8, x: i32, y: i32, zone_type_int: u8) {
        self.push_undo_state(false, false, false, true);
        let zone_type = match zone_type_int {
            1 => crate::simulation::grid::zoning::ZoneType::Residential,
            2 => crate::simulation::grid::zoning::ZoneType::Commercial,
            3 => crate::simulation::grid::zoning::ZoneType::Industrial,
            4 => crate::simulation::grid::zoning::ZoneType::Office,
            5 => crate::simulation::grid::zoning::ZoneType::Mixed,
            _ => crate::simulation::grid::zoning::ZoneType::None,
        };
        self.zoning.set_cell(edge_idx as usize, side, x as usize, y as usize, zone_type);
        self.allocator.dirty = true;
    }

    #[func]
    pub fn get_zoning_grid_data(&self) -> PackedFloat32Array {
        self.zoning.get_render_data(&self.transit_network.graph)
    }

    #[func]
    pub fn get_edge_zoning_info(&self, edge_idx: i32) -> VarDictionary {
        let mut dict = VarDictionary::new();
        if let Some(grid) = self.zoning.edge_grids.get(&(edge_idx as usize)) {
            dict.set("cells_long", grid.cells_long as i32);
            dict.set("cell_size", self.zoning.grid_cell_size);
            dict.set("left_side", PackedByteArray::from_iter(grid.left_side.iter().map(|&z| z as u8)));
            dict.set("right_side", PackedByteArray::from_iter(grid.right_side.iter().map(|&z| z as u8)));
        }
        dict
    }

    #[func]
    pub fn is_zoning_cell_obstructed(&self, edge_idx: i32, side: i32, x: i32, y: i32) -> bool {
        let graph = &self.transit_network.graph;
        if let Some(edge) = graph.edges.get(edge_idx as usize) {
            if (side == 1 && !edge.zoning_left) || (side == -1 && !edge.zoning_right) {
                return true;
            }
        }
        self.zoning.is_cell_obstructed(edge_idx as usize, side as i8, x as usize, y as usize, graph)
    }

    #[func]
    pub fn set_zoning_enabled(&mut self, edge_idx: i32, side: i32, enabled: bool) {
        if let Some(edge) = self.transit_network.graph.edges.get_mut(edge_idx as usize) {
            if side >= 1 { edge.zoning_left = enabled; }
            else if side <= -1 { edge.zoning_right = enabled; }
        }
        // TRIGGER LOCAL RE-FLOW: Now that zoning claims changed, neighbors might want to move in!
        self.recalculate_zoning_local(edge_idx as usize);
    }

    #[func]
    pub fn get_zoning_cell_center(&self, edge_idx: i32, side: i8, x: i32, y: i32) -> godot::prelude::Vector2 {
        let v2 = self.zoning.get_cell_center(edge_idx as usize, side, x as usize, y as usize, &self.transit_network.graph);
        godot::prelude::Vector2::new(v2.x, v2.y)
    }

    #[func]
    pub fn update_zoning_visuals(&self, mut grid_mm: Gd<MultiMesh>, mut paint_mm: Gd<MultiMesh>, hovered_edge: i32, mode: i32, mouse_pos_3d: Vector3) {
        let graph = &self.transit_network.graph;
        let cell_size = self.zoning.grid_cell_size;
        let zoning_depth = config::ZONING_DEPTH;

        // 1. PRE-CALCULATE MOUSE CELL DATA (for brush highlighting)
        let mut m_edge = -1; let mut m_side = 0; let mut m_cx = -1; let mut m_cy = -1;
        if hovered_edge != -1 {
            if let Some(edge) = graph.edges.get(hovered_edge as usize) {
                let world_pos = godot::prelude::Vector2::new(mouse_pos_3d.x, mouse_pos_3d.z);
                let p = self.get_projection_data(edge, world_pos);
                m_edge = hovered_edge;
                m_side = p.side;
                m_cx = (p.t * edge.physical_length / cell_size).floor() as i32;
                m_cy = ((p.dist_from_road - edge.width * 0.5) / cell_size).floor() as i32;
            }
        }

        // 2. GENERATE GRID PREVIEW (The ghost cells)
        let mut grid_instances = Vec::new();
        for (edge_idx, edge) in graph.edges.iter().enumerate() {
            if edge.deleted || edge.physical_length < 0.1 { continue; }
            let cells_long = (edge.physical_length / cell_size).floor() as usize;

            for side in [1, -1] {
                if (side == 1 && !edge.zoning_left) || (side == -1 && !edge.zoning_right) { continue; }

                for x in 0..cells_long {
                    let t_param = (x as f32 + 0.5) * cell_size / edge.physical_length;
                    let (pos_on_edge, tangent) = self.get_edge_pos_and_tangent(edge_idx, t_param);
                    
                    // Standard Normalized Normal (Left-pointing for side=1 in Godot right-handed XZ)
                    let normal = godot::prelude::Vector2::new(tangent.y, -tangent.x) * (side as f32);

                    for y in 0..zoning_depth {
                        if self.zoning.is_blocked(edge_idx, side, x, y) { continue; }
                        
                        let depth = (y as f32 + 0.5) * cell_size;
                        let center_2d = pos_on_edge + normal * (edge.width * 0.5 + depth);
                        
                        // DIRECT BASIS CONSTRUCTION
                        // Align Box X with tangent, Box Z with normal
                        let b_xx = tangent.x; let b_xz = tangent.y; // X-axis on XZ plane
                        let b_zx = -tangent.y; let b_zz = tangent.x; // Z-axis on XZplane (orthogonal)

                        // PUSH TRANSFORM (12 floats, 4 per row) - Godot MultiMesh format
                        // Row 0: [X.x, Y.x, Z.x, O.x]
                        grid_instances.push(b_xx); grid_instances.push(0.0); grid_instances.push(b_zx); grid_instances.push(center_2d.x);
                        // Row 1: [X.y, Y.y, Z.y, O.y]
                        grid_instances.push(0.0); grid_instances.push(1.0); grid_instances.push(0.0); grid_instances.push(0.1);
                        // Row 2: [X.z, Y.z, Z.z, O.z]
                        grid_instances.push(b_xz); grid_instances.push(0.0); grid_instances.push(b_zz); grid_instances.push(center_2d.y);

                        // COLOR DATA (4 floats)
                        let is_hovered = edge_idx as i32 == m_edge && side == m_side;
                        let mut in_brush = false;
                        if is_hovered {
                            match mode {
                                0 => if x == m_cx as usize && y == m_cy as usize { in_brush = true; },
                                1 => if (x as i32 - m_cx).abs() < 2 && (y as i32 - m_cy).abs() < 2 { in_brush = true; },
                                2 | 3 => in_brush = true,
                                _ => {}
                            }
                        }
                        
                        let alpha = (if in_brush { 0.7 } else { 0.3 }) * self.get_depth_alpha(y as i32);
                        let r = if is_hovered { 1.0 } else { 0.1 };
                        let g = if is_hovered { 1.0 } else { 0.8 };
                        let b = if is_hovered { 1.0 } else { 0.1 };
                        grid_instances.push(r); grid_instances.push(g); grid_instances.push(b); grid_instances.push(alpha);
                    }
                }
            }
        }

        // 3. GENERATE PAINTED CELLS
        let mut paint_instances = Vec::new();
        for (&edge_idx, grid) in &self.zoning.edge_grids {
            if let Some(edge) = graph.edges.get(edge_idx) {
                if edge.deleted { continue; }
                
                for side_idx in 0..2 {
                    let side: i8 = if side_idx == 0 { 1 } else { -1 }; // 1=Left, -1=Right
                    let data = if side == 1 { &grid.left_side } else { &grid.right_side };
                    if (side == 1 && !edge.zoning_left) || (side == -1 && !edge.zoning_right) { continue; }

                    for (idx, &z_type) in data.iter().enumerate() {
                        if z_type == ZoneType::None { continue; }
                        let x = idx / ZONING_DEPTH; 
                        let y = idx % ZONING_DEPTH;
                        if self.zoning.is_blocked(edge_idx, side, x, y) { continue; }

                        let t_param = (x as f32 + 0.5) * cell_size / edge.physical_length;
                        let (pos_on_edge, tangent) = self.get_edge_pos_and_tangent(edge_idx, t_param);
                        let normal = godot::prelude::Vector2::new(tangent.y, -tangent.x) * (side as f32);
                        let depth = (y as f32 + 0.5) * cell_size;
                        let center_2d = pos_on_edge + normal * (edge.width * 0.5 + depth);
                        
                        let w = self.heightmap.width as f32;
                        let h = self.heightmap.height as f32;
                        let gx = (center_2d.x + (w - 1.0) * 0.5).round().clamp(0.0, w - 1.0) as usize;
                        let gz = (center_2d.y + (h - 1.0) * 0.5).round().clamp(0.0, h - 1.0) as usize;
                        let world_y = self.heightmap.get_height(gx, gz) * 20.0 + 0.4;

                        // PUSH TRANSFORM (12 floats, 4 per row)
                        let b_xx = tangent.x; let b_xz = tangent.y; 
                        let b_zx = -tangent.y; let b_zz = tangent.x;

                        paint_instances.push(b_xx); paint_instances.push(0.0); paint_instances.push(b_zx); paint_instances.push(center_2d.x);
                        paint_instances.push(0.0); paint_instances.push(1.0); paint_instances.push(0.0); paint_instances.push(world_y);
                        paint_instances.push(b_xz); paint_instances.push(0.0); paint_instances.push(b_zz); paint_instances.push(center_2d.y);

                        let color = self.get_zone_color_rust(z_type as u8);
                        paint_instances.push(color.r); paint_instances.push(color.g); paint_instances.push(color.b); paint_instances.push(0.6);
                    }
                }
            }
        }

        // Apply to Godot (The "Batch Burst")
        let grid_count = (grid_instances.len() / 16) as i32;
        let paint_count = (paint_instances.len() / 16) as i32;
        
        grid_mm.set_instance_count(grid_count);
        if grid_count > 0 {
            grid_mm.set_buffer(&PackedFloat32Array::from_iter(grid_instances));
        }

        paint_mm.set_instance_count(paint_count);
        if paint_count > 0 {
            paint_mm.set_buffer(&PackedFloat32Array::from_iter(paint_instances));
        }
    }

    fn get_projection_data(&self, edge: &crate::simulation::network::graph::Edge, p: godot::prelude::Vector2) -> interaction::ProjectionData {
        let mut best_dist_sq = 1e10;
        let mut best_t = 0.0;
        let mut best_side = 1;
        let mut best_dist_from_road = 0.0;
        
        let geom = &edge.physical_geometry;
        let total_l = edge.physical_length;
        let mut curr_l = 0.0;
        
        for i in 0..geom.len() - 1 {
            let a = godot::prelude::Vector2::new(geom[i].x, geom[i].z);
            let b = godot::prelude::Vector2::new(geom[i+1].x, geom[i+1].z);
            let seg = b - a;
            let l2 = seg.length_squared();
            if l2 < 0.001 { continue; }
            
            let mut t_val = ((p.x - a.x) * seg.x + (p.y - a.y) * seg.y) / l2;
            t_val = t_val.clamp(0.0, 1.0);
            let proj = a + seg * t_val;
            
            let dist_sq = p.distance_squared_to(proj);
            if dist_sq < best_dist_sq {
                best_dist_sq = dist_sq;
                best_t = (curr_l + t_val * f32::sqrt(l2)) / total_l;
                let tangent = seg.normalized();
                let normal = godot::prelude::Vector2::new(tangent.y, -tangent.x);
                let to_pt = p - proj;
                best_side = if to_pt.dot(normal) > 0.0 { 1 } else { -1 };
                best_dist_from_road = f32::sqrt(dist_sq);
            }
            curr_l += f32::sqrt(l2);
        }
        
        interaction::ProjectionData { t: best_t, side: best_side, dist_from_road: best_dist_from_road }
    }

    fn get_zone_color_rust(&self, z_type: u8) -> godot::prelude::Color {
        match z_type {
            1 => Color::from_rgb(0.0, 1.0, 0.0),
            2 => Color::from_rgb(0.0, 0.5, 1.0),
            3 => Color::from_rgb(1.0, 1.0, 0.0),
            4 => Color::from_rgb(0.1, 0.8, 0.8),
            5 => Color::from_rgb(0.8, 0.0, 0.8),
            _ => Color::from_rgb(1.0, 1.0, 1.0),
        }
    }

    fn get_depth_alpha(&self, y: i32) -> f32 {
        if y < 4 { return 1.0; }
        let zoning_depth = ZONING_DEPTH as i32;
        let t = (y - 4) as f32 / (zoning_depth - 1 - 4) as f32;
        1.0 + t * (0.1 - 1.0)
    }

    #[func]
    pub fn get_all_zoning_preview_data(&self) -> PackedFloat32Array {
        let mut data = Vec::new();
        let graph = &self.transit_network.graph;
        
        for (edge_idx, edge) in graph.edges.iter().enumerate() {
            if edge.deleted || edge.physical_length < 0.1 || edge.physical_geometry.len() < 2 { continue; }
            let cells_long = (edge.physical_length / self.zoning.grid_cell_size).floor() as usize;
            let width = edge.width;
            
            for side in [1, -1] {
                // SKIP if zoning is disabled for this side on this road
                if (side == 1 && !edge.zoning_left) || (side == -1 && !edge.zoning_right) {
                    continue;
                }
                for x in 0..cells_long {
                    // Pre-calculate tangent for the whole column to save time
                    let t_param = (x as f32 + 0.5) * self.zoning.grid_cell_size / edge.physical_length;
                    let (pos_on_edge, tangent) = self.get_edge_pos_and_tangent(edge_idx, t_param);
                    let normal = godot::prelude::Vector2::new(-tangent.y, tangent.x) * (side as f32);

                    for y in 0..ZONING_DEPTH {
                        if self.zoning.is_cell_obstructed(edge_idx, side, x, y, graph) {
                            continue;
                        }
                        
                        let depth = (y as f32 + 0.5) * self.zoning.grid_cell_size;
                        let center = pos_on_edge + normal * (width * 0.5 + depth);
                        
                        // Pass [pos.x, pos.z, tangent.x, tangent.y, edge_idx, side, cell_x, cell_y]
                        data.push(center.x);
                        data.push(center.y);
                        data.push(tangent.x);
                        data.push(tangent.y);
                        data.push(edge_idx as f32);
                        data.push(side as f32);
                        data.push(x as f32);
                        data.push(y as f32);
                    }
                }
            }
        }
        PackedFloat32Array::from_iter(data)
    }

    fn get_edge_pos_and_tangent(&self, edge_idx: usize, t: f32) -> (godot::prelude::Vector2, godot::prelude::Vector2) {
        let edge = &self.transit_network.graph.edges[edge_idx];
        let geom = &edge.physical_geometry;
        if geom.len() < 2 {
            return (godot::prelude::Vector2::new(0.0, 0.0), godot::prelude::Vector2::new(1.0, 0.0));
        }
        let total_l = edge.physical_length;
        let target_l = t * total_l;
        
        let mut curr_l = 0.0;
        for i in 0..geom.len() - 1 {
            let p1 = godot::prelude::Vector2::new(geom[i].x, geom[i].z);
            let p2 = godot::prelude::Vector2::new(geom[i+1].x, geom[i+1].z);
            let dist = p2 - p1;
            let d = dist.length();
            if curr_l + d >= target_l || i == geom.len() - 2 {
                let local_t = if d > 1e-6 { ((target_l - curr_l) / d).clamp(0.0, 1.0) } else { 0.0 };
                let tangent = if d > 1e-6 { dist.normalized() } else { godot::prelude::Vector2::new(1.0, 0.0) };
                return (p1 + dist * local_t, tangent);
            }
            curr_l += d;
        }
        (godot::prelude::Vector2::new(0.0, 0.0), godot::prelude::Vector2::new(1.0, 0.0))
    }

    #[func]
    pub fn get_obstacle_polygons_float_array(&self, _ignore_poly_id: i32, ignore_edge_idx: i32) -> PackedFloat32Array {
        let mut data = Vec::new();
        let mut count = 0.0;
        data.push(0.0); // Placeholder for count

        for (i, edge) in self.transit_network.graph.edges.iter().enumerate() {
            if edge.deleted || i as i32 == ignore_edge_idx { continue; }
            let hw = edge.width / 2.0;

            let mut poly = Vec::new();
            if edge.physical_geometry.is_empty() {
                let n1 = godot::prelude::Vector2::new(self.transit_network.graph.nodes[edge.start_node as usize].pos.x, self.transit_network.graph.nodes[edge.start_node as usize].pos.z);
                let n2 = godot::prelude::Vector2::new(self.transit_network.graph.nodes[edge.end_node as usize].pos.x, self.transit_network.graph.nodes[edge.end_node as usize].pos.z);
                let dir = (n2 - n1).normalized();
                if dir.length_squared() > 0.0 {
                    let norm = godot::prelude::Vector2::new(-dir.y, dir.x);
                    poly.push(n1 + norm * hw);
                    poly.push(n2 + norm * hw);
                    poly.push(n2 - norm * hw);
                    poly.push(n1 - norm * hw);
                }
            } else {
                let mut left = Vec::new();
                let mut right = Vec::new();
                let len = edge.physical_geometry.len();
                for j in 0..len {
                    let curr = godot::prelude::Vector2::new(edge.physical_geometry[j].x, edge.physical_geometry[j].z);
                    let tangent = if len < 2 {
                        godot::prelude::Vector2::new(1.0, 0.0)
                    } else if j == 0 {
                        (godot::prelude::Vector2::new(edge.physical_geometry[1].x, edge.physical_geometry[1].z) - curr).normalized()
                    } else if j == len - 1 {
                        (curr - godot::prelude::Vector2::new(edge.physical_geometry[j-1].x, edge.physical_geometry[j-1].z)).normalized()
                    } else {
                        let prev = godot::prelude::Vector2::new(edge.physical_geometry[j-1].x, edge.physical_geometry[j-1].z);
                        let next = godot::prelude::Vector2::new(edge.physical_geometry[j+1].x, edge.physical_geometry[j+1].z);
                        let d1 = (curr - prev).normalized();
                        let d2 = (next - curr).normalized();
                        let t = d1 + d2;
                        if t.length_squared() > 0.0 { t.normalized() } else { d2 }
                    };
                    let norm = godot::prelude::Vector2::new(-tangent.y, tangent.x);
                    left.push(curr + norm * hw);
                    right.push(curr - norm * hw);
                }
                for v in left { poly.push(v); }
                for v in right.into_iter().rev() { poly.push(v); }
            }

            if !poly.is_empty() {
                data.push(poly.len() as f32);
                for v in poly {
                    data.push(v.x);
                    data.push(v.y);
                }
                count += 1.0;
            }
        }

        // Grid-based obstacles? For now, let's just keep the nodes as obstacles.

        // 3. Include Road Nodes as obstacles to prevent zoning through intersections
        for node in &self.transit_network.graph.nodes {
            let r = 5.0; // Intersections are protected 5m radius
            data.push(8.0); // Simple octagon
            for j in 0..8 {
                let ang = (j as f32) * std::f32::consts::TAU / 8.0;
                data.push(node.pos.x + ang.cos() * r);
                data.push(node.pos.z + ang.sin() * r);
            }
            count += 1.0;
        }

        data[0] = count;
        PackedFloat32Array::from_iter(data)
    }

    #[func]
    pub fn get_hovered_edge(&self, world_x: f32, world_z: f32) -> i32 {
        let pos = godot::prelude::Vector3::new(world_x, 0.0, world_z);
        let mut best_dist = f32::MAX;
        let mut best_edge = -1;
        
        for (i, edge) in self.transit_network.graph.edges.iter().enumerate() {
            let pts = &edge.physical_geometry;
            if pts.len() < 2 { continue; }
            for j in 0..pts.len() - 1 {
                let p1 = pts[j];
                let p2 = pts[j+1];
                let p1_2d = godot::prelude::Vector2::new(p1.x, p1.z);
                let p2_2d = godot::prelude::Vector2::new(p2.x, p2.z);
                let mouse_2d = godot::prelude::Vector2::new(pos.x, pos.z);
                
                let l2 = (p2_2d - p1_2d).length_squared();
                let dist_sq;
                if l2 == 0.0 {
                    dist_sq = (mouse_2d - p1_2d).length_squared();
                } else {
                    let t = ((mouse_2d.x - p1_2d.x) * (p2_2d.x - p1_2d.x) + (mouse_2d.y - p1_2d.y) * (p2_2d.y - p1_2d.y)) / l2;
                    let t = t.clamp(0.0, 1.0);
                    let proj = godot::prelude::Vector2::new(p1_2d.x + t * (p2_2d.x - p1_2d.x), p1_2d.y + t * (p2_2d.y - p1_2d.y));
                    dist_sq = (mouse_2d - proj).length_squared();
                }
                
                if dist_sq < best_dist {
                    best_dist = dist_sq;
                    best_edge = i as i32;
                }
            }
        }
        
        // Return if mouse is near the road, or within the 110 meters bounding limit (matching grid depth + padding)
        if best_dist <= (110.0 * 110.0) { 
            best_edge
        } else {
            -1
        }
    }

    #[func]
    pub fn get_max_polygon_depth(&self, origin_x: f32, origin_z: f32, dir_x: f32, dir_z: f32, max_search: f32) -> f32 {
        let o = godot::prelude::Vector2::new(origin_x, origin_z);
        let d = godot::prelude::Vector2::new(dir_x, dir_z).normalized();
        
        let mut min_t = max_search;
        
        for edge in &self.transit_network.graph.edges {
            let pts = &edge.physical_geometry;
            if pts.len() < 2 { continue; }
            for i in 0..pts.len() - 1 {
                let p1 = godot::prelude::Vector2::new(pts[i].x, pts[i].z);
                let p2 = godot::prelude::Vector2::new(pts[i+1].x, pts[i+1].z);
                
                let v = p2 - p1;
                let det = d.x * v.y - d.y * v.x;
                if det.abs() > 0.001 {
                    let diff = p1 - o;
                    let t = (diff.x * v.y - diff.y * v.x) / det;
                    let u = (diff.x * d.y - diff.y * d.x) / det;
                    
                    if u >= 0.0 && u <= 1.0 && t > 0.1 && t < min_t {
                        min_t = t;
                    }
                }
            }
        }
        
        min_t
    }

    #[func]
    pub fn set_simulation_speed(&mut self, speed: f32) {
        self.time.speed_multiplier = speed.max(0.0);
    }

    #[func]
    pub fn get_current_day(&self) -> u32 {
        self.time.current_day
    }

    fn simulate_tick(&mut self) {
        godot_print!("Tick! Day {}", self.time.current_day);
        
        // 1. Environmental Spread (Buildings emit smog and noise)
        self.pollution.tick(&self.allocator);
        let tick_start = std::time::Instant::now();
        
        // ECONOMY: Demand update
        self.demand.tick();
        
        // ZONING: Growth & Immigration
        self.allocator.tick(&mut self.demand, &mut self.zoning, &self.desirability, &self.noise, &mut self.agents, &mut self.transit_network);
        
        // POLLUTION & NOISE: Dissipation & Influence logic
        self.pollution.tick(&self.allocator);
        self.noise.tick(&self.allocator, &self.transit_network.graph);
        self.desirability.tick(&self.zoning, &self.pollution, &self.noise);

        // Reset pathfind count for this tick's budget
        self.agents.pathfind_count = 0;

        self.last_tick_duration = tick_start.elapsed().as_secs_f64() * 1000.0;
        
        if self.benchmark_mode {
            self.log_benchmark_to_csv();
        }
    }

    #[func]
    pub fn get_agent_transforms(&self) -> PackedFloat32Array {
        let mut buffer = Vec::with_capacity(self.agents.count * 12);
        
        let w = self.heightmap.width as f32;
        let h = self.heightmap.height as f32;
        let hw = (w - 1.0) * 0.5;
        let hh = (h - 1.0) * 0.5;

        for i in 0..self.agents.count {
            if !self.agents.is_visible[i] { continue; }
            
            let world_x = self.agents.pos_x[i];
            let world_z = self.agents.pos_y[i];
            
            // Sample terrain height so they walk on the ground
            let map_x = (world_x + hw).clamp(0.0, w - 1.0) as usize;
            let map_z = (world_z + hh).clamp(0.0, h - 1.0) as usize;
            let world_y = self.heightmap.get_height(map_x, map_z) * 20.0 + 1.0;

            let mut scale_x = 1.0;
            let mut scale_y = 1.0;
            let mut scale_z = 1.0;
            let mut basis_x = Vector3::RIGHT;
            let mut basis_y = Vector3::UP;
            let mut basis_z = Vector3::BACK;

            if self.agents.is_driving[i] {
                scale_x = 2.0; // Width
                scale_y = 1.5; // Height
                scale_z = 3.5; // Length
                
                // ORIENT CAR TO ROAD TANGENT
                let edge_idx = self.agents.current_edge[i];
                // Defensive check for stale edge IDs
                if edge_idx != usize::MAX && edge_idx < self.transit_network.graph.edges.len() {
                    let edge = &self.transit_network.graph.edges[edge_idx];
                    let prog = self.agents.edge_progression[i] as usize;
                    if edge.physical_geometry.len() >= 2 {
                        let p1_idx = prog.min(edge.physical_geometry.len() - 2);
                        let p1 = edge.physical_geometry[p1_idx];
                        let p2 = edge.physical_geometry[p1_idx + 1];
                        let mut tangent = (p2 - p1).normalized();
                        
                        // Flip if driving backwards
                        if self.agents.current_lane[i] < 0 {
                            tangent = -tangent;
                        }
                        
                        basis_z = -tangent; // Basis.Z is "forward" but in Godot cameras/assets often -Z is forward. 
                        // However, for this primitive scaling, let's use +Z as the long axis.
                        basis_x = Vector3::UP.cross(basis_z).normalized();
                        basis_y = basis_z.cross(basis_x).normalized();
                    }
                }
            }

            buffer.push(basis_x.x * scale_x); buffer.push(basis_y.x * scale_y); buffer.push(basis_z.x * scale_z); buffer.push(world_x);
            buffer.push(basis_x.y * scale_x); buffer.push(basis_y.y * scale_y); buffer.push(basis_z.y * scale_z); buffer.push(world_y);
            buffer.push(basis_x.z * scale_x); buffer.push(basis_y.z * scale_y); buffer.push(basis_z.z * scale_z); buffer.push(world_z);
        }

        PackedFloat32Array::from_iter(buffer)
    }

    #[func]
    pub fn get_agent_paths_debug(&self) -> PackedVector3Array {
        let mut lines = Vec::new(); // Vec of points, 2 points per line segment
        for i in 0..self.agents.count {
            if self.agents.transit[i] != 0 {
                let curr = self.agents.current_node[i];
                let target = self.agents.target_node[i];
                
                // Get terrain height for raw visual height matching
                let get_h = |pos: Vector3| -> Vector3 {
                    let w = self.heightmap.width as f32;
                    let h = self.heightmap.height as f32;
                    let hw = (w - 1.0) * 0.5;
                    let hh = (h - 1.0) * 0.5;
                    let map_x = (pos.x + hw).clamp(0.0, w - 1.0) as usize;
                    let map_z = (pos.z + hh).clamp(0.0, h - 1.0) as usize;
                    let y = self.heightmap.get_height(map_x, map_z) * 20.0 + 1.0;
                    Vector3::new(pos.x, y, pos.z)
                };
                
                let current_pos = get_h(Vector3::new(self.agents.pos_x[i], 0.0, self.agents.pos_y[i]));
                
                if let Some((_cost, _dist, path)) = self.transit_network.hpa_graph.find_path(curr, target, usize::MAX, &self.transit_network.graph, false) {
                    let mut prev_pos = current_pos;
                    for &n in &path {
                        let np = get_h(self.transit_network.graph.nodes[n as usize].pos);
                        lines.push(prev_pos);
                        lines.push(np);
                        prev_pos = np;
                    }
                }
            }
        }
        PackedVector3Array::from_iter(lines)
    }

    #[func]
    pub fn get_city_demographics(&self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        
        // Calculate population
        let pop = self.agents.count;
        
        // Calculate employment rate and average happiness
        let mut employed = 0;
        let mut sum_happiness = 0.0;
        let mut sum_wealth = 0.0;
        
        if pop > 0 {
            for i in 0..pop {
                if self.agents.work_building[i] != usize::MAX {
                    employed += 1;
                }
                sum_happiness += self.agents.happiness[i];
                sum_wealth += self.agents.money[i];
            }
            let emp_rate = (employed as f32 / pop as f32) * 100.0;
            let avg_hap = sum_happiness / pop as f32;
            let avg_wealth = sum_wealth / pop as f32;
            
            dict.set("population", pop as i32);
            dict.set("employment_rate", emp_rate);
            dict.set("average_happiness", avg_hap);
            dict.set("average_wealth", avg_wealth);
        } else {
            dict.set("population", 0_i32);
            dict.set("employment_rate", 0.0_f32);
            dict.set("average_happiness", 100.0_f32);
            dict.set("average_wealth", 0.0_f32);
        }
        
        dict
    }

    #[func]
    pub fn get_building_transforms(&self, zone_type_int: u8) -> PackedFloat32Array {
        let target_zone = match zone_type_int {
            1 => ZoneType::Residential,
            2 => ZoneType::Commercial,
            3 => ZoneType::Industrial,
            4 => ZoneType::Office,
            5 => ZoneType::Mixed,
            _ => ZoneType::None,
        };

        if target_zone == ZoneType::None {
            return PackedFloat32Array::new();
        }

        let mut buffer = Vec::new();
        let w = self.heightmap.width as f32;
        let h = self.heightmap.height as f32;
        let hw = (w - 1.0) * 0.5;
        let hh = (h - 1.0) * 0.5;

        for b in &self.allocator.buildings {
            if b.zone_type == target_zone {
                // Determine 3D world position
                let world_x = b.center_x;
                let world_z = b.center_y;
                
                let grid_x = b.center_x + hw;
                let grid_y = b.center_y + hh;
                let safe_gx = grid_x.round().clamp(0.0, w - 1.0) as usize;
                let safe_gy = grid_y.round().clamp(0.0, h - 1.0) as usize;
                
                // Godot's height algorithm uses integer cell vertices, grab precisely below
                let world_y = self.heightmap.get_height(safe_gx, safe_gy) * 20.0;

                // Native structural Basis extraction matching mathematical explicit Vectors identically avoiding Euler rotation inversions
                let fd = b.facing_dir.normalized();
                let b_zx = -fd.x;
                let b_zz = -fd.y;
                let b_xx = -fd.y;
                let b_xz = fd.x;

                // Deterministic property heights scaled pseudo-randomly for visual cityscape diversity explicitly!
                let hash = ((b.center_x * 1000.0) as u32).wrapping_mul(12345).wrapping_add((b.center_y * 1000.0) as u32).wrapping_mul(67890);
                let height_scalar = 0.5 + (hash % 100) as f32 / 40.0; // Dynamic scale 0.5x to 3.0x visually

                // Scale matrix structurally inset by 5% to securely enforce physical gaps spanning diagonal grids securely preventing visual clipping!
                let sx = b.width as f32 * 0.95;
                let sy = height_scalar;
                let sz = b.depth as f32 * 0.95;

                // Godot MultiMesh Float Buffer Format (12 floats per transform, ROW MAJOR memory!)
                // Row 0: [Basis.X.x, Basis.Y.x, Basis.Z.x, Origin.x]
                buffer.push(b_xx * sx);
                buffer.push(0.0);
                buffer.push(b_zx * sz);
                buffer.push(world_x);

                // Row 1: [Basis.X.y, Basis.Y.y, Basis.Z.y, Origin.y]
                buffer.push(0.0);
                buffer.push(sy);
                buffer.push(0.0);
                buffer.push(world_y + (5.0 * sy) / 2.0); // Elevated to match the new 5m procedural house standard!

                // Row 2: [Basis.X.z, Basis.Y.z, Basis.Z.z, Origin.z]
                buffer.push(b_xz * sx);
                buffer.push(0.0);
                buffer.push(b_zz * sz);
                buffer.push(world_z);
            }
        }

        PackedFloat32Array::from_iter(buffer)
    }

    #[func]
    pub fn get_closest_point_on_edge(&self, edge_idx: i32, point_x: f32, point_y: f32) -> godot::prelude::Vector2 {
        if edge_idx < 0 || edge_idx as usize >= self.transit_network.graph.edges.len() {
            return godot::prelude::Vector2::new(point_x, point_y);
        }
        let edge = &self.transit_network.graph.edges[edge_idx as usize];
        let hw = edge.width / 2.0;

        if edge. physical_geometry.is_empty() {
            let n1 = self.transit_network.graph.nodes[edge.start_node as usize].pos;
            let n2 = self.transit_network.graph.nodes[edge.end_node as usize].pos;
            let a = godot::prelude::Vector2::new(n1.x, n1.z);
            let b = godot::prelude::Vector2::new(n2.x, n2.z);
            let p = godot::prelude::Vector2::new(point_x, point_y);
            let ab = b - a;
            let dot = ab.dot(ab);
            let t = if dot > 0.0 { ((p - a).dot(ab) / dot).clamp(0.0, 1.0) } else { 0.0 };
            
            let proj = a + ab * t;
            let tangent = if dot > 0.0 { ab.normalized() } else { godot::prelude::Vector2::new(1.0, 0.0) };
            let normal = godot::prelude::Vector2::new(-tangent.y, tangent.x);
            let p1 = proj + normal * hw;
            let p2 = proj - normal * hw;
            return if (p - p1).length_squared() < (p - p2).length_squared() { p1 } else { p2 };
        } else {
            let mut best_dist = std::f32::MAX;
            let mut best_pt = godot::prelude::Vector2::new(point_x, point_y);
            let p = godot::prelude::Vector2::new(point_x, point_y);
            for i in 0..(edge.physical_geometry.len() - 1) {
                let a = edge.physical_geometry[i];
                let b = edge.physical_geometry[i+1];
                let a_vec = godot::prelude::Vector2::new(a.x, a.z);
                let b_vec = godot::prelude::Vector2::new(b.x, b.z);
                let ab = b_vec - a_vec;
                let dot = ab.dot(ab);
                let t = if dot > 0.0 { ((p - a_vec).dot(ab) / dot).clamp(0.0, 1.0) } else { 0.0 };
                
                let proj = a_vec + ab * t;
                let tangent = if dot > 0.0 { ab.normalized() } else { godot::prelude::Vector2::new(1.0, 0.0) };
                let normal = godot::prelude::Vector2::new(-tangent.y, tangent.x);
                let p1 = proj + normal * hw;
                let p2 = proj - normal * hw;
                let e_proj = if (p - p1).length_squared() < (p - p2).length_squared() { p1 } else { p2 };

                let dist = (e_proj - p).length_squared();
                if dist < best_dist {
                    best_dist = dist;
                    best_pt = e_proj;
                }
            }
            return best_pt;
        }
    }

    #[func]
    pub fn get_edge_geometry(&self, edge_idx: i32) -> PackedVector2Array {
        let mut arr = PackedVector2Array::new();
        if edge_idx < 0 || edge_idx as usize >= self.transit_network.graph.edges.len() { return arr; }
        
        let edge = &self.transit_network.graph.edges[edge_idx as usize];
        if edge.physical_geometry.is_empty() {
            let n1 = self.transit_network.graph.nodes[edge.start_node as usize].pos;
            let n2 = self.transit_network.graph.nodes[edge.end_node as usize].pos;
            arr.push(godot::prelude::Vector2::new(n1.x, n1.z));
            arr.push(godot::prelude::Vector2::new(n2.x, n2.z));
        } else {
            for v in &edge.physical_geometry {
                arr.push(godot::prelude::Vector2::new(v.x, v.z));
            }
        }
        arr
    }

    #[func]
    pub fn get_edge_width(&self, edge_idx: i32) -> f32 {
        if edge_idx < 0 || edge_idx as usize >= self.transit_network.graph.edges.len() { return 6.0; }
        self.transit_network.graph.edges[edge_idx as usize].width
    }

    #[func]
    pub fn get_curved_frontage(&self, edge_idx: i32, start_p: godot::prelude::Vector2, end_p: godot::prelude::Vector2) -> PackedVector2Array {
        let mut arr = PackedVector2Array::new();
        if edge_idx < 0 || edge_idx as usize >= self.transit_network.graph.edges.len() { 
            return arr; // Explicitly Fail! No straight-line phantom frontages.
        }
        let edge = &self.transit_network.graph.edges[edge_idx as usize];
        let hw = edge.width / 2.0;

        let get_proj = |p: godot::prelude::Vector2| -> (usize, f32, godot::prelude::Vector2, godot::prelude::Vector2) {
            let mut best_dist = std::f32::MAX;
            let mut best_i = 0;
            let mut best_t = 0.0;
            let mut best_side = godot::prelude::Vector2::new(0.0, 0.0);
            let mut best_normal = godot::prelude::Vector2::new(0.0, 0.0);
            
            let pts: Vec<godot::prelude::Vector2> = if edge.physical_geometry.is_empty() {
                vec![
                    godot::prelude::Vector2::new(self.transit_network.graph.nodes[edge.start_node as usize].pos.x, self.transit_network.graph.nodes[edge.start_node as usize].pos.z),
                    godot::prelude::Vector2::new(self.transit_network.graph.nodes[edge.end_node as usize].pos.x, self.transit_network.graph.nodes[edge.end_node as usize].pos.z)
                ]
            } else {
                edge.physical_geometry.iter().map(|v| godot::prelude::Vector2::new(v.x, v.z)).collect()
            };

            for i in 0..(pts.len() - 1) {
                let a = pts[i]; let b = pts[i+1];
                let ab = b - a;
                let dot = ab.dot(ab);
                let t = if dot > 0.0 { ((p - a).dot(ab) / dot).clamp(0.0, 1.0) } else { 0.0 };
                let proj = a + ab * t;
                let tangent = if dot > 0.0 { ab.normalized() } else { godot::prelude::Vector2::new(1.0, 0.0) };
                let normal = godot::prelude::Vector2::new(-tangent.y, tangent.x);
                let p1 = proj + normal * hw;
                let p2 = proj - normal * hw;
                
                let (e_proj, side_normal) = if (p - p1).length_squared() < (p - p2).length_squared() { (p1, normal) } else { (p2, -normal) };
                let dist = (e_proj - p).length_squared();
                if dist < best_dist {
                    best_dist = dist; best_i = i; best_t = t; best_side = e_proj; best_normal = side_normal;
                }
            }
            (best_i, best_t, best_side, best_normal)
        };

        let (mut i1, t1, e1, n1) = get_proj(start_p);
        let (mut i2, t2, e2, _) = get_proj(end_p);
        
        let mut reverse = false;
        if i1 > i2 || (i1 == i2 && t1 > t2) {
            std::mem::swap(&mut i1, &mut i2);
            reverse = true;
        }

        let pts: Vec<godot::prelude::Vector2> = if edge.physical_geometry.is_empty() {
            vec![
                godot::prelude::Vector2::new(self.transit_network.graph.nodes[edge.start_node as usize].pos.x, self.transit_network.graph.nodes[edge.start_node as usize].pos.z),
                godot::prelude::Vector2::new(self.transit_network.graph.nodes[edge.end_node as usize].pos.x, self.transit_network.graph.nodes[edge.end_node as usize].pos.z)
            ]
        } else {
            edge.physical_geometry.iter().map(|v| godot::prelude::Vector2::new(v.x, v.z)).collect()
        };

        arr.push(if reverse { e2 } else { e1 });
        
        for idx in (i1 + 1)..=i2 {
            let a = pts[idx-1]; let b = pts[idx];
            let ab = b - a;
            let tangent = if ab.dot(ab) > 0.0 { ab.normalized() } else { godot::prelude::Vector2::new(1.0, 0.0) };
            let mut normal = godot::prelude::Vector2::new(-tangent.y, tangent.x);
            if normal.dot(n1) < 0.0 { normal = -normal; } // Keep side consistent!
            arr.push(pts[idx] + normal * hw);
        }
        
        arr.push(if reverse { e1 } else { e2 });

        if reverse {
            let mut rev_arr = PackedVector2Array::new();
            let slice = arr.as_slice();
            for i in (0..slice.len()).rev() { 
                rev_arr.push(godot::prelude::Vector2::new(slice[i].x, slice[i].y)); 
            }
            return rev_arr;
        }
        
        arr
    }

    #[func]
    pub fn add_road(&mut self, points: PackedVector3Array, fwd_lanes: i32, bkw_lanes: i32, zoning_left: bool, zoning_right: bool) {
        self.push_undo_state(false, false, true, false);
        let mut fixed_points = points.to_vec();
        
        // ROAD CONFORMANCE: Snap every point in the spline to the current terrain height
        let w = self.heightmap.width;
        let h = self.heightmap.height;
        let hw = (w - 1) as f32 * 0.5;
        let hh = (h - 1) as f32 * 0.5;
        
        for p in &mut fixed_points {
            let gx = p.x + hw;
            let gz = p.z + hh;
            let terrain_h = self.heightmap.get_height_interpolated(gx, gz) * config::HEIGHT_SCALE;
            p.y = terrain_h;
        }

        self.transit_network.add_road(fixed_points, fwd_lanes as u8, bkw_lanes as u8, zoning_left, zoning_right, &mut self.zoning, &mut self.allocator);

        let nodes = self.transit_network.graph.nodes.len();
        let edges = self.transit_network.graph.edges.len();
        godot_print!("Road added. Total: {} Nodes, {} Edges.", nodes, edges);

        // TRIGGER LOCAL RE-FLOW
        if edges > 0 {
            self.recalculate_zoning_local(edges - 1);
        }
    }

    #[func]
    pub fn get_road_mesh_data(&self) -> VarDictionary {
        let mesh_data = self.transit_network.generate_mesh_data(&self.heightmap);
        let mut dict = VarDictionary::new();
        dict.set("vertices", mesh_data.vertices);
        dict.set("normals", mesh_data.normals);
        dict.set("uvs", mesh_data.uvs);
        dict.set("colors", mesh_data.colors);
        
        dict.set("marking_vertices", mesh_data.marking_vertices);
        dict.set("marking_normals", mesh_data.marking_normals);
        dict.set("marking_uvs", mesh_data.marking_uvs);
        dict.set("marking_colors", mesh_data.marking_colors);
        dict
    }

    #[func]
    pub fn get_closest_network_point(&self, world_pos: Vector3, max_dist: f32) -> Variant {
        if let Some(pos) = interaction::get_closest_point(&self.transit_network.graph, world_pos, max_dist) {
            pos.to_variant()
        } else {
            Variant::nil()
        }
    }

    #[func]
    pub fn get_closest_node(&self, world_pos: Vector3, max_dist: f32) -> i32 {
        let mut best_id = -1;
        let mut min_d = max_dist;
        for (i, n) in self.transit_network.graph.nodes.iter().enumerate() {
            let d = n.pos.distance_to(world_pos);
            if d < min_d {
                min_d = d;
                best_id = i as i32;
            }
        }
        best_id
    }

    #[func]
    pub fn set_node_cul_de_sac(&mut self, _node_id: i32, _enabled: bool, _radius: f32) {
        // Feature removed
    }

    #[func]
    pub fn has_cul_de_sac(&self, _node_id: i32) -> bool {
        false
    }

    #[func]
    pub fn get_node_connection_count(&self, node_id: i32) -> i32 {
        if node_id < 0 { return 0; }
        let mut count = 0;
        for edge in &self.transit_network.graph.edges {
            if !edge.deleted && (edge.start_node == node_id as u32 || edge.end_node == node_id as u32) {
                count += 1;
            }
        }
        count
    }
    #[func]
    pub fn move_network_node(&mut self, node_id: i32, pos: Vector3) {
        if node_id >= 0 && (node_id as usize) < self.transit_network.graph.nodes.len() {
            self.transit_network.graph.move_node(node_id as u32, pos);
            self.push_undo_state(false, false, true, false);
        }
    }

    #[func]
    pub fn get_network_nodes(&self) -> PackedVector3Array {
        let mut arr = PackedVector3Array::new();
        // Return positions for all valid junction nodes
        for node in &self.transit_network.graph.nodes {
            arr.push(node.pos);
        }
        arr
    }

    #[func]
    pub fn set_lane_connection(&mut self, node_id: u32, from_edge: i32, from_lane: i32, to_edge: i32, to_lane: i32) {
        self.push_undo_state(false, false, true, false);
        if let Some(node) = self.transit_network.graph.nodes.get_mut(node_id as usize) {
            let key = (from_edge as usize, from_lane as i8);
            let target = (to_edge as usize, to_lane as i8);
            if !node.lane_connections.entry(key).or_default().contains(&target) {
                node.lane_connections.get_mut(&key).unwrap().push(target);
            }
        }
        self.transit_network.hpa_graph = crate::simulation::pathing::hpa::HpaGraph::build(&self.transit_network.graph);
    }

    #[func]
    pub fn clear_lane_connections(&mut self, node_id: u32) {
        self.push_undo_state(false, false, true, false);
        if let Some(node) = self.transit_network.graph.nodes.get_mut(node_id as usize) {
            node.lane_connections.clear();
        }
        self.transit_network.hpa_graph = crate::simulation::pathing::hpa::HpaGraph::build(&self.transit_network.graph);
    }

    #[func]
    pub fn get_node_pos(&self, node_id: u32) -> Vector3 {
        let valid_id = self.transit_network.graph.get_valid_node(node_id);
        if (valid_id as usize) < self.transit_network.graph.nodes.len() {
            self.transit_network.graph.nodes[valid_id as usize].pos
        } else {
            Vector3::ZERO
        }
    }

    #[func]
    pub fn get_node_lanes(&self, node_id: u32) -> VarArray {
        let mut arr = VarArray::new();
        
        let valid_node_id = self.transit_network.graph.get_valid_node(node_id);
        if valid_node_id as usize >= self.transit_network.graph.nodes.len() { return arr; }
        
        let junction_pos = self.transit_network.graph.nodes[valid_node_id as usize].pos;

        for (e_id, edge) in self.transit_network.graph.edges.iter().enumerate() {
            // Check both ends independently
            let check_start = edge.start_node == valid_node_id;
            let check_end = edge.end_node == valid_node_id;

            if !check_start && !check_end { continue; }

            // PREFER LOGICAL GEOMETRY for robust visuals
            let geo = if edge.geometry.len() >= 2 { &edge.geometry } else { &edge.physical_geometry };
            if geo.len() < 2 { continue; }
            let lc = geo.len();

            // Process each end that matches this junction.
            // If both match (self-loop), we process it twice for both stub ends!
            let possible_ends = if check_start && check_end { vec![true, false] } 
                                else if check_start { vec![true] } 
                                else { vec![false] };

            for is_start_side in possible_ends {
                // 1. Establish robust "Into-the-Leg" direction
                // ANCHOR: We must skip the "stub" (points near the center from merged nodes)
                // Search for the first point at least 3.1m away (HUB_RADIUS + margin)
                const SEARCH_RADIUS: f32 = 3.1;
                let mut diff = Vector3::ZERO;
                let mut best_stub = Vector3::ZERO;
                
                if is_start_side {
                    for j in 0..lc {
                        let d = geo[j] - junction_pos;
                        if d.length() > SEARCH_RADIUS {
                            diff = d;
                            break;
                        }
                        if d.length() > 0.1 { best_stub = d; }
                    }
                } else {
                    for j in (0..lc).rev() {
                        let d = geo[j] - junction_pos;
                        if d.length() > SEARCH_RADIUS {
                            diff = d;
                            break;
                        }
                        if d.length() > 0.1 { best_stub = d; }
                    }
                }
                
                // Fallback: If the road is very short, use the best stub or just the other end.
                if diff.length_squared() < 0.01 {
                   if best_stub.length_squared() > 0.01 {
                       diff = best_stub;
                   } else {
                       // Absolute fallback: other node's pos
                       let other_node = if is_start_side { edge.end_node } else { edge.start_node };
                       diff = self.transit_network.graph.nodes[other_node as usize].pos - junction_pos;
                   }
                }

                if diff.length_squared() < 1e-6 { continue; }
                let dir_to_leg = diff.normalized();

                // ANCHOR: Use a CONSISTENT Forward Tangent to prevent side-flipping (criss-cross)
                // If at start, dir_to_leg is forward. If at end, -dir_to_leg is forward.
                let forward_tangent = if is_start_side { dir_to_leg } else { -dir_to_leg };
                let road_normal = Vector3::new(-forward_tangent.z, 0.0, forward_tangent.x);
                
                // 2. Base position offset (5.0m ensures it's clearly past the 3.0m hub)
                let mut current_pos = junction_pos + dir_to_leg * 5.0;
                current_pos.y += 0.4;
                
                let fwd_lanes = edge.fwd_lanes;
                let bkw_lanes = edge.bkw_lanes;
                let total_lanes = (fwd_lanes + bkw_lanes) as i32;
                let lane_w = 1.0;

                // Process ALL lanes at this end
                for l_idx in 0..total_lanes {
                    let is_fwd = l_idx < fwd_lanes as i32;
                    // RHT Logic: Fwd lanes (lower indices) stay on the Right (+lateral_offset)
                    let lateral_offset = (total_lanes as f32 * 0.5 - l_idx as f32 - 0.5) * lane_w;
                    
                    // Always use road_normal for lateral placement
                    let mut lane_pos = current_pos + road_normal * lateral_offset;
                    lane_pos.y += 0.2; // Slightly lower spheres for schematic view
                    
                    let mut dict = VarDictionary::new();
                    dict.set("edge_id", e_id as i32);
                    dict.set("lane_id", if is_fwd { l_idx } else { l_idx - fwd_lanes as i32 });
                    dict.set("is_incoming", if is_fwd { !is_start_side } else { is_start_side });
                    dict.set("pos", lane_pos);
                    arr.push(&dict.to_variant());
                }
            }
        }
        arr
    }
    
    #[func]
    pub fn get_lane_connections_array(&self, node_id: u32) -> VarArray {
        let mut arr = VarArray::new();
        if node_id as usize >= self.transit_network.graph.nodes.len() { return arr; }
        let node = &self.transit_network.graph.nodes[node_id as usize];
        
        for (src, targets) in &node.lane_connections {
            for tgt in targets {
                let mut dict = VarDictionary::new();
                dict.set("from_edge", src.0 as i32);
                dict.set("from_lane", src.1 as i32);
                dict.set("to_edge", tgt.0 as i32);
                dict.set("to_lane", tgt.1 as i32);
                arr.push(&dict.to_variant());
            }
        }
        arr
    }

    #[func]
    pub fn clear_lane_source(&mut self, node_id: u32, from_edge: i32, from_lane: i32) {
        if node_id as usize >= self.transit_network.graph.nodes.len() { return; }
        
        {
            let node = &mut self.transit_network.graph.nodes[node_id as usize];
            let key = (from_edge as usize, from_lane as i8);
            node.lane_connections.remove(&key);
        }
        
        self.transit_network.hpa_graph = crate::simulation::pathing::hpa::HpaGraph::build(&self.transit_network.graph);
    }

    #[func]
    pub fn get_network_direction_at_point(&self, pos: Vector3) -> Vector3 {
        let mut avg_dir = Vector3::ZERO;
        let mut count = 0;
        
        // Find the node at this position
        for (i, node) in self.transit_network.graph.nodes.iter().enumerate() {
            if node.pos.distance_to(pos) < 0.1 {
                // Find all edges connected to this node
                for edge in &self.transit_network.graph.edges {
                    if edge.start_node == i as u32 {
                        if edge.physical_geometry.len() >= 2 {
                            let dir = (edge.physical_geometry[1] - edge.physical_geometry[0]).normalized();
                            avg_dir += dir;
                            count += 1;
                        }
                    } else if edge.end_node == i as u32 {
                        if edge.physical_geometry.len() >= 2 {
                            let last = edge.physical_geometry.len() - 1;
                            let dir = (edge.physical_geometry[last - 1] - edge.physical_geometry[last]).normalized();
                            avg_dir += dir;
                            count += 1;
                        }
                    }
                }
                break;
            }
        }
        
        if count > 0 {
            avg_dir / count as f32
        } else {
            Vector3::ZERO
        }
    }

    #[func]
    pub fn flatten_terrain_for_roads(&mut self) {
        let size = self.get_heightmap_size();
        
        // SOURCE SEPARATION: Reset visual data to clean source before carving
        self.heightmap.reset_visuals_from_source();
        
        // Create a copy for the reference terrain to satisfy borrow checker
        let ref_terrain = TerrainSystem {
            width: self.heightmap.width,
            height: self.heightmap.height,
            data: self.heightmap.data.clone(),
            source_data: self.heightmap.source_data.clone(),
        };
        self.transit_network.flatten_terrain(&ref_terrain, &mut self.heightmap.data, size);
        self.transit_network.sync_to_terrain(&self.heightmap);
        self.terrain_dirty = true;
    }

    #[func]
    pub fn get_height_at(&self, pos: Vector2) -> f32 {
        let size = self.get_heightmap_size();
        let hw = (size.x - 1.0) * 0.5;
        let hh = (size.y - 1.0) * 0.5;
        let gx = pos.x + hw;
        let gz = pos.y + hh;
        self.heightmap.get_height_interpolated(gx, gz) * config::HEIGHT_SCALE
    }

    #[func]
    pub fn intersect_terrain(&self, ray_origin: Vector3, ray_dir: Vector3) -> Variant {
        if let Some(pos) = self.heightmap.raycast_terrain(ray_origin, ray_dir) {
            pos.to_variant()
        } else {
            Variant::nil()
        }
    }

    #[func]
    pub fn load_heightmap_data(&mut self, data: PackedFloat32Array) {
        if (data.len() as usize) == self.heightmap.width * self.heightmap.height {
            self.heightmap.data = data.to_vec();
            
            // Sync roads to the new map
            self.transit_network.sync_to_terrain(&self.heightmap);
            self.flatten_terrain_for_roads();
            self.terrain_dirty = true;
        } else {
            
        }
    }

    #[func]
    pub fn get_lane_width(&self) -> f32 {
        config::LANE_WIDTH
    }

    #[func]
    pub fn setup_benchmark_city(&mut self, grid_size: i32, agent_count: i32) {
        godot_print!("Setting up benchmark city: {}x{} grid, {} agents", grid_size, grid_size, agent_count);
        self.transit_network.clear(&mut self.zoning, &mut self.allocator);
        self.agents.clear();

        let spacing = 100.0;
        let start_offset = -(grid_size as f32 * spacing * 0.5);

        // 1. Create Road Grid
        for i in 0..=grid_size {
            let offset = start_offset + (i as f32 * spacing);
            // Horizontal
            let mut h_pts = PackedVector3Array::new();
            h_pts.push(Vector3::new(start_offset, 0.0, offset));
            h_pts.push(Vector3::new(-start_offset, 0.0, offset));
            self.add_road(h_pts, 2, 2, true, true);

            // Vertical
            let mut v_pts = PackedVector3Array::new();
            v_pts.push(Vector3::new(offset, 0.0, start_offset));
            v_pts.push(Vector3::new(offset, 0.0, -start_offset));
            self.add_road(v_pts, 2, 2, true, true);
        }

        // 2. Initial Tick to build zoning/pathing
        self.simulate_tick();

        // 3. Fill with buildings (forced growth)
        self.demand.residential = 1000.0;
        self.demand.commercial = 1000.0;
        self.demand.industrial = 1000.0;
        for _ in 0..10 { // Burst growth
            self.allocator.tick(&mut self.demand, &mut self.zoning, &self.desirability, &self.noise, &mut self.agents, &mut self.transit_network);
        }

        // 4. Batch Spawn Agents
        self.agents.spawn_random_agents(agent_count as usize, &self.transit_network.graph, &self.allocator);
        godot_print!("Benchmark city ready. Agents: {}", self.agents.count);
    }

    #[func]
    pub fn get_perf_stats(&self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        let _ = dict.insert("agent_count", self.agents.count as i32);
        let _ = dict.insert("last_tick_ms", self.last_tick_duration);
        let _ = dict.insert("pathfind_calls", self.agents.pathfind_count as i32);
        let _ = dict.insert("fps", godot::classes::Engine::singleton().get_frames_per_second());
        dict
    }

    fn log_benchmark_to_csv(&self) {
        use std::io::Write;
        let path = "benchmark_results.csv";
        let file_exists = std::path::Path::new(path).exists();
        
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            if !file_exists {
                let _ = writeln!(file, "timestamp,version,agents,map_size,tick_ms,fps,pathfind_calls");
            }
            
            let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
            let version = env!("CARGO_PKG_VERSION");
            let agents = self.agents.count;
            let map_size = format!("{}x{}", self.heightmap.width, self.heightmap.height);
            let tick_ms = self.last_tick_duration;
            let fps = godot::classes::Engine::singleton().get_frames_per_second();
            let paths = self.agents.pathfind_count;
            
            let _ = writeln!(file, "{},{},{},{},{:.4},{:.1},{}", now, version, agents, map_size, tick_ms, fps, paths);
        }
    }
}

#[godot_api]
impl INode3D for SimulationNode {
    fn init(base: Base<Node3D>) -> Self {
        godot_print!("Simulation Engine Initialized (Multi-Modal Network)");
        
        // Check for --huge-map command line argument
        let args = godot::classes::Os::singleton().get_cmdline_user_args();
        let mut is_huge = false;
        for arg in args.as_slice() {
            if arg.to_string() == "--huge-map" {
                is_huge = true;
                break;
            }
        }

        let mut w = config::MAP_WIDTH;
        let mut h = config::MAP_HEIGHT;
        if is_huge {
            w = 2000;
            h = 2000;
        }
        let mut sim = Self { 
            base,
            time: TimeSystem::new(),
            time_passed: 0.0,
            heightmap: TerrainSystem::new(w, h),
            watermap: WaterSystem::new(w, h),
            transit_network: TransitNetwork::new(),
            zoning: ZoningSystem::new(),
            pollution: PollutionSystem::new(w, h),
            noise: NoiseSystem::new(w, h),
            desirability: DesirabilitySystem::new(w, h),
            demand: DemandSystem::new(),
            allocator: BuildingAllocator::new(w, h),
            agents: AgentSystem::new(),
            undo_stack: Vec::new(),
            last_tick_duration: 0.0,
            benchmark_mode: is_huge,
            terrain_dirty: true, // Initial push
            water_dirty: true,
        };

        if is_huge {
            godot_print!("HUGE MAP BENCHMARK MODE ENABLED");
            // We'll call setup_benchmark_city once we're in the scene tree / ready
            // For now, let's just adjust the starter highway to the new border
            let mut pts = PackedVector3Array::new();
            let border = (config::MAP_HEIGHT as f32 * 0.5) - 1.0;
            pts.push(Vector3::new(0.0, 0.0, -border));
            pts.push(Vector3::new(0.0, 0.0, -border + 100.0));
            sim.add_road(pts, 2, 2, true, true);
        } else {
            // Standard starter highway
            let mut pts = PackedVector3Array::new();
            let border = (config::MAP_HEIGHT as f32 * 0.5) - 1.0;
            pts.push(Vector3::new(0.0, 0.0, -border));
            pts.push(Vector3::new(0.0, 0.0, -border / 2.0));
            sim.add_road(pts, 2, 2, true, true);
        }

        sim
    }

    fn ready(&mut self) {
        if self.benchmark_mode {
            godot_print!("SimulationNode: Auto-triggering benchmark setup");
            self.setup_benchmark_city(20, 100_000); // 20x20 grid, 100k agents by default
        }
    }

    fn process(&mut self, delta: f64) {
        self.time_passed += delta;
        
        if self.time.process_delta(delta) {
            self.simulate_tick();
        }
        
        // High-frequency agent physics!
        if self.time.speed_multiplier > 0.0 {
            let dt = (delta * self.time.speed_multiplier as f64) as f32;
            self.agents.tick(&self.allocator, &self.transit_network.hpa_graph, &mut self.transit_network.graph, dt);
            
            // Water ticks with game speed too, but keep it at a fixed substep for stability
            let sub_steps = 2;
            let sub_dt = dt / sub_steps as f32;
            for _ in 0..sub_steps {
                self.watermap.tick(&self.heightmap.data, sub_dt);
            }
            self.water_dirty = true;
        }
    }
}

// ------------------------------------------------------------------
// INTERNAL HELPERS (Non-Godot-API)
// ------------------------------------------------------------------
impl SimulationNode {
    fn recalculate_zoning_local(&mut self, edge_idx: usize) {
        let graph = &self.transit_network.graph;
        if edge_idx >= graph.edges.len() { return; }
        
        let edge = &graph.edges[edge_idx];
        if edge.physical_geometry.len() < 2 { return; }
        let center = (edge.physical_geometry[0] + edge.physical_geometry[edge.physical_geometry.len()-1]) * 0.5;
        let radius = edge.physical_length * 0.5 + 150.0;

        let affected_edges = graph.get_edges_near_point(center, radius);
        for &idx in &affected_edges {
            if idx >= graph.edges.len() { continue; }
            let e = &graph.edges[idx];
            if e.deleted { continue; }
            
            let cells_long = (e.physical_length / self.zoning.grid_cell_size).floor() as usize;
            let depth = config::ZONING_DEPTH;
            
            // Collect all (side, x, y) triplets for this edge
            let mut cell_coords = Vec::with_capacity(cells_long * 2 * depth);
            for side in [1, -1] {
                for x in 0..cells_long {
                    for y in 0..depth {
                        cell_coords.push((side, x, y));
                    }
                }
            }

            // PARALLEL SCAN: Use all CPU cores to check cell obstructions
            let results: Vec<bool> = cell_coords.par_iter().map(|&(side, x, y)| {
                self.zoning.is_cell_obstructed(idx, side, x, y, graph)
            }).collect();

            // COMMIT RESULTS (Sequential HashMap update)
            for (i, &blocked) in results.iter().enumerate() {
                let (side, x, y) = cell_coords[i];
                self.zoning.set_blocked(idx, side, x, y, blocked);
            }
        }
    }
}
