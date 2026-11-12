// SPDX-License-Identifier: GPL-2.0-only

//! Vegetation patches drawn through the streaming backend (`RENDER-32-E`, `docs/terrain.md`
//! "Streaming core"): a prepared patch product (`RENDER-32-C`/`-D`) installs as one unit, every
//! batch into a pooled slot, and draws exactly what `vegetation.gd`'s patch nodes drew: the
//! same buffers, meshes, materials, bounds, ranges, shadow settings and the canopy's
//! `hands_over` parameter, at the patch origin; impostors also in the water reflection's world,
//! as `water_reflection.gd` mirrored them; each impostor's shadow caster as a second instance on
//! the impostor's own `MultiMesh` (the node path gave it a copy of the same buffer).
//!
//! `vegetation.gd` keeps the wanted set, the bands and the staleness (round V2b moves them): it
//! requests, forgets, shows, hides and frees patches by key, and learns of installs from the
//! events this layer emits. The layer owns what follows the camera inside a patch: the near
//! canopy's detail level (the mesh on the buffer already written) and its branched shadow.
//!
//! One version of a patch draws at a time: an install hides and releases the patch's previous
//! version in the same unit that shows the new one, so neither a hole nor two overlapping
//! versions are ever drawn. Costs: an install is `O(batches + instances)` (the buffer copy);
//! show, hide and free `O(batches)`; the camera refresh `O(installed patches)` on frames the
//! camera moved and zero otherwise; nothing allocates once the storage has grown.

use super::super::backend::{
    Drawn, RenderBackend, Role, Shadow, SlotId, SlotKind, SlotPool, capacity_class,
};
use super::super::schedule::{Class, Clock, FrameBudget, predicted_us};
use super::super::status::LayerStatus;
use crate::nodes::sim::render::vegetation_product::godot_math::Aabb;
use crate::nodes::sim::render::vegetation_product::{
    Batch, LOD_HEDGE_RUN, LOD_IMPOSTOR, LOD_NEAR, VARIANT_COUNTS, VegetationProduct,
};
use std::collections::HashMap;

type Map<K, V> = HashMap<K, V, foldhash::fast::FixedState>;

/// A vegetation patch key: grid column, row and subdivision (`vegetation.gd`'s `Vector3i`).
pub(crate) type PatchKey = [i32; 3];

const BUSH: u8 = 2;
const ROCK: u8 = 3;
/// Near levels registered per variant (full, reduced, both; one level species repeat theirs).
pub(crate) const NEAR_LEVELS: u32 = 3;
const NEAR_BUCKETS: u32 =
    (VARIANT_COUNTS[0] + VARIANT_COUNTS[1] + VARIANT_COUNTS[2] + VARIANT_COUNTS[3]) as u32;
/// Mesh index of the impostor quad; the near meshes come first, `bucket x NEAR_LEVELS + level`.
pub(crate) const IMPOSTOR_MESH: u32 = NEAR_BUCKETS * NEAR_LEVELS;
/// Mesh index of the first distant hedge box.
pub(crate) const HEDGE_RUN_MESH: u32 = IMPOSTOR_MESH + 1;
/// Meshes the catalogue registers.
pub(crate) const MESH_COUNT: usize = HEDGE_RUN_MESH as usize + 3;
/// Materials the catalogue registers: the impostor material per canopy species, then its shadow
/// caster material.
pub(crate) const MATERIAL_COUNT: usize = 4;
const SHADOW_MATERIAL: u32 = 2;
/// The instance parameter index of `hands_over`.
pub(crate) const HANDS_OVER: u32 = 0;
/// The near canopy's detail levels (`vegetation.gd`'s `DETAIL_*`).
pub(crate) const DETAIL_FULL: u8 = 0;
const DETAIL_REDUCED: u8 = 1;
const DETAIL_BLEND: u8 = 2;
// Install events: key, serial, plants drawn (-1: the core was busy, ask again), install time.
const EVENT_LANES: usize = 6;
// Predicted backend calls of one batch's install (fill, draw, the reflection or caster).
const CALLS_PER_BATCH: usize = 12;

