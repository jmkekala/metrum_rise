// SPDX-License-Identifier: GPL-2.0-only

//! The vegetation's residency (`RENDER-32-F`, `docs/terrain.md` "Streaming core" 5.3, 5.6),
//! moved from `vegetation.gd` with its rules and constants unchanged: which patches are wanted,
//! on which of the two grids, with which bands, when one is stale, what is requested, and what
//! stays drawn meanwhile. It drives the install layer ([`super::VegetationLayer`]) and the
//! background jobs ([`PatchJobs`]); `vegetation.gd` keeps the catalogue and the settings.
//!
//! - **Tiers.** A resident terrain block (pushed by `terrain.gd`) is carried on the fine grid
//!   (`subdivision`² patches) when its nearest corner is within the fine radius, else as one
//!   coarse patch within the canopy's far range; a fine patch is wanted within the far range of
//!   its centre. The tiers live in a [`RingSet`] over the terrain grid, evaluated where the script
//!   evaluated them: on a frame whose camera changed its fine-grid cell or whose terrain
//!   residency changed, at that frame's camera. A crossing re-evaluates the ring boundary blocks
//!   only, a jump the old and new sets.
//! - **Stand-ins.** A patch that leaves the wanted set stays drawn until the wanted patches over
//!   its area are installed, then is hidden into the cache (within the far range) or freed. While
//!   a block's coarse patch draws, the install layer keeps the block's fine patches hidden, so an
//!   area draws from one grid tier in every frame.
//! - **Bands.** Understory, near canopy and shadow caster by patch-centre distance, with
//!   hysteresis around the installed record. A drawn patch waits in [`TravelBuckets`] for the
//!   travel at which a band can first change, so a moving frame checks only the patches due.
//! - **Staleness**, pushed by edits: a terrain commit names its patch, and a move of the land
//!   cover epoch names the patches whose vegetation revision changed; only their patches are
//!   checked.
//! - **Requests**: a queue, missing patches before drawn ones and nearest first as of the last
//!   evaluation, taken up to the upload budget a frame.
//!
//! Costs: a still frame `O(1)` (one `try_lock` read of the land cover epoch); a crossing
//! `O(ring boundary blocks + changes + queue log queue)`; a moving frame `O(patches due + buckets
//! passed)`; an edit `O(patches of the edited blocks)`; a request `O(1)`. Nothing allocates once
//! the storage has grown.

mod rules;

pub(crate) use rules::{Bands, ResidencySettings};

use super::{PatchKey, VegetationLayer};
use crate::nodes::sim::render::stream::backend::RenderBackend;
use crate::nodes::sim::render::stream::rings::{Cell, Lattice, RING_SLACK_M, RingBand, RingSet, TravelBuckets};
use rules::{HALF_DIAGONAL, camera_cell, distance_squared, key_centre, key_distance, key_origin, key_span, owner};
use std::collections::{HashMap, HashSet, VecDeque};

type Map<K, V> = HashMap<K, V, foldhash::fast::FixedState>;
type Set<K> = HashSet<K, foldhash::fast::FixedState>;

// Travel per bucket of the band checks, metres.
const BAND_BUCKET_M: f64 = 8.0;

/// A patch the jobs are asked to prepare.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PatchOrder {
    /// The patch.
    pub(crate) key: PatchKey,
    /// Its minimum corner, world metres.
    pub(crate) origin: [f32; 2],
    /// Its side, metres.
    pub(crate) span: f32,
    /// The bands it is built with.
    pub(crate) bands: Bands,
}

/// The background jobs and the core's vegetation revisions, as the residency uses them.
pub(crate) trait PatchJobs {
    /// Starts `order` with `settings`' draw distances; its serial, or `None` while the jobs are
    /// full (ask again next frame).
    fn request(&mut self, order: &PatchOrder, settings: &ResidencySettings) -> Option<u64>;
    /// Makes the live request of `key` stale.
    fn cancel(&mut self, key: PatchKey);
    /// Makes every live request stale.
    fn clear(&mut self);
    /// The land cover epoch without waiting for the core (`None`: busy, ask next frame) and, when
    /// it differs from `known`, every vegetation revision that is not zero, by terrain patch,
    /// in `table` (cleared first).
    fn generations(&mut self, known: i64, table: &mut Vec<(Cell, i64)>) -> Option<i64>;
}

/// What a drawn patch was built with (`vegetation.gd`'s patch record).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Record {
    /// Plants drawn.
    pub(crate) tree_count: i64,
    /// The owner's surface generation the request read (-1: none committed).
    pub(crate) surface: i64,
    /// The owner's vegetation revision the request read.
    pub(crate) vegetation: i64,
    /// Its bands.
    pub(crate) bands: Bands,
}

// A request in flight.
#[derive(Clone, Copy, Debug)]
struct Request {
    serial: u64,
    surface: i64,
    vegetation: i64,
    bands: Bands,
}

