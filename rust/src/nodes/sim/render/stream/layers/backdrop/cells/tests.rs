// SPDX-License-Identifier: GPL-3.0-or-later

//! The backdrop's cells (`TERRAIN-41` D2) against the recording backend: in every frame each patch
//! of the map draws exactly one of its patch and its cell, on scripted and shuffled residency
//! sequences and on the terrain layer's own frames along a camera walk.

use super::super::{BackdropLayer, Channel, Layout, Product, Sample, build_blocks};
use super::*;
use crate::nodes::sim::render::stream::backend::recorder::Recorder;
use crate::nodes::sim::render::stream::layers::terrain::{
    Activation, PatchExecutor, PatchStream, Payload, Release, TERRAIN_COSTS, TERRAIN_LIMITS,
};
use crate::nodes::sim::render::stream::schedule::{FrameBudget, MonotonicClock};
use crate::simulation::core::sparse_chunk_grid::SparseChunkGrid;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

// 37 x 20 patches of 64 cells of 8 m: 3 x 2 blocks, the last column of blocks 5 patches wide and
// the last row 4 patches high.
const GRID: PatchGrid = PatchGrid {
    cols: 37,
    rows: 20,
    interval: 64,
    cell: 8.0,
    world: [37.0 * 512.0, 20.0 * 512.0],
};
const HEIGHTS: [f32; 2] = [-548.0, 2500.0];
const MESH: u32 = 7;
const PARAMETER: u32 = 3;

