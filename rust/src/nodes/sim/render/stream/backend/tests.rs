// SPDX-License-Identifier: GPL-2.0-only

//! The slot pool against the recorder: reuse by kind, the limit and the spare limit (invariant
//! 6), exhaustion, eviction, fresh storage, and the padding of a buffer.

use super::recorder::{Command, Recorder};
use super::*;

fn kind(mesh: u32, count: u32) -> SlotKind {
    SlotKind {
        mesh,
        class: capacity_class(count).0,
        colors: true,
        custom: false,
        parameter: None,
    }
}

fn drawn(origin: [f32; 3]) -> Drawn {
    Drawn {
        origin,
        aabb: Aabb::default(),
        range: [0.0, 420.0],
        material: None,
        shadow: Shadow::Off,
        visible: true,
    }
}

fn rows(count: u32, x: f32) -> Vec<f32> {
    (0..count)
        .flat_map(|i| {
            let mut row = [0.0f32; 16];
            row[0] = 1.0;
            row[5] = 1.0;
            row[10] = 1.0;
            row[3] = x + i as f32;
            row
        })
        .collect()
}

#[test]
fn capacity_classes_hold_the_count_within_a_third() {
    assert_eq!(capacity_class(1), (0, 16));
    assert_eq!(capacity_class(16), (0, 16));
    assert_eq!(capacity_class(17), (1, 24));
    assert_eq!(capacity_class(25), (2, 32));
    assert_eq!(capacity_class(33), (3, 48));
    assert_eq!(capacity_class(4096), (16, 4096));
    for count in 17..5000 {
        let (_, capacity) = capacity_class(count);
        assert!(capacity >= count && f64::from(capacity) <= f64::from(count) * 1.5 + 1.0);
    }
}

#[test]
fn padding_repeats_the_first_instance() {
    let rows = rows(3, 10.0);
    let mut out = vec![-1.0f32; 16 * 16];
    pad_rows(&mut out, &rows, 16);
    assert_eq!(&out[..48], &rows[..]);
    for pad in out[48..].as_chunks::<16>().0 {
        assert_eq!(pad, &rows[..16]);
    }
}

#[test]
fn a_released_slot_is_reused_for_its_kind_without_new_objects() {
    let mut backend = Recorder::new();
    let mut pool = SlotPool::new(8, 4);
    let a = pool.acquire(&mut backend, kind(7, 20)).unwrap();
    pool.fill(&mut backend, a, &rows(20, 0.0), 20, Aabb::default());
    assert!(pool.draw(&mut backend, a, Role::Main, drawn([1.0, 0.0, 2.0])));
    let instance = pool.slot(a).instance(Role::Main).unwrap();
    assert!(backend.instances[&instance].visible);
    pool.release(&mut backend, a);
    assert!(
        !backend.instances[&instance].visible,
        "a released slot hides at once"
    );
    assert_eq!((pool.live(), pool.spares()), (0, 1));
    let created = backend.multimeshes.len();
    let before = backend.commands.len();
    let b = pool.acquire(&mut backend, kind(7, 18)).unwrap();
    assert_eq!(a, b, "the spare of the same kind comes back");
    assert_eq!(backend.multimeshes.len(), created);
    assert_eq!(
        backend.commands.len(),
        before,
        "a reuse sends nothing until written"
    );
    pool.fill(&mut backend, b, &rows(18, 5.0), 18, Aabb::default());
    pool.draw(&mut backend, b, Role::Main, drawn([1.0, 0.0, 2.0]));
    // Only the buffer, the visible count and the visibility changed.
    assert!(matches!(
        backend.commands[before],
        Command::MultimeshBuffer(..)
    ));
    assert!(matches!(
        backend.commands[before + 1],
        Command::MultimeshVisible(_, 18)
    ));
    assert!(matches!(
        backend.commands[before + 2],
        Command::InstanceVisible(_, true)
    ));
    assert_eq!(backend.commands.len(), before + 3);
    // A different mesh never takes the spare: its buffer would be read back.
    pool.release(&mut backend, b);
    let c = pool.acquire(&mut backend, kind(8, 18)).unwrap();
    assert_ne!(c, b);
    assert_eq!(backend.multimeshes[&pool.slot(c).multimesh].mesh, Some(8));
}

