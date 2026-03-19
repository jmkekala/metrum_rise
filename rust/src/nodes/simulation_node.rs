use godot::prelude::*;
use godot::classes::{Node3D, INode3D};
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
use crate::config;

pub struct SimulationSnapshot {
    pub terrain: Option<Vec<f32>>,
    pub water: Option<Vec<f32>>,
    pub transit: Option<crate::simulation::network::graph::TransitGraph>,
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

    fn push_undo_state(&mut self, inc_terrain: bool, inc_water: bool, inc_transit: bool) {
        if self.undo_stack.len() >= 30 {
            self.undo_stack.remove(0); // Constant 30-size rolling window
        }
        self.undo_stack.push(SimulationSnapshot {
            terrain: if inc_terrain { Some(self.heightmap.data.clone()) } else { None },
            water: if inc_water { Some(self.watermap.depth.clone()) } else { None },
            transit: if inc_transit { Some(self.transit_network.graph.clone()) } else { None },
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

            // Fire cascading sync pipelines to ensure GPU components mirror reverted states
            if sync_transit {
                self.transit_network.hpa_graph = crate::simulation::pathing::hpa::HpaGraph::build(&self.transit_network.graph);
            }
            return true;
        }
        false
    }

    #[func]
    pub fn sculpt_terrain(&mut self, pos: Vector2, radius: f32, strength: f32) {
        self.push_undo_state(true, false, true); // Sculpt triggers transit geometry re-flow
        self.heightmap.sculpt(pos.x, pos.y, radius, strength);
        
        // STICKY ROADS: Sync network and re-flatten
        self.transit_network.sync_to_terrain(&self.heightmap);
        self.flatten_terrain_for_roads();
    }

    #[func]
    pub fn add_water(&mut self, pos: Vector2, amount: f32) {
        self.push_undo_state(false, true, false);
        self.watermap.add_water(pos.x as usize, pos.y as usize, amount);
    }

    #[func]
    pub fn add_water_source(&mut self, pos: Vector2, rate_add: f32) {
        self.watermap.update_source(pos.x as usize, pos.y as usize, rate_add);
    }

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
    pub fn paint_zone(&mut self, pos: Vector2, radius: f32, zone_type_int: u8) {
        let zone_type = match zone_type_int {
            1 => ZoneType::Residential,
            2 => ZoneType::Commercial,
            3 => ZoneType::Industrial,
            4 => ZoneType::Mixed,
            _ => ZoneType::None,
        };
        
        self.zoning.paint_zone(pos.x, pos.y, radius, zone_type);
    }

    #[func]
    pub fn get_zoning_image_data(&self) -> PackedByteArray {
        self.zoning.generate_image_data()
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
        self.noise.tick(&self.allocator, &self.transit_network.graph);
        
        // 2. Desirability Update
        self.desirability.tick(&self.zoning, &self.pollution, &self.noise);

        // 3. Economy & Building Allocation
        self.demand.tick();
        self.allocator.tick(&mut self.demand, &self.zoning, &self.desirability, &self.noise, &mut self.agents, &self.transit_network.graph);
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

            let scale = 1.0; 

            buffer.push(scale); buffer.push(0.0); buffer.push(0.0); buffer.push(world_x);
            buffer.push(0.0); buffer.push(scale); buffer.push(0.0); buffer.push(world_y);
            buffer.push(0.0); buffer.push(0.0); buffer.push(scale); buffer.push(world_z);
        }

        PackedFloat32Array::from_iter(buffer)
    }

