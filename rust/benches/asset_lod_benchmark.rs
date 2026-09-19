// SPDX-License-Identifier: GPL-2.0-only

//! Isolated allocation-free authored LOD projection/selection, without a world or Godot.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use glam::{Mat4, Vec2, Vec3};
use metrum_rise::assets::lod_policy::{LodQuality, projected_size_pixels, select_lod};
use std::time::Duration;

fn bench_policy(c: &mut Criterion) {
    let mut group = c.benchmark_group("AssetLod");
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(2));
    group.sample_size(30);
    let matrix = Mat4::perspective_rh_gl(60_f32.to_radians(), 16.0 / 9.0, 0.1, 5000.0)
        * Mat4::from_translation(Vec3::new(0.0, 0.0, -100.0));
    group.bench_function("project_and_select", |b| {
        b.iter(|| {
            let pixels = projected_size_pixels(
                black_box(-Vec3::splat(10.0)),
                black_box(Vec3::splat(10.0)),
                black_box(matrix),
                black_box(Vec2::new(1920.0, 1080.0)),
            );
            black_box(select_lod(
                pixels,
                black_box(4),
                black_box(Some(1)),
                LodQuality::Balanced,
            ))
        })
    });
    group.bench_function("select_only", |b| {
        b.iter(|| {
            black_box(select_lod(
                black_box(200.0),
                black_box(4),
                black_box(Some(1)),
                LodQuality::Balanced,
            ))
        })
    });
    group.finish();
}

criterion_group!(benches, bench_policy);
criterion_main!(benches);
