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
//! - Each placement triggers a full CCH rebuild via `split_for_frontage`.

use crate::simulation::network::graph::RegionGraph;
use crate::simulation::grid::zoning::{ZoningSystem, ZoneType};
use godot::prelude::Vector2;

/// A placed building occupying a 3 × 3 cell (30 m × 30 m) footprint on a zoning grid.
#[derive(Clone)]
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
    /// T-coordinate (0.0 to 1.0) along the road edge [`edge_idx`] for this building's frontage.
    pub frontage_t: f32,
    /// Signed side of the road: `+1.0` = left, `-1.0` = right (relative to edge direction).
    pub side_offset: f32,
    /// Ticks since this building lost its zoning. Non-zero values are reserved for future
    /// abandonment / decay logic; currently unused.
    pub abandoned_timer: u32,
    /// Index into [`RegionGraph::edges`] for the road segment this building fronts.
    pub edge_idx: usize,
    /// Road side: `1` = left, `-1` = right.
    pub side: i8,
    /// Column index (along the road) of the building's leading cell in the zoning grid.
    pub cell_x: usize,
    /// Row index (depth from road) of the building's leading cell; always `0` (first row).
    pub cell_y: usize,
    /// Number of agents currently living in this building.
    pub occupancy: u8,
}

/// Manages the full lifecycle of [`Building`]s: placement, removal, and immigrant spawning.
#[derive(Clone)]
pub struct BuildingAllocator {
    /// All currently placed buildings. Removal uses `swap_remove` — order is not preserved.
    pub buildings: Vec<Building>,
    /// Set to `true` when the building list changes in a tick, signalling renderers to refresh.
    pub dirty: bool,
    /// Inverted index: `zone_index[ZoneType as usize]` contains building indices (Bug B16).
    pub zone_index: [Vec<usize>; 6],
    /// Inverted index: `vacancy_index[ZoneType as usize]` contains indices of buildings with occupancy < 6 (Bug B16a).
    pub vacancy_index: [Vec<usize>; 6],
    /// Tracks the position of each building in its respective `vacancy_index` list for O(1) removal.
    /// Indexed by building ID; `usize::MAX` if not in any vacancy list.
    pub vacancy_pos: Vec<usize>,
    /// If true, the indices need to be recalculated.
    pub dirty_index: bool,
}

impl BuildingAllocator {
    /// Remaps all building edge indices after a road network compaction.
    pub fn update_edge_indices(&mut self, mapping: &std::collections::HashMap<usize, usize>) {
        let old_len = self.buildings.len();
        for b in &mut self.buildings {
            if let Some(&new_id) = mapping.get(&b.edge_idx) {
                b.edge_idx = new_id;
            } else {
                b.edge_idx = usize::MAX;
            }
        }
        self.buildings.retain(|b| b.edge_idx != usize::MAX);
        if self.buildings.len() != old_len {
             self.dirty = true;
             self.dirty_index = true;
        }
    }

    /// Creates an empty allocator.
    pub fn new() -> Self {
        Self {
            buildings: Vec::new(),
            dirty: false,
            zone_index: [const { Vec::new() }; 6],
            vacancy_index: [const { Vec::new() }; 6],
            vacancy_pos: Vec::new(),
            dirty_index: true,
        }
    }

    /// Removes all buildings and resets the dirty flag.
    pub fn clear(&mut self) {
        self.buildings.clear();
        for list in &mut self.zone_index {
            list.clear();
        }
        for list in &mut self.vacancy_index {
            list.clear();
        }
        self.vacancy_pos.clear();
        self.dirty = false;
        self.dirty_index = false;
    }

