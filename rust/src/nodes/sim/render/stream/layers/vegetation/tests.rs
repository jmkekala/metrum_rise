// SPDX-License-Identifier: GPL-2.0-only

//! The vegetation layer against the recording backend, without Godot (`docs/terrain.md`
//! "Streaming core" 5.5): one version drawn (invariant 1), the budget and the order (2), the
//! caps and pool exhaustion (6), no allocation on a steady frame (7), the command digest of a
//! scripted sequence, and the camera's choices inside a patch.

use super::*;
use crate::nodes::sim::render::stream::backend::recorder::Recorder;
use crate::nodes::sim::render::stream::backend::{Handle, Scenario};
use crate::nodes::sim::render::stream::schedule::STREAM_BUDGET_US;
use crate::nodes::sim::render::vegetation_product::{
    PLACEMENT_STRIDE, ProductCatalogue, ProductInputs, ProductSettings, build_product,
};
use std::cell::Cell;
use std::collections::BTreeSet;

const RULES: VegetationRules = VegetationRules {
    near_detail_m: 45.0,
    detail_fade_m: 10.0,
    shadow_handover_m: 60.0,
    landscape_first_variant: 6,
    hedge_first_variant: 12,
};

const SETTINGS: ProductSettings = ProductSettings {
    density: 1.0,
    near_m: 200.0,
    crossfade_begin_m: 150.0,
    far_m: 4500.0,
    bush_range_m: 420.0,
    rock_range_m: 420.0,
};

const SPAN: f32 = 128.0;

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

// `count` plants on a grid of the patch, every species, a few garden plants among the bushes.
fn placements(count: usize, seed: usize) -> Vec<f32> {
    let mut data = vec![0.0f32; count * PLACEMENT_STRIDE];
    for (i, record) in data
        .as_chunks_mut::<PLACEMENT_STRIDE>()
        .0
        .iter_mut()
        .enumerate()
    {
        let n = i + seed * 7;
        record[0] = (n % 16) as f32 * 7.5 + 1.0;
        record[1] = (n % 5) as f32 * 0.5;
        record[2] = (n / 16 % 16) as f32 * 7.5 + 1.0;
        record[3] = (n % 31) as f32 * 0.2;
        record[4] = 0.8 + (n % 17) as f32 * 0.025;
        record[5] = [0.0, 1.0, 2.0, 2.0, 3.0, 0.0, 1.0, 2.0][n % 8];
        if n % 23 == 0 {
            // A garden plant pinned by the brush (variant 7, biased by one).
            record[5] = (8 << 2 | 2) as f32;
        }
    }
    data
}

fn product(count: usize, seed: usize, near_band: bool, caster: bool) -> VegetationProduct {
    let data = placements(count, seed);
    let inputs = ProductInputs {
        placements: &data,
        cover: None,
        runs: &[],
        origin: [0.0, 0.0],
        near_band,
        caster,
        settings: SETTINGS,
    };
    let mut out = VegetationProduct::default();
    build_product(&inputs, &catalogue(), &mut out);
    out
}

fn delivery(key: PatchKey, serial: u64, product: VegetationProduct) -> Delivery {
    Delivery {
        key,
        serial,
        product,
        origin: [key[0] as f32 * SPAN, key[1] as f32 * SPAN],
        span: SPAN,
    }
}

// A clock that advances `step` microseconds on every read: every unit measures `step`.
struct StepClock {
    now: Cell<f64>,
    step: f64,
}

impl StepClock {
    fn new(step: f64) -> Self {
        Self {
            now: Cell::new(0.0),
            step,
        }
    }
}

impl Clock for StepClock {
    fn now_us(&self) -> f64 {
        let now = self.now.get();
        self.now.set(now + self.step);
        now
    }
}

fn layer(limit: usize, spares: usize) -> VegetationLayer {
    let mut layer = VegetationLayer::new(limit, spares);
    layer.set_rules(RULES);
    layer.set_camera(Some([0.0, 30.0, 0.0]));
    layer
}

fn nowhere(_: [f32; 3], _: [f32; 3]) -> bool {
    false
}

