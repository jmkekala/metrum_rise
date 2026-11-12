// SPDX-License-Identifier: GPL-2.0-only

//! The rings and travel buckets against brute force: annulus enumeration against every cell of a
//! box, ring deltas on random walks against the set differences of the rule evaluated over the
//! whole grid, and travel buckets against the rule evaluated for every key on every move.

use super::*;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::collections::{BTreeMap, BTreeSet};

const LATTICE: Lattice = Lattice {
    origin: [-2550.0, -2550.0],
    cell: 510.0,
};

fn every_cell(lattice: Lattice, reach: f64, point: [f64; 2]) -> impl Iterator<Item = Cell> {
    let n = (reach / lattice.cell).ceil() as i32 + 2;
    let [cx, cz] = [0, 1].map(|axis| ((point[axis] - lattice.origin[axis]) / lattice.cell) as i32);
    (cx - n..=cx + n).flat_map(move |x| (cz - n..=cz + n).map(move |z| [x, z]))
}

#[test]
fn the_annulus_visits_exactly_the_cells_between_its_circles() {
    let mut rng = StdRng::seed_from_u64(7);
    for _ in 0..400 {
        let point = [rng.gen_range(-3000.0..3000.0), rng.gen_range(-3000.0..3000.0)];
        let inner: f64 = rng.gen_range(-200.0..3000.0);
        let outer = inner.max(0.0) + rng.gen_range(0.0..1500.0);
        let lattice = Lattice {
            origin: [rng.gen_range(-600.0..0.0), rng.gen_range(-600.0..0.0)],
            cell: [127.5, 510.0, 64.0][rng.gen_range(0..3)],
        };
        let mut seen = BTreeSet::new();
        let visited = lattice.for_each_in_annulus(point, inner, outer, |cell| {
            assert!(seen.insert(cell), "a cell is visited once");
        });
        assert_eq!(visited, seen.len());
        for cell in every_cell(lattice, outer, point) {
            let d = distance(lattice.centre(cell), point);
            if d >= inner + 1e-6 && d <= outer - 1e-6 {
                assert!(seen.contains(&cell), "{cell:?} at {d} in [{inner}, {outer}]");
            }
            if seen.contains(&cell) {
                assert!(d >= inner - 1e-6 && d <= outer + 1e-6, "{cell:?} at {d}");
            }
        }
    }
}

// A three-tier rule like the vegetation's: near ring, far ring, out; one cell kind excluded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
enum Tier {
    #[default]
    Out,
    Far,
    Near,
}

const NEAR_M: f64 = 781.0;
const FAR_M: f64 = 2210.0;

fn rule(cell: Cell, point: [f64; 2]) -> Tier {
    if (cell[0] * 7 + cell[1] * 3).rem_euclid(11) == 0 {
        return Tier::Out;
    }
    let d = distance(LATTICE.centre(cell), point);
    if d <= NEAR_M {
        Tier::Near
    } else if d <= FAR_M {
        Tier::Far
    } else {
        Tier::Out
    }
}

fn brute(point: [f64; 2]) -> BTreeMap<Cell, Tier> {
    every_cell(LATTICE, FAR_M, point)
        .map(|cell| (cell, rule(cell, point)))
        .filter(|(_, tier)| *tier != Tier::Out)
        .collect()
}

