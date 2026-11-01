// SPDX-License-Identifier: GPL-2.0-only

//! Allocation-free, analytically integrated camera response, independent of Godot and simulation.

use glam::DVec2;

const LN_20: f64 = 2.995_732_273_553_991;
// End tails well below a screen pixel, including the world editor's largest orbit.
const ZOOM_RELATIVE_EPSILON: f64 = 1.0e-6;
const PAN_STOP_SPEED_M_S: f64 = 0.001;

/// Displayed zoom and transient keyboard intent; independent of world coordinates.
pub(super) struct Motion {
    distance: f64,
    target_distance: f64,
    pan_velocity: DVec2,
    pan_request: DVec2,
}

/// Analytic displacement and whether the displayed f32 orbit distance changed this frame.
pub(super) struct Step {
    /// Integrated horizontal motion, in world metres.
    pub pan_offset: DVec2,
    /// True when the camera's projection and transform need the new distance.
    pub zoom_changed: bool,
}

impl Motion {
    /// Starts at a settled orbit with no keyboard intent.
    pub fn new(distance: f32) -> Self {
        Self {
            distance: distance.into(),
            target_distance: distance.into(),
            pan_velocity: DVec2::ZERO,
            pan_request: DVec2::ZERO,
        }
    }

    /// Returns the distance represented by the rendered f32 transform.
    pub fn distance(&self) -> f32 {
        self.distance as f32
    }

    /// Returns the destination on which subsequent wheel notches accumulate.
    pub fn target_distance(&self) -> f32 {
        self.target_distance as f32
    }

    /// Replaces both displayed and requested distance without a transition.
    pub fn set_distance(&mut self, distance: f32) {
        self.distance = distance.into();
        self.target_distance = distance.into();
    }

    /// Retargets zoom without moving the displayed view immediately.
    pub fn zoom_to(&mut self, distance: f32) {
        self.target_distance = distance.into();
    }

    /// Sets the velocity requested for the next advance only.
    pub fn request_pan(&mut self, velocity: DVec2) {
        self.pan_request = velocity;
    }

    /// Discards keyboard intent and braking when another owner takes the pivot.
    pub fn stop_pan(&mut self) {
        self.pan_request = DVec2::ZERO;
        self.pan_velocity = DVec2::ZERO;
    }

    /// Discards motion into a boundary while preserving motion along it.
    pub fn stop_pan_axes(&mut self, x: bool, z: bool) {
        if x {
            self.pan_velocity.x = 0.0;
            self.pan_request.x = 0.0;
        }
        if z {
            self.pan_velocity.y = 0.0;
            self.pan_request.y = 0.0;
        }
    }

    /// Stops at the displayed view without completing any outstanding target.
    pub fn cancel(&mut self) {
        self.stop_pan();
        self.target_distance = self.distance;
    }

    /// Integrates a positive finite delta. Responses are seconds to cover 95% of a step;
    /// nonpositive response times select immediate response.
    pub fn advance(
        &mut self,
        delta: f64,
        zoom_s: f64,
        acceleration_s: f64,
        braking_s: f64,
    ) -> Step {
        let old_distance = self.distance();
        if self.distance != self.target_distance {
            let residual = (self.distance - self.target_distance) * decay(delta, zoom_s);
            self.distance = if residual.abs() <= self.target_distance * ZOOM_RELATIVE_EPSILON {
                self.target_distance
            } else {
                self.target_distance + residual
            };
        }

        // A request is valid for one rendered frame. Missing input releases the keys;
        // interrupted input producers cannot leave the camera moving indefinitely.
        let target = std::mem::take(&mut self.pan_request);
        let response = if target == DVec2::ZERO {
            braking_s
        } else {
            acceleration_s
        };
        let mut pan_offset = DVec2::ZERO;
        if target != DVec2::ZERO || self.pan_velocity != DVec2::ZERO {
            let e = decay(delta, response);
            let difference = self.pan_velocity - target;
            // Integrate velocity over the full frame. Using only the new velocity here
            // makes acceleration distance depend on FPS even with exponential damping.
            pan_offset = target * delta + difference * (response.max(0.0) / LN_20) * (1.0 - e);
            self.pan_velocity = target + difference * e;
            if target == DVec2::ZERO
                && self.pan_velocity.length_squared() < PAN_STOP_SPEED_M_S.powi(2)
            {
                self.pan_velocity = DVec2::ZERO;
            }
        }
        Step {
            pan_offset,
            zoom_changed: old_distance != self.distance(),
        }
    }
}

