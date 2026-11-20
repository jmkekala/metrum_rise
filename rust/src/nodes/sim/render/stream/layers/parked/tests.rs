// SPDX-License-Identifier: GPL-3.0-or-later

//! The parked layer against the recorder: a chunk drawn as its bucket nodes drew it, a chunk
//! updated whole, a full pool refusing a chunk without sending anything, the teardown that
//! frees every object the layer made, and the cars' draw distance (`RENDER-48`) as the engine
//! applies a bucket's visibility range.

use super::super::super::backend::Scenario;
use super::super::super::backend::recorder::{RecordedInstance, Recorder};
use super::*;

fn poses(count: usize, x: f32) -> Vec<f32> {
    (0..count)
        .flat_map(|i| [1.0, 0.0, 0.0, x + i as f32, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0])
        .collect()
}

// A car's bounds: 2 m wide, 1.5 m high, 4.5 m long, standing on its origin.
const CAR: Aabb = Aabb {
    position: [-1.0, 0.0, -2.25],
    size: [2.0, 1.5, 4.5],
};

fn layer() -> ParkedLayer {
    let mut layer = ParkedLayer::new(64, 16);
    layer.register(10, 1, CAR);
    layer.register(21, 2, CAR);
    layer
}

// Every visible main instance: (mesh, its poses), sorted.
fn drawn(backend: &Recorder) -> Vec<(u32, Vec<f32>)> {
    let mut out: Vec<_> = backend
        .visible(Scenario::Main)
        .map(|(_, instance)| {
            let multimesh = &backend.multimeshes[&instance.base];
            let count = multimesh.visible_count.unwrap() as usize;
            (multimesh.mesh.unwrap(), multimesh.rows[..count * STRIDE].to_vec())
        })
        .collect();
    out.sort_by(|a, b| a.partial_cmp(b).unwrap());
    out
}

#[test]
fn a_chunk_draws_each_bucket_as_its_node_did() {
    let mut backend = Recorder::new();
    let mut layer = layer();
    let (a, b) = (poses(3, 0.0), poses(5, 100.0));
    assert!(layer.install(&mut backend, (0, 0), [(10, &a[..]), (21, &b[..])].into_iter()));
    assert_eq!(drawn(&backend), vec![(1, a.clone()), (2, b.clone())]);
    for (_, instance) in backend.visible(Scenario::Main) {
        assert_eq!(instance.shadow, Shadow::On);
        assert_eq!(instance.extra_margin, 12.0);
        assert_eq!(instance.material, None);
        assert_eq!(instance.origin, [0.0; 3]);
        assert!(!instance.dynamic_gi);
        let multimesh = &backend.multimeshes[&instance.base];
        assert!(!multimesh.colors && !multimesh.custom);
        let rows = &multimesh.rows[..multimesh.visible_count.unwrap() as usize * STRIDE];
        assert_eq!(multimesh.aabb, CAR.through_instances(rows, rows.len() / STRIDE), "the engine's bounds of the poses");
    }
    assert_eq!(backend.visible(Scenario::Reflection).count(), 0);
}

#[test]
fn a_chunk_update_hides_the_buckets_that_left_and_reuses_its_objects() {
    let mut backend = Recorder::new();
    let mut layer = layer();
    let (a, b, c) = (poses(3, 0.0), poses(5, 100.0), poses(4, 50.0));
    layer.install(&mut backend, (0, 0), [(10, &a[..]), (21, &b[..])].into_iter());
    let objects = backend.multimeshes.len();
    // Bucket 21 leaves (absent), bucket 10 changes its poses.
    assert!(layer.install(&mut backend, (0, 0), [(10, &c[..])].into_iter()));
    assert_eq!(drawn(&backend), vec![(1, c.clone())]);
    // Bucket 21 again in another chunk: the spare slot, no new object.
    layer.install(&mut backend, (1, 0), [(21, &b[..]), (10, &[][..])].into_iter());
    assert_eq!(backend.multimeshes.len(), objects);
    assert_eq!(drawn(&backend), vec![(1, c.clone()), (2, b.clone())]);
    // An empty install hides the chunk.
    layer.install(&mut backend, (0, 0), std::iter::empty());
    assert_eq!(drawn(&backend), vec![(2, b)]);
}

#[test]
fn a_chunk_the_full_pool_cannot_take_is_refused_whole_until_room_frees() {
    let mut backend = Recorder::new();
    let mut layer = ParkedLayer::new(2, 2);
    layer.register(10, 1, CAR);
    layer.register(21, 2, CAR);
    let (a, b, c) = (poses(3, 0.0), poses(5, 100.0), poses(2, 7.0));
    layer.install(&mut backend, (0, 0), [(10, &a[..]), (21, &b[..])].into_iter());
    let before = (backend.commands.len(), drawn(&backend));
    assert!(!layer.install(&mut backend, (1, 0), [(10, &c[..])].into_iter()));
    assert_eq!((backend.commands.len(), drawn(&backend)), before, "nothing of it sent");
    // A bucket outgrowing its slot moves within its own room, even in a full pool.
    let big = poses(40, 0.0);
    assert!(layer.install(&mut backend, (0, 0), [(10, &big[..]), (21, &b[..])].into_iter()));
    assert_eq!(drawn(&backend), vec![(1, big), (2, b)]);
    // The first chunk leaves: the refused one fits.
    layer.install(&mut backend, (0, 0), std::iter::empty());
    assert!(layer.install(&mut backend, (1, 0), [(10, &c[..])].into_iter()));
    assert_eq!(drawn(&backend), vec![(1, c)]);
}

