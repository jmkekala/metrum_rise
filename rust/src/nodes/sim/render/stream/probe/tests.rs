// SPDX-License-Identifier: GPL-2.0-only

//! The drawn probe's bookkeeping on scripted sets, without Godot.

use super::*;

const UNIT: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
const FAR: [f32; 4] = [100.0, 100.0, 101.0, 101.0];
// One plane: x > 50 is outside.
const PLANES: [[f32; 4]; 1] = [[1.0, 0.0, 0.0, 50.0]];

fn set(chunks: &[([i32; 3], u32, [f32; 4])]) -> Vec<f32> {
    let mut out = Vec::new();
    for &(key, flags, rect) in chunks {
        push_chunk(&mut out, key, flags, rect);
    }
    out
}

#[test]
fn the_first_frame_is_the_baseline_and_entries_wait_until_drawn() {
    let mut layer = LayerTracker::default();
    layer.observe(1, 0, &set(&[([0, 0, 1], 0, UNIT)]), &PLANES, [0.0, 1.0]);
    // An entry drawn at once waits 0; another waits two frames, ready for the second.
    layer.observe(
        2,
        1,
        &set(&[
            ([0, 0, 1], 0, UNIT),
            ([1, 0, 1], DRAWN, UNIT),
            ([2, 0, 1], 0, UNIT),
        ]),
        &PLANES,
        [0.0, 1.0],
    );
    layer.observe(
        3,
        1,
        &set(&[
            ([0, 0, 1], DRAWN, UNIT),
            ([1, 0, 1], DRAWN, UNIT),
            ([2, 0, 1], READY, UNIT),
        ]),
        &PLANES,
        [0.0, 1.0],
    );
    layer.observe(
        4,
        2,
        &set(&[
            ([0, 0, 1], DRAWN, UNIT),
            ([1, 0, 1], DRAWN, UNIT),
            ([2, 0, 1], DRAWN, UNIT),
        ]),
        &PLANES,
        [0.0, 1.0],
    );
    let (stats, open) = layer.report(4);
    assert_eq!(stats[0].entered, 0, "the baseline enters nothing");
    assert_eq!(stats[1].entered, 2);
    assert_eq!(
        stats[1].waits,
        vec![0, 2],
        "waits count for the epoch the chunk entered in"
    );
    assert_eq!(stats[1].ready_frames, vec![0, 1]);
    assert!(open.iter().all(Vec::is_empty));
    // The baseline chunk was a hole in view for frames 1 and 2: one run of two, in epoch 0.
    assert_eq!(stats[0].holes, vec![2]);
    assert_eq!(
        stats[1].holes,
        vec![2],
        "the waiting entry's run, ready or not"
    );
}

#[test]
fn holes_out_of_view_do_not_count_and_a_cover_ends_the_visible_wait() {
    let mut layer = LayerTracker::default();
    layer.observe(1, 0, &[], &PLANES, [0.0, 1.0]);
    let frames = [
        set(&[([0, 0, 4], COVERED, UNIT), ([9, 9, 1], 0, FAR)]),
        set(&[([0, 0, 4], COVERED, UNIT), ([9, 9, 1], 0, FAR)]),
        set(&[([0, 0, 4], DRAWN, UNIT), ([9, 9, 1], DRAWN, FAR)]),
    ];
    for (at, data) in frames.iter().enumerate() {
        layer.observe(2 + at as u64, 0, data, &PLANES, [0.0, 1.0]);
    }
    let (stats, _) = layer.report(4);
    assert_eq!(stats[0].waits, vec![2, 2]);
    assert_eq!(stats[0].waits_visible, vec![0, 2]);
    assert_eq!(
        stats[0].holes,
        vec![2],
        "the far chunk is outside the frustum"
    );
    assert!(stats[0].bare.is_empty(), "the covered chunk was never bare");
    assert_eq!(stats[0].holes_max, 1);
}

#[test]
fn a_chunk_that_leaves_undrawn_is_counted_and_one_still_waiting_is_open() {
    let mut layer = LayerTracker::default();
    layer.observe(1, 0, &[], &PLANES, [0.0, 1.0]);
    layer.observe(
        2,
        0,
        &set(&[([0, 0, 1], 0, UNIT), ([1, 0, 1], 0, UNIT)]),
        &PLANES,
        [0.0, 1.0],
    );
    layer.observe(3, 0, &set(&[([1, 0, 1], 0, UNIT)]), &PLANES, [0.0, 1.0]);
    // A gap of a frame re-enters the chunk.
    layer.observe(5, 0, &set(&[([1, 0, 1], DRAWN, UNIT)]), &PLANES, [0.0, 1.0]);
    let (stats, open) = layer.report(5);
    assert_eq!(stats[0].left_undrawn, 2);
    assert_eq!(stats[0].entered, 3);
    assert_eq!(stats[0].waits, vec![0]);
    assert!(open[0].is_empty());
}

#[test]
fn the_frustum_test_takes_the_inward_corner() {
    assert!(in_frustum(&PLANES, [49.0, 0.0, 0.0], [51.0, 1.0, 1.0]));
    assert!(!in_frustum(&PLANES, [51.0, 0.0, 0.0], [52.0, 1.0, 1.0]));
    assert!(in_frustum(&[], [51.0, 0.0, 0.0], [52.0, 1.0, 1.0]));
}
