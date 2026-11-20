// SPDX-License-Identifier: GPL-3.0-or-later

//! The building layer against the recorder: a frame drawn as the batch nodes drew it, missing
//! looks, size classes, a chunk leaving and coming back without new objects, chunks waiting for
//! slots alone, and the teardown.

use super::super::super::backend::recorder::{Command, Recorder};
use super::super::super::backend::Scenario;
use super::*;
use crate::assets::lod_policy::LodQuality;
use crate::nodes::sim::render::building_lod::{Chunk, Instance, Part, View};
use glam::{Mat4, Quat, Vec2, Vec3};

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
    // the new one, so in a full pool its chunk moves alone; a chunk that needs a slot for a new
    // batch waits whole, undrawn, until a later frame gives room.
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
    let frame = [change((0, 0), key(1, 0), &huge), change((1, 0), key(1, 0), &new)];
    let applied = layer.apply(&mut backend, &[], frame.into_iter());
    assert_eq!((applied.written, applied.blocked), (1, 1));
    assert!(layer.waits((1, 0)) && !layer.waits((0, 0)));
    assert_eq!(
        drawn_rows(&backend),
        sorted(vec![huge.as_flattened().to_vec(), other.as_flattened().to_vec()]),
        "the move drawn once, the new batch not at all"
    );
    // The waiting chunk again, with a frame that empties a batch: its slot is the room.
    let frame = [change((1, 0), key(1, 0), &new), change((0, 0), key(1, 1), &empty)];
    let applied = layer.apply(&mut backend, &[], frame.into_iter());
    assert_eq!((applied.written, applied.hidden, applied.blocked), (1, 1, 0));
    assert!(!layer.waiting());
    assert_eq!(
        drawn_rows(&backend),
        sorted(vec![huge.as_flattened().to_vec(), new.as_flattened().to_vec()])
    );
    assert_eq!((layer.len(), layer.pool().live()), (2, 2));
}

#[test]
fn a_reset_frees_every_object_the_layer_made() {
    // The teardown (`RENDER-41`): a new catalogue or the simulation node leaving the tree frees
    // every `MultiMesh` and instance of the pool, reflection proxies and spares included.
    let mut backend = Recorder::new();
    let mut layer = layer();
    let (a, b) = (rows(5, 0.0), rows(9, 100.0));
    let first = [change((0, 0), key(1, 0), &a), change((0, 0), key(1, 1), &b)];
    layer.apply(&mut backend, &[], first.into_iter());
    let removed = [((0, 0), key(1, 0))];
    layer.apply(&mut backend, &removed, [change((1, 0), key(1, 1), &a)].into_iter());
    assert!(layer.pool().live() > 0 && layer.pool().spares() > 0);
    assert!(backend.instances.values().any(|instance| instance.scenario == Scenario::Reflection));
    layer.reset(&mut backend);
    assert!(backend.multimeshes.is_empty(), "every multimesh freed");
    assert!(backend.instances.is_empty(), "every instance freed");
    let (created, freed) = layer.pool().churn();
    assert_eq!((layer.pool().live(), layer.pool().spares(), created), (0, 0, freed));
    assert_eq!(layer.batches().count(), 0);
}

#[test]
fn the_cast_toggle_reaches_drawn_and_later_batches_but_not_the_proxies() {
    let mut backend = Recorder::new();
    let mut layer = layer();
    let a = rows(5, 0.0);
    layer.apply(&mut backend, &[], [change((0, 0), key(1, 1), &a)].into_iter());
    layer.set_cast_shadows(&mut backend, false);
    let b = rows(3, 50.0);
    layer.apply(&mut backend, &[], [change((1, 0), key(1, 0), &b)].into_iter());
    let main: Vec<_> = backend.visible(Scenario::Main).map(|(_, main)| main.shadow).collect();
    assert_eq!(main, [Shadow::Off, Shadow::Off]);
    assert!(backend.visible(Scenario::Reflection).all(|(_, proxy)| proxy.shadow == Shadow::Off));
    layer.set_cast_shadows(&mut backend, true);
    assert!(backend.visible(Scenario::Main).all(|(_, main)| main.shadow == Shadow::On));
    assert_eq!(backend.visible(Scenario::Main).count(), 2, "the toggle hides nothing");
}

