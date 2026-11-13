// SPDX-License-Identifier: GPL-2.0-only

//! The graded tile result path in Rust (`RENDER-32-K` part 3, `docs/terrain.md` "Streaming
//! core"): the poll of finished bakes within the install budget, each tile's checks (its request
//! and token, the edit batch, a set's missing behind-the-road variant), the results deferred past
//! a frame's budget, the tiles waiting for their sets' variant, and the placing and writing of a
//! tile's copies, as `terrain_graded_tiles.gd`'s `_poll_and_install`, `_install` and
//! `_take_variant_waits` did them, in the same order and with the same budget checks (the traces
//! pin both).
//!
//! The renderer calls [`GradedTileBook::install_results`] once a frame. A tile's bytes come from
//! the simulation's result without a copy ([`Bytes::Owned`], back to its pool once written) or,
//! under a trace or a test's stand-in simulation, from the polled dictionary
//! ([`Bytes::Shared`]). What stays the renderer's is called back in its place, with the book
//! unbound: any take whose effects need its Godot objects (page arrays, cancels, draw states:
//! `_apply_effects`). Every other take's effects (the uploads, the cells' runs, the counted
//! evictions and same-level uploads) the book applies itself; edits and the batch's staged tiles
//! are the book's ([`super::edits`]), and so are the lists and failures a poll returns
//! ([`super::lists`], `RENDER-32-K` part 6).
//!
//! The runs are generic over their [`Host`] (`CODE-30`): the game's [`ScriptHost`] binds the
//! book's `Gd` one step at a time and calls the renderer's script; the book's tests
//! (`book/tests.rs`) drive the same runs without the engine, with scripted polls and clock.
//!
//! Bounds: a poll `O(its results)`; a tile `O(1)` checks, then its placing and writes
//! (`O(copies x (slot bytes + rows))`, one backend call a layer); the variant waits
//! `O(waiting tiles)` a frame while any wait. No allocation on the tile path once the buffers
//! have grown (the book's entry bookkeeping aside).

use super::*;
use super::edits::Edit;
use crate::nodes::sim::render::stream::layers::graded::ordered::OrderedMap;
use crate::nodes::simulation_node::SimulationNode;
use std::collections::VecDeque;

/// Tiles one poll takes at most (the renderer's `POLL_BATCH`).
const POLL_BATCH: usize = 4;

/// What the install budget charges a frame for each tile it placed: the cell rewrites the install
/// causes later in the frame (`apply_cell_changes`, after the poll), so the poll stops while the
/// frame's graded work still fits the budget. A fixed constant (no adaptive estimate, `RENDER-32`
/// 5.4): a placed tile's cell rewrites as measured on the turns, about four cell tiles at 2.0-2.2
/// us each (30k and 100k alike; the 100k turns' patch-wide rewrites are the camera's, not the
/// installs').
const CELL_CHARGE_US: i64 = 8;

// `take_result_counts`: results deferred, tiles that waited for their sets' variant, evictions
// and same-level uploads the book applied, then the longest poll (us, kept), then the perf
// channel's split: nanoseconds in the polls, the tiles' checks and placing, their effects (the
// uploads and cells), the renderer's callbacks, and the tiles taken. All but the longest poll
// are drained by the read.
const DEFERRED: usize = 0;
pub(super) const VARIANT_WAITS: usize = 1;
pub(super) const EVICTIONS: usize = 2;
pub(super) const SAME_LEVEL: usize = 3;
const POLL_US_MAX: usize = 4;
const POLL_NS: usize = 5;
const PLACE_NS: usize = 6;
const EFFECTS_NS: usize = 7;
const CALLBACK_NS: usize = 8;
const TILES: usize = 9;
const COUNTS: usize = 10;

/// A bake's bytes: shared with a polled dictionary, or moved out of the simulation's result.
pub(crate) enum Bytes {
    /// Shared with a dictionary's array (a trace, a test's stand-in simulation, the preview).
    Shared(PackedByteArray),
    /// The simulation's buffer, returned to its pool once written.
    Owned(Vec<u8>),
}