/// The renderer's draw rules the layer applies (`vegetation.gd`, `tree_species.gd`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct VegetationRules {
    /// Where the near canopy hands over to its reduced level (`near_detail_m`).
    pub(crate) near_detail_m: f64,
    /// The width of that handover (`TREE_DETAIL_FADE_M`).
    pub(crate) detail_fade_m: f64,
    /// Within this of the camera the branched canopy casts (`TREE_SHADOW_HANDOVER_M`).
    pub(crate) shadow_handover_m: f64,
    /// The first garden plant variant of the bush species, which casts (`LANDSCAPE_FIRST_VARIANT`).
    pub(crate) landscape_first_variant: u8,
    /// The first clipped hedge variant (`HEDGE_FIRST_VARIANT`).
    pub(crate) hedge_first_variant: u8,
}

impl VegetationRules {
    /// The bridge's packing: near detail, detail fade, shadow handover, first landscape variant,
    /// first hedge variant.
    pub(crate) fn from_lanes(lanes: &[f64]) -> Option<Self> {
        let [
            near_detail_m,
            detail_fade_m,
            shadow_handover_m,
            landscape,
            hedge,
        ] = *lanes.first_chunk::<5>()?;
        Some(Self {
            near_detail_m,
            detail_fade_m,
            shadow_handover_m,
            landscape_first_variant: landscape as u8,
            hedge_first_variant: hedge as u8,
        })
    }
}

/// A prepared product with where its patch stands.
pub(crate) struct Delivery {
    /// The patch.
    pub(crate) key: PatchKey,
    /// Its request serial.
    pub(crate) serial: u64,
    /// The product.
    pub(crate) product: VegetationProduct,
    /// The patch's minimum corner, world metres.
    pub(crate) origin: [f32; 2],
    /// The patch's side, metres.
    pub(crate) span: f32,
}

// A delivered product waiting for its install, with its place in the order.
struct Pending {
    delivery: Delivery,
    class: Class,
    distance_sq: f64,
    // The heights its batches' bounds reach, for the frustum test.
    heights: [f32; 2],
    installed: bool,
}

/// One batch of an installed patch.
#[derive(Clone, Copy, Debug)]
pub(crate) struct InstalledBatch {
    /// The batch's slot.
    pub(crate) slot: SlotId,
    /// Species, level and variant of the batch.
    pub(crate) species: u8,
    /// Level code.
    pub(crate) lod: u8,
    /// Variant.
    pub(crate) variant: u8,
    /// Instances drawn.
    pub(crate) count: u32,
    /// Whether the slot's caster instance is the batch's shadow caster.
    pub(crate) caster: bool,
}

/// An installed version of a patch.
pub(crate) struct Installed {
    /// The request it came from.
    pub(crate) serial: u64,
    /// Shown (`vegetation.gd` hides a patch it keeps for a camera moving back).
    pub(crate) visible: bool,
    /// Its batches in the product's order.
    pub(crate) batches: Vec<InstalledBatch>,
    /// World box of the canopy tree origins.
    pub(crate) canopy_box: Aabb,
    /// Whether the patch has a shadow caster.
    pub(crate) has_caster: bool,
    /// Whether it holds near canopy batches.
    pub(crate) near_canopy: bool,
    /// The near canopy's level.
    pub(crate) detail: u8,
    /// Whether its branched canopy casts.
    pub(crate) branched: bool,
    origin: [f32; 2],
}

/// The vegetation layer: installed patches, the products waiting and the slots they draw with.
pub(crate) struct VegetationLayer {
    rules: Option<VegetationRules>,
    pool: SlotPool,
    installed: Map<PatchKey, Installed>,
    pending: Vec<Pending>,
    // Products installed or dropped, for their storage to go back to the jobs.
    spent: Vec<VegetationProduct>,
    spare_batches: Vec<Vec<InstalledBatch>>,
    events: Vec<i64>,
    enabled: bool,
    cast_shadows: bool,
    camera: Option<[f32; 3]>,
    refreshed: Option<[f32; 3]>,
    status: LayerStatus,
}

/// Slots the vegetation may hold. Each canopy slot takes instance uniform slots of the engine's
/// global buffer (16 per instance, 65,536 in all): 3,072 slots leave a quarter of it to others.
pub(crate) const SLOT_LIMIT: usize = 3072;
/// Spare slots kept for reuse.
pub(crate) const SPARE_LIMIT: usize = 768;

