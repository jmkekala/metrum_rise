// SPDX-License-Identifier: GPL-2.0-only

//! The vegetation residency without Godot (`docs/terrain.md` "Streaming core" 5.5), against the
//! recording backend and scripted jobs: the tiers against the rule evaluated over every resident
//! block on random walks, the bands against the rule over every drawn patch, one grid tier per
//! area in every frame, work only on crossings and moves (invariant 3), yaw independence (4),
//! completion order independence (5), staleness pushed by edits, the hysteresis boundaries and the
//! restore path (`vegetation_invalidation_test`'s contracts), and steady storage.

use super::rules::BAND_HYSTERESIS_M;
use super::*;
use crate::nodes::sim::render::stream::backend::recorder::Recorder;
use crate::nodes::sim::render::stream::backend::Role;
use crate::nodes::sim::render::stream::layers::vegetation::{
    Delivery, VARIANT_COUNTS, VegetationRules,
};
use crate::nodes::sim::render::stream::schedule::{FrameBudget, MonotonicClock, STREAM_BUDGET_US};
use crate::nodes::sim::render::stream::view::CameraSample;
use crate::nodes::sim::render::vegetation_product::{
    PLACEMENT_STRIDE, ProductCatalogue, ProductInputs, VegetationProduct,
    build_product,
};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::collections::{BTreeMap, BTreeSet};

mod golden;


const TERRAIN_SPAN: f64 = 510.0;
const WORLD: [f32; 2] = [10200.0, 10200.0];

const RULES: VegetationRules = VegetationRules {
    near_detail_m: 45.0,
    detail_fade_m: 10.0,
    shadow_handover_m: 60.0,
    landscape_first_variant: 6,
    hedge_first_variant: 12,
};

fn settings() -> ResidencySettings {
    ResidencySettings::from_lanes(&[
        1.0, 200.0, 150.0, 4500.0, 420.0, 420.0, 4.0, 0.0, 400.0, WORLD[0] as f64,
        WORLD[1] as f64,
    ])
    .unwrap()
}

fn catalogue() -> ProductCatalogue {
    let mut lanes = Vec::new();
    for &variants in &VARIANT_COUNTS[..2] {
        lanes.push(0.0);
        for _ in 0..variants {
            lanes.extend_from_slice(&[-0.5, 0.0, -0.5, 1.0, 1.0, 1.0]);
        }
    }
    ProductCatalogue::from_lanes(&lanes).unwrap()
}

// A product that depends on the patch and its bands only: a few plants of every species.
fn product(order: &PatchOrder) -> VegetationProduct {
    let count = 6 + (order.key[0].rem_euclid(5) + order.key[1].rem_euclid(3)) as usize;
    let mut data = vec![0.0f32; count * PLACEMENT_STRIDE];
    for (i, record) in data
        .as_chunks_mut::<PLACEMENT_STRIDE>()
        .0
        .iter_mut()
        .enumerate()
    {
        record[0] = order.origin[0] + (i % 4) as f32 * 9.0 + 1.0;
        record[2] = order.origin[1] + (i / 4) as f32 * 9.0 + 1.0;
        record[4] = 1.0;
        record[5] = (i % 4) as f32;
    }
    let inputs = ProductInputs {
        placements: &data,
        cover: None,
        runs: &[],
        origin: order.origin,
        near_band: order.bands.near_band,
        caster: order.bands.caster,
        settings: settings().product,
    };
    let mut out = VegetationProduct::default();
    build_product(&inputs, &catalogue(), &mut out);
    out
}

// Scripted jobs: requests wait until the test completes them; the revisions are a table.
#[derive(Default)]
struct Jobs {
    serial: u64,
    out: Vec<(u64, PatchOrder)>,
    cancelled: BTreeSet<PatchKey>,
    requests: Vec<PatchKey>,
    epoch: i64,
    revisions: BTreeMap<Cell, i64>,
    busy_core: bool,
    full: bool,
}

impl PatchJobs for Jobs {
    fn request(&mut self, order: &PatchOrder, _: &ResidencySettings) -> Option<u64> {
        if self.full {
            return None;
        }
        self.serial += 1;
        self.out.retain(|(_, out)| out.key != order.key);
        self.out.push((self.serial, *order));
        self.cancelled.remove(&order.key);
        self.requests.push(order.key);
        Some(self.serial)
    }

