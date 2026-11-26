// SPDX-License-Identifier: GPL-3.0-or-later

//! The kept water meshes' book against a recorder of `water.gd`: the kept set under several arrival
//! orders and paces, the nearest-first admission, the farthest-first drops, a still camera, and the
//! events that end a request.

use super::*;
use crate::nodes::sim::render::stream::schedule::STREAM_BUDGET_US;

// 20 x 20 patches of 64 cells of 8 m (the shipped layout's patch).
const GRID: PatchGrid = PatchGrid {
    cols: 20,
    rows: 20,
    interval: 64,
    cell: 8.0,
    world: [10240.0, 10240.0],
};
const EYE: [f32; 3] = [130.0, 900.0, -70.0];

struct FixedClock;
impl Clock for FixedClock {
    fn now_us(&self) -> f64 {
        0.0
    }
}

// `water.gd`'s side: requests in flight in the order asked, the drops it did, and every call.
#[derive(Default)]
struct Water {
    in_flight: Vec<PatchKey>,
    held: std::collections::BTreeMap<PatchKey, u64>,
    drops: Vec<PatchKey>,
    calls: usize,
    refuse: Vec<PatchKey>,
}

impl KeptExecutor for Water {
    fn request(&mut self, keys: &[i32], refused: &mut Vec<i32>) {
        self.calls += 1;
        for &[x, z] in keys.as_chunks::<2>().0 {
            if self.refuse.contains(&[x, z]) {
                refused.extend_from_slice(&[x, z, 2]);
            } else {
                self.in_flight.push([x, z]);
            }
        }
    }
    fn drop_mesh(&mut self, key: PatchKey) -> Option<u64> {
        self.calls += 1;
        self.drops.push(key);
        self.held.remove(&key)
    }
}

// A deterministic generator (an LCG): the tests' orders and paces.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self, below: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % below.max(1)
    }
}

// A mesh's bytes, a deterministic function of its key at most `W`.
fn mesh_bytes(key: PatchKey) -> u64 {
    let w = kept_mesh_bytes_max(&GRID);
    w / 4 + (key[0] as u64 * 7_919 + key[1] as u64 * 104_729) % (w - w / 4 + 1)
}

// Every patch built, hidden and wet without a mesh (a fill's patches beyond the view), but for
// `dry`.
fn book(dry: &[PatchKey]) -> KeptMeshes {
    let mut book = KeptMeshes::default();
    book.reset(Some(GRID));
    for z in 0..GRID.rows {
        for x in 0..GRID.cols {
            let state = if dry.contains(&[x, z]) {
                WaterState::Dry
            } else {
                WaterState::Empty
            };
            book.answer([x, z], state);
        }
    }
    book
}

struct Run {
    kept: Vec<PatchKey>,
    held: u64,
    frames: usize,
}

// Frames of the book with `counted` bytes and `budget`, the meshes arriving `pace(frame)` a frame
// in an order `pick` chooses among those in flight, until nothing is in flight or asked and the
// book asked nothing for 10 frames.
fn run(
    book: &mut KeptMeshes,
    water: &mut Water,
    budget: Option<u64>,
    counted: u64,
    seed: u64,
    max_pace: usize,
) -> Run {
    let mut rng = Lcg(seed);
    let mut held = water.held.values().sum::<u64>();
    let mut quiet = 0;
    let mut frames = 0;
    while quiet < 10 {
        frames += 1;
        assert!(frames < 10_000, "the fill must end");
        let mut ledger = FrameBudget::new(STREAM_BUDGET_US);
        let frame = KeptFrame {
            budget,
            counted,
            kept_bytes: held,
            may_ask: true,
        };
        let work = book.frame(Some(EYE), frame, &mut ledger, &FixedClock, water);
        held -= work.dropped_bytes;
        let pace = rng.next(max_pace + 1);
        for _ in 0..pace.min(water.in_flight.len()) {
            let key = water.in_flight.remove(rng.next(water.in_flight.len()));
            let bytes = mesh_bytes(key);
            water.held.insert(key, bytes);
            held += bytes;
            book.report(key, WaterState::Mesh);
        }
        quiet = if work.asked == 0 && water.in_flight.is_empty() {
            quiet + 1
        } else {
            0
        };
        assert_eq!(held, water.held.values().sum::<u64>());
        assert!(book.asked_count() <= ASKED_MAX);
    }
    Run {
        kept: water.held.keys().copied().collect(),
        held,
        frames,
    }
}

// The patches nearest the eye first, then by index (the fill's order).
fn nearest_first() -> Vec<PatchKey> {
    let mut keys: Vec<PatchKey> = (0..GRID.rows)
        .flat_map(|z| (0..GRID.cols).map(move |x| [x, z]))
        .collect();
    keys.sort_by(|a, b| {
        ground_distance_sq(GRID, Some(EYE), GRID.index(*a))
            .total_cmp(&ground_distance_sq(GRID, Some(EYE), GRID.index(*b)))
            .then(GRID.index(*a).cmp(&GRID.index(*b)))
    });
    keys
}

