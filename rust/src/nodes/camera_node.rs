//! Godot camera node for RTS-style controls.
//!
//! Provides orbit, pan, and zoom functionality tailored for city simulation.


use godot::prelude::*;
use godot::classes::{Camera3D, ICamera3D, Input, InputEvent, InputEventMouseButton};
use godot::classes::camera_3d::ProjectionType;
use godot::global::{Key, MouseButton};

/// A third-person orbit camera controlled via WASD, MMB, and Scroll Wheel.
#[derive(GodotClass)]
#[class(base=Camera3D)]
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
            pivot: Vector3::new(0.0, 0.0, 0.0),
            yaw: -0.785, // -45 degrees
            pitch: -0.785, // -45 degrees
            distance: 400.0,
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
            let move_vec = (yaw_rot * direction.normalized()) * self.speed * speed_mult * zoom_factor * delta;
            self.pivot += move_vec;
            self.update_camera_transform();
        }
    }

    /// Rotates the camera around the focal point.
    #[func]
    pub fn orbit(&mut self, mouse_delta: Vector2, delta: f32) {
        if mouse_delta.length() > 0.0 {
            let delta_v = mouse_delta * delta * self.sensitivity;
            self.yaw -= delta_v.x;
            self.pitch -= delta_v.y;
            self.pitch = self.pitch.clamp(-1.5, -0.1); 
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
            self.distance = self.distance.clamp(10.0, 1000.0);
            
            if self.orthogonal {
                let distance = self.distance;
                self.base_mut().set_size(distance * 0.5);
            }
            self.update_camera_transform();
        }
    }

    fn update_camera_transform(&mut self) {
        let yaw_basis = Basis::from_euler(EulerOrder::YXZ, Vector3::new(0.0, self.yaw, 0.0));
        let pitch_basis = Basis::from_euler(EulerOrder::YXZ, Vector3::new(self.pitch, 0.0, 0.0));
        let rotation = yaw_basis * pitch_basis;
        
        let offset = rotation * Vector3::new(0.0, 0.0, self.distance);
        let pivot = self.pivot;
        let new_pos = pivot + offset;
        self.base_mut().set_global_position(new_pos);
        self.base_mut().look_at(pivot);
    }
}

