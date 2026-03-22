use crate::simulation::network::graph::TransitGraph;
use crate::simulation::grid::zoning::{ZoningSystem, ZoneType};
use godot::prelude::Vector2;

pub struct Building {
    pub center_x: f32,
    pub center_y: f32,
    pub width: u8,
    pub depth: u8,
    pub zone_type: ZoneType,
    pub facing_dir: Vector2,
    pub frontage_node: u32,
    pub side_offset: f32, 
    pub abandoned_timer: u32,
    pub edge_idx: usize,
    pub side: i8,
    pub cell_x: usize,
    pub cell_y: usize,
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

    pub fn tick(&mut self, _demand: &mut crate::simulation::economy::demand::DemandSystem, zoning: &mut ZoningSystem, _desirability: &crate::simulation::grid::desirability::DesirabilitySystem, _noise: &crate::simulation::grid::noise::NoiseSystem, _agents: &mut crate::simulation::economy::agents::AgentSystem, network: &mut crate::simulation::network::TransitNetwork) {
        let mut graph_changed = false;
        
        // 1. Cleanup: Remove buildings if their cells are no longer zoned correctly OR if the edge they're on is gone.
        let mut i = 0;
        while i < self.buildings.len() {
            let b = &self.buildings[i];
            let remove = if let Some(_) = zoning.edge_grids.get(&b.edge_idx) {
                let current_type = zoning.get_cell(b.edge_idx, b.side, b.cell_x, b.cell_y);
                current_type != b.zone_type
            } else {
                true // Edge gone
            };

            if remove {
                network.remove_frontage(b.frontage_node);
                // Clear occupancy
                zoning.set_occupied(b.edge_idx, b.side, b.cell_x, b.cell_y, false);
                self.buildings.swap_remove(i);
                graph_changed = true;
            } else {
                i += 1;
            }
        }

        // 2. Growth Logic: Find areas with un-occupied zones and high demand
        let edges_to_check: Vec<usize> = zoning.edge_grids.keys().cloned().collect();
        
        for edge_idx in edges_to_check {
            let (edge_len, edge_width) = if let Some(edge) = network.graph.edges.get(edge_idx) {
                if edge.physical_geometry.len() < 2 { (0.0, 0.0) }
                else { (edge.physical_length, edge.width) }
            } else {
                (0.0, 0.0)
            };
            
            if edge_len < 0.1 { continue; }
            
            for side in [1, -1] {
                let cells_long = if let Some(g) = zoning.edge_grids.get(&edge_idx) { g.cells_long } else { 0 };
                
                for x in (0..cells_long.saturating_sub(3)).step_by(2) {
                    let z_type = zoning.get_cell(edge_idx, side, x, 0);
                    if z_type == ZoneType::None { continue; }
                    
                    let demand = match z_type {
                        ZoneType::Residential => _demand.residential,
                        ZoneType::Commercial => _demand.commercial,
                        ZoneType::Industrial => _demand.industrial,
                        ZoneType::Office => _demand.commercial * 0.5,
                        ZoneType::Mixed => (_demand.residential + _demand.commercial) * 0.5,
                        _ => 0.0,
                    };
                    
                    if demand < 10.0 { continue; }

                    let mut can_build = true;
                    for dx in 0..4 {
                        for dy in 0..4 {
                            if zoning.get_cell(edge_idx, side, x + dx, dy) != z_type || zoning.is_occupied(edge_idx, side, x + dx, dy) {
                                can_build = false;
                                break;
                            }
                        }
                        if !can_build { break; }
                    }

                    if can_build {
                        let t = (x as f32 + 2.0) * zoning.grid_cell_size / edge_len;
                        let world_pos_on_edge = self.get_pos_on_edge(&network.graph, edge_idx, t);
                        let tangent = self.get_tangent_on_edge(&network.graph, edge_idx, t);
                        let normal = godot::prelude::Vector2::new(-tangent.y, tangent.x) * (side as f32);
                        
                        let depth_offset = 2.0 * zoning.grid_cell_size; 
                        let center_2d = world_pos_on_edge + normal * (edge_width * 0.5 + depth_offset);
                        
                        let frontage_pos_3d = godot::prelude::Vector3::new(world_pos_on_edge.x, 0.0, world_pos_on_edge.y);
                        let frontage_node = network.split_for_frontage(edge_idx, frontage_pos_3d);
                        graph_changed = true;

                        let b = Building {
                            center_x: center_2d.x,
                            center_y: center_2d.y,
                            width: (4.0 * zoning.grid_cell_size) as u8,
                            depth: (4.0 * zoning.grid_cell_size) as u8,
                            zone_type: z_type,
                            facing_dir: -normal,
                            frontage_node,
                            side_offset: side as f32,
                            abandoned_timer: 0,
                            edge_idx,
                            side,
                            cell_x: x,
                            cell_y: 0,
                        };
                        // 2. Check overlap with other buildings/roads
                        let mut blocked = false;
                        for b_x in x..x+4 {
                            for b_y in 0..4 { // Assuming y is always 0 for now, based on `cell_y: 0`
                                if zoning.is_occupied(edge_idx, side, b_x, b_y) {
                                    blocked = true; break;
                                }
                                if zoning.is_cell_obstructed(edge_idx, side, b_x, b_y, &network.graph) {
                                    blocked = true; break;
                                }
                            }
                            if blocked { break; }
                        }
                        
                        // Mark all 16 cells as occupied
                        for dx in 0..4 {
                            for dy in 0..4 {
                                zoning.set_occupied(edge_idx, side, x + dx, dy, true);
                            }
                        }
                        self.buildings.push(b);
                        
                        // Subtract from demand
                        match z_type {
                            ZoneType::Residential => _demand.residential -= 5.0,
                            ZoneType::Commercial => _demand.commercial -= 5.0,
                            ZoneType::Industrial => _demand.industrial -= 5.0,
                            _ => {}
                        }
                    }
                }
            }
        }

        if graph_changed {
            network.rebuild_pathing();
        }

        // 3. Immigration Logic
        let total_capacity: usize = self.buildings.iter()
            .filter(|b| b.zone_type == ZoneType::Residential || b.zone_type == ZoneType::Mixed)
            .count() * 6;
            
        if _agents.count < total_capacity {
            let demand_factor = (_demand.residential / 100.0).max(0.0).min(1.0);
            let gap = total_capacity - _agents.count;
            let num_to_spawn = ((gap as f32 * 0.2 * demand_factor) as usize).max(1).min(10); 
            
            for _ in 0..num_to_spawn {
                let highway_pos = godot::prelude::Vector3::new(0.0, 0.0, -127.0);
                if let Some(highway_node) = crate::simulation::network::interaction::get_closest_node(&network.graph, highway_pos, 1000.0) {
                    let highway_world_pos = network.graph.nodes[highway_node as usize].pos;
                    _agents.spawn_agent(usize::MAX, highway_node, 0.0, 0.0, highway_node, highway_world_pos.x, highway_world_pos.z);
                }
            }
        }
        
        self.dirty = false;
    }

