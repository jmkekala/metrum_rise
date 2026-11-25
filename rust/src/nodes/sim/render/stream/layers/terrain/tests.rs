// SPDX-License-Identifier: GPL-3.0-or-later

//! The terrain and water patch layer against brute-force replicas of the scripts' rules, on
//! scripted and random camera walks.

use super::*;
use crate::nodes::sim::render::stream::schedule::UnitSource;
use std::collections::{BTreeMap, BTreeSet};

// Kuopio's layout: 36 x 36 patches of 51 cells of 10 m, the last one short.
const GRID: PatchGrid = PatchGrid {
    cols: 36,
    rows: 36,
    interval: 51,
    cell: 10.0,
    world: [18000.0, 18000.0],
};

const COSTS: UnitCosts = UnitCosts {
    activate_us: 20.0,
    build_us: 200.0,
    lod_us: 15.0,
    free_us: 10.0,
    release_us: 10.0,
};

struct FixedClock;
impl Clock for FixedClock {
    fn now_us(&self) -> f64 {
        0.0
    }
}

// The scripts' side: a payload arrives the frame after its request; activations and level
// changes are recorded.
#[derive(Default)]
struct Scripts {
    requested: Vec<PatchKey>,
    resident: BTreeMap<PatchKey, u8>,
    built: BTreeSet<PatchKey>,
    activations: Vec<(PatchKey, u8)>,
    lod_changes: Vec<(PatchKey, u8)>,
    blocked: bool,
    // `TERRAIN-41` B1: nodes built (activations of unbuilt patches and prewarm builds), patches
    // released, and every call in order.
    builds: usize,
    released: Vec<PatchKey>,
    calls: Vec<(u8, PatchKey)>,
}

impl PatchExecutor for Scripts {
    fn request(&mut self, keys: &[i32], _prewarm: bool, _news: &mut Vec<i32>) {
        for pair in keys.chunks(2) {
            self.requested.push([pair[0], pair[1]]);
        }
    }
    fn activate(&mut self, key: PatchKey, step: u8) -> (Activation, i64) {
        if self.blocked {
            return (Activation::Blocked, -1);
        }
        assert!(self.resident.insert(key, step).is_none(), "{key:?} twice");
        self.builds += usize::from(self.built.insert(key));
        self.activations.push((key, step));
        self.calls.push((b'a', key));
        (Activation::Done, 0)
    }
    fn deactivate(&mut self, key: PatchKey) {
        assert!(self.resident.remove(&key).is_some());
        self.calls.push((b'h', key));
    }
    fn build(&mut self, key: PatchKey, _step: u8) -> Option<i64> {
        self.calls.push((b'b', key));
        let built = self.built.insert(key);
        self.builds += usize::from(built);
        built.then_some(0)
    }
    fn release(&mut self, key: PatchKey) -> Release {
        assert!(!self.resident.contains_key(&key), "{key:?} released while drawn");
        assert!(self.built.remove(&key), "{key:?} released unbuilt");
        self.released.push(key);
        self.calls.push((b'r', key));
        Release::Done
    }
    fn set_lod(&mut self, key: PatchKey, step: u8) {
        *self.resident.get_mut(&key).unwrap() = step;
        self.lod_changes.push((key, step));
    }
}

fn deliver(layer: &mut PatchStream, scripts: &mut Scripts) {
    for key in scripts.requested.drain(..) {
        layer.payload(key, Payload::Ready);
    }
}

fn frame(layer: &mut PatchStream, scripts: &mut Scripts, eye: [f32; 3], delta: f64) -> PatchWork {
    layer.set_square(GRID.target(eye, 16000.0, false));
    let mut budget = FrameBudget::new(f64::INFINITY);
    let work = layer.frame(Some(eye), delta, &mut budget, &FixedClock, false, &|_| false, scripts);
    deliver(layer, scripts);
    work
}

fn square_keys(square: [i32; 4]) -> BTreeSet<PatchKey> {
    (square[1]..=square[3])
        .flat_map(|z| (square[0]..=square[2]).map(move |x| [x, z]))
        .collect()
}

