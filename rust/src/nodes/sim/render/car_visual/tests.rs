// SPDX-License-Identifier: GPL-2.0-only

//! Car smoothing, rest and the untouched buckets (`RENDER-28`, `RENDER-32-G`).

use super::*;
use std::collections::HashSet;
use std::f32::consts::FRAC_PI_2;
use std::sync::atomic::{AtomicUsize, Ordering};

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

/// Draws one frame of a single car and returns its drawn pose; a solve leaves poses as they are.
fn frame(smoother: &mut CarVisualSmoother, target: &[f32], flag: u8, alpha: f32) -> Vec<f32> {
    smoother.begin_frame(GroundView::default());
    smoother.smooth_bucket(0, target, &[7], &[flag], alpha);
    smoother.solve_pending(|_, _, _| true);
    smoother.finish_frame();
    smoother.buckets[0].drawn.clone()
}

/// A frame with no cars.
fn empty_frame(smoother: &mut CarVisualSmoother) {
    smoother.begin_frame(GroundView::default());
    smoother.finish_frame();
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
    empty_frame(&mut smoother);
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
        assert!(smoother.cars.is_empty());
    }
}

#[test]
fn a_busy_solve_restores_only_the_cars_it_was_to_solve() {
    let ground = GroundView {
        key: [1, 1, u64::from(250.0f32.to_bits())],
        camera: None,
    };
    let (ids, flags) = ([1, 2, 3], [0, 1, 1]);
    let mut targets = [
        pose(0.0, Vec3::ZERO),
        pose(0.5, Vec3::new(30.0, 0.0, 0.0)),
        pose(1.0, Vec3::new(60.0, 0.0, 0.0)),
    ]
    .concat();
    for pose in &mut targets.as_chunks_mut::<STRIDE>().0[1..] {
        fake_solve(pose, true);
    }
    let mut smoother = CarVisualSmoother::default();
    let solved = AtomicUsize::new(0);
    for frame in 0..200 {
        draw(
            &mut smoother,
            ground,
            0,
            &targets,
            &ids,
            &flags,
            alpha_at(frame),
            &solved,
        );
    }
    // Every car rests. Car 3's snapshot pose moves 2 m and the core is busy: car 3 shows its
    // snapshot pose; car 2 keeps its solve, car 1 its pose.
    let rested = smoother.buckets[0].drawn.clone();
    let mut moved = targets.clone();
    moved[2 * STRIDE + 3] += 2.0;
    smoother.begin_frame(ground);
    smoother.smooth_bucket(0, &moved, &ids, &flags, 0.3);
    smoother.solve_pending(|_, _, _| false);
    smoother.finish_frame();
    let drawn = &smoother.buckets[0].drawn;
    assert_eq!(bits(&drawn[..2 * STRIDE]), bits(&rested[..2 * STRIDE]));
    assert_eq!(bits(&drawn[2 * STRIDE..]), bits(&moved[2 * STRIDE..]));
    assert_eq!(smoother.changed(), [0]);
}

