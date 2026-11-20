// SPDX-License-Identifier: GPL-3.0-or-later

//! The agents' frame a frame ahead (`RENDER-47`): the drawn sequence is the synchronous one a
//! frame later, a late job carries its delta, and the caster candidates hold every agent the
//! twins' exact test can keep.

use super::*;
use std::collections::BTreeMap;

// A car pose: yaw about +Y at `origin`.
fn pose(yaw: f32, origin: [f32; 3]) -> [f32; CAR_STRIDE] {
    let (s, c) = yaw.sin_cos();
    [
        c, 0.0, s, origin[0], 0.0, 1.0, 0.0, origin[1], -s, 0.0, c, origin[2],
    ]
}

// Frame `frame`'s snapshot: 30 cars in three buckets moving along circles, their targets
// stepping every third frame (the simulation's tick against the render rate), every fifth car
// off-lane, car 11 leaving for frames 20-29 and bucket 20 empty from frame 40 on.
fn snapshot(frame: usize) -> RenderSnapshot {
    let tick = (frame / 3) as f32;
    let mut buckets: BTreeMap<u8, TestCars> = BTreeMap::new();
    for car in 0..30i64 {
        if car == 11 && (20..30).contains(&frame) {
            continue;
        }
        let key = [0u8, 10, 20][car as usize % 3];
        let entry = buckets
            .entry(key)
            .or_insert_with(|| (key, Vec::new(), Vec::new(), Vec::new()));
        if key == 20 && frame >= 40 {
            continue;
        }
        let angle = 0.05 * tick + car as f32;
        let radius = 50.0 + car as f32 * 7.0;
        entry.1.extend(pose(
            angle + 1.0,
            [radius * angle.cos(), 2.0, radius * angle.sin()],
        ));
        entry.2.push(car + 1);
        entry.3.push(u8::from(car % 5 == 0));
    }
    let cars: Vec<_> = buckets.into_values().collect();
    let mut snapshot = snapshot_with_cars(&cars);
    snapshot.vehicle_ground_key = [1, 1, 250f32.to_bits().into()];
    snapshot
}

// A deterministic ground solve: off-lane cars stand on a slope; busy on every seventh frame.
fn solve(frame: usize, poses: &mut [f32], mask: &[u8]) -> bool {
    if frame % 7 == 3 {
        return false;
    }
    for (pose, &flag) in poses.as_chunks_mut::<CAR_STRIDE>().0.iter_mut().zip(mask) {
        if flag != 0 {
            pose[7] = 0.01 * pose[3] - 0.02 * pose[11];
        }
    }
    true
}

const DELTAS: [f64; 5] = [1.0 / 60.0, 1.0 / 30.0, 1.0 / 144.0, 1.0 / 60.0, 0.05];

// The drawn buffers by bucket after a frame's changes, bit patterns.
type Drawn = BTreeMap<u8, Vec<u32>>;

fn apply(drawn: &mut Drawn, smoother: &CarVisualSmoother) {
    for &key in smoother.changed() {
        drawn.insert(
            key,
            smoother
                .drawn_bucket(key)
                .iter()
                .map(|v| v.to_bits())
                .collect(),
        );
    }
    for &key in smoother.left() {
        drawn.remove(&key);
    }
}

fn view(frame: usize) -> FrameView {
    FrameView {
        alpha: CarVisualSmoother::alpha(DELTAS[frame % DELTAS.len()]),
        camera: Some([10.0 * frame as f32, 0.0]),
        ..FrameView::default()
    }
}

// The synchronous path as the main thread ran it before `RENDER-47`, one frame at a time.
fn control(frames: usize) -> Vec<Drawn> {
    let mut smoother = CarVisualSmoother::default();
    let mut drawn = Drawn::new();
    let mut out = Vec::new();
    for frame in 0..frames {
        let snapshot = snapshot(frame);
        let view = view(frame);
        smoother.begin_frame(GroundView {
            key: snapshot.vehicle_ground_key,
            camera: view.camera,
        });
        for (&key, targets) in &snapshot.car_transforms {
            smoother.smooth_bucket(
                key,
                targets,
                &snapshot.car_render_ids[&key],
                &snapshot.car_ground_flags[&key],
                view.alpha,
            );
        }
        if smoother.needs_solve() {
            smoother.solve_pending(|_, poses, mask| solve(frame, poses, mask));
        }
        smoother.finish_frame();
        apply(&mut drawn, &smoother);
        out.push(drawn.clone());
    }
    out
}

