// SPDX-License-Identifier: GPL-2.0-only

//! The vegetation's prepared set (`RENDER-32-N`, `CODE-12`; `docs/terrain.md` "Streaming core"
//! 5.3 item 2): products of the patches the camera can need next, built on the background pool
//! and held here, never installed, so a patch that enters the wanted set takes a finished product
//! at its request instead of waiting for a job.
//!
//! - **What.** The camera's reach ([`reach_samples`]: the orbit circle, the zoom line in and out,
//!   the pan disc), sample eye by sample eye in travel order: each sample's wanted patches by the
//!   residency's own tier rule (`block_state`), less those wanted and waiting for a request now.
//! - **Variants.** A patch is held by its key and bands. Its variants at a sample are the bands
//!   a request can ask for from a camera within the sample's spacing (a camera on the shape
//!   between two samples is within the spacing of both; the bands are thresholds on the distance,
//!   which moves no faster than the camera): one set of bands, or two to four where band
//!   boundaries fall inside (`band_variants`). A patch without a record takes the rule's bands
//!   from no record, as its request computes them when it enters; a drawn, standing-in or cached
//!   patch through its record's hysteresis, less its own bands while its generations hold (its
//!   retained product). The jobs keep one live request a patch, so one prepare of a patch is out at
//!   a time and its other variants follow when it lands.
//! - **When.** Only while nothing wanted waits (the request queue empty and no request in flight)
//!   and the view held still since the last frame: one sample at the frame's first prepare and
//!   one more after the pull frees the prepares that landed, at most [`PREPARE_IN_FLIGHT`] jobs
//!   out, through the same jobs and the same background pool as the requests; never on the shared
//!   pool. The samples are taken twice: first for the patches without a record (one entering
//!   there would wait for a job, a hole), then for the band changes of those with one (they stay
//!   drawn meanwhile). A variant waiting for its patch's other prepare is deferred and asked for
//!   first once that lands. A new view (eye or pivot moved) starts the samples over; products
//!   held stay.
//! - **Use.** A request whose patch has a product held or being prepared with exactly the
//!   generations and bands it would ask for takes it: a held product is delivered at once (it
//!   installs under the frame's budget like any other), a job in flight is adopted (its product is
//!   delivered when it lands). A prepare in flight that does not fit is cancelled, a held product
//!   of the request's bands from other generations spent, and the patch is asked for as before, so
//!   every product installed is the one its request would have built: the settled view does not
//!   change. Products held for other bands stay for a later request.
//! - **Retained.** An installed product stays here with its record (5.3 item 3), so a patch
//!   that leaves the drawn set and is asked for again with the same generations and bands (a turn
//!   back) installs without a job.
//! - **Memory.** [`PREPARED_CAP_BYTES`] of products, prepared and retained together; over it the
//!   products farthest from the camera are evicted (ties by key and bands) and counted
//!   (`over_cap`).
//!
//! Costs: a frame that prepares `O(blocks within the far range of one sample eye x
//! subdivision²)`, each patch's variants `O(1)` (at most four); a moving frame, a still frame
//! with the samples done or a request waiting `O(1)`; a request `O(1)` more (eight probes for a
//! patch's other variants); an eviction `O(held)`. Storage grows to the cap and the samples, then
//! is reused. Jobs out: at most [`PREPARE_IN_FLIGHT`] prepares.

use super::super::Delivery;
use super::*;
use crate::nodes::sim::render::stream::reach::{ReachSample, reach_samples};
use crate::nodes::sim::render::stream::view::CameraSample;
use crate::nodes::sim::render::vegetation_product::VegetationProduct;

