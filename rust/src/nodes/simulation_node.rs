//! Main GDExtension node for the Metrum Rise simulation.
//!
//! This node is a thin Godot bridge. All simulation state lives in `SimCore`
//! (owned behind `Arc<Mutex<SimCore>>`), which is ticked continuously by a
//! dedicated background thread. The render thread reads only from a
//! `RenderSnapshot` that the sim thread writes after each tick.

use godot::classes::{INode3D, MultiMesh, Node3D};
use godot::prelude::*;

use crate::config;
use crate::nodes::sim::core::{RenderSnapshot, SimCommand, SimCore, run_sim_thread};
use crate::simulation::core::config::MapConfig;
use crate::simulation::core::time::TimeSystem;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::demand::DemandSystem;
use crate::simulation::grid::desirability::DesirabilitySystem;
use crate::simulation::grid::noise::NoiseSystem;
use crate::simulation::grid::pollution::PollutionSystem;
use crate::simulation::grid::zoning::ZoningSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::water::WaterSystem;
use crate::simulation::buildings::allocator::BuildingAllocator;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, RwLock};

#[derive(GodotClass)]
#[class(base=Node3D)]
/// The central simulation node exposed to Godot.
///
/// All simulation state is in [`SimCore`] behind an `Arc<Mutex<>>`. This struct
/// holds only Godot-facing fields plus the handles needed to communicate with the
/// background sim thread.
pub struct SimulationNode {
    /// All simulation state — ticked by the background thread.
    pub(crate) core: Arc<Mutex<SimCore>>,
    /// Latest pre-computed rendering data from the sim thread.
    pub(crate) snapshot: Arc<RwLock<RenderSnapshot>>,
    /// Background sim thread handle.
    pub(crate) sim_thread: Option<std::thread::JoinHandle<()>>,
    /// Channel to send commands (speed changes, quit) to the sim thread.
    pub(crate) cmd_tx: std::sync::mpsc::Sender<SimCommand>,
    /// Receiver held here until `ready()` transfers it to the background thread.
    pub(crate) cmd_rx: Option<std::sync::mpsc::Receiver<SimCommand>>,
    /// True when running in headless benchmark mode.
    pub(crate) benchmark_mode: bool,
    /// Incremented every Godot frame in benchmark mode.
    pub(crate) benchmark_tick_count: u32,
    /// Last day for which benchmark CSV has been written.
    pub(crate) last_logged_day: u32,
    /// Accumulated Godot render time (unused by sim, kept for potential UI use).
    pub(crate) time_passed: f64,
    base: Base<Node3D>,
}

impl SimulationNode {
    /// Acquires the sim-core mutex, recovering silently if it was poisoned by a
    /// prior sim-thread panic.  Using `unwrap()` on a poisoned mutex would
    /// crash Godot on the next frame even though the sim thread has already
    /// recovered; this helper matches the recovery logic in `run_sim_thread`.
    #[inline]
    fn lock_core(&self) -> std::sync::MutexGuard<'_, crate::nodes::sim::core::SimCore> {
        match self.core.lock() {
            Ok(g) => g,
            Err(e) => {
                godot_error!("[sim] mutex poisoned — recovering in Godot main-thread call");
                e.into_inner()
            }
        }
    }

    /// Returns the dimensions of the heightmap.
    pub fn get_heightmap_size_internal(&self) -> Vector2 {
        let core = self.lock_core();
        Vector2::new(core.heightmap.width as f32, core.heightmap.height as f32)
    }

    /// Helper: converts a `DataGrid<f32>` to an upsampled `PackedByteArray` for Godot ImageTexture.
    pub fn grid_to_image_data_internal(
        grid: &crate::simulation::grid::data_grid::DataGrid<f32>,
        target_w: usize,
        target_h: usize,
        r: u8,
        g: u8,
        b: u8,
        max_val: f32,
    ) -> PackedByteArray {
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

    /// Spawns the background simulation thread.
    fn start_sim_thread(&mut self) {
        if let Some(rx) = self.cmd_rx.take() {
            let core = Arc::clone(&self.core);
            let snap = Arc::clone(&self.snapshot);
            self.sim_thread = Some(std::thread::spawn(move || {
                run_sim_thread(core, snap, rx);
            }));
        }
    }
}