// A real LOD chunk's changed batches as the bridge offers them.
fn chunk_changes(at: ChunkKey, chunk: &Chunk) -> Vec<Change<'_>> {
    chunk
        .batches
        .iter()
        .filter(|batch| batch.changed)
        .map(|batch| change(at, batch.key, chunk.transforms(batch)))
        .collect()
}

// A building of one three-tier part, `size` its share of the view (orthographic: its projected
// size is `size` times the render size).
fn building(id: usize, size: f32, x: f32) -> Instance {
    Instance {
        building: id,
        part: 0,
        deserted: false,
        scheme: 0,
        lighting: [0.0; 4],
        transform: Mat4::from_scale_rotation_translation(
            Vec3::splat(size),
            Quat::IDENTITY,
            Vec3::new(x, 0.0, -10.0),
        ),
    }
}

fn ortho(pixels: f32) -> View {
    View {
        world_to_clip: Mat4::orthographic_rh_gl(-1.0, 1.0, -1.0, 1.0, 0.1, 1000.0),
        render_size: Vec2::splat(pixels),
        quality: LodQuality::Balanced,
    }
}

// The live batches of a chunk: (tier, count).
fn live(chunk: &Chunk) -> Vec<(usize, usize)> {
    let live = chunk.batches.iter().filter(|batch| batch.count > 0);
    live.map(|batch| (batch.key.lod, batch.count)).collect()
}

#[test]
fn a_waiting_chunk_keeps_an_emptied_batch_through_a_resync_and_draws_each_building_once() {
    // AUDIT-stream-2 finding 1: the pool is full, a chunk's buildings leave tier 0 for tiers 1
    // and 2 (tier 0 emptied) and the chunk waits; its instances then change (a resync rebuilds
    // its batches) before a slot frees. The emptied batch must still be hidden when the chunk is
    // drawn, or its old rows draw beside the new tiers.
    let parts = [Part::new([-Vec3::ONE, Vec3::ONE], &[true; 3]).unwrap()];
    let mut backend = Recorder::new();
    let mut layer = BuildingLayer::new(3, 0);
    for lod in 0..3 {
        layer.register(BatchKey { lod, ..key(0, 0) }, Look { mesh: 3 + lod as u32, material: None });
    }
    let (a, b): (ChunkKey, ChunkKey) = ((0, 0), (1, 0));
    let mut chunk = Chunk::default();
    let bridge = |layer: &mut BuildingLayer,
                      backend: &mut Recorder,
                      chunk: &mut Chunk,
                      removed: &[(ChunkKey, BatchKey)],
                      other: &[Change]| {
        let changes: Vec<_> = chunk_changes(a, chunk).into_iter().chain(other.iter().copied()).collect();
        let applied = layer.apply(backend, removed, changes.iter().copied());
        if !layer.waits(a) {
            chunk.acknowledge();
        }
        applied
    };
    // Both buildings at tier 0 (one slot); chunk b holds the pool's other two.
    let buildings = [building(0, 0.5, 0.0), building(1, 0.25, 1.0)];
    chunk.synchronize(&buildings, &parts, true);
    chunk.update(&parts, ortho(4096.0), false);
    assert_eq!(live(&chunk), [(0, 2)]);
    let (other_0, other_1) = (rows(2, 500.0), rows(2, 600.0));
    let others = [change(b, key(0, 1), &other_0), change(b, key(0, 2), &other_1)];
    bridge(&mut layer, &mut backend, &mut chunk, &[], &others);
    assert_eq!(layer.pool().available(), 0, "the pool is full");
    // Zoom out: tier 0 empties, tiers 1 and 2 need two slots against the one it gives up.
    chunk.update(&parts, ortho(600.0), false);
    assert_eq!(live(&chunk), [(1, 1), (2, 1)]);
    let applied = bridge(&mut layer, &mut backend, &mut chunk, &[], &[]);
    assert!(layer.waits(a) && applied.blocked == 1, "{applied:?}");
    // A third building arrives in the chunk while it waits: its batches are rebuilt.
    let grown = [buildings[0], buildings[1], building(2, 0.5, 2.0)];
    chunk.synchronize(&grown, &parts, false);
    chunk.update(&parts, ortho(600.0), false);
    assert_eq!(live(&chunk), [(1, 2), (2, 1)]);
    assert!(
        chunk.batches.iter().any(|batch| batch.key.lod == 0 && batch.count == 0 && batch.changed),
        "the emptied batch's change survives the resync"
    );
    // Chunk b leaves: room for chunk a, drawn whole.
    let removed = [(b, key(0, 1)), (b, key(0, 2))];
    let applied = bridge(&mut layer, &mut backend, &mut chunk, &removed, &[]);
    assert!(!layer.waiting(), "{applied:?}");
    let mut want: Vec<_> = chunk
        .batches
        .iter()
        .filter(|batch| batch.count > 0)
        .map(|batch| chunk.transforms(batch).as_flattened().to_vec())
        .collect();
    want.sort_by(|x, y| x.partial_cmp(y).unwrap());
    assert_eq!(drawn_rows(&backend), want, "each building drawn once, at its new tier");
}