/// Where a record stands (tests, probes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Standing {
    /// Wanted and installed.
    Drawn,
    /// Not wanted, drawn until the wanted patches over its area are installed.
    StandIn,
    /// Hidden for a camera moving back.
    Cached,
}

// A terrain block's tier and, on the fine grid, its wanted patches (bit `column x subdivision +
// row`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct BlockState {
    tier: Tier,
    fine: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tier {
    #[default]
    Out,
    Coarse,
    Fine,
}

/// One frame's work and the layer's totals (the perf channel, tests).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ResidencyWork {
    /// The frame evaluated the tiers (a crossing or a residency change).
    pub(crate) evaluated: bool,
    /// Ring cells visited.
    pub(crate) ring_cells: usize,
    /// Drawn patches whose bands were checked.
    pub(crate) band_checks: usize,
    /// Buckets the travel passed.
    pub(crate) buckets: usize,
    /// Patches whose staleness was checked.
    pub(crate) stale_checks: usize,
    /// Cached patches checked for freeing.
    pub(crate) cache_checks: usize,
    /// Patches requested.
    pub(crate) requests: usize,
    /// Installs taken.
    pub(crate) installs: usize,
    /// Products that came back busy.
    pub(crate) busy: usize,
    /// Stand-ins put away.
    pub(crate) released: usize,
}

/// The vegetation's wanted set, tiers, bands, staleness and requests.
pub(crate) struct VegetationResidency {
    settings: Option<ResidencySettings>,
    terrain_span: f64,
    enabled: bool,
    evaluated: bool,
    ring: RingSet<BlockState>,
    resident: Set<Cell>,
    // Blocks whose residency changed, and whether terrain's residency revision moved, since the
    // last evaluation.
    resident_changed: Vec<Cell>,
    revised: bool,
    last_cell: Option<Cell>,
    last_eye: Option<[f32; 3]>,
    camera: Option<[f32; 2]>,
    wanted: Set<PatchKey>,
    patches: Map<PatchKey, Record>,
    standins: Map<PatchKey, Record>,
    cache: Map<PatchKey, Record>,
    // Cached since the last evaluation, so not yet checked against its camera; and that camera.
    cache_unchecked: Vec<PatchKey>,
    cache_camera: Option<[f32; 2]>,
    queue: VecDeque<PatchKey>,
    queued: Set<PatchKey>,
    requested: Map<PatchKey, Request>,
    surface: Map<Cell, i64>,
    vegetation: Map<Cell, i64>,
    vegetation_next: Map<Cell, i64>,
    epoch: i64,
    table: Vec<(Cell, i64)>,
    dirty: Vec<Cell>,
    dirty_set: Set<Cell>,
    bands_due: TravelBuckets<PatchKey>,
    scratch: Vec<PatchKey>,
    entering: Vec<PatchKey>,
    leaving: Vec<PatchKey>,
    // Wanted patches retired out of the drawn set (tests), restored as the next evaluation finds
    // them, as the script's pass over every wanted key did.
    retired: Vec<PatchKey>,
    tree_count: i64,
    generated: u64,
    generation_us_max: i64,
    work: ResidencyWork,
}

impl Default for VegetationResidency {
    fn default() -> Self {
        Self {
            settings: None,
            terrain_span: 0.0,
            enabled: true,
            evaluated: false,
            ring: RingSet::new(Lattice {
                origin: [0.0; 2],
                cell: 1.0,
            }),
            resident: Set::default(),
            resident_changed: Vec::new(),
            revised: false,
            last_cell: None,
            last_eye: None,
            camera: None,
            wanted: Set::default(),
            patches: Map::default(),
            standins: Map::default(),
            cache: Map::default(),
            cache_unchecked: Vec::new(),
            cache_camera: None,
            queue: VecDeque::new(),
            queued: Set::default(),
            requested: Map::default(),
            surface: Map::default(),
            vegetation: Map::default(),
            vegetation_next: Map::default(),
            epoch: -1,
            table: Vec::new(),
            dirty: Vec::new(),
            dirty_set: Set::default(),
            bands_due: TravelBuckets::new(BAND_BUCKET_M),
            scratch: Vec::new(),
            entering: Vec::new(),
            leaving: Vec::new(),
            retired: Vec::new(),
            tree_count: 0,
            generated: 0,
            generation_us_max: 0,
            work: ResidencyWork::default(),
        }
    }
}