#[godot_api]
impl SimulationNode {
    /// Returns the pollution image data as a PackedByteArray (RGBA8).
    #[func]
    pub fn get_pollution_image_data(&self) -> PackedByteArray {
        let core = self.lock_core();
        Self::grid_to_image_data_internal(
            &core.pollution.grid,
            core.heightmap.width,
            core.heightmap.height,
            255, 50, 50, 100.0,
        )
    }

    /// Returns the noise image data as a PackedByteArray (RGBA8).
    #[func]
    pub fn get_noise_image_data(&self) -> PackedByteArray {
        let core = self.lock_core();
        Self::grid_to_image_data_internal(
            &core.noise.grid,
            core.heightmap.width,
            core.heightmap.height,
            200, 200, 200, 100.0,
        )
    }

    /// Returns the desirability image data as a PackedByteArray (RGBA8).
    #[func]
    pub fn get_desirability_image_data(&self) -> PackedByteArray {
        let core = self.lock_core();
        Self::grid_to_image_data_internal(
            &core.desirability.grid,
            core.heightmap.width,
            core.heightmap.height,
            50, 255, 50, 100.0,
        )
    }

    /// Undoes the last action.
    #[func]
    pub fn undo_action(&mut self) -> bool {
        self.lock_core().undo_action_internal()
    }

    /// Sculpts the terrain heightmap.
    #[func]
    pub fn sculpt_terrain(&mut self, pos: Vector2, radius: f32, strength: f32) {
        self.lock_core().sculpt_terrain_internal(pos, radius, strength);
    }

    /// Adds a volume of water at a specific grid position.
    #[func]
    pub fn add_water(&mut self, pos: Vector2, amount: f32) {
        self.lock_core().add_water_internal(pos, amount);
    }

    /// Adds a continuous water source at a specific grid position.
    #[func]
    pub fn add_water_source(&mut self, pos: Vector2, rate_add: f32) {
        self.lock_core().add_water_source_internal(pos, rate_add);
    }

    /// Returns whether the terrain mesh needs rebuilding.
    #[func]
    pub fn is_terrain_dirty(&self) -> bool {
        self.snapshot.read().unwrap().terrain_dirty
    }

    /// Returns whether the water mesh needs rebuilding.
    #[func]
    pub fn is_water_dirty(&self) -> bool {
        self.snapshot.read().unwrap().water_dirty
    }

    /// Clears the terrain dirty flag.
    #[func]
    pub fn clear_terrain_dirty(&mut self) {
        self.lock_core().terrain_dirty = false;
        self.snapshot.write().unwrap().terrain_dirty = false;
    }

    /// Clears the water dirty flag.
    #[func]
    pub fn clear_water_dirty(&mut self) {
        self.lock_core().water_dirty = false;
        self.snapshot.write().unwrap().water_dirty = false;
    }

    /// Returns the raw heightmap data.
    #[func]
    pub fn get_heightmap_data(&self) -> PackedFloat32Array {
        PackedFloat32Array::from_iter(self.lock_core().heightmap.data.iter().cloned())
    }

    /// Returns the raw water depth data.
    #[func]
    pub fn get_water_data(&self) -> PackedFloat32Array {
        PackedFloat32Array::from_iter(self.lock_core().watermap.depth.iter().cloned())
    }

    /// Returns the raw water velocity data.
    #[func]
    pub fn get_water_velocity_data(&self) -> PackedFloat32Array {
        PackedFloat32Array::from_iter(
            self.lock_core().watermap.velocity.iter().cloned(),
        )
    }

    /// Returns the dimensions of the heightmap.
    #[func]
    pub fn get_heightmap_size(&self) -> Vector2 {
        self.get_heightmap_size_internal()
    }

    /// Sets the zone type for a specific 10m cell.
    #[func]
    pub fn set_zoning_cell(&mut self, edge_idx: i32, side: i8, x: i32, y: i32, zone_type_int: u8) {
        self.lock_core().set_zoning_cell_internal(edge_idx, side, x, y, zone_type_int);
    }