// A small deterministic generator (xorshift).
struct Walk(u64);
impl Walk {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[test]
fn the_square_is_the_scripts_with_hysteresis_and_clamped() {
    // 800 m up, far 3 km: the half-side is min(3 km, max(8 km, 3.2 km)); padded by a patch:
    // (9000 -/+ 3510) / 510 floors to 10 and 24, then two patches of hysteresis.
    assert_eq!(GRID.target([0.0, 800.0, 0.0], 3000.0, false), [8, 8, 26, 26]);
    // High up the height sets the side; the far plane caps it; the world clamps it.
    assert_eq!(GRID.target([0.0, 5000.0, 0.0], 30000.0, false), [0, 0, 35, 35]);
    assert_eq!(GRID.target([9000.0, 800.0, 9000.0], 16000.0, true), [0, 0, 35, 35]);
}

#[test]
fn the_centre_is_the_nodes_and_the_last_patch_is_short() {
    assert_eq!(GRID.centre([0, 0]), [-8745.0, -8745.0]);
    // 1,800 cells: the last patch holds 15 of its 51.
    assert_eq!(GRID.centre([35, 2])[0], 35.0 * 510.0 - 9000.0 + 75.0);
}

#[test]
fn residency_follows_the_square_by_its_strips_and_settles_on_it() {
    let mut layer = PatchStream::new(TERRAIN_LIMITS, COSTS);
    layer.reset(Some(GRID));
    let mut scripts = Scripts::default();
    let mut walk = Walk(0x9e37_79b9_7f4a_7c15);
    let mut eye = [0.0_f32, 800.0, 0.0];
    for step in 0..400 {
        let jump = step % 97 == 96;
        let reach = if jump { 6000.0 } else { 300.0 };
        eye[0] = (eye[0] + ((walk.next() - 0.5) * reach) as f32).clamp(-9000.0, 9000.0);
        eye[2] = (eye[2] + ((walk.next() - 0.5) * reach) as f32).clamp(-9000.0, 9000.0);
        eye[1] = (eye[1] + ((walk.next() - 0.5) * 200.0) as f32).clamp(60.0, 4000.0);
        frame(&mut layer, &mut scripts, eye, 1.0 / 60.0);
        // The wanted set is always the square.
        let square = GRID.target(eye, 16000.0, false);
        let wanted: BTreeSet<_> = (0..GRID.len())
            .filter(|&index| layer.flags[index] & WANTED != 0)
            .map(|index| GRID.key(index))
            .collect();
        assert_eq!(wanted, square_keys(square), "frame {step}");
        let resident: BTreeSet<_> = layer.resident_keys().collect();
        assert_eq!(resident, scripts.resident.keys().copied().collect());
    }
    // Held still, it settles on the square with nothing pending.
    for _ in 0..8 {
        frame(&mut layer, &mut scripts, eye, 1.0 / 60.0);
    }
    assert!(!layer.has_pending_work());
    let resident: BTreeSet<_> = layer.resident_keys().collect();
    assert_eq!(resident, square_keys(GRID.target(eye, 16000.0, false)));
}

#[test]
fn the_followed_entries_are_the_scripts_resident_patches_and_steps() {
    // `RENDER-32-K` part 6: a follower replaying the layer's news holds exactly the patches the
    // scripts hold resident, at their steps, on a walk with jumps and level changes.
    let mut layer = PatchStream::new(TERRAIN_LIMITS, COSTS);
    layer.reset(Some(GRID));
    layer.set_followed(true);
    let mut scripts = Scripts::default();
    let mut followed: BTreeMap<PatchKey, u8> = BTreeMap::new();
    let mut walk = Walk(0x6a09_e667_f3bc_c908);
    let mut eye = [0.0_f32, 800.0, 0.0];
    let mut steps = 0;
    for step in 0..400 {
        let reach = if step % 97 == 96 { 6000.0 } else { 300.0 };
        eye[0] = (eye[0] + ((walk.next() - 0.5) * reach) as f32).clamp(-9000.0, 9000.0);
        eye[2] = (eye[2] + ((walk.next() - 0.5) * reach) as f32).clamp(-9000.0, 9000.0);
        eye[1] = (eye[1] + ((walk.next() - 0.5) * 400.0) as f32).clamp(60.0, 4000.0);
        frame(&mut layer, &mut scripts, eye, 0.1);
        for (key, follow) in layer.take_follow() {
            match follow {
                Follow::Resident { step, generation } => {
                    assert_eq!(generation, 0);
                    assert!(followed.insert(key, step).is_none(), "{key:?} resident twice");
                }
                Follow::Step(step) => {
                    *followed.get_mut(&key).expect("a step change of a resident patch") = step;
                    steps += 1;
                }
                Follow::Left => assert!(followed.remove(&key).is_some(), "{key:?} left unheld"),
            }
        }
        assert_eq!(followed, scripts.resident, "frame {step}");
    }
    assert!(steps > 0, "the walk changed no step");
    // A layer nobody follows records nothing.
    layer.set_followed(false);
    frame(&mut layer, &mut scripts, [5000.0, 800.0, 5000.0], 0.1);
    assert_eq!(layer.take_follow().count(), 0);
}

#[test]
fn the_rect_is_the_payloads_and_the_last_patch_is_short() {
    assert_eq!(GRID.rect([0, 0]), [-9000.0, -9000.0, 510.0, 510.0]);
    assert_eq!(GRID.rect([35, 2]), [35.0 * 510.0 - 9000.0, 2.0 * 510.0 - 9000.0, 150.0, 510.0]);
}

#[test]
fn level_checks_queue_exactly_the_patches_the_scripts_sweep_finds() {
    let mut layer = PatchStream::new(TERRAIN_LIMITS, COSTS);
    layer.reset(Some(GRID));
    let mut scripts = Scripts::default();
    let mut walk = Walk(0x2545_f491_4f6c_dd1d);
    let mut eye = [0.0_f32, 800.0, 0.0];
    let mut checks = 0;
    for step in 0..600 {
        eye[0] = (eye[0] + ((walk.next() - 0.5) * 250.0) as f32).clamp(-9000.0, 9000.0);
        eye[2] = (eye[2] + ((walk.next() - 0.5) * 250.0) as f32).clamp(-9000.0, 9000.0);
        if step % 50 == 25 {
            eye[1] = (eye[1] + ((walk.next() - 0.5) * 2000.0) as f32).clamp(60.0, 3000.0);
        }
        layer.set_square(GRID.target(eye, 16000.0, false));
        let installed: BTreeMap<_, _> = scripts.resident.clone();
        let mut budget = FrameBudget::new(f64::INFINITY);
        // An unbounded budget: a check's queue runs in its own frame, at the check's eye.
        let work = layer.frame(Some(eye), 0.1, &mut budget, &FixedClock, false, &|_| false, &mut scripts);
        deliver(&mut layer, &mut scripts);
        if work.lod_checks == 0 {
            continue;
        }
        checks += 1;
        // The scripts' sweep: every resident patch whose installed step differs from the rule at
        // the check's eye; each then takes the eye's step (here the same eye).
        let expected: BTreeMap<_, _> = installed
            .iter()
            .filter(|&(key, step)| lod_step(lod_distance(eye, GRID.centre(*key))) != *step)
            .map(|(key, _)| (*key, lod_step(lod_distance(eye, GRID.centre(*key)))))
            .collect();
        let changed: BTreeMap<_, _> = scripts
            .lod_changes
            .drain(..)
            .filter(|(key, _)| installed.contains_key(key))
            .collect();
        assert_eq!(changed, expected, "check at frame {step}");
    }
    assert!(checks > 50, "{checks} checks");
    // Every resident step is the rule's at the last check's eye, after the queue ran.
    for (key, step) in &scripts.resident {
        assert_eq!(layer.installed_step(*key), Some(*step));
    }
}

#[test]
fn a_still_view_visits_no_ring_cell_and_allocates_nothing() {
    let mut layer = PatchStream::new(TERRAIN_LIMITS, COSTS);
    layer.reset(Some(GRID));
    let mut scripts = Scripts::default();
    let eye = [1200.0, 800.0, -300.0];
    // 32 requests a frame: the 1,296 patches arrive in about 41 frames.
    for _ in 0..80 {
        frame(&mut layer, &mut scripts, eye, 0.25);
    }
    let capacities = |layer: &PatchStream| {
        [
            layer.holes.capacity(),
            layer.leaving.capacity(),
            layer.mismatched.capacity(),
            layer.queue.capacity(),
            layer.prewarm.capacity(),
            layer.order.capacity(),
            layer.changes.capacity(),
            layer.requests.capacity(),
        ]
    };
    let before = capacities(&layer);
    for _ in 0..30 {
        let work = frame(&mut layer, &mut scripts, eye, 0.25);
        assert_eq!(work.ring_cells, 0);
        assert_eq!(work.lod_checks, 0);
        assert_eq!(work.activations + work.lod_changes + work.frees, 0);
    }
    assert_eq!(capacities(&layer), before);
}

#[test]
fn holes_in_view_then_the_near_ring_come_first_and_the_budget_holds() {
    let mut layer = PatchStream::new(TERRAIN_LIMITS, COSTS);
    layer.reset(Some(GRID));
    let mut scripts = Scripts::default();
    let eye = [0.0, 800.0, 0.0];
    layer.set_square(GRID.target(eye, 16000.0, false));
    let mut budget = FrameBudget::new(1000.0);
    // First frame: requests only (no payload yet).
    layer.frame(Some(eye), 0.016, &mut budget, &FixedClock, false, &|_| false, &mut scripts);
    deliver(&mut layer, &mut scripts);
    let in_view = |key: PatchKey| key[0] > 20 && key[1] > 20;
    let mut first = Vec::new();
    for _ in 0..600 {
        budget.begin_frame();
        layer.frame(Some(eye), 0.016, &mut budget, &FixedClock, false, &in_view, &mut scripts);
        deliver(&mut layer, &mut scripts);
        assert!(budget.units <= 1 || budget.predicted_us <= 1000.0);
        first.extend(scripts.activations.drain(..).map(|(key, _)| key));
    }
    // Requests go out nearest first, 32 a frame, so the first activations are near patches; in
    // view before the rest among those ready together.
    let near = |key: &PatchKey| {
        let centre = GRID.centre(*key);
        f64::from(centre[0]).hypot(f64::from(centre[1])) <= NEAR_RING_M
    };
    let first_far = first.iter().position(|key| !near(key) && !in_view(*key)).unwrap();
    assert!(first[..first_far].iter().all(|key| near(key) || in_view(*key)));
    assert!(!layer.has_pending_work());
}

#[test]
fn a_blocked_activation_stops_the_frames_activations() {
    let mut layer = PatchStream::new(TERRAIN_LIMITS, COSTS);
    layer.reset(Some(GRID));
    let mut scripts = Scripts::default();
    let eye = [0.0, 800.0, 0.0];
    frame(&mut layer, &mut scripts, eye, 0.016);
    scripts.blocked = true;
    let work = frame(&mut layer, &mut scripts, eye, 0.016);
    assert_eq!(work.activations, 0);
    scripts.blocked = false;
    let work = frame(&mut layer, &mut scripts, eye, 0.016);
    assert!(work.activations > 0);
}

#[test]
fn water_follows_terrain_residency_and_prewarm_waits_for_the_rest() {
    let mut terrain = PatchStream::new(TERRAIN_LIMITS, COSTS);
    let mut water = PatchStream::new(WATER_LIMITS, COSTS);
    terrain.reset(Some(GRID));
    water.reset(Some(GRID));
    let (mut land, mut lake) = (Scripts::default(), Scripts::default());
    let mut eye = [-4000.0_f32, 800.0, 2000.0];
    for step in 0..120 {
        if step == 60 {
            eye = [3000.0, 800.0, -2500.0];
        }
        frame(&mut terrain, &mut land, eye, 0.016);
        for (key, resident) in terrain.take_changes() {
            water.set_wanted(key, resident);
        }
        water.set_halo(terrain.square().map(|square| GRID.widen(square, WATER_LIMITS.halo)));
        let mut budget = FrameBudget::new(f64::INFINITY);
        water.frame(Some(eye), 0.016, &mut budget, &FixedClock, false, &|_| false, &mut lake);
        deliver(&mut water, &mut lake);
        let _ = water.take_changes().count();
    }
    let land_resident: BTreeSet<_> = terrain.resident_keys().collect();
    let lake_resident: BTreeSet<_> = water.resident_keys().collect();
    assert_eq!(land_resident, lake_resident);
    // Settled: the water's halo (a patch beyond the terrain's square) is built hidden; the
    // terrain builds nothing beyond its square, as its scripts did.
    let square = terrain.square().unwrap();
    assert_eq!(terrain.halo(), Some(square));
    let halo = square_keys(GRID.widen(square, WATER_LIMITS.halo));
    assert!(halo.iter().all(|key| lake.built.contains(key)));
}

// Both layers in one frame (`frame_layers`): the water's holes, the terrain's, then both layers'
// level changes; the water follows the terrain's residency as the stream node feeds it. Returns
// both layers' work and whether the budget held a hole back.
#[allow(clippy::too_many_arguments)]
fn frame_pair(
    terrain: &mut PatchStream,
    land: &mut Scripts,
    water: &mut PatchStream,
    lake: &mut Scripts,
    eye: [f32; 3],
    delta: f64,
    budget_us: f64,
    in_view: &dyn Fn(PatchKey) -> bool,
) -> (PatchWork, PatchWork, bool) {
    terrain.set_square(GRID.target(eye, 3000.0, false));
    water.set_halo(terrain.square().map(|square| GRID.widen(square, WATER_LIMITS.halo)));
    let mut budget = FrameBudget::new(budget_us);
    let held = frame_layers(
        &mut [Some(LayerFrame::new(water, lake)), Some(LayerFrame::new(terrain, land))],
        Some(eye),
        delta,
        &mut budget,
        &FixedClock,
        false,
        in_view,
        None,
    );
    deliver(terrain, land);
    deliver(water, lake);
    for (key, resident) in terrain.take_changes() {
        water.set_wanted(key, resident);
    }
    let _ = water.take_changes().count();
    (terrain.work(), water.work(), held)
}

#[test]
fn no_water_step_runs_ahead_of_a_held_terrain_hole() {
    // `TERRAIN-41` L1: settled at one eye, then 3 km away under a tight budget, so the terrain's
    // new strips wait as holes in view while both layers' resident patches queue level changes.
    // In a frame that held back a terrain hole in view, each layer runs only its one guaranteed
    // in-view change; the water's budgeted changes no longer run ahead of the terrain's holes.
    let mut terrain = PatchStream::new(TERRAIN_LIMITS, COSTS);
    let mut water = PatchStream::new(WATER_LIMITS, COSTS);
    terrain.reset(Some(GRID));
    water.reset(Some(GRID));
    let (mut land, mut lake) = (Scripts::default(), Scripts::default());
    let all = |_: PatchKey| true;
    let mut eye = [0.0_f32, 800.0, 0.0];
    for _ in 0..200 {
        let (.., held) = frame_pair(&mut terrain, &mut land, &mut water, &mut lake, eye, 0.1, f64::INFINITY, &all);
        // `TERRAIN-41` L2: no budget, no hole held back.
        assert!(!held);
    }
    assert!(!terrain.has_pending_work() && !water.has_pending_work());
    eye = [3000.0, 800.0, 1500.0];
    let (mut held, mut water_queued_while_held, mut steps_while_held) = (0, 0, [0, 0]);
    for _ in 0..400 {
        let water_queued = water.pending()[2];
        let (land_work, lake_work, hole_held) =
            frame_pair(&mut terrain, &mut land, &mut water, &mut lake, eye, 0.1, 300.0, &all);
        // `TERRAIN-41` L2: the frame reports a hole held back (what the graded book's builds
        // wait for) whenever one in view was.
        assert!(hole_held || land_work.holes_deferred_in_view + lake_work.holes_deferred_in_view == 0);
        if land_work.holes_deferred_in_view > 0 {
            held += 1;
            water_queued_while_held += usize::from(water_queued > 1);
            assert!(lake_work.lod_changes <= 1, "{lake_work:?}");
            assert!(land_work.lod_changes <= 1, "{land_work:?}");
            steps_while_held[0] += land_work.lod_changes;
            steps_while_held[1] += lake_work.lod_changes;
        }
    }
    assert!(held > 10 && water_queued_while_held > 0, "{held} {water_queued_while_held}");
    assert!(steps_while_held[0] > 0 && steps_while_held[1] > 0, "{steps_while_held:?}");
    assert!(!terrain.has_pending_work() && !water.has_pending_work(), "it settles");
    for (key, step) in terrain.resident_keys().map(|key| (key, terrain.installed_step(key))) {
        assert_eq!(step, Some(lod_step(lod_distance(eye, GRID.centre(key)))), "{key:?}");
    }
}

#[test]
fn a_queued_in_view_step_runs_within_a_frame_while_holes_wait() {
    // A budget that admits only the frame's first unit: the holes take it, and still one queued
    // in-view level change runs every frame, so near ground does not stay at a stale step.
    let mut layer = PatchStream::new(TERRAIN_LIMITS, COSTS);
    layer.reset(Some(GRID));
    let mut scripts = Scripts::default();
    let run = |layer: &mut PatchStream, scripts: &mut Scripts, eye: [f32; 3], budget_us: f64| {
        layer.set_square(GRID.target(eye, 3000.0, false));
        let mut budget = FrameBudget::new(budget_us);
        let work = layer.frame(Some(eye), 0.1, &mut budget, &FixedClock, false, &|_| true, scripts);
        deliver(layer, scripts);
        work
    };
    for _ in 0..200 {
        run(&mut layer, &mut scripts, [0.0, 800.0, 0.0], f64::INFINITY);
    }
    let eye = [3000.0, 800.0, 1500.0];
    let mut frames_with_both = 0;
    for _ in 0..400 {
        let [holes, _, queued] = layer.pending();
        let work = run(&mut layer, &mut scripts, eye, 1.0);
        if queued > 0 && work.holes_deferred_in_view > 0 {
            assert_eq!(work.lod_changes, 1, "{holes} holes, {queued} queued: {work:?}");
            frames_with_both += 1;
        }
    }
    assert!(frames_with_both > 10, "{frames_with_both}");
    assert!(!layer.has_pending_work());
}

// The control's recorded decisions (`dev` `daec355a`'s `terrain.gd`, recorded by the round's
// recorder on the stream benchmark's camera paths): per frame the camera, delta, far plane and
// resident count, the residency's target square, deferred level checks, and at every check the
// resident patches with their installed steps and the queued candidates. Replayed: the square of
// every frame, the frames the gate opens and every check's candidates must be the control's.
// `METRUM_T1_GOLDEN=<dir of *.rec> cargo test --lib terrain::tests::golden -- --ignored`.
#[test]
#[ignore = "replays the control's recordings named by METRUM_T1_GOLDEN"]
fn golden_control_residency_and_levels() {
    let directory = std::env::var("METRUM_T1_GOLDEN").expect("METRUM_T1_GOLDEN");
    let mut files: Vec<_> = std::fs::read_dir(&directory)
        .unwrap()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "rec"))
        .collect();
    files.sort();
    assert!(!files.is_empty());
    for path in files {
        let text = std::fs::read_to_string(&path).unwrap();
        let report = replay(&text);
        eprintln!("GOLDEN {} {report:?}", path.display());
        assert_eq!(report.square_mismatches, 0, "{}", path.display());
        assert_eq!(report.candidate_mismatches, 0, "{}", path.display());
        assert_eq!(report.gate_mismatches, 0, "{}", path.display());
    }
}

