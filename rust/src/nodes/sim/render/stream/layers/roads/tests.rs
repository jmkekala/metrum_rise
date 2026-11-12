// SPDX-License-Identifier: GPL-2.0-only

//! Road chunk hydration: order (view, near ring, distance, key), the budget, busy and gone
//! answers, cancellation, the near-ring count and a still frame without pending chunks.

use super::*;
use std::cell::Cell;

// A clock that advances by each unit's scripted cost.
struct StepClock(Cell<f64>);

impl Clock for StepClock {
    fn now_us(&self) -> f64 {
        self.0.get()
    }
}

struct Exec<'a> {
    clock: &'a StepClock,
    cost_us: f64,
    log: Vec<RoadKey>,
    answer: fn(RoadKey) -> RoadInstall,
}

impl RoadExecutor for Exec<'_> {
    fn install(&mut self, key: RoadKey) -> RoadInstall {
        self.clock.0.set(self.clock.0.get() + self.cost_us);
        self.log.push(key);
        (self.answer)(key)
    }
}

fn installed(_: RoadKey) -> RoadInstall {
    RoadInstall::Installed
}

fn grid(keys: &[RoadKey], vertices: u32) -> RoadHydration {
    let mut roads = RoadHydration::default();
    roads.begin(500.0, [-5000.0, -5000.0], keys.iter().map(|&key| (key, vertices)));
    roads
}

// Chunk [10, 10] spans 0..500 m on both axes.
const EYE: [f32; 3] = [250.0, 300.0, 250.0];

fn all_keys() -> Vec<RoadKey> {
    (0..20).flat_map(|x| (0..20).map(move |z| [x, z])).collect()
}

#[test]
fn units_run_in_view_then_near_ring_then_nearest_first_by_key() {
    let mut roads = grid(&all_keys(), 10);
    let clock = StepClock(Cell::new(0.0));
    let mut exec = Exec { clock: &clock, cost_us: 1.0, log: Vec::new(), answer: installed };
    // In view: only the column x = 14 (its box starts at 2,000 m).
    let in_view = |low: [f32; 3], _high: [f32; 3]| low[0] == 2000.0;
    let mut budget = FrameBudget::new(f64::INFINITY);
    let work = roads.frame(Some(EYE), &mut budget, &clock, &in_view, &mut exec);
    assert_eq!(work.installed, 400);
    assert_eq!(roads.pending(), 0);
    let class_of = |key: RoadKey| {
        if key[0] == 14 {
            0
        } else if roads.nearest_sq(key, EYE) <= 2000.0f64.powi(2) {
            1
        } else {
            2
        }
    };
    for pair in exec.log.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let order = |key: RoadKey| (class_of(key), roads.centre_sq(key, EYE), key);
        let (oa, ob) = (order(a), order(b));
        assert!(
            oa.0 < ob.0 || oa.0 == ob.0 && (oa.1 < ob.1 || oa.1 == ob.1 && oa.2 < ob.2),
            "{a:?} before {b:?}"
        );
    }
    assert_eq!(exec.log[0], [14, 10], "the nearest chunk in view first");
}

#[test]
fn the_budget_admits_the_first_unit_and_what_fits_after_it() {
    let mut roads = grid(&all_keys(), 10_000); // 1,000 us predicted a chunk
    let clock = StepClock(Cell::new(0.0));
    let mut exec = Exec { clock: &clock, cost_us: 1000.0, log: Vec::new(), answer: installed };
    let mut budget = FrameBudget::new(ROAD_HYDRATION_BUDGET_US);
    let work = roads.frame(Some(EYE), &mut budget, &clock, &|_, _| false, &mut exec);
    // 1,000 us predicted and measured a unit: four fit 4,000 us, a fifth does not.
    assert_eq!(work.installed, 4);
    assert_eq!(budget.spent_us(), ROAD_HYDRATION_BUDGET_US);
    let mut huge = grid(&[[0, 0], [1, 1]], 1_000_000);
    let mut budget = FrameBudget::new(ROAD_HYDRATION_BUDGET_US);
    let work = huge.frame(Some(EYE), &mut budget, &clock, &|_, _| false, &mut exec);
    assert_eq!(work.installed, 1, "the frame's first unit always runs");
}

#[test]
fn a_busy_answer_stops_the_frame_and_gone_chunks_leave() {
    let mut roads = grid(&all_keys(), 10);
    let clock = StepClock(Cell::new(0.0));
    let mut exec = Exec { clock: &clock, cost_us: 1.0, log: Vec::new(), answer: |_| RoadInstall::Busy };
    let mut budget = FrameBudget::new(f64::INFINITY);
    let work = roads.frame(Some(EYE), &mut budget, &clock, &|_, _| false, &mut exec);
    assert!(work.busy);
    assert_eq!((work.installed, exec.log.len(), roads.pending()), (0, 1, 400));
    exec.answer = |key| if key[0] % 2 == 0 { RoadInstall::Gone } else { RoadInstall::Installed };
    let work = roads.frame(Some(EYE), &mut budget, &clock, &|_, _| false, &mut exec);
    assert_eq!((work.installed, work.gone, roads.pending()), (200, 200, 0));
    assert_eq!(roads.installed(), 200);
}

#[test]
fn a_cancel_ends_the_hydration() {
    let mut roads = grid(&all_keys(), 10);
    let clock = StepClock(Cell::new(0.0));
    let mut exec = Exec { clock: &clock, cost_us: 1.0, log: Vec::new(), answer: |_| RoadInstall::Cancelled };
    let mut budget = FrameBudget::new(f64::INFINITY);
    roads.frame(Some(EYE), &mut budget, &clock, &|_, _| false, &mut exec);
    assert_eq!(roads.pending(), 0);
}

#[test]
fn holes_near_count_chunks_reaching_into_the_radius() {
    let roads = grid(&all_keys(), 10);
    // The eye inside chunk [10, 10]: a 1 m radius reaches only it; a 300 m radius its eight
    // neighbours' edges too (250 m to the near sides, 250 * sqrt(2) to the corners).
    assert_eq!(roads.holes_near(EYE, 1.0), 1);
    assert_eq!(roads.holes_near(EYE, 300.0), 5);
    assert_eq!(roads.holes_near(EYE, 360.0), 9);
}

#[test]
fn a_frame_without_pending_chunks_does_nothing_and_storage_stays() {
    let mut roads = grid(&all_keys(), 10);
    let clock = StepClock(Cell::new(0.0));
    let mut exec = Exec { clock: &clock, cost_us: 1.0, log: Vec::new(), answer: installed };
    let mut budget = FrameBudget::new(f64::INFINITY);
    roads.frame(Some(EYE), &mut budget, &clock, &|_, _| false, &mut exec);
    let capacity = (roads.pending.capacity(), roads.order.capacity());
    let calls = exec.log.len();
    let work = roads.frame(Some(EYE), &mut budget, &clock, &|_, _| false, &mut exec);
    assert_eq!(work, RoadWork::default());
    assert_eq!(exec.log.len(), calls);
    assert_eq!((roads.pending.capacity(), roads.order.capacity()), capacity);
}