    /// Advances the building lifecycle by one simulation tick.
    ///
    /// Removes stale buildings, grows new ones into high-demand zones, and spawns immigrants.
    /// Calls `network.rebuild_pathing()` once if any building was added or removed.
    pub fn tick(&mut self, _demand: &mut crate::simulation::economy::demand::DemandSystem, zoning: &mut ZoningSystem, desirability: &crate::simulation::grid::desirability::DesirabilitySystem, _noise: &crate::simulation::grid::noise::NoiseSystem, _agents: &mut crate::simulation::economy::agents::AgentSystem, network: &mut crate::simulation::network::TransitNetwork, graph: &mut RegionGraph, config: &crate::simulation::core::config::MapConfig) {
        let mut spawned_this_tick = 0;
        let max_spawns = 10;
        
        // 1. Cleanup: Remove buildings if their cells are no longer zoned correctly OR if the edge they're on is gone.
        let mut i = 0;
        while i < self.buildings.len() {
            let b = &self.buildings[i];
            let remove = if let Some(_) = zoning.edge_grids.get(&b.edge_idx) {
                let current_type = zoning.get_cell(b.edge_idx, b.side, b.cell_x, b.cell_y);
                current_type != b.zone_type || graph.edges[b.edge_idx].deleted
            } else {
                true // Edge gone
            };

            if remove {
                let b_edge_idx = b.edge_idx;
                let b_side = b.side;
                let b_cell_x = b.cell_x;
                let b_cell_y = b.cell_y;
                let b_width = b.width;
                let b_depth = b.depth;

                // network.remove_frontage(frontage_node, zoning, self); // DELETED: Virtual Frontages
                // Clear occupancy for the entire footprint
                let w_cells = (b_width as f32 / zoning.config.zone_cell_m).round() as usize;
                let d_cells = (b_depth as f32 / zoning.config.zone_cell_m).round() as usize;
                for dx in 0..w_cells {
                    for dy in 0..d_cells {
                        zoning.set_occupied(b_edge_idx, b_side, b_cell_x + dx, b_cell_y + dy, false);
                    }
                }
                let last_idx = self.buildings.len() - 1;
                if i < last_idx {
                    let mut mapping = std::collections::HashMap::new();
                    mapping.insert(last_idx, i);
                    _agents.remap_building_indices(&mapping);
                }
                
                self.buildings.swap_remove(i);
                self.dirty_index = true;
            } else {
                i += 1;
            }
        }

        // 2. Growth Logic: Find areas with un-occupied zones and high demand
        let edges_to_check: Vec<usize> = zoning.edge_grids.keys().cloned().collect();
        
        for edge_idx in edges_to_check {
            if spawned_this_tick >= max_spawns { break; }
            let (edge_len, edge_width) = if let Some(edge) = graph.edges.get(edge_idx) {
                if edge.deleted || edge.physical_geometry.len() < 2 { (0.0, 0.0) }
                else { (edge.physical_length, edge.width) }
            } else {
                (0.0, 0.0)
            };
            
            if edge_len < 0.1 { continue; }
            
            // Batch fetch nearby edges for the entire edge to optimize obstruction checks
            let padding = 120.0;
            let edge = &graph.edges[edge_idx];
            let mut min_x = f32::MAX; let mut max_x = f32::MIN;
            let mut min_z = f32::MAX; let mut max_z = f32::MIN;
            for p in &edge.physical_geometry {
                min_x = min_x.min(p.x); max_x = max_x.max(p.x);
                min_z = min_z.min(p.z); max_z = max_z.max(p.z);
            }
            let nearby_edges = graph.get_edges_near_aabb(
                godot::prelude::Vector3::new(min_x - padding, 0.0, min_z - padding),
                godot::prelude::Vector3::new(max_x + padding, 0.0, max_z + padding)
            );

            for side in [1, -1] {
                let cells_long = if let Some(g) = zoning.edge_grids.get(&edge_idx) { g.cells_long } else { 0 };
                if cells_long < 3 { continue; }

                // Node-Proximal Spawning: iterate from both ends towards the middle
                let mid = cells_long / 2;
                let mut x_order: Vec<usize> = Vec::with_capacity(cells_long);
                
                // Build x_order: [0, max-1, 1, max-2, ...]
                for i in 0..mid {
                    x_order.push(i);
                    x_order.push(cells_long.saturating_sub(3).saturating_sub(i));
                }
                if cells_long % 2 != 0 {
                    x_order.push(mid);
                }
                // Filter and deduplicate (saturating_sub might produce duplicates)
                let mut seen = std::collections::HashSet::new();
                let x_order: Vec<usize> = x_order.into_iter()
                    .filter(|&x| x <= cells_long.saturating_sub(3))
                    .filter(|&x| seen.insert(x))
                    .collect();

                for x in x_order {
                    if spawned_this_tick >= max_spawns { break; }
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
                               zoning.is_cell_obstructed(edge_idx, side, x + dx, dy, &graph, Some(&nearby_edges)) {
                                can_build = false;
                                break;
                            }
                        }
                        if !can_build { break; }
                    }

                    // B5: Desirability Gate
                    if can_build {
                        let t_center = (x as f32 + 1.5) * zoning.config.zone_cell_m / edge_len;
                        let world_pos = self.get_pos_on_edge(&graph, edge_idx, t_center);
                        let tangent = self.get_tangent_on_edge(&graph, edge_idx, t_center);
                        let normal = godot::prelude::Vector2::new(tangent.y, -tangent.x) * (side as f32);
                        let depth_offset = crate::config::SIDEWALK_WIDTH + (1.5 * zoning.config.zone_cell_m);
                        let center_2d = world_pos + normal * (edge_width * 0.5 + depth_offset);

                        // Map world to grid coordinates
                        let world_size_x = config.width_m;
                        let world_size_y = config.height_m;
                        let gx = (((center_2d.x / world_size_x) + 0.5) * desirability.grid.width as f32).round() as usize;
                        let gy = (((center_2d.y / world_size_y) + 0.5) * desirability.grid.height as f32).round() as usize;
                        let gx = gx.min(desirability.grid.width.saturating_sub(1));
                        let gy = gy.min(desirability.grid.height.saturating_sub(1));

                        let val = *desirability.grid.get(gx, gy).unwrap_or(&50.0);
                        if val < 20.0 {
                            can_build = false;
                        }
                    }

                    if can_build {
                        let t = (x as f32) * zoning.config.zone_cell_m / edge_len;
                        let world_pos_on_edge = self.get_pos_on_edge(&graph, edge_idx, t);
                        let tangent = self.get_tangent_on_edge(&graph, edge_idx, t);
                        let normal = godot::prelude::Vector2::new(tangent.y, -tangent.x) * (side as f32);
                        
                        let b_width = 3.0 * zoning.config.zone_cell_m;
                        let b_depth = 3.0 * zoning.config.zone_cell_m;
                        // Center of the 3-cell deep footprint (1.5 cells out from road edge)
                        let depth_offset = crate::config::SIDEWALK_WIDTH + (1.5 * zoning.config.zone_cell_m); 
                        let center_2d = world_pos_on_edge + normal * (edge_width * 0.5 + depth_offset);
                        
                        let frontage_t = (x as f32 + 1.5) * zoning.config.zone_cell_m / edge_len;
                        // let (frontage_node, new_edge_id, split_x) = network.split_for_frontage(edge_idx, frontage_pos_3d, zoning, self); // DELETED: Virtual Frontages
                        spawned_this_tick += 1;

                        let b_edge_idx = edge_idx;
                        let b_cell_x = x;
                        /* // DELETED: Virtual Frontages
                        if x >= split_x {
                            b_edge_idx = new_edge_id;
                            b_cell_x = x - split_x;
                        }
                        */

                        // Center along road (1.5 cells in)
                        let center_adjustment = tangent * (1.5 * zoning.config.zone_cell_m);
                        let final_center_2d = center_2d + center_adjustment;

                        let b = Building {
                            center_x: final_center_2d.x,
                            center_y: final_center_2d.y,
                            width: b_width as u8,
                            depth: b_depth as u8,
                            zone_type: z_type,
                            facing_dir: -normal,
                            frontage_t,
                            side_offset: side as f32,
                            abandoned_timer: 0,
                            edge_idx: b_edge_idx,
                            side,
                            cell_x: b_cell_x,
                            cell_y: 0,
                            occupancy: 0,
                        };
                        // Mark all 9 cells as occupied
                        for dx in 0..3 {
                            for dy in 0..3 {
                                zoning.set_occupied(b_edge_idx, side, b_cell_x + dx, dy, true);
                            }
                        }
                        self.buildings.push(b);
                        self.dirty_index = true;
                        
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

        network.rebuild_pathing_if_dirty(graph);
        
        if self.dirty_index {
            self.rebuild_zone_index();
        }

        // 3. Immigration Logic
        let total_capacity: usize = self.buildings.iter()
            .filter(|b| b.zone_type == ZoneType::Residential || b.zone_type == ZoneType::Mixed)
            .fold(0, |acc, b| acc + (6 - b.occupancy as usize));
            
        if _agents.count < total_capacity {
            let demand_factor = (_demand.residential / 100.0).max(0.0).min(1.0);
            let gap = total_capacity - _agents.count;
            let num_to_spawn = ((gap as f32 * 0.2 * demand_factor) as usize).max(1).min(10); 
            
            for _ in 0..num_to_spawn {
                let highway_pos = godot::prelude::Vector3::new(0.0, 0.0, -127.0);
                if let Some(highway_node) = crate::simulation::network::interaction::get_closest_node(graph, highway_pos, 1000.0) {
                    let highway_world_pos = graph.nodes[highway_node as usize].pos;
                    _agents.spawn_agent(usize::MAX, highway_node, 0.0, 0.0, highway_node, highway_world_pos.x, highway_world_pos.z);
                }
            }
        }
        
        self.dirty = false;
    }

    pub fn get_pos_on_edge(&self, graph: &RegionGraph, edge_idx: usize, t: f32) -> Vector2 {
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

    pub fn get_tangent_on_edge(&self, graph: &RegionGraph, edge_idx: usize, t: f32) -> Vector2 {
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

    /// Repopulates the internal zone and vacancy indices (Bug B16/B16a fix).
    pub fn rebuild_zone_index(&mut self) {
        for list in &mut self.zone_index {
            list.clear();
        }
        for list in &mut self.vacancy_index {
            list.clear();
        }
        self.vacancy_pos.clear();
        self.vacancy_pos.resize(self.buildings.len(), usize::MAX);

        for (idx, b) in self.buildings.iter().enumerate() {
            let zi = b.zone_type as usize;
            if zi < 6 {
                self.zone_index[zi].push(idx);
                if b.occupancy < 6 {
                    let v_idx = self.vacancy_index[zi].len();
                    self.vacancy_index[zi].push(idx);
                    self.vacancy_pos[idx] = v_idx;
                }
            }
        }
        self.dirty_index = false;
    }

    /// Increments occupancy for a building and updates vacancy index if it becomes full. O(1).
    pub fn claim_vacancy(&mut self, building_idx: usize) {
        if building_idx >= self.buildings.len() { return; }
        let b = &mut self.buildings[building_idx];
        b.occupancy += 1;
        
        // If it was in the vacancy list and is now full, remove it
        if b.occupancy == 6 {
            let zi = b.zone_type as usize;
            let v_pos = self.vacancy_pos[building_idx];
            if v_pos != usize::MAX {
                let list = &mut self.vacancy_index[zi];
                let last_b_idx = *list.last().unwrap();
                list.swap_remove(v_pos);
                self.vacancy_pos[last_b_idx] = v_pos;
                self.vacancy_pos[building_idx] = usize::MAX;
            }
        }
    }

    /// Decrements occupancy for a building and updates vacancy index if it gained space. O(1).
    pub fn release_vacancy(&mut self, building_idx: usize) {
        if building_idx >= self.buildings.len() { return; }
        let b = &mut self.buildings[building_idx];
        b.occupancy = b.occupancy.saturating_sub(1);
        
        // If it was full and now has space, add it back to vacancy index
        if b.occupancy == 5 {
            let zi = b.zone_type as usize;
            if self.vacancy_pos[building_idx] == usize::MAX {
                let v_idx = self.vacancy_index[zi].len();
                self.vacancy_index[zi].push(building_idx);
                self.vacancy_pos[building_idx] = v_idx;
            }
        }
    }

    /// Pick a random building from a specific zone type. O(1).
    pub fn get_random_building_by_zone(&self, zone: ZoneType, rng: &mut impl rand::Rng) -> Option<usize> {
        let list = &self.zone_index[zone as usize];
        if list.is_empty() { return None; }
        Some(list[rng.gen_range(0..list.len())])
    }

    /// Pick a random building from any of the specified zone types. O(1).
    pub fn get_random_building_by_zones(&self, zones: &[ZoneType], rng: &mut impl rand::Rng) -> Option<usize> {
        // We sum the counts and pick based on weighted probability of lengths
        let mut total = 0;
        for &zone in zones {
            total += self.zone_index[zone as usize].len();
        }
        if total == 0 { return None; }

        let mut pick = rng.gen_range(0..total);
        for &zone in zones {
            let list = &self.zone_index[zone as usize];
            if pick < list.len() {
                return Some(list[pick]);
            }
            pick -= list.len();
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::grid::zoning::ZoneType;
    use crate::simulation::economy::agents::AgentSystem;
    use crate::simulation::core::config::MapConfig;
    use godot::prelude::Vector2;
    use rand::SeedableRng;

    #[test]
    fn test_zone_index_consistency() {
        let mut allocator = BuildingAllocator::new();
        let mut rng = rand::rngs::StdRng::seed_from_u64(42);

        // 1. Add buildings
        for i in 0..10 {
            allocator.buildings.push(Building {
                center_x: i as f32,
                center_y: 0.0,
                width: 30,
                depth: 30,
                zone_type: if i % 2 == 0 { ZoneType::Residential } else { ZoneType::Commercial },
                facing_dir: Vector2::new(0.0, 1.0),
                frontage_t: 0.5,
                side_offset: 0.0,
                abandoned_timer: 0,
                edge_idx: 0,
                side: 1,
                cell_x: i,
                cell_y: 0,
                occupancy: 0,
            });
        }
        allocator.dirty_index = true;
        allocator.rebuild_zone_index();

        assert_eq!(allocator.zone_index[ZoneType::Residential as usize].len(), 5);
        assert_eq!(allocator.zone_index[ZoneType::Commercial as usize].len(), 5);

        // 2. Remove a building (Residential at index 0)
        allocator.buildings.swap_remove(0);
        allocator.dirty_index = true;
        allocator.rebuild_zone_index();

        assert_eq!(allocator.buildings.len(), 9);
        assert_eq!(allocator.zone_index[ZoneType::Residential as usize].len(), 4);
        assert_eq!(allocator.zone_index[ZoneType::Commercial as usize].len(), 5);

        // 3. Random selection
        let pick = allocator.get_random_building_by_zone(ZoneType::Commercial, &mut rng);
        assert!(pick.is_some());
        assert_eq!(allocator.buildings[pick.unwrap()].zone_type, ZoneType::Commercial);
    }

    #[test]
    fn test_vacancy_index_consistency() {
        let mut allocator = BuildingAllocator::new();
        let mut rng = rand::rngs::StdRng::seed_from_u64(42);

        // 1. Add 5 Residential buildings
        for i in 0..5 {
            allocator.buildings.push(Building {
                center_x: i as f32, center_y: 0.0, width: 30, depth: 30,
                zone_type: ZoneType::Residential, facing_dir: Vector2::new(0.0, 1.0),
                frontage_t: 0.5, side_offset: 0.0, abandoned_timer: 0,
                edge_idx: 0, side: 1, cell_x: i, cell_y: 0, occupancy: 0,
            });
        }
        allocator.rebuild_zone_index();

        assert_eq!(allocator.vacancy_index[ZoneType::Residential as usize].len(), 5);

        // 2. Fill one building to capacity
        allocator.claim_vacancy(0); // 1
        allocator.claim_vacancy(0); // 2
        allocator.claim_vacancy(0); // 3
        allocator.claim_vacancy(0); // 4
        allocator.claim_vacancy(0); // 5
        assert_eq!(allocator.vacancy_index[ZoneType::Residential as usize].len(), 5);
        allocator.claim_vacancy(0); // 6 (Full)
        
        assert_eq!(allocator.vacancy_index[ZoneType::Residential as usize].len(), 4);
        assert!(!allocator.vacancy_index[ZoneType::Residential as usize].contains(&0));

        // 3. Release one spot
        allocator.release_vacancy(0); // 5
        assert_eq!(allocator.vacancy_index[ZoneType::Residential as usize].len(), 5);
        assert!(allocator.vacancy_index[ZoneType::Residential as usize].contains(&0));

        // 4. Test swap_remove integrity in BuildingAllocator::tick
        // We'll manually simulate the swap logic in tick:
        // Remove building at index 1, building 4 moves to index 1.
        let mut agents = AgentSystem::new();
        // Setup agents to represent occupancy
        for _ in 0..5 { agents.spawn_agent(usize::MAX, 0, 0.0, 0.0, 0, 0.0, 0.0); }
        
        let last_idx = allocator.buildings.len() - 1; // 4
        let i = 1;
        let mut mapping = std::collections::HashMap::new();
        mapping.insert(last_idx, i);
        agents.remap_building_indices(&mapping); // Not strictly needed for allocator test but good practice
        
        allocator.buildings.swap_remove(i);
        allocator.rebuild_zone_index(); // Rebuild after swap

        assert_eq!(allocator.buildings.len(), 4);
        assert_eq!(allocator.zone_index[ZoneType::Residential as usize].len(), 4);
        assert_eq!(allocator.vacancy_index[ZoneType::Residential as usize].len(), 4);
    }

    #[test]
    fn test_building_placement_demand_subtraction() {
        use crate::simulation::economy::demand::DemandSystem;
        use crate::simulation::grid::desirability::DesirabilitySystem;
        use crate::simulation::grid::noise::NoiseSystem;
        use crate::simulation::grid::pollution::PollutionSystem;
        use crate::simulation::network::TransitNetwork;
        use crate::simulation::grid::zoning::ZoningSystem;
        use godot::prelude::Vector3;

        let mut allocator = BuildingAllocator::new();
        let mut demand = DemandSystem::new();
        demand.residential = 100.0;
        
        let map_cfg = MapConfig::default();
        let mut zoning = ZoningSystem::new(&map_cfg);
        let mut desirability = DesirabilitySystem::new(&map_cfg);
        let (env_w, env_h) = map_cfg.get_env_grid_size();
        for x in 0..env_w { for y in 0..env_h { desirability.grid.set(x, y, 100.0); } } // High desirability
        
        let noise = NoiseSystem::new(&map_cfg);
        let mut agents = AgentSystem::new();
        let mut network = TransitNetwork::new();
        let mut graph = RegionGraph::new();
        
        network.add_road(&mut graph, vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)], 1, 1, true, false, crate::simulation::network::types::EdgeClass::Standard, &mut zoning, &mut allocator);
        for x in 0..3 { for y in 0..3 { zoning.set_cell(0, 1, x, y, ZoneType::Residential); } }
        zoning.recalculate_obstructions(0, &graph);

        allocator.tick(&mut demand, &mut zoning, &desirability, &noise, &mut agents, &mut network, &mut graph, &map_cfg);
        
        assert_eq!(allocator.buildings.len(), 1);
        assert_eq!(demand.residential, 95.0, "Residential demand should decrease by 5.0 after placement");
    }

    #[test]
    fn test_building_placement_desirability_gate() {
        use crate::simulation::economy::demand::DemandSystem;
        use crate::simulation::grid::desirability::DesirabilitySystem;
        use crate::simulation::grid::noise::NoiseSystem;
        use crate::simulation::grid::pollution::PollutionSystem;
        use crate::simulation::network::TransitNetwork;
        use crate::simulation::grid::zoning::ZoningSystem;
        use godot::prelude::Vector3;

        let mut allocator = BuildingAllocator::new();
        let mut demand = DemandSystem::new();
        demand.residential = 14.0; // Enough for placement (>10.0)
        
        let map_cfg = MapConfig::default();
        let mut zoning = ZoningSystem::new(&map_cfg);
        let mut desirability = DesirabilitySystem::new(&map_cfg);
        let (env_w, env_h) = map_cfg.get_env_grid_size();
        for x in 0..env_w { for y in 0..env_h { desirability.grid.set(x, y, 10.0); } } // Below gate threshold (< 20.0)
        
        let mut noise = NoiseSystem::new(&map_cfg);
        let mut agents = AgentSystem::new();
        let mut network = TransitNetwork::new();
        let mut graph = RegionGraph::new();
        
        network.add_road(&mut graph, vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)], 1, 1, true, false, crate::simulation::network::types::EdgeClass::Standard, &mut zoning, &mut allocator);
        for x in 0..3 { for y in 0..3 { zoning.set_cell(0, 1, x, y, ZoneType::Residential); } }
        zoning.recalculate_obstructions(0, &graph);

        allocator.tick(&mut demand, &mut zoning, &desirability, &noise, &mut agents, &mut network, &mut graph, &map_cfg);
        
        assert_eq!(allocator.buildings.len(), 0, "No building should spawn when desirability is below 20.0");
    }

    #[test]
    fn test_building_removal_clears_zoning_occupancy() {
        use crate::simulation::economy::demand::DemandSystem;
        use crate::simulation::grid::desirability::DesirabilitySystem;
        use crate::simulation::grid::noise::NoiseSystem;
        use crate::simulation::grid::pollution::PollutionSystem;
        use crate::simulation::network::TransitNetwork;
        use crate::simulation::grid::zoning::ZoningSystem;
        use godot::prelude::Vector3;

        let mut allocator = BuildingAllocator::new();
        let mut demand = DemandSystem::new();
        let map_cfg = MapConfig::default();
        let mut zoning = ZoningSystem::new(&map_cfg);
        let desirability = DesirabilitySystem::new(&map_cfg);
        let noise = NoiseSystem::new(&map_cfg);
        let mut agents = AgentSystem::new();
        let mut network = TransitNetwork::new();
        let mut graph = RegionGraph::new();
        
        network.add_road(&mut graph, vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)], 1, 1, true, false, crate::simulation::network::types::EdgeClass::Standard, &mut zoning, &mut allocator);
        
        allocator.buildings.push(Building {
            center_x: 5.0, center_y: 10.0, width: 30, depth: 30,
            zone_type: ZoneType::Residential, facing_dir: Vector2::new(0.0, 1.0),
            frontage_t: 0.05, side_offset: 1.0, abandoned_timer: 0, edge_idx: 0, side: 1, cell_x: 0, cell_y: 0, occupancy: 0,
        });
        for dx in 0..3 { for dy in 0..3 { zoning.set_occupied(0, 1, dx, dy, true); } }
        
        // Remove zoning trigger
        for dx in 0..3 { for dy in 0..3 { zoning.set_cell(0, 1, dx, dy, ZoneType::None); } }

        allocator.tick(&mut demand, &mut zoning, &desirability, &noise, &mut agents, &mut network, &mut graph, &map_cfg);
        
        assert_eq!(allocator.buildings.len(), 0, "Building should have been removed");
        assert!(!zoning.is_occupied(0, 1, 0, 0), "Zoning cell occupancy should be cleared after building removal");
    }
}