    fn cancel(&mut self, key: PatchKey) {
        self.out.retain(|(_, out)| out.key != key);
        self.cancelled.insert(key);
    }

    fn clear(&mut self) {
        self.out.clear();
    }

    fn generations(&mut self, known: i64, table: &mut Vec<(Cell, i64)>) -> Option<i64> {
        if self.busy_core {
            return None;
        }
        table.clear();
        if self.epoch != known {
            table.extend(self.revisions.iter().map(|(k, v)| (*k, *v)));
        }
        Some(self.epoch)
    }
}

impl Jobs {
    fn edit(&mut self, block: Cell) {
        *self.revisions.entry(block).or_insert(0) += 1;
        self.epoch += 1;
    }

    // Hands every finished request (all, or those `pick` chooses) to the layer.
    fn complete(&mut self, layer: &mut VegetationLayer, mut pick: impl FnMut(usize) -> bool) {
        let mut at = 0;
        let mut index = 0;
        while at < self.out.len() {
            if pick(index) {
                let (serial, order) = self.out.remove(at);
                layer.deliver(Delivery {
                    key: order.key,
                    serial,
                    product: product(&order),
                    origin: order.origin,
                    span: order.span,
                });
            } else {
                at += 1;
            }
            index += 1;
        }
    }
}

struct World {
    residency: VegetationResidency,
    layer: VegetationLayer,
    backend: Recorder,
    jobs: Jobs,
    clock: MonotonicClock,
}

impl World {
    fn new(resident: impl IntoIterator<Item = Cell>) -> Self {
        let mut world = Self {
            residency: VegetationResidency::default(),
            layer: VegetationLayer::new(1 << 16, 1024),
            backend: Recorder::new(),
            jobs: Jobs::default(),
            clock: MonotonicClock::default(),
        };
        world.layer.set_rules(RULES);
        world.layer.set_subdivision(settings().subdivision);
        world.residency.set_terrain_span(TERRAIN_SPAN);
        for block in resident {
            world.residency.set_resident(block, true);
            world.residency.set_surface_generation(block, 4);
        }
        world.residency.revise_residency();
        let World {
            residency,
            layer,
            backend,
            jobs,
            ..
        } = &mut world;
        residency.reset(settings(), layer, backend, jobs);
        world
    }

    // One frame: the residency, then (completing `pick`ed jobs) an unbudgeted install pass and
    // its events, as the stream node runs them.
    fn frame(&mut self, eye: [f32; 3], pick: impl FnMut(usize) -> bool) -> ResidencyWork {
        self.layer.set_camera(Some(eye));
        self.residency
            .frame(Some(eye), &mut self.layer, &mut self.backend, &mut self.jobs);
        let work = self.residency.work();
        self.jobs.complete(&mut self.layer, pick);
        self.layer.refresh(&mut self.backend);
        let mut budget = FrameBudget::new(STREAM_BUDGET_US);
        self.layer
            .install(&mut self.backend, &mut budget, &self.clock, &|_, _| true, true);
        self.residency
            .take_installs(&mut self.layer, &mut self.backend);
        self.layer.take_spent().for_each(drop);
        check_one_tier(&self.layer, &self.backend);
        work
    }

    // A frame with the prepared set (`RENDER-32-N`): the residency, then its prepare at `view`;
    // with `complete`, every finished job goes through `take_prepared` as the stream node routes
    // them (a prepare's product is held, any other delivered).
    fn frame_prepared(&mut self, view: CameraSample, complete: bool) {
        self.layer.set_camera(Some(view.eye));
        self.residency
            .frame(Some(view.eye), &mut self.layer, &mut self.backend, &mut self.jobs);
        self.residency.prepare(Some(view), &mut self.jobs);
        if complete {
            let finished: Vec<_> = self.jobs.out.drain(..).collect();
            for (serial, order) in finished {
                let delivery = Delivery {
                    key: order.key,
                    serial,
                    product: product(&order),
                    origin: order.origin,
                    span: order.span,
                };
                if let Some(delivery) = self.residency.take_prepared(delivery) {
                    self.layer.deliver(delivery);
                }
            }
        }
        self.layer.refresh(&mut self.backend);
        let mut budget = FrameBudget::new(STREAM_BUDGET_US);
        self.layer
            .install(&mut self.backend, &mut budget, &self.clock, &|_, _| true, true);
        self.residency
            .take_installs(&mut self.layer, &mut self.backend);
        self.layer.take_spent().for_each(drop);
        self.residency.take_prepared_spent().for_each(drop);
        check_one_tier(&self.layer, &self.backend);
    }