// The recordings' first 140 KB each (30k: 1,148 frames and 32 checks; Kuopio: its load, 759
// frames and 6 checks), kept in the repository so the replay runs in every gate.
#[test]
fn golden_control_subsets_replay() {
    for (name, text) in [
        ("30k", include_str!("golden/30k.rec")),
        ("kuopio", include_str!("golden/kuopio.rec")),
    ] {
        let report = replay(text);
        assert!(report.checks > 0 && report.squares > 0, "{name}: {report:?}");
        assert_eq!(report.square_mismatches, 0, "{name}");
        assert_eq!(report.candidate_mismatches, 0, "{name}");
        assert_eq!(report.gate_mismatches, 0, "{name}");
    }
}

#[derive(Debug, Default)]
struct Replay {
    frames: usize,
    squares: usize,
    square_mismatches: usize,
    checks: usize,
    gate_mismatches: usize,
    candidate_mismatches: usize,
    candidates: usize,
}

fn replay(text: &str) -> Replay {
    let mut report = Replay::default();
    let mut grid = None;
    let mut gate = LodGate::default();
    let mut frame: Option<([f32; 3], f32, f64, usize)> = None;
    let mut deferred = false;
    let mut checked = false;
    let mut resident: Vec<(PatchKey, u8)> = Vec::new();
    let mut check_eye = [0.0_f32; 3];
    let close_frame = |gate: &mut LodGate,
                           frame: Option<([f32; 3], f32, f64, usize)>,
                           deferred: bool,
                           checked: bool,
                           report: &mut Replay| {
        let Some((eye, _, delta, residents)) = frame else {
            return;
        };
        let opened = if deferred {
            gate.defer(delta);
            false
        } else {
            gate.frame(residents > 0, Some(eye), delta)
        };
        if opened != checked {
            report.gate_mismatches += 1;
        }
    };
    for line in text.lines() {
        let mut fields = line.split(' ');
        match fields.next() {
            Some("L") => {
                let v: Vec<f64> = fields.map(|f| f.parse().unwrap()).collect();
                grid = Some(PatchGrid {
                    cols: v[0] as i32,
                    rows: v[1] as i32,
                    interval: v[2] as i32,
                    cell: v[3] as f32,
                    world: [v[4] as f32, v[5] as f32],
                });
                gate = LodGate::default();
                // The load's own sync runs before the new world's first frame.
                close_frame(&mut gate, frame.take(), deferred, checked, &mut report);
                gate = LodGate::default();
            }
            Some("F") => {
                close_frame(&mut gate, frame, deferred, checked, &mut report);
                let fields: Vec<&str> = fields.collect();
                // The delta is Godot's `var_to_bytes` of the double, hex: a 4-byte header, then
                // its 8 bytes little-endian.
                let bytes: Vec<u8> = (0..fields[1].len())
                    .step_by(2)
                    .map(|at| u8::from_str_radix(&fields[1][at..at + 2], 16).unwrap())
                    .collect();
                let delta = f64::from_le_bytes(bytes[4..12].try_into().unwrap());
                let v: Vec<f64> = fields[2..].iter().map(|f| f.parse().unwrap()).collect();
                frame = Some((
                    [v[0] as f32, v[1] as f32, v[2] as f32],
                    v[3] as f32,
                    delta,
                    v[4] as usize,
                ));
                deferred = false;
                checked = false;
                report.frames += 1;
            }
            Some("T") => {
                let (Some(grid), Some((eye, far, _, _))) = (grid, frame) else {
                    continue;
                };
                let v: Vec<i32> = fields.map(|f| f.parse().unwrap()).collect();
                report.squares += 1;
                if grid.target(eye, far, false) != [v[0], v[1], v[2], v[3]] {
                    report.square_mismatches += 1;
                }
            }
            Some("D") => deferred = true,
            Some("R") => {
                checked = true;
                report.checks += 1;
                let eye: Vec<f32> = fields.by_ref().take(3).map(|f| f.parse().unwrap()).collect();
                check_eye = [eye[0], eye[1], eye[2]];
                resident = fields
                    .map(|entry| {
                        let v: Vec<i32> = entry.split(',').map(|f| f.parse().unwrap()).collect();
                        ([v[0], v[1]], v[2] as u8)
                    })
                    .collect();
            }
            Some("C") => {
                let grid = grid.expect("a layout before a check");
                let recorded: BTreeSet<PatchKey> = fields
                    .map(|entry| {
                        let v: Vec<i32> = entry.split(',').map(|f| f.parse().unwrap()).collect();
                        [v[0], v[1]]
                    })
                    .collect();
                let rule: BTreeSet<PatchKey> = resident
                    .iter()
                    .filter(|(key, step)| {
                        lod_step(lod_distance(check_eye, grid.centre(*key))) != *step
                    })
                    .map(|(key, _)| *key)
                    .collect();
                report.candidates += recorded.len();
                if rule != recorded {
                    report.candidate_mismatches += 1;
                }
            }
            _ => {}
        }
    }
    close_frame(&mut gate, frame, deferred, checked, &mut report);
    report
}

