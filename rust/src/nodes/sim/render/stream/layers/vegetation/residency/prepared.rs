// SPDX-License-Identifier: GPL-2.0-only

//! The vegetation's prepared set (`RENDER-32-N`, `CODE-12`; `docs/terrain.md` "Streaming core"
//! 5.3 item 2): products of the patches the camera can need next, built on the background pool
//! and held here, never installed, so a patch that enters the wanted set takes a finished product
//! at its request instead of waiting for a job.
//!
//! - **What.** The camera's reach ([`reach_samples`]: the orbit circle, the eye-to-pivot segment,
//!   the pan disc), sample eye by sample eye in travel order: each sample's wanted patches by the
//!   residency's own tier rule (`block_state`), less those wanted, drawn, cached or standing in
//!   now, already held or asked for. A patch's bands are the rule's at that sample eye for a
//!   patch without a record, which is what its request computes when it enters there.
//! - **When.** Only while nothing wanted waits (the request queue empty and no request in flight)
//!   and the view held still since the last frame, one sample a frame, at most [`PREPARE_IN_FLIGHT`] jobs out, through the same jobs and the same
//!   background pool as the requests; never on the shared pool. A new view (eye or pivot moved)
//!   starts the samples over; products held stay.
//! - **Use.** A request whose patch has a product held or being prepared with the generations and
//!   bands it would ask for takes it: a held product is delivered at once (it installs under the
//!   frame's budget like any other), a job in flight is adopted (its product is delivered when it
//!   lands). Anything else is dropped and the patch is asked for as before, so every product
//!   installed is the one its request would have built: the settled view does not change.
//! - **Retained.** An installed product stays here with its record (5.3 item 3), so a patch
//!   that leaves the drawn set and is asked for again with the same generations and bands (a turn
//!   back) installs without a job.
//! - **Memory.** [`PREPARED_CAP_BYTES`] of products, prepared and retained together; over it the
//!   products farthest from the camera are evicted (ties by key) and counted (`over_cap`).
//!
//! Costs: a frame that prepares `O(blocks within the far range of one sample eye x
//! subdivision²)`; a moving frame, a still frame with the samples done or a request waiting
//! `O(1)`; a request
//! `O(1)` more; an eviction `O(held)`. Storage grows to the cap and the samples, then is reused.

use super::super::Delivery;
use super::*;
use crate::nodes::sim::render::stream::reach::{ReachSample, reach_samples};
use crate::nodes::sim::render::stream::view::CameraSample;
use crate::nodes::sim::render::vegetation_product::VegetationProduct;

/// Bytes of prepared vegetation products held (the plan's default, a setting later).
pub(crate) const PREPARED_CAP_BYTES: usize = 64 << 20;
/// Prepare jobs out at once: what the vegetation's running jobs take (`MAX_RUNNING`), so a
/// request that arrives waits behind at most one round of them.
pub(crate) const PREPARE_IN_FLIGHT: usize = 4;

/// One product held: what it was built from and where its patch is.
pub(crate) struct Held<P> {
    /// The product.
    pub(crate) product: P,
    /// Its patch's centre, for the eviction's distances.
    pub(crate) centre: [f32; 2],
    /// Bytes it holds.
    pub(crate) bytes: usize,
}

/// Products held by key within a byte cap; over the cap the farthest from the camera leave.
pub(crate) struct HeldProducts<P> {
    held: Map<PatchKey, Held<P>>,
    bytes: usize,
    cap: usize,
    over_cap: u64,
}

impl<P> HeldProducts<P> {
    /// An empty store of at most `cap` bytes.
    pub(crate) fn new(cap: usize) -> Self {
        Self {
            held: Map::default(),
            bytes: 0,
            cap,
            over_cap: 0,
        }
    }

    /// Holds `held` for `key` (replacing an older product, returned in `out`), then evicts the
    /// products farthest from `camera` into `out` while over the cap, each counted.
    /// `O(1)`, an eviction `O(held)`.
    pub(crate) fn hold(
        &mut self,
        key: PatchKey,
        held: Held<P>,
        camera: [f32; 2],
        out: &mut Vec<P>,
    ) {
        self.bytes += held.bytes;
        if let Some(old) = self.held.insert(key, held) {
            self.bytes -= old.bytes;
            out.push(old.product);
        }
        while self.bytes > self.cap {
            let farthest = self
                .held
                .iter()
                .map(|(key, held)| (distance_squared(held.centre, camera), *key))
                .max_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
                .map(|(_, key)| key);
            let Some(evicted) = farthest.and_then(|key| self.take(key)) else {
                break;
            };
            self.over_cap += 1;
            out.push(evicted.product);
        }
    }

    /// Takes `key`'s product.
    pub(crate) fn take(&mut self, key: PatchKey) -> Option<Held<P>> {
        let held = self.held.remove(&key)?;
        self.bytes -= held.bytes;
        Some(held)
    }

