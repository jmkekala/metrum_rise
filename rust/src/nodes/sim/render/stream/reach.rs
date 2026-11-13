// SPDX-License-Identifier: GPL-2.0-only

//! The camera's reach (`RENDER-32-N`, `docs/terrain.md` "Streaming core" 5.3 item 2): where the
//! eye can be next, as sample eyes each with its travel, the camera travel along its path from
//! the current eye, and its spacing, how far its shape's neighbouring samples lie. Three shapes,
//! all from the view and none from the frame rate:
//!
//! - **The orbit circle** about the pivot at the current distance (a yaw turn): the eye's ground
//!   offset from the pivot turned by `k x 180 / ORBIT_STEPS` degrees each way, travel the arc.
//! - **The eye-to-pivot segment** (a zoom in): the eye moved `j / ZOOM_STEPS` of the way to the
//!   pivot, travel that share of the distance.
//! - **The pan disc** about the eye of radius pan speed x 0.5 s (`PAN_SPEED_M_S` x distance /
//!   `PAN_REFERENCE_M`, `CameraNode::pan`'s scaling: 240 m at 800 m), sampled on two rings of
//!   `PAN_DIRECTIONS` points, travel the ring's radius.
//!
//! A layer that prepares ahead takes the samples in travel order, nearest first (ties in the
//! order the shapes are listed), and asks its own rule which chunks each sample eye wants: a
//! chunk's travel-to-need is the travel of the first sample that wants it. A camera on a shape
//! between two of its samples is within the spacing of both, so a rule of the ground distance
//! from the eye (which moves no faster than the eye) takes there a value it takes within the
//! spacing of a sample. The samples are a function of the view alone, so the same view always
//! gives the same order. `O(samples)` per view, at most [`MAX_SAMPLES`].

use super::view::CameraSample;

/// Orbit samples each way over half a turn.
const ORBIT_STEPS: usize = 24;
/// Samples along the eye-to-pivot segment.
const ZOOM_STEPS: usize = 16;
/// Directions on each ring of the pan disc.
const PAN_DIRECTIONS: usize = 8;
/// `CameraNode`'s keyboard pan speed at its reference distance, metres a second.
const PAN_SPEED_M_S: f64 = 240.0;
/// The orbit distance at which the pan moves at `PAN_SPEED_M_S` (`INITIAL_DISTANCE_M`).
const PAN_REFERENCE_M: f64 = 400.0;
/// The pan disc's horizon, seconds of keyboard pan.
const PAN_HORIZON_S: f64 = 0.5;

/// Samples a view gives: both orbit directions, the zoom segment and the pan disc's two rings.
pub(crate) const MAX_SAMPLES: usize = 2 * ORBIT_STEPS + ZOOM_STEPS + 2 * PAN_DIRECTIONS;

/// One sample eye: its ground position (XZ), the camera travel to it and its spacing, metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ReachSample {
    /// The eye's ground position, `[x, z]`.
    pub(crate) eye: [f32; 2],
    /// Metres of camera travel from the current eye along the sample's path.
    pub(crate) travel: f64,
    /// The ground distance to the farthest of its shape's neighbouring samples, metres: a camera
    /// on the shape between this sample and a neighbour is within it.
    pub(crate) spacing: f64,
}