#[test]
fn a_refused_payload_leaves_no_pending_work() {
    // A wanted patch whose payload is refused is never asked again: the layer settles without
    // it (AUDIT-stream-2 finding 5), and it counts as missing again only when wanted anew.
    let mut layer = PatchStream::new(TERRAIN_LIMITS, COSTS);
    layer.reset(Some(GRID));
    let mut scripts = Scripts::default();
    let eye = [0.0, 800.0, 0.0];
    layer.set_square(GRID.target(eye, 16000.0, false));
    let mut budget = FrameBudget::new(f64::INFINITY);
    layer.frame(Some(eye), 0.016, &mut budget, &FixedClock, false, &|_| false, &mut scripts);
    let refused = scripts.requested.remove(0);
    layer.payload(refused, Payload::Refused);
    for _ in 0..200 {
        frame(&mut layer, &mut scripts, eye, 0.016);
    }
    assert!(!scripts.resident.contains_key(&refused));
    assert_eq!(layer.refused_count(), 1);
    assert_eq!(layer.pending()[0], 0);
    assert!(!layer.has_pending_work(), "a refused hole holds no settle");
    assert_ne!(refused, [0, 0]);
    layer.set_square([0, 0, 0, 0]);
    assert_eq!(layer.refused_count(), 0, "out of the square it is no hole");
}