    fn settle(&mut self, eye: [f32; 3]) {
        for _ in 0..64 {
            self.frame(eye, |_| true);
        }
        assert!(!self.residency.has_pending_work(), "settled");
    }

    fn drawn(&self) -> BTreeSet<PatchKey> {
        self.layer
            .installed
            .keys()
            .copied()
            .filter(|key| self.layer.is_drawn(*key))
            .collect()
    }

    fn serial(&self, key: PatchKey) -> Option<u64> {
        self.layer.installed(key).map(|patch| patch.serial)
    }
}

// One grid tier per area: no block draws its coarse patch and a fine patch at once, read from the
// recorder's visible main instances.
fn check_one_tier(layer: &VegetationLayer, backend: &Recorder) {
    let mut coarse = BTreeSet::new();
    let mut fine = BTreeSet::new();
    for (key, patch) in &layer.installed {
        let draws = patch.batches.iter().any(|batch| {
            let main = layer.pool.slot(batch.slot).instance(Role::Main).unwrap();
            backend.instances[&main].visible
        });
        if draws {
            let block = super::super::coarse_of(*key);
            if key[2] == 1 {
                coarse.insert(block);
            } else {
                fine.insert(block);
            }
        }
    }
    let both: Vec<_> = coarse.intersection(&fine).collect();
    assert!(both.is_empty(), "two tiers draw over {both:?}");
}

// The wanted set the script's residency pass gives at `camera` over `resident` blocks.
fn rule_wanted(resident: &BTreeSet<Cell>, camera: [f32; 2]) -> BTreeSet<PatchKey> {
    let settings = settings();
    let set: Set<Cell> = resident.iter().copied().collect();
    let mut out = BTreeSet::new();
    for &block in resident {
        let state = block_state(&settings, TERRAIN_SPAN, &set, block, camera);
        let mut entering = Vec::new();
        diff_keys(
            settings.subdivision,
            block,
            BlockState::default(),
            state,
            &mut Vec::new(),
            &mut entering,
        );
        out.extend(entering);
    }
    out
}

fn eye(x: f32, z: f32) -> [f32; 3] {
    [x, 120.0, z]
}

fn all_blocks() -> BTreeSet<Cell> {
    (0..20).flat_map(|x| (0..20).map(move |z| [x, z])).collect()
}

#[test]
fn the_tiers_and_bands_equal_the_rule_on_random_walks() {
    for seed in 0..3 {
        let mut rng = StdRng::seed_from_u64(seed);
        let mut resident = all_blocks();
        let mut world = World::new(resident.iter().copied());
        let mut camera = [rng.gen_range(-3000.0..3000.0f32), rng.gen_range(-3000.0..3000.0f32)];
        let span = TERRAIN_SPAN / 4.0;
        let mut evaluated_at = None;
        let mut last_cell = None;
        for step in 0..400 {
            let (dx, dz) = match rng.gen_range(0..30) {
                0 => (rng.gen_range(-3000.0..3000.0), rng.gen_range(-3000.0..3000.0)),
                1..=4 => (0.0, 0.0),
                _ => (rng.gen_range(-40.0..40.0), rng.gen_range(-40.0..40.0)),
            };
            camera = [
                (camera[0] + dx).clamp(-5000.0, 5000.0),
                (camera[1] + dz).clamp(-5000.0, 5000.0),
            ];
            // Terrain residency changes now and then.
            let mut revised = false;
            if rng.gen_range(0..25) == 0 {
                let block = [rng.gen_range(0..20), rng.gen_range(0..20)];
                let now = !resident.contains(&block);
                if now {
                    resident.insert(block);
                } else {
                    resident.remove(&block);
                }
                world.residency.set_resident(block, now);
                world.residency.revise_residency();
                revised = true;
            }
            let cell = rules::camera_cell(camera, span);
            if revised || last_cell != Some(cell) {
                evaluated_at = Some(camera);
            }
            last_cell = Some(cell);
            let work = world.frame(eye(camera[0], camera[1]), |_| rng.gen_range(0..3) > 0);
            let wanted: BTreeSet<PatchKey> = world.residency.wanted().collect();
            assert_eq!(wanted, rule_wanted(&resident, evaluated_at.unwrap()), "seed {seed} step {step}");
            if !work.evaluated {
                assert_eq!(work.ring_cells, 0, "no crossing, no ring cell");
            }
            check_bands(&world, camera);
        }
    }
}

