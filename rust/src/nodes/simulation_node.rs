//! Main GDExtension node for the Metrum Rise simulation.
//! 
//! This node acts as the central hub for all simulation systems, bridging 
//! the Rust simulation backend with the Godot engine frontend.

use godot::prelude::*;
use godot::classes::{Node3D, INode3D, MultiMesh};
use std::collections::VecDeque;

use crate::simulation::terrain::TerrainSystem;
use crate::simulation::water::WaterSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::grid::zoning::{ZoningSystem};
use crate::simulation::grid::pollution::PollutionSystem;
use crate::simulation::grid::noise::NoiseSystem;
use crate::simulation::grid::desirability::DesirabilitySystem;
use crate::simulation::core::config::MapConfig;
use crate::simulation::core::time::TimeSystem;
use crate::simulation::economy::demand::DemandSystem;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::AgentSystem;
use crate::config;

// Removed sim mod; now in nodes/mod.rs

/// A snapshot of the simulation state for undo/redo purposes.
pub struct SimulationSnapshot {
    /// Terrain heightmap data.
    pub terrain: Option<Vec<f32>>,
    /// Water depth data.
    pub water: Option<Vec<f32>>,
    /// Road network graph state.
    pub trans_graph: Option<crate::simulation::network::graph::RegionGraph>,
    /// Zoning system state.
    pub zoning: Option<crate::simulation::grid::zoning::ZoningSystem>,
}

#[derive(GodotClass)]
#[class(base=Node3D)]
/// The central simulation node exposed to Godot.
pub struct SimulationNode {
    pub(crate) time: TimeSystem,
    pub(crate) time_passed: f64,
    pub(crate) heightmap: TerrainSystem,
    pub(crate) watermap: WaterSystem,
    pub(crate) region_graph: crate::simulation::network::graph::RegionGraph,
    pub(crate) transit_network: TransitNetwork,
    pub(crate) zoning: ZoningSystem,
    pub(crate) pollution: PollutionSystem,
    pub(crate) noise: NoiseSystem,
    pub(crate) desirability: DesirabilitySystem,
    pub(crate) demand: DemandSystem,
    pub(crate) allocator: BuildingAllocator,
    pub(crate) agents: AgentSystem,
    pub(crate) undo_stack: VecDeque<SimulationSnapshot>,
    pub(crate) last_tick_duration: f64,
    pub(crate) benchmark_mode: bool,
    pub(crate) terrain_dirty: bool,
    pub(crate) water_dirty: bool,
    pub(crate) config: MapConfig,
    base: Base<Node3D>,
}

impl SimulationNode {
    /// Executes a single simulation tick.
    pub fn simulate_tick(&mut self) {
        godot_print!("Tick! Day {}", self.time.current_day);
        
        // 1. Environmental Spread
        let tick_start = std::time::Instant::now();
        
        // ECONOMY: Demand update
        self.demand.tick();
        
        // ZONING: Growth & Immigration
        self.allocator.tick(&mut self.demand, &mut self.zoning, &self.desirability, &self.noise, &mut self.agents, &mut self.transit_network, &mut self.region_graph, &self.config);
        
        // POLLUTION & NOISE: Dissipation
        self.pollution.tick(&self.allocator, &self.config);
        self.noise.tick(&self.allocator, &self.region_graph, &self.config);
        self.desirability.tick(&self.zoning, &self.pollution, &self.noise);

        // AGENTS: Daily update (happiness, money, pollution)
        self.agents.daily_update(&self.pollution, &self.config);

        self.agents.pathfind_count = 0;
        self.last_tick_duration = tick_start.elapsed().as_secs_f64() * 1000.0;
        
        if self.benchmark_mode {
            self.log_benchmark_to_csv();
        }
    }

    /// Recalculates zoning for a local area around an edge.
    pub fn recalculate_zoning_local(&mut self, edge_idx: usize) {
        if edge_idx >= self.region_graph.edges.len() { return; }
        
        // Use the batched invalidation and parallelized flush
        self.transit_network.zoning_dirty_edges.insert(edge_idx);
        self.transit_network.invalidate_zoning_near_edge(edge_idx, &self.region_graph);
        self.transit_network.flush_zoning_updates(&mut self.zoning, &self.region_graph);
    }