#[test]
fn the_requested_count_is_the_requests_without_news() {
    // `TERRAIN-35`'s guard: the scripts check that the core waits on no request they dropped.
    let mut layer = PatchStream::new(TERRAIN_LIMITS, COSTS);
    layer.reset(Some(GRID));
    let mut scripts = Scripts::default();
    let eye = [0.0, 800.0, 0.0];
    layer.set_square(GRID.target(eye, 16000.0, false));
    let mut budget = FrameBudget::new(f64::INFINITY);
    layer.frame(Some(eye), 0.016, &mut budget, &FixedClock, false, &|_| false, &mut scripts);
    assert!(!scripts.requested.is_empty());
    assert_eq!(layer.requested_count(), scripts.requested.len());
    let dropped = scripts.requested.pop().unwrap();
    layer.payload(dropped, Payload::Dropped);
    // News for a patch not marked requested changes nothing.
    layer.payload(dropped, Payload::Dropped);
    assert_eq!(layer.requested_count(), scripts.requested.len());
    deliver(&mut layer, &mut scripts);
    assert_eq!(layer.requested_count(), 0);
    // The dropped one is asked for again.
    layer.frame(Some(eye), 0.016, &mut budget, &FixedClock, false, &|_| false, &mut scripts);
    assert!(scripts.requested.contains(&dropped));
    assert_eq!(layer.requested_count(), scripts.requested.len());
    layer.reset(Some(GRID));
    assert_eq!(layer.requested_count(), 0);
}

// `TERRAIN-41` B1: the budget's keep, release and fill, on the 60 km world's layout (117 x 117
// patches of 64 cells of 8 m), where the wanted square at 800 m is a fraction of the grid.
const WIDE: PatchGrid = PatchGrid {
    cols: 117,
    rows: 117,
    interval: 64,
    cell: 8.0,
    world: [59904.0, 59904.0],
};

// One frame on `WIDE` at `eye` with `places` (none: no budget) and a frame budget of
// `budget_us`; payloads arrive the frame after their request.
fn wide_frame(
    layer: &mut PatchStream,
    scripts: &mut Scripts,
    eye: [f32; 3],
    budget_us: f64,
) -> PatchWork {
    layer.set_square(WIDE.target(eye, 1.0e6, false));
    let mut budget = FrameBudget::new(budget_us);
    let work = layer.frame(Some(eye), 0.05, &mut budget, &FixedClock, false, &|_| false, scripts);
    deliver(layer, scripts);
    let _ = layer.take_changes().count();
    let _ = layer.take_generations().count();
    let _ = layer.take_released().count();
    work
}

fn wide_layer(places: Option<u64>) -> PatchStream {
    let mut layer = PatchStream::new(TERRAIN_LIMITS, COSTS);
    layer.reset(Some(WIDE));
    layer.set_places(places);
    layer
}

// A flight across the grid at 800 m, a turn of the eye's height, and a 20 km jump and back.
fn wide_walk() -> Vec<[f32; 3]> {
    let mut eyes = Vec::new();
    for step in 0..160 {
        eyes.push([-12000.0 + step as f32 * 120.0, 800.0, -3000.0]);
    }
    for step in 0..40 {
        eyes.push([7200.0, 800.0 + step as f32 * 20.0, -3000.0]);
    }
    for _ in 0..60 {
        eyes.push([7200.0, 1600.0, 17000.0]);
    }
    for _ in 0..60 {
        eyes.push([7200.0, 1600.0, -3000.0]);
    }
    eyes
}

