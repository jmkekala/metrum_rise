// SPDX-License-Identifier: GPL-2.0-only

//! The camera's reach (`RENDER-32-N`, `docs/terrain.md` "Streaming core" 5.3 item 2): where the
//! eye can be next, as sample eyes each with its travel, the camera travel along its path from
//! the current eye, and its spacing, how far its shape's neighbouring samples lie. Three shapes,
//! all from the view and none from the frame rate:
//!
//! - **The orbit circle** about the pivot at the current distance (a yaw turn): the eye's ground
//!   offset from the pivot turned by `k x 180 / ORBIT_STEPS` degrees each way, travel the arc.
//! - **The zoom line** through the eye and the pivot (a zoom in or out): the eye at the
//!   distances the camera's zoom steps reach, the current distance divided (in, towards the
//!   pivot, to the camera's minimum distance) or multiplied (out, away from it, to its maximum)
//!   by its zoom rate once a step, each end taken exactly; travel the change of distance. At most
//!   [`ZOOM_LINE_SAMPLES`]: a range that needs more takes a faster rate.
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
/// Most samples on the zoom line, in and out together (the gameplay camera's 10 m to 40 km at
/// its zoom rate of 1.2 a step is 46).
const ZOOM_LINE_SAMPLES: usize = 64;
/// Directions on each ring of the pan disc.
const PAN_DIRECTIONS: usize = 8;
/// `CameraNode`'s keyboard pan speed at its reference distance, metres a second.
const PAN_SPEED_M_S: f64 = 240.0;
/// The orbit distance at which the pan moves at `PAN_SPEED_M_S` (`INITIAL_DISTANCE_M`).
const PAN_REFERENCE_M: f64 = 400.0;
/// The pan disc's horizon, seconds of keyboard pan.
const PAN_HORIZON_S: f64 = 0.5;

/// Most samples a view gives: both orbit directions, the zoom line and the pan disc's two rings.
pub(crate) const MAX_SAMPLES: usize = 2 * ORBIT_STEPS + ZOOM_LINE_SAMPLES + 2 * PAN_DIRECTIONS;

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

/// Writes `view`'s reach samples into `out` (cleared first), nearest travel first.
/// `O(MAX_SAMPLES)`.
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
    // The chord between neighbours `steps` to a half turn apart on a circle of radius `ring`.
    let chord =
        |ring: f64, steps: usize| 2.0 * ring * (std::f64::consts::PI / steps as f64 * 0.5).sin();
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
    zoom_line(view, pivot, offset, out);
    let distance = f64::from(view.distance);
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

