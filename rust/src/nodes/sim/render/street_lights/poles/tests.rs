// SPDX-License-Identifier: GPL-2.0-only

//! The pole batches' cell levels (`RENDER-46`): the engine's range rule and hysteresis, the cells
//! a chunk batch holds from a camera, a still camera's empty frame and a far jump's release, and
//! a cell's centre on its padded slot. The centres' parity with the engine's own bounds is
//! `street_lights_test.gd`'s (it needs Godot's math).

use super::super::LampBook;
use super::*;

const LEVELS: [Level; LODS] = [
    Level {
        begin: 0.0,
        end: 140.0,
        begin_margin: 0.0,
        end_margin: 10.0,
        cull_margin: 32.0,
    },
    Level {
        begin: 140.0,
        end: 420.0,
        begin_margin: 10.0,
        end_margin: 10.0,
        cull_margin: 32.0,
    },
    Level {
        begin: 420.0,
        end: 1400.0,
        begin_margin: 10.0,
        end_margin: 10.0,
        cull_margin: 2.0,
    },
];
const SPAN: f32 = 512.0;
const MESH: Aabb = Aabb {
    position: [-0.5, 0.0, -0.5],
    size: [2.0, 9.0, 1.0],
};

// Lights along x at z = 10 every 20 m from x = 4 (chunk-local, kind 0 and every fifth kind 2):
// cells of 64 m along the chunk.
fn chunk_lights() -> Vec<f32> {
    (0..25)
        .flat_map(|i| {
            [
                4.0 + 20.0 * i as f32,
                0.0,
                10.0,
                0.0,
                if i % 5 == 0 { 2.0 } else { 0.0 },
            ]
        })
        .collect()
}

fn chunk(origin: [f32; 3]) -> ChunkPoles {
    let lamps = LampBook::default();
    let (batches, _) = lamps.chunk_batches(origin, &chunk_lights());
    ChunkPoles::new(&[[Some(MESH); LODS]; KINDS], &LEVELS, origin, &batches)
}

fn select(poles: &mut PoleLods, eye: [f32; 3]) -> Vec<i32> {
    poles.select(eye, SPAN, [0.0, 0.0]).to_vec()
}

// The poles a batch holds, as their x.
fn held(poles: &PoleLods, key: (i32, i32), slot: usize, lod: usize) -> Vec<f32> {
    let mut out = Vec::new();
    poles.chunk(key).unwrap().batch(slot, lod, &mut out);
    out.as_chunks::<12>().0.iter().map(|row| row[3]).collect()
}

#[test]
fn the_range_rule_keeps_a_shown_level_through_its_margins() {
    let far = LEVELS[2];
    // Hidden: enters only inside the narrowed range.
    assert!(!far.shows(false, 425.0));
    assert!(far.shows(false, 431.0));
    assert!(!far.shows(false, 1395.0));
    // Shown: stays inside the widened range.
    assert!(far.shows(true, 415.0));
    assert!(far.shows(true, 1405.0));
    assert!(!far.shows(true, 1411.0));
    assert!(!far.shows(true, 409.0));
    // The first level has no begin.
    assert!(LEVELS[0].shows(false, 0.0));
}

#[test]
fn a_cell_measures_from_its_padded_slots_bounds() {
    // The first cell (one pole of kind 2): its slot's 16 instances repeat it.
    let chunk = chunk([0.0; 3]);
    let first = chunk.centres().next().unwrap();
    let rows = &chunk.transforms[0][..12];
    let expect = slot_bounds(MESH, rows, 16).grow(32.0).center();
    assert_eq!(first[0], expect);
}

