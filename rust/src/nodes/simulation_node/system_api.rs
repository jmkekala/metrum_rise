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

    /// Test hook (`CODE-25`): the simulation thread holds the core, as a long frame would,
    /// until a call waits for it or `max_ms` passes. Returns once the core is held (true), or
    /// false if the simulation thread did not take it within `max_ms`. Tests use it to show
    /// that one-shot input waits for the core while per-frame queries answer busy.
    #[func]
    pub fn debug_hold_core(&self, max_ms: i32) -> bool {
        let longest = std::time::Duration::from_millis(max_ms.max(0) as u64);
        let (held, on_hold) = std::sync::mpsc::sync_channel(1);
        self.cmd_tx
            .send(SimCommand::HoldCore { longest, held })
            .is_ok()
            && on_hold.recv_timeout(longest).is_ok()
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
    /// `camera_position` is the eye the agents' draw distances measure from (`RENDER-28`).
    #[func]
    pub fn set_camera_aabb(
        &mut self,
        x_min: f32,
        x_max: f32,
        z_min: f32,
        z_max: f32,
        camera_position: Vector3,
    ) {
        let eye = camera_position.is_finite().then_some(camera_position);
        let camera = eye.map(|eye| Vector2::new(eye.x, eye.z));
        self.render_camera_position = camera;
        let _ = self.cmd_tx.send(SimCommand::SetCameraAabb {
            bounds: (x_min, x_max, z_min, z_max),
            camera,
            eye,
        });
    }

    /// Takes the agents' view from `camera` (`RENDER-32-G`): the ground rectangle the snapshot
    /// culls agents to (the viewport's corner rays down to the ground, at most
    /// `AGENT_CULL_FAR_M` along each, padded by `AGENT_CULL_PADDING_M`) and the camera position
    /// the off-lane ground solve's level of detail measures from, as `set_camera_aabb` sets
    /// them. Returns the rectangle as (x_min, x_max, z_min, z_max). O(1): four corner rays.
    #[func]
    pub fn update_agent_view(&mut self, camera: Gd<godot::classes::Camera3D>) -> Vector4 {
        use crate::nodes::sim::render::stream::view::agent_ground_rect;
        let Some(viewport) = camera.get_viewport() else {
            return Vector4::ZERO;
        };
        let size = viewport.get_visible_rect().size;
        let corners = [
            Vector2::ZERO,
            Vector2::new(size.x, 0.0),
            size,
            Vector2::new(0.0, size.y),
        ];
        let rays = corners.map(|corner| {
            (
                camera.project_ray_origin(corner),
                camera.project_ray_normal(corner),
            )
        });
        let [x_min, x_max, z_min, z_max] = agent_ground_rect(
            rays,
            crate::config::AGENT_CULL_FAR_M,
            crate::config::AGENT_CULL_PADDING_M,
        );
        self.set_camera_aabb(x_min, x_max, z_min, z_max, camera.get_global_position());
        Vector4::new(x_min, x_max, z_min, z_max)
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

    /// Horizontal camera radius for the exact off-lane vehicle support solve.
    #[func]
    pub fn get_vehicle_ground_full_detail_m() -> f32 {
        crate::nodes::sim::render::vehicle_ground::VEHICLE_GROUND_FULL_DETAIL_M
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

    /// Returns pedestrian_type → MultiMesh buffer (transform and walk phase, 16 floats a walker)
    /// of the walkers the latest snapshot draws, the node's own dictionary, valid until the next
    /// call. [`Self::get_changed_walker_buckets`] lists the buckets whose buffer changed since
    /// the last call, the only ones to upload; a bucket that left is absent (draw none)
    /// (`RENDER-28`). O(drawn walkers) bitwise compares, a copy of the changed buckets only.
    #[func]
    pub fn update_walker_buffers(&mut self) -> VarDictionary {
        self.walker_visuals
            .update(&self.snapshot.read().unwrap().pedestrian_transforms);
        self.changed_walker_buckets.clear();
        for &key in self.walker_visuals.changed() {
            let drawn = self.walker_visuals.drawn(key);
            if drawn.is_empty() {
                self.walker_buffers.remove(i64::from(key));
            } else {
                self.walker_buffers
                    .set(i64::from(key), PackedFloat32Array::from(drawn));
            }
            self.changed_walker_buckets.push(i64::from(key));
        }
        self.walker_buffers.clone()
    }

    /// The buckets whose walker buffer the last `update_walker_buffers` call changed, including
    /// those that left. O(changed buckets).
    #[func]
    pub fn get_changed_walker_buckets(&self) -> PackedInt64Array {
        self.changed_walker_buckets.clone()
    }

    /// The walkers' and moving cars' draw distances (`RENDER-28`), metres from the camera: walker
    /// fade start, walker draw distance, car fade start, car draw distance. The agents' shaders
    /// fade over each band; the snapshot stops posing past its end.
    #[func]
    pub fn get_agent_draw_distances() -> PackedFloat32Array {
        use crate::config::{
            CAR_DRAW_DISTANCE_M, CAR_FADE_START_M, WALKER_DRAW_DISTANCE_M, WALKER_FADE_START_M,
        };
        PackedFloat32Array::from(
            [
                WALKER_FADE_START_M,
                WALKER_DRAW_DISTANCE_M,
                CAR_FADE_START_M,
                CAR_DRAW_DISTANCE_M,
            ]
            .as_slice(),
        )
    }

    /// The latest snapshot's agent parts for the perf channel (`RENDER-28`): walkers and cars
    /// inside the camera's rectangle that their draw distances cut, and the microseconds the
    /// snapshot spent on agent instances. O(1).
    #[func]
    pub fn get_agent_snapshot_parts(&self) -> PackedInt64Array {
        let snapshot = self.snapshot.read().unwrap();
        let cut = snapshot.agent_draw_cut;
        PackedInt64Array::from(
            [
                i64::from(cut[0]),
                i64::from(cut[1]),
                i64::from(snapshot.agent_instances_us),
            ]
            .as_slice(),
        )
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
        usize::try_from(vehicle)
            .is_ok_and(|id| self.lock_core().set_vehicle_ground_support(id, profile))
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
        if core.ground_vehicle_transforms(
            vehicle,
            transforms.as_mut_slice(),
            flags.as_slice(),
            self.render_camera_position,
        ) {
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

    /// Returns bucket → MultiMesh transform buffer of this frame's drawn cars, from buckets shaped
    /// like `get_car_render_data`. Poses move toward their snapshot poses and off-lane cars are
    /// grounded; if the support solve is busy they show their validated snapshot pose. A bucket
    /// with mismatched identity or flag counts is drawn unsmoothed. Cars at rest are drawn as
    /// they were, with no solve (`RENDER-28`); [`Self::get_changed_car_buckets`] lists the
    /// buckets whose buffer changed, the only ones to upload. The dictionary is the node's own,
    /// valid until the next call. O(cars) per frame for the buckets with a moving car, one
    /// non-blocking core attempt and one parallel solve pass a frame for the off-lane cars not
    /// at rest; O(1) plus a compare of its inputs for an untouched bucket.
    #[func]
    pub fn smooth_car_render_data(&mut self, car_data: VarDictionary, delta: f64) -> VarDictionary {
        use crate::nodes::sim::render::car_visual::{CarVisualSmoother, GroundView};
        let alpha = CarVisualSmoother::alpha(delta);
        let camera = self.render_camera_position;
        self.car_visuals.begin_frame(GroundView {
            key: self.snapshot.read().unwrap().vehicle_ground_key,
            camera: camera.map(|camera| [camera.x, camera.y]),
        });
        self.changed_car_buckets.clear();
        for (key, bucket) in car_data.iter_shared() {
            let (Ok(bucket), Some(slot)) = (
                bucket.try_to::<VarDictionary>(),
                key.try_to::<i64>()
                    .ok()
                    .and_then(|key| u8::try_from(key).ok()),
            ) else {
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
                self.car_visuals.pass_through(slot, targets.as_slice());
            } else {
                self.car_visuals.smooth_bucket(
                    slot,
                    targets.as_slice(),
                    ids.as_slice(),
                    flags.as_slice(),
                    alpha,
                );
            }
        }
        if self.car_visuals.needs_solve() {
            // One non-blocking core attempt a frame, held for the one solve pass.
            let core = Self::try_lock_shared_core(&self.core);
            self.car_visuals.solve_pending(|slot, poses, solve| {
                core.as_ref().is_some_and(|core| {
                    core.ground_vehicle_transforms(slot / 10, poses, solve, camera)
                })
            });
        }
        self.car_visuals.finish_frame();
        for &slot in self.car_visuals.changed() {
            self.car_buffers.set(
                i64::from(slot),
                PackedFloat32Array::from(self.car_visuals.drawn_bucket(slot)),
            );
            self.changed_car_buckets.push(i64::from(slot));
        }
        for &slot in self.car_visuals.left() {
            self.car_buffers.remove(i64::from(slot));
            self.changed_car_buckets.push(i64::from(slot));
        }
        self.car_buffers.clone()
    }

    /// The buckets whose drawn buffer the last `smooth_car_render_data` call changed, including
    /// those that left (now absent from its dictionary: draw none). O(changed buckets).
    #[func]
    pub fn get_changed_car_buckets(&self) -> PackedInt64Array {
        self.changed_car_buckets.clone()
    }

    /// The last `smooth_car_render_data` call's split (`RENDER-28`), for the perf channel:
    /// microseconds smoothing (with the copy of the snapshot poses), in the off-lane ground solve
    /// (with its core attempt) and recording; then cars, off-lane cars, off-lane cars solved,
    /// cars at rest, such off-lane cars, buckets uploaded and buckets left untouched. O(1).
    #[func]
    pub fn get_car_frame_parts(&self) -> PackedInt64Array {
        PackedInt64Array::from(self.car_visuals.parts().as_slice())
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

    /// Turns the perf channel's main-thread core lock counts on (called on the main thread) or
    /// off (`CODE-26`); either way drops the counts so far. Off by default.
    #[func]
    pub fn set_core_lock_stats(&self, enabled: bool) {
        super::core_lock_stats::set_enabled(enabled);
    }

    /// The main thread's core locks since the last call, by call site (`file:line`): tries, tries
    /// that found the core busy, blocking waits, their total and longest wait in microseconds.
    /// Resets the counts; empty while they are off (`CODE-26`).
    #[func]
    pub fn take_core_lock_stats(&self) -> VarDictionary {
        let mut out = VarDictionary::new();
        for site in super::core_lock_stats::take() {
            let counts = PackedInt64Array::from(
                [site.tries, site.busy, site.waits, site.wait_us, site.wait_max_us]
                    .map(|value| value as i64)
                    .as_slice(),
            );
            out.set(
                format!("{}:{}", site.location.file(), site.location.line()),
                counts,
            );
        }
        out
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
