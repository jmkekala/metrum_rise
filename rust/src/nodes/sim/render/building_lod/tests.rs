// SPDX-License-Identifier: GPL-2.0-only

//! Pure batching regressions; no Godot runtime or user-installed content.

use super::*;

fn part(tiers: usize) -> Part {
    Part::new([-Vec3::ONE, Vec3::ONE], &vec![true; tiers]).unwrap()
}

fn view(pixels: f32) -> View {
    View {
        world_to_clip: Mat4::orthographic_rh_gl(-1.0, 1.0, -1.0, 1.0, 0.1, 1000.0),
        render_size: Vec2::splat(pixels),
        quality: LodQuality::Balanced,
    }
}

fn instance(building: usize, part: usize, x: f32) -> Instance {
    Instance {
        building,
        part,
        deserted: false,
        scheme: 0,
        lighting: window_lighting::parameters(building as u64, true, false),
        transform: Mat4::from_translation(Vec3::new(x, 0.0, -10.0)),
    }
}

fn active(chunk: &Chunk) -> Vec<(usize, usize, usize)> {
    chunk
        .batches
        .iter()
        .filter(|batch| batch.count > 0)
        .map(|batch| (batch.key.part, batch.key.lod, batch.count))
        .collect()
}

#[test]
fn variable_chains_and_multipart_share_policy() {
    let parts = [part(1), part(4), part(5)];
    let mut chunk = Chunk::default();
    let instances = [
        instance(0, 0, 0.0),
        instance(0, 1, 2.0),
        instance(1, 2, 4.0),
    ];
    chunk.synchronize(&instances, &parts, true);
    chunk.update(&parts, view(32.0), false);
    assert_eq!(active(&chunk), [(0, 0, 1), (1, 3, 1), (2, 4, 1)]);
    chunk.acknowledge();
    chunk.update(&parts, view(900.0), false);
    assert_eq!(active(&chunk), [(0, 0, 1), (1, 0, 1), (2, 0, 1)]);
    assert!(
        !chunk.batches[0].changed,
        "unchanged single-tier group is not uploaded"
    );
    assert_eq!(chunk.batches.iter().filter(|b| b.changed).count(), 4);
}

#[test]
fn hysteresis_quality_and_large_jumps() {
    let parts = [part(5)];
    let mut chunk = Chunk::default();
    chunk.synchronize(&[instance(0, 0, 0.0)], &parts, true);
    for (pixels, lod) in [
        (600.0, 0),
        (480.0, 0),
        (450.0, 1),
        (550.0, 1),
        (570.0, 0),
        (20.0, 4),
    ] {
        chunk.update(&parts, view(pixels), false);
        assert_eq!(active(&chunk), [(0, lod, 1)]);
        chunk.acknowledge();
    }
    let mut quality = view(300.0);
    quality.quality = LodQuality::Quality;
    chunk.update(&parts, quality, true);
    assert_eq!(active(&chunk), [(0, 0, 1)]);
}

#[test]
fn warm_scatter_reuses_buffers_and_does_not_duplicate_instances() {
    let parts = [part(5)];
    let mut chunk = Chunk::default();
    let instances: Vec<_> = (0..256).map(|i| instance(i, 0, i as f32)).collect();
    chunk.synchronize(&instances, &parts, true);
    let capacities = (
        chunk.packed.capacity(),
        chunk.scratch.capacity(),
        chunk.entries.capacity(),
        chunk.batches.capacity(),
    );
    for pixels in [900.0, 300.0, 150.0, 80.0, 30.0, 900.0] {
        chunk.update(&parts, view(pixels), false);
        assert_eq!(chunk.batches.iter().map(|b| b.count).sum::<usize>(), 256);
        let batch = chunk.batches.iter().find(|b| b.count != 0).unwrap();
        for (packed, instance) in chunk.transforms(batch).iter().zip(&instances) {
            assert_eq!(&packed[12..], &instance.lighting);
        }
        assert_eq!(
            chunk
                .transforms(batch)
                .iter()
                .map(|t| t[3])
                .collect::<Vec<_>>(),
            (0..256).map(|x| x as f32).collect::<Vec<_>>()
        );
        assert_eq!(
            capacities,
            (
                chunk.packed.capacity(),
                chunk.scratch.capacity(),
                chunk.entries.capacity(),
                chunk.batches.capacity()
            )
        );
        chunk.acknowledge();
        chunk.update(&parts, view(pixels), false);
        assert!(chunk.batches.iter().all(|b| !b.changed));
        chunk.acknowledge();
    }
}