impl Bytes {
    /// The bytes.
    pub(crate) fn as_slice(&self) -> &[u8] {
        match self {
            Bytes::Shared(array) => array.as_slice(),
            Bytes::Owned(bytes) => bytes,
        }
    }
}

/// A finished tile bake as the result path takes it (`poll_graded_tile_results`' `tiles`).
pub(crate) struct Baked {
    /// The tile.
    pub(crate) tile: Tile,
    /// Its level code and `GL3` step (0 below `GL3`).
    pub(crate) code: u8,
    /// The `GL3` step.
    pub(crate) step: u8,
    /// The token it was requested with.
    pub(crate) token: i64,
    /// Its cells draw the behind-the-road variant (T49 (c)).
    pub(crate) behind: bool,
    /// The slot layer, row atlas run and wall faces, packed as `poll_graded_tile_results`
    /// documents.
    pub(crate) texels: Bytes,
    /// The row atlas run (whole layers).
    pub(crate) rows: Bytes,
    /// The wall faces (`pack_walls`).
    pub(crate) walls: Bytes,
}

impl Baked {
    /// A tile result from its parts (the simulation's poll, bytes moved).
    pub(crate) fn owned(
        tile: Tile,
        level: (i64, i64),
        token: i64,
        behind: bool,
        [texels, rows, walls]: [Vec<u8>; 3],
    ) -> Self {
        Self {
            tile,
            code: level.0 as u8,
            step: level.1 as u8,
            token,
            behind,
            texels: Bytes::Owned(texels),
            rows: Bytes::Owned(rows),
            walls: Bytes::Owned(walls),
        }
    }

    // A polled tile dictionary (a trace's, a stand-in simulation's), its arrays shared.
    pub(super) fn from_dict(dict: VarDictionary) -> Self {
        let int = |key: &str| dict.get(key).map_or(0, |value| value.to::<i64>());
        let bytes = |key: &str| {
            Bytes::Shared(dict.get(key).map_or_else(PackedByteArray::new, |value| value.to::<PackedByteArray>()))
        };
        Self {
            tile: Tile { x: int("tile_x") as i32, z: int("tile_z") as i32 },
            code: int("level") as u8,
            step: int("step") as u8,
            token: int("token"),
            behind: dict.get("behind").is_some_and(|value| value.to::<bool>()),
            texels: bytes("texels"),
            rows: bytes("rows"),
            walls: bytes("walls"),
        }
    }
}

/// One polled result in the order the renderer takes them: a tile, a tile's transient failure,
/// or an edit, list or other failure (`edits`, `patches`, `failed`) as the poll's dictionary,
/// which the renderer takes.
pub(crate) enum Polled {
    /// A finished tile bake.
    Tile(Baked),
    /// A tile's bake failed for a transient reason (a busy core, an unready site index): the
    /// book drops the request and the tile is asked again (`_failed` with `retry`).
    Retry {
        /// The tile.
        tile: Tile,
        /// Its level code.
        code: u8,
        /// Its `GL3` step.
        step: u8,
        /// The request's token.
        token: i64,
    },
    /// An edit the simulation handed over, decoded from the poll's `edits` ([`Edit`]).
    Edit(Edit),
    /// A list or a failure (`patches`, `failed`) as the poll's dictionary ([`super::lists`]).
    Other(&'static str, VarDictionary),
}

/// The result path's state in the book.
#[derive(Default)]
pub(super) struct Results {
    // Results polled after a frame's budget ran out, in arrival order: the next frame takes
    // them first (the simulation hands a result over once).
    deferred: VecDeque<Polled>,
    // One poll's results, reused.
    polled: Vec<Polled>,
    // The flagged tiles waiting for their sets' variant, by tile (still requested meanwhile),
    // and a snapshot of their keys.
    waits: OrderedMap<Tile, Baked>,
    wait_keys: Vec<Tile>,
    // Owned bake buffers written or dropped, for the simulation's pool (its next poll).
    spent: Vec<Vec<u8>>,
    // A placed tile's wall faces as floats, reused.
    pub(super) walls: Vec<f32>,
    pub(super) counts: [i64; COUNTS],
    // Tile bakes waiting in the simulation's queue, as of the last poll.
    tiles_pending: i64,
    // Tests: every result taken from now on is flagged behind the road.
    behind: bool,
}

impl Results {
    pub(super) fn recycle_bytes(&mut self, bytes: Bytes) {
        if let Bytes::Owned(buffer) = bytes {
            self.spent.push(buffer);
        }
    }