// The zoom line's samples: the eye at `distance / rate^k` towards the pivot down to the minimum
// distance and `distance x rate^k` away from it up to the maximum, each end exact. The eye's
// ground offset from the pivot scales with the distance (the pitch holds while zooming).
fn zoom_line(view: &CameraSample, pivot: [f64; 2], offset: [f64; 2], out: &mut Vec<ReachSample>) {
    let distance = f64::from(view.distance);
    let (low, high) = (f64::from(view.min_distance), f64::from(view.max_distance));
    let usable = distance > 0.0 && low > 0.0 && high >= low;
    if !usable {
        return;
    }
    // The steps the whole range takes at the camera's rate; a rate slower than the bound allows
    // is raised to it (`ZOOM_LINE_SAMPLES - 4`: a step past each end and rounding).
    let range = (high / low).ln();
    let rate = f64::from(view.zoom_rate)
        .ln()
        .max(range / (ZOOM_LINE_SAMPLES - 4) as f64);
    if rate <= 0.0 {
        return;
    }
    let ground = |share: f64| [pivot[0] + offset[0] * share, pivot[1] + offset[1] * share];
    // Ground metres per metre of distance.
    let slope = offset[0].hypot(offset[1]) / distance;
    for direction in [-1.0, 1.0] {
        let end = if direction < 0.0 { low } else { high };
        let mut previous = distance;
        let mut step = 1.0;
        while (end - previous) * direction > 0.0 {
            let next = (distance * (direction * rate * step).exp()).clamp(low, high);
            let after = (next * (direction * rate).exp()).clamp(low, high);
            let [x, z] = ground(next / distance);
            out.push(ReachSample {
                eye: [x as f32, z as f32],
                travel: (next - distance).abs(),
                spacing: (next - previous).abs().max((after - next).abs()) * slope,
            });
            previous = next;
            step += 1.0;
        }
    }
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
            min_distance: 10.0,
            max_distance: 40_000.0,
            zoom_rate: 1.2,
        }
    }

    fn from_pivot(sample: &ReachSample) -> f64 {
        (f64::from(sample.eye[0]) - 100.0).hypot(f64::from(sample.eye[1]) + 20.0)
    }

    // The zoom line's samples: those on the pivot's ray through the eye (the orbit's are off it
    // but the half turn, the pan's at their own travels).
    fn on_line(view: &CameraSample, out: &[ReachSample]) -> Vec<ReachSample> {
        let ground = f64::from(view.eye[0] - view.pivot[0]);
        out.iter()
            .filter(|s| (f64::from(s.eye[1]) + 20.0).abs() < 1e-3 && f64::from(s.eye[0]) > 100.0)
            .filter(|s| {
                let share = (f64::from(s.eye[0]) - 100.0) / ground;
                (s.travel - (share - 1.0).abs() * f64::from(view.distance)).abs() < 0.05
            })
            .copied()
            .collect()
    }

    #[test]
    fn the_samples_follow_the_three_shapes_in_travel_order() {
        let mut out = Vec::new();
        let view = view(800.0);
        reach_samples(&view, &mut out);
        // 800 m to 10 m in at 1.2 a step is 25 samples, to 40 km out 22.
        assert_eq!(out.len(), 2 * ORBIT_STEPS + 25 + 22 + 2 * PAN_DIRECTIONS);
        assert!(out.len() <= MAX_SAMPLES);
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
        // The zoom line's inward end is the eye at the minimum distance, after 790 m.
        let last_in = out
            .iter()
            .find(|s| (s.travel - 790.0).abs() < 1e-6)
            .expect("the inward end");
        assert!((from_pivot(last_in) - radius / 80.0).abs() < 0.01);
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
    fn the_zoom_out_ray_steps_at_the_zoom_rate_to_the_maximum_distance() {
        let mut out = Vec::new();
        let view = view(800.0);
        reach_samples(&view, &mut out);
        let outward: Vec<_> = on_line(&view, &out)
            .into_iter()
            .filter(|s| from_pivot(s) > 800.0)
            .collect();
        assert_eq!(outward.len(), 22, "800 m x 1.2^k below 40 km, then 40 km");
        let ground = f64::from(view.eye[0] - view.pivot[0]) / 800.0;
        for (k, sample) in outward.iter().enumerate() {
            let distance = (800.0 * f64::from(1.2f32).powi(k as i32 + 1)).min(40_000.0);
            assert!(
                (sample.travel - (distance - 800.0)).abs() < 1e-6,
                "travel the distance gained: {k} {} {distance}",
                sample.travel
            );
            assert!(
                (from_pivot(sample) - distance * ground).abs() < 0.05,
                "on the ray away from the pivot, the offset scaled with the distance"
            );
        }
        let last = outward.last().unwrap();
        assert!((last.travel - 39_200.0).abs() < 1e-6, "the maximum distance exactly");
        // Spacing: the larger of the steps to its neighbours on the line, on the ground.
        let first = outward[0];
        let rate = f64::from(1.2f32);
        assert!((first.spacing - 800.0 * rate * (rate - 1.0) * ground).abs() < 1e-6);
        // At the maximum distance there is no ray out; at the minimum none in.
        let mut far = Vec::new();
        reach_samples(&self::view(40_000.0), &mut far);
        let far_view = self::view(40_000.0);
        assert!(on_line(&far_view, &far).iter().all(|s| from_pivot(s) < 40_000.0 * 0.995));
        // A slow rate is raised so the line keeps its bound; no range, no line.
        let mut slow = self::view(800.0);
        slow.zoom_rate = 1.001;
        reach_samples(&slow, &mut out);
        assert!(out.len() <= MAX_SAMPLES);
        // 8.29 e-folds over 60 steps: 32 in, 29 out.
        assert_eq!(on_line(&slow, &out).len(), 61);
        let mut fixed = self::view(800.0);
        fixed.min_distance = 800.0;
        fixed.max_distance = 800.0;
        reach_samples(&fixed, &mut out);
        assert_eq!(out.len(), 2 * ORBIT_STEPS + 2 * PAN_DIRECTIONS);
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
