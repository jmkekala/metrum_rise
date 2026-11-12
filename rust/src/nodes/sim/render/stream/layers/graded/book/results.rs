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
//! unbound: the edits, lists and failures a poll returns (`_take_polled`), a tile the edit batch
//! takes (`_stage_edit`, part 4's), and any take whose effects need its Godot objects (page
//! arrays, cancels, draw states: `_apply_effects`). Every other take's effects (the uploads, the
//! cells' runs, the counted evictions and same-level uploads) the book applies itself.
//!
//! Bounds: a poll `O(its results)`; a tile `O(1)` checks, then its placing and writes
//! (`O(copies x (slot bytes + rows))`, one backend call a layer); the variant waits
//! `O(waiting tiles)` a frame while any wait. No allocation on the tile path once the buffers
//! have grown (the book's entry bookkeeping aside).

use super::*;
use crate::nodes::sim::render::stream::layers::graded::ordered::OrderedMap;
use crate::nodes::simulation_node::SimulationNode;
use std::collections::VecDeque;

/// Tiles one poll takes at most (the renderer's `POLL_BATCH`).
const POLL_BATCH: usize = 4;

// `take_result_counts`: results deferred, tiles that waited for their sets' variant, evictions
// and same-level uploads the book applied (drained by the read), then the longest poll (us).
const DEFERRED: usize = 0;
const VARIANT_WAITS: usize = 1;
const EVICTIONS: usize = 2;
const SAME_LEVEL: usize = 3;
const POLL_US_MAX: usize = 4;
const COUNTS: usize = 5;

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

    fn to_packed(&self) -> PackedByteArray {
        match self {
            Bytes::Shared(array) => array.clone(),
            Bytes::Owned(bytes) => PackedByteArray::from(bytes.as_slice()),
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
    /// Whether the tile is graded (`graded`).
    pub(crate) graded: bool,
    /// `samples_x`, `samples_z`, `seam_count`, `edge_count`.
    pub(crate) counts: [i64; 4],
    /// The slot layer, row atlas run and wall faces, packed as `poll_graded_tile_results`
    /// documents.
    pub(crate) texels: Bytes,
    /// The row atlas run (whole layers).
    pub(crate) rows: Bytes,
    /// The wall faces (`pack_walls`).
    pub(crate) walls: Bytes,
    // The polled dictionary it came from (handed back as it is where the renderer keeps it).
    source: Option<VarDictionary>,
}

impl Baked {
    /// A tile result from its parts (the simulation's poll, bytes moved).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn owned(
        tile: Tile,
        level: (i64, i64),
        token: i64,
        behind: bool,
        graded: bool,
        counts: [i64; 4],
        [texels, rows, walls]: [Vec<u8>; 3],
    ) -> Self {
        Self {
            tile,
            code: level.0 as u8,
            step: level.1 as u8,
            token,
            behind,
            graded,
            counts,
            texels: Bytes::Owned(texels),
            rows: Bytes::Owned(rows),
            walls: Bytes::Owned(walls),
            source: None,
        }
    }

    // A polled tile dictionary (a trace's, a stand-in simulation's), its arrays shared.
    fn from_dict(dict: VarDictionary) -> Self {
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
            graded: dict.get("graded").is_none_or(|value| value.to::<bool>()),
            counts: [int("samples_x"), int("samples_z"), int("seam_count"), int("edge_count")],
            texels: bytes("texels"),
            rows: bytes("rows"),
            walls: bytes("walls"),
            source: Some(dict),
        }
    }

    // The tile as `poll_graded_tile_results` returns it (`behind` as taken).
    fn to_dict(&self) -> VarDictionary {
        if let Some(source) = &self.source {
            let mut dict = source.clone();
            dict.set("behind", self.behind);
            return dict;
        }
        let mut dict = VarDictionary::new();
        dict.set("tile_x", i64::from(self.tile.x));
        dict.set("tile_z", i64::from(self.tile.z));
        dict.set("level", i64::from(self.code));
        dict.set("step", i64::from(self.step));
        dict.set("token", self.token);
        dict.set("samples_x", self.counts[0]);
        dict.set("samples_z", self.counts[1]);
        dict.set("graded", self.graded);
        dict.set("behind", self.behind);
        dict.set("seam_count", self.counts[2]);
        dict.set("edge_count", self.counts[3]);
        dict.set("wall_count", (self.walls.as_slice().len() / crate::simulation::terrain::graded::WALL_FACE_BYTES) as i64);
        dict.set("texels", self.texels.to_packed());
        dict.set("rows", self.rows.to_packed());
        dict.set("walls", self.walls.to_packed());
        dict
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
    /// A result of another kind, by its key in the poll's dictionary.
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
    walls: Vec<f32>,
    counts: [i64; COUNTS],
    // Tile bakes waiting in the simulation's queue, as of the last poll.
    tiles_pending: i64,
    // Tests: every result taken from now on is flagged behind the road.
    behind: bool,
}