    /// Returns a bake's owned buffers to the simulation's pool.
    pub(super) fn recycle_bake(&mut self, texels: Bytes, rows: Bytes) {
        self.recycle_bytes(texels);
        self.recycle_bytes(rows);
    }

    /// Returns a whole bake's owned buffers to the simulation's pool.
    pub(super) fn recycle_baked(&mut self, baked: Baked) {
        self.recycle_bake(baked.texels, baked.rows);
        self.recycle_bytes(baked.walls);
    }

    /// Drops the deferred results and the variant waits, as the renderer resets.
    pub(super) fn clear(&mut self) {
        while let Some(item) = self.deferred.pop_front() {
            if let Polled::Tile(baked) = item {
                self.recycle_baked(baked);
            }
        }
        let mut keys = std::mem::take(&mut self.wait_keys);
        keys.clear();
        keys.extend(self.waits.keys().copied());
        for tile in &keys {
            if let Some(baked) = self.waits.remove(tile) {
                self.recycle_baked(baked);
            }
        }
        keys.clear();
        self.wait_keys = keys;
        self.tiles_pending = 0;
    }
}

// A polled failure of a tile's bake that is asked again, as `_failed` reads it.
fn retried(failure: &VarDictionary) -> Option<Polled> {
    let tile = failure.get("kind").is_some_and(|kind| kind.to::<GString>() == GString::from("tile"));
    let retry = failure.get("retry").is_some_and(|retry| retry.to::<bool>());
    if !(tile && retry) {
        return None;
    }
    let keys = failure.get("keys")?.to::<PackedInt64Array>();
    let token = failure.get("token")?.to::<i64>();
    Some(Polled::Retry {
        tile: Tile { x: keys.get(0)? as i32, z: keys.get(1)? as i32 },
        code: keys.get(2)? as u8,
        step: keys.get(3)? as u8,
        token,
    })
}

// Whether the book applies `effect` itself (the uploads, the cells' runs once their objects are
// attached, the counted evictions and same-level uploads); the rest need the renderer.
fn own_effect(effect: &Effect, attached: bool) -> bool {
    matches!(effect, Effect::Write { .. } | Effect::Evicted { .. } | Effect::SameLevel | Effect::EditDropped { .. })
        || (attached && is_cell_effect(effect))
}

impl GradedTileBook {
    // `take_result_counts`' counts, drained but the longest poll.
    pub(super) fn result_counts(&mut self) -> [i64; COUNTS] {
        let counts = self.taken.counts;
        self.taken.counts[..POLL_US_MAX].fill(0);
        self.taken.counts[POLL_US_MAX + 1..].fill(0);
        counts
    }

    // Results deferred past a frame's budget (tests).
    #[cfg(test)]
    pub(super) fn taken_deferred(&self) -> usize {
        self.taken.deferred.len()
    }