    /// Returns the dimensions of the heightmap.
    pub fn get_heightmap_size_internal(&self) -> Vector2 {
        Vector2::new(self.heightmap.width as f32, self.heightmap.height as f32)
    }

    /// Performs a full edge compaction, removing deleted edges and remapping all internal indices.
    pub fn perform_edge_compaction_internal(&mut self) {
        let deleted_count = self.region_graph.edges.iter().filter(|e| e.deleted).count();
        if deleted_count == 0 { return; }
        
        godot_print!("SimulationNode: Compacting road network (removing {} deleted edges)...", deleted_count);
        
        let mapping = self.region_graph.compact_edges();
        if mapping.is_empty() { return; }
        
        // 1. Update Agents
        self.agents.update_edge_indices(&mapping);
        
        // 2. Update Zoning
        self.zoning.update_edge_indices(&mapping);

        // 3. Update Buildings
        self.allocator.update_edge_indices(&mapping);
        
        // 4. Rebuild CCH Graph (as its internal cached indices are now invalid)
        self.transit_network.cch_graph = crate::simulation::pathing::cch::CchGraph::build(&self.region_graph);
        
        godot_print!("SimulationNode: Compaction complete. Edge count: {}", self.region_graph.edges.len());
    }
}

#[godot_api]
impl SimulationNode {
    /// Returns the pollution image data as a PackedByteArray (RGBA8).
    #[func]
    pub fn get_pollution_image_data(&self) -> PackedByteArray {
        Self::grid_to_image_data_internal(&self.pollution.grid, self.heightmap.width, self.heightmap.height, 255, 50, 50, 100.0)
    }

    /// Returns the noise image data as a PackedByteArray (RGBA8).
    #[func]
    pub fn get_noise_image_data(&self) -> PackedByteArray {
        Self::grid_to_image_data_internal(&self.noise.grid, self.heightmap.width, self.heightmap.height, 200, 200, 200, 100.0)
    }

    /// Returns the desirability image data as a PackedByteArray (RGBA8).
    #[func]
    pub fn get_desirability_image_data(&self) -> PackedByteArray {
        Self::grid_to_image_data_internal(&self.desirability.grid, self.heightmap.width, self.heightmap.height, 50, 255, 50, 100.0)
    }

    /// Undoes the last action. Returns true if successful.
    #[func]
    pub fn undo_action(&mut self) -> bool {
        self.undo_action_internal()
    }

    /// Sculpts the terrain heightmap.
    #[func]
    pub fn sculpt_terrain(&mut self, pos: Vector2, radius: f32, strength: f32) {
        self.sculpt_terrain_internal(pos, radius, strength);
    }

    /// Adds a volume of water at a specific grid position.
    #[func]
    pub fn add_water(&mut self, pos: Vector2, amount: f32) {
        self.add_water_internal(pos, amount);
    }

    /// Adds a continuous water source at a specific grid position.
    #[func]
    pub fn add_water_source(&mut self, pos: Vector2, rate_add: f32) {
        self.add_water_source_internal(pos, rate_add);
    }

    /// Returns whether the terrain mesh needs rebuilding.
    #[func]
    pub fn is_terrain_dirty(&self) -> bool { self.terrain_dirty }
    
    /// Returns whether the water mesh needs rebuilding.
    #[func]
    pub fn is_water_dirty(&self) -> bool { self.water_dirty }

    /// Clears the terrain dirty flag.
    #[func]
    pub fn clear_terrain_dirty(&mut self) { self.terrain_dirty = false; }
    
    /// Clears the water dirty flag.
    #[func]
    pub fn clear_water_dirty(&mut self) { self.water_dirty = false; }