impl VegetationResidency {
    /// Starts over for a loaded world with `settings` (`rebuild_from_simulation_state`): every
    /// patch freed, every request dropped; the terrain's residency and surface generations, which
    /// `terrain.gd` pushes as they change, are kept.
    pub(crate) fn reset(
        &mut self,
        settings: ResidencySettings,
        layer: &mut VegetationLayer,
        backend: &mut impl RenderBackend,
        jobs: &mut impl PatchJobs,
    ) {
        layer.clear(backend);
        jobs.clear();
        self.settings = Some(settings);
        self.evaluated = false;
        self.ring.clear();
        self.resident_changed.clear();
        self.revised = false;
        self.last_cell = None;
        self.camera = None;
        self.wanted.clear();
        self.patches.clear();
        self.standins.clear();
        self.cache.clear();
        self.cache_unchecked.clear();
        self.cache_camera = None;
        self.queue.clear();
        self.queued.clear();
        self.requested.clear();
        self.retired.clear();
        self.vegetation.clear();
        self.epoch = -1;
        self.dirty.clear();
        self.dirty_set.clear();
        self.bands_due.clear();
        self.tree_count = 0;
        self.generated = 0;
        self.generation_us_max = 0;
    }

    /// The terrain's render patch side (`get_render_patch_span_m`); 0 holds the residency.
    pub(crate) fn set_terrain_span(&mut self, span: f64) {
        if span != self.terrain_span {
            self.terrain_span = span;
            self.evaluated = false;
        }
    }

    /// Shows or stops the layer's residency (`enabled`): a disabled layer does no work.
    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// A terrain patch became resident or left (`terrain.gd`). `O(1)`.
    pub(crate) fn set_resident(&mut self, block: Cell, resident: bool) {
        let changed = if resident {
            self.resident.insert(block)
        } else {
            self.resident.remove(&block)
        };
        if changed {
            self.resident_changed.push(block);
        }
    }

    /// The terrain's residency revision moved: the next frame evaluates the tiers.
    pub(crate) fn revise_residency(&mut self) {
        self.revised = true;
    }

    /// Every terrain patch left (`terrain.gd` cleared its patches).
    pub(crate) fn clear_resident(&mut self) {
        for block in self.resident.drain() {
            self.resident_changed.push(block);
        }
        self.surface.clear();
        self.revised = true;
    }

    /// A terrain patch committed a payload of surface generation `generation` (-1: it has none
    /// any more): its patches are checked for staleness. `O(1)`.
    pub(crate) fn set_surface_generation(&mut self, block: Cell, generation: i64) {
        if generation < 0 {
            self.surface.remove(&block);
        } else {
            self.surface.insert(block, generation);
        }
        self.mark_dirty(block);
    }

    /// The frame's work.
    pub(crate) fn work(&self) -> ResidencyWork {
        self.work
    }

    /// Whether anything waits: a first evaluation, a queued or a requested patch
    /// (`has_pending_work`).
    pub(crate) fn has_pending_work(&self) -> bool {
        self.enabled
            && self.settings.is_some()
            && (!self.evaluated || !self.queue.is_empty() || !self.requested.is_empty())
    }

    /// Counts for `vegetation.gd`'s `metrics()` and the benchmark: wanted, drawn, stand-ins,
    /// cached, queued, requested, fine and coarse drawn, plants drawn, installs, the longest
    /// install (microseconds).
    pub(crate) fn counts(&self) -> [i64; 11] {
        let fine = self.patches.keys().filter(|key| key[2] > 1).count();
        [
            self.wanted.len() as i64,
            self.patches.len() as i64,
            self.standins.len() as i64,
            self.cache.len() as i64,
            self.queue.len() as i64,
            self.requested.len() as i64,
            fine as i64,
            (self.patches.len() - fine) as i64,
            self.tree_count,
            self.generated as i64,
            self.generation_us_max,
        ]
    }

