// SPDX-License-Identifier: GPL-3.0-or-later

//! Guide kernel benchmark (`ROAD-112` round A): flattening a 1 km arc and a 1 km cubic Bezier
//! into a reused buffer, at 10 km from the origin.
//!
//! Allocation check without an allocator hook: with `METRUM_GUIDE_ALLOC_PROBE=<n>` the binary
//! warms one buffer per path, flattens both paths `n` times and exits before Criterion starts.
//! Run it under `heaptrack --record-only` with `n = 0` and with `n = 100000`: equal allocation
//! counts mean a warmed flatten allocates nothing.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use metrum_rise::simulation::network::guide::{Arc, CubicBezier, DVec2, GuidePath, GuideSegment};

const ORIGIN: DVec2 = DVec2::new(10_000.0, -10_000.0);

// A 1 km arc: radius 500 m through 2 rad (250 points by the 4 m step).
fn arc_path() -> GuidePath {
    GuidePath::from_segment(GuideSegment::Arc(
        Arc::from_centre(ORIGIN, 500.0, 0.3, 2.0).expect("arc"),
    ))
}

// A gently curving span of about 1 km.
fn bezier_path() -> GuidePath {
    let p = |x: f64, z: f64| ORIGIN + DVec2::new(x, z);
    GuidePath::from_segment(GuideSegment::Bezier(
        CubicBezier::new(p(0.0, 0.0), p(350.0, 0.0), p(650.0, 200.0), p(950.0, 250.0))
            .expect("bezier"),
    ))
}

// The allocation probe (module docs): `n` warmed flattens of each path, then exit.
fn allocation_probe(iterations: usize, paths: &[GuidePath]) -> ! {
    let mut buffers: Vec<Vec<DVec2>> = paths
        .iter()
        .map(|path| {
            let mut points = Vec::new();
            path.flatten_into(&mut points).expect("within the step cap");
            points
        })
        .collect();
    for _ in 0..iterations {
        for (path, points) in paths.iter().zip(&mut buffers) {
            path.flatten_into(black_box(points))
                .expect("within the step cap");
        }
    }
    std::process::exit(0)
}

fn bench_flatten(c: &mut Criterion) {
    let (arc, bezier) = (arc_path(), bezier_path());
    if let Some(iterations) = std::env::var("METRUM_GUIDE_ALLOC_PROBE")
        .ok()
        .and_then(|value| value.parse().ok())
    {
        allocation_probe(iterations, &[arc, bezier]);
    }
    let mut group = c.benchmark_group("road_tool_guide");
    let mut points = Vec::new();
    group.bench_function("flatten_arc_1km", |b| {
        b.iter(|| black_box(&arc).flatten_into(&mut points))
    });
    group.bench_function("flatten_bezier_1km", |b| {
        b.iter(|| black_box(&bezier).flatten_into(&mut points))
    });
    group.finish();
}

criterion_group!(benches, bench_flatten);
criterion_main!(benches);
