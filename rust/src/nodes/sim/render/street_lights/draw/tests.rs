// SPDX-License-Identifier: GPL-2.0-only

//! The lamp draw against the recording backend: a chunk's batches draw with the settings the
//! batch nodes had, a batch a pole kind and level holding its cells at their levels, only once
//! all are made, and every slot is freed by a reset.

use super::*;
use crate::nodes::sim::render::stream::backend::Scenario;
use crate::nodes::sim::render::stream::backend::recorder::{Command, Recorder};
use crate::nodes::sim::render::street_lights::LampBook;
use crate::nodes::sim::render::vegetation_product::godot_math::Aabb;

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

const MESH: Aabb = Aabb {
    position: [-0.5, 0.0, -0.5],
    size: [2.0, 9.0, 1.0],
};
const SPAN: f32 = 512.0;

fn draw() -> LampDraw {
    let mut draw = LampDraw::default();
    draw.set_mesh_bounds([[Some(MESH); LODS]; KINDS]);
    draw
}

fn select(draw: &mut LampDraw, backend: &mut Recorder, eye: [f32; 3]) -> u32 {
    draw.select(backend, eye, SPAN, [0.0, 0.0])
}

#[test]
fn a_chunk_draws_a_batch_a_kind_and_level_holding_its_cells_levels() {
    let origin = [768.0, 0.0, -768.0];
    let expected = batches(origin);
    let mut backend = Recorder::new();
    let mut draw = draw();
    draw.begin(&mut backend, (1, -2), origin, batches(origin));
    // One kind a part: nothing draws until the last.
    while !draw.build_part(&mut backend, (1, -2), 0.0) {
        assert_eq!(backend.visible(Scenario::Main).count(), 0);
        assert!(draw.is_building((1, -2)));
    }
    // The glow draws at once, the level batches from the first selection.
    assert_eq!(draw.drawn_instances((1, -2)), (0, 48));
    let mut kinds = expected.kinds.clone();
    kinds.sort_unstable();
    kinds.dedup();
    let slots = draw.drawn[&(1, -2)].clone();
    assert_eq!(slots.len(), kinds.len() * LODS + 1);
    for (batch, &id) in slots.iter().enumerate() {
        let slot = draw.pool.slot(id);
        let instance = &backend.instances[&slot.instance(Role::Main).unwrap()];
        assert_eq!(instance.origin, origin);
        let multimesh = &backend.multimeshes[&instance.base];
        if batch == kinds.len() * LODS {
            assert_eq!(multimesh.mesh, Some(GLOW_MESH));
            assert_eq!(multimesh.rows, expected.glow);
            assert!(multimesh.colors && instance.visible);
            assert_eq!((instance.material, instance.shadow), (None, Shadow::Off));
            assert_eq!(instance.extra_margin, RECEIVER_MARGIN_M);
            continue;
        }
        let lod = batch % LODS;
        let kind = u32::from(draw.poles.chunk((1, -2)).unwrap().kinds()[batch / LODS]);
        assert_eq!(multimesh.mesh, Some(kind * LODS as u32 + lod as u32));
        assert_eq!(instance.material, Some(LAMP_MATERIAL));
        // No range of their own: the cells' levels are the selection's.
        assert_eq!(instance.range, [0.0; 2]);
        let (shadow, margin) = if lod < 2 {
            (Shadow::On, CASTER_MARGIN_M)
        } else {
            (Shadow::Off, RECEIVER_MARGIN_M)
        };
        assert_eq!((instance.shadow, instance.extra_margin), (shadow, margin));
        assert!(!instance.visible);
    }
    // From a camera above the chunk: each batch holds its kind's poles at that level, shown
    // when it holds any.
    let check = |draw: &LampDraw, backend: &Recorder| {
        let mut held = 0;
        for (batch, &id) in slots[..kinds.len() * LODS].iter().enumerate() {
            let mut rows = Vec::new();
            draw.pole_batch((1, -2), batch / LODS, batch % LODS, &mut rows);
            let slot = draw.pool.slot(id);
            let instance = &backend.instances[&slot.instance(Role::Main).unwrap()];
            assert_eq!(instance.visible, !rows.is_empty(), "batch {batch}");
            if !rows.is_empty() {
                let multimesh = &backend.multimeshes[&instance.base];
                assert_eq!(multimesh.rows, rows);
                assert_eq!(multimesh.visible_count, Some((rows.len() / STRIDE) as u32));
                held += rows.len() / STRIDE;
            }
        }
        held
    };
    assert!(select(&mut draw, &mut backend, [768.0, 300.0, -768.0]) > 0);
    let near = check(&draw, &backend);
    assert!(near > 0);
    assert_eq!(draw.drawn_instances((1, -2)), (near as u32, 48));
    // A still camera writes nothing; a far one hides every level batch.
    let commands = backend.commands.len();
    assert_eq!(select(&mut draw, &mut backend, [768.0, 300.0, -768.0]), 0);
    assert_eq!(backend.commands.len(), commands);
    select(&mut draw, &mut backend, [768.0, 300.0, 4000.0]);
    assert_eq!(check(&draw, &backend), 0);
    assert_eq!(draw.drawn_instances((1, -2)), (0, 48));
}

#[test]
fn prewarmed_spares_serve_the_first_batches_and_a_reset_frees_every_slot() {
    let origin = [768.0, 0.0, -768.0];
    let mut backend = Recorder::new();
    let mut draw = draw();
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
    let mut draw = draw();
    draw.begin(&mut backend, (1, -2), origin, batches(origin));
    assert!(draw.build_part(&mut backend, (1, -2), f64::INFINITY));
    draw.set_cast_shadows(&mut backend, false);
    assert!(backend.instances.values().all(|i| i.shadow == Shadow::Off));
    draw.set_cast_shadows(&mut backend, true);
    let casting = backend
        .instances
        .values()
        .filter(|i| i.shadow == Shadow::On)
        .count();
    // A batch a kind and level: the first two levels cast.
    assert_eq!(casting, draw.poles.chunk((1, -2)).unwrap().kinds().len() * 2);
}

#[test]
fn a_prebuilt_chunk_only_shows_when_its_batches_come() {
    let origin = [768.0, 0.0, -768.0];
    let mut backend = Recorder::new();
    let mut draw = draw();
    let made = draw.prebuild(
        &mut backend,
        [((1, -2), origin, batches(origin))].into_iter(),
    );
    // The prewarm's spares are made first and taken where a batch's class is the smallest.
    assert_eq!(made + MESHES as u64, draw.ledger().made);
    assert_eq!(backend.visible(Scenario::Main).count(), 0);
    let commands = backend.commands.len();
    // The same batches: one part shows the glow, nothing made or written; the first selection
    // writes the level batches.
    draw.begin(&mut backend, (1, -2), origin, batches(origin));
    assert!(draw.build_part(&mut backend, (1, -2), 0.0));
    assert_eq!(draw.ledger().made, made + MESHES as u64);
    assert!(
        backend.commands[commands..]
            .iter()
            .all(|command| matches!(command, Command::InstanceVisible(_, true)))
    );
    assert_eq!(draw.drawn_instances((1, -2)), (0, 48));
    assert!(select(&mut draw, &mut backend, [768.0, 300.0, -768.0]) > 0);
    assert_eq!(draw.ledger().made, made + MESHES as u64);
    assert!(draw.drawn_instances((1, -2)).0 > 0);
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