    /// Whether `key` has a product held.
    pub(crate) fn contains(&self, key: PatchKey) -> bool {
        self.held.contains_key(&key)
    }

    /// Every product into `out`.
    pub(crate) fn clear(&mut self, out: &mut Vec<P>) {
        out.extend(self.held.drain().map(|(_, held)| held.product));
        self.bytes = 0;
    }

    /// Products held, their bytes and the evictions so far.
    pub(crate) fn counts(&self) -> (usize, usize, u64) {
        (self.held.len(), self.bytes, self.over_cap)
    }
}

// A product held with the request it answered.
pub(super) struct Ahead {
    delivery: Delivery,
    request: Request,
}

/// The prepared set's state in the residency.
pub(crate) struct Prepared {
    samples: Vec<ReachSample>,
    next: usize,
    // The eye and pivot the samples are for.
    view: Option<([f32; 3], [f32; 3])>,
    // Patches already prepared (or found held, wanted or drawn) for this view's samples.
    considered: Set<PatchKey>,
    preparing: Map<PatchKey, Request>,
    held: HeldProducts<Ahead>,
    spent: Vec<VegetationProduct>,
    evicted: Vec<Ahead>,
    entering: Vec<PatchKey>,
    leaving: Vec<PatchKey>,
    // The installs a frame's events accepted, by key and serial (scratch for `take_installs`).
    pub(super) accepted: Vec<(PatchKey, u64)>,
    used: u64,
    dropped: u64,
    replaced: u64,
}

impl Default for Prepared {
    fn default() -> Self {
        Self {
            samples: Vec::new(),
            next: 0,
            view: None,
            considered: Set::default(),
            preparing: Map::default(),
            held: HeldProducts::new(PREPARED_CAP_BYTES),
            spent: Vec::new(),
            evicted: Vec::new(),
            entering: Vec::new(),
            leaving: Vec::new(),
            accepted: Vec::new(),
            used: 0,
            dropped: 0,
            replaced: 0,
        }
    }
}

impl Prepared {
    // Drops every product and request (a reset; the jobs' requests were cleared with it).
    pub(super) fn clear(&mut self) {
        self.held.clear(&mut self.evicted);
        self.spent
            .extend(self.evicted.drain(..).map(|ahead| ahead.delivery.product));
        self.preparing.clear();
        self.considered.clear();
        self.view = None;
        self.next = 0;
    }
}

impl VegetationResidency {
    /// Prepares the next sample of the view's reach while nothing wanted waits and the view holds
    /// still (the frame's first pass, after [`Self::frame`]). `O(1)` unless it prepares: then
    /// the samples once a view, and one sample's blocks.
    pub(crate) fn prepare(&mut self, view: Option<CameraSample>, jobs: &mut impl PatchJobs) {
        let (Some(settings), true, Some(view)) = (self.settings, self.enabled, view) else {
            return;
        };
        if self.terrain_span <= 0.0
            || !self.evaluated
            || !self.queue.is_empty()
            || !self.requested.is_empty()
        {
            return;
        }
        let prepared = &mut self.prepared;
        // A moving view only marks the samples out of date; they are taken once it holds for a
        // frame, so a moving frame costs nothing here.
        if prepared.view != Some((view.eye, view.pivot)) {
            prepared.view = Some((view.eye, view.pivot));
            prepared.next = usize::MAX;
            return;
        }
        if prepared.next == usize::MAX {
            reach_samples(&view, &mut prepared.samples);
            prepared.next = 0;
            prepared.considered.clear();
        }
        if prepared.next >= prepared.samples.len() || prepared.preparing.len() >= PREPARE_IN_FLIGHT
        {
            return;
        }
        let eye = prepared.samples[prepared.next].eye;
        if self.prepare_sample(&settings, eye, jobs) {
            self.prepared.next += 1;
        }
    }