fn settle(layer: &mut PatchStream, scripts: &mut Scripts, eye: [f32; 3], budget_us: f64) {
    for _ in 0..4000 {
        wide_frame(layer, scripts, eye, budget_us);
        if !layer.has_pending_work() && layer.fill_left() == 0 && layer.releasable.is_empty() {
            return;
        }
    }
    panic!("never settled");
}

#[test]
fn a_whole_grid_budget_fills_once_then_builds_and_releases_nothing() {
    let mut layer = wide_layer(Some(u64::MAX));
    let mut plain = wide_layer(None);
    let (mut scripts, mut plain_scripts) = (Scripts::default(), Scripts::default());
    let start = [-12000.0, 800.0, -3000.0];
    settle(&mut layer, &mut scripts, start, f64::INFINITY);
    assert_eq!(layer.built_count(), (WIDE.cols * WIDE.rows) as usize, "the fill builds the grid");
    assert_eq!(layer.release_rect(), None);
    settle(&mut plain, &mut plain_scripts, start, f64::INFINITY);
    let builds = scripts.builds;
    for (step, eye) in wide_walk().into_iter().enumerate() {
        wide_frame(&mut layer, &mut scripts, eye, 2000.0);
        wide_frame(&mut plain, &mut plain_scripts, eye, f64::INFINITY);
        assert_eq!(scripts.builds, builds, "frame {step}: a build after the fill");
        assert!(scripts.released.is_empty(), "frame {step}: a release");
    }
    settle(&mut layer, &mut scripts, [7200.0, 1600.0, -3000.0], 2000.0);
    // What is drawn is what a layer without a budget draws.
    assert_eq!(scripts.resident, plain_scripts.resident);
}

#[test]
fn the_smallest_budget_keeps_its_places_and_releases_beyond_the_ring() {
    let start = [-12000.0, 800.0, -3000.0];
    let square = WIDE.target(start, 1.0e6, false);
    let ring = WIDE.widen(square, RELEASE_RING);
    // The wanted square and its release ring, no more (`K` = 0).
    let places = ((ring[2] - ring[0] + 1) * (ring[3] - ring[1] + 1)) as u64;
    let mut layer = wide_layer(Some(places));
    let mut plain = wide_layer(None);
    let (mut scripts, mut plain_scripts) = (Scripts::default(), Scripts::default());
    settle(&mut layer, &mut scripts, start, f64::INFINITY);
    assert_eq!(layer.keep().map(|keep| (keep.ring, keep.overrun)), Some((0, false)));
    for budget_us in [f64::INFINITY, 1000.0] {
        for (step, eye) in wide_walk().into_iter().enumerate() {
            let work = wide_frame(&mut layer, &mut scripts, eye, budget_us);
            wide_frame(&mut plain, &mut plain_scripts, eye, f64::INFINITY);
            assert!(layer.built_count() as u64 <= places, "frame {step}: {} built", layer.built_count());
            assert_eq!(layer.built_count(), scripts.built.len());
            // In a frame where a hole waits (a hole the budget held back stays missing), a
            // release is a build's donor only.
            if layer.pending()[0] > 0 {
                assert_eq!(work.releases, work.donors, "frame {step}");
            }
        }
        let eye = [7200.0, 1600.0, -3000.0];
        settle(&mut layer, &mut scripts, eye, budget_us);
        settle(&mut plain, &mut plain_scripts, eye, f64::INFINITY);
        // Settled, every built patch lies in the release rectangle, and the drawn set is the
        // one without a budget.
        let release = layer.release_rect().expect("a budget releases");
        assert!(scripts.built.iter().all(|key| {
            key[0] >= release[0] && key[1] >= release[1] && key[0] <= release[2] && key[1] <= release[3]
        }));
        assert_eq!(scripts.resident, plain_scripts.resident);
        assert!(!scripts.released.is_empty(), "the walk released nothing");
    }
}

#[test]
fn a_wobble_across_a_boundary_builds_and_frees_nothing_after_its_first_cycle() {
    // 4,000 places: a keep ring of a few patches. The eye wobbles within one patch of a patch
    // boundary (x = 0 is a boundary of the 512 m lattice).
    let mut layer = wide_layer(Some(4000));
    let mut scripts = Scripts::default();
    let mut walk = Walk(0x3c6e_f372_fe94_f82b);
    let eye_at = |walk: &mut Walk| [((walk.next() - 0.5) * 1024.0) as f32, 800.0, 100.0];
    for _ in 0..400 {
        let eye = eye_at(&mut walk);
        wide_frame(&mut layer, &mut scripts, eye, f64::INFINITY);
    }
    settle(&mut layer, &mut scripts, [0.0, 800.0, 100.0], f64::INFINITY);
    let (builds, released) = (scripts.builds, scripts.released.len());
    for _ in 0..400 {
        let eye = eye_at(&mut walk);
        wide_frame(&mut layer, &mut scripts, eye, f64::INFINITY);
    }
    assert_eq!((scripts.builds, scripts.released.len()), (builds, released));
}

#[test]
fn the_same_eyes_give_the_same_units_and_a_still_frame_does_nothing() {
    let run = || {
        let mut layer = wide_layer(Some(3500));
        let mut scripts = Scripts::default();
        for eye in wide_walk() {
            wide_frame(&mut layer, &mut scripts, eye, 1000.0);
        }
        (layer, scripts)
    };
    let (mut layer, mut scripts) = run();
    let (_, again) = run();
    assert!(scripts.calls.iter().any(|call| call.0 == b'r'), "the walk released nothing");
    assert_eq!(scripts.calls, again.calls);
    let eye = [7200.0, 1600.0, -3000.0];
    settle(&mut layer, &mut scripts, eye, 1000.0);
    let capacity = (layer.releasable.capacity(), layer.release_busy.capacity(), layer.prewarm.capacity());
    let calls = scripts.calls.len();
    for _ in 0..30 {
        let work = wide_frame(&mut layer, &mut scripts, eye, 1000.0);
        assert_eq!((work.ring_cells, work.releases, work.builds, work.activations), (0, 0, 0, 0));
    }
    assert_eq!(scripts.calls.len(), calls);
    assert_eq!(
        (layer.releasable.capacity(), layer.release_busy.capacity(), layer.prewarm.capacity()),
        capacity
    );
}