impl VegetationLayer {
    /// An empty layer whose pool holds at most `limit` slots, `spare_limit` of them spare.
    pub(crate) fn new(limit: usize, spare_limit: usize) -> Self {
        Self {
            rules: None,
            pool: SlotPool::new(limit, spare_limit),
            installed: Map::default(),
            pending: Vec::new(),
            spent: Vec::new(),
            spare_batches: Vec::new(),
            events: Vec::new(),
            enabled: true,
            cast_shadows: true,
            camera: None,
            refreshed: None,
            status: LayerStatus::default(),
        }
    }

    /// Sets the draw rules; nothing installs before them.
    pub(crate) fn set_rules(&mut self, rules: VegetationRules) {
        self.rules = Some(rules);
    }

    /// Gives reused slots fresh storage (motion vectors drawn).
    pub(crate) fn set_fresh_buffers(&mut self, fresh: bool) {
        self.pool.set_fresh(fresh);
    }

    /// The camera the detail and shadow choices measure from (`None`: no camera, every patch
    /// takes its nearest choices, as `vegetation.gd` did headless).
    pub(crate) fn set_camera(&mut self, camera: Option<[f32; 3]>) {
        self.camera = camera;
    }

    /// Takes a prepared product: it waits for its install, replacing a waiting one of its patch.
    pub(crate) fn deliver(&mut self, delivery: Delivery) {
        self.forget(delivery.key);
        let heights = product_heights(&delivery.product);
        self.pending.push(Pending {
            delivery,
            class: Class::Hole,
            distance_sq: 0.0,
            heights,
            installed: false,
        });
    }

    /// Reports a request whose core was busy: `vegetation.gd` asks for the patch again.
    pub(crate) fn deliver_busy(&mut self, key: PatchKey, serial: u64) {
        let event: [i64; EVENT_LANES] = [
            i64::from(key[0]),
            i64::from(key[1]),
            i64::from(key[2]),
            serial as i64,
            -1,
            0,
        ];
        self.events.extend_from_slice(&event);
    }

    /// Drops a waiting product of `key` (its request was superseded or cancelled). `O(pending)`.
    pub(crate) fn forget(&mut self, key: PatchKey) {
        if let Some(at) = self
            .pending
            .iter()
            .position(|pending| pending.delivery.key == key)
        {
            let pending = self.pending.swap_remove(at);
            self.spent.push(pending.delivery.product);
        }
    }

