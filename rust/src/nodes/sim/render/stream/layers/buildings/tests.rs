// SPDX-License-Identifier: GPL-2.0-only

//! The building layer against the recorder: a frame drawn as the batch nodes drew it, missing
//! looks, size classes, and a chunk leaving and coming back without new objects.

use super::super::super::backend::recorder::{Command, Recorder};
use super::super::super::backend::Scenario;
use super::*;

fn key(part: usize, lod: usize) -> BatchKey {
    BatchKey {
        part,
        deserted: false,
        scheme: 0,
        lod,
    }
}

fn rows(count: usize, x: f32) -> Vec<[f32; 16]> {
    (0..count)
        .map(|i| {
            let mut row = [0.0f32; 16];
            row[0] = 1.0;
            row[5] = 1.0;
            row[10] = 1.0;
            row[3] = x + i as f32;
            row[12] = 0.5;
            row
        })
        .collect()
}

fn change(chunk: ChunkKey, key: BatchKey, rows: &[[f32; 16]]) -> Change<'_> {
    Change {
        chunk,
        key,
        rows,
        bounds: Aabb::new([0.0; 3], [10.0, 10.0, 10.0]),
    }
}

fn layer() -> BuildingLayer {
    let mut layer = BuildingLayer::new(64, 16);
    layer.register(key(1, 0), Look { mesh: 3, material: None });
    layer.register(key(1, 1), Look { mesh: 4, material: Some(2) });
    layer
}

#[test]
fn a_frame_draws_each_batch_as_its_node_did_with_its_reflection_proxy() {
    let mut backend = Recorder::new();
    let mut layer = layer();
    let a = rows(5, 0.0);
    let applied = layer.apply(&mut backend, &[], [change((0, 0), key(1, 1), &a)].into_iter());
    assert_eq!(applied.written, 1);
    assert_eq!(applied.bytes, 5 * 16 * 4);
    let main: Vec<_> = backend.visible(Scenario::Main).collect();
    let reflection: Vec<_> = backend.visible(Scenario::Reflection).collect();
    assert_eq!((main.len(), reflection.len()), (1, 1));
    let (_, main) = main[0];
    assert_eq!(main.material, Some(2));
    assert_eq!(main.shadow, Shadow::On);
    assert!(main.dynamic_gi);
    assert_eq!(main.extra_margin, 32.0);
    let (_, proxy) = reflection[0];
    assert_eq!(proxy.base, main.base, "the proxy shares the batch's buffer");
    assert_eq!(proxy.shadow, Shadow::Off);
    assert_eq!(proxy.material, Some(2));
    assert_eq!(proxy.flags, [(REFLECTION_FLAG, true)]);
    assert_eq!((proxy.range, proxy.range_margins), ([0.0, 1000.0], [0.0, 32.0]));
    let multimesh = &backend.multimeshes[&main.base];
    assert_eq!(multimesh.mesh, Some(4));
    assert_eq!(multimesh.visible_count, Some(5));
    assert_eq!(multimesh.rows, a.as_flattened());
    assert_eq!(multimesh.aabb, Aabb::new([0.0; 3], [10.0, 10.0, 10.0]));
}

#[test]
fn a_group_without_its_look_is_named_once() {
    let mut layer = layer();
    let (a, empty) = (rows(2, 0.0), rows(0, 0.0));
    let missing = layer
        .missing(
            [
                change((0, 0), key(1, 0), &a),
                change((0, 0), key(7, 2), &a),
                change((1, 0), key(7, 2), &a),
                change((1, 0), key(8, 0), &empty),
            ]
            .into_iter(),
        )
        .to_vec();
    assert_eq!(missing, [key(7, 2)], "an empty batch needs no look");
}

