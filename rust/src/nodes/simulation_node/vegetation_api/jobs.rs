// SPDX-License-Identifier: GPL-2.0-only

//! Vegetation patches prepared on the background pool (`RENDER-32-D`): the renderer requests a
//! patch by key with the bands it chose; a worker takes the patch's
//! [`VegetationPatchInputs`] under `try_lock`, runs the scatter, the stand cover, the hedge
//! runs and the product off the lock, and lands the product in the cache; the stream node polls
//! finished products and installs them (`RENDER-32-E`). Nothing of the query runs on Godot's main thread.

use super::land_cover::Coverage;
use super::world::VegetationPatchInputs;
use super::*;
use crate::nodes::sim::render::stream::cache::ProductCache;
use crate::nodes::sim::render::stream::layers::vegetation::residency::{
    PatchJobs, PatchOrder, ResidencySettings,
};
use crate::nodes::sim::render::stream::prepare::PrepareQueue;
use crate::nodes::sim::render::vegetation_product::{
    ProductCatalogue, ProductInputs, ProductSettings, StandCover, VegetationProduct, build_product,
};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};

/// Most patch jobs out at once, from the request until the poll takes the result: twice the
/// renderer's per-frame request budget (16 sub-patches), so a frame's requests never wait for
/// the last frame's to be polled.
const MAX_IN_FLIGHT: usize = 32;
/// Bytes of finished products the cache may hold before no job starts (`docs/terrain.md`
/// "Streaming core": 64 MB a layer).
const CACHE_CAP_BYTES: usize = 64 << 20;

/// Vegetation jobs running at once on the background pool. Each runs its queries sequentially,
/// so this many of the pool's threads at most work for the vegetation while the frame runs beside
/// them on the same cores; the rest wait in order of request. Measured in `RENDER-32-D`: at
/// eight (every thread, each job fanning out) the 30k turns' worst frames rose 4-13 ms.
const MAX_RUNNING: usize = 4;

/// Calls of the synchronous vegetation queries (`get_decorative_tree_patch`,
/// `get_vegetation_stand_cover`, `get_vegetation_hedge_runs`) since the extension loaded: the
/// renderer asks none of them, so a benchmark frame that moves it ran a query on the main
/// thread.
pub(super) static MAIN_THREAD_QUERIES: AtomicU64 = AtomicU64::new(0);

/// A vegetation patch key: grid column, row and subdivision (`vegetation.gd`'s `Vector3i`).
pub(crate) type PatchKey = [i32; 3];

/// One patch request: where the patch is and what the renderer's bands put in it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PatchRequest {
    /// The patch key.
    pub(crate) key: PatchKey,
    /// The patch's minimum corner, world metres.
    pub(crate) origin: Vector2,
    /// The patch's side, metres.
    pub(crate) span: f32,
    /// Whether the understory grid is scattered.
    pub(crate) understory: bool,
    /// Whether the near canopy band is drawn.
    pub(crate) near_band: bool,
    /// Whether the patch casts a shadow.
    pub(crate) caster: bool,
    /// The renderer's draw distances.
    pub(crate) settings: ProductSettings,
}

/// The vegetation patches being prepared and the finished ones waiting for the poll.
pub(crate) struct VegetationJobs {
    queue: PrepareQueue<PatchKey>,
    cache: ProductCache<PatchKey, VegetationProduct>,
    catalogue: Option<Arc<ProductCatalogue>>,
    // Started requests not yet running, oldest first, and the jobs running. Products prepared
    // ahead of need (`RENDER-32-N`) wait in their own queue, taken only when no request waits, so
    // a request waits behind the prepares running, never behind those still waiting.
    waiting: VecDeque<Waiting>,
    ahead: VecDeque<Waiting>,
    running: usize,
    // Of them, prepares (`RENDER-32-N`).
    running_ahead: usize,
    // The simulation ran (its speed above zero) when a snapshot was last taken: the observed
    // core state that paces the prepares.
    simulating: bool,
    // The snapshot taker is out.
    taking: bool,
}

// A started request waiting for a running place: what its job needs.
struct Waiting {
    request: PatchRequest,
    // Prepared ahead of need: waits behind the requests.
    ahead: bool,
    serial: u64,
    product: VegetationProduct,
    catalogue: Arc<ProductCatalogue>,
}