    /// Sets a range of zoning cells with a specific depth.
    #[func]
    pub fn set_zoning_range(
        &mut self,
        edge_idx: i32,
        side: i8,
        start_t: f32,
        end_t: f32,
        depth: i32,
        zone_type_int: u8,
    ) {
        self.lock_core().set_zoning_range_internal(edge_idx, side, start_t, end_t, depth, zone_type_int);
    }

    /// Returns a PackedFloat32Array for rendering the zone grid.
    #[func]
    pub fn get_zoning_grid_data(&self) -> PackedFloat32Array {
        let core = self.lock_core();
        core.zoning.get_render_data(&core.region_graph)
    }

    /// Returns information about zoning on a particular edge.
    #[func]
    pub fn get_edge_zoning_info(&self, edge_idx: i32) -> VarDictionary {
        let core = self.lock_core();
        let mut dict = VarDictionary::new();
        if let Some(grid) = core.zoning.edge_grids.get(&(edge_idx as usize)) {
            dict.set("cells_long", grid.cells_long as i32);
            dict.set("cell_size", core.config.zone_cell_m);
            dict.set(
                "left_side",
                PackedByteArray::from_iter(grid.left_side.iter().map(|&z| z as u8)),
            );
            dict.set(
                "right_side",
                PackedByteArray::from_iter(grid.right_side.iter().map(|&z| z as u8)),
            );
        }
        dict
    }

    /// Returns whether a specific zoning cell is obstructed.
    #[func]
    pub fn is_zoning_cell_obstructed(&self, edge_idx: i32, side: i32, x: i32, y: i32) -> bool {
        let core = self.lock_core();
        let graph = &core.region_graph;
        if let Some(edge) = graph.edges.get(edge_idx as usize) {
            if (side == 1 && !edge.zoning_left) || (side == -1 && !edge.zoning_right) {
                return true;
            }
        }
        core.zoning.is_cell_obstructed(
            edge_idx as usize,
            side as i8,
            x as usize,
            y as usize,
            graph,
            None,
        )
    }

    /// Enables or disables zoning for a specific side of a road edge.
    #[func]
    pub fn set_zoning_enabled(&mut self, edge_idx: i32, side: i32, enabled: bool) {
        self.lock_core().set_zoning_enabled_internal(edge_idx, side, enabled);
    }

    /// Returns the world-space center position of a specific zoning cell.
    #[func]
    pub fn get_zoning_cell_center(&self, edge_idx: i32, side: i8, x: i32, y: i32) -> Vector2 {
        let core = self.lock_core();
        let v2 = core.zoning.get_cell_center(
            edge_idx as usize,
            side,
            x as usize,
            y as usize,
            &core.region_graph,
        );
        Vector2::new(v2.x, v2.y)
    }

    /// Updates the MultiMesh visualizers for the zoning tool.
    #[func]
    pub fn update_zoning_visuals(
        &self,
        grid_mm: Gd<MultiMesh>,
        paint_mm: Gd<MultiMesh>,
        hovered_edges: VarArray,
        is_painting: bool,
        side: i32,
        t1: f32,
        t2: f32,
        depth: i32,
        zone_type: u8,
    ) {
        // Convert GDScript VarArray to pure Rust slice before locking SimCore.
        let edges: Vec<i32> = hovered_edges
            .iter_shared()
            .map(|v| v.to::<i32>())
            .collect();
        self.lock_core().update_zoning_visuals_internal(
            grid_mm, paint_mm, &edges, is_painting, side, t1, t2, depth, zone_type,
        );
    }

    /// Returns obstacle polygons for zoning tool overlap checks.
    #[func]
    pub fn get_obstacle_polygons_float_array(
        &self,
        ignore_poly_id: i32,
        ignore_edge_idx: i32,
    ) -> PackedFloat32Array {
        self.lock_core().get_obstacle_polygons_internal(ignore_poly_id, ignore_edge_idx)
    }

    /// Returns the ID of the edge hovered by the mouse.
    #[func]
    pub fn get_hovered_edge(&self, world_x: f32, world_z: f32) -> i32 {
        self.lock_core().get_hovered_edge_internal(world_x, world_z)
    }

