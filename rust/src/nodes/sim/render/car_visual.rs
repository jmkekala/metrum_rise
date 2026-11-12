// SPDX-License-Identifier: GPL-2.0-only

//! Render-side smoothing of car poses between simulation snapshots.
//!
//! Presentation state only: it is never saved and never read by the simulation. Every frame
//! moves each visible car's drawn pose toward its latest snapshot pose, keyed by the stable
//! render ID, so cars do not visibly step at the simulation tick rate.
//!
//! **Rest (`RENDER-28`, `RENDER-32-G`).** The smoothing never closes its gap exactly: on a fixed
//! snapshot pose it wanders one or two units in the last place frame to frame, forever. A car
//! whose drawn pose has come within [`REST_ULPS`] of its snapshot pose is at rest: while its
//! snapshot pose stays the same bit for bit it is drawn exactly as it was, with no smoothing and,
//! off-lane, no ground solve. The solve it keeps is the one that grounded that very pose, so it
//! stays a fresh solve's answer while the ground ([`GroundView::key`]) and the car's side of the
//! full-detail radius stay. A bucket whose inputs are last frame's bit for bit and whose cars all
//! rest is not touched at all, and its drawn buffer is not uploaded again.

use glam::{Mat3, Quat, Vec2, Vec3};
use godot::prelude::Vector2;
use rayon::prelude::*;
use std::collections::HashMap;
use std::time::Instant;

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
/// Units in the last place (of the larger magnitude, at least 1) within which a drawn pose has
/// reached its snapshot pose. Four times the smoothing's own wander on fixed targets (1-2 units,
/// measured over 2,000 frames of 5,000 cars at varying frame times): under a millimetre at 4 km
/// from the origin, a few microradians of heading.
const REST_ULPS: f32 = 8.0;

type CarMap = HashMap<i64, DrawnCar, foldhash::fast::FixedState>;
// One car of a bucket being smoothed: its pose, solve mask and reuse mark, ID and flag.
type CarItem<'a> = (
    (&'a mut [f32], (&'a mut u8, &'a mut bool)),
    (&'a i64, &'a u8),
);

#[derive(Clone, Copy)]
struct VisualPose {
    origin: Vec3,
    rotation: Quat,
    // Lane geometry and off-lane support are distinct height owners; never blend across them.
    grounded: bool,
}

/// One car as drawn, kept for the next frame.
#[derive(Clone, Copy)]
struct DrawnCar {
    pose: VisualPose,
    // The snapshot pose it moved toward, bit for bit, and the pose drawn.
    target: [u32; STRIDE],
    drawn: [f32; STRIDE],
    // Drawn at its target to f32 resolution (`REST_ULPS`): drawn again unchanged while the
    // target stays.
    rest: bool,
    // Off-lane: the solve that grounded `drawn` was the exact one, inside the full-detail radius.
    near: bool,
    // The bucket and frame it was recorded in. It is still drawn while that bucket's recording
    // stands (`Bucket::recorded`) and the bucket is drawn.
    bucket: u8,
    frame: u64,
}

/// A bucket's inputs and drawn buffer, aligned by instance, from the frame it was last smoothed,
/// and this frame's poses while it is smoothed.
#[derive(Default)]
struct Bucket {
    ids: Vec<i64>,
    targets: Vec<u32>,
    flags: Vec<u8>,
    near: Vec<bool>,
    drawn: Vec<f32>,
    // Cars not at rest (or not recordable), and off-lane cars, as recorded.
    moving: usize,
    off_lane: usize,
    // The frame its cars were recorded in and the last frame it was drawn in (0: never); the
    // same as of the previous frame, which the cars' lookups read.
    recorded: u64,
    present: u64,
    recorded_before: u64,
    present_before: u64,
    // Smoothed this frame and not yet recorded: its poses, the off-lane cars to solve, the cars
    // drawn as they were, and whether the solve succeeded.
    pending: bool,
    next: Vec<f32>,
    solve: Vec<u8>,
    reused: Vec<bool>,
    solved: bool,
}