impl Default for VegetationJobs {
    fn default() -> Self {
        Self {
            queue: PrepareQueue::new(MAX_IN_FLIGHT),
            cache: ProductCache::new(CACHE_CAP_BYTES),
            catalogue: None,
            waiting: VecDeque::new(),
            ahead: VecDeque::new(),
            running: 0,
            running_ahead: 0,
            simulating: false,
            taking: false,
        }
    }
}

// Prepares running at once: every running place while the simulation is paused; one while it
// runs (observed under the core's lock by the last snapshot), so their snapshots and builds beside
// the simulation's frames keep the main thread's core waits at the control's (`RENDER-32-N` part
// 2: four at once lengthened them by a third on the 30k city at 1x).
fn ahead_running_limit(simulating: bool) -> usize {
    if simulating { 1 } else { MAX_RUNNING }
}

/// A finished poll: the request's key and serial, and its product (`None`: the inputs were
/// busy, ask again).
pub(crate) struct Delivered {
    /// The patch key.
    pub(crate) key: PatchKey,
    /// The request serial.
    pub(crate) serial: u64,
    /// The product; return it with [`VegetationJobs::recycle`].
    pub(crate) product: Option<VegetationProduct>,
}

impl VegetationJobs {
    /// Sets the renderer's mesh catalogue the products are built against.
    pub(crate) fn set_catalogue(&mut self, catalogue: ProductCatalogue) {
        self.catalogue = Some(Arc::new(catalogue));
    }

    /// Starts `key`'s request: its serial, product storage and the catalogue. `None` while the
    /// in-flight bound or the byte cap is reached, or before a catalogue is set.
    pub(super) fn start(
        &mut self,
        key: PatchKey,
    ) -> Option<(u64, VegetationProduct, Arc<ProductCatalogue>)> {
        let catalogue = Arc::clone(self.catalogue.as_ref()?);
        if self.cache.over_cap() {
            return None;
        }
        let serial = self.queue.start(key)?;
        Some((serial, self.cache.storage(), catalogue))
    }

    /// Lands a job's result for the poll.
    pub(super) fn land(&mut self, key: PatchKey, serial: u64, product: Option<VegetationProduct>) {
        let bytes = product.as_ref().map_or(0, VegetationProduct::bytes);
        self.cache.land(key, serial, product, bytes);
    }

    /// The oldest landed result of a live request; stale results met on the way are dropped
    /// (their storage kept for reuse). `O(1)` per result.
    pub(crate) fn poll(&mut self) -> Option<Delivered> {
        while let Some(landed) = self.cache.take() {
            if self.queue.finish(landed.key, landed.serial) {
                return Some(Delivered {
                    key: landed.key,
                    serial: landed.serial,
                    product: landed.product,
                });
            }
            if let Some(product) = landed.product {
                self.cache.recycle(product);
            }
        }
        None
    }

    /// Returns a delivered product's storage.
    pub(crate) fn recycle(&mut self, product: VegetationProduct) {
        self.cache.recycle(product);
    }

    /// Drops `key`'s live request: its result will not be delivered.
    pub(crate) fn cancel(&mut self, key: PatchKey) {
        self.queue.cancel(key);
    }

    /// Drops every live request (the renderer's reset). Waiting jobs land unbuilt when their
    /// turn comes.
    pub(crate) fn clear(&mut self) {
        self.queue.clear();
    }

    /// Jobs out (running or landed and not polled).
    pub(crate) fn in_flight(&self) -> usize {
        self.queue.in_flight()
    }
}

/// What a job needs to start the next waiting one: the core, the jobs and the pool's tracker.
#[derive(Clone)]
pub(crate) struct Spawner {
    /// The simulation core the snapshots are taken from.
    pub(crate) core: Arc<Mutex<SimCore>>,
    /// The core's turnstile (`CODE-25`): a thread waiting for the core holds it, so a prepare
    /// never takes or keeps the core while one waits.
    pub(crate) turnstile: Arc<crate::nodes::sim::core::CoreTurnstile>,
    /// The vegetation jobs.
    pub(crate) jobs: Arc<Mutex<VegetationJobs>>,
    /// The background pool's job tracker.
    pub(crate) async_jobs: Arc<super::super::async_jobs::AsyncJobs>,
}