#[test]
fn the_largest_kept_mesh_is_every_cell_drawn_on_its_own() {
    // 64 cells at step 8: 8 x 8 mesh cells, 6 vertices each of 32 bytes.
    assert_eq!(kept_mesh_bytes_max(&GRID), 12_288);
}

#[test]
fn the_kept_set_does_not_depend_on_the_arrival_order_or_pace() {
    let w = kept_mesh_bytes_max(&GRID);
    let counted = 40_000_000;
    let budget = counted + 90 * w + w / 3;
    let mut sets = Vec::new();
    for (seed, pace) in [(1, 1), (2, 3), (3, 16), (4, 0), (5, 40), (6, 2)] {
        let mut book = book(&[]);
        let mut water = Water::default();
        let run = run(
            &mut book,
            &mut water,
            Some(budget),
            counted,
            seed,
            pace.max(1),
        );
        assert!(counted + run.held <= budget);
        sets.push(run.kept);
    }
    assert!(
        sets.windows(2).all(|pair| pair[0] == pair[1]),
        "one kept set"
    );
    // The set is the nearest-first prefix whose real bytes leave less than W of room.
    let order = nearest_first();
    let mut sum = 0;
    let mut prefix = Vec::new();
    for key in order {
        if counted + sum + w > budget {
            break;
        }
        sum += mesh_bytes(key);
        prefix.push(key);
    }
    prefix.sort();
    assert_eq!(sets[0], prefix);
    assert!(
        prefix.len() > 90,
        "smaller meshes than W leave room for more"
    );
}

#[test]
fn at_a_target_with_room_for_k_meshes_exactly_the_nearest_k_are_kept() {
    let w = kept_mesh_bytes_max(&GRID);
    let counted = 7_000_000;
    let dry = [[10, 10], [3, 4]];
    for k in [0, 1, 17, 64] {
        let mut book = book(&dry);
        let mut water = Water::default();
        // Every mesh W bytes: room for exactly k.
        let mut rng = Lcg(k as u64);
        let budget = counted + k as u64 * w;
        let mut held = 0;
        for _ in 0..400 {
            let mut ledger = FrameBudget::new(STREAM_BUDGET_US);
            let frame = KeptFrame {
                budget: Some(budget),
                counted,
                kept_bytes: held,
                may_ask: true,
            };
            book.frame(Some(EYE), frame, &mut ledger, &FixedClock, &mut water);
            let pace = rng.next(5);
            for _ in 0..pace.min(water.in_flight.len()) {
                let key = water.in_flight.remove(0);
                water.held.insert(key, w);
                held += w;
                book.report(key, WaterState::Mesh);
            }
        }
        let expected: Vec<PatchKey> = nearest_first()
            .into_iter()
            .filter(|key| !dry.contains(key))
            .take(k)
            .collect();
        let mut expected = expected;
        expected.sort();
        assert_eq!(
            water.held.keys().copied().collect::<Vec<_>>(),
            expected,
            "k = {k}"
        );
        assert_eq!(book.kept_count(), k);
    }
}

#[test]
fn without_a_target_every_wanting_patch_is_asked_for_a_few_at_a_time() {
    let mut book = book(&[[0, 0]]);
    let mut water = Water::default();
    let mut ledger = FrameBudget::new(STREAM_BUDGET_US);
    let frame = KeptFrame {
        budget: None,
        counted: 0,
        kept_bytes: 0,
        may_ask: true,
    };
    let work = book.frame(Some(EYE), frame, &mut ledger, &FixedClock, &mut water);
    assert_eq!(work.asked, ASK_PER_FRAME);
    book.frame(Some(EYE), frame, &mut ledger, &FixedClock, &mut water);
    let work = book.frame(Some(EYE), frame, &mut ledger, &FixedClock, &mut water);
    assert_eq!(work.asked, 0, "at most {ASKED_MAX} out at once");
    assert_eq!(water.in_flight, nearest_first()[..ASKED_MAX].to_vec());
    let run = run(&mut book, &mut water, None, 0, 9, 7);
    assert_eq!(run.kept.len(), 399, "every wet patch keeps a mesh");
    assert_eq!(book.wanting_count(), 0);
}