    // Takes the effects of the last calls and applies them where every one is the book's own
    // (as `_apply_effects` applies them, in order). Returns whether the renderer must apply them
    // instead (they stay queued for its `take_effects`).
    pub(super) fn apply_own_effects(&mut self) -> bool {
        self.state.settle();
        let attached = self.objects.is_some();
        if !self.state.out.iter().all(|effect| own_effect(effect, attached)) {
            return true;
        }
        self.encode_effects();
        let mut at = 0;
        while at < self.encoded.len() {
            let (kind, first, end) = (self.encoded[at], self.encoded[at + 1], self.encoded[at + 2]);
            match kind {
                47 => self.apply_cells(first, end),
                48 => self.apply_writes(first, end),
                // Counted as taken (`count_effect`).
                14 | 16 => {}
                _ => debug_assert!(false, "effect {kind} is the renderer's"),
            }
            at += EFFECT_FIELDS;
        }
        false
    }

    // `_upload` of a polled tile: placed into every page its patches' blocks use (the book's
    // `upload`), its bytes staged for the take's writes.
    fn upload_baked(&mut self, baked: Baked) {
        let tile = Vector2i::new(baked.tile.x, baked.tile.z);
        // As the renderer's `Vector4i`: the token in 32 bits.
        let level = Vector4i::new(i32::from(baked.code), i32::from(baked.step), baked.token as i32, i32::from(baked.behind));
        let layers = (baked.rows.as_slice().len() / uploads::ATLAS_LAYER_BYTES) as i64;
        let mut walls = std::mem::take(&mut self.taken.walls);
        walls.clear();
        walls.extend(baked.walls.as_slice().as_chunks::<4>().0.iter().map(|bytes| f32::from_le_bytes(*bytes)));
        self.taken.recycle_bytes(baked.walls);
        self.stage(tile, level, baked.texels, baked.rows);
        self.upload_with(tile, level, layers, None, &walls);
        self.taken.walls = walls;
    }
}

/// The renderer's side of a run of the result path or the edit batch (`CODE-30`, the streaming
/// audit's finding 5): the book, borrowed one step at a time, and the callbacks into what stays
/// the renderer's. The game's is [`ScriptHost`] (the book's `Gd`, unbound around each callback,
/// and `terrain_graded_tiles.gd`); the book's tests use a recording one, without the engine.
pub(crate) trait Host {
    /// The book borrowed for one step.
    type Book<'a>: std::ops::DerefMut<Target = GradedTileBook>
    where
        Self: 'a;
    /// Borrows the book; no callback runs while the borrow lives.
    fn book(&mut self) -> Self::Book<'_>;
    /// `_apply_effects`: the renderer applies the effects queued in the book (page arrays,
    /// cancels, draw states, and the cells' runs while the book has no objects).
    fn apply_effects(&mut self);
    /// The renderer's `release_preview(first, last)` (its own entry point, which a trace records).
    fn release_preview(&mut self, first: i64, last: i64);
    /// One poll of finished work into `polled` (edits, lists, failures, tiles, in that order);
    /// `spent` returns written bake buffers to the simulation's pool. Returns the tile bakes
    /// pending.
    fn poll(&mut self, polled: &mut Vec<Polled>, spent: &mut Vec<Vec<u8>>) -> i64;
    /// The budget's clock, microseconds on the frame start's scale.
    fn now_us(&mut self) -> i64;
}

/// The game's [`Host`]: the book's `Gd`, the renderer's script, and the simulation (none under a
/// trace or a test's stand-in, which poll through the renderer's `_sim_poll`).
pub(crate) struct ScriptHost {
    book: Gd<GradedTileBook>,
    renderer: Gd<RefCounted>,
    simulation: Option<Gd<SimulationNode>>,
    // The engine's clock (`Time.get_ticks_usec`, which the frame's start reads) when the run
    // began and a monotonic clock from then: a budget check without a call into the engine.
    clock: (i64, std::time::Instant),
}

impl ScriptHost {
    // A host whose clock starts at the engine's (the frame's result path).
    pub(super) fn timed(book: Gd<GradedTileBook>, renderer: Gd<RefCounted>, simulation: Option<Gd<SimulationNode>>) -> Self {
        let engine = godot::classes::Time::singleton().get_ticks_usec() as i64;
        Self { book, renderer, simulation, clock: (engine, std::time::Instant::now()) }
    }