// Invariant 1: the main-scenario instances drawn are exactly those of the shown patches' installed
// versions (no old version, no stray instance), each at its patch's origin; the reflection draws
// exactly their impostors.
fn check_one_version(layer: &VegetationLayer, backend: &Recorder) {
    let mut expected = BTreeSet::new();
    let mut reflected = BTreeSet::new();
    for patch in layer.installed.values() {
        if !(layer.enabled && patch.visible) {
            continue;
        }
        for batch in &patch.batches {
            let slot = layer.pool.slot(batch.slot);
            let main = slot.instance(Role::Main).unwrap();
            expected.insert(main);
            let origin = backend.instances[&main].origin;
            assert_eq!(
                origin,
                [patch.origin[0], 0.0, patch.origin[1]],
                "an instance at its patch"
            );
            if batch.caster && slot.drawn(Role::Caster).unwrap().visible {
                expected.insert(slot.instance(Role::Caster).unwrap());
            }
            if batch.lod == LOD_IMPOSTOR {
                reflected.insert(slot.instance(Role::Reflection).unwrap());
            }
        }
    }
    let visible: BTreeSet<Handle> = backend
        .visible(Scenario::Main)
        .map(|(&handle, _)| handle)
        .collect();
    assert_eq!(visible, expected, "exactly the installed versions draw");
    let mirrored: BTreeSet<Handle> = backend
        .visible(Scenario::Reflection)
        .map(|(&handle, _)| handle)
        .collect();
    assert_eq!(
        mirrored, reflected,
        "the reflection draws the shown impostors"
    );
}

// Invariant 2 for one frame: past its first unit the frame's predicted spend fits the budget.
fn check_budget(budget: &FrameBudget) {
    if budget.units > 1 {
        assert!(budget.predicted_us <= STREAM_BUDGET_US, "{budget:?}");
    }
}

// One frame: a pass with the budget, the invariants, and the installs' keys in order.
fn frame(
    layer: &mut VegetationLayer,
    backend: &mut Recorder,
    clock: &StepClock,
    in_view: &impl Fn([f32; 3], [f32; 3]) -> bool,
) -> Vec<PatchKey> {
    let mut budget = FrameBudget::new(STREAM_BUDGET_US);
    layer.refresh(backend);
    layer.install(backend, &mut budget, clock, in_view, false);
    check_budget(&budget);
    check_one_version(layer, backend);
    layer.take_spent().for_each(drop);
    let keys = layer
        .events()
        .chunks_exact(EVENT_LANES)
        .map(|event| [event[0] as i32, event[1] as i32, event[2] as i32])
        .collect();
    layer.clear_events();
    keys
}

// A scripted sequence touching every operation; returns the recorder's digest.
fn scripted_sequence() -> (u64, usize) {
    let mut backend = Recorder::new();
    let mut layer = layer(256, 64);
    let clock = StepClock::new(500.0);
    let keys: Vec<PatchKey> = (0..6).map(|i| [i % 3, i / 3, 4]).collect();
    for (serial, &key) in keys.iter().enumerate() {
        layer.deliver(delivery(
            key,
            serial as u64 + 1,
            product(40 + 10 * serial, serial, true, true),
        ));
    }
    let mut drawn = BTreeSet::new();
    let mut frames = 0;
    while layer.pending() > 0 {
        for key in frame(&mut layer, &mut backend, &clock, &nowhere) {
            drawn.insert(key);
        }
        frames += 1;
        for &key in &drawn {
            assert!(layer.is_drawn(key), "a patch once drawn is never a hole");
        }
    }
    // Upgrades: the new version replaces the old in one unit, never both and never none.
    layer.deliver(delivery(keys[0], 10, product(30, 9, false, false)));
    layer.deliver(delivery(keys[1], 11, product(90, 3, true, false)));
    // A superseded product never installs.
    layer.deliver(delivery(keys[2], 12, product(20, 1, true, true)));
    layer.forget(keys[2]);
    while layer.pending() > 0 {
        frame(&mut layer, &mut backend, &clock, &nowhere);
        frames += 1;
        for &key in &drawn {
            assert!(layer.is_drawn(key));
        }
    }
    assert_eq!(layer.installed(keys[0]).unwrap().serial, 10);
    assert_eq!(layer.installed(keys[2]).unwrap().serial, 3);
    // Hidden, shown, the camera moved, the toggles, freed.
    layer.set_visible(&mut backend, keys[3], false);
    check_one_version(&layer, &backend);
    layer.set_camera(Some([200.0, 30.0, 40.0]));
    frame(&mut layer, &mut backend, &clock, &nowhere);
    layer.set_visible(&mut backend, keys[3], true);
    check_one_version(&layer, &backend);
    layer.set_cast_shadows(&mut backend, false);
    check_one_version(&layer, &backend);
    layer.set_cast_shadows(&mut backend, true);
    layer.set_enabled(&mut backend, false);
    check_one_version(&layer, &backend);
    assert_eq!(backend.visible(Scenario::Main).count(), 0);
    layer.set_enabled(&mut backend, true);
    check_one_version(&layer, &backend);
    layer.free(&mut backend, keys[4]);
    check_one_version(&layer, &backend);
    layer.drop_reflections(&mut backend);
    layer.redraw_reflections(&mut backend);
    check_one_version(&layer, &backend);
    layer.clear(&mut backend);
    assert!(backend.instances.is_empty() && backend.multimeshes.is_empty());
    (backend.digest(), frames)
}