/// Bytes of prepared vegetation products held (the plan's default, a setting later).
pub(crate) const PREPARED_CAP_BYTES: usize = 64 << 20;
/// Prepare jobs out at once: twice what the vegetation's running jobs take (`MAX_RUNNING`), so
/// the pool runs a full round while the last round's products wait for the pull; a request that
/// arrives waits behind at most two rounds of them (measured `RENDER-32-N` part 2: at one round
/// the 30k city's first turn started before its reach was prepared).
pub(crate) const PREPARE_IN_FLIGHT: usize = 8;

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
pub(crate) struct HeldProducts<K, P> {
    held: Map<K, Held<P>>,
    bytes: usize,
    cap: usize,
    over_cap: u64,
}

impl<K: Copy + Ord + std::hash::Hash, P> HeldProducts<K, P> {
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
        key: K,
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
    pub(crate) fn take(&mut self, key: K) -> Option<Held<P>> {
        let held = self.held.remove(&key)?;
        self.bytes -= held.bytes;
        Some(held)
    }

    /// Whether `key` has a product held.
    pub(crate) fn contains(&self, key: K) -> bool {
        self.held.contains_key(&key)
    }

    /// `key`'s product, if held.
    pub(crate) fn get(&self, key: K) -> Option<&Held<P>> {
        self.held.get(&key)
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

// A patch's product for one set of bands: what a prepared or retained product is held by.
type Variant = (PatchKey, Bands);

// Every set of bands, for a patch's variants.
fn every_bands() -> impl Iterator<Item = Bands> {
    (0..8u8).map(|bits| Bands {
        understory: bits & 1 != 0,
        near_band: bits & 2 != 0,
        caster: bits & 4 != 0,
    })
}

// What asking for one variant came to.
enum Step {
    // Asked for, or not needed.
    Done,
    // Another variant of the patch is out.
    Wait,
    // The in-flight bound or the jobs refused: ask again next time.
    Full,
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
    // The samples are taken twice: first for the patches without a record (a patch entering
    // there would wait for a job), then for the band changes of those with one (they stay drawn
    // meanwhile).
    records: bool,
    // The eye and pivot the samples are for.
    view: Option<([f32; 3], [f32; 3])>,
    // Variants already prepared for this view's samples.
    considered: Set<Variant>,
    // At most one prepare of a patch is out (the jobs keep one live request a patch).
    preparing: Map<PatchKey, Request>,
    // Variants waiting for another variant of their patch to land, in sample order.
    deferred: Vec<Variant>,
    // The view held still at this frame's first prepare (a later one may continue).
    still: bool,
    held: HeldProducts<Variant, Ahead>,
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
            records: false,
            view: None,
            considered: Set::default(),
            preparing: Map::default(),
            deferred: Vec::new(),
            still: false,
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
        self.deferred.clear();
        self.still = false;
        self.view = None;
        self.next = 0;
    }
}

impl VegetationResidency {
    /// Prepares the next sample of the view's reach while nothing wanted waits and the view holds
    /// still (the frame's first pass, after [`Self::frame`]). `O(1)` unless it prepares: then
    /// the samples once a view, the deferred variants and one sample's blocks.
    pub(crate) fn prepare(&mut self, view: Option<CameraSample>, jobs: &mut impl PatchJobs) {
        self.prepared.still = false;
        let (Some(settings), true, Some(view)) = (self.settings, self.enabled, view) else {
            return;
        };
        if !self.may_prepare() {
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
            prepared.records = false;
            prepared.considered.clear();
            prepared.deferred.clear();
        }
        prepared.still = true;
        self.prepare_step(&settings, jobs);
    }

    /// Prepares once more in the frame's first pass, after the pull freed the prepares that
    /// landed, when the frame's [`Self::prepare`] found the view still and nothing wanted waits
    /// yet: the same work as one `prepare`, so a still frame takes at most two samples.
    pub(crate) fn prepare_after_pull(&mut self, jobs: &mut impl PatchJobs) {
        let Some(settings) = self.settings.filter(|_| self.prepared.still) else {
            return;
        };
        self.prepared.still = false;
        if self.may_prepare() {
            self.prepare_step(&settings, jobs);
        }
    }