    /// Returns the raycast depth against the road network.
    #[func]
    pub fn get_max_polygon_depth(
        &self,
        origin_x: f32,
        origin_z: f32,
        dir_x: f32,
        dir_z: f32,
        max_search: f32,
    ) -> f32 {
        self.lock_core().get_max_polygon_depth_internal(
            origin_x, origin_z, dir_x, dir_z, max_search,
        )
    }

    /// Sets the simulation speed multiplier.
    #[func]
    pub fn set_simulation_speed(&mut self, speed: f32) {
        // Use channel so we don't block waiting for the tick lock.
        let _ = self.cmd_tx.send(SimCommand::SetSpeed(speed.max(0.0)));
    }

    /// Updates the camera world-space AABB used to cull agent transform uploads.
    ///
    /// Call once per frame from GDScript with the camera's visible world rect,
    /// padded by ~200 m to avoid pop-in at the viewport edge. Agents outside the
    /// rect are excluded from the next `RenderSnapshot` transform buffers, reducing
    /// GPU upload cost from O(A_total) to O(A_visible).
    #[func]
    pub fn set_camera_aabb(&mut self, x_min: f32, x_max: f32, z_min: f32, z_max: f32) {
        let _ = self
            .cmd_tx
            .send(SimCommand::SetCameraAabb(x_min, x_max, z_min, z_max));
    }

    /// Maximum far-plane distance used when building the camera frustum AABB for agent culling.
    #[func]
    pub fn get_agent_cull_far_m() -> f32 {
        crate::config::AGENT_CULL_FAR_M
    }

    /// Padding added to each side of the camera frustum AABB to prevent pop-in.
    #[func]
    pub fn get_agent_cull_padding_m() -> f32 {
        crate::config::AGENT_CULL_PADDING_M
    }

    /// Target render FPS cap. Applied to `Engine.max_fps` at startup.
    #[func]
    pub fn get_target_fps() -> u32 {
        crate::config::TARGET_FPS
    }

    /// Returns the current simulation day count.
    #[func]
    pub fn get_current_day(&self) -> u32 {
        self.snapshot.read().unwrap().current_day
    }

    /// Returns a Dictionary of packed transforms for visible non-car agents, keyed by pedestrian_type.
    #[func]
    pub fn get_agent_transforms(&self) -> VarDictionary {
        let snap = self.snapshot.read().unwrap();
        let mut dict = VarDictionary::new();
        for (&k, v) in &snap.pedestrian_transforms {
            dict.set(k as i32, PackedFloat32Array::from_iter(v.iter().cloned()));
        }
        dict
    }

    /// Returns a Dictionary of packed transforms for visible car agents, keyed by vehicle type.
    #[func]
    pub fn get_car_transforms(&self) -> VarDictionary {
        let snap = self.snapshot.read().unwrap();
        let mut dict = VarDictionary::new();
        for (&k, v) in &snap.car_transforms {
            dict.set(k as i32, PackedFloat32Array::from_iter(v.iter().cloned()));
        }
        dict
    }

    /// Returns debug path geometry for active agents.
    #[func]
    pub fn get_agent_paths_debug(&self) -> VarDictionary {
        self.lock_core().get_agent_paths_debug_internal()
    }

    /// Returns city demographic statistics.
    #[func]
    pub fn get_city_demographics(&self) -> VarDictionary {
        self.lock_core().get_city_demographics_internal()
    }

    /// Returns current residential, commercial, and industrial demand values (-100 to 100).
    #[func]
    pub fn get_demand_stats(&self) -> VarDictionary {
        self.lock_core().get_demand_stats_internal()
    }

    /// Returns the packed transforms for buildings of a specific zone type.
    #[func]
    pub fn get_building_transforms(&self, zone_type_int: u8, variant: u8) -> PackedFloat32Array {
        self.lock_core().get_building_transforms_internal(zone_type_int, variant)
    }

    /// Returns the packed transforms for building plots/foundations of a specific zone type.
    #[func]
    pub fn get_building_plot_transforms(&self, zone_type_int: u8) -> PackedFloat32Array {
        self.lock_core().get_building_plot_transforms_internal(zone_type_int)
    }