// Every drawn patch whose record's bands differ from the rule at `camera` is queued or in flight.
fn check_bands(world: &World, camera: [f32; 2]) {
    let settings = settings();
    for (key, record, standing) in world.residency.records() {
        if standing != Standing::Drawn {
            continue;
        }
        let distance = key_distance(key, TERRAIN_SPAN, WORLD, camera);
        let bands = settings.bands(distance, key_span(key, TERRAIN_SPAN), Some(record.bands));
        if bands != record.bands {
            assert!(
                world.residency.is_queued(key) || world.residency.requested.contains_key(&key),
                "{key:?} out of its bands unseen"
            );
        }
    }
}

#[test]
fn a_still_frame_does_no_work_and_a_steady_walk_grows_no_storage() {
    let mut world = World::new(all_blocks());
    world.settle(eye(10.0, 10.0));
    for _ in 0..4 {
        let work = world.frame(eye(10.0, 10.0), |_| true);
        assert_eq!(
            (work.evaluated, work.ring_cells, work.band_checks, work.stale_checks, work.requests),
            (false, 0, 0, 0, 0)
        );
        assert_eq!(work.buckets, 0);
    }
    // Small moves inside one fine cell: bands are checked as due, nothing else.
    let footprint = |world: &World| {
        let r = &world.residency;
        [
            r.patches.capacity(),
            r.wanted.capacity(),
            r.queue.capacity(),
            r.queued.capacity(),
            r.scratch.capacity(),
            r.cache.capacity(),
        ]
    };
    for step in 0..400 {
        world.frame(eye(12.0 + (step % 7) as f32, 10.0), |_| true);
    }
    let before = (footprint(&world), world.residency.bands_due.footprint());
    for step in 0..40 {
        let work = world.frame(eye(12.0 + (step % 7) as f32, 10.0), |_| true);
        assert!(!work.evaluated);
    }
    assert_eq!((footprint(&world), world.residency.bands_due.footprint()), before);
}

#[test]
fn views_that_differ_only_in_yaw_give_the_same_sets() {
    // The residency takes the eye only; the frustum orders installs and never changes the sets.
    let mut a = World::new(all_blocks());
    let mut b = World::new(all_blocks());
    let mut rng = StdRng::seed_from_u64(9);
    let mut camera = [0.0f32, 0.0];
    for _ in 0..200 {
        camera[0] += rng.gen_range(-60.0..60.0);
        camera[1] += rng.gen_range(-60.0..60.0);
        let e = eye(camera[0], camera[1]);
        for (world, in_view) in [(&mut a, true), (&mut b, false)] {
            world.layer.set_camera(Some(e));
            world
                .residency
                .frame(Some(e), &mut world.layer, &mut world.backend, &mut world.jobs);
            world.jobs.complete(&mut world.layer, |_| true);
            let mut budget = FrameBudget::new(STREAM_BUDGET_US);
            let test = |low: [f32; 3], _: [f32; 3]| (low[0] > camera[0]) == in_view;
            world
                .layer
                .install(&mut world.backend, &mut budget, &world.clock, &test, true);
            world
                .residency
                .take_installs(&mut world.layer, &mut world.backend);
        }
        let wanted = |w: &World| w.residency.wanted().collect::<BTreeSet<_>>();
        assert_eq!(wanted(&a), wanted(&b));
        assert_eq!(a.drawn(), b.drawn());
    }
}