    fn get_pos_on_edge(&self, graph: &TransitGraph, edge_idx: usize, t: f32) -> Vector2 {
        let edge = &graph.edges[edge_idx];
        let geo = &edge.physical_geometry;
        if geo.is_empty() { return Vector2::ZERO; }
        
        let target_dist = t * edge.physical_length;
        let mut curr_dist = 0.0;
        
        for i in 0..geo.len() - 1 {
            let p1 = Vector2::new(geo[i].x, geo[i].z);
            let p2 = Vector2::new(geo[i+1].x, geo[i+1].z);
            let d = (p2 - p1).length();
            if curr_dist + d >= target_dist {
                let local_t = (target_dist - curr_dist) / d;
                return p1 + (p2 - p1) * local_t;
            }
            curr_dist += d;
        }
        Vector2::new(geo.last().unwrap().x, geo.last().unwrap().z)
    }

    fn get_tangent_on_edge(&self, graph: &TransitGraph, edge_idx: usize, t: f32) -> Vector2 {
        let edge = &graph.edges[edge_idx];
        let geo = &edge.physical_geometry;
        if geo.len() < 2 { return Vector2::new(1.0, 0.0); }
        
        let target_dist = t * edge.physical_length;
        let mut curr_dist = 0.0;
        
        for i in 0..geo.len() - 1 {
            let p1 = Vector2::new(geo[i].x, geo[i].z);
            let p2 = Vector2::new(geo[i+1].x, geo[i+1].z);
            let d = (p2 - p1).length();
            if curr_dist + d >= target_dist {
                return (p2 - p1).normalized();
            }
            curr_dist += d;
        }
        (Vector2::new(geo.last().unwrap().x, geo.last().unwrap().z) - Vector2::new(geo[geo.len()-2].x, geo[geo.len()-2].z)).normalized()
    }
}