    /// Returns the raw heightmap data.
    #[func]
    pub fn get_heightmap_data(&self) -> PackedFloat32Array {
        PackedFloat32Array::from_iter(self.heightmap.data.iter().cloned())
    }

    /// Returns the raw water depth data.
    #[func]
    pub fn get_water_data(&self) -> PackedFloat32Array {
        PackedFloat32Array::from_iter(self.watermap.depth.iter().cloned())
    }

    /// Returns the raw water velocity data.
    #[func]
    pub fn get_water_velocity_data(&self) -> PackedFloat32Array {
        PackedFloat32Array::from_iter(self.watermap.velocity.iter().cloned())
    }

    /// Returns the dimensions of the heightmap.
    #[func]
    pub fn get_heightmap_size(&self) -> Vector2 {
        self.get_heightmap_size_internal()
    }

    /// Sets the zone type for a specific 10m cell.
    #[func]
    pub fn set_zoning_cell(&mut self, edge_idx: i32, side: i8, x: i32, y: i32, zone_type_int: u8) {
        self.set_zoning_cell_internal(edge_idx, side, x, y, zone_type_int);
    }

    /// Returns a PackedFloat32Array for rendering the zone grid.
    #[func]
    pub fn get_zoning_grid_data(&self) -> PackedFloat32Array {
        self.zoning.get_render_data(&self.region_graph)
    }

    /// Returns information about zoning on a particular edge.
    #[func]
    pub fn get_edge_zoning_info(&self, edge_idx: i32) -> VarDictionary {
        let mut dict = VarDictionary::new();
        if let Some(grid) = self.zoning.edge_grids.get(&(edge_idx as usize)) {
            dict.set("cells_long", grid.cells_long as i32);
            dict.set("cell_size", self.config.zone_cell_m);
            dict.set("left_side", PackedByteArray::from_iter(grid.left_side.iter().map(|&z| z as u8)));
            dict.set("right_side", PackedByteArray::from_iter(grid.right_side.iter().map(|&z| z as u8)));
        }
        dict
    }

    /// Returns whether a specific zoning cell is obstructed.
    #[func]
    pub fn is_zoning_cell_obstructed(&self, edge_idx: i32, side: i32, x: i32, y: i32) -> bool {
        let graph = &self.region_graph;
        if let Some(edge) = graph.edges.get(edge_idx as usize) {
            if (side == 1 && !edge.zoning_left) || (side == -1 && !edge.zoning_right) {
                return true;
            }
        }
        self.zoning.is_cell_obstructed(edge_idx as usize, side as i8, x as usize, y as usize, graph, None)
    }

    /// Enables or disables zoning for a specific side of a road edge.
    #[func]
    pub fn set_zoning_enabled(&mut self, edge_idx: i32, side: i32, enabled: bool) {
        self.set_zoning_enabled_internal(edge_idx, side, enabled);
    }

    /// Returns the world-space center position of a specific zoning cell.
    #[func]
    pub fn get_zoning_cell_center(&self, edge_idx: i32, side: i8, x: i32, y: i32) -> Vector2 {
        let v2 = self.zoning.get_cell_center(edge_idx as usize, side, x as usize, y as usize, &self.region_graph);
        Vector2::new(v2.x, v2.y)
    }

    /// Updates the MultiMesh visualizers for the zoning tool.
    #[func]
    pub fn update_zoning_visuals(&self, grid_mm: Gd<MultiMesh>, paint_mm: Gd<MultiMesh>, hovered_edge: i32, mode: i32, mouse_pos_3d: Vector3) {
        self.update_zoning_visuals_internal(grid_mm, paint_mm, hovered_edge, mode, mouse_pos_3d);
    }

    /// Returns obstacle polygons for zoning tool overlap checks.
    #[func]
    pub fn get_obstacle_polygons_float_array(&self, ignore_poly_id: i32, ignore_edge_idx: i32) -> PackedFloat32Array {
        self.get_obstacle_polygons_internal(ignore_poly_id, ignore_edge_idx)
    }