    // Asks for the patches sample eye `eye` wants that nothing holds; false when the jobs or the
    // in-flight bound stopped it (the same sample is taken again next frame).
    fn prepare_sample(
        &mut self,
        settings: &ResidencySettings,
        eye: [f32; 2],
        jobs: &mut impl PatchJobs,
    ) -> bool {
        let (span, world) = (self.terrain_span, settings.world);
        let reach = settings.product.far_m + span + span * HALF_DIAGONAL;
        let origin = [-f64::from(world[0]) * 0.5, -f64::from(world[1]) * 0.5];
        let cell = |value: f64, axis: usize| ((value - origin[axis]) / span).floor() as i32;
        let low = [
            cell(f64::from(eye[0]) - reach, 0),
            cell(f64::from(eye[1]) - reach, 1),
        ];
        let high = [
            cell(f64::from(eye[0]) + reach, 0),
            cell(f64::from(eye[1]) + reach, 1),
        ];
        let mut entering = std::mem::take(&mut self.prepared.entering);
        let mut leaving = std::mem::take(&mut self.prepared.leaving);
        entering.clear();
        for z in low[1]..=high[1] {
            for x in low[0]..=high[0] {
                let block = [x, z];
                if !self.resident.contains(&block) {
                    continue;
                }
                let state = block_state(settings, span, &self.resident, block, eye);
                diff_keys(
                    settings.subdivision,
                    block,
                    BlockState::default(),
                    state,
                    &mut leaving,
                    &mut entering,
                );
            }
        }
        let mut finished = true;
        for &key in &entering {
            let prepared = &self.prepared;
            if prepared.considered.contains(&key)
                || prepared.held.contains(key)
                || prepared.preparing.contains_key(&key)
                || self.wanted.contains(&key)
                || self.patches.contains_key(&key)
                || self.cache.contains_key(&key)
                || self.standins.contains_key(&key)
            {
                continue;
            }
            if prepared.preparing.len() >= PREPARE_IN_FLIGHT {
                finished = false;
                break;
            }
            let distance = key_distance(key, span, world, eye);
            let bands = settings.bands(distance, key_span(key, span), None);
            let (surface, vegetation) = self.owner_generations(key);
            let order = PatchOrder {
                key,
                origin: key_origin(key, span, world),
                span: key_span(key, span) as f32,
                bands,
            };
            let Some(serial) = jobs.request(&order, settings) else {
                finished = false;
                break;
            };
            self.prepared.considered.insert(key);
            self.prepared.preparing.insert(
                key,
                Request {
                    serial,
                    surface,
                    vegetation,
                    bands,
                },
            );
        }
        entering.clear();
        leaving.clear();
        self.prepared.entering = entering;
        self.prepared.leaving = leaving;
        finished
    }

    /// A product that landed: a prepare's is held (`None`), anything else goes on to the install
    /// layer (`Some`). `O(1)`, an eviction `O(held)`.
    pub(crate) fn take_prepared(&mut self, delivery: Delivery) -> Option<Delivery> {
        let key = delivery.key;
        let Some(&request) = self
            .prepared
            .preparing
            .get(&key)
            .filter(|request| request.serial == delivery.serial)
        else {
            return Some(delivery);
        };
        self.prepared.preparing.remove(&key);
        let (Some(settings), Some(camera)) = (self.settings, self.camera) else {
            self.prepared.spent.push(delivery.product);
            return None;
        };
        let held = Held {
            centre: key_centre(key, self.terrain_span, settings.world),
            bytes: delivery.product.bytes(),
            product: Ahead { delivery, request },
        };
        let prepared = &mut self.prepared;
        prepared.held.hold(key, held, camera, &mut prepared.evicted);
        prepared.spent.extend(
            prepared
                .evicted
                .drain(..)
                .map(|ahead| ahead.delivery.product),
        );
        None
    }

    // The products the layer installed: one its residency accepted is kept with its record as
    // the patch's retained product (the plan's retained set: a patch that leaves and comes back
    // within the cap installs without a job); any other is spent. `O(installed x accepted)`.
    pub(super) fn retain_installed(
        &mut self,
        accepted: &[(PatchKey, u64)],
        layer: &mut VegetationLayer,
    ) {
        let (Some(settings), Some(camera)) = (self.settings, self.camera) else {
            self.prepared
                .spent
                .extend(layer.take_retained().map(|delivery| delivery.product));
            return;
        };
        for delivery in layer.take_retained() {
            let record = self.patches.get(&delivery.key).copied();
            let Some(record) =
                record.filter(|_| accepted.contains(&(delivery.key, delivery.serial)))
            else {
                self.prepared.spent.push(delivery.product);
                continue;
            };
            let key = delivery.key;
            let request = Request {
                serial: delivery.serial,
                surface: record.surface,
                vegetation: record.vegetation,
                bands: record.bands,
            };
            let held = Held {
                centre: key_centre(key, self.terrain_span, settings.world),
                bytes: delivery.product.bytes(),
                product: Ahead { delivery, request },
            };
            let prepared = &mut self.prepared;
            prepared.held.hold(key, held, camera, &mut prepared.evicted);
            prepared.spent.extend(
                prepared
                    .evicted
                    .drain(..)
                    .map(|ahead| ahead.delivery.product),
            );
        }
    }

    /// A prepare whose core was busy: it is dropped (true), not asked again for this view.
    pub(crate) fn prepared_busy(&mut self, key: PatchKey, serial: u64) -> bool {
        if self
            .prepared
            .preparing
            .get(&key)
            .is_some_and(|request| request.serial == serial)
        {
            self.prepared.preparing.remove(&key);
            return true;
        }
        false
    }

