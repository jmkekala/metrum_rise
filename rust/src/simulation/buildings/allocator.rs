//! Building placement and lifecycle management.
//!
//! [`BuildingAllocator::tick`] runs once per simulation tick. It:
//! 1. Removes buildings whose zoning cell has been changed or whose road edge was deleted.
//! 2. Scans zoned, unoccupied cells with sufficient demand and spawns new buildings.
//! 3. Spawns immigrant agents up to the current residential capacity.
//!
//! **Known issues (see `docs/project.md`):**
//! - No spawn throttle — can spawn hundreds of buildings per tick (bug B6).
//! - Desirability is not checked before placement (bug B5).
//! - Each placement triggers a full HPA* rebuild via `split_for_frontage`.

use crate::simulation::network::graph::TransitGraph;
use crate::simulation::grid::zoning::{ZoningSystem, ZoneType};
use godot::prelude::Vector2;

/// A placed building occupying a 3 × 3 cell (30 m × 30 m) footprint on a zoning grid.
pub struct Building {
    /// World-space X centre of the building footprint (metres).
    pub center_x: f32,
    /// World-space Z centre of the building footprint (metres, Godot's forward axis).
    pub center_y: f32,
    /// Footprint width in metres (always 30 at present).
    pub width: u8,
    /// Footprint depth in metres (always 30 at present).
    pub depth: u8,
    /// Zone category this building was spawned into.
    pub zone_type: ZoneType,
    /// Unit vector pointing from the road toward the building (outward normal from the road edge).
    pub facing_dir: Vector2,
    /// Node ID in [`TransitGraph`] created by splitting the road at this building's frontage.
    pub frontage_node: u32,
    /// Signed side of the road: `+1.0` = left, `-1.0` = right (relative to edge direction).
    pub side_offset: f32,
    /// Ticks since this building lost its zoning. Non-zero values are reserved for future
    /// abandonment / decay logic; currently unused.
    pub abandoned_timer: u32,
    /// Index into [`TransitGraph::edges`] for the road segment this building fronts.
    pub edge_idx: usize,
    /// Road side: `1` = left, `-1` = right.
    pub side: i8,
    /// Column index (along the road) of the building's leading cell in the zoning grid.
    pub cell_x: usize,
    /// Row index (depth from road) of the building's leading cell; always `0` (first row).
    pub cell_y: usize,
}

/// Manages the full lifecycle of [`Building`]s: placement, removal, and immigrant spawning.
pub struct BuildingAllocator {
    /// All currently placed buildings. Removal uses `swap_remove` — order is not preserved.
    pub buildings: Vec<Building>,
    /// Set to `true` when the building list changes in a tick, signalling renderers to refresh.
    pub dirty: bool,
}

impl BuildingAllocator {
    /// Creates an empty allocator. `_width` and `_height` are reserved for future spatial indexing.
    pub fn new(_width: usize, _height: usize) -> Self {
        Self {
            buildings: Vec::new(),
            dirty: false,
        }
    }

    /// Removes all buildings and resets the dirty flag.
    pub fn clear(&mut self) {
        self.buildings.clear();
        self.dirty = false;
    }

