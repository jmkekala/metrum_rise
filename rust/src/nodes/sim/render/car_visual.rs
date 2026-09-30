// SPDX-License-Identifier: GPL-2.0-only

//! Render-side smoothing of car poses between simulation snapshots.
//!
//! Presentation state only: it is never saved and never read by the simulation. Every frame
//! moves each visible car's drawn pose toward its latest snapshot pose, keyed by the stable
//! render ID, so cars do not visibly step at the simulation tick rate.

use glam::{Mat3, Quat, Vec3};
use rayon::prelude::*;
use std::collections::HashMap;

/// Fraction of the remaining origin gap closed per second of frame time.
const INTERPOLATION_RATE: f32 = 24.0;
/// Rotation follows more slowly than position.
const ROTATION_INTERPOLATION_RATE: f32 = 18.0;
/// A longer jump (respawn, access handoff across the map) snaps instead of sliding.
const SNAP_DISTANCE_M: f32 = 80.0;
/// Floats per car in the MultiMesh `TRANSFORM_3D` row-major 3×4 layout.
const STRIDE: usize = 12;
/// Below this a basis is degenerate or mirrored and has no rotation to interpolate.
const MIN_DETERMINANT: f32 = 1e-6;
/// Cars per Rayon task. A bucket below this stays on the calling thread: at ~40 ns of math per
/// car, a smaller split costs more in scheduling (and in waiting on a pool the simulation tick
/// may be using) than it saves.
const PARALLEL_MIN_CARS: usize = 2048;
/// Godot's `CMP_EPSILON`: headings closer than this blend linearly in `slerp`.
const CMP_EPSILON: f32 = 1e-5;

type PoseMap = HashMap<i64, VisualPose, foldhash::fast::FixedState>;

#[derive(Clone, Copy)]
struct VisualPose {
    origin: Vec3,
    rotation: Quat,
    // Lane geometry and off-lane support are distinct height owners; never blend across them.
    grounded: bool,
}

/// Drawn car poses from the previous frame, double-buffered so cars that left the snapshot are
/// forgotten after one frame. Both maps keep their capacity, so steady frames do not allocate.
#[derive(Default)]
pub(crate) struct CarVisualSmoother {
    drawn: PoseMap,
    next: PoseMap,
}

impl CarVisualSmoother {
    /// Per-frame origin weight; a zero or negative delta shows the snapshot pose.
    pub(crate) fn alpha(delta: f64) -> f32 {
        if delta > 0.0 {
            (delta as f32 * INTERPOLATION_RATE).clamp(0.0, 1.0)
        } else {
            1.0
        }
    }

    /// Replaces snapshot poses in `poses` with this frame's drawn poses. Cars first seen, jumping
    /// past the snap distance or changing height owner show their orthonormalized snapshot pose.
    /// A degenerate or non-finite snapshot pose is passed through unchanged. O(cars), one lookup
    /// each; the math mirrors Godot's `Vector3.lerp` and `Basis.slerp(...).orthonormalized()`.
    pub(crate) fn interpolate(&self, poses: &mut [f32], ids: &[i64], flags: &[u8], alpha: f32) {
        let rotation_alpha =
            (alpha * (ROTATION_INTERPOLATION_RATE / INTERPOLATION_RATE)).clamp(0.0, 1.0);
        poses
            .par_chunks_exact_mut(STRIDE)
            .zip(ids.par_iter().zip(flags))
            .with_min_len(PARALLEL_MIN_CARS)
            .for_each(|(pose, (&id, &flag))| {
                let Some((target_origin, target_basis)) = read_pose(pose) else {
                    return;
                };
                let target_basis = orthonormalized(target_basis);
                match self.drawn.get(&id) {
                    Some(previous)
                        if previous.grounded == (flag != 0)
                            && previous.origin.distance_squared(target_origin)
                                <= SNAP_DISTANCE_M * SNAP_DISTANCE_M =>
                    {
                        let rotation = slerp(
                            previous.rotation,
                            Quat::from_mat3(&target_basis),
                            rotation_alpha,
                        );
                        let origin = previous.origin.lerp(target_origin, alpha);
                        write_pose(pose, origin, Mat3::from_quat(rotation));
                    }
                    _ => write_pose(pose, target_origin, target_basis),
                }
            });
    }

    /// Stores the poses actually uploaded, including any support correction or busy fallback,
    /// as next frame's starting points. Unusable poses are not stored and so snap next frame.
    pub(crate) fn record(&mut self, poses: &[f32], ids: &[i64], flags: &[u8]) {
        for ((pose, &id), &flag) in poses.as_chunks::<STRIDE>().0.iter().zip(ids).zip(flags) {
            if let Some((origin, basis)) = read_pose(pose) {
                // Drawn, supported and snapshot bases are already orthonormal.
                let rotation = Quat::from_mat3(&basis);
                self.next.insert(
                    id,
                    VisualPose {
                        origin,
                        rotation,
                        grounded: flag != 0,
                    },
                );
            }
        }
    }