#[test]
fn shuffled_completion_settles_to_the_same_set_and_products() {
    let path: Vec<[f32; 3]> = (0..60).map(|i| eye(i as f32 * 37.0, -(i as f32) * 21.0)).collect();
    let mut digests = Vec::new();
    for seed in 0..4 {
        let mut rng = StdRng::seed_from_u64(seed);
        let mut world = World::new(all_blocks());
        for &e in &path {
            world.frame(e, |_| rng.gen_range(0..4) == 0);
        }
        world.settle(*path.last().unwrap());
        let mut digest = Vec::new();
        for key in world.drawn() {
            let patch = world.layer.installed(key).unwrap();
            for batch in &patch.batches {
                let slot = world.layer.pool.slot(batch.slot);
                let rows = &world.backend.multimeshes[&slot.multimesh].rows;
                digest.push((key, batch.species, batch.lod, batch.count, rows.iter().map(|v| v.to_bits()).collect::<Vec<_>>()));
            }
        }
        let records: BTreeMap<PatchKey, (Record, Standing)> = world
            .residency
            .records()
            .filter(|(_, _, standing)| *standing != Standing::Cached)
            .map(|(key, record, standing)| (key, (record, standing)))
            .collect();
        digests.push((digest, records));
    }
    // The settled sets are equal; a record (and so its product) differs only where its distance
    // at the last camera lies inside a band's hysteresis, whose state is the path's.
    let camera = [path.last().unwrap()[0], path.last().unwrap()[2]];
    let settings = settings();
    for pair in digests.windows(2) {
        let keys = |records: &BTreeMap<PatchKey, (Record, Standing)>| {
            records.keys().copied().collect::<Vec<_>>()
        };
        assert_eq!(keys(&pair[0].1), keys(&pair[1].1));
        let mut differing = BTreeSet::new();
        for (key, (a, standing)) in &pair[0].1 {
            let (b, other) = pair[1].1[key];
            assert_eq!(*standing, other);
            assert_eq!((a.surface, a.vegetation), (b.surface, b.vegetation));
            if a.bands != b.bands {
                let distance = key_distance(*key, TERRAIN_SPAN, WORLD, camera);
                let span = key_span(*key, TERRAIN_SPAN);
                let entered = settings.bands(distance, span, None);
                let kept = settings.bands(distance, span, Some(Bands { understory: true, near_band: true, caster: true }));
                for (x, y, low, high) in [
                    (a.bands.understory, b.bands.understory, entered.understory, kept.understory),
                    (a.bands.near_band, b.bands.near_band, entered.near_band, kept.near_band),
                    (a.bands.caster, b.bands.caster, entered.caster, kept.caster),
                ] {
                    assert!(x == y || low != high, "{key:?} differs outside a hysteresis interval");
                }
                differing.insert(*key);
            } else {
                assert_eq!(a.tree_count, b.tree_count);
            }
        }
        let same = |digest: &Vec<(PatchKey, u8, u8, u32, Vec<u32>)>| {
            digest
                .iter()
                .filter(|entry| !differing.contains(&entry.0))
                .cloned()
                .collect::<Vec<_>>()
        };
        assert_eq!(same(&pair[0].0), same(&pair[1].0), "equal records draw equal products");
    }
}

#[test]
fn a_tier_change_draws_one_tier_and_no_hole() {
    // The camera crosses into a block, which moves from the coarse grid to the fine one; the fine
    // patches install over several frames while the coarse patch still draws.
    let mut world = World::new(all_blocks());
    let far = eye(-1800.0, 0.0);
    world.settle(far);
    let block = [10, 10];
    let coarse = [10, 10, 1];
    assert!(world.drawn().contains(&coarse));
    let near = eye(10.0, 10.0);
    let mut n = 0;
    loop {
        // One finished job a frame.
        world.frame(near, |at| at == 0);
        let fine_drawn = world
            .drawn()
            .iter()
            .filter(|key| key[2] == 4 && super::super::coarse_of(**key) == coarse)
            .count();
        let coarse_drawn = world.drawn().contains(&coarse);
        assert!(coarse_drawn || fine_drawn == 16, "no hole over {block:?}");
        if !coarse_drawn {
            break;
        }
        n += 1;
        assert!(n < 200);
    }
    assert!(n > 2, "the fine patches took several frames");
    // And back: the coarse patch replaces the fine ones in one frame.
    world.settle(far);
    assert!(world.drawn().contains(&coarse));
}