    /// Registers metadata for a building model to aid in footprint calculation.
    #[func]
    pub fn register_building_metadata(
        &mut self,
        zone_id: u8,
        variant: u8,
        size_x: f32,
        size_y: f32,
        size_z: f32,
    ) {
        self.lock_core().allocator.set_model_metadata(
            zone_id,
            variant,
            crate::simulation::buildings::allocator::ModelMetadata { size_x, size_y, size_z },
        );
    }

    /// Returns the closest boundary point on a road edge to the given position.
    #[func]
    pub fn get_closest_point_on_edge(&self, edge_idx: i32, point_x: f32, point_y: f32) -> Vector2 {
        self.lock_core().get_closest_point_on_edge_internal(edge_idx, point_x, point_y)
    }

    /// Returns the physical segment geometry for a road edge.
    #[func]
    pub fn get_edge_geometry(&self, edge_idx: i32) -> PackedVector2Array {
        self.lock_core().get_edge_geometry_internal(edge_idx)
    }

    /// Returns the 3D geometry for a road edge.
    #[func]
    pub fn get_edge_geometry_3d(&self, edge_idx: i32) -> PackedVector3Array {
        let core = self.lock_core();
        if edge_idx < 0 || edge_idx as usize >= core.region_graph.edges.len() {
            return PackedVector3Array::new();
        }
        let edge = &core.region_graph.edges[edge_idx as usize];
        PackedVector3Array::from_iter(edge.physical_geometry.iter().cloned())
    }

    /// Returns the width of a specific road edge.
    #[func]
    pub fn get_edge_width(&self, edge_idx: i32) -> f32 {
        let core = self.lock_core();
        if edge_idx < 0 || edge_idx as usize >= core.region_graph.edges.len() {
            return 6.0;
        }
        core.region_graph.edges[edge_idx as usize].width
    }

    /// Returns a curved frontage between two points on an edge.
    #[func]
    pub fn get_curved_frontage(
        &self,
        edge_idx: i32,
        start_p: Vector2,
        end_p: Vector2,
    ) -> PackedVector2Array {
        self.lock_core().get_curved_frontage_internal(edge_idx, start_p, end_p)
    }

    /// Adds a new road segment to the network.
    #[func]
    pub fn add_road(
        &mut self,
        points: PackedVector3Array,
        fwd_lanes: i32,
        bkw_lanes: i32,
        zoning_left: bool,
        zoning_right: bool,
    ) {
        self.lock_core().add_road_internal(points, fwd_lanes, bkw_lanes, zoning_left, zoning_right);
    }

    /// Returns the node ID of the nearest graph node near the border, or -1.
    #[func]
    pub fn check_border_candidate(&self, pos: Vector3) -> i64 {
        self.lock_core().check_border_candidate_internal(pos)
    }

    /// Marks the node at `node_id` as an external border connection.
    #[func]
    pub fn set_border_connection(&mut self, node_id: i32) {
        self.lock_core().set_border_connection_internal(node_id);
    }

    /// Returns the world-space positions of all active border nodes as a flat float array.
    #[func]
    pub fn get_border_nodes(&self) -> PackedFloat32Array {
        self.lock_core().get_border_nodes_internal()
    }

    /// Sets the classification of an edge (Standard, Bridge, Tunnel).
    #[func]
    pub fn set_edge_class(&mut self, edge_idx: i32, class_int: u8) {
        self.lock_core().set_edge_class_internal(edge_idx, class_int);
    }

    /// Returns dictionary of road/intersection mesh data.
    #[func]
    pub fn get_road_mesh_data(&self) -> VarDictionary {
        self.lock_core().get_road_mesh_data_internal()
    }

    /// Returns the closest network point (node/edge) within range.
    #[func]
    pub fn get_closest_network_point(&self, world_pos: Vector3, max_dist: f32) -> Variant {
        match self.lock_core().get_closest_network_point_internal(world_pos, max_dist) {
            Some(p) => p.to_variant(),
            None => Variant::nil(),
        }
    }

    /// Returns the ID of the closest network node.
    #[func]
    pub fn get_closest_node(&self, world_pos: Vector3, max_dist: f32) -> i32 {
        self.lock_core().get_closest_node_internal(world_pos, max_dist)
    }

    /// Placeholder for cul-de-sac tools.
    #[func]
    pub fn set_node_cul_de_sac(&mut self, _node_id: i32, _enabled: bool, _radius: f32) {}

