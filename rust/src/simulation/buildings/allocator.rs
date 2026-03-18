use crate::simulation::grid::data_grid::DataGrid;
use crate::simulation::grid::zoning::{ZoningSystem, ZoneType};
use crate::simulation::grid::desirability::DesirabilitySystem;
use crate::simulation::economy::demand::DemandSystem;
use crate::simulation::economy::agents::AgentSystem;
use rand::Rng;

pub struct Building {
    pub x: usize,
    pub y: usize,
    pub zone_type: ZoneType,
    pub rotation_seed: u32,
    pub road_node: u32, // The precise graph node this building is attached to!
    pub abandoned_timer: u8,
}

pub struct BuildingAllocator {
    pub buildings: Vec<Building>,
    pub occupancy_grid: DataGrid<bool>,
}

impl BuildingAllocator {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            buildings: Vec::new(),
            occupancy_grid: DataGrid::new(width, height, false),
        }
    }

    pub fn tick(&mut self, demand: &mut DemandSystem, zoning: &ZoningSystem, desirability: &DesirabilitySystem, noise: &crate::simulation::grid::noise::NoiseSystem, agents: &mut AgentSystem, graph: &crate::simulation::network::graph::TransitGraph) {
        let mut rng = rand::thread_rng();
        
        // 1. Despawn orphaned/unzoned buildings (Decay Timer)
        for b_id in 0..self.buildings.len() {
            if self.buildings[b_id].zone_type == ZoneType::None { continue; } // Already destroyed slot
            
            let b = &mut self.buildings[b_id];
            let current_zone = zoning.zones.get(b.x, b.y).unwrap_or(&ZoneType::None);
            
            if *current_zone != b.zone_type {
                // Determine a staggered threshold randomly off its permanent seed (5-20 days)
                let limit = 5 + (b.rotation_seed % 16) as u8; 
                b.abandoned_timer += 1;
                
                if b.abandoned_timer >= limit {
                    b.zone_type = ZoneType::None; // Render Invisible / Structurally Dead
                    self.occupancy_grid.set(b.x, b.y, false); // Free the physical lot for future spawns!
                    agents.evict_building(b_id); // Vaporize residents and workers
                }
            } else {
                b.abandoned_timer = 0; // Zone was restored, reset decay
            }
        }
        
        let max_spawns_per_tick = 5;
        let mut spawned = 0;

        // Try to spawn buildings by taking 100 random samples across the grid
        for _ in 0..100 {
            if spawned >= max_spawns_per_tick { break; }

            let x = rng.gen_range(0..self.occupancy_grid.width);
            let y = rng.gen_range(0..self.occupancy_grid.height);

            // Skip if already occupied
            if *self.occupancy_grid.get(x, y).unwrap_or(&true) { continue; }

            // Get Zoning
            let zone_type = match zoning.zones.get(x, y) {
                Some(z) => *z,
                None => continue,
            };

            if zone_type == ZoneType::None { continue; }

            // Get Desirability & Road Access
            let des = *desirability.grid.get(x, y).unwrap_or(&0.0);
            let nse = *noise.grid.get(x, y).unwrap_or(&0.0);
            let is_near_road = *zoning.validity_mask.get(x, y).unwrap_or(&false);
            
            if zone_type == ZoneType::Industrial {
                // Industrial buildings don't care about pollution/noise, 
                // they just need to be near a road.
                if !is_near_road { continue; }
            } else if zone_type == ZoneType::Commercial || zone_type == ZoneType::Mixed {
                // Commercial and Mixed use zones actually *thrive* on high traffic/noise.
                // We add the standard noise penalty (1.5x) back so it doesn't hurt them.
                let effective_des = des + (nse * 1.5);
                if effective_des < 40.0 { continue; }
            } else {
                // Residential strictly demands high Land Value
                if des < 40.0 { continue; } 
            }

            // Check Demand
            let mut can_spawn = false;
            let mut expected_residents = 0;

            match zone_type {
                ZoneType::Residential => {
                    if demand.residential >= 10.0 {
                        demand.residential -= 10.0;
                        can_spawn = true;
                        expected_residents = 5; // Spawn 5 citizens per house
                    }
                }
                ZoneType::Commercial => {
                    if demand.commercial >= 10.0 {
                        demand.commercial -= 10.0;
                        can_spawn = true;
                    }
                }
                ZoneType::Industrial => {
                    if demand.industrial >= 10.0 {
                        demand.industrial -= 10.0;
                        can_spawn = true;
                    }
                }
                ZoneType::Mixed => {
                    // Mixed consumes heavily from both
                    if demand.residential >= 5.0 || demand.commercial >= 5.0 {
                        if demand.residential >= 5.0 { demand.residential -= 5.0; }
                        if demand.commercial >= 5.0 { demand.commercial -= 5.0; }
                        can_spawn = true;
                        expected_residents = 8; // Mixed buildings hold more people (apartments)
                    }
                }
                ZoneType::None => {}
            }

            if can_spawn {
                self.occupancy_grid.set(x, y, true);
                
                let w = self.occupancy_grid.width as f32;
                let h = self.occupancy_grid.height as f32;
                let world_x = x as f32 - (w - 1.0) * 0.5;
                let world_z = y as f32 - (h - 1.0) * 0.5;
                let world_pos = godot::prelude::Vector3::new(world_x, 0.0, world_z);
                
                // Bind the building to the nearest physical road intersection
                let road_node = crate::simulation::network::interaction::get_closest_node(graph, world_pos, 40.0).unwrap_or(0);

                // Reuse dead slots natively!
                let mut slot_id = self.buildings.len();
                for (idx, b) in self.buildings.iter().enumerate() {
                    if b.zone_type == ZoneType::None {
                        slot_id = idx;
                        break;
                    }
                }
                
                let b_data = Building {
                    x,
                    y,
                    zone_type,
                    rotation_seed: rng.gen_range(0..360),
                    road_node,
                    abandoned_timer: 0,
                };

                if slot_id == self.buildings.len() {
                    self.buildings.push(b_data);
                } else {
                    self.buildings[slot_id] = b_data;
                }
                
                // Identify the absolute closest node to the border (Z=-127) for immigrant routing
                let highway_pos = godot::prelude::Vector3::new(0.0, 0.0, -127.0);
                let highway_node = crate::simulation::network::interaction::get_closest_node(graph, highway_pos, 1000.0).unwrap_or(road_node);
                let highway_world_pos = if highway_node != road_node {
                    graph.nodes[highway_node as usize].pos
                } else {
                    world_pos
                };
                
                // Construct physical agents driving in from the highway!
                for _ in 0..expected_residents {
                    agents.spawn_agent(slot_id, road_node, world_x, world_z, highway_node, highway_world_pos.x, highway_world_pos.z);
                }
                
                spawned += 1;
            }
        }
    }
}