#[test]
fn over_the_budget_releases_wait_for_the_view_then_return_the_count_within_its_places() {
    // The extras grew while the view had work, so the places shrank under the built count
    // (`TERRAIN-41` B2: the budget is a target). A release that is not a donor waits while the
    // view waits for anything (here the vegetation, then holes waiting for their payloads); once
    // the view is in step the releases bring the built count within the places.
    let mut layer = wide_layer(Some(3500));
    let mut scripts = Scripts::default();
    let eye = [-12000.0, 800.0, -3000.0];
    settle(&mut layer, &mut scripts, eye, f64::INFINITY);
    let places = 2000;
    assert!(layer.built_count() > places);
    layer.set_places(Some(places as u64));
    assert!(!layer.releasable.is_empty());
    layer.hold_releases(true);
    for _ in 0..20 {
        assert_eq!(wide_frame(&mut layer, &mut scripts, eye, f64::INFINITY).releases, 0);
    }
    layer.hold_releases(false);
    // A jump: the new square's holes wait for their payloads (no build, so no donor).
    let far = [12000.0, 800.0, 12000.0];
    layer.set_square(WIDE.target(far, 1.0e6, false));
    let mut budget = FrameBudget::new(1.0e6);
    let work = layer.frame(Some(far), 0.05, &mut budget, &FixedClock, false, &|_| false, &mut scripts);
    assert!(layer.pending()[0] > 0 && work.activations == 0);
    assert_eq!(work.releases, 0, "a release ran while holes waited");
    // In step at the new eye, the count returns within the places, at the frame budget's pace.
    settle(&mut layer, &mut scripts, far, 2000.0);
    assert!(layer.built_count() <= places, "{} built", layer.built_count());
}

#[test]
fn a_build_takes_a_donor_only_while_the_budget_is_full() {
    // A jump at a small budget: its holes pair with donors while one more place would not fit;
    // with room left they build without one (`TERRAIN-41` B1c).
    for full in [false, true] {
        let mut layer = wide_layer(Some(3500));
        let mut scripts = Scripts::default();
        settle(&mut layer, &mut scripts, [-12000.0, 800.0, -3000.0], f64::INFINITY);
        layer.set_budget_full(full);
        let mut donors = 0;
        for _ in 0..200 {
            donors += wide_frame(&mut layer, &mut scripts, [12000.0, 800.0, 12000.0], f64::INFINITY).donors;
        }
        assert_eq!(donors > 0, full, "full: {full}");
    }
}

#[test]
fn releases_wait_while_the_other_layer_has_work() {
    // Settled after a jump's holes were built: the old square's patches are candidates. While the
    // other layer has work a settled view waits for, they wait; then they go.
    let mut layer = wide_layer(Some(3500));
    let mut scripts = Scripts::default();
    settle(&mut layer, &mut scripts, [-12000.0, 800.0, -3000.0], f64::INFINITY);
    layer.hold_releases(true);
    for _ in 0..200 {
        wide_frame(&mut layer, &mut scripts, [12000.0, 800.0, 12000.0], f64::INFINITY);
    }
    assert!(!layer.has_pending_work() && !layer.releasable.is_empty());
    let held = scripts.released.len();
    layer.hold_releases(false);
    let work = wide_frame(&mut layer, &mut scripts, [12000.0, 800.0, 12000.0], f64::INFINITY);
    assert!(work.releases > work.donors && scripts.released.len() > held);
}

// `VEG-35` R1: a scripted unit source (the vegetation's waiting holes and replacements in its
// order), each install a unit of `cost`, recording which ran as which unit of the frame.
struct Trees {
    holes: Vec<(Class, f64)>,
    at: usize,
    closed: bool,
    cost: f64,
    ran: Vec<(usize, u32)>,
}

impl Trees {
    fn new(mut holes: Vec<(Class, f64)>, cost: f64) -> Self {
        holes.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
        Self { holes, at: 0, closed: false, cost, ran: Vec::new() }
    }
}

impl UnitSource for Trees {
    fn next_unit(&self) -> Option<(Class, f64)> {
        if self.closed { None } else { self.holes.get(self.at).copied() }
    }
    fn install_next(&mut self, budget: &mut FrameBudget, _: &dyn Clock, reserve_us: Option<f64>) -> bool {
        let upgrade = self.holes[self.at].0 >= Class::UpgradeInView;
        let admitted = match (upgrade, reserve_us) {
            (false, _) => budget.admits(self.cost),
            (true, Some(reserve_us)) => budget.admits_upgrade(self.cost, reserve_us),
            (true, None) => true,
        };
        if !admitted {
            self.closed = true;
            return false;
        }
        self.ran.push((self.at, budget.units));
        budget.record(self.cost, self.cost);
        self.at += 1;
        true
    }
}

// A fresh layer's second frame (its first requested the nearest payloads) with `trees` in the
// hole order under `budget`: the scripts, the class and distance of each activation in order, and
// whether the frame held a hole back.
fn holes_with_trees(
    trees: Option<&mut Trees>,
    budget: &mut FrameBudget,
    in_view: &dyn Fn(PatchKey) -> bool,
) -> (Scripts, Vec<(Class, f64)>, bool) {
    let mut layer = PatchStream::new(TERRAIN_LIMITS, COSTS);
    layer.reset(Some(GRID));
    let mut scripts = Scripts::default();
    let eye = [0.0, 800.0, 0.0];
    frame(&mut layer, &mut scripts, eye, 0.016);
    layer.set_square(GRID.target(eye, 16000.0, false));
    let held = frame_layers(
        &mut [Some(LayerFrame::new(&mut layer, &mut scripts))],
        Some(eye),
        0.016,
        budget,
        &FixedClock,
        false,
        in_view,
        trees.map(|trees| trees as &mut dyn UnitSource),
    );
    let places = scripts
        .activations
        .iter()
        .map(|&(key, _)| {
            let index = GRID.index(key);
            let distance = ground_distance_sq(GRID, Some(eye), index);
            let class = if in_view(key) {
                Class::HoleInView
            } else if distance <= NEAR_RING_M * NEAR_RING_M {
                Class::HoleNear
            } else {
                Class::Hole
            };
            (class, distance)
        })
        .collect();
    (scripts, places, held)
}

#[test]
fn a_vegetation_hole_runs_before_the_farther_patch_holes_of_its_class_and_after_the_nearer() {
    let nowhere = |_: PatchKey| false;
    let (_, places, _) = holes_with_trees(None, &mut FrameBudget::new(f64::INFINITY), &nowhere);
    assert!(places.len() >= 8 && places.is_sorted_by(|a, b| !orders_before(*b, *a)), "{places:?}");
    // Trees between the patch holes, one tied with a patch hole (it goes after it), and farther
    // ones of a later class.
    let near = places.iter().filter(|place| place.0 == Class::HoleNear).count();
    assert!(near >= 4, "{places:?}");
    let mut wanted = vec![
        (Class::HoleNear, 0.0),
        (Class::HoleNear, (places[1].1 + places[2].1) * 0.5),
        places[3],
        (Class::Hole, 0.0),
    ];
    wanted.push((Class::Hole, places.last().unwrap().1 + 1.0));
    let mut trees = Trees::new(wanted, 5.0);
    let (scripts, with_trees, _) = holes_with_trees(Some(&mut trees), &mut FrameBudget::new(f64::INFINITY), &nowhere);
    assert_eq!(with_trees, places, "the patch holes keep their order");
    // Each tree runs as the unit after every patch hole ordered before it and the trees before it.
    for &(at, unit) in &trees.ran {
        let tree = trees.holes[at];
        let before = places.iter().filter(|&&place| !orders_before(tree, place)).count();
        assert_eq!(unit as usize, before + at, "tree {at} {tree:?}");
    }
    assert_eq!(trees.ran.len(), trees.holes.len());
    assert_eq!(scripts.activations.len(), places.len());
}