#[test]
fn removal_abandonment_and_reused_indices_clear_old_groups() {
    let parts = [part(4)];
    let mut chunk = Chunk::default();
    chunk.synchronize(&[instance(0, 0, 0.0), instance(1, 0, 2.0)], &parts, true);
    chunk.update(&parts, view(400.0), false);
    chunk.acknowledge();
    let mut survivor = instance(0, 0, 2.0);
    survivor.deserted = true;
    chunk.synchronize(&[survivor], &parts, true);
    chunk.update(&parts, view(550.0), false);
    assert_eq!(
        active(&chunk),
        [(0, 0, 1)],
        "reused index must not inherit old LOD1 hysteresis"
    );
    assert!(
        chunk
            .batches
            .iter()
            .any(|b| !b.key.deserted && b.key.lod == 1 && b.changed && b.count == 0)
    );
    chunk.acknowledge();
    chunk.synchronize(&[], &parts, true);
    chunk.update(&parts, view(550.0), false);
    assert!(active(&chunk).is_empty());
    assert!(chunk.batches.iter().any(|b| b.changed && b.key.deserted));
}

#[test]
fn visibility_bounds_include_coarse_geometry_but_ignore_spare_capacity() {
    let mut source = part(5);
    source.cull_bounds = [-Vec3::splat(2.0), Vec3::splat(2.0)];
    let parts = [source];
    let mut chunk = Chunk::default();
    let mut placed = instance(0, 0, 1000.0);
    placed.transform *= Mat4::from_scale(Vec3::new(2.0, 3.0, 4.0));
    chunk.synchronize(&[placed], &parts, true);
    chunk.packed.reserve(128);
    chunk.update(&parts, view(32.0), false);
    let batch = chunk.batches.iter().find(|batch| batch.count != 0).unwrap();
    assert_eq!(
        chunk.bounds(batch, &parts[0]),
        [Vec3::new(996.0, -6.0, -18.0), Vec3::new(1004.0, 6.0, -2.0)]
    );
    assert_eq!(parts[0].bounds, [-Vec3::ONE, Vec3::ONE]);
}

#[test]
fn failed_tiers_preserve_ordinals_and_fall_back_without_disappearing() {
    assert!(Part::new([-Vec3::ONE, Vec3::ONE], &[false; 3]).is_none());
    let fallback = Part::new([-Vec3::ONE, Vec3::ONE], &[true, false, true, false, true]).unwrap();
    assert_eq!(fallback.fallback, [0, 0, 2, 2, 4]);
    let mut chunk = Chunk::default();
    let parts = [fallback];
    chunk.synchronize(&[instance(0, 0, 0.0)], &parts, true);
    chunk.update(&parts, view(80.0), false);
    assert_eq!(active(&chunk), [(0, 2, 1)]);
}

#[test]
fn colour_schemes_split_groups_without_splitting_geometry() {
    let parts = [part(4)];
    let mut chunk = Chunk::default();
    let instances: Vec<_> = (0..6)
        .map(|i| {
            let mut placed = instance(i, 0, i as f32);
            placed.scheme = (i % 3) as u16;
            placed
        })
        .collect();
    chunk.synchronize(&instances, &parts, true);
    chunk.update(&parts, view(900.0), false);
    let occupied = |chunk: &Chunk| {
        chunk
            .batches
            .iter()
            .filter(|batch| batch.count > 0)
            .map(|batch| (batch.key.scheme, batch.key.lod, batch.count))
            .collect::<Vec<_>>()
    };
    assert_eq!(occupied(&chunk), [(0, 0, 2), (1, 0, 2), (2, 0, 2)]);
    assert_eq!(chunk.batches.iter().map(|b| b.count).sum::<usize>(), 6);
    assert!(
        chunk
            .batches
            .windows(2)
            .all(|pair| pair[0].key < pair[1].key),
        "synchronize binary-searches this list, so scheme must sort ahead of lod"
    );

    // Recolouring one instance moves it between groups without touching the rest.
    chunk.acknowledge();
    let mut recoloured = instances.clone();
    recoloured[0].scheme = 2;
    chunk.synchronize(&recoloured, &parts, true);
    chunk.update(&parts, view(900.0), false);
    assert_eq!(occupied(&chunk), [(0, 0, 1), (1, 0, 2), (2, 0, 3)]);
}

