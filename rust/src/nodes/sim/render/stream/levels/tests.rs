// SPDX-License-Identifier: GPL-2.0-only

//! The level rule against the GDScript rule's golden vectors, and the travel buckets against
//! the rule evaluated directly on random view walks.

use super::*;

// The golden vectors `levels_golden.txt` holds: the GDScript renderer's `desired_level` and
// `_level_margin` at every level end and GL3 step end, both hysteresis sides, every held level.
#[test]
fn the_rule_equals_the_gdscript_rule_at_every_boundary() {
    let mut checked = 0;
    for line in include_str!("../levels_golden.txt").lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        let distance: f64 = fields[0].parse().unwrap();
        let held = held_code(fields[1].parse().unwrap());
        let code: i64 = fields[2].parse().unwrap();
        let step: i64 = fields[3].parse().unwrap();
        let margin: f64 = fields[4].parse().unwrap();
        let level = desired_level(distance, held);
        assert_eq!(
            level_code(level),
            (code, step),
            "level at {distance} m held at {held:?}"
        );
        // Godot prints the margin rounded; the levels are exact.
        let ours = level_margin(distance, held);
        assert!(
            (ours - margin).abs() <= 1e-9 * margin.max(1.0),
            "margin at {distance} m held at {held:?}: {ours} against {margin}"
        );
        checked += 1;
    }
    assert!(checked > 600, "golden vectors read: {checked}");
}

// A small deterministic generator (splitmix64), so a failing walk is reproducible.
struct Walk(u64);

impl Walk {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    // A float in [-spread, spread), on Godot's f32 grid as the renderer's camera is.
    fn offset(&mut self, spread: f64) -> f64 {
        let unit = (self.next() >> 11) as f64 / (1u64 << 53) as f64;
        f64::from(((unit * 2.0 - 1.0) * spread) as f32)
    }
}

fn assert_exact(book: &LevelBook, label: &str) {
    assert_eq!(book.mismatches(), 0, "{label}: a tile's level is not its rule's");
    let mut by_code = [0; 4];
    for tile in book.tiles() {
        by_code[level_code(book.level_of(tile).unwrap()).0 as usize] += 1;
    }
    assert_eq!(book.by_code(), by_code, "{label}: the per-level counts");
}

// After any random walk of the camera (small steps, turns, climbs, jumps) mixed with tiles
// entering, leaving and being installed at new levels, every desired tile's level equals the
// rule evaluated directly: the lazy buckets against brute force.
#[test]
fn random_view_walks_keep_every_tile_at_its_rules_level() {
    for seed in 0..6u64 {
        let mut walk = Walk(seed);
        let mut book = LevelBook::default();
        let mut camera = [walk.offset(500.0), 40.0, walk.offset(500.0)];
        book.move_camera(camera);
        let tile = |walk: &mut Walk| TileKey {
            x: walk.below(160) as i32 - 80,
            z: walk.below(160) as i32 - 80,
        };
        for _ in 0..2_000 {
            book.desire(tile(&mut walk), None);
        }
        for frame in 0..1_500 {
            match walk.below(10) {
                // A jump across the map.
                0 => camera = [walk.offset(5_000.0), walk.offset(3_000.0).abs(), walk.offset(5_000.0)],
                // A climb or a descent.
                1 => camera[1] = (camera[1] + walk.offset(200.0)).abs(),
                // A pan or an orbit step.
                _ => {
                    camera[0] = f64::from((camera[0] + walk.offset(40.0)) as f32);
                    camera[2] = f64::from((camera[2] + walk.offset(40.0)) as f32);
                }
            }
            book.move_camera(camera);
            for _ in 0..walk.below(8) {
                let tile = tile(&mut walk);
                match walk.below(3) {
                    0 => {
                        book.desire(tile, held_code(walk.below(5) as i64 - 1));
                    }
                    1 => {
                        book.undesire(tile);
                    }
                    _ => {
                        let held = book.level_of(tile).map(|level| level_code(level).0 as u8);
                        book.hold(tile, held);
                    }
                }
            }
            if frame % 50 == 0 {
                assert_exact(&book, &format!("seed {seed} frame {frame}"));
            }
        }
        assert_exact(&book, &format!("seed {seed} end"));
    }
}

// Invariant 3 of the streaming core: a still frame re-levels nothing and visits no bucket; a
// move re-levels only the tiles due.
#[test]
fn a_still_frame_does_no_work() {
    let mut book = LevelBook::default();
    let camera = [10.0, 40.0, 10.0];
    book.move_camera(camera);
    for x in -40..40 {
        for z in -40..40 {
            book.desire(TileKey { x, z }, None);
        }
    }
    let moved = [11.0, 40.0, 10.0];
    book.move_camera(moved);
    let (relevelled, visited) = book.last_move_work();
    assert!(visited <= 1, "a 1 m move passes one bucket at most: {visited}");
    assert!(
        relevelled < book.len() / 10,
        "a 1 m move re-levels only the tiles due: {relevelled} of {}",
        book.len()
    );
    for _ in 0..3 {
        assert!(book.move_camera(moved).is_empty());
        assert_eq!(book.last_move_work(), (0, 0), "a still frame does no work");
    }
}

// A tile installed at a new level moves its hysteresis: held at GL0, a tile just past GL0's end
// stays GL0 until the hysteresis ends; released, it takes GL1.
#[test]
fn holding_a_level_moves_its_hysteresis() {
    let mut book = LevelBook::default();
    // Tile (1, 0)'s centre is 96 m from x = 0: GL1 unheld, GL0 held at GL0 (64 + 32 m).
    book.move_camera([0.0, 0.0, 32.0]);
    let tile = TileKey { x: 1, z: 0 };
    assert_eq!(book.desire(tile, None), Level::Gl1);
    book.move_camera([0.5, 0.0, 32.0]);
    assert_eq!(book.hold(tile, Some(0)), Some((Level::Gl0, true)));
    assert_eq!(book.hold(tile, None), Some((Level::Gl1, true)));
    assert_eq!(book.hold(TileKey { x: 9, z: 9 }, Some(0)), None);
    assert_exact(&book, "holds");
}
