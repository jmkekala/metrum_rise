use crate::simulation::network::graph::TransitGraph;
use crate::simulation::grid::zoning::{ZoningSystem, ZoneType};
use godot::prelude::{Vector3, Vector2};
use std::collections::HashMap;

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
    pub road_node: u32,
    pub road_edge: usize,
    pub abandoned_timer: u32,
    pub polygon_id: u32,
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

    pub fn tick(&mut self, _demand: &mut crate::simulation::economy::demand::DemandSystem, _zoning: &ZoningSystem, _desirability: &crate::simulation::grid::desirability::DesirabilitySystem, _noise: &crate::simulation::grid::noise::NoiseSystem, _agents: &mut crate::simulation::economy::agents::AgentSystem, _graph: &TransitGraph) {
        if !self.dirty { return; }
        
        let mut handled_polys = std::collections::HashSet::new();
        for b in &self.buildings {
            handled_polys.insert(b.polygon_id);
        }

        for poly in &_zoning.polygons {
            if handled_polys.contains(&poly.id) { continue; }
            if poly.vertices.len() < 2 { continue; } // Require at least a frontage edge explicitly
            
            let v0 = poly.vertices[0];
            let v1 = poly.vertices[1];
            
            let front_vec = v1 - v0;
            let front_len = front_vec.length();
            if front_len < 4.0 { continue; } // Failsafe against infinitesimally drawn shapes completely!
            
            // Subdivide the drawn user frontage geometrically!
            let density_width = 12.0; // 12 meter average plot
            let num_plots = (front_len / density_width).max(1.0) as u32;
            let slice_width = front_len / (num_plots as f32);
            
            for i in 0..num_plots {
                let t_center = (i as f32 + 0.5) / (num_plots as f32);
                let front_center = v0 + front_vec * t_center;
                let in_dir = -poly.facing_dir;
                let r1 = godot::prelude::Vector2::new(in_dir.y, -in_dir.x);
                let bw = (slice_width * 0.95) as u8;
                let hw = bw as f32 / 2.0 - 0.1;
                
                // Iteratively discover the maximum bounding box cleanly enveloped natively inside skewed bounds
                let mut safe_bd = 0;
                for try_bd in (4..=15).rev() {
                    let d = try_bd as f32;
                    let c = front_center + in_dir * (d / 2.0);
                    let hd = d / 2.0 - 0.1; // Marginal shrinkage suppressing boundaries testing
                    
                    let back_left = c + in_dir * hd - r1 * hw;
                    let back_right = c + in_dir * hd + r1 * hw;
                    let front_left = c - in_dir * hd - r1 * hw;
                    let front_right = c - in_dir * hd + r1 * hw;
                    
                    if point_in_polygon(back_left, &poly.vertices) && 
                       point_in_polygon(back_right, &poly.vertices) &&
                       point_in_polygon(front_left, &poly.vertices) &&
                       point_in_polygon(front_right, &poly.vertices) {
                        safe_bd = try_bd;
                        break;
                    }
                }
                
                let bd = safe_bd;
                if bd < 4 { continue; } // Exclude disjoint fragments extending off grids!
                
                let center_2d = front_center + in_dir * ((bd as f32) / 2.0);
                
                let road_node = _graph.edges[poly.edge_idx].start_node;

                // Absolute OBB collision resolution guaranteeing physical bounds avoiding intersecting backroads!
                let mut overlap = false;
                let f1 = in_dir; // Depth vector direction
                let r1 = godot::prelude::Vector2::new(f1.y, -f1.x); // Width vector
                for other in &self.buildings {
                    let f2 = -other.facing_dir;
                    let r2 = godot::prelude::Vector2::new(f2.y, -f2.x);
                    let diff = center_2d - godot::prelude::Vector2::new(other.center_x, other.center_y);
                    
                    let axes = [f1, r1, f2, r2];
                    let mut sep_found = false;
                    for &axis in &axes {
                        let proj1 = (bd as f32 / 2.0 * f1.dot(axis).abs()) + (bw as f32 / 2.0 * r1.dot(axis).abs());
                        let proj2 = (other.depth as f32 / 2.0 * f2.dot(axis).abs()) + (other.width as f32 / 2.0 * r2.dot(axis).abs());
                        if diff.dot(axis).abs() >= proj1 + proj2 + 0.1 { // Tiny spacing margin added
                            sep_found = true;
                            break;
                        }
                    }
                    if !sep_found {
                        overlap = true;
                        break;
                    }
                }
                if overlap { continue; } // Rigid constraints applied!

                let b_data = Building {
                    center_x: center_2d.x,
                    center_y: center_2d.y,
                    width: bw,
                    depth: bd,
                    zone_type: poly.zone_type,
                    facing_dir: poly.facing_dir,
                    road_node,
                    road_edge: poly.edge_idx,
                    abandoned_timer: 0,
                    polygon_id: poly.id,
                };
                
                let slot_id = self.buildings.len();
                self.buildings.push(b_data);
                
                // Construct physical agents driving in from the boundary natively
                let highway_pos = godot::prelude::Vector3::new(0.0, 0.0, -127.0);
                let highway_node = crate::simulation::network::interaction::get_closest_node(_graph, highway_pos, 1000.0).unwrap_or(road_node);
                let highway_world_pos = if highway_node != road_node {
                    _graph.nodes[highway_node as usize].pos
                } else {
                    godot::prelude::Vector3::new(center_2d.x, 0.0, center_2d.y)
                };
                
                for _ in 0..6 {
                    _agents.spawn_agent(slot_id, road_node, front_center.x, front_center.y, highway_node, highway_world_pos.x, highway_world_pos.z);
                }
            }
        }
        
        self.dirty = false;
    }
}