    /// Ends the frame: recorded poses become the drawn set and unrecorded cars are forgotten.
    pub(crate) fn finish_frame(&mut self) {
        std::mem::swap(&mut self.drawn, &mut self.next);
        self.next.clear();
    }
}

/// Restores the snapshot pose of every off-lane car. Used when the support solve is busy or
/// unprepared: an interpolated off-lane pose is never shown without its support correction.
pub(crate) fn restore_off_lane_targets(poses: &mut [f32], targets: &[f32], flags: &[u8]) {
    for ((pose, target), &flag) in poses
        .as_chunks_mut::<STRIDE>()
        .0
        .iter_mut()
        .zip(targets.as_chunks::<STRIDE>().0)
        .zip(flags)
    {
        if flag != 0 {
            *pose = *target;
        }
    }
}

/// Godot's `Quaternion.slerp`, normalized as Godot's `Basis(Quaternion)` constructor does.
fn slerp(from: Quat, to: Quat, weight: f32) -> Quat {
    let mut cosom = from.dot(to);
    let to = if cosom < 0.0 {
        cosom = -cosom;
        -to
    } else {
        to
    };
    let (from_scale, to_scale) = if 1.0 - cosom > CMP_EPSILON {
        let omega = cosom.acos();
        let sinom = omega.sin();
        (
            ((1.0 - weight) * omega).sin() / sinom,
            (weight * omega).sin() / sinom,
        )
    } else {
        (1.0 - weight, weight)
    };
    (from * from_scale + to * to_scale).normalize()
}

/// Godot's `Basis.orthonormalized` Gram-Schmidt order: x, then y, then z.
fn orthonormalized(basis: Mat3) -> Mat3 {
    let x = basis.x_axis.normalize();
    let y = (basis.y_axis - x * x.dot(basis.y_axis)).normalize();
    let z = (basis.z_axis - x * x.dot(basis.z_axis) - y * y.dot(basis.z_axis)).normalize();
    Mat3::from_cols(x, y, z)
}

fn read_pose(pose: &[f32]) -> Option<(Vec3, Mat3)> {
    if !pose.iter().all(|value| value.is_finite()) {
        return None;
    }
    let basis = Mat3::from_cols(
        Vec3::new(pose[0], pose[4], pose[8]),
        Vec3::new(pose[1], pose[5], pose[9]),
        Vec3::new(pose[2], pose[6], pose[10]),
    );
    (basis.determinant() > MIN_DETERMINANT)
        .then_some((Vec3::new(pose[3], pose[7], pose[11]), basis))
}