    // A host outside the frame's poll (no budget, no simulation).
    pub(super) fn new(book: Gd<GradedTileBook>, renderer: Gd<RefCounted>) -> Self {
        Self { book, renderer, simulation: None, clock: (0, std::time::Instant::now()) }
    }
}

impl Host for ScriptHost {
    type Book<'a> = godot::obj::GdMut<'a, GradedTileBook>;

    fn book(&mut self) -> Self::Book<'_> {
        self.book.bind_mut()
    }

    fn apply_effects(&mut self) {
        self.renderer.call("_apply_effects", &[]);
    }

    fn release_preview(&mut self, first: i64, last: i64) {
        self.renderer.call("release_preview", &[first.to_variant(), last.to_variant()]);
    }

    fn poll(&mut self, polled: &mut Vec<Polled>, spent: &mut Vec<Vec<u8>>) -> i64 {
        if let Some(simulation) = self.simulation.as_mut() {
            return simulation.bind_mut().poll_graded_results(POLL_BATCH, polled, spent);
        }
        let result = self.renderer.call("_sim_poll", &[]).to::<VarDictionary>();
        for kind in ["edits", "patches", "failed", "tiles"] {
            let Some(entries) = result.get(kind) else {
                continue;
            };
            for entry in entries.to::<VarArray>().iter_shared() {
                let dict = entry.to::<VarDictionary>();
                polled.push(match kind {
                    "tiles" => Polled::Tile(Baked::from_dict(dict)),
                    "edits" => Polled::Edit(Edit::from_dict(&dict)),
                    "failed" => retried(&dict).unwrap_or(Polled::Other(kind, dict)),
                    kind => Polled::Other(kind, dict),
                });
            }
        }
        result.get("tiles_pending").map_or(0, |value| value.to::<i64>())
    }

    fn now_us(&mut self) -> i64 {
        self.clock.0 + self.clock.1.elapsed().as_micros() as i64
    }
}

// One frame's result path: the host (the book and the renderer it calls back), the frame's start
// and whether it took a result or count yet.
pub(super) struct Run<H: Host> {
    pub(super) host: H,
    start_us: i64,
    progressed: bool,
    // The tiles placed this frame, each charged `CELL_CHARGE_US` in the budget checks.
    placed: i64,
    // Whether the perf channel's split counts this run's time (the frame's result path does; the
    // edit batch and the preview, outside it, do not).
    split: bool,
}

impl<H: Host> Run<H> {
    // The frame's result path from `start_us` (on the host's clock).
    pub(super) fn budgeted(host: H, start_us: i64) -> Self {
        Self { host, start_us, progressed: false, placed: 0, split: true }
    }

    // A run outside the frame's poll (no budget, no split): the edit batch, the preview.
    pub(super) fn unbudgeted(host: H) -> Self {
        Self { host, start_us: 0, progressed: true, placed: 0, split: false }
    }

    // `_over_budget` with the placed tiles' cell rewrites charged (as if the frame had started
    // that much earlier): the trace's answer where one records or replays.
    fn over(&mut self) -> bool {
        let start_us = self.start_us - self.placed * CELL_CHARGE_US;
        let now_us = self.host.now_us();
        let mut book = self.host.book();
        let book = &mut *book;
        if book.trace.is_some() {
            return over_budget(&mut book.trace, book.state.unbudgeted, start_us);
        }
        !book.state.unbudgeted && now_us - start_us >= 1000
    }

    fn has_counts(&mut self) -> bool {
        !self.host.book().state.requests.counts.is_empty()
    }

    // `_apply_count`.
    fn count(&mut self) {
        self.host.book().state.apply_count();
        self.effects();
    }