impl Bucket {
    fn wants_solve(&self) -> bool {
        self.pending && self.solve.iter().any(|&flag| flag != 0)
    }
}

/// What an off-lane car's ground solve reads besides its pose.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct GroundView {
    /// The snapshot's `vehicle_ground_key`: world epoch, ground generation, full-detail radius
    /// bits. A change re-solves every off-lane car once.
    pub key: [u64; 3],
    /// The camera XZ of the ground solve's level of detail; `None` solves exactly everywhere.
    pub camera: Option<[f32; 2]>,
}

impl GroundView {
    // Whether a car at `pose` is solved exactly, as `SimCore::vehicle_ground_pose` decides.
    fn near(&self, pose: &[f32]) -> bool {
        let full = f32::from_bits(self.key[2] as u32);
        self.camera.is_none_or(|[x, z]| {
            Vector2::new(x, z).distance_squared_to(Vector2::new(pose[3], pose[11])) <= full * full
        })
    }
}

/// The last frame's split (`RENDER-28`), for the perf channel: microseconds smoothing, in the
/// ground solve (with its core attempt) and recording; then cars, off-lane cars, off-lane cars
/// solved, cars at rest and such off-lane cars, buckets uploaded and buckets kept untouched.
pub(crate) type CarFrameParts = [i64; 10];

/// Drawn car poses by render ID and the buckets they were drawn in. Maps and buffers keep their
/// capacity, so steady frames do not allocate. Per frame: [`Self::begin_frame`]; per bucket
/// [`Self::smooth_bucket`] or [`Self::pass_through`]; one [`Self::solve_pending`] for every
/// bucket's off-lane cars together; [`Self::finish_frame`]; then [`Self::changed`] and
/// [`Self::left`] say what to upload.
#[derive(Default)]
pub(crate) struct CarVisualSmoother {
    cars: CarMap,
    // By bucket key.
    buckets: Vec<Bucket>,
    frame: u64,
    ground: GroundView,
    ground_changed: bool,
    camera_moved: bool,
    // A bucket was smoothed or left this frame: some recorded cars may be stale.
    stale: bool,
    changed: Vec<u8>,
    left: Vec<u8>,
    parts: CarFrameParts,
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

    /// Starts a frame whose off-lane cars are grounded against `ground`. O(buckets).
    pub(crate) fn begin_frame(&mut self, ground: GroundView) {
        self.frame += 1;
        self.ground_changed = ground.key != self.ground.key;
        self.camera_moved = ground.camera != self.ground.camera;
        self.ground = ground;
        self.stale = false;
        self.changed.clear();
        self.left.clear();
        self.parts = [0; 10];
        for bucket in &mut self.buckets {
            bucket.recorded_before = bucket.recorded;
            bucket.present_before = bucket.present;
        }
    }

    /// Smooths one bucket: `targets` are its snapshot poses, `ids` and `flags` (1: off-lane) are
    /// aligned with them. Leaves an untouched bucket as it was drawn; O(1) plus a compare of the
    /// inputs for it. Otherwise O(cars) with one lookup each; the bucket's off-lane cars not at
    /// rest then wait for [`Self::solve_pending`] and its recording for [`Self::finish_frame`].
    pub(crate) fn smooth_bucket(
        &mut self,
        key: u8,
        targets: &[f32],
        ids: &[i64],
        flags: &[u8],
        alpha: f32,
    ) {
        let count = ids.len();
        debug_assert!(targets.len() == count * STRIDE && flags.len() == count);
        self.parts[3] += count as i64;
        let off_lane = flags.iter().filter(|&&flag| flag != 0).count();
        self.parts[4] += off_lane as i64;
        if self.untouched(key, targets, ids, flags) {
            self.parts[6] += count as i64;
            self.parts[7] += off_lane as i64;
            self.parts[9] += 1;
            self.buckets[usize::from(key)].present = self.frame;
            return;
        }
        self.stale = true;
        let started = Instant::now();
        let bucket = &mut self.buckets[usize::from(key)];
        bucket.ids.clear();
        bucket.ids.extend_from_slice(ids);
        bucket.flags.clear();
        bucket.flags.extend_from_slice(flags);
        bucket.targets.clear();
        bucket
            .targets
            .extend(targets.iter().map(|value| value.to_bits()));
        bucket.pending = true;
        bucket.solved = true;
        let mut next = std::mem::take(&mut bucket.next);
        let mut solve = std::mem::take(&mut bucket.solve);
        let mut reused = std::mem::take(&mut bucket.reused);
        next.clear();
        next.extend_from_slice(targets);
        solve.clear();
        solve.resize(count, 0);
        reused.clear();
        reused.resize(count, false);
        self.interpolate(&mut next, &mut solve, &mut reused, ids, flags, alpha);
        let bucket = &mut self.buckets[usize::from(key)];
        bucket.next = next;
        bucket.solve = solve;
        bucket.reused = reused;
        self.parts[0] += started.elapsed().as_micros() as i64;
    }