#[test]
#[ignore = "release timing of the per-frame car smoothing; run with --nocapture"]
fn car_smoothing_frame_cost() {
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
        let mut samples = Vec::new();
        let mut interpolate_samples = Vec::new();
        for step in 0..40 {
            // Each car advances 0.1 m along its heading, as at 12 m/s and 120 Hz.
            for (target, yaw) in targets.as_chunks_mut::<STRIDE>().0.iter_mut().zip(&yaws) {
                target[3] += 0.1 * yaw.sin();
                target[11] += 0.1 * yaw.cos();
            }
            let start = Instant::now();
            draw(
                &mut smoother,
                GroundView::default(),
                0,
                &targets,
                &ids,
                &flags,
                0.2,
                &AtomicUsize::new(0),
            );
            let interpolated = smoother.parts()[0] as f64 / 1e3;
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

#[test]
fn nearest_cars_picks_the_closest_within_radius_nearest_first() {
    let eye = Vec3::new(0.0, 2.0, 0.0);
    let first: Vec<f32> = [
        pose(0.0, Vec3::new(30.0, 0.0, 0.0)),
        pose(0.0, Vec3::new(5.0, 0.0, 0.0)),
        pose(0.0, Vec3::new(500.0, 0.0, 0.0)),
    ]
    .concat();
    let second: Vec<f32> = [
        pose(0.0, Vec3::new(0.0, 0.0, -10.0)),
        pose(0.0, Vec3::new(0.0, 0.0, 20.0)),
    ]
    .concat();
    let buffers = [first.as_slice(), second.as_slice()];
    assert_eq!(
        nearest_cars(&buffers, eye, 100.0, 3),
        vec![(0, 1), (1, 0), (1, 1)]
    );
    // Beyond the radius is never chosen, however few cars are near.
    assert_eq!(nearest_cars(&buffers, eye, 100.0, 10).len(), 4);
    assert!(nearest_cars(&buffers, eye, 100.0, 0).is_empty());
}

// A stand-in for the ground solve, shaped like it: height from the XZ, a tilt about the
// horizontal heading, a step at the full-detail radius. Deterministic, so the control and the
// branch see the same answers for the same inputs.
fn fake_solve(pose: &mut [f32], near: bool) {
    let heading = Vec2::new(pose[2], pose[10]).normalize_or_zero();
    let heading = if heading == Vec2::ZERO {
        Vec2::Y
    } else {
        heading
    };
    let z = Vec3::new(heading.x, 0.05, heading.y).normalize();
    let x = Vec3::Y.cross(z).normalize();
    let y = z.cross(x).normalize();
    let height = 0.01 * pose[3] + 0.02 * pose[11] + if near { 0.0 } else { 0.005 };
    pose[0] = x.x;
    pose[1] = y.x;
    pose[2] = z.x;
    pose[4] = x.y;
    pose[5] = y.y;
    pose[6] = z.y;
    pose[8] = x.z;
    pose[9] = y.z;
    pose[10] = z.z;
    pose[7] = height;
}

// Solves the poses flagged in `mask` with `fake_solve`, counting the calls and the cars.
fn solve_masked(poses: &mut [f32], mask: &[u8], ground: GroundView, cars: &AtomicUsize) -> bool {
    for (pose, &flag) in poses.as_chunks_mut::<STRIDE>().0.iter_mut().zip(mask) {
        if flag != 0 {
            let near = ground.near(pose);
            fake_solve(pose, near);
            cars.fetch_add(1, Ordering::Relaxed);
        }
    }
    true
}

// One frame of one bucket, solved by `fake_solve`; whether its drawn buffer changed.
#[allow(clippy::too_many_arguments)]
fn draw(
    smoother: &mut CarVisualSmoother,
    ground: GroundView,
    key: u8,
    targets: &[f32],
    ids: &[i64],
    flags: &[u8],
    alpha: f32,
    solved: &AtomicUsize,
) -> bool {
    smoother.begin_frame(ground);
    smoother.smooth_bucket(key, targets, ids, flags, alpha);
    smoother.solve_pending(|_, poses, mask| solve_masked(poses, mask, ground, solved));
    smoother.finish_frame();
    smoother.changed().contains(&key)
}

fn bits(values: &[f32]) -> Vec<u32> {
    values.iter().map(|value| value.to_bits()).collect()
}

fn alpha_at(frame: usize) -> f32 {
    // Frame times vary: 6-21 ms.
    0.15 + 0.35 * (frame as f32 * 0.618_034).fract()
}

#[test]
fn still_cars_come_to_rest_and_their_bucket_is_left_alone() {
    // No camera: every car is solved exactly, inside the full-detail radius.
    let ground = GroundView {
        key: [1, 1, u64::from(250.0f32.to_bits())],
        camera: None,
    };
    let ids: Vec<i64> = (1..=40).collect();
    let flags: Vec<u8> = (0..40).map(|i| (i % 2) as u8).collect();
    let mut targets = Vec::new();
    let mut start = Vec::new();
    for i in 0..40 {
        let at = Vec3::new(i as f32 * 37.0 - 700.0, 3.0, 1500.0 - i as f32 * 61.0);
        targets.extend(pose(i as f32 * 0.37, at));
        start.extend(pose(i as f32 * 0.37 + 0.3, at + Vec3::new(3.0, 0.0, -2.0)));
    }
    // Off-lane snapshot poses are grounded, as the snapshot grounds them.
    for poses in [&mut targets, &mut start] {
        for (pose, &flag) in poses.as_chunks_mut::<STRIDE>().0.iter_mut().zip(&flags) {
            if flag != 0 {
                let near = ground.near(pose);
                fake_solve(pose, near);
            }
        }
    }
    let solved = AtomicUsize::new(0);
    let mut smoother = CarVisualSmoother::default();
    let draw_cars =
        |smoother: &mut CarVisualSmoother, ground: GroundView, targets: &[f32], alpha| {
            draw(smoother, ground, 3, targets, &ids, &flags, alpha, &solved)
        };
    draw_cars(&mut smoother, ground, &start, 1.0);
    let rest_frame = (0..400).find(|&frame| {
        let before = solved.load(Ordering::Relaxed);
        !draw_cars(&mut smoother, ground, &targets, alpha_at(frame))
            && solved.load(Ordering::Relaxed) == before
    });
    assert!(rest_frame.is_some(), "still cars must come to rest");
    // A still frame: no smoothing, no solve, no upload.
    for frame in 0..60 {
        let before = solved.load(Ordering::Relaxed);
        assert!(!draw_cars(&mut smoother, ground, &targets, alpha_at(frame)));
        assert_eq!(solved.load(Ordering::Relaxed), before);
        assert_eq!(smoother.parts()[3..], [40, 20, 0, 40, 20, 0, 1]);
    }
    let rested = smoother.buckets[3].drawn.clone();
    for (pose, target) in rested
        .as_chunks::<STRIDE>()
        .0
        .iter()
        .zip(targets.as_chunks::<STRIDE>().0)
    {
        for index in [3, 11] {
            assert!(close(pose[index], target[index]));
        }
    }

    // The camera beyond the full-detail radius of every car: each off-lane car solves once, on
    // its far side, and only those move.
    let far = GroundView {
        camera: Some([20_000.0, 0.0]),
        ..ground
    };
    assert!(draw_cars(&mut smoother, far, &targets, 0.3));
    assert_eq!(smoother.parts()[5..8], [20, 20, 0]);
    assert!(!draw_cars(&mut smoother, far, &targets, 0.3));
    assert_eq!(smoother.parts()[5], 0);

    // A new ground generation solves every off-lane car again, once.
    let edited = GroundView {
        key: [1, 2, ground.key[2]],
        ..far
    };
    draw_cars(&mut smoother, edited, &targets, 0.3);
    assert_eq!(smoother.parts()[5..8], [20, 20, 0]);
    assert!(!draw_cars(&mut smoother, edited, &targets, 0.3));
    assert_eq!(smoother.parts()[9], 1);

    // One car's snapshot pose moves: only it is smoothed and solved; the others stay at rest.
    let mut moved = targets.clone();
    moved[STRIDE + 3] += 1.0;
    assert!(draw_cars(&mut smoother, edited, &moved, 0.3));
    assert_eq!(smoother.parts()[5..9], [1, 39, 19, 1]);
}

// The pre-rest smoother, kept as the control: every car is smoothed and every off-lane car
// solved every frame.
#[derive(Default)]
struct Control {
    drawn: HashMap<i64, VisualPose>,
    next: HashMap<i64, VisualPose>,
}

impl Control {
    fn bucket(
        &mut self,
        targets: &[f32],
        ids: &[i64],
        flags: &[u8],
        alpha: f32,
        ground: GroundView,
    ) -> Vec<f32> {
        let mut poses = targets.to_vec();
        let rotation_alpha =
            (alpha * (ROTATION_INTERPOLATION_RATE / INTERPOLATION_RATE)).clamp(0.0, 1.0);
        for ((pose, &id), &flag) in poses
            .as_chunks_mut::<STRIDE>()
            .0
            .iter_mut()
            .zip(ids)
            .zip(flags)
        {
            let Some((target_origin, target_basis)) = read_pose(pose) else {
                continue;
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
        }
        if flags.iter().any(|&flag| flag != 0) {
            solve_masked(&mut poses, flags, ground, &AtomicUsize::new(0));
        }
        for (pose, (&id, &flag)) in poses
            .as_chunks::<STRIDE>()
            .0
            .iter()
            .zip(ids.iter().zip(flags))
        {
            if let Some((origin, basis)) = read_pose(pose) {
                self.next.insert(
                    id,
                    VisualPose {
                        origin,
                        rotation: Quat::from_mat3(&basis),
                        grounded: flag != 0,
                    },
                );
            }
        }
        poses
    }

    fn finish(&mut self) {
        std::mem::swap(&mut self.drawn, &mut self.next);
        self.next.clear();
    }
}

// One scripted car: where its snapshot pose is at a frame, or `None` while it is not drawn.
struct Scripted {
    id: i64,
    flag: u8,
    start: Vec3,
    yaw: f32,
    speed: f32,
    enter: usize,
    leave: usize,
}

const PAUSE: std::ops::Range<usize> = 250..400;

impl Scripted {
    fn target(&self, frame: usize, ground: GroundView) -> Option<Vec<f32>> {
        if !(self.enter..self.leave).contains(&frame) {
            return None;
        }
        // The simulation ticks every second frame, never in the pause; a car drives for a
        // while, then waits as long.
        let ticks = (0..frame)
            .filter(|frame| frame.is_multiple_of(2) && !PAUSE.contains(frame))
            .filter(|frame| (frame / (40 + 2 * (self.id as usize % 17))).is_multiple_of(2))
            .count();
        let mut at =
            self.start + Vec3::new(self.yaw.sin(), 0.0, self.yaw.cos()) * self.speed * ticks as f32;
        // One car jumps beyond the snap distance.
        if self.id == 5 && frame >= 450 {
            at.x += 120.0;
        }
        let mut flag = self.flag;
        if self.id == 1_001 && frame >= 470 {
            flag = 1 - flag;
        }
        let mut target = pose(self.yaw + 0.002 * ticks as f32, at);
        if flag != 0 {
            let near = ground.near(&target);
            fake_solve(&mut target, near);
        }
        Some(target)
    }

    fn flag(&self, frame: usize) -> u8 {
        if self.id == 1_001 && frame >= 470 {
            1 - self.flag
        } else {
            self.flag
        }
    }
}

#[test]
fn drawn_buffers_follow_the_control_frame_by_frame() {
    let keys = [0u8, 11, 23];
    let cars: Vec<Vec<Scripted>> = (0..keys.len())
        .map(|bucket| {
            (0..60)
                .map(|i| Scripted {
                    id: bucket as i64 * 1_000 + i as i64,
                    flag: u8::from(i % 3 == 0),
                    start: Vec3::new(i as f32 * 41.0 - 1200.0, 2.0, 900.0 - i as f32 * 29.0),
                    yaw: i as f32 * 0.71,
                    speed: 0.05 + 0.01 * (i % 7) as f32,
                    enter: (i * 7) % 50,
                    leave: 600 - (i * 11) % 80,
                })
                .collect()
        })
        .collect();
    let mut smoother = CarVisualSmoother::default();
    let mut control = Control::default();
    let mut ever_rested = HashSet::new();
    let (solved, mut control_solved) = (AtomicUsize::new(0), 0);
    for frame in 0..600 {
        // The camera sweeps across the cars, except in the pause; the ground changes once.
        let camera_x = frame.min(PAUSE.start) as f32 * 3.0 - 900.0
            + frame.saturating_sub(PAUSE.end) as f32 * 3.0;
        let ground = GroundView {
            key: [1, u64::from(frame >= 520), u64::from(250.0f32.to_bits())],
            camera: Some([camera_x, 300.0]),
        };
        let alpha = alpha_at(frame);
        smoother.begin_frame(ground);
        let before = solved.load(Ordering::Relaxed);
        let mut expected = Vec::new();
        let mut previous = Vec::new();
        for (&key, bucket) in keys.iter().zip(&cars) {
            // Bucket 23 is gone for two frames: its cars are forgotten.
            if key == 23 && (200..202).contains(&frame) {
                continue;
            }
            let (mut targets, mut ids, mut flags) = (Vec::new(), Vec::new(), Vec::new());
            for car in bucket {
                if let Some(target) = car.target(frame, ground) {
                    targets.extend(target);
                    ids.push(car.id);
                    flags.push(car.flag(frame));
                }
            }
            expected.push((
                key,
                ids.clone(),
                control.bucket(&targets, &ids, &flags, alpha, ground),
            ));
            control_solved += flags.iter().filter(|&&flag| flag != 0).count();
            previous.push(
                smoother
                    .buckets
                    .get(usize::from(key))
                    .map(|b| b.drawn.clone()),
            );
            smoother.smooth_bucket(key, &targets, &ids, &flags, alpha);
        }
        smoother.solve_pending(|_, poses, mask| solve_masked(poses, mask, ground, &solved));
        smoother.finish_frame();
        let uploads = smoother.changed().len();
        for ((key, ids, expected), previous) in expected.iter().zip(previous) {
            let drawn = &smoother.buckets[usize::from(*key)].drawn;
            if !smoother.changed().contains(key) {
                assert_eq!(
                    previous.map(|p| bits(&p)),
                    Some(bits(drawn)),
                    "frame {frame}"
                );
            }
            for ((drawn, expected), id) in drawn
                .as_chunks::<STRIDE>()
                .0
                .iter()
                .zip(expected.as_chunks::<STRIDE>().0)
                .zip(ids)
            {
                if ever_rested.contains(id) {
                    // The accepted rule (the user, 2026-11-12): after a car's first rest, within
                    // 16 units in the last place of the control, in `REST_ULPS`'s unit.
                    assert!(
                        drawn
                            .iter()
                            .zip(expected)
                            .all(|(&a, &b)| ulps_apart(a, b) <= 16.0),
                        "frame {frame} car {id}: {drawn:?} vs {expected:?}"
                    );
                } else {
                    // Before it: bit for bit.
                    assert_eq!(bits(drawn), bits(expected), "frame {frame} car {id}");
                }
            }
        }
        control.finish();
        ever_rested.extend(
            smoother
                .cars
                .iter()
                .filter(|(_, car)| car.rest)
                .map(|(&id, _)| id),
        );
        // Late in the pause every car rests: nothing is solved or uploaded.
        if (PAUSE.start + 90..PAUSE.end).contains(&frame) {
            assert_eq!(
                (solved.load(Ordering::Relaxed) - before, uploads),
                (0, 0),
                "frame {frame}"
            );
            assert_eq!(smoother.parts()[9], 3);
        }
    }
    assert!(ever_rested.len() > 150);
    // Cars wait 40-72 frames at a time and take about 30 to come to rest.
    assert!(
        solved.load(Ordering::Relaxed) * 3 < control_solved * 2,
        "{} vs {control_solved}",
        solved.load(Ordering::Relaxed)
    );
}