    /// Products whose storage can go back to the jobs.
    pub(crate) fn take_spent(&mut self) -> impl Iterator<Item = VegetationProduct> + '_ {
        self.spent.drain(..)
    }

    /// The install events since the last call, six lanes each.
    pub(crate) fn events(&self) -> &[i64] {
        &self.events
    }

    /// Forgets the events read.
    pub(crate) fn clear_events(&mut self) {
        self.events.clear();
    }

    /// Whether `key` has an installed version that draws.
    pub(crate) fn is_drawn(&self, key: PatchKey) -> bool {
        self.installed.get(&key).is_some_and(|patch| patch.visible)
    }

    /// An installed patch.
    pub(crate) fn installed(&self, key: PatchKey) -> Option<&Installed> {
        self.installed.get(&key)
    }

    /// The capacities of the layer's storage, which a steady frame must leave unchanged (tests).
    #[cfg(test)]
    pub(crate) fn footprint(&self) -> Vec<usize> {
        let mut out = vec![
            self.installed.capacity(),
            self.pending.capacity(),
            self.spent.capacity(),
            self.spare_batches.capacity(),
            self.events.capacity(),
        ];
        let mut batches: Vec<usize> = self.spare_batches.iter().map(Vec::capacity).collect();
        batches.extend(
            self.installed
                .values()
                .map(|patch| patch.batches.capacity()),
        );
        batches.sort_unstable();
        out.extend(batches);
        out.extend(self.pool.footprint());
        out
    }

    /// The slot pool, for inspection.
    pub(crate) fn pool(&self) -> &SlotPool {
        &self.pool
    }

    /// Products waiting.
    pub(crate) fn pending(&self) -> usize {
        self.pending.len()
    }

    /// The layer's status, this frame's spend included.
    pub(crate) fn status(&self) -> LayerStatus {
        let (created, freed) = self.pool.churn();
        LayerStatus {
            installed: self.installed.len(),
            visible: self
                .installed
                .values()
                .filter(|patch| patch.visible)
                .count(),
            pending: self.pending.len(),
            slots_live: self.pool.live(),
            slots_spare: self.pool.spares(),
            slots_created: created,
            slots_freed: freed,
            ..self.status
        }
    }

    /// Shows or hides an installed patch. `O(batches)`.
    pub(crate) fn set_visible(
        &mut self,
        backend: &mut impl RenderBackend,
        key: PatchKey,
        visible: bool,
    ) {
        let Some(mut patch) = self.installed.remove(&key) else {
            return;
        };
        if patch.visible != visible {
            patch.visible = visible;
            if visible {
                self.refresh_patch(backend, &mut patch);
            }
            self.apply_visibility(backend, &patch);
        }
        self.installed.insert(key, patch);
    }

    /// Frees an installed patch's slots back to the pool. `O(batches)`.
    pub(crate) fn free(&mut self, backend: &mut impl RenderBackend, key: PatchKey) {
        if let Some(patch) = self.installed.remove(&key) {
            self.release(backend, patch);
        }
    }

    /// Frees everything and drops what waits (a renderer reset).
    pub(crate) fn clear(&mut self, backend: &mut impl RenderBackend) {
        for (_, patch) in self.installed.drain() {
            self.spare_batches.push(patch.batches);
        }
        for pending in self.pending.drain(..) {
            self.spent.push(pending.delivery.product);
        }
        self.pool.clear(backend);
        self.events.clear();
        self.refreshed = None;
    }

    /// Shows or hides the whole layer (`vegetation.gd`'s `enabled`).
    pub(crate) fn set_enabled(&mut self, backend: &mut impl RenderBackend, enabled: bool) {
        if self.enabled == enabled {
            return;
        }
        self.enabled = enabled;
        let keys: Vec<PatchKey> = self.installed.keys().copied().collect();
        for key in keys {
            if let Some(patch) = self.installed.remove(&key) {
                self.apply_visibility(backend, &patch);
                self.installed.insert(key, patch);
            }
        }
    }

    /// The runtime shadow toggle: every installed patch's shadow settings again, the branched
    /// canopy chosen anew (`set_cast_shadows`).
    pub(crate) fn set_cast_shadows(&mut self, backend: &mut impl RenderBackend, cast: bool) {
        self.cast_shadows = cast;
        let keys: Vec<PatchKey> = self.installed.keys().copied().collect();
        for key in keys {
            let Some(mut patch) = self.installed.remove(&key) else {
                continue;
            };
            patch.branched = self.branched_wanted(&patch);
            self.apply_shadows(backend, &patch);
            self.apply_visibility(backend, &patch);
            self.installed.insert(key, patch);
        }
    }

    /// Drops the reflection instances (the reflection's world went or changed); impostors take
    /// new ones in the world registered next ([`Self::redraw_reflections`]).
    pub(crate) fn drop_reflections(&mut self, backend: &mut impl RenderBackend) {
        self.pool.drop_reflections(backend);
    }

    /// Draws every installed impostor in the reflection's world.
    pub(crate) fn redraw_reflections(&mut self, backend: &mut impl RenderBackend) {
        for patch in self.installed.values() {
            for batch in &patch.batches {
                if batch.lod == LOD_IMPOSTOR {
                    let main = self.pool.slot(batch.slot).drawn(Role::Main).expect("drawn");
                    let reflection = reflection_drawn(&main, self.enabled && patch.visible);
                    self.pool
                        .draw(backend, batch.slot, Role::Reflection, reflection);
                }
            }
        }
    }

    /// Moves the near canopy's detail level and branched shadow with the camera, for every
    /// shown patch, on frames the camera moved. `O(shown patches)`, zero when it did not move.
    pub(crate) fn refresh(&mut self, backend: &mut impl RenderBackend) -> usize {
        if self.camera == self.refreshed {
            return 0;
        }
        self.refreshed = self.camera;
        let mut visited = 0;
        let rules = self.rules;
        let camera = self.camera;
        let cast_shadows = self.cast_shadows;
        for patch in self.installed.values_mut() {
            if !patch.visible || !patch.near_canopy {
                continue;
            }
            visited += 1;
            refresh_one(&mut self.pool, backend, rules, camera, cast_shadows, patch);
        }
        visited
    }

    fn refresh_patch(&mut self, backend: &mut impl RenderBackend, patch: &mut Installed) {
        if patch.near_canopy {
            refresh_one(
                &mut self.pool,
                backend,
                self.rules,
                self.camera,
                self.cast_shadows,
                patch,
            );
        }
    }

    /// Installs waiting products within the frame's budget (all of them when `unlimited`), in
    /// class order, nearest first, then by key; `in_view` places a patch's box (x, z of its
    /// square and the heights of its bounds) in the view. Returns the installs.
    pub(crate) fn install(
        &mut self,
        backend: &mut impl RenderBackend,
        budget: &mut FrameBudget,
        clock: &impl Clock,
        in_view: &impl Fn([f32; 3], [f32; 3]) -> bool,
        unlimited: bool,
    ) -> usize {
        self.status.blocked = 0;
        if self.pending.is_empty() || self.rules.is_none() {
            return 0;
        }
        self.order(in_view);
        let mut done = 0;
        let mut at = 0;
        while at < self.pending.len() {
            let pending = &self.pending[at];
            let product = &pending.delivery.product;
            let predicted = predicted_cost(product);
            if !unlimited && !budget.admits(predicted) {
                break;
            }
            let needed = product.batches.iter().filter(|batch| !batch.proxy).count();
            let freed = self
                .installed
                .get(&pending.delivery.key)
                .map_or(0, |patch| patch.batches.len());
            if needed > self.pool.available() + freed {
                // Every slot of the limit is live: this patch waits; a later one (a replacement
                // that frees its own slots) may still fit.
                self.status.blocked += 1;
                at += 1;
                continue;
            }
            let start = clock.now_us();
            self.install_one(backend, at);
            let elapsed = clock.now_us() - start;
            budget.record(predicted, elapsed);
            let pending = &mut self.pending[at];
            pending.installed = true;
            let delivery = &pending.delivery;
            let [x, y, z] = delivery.key;
            let event: [i64; EVENT_LANES] = [
                i64::from(x),
                i64::from(y),
                i64::from(z),
                delivery.serial as i64,
                delivery.product.tree_count,
                elapsed as i64,
            ];
            self.events.extend_from_slice(&event);
            done += 1;
            at += 1;
        }
        // The order is rebuilt each pass, so the installed leave by swapping.
        for at in (0..self.pending.len()).rev() {
            if self.pending[at].installed {
                let pending = self.pending.swap_remove(at);
                self.spent.push(pending.delivery.product);
            }
        }
        self.status.installs_total += done as u64;
        self.status.frame = *budget;
        done
    }

    // Classes and distances from the camera, then the order.
    fn order(&mut self, in_view: &impl Fn([f32; 3], [f32; 3]) -> bool) {
        let camera = self.camera;
        for pending in &mut self.pending {
            let [ox, oz] = pending.delivery.origin;
            let span = pending.delivery.span;
            let hole = !self
                .installed
                .get(&pending.delivery.key)
                .is_some_and(|patch| patch.visible);
            let low = [ox, pending.heights[0], oz];
            let high = [ox + span, pending.heights[1], oz + span];
            let seen = in_view(low, high);
            pending.class = match (hole, seen) {
                (true, true) => Class::HoleInView,
                (true, false) if pending.delivery.key[2] > 1 => Class::HoleNear,
                (true, false) => Class::Hole,
                (false, true) => Class::UpgradeInView,
                (false, false) => Class::Upgrade,
            };
            pending.distance_sq = camera.map_or(0.0, |[cx, _, cz]| {
                let dx = f64::from(ox) + f64::from(span) * 0.5 - f64::from(cx);
                let dz = f64::from(oz) + f64::from(span) * 0.5 - f64::from(cz);
                dx * dx + dz * dz
            });
        }
        self.pending.sort_unstable_by(|a, b| {
            a.class
                .cmp(&b.class)
                .then(a.distance_sq.total_cmp(&b.distance_sq))
                .then(a.delivery.key.cmp(&b.delivery.key))
        });
    }

    // Installs the waiting product at `at`: the previous version is hidden and released, every
    // batch filled into a slot and drawn, in one unit.
    fn install_one(&mut self, backend: &mut impl RenderBackend, at: usize) {
        let rules = self.rules.expect("rules before installs");
        let key = self.pending[at].delivery.key;
        if let Some(previous) = self.installed.remove(&key) {
            self.release(backend, previous);
        }
        let delivery = &self.pending[at].delivery;
        let product = &delivery.product;
        let origin = [delivery.origin[0], 0.0, delivery.origin[1]];
        let mut patch = Installed {
            serial: delivery.serial,
            visible: true,
            batches: self.spare_batches.pop().unwrap_or_default(),
            canopy_box: product.canopy_box,
            has_caster: product.batches.iter().any(|batch| batch.proxy),
            near_canopy: product
                .batches
                .iter()
                .any(|batch| batch.lod == LOD_NEAR && batch.species < BUSH),
            detail: DETAIL_FULL,
            branched: false,
            origin: delivery.origin,
        };
        patch.batches.clear();
        // The near canopy's level is chosen before its meshes are set (`vegetation.gd` did too).
        if patch.near_canopy {
            patch.detail = near_detail(&rules, patch.canopy_box, self.camera);
        }
        patch.branched = self.branched_wanted(&patch);
        let shown = self.enabled;
        for batch in &product.batches {
            if batch.count == 0 {
                continue;
            }
            if batch.proxy {
                // The caster follows its impostor and draws on the impostor's `MultiMesh`.
                let impostor = patch
                    .batches
                    .last_mut()
                    .expect("an impostor before its caster");
                debug_assert!(impostor.lod == LOD_IMPOSTOR && impostor.species == batch.species);
                impostor.caster = true;
                let drawn = caster_drawn(batch, origin, self.cast_shadows, true, shown);
                self.pool.draw(backend, impostor.slot, Role::Caster, drawn);
                continue;
            }
            let (class, _) = capacity_class(batch.count);
            let kind = SlotKind {
                mesh: mesh_index(&rules, batch, patch.detail),
                class,
                colors: batch.colors,
                custom: batch.custom,
                parameter: (batch.lod == LOD_NEAR && batch.species < BUSH)
                    .then_some((HANDS_OVER, 1.0f32.to_bits())),
            };
            let slot = self
                .pool
                .acquire(backend, kind)
                .expect("the install checked the pool's room");
            let multimesh_aabb = if batch.lod == LOD_IMPOSTOR {
                batch.mesh_bounds
            } else {
                Aabb::default()
            };
            self.pool.fill(
                backend,
                slot,
                product.buffer(batch.buffer),
                batch.count,
                multimesh_aabb,
            );
            let shadow = main_shadow(&rules, batch, self.cast_shadows, patch.branched);
            let drawn = Drawn {
                origin,
                aabb: batch.shared_bounds,
                range: batch.range,
                material: (batch.lod == LOD_IMPOSTOR).then_some(u32::from(batch.species)),
                shadow,
                visible: shown,
            };
            self.pool.draw(backend, slot, Role::Main, drawn);
            if batch.lod == LOD_IMPOSTOR {
                self.pool.draw(
                    backend,
                    slot,
                    Role::Reflection,
                    reflection_drawn(&drawn, shown),
                );
            }
            patch.batches.push(InstalledBatch {
                slot,
                species: batch.species,
                lod: batch.lod,
                variant: batch.variant,
                count: batch.count,
                caster: false,
            });
        }
        self.installed.insert(key, patch);
    }

    fn release(&mut self, backend: &mut impl RenderBackend, mut patch: Installed) {
        for batch in patch.batches.drain(..) {
            self.pool.release(backend, batch.slot);
        }
        self.spare_batches.push(patch.batches);
    }

    fn branched_wanted(&self, patch: &Installed) -> bool {
        branched_wanted(self.rules, self.camera, self.cast_shadows, patch)
    }

    // Every batch's shadow settings from the patch's choices.
    fn apply_shadows(&mut self, backend: &mut impl RenderBackend, patch: &Installed) {
        let Some(rules) = self.rules else {
            return;
        };
        let shown = self.enabled && patch.visible;
        for batch in &patch.batches {
            let main = Batch {
                species: batch.species,
                lod: batch.lod,
                variant: batch.variant,
                ..PLAIN_BATCH
            };
            let shadow = main_shadow(&rules, &main, self.cast_shadows, patch.branched);
            self.pool
                .set_shadow(backend, batch.slot, Role::Main, shadow, shown);
            if batch.caster {
                let shadow = caster_shadow(self.cast_shadows, true);
                self.pool.set_shadow(
                    backend,
                    batch.slot,
                    Role::Caster,
                    shadow,
                    shown && shadow != Shadow::Off,
                );
            }
        }
    }

    // Every instance's visibility from the layer's and the patch's.
    fn apply_visibility(&mut self, backend: &mut impl RenderBackend, patch: &Installed) {
        let shown = self.enabled && patch.visible;
        for batch in &patch.batches {
            self.pool
                .set_visible(backend, batch.slot, Role::Main, shown);
            if batch.caster {
                let casts = caster_shadow(self.cast_shadows, patch.has_caster) != Shadow::Off;
                self.pool
                    .set_visible(backend, batch.slot, Role::Caster, shown && casts);
            }
            if batch.lod == LOD_IMPOSTOR {
                self.pool
                    .set_visible(backend, batch.slot, Role::Reflection, shown);
            }
        }
    }
}