/// Starts `request`, or returns `None` when the jobs refuse it for now. It runs on the
/// background pool once fewer than [`MAX_RUNNING`] jobs run, after every request waiting when
/// `ahead` (a product prepared ahead of need) is set. The worker takes the snapshot under
/// `try_lock` and never waits: a busy core puts the request back at the front of its queue, and
/// the next request, build or poll tries again. `O(1)` on the calling thread.
pub(crate) fn spawn_request(spawner: &Spawner, request: PatchRequest, ahead: bool) -> Option<u64> {
    let serial = {
        let mut jobs = spawner.jobs.lock().unwrap();
        let (serial, product, catalogue) = jobs.start(request.key)?;
        let waiting = Waiting {
            request,
            ahead,
            serial,
            product,
            catalogue,
        };
        if ahead {
            jobs.ahead.push_back(waiting);
        } else {
            jobs.waiting.push_back(waiting);
        }
        serial
    };
    pump(spawner);
    Some(serial)
}

// Starts the snapshot taker when requests wait and fewer than `MAX_RUNNING` jobs run. One taker
// at a time takes the core once, under `try_lock`, and snapshots every request it was handed in
// that one hold, so the vegetation's own jobs never collide on the core; it then starts one
// build job per snapshot. A busy core puts the batch back at the front of the queue; nothing
// waits. A waiting request that went stale lands unbuilt (the poll drops
// it and keeps its storage). `O(batch)` per start.
pub(crate) fn pump(spawner: &Spawner) {
    let batch: Vec<Waiting> = {
        let mut jobs = spawner.jobs.lock().unwrap();
        if jobs.taking {
            return;
        }
        let mut batch = Vec::new();
        // Prepares are taken only into the running places their limit leaves.
        let ahead_limit = ahead_running_limit(jobs.simulating);
        while jobs.running + batch.len() < MAX_RUNNING
            && let Some(waiting) = jobs.waiting.pop_front().or_else(|| {
                let ahead = batch.iter().filter(|w: &&Waiting| w.ahead).count();
                (jobs.running_ahead + ahead < ahead_limit)
                    .then(|| jobs.ahead.pop_front())
                    .flatten()
            })
        {
            if jobs.queue.is_live(waiting.request.key, waiting.serial) {
                batch.push(waiting);
            } else {
                jobs.land(waiting.request.key, waiting.serial, Some(waiting.product));
            }
        }
        if batch.is_empty() {
            return;
        }
        jobs.taking = true;
        jobs.running += batch.len();
        jobs.running_ahead += batch.iter().filter(|w| w.ahead).count();
        batch
    };
    let next = spawner.clone();
    spawner.async_jobs.spawn(move || {
        let mut inputs: Vec<Option<VegetationPatchInputs>> = Vec::with_capacity(batch.len());
        // A product prepared ahead of need (`RENDER-32-N`) is snapshotted only while no thread
        // waits for the core (the main thread's blocking queries and the simulation thread both
        // wait through the turnstile): the observed lock state, checked before the core is taken
        // and before each prepare's snapshot, so a prepare holds a contended core for at most one
        // snapshot and the rest of the batch goes back as busy. Requests are taken as before.
        let waited = |waiting: &Waiting| waiting.ahead && next.turnstile.has_waiter();
        let core = if batch.iter().all(waited) {
            None
        } else {
            SimulationNode::try_lock_shared_core(&next.core)
        };
        // The simulation's speed as the core shows it (paces the prepares; set below, with the
        // jobs' lock, after the core is freed).
        let mut simulating = None;
        if let Some(mut core) = core {
            simulating = Some(core.time.speed_multiplier > 0.0);
            for waiting in &batch {
                if waited(waiting) {
                    break;
                }
                let request = &waiting.request;
                inputs.push(VegetationPatchInputs::take(
                    &mut core,
                    request.origin,
                    request.span,
                ));
            }
        }
        inputs.resize_with(batch.len(), || None);
        let mut busy = Vec::new();
        for (waiting, inputs) in batch.into_iter().zip(inputs) {
            match inputs {
                Some(inputs) => build(&next, waiting, inputs),
                None => busy.push(waiting),
            }
        }
        let retry = busy.is_empty();
        {
            let mut jobs = next.jobs.lock().unwrap();
            // A busy batch goes back to the front of the queue, in order, without waiting: the
            // next event that pumps (a build job ending, a request or the renderer's poll) tries
            // the core again. It does not pump itself, so a held core is not spun on.
            jobs.running -= busy.len();
            jobs.running_ahead -= busy.iter().filter(|w| w.ahead).count();
            if let Some(simulating) = simulating {
                jobs.simulating = simulating;
            }
            for waiting in busy.into_iter().rev() {
                if waiting.ahead {
                    jobs.ahead.push_front(waiting);
                } else {
                    jobs.waiting.push_front(waiting);
                }
            }
            jobs.taking = false;
        }
        if retry {
            pump(&next);
        }
    });
}

