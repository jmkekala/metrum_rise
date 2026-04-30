#![allow(missing_docs)]
//! Godot camera node for RTS-style controls.
//!
//! Provides orbit, pan, and zoom functionality tailored for city simulation.

use crate::nodes::simulation_node::SimulationNode;
use godot::classes::camera_3d::ProjectionType;
use godot::classes::{Camera3D, ICamera3D, InputEvent};
use godot::prelude::*;

#[derive(GodotClass)]
#[class(base=Camera3D)]
/// A third-person orbit camera controlled via WASD, MMB, and Scroll Wheel.
pub struct CameraNode {
    /// Panning speed in meters per second.
    #[export]
    speed: f32,
    /// Mouse rotation sensitivity.
    #[export]
    sensitivity: f32,
    /// Multiplier for zoom distance changes.
    #[export]
    zoom_speed: f32,
    /// Minimum allowed orbit distance.
    #[export]
    min_distance: f32,
    /// Maximum allowed orbit distance.
    #[export]
    max_distance: f32,
    /// Near clip plane in metres.
    #[export]
    near_clip_m: f32,
    /// Minimum far clip distance in metres.
    #[export]
    min_far_m: f32,
    /// Extra far clip margin beyond the current distance.
    #[export]
    far_margin_m: f32,
    /// Focus framing multiplier applied to `focus_on()`.
    #[export]
    focus_padding_mult: f32,
    /// Mouse-drag pan scale relative to current zoom distance.
    #[export]
    drag_pan_scale: f32,
    /// Minimum world-space pan step when dragging with the mouse.
    #[export]
    drag_pan_min_step: f32,
    /// Whether this camera should stay above the terrain surface.
    #[export]
    terrain_clearance_enabled: bool,
    /// Debug inspection mode: disables terrain clearance and permits orbiting below the pivot.
    debug_under_terrain_enabled: bool,
    /// Minimum focus-point clearance above terrain in metres.
    #[export]
    terrain_pivot_clearance_m: f32,
    /// Minimum camera-position clearance above terrain in metres.
    #[export]
    terrain_camera_clearance_m: f32,
    /// Whether to use orthographic projection for a classic "flat" look.
    #[export]
    orthogonal: bool,

    /// The focal point the camera is looking at.
    pivot: Vector3,
    /// Horizontal rotation in radians.
    yaw: f32,
    /// Vertical rotation in radians.
    pitch: f32,
    /// Distance from the pivot point.
    distance: f32,
    /// Cached terrain/runtime source for height sampling.
    simulation_node: Option<Gd<SimulationNode>>,

    base: Base<Camera3D>,
}

#[godot_api]
impl ICamera3D for CameraNode {
    fn init(base: Base<Camera3D>) -> Self {
        Self {
            base,
            speed: 240.0,
            sensitivity: 0.003,
            zoom_speed: 1.2,
            min_distance: 10.0,
            max_distance: 1000.0,
            near_clip_m: 0.5,
            min_far_m: 5000.0,
            far_margin_m: 1000.0,
            focus_padding_mult: 2.5,
            drag_pan_scale: 0.0025,
            drag_pan_min_step: 0.5,
            terrain_clearance_enabled: false,
            debug_under_terrain_enabled: false,
            terrain_pivot_clearance_m: 0.25,
            terrain_camera_clearance_m: 1.5,
            pivot: Vector3::new(0.0, 0.0, 0.0),
            yaw: -0.785,   // -45 degrees
            pitch: -0.785, // -45 degrees
            distance: 400.0,
            simulation_node: None,
            orthogonal: false,
        }
    }

    fn ready(&mut self) {
        if self.orthogonal {
            self.base_mut().set_projection(ProjectionType::ORTHOGONAL);
            let distance = self.distance;
            self.base_mut().set_size(distance * 0.5);
        } else {
            self.base_mut().set_projection(ProjectionType::PERSPECTIVE);
        }
        self.update_camera_transform();
    }

    fn input(&mut self, _event: Gd<InputEvent>) {
        // Input handling moved to InputManager.gd
    }

    fn process(&mut self, _delta: f64) {
        // Input handling moved to InputManager.gd for centralized routing
    }
}

