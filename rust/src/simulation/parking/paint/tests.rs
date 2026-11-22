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
    // Four side lines, the two inner ones shared and centred on the shared edge, the outer ones
    // inside their places; and the three heads as one line inside the places, its outer edge on
    // their head edge.
    assert_eq!(lines.len(), 5, "{lines:?}");
    for x in [-1.25 + HALF_LINE, 1.25, 3.75, 6.25 - HALF_LINE] {
        assert!(
            holds(&lines, [x, 2.5], [x, -2.5]),
            "side at x = {x}: {lines:?}"
        );
    }
    assert!(
        holds(&lines, [-1.25, -2.5 + HALF_LINE], [6.25, -2.5 + HALF_LINE]),
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
    // Along the forward axis: the three sides; square to it: one head across both places.
    let along = |s: &Segment| {
        let direction = [s[1][0] - s[0][0], s[1][1] - s[0][1]];
        (direction[0] * forward[0] + direction[1] * forward[1]).abs() / length(s)
    };
    let mut sides = lines.iter().filter(|s| (along(s) - 1.0).abs() < 1e-3);
    assert_eq!(sides.clone().count(), 3, "{lines:?}");
    assert!(sides.all(|s| (length(s) - 5.0).abs() < 1e-3));
    let head = lines
        .iter()
        .find(|s| along(s) < 1e-3)
        .expect("one head line across both places");
    assert!((length(head) - 5.0).abs() < 1e-3, "{head:?}");
    assert_paint_inside(&places, &lines);
}