#[test]
fn a_scripted_sequence_draws_one_version_and_its_command_digest_is_pinned() {
    let (digest, frames) = scripted_sequence();
    assert_eq!(
        scripted_sequence().0,
        digest,
        "the commands are a function of the inputs"
    );
    assert!(
        frames >= 3,
        "the budget spread the installs over frames: {frames}"
    );
    // The command stream of the sequence (a change to what the layer sends changes it).
    assert_eq!(digest, GOLDEN_DIGEST, "digest {digest:#018x}");
}

const GOLDEN_DIGEST: u64 = 0x3ca8_9537_c16b_ddd5;

#[test]
fn installs_take_class_then_distance_then_key_order_within_the_budget() {
    let mut backend = Recorder::new();
    let mut layer = layer(1024, 64);
    let clock = StepClock::new(700.0);
    // Holes at several distances; patch [5, 0] and [6, 0] are in the view; [0, 1] is drawn already
    // and gets an upgrade; [3, 3] is on the coarse grid.
    layer.deliver(delivery([0, 1, 4], 1, product(10, 0, true, true)));
    let mut budget = FrameBudget::new(STREAM_BUDGET_US);
    layer.install(&mut backend, &mut budget, &clock, &nowhere, true);
    layer.clear_events();
    let keys: [PatchKey; 7] = [
        [0, 1, 4],
        [2, 0, 4],
        [5, 0, 4],
        [1, 0, 4],
        [6, 0, 4],
        [3, 3, 1],
        [0, 2, 4],
    ];
    for (serial, &key) in keys.iter().enumerate() {
        layer.deliver(delivery(
            key,
            serial as u64 + 2,
            product(12, serial, true, true),
        ));
    }
    let in_view = |low: [f32; 3], _: [f32; 3]| low[0] >= 5.0 * SPAN && low[2] < SPAN;
    let mut order = Vec::new();
    let mut frames = 0;
    while layer.pending() > 0 {
        let installed = frame(&mut layer, &mut backend, &clock, &in_view);
        assert!(!installed.is_empty(), "every frame makes progress");
        assert!(
            installed.len() <= 3,
            "a 700 us unit leaves room for three: {installed:?}"
        );
        order.extend(installed);
        frames += 1;
    }
    assert!(frames >= 3);
    assert_eq!(
        order,
        // Holes in view nearest first, holes in the near ring nearest first (then by key), the
        // coarse hole, then the upgrade.
        vec![
            [5, 0, 4],
            [6, 0, 4],
            [1, 0, 4],
            [0, 2, 4],
            [2, 0, 4],
            [3, 3, 1],
            [0, 1, 4]
        ]
    );
}

