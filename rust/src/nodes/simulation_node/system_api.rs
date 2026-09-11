// SPDX-License-Identifier: GPL-2.0-only

//! System and environment Godot API methods.

use super::*;

#[godot_api(secondary)]
impl SimulationNode {
    // ── Environment ──

    /// Returns the pollution image data as a PackedByteArray (RGBA8).
    #[func]
    pub fn get_pollution_image_data(&self) -> PackedByteArray {
        self.lock_core().get_pollution_image_data_internal()
    }

    /// Returns the noise image data as a PackedByteArray (RGBA8).
    #[func]
    pub fn get_noise_image_data(&self) -> PackedByteArray {
        self.lock_core().get_noise_image_data_internal()
    }

    /// Returns the desirability image data as a PackedByteArray (RGBA8).
    #[func]
    pub fn get_desirability_image_data(&self) -> PackedByteArray {
        self.lock_core().get_desirability_image_data_internal()
    }

    // ── System ──

    /// Queues the latest authoring action for undo on the simulation thread.
    #[func]
    pub fn undo_action(&mut self) -> bool {
        if self.cmd_tx.send(SimCommand::Undo).is_err() {
            return false;
        }
        self.lock_terrain_patch_payload_jobs().clear();
        true
    }

    // ── Simulation ──

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

    /// Returns the current operational minute since midnight.
    #[func]
    pub fn get_current_minute_of_day(&self) -> u16 {
        self.snapshot.read().unwrap().current_minute_of_day
    }

    /// Returns a Dictionary of packed transforms for visible non-car agents, keyed by pedestrian_type.
    #[func]
    pub fn get_agent_transforms(&self) -> VarDictionary {
        use crate::nodes::sim::bridge::agents::get_agent_transforms;
        get_agent_transforms(&self.snapshot.read().unwrap())
    }

    /// Returns coherent per-model car transforms, render IDs and off-lane support flags.
    #[func]
    pub fn get_car_render_data(&self) -> VarDictionary {
        crate::nodes::sim::bridge::agents::get_car_render_data(&self.snapshot.read().unwrap())
    }

    /// Registers model-local ground geometry once at mesh load; never alters agent movement.
    #[func]
    pub fn set_vehicle_ground_support(
        &self,
        vehicle: i32,
        bounds: Aabb,
        contacts: PackedVector3Array,
    ) -> bool {
        use crate::nodes::sim::render::vehicle_ground::VehicleGroundSupport;
        let Some(profile) = VehicleGroundSupport::new(
            [
                bounds.position.x,
                bounds.position.z,
                bounds.end().x,
                bounds.end().z,
            ],
            contacts.as_slice(),
        ) else {
            return false;
        };
        let mut core = self.lock_core();
        let Some(target) = usize::try_from(vehicle)
            .ok()
            .and_then(|id| core.vehicle_ground_support.get_mut(id))
        else {
            return false;
        };
        *target = profile;
        true
    }

    /// Grounds an interpolated car bucket in Rust. Empty means busy/unprepared: use its already
    /// validated snapshot poses, never publish unvalidated interpolated off-lane geometry.
    #[func]
    pub fn ground_car_transforms(
        &self,
        vehicle: i32,
        mut transforms: PackedFloat32Array,
        flags: PackedByteArray,
    ) -> PackedFloat32Array {
        let Some(core) = self.try_lock_core() else {
            return PackedFloat32Array::new();
        };
        let Ok(vehicle) = u8::try_from(vehicle) else {
            return PackedFloat32Array::new();
        };
        if core.ground_vehicle_transforms(vehicle, transforms.as_mut_slice(), flags.as_slice()) {
            transforms
        } else {
            PackedFloat32Array::new()
        }
    }

    /// Returns debug path geometry for active agents.
    #[func]
    pub fn get_agent_paths_debug(&self) -> VarDictionary {
        self.try_lock_core()
            .map(|core| core.get_agent_paths_debug_internal())
            .unwrap_or_default()
    }

    /// Returns `true` when the node was launched with `--asset-editor`.
    #[func]
    pub fn is_asset_editor_mode(&self) -> bool {
        self.asset_editor_mode
    }

    /// Returns `true` when the node was launched with `--economy-editor`.
    #[func]
    pub fn is_economy_editor_mode(&self) -> bool {
        self.economy_editor_mode
    }

    /// Returns `true` when the node was launched with `--world-editor`.
    #[func]
    pub fn is_world_editor_mode(&self) -> bool {
        self.world_editor_mode
    }

    /// Returns the current city treasury balance in currency units. May be negative.
    #[func]
    pub fn get_treasury_balance(&self) -> f64 {
        self.snapshot.read().unwrap().treasury_balance
    }

    /// Returns the total number of live agents from the latest render snapshot.
    #[func]
    pub fn get_agent_count(&self) -> i32 {
        self.snapshot.read().unwrap().agent_count
    }

    /// Returns global lane width.
    #[func]
    pub fn get_lane_width(&self) -> f32 {
        config::LANE_WIDTH
    }

    /// High-level city setup for performance testing.
    #[func]
    pub fn setup_benchmark_city(&mut self, grid_size: i32, agent_count: i32) {
        self.lock_core()
            .setup_benchmark_city_internal(grid_size, agent_count);
    }

    /// Returns performance stats (ms, FPS, agents).
    #[func]
    pub fn get_perf_stats(&self) -> VarDictionary {
        self.get_perf_stats_internal()
    }

    /// Captures road-workload cardinalities outside benchmark timing intervals.
    ///
    /// Counting live edges is O(edge slots), including tombstones. Do not poll this per frame.
    #[func]
    pub fn get_road_benchmark_state(&self) -> VarDictionary {
        let core = self.lock_core();
        vdict! {
            "nodes": core.region_graph.node_count() as i64,
            "edge_slots": core.region_graph.edge_count() as i64,
            "live_edges": core.region_graph.edges().iter().filter(|edge| !edge.deleted).count() as i64,
            "agents": core.agents.len() as i64,
            "buildings": core.allocator.buildings.len() as i64,
            "lanes": core.transit_network.lane_system.lanes.len() as i64,
            "rayon_threads": rayon::current_num_threads() as i64,
            "generation": core.road_tool_surface_generation as i64,
            "simulation_speed": core.time.speed_multiplier,
            "command": core.last_road_edit_metrics.to_dictionary(),
        }
    }
}
