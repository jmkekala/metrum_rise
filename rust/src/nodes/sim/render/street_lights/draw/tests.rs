// SPDX-License-Identifier: GPL-2.0-only

//! The lamp draw against the recording backend: a chunk's batches draw with the settings the
//! batch nodes had, only once all are made, and every slot is freed by a reset.

use super::*;
use crate::nodes::sim::render::stream::backend::Scenario;
use crate::nodes::sim::render::stream::backend::recorder::{Command, Recorder};
use crate::nodes::sim::render::street_lights::LampBook;

// Forty lights over every kind in one 512 m chunk (the GDScript test's fixture).
fn lights() -> Vec<f32> {
    (0..40)
        .flat_map(|i| {
            let i = i as f32;
            [
                -250.0 + i * 12.31,
                3.7 + i * 0.05,
                240.0 - i * 11.77,
                -3.1 + i * 0.157,
                i % 5.0,
            ]
        })
        .collect()
}

fn batches(origin: [f32; 3]) -> ChunkBatches {
    let mut book = LampBook::default();
    let mut far = vec![0u8; super::super::FAR_BYTES];
    book.set_chunk((1, -2), origin, &lights(), &mut far)
}

#[test]
fn a_chunk_draws_its_batches_as_the_nodes_did() {
    let origin = [768.0, 0.0, -768.0];
    let expected = batches(origin);
    let mut backend = Recorder::new();
    let mut draw = LampDraw::default();
    draw.begin(&mut backend, (1, -2), origin, batches(origin));
    // One batch a part: nothing draws until the last.
    while !draw.build_part(&mut backend, (1, -2), 0.0) {
        assert_eq!(backend.visible(Scenario::Main).count(), 0);
        assert!(draw.is_building((1, -2)));
    }
    let drawn: Vec<_> = backend.visible(Scenario::Main).collect();
    assert_eq!(drawn.len(), expected.kinds.len() * LODS + 1);
    let mut lod_rows = 0;
    for (_, instance) in &drawn {
        assert_eq!(instance.origin, origin);
        let multimesh = &backend.multimeshes[&instance.base];
        let mesh = multimesh.mesh.expect("a mesh");
        if mesh == GLOW_MESH {
            assert_eq!(multimesh.rows, expected.glow);
            assert!(multimesh.colors);
            assert_eq!((instance.material, instance.shadow), (None, Shadow::Off));
            assert_eq!(instance.extra_margin, RECEIVER_MARGIN_M);
            assert_eq!(instance.range, [0.0; 2]);
            continue;
        }
        let lod = mesh as usize % LODS;
        assert_eq!(instance.material, Some(LAMP_MATERIAL));
        assert_eq!(instance.range, LOD_RANGES[lod]);
        let begin = if lod > 0 { LOD_HYSTERESIS_M } else { 0.0 };
        assert_eq!(instance.range_margins, [begin, LOD_HYSTERESIS_M]);
        let (shadow, margin) = if lod < 2 {
            (Shadow::On, CASTER_MARGIN_M)
        } else {
            (Shadow::Off, RECEIVER_MARGIN_M)
        };
        assert_eq!((instance.shadow, instance.extra_margin), (shadow, margin));
        assert_eq!(
            multimesh.visible_count,
            Some((multimesh.rows.len() / STRIDE) as u32)
        );
        lod_rows += multimesh.rows.len();
    }
    let lamp_floats: usize = expected.transforms.iter().map(Vec::len).sum();
    assert_eq!(lod_rows, lamp_floats * LODS);
    assert_eq!(draw.drawn_instances((1, -2)), (40 * 3, 48));
}