    // Nothing wanted waits: the request queue is empty and no request is out.
    fn may_prepare(&self) -> bool {
        self.enabled
            && self.terrain_span > 0.0
            && self.evaluated
            && self.queue.is_empty()
            && self.requested.is_empty()
    }

    // The deferred variants whose patch has no prepare out, then the next sample while the
    // in-flight bound allows.
    fn prepare_step(&mut self, settings: &ResidencySettings, jobs: &mut impl PatchJobs) {
        let mut deferred = std::mem::take(&mut self.prepared.deferred);
        let mut full = false;
        deferred.retain(|&(key, bands)| {
            if full {
                return true;
            }
            match self.prepare_variant(settings, key, bands, jobs) {
                Step::Done => false,
                Step::Wait => true,
                Step::Full => {
                    full = true;
                    true
                }
            }
        });
        self.prepared.deferred = deferred;
        let prepared = &mut self.prepared;
        if prepared.next >= prepared.samples.len() && !prepared.records {
            prepared.next = 0;
            prepared.records = true;
        }
        if full
            || prepared.next >= prepared.samples.len()
            || prepared.preparing.len() >= PREPARE_IN_FLIGHT
        {
            return;
        }
        let sample = prepared.samples[prepared.next];
        if self.prepare_sample(settings, sample, jobs) {
            self.prepared.next += 1;
        }
    }

    // Asks for the variants of the patches sample eye `sample` wants that nothing holds: for
    // each, the bands a request can ask for from a camera within the sample's spacing (a patch
    // not drawn without a record; a drawn, standing-in or cached one through its record's
    // hysteresis, its own bands excluded while its generations hold). False when the jobs, the
    // in-flight bound or another variant of a patch still out stopped it (the same sample is
    // taken again next frame).
    fn prepare_sample(
        &mut self,
        settings: &ResidencySettings,
        sample: ReachSample,
        jobs: &mut impl PatchJobs,
    ) -> bool {
        let (span, world, eye) = (self.terrain_span, settings.world, sample.eye);
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
        let mut variants = [Bands::default(); 4];
        'keys: for &key in &entering {
            let record = self.record(key);
            // A wanted patch without a record waits for its request (none while preparing).
            if record.is_some() != self.prepared.records
                || record.is_none() && self.wanted.contains(&key)
            {
                continue;
            }
            let distance = key_distance(key, span, world, eye);
            let count = settings.band_variants(
                distance,
                sample.spacing,
                key_span(key, span),
                record.map(|record| record.bands),
                &mut variants,
            );
            for &bands in &variants[..count] {
                match self.prepare_variant(settings, key, bands, jobs) {
                    Step::Done => {}
                    // One prepare of a patch at a time: this variant follows when it lands.
                    // Bounded: only patches with a prepare out wait (at most
                    // `PREPARE_IN_FLIGHT` patches x 7 other bands), so the check is short.
                    Step::Wait => {
                        if !self.prepared.deferred.contains(&(key, bands)) {
                            self.prepared.deferred.push((key, bands));
                        }
                    }
                    Step::Full => {
                        finished = false;
                        break 'keys;
                    }
                }
            }
        }
        entering.clear();
        leaving.clear();
        self.prepared.entering = entering;
        self.prepared.leaving = leaving;
        finished
    }

    // The record of `key`: drawn, standing in or cached.
    fn record(&self, key: PatchKey) -> Option<Record> {
        self.patches
            .get(&key)
            .or_else(|| self.standins.get(&key))
            .or_else(|| self.cache.get(&key))
            .copied()
    }