#[test]
fn a_chunk_leaving_and_coming_back_reuses_its_objects() {
    let mut backend = Recorder::new();
    let mut layer = layer();
    let (a, b) = (rows(5, 0.0), rows(9, 100.0));
    let frame = |chunk| [change(chunk, key(1, 0), &a), change(chunk, key(1, 1), &b)];
    layer.apply(&mut backend, &[], frame((0, 0)).into_iter());
    let objects = backend.multimeshes.len();
    // The chunk leaves the query: hidden, kept as spares.
    let removed = [((0, 0), key(1, 0)), ((0, 0), key(1, 1))];
    let applied = layer.apply(&mut backend, &removed, std::iter::empty());
    assert_eq!(applied.hidden, 2);
    assert_eq!(backend.visible(Scenario::Main).count(), 0);
    assert_eq!(backend.visible(Scenario::Reflection).count(), 0);
    // Another chunk with the same groups and sizes takes the spares: rows only.
    let before = backend.commands.len();
    layer.apply(&mut backend, &[], frame((3, 1)).into_iter());
    assert_eq!(backend.multimeshes.len(), objects);
    assert!(
        backend.commands[before..]
            .iter()
            .all(|command| !matches!(command, Command::MultimeshCreate(..) | Command::InstanceCreate(..))),
        "no object made on a frame path"
    );
    assert_eq!(backend.visible(Scenario::Main).count(), 2);
    assert_eq!(backend.visible(Scenario::Reflection).count(), 2);
}

#[test]
fn a_batch_keeps_its_slot_while_it_fits_and_moves_when_it_grows() {
    let mut backend = Recorder::new();
    let mut layer = layer();
    let big = rows(40, 0.0);
    layer.apply(&mut backend, &[], [change((0, 0), key(1, 0), &big)].into_iter());
    let first = layer.batches().next().unwrap().2;
    let small = rows(3, 0.0);
    layer.apply(&mut backend, &[], [change((0, 0), key(1, 0), &small)].into_iter());
    assert_eq!(layer.batches().next().unwrap().2, first, "a shrinking batch keeps its slot");
    let huge = rows(200, 0.0);
    layer.apply(&mut backend, &[], [change((0, 0), key(1, 0), &huge)].into_iter());
    let moved = layer.batches().next().unwrap().2;
    assert_ne!(moved, first);
    assert_eq!(layer.len(), 1);
    assert_eq!(backend.visible(Scenario::Main).count(), 1, "one version drawn");
    let empty = rows(0, 0.0);
    let applied = layer.apply(&mut backend, &[], [change((0, 0), key(1, 0), &empty)].into_iter());
    assert_eq!((applied.hidden, layer.len()), (1, 0));
    assert_eq!(backend.visible(Scenario::Main).count(), 0);
}

// The rows every visible main instance draws, by multimesh, for "exactly one version" checks.
fn drawn_rows(backend: &Recorder) -> Vec<Vec<f32>> {
    let mut out: Vec<_> = backend
        .visible(Scenario::Main)
        .map(|(_, instance)| {
            let multimesh = &backend.multimeshes[&instance.base];
            let count = multimesh.visible_count.unwrap() as usize;
            multimesh.rows[..count * 16].to_vec()
        })
        .collect();
    out.sort_by(|a, b| a.partial_cmp(b).unwrap());
    out
}