    /// The wanted patches (tests, the benchmark's wanted-to-drawn).
    pub(crate) fn wanted(&self) -> impl Iterator<Item = PatchKey> + '_ {
        self.wanted.iter().copied()
    }

    /// The stream benchmark's drawn probe (`RENDER-32-N`): every wanted patch with its ground
    /// rectangle `[min x, min z, max x, max z]` and whether a hidden version of it is cached (it
    /// waits for its install only). `O(wanted)`.
    pub(crate) fn probe_wanted(&self) -> impl Iterator<Item = (PatchKey, [f32; 4], bool)> + '_ {
        let world = self.settings.map_or([0.0; 2], |settings| settings.world);
        self.wanted.iter().map(move |&key| {
            let origin = key_origin(key, self.terrain_span, world);
            let span = key_span(key, self.terrain_span) as f32;
            (key, [origin[0], origin[1], origin[0] + span, origin[1] + span], self.cache.contains_key(&key))
        })
    }

    /// Every record and where it stands (tests, probes). `O(records)`.
    pub(crate) fn records(&self) -> impl Iterator<Item = (PatchKey, Record, Standing)> + '_ {
        let drawn = self.patches.iter().map(|(k, r)| (*k, *r, Standing::Drawn));
        let standins = self.standins.iter().map(|(k, r)| (*k, *r, Standing::StandIn));
        let cached = self.cache.iter().map(|(k, r)| (*k, *r, Standing::Cached));
        drawn.chain(standins).chain(cached)
    }

    /// Whether `key` waits in the request queue.
    pub(crate) fn is_queued(&self, key: PatchKey) -> bool {
        self.queued.contains(&key)
    }

    /// Whether a drawn patch's owner moved past what it was built with (`_is_patch_stale`).
    #[cfg(test)]
    pub(crate) fn is_stale(&self, key: PatchKey) -> bool {
        self.patches
            .get(&key)
            .is_some_and(|record| self.stale(key, record))
    }

    /// The bands `key` takes from `camera` with its record's hysteresis (tests, the appearance
    /// test's deliveries). `None` before the settings and the terrain span.
    pub(crate) fn bands_for(&self, key: PatchKey, camera: Option<[f32; 2]>) -> Option<Bands> {
        let settings = self.settings?;
        if self.terrain_span <= 0.0 {
            return None;
        }
        let distance = camera.map_or(-1.0, |camera| {
            key_distance(key, self.terrain_span, settings.world, camera)
        });
        let span = key_span(key, self.terrain_span);
        let previous = self.patches.get(&key).map(|record| record.bands);
        Some(settings.bands(distance, span, previous))
    }

    /// Queues `key` for a request check (probes: an obsolete band request must not rebuild).
    pub(crate) fn queue(&mut self, key: PatchKey) {
        self.queue_patch(key);
    }

    /// Takes a drawn patch out of the drawn set as if it had left it (hidden into the cache, or
    /// freed out of range; `_retire_patch`), for tests of the restore path.
    pub(crate) fn retire(
        &mut self,
        key: PatchKey,
        layer: &mut VegetationLayer,
        backend: &mut impl RenderBackend,
    ) {
        let (Some(record), Some(camera)) = (self.patches.remove(&key), self.camera) else {
            return;
        };
        self.tree_count -= record.tree_count;
        self.bands_due.remove(key);
        self.put_away(key, record, camera, layer, backend);
        self.retired.push(key);
    }

    /// The frame's residency, once a frame after the camera moved (`vegetation.gd`'s
    /// `_process`): evaluate the tiers on a crossing or residency change, check the bands of the
    /// patches due, the staleness of the edited ones, request up to the budget and put away the
    /// stand-ins whose area is built. `eye` is the camera (`None`: no camera, nothing to do).
    pub(crate) fn frame(
        &mut self,
        eye: Option<[f32; 3]>,
        layer: &mut VegetationLayer,
        backend: &mut impl RenderBackend,
        jobs: &mut impl PatchJobs,
    ) {
        self.work = ResidencyWork::default();
        let (Some(settings), true, Some(eye)) = (self.settings, self.enabled, eye) else {
            return;
        };
        if self.terrain_span <= 0.0 {
            return;
        }
        let camera = [eye[0], eye[2]];
        self.camera = Some(camera);
        let fresh = self.refresh_generations(jobs);
        let span = self.terrain_span / f64::from(settings.subdivision);
        let cell = camera_cell(camera, span);
        let moved = self.last_eye.map_or(0.0, |last| travel(last, eye));
        self.last_eye = Some(eye);
        if !self.evaluated || self.revised || self.last_cell != Some(cell) {
            self.evaluate(&settings, camera, layer, backend, jobs);
            self.last_cell = Some(cell);
        }
        if moved > 0.0 {
            self.check_due_bands(&settings, camera, moved);
        }
        if fresh && !self.dirty.is_empty() {
            self.check_stale_owners(&settings);
        }
        self.request(&settings, camera, layer, jobs);
        if !self.standins.is_empty() {
            self.release_standins(&settings, camera, layer, backend);
        }
    }

    /// Takes the install layer's events after an install pass: an installed patch becomes drawn
    /// with its request's record (queued again if the camera or an edit moved since); a busy one
    /// is queued again; then the stand-ins whose area is now built are put away, in the same
    /// pass, so a tier change draws one tier in every frame.
    pub(crate) fn take_installs(
        &mut self,
        layer: &mut VegetationLayer,
        backend: &mut impl RenderBackend,
    ) {
        let Some(settings) = self.settings else {
            layer.clear_events();
            return;
        };
        let mut installs = 0;
        let mut at = 0;
        while at + super::EVENT_LANES <= layer.events().len() {
            let event: [i64; super::EVENT_LANES] = layer.events()[at..at + super::EVENT_LANES]
                .try_into()
                .expect("an event's lanes");
            at += super::EVENT_LANES;
            let key = [event[0] as i32, event[1] as i32, event[2] as i32];
            // A product handed over without a request (tests, previews) is not the residency's.
            let Some(request) = self
                .requested
                .get(&key)
                .copied()
                .filter(|request| request.serial == event[3] as u64)
            else {
                continue;
            };
            self.requested.remove(&key);
            if event[4] < 0 {
                self.work.busy += 1;
                self.queue_patch(key);
                continue;
            }
            if let Some(old) = self.patches.get(&key) {
                self.tree_count -= old.tree_count;
            }
            let record = Record {
                tree_count: event[4],
                surface: request.surface,
                vegetation: request.vegetation,
                bands: request.bands,
            };
            self.patches.insert(key, record);
            self.tree_count += record.tree_count;
            self.generated += 1;
            self.generation_us_max = self.generation_us_max.max(event[5]);
            installs += 1;
            if let Some(camera) = self.camera
                && self.stale(key, &record) | self.check_bands(&settings, key, camera)
            {
                self.queue_patch(key);
            }
        }
        layer.clear_events();
        self.work.installs += installs;
        if installs > 0 && !self.standins.is_empty() && let Some(camera) = self.camera {
            self.release_standins(&settings, camera, layer, backend);
        }
    }

    // Brings the vegetation revisions up to date without waiting for the core; the owners whose
    // revision moved are marked for the staleness check. False while the core is busy.
    fn refresh_generations(&mut self, jobs: &mut impl PatchJobs) -> bool {
        let Some(epoch) = jobs.generations(self.epoch, &mut self.table) else {
            return false;
        };
        if epoch == self.epoch {
            return true;
        }
        self.epoch = epoch;
        let mut next = std::mem::take(&mut self.vegetation_next);
        next.clear();
        let mut table = std::mem::take(&mut self.table);
        for &(block, generation) in &table {
            next.insert(block, generation);
            if self.vegetation.get(&block) != Some(&generation) {
                self.mark_dirty(block);
            }
        }
        table.clear();
        self.table = table;
        let Self {
            vegetation,
            dirty,
            dirty_set,
            ..
        } = self;
        for block in vegetation.keys() {
            if !next.contains_key(block) && dirty_set.insert(*block) {
                dirty.push(*block);
            }
        }
        self.vegetation_next = std::mem::replace(&mut self.vegetation, next);
        true
    }

    fn mark_dirty(&mut self, block: Cell) {
        if self.dirty_set.insert(block) {
            self.dirty.push(block);
        }
    }

    fn owner_generations(&self, key: PatchKey) -> (i64, i64) {
        let block = owner(key);
        (
            self.surface.get(&block).copied().unwrap_or(-1),
            self.vegetation.get(&block).copied().unwrap_or(0),
        )
    }

    /// Whether a record's owner moved past what it was built with (`_is_patch_stale`): -1 (no
    /// payload committed) keeps the current scatter.
    pub(crate) fn stale(&self, key: PatchKey, record: &Record) -> bool {
        let (surface, vegetation) = self.owner_generations(key);
        (surface >= 0 && surface != record.surface) || vegetation != record.vegetation
    }

    // `_queue_patch`: a patch that already draws waits behind every patch that draws nothing.
    fn queue_patch(&mut self, key: PatchKey) {
        if self.queued.insert(key) {
            if self.patches.contains_key(&key) {
                self.queue.push_front(key);
            } else {
                self.queue.push_back(key);
            }
        }
    }

    // Checks a drawn patch's bands at `camera` and buckets it for the next check. Returns whether
    // they changed (`_bands_changed`).
    fn check_bands(&mut self, settings: &ResidencySettings, key: PatchKey, camera: [f32; 2]) -> bool {
        let Some(record) = self.patches.get(&key) else {
            return false;
        };
        let distance = key_distance(key, self.terrain_span, settings.world, camera);
        let span = key_span(key, self.terrain_span);
        let held = record.bands;
        self.work.band_checks += 1;
        self.bands_due
            .put(key, settings.band_margin(distance, span, held));
        settings.bands(distance, span, Some(held)) != held
    }

    fn check_due_bands(&mut self, settings: &ResidencySettings, camera: [f32; 2], moved: f64) {
        let mut due = std::mem::take(&mut self.scratch);
        due.clear();
        // At most every bucketed patch is due: sized once, a later frame grows nothing.
        due.reserve(self.bands_due.len());
        self.work.buckets += self.bands_due.advance(moved, &mut due);
        for &key in &due {
            if self.check_bands(settings, key, camera) {
                self.queue_patch(key);
            }
        }
        due.clear();
        self.scratch = due;
    }

    // Every drawn patch of the owners an edit moved (`O(subdivision² + 1)` each).
    fn check_stale_owners(&mut self, settings: &ResidencySettings) {
        let divisor = settings.subdivision;
        let mut dirty = std::mem::take(&mut self.dirty);
        for &block in &dirty {
            self.dirty_set.remove(&block);
            let keys = std::iter::once([block[0], block[1], 1]).chain((0..divisor * divisor).map(
                |at| [block[0] * divisor + at / divisor, block[1] * divisor + at % divisor, divisor],
            ));
            for key in keys {
                let Some(record) = self.patches.get(&key) else {
                    continue;
                };
                self.work.stale_checks += 1;
                if self.stale(key, record) {
                    self.queue_patch(key);
                }
            }
        }
        dirty.clear();
        self.dirty = dirty;
    }

    // The tiers at `camera`: the ring's boundary blocks and the blocks whose residency changed,
    // then the script's residency pass over the keys that left and entered the wanted set.
    fn evaluate(
        &mut self,
        settings: &ResidencySettings,
        camera: [f32; 2],
        layer: &mut VegetationLayer,
        backend: &mut impl RenderBackend,
        jobs: &mut impl PatchJobs,
    ) {
        self.work.evaluated = true;
        let world = settings.world;
        let terrain_span = self.terrain_span;
        let lattice = Lattice {
            origin: [-f64::from(world[0]) * 0.5, -f64::from(world[1]) * 0.5],
            cell: terrain_span,
        };
        let full = !self.evaluated || self.ring.lattice() != lattice;
        if self.ring.lattice() != lattice {
            // A new terrain grid: every key of the old one leaves.
            for (block, state) in self.ring.members() {
                diff_keys(
                    settings.subdivision,
                    block,
                    state,
                    BlockState::default(),
                    &mut self.leaving,
                    &mut self.entering,
                );
            }
            self.ring = RingSet::new(lattice);
        }
        self.evaluated = true;
        self.revised = false;
        let point = [f64::from(camera[0]), f64::from(camera[1])];
        let moved = self.ring.point().map_or(f64::INFINITY, |last| {
            ((last[0] - point[0]).powi(2) + (last[1] - point[1]).powi(2)).sqrt()
        });
        let span = terrain_span / f64::from(settings.subdivision);
        let fine_radius = settings.fine_radius();
        let far = settings.product.far_m;
        let block_reach = terrain_span * HALF_DIAGONAL;
        let bands = [
            RingBand {
                radius: fine_radius + block_reach,
                reach: 0.0,
            },
            RingBand {
                radius: far + terrain_span,
                reach: 0.0,
            },
            // A fine block's patches cross the far range within a block's half-diagonal of it.
            RingBand {
                radius: far + span,
                reach: block_reach,
            },
        ];
        let fine_patches_reach = far + span - block_reach - moved - RING_SLACK_M;
        let band_count = if fine_patches_reach > fine_radius + 2.0 * block_reach {
            2
        } else {
            3
        };
        let Self {
            ring,
            resident,
            resident_changed,
            entering,
            leaving,
            ..
        } = self;
        ring.reset_visited();
        let evaluate = |block: Cell| block_state(settings, terrain_span, resident, block, camera);
        let mut changed = |block: Cell, old: BlockState, new: BlockState| {
            diff_keys(settings.subdivision, block, old, new, leaving, entering);
        };
        ring.update(point, &bands[..band_count], full, evaluate, &mut changed);
        for &block in resident_changed.iter() {
            ring.refresh(block, evaluate, &mut changed);
        }
        resident_changed.clear();
        self.work.ring_cells += self.ring.visited();
        self.apply_wanted_changes(settings, camera, layer, backend, jobs);
    }

    // The script's residency pass over the changed keys, in its order: leaving patches become
    // stand-ins, the cache frees what is out of range, leaving requests are dropped, entering
    // patches come back from the stand-ins or the cache or are queued; then the queue is sorted.
    fn apply_wanted_changes(
        &mut self,
        settings: &ResidencySettings,
        camera: [f32; 2],
        layer: &mut VegetationLayer,
        backend: &mut impl RenderBackend,
        jobs: &mut impl PatchJobs,
    ) {
        let mut leaving = std::mem::take(&mut self.leaving);
        let mut entering = std::mem::take(&mut self.entering);
        let mut dropped_queued = false;
        for &key in &leaving {
            self.wanted.remove(&key);
            if let Some(record) = self.patches.remove(&key) {
                self.tree_count -= record.tree_count;
                self.bands_due.remove(key);
                self.standins.insert(key, record);
            }
            dropped_queued |= self.queued.remove(&key);
        }
        self.free_cache_out_of_range(settings, camera, layer, backend);
        if dropped_queued {
            let queued = &self.queued;
            self.queue.retain(|key| queued.contains(key));
        }
        for &key in &leaving {
            if self.requested.remove(&key).is_some() {
                jobs.cancel(key);
                layer.forget(key);
            }
        }
        entering.extend(
            self.retired
                .drain(..)
                .filter(|key| self.wanted.contains(key)),
        );
        for &key in &entering {
            self.wanted.insert(key);
            if self.patches.contains_key(&key) {
                continue;
            }
            let record = if let Some(record) = self.standins.remove(&key) {
                record
            } else if let Some(record) = self.cache.remove(&key) {
                layer.set_visible(backend, key, true);
                record
            } else {
                self.queue_patch(key);
                continue;
            };
            self.patches.insert(key, record);
            self.tree_count += record.tree_count;
            if self.stale(key, &record) | self.check_bands(settings, key, camera) {
                self.queue_patch(key);
            }
        }
        leaving.clear();
        entering.clear();
        self.leaving = leaving;
        self.entering = entering;
        self.sort_queue(settings, camera);
    }

    // Frees the cached patches the camera left behind: past the far range, or a fine patch past
    // the fine radius and a block and a half (`fine_keep`). Residency covers every direction, so
    // the cache only serves a camera moving back; keeping fine patches over the whole disc once
    // held dozens of nodes each, and every patch's instances take instance uniform slots of the
    // engine's 65,536, so a long flight overflowed the buffer and new trees failed to draw. A
    // cached patch passed the test at the last evaluation's camera, so only those within the
    // camera's move of a limit, and those cached since, are checked.
    fn free_cache_out_of_range(
        &mut self,
        settings: &ResidencySettings,
        camera: [f32; 2],
        layer: &mut VegetationLayer,
        backend: &mut impl RenderBackend,
    ) {
        let world = settings.world;
        let terrain_span = self.terrain_span;
        let divisor = settings.subdivision;
        let span = terrain_span / f64::from(divisor);
        let far = settings.product.far_m;
        let fine_keep = settings.fine_radius() + terrain_span * (1.0 + HALF_DIAGONAL);
        let moved = self.cache_camera.map_or(f64::INFINITY, |last| {
            f64::from(rules::distance(last, camera))
        });
        self.cache_camera = Some(camera);
        let mut candidates = std::mem::take(&mut self.scratch);
        candidates.clear();
        candidates.append(&mut self.cache_unchecked);
        if moved > fine_keep * 0.5 {
            candidates.extend(self.cache.keys().copied());
        } else if !self.cache.is_empty() {
            let point = [f64::from(camera[0]), f64::from(camera[1])];
            let origin = [-f64::from(world[0]) * 0.5, -f64::from(world[1]) * 0.5];
            let width = moved + RING_SLACK_M;
            for (cell, divisor, radius) in [
                (terrain_span, 1, far + terrain_span),
                (span, divisor, far + span),
                (span, divisor, fine_keep),
            ] {
                let lattice = Lattice { origin, cell };
                let cache = &self.cache;
                lattice.for_each_in_annulus(point, radius - width, radius + width, |at| {
                    let key = [at[0], at[1], divisor];
                    if cache.contains_key(&key) {
                        candidates.push(key);
                    }
                });
            }
        }
        for &key in &candidates {
            if !self.cache.contains_key(&key) {
                continue;
            }
            self.work.cache_checks += 1;
            let distance = key_distance(key, terrain_span, world, camera);
            if distance > far + key_span(key, terrain_span) || (key[2] > 1 && distance > fine_keep) {
                self.cache.remove(&key);
                layer.free(backend, key);
            }
        }
        candidates.clear();
        self.scratch = candidates;
    }

    // Missing patches last (they are taken from the back), farther before nearer, ties by key.
    fn sort_queue(&mut self, settings: &ResidencySettings, camera: [f32; 2]) {
        let (terrain_span, world) = (self.terrain_span, settings.world);
        let patches = &self.patches;
        let rank = |key: &PatchKey| {
            let missing = !patches.contains_key(key);
            let distance = distance_squared(key_centre(*key, terrain_span, world), camera);
            (missing, std::cmp::Reverse(distance.to_bits()), *key)
        };
        self.queue
            .make_contiguous()
            .sort_unstable_by_key(rank);
    }

    // `_upload_patch` up to the budget: a drawn patch neither stale nor out of its bands, or one
    // in flight with the same bands and generations, is not asked again; a refused request stays
    // queued and ends the frame's requests.
    fn request(
        &mut self,
        settings: &ResidencySettings,
        camera: [f32; 2],
        layer: &mut VegetationLayer,
        jobs: &mut impl PatchJobs,
    ) {
        let mut budget = settings.upload_budget;
        while budget > 0 {
            let Some(key) = self.queue.pop_back() else {
                break;
            };
            self.queued.remove(&key);
            let distance = key_distance(key, self.terrain_span, settings.world, camera);
            let span = key_span(key, self.terrain_span);
            let previous = self.patches.get(&key).copied();
            let bands = settings.bands(distance, span, previous.map(|record| record.bands));
            if let Some(record) = previous
                && !self.stale(key, &record)
                && bands == record.bands
            {
                continue;
            }
            let (surface, vegetation) = self.owner_generations(key);
            if let Some(request) = self.requested.get(&key)
                && request.surface == surface
                && request.vegetation == vegetation
                && request.bands == bands
            {
                continue;
            }
            // A product of an older request still waiting for its install is dropped first.
            layer.forget(key);
            let order = PatchOrder {
                key,
                origin: key_origin(key, self.terrain_span, settings.world),
                span: span as f32,
                bands,
            };
            let Some(serial) = jobs.request(&order, settings) else {
                self.queue_patch(key);
                break;
            };
            self.requested.insert(
                key,
                Request {
                    serial,
                    surface,
                    vegetation,
                    bands,
                },
            );
            budget -= 1;
            self.work.requests += 1;
        }
    }

    // `_release_standins`: every stand-in whose area the wanted patches now cover is put away.
    fn release_standins(
        &mut self,
        settings: &ResidencySettings,
        camera: [f32; 2],
        layer: &mut VegetationLayer,
        backend: &mut impl RenderBackend,
    ) {
        let mut keys = std::mem::take(&mut self.scratch);
        keys.clear();
        keys.extend(self.standins.keys().copied());
        keys.sort_unstable();
        for &key in &keys {
            if self.area_built(settings, key) {
                let record = self.standins.remove(&key).expect("a stand-in");
                self.put_away(key, record, camera, layer, backend);
                self.work.released += 1;
            }
        }
        keys.clear();
        self.scratch = keys;
    }

    // `_area_built`: no wanted patch over this key's area still waits for its first install.
    fn area_built(&self, settings: &ResidencySettings, key: PatchKey) -> bool {
        let divisor = settings.subdivision;
        let waiting = |key: PatchKey| self.wanted.contains(&key) && !self.patches.contains_key(&key);
        if key[2] == 1 {
            return !(0..divisor * divisor).any(|at| {
                waiting([key[0] * divisor + at / divisor, key[1] * divisor + at % divisor, divisor])
            });
        }
        let block = owner(key);
        !waiting([block[0], block[1], 1])
    }

    // `_put_away`: hidden into the cache within the far range, else freed.
    fn put_away(
        &mut self,
        key: PatchKey,
        record: Record,
        camera: [f32; 2],
        layer: &mut VegetationLayer,
        backend: &mut impl RenderBackend,
    ) {
        let settings = self.settings.expect("settings");
        let distance = key_distance(key, self.terrain_span, settings.world, camera);
        if distance <= settings.product.far_m + key_span(key, self.terrain_span) {
            layer.set_visible(backend, key, false);
            self.cache.insert(key, record);
            self.cache_unchecked.push(key);
        } else {
            layer.free(backend, key);
        }
    }
}