#[test]
fn a_chunk_batch_holds_the_cells_in_its_level_from_the_camera() {
    let mut poles = PoleLods::new(LEVELS);
    poles.insert((0, 0), chunk([0.0; 3]));
    let lods = poles.chunk((0, 0)).unwrap();
    assert_eq!(lods.kinds(), &[2, 0]);
    // Thirteen cells: eight of kind 0 and five of kind 2, in first-seen order.
    let centres: Vec<_> = lods.centres().copied().collect();
    assert_eq!(centres.len(), 13);
    let eye = [0.0, 50.0, 400.0];
    assert!(!select(&mut poles, eye).is_empty());
    let chunk = poles.chunk((0, 0)).unwrap();
    for (slot, lod) in [(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 2)] {
        let mut expect: Vec<f32> = Vec::new();
        for (cell, centre) in chunk.cells.iter().zip(&centres) {
            let distance = length3(sub3(centre[lod], eye));
            if usize::from(cell.slot) == slot && LEVELS[lod].shows(false, distance) {
                let transforms = &chunk.transforms[slot];
                let first = cell.first as usize * 12;
                let rows = &transforms[first..first + cell.count as usize * 12];
                expect.extend(rows.as_chunks::<12>().0.iter().map(|row| row[3]));
            }
        }
        assert_eq!(
            held(&poles, (0, 0), slot, lod),
            expect,
            "slot {slot} level {lod}"
        );
    }
    assert!(!held(&poles, (0, 0), 1, 1).is_empty());
    assert!(!held(&poles, (0, 0), 1, 2).is_empty());
    // A still camera changes nothing.
    assert!(select(&mut poles, eye).is_empty());
}

#[test]
fn a_cell_keeps_its_level_inside_the_hysteresis_on_the_way_back() {
    let mut poles = PoleLods::new(LEVELS);
    poles.insert((0, 0), chunk([0.0; 3]));
    let centre = poles.chunk((0, 0)).unwrap().centres().next().unwrap()[2];
    // From 1,000 m out to 1,405 m (past the end, inside its margin), then back to 1,395 m.
    let at = |d: f32| [centre[0], centre[1], centre[2] + d];
    select(&mut poles, at(1000.0));
    let first = held(&poles, (0, 0), 0, 2);
    assert!(!first.is_empty());
    select(&mut poles, at(1405.0));
    assert!(held(&poles, (0, 0), 0, 2).contains(&first[0]), "shown inside the margin");
    select(&mut poles, at(1412.0));
    assert!(!held(&poles, (0, 0), 0, 2).contains(&first[0]), "hidden past it");
    select(&mut poles, at(1405.0));
    assert!(!held(&poles, (0, 0), 0, 2).contains(&first[0]), "not back inside it");
    select(&mut poles, at(1389.0));
    assert!(held(&poles, (0, 0), 0, 2).contains(&first[0]), "back inside the range");
}

#[test]
fn a_jump_releases_every_drawn_level_and_a_reinserted_chunk_starts_hidden() {
    let mut poles = PoleLods::new(LEVELS);
    let far = [4608.0, 0.0, 4608.0];
    poles.insert((0, 0), chunk([0.0; 3]));
    poles.insert((9, 9), chunk(far));
    select(&mut poles, [100.0, 30.0, 100.0]);
    assert!(poles.lit.contains(&(0, 0)) && !poles.lit.contains(&(9, 9)));
    // Thousands of metres away: the first chunk is evaluated (it is lit) and empties.
    let changed = select(&mut poles, [4700.0, 30.0, 4700.0]);
    let keys: Vec<_> = changed.as_chunks::<3>().0.iter().map(|t| (t[0], t[1])).collect();
    assert!(keys.contains(&(0, 0)) && keys.contains(&(9, 9)));
    for slot in 0..2 {
        for lod in 0..LODS {
            assert!(held(&poles, (0, 0), slot, lod).is_empty());
        }
    }
    assert!(!poles.lit.contains(&(0, 0)));
    // A replaced chunk holds nothing until the next frame evaluates it, from the same camera.
    poles.insert((9, 9), chunk(far));
    assert!(held(&poles, (9, 9), 1, 0).is_empty());
    assert!(!select(&mut poles, [4700.0, 30.0, 4700.0]).is_empty());
    assert!(!held(&poles, (9, 9), 1, 0).is_empty());
    // A removed chunk is gone from the next selection.
    assert!(poles.remove((9, 9)).is_some());
    assert!(select(&mut poles, [4701.0, 30.0, 4700.0]).is_empty());
}