    // Asks for `key`'s product with `bands` unless it is not needed (its record's own bands with
    // its generations, already asked for this view, or held for its generations): done; waits
    // while another variant of the patch is out; full while the in-flight bound or the jobs
    // refuse.
    fn prepare_variant(
        &mut self,
        settings: &ResidencySettings,
        key: PatchKey,
        bands: Bands,
        jobs: &mut impl PatchJobs,
    ) -> Step {
        let (surface, vegetation) = self.owner_generations(key);
        let built = |request: &Request| {
            request.surface == surface && request.vegetation == vegetation && request.bands == bands
        };
        let own = self
            .record(key)
            .is_some_and(|record| record.bands == bands && !self.stale(key, &record));
        let prepared = &self.prepared;
        if own
            || prepared.considered.contains(&(key, bands))
            || prepared
                .held
                .get((key, bands))
                .is_some_and(|held| built(&held.product.request))
        {
            return Step::Done;
        }
        if let Some(request) = prepared.preparing.get(&key) {
            return if built(request) { Step::Done } else { Step::Wait };
        }
        if prepared.preparing.len() >= PREPARE_IN_FLIGHT {
            return Step::Full;
        }
        let span = key_span(key, self.terrain_span);
        let order = PatchOrder {
            key,
            origin: key_origin(key, self.terrain_span, settings.world),
            span: span as f32,
            bands,
        };
        let Some(serial) = jobs.request(&order, settings) else {
            return Step::Full;
        };
        self.prepared.considered.insert((key, bands));
        self.prepared.preparing.insert(
            key,
            Request {
                serial,
                surface,
                vegetation,
                bands,
            },
        );
        Step::Done
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
        prepared
            .held
            .hold((key, request.bands), held, camera, &mut prepared.evicted);
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
            prepared
                .held
                .hold((key, record.bands), held, camera, &mut prepared.evicted);
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
    // generations and bands (a held one delivered now); its serial, else `None`. A prepare of
    // the patch still out that does not fit is cancelled (the request's job would supersede it;
    // so no prepare is out for a requested patch), a held product of these bands built from
    // other generations is spent; held products of other bands stay for a later request.
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
        // A drawn patch asked for again (new bands, an edit) whose own products do not fit is a
        // replacement; any other patch's is a drop (built for other bands or generations).
        let drawn = self.patches.contains_key(&key);
        // A request of the patch still out (other bands or generations) is cancelled before a
        // held product stands in for it, as a new job would supersede it.
        let asked = self.requested.contains_key(&key);
        let prepared = &mut self.prepared;
        let mut other = false;
        if let Some(request) = prepared.preparing.remove(&key) {
            if fits(&request) {
                prepared.used += 1;
                return Some(request.serial);
            }
            jobs.cancel(key);
            other = true;
        }
        if let Some(held) = prepared.held.take((key, bands)) {
            if fits(&held.product.request) {
                if asked {
                    jobs.cancel(key);
                }
                prepared.used += 1;
                let serial = held.product.request.serial;
                layer.deliver(held.product.delivery);
                return Some(serial);
            }
            prepared.spent.push(held.product.delivery.product);
            other = true;
        }
        other |= every_bands().any(|other| prepared.held.contains((key, other)));
        if other {
            if drawn {
                prepared.replaced += 1;
            } else {
                prepared.dropped += 1;
            }
        }
        None
    }

    /// The products dropped or evicted, for their storage to go back to the jobs.
    pub(crate) fn take_prepared_spent(&mut self) -> std::vec::Drain<'_, VegetationProduct> {
        self.prepared.spent.drain(..)
    }

    /// The bands of `key`'s held products (tests).
    #[cfg(test)]
    pub(crate) fn prepared_variants(&self, key: PatchKey) -> Vec<Bands> {
        every_bands()
            .filter(|&bands| self.prepared.held.contains((key, bands)))
            .collect()
    }

    /// The patches with a product held (tests).
    #[cfg(test)]
    pub(crate) fn prepared_patches(&self) -> std::collections::BTreeSet<PatchKey> {
        self.prepared.held.held.keys().map(|(key, _)| *key).collect()
    }

    /// The prepared set's counts: products held, their bytes, prepares in flight, evictions over
    /// the cap, products used by a request, dropped (an entering patch's request whose bands and
    /// generations no product held or in flight was built for, while one for other bands or
    /// generations was) and replaced (the same for a drawn patch asked for again).
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