#[test]
fn the_pool_holds_its_limits_and_evicts_the_oldest_spare() {
    let mut backend = Recorder::new();
    let mut pool = SlotPool::new(4, 2);
    let slots: Vec<SlotId> = (0..4)
        .map(|mesh| pool.acquire(&mut backend, kind(mesh, 4)).unwrap())
        .collect();
    assert_eq!(
        pool.acquire(&mut backend, kind(9, 4)),
        None,
        "every slot of the limit is live"
    );
    assert_eq!(pool.available(), 0);
    for &slot in &slots {
        pool.release(&mut backend, slot);
        assert!(pool.live() + pool.spares() <= 4 && pool.spares() <= 2);
    }
    assert_eq!(
        (pool.live(), pool.spares()),
        (0, 2),
        "past the spare limit a slot is freed"
    );
    assert_eq!(backend.multimeshes.len(), 2);
    // The spares are meshes 0 and 1 (2 and 3 found the spare limit reached and were freed); a new
    // kind evicts the oldest of them only once the limit is reached.
    let a = pool.acquire(&mut backend, kind(10, 4)).unwrap();
    let b = pool.acquire(&mut backend, kind(11, 4)).unwrap();
    assert_eq!(backend.multimeshes.len(), 4);
    let c = pool.acquire(&mut backend, kind(12, 4)).unwrap();
    let meshes: Vec<_> = backend
        .multimeshes
        .values()
        .map(|multimesh| multimesh.mesh)
        .collect();
    assert!(
        !meshes.contains(&Some(0)) && meshes.contains(&Some(1)),
        "the oldest spare goes first"
    );
    assert_eq!(pool.live(), 3);
    let d = pool.acquire(&mut backend, kind(3, 4)).unwrap();
    assert_eq!(pool.live(), 4);
    assert_eq!(pool.acquire(&mut backend, kind(13, 4)), None);
    for slot in [a, b, c, d] {
        pool.release(&mut backend, slot);
    }
    pool.clear(&mut backend);
    assert!(backend.multimeshes.is_empty() && backend.instances.is_empty());
    let (created, freed) = pool.churn();
    assert_eq!(created, freed);
}

#[test]
fn fresh_storage_resets_a_reused_slot_and_forces_its_count() {
    let mut backend = Recorder::new();
    let mut pool = SlotPool::new(4, 4);
    pool.set_fresh(true);
    let a = pool.acquire(&mut backend, kind(1, 16)).unwrap();
    pool.fill(&mut backend, a, &rows(16, 0.0), 16, Aabb::default());
    pool.release(&mut backend, a);
    let before = backend.commands.len();
    let b = pool.acquire(&mut backend, kind(1, 16)).unwrap();
    assert!(matches!(
        backend.commands[before],
        Command::MultimeshReallocate(_, 16)
    ));
    pool.fill(&mut backend, b, &rows(16, 0.0), 16, Aabb::default());
    let multimesh = &backend.multimeshes[&pool.slot(b).multimesh];
    assert_eq!(
        multimesh.visible_count,
        Some(16),
        "the count is sent again after fresh storage"
    );
}

#[test]
fn reflection_and_caster_instances_share_the_multimesh() {
    let mut backend = Recorder::new();
    backend.reflection = false;
    let mut pool = SlotPool::new(2, 2);
    let a = pool.acquire(&mut backend, kind(1, 4)).unwrap();
    pool.fill(&mut backend, a, &rows(4, 0.0), 4, Aabb::default());
    assert!(pool.draw(&mut backend, a, Role::Caster, drawn([0.0; 3])));
    assert!(
        !pool.draw(&mut backend, a, Role::Reflection, drawn([0.0; 3])),
        "no reflection world"
    );
    backend.reflection = true;
    assert!(pool.draw(&mut backend, a, Role::Reflection, drawn([0.0; 3])));
    let slot = pool.slot(a);
    for role in [Role::Main, Role::Caster, Role::Reflection] {
        let instance = slot.instance(role).unwrap();
        assert_eq!(backend.instances[&instance].base, slot.multimesh);
    }
    let reflection = slot.instance(Role::Reflection).unwrap();
    assert_eq!(
        backend.instances[&reflection].scenario,
        Scenario::Reflection
    );
    pool.drop_reflections(&mut backend);
    assert!(!backend.instances.contains_key(&reflection));
}

#[test]
fn a_kept_slot_swaps_its_mesh_into_fresh_storage_without_a_read_back() {
    let mut backend = Recorder::new();
    let mut pool = SlotPool::new(4, 4);
    let a = pool.acquire(&mut backend, kind(1, 20)).unwrap();
    let written = rows(20, 5.0);
    pool.fill_kept(&mut backend, a, &written, 20, Aabb::default());
    let multimesh = pool.slot(a).multimesh;
    let before = backend.commands.len();
    pool.swap_mesh(&mut backend, a, 7);
    // Fresh storage first, the mesh before any buffer, then the kept rows and the count: never a
    // mesh change on a written buffer (`RENDER-37`).
    assert!(matches!(
        &backend.commands[before..],
        [
            Command::MultimeshReallocate(m0, 24),
            Command::MultimeshMesh(m1, 7),
            Command::MultimeshBuffer(m2, _, 16, 24),
            Command::MultimeshVisible(m3, 20),
        ] if [*m0, *m1, *m2, *m3] == [multimesh; 4]
    ));
    let recorded = &backend.multimeshes[&multimesh];
    assert_eq!(recorded.mesh, Some(7));
    assert_eq!(recorded.rows, written);
    assert_eq!(recorded.visible_count, Some(20));
    assert_eq!(pool.slot(a).kind().mesh, 7, "pooled under the new mesh");
    // The same mesh again sends nothing; a plain fill drops the copy, so a later swap falls back.
    let before = backend.commands.len();
    pool.swap_mesh(&mut backend, a, 7);
    assert_eq!(backend.commands.len(), before);
    pool.fill(&mut backend, a, &rows(3, 0.0), 3, Aabb::default());
    let before = backend.commands.len();
    pool.swap_mesh(&mut backend, a, 2);
    assert_eq!(&backend.commands[before..], &[Command::MultimeshMesh(multimesh, 2)]);
}