fn decay(delta: f64, response: f64) -> f64 {
    if response > 0.0 {
        (-LN_20 * delta / response).exp()
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pan_integral_and_zoom_are_independent_of_frame_partition() {
        let run = |fps: usize| {
            let mut motion = Motion::new(400.0);
            motion.zoom_to(400.0 / 1.2);
            let mut position = DVec2::ZERO;
            for frame in 0..fps * 2 {
                if frame < fps {
                    motion.request_pan(DVec2::new(240.0, -100.0));
                }
                position += motion
                    .advance(1.0 / fps as f64, 0.15, 0.10, 0.06)
                    .pan_offset;
            }
            assert_eq!(motion.pan_velocity, DVec2::ZERO);
            assert_eq!(motion.distance(), motion.target_distance());
            (position, motion.distance())
        };
        let reference = run(60);
        for fps in [30, 120, 240] {
            let actual = run(fps);
            assert!(actual.0.distance(reference.0) < 0.00002);
            assert_eq!(actual.1, reference.1);
        }
    }

    #[test]
    fn pan_accelerates_brakes_and_cannot_retain_a_request() {
        let mut motion = Motion::new(400.0);
        motion.request_pan(DVec2::X * 240.0);
        let first = motion.advance(1.0 / 60.0, 0.15, 0.1, 0.06);
        assert!(first.pan_offset.x > 0.0 && first.pan_offset.x < 4.0);
        let velocity = motion.pan_velocity.x;
        motion.advance(1.0 / 60.0, 0.15, 0.1, 0.06);
        assert!(motion.pan_velocity.x > 0.0 && motion.pan_velocity.x < velocity);
        motion.cancel();
        assert_eq!(motion.advance(1.0, 0.15, 0.1, 0.06).pan_offset, DVec2::ZERO);
    }

    #[test]
    fn zoom_response_is_monotonic_retargetable_and_finishes_exactly() {
        let mut motion = Motion::new(400.0);
        motion.zoom_to(200.0);
        motion.advance(0.15, 0.15, 0.1, 0.06);
        assert!((motion.distance() - 210.0).abs() < 0.0001);
        let displayed = motion.distance();
        motion.zoom_to(500.0);
        assert_eq!(motion.distance(), displayed);
        motion.advance(1.0 / 60.0, 0.15, 0.1, 0.06);
        assert!(motion.distance() > displayed && motion.distance() < 500.0);
        motion.advance(2.0, 0.15, 0.1, 0.06);
        assert_eq!(motion.distance(), 500.0);
        motion.zoom_to(10.0);
        motion.cancel();
        assert!(!motion.advance(1.0, 0.15, 0.1, 0.06).zoom_changed);
    }

    #[test]
    fn boundary_discards_blocked_axis_and_preserves_slide() {
        let mut motion = Motion::new(400.0);
        motion.request_pan(DVec2::new(240.0, 100.0));
        motion.advance(0.1, 0.15, 0.1, 0.06);
        let tangent = motion.pan_velocity.y;
        motion.request_pan(DVec2::new(240.0, 100.0));
        motion.stop_pan_axes(true, false);
        assert_eq!(motion.pan_velocity, DVec2::new(0.0, tangent));
        assert_eq!(motion.pan_request, DVec2::new(0.0, 100.0));
        motion.request_pan(DVec2::new(-240.0, 100.0));
        assert!(motion.advance(1.0 / 60.0, 0.15, 0.1, 0.06).pan_offset.x < 0.0);
        motion.stop_pan_axes(false, true);
        assert_eq!(motion.pan_velocity.y, 0.0);
    }

    #[test]
    fn zero_response_is_immediate_and_idle_does_not_move() {
        let mut motion = Motion::new(400.0);
        motion.zoom_to(100.0);
        motion.request_pan(DVec2::X * 240.0);
        let step = motion.advance(0.1, 0.0, 0.0, 0.0);
        assert_eq!(step.pan_offset, DVec2::X * 24.0);
        assert_eq!(motion.distance(), 100.0);
        let idle = motion.advance(0.1, 0.0, 0.0, 0.0);
        assert_eq!(idle.pan_offset, DVec2::ZERO);
        assert!(!idle.zoom_changed);
    }
}