#[test]
fn the_pool_limit_blocks_the_farthest_and_frees_resume_it() {
    let mut backend = Recorder::new();
    let one = product(64, 0, true, true);
    let slots = one.batches.iter().filter(|batch| !batch.proxy).count();
    // Room for two patches and a half.
    let mut layer = layer(slots * 5 / 2, slots);
    let clock = StepClock::new(1.0);
    for (serial, x) in [3, 1, 2].into_iter().enumerate() {
        layer.deliver(delivery(
            [x, 0, 4],
            serial as u64 + 1,
            product(64, 0, true, true),
        ));
    }
    let installed = frame(&mut layer, &mut backend, &clock, &nowhere);
    assert_eq!(
        installed,
        vec![[1, 0, 4], [2, 0, 4]],
        "the nearest fit; the farthest waits"
    );
    assert_eq!(layer.status().blocked, 1);
    assert!(layer.pool.live() + layer.pool.spares() <= slots * 5 / 2);
    assert!(
        frame(&mut layer, &mut backend, &clock, &nowhere).is_empty(),
        "still no room"
    );
    // A replacement of an installed patch fits in its own slots, past the blocked hole.
    layer.deliver(delivery([1, 0, 4], 9, product(64, 1, true, true)));
    let installed = frame(&mut layer, &mut backend, &clock, &nowhere);
    assert_eq!(installed, vec![[1, 0, 4]]);
    assert_eq!(layer.status().blocked, 1);
    layer.free(&mut backend, [2, 0, 4]);
    let installed = frame(&mut layer, &mut backend, &clock, &nowhere);
    assert_eq!(installed, vec![[3, 0, 4]], "a free makes room");
    assert_eq!(layer.status().blocked, 0);
    assert!(layer.pool.live() <= slots * 5 / 2 && layer.pool.spares() <= slots);
}

#[test]
fn a_steady_frame_grows_no_storage() {
    // A backend that counts calls and keeps nothing, so only the layer's own storage is measured.
    #[derive(Default)]
    struct Tally {
        calls: u64,
        next: Handle,
    }
    impl RenderBackend for Tally {
        fn multimesh_create(&mut self, _: u32, _: bool, _: bool) -> Handle {
            self.next += 1;
            self.next
        }
        fn multimesh_reallocate(&mut self, _: Handle, _: u32, _: bool, _: bool) {
            self.calls += 1;
        }
        fn multimesh_set_mesh(&mut self, _: Handle, _: u32) {
            self.calls += 1;
        }
        fn multimesh_set_buffer(&mut self, _: Handle, _: &[f32], _: usize, _: u32) {
            self.calls += 1;
        }
        fn multimesh_set_visible_instances(&mut self, _: Handle, _: u32) {
            self.calls += 1;
        }
        fn multimesh_set_custom_aabb(&mut self, _: Handle, _: Aabb) {
            self.calls += 1;
        }
        fn instance_create(&mut self, _: Handle, _: Scenario) -> Option<Handle> {
            self.next += 1;
            Some(self.next)
        }
        fn instance_set_origin(&mut self, _: Handle, _: [f32; 3]) {
            self.calls += 1;
        }
        fn instance_set_visible(&mut self, _: Handle, _: bool) {
            self.calls += 1;
        }
        fn instance_set_shadow(&mut self, _: Handle, _: Shadow) {
            self.calls += 1;
        }
        fn instance_set_range(&mut self, _: Handle, _: [f32; 2]) {
            self.calls += 1;
        }
        fn instance_set_custom_aabb(&mut self, _: Handle, _: Aabb) {
            self.calls += 1;
        }
        fn instance_set_material(&mut self, _: Handle, _: Option<u32>) {
            self.calls += 1;
        }
        fn instance_set_parameter(&mut self, _: Handle, _: u32, _: f32) {
            self.calls += 1;
        }
        fn free(&mut self, _: Handle) {
            self.calls += 1;
        }
    }
    let mut backend = Tally::default();
    let mut layer = layer(512, 256);
    let clock = StepClock::new(1.0);
    let products: Vec<VegetationProduct> = (0..44)
        .map(|seed| product(80, seed % 2, true, true))
        .collect();
    let run =
        |layer: &mut VegetationLayer, backend: &mut Tally, product: Option<VegetationProduct>| {
            if let Some(product) = product {
                layer.deliver(delivery([0, 0, 4], 1, product));
            }
            let mut budget = FrameBudget::new(STREAM_BUDGET_US);
            layer.refresh(backend);
            layer.install(backend, &mut budget, &clock, &nowhere, false);
            layer.take_spent().for_each(drop);
            layer.clear_events();
        };
    let mut products = products.into_iter();
    // Warm up until the spares' bookkeeping has met its compaction bound: twenty installs of each
    // shape, and a camera move each way.
    for _ in 0..20 {
        run(&mut layer, &mut backend, products.next());
        run(&mut layer, &mut backend, products.next());
    }
    layer.set_camera(Some([10.0, 30.0, 10.0]));
    run(&mut layer, &mut backend, None);
    layer.set_camera(Some([0.0, 30.0, 0.0]));
    run(&mut layer, &mut backend, None);
    let footprint = layer.footprint();
    // An idle frame.
    let calls = backend.calls;
    run(&mut layer, &mut backend, None);
    assert_eq!(backend.calls, calls, "an idle frame sends nothing");
    assert_eq!(layer.footprint(), footprint);
    // A frame the camera moved, and a replacement of the same shape.
    layer.set_camera(Some([10.0, 30.0, 10.0]));
    run(&mut layer, &mut backend, None);
    run(&mut layer, &mut backend, products.next());
    assert_eq!(
        layer.footprint(),
        footprint,
        "steady frames reuse the layer's storage"
    );
}