#[test]
fn over_the_target_the_farthest_kept_meshes_go_first_and_a_still_camera_then_does_nothing() {
    let w = kept_mesh_bytes_max(&GRID);
    let counted = 5_000_000;
    let budget = counted + 50 * w;
    let mut book = book(&[]);
    let mut water = Water::default();
    let first = run(&mut book, &mut water, Some(budget), counted, 11, 4);
    let kept_before = first.kept.clone();
    // The view's own bytes grew by 2.5 W: kept meshes go, farthest first, until the sum fits;
    // the book has no unit that touches a place.
    let grown = counted + 5 * w / 2;
    water.drops.clear();
    let mut ledger = FrameBudget::new(STREAM_BUDGET_US);
    let frame = KeptFrame {
        budget: Some(budget),
        counted: grown,
        kept_bytes: first.held,
        may_ask: true,
    };
    let work = book.frame(Some(EYE), frame, &mut ledger, &FixedClock, &mut water);
    assert!(grown + first.held - work.dropped_bytes <= budget);
    let mut farthest: Vec<PatchKey> = nearest_first()
        .into_iter()
        .filter(|key| kept_before.contains(key))
        .collect();
    farthest.reverse();
    assert_eq!(
        water.drops,
        farthest[..work.dropped].to_vec(),
        "farthest first"
    );
    assert!(work.dropped >= 2 && work.asked == 0);
    // A still camera after that: nothing asked, applied or dropped for 120 frames (the band of W
    // between "drop above B" and "ask below B - W").
    let held = first.held - work.dropped_bytes;
    let calls = water.calls;
    for _ in 0..120 {
        let mut ledger = FrameBudget::new(STREAM_BUDGET_US);
        let frame = KeptFrame {
            budget: Some(budget),
            counted: grown,
            kept_bytes: held,
            may_ask: true,
        };
        let work = book.frame(Some(EYE), frame, &mut ledger, &FixedClock, &mut water);
        assert_eq!(work, KeptWork::default());
    }
    assert_eq!(water.calls, calls, "no call to the script");
}

#[test]
fn a_show_a_report_a_release_and_a_refusal_end_a_request() {
    let mut book = book(&[]);
    let mut water = Water::default();
    let frame = KeptFrame {
        budget: None,
        counted: 0,
        kept_bytes: 0,
        may_ask: true,
    };
    let mut ledger = FrameBudget::new(STREAM_BUDGET_US);
    book.frame(Some(EYE), frame, &mut ledger, &FixedClock, &mut water);
    let [a, b, c, d] = [
        water.in_flight[0],
        water.in_flight[1],
        water.in_flight[2],
        water.in_flight[3],
    ];
    assert_eq!(book.asked_count(), ASK_PER_FRAME);
    // Shown while asked: the request becomes the show's own; hidden again without a mesh it wants
    // one again.
    book.set_shown(a, true);
    assert_eq!(book.asked_count(), ASK_PER_FRAME - 1);
    book.set_shown(a, false);
    // A hidden patch's stale mesh dropped (its depth changed, still wet): asked again later.
    book.report(b, WaterState::Empty);
    // Released while asked (the script answers busy while its job is in flight; a queued one goes).
    book.forget(c);
    // Dried while asked.
    book.report(d, WaterState::Dry);
    assert_eq!(book.asked_count(), ASK_PER_FRAME - 4);
    let wanting = book.wanting_count();
    assert_eq!(
        wanting,
        400 - ASK_PER_FRAME + 2,
        "a and b want a mesh again; c and d do not"
    );
    // A mesh that answers a request is kept; shown, it is an ordinary mesh.
    let e = water.in_flight[4];
    book.report(e, WaterState::Mesh);
    assert_eq!(book.kept_count(), 1);
    book.set_shown(e, true);
    assert_eq!(book.kept_count(), 0);
    book.set_shown(e, false);
    assert_eq!(
        book.wanting_count(),
        wanting,
        "a shown mesh kept after a hide is no want"
    );
    // A refused request bars the patch until its next news.
    let mut book = self::book(&[]);
    let mut water = Water {
        refuse: nearest_first()[..2].to_vec(),
        ..Water::default()
    };
    book.frame(Some(EYE), frame, &mut ledger, &FixedClock, &mut water);
    assert_eq!(book.totals()[2], 2);
    assert_eq!(book.asked_count(), ASK_PER_FRAME - 2);
    assert_eq!(book.wanting_count(), 400 - ASK_PER_FRAME);
    book.report(nearest_first()[0], WaterState::Empty);
    assert_eq!(book.wanting_count(), 400 - ASK_PER_FRAME + 1);
}

#[test]
fn nothing_is_asked_while_the_view_waits() {
    let mut book = book(&[]);
    let mut water = Water::default();
    let mut ledger = FrameBudget::new(STREAM_BUDGET_US);
    let frame = KeptFrame {
        budget: None,
        counted: 0,
        kept_bytes: 0,
        may_ask: false,
    };
    for _ in 0..10 {
        assert_eq!(
            book.frame(Some(EYE), frame, &mut ledger, &FixedClock, &mut water),
            KeptWork::default()
        );
    }
    assert_eq!(water.calls, 0);
}