    /// Whether a bucket smoothed this frame has an off-lane car to solve. O(buckets + cars).
    pub(crate) fn needs_solve(&self) -> bool {
        self.buckets.iter().any(Bucket::wants_solve)
    }

    /// Grounds every bucket's off-lane cars not at rest in one parallel pass: `solve(key,
    /// poses, mask)` grounds the poses flagged in `mask` in place and returns false when it
    /// could not (busy or unprepared); those cars then show their snapshot pose. One wait on
    /// the pool a frame, not one a bucket.
    pub(crate) fn solve_pending(&mut self, solve: impl Fn(u8, &mut [f32], &[u8]) -> bool + Sync) {
        if !self.needs_solve() {
            return;
        }
        let started = Instant::now();
        // Godot's main thread solves every frame: shared pool (CODE-17).
        crate::thread_pools::install(|| {
            self.buckets
                .par_iter_mut()
                .enumerate()
                .filter(|(_, bucket)| bucket.wants_solve())
                .for_each(|(key, bucket)| {
                    bucket.solved = solve(key as u8, &mut bucket.next, &bucket.solve);
                });
        });
        for bucket in self
            .buckets
            .iter_mut()
            .filter(|bucket| bucket.wants_solve())
        {
            if bucket.solved {
                self.parts[5] += bucket.solve.iter().filter(|&&flag| flag != 0).count() as i64;
                continue;
            }
            // The snapshot pose of every car the solve was to ground.
            for ((pose, target), &flag) in bucket
                .next
                .as_chunks_mut::<STRIDE>()
                .0
                .iter_mut()
                .zip(bucket.targets.as_chunks::<STRIDE>().0)
                .zip(&bucket.solve)
            {
                if flag != 0 {
                    *pose = target.map(f32::from_bits);
                }
            }
        }
        self.parts[1] += started.elapsed().as_micros() as i64;
    }

    /// Draws a malformed bucket (counts that do not match) as its snapshot poses, unsmoothed
    /// and unrecorded. O(cars).
    pub(crate) fn pass_through(&mut self, key: u8, targets: &[f32]) {
        self.stale = true;
        let frame = self.frame;
        let bucket = self.bucket_mut(key);
        bucket.ids.clear();
        bucket.targets.clear();
        bucket.flags.clear();
        bucket.near.clear();
        // Never untouched: such a bucket is drawn as given every frame.
        bucket.moving = 1;
        bucket.off_lane = 0;
        bucket.recorded = frame;
        bucket.present = frame;
        if !same_bits(&bucket.drawn, targets) {
            bucket.drawn.clear();
            bucket.drawn.extend_from_slice(targets);
            self.changed.push(key);
        }
    }

    /// Ends the frame: records the smoothed buckets, forgets the buckets drawn last frame and
    /// not this one with their cars. O(buckets + smoothed cars), plus O(recorded cars) on a
    /// frame that smoothed or lost a bucket.
    pub(crate) fn finish_frame(&mut self) {
        let started = Instant::now();
        let frame = self.frame;
        for key in 0..self.buckets.len() {
            if self.buckets[key].pending {
                self.record(key as u8);
            }
        }
        for (key, bucket) in self.buckets.iter_mut().enumerate() {
            if was_drawn_before(bucket, frame) && bucket.present != frame {
                *bucket = Bucket::default();
                self.left.push(key as u8);
                self.stale = true;
            }
        }
        if self.stale {
            let buckets = &self.buckets;
            self.cars
                .retain(|_, car| is_drawn(car, &buckets[usize::from(car.bucket)], frame));
        }
        self.parts[2] += started.elapsed().as_micros() as i64;
    }