#[test]
fn the_near_detail_rule_matches_the_renderers_boundaries() {
    let trees = Aabb::new([10.0, 0.0, 10.0], [40.0, 12.0, 30.0]);
    let band_end = RULES.near_detail_m + RULES.detail_fade_m;
    let tiny = Aabb::new(trees.center(), [1.0; 3]);
    let above = tiny.center();
    assert_eq!(
        near_detail(&RULES, tiny, Some([above[0], above[1] + 10.0, above[2]])),
        DETAIL_FULL
    );
    let end = [50.0f32, 12.0, 40.0];
    assert_eq!(
        near_detail(
            &RULES,
            trees,
            Some([end[0] + band_end as f32, end[1], end[2]])
        ),
        DETAIL_REDUCED
    );
    assert_eq!(
        near_detail(
            &RULES,
            trees,
            Some([end[0] + band_end as f32 - 1.0, end[1], end[2]])
        ),
        DETAIL_BLEND
    );
    assert_eq!(near_detail(&RULES, trees, None), DETAIL_FULL);
}

#[test]
fn the_camera_moves_the_near_level_and_the_branched_shadow() {
    let mut backend = Recorder::new();
    let mut layer = layer(256, 64);
    let clock = StepClock::new(1.0);
    layer.deliver(delivery([0, 0, 4], 1, product(64, 0, true, true)));
    frame(&mut layer, &mut backend, &clock, &nowhere);
    let near_canopy = |layer: &VegetationLayer| -> Vec<SlotId> {
        let patch = layer.installed([0, 0, 4]).unwrap();
        patch
            .batches
            .iter()
            .filter(|batch| batch.lod == LOD_NEAR && batch.species < BUSH)
            .map(|batch| batch.slot)
            .collect()
    };
    let patch = layer.installed([0, 0, 4]).unwrap();
    assert!(patch.near_canopy && patch.has_caster);
    let trees = patch.canopy_box;
    // Far above: the reduced level, no branched shadow.
    let far = [trees.center()[0], 2000.0, trees.center()[2]];
    layer.set_camera(Some(far));
    frame(&mut layer, &mut backend, &clock, &nowhere);
    let patch = layer.installed([0, 0, 4]).unwrap();
    assert_eq!((patch.detail, patch.branched), (DETAIL_REDUCED, false));
    for slot in near_canopy(&layer) {
        let slot = layer.pool.slot(slot);
        assert_eq!(slot.kind().mesh % NEAR_LEVELS, u32::from(DETAIL_REDUCED));
        let main = slot.instance(Role::Main).unwrap();
        assert_eq!(backend.instances[&main].shadow, Shadow::Off);
        assert_eq!(
            backend.multimeshes[&slot.multimesh].mesh,
            Some(slot.kind().mesh)
        );
    }
    // Inside the box: both levels, and the branched canopy casts.
    layer.set_camera(Some(trees.center()));
    frame(&mut layer, &mut backend, &clock, &nowhere);
    let patch = layer.installed([0, 0, 4]).unwrap();
    assert_eq!((patch.detail, patch.branched), (DETAIL_BLEND, true));
    for slot in near_canopy(&layer) {
        let main = layer.pool.slot(slot).instance(Role::Main).unwrap();
        assert_eq!(backend.instances[&main].shadow, Shadow::On);
    }
    // The toggle silences the branched canopy and the casters, which hide.
    layer.set_cast_shadows(&mut backend, false);
    assert!(
        backend
            .instances
            .values()
            .all(|instance| instance.shadow == Shadow::Off)
    );
    check_one_version(&layer, &backend);
    layer.set_cast_shadows(&mut backend, true);
    for slot in near_canopy(&layer) {
        let main = layer.pool.slot(slot).instance(Role::Main).unwrap();
        assert_eq!(backend.instances[&main].shadow, Shadow::On);
    }
    // A still camera visits nothing.
    assert_eq!(layer.refresh(&mut backend), 0);
}