// Builds one snapshot's product on the background pool, sequentially, and lands it.
fn build(spawner: &Spawner, waiting: Waiting, inputs: VegetationPatchInputs) {
    let next = spawner.clone();
    spawner.async_jobs.spawn(move || {
        let Waiting {
            request,
            ahead,
            serial,
            mut product,
            catalogue,
        } = waiting;
        prepare_patch(&inputs, &request, &catalogue, &mut product);
        product.origin = [request.origin.x, request.origin.y];
        product.span = request.span;
        {
            let mut jobs = next.jobs.lock().unwrap();
            jobs.land(request.key, serial, Some(product));
            jobs.running -= 1;
            if ahead {
                jobs.running_ahead -= 1;
            }
        }
        pump(&next);
    });
}

/// Builds `request`'s product from `world` into `product`: the three queries the renderer
/// asked of the core one by one before (`RENDER-32-C`'s inputs), then the product. Equal to the
/// product of the synchronous queries on the same world, bitwise.
pub(crate) fn prepare_patch(
    world: &impl VegetationWorld,
    request: &PatchRequest,
    catalogue: &ProductCatalogue,
    product: &mut VegetationProduct,
) {
    let PatchRequest {
        origin,
        span,
        understory,
        near_band,
        ..
    } = *request;
    let placements = decorative_tree_patch(world, origin, span, understory);
    let margin = STAND_RADIUS_TEXELS as f32 * land_cover::TEXEL_M;
    let (bytes, width, height, bounds) =
        Coverage::build(world, origin - Vector2::splat(margin), span + 2.0 * margin)
            .stand_parts(STAND_RADIUS_TEXELS);
    // The distant hedge: drawn only outside the near band, as the renderer asked for it.
    let runs = if near_band {
        Vec::new()
    } else {
        hedge_runs::hedge_runs(world, origin, span)
    };
    let inputs = ProductInputs {
        placements: &placements,
        cover: (!bytes.is_empty()).then_some(StandCover {
            bytes: &bytes,
            width: width as i64,
            height: height as i64,
            bounds,
        }),
        runs: &runs,
        origin: [origin.x, origin.y],
        near_band,
        caster: request.caster,
        settings: request.settings,
    };
    build_product(&inputs, catalogue, product);
}

/// The land cover epoch and, when it differs from `known`, every vegetation revision that is
/// not zero by terrain patch in `out` (cleared first), without waiting for the core: `None` while
/// the simulation holds it. `O(1)` unchanged, `O(edited patches)` when the epoch moved.
pub(crate) fn try_generations(
    core: &Mutex<SimCore>,
    known: i64,
    out: &mut Vec<([i32; 2], i64)>,
) -> Option<i64> {
    let core = SimulationNode::try_lock_shared_core(core)?;
    let epoch = core
        .terrain_payload_generation_counter
        .wrapping_add(core.vegetation_edits.epoch()) as i64;
    out.clear();
    if epoch != known {
        out.extend(
            core.vegetation_edits
                .patch_generations()
                .map(|(key, generation)| ([(key >> 32) as i32, key as u32 as i32], generation as i64)),
        );
    }
    Some(epoch)
}

/// The vegetation residency's jobs (`RENDER-32-F`): requests, cancels and the core's
/// vegetation revisions, all without waiting on the main thread.
impl PatchJobs for Spawner {
    fn request(&mut self, order: &PatchOrder, settings: &ResidencySettings) -> Option<u64> {
        let request = PatchRequest {
            key: order.key,
            origin: Vector2::new(order.origin[0], order.origin[1]),
            span: order.span,
            understory: order.bands.understory,
            near_band: order.bands.near_band,
            caster: order.bands.caster,
            settings: settings.product,
        };
        spawn_request(self, request, order.ahead)
    }

    fn cancel(&mut self, key: PatchKey) {
        self.jobs.lock().unwrap().cancel(key);
    }

    fn clear(&mut self) {
        self.jobs.lock().unwrap().clear();
    }

    fn generations(&mut self, known: i64, table: &mut Vec<([i32; 2], i64)>) -> Option<i64> {
        try_generations(&self.core, known, table)
    }
}

/// Counts one synchronous query on the main thread.
pub(super) fn count_main_thread_query() {
    MAIN_THREAD_QUERIES.fetch_add(1, Ordering::Relaxed);
}

/// The synchronous queries counted so far.
pub(super) fn main_thread_queries() -> u64 {
    MAIN_THREAD_QUERIES.load(Ordering::Relaxed)
}
