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