// A template for the shadow rule, which reads species, level and variant only.
const PLAIN_BATCH: Batch = Batch {
    species: 0,
    lod: 0,
    variant: 0,
    proxy: false,
    count: 0,
    colors: false,
    custom: false,
    buffer: 0,
    mesh_bounds: Aabb {
        position: [0.0; 3],
        size: [0.0; 3],
    },
    shared_bounds: Aabb {
        position: [0.0; 3],
        size: [0.0; 3],
    },
    range: [0.0; 2],
};

fn refresh_one(
    pool: &mut SlotPool,
    backend: &mut impl RenderBackend,
    rules: Option<VegetationRules>,
    camera: Option<[f32; 3]>,
    cast_shadows: bool,
    patch: &mut Installed,
) {
    let Some(rules) = rules else {
        return;
    };
    let detail = near_detail(&rules, patch.canopy_box, camera);
    let branched = branched_wanted(Some(rules), camera, cast_shadows, patch);
    if detail == patch.detail && branched == patch.branched {
        return;
    }
    let detail_changed = detail != patch.detail;
    patch.detail = detail;
    patch.branched = branched;
    for batch in &patch.batches {
        if batch.lod != LOD_NEAR || batch.species >= BUSH {
            continue;
        }
        if detail_changed {
            let mesh = near_mesh(batch.species, batch.variant, detail);
            pool.set_mesh(backend, batch.slot, mesh);
        }
        let shown = pool
            .slot(batch.slot)
            .drawn(Role::Main)
            .is_some_and(|drawn| drawn.visible);
        let shadow = if branched { Shadow::On } else { Shadow::Off };
        pool.set_shadow(backend, batch.slot, Role::Main, shadow, shown);
    }
}

