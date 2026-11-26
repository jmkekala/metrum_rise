// SPDX-License-Identifier: GPL-3.0-or-later

//! The wanted set at the eye's height (`VEG-36`): equal to the ground rule at a low eye, empty
//! above the canopy's far range, the stated rule in between (the ground rule's patches whose
//! square has a point within the far range and a side of the eye in three dimensions, the eye's
//! rise taken over the map's ceiling), checked on random walks that move in height alone too;
//! `keeps` stays one-sided under rise changes; a zoom out and back requests and frees nothing.

use super::*;

const CEILING_M: f64 = 150.0;

// The stated rule, independently of the residency's arithmetic: the ground rule's patches whose
// square's nearest point (clamped per axis) lies within `far + side` of the eye in three
// dimensions, the eye `eye[1] - CEILING_M` above the highest ground (no cut at or below it).
fn stated_wanted(resident: &BTreeSet<Cell>, eye: [f32; 3]) -> BTreeSet<PatchKey> {
    let settings = settings();
    let rise = (f64::from(eye[1]) - CEILING_M).max(0.0);
    let camera = [eye[0], eye[2]];
    rule_wanted(resident, camera)
        .into_iter()
        .filter(|&key| {
            let side = key_span(key, TERRAIN_SPAN);
            let origin = key_origin(key, TERRAIN_SPAN, WORLD);
            let near = |axis: usize| {
                let low = f64::from(origin[axis]);
                f64::from(camera[axis]).clamp(low, low + side) - f64::from(camera[axis])
            };
            let reach = settings.product.far_m + side;
            near(0).powi(2) + near(1).powi(2) + rise * rise <= reach * reach
        })
        .collect()
}

fn world_with_ceiling() -> World {
    let mut world = World::new(all_blocks());
    world.jobs.ceiling = Some(CEILING_M);
    world
}

// The rise the residency evaluates at: 0 at or below the floor, where the cut keeps everything.
fn snapped_rise(height: f32) -> f64 {
    let rise = f64::from(height) - CEILING_M;
    if rise > settings().rise_floor(TERRAIN_SPAN) { rise } else { 0.0 }
}

#[test]
fn a_low_eye_wants_the_ground_set_and_an_eye_above_the_far_range_wants_nothing() {
    let settings = settings();
    let resident = all_blocks();
    let mut world = world_with_ceiling();
    // The highest eye the floor leaves on the ground rule, and one just above the far range.
    let low = (CEILING_M + settings.rise_floor(TERRAIN_SPAN)) as f32;
    let high = (CEILING_M + settings.product.far_m + TERRAIN_SPAN) as f32 + 1.0;
    for (x, z) in [(0.0, 0.0), (1234.5, -987.0), (-4321.0, 2500.0)] {
        for height in [120.0, 900.0, low] {
            world.frame([x, height, z], |_| true);
            let wanted: BTreeSet<PatchKey> = world.residency.wanted().collect();
            assert!(!wanted.is_empty());
            assert_eq!(wanted, rule_wanted(&resident, [x, z]), "the ground set at {height} m");
            assert_eq!(wanted, stated_wanted(&resident, [x, height, z]), "the stated rule at {height} m");
        }
        world.frame([x, high, z], |_| true);
        assert_eq!(world.residency.wanted().count(), 0, "nothing within the far range at {high} m");
    }
    // Without a ceiling (no core) nothing is cut.
    let mut world = World::new(all_blocks());
    world.frame([0.0, high, 0.0], |_| true);
    let wanted: BTreeSet<PatchKey> = world.residency.wanted().collect();
    assert_eq!(wanted, rule_wanted(&resident, [0.0, 0.0]));
}