    // At a request: the product held or in flight for `key` if it was built for `request`'s
    // generations and bands (a held one delivered now); its serial, else `None` with any other
    // dropped (an in-flight one cancelled, so it is never delivered).
    pub(super) fn take_ahead(
        &mut self,
        key: PatchKey,
        surface: i64,
        vegetation: i64,
        bands: Bands,
        layer: &mut VegetationLayer,
        jobs: &mut impl PatchJobs,
    ) -> Option<u64> {
        let fits = |request: &Request| {
            request.surface == surface && request.vegetation == vegetation && request.bands == bands
        };
        // A drawn patch asked for again (new bands, an edit) replaces its own retained product;
        // any other mismatch is a product prepared for other bands or generations.
        let drawn = self.patches.contains_key(&key);
        if let Some(request) = self.prepared.preparing.remove(&key) {
            if fits(&request) {
                self.prepared.used += 1;
                return Some(request.serial);
            }
            jobs.cancel(key);
            if drawn {
                self.prepared.replaced += 1;
            } else {
                self.prepared.dropped += 1;
            }
        }
        let held = self.prepared.held.take(key)?;
        if fits(&held.product.request) {
            self.prepared.used += 1;
            let serial = held.product.request.serial;
            layer.deliver(held.product.delivery);
            return Some(serial);
        }
        if drawn {
            self.prepared.replaced += 1;
        } else {
            self.prepared.dropped += 1;
        }
        self.prepared.spent.push(held.product.delivery.product);
        None
    }

    /// The products dropped or evicted, for their storage to go back to the jobs.
    pub(crate) fn take_prepared_spent(&mut self) -> std::vec::Drain<'_, VegetationProduct> {
        self.prepared.spent.drain(..)
    }

    /// The prepared set's counts: products held, their bytes, prepares in flight, evictions over
    /// the cap, products used by a request, dropped (an entering patch's, built for other
    /// generations or bands) and replaced (a drawn patch's own, asked for again).
    pub(crate) fn prepared_counts(&self) -> [i64; 7] {
        let (held, bytes, over_cap) = self.prepared.held.counts();
        [
            held as i64,
            bytes as i64,
            self.prepared.preparing.len() as i64,
            over_cap as i64,
            self.prepared.used as i64,
            self.prepared.dropped as i64,
            self.prepared.replaced as i64,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held(product: u32, centre: [f32; 2], bytes: usize) -> Held<u32> {
        Held {
            product,
            centre,
            bytes,
        }
    }

    #[test]
    fn the_cap_keeps_the_nearest_and_counts_what_it_evicts() {
        let mut store = HeldProducts::new(100);
        let mut out = Vec::new();
        store.hold([0, 0, 1], held(1, [10.0, 0.0], 40), [0.0, 0.0], &mut out);
        store.hold([1, 0, 1], held(2, [500.0, 0.0], 40), [0.0, 0.0], &mut out);
        assert!(out.is_empty());
        // A third over the cap: the farthest from the camera leaves, whichever was held last.
        store.hold([2, 0, 1], held(3, [50.0, 0.0], 40), [0.0, 0.0], &mut out);
        assert_eq!(out, vec![2]);
        assert_eq!(store.counts(), (2, 80, 1));
        // The camera moved: now the first is the farthest.
        store.hold([3, 0, 1], held(4, [900.0, 0.0], 40), [900.0, 0.0], &mut out);
        assert_eq!(out, vec![2, 1]);
        assert!(store.contains([3, 0, 1]) && store.contains([2, 0, 1]));
        // Equal distances leave the larger key first.
        let mut ties = HeldProducts::new(50);
        let mut gone = Vec::new();
        ties.hold([0, 0, 1], held(5, [10.0, 0.0], 40), [0.0, 0.0], &mut gone);
        ties.hold([0, 1, 1], held(6, [0.0, 10.0], 40), [0.0, 0.0], &mut gone);
        assert_eq!(gone, vec![6]);
        // A product too large for the cap alone is held by nothing.
        ties.hold([9, 9, 1], held(7, [0.0, 0.0], 60), [0.0, 0.0], &mut gone);
        assert_eq!(ties.counts().0, 0);
    }

    #[test]
    fn a_replaced_product_comes_back_and_take_releases_its_bytes() {
        let mut store = HeldProducts::new(100);
        let mut out = Vec::new();
        store.hold([0, 0, 1], held(1, [0.0, 0.0], 30), [0.0, 0.0], &mut out);
        store.hold([0, 0, 1], held(2, [0.0, 0.0], 20), [0.0, 0.0], &mut out);
        assert_eq!(out, vec![1]);
        assert_eq!(store.counts(), (1, 20, 0));
        assert_eq!(store.take([0, 0, 1]).map(|held| held.product), Some(2));
        assert_eq!(store.counts(), (0, 0, 0));
    }
}