fn write_pose(pose: &mut [f32], origin: Vec3, basis: Mat3) {
    let (x, y, z) = (basis.x_axis, basis.y_axis, basis.z_axis);
    pose.copy_from_slice(&[
        x.x, y.x, z.x, origin.x, x.y, y.y, z.y, origin.y, x.z, y.z, z.z, origin.z,
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    fn pose(yaw: f32, origin: Vec3) -> Vec<f32> {
        let mut pose = vec![0.0; STRIDE];
        write_pose(&mut pose, origin, Mat3::from_rotation_y(yaw));
        pose
    }

    fn origin(pose: &[f32]) -> Vec3 {
        Vec3::new(pose[3], pose[7], pose[11])
    }

    fn yaw(pose: &[f32]) -> f32 {
        // Column z of a yaw rotation is (sin, 0, cos).
        pose[2].atan2(pose[10])
    }

    /// Draws one frame of a single car and returns its uploaded pose.
    fn frame(smoother: &mut CarVisualSmoother, target: &[f32], flag: u8, alpha: f32) -> Vec<f32> {
        let mut poses = target.to_vec();
        smoother.interpolate(&mut poses, &[7], &[flag], alpha);
        smoother.record(&poses, &[7], &[flag]);
        smoother.finish_frame();
        poses
    }

    #[test]
    fn first_sight_shows_the_snapshot_pose() {
        let mut smoother = CarVisualSmoother::default();
        let target = pose(0.3, Vec3::new(4.0, 1.0, -2.0));
        let drawn = frame(&mut smoother, &target, 0, 0.25);
        for (a, b) in drawn.iter().zip(&target) {
            assert!((a - b).abs() < 1e-6, "{drawn:?} vs {target:?}");
        }
    }

    #[test]
    fn origin_and_heading_close_their_gaps_at_their_own_rates() {
        let mut smoother = CarVisualSmoother::default();
        frame(&mut smoother, &pose(0.0, Vec3::ZERO), 0, 1.0);
        let alpha = 0.4;
        let drawn = frame(
            &mut smoother,
            &pose(FRAC_PI_2, Vec3::new(10.0, 0.0, 0.0)),
            0,
            alpha,
        );
        assert!((origin(&drawn) - Vec3::new(4.0, 0.0, 0.0)).length() < 1e-5);
        let expected_yaw = FRAC_PI_2 * alpha * ROTATION_INTERPOLATION_RATE / INTERPOLATION_RATE;
        assert!((yaw(&drawn) - expected_yaw).abs() < 1e-4, "{}", yaw(&drawn));
        // The drawn basis stays a rotation, not a scaled blend.
        let basis = read_pose(&drawn).unwrap().1;
        assert!((basis.determinant() - 1.0).abs() < 1e-5);
        // The next frame starts from the drawn pose, not the previous snapshot pose.
        let next = frame(
            &mut smoother,
            &pose(FRAC_PI_2, Vec3::new(10.0, 0.0, 0.0)),
            0,
            alpha,
        );
        assert!((origin(&next) - Vec3::new(6.4, 0.0, 0.0)).length() < 1e-5);
    }

    #[test]
    fn long_jumps_and_height_owner_changes_snap() {
        for (target_origin, flag) in [(Vec3::new(81.0, 0.0, 0.0), 0), (Vec3::ONE, 1)] {
            let mut smoother = CarVisualSmoother::default();
            frame(&mut smoother, &pose(0.0, Vec3::ZERO), 0, 1.0);
            let target = pose(1.0, target_origin);
            let drawn = frame(&mut smoother, &target, flag, 0.1);
            assert!((origin(&drawn) - target_origin).length() < 1e-6);
            assert!((yaw(&drawn) - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn cars_absent_for_a_frame_are_forgotten() {
        let mut smoother = CarVisualSmoother::default();
        frame(&mut smoother, &pose(0.0, Vec3::ZERO), 0, 1.0);
        smoother.finish_frame();
        let target = pose(0.0, Vec3::new(10.0, 0.0, 0.0));
        assert_eq!(
            origin(&frame(&mut smoother, &target, 0, 0.1)),
            origin(&target)
        );
    }

    #[test]
    fn degenerate_and_non_finite_poses_pass_through_and_are_not_recorded() {
        let mut smoother = CarVisualSmoother::default();
        let mut degenerate = pose(0.0, Vec3::new(1.0, 2.0, 3.0));
        // A zero lane tangent produces a zero z column.
        for index in [2, 6, 10] {
            degenerate[index] = 0.0;
        }
        let mut non_finite = pose(0.0, Vec3::ZERO);
        non_finite[3] = f32::NAN;
        for target in [degenerate, non_finite] {
            let drawn = frame(&mut smoother, &target, 0, 0.5);
            assert_eq!(
                drawn.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                target.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
            );
            assert!(smoother.drawn.is_empty());
        }
    }

    #[test]
    fn busy_support_restores_only_off_lane_snapshot_poses() {
        let targets = [pose(0.0, Vec3::ZERO), pose(0.0, Vec3::ONE)].concat();
        let mut poses = vec![5.0; targets.len()];
        restore_off_lane_targets(&mut poses, &targets, &[0, 1]);
        assert_eq!(&poses[..STRIDE], &[5.0; STRIDE]);
        assert_eq!(&poses[STRIDE..], &targets[STRIDE..]);
    }

    #[test]
    #[ignore = "release timing of the per-frame car smoothing; run with --nocapture"]
    fn car_smoothing_frame_cost() {
        use std::time::Instant;
        for count in [1_000usize, 5_000, 20_000] {
            let ids: Vec<i64> = (0..count as i64).map(|i| i * 1_000_003 + 17).collect();
            let flags = vec![0u8; count];
            let yaws: Vec<f32> = (0..count)
                .map(|i| (i as f32 * 0.618).fract() * std::f32::consts::TAU)
                .collect();
            let mut targets: Vec<f32> = (0..count)
                .flat_map(|i| {
                    let x = (i % 200) as f32 * 50.0 - 5000.0;
                    pose(yaws[i], Vec3::new(x, 1.0, (i / 200) as f32 * 50.0 - 5000.0))
                })
                .collect();
            let mut smoother = CarVisualSmoother::default();
            let mut poses = targets.clone();
            let mut samples = Vec::new();
            let mut interpolate_samples = Vec::new();
            for step in 0..40 {
                // Each car advances 0.1 m along its heading, as at 12 m/s and 120 Hz.
                for (target, yaw) in targets.as_chunks_mut::<STRIDE>().0.iter_mut().zip(&yaws) {
                    target[3] += 0.1 * yaw.sin();
                    target[11] += 0.1 * yaw.cos();
                }
                let start = Instant::now();
                poses.copy_from_slice(&targets);
                smoother.interpolate(&mut poses, &ids, &flags, 0.2);
                let interpolated = start.elapsed().as_secs_f64() * 1e3;
                smoother.record(&poses, &ids, &flags);
                smoother.finish_frame();
                if step >= 8 {
                    samples.push(start.elapsed().as_secs_f64() * 1e3);
                    interpolate_samples.push(interpolated);
                }
            }
            samples.sort_by(f64::total_cmp);
            interpolate_samples.sort_by(f64::total_cmp);
            let median = samples[samples.len() / 2];
            eprintln!(
                "CAR_SMOOTHING count={count} median_ms={median:.3} p90_ms={:.3} \
                 interpolate_median_ms={:.3} us_per_car={:.3}",
                samples[samples.len() * 9 / 10],
                interpolate_samples[interpolate_samples.len() / 2],
                median * 1e3 / count as f64
            );
        }
    }
}