    /// The buckets whose drawn buffer this frame changed: the ones to upload.
    pub(crate) fn changed(&self) -> &[u8] {
        &self.changed
    }

    /// The buckets drawn last frame and not this one: they draw nothing now.
    pub(crate) fn left(&self) -> &[u8] {
        &self.left
    }

    /// Bucket `key`'s drawn buffer.
    pub(crate) fn drawn_bucket(&self, key: u8) -> &[f32] {
        &self.buckets[usize::from(key)].drawn
    }

    /// The last frame's split ([`CarFrameParts`]).
    pub(crate) fn parts(&self) -> CarFrameParts {
        self.parts
    }

    /// Where car `id` was drawn this frame, if it was. One lookup.
    pub(crate) fn drawn_origin(&self, id: i64) -> Option<Vec3> {
        self.cars
            .get(&id)
            .filter(|car| is_drawn(car, &self.buckets[usize::from(car.bucket)], self.frame))
            .map(|car| car.pose.origin)
    }

    fn bucket_mut(&mut self, key: u8) -> &mut Bucket {
        if self.buckets.len() <= usize::from(key) {
            self.buckets
                .resize_with(usize::from(key) + 1, Bucket::default);
        }
        &mut self.buckets[usize::from(key)]
    }

    // Whether bucket `key` stands as drawn last frame: drawn then, every car at rest, the same
    // inputs bit for bit and, for its off-lane cars, the same ground and side of the full-detail
    // radius. O(1) plus the compare; O(off-lane cars) more when the camera moved.
    fn untouched(&mut self, key: u8, targets: &[f32], ids: &[i64], flags: &[u8]) -> bool {
        let frame = self.frame;
        let (ground, ground_changed, camera_moved) =
            (self.ground, self.ground_changed, self.camera_moved);
        let bucket = self.bucket_mut(key);
        if !was_drawn_before(bucket, frame)
            || bucket.moving != 0
            || bucket.ids != ids
            || bucket.flags != flags
            || !targets
                .iter()
                .zip(&bucket.targets)
                .all(|(target, &bits)| target.to_bits() == bits)
        {
            return false;
        }
        if bucket.off_lane == 0 || !camera_moved {
            return bucket.off_lane == 0 || !ground_changed;
        }
        !ground_changed
            && bucket
                .drawn
                .as_chunks::<STRIDE>()
                .0
                .iter()
                .zip(&bucket.flags)
                .zip(&bucket.near)
                .all(|((pose, &flag), &near)| flag == 0 || ground.near(pose) == near)
    }