// The heights a product's bounds reach (patch-local heights are world heights), or a band
// about the ground when it has none.
fn product_heights(product: &VegetationProduct) -> [f32; 2] {
    let mut low = f32::INFINITY;
    let mut high = f32::NEG_INFINITY;
    for batch in &product.batches {
        for bounds in [batch.mesh_bounds, batch.shared_bounds] {
            if !bounds.is_zero_size() {
                low = low.min(bounds.position[1]);
                high = high.max(bounds.position[1] + bounds.size[1]);
            }
        }
    }
    if low > high {
        [-50.0, 100.0]
    } else {
        [low, high]
    }
}

fn predicted_cost(product: &VegetationProduct) -> f64 {
    let mut bytes = 0;
    for batch in &product.batches {
        if !batch.proxy {
            let stride = 12 + 4 * usize::from(batch.colors) + 4 * usize::from(batch.custom);
            bytes += capacity_class(batch.count).1 as usize * stride * 4;
        }
    }
    predicted_us(product.batches.len() * CALLS_PER_BATCH, bytes)
}

fn bucket(species: u8, variant: u8) -> u32 {
    VARIANT_COUNTS[..usize::from(species)].iter().sum::<usize>() as u32 + u32::from(variant)
}

fn near_mesh(species: u8, variant: u8, level: u8) -> u32 {
    bucket(species, variant) * NEAR_LEVELS + u32::from(level)
}