#[test]
fn ring_deltas_equal_the_brute_force_set_differences_on_random_walks() {
    let bands = [
        RingBand {
            radius: NEAR_M,
            reach: 0.0,
        },
        RingBand {
            radius: FAR_M,
            reach: 0.0,
        },
    ];
    for seed in 0..12 {
        let mut rng = StdRng::seed_from_u64(seed);
        let mut ring = RingSet::<Tier>::new(LATTICE);
        let mut point = [rng.gen_range(-2000.0..2000.0), rng.gen_range(-2000.0..2000.0)];
        let mut before: BTreeMap<Cell, Tier> = BTreeMap::new();
        for step in 0..300 {
            // Mostly short moves, a jump now and then, and still frames.
            let scale = match rng.gen_range(0..20) {
                0 => 4000.0,
                1 | 2 => 0.0,
                _ => rng.gen_range(1.0..160.0),
            };
            let angle: f64 = rng.gen_range(0.0..std::f64::consts::TAU);
            point = [point[0] + scale * angle.cos(), point[1] + scale * angle.sin()];
            let mut events = BTreeMap::new();
            ring.reset_visited();
            ring.update(
                point,
                &bands,
                false,
                |cell| rule(cell, point),
                |cell, old, new| {
                    assert!(events.insert(cell, (old, new)).is_none());
                },
            );
            let after = brute(point);
            let mut expected = BTreeMap::new();
            for cell in before.keys().chain(after.keys()) {
                let old = before.get(cell).copied().unwrap_or_default();
                let new = after.get(cell).copied().unwrap_or_default();
                if old != new {
                    expected.insert(*cell, (old, new));
                }
            }
            assert_eq!(events, expected, "seed {seed} step {step}");
            let members: BTreeMap<Cell, Tier> = ring.members().collect();
            assert_eq!(members, after);
            if scale == 0.0 && step > 0 {
                assert_eq!(ring.visited(), 0, "a still point visits no cell");
            }
            before = after;
        }
    }
}

#[test]
fn a_short_move_visits_the_ring_boundaries_only() {
    let bands = [RingBand {
        radius: FAR_M,
        reach: 0.0,
    }];
    let mut ring = RingSet::<Tier>::new(LATTICE);
    ring.update([0.0, 0.0], &bands, false, |c| rule(c, [0.0, 0.0]), |_, _, _| {});
    let full = ring.visited();
    ring.reset_visited();
    ring.update([60.0, 0.0], &bands, false, |c| rule(c, [60.0, 0.0]), |_, _, _| {});
    // The 2.2 km ring on a 510 m grid: its perimeter is about 27 cells; a 60 m move visits a
    // band two cells wide at most.
    assert!(ring.visited() < full, "{} of {full}", ring.visited());
    assert!(ring.visited() <= 64, "{}", ring.visited());
}

#[test]
fn travel_buckets_hand_back_every_key_before_its_rule_can_change() {
    // Keys at fixed points; the "rule" is which side of 300 m each is from the view. A key is
    // bucketed by its margin; after every move, a key not handed back must keep its side.
    let mut rng = StdRng::seed_from_u64(3);
    let keys: Vec<[f64; 2]> = (0..300)
        .map(|_| [rng.gen_range(-1500.0..1500.0), rng.gen_range(-1500.0..1500.0)])
        .collect();
    let mut buckets = TravelBuckets::<usize>::new(8.0);
    let mut view = [0.0, 0.0];
    let side = |key: [f64; 2], view: [f64; 2]| distance(key, view) <= 300.0;
    let margin = |key: [f64; 2], view: [f64; 2]| (distance(key, view) - 300.0).abs();
    let mut sides: Vec<bool> = keys.iter().map(|&k| side(k, view)).collect();
    for (at, &key) in keys.iter().enumerate() {
        buckets.put(at, margin(key, view));
    }
    let mut due = Vec::new();
    for _ in 0..2000 {
        let step = [rng.gen_range(-20.0..20.0), rng.gen_range(-20.0..20.0)];
        let next = [view[0] + step[0], view[1] + step[1]];
        due.clear();
        buckets.advance(distance(view, next), &mut due);
        view = next;
        let mut handed = vec![false; keys.len()];
        for &at in &due {
            handed[at] = true;
            sides[at] = side(keys[at], view);
            buckets.put(at, margin(keys[at], view));
        }
        for (at, &key) in keys.iter().enumerate() {
            assert_eq!(sides[at], side(key, view), "key {at} changed side unseen");
            assert!(handed[at] || buckets.contains(at));
        }
    }
    assert_eq!(buckets.len(), keys.len());
    due.clear();
    assert_eq!(buckets.advance(0.0, &mut due), 0, "a still frame visits no bucket");
    assert!(due.is_empty());
    assert!(buckets.remove(5) && !buckets.remove(5));
}