#[test]
fn the_wanted_set_equals_the_stated_rule_on_walks_in_height() {
    let span = TERRAIN_SPAN / 4.0;
    let mut cut = 0;
    for seed in 0..3 {
        let mut rng = StdRng::seed_from_u64(seed);
        let resident = all_blocks();
        let mut world = world_with_ceiling();
        let mut at = [rng.gen_range(-3000.0..3000.0f32), 400.0, rng.gen_range(-3000.0..3000.0f32)];
        let mut evaluated_at: Option<[f32; 3]> = None;
        let mut last = None;
        for step in 0..400 {
            match rng.gen_range(0..30) {
                // A jump, anywhere and at any height.
                0 => {
                    at = [
                        rng.gen_range(-5000.0..5000.0),
                        rng.gen_range(100.0..9000.0),
                        rng.gen_range(-5000.0..5000.0),
                    ]
                }
                1..=4 => {}
                // A zoom: height alone.
                5..=14 => at[1] = (at[1] * rng.gen_range(0.8..1.25f32)).clamp(100.0, 9000.0),
                _ => {
                    at[0] = (at[0] + rng.gen_range(-40.0..40.0)).clamp(-5000.0, 5000.0);
                    at[2] = (at[2] + rng.gen_range(-40.0..40.0)).clamp(-5000.0, 5000.0);
                }
            }
            let state = (rules::camera_cell([at[0], at[2]], span), snapped_rise(at[1]).to_bits());
            let evaluates = last != Some(state);
            last = Some(state);
            if evaluates {
                evaluated_at = Some(at);
            }
            let work = world.frame(at, |_| rng.gen_range(0..3) > 0);
            assert_eq!(work.evaluated, evaluates, "seed {seed} step {step}: evaluated on a crossing or a rise change");
            let wanted: BTreeSet<PatchKey> = world.residency.wanted().collect();
            let stated = stated_wanted(&resident, evaluated_at.unwrap());
            assert_eq!(wanted, stated, "seed {seed} step {step} at {at:?}");
            let camera = [evaluated_at.unwrap()[0], evaluated_at.unwrap()[2]];
            cut += rule_wanted(&resident, camera).len() - stated.len();
        }
    }
    assert!(cut > 10_000, "the walks cut {cut} patches");
}

#[test]
fn a_product_kept_before_the_patch_units_is_still_wanted_after_a_rise_change() {
    // `keeps` reads the ceiling the frame reads and the eye's height: on walks in height, every
    // waiting product it keeps is still wanted once the frame evaluated.
    let mut kept_count = 0;
    for seed in 0..3 {
        let mut rng = StdRng::seed_from_u64(seed);
        let mut world = world_with_ceiling();
        let mut at = [rng.gen_range(-3000.0..3000.0f32), 1500.0, rng.gen_range(-3000.0..3000.0f32)];
        for step in 0..300 {
            at[1] = (at[1] * rng.gen_range(0.7..1.4f32)).clamp(100.0, 6000.0);
            at[0] = (at[0] + rng.gen_range(-100.0..100.0)).clamp(-5000.0, 5000.0);
            at[2] = (at[2] + rng.gen_range(-100.0..100.0)).clamp(-5000.0, 5000.0);
            world.residency.read_ceiling(&mut world.jobs);
            let kept: Vec<PatchKey> = world
                .layer
                .pending
                .iter()
                .map(|pending| pending.delivery.key)
                .filter(|&key| world.residency.keeps(key, Some(at), |_| true))
                .collect();
            world.frame(at, |_| false);
            let wanted: BTreeSet<PatchKey> = world.residency.wanted().collect();
            for key in &kept {
                assert!(wanted.contains(key), "seed {seed} step {step}: {key:?} kept and dropped");
            }
            kept_count += kept.len();
            world.jobs.complete(&mut world.layer, |_| rng.gen_range(0..3) > 0);
        }
    }
    assert!(kept_count > 1000, "{kept_count} kept");
}

#[test]
fn a_zoom_out_past_the_far_range_and_back_requests_and_frees_nothing() {
    let mut world = world_with_ceiling();
    let ground = [300.0, 400.0, -200.0];
    world.settle(ground);
    let drawn = world.drawn();
    let serials: Vec<Option<u64>> = drawn.iter().map(|&key| world.serial(key)).collect();
    let installed = world.layer.installed.len();
    let requests = world.jobs.requests.len();
    for height in [800.0, 1600.0, 3200.0, 5000.0, 6400.0, 9000.0, 6400.0, 3200.0, 1600.0, 400.0] {
        world.frame([ground[0], height, ground[2]], |_| true);
    }
    assert_eq!(world.residency.wanted().count(), drawn.len());
    world.settle(ground);
    assert_eq!(world.jobs.requests.len(), requests, "no request: the patches came back from the cache");
    assert_eq!(world.layer.installed.len(), installed, "no patch freed");
    assert_eq!(world.drawn(), drawn);
    let after: Vec<Option<u64>> = drawn.iter().map(|&key| world.serial(key)).collect();
    assert_eq!(after, serials, "the same products");
}