#[test]
fn level_change_rise_eases_out_and_finishes() {
    use super::spatial::level_change_rise_progress;
    use std::time::Duration;
    assert_eq!(level_change_rise_progress(Duration::ZERO), 0.0);
    let half = level_change_rise_progress(Duration::from_millis(1250));
    assert!(
        (half - 0.75).abs() < 1e-6,
        "ease-out is past halfway at half time: {half}"
    );
    assert_eq!(level_change_rise_progress(Duration::from_millis(2500)), 1.0);
    assert_eq!(level_change_rise_progress(Duration::from_secs(60)), 1.0);
}

// A deterministic generator for the walks (xorshift).
struct Walk(u64);

impl Walk {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.next()
    }
}

// The camera an orbit view gives: projection, world to clip as the bridge forms it, and its pose.
fn orbit(pivot: Vec3, yaw: f32, pitch: f32, distance: f32, size: Vec2) -> (View, spatial::Pose) {
    let direction = Vec3::new(yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos());
    let eye = pivot - direction * distance;
    let view_matrix = Mat4::look_at_rh(eye, pivot, Vec3::Y);
    // The far plane follows the distance, as `CameraNode` sets it.
    let projection = Mat4::perspective_rh_gl(
        70_f32.to_radians(),
        size.x / size.y,
        0.5,
        (distance * 4.0 + 3000.0).max(6000.0),
    );
    let camera = view_matrix.inverse();
    (
        View {
            world_to_clip: projection * view_matrix,
            render_size: size,
            quality: LodQuality::Balanced,
        },
        spatial::Pose {
            eye: camera.w_axis.truncate(),
            basis: Mat3::from_mat4(camera),
            projection,
        },
    )
}

fn selections(chunk: &Chunk) -> Vec<(usize, Option<usize>)> {
    chunk
        .entries
        .iter()
        .map(|entry| (entry.selected, entry.previous))
        .collect()
}

fn packed_groups(chunk: &Chunk) -> Vec<(BatchKey, Vec<[f32; 16]>)> {
    chunk
        .batches
        .iter()
        .map(|batch| (batch.key, chunk.transforms(batch).to_vec()))
        .collect()
}

#[test]
fn travel_buckets_keep_every_tier_the_per_frame_rule_gives_on_random_walks() {
    // Parts of many sizes and chains; instances over 3 km, rotated, scaled, at heights.
    let mut walk = Walk(0x9e37_79b9_7f4a_7c15);
    let parts: Vec<Part> = (0..12)
        .map(|index| {
            let half = Vec3::new(2.0 + 3.0 * index as f32, 3.0 + 5.0 * (index % 4) as f32, 4.0);
            Part::new([-half, half], &vec![true; 2 + index % 5]).unwrap()
        })
        .collect();
    let instances: Vec<Instance> = (0..1500)
        .map(|building| {
            let transform = Mat4::from_scale_rotation_translation(
                Vec3::splat(walk.range(0.6, 1.6)),
                glam::Quat::from_rotation_y(walk.range(0.0, 6.28)),
                Vec3::new(
                    walk.range(-1500.0, 1500.0),
                    walk.range(0.0, 40.0),
                    walk.range(-1500.0, 1500.0),
                ),
            );
            Instance {
                building,
                part: building % parts.len(),
                deserted: building % 7 == 0,
                scheme: 0,
                lighting: [0.0; 4],
                transform,
            }
        })
        .collect();
    for seed in 1..=4u64 {
        let mut lazy = Chunk::default();
        let mut reference = Chunk::default();
        lazy.synchronize(&instances, &parts, true);
        reference.synchronize(&instances, &parts, true);
        let mut travel = spatial::Travel::default();
        let size = Vec2::new(1920.0, 1080.0);
        let mut pivot = Vec3::new(0.0, 0.0, 0.0);
        let (mut yaw, mut pitch, mut distance) = (0.3f32, 0.1f32, 800.0f32);
        let mut walk = Walk(seed * 0x2545_f491_4f6c_dd1d);
        let (mut lazy_work, mut reference_work) = (0, 0);
        for frame in 0..900 {
            // Runs of pans, orbit turns, zooms, pitch changes, still frames and jumps.
            match (frame / 30 + seed as usize) % 6 {
                0 => pivot += Vec3::new(walk.range(-12.0, 12.0), 0.0, walk.range(-12.0, 12.0)),
                1 => yaw += walk.range(0.02, 0.14),
                2 => distance = (distance * walk.range(0.9, 1.1)).clamp(20.0, 3000.0),
                3 => pitch = (pitch + walk.range(-0.05, 0.05)).clamp(-0.2, 1.4),
                4 => {}
                _ => {
                    if frame % 30 == 0 {
                        pivot = Vec3::new(walk.range(-1500.0, 1500.0), 0.0, walk.range(-1500.0, 1500.0));
                    }
                    yaw -= walk.range(0.0, 0.2);
                    pivot.x += walk.range(-30.0, 30.0);
                }
            }
            let (view, pose) = orbit(pivot, yaw, pitch, distance, size);
            let motion = travel.advance(Some(pose), view);
            lazy.update_due(&parts, view, false, &motion);
            reference.update(&parts, view, false);
            lazy_work += lazy.evaluations;
            reference_work += reference.evaluations;
            assert_eq!(selections(&lazy), selections(&reference), "seed {seed} frame {frame}");
            assert_eq!(packed_groups(&lazy), packed_groups(&reference), "seed {seed} frame {frame}");
            lazy.acknowledge();
            reference.acknowledge();
        }
        assert!(
            lazy_work * 2 < reference_work,
            "the buckets skip most evaluations: {lazy_work} of {reference_work}"
        );
    }
}