#[test]
fn edits_rebuild_only_the_patches_of_the_edited_block() {
    let near = [20, 20];
    let far = [21, 20];
    let mut world = World::new([near, far]);
    let span = TERRAIN_SPAN as f32;
    let centre = |b: Cell| {
        eye(
            (b[0] as f32 + 0.5) * span - WORLD[0] * 0.5,
            (b[1] as f32 + 0.5) * span - WORLD[1] * 0.5,
        )
    };
    world.settle(centre(near));
    let sub_keys = |b: Cell| -> BTreeSet<PatchKey> {
        (0..16).map(|at| [b[0] * 4 + at / 4, b[1] * 4 + at % 4, 4]).collect()
    };
    assert_eq!(world.drawn(), &sub_keys(near) | &sub_keys(far));
    // A terrain commit at a newer generation.
    world.jobs.requests.clear();
    world.residency.set_surface_generation(near, 9);
    world.settle(centre(near));
    assert_eq!(world.jobs.requests.iter().copied().collect::<BTreeSet<_>>(), sub_keys(near));
    for key in sub_keys(near) {
        assert_eq!(world.residency.patches[&key].surface, 9);
    }
    // No committed payload: no churn.
    world.jobs.requests.clear();
    world.residency.set_surface_generation(far, -1);
    world.settle(centre(near));
    assert!(world.jobs.requests.is_empty());
    // A vegetation edit: only its block, stamped with the revision.
    world.jobs.edit(near);
    world.settle(centre(near));
    assert_eq!(world.jobs.requests.iter().copied().collect::<BTreeSet<_>>(), sub_keys(near));
    for key in sub_keys(near) {
        assert_eq!(world.residency.patches[&key].vegetation, 1);
    }
    for key in sub_keys(far) {
        assert!(!world.residency.is_stale(key));
    }
    // An edit while the request is out stays detectable: revisions are read before the request.
    let probe = [near[0] * 4, near[1] * 4, 4];
    world.residency.queue(probe);
    world.residency.set_surface_generation(near, 10);
    world.frame(centre(near), |_| false);
    world.jobs.edit(near);
    world.frame(centre(near), |_| true);
    world.frame(centre(near), |_| true);
    assert!(world.residency.is_queued(probe) || world.residency.requested.contains_key(&probe) || !world.residency.is_stale(probe));
    world.settle(centre(near));
    assert!(!world.residency.is_stale(probe), "a later request settles the revision");
    // A busy core holds the staleness check until it reads the revisions.
    world.jobs.requests.clear();
    world.jobs.busy_core = true;
    world.jobs.edit(far);
    for _ in 0..4 {
        let work = world.frame(centre(near), |_| true);
        assert_eq!(work.stale_checks, 0);
    }
    world.jobs.busy_core = false;
    world.settle(centre(near));
    assert_eq!(world.jobs.requests.iter().copied().collect::<BTreeSet<_>>(), sub_keys(far));
}

#[test]
fn the_bands_enter_at_their_boundary_and_keep_through_the_hysteresis() {
    let settings = settings();
    let span = TERRAIN_SPAN / 4.0;
    let half = span * HALF_DIAGONAL;
    let slack = BAND_HYSTERESIS_M;
    let near = settings.product.near_m + half;
    let under = 420.0 + half;
    let shadow = settings.shadow_max_m + half;
    for (boundary, rule) in [
        (near, ResidencySettings::near_band as fn(&ResidencySettings, f64, f64, bool) -> bool),
        (under, ResidencySettings::understory),
        (shadow, ResidencySettings::caster),
    ] {
        assert!(rule(&settings, boundary, span, false), "approach enters at the boundary");
        assert!(!rule(&settings, boundary + 1.0, span, false), "entry keeps its distance");
        assert!(rule(&settings, boundary + slack, span, true), "retreat keeps through the slack");
        assert!(!rule(&settings, boundary + slack + 1.0, span, true), "and drops past it");
    }
}

#[test]
fn a_restored_patch_keeps_valid_content_and_rebuilds_only_when_edited() {
    let near = [20, 20];
    let mut world = World::new([near, [21, 20]]);
    let span = TERRAIN_SPAN as f32;
    let centre = eye(
        (near[0] as f32 + 0.5) * span - WORLD[0] * 0.5,
        (near[1] as f32 + 0.5) * span - WORLD[1] * 0.5,
    );
    world.settle(centre);
    let probe = [near[0] * 4, near[1] * 4, 4];
    let serial = world.serial(probe).unwrap();
    assert!(world.residency.patches[&probe].bands.near_band);
    // Out of the drawn set; the camera then stands inside the near band's retention interval and
    // a residency change restores the patch from the cache.
    world.residency.retire(probe, &mut world.layer, &mut world.backend);
    let fine = 127.5f32;
    let probe_centre = [
        probe[0] as f32 * fine - WORLD[0] * 0.5 + fine * 0.5,
        probe[1] as f32 * fine - WORLD[1] * 0.5 + fine * 0.5,
    ];
    let retain = (200.0 + 127.5 * HALF_DIAGONAL + BAND_HYSTERESIS_M * 0.5) as f32;
    let away = eye(probe_centre[0] + retain, probe_centre[1]);
    world.residency.revise_residency();
    world.settle(away);
    assert_eq!(world.serial(probe), Some(serial), "a restored patch in the slack keeps its content");
    assert!(world.residency.patches[&probe].bands.near_band);
    // An obsolete band request does not rebuild.
    world.residency.queue(probe);
    world.settle(away);
    assert_eq!(world.serial(probe), Some(serial));
    // Edited while hidden: rebuilt on restore, with its band history.
    world.residency.retire(probe, &mut world.layer, &mut world.backend);
    world.jobs.edit(near);
    world.residency.revise_residency();
    world.settle(away);
    assert_ne!(world.serial(probe), Some(serial));
    assert!(!world.residency.is_stale(probe));
    assert!(world.residency.patches[&probe].bands.near_band);
}