#[test]
fn a_reset_frees_every_object_the_layer_made() {
    // The teardown (`RENDER-41`'s lesson): a world replaced or the stream node leaving frees
    // the pool's every multimesh and instance, spares included.
    let mut backend = Recorder::new();
    let mut layer = layer();
    let (a, b) = (poses(3, 0.0), poses(5, 100.0));
    layer.install(&mut backend, (0, 0), [(10, &a[..]), (21, &b[..])].into_iter());
    layer.install(&mut backend, (1, 1), [(10, &b[..])].into_iter());
    layer.install(&mut backend, (0, 0), std::iter::empty());
    assert!(layer.pool().spares() > 0 && layer.pool().live() > 0);
    layer.reset(&mut backend);
    assert!(backend.multimeshes.is_empty(), "every multimesh freed");
    assert!(backend.instances.is_empty(), "every instance freed");
    assert_eq!((layer.pool().live(), layer.pool().spares()), (0, 0));
    assert_eq!(layer.drawn().count(), 0);
}

#[test]
fn the_cast_toggle_reaches_drawn_and_later_chunks() {
    let mut backend = Recorder::new();
    let mut layer = layer();
    let (a, b) = (poses(3, 0.0), poses(5, 100.0));
    assert!(layer.install(&mut backend, (0, 0), [(10, &a[..])].into_iter()));
    layer.set_cast_shadows(&mut backend, false);
    assert!(layer.install(&mut backend, (1, 0), [(21, &b[..])].into_iter()));
    assert_eq!(backend.visible(Scenario::Main).count(), 2, "the toggle hides nothing");
    assert!(backend.visible(Scenario::Main).all(|(_, car)| car.shadow == Shadow::Off));
    layer.set_cast_shadows(&mut backend, true);
    assert!(backend.visible(Scenario::Main).all(|(_, car)| car.shadow == Shadow::On));
}

// Whether the engine draws a batch for a camera at `camera`, in the main and the shadow passes
// alike: `RendererSceneCull::_visibility_range_check` (Godot 4.7) with no margins, measured from
// the centre of the batch's bounds (its instance stands at the origin).
fn engine_draws(backend: &Recorder, instance: &RecordedInstance, camera: [f32; 3]) -> bool {
    let centre = backend.multimeshes[&instance.base].aabb.center();
    let [begin, end] = instance.range;
    let distance = length3(sub3(camera, centre));
    !(end > 0.0 && distance > end) && !(begin > 0.0 && distance < begin)
}

#[test]
fn a_bucket_draws_while_any_of_its_cars_is_within_the_draw_distance() {
    // The cars stand from x = 0 to 300 on a row and from z = 0 to 30 across it.
    let rows: Vec<f32> = (0..40)
        .flat_map(|i| {
            let (x, z) = ((i % 10) as f32 * 33.3, (i / 10) as f32 * 10.0);
            [0.0, 0.0, 1.0, x, 0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, z]
        })
        .collect();
    let mut backend = Recorder::new();
    let mut layer = layer();
    assert!(layer.install(&mut backend, (0, 0), [(10, &rows[..])].into_iter()));
    let (_, car) = backend.visible(Scenario::Main).next().expect("drawn");
    let car = car.clone();
    let origins: Vec<[f32; 3]> = rows.as_chunks::<STRIDE>().0.iter().map(|p| [p[3], p[7], p[11]]).collect();
    let nearest = |camera: [f32; 3]| {
        origins.iter().map(|&o| length3(sub3(camera, o))).fold(f32::INFINITY, f32::min)
    };
    // Cameras on rings about the bucket, from inside it to far beyond the draw distance.
    let mut cut = 0;
    let mut straddling = 0;
    for ring in 0..60 {
        let radius = 50.0 + 60.0 * ring as f32;
        for step in 0..64 {
            let angle = step as f32 * std::f32::consts::TAU / 64.0;
            let height = [2.0, 80.0, 900.0][step % 3];
            let camera = [150.0 + radius * angle.cos(), height, 15.0 + radius * angle.sin()];
            let drawn = engine_draws(&backend, &car, camera);
            let near = nearest(camera);
            if near < CAR_DRAW_DISTANCE_M {
                // A car not yet faded out whole: the batch must draw.
                assert!(drawn, "a car {near} m away is cut at {camera:?}");
            }
            if !drawn {
                cut += 1;
            }
            let far = origins.iter().map(|&o| length3(sub3(camera, o))).fold(0.0, f32::max);
            straddling += usize::from(drawn && near < CAR_DRAW_DISTANCE_M && far > CAR_DRAW_DISTANCE_M);
        }
    }
    assert!(cut > 0, "a bucket beyond the draw distance draws nothing");
    assert!(straddling > 0, "some poses straddle the draw distance (the material fades those cars)");
    // The range ends no farther out than the bucket's own extent requires.
    let centre = backend.multimeshes[&car.base].aabb.center();
    let reach = origins.iter().map(|&o| length3(sub3(centre, o))).fold(0.0, f32::max);
    assert!(car.range[1] >= CAR_DRAW_DISTANCE_M + reach);
    assert!(car.range[1] <= CAR_DRAW_DISTANCE_M + reach + 20.0, "{:?}", car.range);
    // A camera 1 m past the range from the centre, straight up: the batch is skipped.
    let above = [centre[0], centre[1] + car.range[1] + 1.0, centre[2]];
    assert!(!engine_draws(&backend, &car, above));
    assert!(nearest(above) > CAR_DRAW_DISTANCE_M);
}
