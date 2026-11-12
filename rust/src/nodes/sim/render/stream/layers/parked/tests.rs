// SPDX-License-Identifier: GPL-2.0-only

//! The parked layer against the recorder: a chunk drawn as its bucket nodes drew it, a chunk
//! updated whole, a full pool refusing a chunk without sending anything, and the teardown that
//! frees every object the layer made.

use super::super::super::backend::Scenario;
use super::super::super::backend::recorder::Recorder;
use super::*;

fn poses(count: usize, x: f32) -> Vec<f32> {
    (0..count)
        .flat_map(|i| [1.0, 0.0, 0.0, x + i as f32, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0])
        .collect()
}

fn layer() -> ParkedLayer {
    let mut layer = ParkedLayer::new(64, 16);
    layer.register(10, 1);
    layer.register(21, 2);
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
        assert_eq!(multimesh.aabb, Aabb::default(), "the engine bounds the poses");
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
    layer.register(10, 1);
    layer.register(21, 2);
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
