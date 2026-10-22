// SPDX-License-Identifier: GPL-2.0-only

//! System and environment Godot API methods.

use super::*;
use crate::simulation::core::time::{SIMULATION_SPEED_STEPS, validated_simulation_speed};
use crate::simulation::core::weather::WeatherScenario;

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

    /// Returns the ascending supported speed steps, including pause and the maximum multiplier.
    #[func]
    pub fn get_simulation_speed_steps() -> PackedFloat32Array {
        PackedFloat32Array::from(SIMULATION_SPEED_STEPS.as_slice())
    }

    /// Queues a supported speed, clamping negative values to pause. Returns false for invalid
    /// input or a closed simulation command channel, so the UI can preserve its previous display.
    #[func]
    pub fn set_simulation_speed(&mut self, speed: f32) -> bool {
        let Some(speed) = validated_simulation_speed(speed) else {
            return false;
        };
        // Use channel so we don't block waiting for the tick lock.
        self.cmd_tx.send(SimCommand::SetSpeed(speed)).is_ok()
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

    /// Returns the position inside the current operational day in `0.0..1.0`.
    ///
    /// `0.0` is midnight and `0.5` is midday. This is the clock the scene lighting samples for
    /// the day/night cycle, so it carries the partial minute that `get_current_minute_of_day`
    /// truncates. A paused simulation holds the value still, which parks the sun.
    #[func]
    pub fn get_day_fraction(&self) -> f32 {
        self.snapshot.read().unwrap().current_day_fraction
    }

    /// Returns the cloud field state for this frame as `(origin_x_m, origin_z_m, coverage,
    /// optical_depth)`, from the same snapshot as the clock.
    ///
    /// The origin is added to a world position before the cloud field lookup; it combines the
    /// seeded field origin with the accumulated drift, so a paused game holds the clouds still.
    #[func]
    pub fn get_weather_clouds(&self) -> Vector4 {
        let weather = self.snapshot.read().unwrap().weather;
        Vector4::new(
            weather.cloud_origin_m[0],
            weather.cloud_origin_m[1],
            weather.cloud_coverage,
            weather.cloud_optical_depth,
        )
    }

    /// Returns the wind for this frame as `(clock_s, strength, offset_x_m, offset_z_m)`, from the
    /// same snapshot as the clock: the wrapped wind clock every sway and ripple phase reads, the
    /// wind speed over the authored one, and the distance the air has moved, for the gust field.
    #[func]
    pub fn get_weather_wind(&self) -> Vector4 {
        let weather = self.snapshot.read().unwrap().weather;
        Vector4::new(
            weather.wind_clock_s,
            weather.wind_strength,
            weather.wind_offset_m[0],
            weather.wind_offset_m[1],
        )
    }

    /// Returns the high cloud layer for this frame as `(origin_x_m, origin_z_m, cover, 0)`, from
    /// the same snapshot as the clock. The origin is added to a world position before the high
    /// layer's lookup; the layer drifts on its own, faster and veered wind.
    #[func]
    pub fn get_weather_high_clouds(&self) -> Vector4 {
        let weather = self.snapshot.read().unwrap().weather;
        Vector4::new(
            weather.high_origin_m[0],
            weather.high_origin_m[1],
            weather.high_cover,
            0.0,
        )
    }

    /// Returns the rain intensity this frame in `[0, 1]`, from dry to a downpour. It follows the
    /// cloud deck: rain falls only from a closed deck thicker than an overcast one.
    #[func]
    pub fn get_weather_rain(&self) -> f32 {
        self.snapshot.read().unwrap().weather.rain
    }

    /// Returns the unit X/Z direction the wind blows towards this frame, from the same snapshot
    /// as the clock. Trees bend and whitecaps streak along it.
    #[func]
    pub fn get_weather_wind_direction(&self) -> Vector2 {
        let direction = self.snapshot.read().unwrap().weather.wind_direction;
        Vector2::new(direction[0], direction[1])
    }

    /// Whether the seeded schedule moves the weather, from the same snapshot as the clock.
    #[func]
    pub fn get_weather_automatic(&self) -> bool {
        self.snapshot.read().unwrap().weather.automatic
    }

    /// Hands the weather to the seeded schedule, or takes it away so the current scenario holds.
    /// Returns false for a closed simulation command channel.
    #[func]
    pub fn set_weather_automatic(&mut self, automatic: bool) -> bool {
        self.cmd_tx
            .send(SimCommand::SetWeatherAutomatic(automatic))
            .is_ok()
    }

    /// Moves the weather to scenario `index` (0 clear, 1 fair cumulus, 2 overcast, 3 storm) over
    /// `duration_s` speed-scaled seconds and holds it there: the schedule stops. Zero switches at
    /// once. Returns false for an unknown index or a closed simulation command channel.
    #[func]
    pub fn set_weather_scenario(&mut self, index: i32, duration_s: f64) -> bool {
        let Some(scenario) = u8::try_from(index)
            .ok()
            .and_then(WeatherScenario::from_index)
        else {
            return false;
        };
        self.cmd_tx
            .send(SimCommand::SetWeather(scenario, duration_s))
            .is_ok()
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

    /// Up to `max_count` drawn cars nearest `eye` within `radius_m`, nearest first, from the
    /// bucket → transform buffers `smooth_car_render_data` returns: 13 floats per car, its
    /// 12-float MultiMesh transform then its bucket key. Feeds the headlight spot lights.
    /// O(cars · log max_count).
    #[func]
    pub fn nearest_car_poses(
        &self,
        car_buffers: VarDictionary,
        eye: Vector3,
        radius_m: f32,
        max_count: i64,
    ) -> PackedFloat32Array {
        use crate::nodes::sim::render::car_visual::nearest_cars;
        let mut keys = Vec::new();
        let mut buffers = Vec::new();
        for (key, buffer) in car_buffers.iter_shared() {
            if let (Ok(key), Ok(buffer)) =
                (key.try_to::<i64>(), buffer.try_to::<PackedFloat32Array>())
            {
                keys.push(key as f32);
                buffers.push(buffer);
            }
        }
        let slices: Vec<&[f32]> = buffers.iter().map(|buffer| buffer.as_slice()).collect();
        let chosen = nearest_cars(
            &slices,
            glam::Vec3::new(eye.x, eye.y, eye.z),
            radius_m,
            usize::try_from(max_count).unwrap_or(0),
        );
        let mut out = PackedFloat32Array::new();
        out.resize(chosen.len() * 13);
        let out_slice = out.as_mut_slice();
        for (i, (buffer, car)) in chosen.into_iter().enumerate() {
            out_slice[i * 13..i * 13 + 12]
                .copy_from_slice(&slices[buffer][car * 12..car * 12 + 12]);
            out_slice[i * 13 + 12] = keys[buffer];
        }
        out
    }

    /// Returns bucket → MultiMesh transform buffer for this frame's drawn cars, from buckets
    /// shaped like `get_car_render_data`. Poses move toward their snapshot poses and off-lane cars
    /// are grounded; if the support solve is busy they show their validated snapshot pose. A
    /// bucket with mismatched identity or flag counts is returned unsmoothed.
    /// O(cars) per frame; one non-blocking core attempt per bucket with off-lane cars.
    #[func]
    pub fn smooth_car_render_data(&mut self, car_data: VarDictionary, delta: f64) -> VarDictionary {
        use crate::nodes::sim::render::car_visual::{CarVisualSmoother, restore_off_lane_targets};
        let alpha = CarVisualSmoother::alpha(delta);
        let mut result = VarDictionary::new();
        for (key, bucket) in car_data.iter_shared() {
            let Ok(bucket) = bucket.try_to::<VarDictionary>() else {
                continue;
            };
            let field = |name: &str| bucket.get(name).unwrap_or_default();
            let targets = field("transforms")
                .try_to::<PackedFloat32Array>()
                .unwrap_or_default();
            let ids = field("ids")
                .try_to::<PackedInt64Array>()
                .unwrap_or_default();
            let flags = field("ground_flags")
                .try_to::<PackedByteArray>()
                .unwrap_or_default();
            let count = targets.len() / 12;
            if targets.len() % 12 != 0 || ids.len() != count || flags.len() != count {
                result.set(key, targets);
                continue;
            }
            let mut poses = PackedFloat32Array::from(targets.as_slice());
            self.car_visuals.interpolate(
                poses.as_mut_slice(),
                ids.as_slice(),
                flags.as_slice(),
                alpha,
            );
            if flags.as_slice().contains(&1) {
                let vehicle = key
                    .try_to::<i64>()
                    .ok()
                    .and_then(|k| u8::try_from(k / 10).ok());
                let grounded = vehicle.is_some_and(|vehicle| {
                    Self::try_lock_shared_core(&self.core).is_some_and(|core| {
                        core.ground_vehicle_transforms(
                            vehicle,
                            poses.as_mut_slice(),
                            flags.as_slice(),
                        )
                    })
                });
                if !grounded {
                    restore_off_lane_targets(
                        poses.as_mut_slice(),
                        targets.as_slice(),
                        flags.as_slice(),
                    );
                }
            }
            self.car_visuals
                .record(poses.as_slice(), ids.as_slice(), flags.as_slice());
            result.set(key, poses);
        }
        self.car_visuals.finish_frame();
        result
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
            // The pool the extension's work runs in (CODE-17).
            "rayon_threads": crate::thread_pools::install(rayon::current_num_threads) as i64,
            "generation": core.road_tool_surface_generation as i64,
            "simulation_speed": core.time.speed_multiplier,
            "command": core.last_road_edit_metrics.to_dictionary(),
        }
    }
}
