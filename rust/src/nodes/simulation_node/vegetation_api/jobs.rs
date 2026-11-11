// SPDX-License-Identifier: GPL-2.0-only

//! Vegetation patches prepared on the background pool (`RENDER-32-D`): the renderer requests a
//! patch by key with the bands it chose; a worker takes the patch's
//! [`VegetationPatchInputs`] under `try_lock`, runs the scatter, the stand cover, the hedge
//! runs and the product off the lock, and lands the product in the cache; the renderer polls
//! finished products and installs them. Nothing of the query runs on Godot's main thread.

use super::land_cover::Coverage;
use super::world::VegetationPatchInputs;
use super::*;
use crate::nodes::sim::render::stream::cache::ProductCache;
use crate::nodes::sim::render::stream::prepare::PrepareQueue;
use crate::nodes::sim::render::vegetation_product::{
    ProductCatalogue, ProductInputs, ProductSettings, StandCover, VegetationProduct, build_product,
};
use std::sync::atomic::{AtomicU64, Ordering};

/// Most patch jobs out at once, from the request until the poll takes the result: twice the
/// renderer's per-frame request budget (16 sub-patches), so a frame's requests never wait for
/// the last frame's to be polled.
const MAX_IN_FLIGHT: usize = 32;
/// Bytes of finished products the cache may hold before no job starts (`docs/terrain.md`
/// "Streaming core": 64 MB a layer).
const CACHE_CAP_BYTES: usize = 64 << 20;

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
}

impl Default for VegetationJobs {
    fn default() -> Self {
        Self {
            queue: PrepareQueue::new(MAX_IN_FLIGHT),
            cache: ProductCache::new(CACHE_CAP_BYTES),
            catalogue: None,
        }
    }
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

    /// Drops every live request (the renderer's reset).
    pub(crate) fn clear(&mut self) {
        self.queue.clear();
    }

    /// Jobs out (running or landed and not polled).
    pub(crate) fn in_flight(&self) -> usize {
        self.queue.in_flight()
    }
}

/// Starts `request` on the background pool, or returns `None` when the jobs refuse it for now.
/// The worker takes the snapshot under `try_lock`: a busy core lands a `None` product.
pub(crate) fn spawn_request(
    core: &Arc<Mutex<SimCore>>,
    jobs: &Arc<Mutex<VegetationJobs>>,
    async_jobs: &Arc<super::super::async_jobs::AsyncJobs>,
    request: PatchRequest,
) -> Option<u64> {
    let (serial, mut product, catalogue) = jobs.lock().unwrap().start(request.key)?;
    let (core, jobs) = (Arc::clone(core), Arc::clone(jobs));
    async_jobs.spawn(move || {
        let inputs = SimulationNode::try_lock_shared_core(&core).and_then(|mut core| {
            VegetationPatchInputs::take(&mut core, request.origin, request.span)
        });
        let built = inputs.map(|inputs| {
            prepare_patch(&inputs, &request, &catalogue, &mut product);
            product
        });
        jobs.lock().unwrap().land(request.key, serial, built);
    });
    Some(serial)
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

/// Counts one synchronous query on the main thread.
pub(super) fn count_main_thread_query() {
    MAIN_THREAD_QUERIES.fetch_add(1, Ordering::Relaxed);
}

/// The synchronous queries counted so far.
pub(super) fn main_thread_queries() -> u64 {
    MAIN_THREAD_QUERIES.load(Ordering::Relaxed)
}