#[test]
fn a_frame_the_full_pool_refuses_waits_whole_and_draws_once_a_slot_frees() {
    // `RENDER-42`: two slots, both live; a third batch's frame sends nothing, keeps the old
    // frame drawn, and draws whole when a later frame's removal makes room.
    let mut backend = Recorder::new();
    let mut layer = BuildingLayer::new(2, 2);
    layer.register(key(1, 0), Look { mesh: 3, material: None });
    layer.register(key(1, 1), Look { mesh: 4, material: None });
    let (a, b, c) = (rows(3, 0.0), rows(4, 50.0), rows(5, 100.0));
    let first = [change((0, 0), key(1, 0), &a), change((0, 0), key(1, 1), &b)];
    assert_eq!(layer.apply(&mut backend, &[], first.into_iter()).written, 2);
    let before = (backend.commands.len(), drawn_rows(&backend));
    let blocked = [change((1, 0), key(1, 0), &c)];
    let applied = layer.apply(&mut backend, &[], blocked.into_iter());
    assert_eq!((applied.written, applied.hidden, applied.blocked), (0, 0, 1));
    assert!(layer.waiting(), "the refused frame waits");
    assert_eq!(
        (backend.commands.len(), drawn_rows(&backend)),
        before,
        "nothing of a refused frame is sent"
    );
    // Offered again unchanged: still short, still nothing sent.
    layer.apply(&mut backend, &[], blocked.into_iter());
    assert!(layer.waiting());
    assert_eq!(backend.commands.len(), before.0);
    // The bridge offers the waiting changes with the next frame's: a chunk leaves.
    let removed = [((0, 0), key(1, 1))];
    let applied = layer.apply(&mut backend, &removed, blocked.into_iter());
    assert_eq!((applied.written, applied.hidden, applied.blocked), (1, 1, 0));
    assert!(!layer.waiting());
    assert_eq!(drawn_rows(&backend), {
        let mut want = vec![a.as_flattened().to_vec(), c.as_flattened().to_vec()];
        want.sort_by(|x, y| x.partial_cmp(y).unwrap());
        want
    });
}

#[test]
fn a_batch_that_outgrows_its_slot_in_a_full_pool_is_drawn_once_or_waits_whole() {
    // `RENDER-42`: a batch moving to a larger size class gives up its old slot before it takes
    // the new one, so in a full pool it moves alone; a frame that also needs a slot for a new
    // batch waits whole, with the old rows the only version drawn, until a removal gives room.
    let sorted = |mut rows: Vec<Vec<f32>>| {
        rows.sort_by(|a, b| a.partial_cmp(b).unwrap());
        rows
    };
    let mut backend = Recorder::new();
    let mut layer = BuildingLayer::new(2, 0);
    layer.register(key(1, 0), Look { mesh: 3, material: None });
    layer.register(key(1, 1), Look { mesh: 4, material: None });
    let (small, other, medium) = (rows(5, 0.0), rows(6, 50.0), rows(40, 0.0));
    let (huge, new, empty) = (rows(200, 0.0), rows(3, 900.0), rows(0, 0.0));
    layer.apply(
        &mut backend,
        &[],
        [change((0, 0), key(1, 0), &small), change((0, 0), key(1, 1), &other)].into_iter(),
    );
    let applied = layer.apply(&mut backend, &[], [change((0, 0), key(1, 0), &medium)].into_iter());
    assert_eq!((applied.written, applied.blocked), (1, 0), "a move alone fits a full pool");
    assert_eq!(
        drawn_rows(&backend),
        sorted(vec![medium.as_flattened().to_vec(), other.as_flattened().to_vec()])
    );
    let before = (backend.commands.len(), drawn_rows(&backend));
    let frame = [change((0, 0), key(1, 0), &huge), change((1, 0), key(1, 0), &new)];
    let applied = layer.apply(&mut backend, &[], frame.into_iter());
    assert_eq!((applied.written, applied.blocked), (0, 1));
    assert!(layer.waiting());
    assert_eq!(
        (backend.commands.len(), drawn_rows(&backend)),
        before,
        "the old rows, once, until the frame fits"
    );
    let applied = layer.apply(
        &mut backend,
        &[],
        frame
            .into_iter()
            .chain([change((0, 0), key(1, 1), &empty)]),
    );
    assert_eq!((applied.written, applied.hidden, applied.blocked), (2, 1, 0));
    assert!(!layer.waiting());
    assert_eq!(
        drawn_rows(&backend),
        sorted(vec![huge.as_flattened().to_vec(), new.as_flattened().to_vec()])
    );
    assert_eq!((layer.len(), layer.pool().live()), (2, 2));
}