    // Writes this frame's poses over the snapshot poses in `poses` and marks the off-lane cars
    // to solve in `solve`: a car at rest copies its drawn pose (`reused`); any other moves
    // toward its target as Godot's `Vector3.lerp` and `Basis.slerp(...).orthonormalized()`
    // would. Cars first seen, jumping past the snap distance or changing height owner show their
    // orthonormalized snapshot pose; a degenerate or non-finite snapshot pose passes through
    // unchanged. A bucket below `PARALLEL_MIN_CARS` stays on the calling thread: waking the pool
    // for it costs more than its math.
    fn interpolate(
        &self,
        poses: &mut [f32],
        solve: &mut [u8],
        reused: &mut [bool],
        ids: &[i64],
        flags: &[u8],
        alpha: f32,
    ) {
        let rotation_alpha =
            (alpha * (ROTATION_INTERPOLATION_RATE / INTERPOLATION_RATE)).clamp(0.0, 1.0);
        let (cars, buckets, frame) = (&self.cars, &self.buckets, self.frame);
        let (ground, ground_changed) = (self.ground, self.ground_changed);
        let step = |((pose, (solve, reused)), (&id, &flag)): CarItem<'_>| {
            let previous = cars.get(&id).filter(|car| {
                was_drawn_before(&buckets[usize::from(car.bucket)], frame)
                    && car.frame == buckets[usize::from(car.bucket)].recorded_before
                    && car.pose.grounded == (flag != 0)
            });
            if let Some(car) = previous.filter(|car| {
                car.rest
                    && same_target(&car.target, pose)
                    && (flag == 0 || (!ground_changed && ground.near(&car.drawn) == car.near))
            }) {
                pose.copy_from_slice(&car.drawn);
                *reused = true;
                return;
            }
            *solve = flag;
            let Some((target_origin, target_basis)) = read_pose(pose) else {
                return;
            };
            let target_basis = orthonormalized(target_basis);
            match previous {
                Some(car)
                    if car.pose.origin.distance_squared(target_origin)
                        <= SNAP_DISTANCE_M * SNAP_DISTANCE_M =>
                {
                    let rotation = slerp(
                        car.pose.rotation,
                        Quat::from_mat3(&target_basis),
                        rotation_alpha,
                    );
                    let origin = car.pose.origin.lerp(target_origin, alpha);
                    write_pose(pose, origin, Mat3::from_quat(rotation));
                }
                _ => write_pose(pose, target_origin, target_basis),
            }
        };
        if ids.len() < PARALLEL_MIN_CARS {
            poses
                .as_chunks_mut::<STRIDE>()
                .0
                .iter_mut()
                .map(<[f32; STRIDE]>::as_mut_slice)
                .zip(solve.iter_mut().zip(reused.iter_mut()))
                .zip(ids.iter().zip(flags))
                .for_each(step);
            return;
        }
        // Godot's main thread calls this every frame: shared pool (CODE-17).
        crate::thread_pools::install(|| {
            poses
                .par_chunks_exact_mut(STRIDE)
                .zip(solve.par_iter_mut().zip(reused.par_iter_mut()))
                .zip(ids.par_iter().zip(flags))
                .with_min_len(PARALLEL_MIN_CARS)
                .for_each(step);
        });
    }

    // Records a smoothed bucket: its poses as next frame's starting points (including any
    // support correction or busy fallback), unusable poses not stored so they snap next frame.
    // A car is at rest when it was, or when its drawn pose has reached its target
    // ([`at_target`]) through a solve that succeeded. Notes the bucket as changed when its
    // drawn buffer differs from last frame's.
    fn record(&mut self, key: u8) {
        let frame = self.frame;
        let ground = self.ground;
        let mut bucket = std::mem::take(self.bucket_mut(key));
        bucket.near.clear();
        bucket.moving = 0;
        bucket.off_lane = 0;
        for (index, ((pose, target), &flag)) in bucket
            .next
            .as_chunks::<STRIDE>()
            .0
            .iter()
            .zip(bucket.targets.as_chunks::<STRIDE>().0)
            .zip(&bucket.flags)
            .enumerate()
        {
            let id = bucket.ids[index];
            bucket.off_lane += usize::from(flag != 0);
            let previous = bucket.reused[index].then(|| self.cars[&id]);
            let near = match previous {
                Some(car) => car.near,
                None => flag != 0 && ground.near(pose),
            };
            bucket.near.push(near);
            let Some((origin, basis)) = read_pose(pose) else {
                bucket.moving += 1;
                continue;
            };
            let target = target.map(f32::from_bits);
            let rest = previous.is_some()
                || ((bucket.solve[index] == 0 || bucket.solved) && at_target(pose, &target, flag));
            if !rest {
                bucket.moving += 1;
            } else if previous.is_some() {
                self.parts[6] += 1;
                self.parts[7] += i64::from(flag != 0);
            }
            self.cars.insert(
                id,
                DrawnCar {
                    pose: match previous {
                        Some(car) => car.pose,
                        // Drawn, supported and snapshot bases are already orthonormal.
                        None => VisualPose {
                            origin,
                            rotation: Quat::from_mat3(&basis),
                            grounded: flag != 0,
                        },
                    },
                    target: target.map(f32::to_bits),
                    drawn: *pose,
                    rest,
                    near,
                    bucket: key,
                    frame,
                },
            );
        }
        bucket.recorded = frame;
        bucket.present = frame;
        bucket.pending = false;
        if !same_bits(&bucket.drawn, &bucket.next) {
            std::mem::swap(&mut bucket.drawn, &mut bucket.next);
            self.changed.push(key);
            self.parts[8] += 1;
        }
        *self.bucket_mut(key) = bucket;
    }
}