impl Results {
    fn recycle_bytes(&mut self, bytes: Bytes) {
        if let Bytes::Owned(buffer) = bytes {
            self.spent.push(buffer);
        }
    }

    /// Returns a bake's owned buffers to the simulation's pool.
    pub(super) fn recycle_bake(&mut self, texels: Bytes, rows: Bytes) {
        self.recycle_bytes(texels);
        self.recycle_bytes(rows);
    }

    fn recycle(&mut self, baked: Baked) {
        self.recycle_bake(baked.texels, baked.rows);
        self.recycle_bytes(baked.walls);
    }

    /// Drops the deferred results and the variant waits, as the renderer resets.
    pub(super) fn clear(&mut self) {
        while let Some(item) = self.deferred.pop_front() {
            if let Polled::Tile(baked) = item {
                self.recycle(baked);
            }
        }
        let mut keys = std::mem::take(&mut self.wait_keys);
        keys.clear();
        keys.extend(self.waits.keys().copied());
        for tile in &keys {
            if let Some(baked) = self.waits.remove(tile) {
                self.recycle(baked);
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
    matches!(effect, Effect::Write { .. } | Effect::Evicted { .. } | Effect::SameLevel)
        || (attached && is_cell_effect(effect))
}

impl GradedTileBook {
    // Takes the effects of the last calls and applies them where every one is the book's own
    // (as `_apply_effects` applies them, in order). Returns whether the renderer must apply them
    // instead (they stay queued for its `take_effects`).
    fn apply_own_effects(&mut self) -> bool {
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
                14 => self.taken.counts[EVICTIONS] += 1,
                16 => self.taken.counts[SAME_LEVEL] += 1,
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

// One frame's result path: the book and the renderer it calls back, the simulation (none under
// a trace or a stand-in), the frame's start and whether it took a result or count yet.
struct Run {
    book: Gd<GradedTileBook>,
    renderer: Gd<RefCounted>,
    simulation: Option<Gd<SimulationNode>>,
    start_us: i64,
    progressed: bool,
}

impl Run {
    // `_over_budget`: the trace's answer where one records or replays.
    fn over(&mut self) -> bool {
        let start_us = self.start_us;
        let mut book = self.book.bind_mut();
        let book = &mut *book;
        over_budget(&mut book.trace, book.state.unbudgeted, start_us)
    }

    fn has_counts(&self) -> bool {
        !self.book.bind().state.requests.counts.is_empty()
    }

    // `_apply_count`.
    fn count(&mut self) {
        self.book.bind_mut().state.apply_count();
        self.effects();
    }

    fn effects(&mut self) {
        let renderer = self.book.bind_mut().apply_own_effects();
        if renderer {
            self.renderer.call("_apply_effects", &[]);
        }
    }

    fn take(&mut self, item: Polled) {
        match item {
            Polled::Tile(baked) => self.install(baked),
            Polled::Retry { tile, code, step, token } => {
                let tile = Vector2i::new(tile.x, tile.z);
                self.book.bind_mut().failed_tile(tile, i64::from(code), i64::from(step), token, true);
            }
            Polled::Other(kind, dict) => {
                self.renderer.call("_take_polled", &[kind.to_variant(), dict.to_variant()]);
            }
        }
    }

    // `_install`: a result of the tile's current request installs, waits for its sets' variant
    // or, the edit batch's, stages for it; any other is stale and dropped. O(1) + its placing.
    fn install(&mut self, mut baked: Baked) {
        let mut guard = self.book.bind_mut();
        let book = &mut *guard;
        if book.taken.behind {
            baked.behind = true;
        }
        let tile = Vector2i::new(baked.tile.x, baked.tile.z);
        let (code, step, token) = (i64::from(baked.code), i64::from(baked.step), baked.token);
        if book.requested_token(tile, code, step) != token {
            book.taken.recycle(baked);
            return;
        }
        if book.edit_batch_token(tile) == token {
            // Part of an edit: it waits for the batch's other tiles (and, flagged, its sets'
            // variant) in the renderer's batch.
            book.requested_erase(tile, code, step, token);
            if baked.behind {
                book.variant_missing(tile);
            }
            let dict = baked.to_dict();
            book.taken.recycle(baked);
            drop(guard);
            self.renderer.call("_stage_edit", &[dict.to_variant()]);
            return;
        }
        // A later result of the tile supersedes one that waited (an older level's or token's).
        if let Some(waited) = book.taken.waits.remove(&baked.tile) {
            book.requested_erase(tile, i64::from(waited.code), i64::from(waited.step), waited.token);
            book.taken.recycle(waited);
        }
        if baked.behind && book.variant_missing(tile) {
            // Still requested while it waits (`take_variant_waits`).
            book.taken.counts[VARIANT_WAITS] += 1;
            book.taken.waits.insert(baked.tile, baked);
            return;
        }
        book.requested_erase(tile, code, step, token);
        book.upload_baked(baked);
        drop(guard);
        self.effects();
    }

    // One poll into `polled`: returns its tiles. The simulation's poll moves its tiles' bytes;
    // under a trace or a stand-in the renderer's `_sim_poll` gives the dictionary.
    fn poll(&mut self, polled: &mut Vec<Polled>) -> usize {
        let start = std::time::Instant::now();
        polled.clear();
        let pending = match self.simulation.as_mut() {
            Some(simulation) => {
                let mut spent = std::mem::take(&mut self.book.bind_mut().taken.spent);
                let pending = simulation.bind_mut().poll_graded_results(POLL_BATCH, polled, &mut spent);
                self.book.bind_mut().taken.spent = spent;
                pending
            }
            None => {
                let result = self.renderer.call("_sim_poll", &[]).to::<VarDictionary>();
                for kind in ["edits", "patches", "failed", "tiles"] {
                    let Some(entries) = result.get(kind) else {
                        continue;
                    };
                    for entry in entries.to::<VarArray>().iter_shared() {
                        let dict = entry.to::<VarDictionary>();
                        polled.push(match kind {
                            "tiles" => Polled::Tile(Baked::from_dict(dict)),
                            "failed" => retried(&dict).unwrap_or(Polled::Other(kind, dict)),
                            kind => Polled::Other(kind, dict),
                        });
                    }
                }
                result.get("tiles_pending").map_or(0, |value| value.to::<i64>())
            }
        };
        let tiles = polled.iter().filter(|item| matches!(item, Polled::Tile(_))).count();
        let mut book = self.book.bind_mut();
        book.taken.tiles_pending = pending;
        let counts = &mut book.taken.counts;
        counts[POLL_US_MAX] = counts[POLL_US_MAX].max(start.elapsed().as_micros() as i64);
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
            if self.book.bind().taken.deferred.is_empty() {
                break;
            }
            if self.progressed && self.over() {
                return;
            }
            let item = self.book.bind_mut().taken.deferred.pop_front();
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
                    let mut book = self.book.bind_mut();
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
}

#[godot_api(secondary)]
impl GradedTileBook {
    /// The frame's tile results (`_poll_and_install`, `RENDER-32-K` part 3): the queued counts
    /// and deferred results, then polls of finished work (edits, lists, failures, tiles, in that
    /// order a poll) until the frame's install budget from `start_us` is spent, checked before
    /// each result and count. Tiles install here; the renderer's `_take_polled` takes the other
    /// kinds, `_stage_edit` a tile of the edit batch and `_apply_effects` a take whose effects
    /// need it. `simulation` polls without dictionaries; null (a trace, a stand-in) polls through
    /// the renderer's `_sim_poll`. O(results and counts taken + one poll's results).
    #[func]
    pub fn install_results(
        book: Gd<GradedTileBook>,
        renderer: Gd<RefCounted>,
        simulation: Option<Gd<SimulationNode>>,
        start_us: i64,
    ) {
        let mut polled = std::mem::take(&mut book.clone().bind_mut().taken.polled);
        let mut run = Run { book, renderer, simulation, start_us, progressed: false };
        run.run(&mut polled);
        // A poll's results the frame left are deferred above; the buffer is empty again.
        polled.clear();
        run.book.bind_mut().taken.polled = polled;
    }

    /// `_take_variant_waits`' results: the flagged tiles whose sets all hold the variant now
    /// install (in the order they began waiting). O(waiting tiles).
    #[func]
    pub fn take_variant_waits(book: Gd<GradedTileBook>, renderer: Gd<RefCounted>) {
        let mut keys = {
            let mut guard = book.clone();
            let mut bound = guard.bind_mut();
            let mut keys = std::mem::take(&mut bound.taken.wait_keys);
            keys.clear();
            keys.extend(bound.taken.waits.keys().copied());
            keys
        };
        let mut run = Run { book, renderer, simulation: None, start_us: 0, progressed: true };
        for &tile in &keys {
            let ready = {
                let mut book = run.book.bind_mut();
                if book.variant_missing(Vector2i::new(tile.x, tile.z)) {
                    None
                } else {
                    book.taken.waits.remove(&tile)
                }
            };
            if let Some(baked) = ready {
                run.install(baked);
            }
        }
        keys.clear();
        run.book.bind_mut().taken.wait_keys = keys;
    }

    /// The result path's counts since the last read: results deferred, tiles that waited for
    /// their sets' variant, evictions and same-level uploads it applied; then the longest poll
    /// in microseconds (kept).
    #[func]
    pub fn take_result_counts(&mut self) -> PackedInt64Array {
        let counts = PackedInt64Array::from(self.taken.counts.as_slice());
        self.taken.counts[..POLL_US_MAX].fill(0);
        counts
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