#[test]
fn an_in_view_vegetation_hole_runs_before_an_out_of_view_patch_hole_at_any_distance() {
    let mut trees = Trees::new(vec![(Class::HoleInView, 1.0e12), (Class::Hole, 0.0)], 5.0);
    let (_, places, _) = holes_with_trees(Some(&mut trees), &mut FrameBudget::new(f64::INFINITY), &|_| false);
    assert!(places.iter().all(|place| place.0 > Class::HoleInView && place.1 < 1.0e12));
    assert_eq!(trees.ran[0], (0, 0), "the frame's first unit");
    assert_eq!(trees.ran[1].1 as usize, 1 + places.iter().filter(|place| place.0 < Class::Hole).count());
}

#[test]
fn a_vegetation_hole_the_budget_refuses_waits_and_holds_the_frames_level_changes() {
    // Nearer patch holes spend the budget (five 200 us builds of 1,000): the farther tree waits,
    // its source closed for the frame, and is the next hole of the next frame.
    let mut trees = Trees::new(vec![(Class::Hole, 1.0e12)], 50.0);
    let (_, places, held) = holes_with_trees(Some(&mut trees), &mut FrameBudget::new(1000.0), &|_| false);
    assert_eq!(places.len(), 5);
    assert!(held && trees.closed && trees.ran.is_empty());
    // A settled layer whose level changes are queued (the eye rose 800 m; the square, set by the
    // 8 km floor, stays): with the frame's room spent before the vegetation's turn, the refused tree holds
    // every budgeted level change; without it they run.
    let run = |trees: Option<&mut Trees>| {
        let mut layer = PatchStream::new(TERRAIN_LIMITS, COSTS);
        layer.reset(Some(GRID));
        let mut scripts = Scripts::default();
        for _ in 0..200 {
            frame(&mut layer, &mut scripts, [0.0, 800.0, 0.0], 0.1);
        }
        assert!(!layer.has_pending_work());
        let eye = [0.0, 1600.0, 0.0];
        layer.set_square(GRID.target(eye, 16000.0, false));
        let mut budget = FrameBudget::new(2000.0);
        budget.record(1900.0, 1900.0);
        let held = frame_layers(
            &mut [Some(LayerFrame::new(&mut layer, &mut scripts))],
            Some(eye),
            0.1,
            &mut budget,
            &FixedClock,
            false,
            &|_| false,
            trees.map(|trees| trees as &mut dyn UnitSource),
        );
        (layer.work(), held)
    };
    let mut trees = Trees::new(vec![(Class::Hole, 0.0)], 300.0);
    let (with_trees, held) = run(Some(&mut trees));
    let (without, held_without) = run(None);
    assert_eq!((with_trees.activations, without.activations), (0, 0), "no patch hole");
    assert!(without.lod_changes > 0 && !held_without, "{without:?}");
    assert!(held && trees.closed && with_trees.lod_changes == 0, "{with_trees:?}");
}

#[test]
fn the_same_inputs_give_the_same_units_with_vegetation_holes() {
    let in_view = |key: PatchKey| key[0] > 18;
    let run = || {
        let mut trees = Trees::new(
            vec![(Class::HoleInView, 1.0e5), (Class::HoleNear, 1.0e6), (Class::HoleNear, 3.0e6), (Class::Hole, 0.0)],
            120.0,
        );
        let (scripts, _, held) = holes_with_trees(Some(&mut trees), &mut FrameBudget::new(3000.0), &in_view);
        (scripts.calls, trees.ran, trees.closed, held)
    };
    let first = run();
    assert!(!first.1.is_empty() && !first.0.is_empty());
    assert_eq!(run(), first);
}

#[test]
fn a_vegetation_replacement_runs_among_the_level_changes_by_class_and_distance() {
    // A settled layer whose level changes are queued (the eye rose 800 m): a nearer replacement
    // runs before every level change of its class, a farther one after them, and in a frame that
    // held a hole back (the source's own) none runs.
    let run = |trees: &mut Trees, spent: f64| {
        let mut layer = PatchStream::new(TERRAIN_LIMITS, COSTS);
        layer.reset(Some(GRID));
        let mut scripts = Scripts::default();
        for _ in 0..200 {
            frame(&mut layer, &mut scripts, [0.0, 800.0, 0.0], 0.1);
        }
        let eye = [0.0, 1600.0, 0.0];
        layer.set_square(GRID.target(eye, 16000.0, false));
        let mut budget = FrameBudget::new(2000.0);
        if spent > 0.0 {
            budget.record(spent, spent);
        }
        frame_layers(
            &mut [Some(LayerFrame::new(&mut layer, &mut scripts))],
            Some(eye),
            0.1,
            &mut budget,
            &FixedClock,
            false,
            &|_| false,
            Some(trees as &mut dyn UnitSource),
        );
        (layer.work(), budget.units)
    };
    let mut trees = Trees::new(vec![(Class::Upgrade, 0.0), (Class::Upgrade, 1.0e12)], 5.0);
    let (work, units) = run(&mut trees, 0.0);
    assert!(work.lod_changes > 2, "{work:?}");
    assert_eq!(trees.ran, vec![(0, 0), (1, 1 + work.lod_changes as u32)]);
    assert_eq!(units as usize, work.lod_changes + 2);
    // A hole that does not fit holds the replacements behind it, but for the first in view of the
    // frame, which runs as a layer's first in-view level change does.
    let mut trees = Trees::new(vec![(Class::Hole, 0.0), (Class::Upgrade, 0.0)], 300.0);
    let (work, _) = run(&mut trees, 1900.0);
    assert!(trees.closed && trees.ran.is_empty() && work.lod_changes == 0, "{work:?}");
    let mut trees = Trees::new(vec![(Class::UpgradeInView, 0.0), (Class::UpgradeInView, 1.0), (Class::Upgrade, 0.0)], 300.0);
    let (work, _) = run(&mut trees, 1900.0);
    assert_eq!((trees.ran.len(), trees.at, work.lod_changes), (1, 1, 0), "{work:?}");
}