#[test]
fn a_loading_bay_gets_corner_marks_and_no_outline() {
    let lines = paint_segments([place([0.0, 0.0], [0.0, 1.0], true)]);
    assert_eq!(lines.len(), 8, "{lines:?}");
    for line in &lines {
        assert!(
            (length(line) - CORNER_LEG_M).abs() < 1e-3,
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
        .filter(|s| (length(s) - 1.2).abs() < 1e-3)
        .count();
    assert_eq!(ends, 2, "{lines:?}");
    assert_eq!(lines.len(), 6, "{lines:?}");
    assert_paint_inside(&[dock], &lines);
}

// The four corners of the 0.10 m quad `paint_rows` draws for `line`.
fn quad_corners([a, b]: &Segment) -> [[f32; 2]; 4] {
    let d = [b[0] - a[0], b[1] - a[1]];
    let n = (d[0] * d[0] + d[1] * d[1]).sqrt();
    let side = [-d[1] / n * HALF_LINE, d[0] / n * HALF_LINE];
    [
        [a[0] + side[0], a[1] + side[1]],
        [a[0] - side[0], a[1] - side[1]],
        [b[0] + side[0], b[1] + side[1]],
        [b[0] - side[0], b[1] - side[1]],
    ]
}

// Whether `p` lies in `place`'s rectangle, to a tenth of a millimetre.
fn inside(place: &PaintPlace, p: [f32; 2]) -> bool {
    let (s, c) = place.heading.sin_cos();
    let forward = [-s, -c];
    let across = [-forward[1], forward[0]];
    let r = [p[0] - place.centre[0], p[1] - place.centre[1]];
    let (f, a) = (
        r[0] * forward[0] + r[1] * forward[1],
        r[0] * across[0] + r[1] * across[1],
    );
    f.abs() <= place.length * 0.5 + 1e-4 && a.abs() <= place.width * 0.5 + 1e-4
}

// Every painted quad lies inside the places: each of its corners in one of them, and its middle
// too (a line centred on a shared edge has its corners in either place).
fn assert_paint_inside(places: &[PaintPlace], lines: &[Segment]) {
    for line in lines {
        for p in quad_corners(line) {
            assert!(
                places.iter().any(|place| inside(place, p)),
                "{line:?} reaches {p:?} outside the places"
            );
        }
    }
}

// `PARK-30` (owner, 2026-11-22): a line lies fully inside the place, its outer edge on the
// place's edge, so it never paints a kerb, a walkway or the ground beside the lot. Where two
// places share an edge, one line centred on it lies half in each.
#[test]
fn no_paint_lies_outside_the_places() {
    let angle = 30f32.to_radians();
    let tilted = [angle.sin(), angle.cos()];
    for single in [
        place([0.0, 0.0], [0.0, -1.0], false),
        place([3.0, -7.0], tilted, false),
        place([0.0, 0.0], [0.0, 1.0], true),
        place([5.0, 2.0], tilted, true),
        PaintPlace::from_forward([0.0, 0.0], [1.0, 0.0], 1.2, 15.0, true),
    ] {
        let lines = paint_segments([single]);
        assert!(!lines.is_empty());
        assert_paint_inside(&[single], &lines);
        // A lone place's head line lies on its head edge, inward: the outer edge of the quad
        // touches the edge.
        if !single.dock {
            let reach = lines
                .iter()
                .flat_map(quad_corners)
                .map(|p| {
                    let (s, c) = single.heading.sin_cos();
                    (p[0] - single.centre[0]) * -s + (p[1] - single.centre[1]) * -c
                })
                .fold(f32::MIN, f32::max);
            assert!((reach - single.length * 0.5).abs() < 1e-4, "head reach {reach}");
        }
    }
    // A row, two rows head to head, and two docks side by side.
    let row: Vec<_> = (0..3)
        .map(|i| place([i as f32 * 2.5, 0.0], [0.0, -1.0], false))
        .collect();
    let mut back_to_back = row.clone();
    back_to_back.extend((0..3).map(|i| place([i as f32 * 2.5, -5.0], [0.0, 1.0], false)));
    let docks = [0.0, 3.5].map(|x| place([x, 0.0], [0.0, 1.0], true));
    for places in [row, back_to_back, docks.to_vec()] {
        let lines = paint_segments(places.iter().copied());
        assert_paint_inside(&places, &lines);
    }
    // The shared head of two rows is one line, centred on the shared edge (z = -2.5).
    let lines = paint_segments(
        (0..3)
            .map(|i| place([i as f32 * 2.5, 0.0], [0.0, -1.0], false))
            .chain((0..3).map(|i| place([i as f32 * 2.5, -5.0], [0.0, 1.0], false))),
    );
    assert!(holds(&lines, [-1.25, -2.5], [6.25, -2.5]), "{lines:?}");
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

// Builds every dirty building's paint on flat ground; returns the buildings rebuilt.
fn build(s: &mut ParkingSystem) -> Vec<u64> {
    let dirty = s.take_paint_dirty();
    let built = dirty
        .iter()
        .map(|&building| {
            let (lines, chunk) = s.paint_lines(building);
            let mut rows = Vec::new();
            paint_rows(&lines, |_, _| 0.0, &mut rows);
            (building, chunk, rows)
        })
        .collect();
    s.install_paint(built);
    dirty
}

#[test]
fn a_changed_site_rebuilds_only_its_own_paint_and_custody_rebuilds_nothing() {
    let mut s = ParkingSystem::default();
    // Two buildings 600 m apart, in two chunks; a kerb bay beside the first.
    s.replace_building(1, &[(0, bay(10.0, 10.0)), (1, bay(12.5, 10.0))], &[]);
    s.replace_building(2, &[(0, bay(610.0, 10.0))], &[]);
    s.replace_kerb(7, &[vec![bay(20.0, 10.0)], Vec::new()]);
    assert_eq!(build(&mut s), vec![1, 2]);
    let first = s.paint_publication();
    assert_eq!(
        first.chunks[&(0, 0)].rows().len(),
        4 * STRIDE,
        "the kerb bay draws nothing"
    );
    assert_eq!(first.chunks[&(1, 0)].rows().len(), 3 * STRIDE);
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
    assert_eq!(build(&mut s), vec![2]);
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
    assert!(third.chunks[&(1, 0)].is_empty());
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
        s.paint_publication().chunks[&(0, 0)].rows().len(),
        16 * STRIDE
    );
    // The first dock goes: the second keeps its row and its slot is fixed up.
    s.replace_docks(5, &[(1, dock(20.0))], &[], &[]);
    assert_eq!(build(&mut s), vec![5]);
    assert_eq!(
        s.paint_publication().chunks[&(0, 0)].rows().len(),
        8 * STRIDE
    );
    s.replace_docks(5, &[], &[], &[]);
    build(&mut s);
    assert!(s.paint_publication().chunks[&(0, 0)].is_empty());
    assert!(s.paint.docks.is_empty());
}

#[test]
fn a_chunk_shows_its_buildings_paint_one_after_another_and_a_ground_change_repaints_its_lots() {
    let mut s = ParkingSystem::default();
    s.replace_building(1, &[(0, bay(10.0, 10.0))], &[]);
    s.replace_building(2, &[(0, bay(40.0, 10.0))], &[]);
    build(&mut s);
    let both = s.paint_publication();
    assert_eq!(both.chunks[&(0, 0)].rows().len(), 6 * STRIDE);
    // A ground change under the second lot repaints it alone.
    s.invalidate_ground([39.0, 9.0, 41.0, 11.0]);
    assert_eq!(s.take_paint_dirty(), vec![2]);
    // Its lot gone, the chunk keeps the first lot's rows, as they were.
    s.replace_building(2, &[], &[]);
    build(&mut s);
    let one = s.paint_publication();
    assert_eq!(
        &one.chunks[&(0, 0)].rows()[..],
        &both.chunks[&(0, 0)].rows()[..3 * STRIDE]
    );
}

#[test]
fn a_burst_of_marked_lots_paints_over_snapshots_to_the_rows_of_one_build() {
    // Forty lots of three places (120): more than one snapshot's share.
    let lots = |s: &mut ParkingSystem| {
        for lot in 0..40u64 {
            let x = lot as f32 * 10.0;
            let bays = [
                (0, bay(x, 10.0)),
                (1, bay(x + 2.5, 10.0)),
                (2, bay(x + 5.0, 10.0)),
            ];
            s.replace_building(lot + 1, &bays, &[]);
        }
    };
    let mut s = ParkingSystem::default();
    lots(&mut s);
    let mut batches = Vec::new();
    while s.has_paint_dirty() {
        batches.push(build(&mut s).len());
    }
    // 32 places a snapshot: the 11th lot of three crosses it, the rest wait for the next ones.
    assert_eq!(batches, vec![11, 11, 11, 7]);
    // Marked all at once (a load), the same lots paint in one snapshot, to the same rows.
    let mut whole = ParkingSystem::default();
    lots(&mut whole);
    whole.mark_all_paint();
    assert_eq!(build(&mut whole).len(), 40);
    assert!(!whole.has_paint_dirty());
    let bits = |s: &ParkingSystem| {
        let publication = s.paint_publication();
        let chunks = publication.chunks.iter();
        chunks
            .map(|(key, chunk)| (*key, chunk.rows().iter().map(|f| f.to_bits()).collect()))
            .collect::<Vec<(_, Vec<u32>)>>()
    };
    assert_eq!(bits(&s), bits(&whole));
    // A snapshot with nothing marked takes nothing and publishes nothing.
    let revision = s.paint_publication().revision;
    assert!(s.take_paint_dirty().is_empty());
    s.install_paint(Vec::new());
    assert_eq!(s.paint_publication().revision, revision);
}