fn mesh_index(rules: &VegetationRules, batch: &Batch, detail: u8) -> u32 {
    match batch.lod {
        LOD_IMPOSTOR => IMPOSTOR_MESH,
        LOD_HEDGE_RUN => HEDGE_RUN_MESH + u32::from(batch.variant - rules.hedge_first_variant),
        _ => near_mesh(
            batch.species,
            batch.variant,
            if batch.species < BUSH {
                detail
            } else {
                DETAIL_FULL
            },
        ),
    }
}

// What a batch's own instance does in the shadow pass (`_shadow_setting`): the branched canopy
// casts while near the camera; rocks, garden plants and hedges cast; ground cover and the
// visible impostor never do.
fn main_shadow(
    rules: &VegetationRules,
    batch: &Batch,
    cast_shadows: bool,
    branched: bool,
) -> Shadow {
    if !cast_shadows {
        return Shadow::Off;
    }
    if batch.lod == LOD_NEAR && batch.species < BUSH {
        return if branched { Shadow::On } else { Shadow::Off };
    }
    if batch.species == ROCK
        || (batch.species == BUSH && batch.variant >= rules.landscape_first_variant)
    {
        Shadow::On
    } else {
        Shadow::Off
    }
}

fn caster_shadow(cast_shadows: bool, has_caster: bool) -> Shadow {
    if cast_shadows && has_caster {
        Shadow::ShadowsOnly
    } else {
        Shadow::Off
    }
}