/// Writes `view`'s reach samples into `out` (cleared first), nearest travel first. `O(MAX_SAMPLES)`.
pub(crate) fn reach_samples(view: &CameraSample, out: &mut Vec<ReachSample>) {
    out.clear();
    out.reserve(MAX_SAMPLES);
    let pivot = [f64::from(view.pivot[0]), f64::from(view.pivot[2])];
    let offset = [
        f64::from(view.eye[0]) - pivot[0],
        f64::from(view.eye[2]) - pivot[1],
    ];
    let radius = offset[0].hypot(offset[1]);
    let at = |x: f64, z: f64| [x as f32, z as f32];
    let chord = |ring: f64, steps: usize| 2.0 * ring * (std::f64::consts::PI / steps as f64 * 0.5).sin();
    let orbit_spacing = chord(radius, ORBIT_STEPS);
    for step in 1..=ORBIT_STEPS {
        let angle = std::f64::consts::PI * step as f64 / ORBIT_STEPS as f64;
        for turn in [angle, -angle] {
            let (sin, cos) = turn.sin_cos();
            out.push(ReachSample {
                eye: at(
                    pivot[0] + offset[0] * cos - offset[1] * sin,
                    pivot[1] + offset[0] * sin + offset[1] * cos,
                ),
                travel: radius * angle,
                spacing: orbit_spacing,
            });
        }
    }
    let distance = f64::from(view.distance);
    for step in 1..=ZOOM_STEPS {
        let share = step as f64 / ZOOM_STEPS as f64;
        out.push(ReachSample {
            eye: at(
                pivot[0] + offset[0] * (1.0 - share),
                pivot[1] + offset[1] * (1.0 - share),
            ),
            travel: distance * share,
            spacing: radius / ZOOM_STEPS as f64,
        });
    }
    let pan = PAN_SPEED_M_S * distance / PAN_REFERENCE_M * PAN_HORIZON_S;
    let eye = [f64::from(view.eye[0]), f64::from(view.eye[2])];
    for ring in [0.5, 1.0] {
        // The neighbours on the ring, and the other ring along the same direction.
        let spacing = chord(pan * ring, PAN_DIRECTIONS / 2).max(pan * 0.5);
        for direction in 0..PAN_DIRECTIONS {
            let angle = std::f64::consts::TAU * direction as f64 / PAN_DIRECTIONS as f64;
            let (sin, cos) = angle.sin_cos();
            out.push(ReachSample {
                eye: at(eye[0] + pan * ring * cos, eye[1] + pan * ring * sin),
                travel: pan * ring,
                spacing,
            });
        }
    }
    // Stable: equal travels keep the shapes' order.
    out.sort_by(|a, b| a.travel.total_cmp(&b.travel));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(distance: f32) -> CameraSample {
        // Pitch -0.1 rad: the eye about `distance` from the pivot on the ground, 10 % up.
        let ground = distance * 0.1f32.cos();
        CameraSample {
            eye: [100.0 + ground, 50.0 + distance * 0.1f32.sin(), -20.0],
            pivot: [100.0, 50.0, -20.0],
            yaw: 0.0,
            pitch: -0.1,
            distance,
        }
    }

    fn from_pivot(sample: &ReachSample) -> f64 {
        (f64::from(sample.eye[0]) - 100.0).hypot(f64::from(sample.eye[1]) + 20.0)
    }

    #[test]
    fn the_samples_follow_the_three_shapes_in_travel_order() {
        let mut out = Vec::new();
        let view = view(800.0);
        reach_samples(&view, &mut out);
        assert_eq!(out.len(), MAX_SAMPLES);
        assert!(
            out.windows(2).all(|pair| pair[0].travel <= pair[1].travel),
            "nearest travel first"
        );
        let radius = f64::from(view.eye[0] - view.pivot[0]);
        // Every orbit sample lies on the circle; the half turn is the opposite point.
        let half = out
            .iter()
            .filter(|s| (s.travel - radius * std::f64::consts::PI).abs() < 1e-6)
            .collect::<Vec<_>>();
        assert_eq!(half.len(), 2, "the half turn, reached either way");
        assert!(half.iter().all(|s| (from_pivot(s) - radius).abs() < 0.01
            && (f64::from(s.eye[0]) - (100.0 - radius)).abs() < 0.01));
        // The orbit's spacing is the chord of a step.
        let chord = 2.0 * radius * (std::f64::consts::PI / 48.0).sin();
        assert!(half.iter().all(|s| (s.spacing - chord).abs() < 1e-9));
        // The zoom segment ends at the pivot, after the whole distance.
        let last_zoom = out
            .iter()
            .find(|s| (s.travel - 800.0).abs() < 1e-6 && from_pivot(s) < 0.01);
        assert!(last_zoom.is_some(), "the zoom's last sample is the pivot");
        assert!((last_zoom.unwrap().spacing - radius / 16.0).abs() < 1e-9);
        // The pan disc's outer ring: 240 m at 800 m; its spacing the chord between directions.
        let pan: Vec<_> = out
            .iter()
            .filter(|s| (s.travel - 240.0).abs() < 1e-9)
            .collect();
        assert_eq!(pan.len(), PAN_DIRECTIONS);
        let pan_chord = 2.0 * 240.0 * (std::f64::consts::PI / 8.0).sin();
        assert!(pan.iter().all(|s| (s.spacing - pan_chord).abs() < 1e-9));
    }

    #[test]
    fn the_same_view_gives_the_same_samples_and_the_pan_scales_with_distance() {
        let (mut a, mut b) = (Vec::new(), Vec::new());
        reach_samples(&view(400.0), &mut a);
        reach_samples(&view(400.0), &mut b);
        assert_eq!(a, b);
        assert!(
            a.iter().any(|s| (s.travel - 120.0).abs() < 1e-9),
            "120 m of pan at 400 m"
        );
    }
}