// The straight distance between two eyes, metres.
fn travel(a: [f32; 3], b: [f32; 3]) -> f64 {
    let [dx, dy, dz] = [0, 1, 2].map(|axis| f64::from(b[axis]) - f64::from(a[axis]));
    (dx * dx + dy * dy + dz * dz).sqrt()
}

// A block's tier at `camera` (`vegetation.gd`'s residency pass): out unless resident; fine when
// its nearest corner is within the fine radius, with the patches whose centre is within the far
// range; else coarse within the far range.
fn block_state(
    settings: &ResidencySettings,
    terrain_span: f64,
    resident: &Set<Cell>,
    block: Cell,
    camera: [f32; 2],
) -> BlockState {
    if !resident.contains(&block) {
        return BlockState::default();
    }
    let world = settings.world;
    let far = settings.product.far_m;
    let distance = key_distance([block[0], block[1], 1], terrain_span, world, camera);
    if distance - terrain_span * HALF_DIAGONAL <= settings.fine_radius() {
        let divisor = settings.subdivision;
        let span = terrain_span / f64::from(divisor);
        let mut fine = 0u64;
        for at in 0..divisor * divisor {
            let key = [block[0] * divisor + at / divisor, block[1] * divisor + at % divisor, divisor];
            if key_distance(key, terrain_span, world, camera) <= far + span {
                fine |= 1 << at;
            }
        }
        return BlockState {
            tier: Tier::Fine,
            fine,
        };
    }
    if distance <= far + terrain_span {
        return BlockState {
            tier: Tier::Coarse,
            fine: 0,
        };
    }
    BlockState::default()
}

// The wanted keys a block's state change removes and adds.
fn diff_keys(
    divisor: i32,
    block: Cell,
    old: BlockState,
    new: BlockState,
    leaving: &mut Vec<PatchKey>,
    entering: &mut Vec<PatchKey>,
) {
    let coarse = [block[0], block[1], 1];
    match (old.tier == Tier::Coarse, new.tier == Tier::Coarse) {
        (true, false) => leaving.push(coarse),
        (false, true) => entering.push(coarse),
        _ => {}
    }
    let fine = |state: BlockState| if state.tier == Tier::Fine { state.fine } else { 0 };
    let (old, new) = (fine(old), fine(new));
    for at in 0..divisor * divisor {
        let bit = 1u64 << at;
        let key = [block[0] * divisor + at / divisor, block[1] * divisor + at % divisor, divisor];
        match (old & bit != 0, new & bit != 0) {
            (true, false) => leaving.push(key),
            (false, true) => entering.push(key),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