#[test]
fn prewarmed_spares_serve_the_first_batches_and_a_reset_frees_every_slot() {
    let origin = [768.0, 0.0, -768.0];
    let mut backend = Recorder::new();
    let mut draw = LampDraw::default();
    draw.prewarm(&mut backend);
    let warm = draw.ledger();
    assert_eq!(
        (warm.made, warm.live, warm.spares),
        (MESHES as u64, 0, MESHES)
    );
    assert_eq!(backend.visible(Scenario::Main).count(), 0);
    draw.begin(&mut backend, (1, -2), origin, batches(origin));
    assert!(draw.build_part(&mut backend, (1, -2), f64::INFINITY));
    let built = draw.ledger();
    // Every lamp mesh's first small batch took its spare (the glow's 48 heads are a larger
    // class): nothing made beyond the chunk's slots.
    assert_eq!((built.spares, built.made), (1, built.live as u64 + 1));
    // A replaced chunk takes its own slots back.
    draw.begin(&mut backend, (1, -2), origin, batches(origin));
    assert_eq!(backend.visible(Scenario::Main).count(), 0);
    assert!(draw.build_part(&mut backend, (1, -2), f64::INFINITY));
    assert_eq!(draw.ledger().made, built.made);
    draw.remove(&mut backend, (1, -2));
    assert_eq!(backend.visible(Scenario::Main).count(), 0);
    assert_eq!(draw.drawn_instances((1, -2)), (0, 0));
    // A reset keeps the slots as spares; the exit frees them all.
    draw.begin(&mut backend, (1, -2), origin, batches(origin));
    assert!(draw.build_part(&mut backend, (1, -2), f64::INFINITY));
    draw.clear(&mut backend);
    let reset = draw.ledger();
    assert_eq!((reset.live, reset.spares), (0, built.live + 1));
    assert_eq!(backend.visible(Scenario::Main).count(), 0);
    draw.free_all(&mut backend);
    let end = draw.ledger();
    assert_eq!((end.made, end.live, end.spares), (end.freed, 0, 0));
    assert!(backend.instances.is_empty() && backend.multimeshes.is_empty());
}

#[test]
fn the_shadow_toggle_reaches_drawn_casters_only() {
    let origin = [0.0; 3];
    let mut backend = Recorder::new();
    let mut draw = LampDraw::default();
    draw.begin(&mut backend, (1, -2), origin, batches(origin));
    assert!(draw.build_part(&mut backend, (1, -2), f64::INFINITY));
    draw.set_cast_shadows(&mut backend, false);
    assert!(
        backend
            .visible(Scenario::Main)
            .all(|(_, i)| i.shadow == Shadow::Off)
    );
    draw.set_cast_shadows(&mut backend, true);
    let casting = backend
        .visible(Scenario::Main)
        .filter(|(_, i)| i.shadow == Shadow::On)
        .count();
    assert_eq!(casting, batches(origin).kinds.len() * 2);
}

#[test]
fn a_prebuilt_chunk_only_shows_when_its_batches_come() {
    let origin = [768.0, 0.0, -768.0];
    let mut backend = Recorder::new();
    let mut draw = LampDraw::default();
    let made = draw.prebuild(
        &mut backend,
        [((1, -2), origin, batches(origin))].into_iter(),
    );
    // The prewarm's spares are made first and taken where a batch's class is the smallest.
    assert_eq!(made + MESHES as u64, draw.ledger().made);
    assert_eq!(backend.visible(Scenario::Main).count(), 0);
    let commands = backend.commands.len();
    // The same batches: one part shows them, nothing made or written.
    draw.begin(&mut backend, (1, -2), origin, batches(origin));
    assert!(draw.build_part(&mut backend, (1, -2), 0.0));
    assert_eq!(draw.ledger().made, made + MESHES as u64);
    assert!(
        backend.commands[commands..]
            .iter()
            .all(|command| matches!(command, Command::InstanceVisible(_, true)))
    );
    assert_eq!(draw.drawn_instances((1, -2)), (40 * 3, 48));
    // Another world's prebuild releases what is not drawn and frees the spares.
    let other = [0.0, 0.0, 0.0];
    draw.prebuild(&mut backend, [((0, 0), other, batches(other))].into_iter());
    // A chunk whose batches differ from its prebuilt ones builds them anew.
    draw.begin(
        &mut backend,
        (0, 0),
        [1.0, 0.0, 0.0],
        batches([1.0, 0.0, 0.0]),
    );
    assert!(draw.build_part(&mut backend, (0, 0), f64::INFINITY));
    assert!(
        backend
            .visible(Scenario::Main)
            .all(|(_, instance)| instance.origin != other)
    );
    draw.free_all(&mut backend);
    assert!(backend.instances.is_empty() && backend.multimeshes.is_empty());
}
