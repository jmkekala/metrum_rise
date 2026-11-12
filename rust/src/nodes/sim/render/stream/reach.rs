// SPDX-License-Identifier: GPL-2.0-only

//! The camera's reach (`RENDER-32-N`, `docs/terrain.md` "Streaming core" 5.3 item 2): where the
//! eye can be next, as sample eyes each with its travel, the camera travel along its path from
//! the current eye. Three shapes, all from the view and none from the frame rate:
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
//! chunk's travel-to-need is the travel of the first sample that wants it. The samples are a
//! function of the eye and the pivot alone, so the same view always gives the same order.
//! `O(samples)` per view, a fixed 88.

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
pub(crate) const SAMPLES: usize = 2 * ORBIT_STEPS + ZOOM_STEPS + 2 * PAN_DIRECTIONS;

/// One sample eye: its ground position (XZ) and the camera travel to it, metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ReachSample {
    /// The eye's ground position, `[x, z]`.
    pub(crate) eye: [f32; 2],
    /// Metres of camera travel from the current eye along the sample's path.
    pub(crate) travel: f64,
}

/// Writes `view`'s reach samples into `out` (cleared first), nearest travel first. `O(SAMPLES)`.
pub(crate) fn reach_samples(view: &CameraSample, out: &mut Vec<ReachSample>) {
    out.clear();
    out.reserve(SAMPLES);
    let pivot = [f64::from(view.pivot[0]), f64::from(view.pivot[2])];
    let offset = [
        f64::from(view.eye[0]) - pivot[0],
        f64::from(view.eye[2]) - pivot[1],
    ];
    let radius = offset[0].hypot(offset[1]);
    let at = |x: f64, z: f64| [x as f32, z as f32];
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
        });
    }
    let pan = PAN_SPEED_M_S * distance / PAN_REFERENCE_M * PAN_HORIZON_S;
    let eye = [f64::from(view.eye[0]), f64::from(view.eye[2])];
    for ring in [0.5, 1.0] {
        for direction in 0..PAN_DIRECTIONS {
            let angle = std::f64::consts::TAU * direction as f64 / PAN_DIRECTIONS as f64;
            let (sin, cos) = angle.sin_cos();
            out.push(ReachSample {
                eye: at(eye[0] + pan * ring * cos, eye[1] + pan * ring * sin),
                travel: pan * ring,
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

    #[test]
    fn the_samples_follow_the_three_shapes_in_travel_order() {
        let mut out = Vec::new();
        let view = view(800.0);
        reach_samples(&view, &mut out);
        assert_eq!(out.len(), SAMPLES);
        assert!(
            out.windows(2).all(|pair| pair[0].travel <= pair[1].travel),
            "nearest travel first"
        );
        let radius = f64::from(view.eye[0] - view.pivot[0]);
        let from_pivot = |sample: &ReachSample| {
            (f64::from(sample.eye[0]) - 100.0).hypot(f64::from(sample.eye[1]) + 20.0)
        };
        // Every orbit sample lies on the circle; the half turn is the opposite point.
        let half = out
            .iter()
            .filter(|s| (s.travel - radius * std::f64::consts::PI).abs() < 1e-6)
            .collect::<Vec<_>>();
        assert_eq!(half.len(), 2, "the half turn, reached either way");
        assert!(half.iter().all(|s| (from_pivot(s) - radius).abs() < 0.01
            && (f64::from(s.eye[0]) - (100.0 - radius)).abs() < 0.01));
        // The zoom segment ends at the pivot, after the whole distance.
        let last_zoom = out
            .iter()
            .find(|s| (s.travel - 800.0).abs() < 1e-6 && from_pivot(s) < 0.01);
        assert!(last_zoom.is_some(), "the zoom's last sample is the pivot");
        // The pan disc's outer ring: 240 m at 800 m.
        let pan = out
            .iter()
            .filter(|s| (s.travel - 240.0).abs() < 1e-9)
            .count();
        assert_eq!(pan, PAN_DIRECTIONS);
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