    pub(super) fn effects(&mut self) {
        let start = std::time::Instant::now();
        let renderer = self.host.book().apply_own_effects();
        if renderer {
            self.host.apply_effects();
        }
        self.add(EFFECTS_NS, start);
    }

    // Adds the time since `start` to the split's `slot`.
    fn add(&mut self, slot: usize, start: std::time::Instant) {
        if !self.split {
            return;
        }
        self.host.book().taken.counts[slot] += start.elapsed().as_nanos() as i64;
    }

    // The renderer's `release_preview`, timed as a callback.
    pub(super) fn release_preview(&mut self, first: i64, last: i64) {
        let start = std::time::Instant::now();
        self.host.release_preview(first, last);
        self.add(CALLBACK_NS, start);
    }

    fn take(&mut self, item: Polled) {
        match item {
            Polled::Tile(baked) => self.install(baked),
            Polled::Retry { tile, code, step, token } => {
                let tile = Vector2i::new(tile.x, tile.z);
                self.host.book().failed_tile(tile, i64::from(code), i64::from(step), token, true);
            }
            Polled::Edit(edit) => {
                let start = std::time::Instant::now();
                self.apply_edit(edit);
                self.add(CALLBACK_NS, start);
            }
            Polled::Other(kind, dict) => {
                let start = std::time::Instant::now();
                if kind == "patches" {
                    self.take_list(&dict);
                } else {
                    self.failed(&dict);
                }
                self.add(CALLBACK_NS, start);
            }
        }
    }

    // `_install`: a result of the tile's current request installs, waits for its sets' variant
    // or, the edit batch's, stages for it; any other is stale and dropped. O(1) + its placing.
    pub(super) fn install(&mut self, mut baked: Baked) {
        let start = std::time::Instant::now();
        let mut guard = self.host.book();
        let book = &mut *guard;
        book.taken.counts[TILES] += 1;
        if book.taken.behind {
            baked.behind = true;
        }
        let tile = Vector2i::new(baked.tile.x, baked.tile.z);
        let (code, step, token) = (i64::from(baked.code), i64::from(baked.step), baked.token);
        if book.requested_token(tile, code, step) != token {
            book.taken.recycle_baked(baked);
            return;
        }
        if book.edit_batch_token(tile) == token {
            // Part of an edit: it waits for the batch's other tiles (and, flagged, its sets'
            // variant) in the batch ([`super::edits`]).
            book.requested_erase(tile, code, step, token);
            if baked.behind {
                book.variant_missing(tile);
            }
            book.stage_baked(baked);
            return;
        }
        // A later result of the tile supersedes one that waited (an older level's or token's).
        if let Some(waited) = book.taken.waits.remove(&baked.tile) {
            book.requested_erase(tile, i64::from(waited.code), i64::from(waited.step), waited.token);
            book.taken.recycle_baked(waited);
        }
        if baked.behind && book.variant_missing(tile) {
            // Still requested while it waits (`take_variant_waits`).
            book.taken.counts[VARIANT_WAITS] += 1;
            book.taken.waits.insert(baked.tile, baked);
            return;
        }
        book.requested_erase(tile, code, step, token);
        book.upload_baked(baked);
        book.taken.counts[PLACE_NS] += start.elapsed().as_nanos() as i64;
        drop(guard);
        self.placed += 1;
        self.effects();
    }

    // One poll into `polled`: returns its tiles. The simulation's poll moves its tiles' bytes;
    // under a trace or a stand-in the renderer's `_sim_poll` gives the dictionary.
    fn poll(&mut self, polled: &mut Vec<Polled>) -> usize {
        let start = std::time::Instant::now();
        polled.clear();
        let mut spent = std::mem::take(&mut self.host.book().taken.spent);
        let pending = self.host.poll(polled, &mut spent);
        let tiles = polled.iter().filter(|item| matches!(item, Polled::Tile(_))).count();
        let mut book = self.host.book();
        book.taken.spent = spent;
        book.taken.tiles_pending = pending;
        let counts = &mut book.taken.counts;
        counts[POLL_US_MAX] = counts[POLL_US_MAX].max(start.elapsed().as_micros() as i64);
        counts[POLL_NS] += start.elapsed().as_nanos() as i64;
        tiles
    }