#[test]
fn a_refused_request_stays_queued_and_progress_resumes() {
    let mut world = World::new(all_blocks());
    world.jobs.full = true;
    for _ in 0..3 {
        world.frame(eye(0.0, 0.0), |_| true);
    }
    assert!(world.residency.has_pending_work());
    assert!(world.drawn().is_empty());
    world.jobs.full = false;
    world.settle(eye(0.0, 0.0));
    assert_eq!(
        world.residency.wanted().collect::<BTreeSet<_>>(),
        world
            .residency
            .records()
            .filter(|(_, _, standing)| *standing == Standing::Drawn)
            .map(|(key, _, _)| key)
            .collect::<BTreeSet<_>>()
    );
}

// The camera on an 800 m orbit about a pivot at the world's centre, pitch -0.1 rad.
fn orbit(angle: f32) -> CameraSample {
    let ground = 800.0 * 0.1f32.cos();
    CameraSample {
        eye: [ground * angle.cos(), 180.0, ground * angle.sin()],
        pivot: [0.0, 100.0, 0.0],
        yaw: angle,
        pitch: -0.1,
        distance: 800.0,
    }
}

#[test]
fn a_turn_draws_products_prepared_along_the_orbit_and_settles_to_the_same_set() {
    let mut ahead = World::new(all_blocks());
    let mut plain = World::new(all_blocks());
    let start = orbit(0.0);
    ahead.settle(start.eye);
    plain.settle(start.eye);
    // Still frames: the prepared world prepares its reach; nothing it holds draws.
    for _ in 0..240 {
        ahead.frame_prepared(start, true);
    }
    let [held, bytes, ..] = ahead.residency.prepared_counts();
    assert!(held > 0 && bytes > 0, "products held along the reach");
    assert_eq!(ahead.drawn(), plain.drawn(), "nothing prepared is drawn");
    assert!(!ahead.residency.has_pending_work(), "preparing is not pending work");
    // A half turn over 12 frames with no job finishing: only prepared products can draw.
    for step in 1..=12 {
        let view = orbit(std::f32::consts::PI * step as f32 / 12.0);
        ahead.frame_prepared(view, false);
        plain.frame(view.eye, |_| false);
    }
    let drawn_wanted = |world: &World| {
        let wanted: BTreeSet<PatchKey> = world.residency.wanted().collect();
        world.drawn().intersection(&wanted).count()
    };
    let [_, _, _, over_cap, used, ..] = ahead.residency.prepared_counts();
    assert!(used > 0 && over_cap == 0);
    assert!(drawn_wanted(&ahead) > drawn_wanted(&plain), "prepared patches drew without a job");
    // Settled, both draw the same patches from records of the same generations.
    let end = orbit(std::f32::consts::PI);
    for _ in 0..64 {
        ahead.frame_prepared(end, true);
    }
    plain.settle(end.eye);
    assert!(!ahead.residency.has_pending_work());
    assert_eq!(ahead.drawn(), plain.drawn());
    let records = |world: &World| -> BTreeMap<PatchKey, (i64, i64, Standing)> {
        world
            .residency
            .records()
            .filter(|(_, _, standing)| *standing != Standing::Cached)
            .map(|(key, record, standing)| (key, (record.surface, record.vegetation, standing)))
            .collect()
    };
    assert_eq!(records(&ahead), records(&plain));
}
