//! Rendering and visual helper logic for Godot interaction.

use godot::prelude::*;
use godot::classes::MultiMesh;
use crate::nodes::simulation_node::SimulationNode;
use crate::simulation::grid::zoning::{ZoneType};
use crate::simulation::network::types::TransitFlags;
use crate::config::{self, ZONING_DEPTH};

impl SimulationNode {
    /// Updates the zoning visual MultiMeshes for tool feedback.
    pub fn update_zoning_visuals_internal(&self, mut grid_mm: Gd<MultiMesh>, mut paint_mm: Gd<MultiMesh>, hovered_edge: i32, mode: i32, mouse_pos_3d: Vector3) {
        let graph = &self.region_graph;
        let cell_size = self.config.zone_cell_m;
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
                    
                    let normal = godot::prelude::Vector2::new(tangent.y, -tangent.x) * (side as f32);

                    for y in 0..zoning_depth {
                        if self.zoning.is_blocked(edge_idx, side, x, y) { continue; }
                        
                        let depth = (y as f32 + 0.5) * cell_size;
                        let center_2d = pos_on_edge + normal * (edge.width * 0.5 + depth);
                        
                        let b_xx = tangent.x; let b_xz = tangent.y; 
                        let b_zx = -tangent.y; let b_zz = tangent.x; 

                        grid_instances.push(b_xx); grid_instances.push(0.0); grid_instances.push(b_zx); grid_instances.push(center_2d.x);
                        grid_instances.push(0.0); grid_instances.push(1.0); grid_instances.push(0.0); grid_instances.push(0.1);
                        grid_instances.push(b_xz); grid_instances.push(0.0); grid_instances.push(b_zz); grid_instances.push(center_2d.y);

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
                    let side: i8 = if side_idx == 0 { 1 } else { -1 };
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

    /// Returns the Godot Color associated with a ZoneType ID.
    pub fn get_zone_color_rust(&self, z_type: u8) -> godot::prelude::Color {
        match z_type {
            1 => Color::from_rgb(0.0, 1.0, 0.0),
            2 => Color::from_rgb(0.0, 0.5, 1.0),
            3 => Color::from_rgb(1.0, 1.0, 0.0),
            4 => Color::from_rgb(0.1, 0.8, 0.8),
            5 => Color::from_rgb(0.8, 0.0, 0.8),
            _ => Color::from_rgb(1.0, 1.0, 1.0),
        }
    }

    /// Returns the visual alpha for a zoning cell at a given depth.
    pub fn get_depth_alpha(&self, y: i32) -> f32 {
        if y < 4 { return 1.0; }
        let zoning_depth = ZONING_DEPTH as i32;
        let t = (y - 4) as f32 / (zoning_depth - 1 - 4) as f32;
        1.0 + t * (0.1 - 1.0)
    }

    /// Returns the 12-float transforms for all visible non-car agents (walkers, cyclists, etc.).
    /// Car agents are excluded — use `get_car_transforms_internal` for those.
    pub fn get_agent_transforms_internal(&self) -> PackedFloat32Array {
        use crate::simulation::economy::agents::MODE_CAR;
        let mut buffer = Vec::with_capacity(self.agents.count * 12);

        let w = self.heightmap.width as f32;
        let h = self.heightmap.height as f32;
        let hw = (w - 1.0) * 0.5;
        let hh = (h - 1.0) * 0.5;

        for i in 0..self.agents.count {
            if !self.agents.is_visible[i] { continue; }
            if self.agents.transit_mode[i] == MODE_CAR { continue; }

            let world_x = self.agents.pos_x[i];
            let world_z = self.agents.pos_y[i];

            let map_x = (world_x + hw).clamp(0.0, w - 1.0) as usize;
            let map_z = (world_z + hh).clamp(0.0, h - 1.0) as usize;
            let terrain_y = self.heightmap.get_height(map_x, map_z) * 20.0 + 1.0;
            let world_y = {
                let eidx = self.agents.current_edge[i];
                if eidx != usize::MAX && eidx < self.region_graph.edges.len() {
                    let e = &self.region_graph.edges[eidx];
                    let prog = (self.agents.edge_progression[i].max(0) as usize)
                        .min(e.physical_geometry.len().saturating_sub(1));
                    if !e.physical_geometry.is_empty() {
                        e.physical_geometry[prog].y + 1.0
                    } else {
                        terrain_y
                    }
                } else {
                    terrain_y
                }
            };

            buffer.push(1.0_f32); buffer.push(0.0_f32); buffer.push(0.0_f32); buffer.push(world_x);
            buffer.push(0.0_f32); buffer.push(1.0_f32); buffer.push(0.0_f32); buffer.push(world_y);
            buffer.push(0.0_f32); buffer.push(0.0_f32); buffer.push(1.0_f32); buffer.push(world_z);
        }

        PackedFloat32Array::from_iter(buffer)
    }

    /// Returns the 12-float transforms for all visible car agents, with road-aligned orientation.
    /// The car mesh is expected to be pre-sized (no scale applied). Origin sits at road surface + 0.1 m.
    pub fn get_car_transforms_internal(&self) -> PackedFloat32Array {
        use crate::simulation::economy::agents::MODE_CAR;
        let mut buffer = Vec::with_capacity(self.agents.count * 12);

        let w = self.heightmap.width as f32;
        let h = self.heightmap.height as f32;
        let hw = (w - 1.0) * 0.5;
        let hh = (h - 1.0) * 0.5;

        for i in 0..self.agents.count {
            if !self.agents.is_visible[i] { continue; }
            if self.agents.transit_mode[i] != MODE_CAR { continue; }

            let world_x = self.agents.pos_x[i];
            let world_z = self.agents.pos_y[i];

            let map_x = (world_x + hw).clamp(0.0, w - 1.0) as usize;
            let map_z = (world_z + hh).clamp(0.0, h - 1.0) as usize;
            let terrain_y = self.heightmap.get_height(map_x, map_z) * 20.0 + 0.1;
            let world_y = {
                let eidx = self.agents.current_edge[i];
                if eidx != usize::MAX && eidx < self.region_graph.edges.len() {
                    let e = &self.region_graph.edges[eidx];
                    let prog = (self.agents.edge_progression[i].max(0) as usize)
                        .min(e.physical_geometry.len().saturating_sub(1));
                    if !e.physical_geometry.is_empty() {
                        e.physical_geometry[prog].y + 0.15
                    } else {
                        terrain_y
                    }
                } else {
                    terrain_y
                }
            };

            let mut basis_x = Vector3::RIGHT;
            let mut basis_y = Vector3::UP;
            let mut basis_z = Vector3::BACK;

            use crate::simulation::economy::agents::{TRANSIT_ARRIVING, TRANSIT_DEPARTING};
            let transit = self.agents.transit[i];
            let edge_idx = self.agents.current_edge[i];

            if transit == TRANSIT_ARRIVING && edge_idx != usize::MAX && edge_idx < self.region_graph.edges.len() {
                // Car is peeling off the road toward a building: face perpendicular to the road edge,
                // on whichever side the agent's current position is relative to the edge centreline.
                let edge = &self.region_graph.edges[edge_idx];
                let prog = (self.agents.edge_progression[i].max(0) as usize)
                    .min(edge.physical_geometry.len().saturating_sub(2));
                if edge.physical_geometry.len() >= 2 {
                    let p1 = edge.physical_geometry[prog];
                    let p2 = edge.physical_geometry[prog + 1];
                    let tangent = (p2 - p1).normalized();
                    // Perpendicular in XZ plane (right-hand normal)
                    let perp = Vector3::new(-tangent.z, 0.0, tangent.x);
                    // Determine which side of the centreline the agent is on
                    let to_agent = Vector3::new(world_x - p1.x, 0.0, world_z - p1.z);
                    let side = if to_agent.dot(perp) >= 0.0 { 1.0_f32 } else { -1.0_f32 };
                    let facing = perp * side;
                    basis_z = -facing;
                    basis_x = Vector3::UP.cross(basis_z).normalized();
                    basis_y = basis_z.cross(basis_x).normalized();
                }
            } else if transit == TRANSIT_DEPARTING {
                // Car is leaving a building toward the road: face toward the target node.
                let node_idx = self.agents.current_node[i] as usize;
                if node_idx < self.region_graph.nodes.len() {
                    let npos = self.region_graph.nodes[node_idx].pos;
                    let dir = Vector3::new(npos.x - world_x, 0.0, npos.z - world_z);
                    if dir.length_squared() > 1e-6 {
                        basis_z = -dir.normalized();
                        basis_x = Vector3::UP.cross(basis_z).normalized();
                        basis_y = basis_z.cross(basis_x).normalized();
                    }
                }
            } else if edge_idx != usize::MAX && edge_idx < self.region_graph.edges.len() {
                // Normal on-road movement: align with road tangent.
                let edge = &self.region_graph.edges[edge_idx];
                let prog = self.agents.edge_progression[i] as usize;
                if edge.physical_geometry.len() >= 2 {
                    let p1_idx = prog.min(edge.physical_geometry.len() - 2);
                    let p1 = edge.physical_geometry[p1_idx];
                    let p2 = edge.physical_geometry[p1_idx + 1];
                    let mut tangent = (p2 - p1).normalized();
                    if self.agents.current_lane[i] < 0 {
                        tangent = -tangent;
                    }
                    basis_z = -tangent;
                    basis_x = Vector3::UP.cross(basis_z).normalized();
                    basis_y = basis_z.cross(basis_x).normalized();
                }
            }

            buffer.push(basis_x.x); buffer.push(basis_y.x); buffer.push(basis_z.x); buffer.push(world_x);
            buffer.push(basis_x.y); buffer.push(basis_y.y); buffer.push(basis_z.y); buffer.push(world_y);
            buffer.push(basis_x.z); buffer.push(basis_y.z); buffer.push(basis_z.z); buffer.push(world_z);
        }

        PackedFloat32Array::from_iter(buffer)
    }

    /// Returns debug path geometry for active agents.
    pub fn get_agent_paths_debug_internal(&self) -> PackedVector3Array {
        let mut lines = Vec::new();
        for i in 0..self.agents.count {
            if self.agents.transit[i] != 0 {
                let curr = self.agents.current_node[i];
                let target = self.agents.target_node[i];
                
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
                
                if let Some((_cost, _dist, path)) = self.transit_network.cch_graph.find_path(curr, target, usize::MAX, &self.region_graph, TransitFlags::CAR) {
                    let mut prev_pos = current_pos;
                    for &n in &path {
                        let np = get_h(self.region_graph.nodes[n as usize].pos);
                        lines.push(prev_pos);
                        lines.push(np);
                        prev_pos = np;
                    }
                }
            }
        }
        PackedVector3Array::from_iter(lines)
    }

    /// Returns the 12-float transforms for building MultiMeshes.
    pub fn get_building_transforms_internal(&self, zone_type_int: u8) -> PackedFloat32Array {
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
                let world_x = b.center_x;
                let world_z = b.center_y;
                
                let grid_x = b.center_x + hw;
                let grid_y = b.center_y + hh;
                let safe_gx = grid_x.round().clamp(0.0, w - 1.0) as usize;
                let safe_gy = grid_y.round().clamp(0.0, h - 1.0) as usize;
                
                let world_y = self.heightmap.get_height(safe_gx, safe_gy) * 20.0;

                let fd = b.facing_dir.normalized();
                let b_zx = -fd.x;
                let b_zz = -fd.y;
                let b_xx = -fd.y;
                let b_xz = fd.x;

                let hash = ((b.center_x * 1000.0) as u32).wrapping_mul(12345).wrapping_add((b.center_y * 1000.0) as u32).wrapping_mul(67890);
                let height_scalar = 0.5 + (hash % 100) as f32 / 40.0; 

                let sx = b.width as f32 * 0.95;
                let sy = height_scalar;
                let sz = b.depth as f32 * 0.95;

                buffer.push(b_xx * sx);
                buffer.push(0.0);
                buffer.push(b_zx * sz);
                buffer.push(world_x);

                buffer.push(0.0);
                buffer.push(sy);
                buffer.push(0.0);
                buffer.push(world_y + (5.0 * sy) / 2.0); 

                buffer.push(b_xz * sx);
                buffer.push(0.0);
                buffer.push(b_zz * sz);
                buffer.push(world_z);
            }
        }

        PackedFloat32Array::from_iter(buffer)
    }

    /// Returns a dictionary containing all road mesh geometry for Godot.
    pub fn get_road_mesh_data_internal(&self) -> VarDictionary {
        let mesh_data = self.transit_network.generate_mesh_data(&self.region_graph, &self.heightmap);
        let mut dict = VarDictionary::new();
        dict.set("vertices", mesh_data.vertices);
        dict.set("normals", mesh_data.normals);
        dict.set("uvs", mesh_data.uvs);
        dict.set("colors", mesh_data.colors);
        
        dict.set("marking_vertices", mesh_data.marking_vertices);
        dict.set("marking_normals", mesh_data.marking_normals);
        dict.set("marking_uvs", mesh_data.marking_uvs);
        dict.set("marking_colors", mesh_data.marking_colors);
        
        dict.set("concrete_vertices", mesh_data.concrete_vertices);
        dict.set("concrete_normals", mesh_data.concrete_normals);
        dict.set("concrete_uvs", mesh_data.concrete_uvs);
        dict.set("concrete_colors", mesh_data.concrete_colors);
        dict
    }
}