    /// Returns the ID of the edge hovered by the mouse.
    #[func]
    pub fn get_hovered_edge(&self, world_x: f32, world_z: f32) -> i32 {
        self.get_hovered_edge_internal(world_x, world_z)
    }

    /// Returns the raycast depth against the road network.
    #[func]
    pub fn get_max_polygon_depth(&self, origin_x: f32, origin_z: f32, dir_x: f32, dir_z: f32, max_search: f32) -> f32 {
        self.get_max_polygon_depth_internal(origin_x, origin_z, dir_x, dir_z, max_search)
    }

    /// Sets the simulation speed multiplier.
    #[func]
    pub fn set_simulation_speed(&mut self, speed: f32) {
        self.time.speed_multiplier = speed.max(0.0);
    }

    /// Returns the current simulation day count.
    #[func]
    pub fn get_current_day(&self) -> u32 {
        self.time.current_day
    }

    /// Returns the packed transforms for all visible agents.
    #[func]
    pub fn get_agent_transforms(&self) -> PackedFloat32Array {
        self.get_agent_transforms_internal()
    }

    /// Returns debug path geometry for active agents.
    #[func]
    pub fn get_agent_paths_debug(&self) -> PackedVector3Array {
        self.get_agent_paths_debug_internal()
    }

    /// Returns city demographic statistics.
    #[func]
    pub fn get_city_demographics(&self) -> VarDictionary {
        self.get_city_demographics_internal()
    }

    /// Returns the packed transforms for buildings of a specific zone type.
    #[func]
    pub fn get_building_transforms(&self, zone_type_int: u8) -> PackedFloat32Array {
        self.get_building_transforms_internal(zone_type_int)
    }

    /// Returns the closest boundary point on a road edge to the given position.
    #[func]
    pub fn get_closest_point_on_edge(&self, edge_idx: i32, point_x: f32, point_y: f32) -> Vector2 {
        self.get_closest_point_on_edge_internal(edge_idx, point_x, point_y)
    }

    /// Returns the physical segment geometry for a road edge.
    #[func]
    pub fn get_edge_geometry(&self, edge_idx: i32) -> PackedVector2Array {
        self.get_edge_geometry_internal(edge_idx)
    }

    /// Returns the width of a specific road edge.
    #[func]
    pub fn get_edge_width(&self, edge_idx: i32) -> f32 {
        if edge_idx < 0 || edge_idx as usize >= self.region_graph.edges.len() { return 6.0; }
        self.region_graph.edges[edge_idx as usize].width
    }

    /// Returns a curved frontage between two points on an edge.
    #[func]
    pub fn get_curved_frontage(&self, edge_idx: i32, start_p: Vector2, end_p: Vector2) -> PackedVector2Array {
        self.get_curved_frontage_internal(edge_idx, start_p, end_p)
    }

    /// Adds a new road segment to the network.
    #[func]
    pub fn add_road(&mut self, points: PackedVector3Array, fwd_lanes: i32, bkw_lanes: i32, zoning_left: bool, zoning_right: bool) {
        self.add_road_internal(points, fwd_lanes, bkw_lanes, zoning_left, zoning_right);
    }

    /// Sets the classification of an edge (Standard, Bridge, Tunnel).
    #[func]
    pub fn set_edge_class(&mut self, edge_idx: i32, class_int: u8) {
        self.set_edge_class_internal(edge_idx, class_int);
    }

    /// Returns dictionary of road/intersection mesh data.
    #[func]
    pub fn get_road_mesh_data(&self) -> VarDictionary {
        self.get_road_mesh_data_internal()
    }

    /// Returns the closest network point (node/edge) within range.
    #[func]
    pub fn get_closest_network_point(&self, world_pos: Vector3, max_dist: f32) -> Variant {
        match self.get_closest_network_point_internal(world_pos, max_dist) {
            Some(p) => p.to_variant(),
            None => Variant::nil(),
        }
    }

    /// Returns the ID of the closest network node.
    #[func]
    pub fn get_closest_node(&self, world_pos: Vector3, max_dist: f32) -> i32 {
        self.get_closest_node_internal(world_pos, max_dist)
    }

