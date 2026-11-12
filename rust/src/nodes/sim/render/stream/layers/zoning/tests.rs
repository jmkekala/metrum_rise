// SPDX-License-Identifier: GPL-2.0-only

//! The zoning ring against a brute-force pass over every chunk of the world: random walks with
//! steps, still frames and jumps; a still frame visits nothing; nearest-first order.

use super::*;
use std::collections::BTreeSet;

const SPAN: f64 = 512.0;
const WORLD: [f64; 2] = [12_000.0, 9_000.0];

// The rule over every chunk of the world, with the same hysteresis.
fn brute(resident: &mut BTreeSet<Cell>, eye: [f64; 2]) {
    let index = |metres: f64| (metres / SPAN).floor() as i32;
    let (first, last) = (
        [index(-WORLD[0] * 0.5), index(-WORLD[1] * 0.5)],
        [index(WORLD[0] * 0.5), index(WORLD[1] * 0.5)],
    );
    for x in first[0] - 2..=last[0] + 2 {
        for z in first[1] - 2..=last[1] + 2 {
            match band([x, z], eye, SPAN, first, last) {
                Band::Inside => {
                    resident.insert([x, z]);
                }
                Band::Out => {
                    resident.remove(&[x, z]);
                }
                Band::Hysteresis => {}
            }
        }
    }
}

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[test]
fn the_ring_equals_the_rule_over_every_chunk_on_random_walks() {
    for seed in 1..=4u64 {
        let mut rng = Lcg(seed);
        let mut ring = ZoningRing::new(SPAN, WORLD);
        let mut expected = BTreeSet::new();
        let mut eye = [0.0, 0.0];
        for step in 0..600 {
            let roll = rng.next();
            if roll < 0.05 {
                // A jump anywhere, edges of the world included.
                eye = [(rng.next() - 0.5) * 14_000.0, (rng.next() - 0.5) * 11_000.0];
            } else if roll > 0.2 {
                eye[0] += (rng.next() - 0.5) * 120.0;
                eye[1] += (rng.next() - 0.5) * 120.0;
            }
            let before = ring.revision();
            let changed = ring.update(eye);
            let old = expected.clone();
            brute(&mut expected, eye);
            let got: BTreeSet<Cell> = ring.resident().collect();
            assert_eq!(got, expected, "seed {seed} step {step} eye {eye:?}");
            assert_eq!(changed, old != expected);
            assert_eq!(ring.revision() != before, changed);
        }
    }
}

#[test]
fn a_still_eye_visits_nothing_and_the_list_is_nearest_first() {
    let mut ring = ZoningRing::new(SPAN, WORLD);
    ring.update([100.0, -40.0]);
    ring.visited();
    assert!(!ring.update([100.0, -40.0]));
    assert_eq!(ring.visited(), 0, "a still frame visits no chunk");
    let list: Vec<Cell> = ring.resident().collect();
    assert_eq!(list[0], [0, -1], "the eye's own chunk first");
    let distance = |cell: Cell| {
        let centre = |at: i32, eye: f64| (f64::from(at) + 0.5) * SPAN - eye;
        centre(cell[0], 100.0).powi(2) + centre(cell[1], -40.0).powi(2)
    };
    for pair in list.windows(2) {
        assert!(distance(pair[0]) <= distance(pair[1]));
    }
    // Every chunk any point of which lies within 2 km: the corner rays' reach is covered.
    assert!(list.contains(&[-4, 2]) && list.contains(&[3, -3]) && !list.contains(&[4, -4]));
}