    /// Placeholder for cul-de-sac tools.
    #[func]
    pub fn has_cul_de_sac(&self, _node_id: i32) -> bool {
        false
    }

    /// Returns the number of road connections for a node.
    #[func]
    pub fn get_node_connection_count(&self, node_id: i32) -> i32 {
        self.lock_core().get_node_connection_count_internal(node_id)
    }

    /// Repositions a network node.
    #[func]
    pub fn move_network_node(&mut self, node_id: i32, pos: Vector3) {
        self.lock_core().move_network_node_internal(node_id, pos);
    }

    /// Returns all junction node positions.
    #[func]
    pub fn get_network_nodes(&self) -> PackedVector3Array {
        self.lock_core().get_network_nodes_internal()
    }

    /// Configures a lane connection rule at a junction.
    #[func]
    pub fn set_lane_connection(
        &mut self,
        node_id: u32,
        from_edge: i32,
        from_lane: i32,
        to_edge: i32,
        to_lane: i32,
    ) {
        self.lock_core().set_lane_connection_internal(
            node_id, from_edge, from_lane, to_edge, to_lane,
        );
    }

    /// Clears all lane rules at a junction node.
    #[func]
    pub fn clear_lane_connections(&mut self, node_id: u32) {
        self.lock_core().clear_lane_connections_internal(node_id);
    }

    /// Returns the world-space position of a node.
    #[func]
    pub fn get_node_pos(&self, node_id: u32) -> Vector3 {
        self.lock_core().get_node_pos_internal(node_id)
    }

    /// Returns information about all lanes entering/leaving a junction.
    #[func]
    pub fn get_node_lanes(&self, node_id: u32) -> VarArray {
        self.lock_core().get_node_lanes_internal(node_id)
    }

    /// Returns an array of current lane turn restrictions at a node.
    #[func]
    pub fn get_lane_connections_array(&self, node_id: u32) -> VarArray {
        self.lock_core().get_lane_connections_array_internal(node_id)
    }

    /// Clears lane rules for a specific source lane.
    #[func]
    pub fn clear_lane_source(&mut self, node_id: u32, from_edge: i32, from_lane: i32) {
        self.lock_core().clear_lane_source_internal(node_id, from_edge, from_lane);
    }

    /// Returns the average network direction at a given point.
    #[func]
    pub fn get_network_direction_at_point(&self, pos: Vector3) -> Vector3 {
        self.lock_core().get_network_direction_at_point_internal(pos)
    }

    /// Snap terrain to road levels.
    #[func]
    pub fn flatten_terrain_for_roads(&mut self) {
        self.lock_core().flatten_terrain_for_roads_internal();
    }

    /// Returns terrain height at a position.
    #[func]
    pub fn get_height_at(&self, pos: Vector2) -> f32 {
        self.lock_core().get_height_at_internal(pos)
    }

    /// Raycasts against the terrain heightmap.
    #[func]
    pub fn intersect_terrain(&self, ray_origin: Vector3, ray_dir: Vector3) -> Variant {
        match self.lock_core().intersect_terrain_internal(ray_origin, ray_dir) {
            Some(p) => p.to_variant(),
            None => Variant::nil(),
        }
    }

    /// Loads heightmap from a PackedFloat32Array.
    #[func]
    pub fn load_heightmap_data(&mut self, data: PackedFloat32Array) {
        self.lock_core().load_heightmap_data_internal(data);
    }

    /// Saves the current simulation into a single SQLite snapshot file.
    #[func]
    pub fn save_game(&self, path: GString) -> bool {
        match self.lock_core().save_game_internal(&path.to_string()) {
            Ok(()) => true,
            Err(err) => {
                godot_print!("Save failed: {}", err);
                false
            }
        }
    }

    /// Loads a SQLite save snapshot and replaces the live simulation state.
    #[func]
    pub fn load_game(&mut self, path: GString) -> bool {
        match self.lock_core().load_game_internal(&path.to_string()) {
            Ok(()) => true,
            Err(err) => {
                godot_print!("Load failed: {}", err);
                false
            }
        }
    }

    /// Returns global lane width.
    #[func]
    pub fn get_lane_width(&self) -> f32 {
        config::LANE_WIDTH
    }