// An impostor's shadow caster: its own range and material; a silent caster is hidden, since an
// instance that casts nothing draws its quads in colour.
fn caster_drawn(
    batch: &Batch,
    origin: [f32; 3],
    cast_shadows: bool,
    has_caster: bool,
    shown: bool,
) -> Drawn {
    let shadow = caster_shadow(cast_shadows, has_caster);
    Drawn {
        origin,
        aabb: batch.shared_bounds,
        range: batch.range,
        material: Some(SHADOW_MATERIAL + u32::from(batch.species)),
        shadow,
        visible: shown && shadow != Shadow::Off,
    }
}

// An impostor in the reflection's world (`water_reflection.gd`'s tree proxies): the bounds and
// material of the scene's, no range, no shadow.
fn reflection_drawn(main: &Drawn, shown: bool) -> Drawn {
    Drawn {
        origin: main.origin,
        aabb: main.aabb,
        range: [0.0; 2],
        material: main.material,
        shadow: Shadow::Off,
        visible: shown,
    }
}

// Godot's `Vector3.clamp(low, high).distance_to(point)` from `point`, and `distance_to` itself,
// in single precision.
fn clamp_distance(point: [f32; 3], low: [f32; 3], high: [f32; 3]) -> f32 {
    let clamped: [f32; 3] = std::array::from_fn(|i| {
        if point[i] < low[i] {
            low[i]
        } else if point[i] > high[i] {
            high[i]
        } else {
            point[i]
        }
    });
    distance(clamped, point)
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

/// The near canopy's level from the box of its tree origins (`_near_detail_lod`): one level
/// where the whole box lies on one side of the handover band, both where it can reach a tree.
pub(crate) fn near_detail(rules: &VegetationRules, trees: Aabb, camera: Option<[f32; 3]>) -> u8 {
    let Some(camera) = camera else {
        return DETAIL_FULL;
    };
    let end = [
        trees.position[0] + trees.size[0],
        trees.position[1] + trees.size[1],
        trees.position[2] + trees.size[2],
    ];
    if f64::from(clamp_distance(camera, trees.position, end))
        >= rules.near_detail_m + rules.detail_fade_m
    {
        return DETAIL_REDUCED;
    }
    let size = trees.size;
    let length = (size[0] * size[0] + size[1] * size[1] + size[2] * size[2]).sqrt();
    if f64::from(distance(trees.center(), camera)) + f64::from(length) * 0.5 <= rules.near_detail_m
    {
        return DETAIL_FULL;
    }
    DETAIL_BLEND
}

// Whether the branched canopy casts (`_refresh_branched_shadow`): the toggle on, a caster, and
// a tree that may stand within the handover of the camera.
fn branched_wanted(
    rules: Option<VegetationRules>,
    camera: Option<[f32; 3]>,
    cast_shadows: bool,
    patch: &Installed,
) -> bool {
    let Some(rules) = rules else {
        return false;
    };
    if !(cast_shadows && patch.near_canopy && patch.has_caster) {
        return false;
    }
    let Some(camera) = camera else {
        return true;
    };
    let trees = patch.canopy_box;
    let end = [
        trees.position[0] + trees.size[0],
        trees.position[1] + trees.size[1],
        trees.position[2] + trees.size[2],
    ];
    f64::from(clamp_distance(camera, trees.position, end)) < rules.shadow_handover_m
}

impl Installed {
    /// The patch's minimum corner.
    pub(crate) fn origin(&self) -> [f32; 2] {
        self.origin
    }
}

#[cfg(test)]
mod tests;
