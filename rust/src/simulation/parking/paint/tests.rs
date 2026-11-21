// SPDX-License-Identifier: GPL-3.0-or-later

//! Generated bay paint (`PARK-30`): shared lines drawn once, angled places, loading bay corner
//! marks, lines on graded ground, and a changed site rebuilding only its own chunk.

use super::*;

const HALF_LINE: f32 = LINE_WIDTH_M * 0.5;

fn place(centre: [f32; 2], forward: [f32; 2], dock: bool) -> PaintPlace {
    let (width, length) = if dock { (3.5, 15.0) } else { (2.5, 5.0) };
    PaintPlace::from_forward(centre, forward, width, length, dock)
}

fn length(segment: &Segment) -> f32 {
    let [a, b] = segment;
    ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt()
}

fn close(a: [f32; 2], b: [f32; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
}

// Whether `lines` holds the segment `a`-`b`, either way round.
fn holds(lines: &[Segment], a: [f32; 2], b: [f32; 2]) -> bool {
    lines
        .iter()
        .any(|&[p, q]| (close(p, a) && close(q, b)) || (close(p, b) && close(q, a)))
}

#[test]
fn neighbouring_places_share_one_side_line_and_one_head_line() {
    // Three places side by side along x, heads to -z (forward (0, -1)), centres 2.5 m apart.
    let row: Vec<_> = (0..3)
        .map(|i| place([i as f32 * 2.5, 0.0], [0.0, -1.0], false))
        .collect();
    let lines = paint_segments(row.iter().copied());
    // Four side lines, the two inner ones shared, and the three heads as one line.
    assert_eq!(lines.len(), 5, "{lines:?}");
    for x in [-1.25, 1.25, 3.75, 6.25] {
        assert!(
            holds(&lines, [x, 2.5], [x, -2.5]),
            "side at x = {x}: {lines:?}"
        );
    }
    assert!(
        holds(&lines, [-1.25 - HALF_LINE, -2.5], [6.25 + HALF_LINE, -2.5]),
        "{lines:?}"
    );
    // The entry edge (z = 2.5) stays open.
    assert!(
        lines
            .iter()
            .all(|[a, b]| a[1] < 2.5 - 1e-3 || b[1] < 2.5 - 1e-3)
    );
    // The order of the places does not change the paint.
    let reversed = paint_segments(row.iter().rev().copied());
    assert_eq!(
        lines
            .iter()
            .flatten()
            .flatten()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        reversed
            .iter()
            .flatten()
            .flatten()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>()
    );
}

#[test]
fn places_facing_across_an_aisle_keep_the_aisle_unpainted() {
    // Two rows of two facing each other across a 7 m aisle along x = 0 (the bundled lots' plan).
    let mut places = Vec::new();
    for z in [0.0, 2.5] {
        places.push(place([-6.0, z], [-1.0, 0.0], false));
        places.push(place([6.0, z], [1.0, 0.0], false));
    }
    let lines = paint_segments(places);
    // Each row: three sides (one shared) and one head; no line crosses the aisle.
    assert_eq!(lines.len(), 8, "{lines:?}");
    for [a, b] in &lines {
        assert!(
            a[0].abs() >= 3.5 - 1e-3 && b[0].abs() >= 3.5 - 1e-3,
            "{a:?} {b:?}"
        );
    }
}

#[test]
fn angled_places_follow_their_own_axes() {
    // Two places at 30 degrees, side by side along their own across axis.
    let angle = 30f32.to_radians();
    let forward = [angle.sin(), angle.cos()];
    let across = [-forward[1], forward[0]];
    let places = [0.0, 2.5].map(|s| place([across[0] * s, across[1] * s], forward, false));
    let lines = paint_segments(places);
    assert_eq!(lines.len(), 4, "{lines:?}");
    let sides = lines
        .iter()
        .filter(|s| (length(s) - 5.0).abs() < 1e-3)
        .count();
    assert_eq!(sides, 3, "{lines:?}");
    let head = lines
        .iter()
        .find(|s| (length(s) - (5.0 + LINE_WIDTH_M)).abs() < 1e-3)
        .expect("one head line across both places");
    // The head runs across the places, square to their forward axis.
    let direction = [head[1][0] - head[0][0], head[1][1] - head[0][1]];
    assert!((direction[0] * forward[0] + direction[1] * forward[1]).abs() < 1e-3);
}

#[test]
fn a_loading_bay_gets_corner_marks_and_no_outline() {
    let lines = paint_segments([place([0.0, 0.0], [0.0, 1.0], true)]);
    assert_eq!(lines.len(), 8, "{lines:?}");
    for line in &lines {
        assert!(
            (length(line) - (CORNER_LEG_M + HALF_LINE)).abs() < 1e-3,
            "{line:?}"
        );
    }
    // Each corner of the 3.5 x 15 m bay is the outer end of two legs.
    for corner in [[-1.75, -7.5], [1.75, -7.5], [-1.75, 7.5], [1.75, 7.5]] {
        let legs = lines
            .iter()
            .filter(|[a, b]| {
                [a, b].iter().any(|p| {
                    (p[0] - corner[0]).abs() <= HALF_LINE + 1e-3
                        && (p[1] - corner[1]).abs() <= HALF_LINE + 1e-3
                })
            })
            .count();
        assert_eq!(legs, 2, "corner {corner:?}: {lines:?}");
    }
    // Two docks side by side: the shared corners' cross legs join, the shared side legs merge.
    let pair = paint_segments([0.0, 3.5].map(|x| place([x, 0.0], [0.0, 1.0], true)));
    assert_eq!(pair.len(), 12, "{pair:?}");
}

#[test]
fn a_narrow_loading_bay_edge_is_marked_whole_rather_than_overshot() {
    let dock = PaintPlace::from_forward([0.0, 0.0], [0.0, 1.0], 1.2, 15.0, true);
    let lines = paint_segments([dock]);
    // The 1.2 m ends' legs meet in the middle and merge into one end line each.
    let ends = lines
        .iter()
        .filter(|s| (length(s) - (1.2 + LINE_WIDTH_M)).abs() < 1e-3)
        .count();
    assert_eq!(ends, 2, "{lines:?}");
    assert_eq!(lines.len(), 6, "{lines:?}");
}

#[test]
fn a_line_on_a_level_pad_is_one_instance_and_a_bend_splits_it() {
    let line = [[[0.0, 0.0], [10.0, 0.0]]];
    let mut rows = Vec::new();
    paint_rows(&line, |_, _| 3.0, &mut rows);
    assert_eq!(rows.len(), STRIDE);
    // Origin at the middle, lifted above the pad; x along the line, z the line's width.
    assert_eq!([rows[3], rows[7], rows[11]], [5.0, 3.0 + LIFT_M, 0.0]);
    assert_eq!([rows[0], rows[4], rows[8]], [10.0, 0.0, 0.0]);
    assert!((rows[2].abs() + (rows[10].abs() - LINE_WIDTH_M).abs()) < 1e-6);
    // A uniform slope stays one tilted instance, its normal square to the line.
    rows.clear();
    paint_rows(&line, |x, _| x * 0.1, &mut rows);
    assert_eq!(rows.len(), STRIDE);
    let (x_axis, y_axis) = ([rows[0], rows[4], rows[8]], [rows[1], rows[5], rows[9]]);
    assert!((x_axis[1] - 1.0).abs() < 1e-5);
    assert!((x_axis[0] * y_axis[0] + x_axis[1] * y_axis[1] + x_axis[2] * y_axis[2]).abs() < 1e-5);
    // A pad that ends at x = 6 into a tie-in rising 1:2: the pieces follow the bend.
    rows.clear();
    let ground = |x: f32, _: f32| if x <= 6.0 { 0.0 } else { (x - 6.0) * 0.5 };
    paint_rows(&line, ground, &mut rows);
    let pieces = rows.len() / STRIDE;
    assert!((2..=12).contains(&pieces), "{pieces} pieces");
    for piece in rows.as_chunks::<STRIDE>().0 {
        let ends = [-0.5f32, 0.5].map(|t| {
            let x = piece[3] + piece[0] * t;
            (x, piece[7] - LIFT_M + piece[4] * t)
        });
        for (x, y) in ends {
            assert!((y - ground(x, 0.0)).abs() < 0.03, "piece end ({x}, {y})");
        }
    }
}

fn bay(x: f32, z: f32) -> Bay {
    Bay {
        identity: [0, 0],
        position: [x, 0.0, z],
        path: [[x, z]; 4],
        heading: 0.0,
        width: 2.5,
        length: 5.0,
    }
}

// Builds every dirty chunk's paint on flat ground; returns the chunks rebuilt.
fn build(s: &mut ParkingSystem) -> Vec<(i32, i32)> {
    let dirty = s.take_paint_dirty();
    let built = dirty
        .iter()
        .map(|&key| {
            let mut rows = Vec::new();
            paint_rows(&s.paint_lines(key), |_, _| 0.0, &mut rows);
            (key, rows)
        })
        .collect();
    s.install_paint(built);
    dirty
}

#[test]
fn a_changed_site_rebuilds_only_its_own_chunk_and_custody_rebuilds_nothing() {
    let mut s = ParkingSystem::default();
    // Two buildings 600 m apart, in two chunks; a kerb bay beside the first.
    s.replace_building(1, &[(0, bay(10.0, 10.0)), (1, bay(12.5, 10.0))], &[]);
    s.replace_building(2, &[(0, bay(610.0, 10.0))], &[]);
    s.replace_kerb(7, &[vec![bay(20.0, 10.0)], Vec::new()]);
    assert_eq!(build(&mut s), vec![(0, 0), (1, 0)]);
    let first = s.paint_publication();
    assert_eq!(
        first.chunks[&(0, 0)].rows.len(),
        4 * STRIDE,
        "the kerb bay draws nothing"
    );
    assert_eq!(first.chunks[&(1, 0)].rows.len(), 3 * STRIDE);
    // Cars arriving and leaving change no paint.
    s.fill_test_fleet();
    let car = s.first_parked_car_for_test();
    s.release(car);
    assert!(!s.has_paint_dirty());
    // The second building loses a bay's width: only its chunk is rebuilt and republished.
    s.replace_building(
        2,
        &[(
            0,
            Bay {
                width: 2.3,
                ..bay(610.0, 10.0)
            },
        )],
        &[],
    );
    assert_eq!(build(&mut s), vec![(1, 0)]);
    let second = s.paint_publication();
    assert!(second.revision > first.revision);
    assert_eq!(
        second.chunks[&(0, 0)].version,
        first.chunks[&(0, 0)].version
    );
    assert_eq!(second.chunks[&(1, 0)].version, second.revision);
    // Rebuilding unchanged paint publishes nothing new.
    s.replace_building(1, &[(0, bay(10.0, 10.0)), (1, bay(12.5, 10.0))], &[]);
    build(&mut s);
    assert_eq!(s.paint_publication().revision, second.revision);
    // A demolished site's chunk keeps an empty entry, so the renderer clears it.
    s.replace_building(2, &[], &[]);
    build(&mut s);
    let third = s.paint_publication();
    assert!(third.chunks[&(1, 0)].rows.is_empty());
    assert_eq!(third.chunks[&(1, 0)].version, third.revision);
}

#[test]
fn docks_paint_from_their_own_chunk_list_through_removal() {
    let mut s = ParkingSystem::default();
    let dock = |x: f32| Bay {
        width: 3.5,
        length: 15.0,
        ..bay(x, 10.0)
    };
    s.replace_docks(5, &[(0, dock(10.0)), (1, dock(20.0))], &[], &[]);
    build(&mut s);
    assert_eq!(
        s.paint_publication().chunks[&(0, 0)].rows.len(),
        16 * STRIDE
    );
    // The first dock goes: the second keeps its row and its slot is fixed up.
    s.replace_docks(5, &[(1, dock(20.0))], &[], &[]);
    assert_eq!(build(&mut s), vec![(0, 0)]);
    assert_eq!(s.paint_publication().chunks[&(0, 0)].rows.len(), 8 * STRIDE);
    s.replace_docks(5, &[], &[], &[]);
    build(&mut s);
    assert!(s.paint_publication().chunks[&(0, 0)].rows.is_empty());
    assert!(s.paint.docks.is_empty());
}