    /// Placeholder for cul-de-sac tools.
    #[func]
    pub fn set_node_cul_de_sac(&mut self, _node_id: i32, _enabled: bool, _radius: f32) { }

    /// Placeholder for cul-de-sac tools.
    #[func]
    pub fn has_cul_de_sac(&self, _node_id: i32) -> bool { false }

    /// Returns the number of road connections for a node.
    #[func]
    pub fn get_node_connection_count(&self, node_id: i32) -> i32 {
        self.get_node_connection_count_internal(node_id)
    }

    /// Repositions a network node.
    #[func]
    pub fn move_network_node(&mut self, node_id: i32, pos: Vector3) {
        self.move_network_node_internal(node_id, pos);
    }

    /// Returns all junction node positions.
    #[func]
    pub fn get_network_nodes(&self) -> PackedVector3Array {
        self.get_network_nodes_internal()
    }

    /// Configures a lane connection rule at a junction.
    #[func]
    pub fn set_lane_connection(&mut self, node_id: u32, from_edge: i32, from_lane: i32, to_edge: i32, to_lane: i32) {
        self.set_lane_connection_internal(node_id, from_edge, from_lane, to_edge, to_lane);
    }

    /// Clears all lane rules at a junction node.
    #[func]
    pub fn clear_lane_connections(&mut self, node_id: u32) {
        self.clear_lane_connections_internal(node_id);
    }

    /// Returns the world-space position of a node.
    #[func]
    pub fn get_node_pos(&self, node_id: u32) -> Vector3 {
        self.get_node_pos_internal(node_id)
    }

    /// Returns information about all lanes entering/leaving a junction.
    #[func]
    pub fn get_node_lanes(&self, node_id: u32) -> VarArray {
        self.get_node_lanes_internal(node_id)
    }

    /// Returns an array of current lane turn restrictions at a node.
    #[func]
    pub fn get_lane_connections_array(&self, node_id: u32) -> VarArray {
        self.get_lane_connections_array_internal(node_id)
    }

    /// Clears lane rules for a specific source lane.
    #[func]
    pub fn clear_lane_source(&mut self, node_id: u32, from_edge: i32, from_lane: i32) {
        self.clear_lane_source_internal(node_id, from_edge, from_lane);
    }

    /// Returns the average network direction at a given point.
    #[func]
    pub fn get_network_direction_at_point(&self, pos: Vector3) -> Vector3 {
        self.get_network_direction_at_point_internal(pos)
    }

    /// Snap terrain to road levels.
    #[func]
    pub fn flatten_terrain_for_roads(&mut self) {
        self.flatten_terrain_for_roads_internal();
    }

    /// Returns terrain height at a position.
    #[func]
    pub fn get_height_at(&self, pos: Vector2) -> f32 {
        self.get_height_at_internal(pos)
    }

    /// Raycasts against the terrain heightmap.
    #[func]
    pub fn intersect_terrain(&self, ray_origin: Vector3, ray_dir: Vector3) -> Variant {
        match self.intersect_terrain_internal(ray_origin, ray_dir) {
            Some(p) => p.to_variant(),
            None => Variant::nil(),
        }
    }

    /// Loads heightmap from a PackedFloat32Array.
    #[func]
    pub fn load_heightmap_data(&mut self, data: PackedFloat32Array) {
        self.load_heightmap_data_internal(data);
    }

    /// Returns global lane width.
    #[func]
    pub fn get_lane_width(&self) -> f32 {
        config::LANE_WIDTH
    }

    /// High-level city setup for performance testing.
    #[func]
    pub fn setup_benchmark_city(&mut self, grid_size: i32, agent_count: i32) {
        self.setup_benchmark_city_internal(grid_size, agent_count);
    }

    /// Returns performance stats (ms, FPS, agents).
    #[func]
    pub fn get_perf_stats(&self) -> VarDictionary {
        self.get_perf_stats_internal()
    }