    /// Advances the building lifecycle by one simulation tick.
    ///
    /// Removes stale buildings, grows new ones into high-demand zones, and spawns immigrants.
    /// Calls `network.rebuild_pathing()` once if any building was added or removed.
    pub fn tick(&mut self, _demand: &mut crate::simulation::economy::demand::DemandSystem, zoning: &mut ZoningSystem, _desirability: &crate::simulation::grid::desirability::DesirabilitySystem, _noise: &crate::simulation::grid::noise::NoiseSystem, _agents: &mut crate::simulation::economy::agents::AgentSystem, network: &mut crate::simulation::network::TransitNetwork) {
        let mut graph_changed = false;
        
        // 1. Cleanup: Remove buildings if their cells are no longer zoned correctly OR if the edge they're on is gone.
        let mut i = 0;
        while i < self.buildings.len() {
            let b = &self.buildings[i];
            let remove = if let Some(_) = zoning.edge_grids.get(&b.edge_idx) {
                let current_type = zoning.get_cell(b.edge_idx, b.side, b.cell_x, b.cell_y);
                current_type != b.zone_type || network.graph.edges[b.edge_idx].deleted
            } else {
                true // Edge gone
            };

            if remove {
                let frontage_node = b.frontage_node;
                let b_edge_idx = b.edge_idx;
                let b_side = b.side;
                let b_cell_x = b.cell_x;
                let b_cell_y = b.cell_y;
                let b_width = b.width;
                let b_depth = b.depth;

                network.remove_frontage(frontage_node, zoning, self);
                // Clear occupancy for the entire footprint
                let w_cells = (b_width as f32 / zoning.grid_cell_size).round() as usize;
                let d_cells = (b_depth as f32 / zoning.grid_cell_size).round() as usize;
                for dx in 0..w_cells {
                    for dy in 0..d_cells {
                        zoning.set_occupied(b_edge_idx, b_side, b_cell_x + dx, b_cell_y + dy, false);
                    }
                }
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
                if edge.deleted || edge.physical_geometry.len() < 2 { (0.0, 0.0) }
                else { (edge.physical_length, edge.width) }
            } else {
                (0.0, 0.0)
            };
            
            if edge_len < 0.1 { continue; }
            
            // Batch fetch nearby edges for the entire edge to optimize obstruction checks
            let padding = 120.0;
            let edge = &network.graph.edges[edge_idx];
            let mut min_x = f32::MAX; let mut max_x = f32::MIN;
            let mut min_z = f32::MAX; let mut max_z = f32::MIN;
            for p in &edge.physical_geometry {
                min_x = min_x.min(p.x); max_x = max_x.max(p.x);
                min_z = min_z.min(p.z); max_z = max_z.max(p.z);
            }
            let nearby_edges = network.graph.get_edges_near_aabb(
                godot::prelude::Vector3::new(min_x - padding, 0.0, min_z - padding),
                godot::prelude::Vector3::new(max_x + padding, 0.0, max_z + padding)
            );

            for side in [1, -1] {
                let cells_long = if let Some(g) = zoning.edge_grids.get(&edge_idx) { g.cells_long } else { 0 };
                
                for x in 0..cells_long.saturating_sub(2) {
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
                    // Check 3x3 footprint for zone type and occupancy
                    for dx in 0..3 {
                        for dy in 0..3 {
                            if zoning.get_cell(edge_idx, side, x + dx, dy) != z_type || 
                               zoning.is_occupied(edge_idx, side, x + dx, dy) ||
                               zoning.is_cell_obstructed(edge_idx, side, x + dx, dy, &network.graph, Some(&nearby_edges)) {
                                can_build = false;
                                break;
                            }
                        }
                        if !can_build { break; }
                    }

                    if can_build {
                        let t = (x as f32) * zoning.grid_cell_size / edge_len;
                        let world_pos_on_edge = self.get_pos_on_edge(&network.graph, edge_idx, t);
                        let tangent = self.get_tangent_on_edge(&network.graph, edge_idx, t);
                        let normal = godot::prelude::Vector2::new(tangent.y, -tangent.x) * (side as f32);
                        
                        let b_width = 3.0 * zoning.grid_cell_size;
                        let b_depth = 3.0 * zoning.grid_cell_size;
                        // Center of the 3-cell deep footprint (1.5 cells out from road edge)
                        let depth_offset = crate::config::SIDEWALK_WIDTH + (1.5 * zoning.grid_cell_size); 
                        let center_2d = world_pos_on_edge + normal * (edge_width * 0.5 + depth_offset);
                        
                        let frontage_pos_3d = godot::prelude::Vector3::new(world_pos_on_edge.x, 0.0, world_pos_on_edge.y);
                        let (frontage_node, new_edge_id, split_x) = network.split_for_frontage(edge_idx, frontage_pos_3d, zoning, self);
                        graph_changed = true;

                        let mut b_edge_idx = edge_idx;
                        let mut b_cell_x = x;
                        if x >= split_x {
                            b_edge_idx = new_edge_id;
                            b_cell_x = x - split_x;
                        }

                        // Center along road (1.5 cells in)
                        let center_adjustment = tangent * (1.5 * zoning.grid_cell_size);
                        let final_center_2d = center_2d + center_adjustment;

                        let b = Building {
                            center_x: final_center_2d.x,
                            center_y: final_center_2d.y,
                            width: b_width as u8,
                            depth: b_depth as u8,
                            zone_type: z_type,
                            facing_dir: -normal,
                            frontage_node,
                            side_offset: side as f32,
                            abandoned_timer: 0,
                            edge_idx: b_edge_idx,
                            side,
                            cell_x: b_cell_x,
                            cell_y: 0,
                        };
                        // Mark all 9 cells as occupied
                        for dx in 0..3 {
                            for dy in 0..3 {
                                zoning.set_occupied(b_edge_idx, side, b_cell_x + dx, dy, true);
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
            let dist = p2 - p1;
            let d = dist.length();
            if curr_dist + d >= target_dist {
                return if d > 1e-6 { dist.normalized() } else { Vector2::new(1.0, 0.0) };
            }
            curr_dist += d;
        }
        let p_end = Vector2::new(geo.last().unwrap().x, geo.last().unwrap().z);
        let p_prev = Vector2::new(geo[geo.len()-2].x, geo[geo.len()-2].z);
        let dist = p_end - p_prev;
        if dist.length() > 1e-6 { dist.normalized() } else { Vector2::new(1.0, 0.0) }
    }
}