    // `_poll_and_install`: the counts and deferred results first, then polls until a poll
    // returns fewer than a batch of tiles, each result and count only while the budget lasts
    // (checked before each; the frame's first is taken unconditionally), the rest deferred.
    fn run(&mut self, polled: &mut Vec<Polled>) {
        loop {
            while self.has_counts() {
                if self.progressed && self.over() {
                    return;
                }
                self.count();
                self.progressed = true;
            }
            if self.host.book().taken.deferred.is_empty() {
                break;
            }
            if self.progressed && self.over() {
                return;
            }
            let item = self.host.book().taken.deferred.pop_front();
            if let Some(item) = item {
                self.take(item);
            }
            self.progressed = true;
        }
        loop {
            if self.progressed && self.over() {
                return;
            }
            let tiles = self.poll(polled);
            for item in polled.drain(..) {
                if self.progressed && self.over() {
                    let mut book = self.host.book();
                    book.taken.deferred.push_back(item);
                    book.taken.counts[DEFERRED] += 1;
                    continue;
                }
                self.take(item);
                self.progressed = true;
                // The budget spent, the rest of the counts and every later result wait.
                while self.has_counts() && !self.over() {
                    self.count();
                }
            }
            if tiles < POLL_BATCH {
                return;
            }
        }
    }

    // The frame's tile results (`install_results`): the poll buffer is the book's, reused.
    pub(super) fn results(&mut self) {
        let mut polled = std::mem::take(&mut self.host.book().taken.polled);
        self.run(&mut polled);
        // A poll's results the frame left are deferred above; the buffer is empty again.
        polled.clear();
        self.host.book().taken.polled = polled;
    }

    // `_advance_builds`: the sets a flagged tile waits for take the variant first (the first in
    // every frame, the rest while the install budget lasts), then the queued set builds: the
    // patches a shown tile waits for first, at least one tile of the first of them in every
    // frame, then the rest while the budget lasts. A set first syncs its materials, then writes
    // its tiles; a built set starts drawing if a tile of it shows graded. O(tiles written this
    // frame + uniforms synced).
    pub(super) fn advance_builds(&mut self, start_us: i64) {
        let mut added = 0;
        while self.host.book().has_variants_wanted() {
            if added > 0 && self.over() {
                return;
            }
            if self.host.book().add_next_variant() == 1 {
                self.effects();
                added += 1;
            }
        }
        let mut minimum = 1;
        loop {
            let next = self.host.book().next_build();
            if next.z < 0 {
                return;
            }
            let key = Vector2i::new(next.x, next.y);
            if next.z == 0 {
                minimum = 0;
            }
            if minimum == 0 && self.over() {
                return;
            }
            if !self.host.book().is_resident(key) {
                self.host.book().build_finished(key, false);
                continue;
            }
            let set = self.host.book().begin_build(key);
            self.effects();
            let mut done = self.host.book().set_built(set);
            if !done && self.host.book().sync_build(set, start_us) {
                done = self.host.book().build_tiles(set, start_us, minimum);
                self.effects();
            }
            minimum = 0;
            if !done {
                return;
            }
            self.host.book().build_finished(key, true);
        }
    }

    // `_take_variant_waits`' results: the flagged tiles whose sets all hold the variant now
    // install, in the order they began waiting.
    pub(super) fn variant_waits(&mut self) {
        let mut keys = {
            let mut book = self.host.book();
            let mut keys = std::mem::take(&mut book.taken.wait_keys);
            keys.clear();
            keys.extend(book.taken.waits.keys().copied());
            keys
        };
        for &tile in &keys {
            let ready = {
                let mut book = self.host.book();
                if book.variant_missing(Vector2i::new(tile.x, tile.z)) {
                    None
                } else {
                    book.taken.waits.remove(&tile)
                }
            };
            if let Some(baked) = ready {
                self.install(baked);
            }
        }
        keys.clear();
        self.host.book().taken.wait_keys = keys;
    }
}