#[test]
fn a_patch_draws_what_its_product_names() {
    let mut backend = Recorder::new();
    let mut layer = layer(256, 64);
    let clock = StepClock::new(1.0);
    let product = product(64, 0, true, true);
    let expected = product.clone();
    layer.deliver(delivery([2, 1, 4], 7, product));
    frame(&mut layer, &mut backend, &clock, &nowhere);
    let patch = layer.installed([2, 1, 4]).unwrap();
    let mut batches = patch.batches.iter();
    for batch in &expected.batches {
        if batch.proxy {
            continue;
        }
        let installed = batches.next().unwrap();
        let slot = layer.pool.slot(installed.slot);
        let multimesh = &backend.multimeshes[&slot.multimesh];
        assert_eq!(
            multimesh.rows,
            expected.buffer(batch.buffer),
            "the product's buffer, bit for bit"
        );
        assert_eq!(
            multimesh.visible_count.unwrap_or(multimesh.capacity),
            batch.count
        );
        assert_eq!(
            (multimesh.colors, multimesh.custom),
            (batch.colors, batch.custom)
        );
        let main = &backend.instances[&slot.instance(Role::Main).unwrap()];
        assert_eq!(main.range, batch.range);
        assert_eq!(main.aabb, batch.shared_bounds);
        let canopy_near = batch.lod == LOD_NEAR && batch.species < BUSH;
        assert_eq!(
            main.parameters,
            if canopy_near {
                vec![(HANDS_OVER, 1.0)]
            } else {
                vec![]
            }
        );
        if batch.lod == LOD_IMPOSTOR {
            assert_eq!(multimesh.aabb, batch.mesh_bounds);
            assert_eq!(main.material, Some(u32::from(batch.species)));
            assert_eq!(main.shadow, Shadow::Off);
            assert!(installed.caster);
            let caster = &backend.instances[&slot.instance(Role::Caster).unwrap()];
            assert_eq!(
                (caster.shadow, caster.material),
                (Shadow::ShadowsOnly, Some(2 + u32::from(batch.species)))
            );
            let reflection = &backend.instances[&slot.instance(Role::Reflection).unwrap()];
            assert_eq!(
                (reflection.material, reflection.range, reflection.aabb),
                (main.material, [0.0; 2], main.aabb)
            );
        } else {
            assert_eq!(main.material, None);
            let casts = batch.species == ROCK
                || (batch.species == BUSH && batch.variant >= RULES.landscape_first_variant);
            if !canopy_near {
                assert_eq!(main.shadow, if casts { Shadow::On } else { Shadow::Off });
            }
        }
    }
    assert!(batches.next().is_none());
}