#[godot_api]
impl CameraNode {
    /// Pans the camera on the XZ plane relative to the current view.
    #[func]
    pub fn pan(&mut self, direction: Vector3, speed_mult: f32, delta: f32) {
        if direction.length() > 0.0 {
            let yaw_rot = Basis::from_euler(EulerOrder::YXZ, Vector3::new(0.0, self.yaw, 0.0));
            // Scale pan speed by zoom distance: faster when zoomed out, slower when close
            let zoom_factor = (self.distance / 400.0).max(0.1);
            let move_vec =
                (yaw_rot * direction.normalized()) * self.speed * speed_mult * zoom_factor * delta;
            self.pivot += move_vec;
            self.update_camera_transform();
        }
    }

    /// Pans the camera on the ground plane from a screen-space drag.
    #[func]
    pub fn pan_screen(&mut self, mouse_delta: Vector2) {
        if mouse_delta.length_squared() <= 0.0 {
            return;
        }
        let yaw_rot = Basis::from_euler(EulerOrder::YXZ, Vector3::new(0.0, self.yaw, 0.0));
        let right = yaw_rot * Vector3::RIGHT;
        let forward = yaw_rot * Vector3::new(0.0, 0.0, -1.0);
        let pan_scale = (self.distance * self.drag_pan_scale).max(self.drag_pan_min_step);
        self.pivot -= right * mouse_delta.x * pan_scale;
        self.pivot -= forward * mouse_delta.y * pan_scale;
        self.update_camera_transform();
    }

    /// Rotates the camera around the focal point.
    #[func]
    pub fn orbit(&mut self, mouse_delta: Vector2) {
        if mouse_delta.length() > 0.0 {
            let delta_v = mouse_delta * self.sensitivity;
            self.yaw -= delta_v.x;
            self.pitch -= delta_v.y;
            self.pitch = Self::clamp_pitch(self.pitch, self.debug_under_terrain_enabled);
            self.update_camera_transform();
        }
    }

    /// Zooms the camera in or out.
    #[func]
    pub fn zoom(&mut self, amount: f32) {
        if amount != 0.0 {
            if amount > 0.0 {
                self.distance /= self.zoom_speed;
            } else {
                self.distance *= self.zoom_speed;
            }
            self.distance = self.distance.clamp(self.min_distance, self.max_distance);

            if self.orthogonal {
                let distance = self.distance;
                self.base_mut().set_size(distance * 0.5);
            }
            self.update_camera_transform();
        }
    }

    /// Frames a spherical area around `center` using the configured focus padding.
    #[func]
    pub fn focus_on(&mut self, center: Vector3, radius: f32) {
        self.pivot = center;
        self.distance = (radius * self.focus_padding_mult)
            .max(self.min_distance)
            .clamp(self.min_distance, self.max_distance);
        if self.orthogonal {
            let size = self.distance * 0.5;
            self.base_mut().set_size(size);
        }
        self.update_camera_transform();
    }

    /// Sets the allowed zoom-distance range for the current scene.
    #[func]
    pub fn set_distance_bounds(&mut self, min_distance: f32, max_distance: f32) {
        self.min_distance = min_distance.max(0.01);
        self.max_distance = max_distance.max(self.min_distance);
        self.distance = self.distance.clamp(self.min_distance, self.max_distance);
        self.update_camera_transform();
    }

    /// Sets the clip-plane and far-distance policy for the current scene.
    #[func]
    pub fn set_clip_policy(&mut self, near_clip_m: f32, min_far_m: f32, far_margin_m: f32) {
        self.near_clip_m = near_clip_m.max(0.01);
        self.min_far_m = min_far_m.max(self.near_clip_m + 1.0);
        self.far_margin_m = far_margin_m.max(0.0);
        self.update_camera_transform();
    }

    /// Sets the focus framing multiplier for `focus_on()`.
    #[func]
    pub fn set_focus_padding(&mut self, focus_padding_mult: f32) {
        self.focus_padding_mult = focus_padding_mult.max(1.0);
    }

    /// Enables or disables terrain clearance and sets the shared world-camera offsets.
    #[func]
    pub fn set_terrain_clearance_policy(
        &mut self,
        enabled: bool,
        pivot_clearance_m: f32,
        camera_clearance_m: f32,
    ) {
        self.terrain_clearance_enabled = enabled;
        self.terrain_pivot_clearance_m = pivot_clearance_m.max(0.0);
        self.terrain_camera_clearance_m = camera_clearance_m.max(self.terrain_pivot_clearance_m);
        self.update_camera_transform();
    }