#[godot_api(secondary)]
impl GradedTileBook {
    /// The frame's tile results (`_poll_and_install`, `RENDER-32-K` part 3): the queued counts
    /// and deferred results, then polls of finished work (edits, lists, failures, tiles, in that
    /// order a poll) until the frame's install budget from `start_us` is spent, checked before
    /// each result and count, each tile placed charged its later cell rewrites
    /// (`CELL_CHARGE_US`). Tiles install or stage for the edit batch here, edits apply here
    /// ([`super::edits`]), lists and failures here ([`super::lists`]); the renderer's
    /// `_apply_effects` takes a take whose effects need it. `simulation` polls without dictionaries; null (a trace, a stand-in) polls through
    /// the renderer's `_sim_poll`. O(results and counts taken + one poll's results).
    #[func]
    pub fn install_results(
        book: Gd<GradedTileBook>,
        renderer: Gd<RefCounted>,
        simulation: Option<Gd<SimulationNode>>,
        start_us: i64,
    ) {
        Run::budgeted(ScriptHost::timed(book, renderer, simulation), start_us).results();
    }

    /// The frame's cell set builds (`_advance_builds`, `RENDER-32-K` part 6): the variant the
    /// waiting flagged tiles need first, then the queued builds, within the install budget from
    /// `start_us` (the trace's answers where one records or replays). O(tiles written this frame
    /// + uniforms synced).
    #[func]
    pub fn advance_builds(book: Gd<GradedTileBook>, renderer: Gd<RefCounted>, start_us: i64) {
        let mut run = Run::budgeted(ScriptHost::timed(book, renderer, None), start_us);
        // Outside the result path's split.
        run.split = false;
        run.advance_builds(start_us);
    }

    /// `_take_variant_waits`' results: the flagged tiles whose sets all hold the variant now
    /// install (in the order they began waiting). O(waiting tiles).
    #[func]
    pub fn take_variant_waits(book: Gd<GradedTileBook>, renderer: Gd<RefCounted>) {
        let mut run = Run::unbudgeted(ScriptHost::new(book, renderer));
        // The waits' time counts in the perf channel's split, as before.
        run.split = true;
        run.variant_waits();
    }

    /// The result path's counts since the last read: results deferred, tiles that waited for
    /// their sets' variant, evictions and same-level uploads it applied; the longest poll in
    /// microseconds (kept); then nanoseconds in the polls, in the tiles' checks and placing, in
    /// their effects (uploads, cells, the renderer's `_apply_effects`), in the other callbacks,
    /// and the tiles taken.
    #[func]
    pub fn take_result_counts(&mut self) -> PackedInt64Array {
        PackedInt64Array::from(self.result_counts().as_slice())
    }

    /// Tile bakes waiting in the simulation's queue, as of the last poll.
    #[func]
    pub fn tiles_pending(&self) -> i64 {
        self.taken.tiles_pending
    }

    /// Whether results wait: deferred past a frame's budget, or for their sets' variant.
    #[func]
    pub fn results_waiting(&self) -> bool {
        !self.taken.deferred.is_empty() || self.taken.waits.len() > 0
    }

    /// Whether a flagged tile waits for its sets' variant.
    #[func]
    pub fn has_variant_waits(&self) -> bool {
        self.taken.waits.len() > 0
    }

    /// For tests: every result taken from now on is flagged behind the road (`behind`), as a
    /// hilly map's are.
    #[func]
    pub fn flag_results_behind(&mut self, behind: bool) {
        self.taken.behind = behind;
    }
}