    #[func]
    pub fn get_agent_paths_debug(&self) -> PackedVector3Array {
        let mut lines = Vec::new(); // Vec of points, 2 points per line segment
        for i in 0..self.agents.count {
            if self.agents.transit[i] != 0 {
                let mut curr = self.agents.current_node[i];
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
                
                if let Some(path) = self.transit_network.hpa_graph.find_path(curr, target, usize::MAX, &self.transit_network.graph) {
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
            4 => ZoneType::Mixed,
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
                let world_x = b.x as f32 - hw;
                let world_z = b.y as f32 - hh;
                
                // Get terrain height at this exact point (assuming 20.0 terrain scale modifier)
                let world_y = self.heightmap.get_height(b.x, b.y) * 20.0;

                // For rotation, we can construct 3 basis vectors (Euler Y rotation)
                let rad = (b.rotation_seed as f32).to_radians();
                let cos_r = rad.cos();
                let sin_r = rad.sin();

                // Godot MultiMesh Float Buffer Format (12 floats per transform, transposed)
                // Basis X axis: (cos, 0, -sin)
                buffer.push(cos_r);
                buffer.push(0.0);
                buffer.push(-sin_r);
                buffer.push(world_x);

                // Basis Y axis: (0, 1, 0)
                buffer.push(0.0);
                buffer.push(1.0);
                buffer.push(0.0);
                buffer.push(world_y);

                // Basis Z axis: (sin, 0, cos)
                buffer.push(sin_r);
                buffer.push(0.0);
                buffer.push(cos_r);
                buffer.push(world_z);
            }
        }

        PackedFloat32Array::from_iter(buffer)
    }

    #[func]
    pub fn add_road(&mut self, points: PackedVector3Array, fwd_lanes: i32, bkw_lanes: i32) {
        self.push_undo_state(false, false, true);
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

        self.transit_network.add_road(fixed_points, fwd_lanes as u8, bkw_lanes as u8);
        self.zoning.update_validity_mask(&self.transit_network.graph, 40.0);

        let nodes = self.transit_network.graph.nodes.len();
        let edges = self.transit_network.graph.edges.len();
        let islands = self.transit_network.graph.get_island_count();
        godot_print!("Road added. Total: {} Nodes, {} Edges. Separate Networks: {}", nodes, edges, islands);
    }

    #[func]
    pub fn get_road_mesh_data(&self) -> VarDictionary {
        let (verts, norms, uvs, colors) = self.transit_network.generate_mesh_data(&self.heightmap);
        let mut dict = VarDictionary::new();
        dict.set("vertices", verts);
        dict.set("normals", norms);
        dict.set("uvs", uvs);
        dict.set("colors", colors);
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
    pub fn set_lane_connection(&mut self, node_id: u32, from_edge: i32, from_lane: i32, to_edge: i32, to_lane: i32) {
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
        if let Some(node) = self.transit_network.graph.nodes.get_mut(node_id as usize) {
            node.lane_connections.clear();
        }
        self.transit_network.hpa_graph = crate::simulation::pathing::hpa::HpaGraph::build(&self.transit_network.graph);
    }

    #[func]
    pub fn get_node_lanes(&self, node_id: u32) -> VarArray {
        let mut arr = VarArray::new();
        if node_id as usize >= self.transit_network.graph.nodes.len() { return arr; }
        
        let node_pos = self.transit_network.graph.nodes[node_id as usize].pos;
        
        for (e_id, edge) in self.transit_network.graph.edges.iter().enumerate() {
            let is_start = edge.start_node == node_id;
            let is_end = edge.end_node == node_id;
            
            if !is_start && !is_end { continue; }
            if edge.physical_geometry.len() < 2 { continue; }
            
            let tangent = if is_start {
                (edge.physical_geometry[1] - edge.physical_geometry[0]).normalized()
            } else {
                let last = edge.physical_geometry.len() - 1;
                (edge.physical_geometry[last] - edge.physical_geometry[last - 1]).normalized()
            };

            // Use the physical exterior edge to set the node position instead of the center!
            let current_pos = if is_start { edge.physical_geometry[0] } else { *edge.physical_geometry.last().unwrap() };
            
            let normal = Vector3::new(-tangent.z, 0.0, tangent.x);
            let fwd_lanes = edge.fwd_lanes;
            let bkw_lanes = edge.bkw_lanes;
            
            for lane in 0..fwd_lanes {
                let offset = (lane as f32 + 0.5) * 3.0;
                let mut lane_pos = current_pos + normal * offset;
                lane_pos.y += 0.5; // Lift up slightly for visual clicking
                
                let mut dict = VarDictionary::new();
                dict.set("edge_id", e_id as i32);
                dict.set("lane_id", lane as i32);
                dict.set("is_incoming", is_end);
                dict.set("pos", lane_pos);
                arr.push(&dict.to_variant());
            }
            
            for lane in 0..bkw_lanes {
                let lane_idx = -(lane as i32) - 1;
                let offset = (lane_idx as f32 + 0.5) * 3.0;
                let mut lane_pos = current_pos + normal * offset;
                lane_pos.y += 0.5;
                
                let mut dict = VarDictionary::new();
                dict.set("edge_id", e_id as i32);
                dict.set("lane_id", lane_idx);
                dict.set("is_incoming", is_start);
                dict.set("pos", lane_pos);
                arr.push(&dict.to_variant());
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
        } else {
            godot_error!("Invalid heightmap data size for import!");
        }
    }
}

#[godot_api]
impl INode3D for SimulationNode {
    fn init(base: Base<Node3D>) -> Self {
        godot_print!("Simulation Engine Initialized (Multi-Modal Network)");
        let w = config::MAP_WIDTH;
        let h = config::MAP_HEIGHT;
        let mut sim = Self { 
            base,
            time: TimeSystem::new(),
            time_passed: 0.0,
            heightmap: TerrainSystem::new(w, h),
            watermap: WaterSystem::new(w, h),
            transit_network: TransitNetwork::new(),
            zoning: ZoningSystem::new(w, h),
            pollution: PollutionSystem::new(w, h),
            noise: NoiseSystem::new(w, h),
            desirability: DesirabilitySystem::new(w, h),
            demand: DemandSystem::new(),
            allocator: BuildingAllocator::new(w, h),
            agents: AgentSystem::new(),
            undo_stack: Vec::new(),
        };

        // Inject starter highway to border (Z = -127) for the player to connect to
        let mut pts = PackedVector3Array::new();
        pts.push(Vector3::new(0.0, 0.0, -127.0));
        pts.push(Vector3::new(0.0, 0.0, -60.0));
        sim.add_road(pts, 2, 2);

        sim
    }

    fn process(&mut self, delta: f64) {
        self.time_passed += delta;
        
        if self.time.process_delta(delta) {
            self.simulate_tick();
        }
        
        // Let's keep water ticking fast, unrelated to the slow "Game Day" clock.
        let sub_steps = 2;
        let sub_dt = delta as f32 / sub_steps as f32;
        for _ in 0..sub_steps {
            self.watermap.tick(&self.heightmap.data, sub_dt);
        }

        // High-frequency agent physics!
        if self.time.speed_multiplier > 0.0 {
            let dt = (delta * self.time.speed_multiplier as f64) as f32;
            self.agents.tick(&self.allocator, &self.transit_network.hpa_graph, &self.transit_network.graph, dt);
        }
    }
}