    /// Helper to convert a DataGrid<f32> to an upsampled PackedByteArray for Godot ImageTexture.
    pub fn grid_to_image_data_internal(grid: &crate::simulation::grid::data_grid::DataGrid<f32>, target_w: usize, target_h: usize, r: u8, g: u8, b: u8, max_val: f32) -> PackedByteArray {
        let mut pixels = Vec::with_capacity(target_w * target_h * 4);
        let scale_x = grid.width as f32 / target_w as f32;
        let scale_y = grid.height as f32 / target_h as f32;

        for y in 0..target_h {
            for x in 0..target_w {
                let val = grid.sample_bilinear(x as f32 * scale_x, y as f32 * scale_y);
                if val <= 0.01 {
                    pixels.extend_from_slice(&[0, 0, 0, 0]);
                } else {
                    let alpha = ((val / max_val).clamp(0.0, 1.0) * 200.0) as u8;
                    pixels.extend_from_slice(&[r, g, b, alpha]);
                }
            }
        }
        PackedByteArray::from_iter(pixels)
    }
}

#[godot_api]
impl INode3D for SimulationNode {
    fn init(base: Base<Node3D>) -> Self {
        godot_print!("Simulation Engine Initialized (Modular)");
        
        let args = godot::classes::Os::singleton().get_cmdline_user_args();
        let mut is_huge = false;
        for arg in args.as_slice() {
            if arg.to_string() == "--huge-map" {
                is_huge = true;
                break;
            }
        }

        let mut config = MapConfig::default();
        if is_huge {
            config.width_m = 20000.0;
            config.height_m = 20000.0;
        } else {
            config.width_m = 10000.0;
            config.height_m = 10000.0;
        }
        
        let w = config.zone_grid_width();
        let h = config.zone_grid_height();

        let mut sim = Self { 
            base,
            time: TimeSystem::new(),
            time_passed: 0.0,
            heightmap: TerrainSystem::new(w, h),
            watermap: WaterSystem::new(w, h),
            region_graph: crate::simulation::network::graph::RegionGraph::new(),
            transit_network: TransitNetwork::new(),
            zoning: ZoningSystem::new(&config),
            pollution: PollutionSystem::new(&config),
            noise: NoiseSystem::new(&config),
            desirability: DesirabilitySystem::new(&config),
            demand: DemandSystem::new(),
            allocator: BuildingAllocator::new(),
            agents: AgentSystem::new(),
            undo_stack: VecDeque::new(),
            last_tick_duration: 0.0,
            benchmark_mode: is_huge,
            terrain_dirty: true,
            water_dirty: true,
            config,
        };

        if is_huge {
            godot_print!("HUGE MAP BENCHMARK MODE ENABLED");
            let mut pts = PackedVector3Array::new();
            let border = (config.height_m * 0.5) - 1.0;
            pts.push(Vector3::new(0.0, 0.0, -border));
            pts.push(Vector3::new(0.0, 0.0, -border + 100.0));
            sim.add_road_internal(pts, 2, 2, true, true);
        } else {
            let mut pts = PackedVector3Array::new();
            let border = (config.height_m * 0.5) - 1.0;
            pts.push(Vector3::new(0.0, 0.0, -border));
            pts.push(Vector3::new(0.0, 0.0, -border / 2.0));
            sim.add_road_internal(pts, 2, 2, true, true);
        }

        sim
    }

    fn ready(&mut self) {
        if self.benchmark_mode {
            godot_print!("SimulationNode: Auto-triggering benchmark setup");
            self.setup_benchmark_city_internal(20, 100_000);
        }
    }

    fn process(&mut self, delta: f64) {
        self.time_passed += delta;
        
        if self.time.process_delta(delta) {
            self.simulate_tick();
        }
        
        if self.time.speed_multiplier > 0.0 {
            let dt = (delta * self.time.speed_multiplier as f64) as f32;
            self.agents.tick(&mut self.allocator, &self.transit_network.cch_graph, &mut self.region_graph, dt);
            
            let _sub_steps = 2;
            // ... water process logic ...
        }
    }
}
