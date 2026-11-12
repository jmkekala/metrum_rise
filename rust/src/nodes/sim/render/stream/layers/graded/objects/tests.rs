// SPDX-License-Identifier: GPL-2.0-only

//! The cell objects against the recorder: a set draws only while shown under its patch node and
//! only its meshes with instances, binds its materials only while it draws, and its base cells'
//! mesh change on a written buffer reallocates first (`TERRAIN-10`).

use super::*;
use crate::nodes::sim::render::stream::backend::recorder::{Command, Recorder};

fn catalogue() -> CellCatalogue {
    CellCatalogue {
        lattice: [0, 1, 2, 3, 4, 5, 6],
        wall: 7,
        mode: 0,
        rect: 1,
        anchor: 2,
        texels: 3,
        atlas: 4,
        graded_shader: 9001,
        behind_shader: 9002,
        wall_material: 9003,
        anchor_hm: [-100, -100],
        triangles: [1; 7],
    }
}

fn resident() -> OrderedMap<PatchKey, Patch> {
    let mut resident = OrderedMap::default();
    resident.insert(
        (0, 0),
        Patch {
            rect: [-50.0, -50.0, 510.0, 510.0],
            step: 1,
            generation: 0,
        },
    );
    resident
}

fn drawing(recorder: &Recorder, objects: &CellObjects) -> usize {
    objects.sets[0]
        .parts
        .iter()
        .chain([&objects.sets[0].walls])
        .filter(|part| recorder.instances[&part.instance].visible)
        .count()
}

#[test]
fn a_set_draws_its_meshes_with_instances_only_while_shown() {
    let mut recorder = Recorder::new();
    let mut objects = CellObjects::new(catalogue(), 81);
    objects.ensure(&mut recorder, 1);
    objects.patches.insert(
        (0, 0),
        PatchDraw {
            node: 7,
            material: 9100,
            aabb: Aabb::default(),
        },
    );
    let resident = resident();
    let published = vec![0.5f32; 81 * 16];
    let apply = |objects: &mut CellObjects, recorder: &mut Recorder, effect: Effect| {
        objects.apply(recorder, &effect, &resident, &published, &[])
    };
    apply(
        &mut objects,
        &mut recorder,
        Effect::CellAcquired {
            set: 0,
            key: (0, 0),
        },
    );
    apply(
        &mut objects,
        &mut recorder,
        Effect::CellPark {
            set: 0,
            key: Some((0, 0)),
        },
    );
    apply(
        &mut objects,
        &mut recorder,
        Effect::CellBegin {
            set: 0,
            key: (0, 0),
            step: 1,
        },
    );
    apply(
        &mut objects,
        &mut recorder,
        Effect::CellBaseMesh {
            set: 0,
            code: 3,
            reset: false,
        },
    );
    apply(
        &mut objects,
        &mut recorder,
        Effect::CellCount {
            set: 0,
            mesh: 4,
            count: 60,
        },
    );
    apply(
        &mut objects,
        &mut recorder,
        Effect::CellCount {
            set: 0,
            mesh: 0,
            count: 4,
        },
    );
    assert_eq!(drawing(&recorder, &objects), 0, "a built set waits hidden");
    let base = objects.sets[0].parts[CELL_BASE].instance;
    assert_eq!(
        recorder.instances[&base].material_handle, None,
        "a pooled set binds no material"
    );
    assert_eq!(
        apply(
            &mut objects,
            &mut recorder,
            Effect::CellShow {
                set: 0,
                key: (0, 0)
            }
        ),
        Applied::Shown {
            set: 0,
            key: (0, 0)
        }
    );
    assert_eq!(
        drawing(&recorder, &objects),
        2,
        "the base cells and GL0 draw, nothing else"
    );
    assert_eq!(
        recorder.instances[&base].material_handle,
        Some(9100),
        "the base cells take the patch's material"
    );
    assert_eq!(
        recorder.instances[&base].origin,
        [205.0, 0.0, 205.0],
        "at the patch node's place"
    );
    assert_eq!(
        recorder.instances[&base].vectors[&1],
        Vector::Int4([-100, -100, 920, 920])
    );
    let gl0 = objects.sets[0].parts[0].instance;
    assert_eq!(
        recorder.instances[&gl0].material_handle,
        objects.sets[0].parts[0].material
    );
    assert_eq!(
        recorder.materials[&objects.sets[0].parts[0].material.unwrap()].shader,
        9001
    );
    apply(&mut objects, &mut recorder, Effect::CellHide { set: 0 });
    assert_eq!(drawing(&recorder, &objects), 0);
    assert!(
        objects.sets[0]
            .parts
            .iter()
            .all(|part| recorder.instances[&part.instance].material_handle.is_none()),
        "a hidden set binds nothing"
    );
}

#[test]
fn a_base_mesh_change_drops_the_buffer_first() {
    let mut recorder = Recorder::new();
    let mut objects = CellObjects::new(catalogue(), 81);
    objects.ensure(&mut recorder, 1);
    let resident = resident();
    let published = vec![0.5f32; 81 * 16];
    objects.apply(
        &mut recorder,
        &Effect::CellBaseMesh {
            set: 0,
            code: 3,
            reset: false,
        },
        &resident,
        &published,
        &[],
    );
    objects.apply(
        &mut recorder,
        &Effect::CellPublish {
            set: 0,
            mesh: 4,
            at: 0,
        },
        &resident,
        &published,
        &[],
    );
    let multimesh = objects.sets[0].parts[CELL_BASE].multimesh;
    let mark = recorder.commands.len();
    objects.apply(
        &mut recorder,
        &Effect::CellBaseMesh {
            set: 0,
            code: 4,
            reset: true,
        },
        &resident,
        &published,
        &[],
    );
    assert_eq!(
        recorder.commands[mark..],
        [
            Command::MultimeshReallocate(multimesh, 81),
            Command::MultimeshMesh(multimesh, 4)
        ]
    );
    objects.free_all(&mut recorder);
    assert!(
        recorder.instances.is_empty()
            && recorder.multimeshes.is_empty()
            && recorder.materials.is_empty()
    );
}