#[test]
fn pipelined_frames_draw_the_control_sequence_one_frame_later() {
    const FRAMES: usize = 60;
    let expected = control(FRAMES);
    let mut lane = AgentFrameLane::default();
    let mut drawn = Drawn::new();
    let mut installed = Vec::new();
    for frame in 0..=FRAMES {
        let (taken, delta) = lane.take(DELTAS[frame % DELTAS.len()]);
        let next = match taken {
            Taken::Landed(prepared) => {
                apply(&mut drawn, &prepared.smoother);
                installed.push(drawn.clone());
                prepared
            }
            Taken::Resident(first) => first,
            Taken::Late => unreachable!("every job lands before the next frame here"),
        };
        if frame == FRAMES {
            break;
        }
        let mut next = next;
        next.view = view(frame);
        assert_eq!(next.view.alpha, CarVisualSmoother::alpha(delta));
        let snapshot = snapshot(frame);
        crate::thread_pools::spawn(lane.job(next, move |prepared| {
            prepared.take_snapshot(&snapshot);
            prepared.smooth_cars();
            if prepared.smoother.needs_solve() {
                prepared
                    .smoother
                    .solve_pending(|_, poses, mask| solve(frame, poses, mask));
            }
            prepared.finish();
        }));
        lane.wait_landed();
    }
    assert_eq!(installed.len(), FRAMES);
    for (frame, (got, want)) in installed.iter().zip(&expected).enumerate() {
        assert_eq!(
            got,
            want,
            "frame {} draws what the control drew in frame {frame}",
            frame + 1
        );
    }
    assert_eq!(lane.late_frames(), 0);
}

#[test]
fn a_late_job_carries_its_delta_to_the_next() {
    let mut lane = AgentFrameLane::default();
    let (taken, delta) = lane.take(0.01);
    let Taken::Resident(frame) = taken else {
        panic!("the first frame is resident");
    };
    assert_eq!(delta, 0.01);
    let job = lane.job(frame, |_| {});
    assert!(matches!(lane.take(0.02).0, Taken::Late));
    assert!(matches!(lane.take(0.03).0, Taken::Late));
    job();
    let (taken, delta) = lane.take(0.04);
    assert!(matches!(taken, Taken::Landed(_)));
    assert!(
        (delta - 0.09).abs() < 1e-12,
        "the late frames' deltas carry: {delta}"
    );
    assert_eq!(lane.late_frames(), 2);
}

#[test]
fn candidates_hold_every_agent_the_exact_test_keeps_after_the_camera_moved() {
    let range = 420.0;
    let radius = candidate_radius_m(range);
    let job_eye = Vec3::new(0.0, 300.0, 0.0);
    let mut rows = Vec::new();
    for x in (-1200..=1200).step_by(40) {
        for z in (-1200..=1200).step_by(40) {
            rows.extend(pose(0.0, [x as f32, 0.0, z as f32]));
        }
    }
    let mut candidates = Vec::new();
    keep_candidates(&mut candidates, &rows, CAR_STRIDE, job_eye, radius);
    assert!(candidates.len() < rows.len());
    // The drawing camera stands up to the slack away, looking any way.
    for (moved, forward) in [
        (Vec3::new(CAMERA_SLACK_M - 1.0, 0.0, 0.0), Vec3::X),
        (
            Vec3::new(0.0, -100.0, 150.0),
            Vec3::new(0.0, -0.5, -1.0).normalize(),
        ),
        (Vec3::ZERO, Vec3::new(-1.0, -1.0, 0.0).normalize()),
    ] {
        let eye = job_eye + moved;
        let keep = |rows: &[f32]| -> Vec<[u32; 3]> {
            rows.as_chunks::<CAR_STRIDE>()
                .0
                .iter()
                .map(|row| Vec3::new(row[3], row[7], row[11]))
                .filter(|&origin| within_reach(origin - eye, forward, range))
                .map(|origin| origin.to_array().map(f32::to_bits))
                .collect()
        };
        let kept = keep(&rows);
        assert!(!kept.is_empty());
        assert_eq!(keep(&candidates), kept, "eye moved {moved:?}");
    }
}