#[test]
fn a_still_view_evaluates_nothing_and_a_pan_a_fraction() {
    let parts = [Part::new([-Vec3::splat(8.0), Vec3::splat(8.0)], &[true; 5]).unwrap()];
    let instances: Vec<Instance> = (0..400)
        .map(|i| instance(i, 0, (i as f32 - 200.0) * 8.0))
        .collect();
    let mut chunk = Chunk::default();
    chunk.synchronize(&instances, &parts, true);
    let mut travel = spatial::Travel::default();
    let size = Vec2::new(1920.0, 1080.0);
    let (view, pose) = orbit(Vec3::ZERO, 0.0, 0.1, 400.0, size);
    chunk.update_due(&parts, view, false, &travel.advance(Some(pose), view));
    assert_eq!(chunk.evaluations, 400, "a first frame evaluates every part");
    chunk.update_due(&parts, view, false, &travel.advance(Some(pose), view));
    assert_eq!(chunk.evaluations, 0, "a still view evaluates nothing");
    let (view, pose) = orbit(Vec3::new(4.0, 0.0, 0.0), 0.0, 0.1, 400.0, size);
    chunk.update_due(&parts, view, false, &travel.advance(Some(pose), view));
    assert!(chunk.evaluations < 100, "a 4 m pan evaluates a few parts: {}", chunk.evaluations);
}

#[test]
fn a_chunk_coming_back_equals_a_new_one_and_publishes_every_batch() {
    let parts = [Part::new([-Vec3::splat(8.0), Vec3::splat(8.0)], &[true; 5]).unwrap()];
    let instances: Vec<Instance> = (0..300)
        .map(|i| instance(i, 0, (i as f32 - 150.0) * 6.0))
        .collect();
    let size = Vec2::new(1920.0, 1080.0);
    let mut kept = Chunk::default();
    let mut travel = spatial::Travel::default();
    kept.synchronize(&instances, &parts, true);
    // History from a walk, then the chunk leaves and the camera moves on.
    for step in 0..40 {
        let (view, pose) = orbit(Vec3::new(step as f32 * 9.0, 0.0, 0.0), 0.2, 0.1, 300.0, size);
        kept.update_due(&parts, view, false, &travel.advance(Some(pose), view));
        kept.acknowledge();
    }
    let (view, pose) = orbit(Vec3::new(-200.0, 0.0, 40.0), 1.1, 0.3, 500.0, size);
    let motion = travel.advance(Some(pose), view);
    kept.reenter();
    assert!(kept.needs_update());
    kept.update_due(&parts, view, false, &motion);
    let mut fresh = Chunk::default();
    fresh.synchronize(&instances, &parts, true);
    fresh.update_due(&parts, view, false, &motion);
    assert_eq!(selections(&kept), selections(&fresh), "no history survives the chunk leaving");
    let published = |chunk: &Chunk| {
        let mut groups: Vec<_> = chunk
            .batches
            .iter()
            .filter(|batch| batch.changed && batch.count > 0)
            .map(|batch| (batch.key, chunk.transforms(batch).to_vec()))
            .collect();
        groups.sort_by_key(|group| group.0);
        groups
    };
    assert_eq!(published(&kept), published(&fresh), "every non-empty batch is published again");
    assert!(!kept.needs_update());
}