// Whether `bucket` was drawn in the frame before `frame`.
fn was_drawn_before(bucket: &Bucket, frame: u64) -> bool {
    bucket.present_before != 0 && bucket.present_before + 1 == frame
}

// Whether `car` is drawn in `frame`: its bucket is, and its recording there stands.
fn is_drawn(car: &DrawnCar, bucket: &Bucket, frame: u64) -> bool {
    bucket.present == frame && car.frame == bucket.recorded
}

fn same_target(target: &[u32; STRIDE], pose: &[f32]) -> bool {
    target
        .iter()
        .zip(pose)
        .all(|(&bits, value)| value.to_bits() == bits)
}

fn same_bits(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.to_bits() == b.to_bits())
}

// How far apart `a` and `b` are in units in the last place of the larger magnitude, at least 1:
// the unit of `REST_ULPS`.
fn ulps_apart(a: f32, b: f32) -> f32 {
    (a - b).abs() / (f32::EPSILON * a.abs().max(b.abs()).max(1.0))
}

// Within `REST_ULPS`.
fn close(a: f32, b: f32) -> bool {
    ulps_apart(a, b) <= REST_ULPS
}

/// Whether a drawn pose has reached its target to f32 resolution ([`REST_ULPS`]). A lane car
/// compares its whole transform with the orthonormalized target the smoothing approaches; an
/// off-lane car the inputs of its ground solve, its XZ and its horizontal heading, because the
/// solve owns its height and tilt.
fn at_target(pose: &[f32; STRIDE], target: &[f32; STRIDE], flag: u8) -> bool {
    if flag == 0 {
        let Some((origin, basis)) = read_pose(target) else {
            return false;
        };
        let mut wanted = [0.0; STRIDE];
        write_pose(&mut wanted, origin, orthonormalized(basis));
        return pose.iter().zip(&wanted).all(|(&a, &b)| close(a, b));
    }
    let heading = |pose: &[f32; STRIDE]| Vec2::new(pose[2], pose[10]).normalize_or_zero();
    let (drawn, wanted) = (heading(pose), heading(target));
    close(pose[3], target[3])
        && close(pose[11], target[11])
        && close(drawn.x, wanted.x)
        && close(drawn.y, wanted.y)
}

/// Godot's `Quaternion.slerp`, normalized as Godot's `Basis(Quaternion)` constructor does.
/// Up to `max` cars nearest `eye` within `radius` across row-major 3×4 pose buffers, nearest
/// first, as (buffer index, car index). Ties go to the lower buffer, then car, so the choice is
/// stable frame to frame. O(cars · log max) with a heap bounded at `max`.
pub(crate) fn nearest_cars(
    buffers: &[&[f32]],
    eye: Vec3,
    radius: f32,
    max: usize,
) -> Vec<(usize, usize)> {
    if max == 0 {
        return Vec::new();
    }
    let mut heap = std::collections::BinaryHeap::with_capacity(max + 1);
    let limit = radius * radius;
    for (buffer, poses) in buffers.iter().enumerate() {
        for (car, pose) in poses.as_chunks::<STRIDE>().0.iter().enumerate() {
            let d2 = Vec3::new(pose[3], pose[7], pose[11]).distance_squared(eye);
            // Non-negative finite f32s order like their bit patterns.
            if d2.is_nan() || d2 > limit {
                continue;
            }
            heap.push((d2.to_bits(), buffer, car));
            if heap.len() > max {
                heap.pop();
            }
        }
    }
    heap.into_sorted_vec()
        .into_iter()
        .map(|(_, buffer, car)| (buffer, car))
        .collect()
}

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
mod tests;
