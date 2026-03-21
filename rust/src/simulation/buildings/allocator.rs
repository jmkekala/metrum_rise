use crate::simulation::network::graph::TransitGraph;
use crate::simulation::grid::zoning::{ZoningSystem, ZoneType};
use godot::prelude::Vector2;

fn point_in_polygon(pt: godot::prelude::Vector2, poly: &[godot::prelude::Vector2]) -> bool {
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[j];
        if (a.y > pt.y) != (b.y > pt.y) {
            let intersect_x = a.x + (pt.y - a.y) / (b.y - a.y) * (b.x - a.x);
            if pt.x < intersect_x {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

pub struct Building {
    pub center_x: f32,
    pub center_y: f32,
    pub width: u8,
    pub depth: u8,
    pub zone_type: ZoneType,
    pub facing_dir: Vector2,
    pub frontage_node: u32,
    pub side_offset: f32, // +1.0 for Left, -1.0 for Right
    pub abandoned_timer: u32,
    pub polygon_id: u32,
    pub polygon_version: u32,
}

pub struct BuildingAllocator {
    pub buildings: Vec<Building>,
    pub dirty: bool,
}

impl BuildingAllocator {
    pub fn new(_width: usize, _height: usize) -> Self {
        Self {
            buildings: Vec::new(),
            dirty: false,
        }
    }

    pub fn clear(&mut self) {
        self.buildings.clear();
        self.dirty = false;
    }

    pub fn tick(&mut self, _demand: &mut crate::simulation::economy::demand::DemandSystem, _zoning: &ZoningSystem, _desirability: &crate::simulation::grid::desirability::DesirabilitySystem, _noise: &crate::simulation::grid::noise::NoiseSystem, _agents: &mut crate::simulation::economy::agents::AgentSystem, _network: &mut crate::simulation::network::TransitNetwork) {
        let mut graph_changed = false;
        if self.dirty {
            // 1. Identify buildings to remove (based on zoning deletion)
            let mut to_remove_indices = Vec::new();
            for (i, b) in self.buildings.iter().enumerate() {
                if !_zoning.polygons.iter().any(|p| p.id == b.polygon_id && p.version == b.polygon_version) {
                    to_remove_indices.push(i);
                }
            }

            // 2. Cleanup frontage nodes before removing buildings
            for &idx in to_remove_indices.iter().rev() {
                let b = &self.buildings[idx];
                _network.remove_frontage(b.frontage_node);
                self.buildings.remove(idx);
                graph_changed = true;
            }
            self.dirty = false;
        }
        
        // 3. Process new building allocation
        let mut handled_polys = std::collections::HashSet::new();
        for b in &self.buildings {
            handled_polys.insert(b.polygon_id);
        }

        for poly in &_zoning.polygons {
            if handled_polys.contains(&poly.id) { continue; }
            for frontage in &poly.frontages {
                let n = frontage.count;
                if n < 2 { continue; }
                let s_idx = frontage.start_idx;
                
                let mut front_len = 0.0;
                for i in 0..n-1 {
                    let idx1 = (s_idx + i) % poly.vertices.len();
                    let idx2 = (s_idx + i + 1) % poly.vertices.len();
                    front_len += (poly.vertices[idx2] - poly.vertices[idx1]).length();
                }
                if front_len < 4.0 { continue; }
                
                let density_width = 12.0; 
                let num_plots = (front_len / density_width).max(1.0) as u32;
                let slice_width = front_len / (num_plots as f32);
                
                for i in 0..num_plots {
                    let target_dist = (i as f32 + 0.5) * slice_width;
                    let mut accumulated = 0.0;
                    let mut front_center = poly.vertices[s_idx % poly.vertices.len()];
                    let mut in_dir = godot::prelude::Vector2::new(0.0, 0.0);
                    
                    for j in 0..n-1 {
                        let idx1 = (s_idx + j) % poly.vertices.len();
                        let idx2 = (s_idx + j + 1) % poly.vertices.len();
                        let v0 = poly.vertices[idx1];
                        let v1 = poly.vertices[idx2];
                        let seg_len = (v1 - v0).length();
                        if accumulated + seg_len >= target_dist || j == n-2 {
                            let t = if seg_len > 0.0 { ((target_dist - accumulated) / seg_len).clamp(0.0, 1.0) } else { 0.0 };
                            front_center = v0 + (v1 - v0) * t;
                            let tangent = if seg_len > 0.0 { (v1 - v0).normalized() } else { godot::prelude::Vector2::new(1.0, 0.0) };
                            let normal = godot::prelude::Vector2::new(-tangent.y, tangent.x);
                            
                            // Check inward normal direction using inclusion test
                            in_dir = normal;
                            if !point_in_polygon(front_center + in_dir * 4.0, &poly.vertices) {
                                in_dir = -normal;
                            }
                            break;
                        }
                        accumulated += seg_len;
                    }
                    
                    if in_dir.length_squared() < 0.1 { continue; } // safe fallback
                    
                    let r1 = godot::prelude::Vector2::new(in_dir.y, -in_dir.x);
                    let bw = (slice_width * 0.95) as u8;
                    let hw = bw as f32 / 2.0 - 0.1;
                    
                    let mut safe_bd = 0;
                    for try_bd in (4..=15).rev() {
                        let d = try_bd as f32;
                        let c = front_center + in_dir * (d / 2.0);
                        let hd = d / 2.0 - 0.1;
                        let back_left = c + in_dir * hd - r1 * hw;
                        let back_right = c + in_dir * hd + r1 * hw;
                        let front_left = c - in_dir * hd - r1 * hw;
                        let front_right = c - in_dir * hd + r1 * hw;
                        if point_in_polygon(back_left, &poly.vertices) && point_in_polygon(back_right, &poly.vertices) &&
                           point_in_polygon(front_left, &poly.vertices) && point_in_polygon(front_right, &poly.vertices) {
                            safe_bd = try_bd;
                            break;
                        }
                    }
                    
                    let bd = safe_bd;
                    if bd < 4 { continue; }
                    
                    let center_2d = front_center + in_dir * ((bd as f32) / 2.0);
                    
                    // Overlap Check
                    let mut overlap = false;
                    let f1 = in_dir; 
                    let r1_vec = godot::prelude::Vector2::new(f1.y, -f1.x); 
                    for other in &self.buildings {
                        let f2 = -other.facing_dir;
                        let r2 = godot::prelude::Vector2::new(f2.y, -f2.x);
                        let diff = center_2d - godot::prelude::Vector2::new(other.center_x, other.center_y);
                        let axes = [f1, r1_vec, f2, r2];
                        let mut sep_found = false;
                        for &axis in &axes {
                            let proj1 = (bd as f32 / 2.0 * f1.dot(axis).abs()) + (bw as f32 / 2.0 * r1_vec.dot(axis).abs());
                            let proj2 = (other.depth as f32 / 2.0 * f2.dot(axis).abs()) + (other.width as f32 / 2.0 * r2.dot(axis).abs());
                            if diff.dot(axis).abs() >= proj1 + proj2 + 0.1 {
                                sep_found = true;
                                break;
                            }
                        }
                        if !sep_found { overlap = true; break; }
                    }
                    if overlap { continue; }

                    // 4. Create building and its frontage node
                    let front_center_3d = godot::prelude::Vector3::new(front_center.x, 0.0, front_center.y);
                    let frontage_node = _network.split_for_frontage(frontage.edge_idx, front_center_3d);
                    graph_changed = true;

                    let b_data = Building {
                        center_x: center_2d.x,
                        center_y: center_2d.y,
                        width: bw,
                        depth: bd,
                        zone_type: poly.zone_type,
                        facing_dir: -in_dir,
                        frontage_node,
                        side_offset: 1.0, // Used by visuals, arbitrarily left/right is fine for now
                        abandoned_timer: 0,
                        polygon_id: poly.id,
                        polygon_version: poly.version,
                    };
                    self.buildings.push(b_data);
                }
            }
        }
        
        // 5. Finalize Graph if needed
        if graph_changed {
            _network.rebuild_pathing();
        }

        // Immigration Logic
        let total_capacity: usize = self.buildings.iter()
            .filter(|b| b.zone_type == ZoneType::Residential)
            .count() * 6;
            
        if _agents.count < total_capacity {
            let demand_factor = (_demand.residential / 100.0).max(0.0).min(1.0);
            let gap = total_capacity - _agents.count;
            let num_to_spawn = ((gap as f32 * 0.2 * demand_factor) as usize).max(1).min(10); 
            
            for _ in 0..num_to_spawn {
                let highway_pos = godot::prelude::Vector3::new(0.0, 0.0, -127.0);
                if let Some(highway_node) = crate::simulation::network::interaction::get_closest_node(&_network.graph, highway_pos, 1000.0) {
                    let highway_world_pos = _network.graph.nodes[highway_node as usize].pos;
                    _agents.spawn_agent(usize::MAX, highway_node, 0.0, 0.0, highway_node, highway_world_pos.x, highway_world_pos.z);
                }
            }
        }
        
        self.dirty = false;
    }
}