#[test]
fn a_view_over_the_slot_limit_keeps_drawing_the_chunks_that_fit() {
    // AUDIT-stream-2 finding 6: a chunk that does not fit waits alone; the frame's removals and
    // every chunk that fits are drawn, and a later change of a drawn chunk is drawn at once.
    let mut backend = Recorder::new();
    let mut layer = BuildingLayer::new(2, 0);
    layer.register(key(1, 0), Look { mesh: 3, material: None });
    layer.register(key(1, 1), Look { mesh: 4, material: None });
    let (a, b, c) = (rows(3, 0.0), rows(4, 50.0), rows(5, 100.0));
    let first = [change((0, 0), key(1, 0), &a), change((1, 0), key(1, 0), &b)];
    assert_eq!(layer.apply(&mut backend, &[], first.into_iter()).written, 2);
    // Chunk (2, 0) needs two slots the full pool does not have; chunk (0, 0) changes in place.
    let a2 = rows(3, 10.0);
    let frame = [
        change((2, 0), key(1, 0), &c),
        change((2, 0), key(1, 1), &c),
        change((0, 0), key(1, 0), &a2),
    ];
    let applied = layer.apply(&mut backend, &[], frame.into_iter());
    assert_eq!((applied.written, applied.blocked), (1, 2));
    assert!(layer.waits((2, 0)) && !layer.waits((0, 0)) && !layer.waits((1, 0)));
    let sorted = |mut rows: Vec<Vec<f32>>| {
        rows.sort_by(|x, y| x.partial_cmp(y).unwrap());
        rows
    };
    assert_eq!(
        drawn_rows(&backend),
        sorted(vec![a2.as_flattened().to_vec(), b.as_flattened().to_vec()]),
        "the chunk that fits is drawn while the other waits"
    );
    // Chunk (1, 0) leaves: one slot, still short of the waiting chunk's two; it stays undrawn.
    let frame = [change((2, 0), key(1, 0), &c), change((2, 0), key(1, 1), &c)];
    let applied = layer.apply(&mut backend, &[((1, 0), key(1, 0))], frame.into_iter());
    assert_eq!((applied.hidden, applied.blocked), (1, 1));
    assert_eq!(drawn_rows(&backend), vec![a2.as_flattened().to_vec()]);
    // Chunk (0, 0) leaves too: the waiting chunk is drawn whole.
    let applied = layer.apply(&mut backend, &[((0, 0), key(1, 0))], frame.into_iter());
    assert_eq!((applied.written, applied.hidden, applied.blocked), (2, 1, 0));
    assert!(!layer.waiting());
    assert_eq!(drawn_rows(&backend), vec![c.as_flattened().to_vec(); 2]);
}