    /// High-level city setup for performance testing.
    #[func]
    pub fn setup_benchmark_city(&mut self, grid_size: i32, agent_count: i32) {
        self.lock_core().setup_benchmark_city_internal(grid_size, agent_count);
    }

    /// Returns performance stats (ms, FPS, agents).
    #[func]
    pub fn get_perf_stats(&self) -> VarDictionary {
        self.get_perf_stats_internal()
    }
}

#[godot_api]
impl INode3D for SimulationNode {
    fn init(base: Base<Node3D>) -> Self {
        godot_print!("Simulation Engine Initialized (Modular)");

        let args = godot::classes::Os::singleton().get_cmdline_user_args();
        let mut is_huge = false;
        let mut generate_benchmark = false;
        let mut run_benchmark = false;
        for arg in args.as_slice() {
            match arg.to_string().as_str() {
                "--huge-map" | "--benchmark" => {
                    is_huge = true;
                    run_benchmark = true;
                }
                "--generate-benchmark" => {
                    is_huge = true;
                    generate_benchmark = true;
                }
                _ => {}
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

        let benchmark_mode = run_benchmark || generate_benchmark;

        let core = SimCore {
            time: TimeSystem::new(),
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
            config,
            undo_stack: VecDeque::new(),
            terrain_dirty: true,
            water_dirty: true,
            benchmark_mode,
            last_tick_duration: 0.0,
            last_agent_tick_us: 0,
            camera_aabb: (0.0, 0.0, 0.0, 0.0), // 0.0 == 0.0 → cull disabled by default
        };

        let core_arc = Arc::new(Mutex::new(core));
        let snapshot = Arc::new(RwLock::new(RenderSnapshot::default()));
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();

        if generate_benchmark {
            godot_print!("BENCHMARK GENERATION MODE — will build city, save, and exit");
        } else if run_benchmark {
            godot_print!("BENCHMARK RUN MODE — will load benchmark.sav and simulate");
        }

        Self {
            core: core_arc,
            snapshot,
            sim_thread: None,
            cmd_tx,
            cmd_rx: Some(cmd_rx),
            benchmark_mode,
            benchmark_tick_count: 0,
            last_logged_day: 0,
            time_passed: 0.0,
            base,
        }
    }

    fn ready(&mut self) {
        godot::classes::Engine::singleton()
            .set_max_fps(crate::config::TARGET_FPS as i32);

        let args = godot::classes::Os::singleton().get_cmdline_user_args();
        let generate = args
            .as_slice()
            .iter()
            .any(|a| a.to_string() == "--generate-benchmark");
        let run = args.as_slice().iter().any(|a| {
            matches!(a.to_string().as_str(), "--benchmark" | "--huge-map")
        });

        if generate {
            self.generate_benchmark_map();
            return; // generate_benchmark_map() calls quit() — never reaches thread spawn
        } else if run {
            self.run_benchmark_from_save();
        }

        // Start the background simulation thread.
        self.start_sim_thread();
    }

    fn process(&mut self, delta: f64) {
        self.time_passed += delta;

        if self.benchmark_mode {
            self.benchmark_tick_count += 1;

            // Periodic console log every 600 frames.
            if self.benchmark_tick_count % 600 == 0 {
                let snap = self.snapshot.read().unwrap();
                godot_print!(
                    "[bench] frame={} agents={} agent_tick_us={} sim_tick_ms={:.2} pathfinds={} RSS={}MB",
                    self.benchmark_tick_count,
                    snap.agent_count,
                    snap.last_agent_tick_us,
                    snap.last_tick_ms,
                    snap.pathfind_count,
                    crate::nodes::sim::benchmark::rss_mb()
                );
            }

            // CSV log once per in-game day.
            {
                let day = self.snapshot.read().unwrap().current_day;
                if day > self.last_logged_day {
                    self.last_logged_day = day;
                    self.log_benchmark_to_csv();
                }
            }

            if self.benchmark_tick_count >= 3000 {
                godot_print!("[bench] DONE — 3000 frames complete. See benchmark_results.csv.");
                self.base_mut().get_tree().unwrap().quit();
            }
        }
        // All simulation ticking happens in the background thread.
    }
}