// A deterministic generator (xorshift).
struct Walk(u64);
impl Walk {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

fn all_keys() -> Vec<PatchKey> {
    (0..GRID.rows)
        .flat_map(|z| (0..GRID.cols).map(move |x| [x, z]))
        .collect()
}

// A layer with the product of GRID's map uploaded and one material a block.
fn drawn_layer(recorder: &mut Recorder) -> BackdropLayer {
    drawn_layer_and_product(recorder).0
}

fn drawn_layer_and_product(recorder: &mut Recorder) -> (BackdropLayer, Product) {
    let samples = [GRID.cols as usize * 64 + 1, GRID.rows as usize * 64 + 1];
    let layout = Layout::new([GRID.cols as usize, GRID.rows as usize], 64, samples).unwrap();
    let terrain = SparseChunkGrid::new(samples[0], samples[1], 64, 12.0);
    let water = Arc::new(SparseChunkGrid::new(samples[0], samples[1], 64, 0.0));
    let mut product = Product::new(layout, 1, build_blocks(&terrain, &water, &layout));
    let mut layer = BackdropLayer::default();
    layer.reset_cells(Some(GRID), Some(&mut *recorder));
    let materials = (0..layout.block_count())
        .map(|_| recorder.material_create(99))
        .collect();
    layer.set_drawing(MESH, PARAMETER, materials, recorder);
    let mut budget = FrameBudget::new(f64::INFINITY);
    layer.upload(Some(&mut product), recorder, &mut budget, &MonotonicClock::default(), true);
    (layer, product)
}

// The patches whose cell the recorder draws: every visible main instance's first `count` rows,
// each patch once (a patch drawn twice fails).
fn drawn_cells(recorder: &Recorder) -> BTreeSet<PatchKey> {
    let mut cells = BTreeSet::new();
    for (_, instance) in recorder.visible(Scenario::Main) {
        let multimesh = &recorder.multimeshes[&instance.base];
        assert_eq!(multimesh.mesh, Some(MESH));
        assert!(instance.material_handle.is_some(), "a cell draws with its block's material");
        let count = multimesh.visible_count.expect("a count") as usize;
        assert!(count > 0, "a block with no cell does not draw");
        for row in multimesh.rows.chunks(STRIDE).take(count) {
            let [x, z] = [row[3], row[11]];
            let column = ((x + GRID.world[0] * 0.5) / 512.0).round() as i32;
            let row_index = ((z + GRID.world[1] * 0.5) / 512.0).round() as i32;
            // The custom lane names the same patch in its block.
            assert_eq!(row[12] as i32, column % 16);
            assert_eq!(row[13] as i32, row_index % 16);
            assert_eq!(GRID.rect([column, row_index]), [x, z, row[14], row[15]]);
            assert!(cells.insert([column, row_index]), "{column},{row_index} drawn twice");
        }
    }
    cells
}

// Exactly one of patch and cell for every patch of the map.
fn assert_exactly_one(recorder: &Recorder, resident: &BTreeSet<PatchKey>, label: &str) {
    let cells = drawn_cells(recorder);
    for key in all_keys() {
        assert!(
            resident.contains(&key) != cells.contains(&key),
            "{label}: patch {key:?} resident {} cell {}",
            resident.contains(&key),
            cells.contains(&key)
        );
    }
}

#[test]
fn every_patch_draws_exactly_one_of_patch_and_cell_on_scripted_and_shuffled_sequences() {
    let mut recorder = Recorder::new();
    let mut layer = drawn_layer(&mut recorder);
    let mut resident = BTreeSet::new();
    // Before any flush nothing draws; the first flush draws every patch's cell.
    assert_eq!(drawn_cells(&recorder).len(), 0);
    layer.flush(&mut recorder, HEIGHTS);
    assert_exactly_one(&recorder, &resident, "load");
    assert_eq!(layer.cell_counts(), [740, 740, 6]);
    // Scripted: a square comes in, moves by a column, a whole block goes resident, then leaves.
    let mut script: Vec<Vec<(PatchKey, bool)>> = Vec::new();
    let square = |x0: i32, z0: i32, side: i32| {
        (z0..z0 + side).flat_map(move |z| (x0..x0 + side).map(move |x| [x, z]))
    };
    script.push(square(3, 2, 12).map(|key| (key, true)).collect());
    script.push(
        (2..14)
            .map(|z| ([3, z], false))
            .chain((2..14).map(|z| ([15, z], true)))
            .collect(),
    );
    script.push(square(16, 0, 16).map(|key| (key, true)).collect());
    script.push(square(16, 0, 16).map(|key| (key, false)).collect());
    for (frame, changes) in script.iter().enumerate() {
        for &(key, on) in changes {
            layer.set_resident(key, on);
            if on {
                resident.insert(key);
            } else {
                resident.remove(&key);
            }
        }
        layer.flush(&mut recorder, HEIGHTS);
        assert_exactly_one(&recorder, &resident, &format!("scripted frame {frame}"));
    }
    // Shuffled: 300 frames of up to 60 random changes each, a patch toggled several times in a
    // frame included; then every patch resident (nothing draws), then none.
    let mut walk = Walk(0x2545_f491_4f6c_dd1d);
    let keys = all_keys();
    for frame in 0..300 {
        let changes = walk.next() % 60;
        for _ in 0..changes {
            let key = keys[(walk.next() % keys.len() as u64) as usize];
            let on = walk.next().is_multiple_of(2);
            layer.set_resident(key, on);
            if on {
                resident.insert(key);
            } else {
                resident.remove(&key);
            }
        }
        layer.flush(&mut recorder, HEIGHTS);
        assert_exactly_one(&recorder, &resident, &format!("shuffled frame {frame}"));
    }
    for &key in &keys {
        layer.set_resident(key, true);
        resident.insert(key);
    }
    layer.flush(&mut recorder, HEIGHTS);
    assert_exactly_one(&recorder, &resident, "all resident");
    assert_eq!(recorder.visible(Scenario::Main).count(), 0);
    assert_eq!(layer.cell_counts(), [0, 0, 0]);
    // A still frame sends nothing.
    let before = recorder.commands.len();
    layer.flush(&mut recorder, HEIGHTS);
    assert_eq!(recorder.commands.len(), before);
}

// `TERRAIN-41` D3: a resident patch that draws nothing (the graded book's blank: its list not
// known yet) keeps its cell; the cell goes in the flush after the patch draws and comes back when
// it stops, whichever of residency and blank changes first, in any order inside a frame.
#[test]
fn a_resident_patch_drawing_nothing_keeps_its_cell() {
    let mut recorder = Recorder::new();
    let mut layer = drawn_layer(&mut recorder);
    let mut state: BTreeMap<PatchKey, [bool; 2]> = BTreeMap::new();
    let drawn = |state: &BTreeMap<PatchKey, [bool; 2]>| -> BTreeSet<PatchKey> {
        state
            .iter()
            .filter(|(_, [resident, blank])| *resident && !*blank)
            .map(|(key, _)| *key)
            .collect()
    };
    // A patch arrives blank in the pass that made it resident: its cell stays.
    layer.set_resident([3, 2], true);
    layer.set_blank([3, 2], true);
    state.insert([3, 2], [true, true]);
    layer.flush(&mut recorder, HEIGHTS);
    assert!(layer.cell_shown([3, 2]) && !layer.patch_draws([3, 2]));
    assert_exactly_one(&recorder, &drawn(&state), "arrived blank");
    // Its list came: it draws, its cell goes.
    layer.set_blank([3, 2], false);
    state.insert([3, 2], [true, false]);
    layer.flush(&mut recorder, HEIGHTS);
    assert!(!layer.cell_shown([3, 2]) && layer.patch_draws([3, 2]));
    assert_exactly_one(&recorder, &drawn(&state), "drawing");
    // Shuffled residency and blank changes, both orders inside a frame.
    let mut walk = Walk(0x9e37_79b9_7f4a_7c15);
    let keys = all_keys();
    for frame in 0..300 {
        for _ in 0..walk.next() % 60 {
            let key = keys[(walk.next() % keys.len() as u64) as usize];
            let on = walk.next().is_multiple_of(2);
            let entry = state.entry(key).or_insert([false, false]);
            if walk.next().is_multiple_of(2) {
                layer.set_resident(key, on);
                entry[0] = on;
            } else {
                layer.set_blank(key, on);
                entry[1] = on;
            }
        }
        layer.flush(&mut recorder, HEIGHTS);
        assert_exactly_one(&recorder, &drawn(&state), &format!("shuffled frame {frame}"));
        for &key in &keys {
            assert_eq!(layer.patch_draws(key), drawn(&state).contains(&key), "frame {frame} {key:?}");
        }
    }
}

#[test]
fn a_block_draws_only_with_its_textures_and_material_and_never_in_the_reflection() {
    let mut recorder = Recorder::new();
    // No textures yet: listed, not drawn.
    let mut layer = BackdropLayer::default();
    layer.reset_cells(Some(GRID), Some(&mut recorder));
    let materials: Vec<_> = (0..6).map(|_| recorder.material_create(99)).collect();
    layer.set_drawing(MESH, PARAMETER, materials.clone(), &mut recorder);
    layer.flush(&mut recorder, HEIGHTS);
    assert_eq!(layer.cell_counts(), [740, 0, 0]);
    assert_eq!(recorder.visible(Scenario::Main).count(), 0);
    // The upload binds each block's three textures to its material, then the blocks draw.
    let mut layer = drawn_layer(&mut recorder);
    layer.flush(&mut recorder, HEIGHTS);
    assert_eq!(layer.cell_counts(), [740, 740, 6]);
    for block in 0..6 {
        let material = layer.drawing.as_ref().unwrap().materials[block];
        for channel in [Channel::Height, Channel::Water, Channel::Cover] {
            assert_eq!(
                recorder.materials[&material].textures.get(&(PARAMETER + channel as u32)),
                layer.texture(block, channel).as_ref()
            );
        }
    }
    assert_eq!(recorder.visible(Scenario::Reflection).count(), 0);
    // Freed textures (another world): unbound, and the blocks stop drawing at the next flush.
    layer.clear(&mut recorder);
    layer.flush(&mut recorder, HEIGHTS);
    assert_eq!(recorder.visible(Scenario::Main).count(), 0);
    let material = layer.drawing.as_ref().unwrap().materials[0];
    assert!(recorder.materials[&material].textures.is_empty());
}

// The scripts' side for the terrain layer: payloads arrive the frame after their request.
#[derive(Default)]
struct Scripts {
    requested: Vec<PatchKey>,
    resident: BTreeSet<PatchKey>,
}

impl PatchExecutor for Scripts {
    fn request(&mut self, keys: &[i32], _prewarm: bool, _news: &mut Vec<i32>) {
        self.requested
            .extend(keys.chunks(2).map(|pair| [pair[0], pair[1]]));
    }
    fn activate(&mut self, key: PatchKey, _step: u8) -> (Activation, i64) {
        self.resident.insert(key);
        (Activation::Done, 0)
    }
    fn deactivate(&mut self, key: PatchKey) {
        self.resident.remove(&key);
    }
    fn build(&mut self, _key: PatchKey, _step: u8) -> Option<i64> {
        Some(0)
    }
    fn set_lod(&mut self, _key: PatchKey, _step: u8) {}
    fn release(&mut self, _key: PatchKey) -> Release {
        Release::Done
    }
}

#[test]
fn the_terrain_layers_frames_hand_over_exactly_one_of_patch_and_cell() {
    // The terrain layer's own units along a walk with jumps and a small per-frame budget, so
    // holes wait over several frames; the cells follow its changes as the stream node does
    // (`follow_terrain`), and each frame's drawn set is checked against the scripts' resident
    // patches, inside and outside the wanted square.
    let mut recorder = Recorder::new();
    let mut layer = drawn_layer(&mut recorder);
    let mut terrain = PatchStream::new(TERRAIN_LIMITS, TERRAIN_COSTS);
    terrain.reset(Some(GRID));
    let mut scripts = Scripts::default();
    let mut walk = Walk(0x9e37_79b9_7f4a_7c15);
    let mut eye = [0.0_f32, 900.0, 0.0];
    let clock = MonotonicClock::default();
    let mut outside = 0;
    for step in 0..500 {
        let jump = step % 83 == 82;
        let reach = if jump { 9000.0 } else { 400.0 };
        let mut offset = |range: f32| ((walk.next() % 10_000) as f32 / 10_000.0 - 0.5) * range;
        eye[0] = (eye[0] + offset(reach)).clamp(-9400.0, 9400.0);
        eye[2] = (eye[2] + offset(reach)).clamp(-5000.0, 5000.0);
        eye[1] = (eye[1] + offset(300.0)).clamp(60.0, 3000.0);
        terrain.set_square(GRID.target(eye, 12000.0, false));
        let mut budget = FrameBudget::new(600.0);
        terrain.frame(Some(eye), 1.0 / 60.0, &mut budget, &clock, false, &|_| true, &mut scripts);
        for key in scripts.requested.drain(..) {
            terrain.payload(key, Payload::Ready);
        }
        for (key, resident) in terrain.take_changes() {
            layer.set_resident(key, resident);
        }
        layer.flush(&mut recorder, HEIGHTS);
        assert_exactly_one(&recorder, &scripts.resident, &format!("walk frame {step}"));
        let square = terrain.square().unwrap();
        outside += all_keys()
            .iter()
            .filter(|key| {
                key[0] < square[0] || key[0] > square[2] || key[1] < square[1] || key[1] > square[3]
            })
            .count();
    }
    // The walk saw patches outside the square (they draw cells) and settled holes inside it.
    assert!(outside > 0);
}

#[test]
fn an_edit_under_drawn_cells_reaches_them_through_the_bound_textures() {
    // A brush edit (heights) and a lake edit (water surface) in block 4, refreshed as D1's job
    // refreshes: the layer replaces the block's textures under their handles, which its material
    // still binds, and the cells keep drawing without a resend.
    let mut recorder = Recorder::new();
    let (mut layer, mut product) = drawn_layer_and_product(&mut recorder);
    layer.flush(&mut recorder, HEIGHTS);
    let bound = |recorder: &Recorder, layer: &BackdropLayer, channel: Channel| {
        let material = layer.drawing.as_ref().unwrap().materials[4];
        recorder.materials[&material].textures[&(PARAMETER + channel as u32)]
    };
    let before = [Channel::Height, Channel::Water].map(|channel| bound(&recorder, &layer, channel));
    let bytes = |recorder: &Recorder, texture| recorder.textures_2d[&texture].2.clone();
    let old = before.map(|texture| bytes(&recorder, texture));
    // Block 4 is the second row's second block: lattice points from (128, 128).
    let changed = product.apply_samples(&[
        Sample { lattice: [140, 140], values: [31.0, 31.0] },
        Sample { lattice: [150, 141], values: [12.0, 19.5] },
    ]);
    assert_eq!(changed, 2);
    let mut budget = FrameBudget::new(f64::INFINITY);
    let commands = recorder.commands.len();
    layer.upload(Some(&mut product), &mut recorder, &mut budget, &MonotonicClock::default(), true);
    layer.flush(&mut recorder, HEIGHTS);
    let sent = &recorder.commands[commands..];
    assert_eq!(sent.len(), 2, "two replaces, no cell resent: {sent:?}");
    for (at, channel) in [Channel::Height, Channel::Water].into_iter().enumerate() {
        assert_eq!(bound(&recorder, &layer, channel), before[at], "the handle stays bound");
        assert_ne!(bytes(&recorder, before[at]), old[at], "{channel:?} holds the edit");
    }
    assert_eq!(drawn_cells(&recorder).len(), 740);
}