    /// Enables debug inspection below terrain by disabling terrain-follow clamping and relaxing pitch.
    #[func]
    pub fn set_debug_under_terrain_enabled(&mut self, enabled: bool) {
        self.debug_under_terrain_enabled = enabled;
        self.pitch = Self::clamp_pitch(self.pitch, enabled);
        self.update_camera_transform();
    }

    fn resolve_simulation_node_if_needed(&mut self) {
        if self.simulation_node.is_some() {
            return;
        }
        let Some(parent) = self.base().get_parent() else {
            return;
        };
        self.simulation_node = parent.try_get_node_as::<SimulationNode>("SimulationNode");
    }

    fn terrain_height_at(&mut self, world_x: f32, world_z: f32) -> Option<f32> {
        if !self.terrain_clearance_enabled || self.debug_under_terrain_enabled {
            return None;
        }
        self.resolve_simulation_node_if_needed();
        self.simulation_node.as_ref().map(|simulation| {
            simulation
                .bind()
                .get_height_at(Vector2::new(world_x, world_z))
        })
    }

    fn terrain_follow_pivot_y(
        offset_y: f32,
        pivot_surface_y: f32,
        camera_surface_y: Option<f32>,
        pivot_clearance_m: f32,
        camera_clearance_m: f32,
    ) -> f32 {
        let terrain_anchored_pivot_y = pivot_surface_y + pivot_clearance_m;
        camera_surface_y
            .map(|surface_y| {
                terrain_anchored_pivot_y.max(surface_y + camera_clearance_m - offset_y)
            })
            .unwrap_or(terrain_anchored_pivot_y)
    }

    fn clamp_pitch(pitch: f32, debug_under_terrain_enabled: bool) -> f32 {
        if debug_under_terrain_enabled {
            pitch.clamp(-1.5, 1.5)
        } else {
            pitch.clamp(-1.5, -0.1)
        }
    }

    fn update_camera_transform(&mut self) {
        let yaw_basis = Basis::from_euler(EulerOrder::YXZ, Vector3::new(0.0, self.yaw, 0.0));
        let pitch_basis = Basis::from_euler(EulerOrder::YXZ, Vector3::new(self.pitch, 0.0, 0.0));
        let rotation = yaw_basis * pitch_basis;

        let offset = rotation * Vector3::new(0.0, 0.0, self.distance);
        let mut pivot = self.pivot;
        if let Some(pivot_surface_y) = self.terrain_height_at(pivot.x, pivot.z) {
            let camera_surface_y = self.terrain_height_at(pivot.x + offset.x, pivot.z + offset.z);
            pivot.y = Self::terrain_follow_pivot_y(
                offset.y,
                pivot_surface_y,
                camera_surface_y,
                self.terrain_pivot_clearance_m,
                self.terrain_camera_clearance_m,
            );
            self.pivot.y = pivot.y;
        }
        let new_pos = pivot + offset;
        let near_clip = self.near_clip_m;
        let far_clip = (self.distance * 4.0 + self.far_margin_m).max(self.min_far_m);
        self.base_mut().set_near(near_clip);
        self.base_mut().set_far(far_clip);
        self.base_mut().set_global_position(new_pos);
        self.base_mut().look_at(pivot);
    }
}

#[cfg(test)]
mod tests {
    use super::CameraNode;

    #[test]
    fn terrain_follow_pivot_y_keeps_camera_above_surface_and_resets_to_ground_anchor() {
        let lifted = CameraNode::terrain_follow_pivot_y(-20.0, 100.0, Some(118.0), 0.25, 1.5);
        assert!((lifted - 139.5).abs() < 0.001);

        let anchored = CameraNode::terrain_follow_pivot_y(-20.0, 100.0, Some(70.0), 0.25, 1.5);
        assert!((anchored - 100.25).abs() < 0.001);
    }

    #[test]
    fn debug_under_terrain_mode_allows_camera_below_pivot_pitch() {
        assert!((CameraNode::clamp_pitch(0.75, true) - 0.75).abs() < 0.001);
        assert!((CameraNode::clamp_pitch(0.75, false) + 0.1).abs() < 0.001);
    }
}
