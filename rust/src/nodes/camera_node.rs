use godot::prelude::*;
use godot::classes::{Camera3D, ICamera3D, Input, InputEvent, InputEventMouseButton};
use godot::global::{Key, MouseButton};

#[derive(GodotClass)]
#[class(base=Camera3D)]
pub struct CameraNode {
    #[export]
    speed: f32,
    #[export]
    sensitivity: f32,
    base: Base<Camera3D>,
}

#[godot_api]
impl ICamera3D for CameraNode {
    fn init(base: Base<Camera3D>) -> Self {
        Self { 
            base,
            speed: 20.0,
            sensitivity: 0.003,
        }
    }

    fn input(&mut self, event: Gd<InputEvent>) {
        if let Ok(mouse_event) = event.try_cast::<InputEventMouseButton>() {
            if mouse_event.is_pressed() {
                let zoom_amount = self.speed * 0.1; // Discrete jump per tick
                let basis = self.base().get_global_transform().basis;
                let mut move_vec = Vector3::ZERO;
                
                match mouse_event.get_button_index() {
                    MouseButton::WHEEL_UP => {
                        move_vec = basis * Vector3::new(0.0, 0.0, -zoom_amount);
                    }
                    MouseButton::WHEEL_DOWN => {
                        move_vec = basis * Vector3::new(0.0, 0.0, zoom_amount);
                    }
                    _ => {}
                }
                
                if move_vec.length() > 0.0 {
                    let new_pos = self.base().get_global_position() + move_vec;
                    self.base_mut().set_global_position(new_pos);
                }
            }
        }
    }

    fn process(&mut self, delta: f64) {
        let mut input = Input::singleton();
        let mut velocity = Vector3::ZERO;

        // WASD Movement
        if input.is_key_pressed(Key::W) { velocity.z -= 1.0; }
        if input.is_key_pressed(Key::S) { velocity.z += 1.0; }
        if input.is_key_pressed(Key::A) { velocity.x -= 1.0; }
        if input.is_key_pressed(Key::D) { velocity.x += 1.0; }
        if input.is_key_pressed(Key::Q) { velocity.y += 1.0; }
        if input.is_key_pressed(Key::E) { velocity.y -= 1.0; }

        if velocity.length() > 0.0 {
            velocity = velocity.normalized() * self.speed * delta as f32;
            let basis = self.base().get_global_transform().basis;
            let move_vec = basis * velocity;
            let new_pos = self.base().get_global_position() + move_vec;
            self.base_mut().set_global_position(new_pos);
        }
        
        // MMB Rotation - Polling is fine for held buttons
        if input.is_mouse_button_pressed(MouseButton::MIDDLE) {
            let mouse_vel = input.get_last_mouse_velocity();
            if mouse_vel.length() > 0.0 {
                let delta_v = mouse_vel * delta as f32 * self.sensitivity;
                self.base_mut().rotate_y(-delta_v.x);
                self.base_mut().rotate_object_local(Vector3::new(1.0, 0.0, 0.0), -delta_v.y);
            }
        }
    }
}
