// SPDX-License-Identifier: GPL-2.0-only

//! The graded edit batch and the road preview's tiles in Rust (`RENDER-32-K` part 4,
//! `docs/terrain.md` "Streaming core"), as `terrain_graded_tiles.gd`'s `_apply_edit`,
//! `_staged_dirty`, `_install_edit_batch`, `_reserve_edit_copies`, `_adopt_preview`,
//! `_show_preview_tile`, `_drop_preview` and the preview's variant waits did them, in the same
//! order and with the effects applied at the same points (the traces pin both).
//!
//! An edit's batch: the edit dirties the tiles it changed (each takes the edit's serial as its
//! token) and stages the commit's own tiles; a polled bake of a batched tile stages too
//! ([`super::results`]). Once every batched tile has its result and none waits for its sets'
//! behind-the-road variant, the batch installs whole in one frame: committed tiles that are the
//! preview's very payload adopt its slots, every other copy is reserved first (all or none) and
//! then written, and the preview tiles of the epochs the batch's edits released drop in the same
//! frame. The preview: a pose's tiles take slots of their own and draw in place of the installed
//! tile until dropped; a flagged tile waits for its sets' variant (the preview's lag, T27 (d)).
//!
//! Bounds: an edit `O(its dirty and staged tiles)`; a batch install `O(batch copies + free runs of
//! the touched pages)` plus the writes (`O(copies x (slot bytes + rows))`); a preview tile `O(1)`
//! plus its placing; the preview waits `O(waiting tiles)` a frame while any wait. Nothing scales
//! with the world or the resident set. Staged and waiting bakes keep their bytes (moved, not
//! copied); the batch's scratch buffers are reused.

use super::results::{Host, Run, ScriptHost, VARIANT_WAITS};
use super::*;
use crate::nodes::sim::render::stream::layers::graded::ordered::OrderedMap;

// `take_edit_counts`: edits taken, committed tiles that adopted the preview's slot, preview tiles
// a full page left to a later pose, preview tiles whose level's pool is exhausted. Drained by the
// read.
const EDITS: usize = 0;
const ADOPTED: usize = 1;
const PREVIEW_PAGE_WAITS: usize = 2;
const PREVIEW_EXHAUSTED: usize = 3;
const COUNTS: usize = 4;

// A staged tile of the edit batch: its bake (the token the batch's), and, for a commit's own tile,
// whether it is bitwise the preview's payload and the preview epoch it adopts.
struct Staged {
    baked: Baked,
    adopted: bool,
    preview_epoch: i64,
}

/// The edit batch and preview state in the book.
pub(super) struct Edits {
    // The batch's staged results by tile, in the order the GDScript's dictionary kept them.
    staged: OrderedMap<Tile, Staged>,
    // The highest preview epoch the batch's edits release when it installs (-1: none).
    release: i64,
    // The serial of the last edit taken, and the one the last installed batch reached
    // (`installed_edit_serial`, T61 (2)).
    serial_taken: i64,
    serial_batched: i64,
    // The tiles the last batch installed (tests and the probe).
    last_batch: Vec<Tile>,
    // Flagged preview tiles waiting for their sets' variant (their token is the preview epoch).
    preview_waits: OrderedMap<Tile, Baked>,
    // Scratch: tiles of a call, a reservation's flat request.
    keys: Vec<Tile>,
    reserve: Vec<i64>,
    counts: [i64; COUNTS],
}

impl Default for Edits {
    fn default() -> Self {
        Self {
            staged: OrderedMap::default(),
            release: -1,
            serial_taken: 0,
            serial_batched: 0,
            last_batch: Vec::new(),
            preview_waits: OrderedMap::default(),
            keys: Vec::new(),
            reserve: Vec::new(),
            counts: [0; COUNTS],
        }
    }
}

impl Edits {
    /// As the renderer clears: the batch's staged results and release and the preview waits
    /// dropped (the serials and the last batch stay, as they did).
    pub(super) fn clear(&mut self, results: &mut super::results::Results) {
        self.release = -1;
        self.keys.clear();
        self.keys.extend(self.staged.keys().copied());
        for tile in &self.keys {
            if let Some(staged) = self.staged.remove(tile) {
                results.recycle_baked(staged.baked);
            }
        }
        self.keys.clear();
        self.keys.extend(self.preview_waits.keys().copied());
        for tile in &self.keys {
            if let Some(baked) = self.preview_waits.remove(tile) {
                results.recycle_baked(baked);
            }
        }
        self.keys.clear();
    }

    /// An eviction dropped `tile`'s staged result (`EditDropped`).
    pub(super) fn dropped(&mut self, tile: Tile, results: &mut super::results::Results) {
        if let Some(staged) = self.staged.remove(&tile) {
            results.recycle_baked(staged.baked);
        }
    }
}

// A dictionary's integer, 0 when absent.
fn int(dict: &VarDictionary, key: &str) -> i64 {
    dict.get(key).map_or(0, |value| value.to::<i64>())
}

/// An edit the simulation's poll hands over (`poll_graded_tile_results`' `edits`), decoded: the
/// tiles it dirtied per level and a commit's or undo's own tiles.
pub(crate) struct Edit {
    /// The edit's serial (the dirtied tiles' new token).
    pub(crate) serial: i64,
    /// Every batched, installed, requested and refused tile is dirty (`levels` unused).
    pub(crate) all: bool,
    /// Per level code and `GL3` step the tiles it dirtied there, in the poll's order.
    pub(crate) levels: Vec<(u8, u8, Vec<Tile>)>,
    /// The preview epoch its commit adopted (-1: none).
    pub(crate) preview_epoch: i64,
    /// A commit's or undo's own tiles, in order.
    pub(crate) staged: Vec<EditTile>,
}

/// A commit's or undo's own tile in an [`Edit`].
pub(crate) struct EditTile {
    /// Its bake (the token is replaced by the batch's).
    pub(crate) baked: Baked,
    /// It is graded (a base tile is not staged where nothing holds it).
    pub(crate) graded: bool,
    /// It is bitwise the preview's payload and adopts the preview's slots.
    pub(crate) adopted: bool,
}

impl Edit {
    /// The poll's dictionary decoded (its arrays shared, not copied). O(its tiles).
    pub(crate) fn from_dict(edit: &VarDictionary) -> Self {
        let flag = |dict: &VarDictionary, key: &str| dict.get(key).is_some_and(|value| value.to::<bool>());
        let array = |key: &str| edit.get(key).map_or_else(VarArray::new, |value| value.to::<VarArray>());
        let levels = array("levels")
            .iter_shared()
            .map(|entry| {
                let entry = entry.to::<VarDictionary>();
                let flat = entry.get("tiles").map_or_else(PackedInt32Array::new, |tiles| tiles.to::<PackedInt32Array>());
                let tiles = flat.as_slice().as_chunks::<2>().0.iter().map(|tile| Tile { x: tile[0], z: tile[1] }).collect();
                (int(&entry, "level") as u8, int(&entry, "step") as u8, tiles)
            })
            .collect();
        let staged = array("staged")
            .iter_shared()
            .map(|entry| {
                let dict = entry.to::<VarDictionary>();
                let (graded, adopted) = (flag(&dict, "graded"), flag(&dict, "adopted"));
                EditTile { baked: Baked::from_dict(dict), graded, adopted }
            })
            .collect();
        Self {
            serial: int(edit, "serial"),
            all: flag(edit, "all"),
            levels,
            preview_epoch: edit.get("preview_epoch").map_or(-1, |epoch| epoch.to::<i64>()),
            staged,
        }
    }
}

impl GradedTileBook {
    // `take_edit_counts`' counts, drained.
    pub(super) fn edit_counts(&mut self) -> [i64; COUNTS] {
        std::mem::replace(&mut self.edits.counts, [0; COUNTS])
    }

    // The tiles the last batch installed (tests).
    #[cfg(test)]
    pub(super) fn last_batch_tiles(&self) -> &[Tile] {
        &self.edits.last_batch
    }

    // A batched tile's polled bake (`_stage_edit`): it waits for the batch's other tiles (and,
    // flagged, its sets' variant, queued as it stages).
    pub(super) fn stage_baked(&mut self, baked: Baked) {
        let tile = baked.tile;
        if let Some(old) = self.edits.staged.insert(
            tile,
            Staged {
                baked,
                adopted: false,
                preview_epoch: -1,
            },
        ) {
            self.taken.recycle_baked(old.baked);
        }
    }

    // `_failed`'s refusal of a batched tile: it leaves the batch without a result.
    pub(super) fn unstage(&mut self, tile: Tile) {
        let mut taken = std::mem::take(&mut self.taken);
        self.edits.dropped(tile, &mut taken);
        self.taken = taken;
    }

    // Whether any staged tile lacks its sets' variant. Every flagged tile is asked (each asking
    // queues the variant), as the GDScript's loop did.
    fn batch_waits_variant(&mut self) -> bool {
        let mut keys = std::mem::take(&mut self.edits.keys);
        keys.clear();
        keys.extend(
            self.edits
                .staged
                .iter()
                .filter(|(_, staged)| staged.baked.behind)
                .map(|(tile, _)| *tile),
        );
        let mut waiting = false;
        for &tile in &keys {
            waiting |= self.variant_missing(Vector2i::new(tile.x, tile.z));
        }
        keys.clear();
        self.edits.keys = keys;
        waiting
    }

    // `_adopt_preview` of `tile` (a commit's own tile that is the preview's payload). Returns
    // whether it adopted.
    fn adopt_staged(&mut self, tile: Tile) -> bool {
        let Some(staged) = self.edits.staged.get(&tile) else {
            return false;
        };
        if !staged.adopted {
            return false;
        }
        let level = (staged.baked.code, staged.baked.step, staged.baked.token);
        self.state.adopt_preview(tile, level, staged.preview_epoch)
    }

    // `_reserve_edit_copies`: every copy of the batch placed, all or none, as `reserve_edit`
    // answers it (`[0, then per tile its count and copies]`, `[1]` or `[2, x, z]`).
    fn reserve_batch(&mut self, out: &mut Vec<i64>) {
        let mut flat = std::mem::take(&mut self.edits.reserve);
        flat.clear();
        for (tile, staged) in self.edits.staged.iter() {
            let baked = &staged.baked;
            let layers = (baked.rows.as_slice().len() / uploads::ATLAS_LAYER_BYTES) as i64;
            flat.extend([
                i64::from(tile.x),
                i64::from(tile.z),
                i64::from(baked.code),
                i64::from(baked.step),
                baked.token,
                layers,
            ]);
        }
        self.reserve_edit(&flat, out);
        self.edits.reserve = flat;
    }

    // `_upload` of a batched tile into its reserved copies (flat page, slot, first, layers).
    fn upload_staged(&mut self, baked: Baked, copies: &[i64]) {
        let tile = Vector2i::new(baked.tile.x, baked.tile.z);
        let level = Vector4i::new(
            i32::from(baked.code),
            i32::from(baked.step),
            baked.token as i32,
            i32::from(baked.behind),
        );
        let layers = (baked.rows.as_slice().len() / uploads::ATLAS_LAYER_BYTES) as i64;
        let mut walls = std::mem::take(&mut self.taken.walls);
        walls.clear();
        walls.extend(
            baked
                .walls
                .as_slice()
                .as_chunks::<4>()
                .0
                .iter()
                .map(|bytes| f32::from_le_bytes(*bytes)),
        );
        let copies = copies
            .as_chunks::<4>()
            .0
            .iter()
            .map(|copy| {
                (
                    copy[0] as u32,
                    Copy {
                        slot: copy[1] as u32,
                        first: copy[2] as u32,
                        layers: copy[3] as u32,
                    },
                )
            })
            .collect();
        self.taken.recycle_bytes(baked.walls);
        self.stage(tile, level, baked.texels, baked.rows);
        self.upload_with(tile, level, layers, Some(copies), &walls);
        self.taken.walls = walls;
    }

    // `_show_preview_tile`'s placing: 0 placed, 1 a full page waits, 2 exhausted.
    fn place_preview_baked(&mut self, baked: Baked) -> i64 {
        let tile = Vector2i::new(baked.tile.x, baked.tile.z);
        let level = Vector4i::new(
            i32::from(baked.code),
            i32::from(baked.step),
            baked.token as i32,
            i32::from(baked.behind),
        );
        let layers = (baked.rows.as_slice().len() / uploads::ATLAS_LAYER_BYTES) as u32;
        // The faces into the reused scratch (a preview pose's frame allocates none, AUDIT-stream 11).
        let mut walls = std::mem::take(&mut self.walls_scratch);
        walls.clear();
        walls.extend(
            baked
                .walls
                .as_slice()
                .as_chunks::<4>()
                .0
                .iter()
                .map(|bytes| f32::from_le_bytes(*bytes)),
        );
        self.taken.recycle_bytes(baked.walls);
        self.stage(tile, level, baked.texels, baked.rows);
        let tile = tile_of(tile);
        let level = (
            level.x as u8,
            level.y as u8,
            i64::from(level.z),
            level.w != 0,
        );
        self.state.settle();
        self.state.shown_copy(tile, &mut self.before);
        self.state.cells.staged_walls = Some((tile, walls));
        let placed = place_code(
            self.state
                .place_preview(tile, level, layers, &mut self.scratch),
        );
        self.state.settle();
        if let Some((_, walls)) = self.state.cells.staged_walls.take() {
            self.walls_scratch = walls;
        }
        if placed == 0 {
            let before = std::mem::take(&mut self.before);
            self.state.shown_changed(tile, &before);
            self.before = before;
        }
        placed
    }
}

// The edit batch's and the preview's steps, the book unbound around the renderer's callbacks.
impl<H: Host> Run<H> {
    // `_staged_dirty`: the effects of the dirtying call applied, then a staged result of a
    // dirtied tile dropped, its level kept for the rebake.
    fn staged_dirty(&mut self, tiles: &[Tile]) {
        self.effects();
        let mut book = self.host.book();
        for &tile in tiles {
            if let Some(staged) = book.edits.staged.remove(&tile) {
                book.state
                    .requests
                    .edit_levels
                    .insert(tile, (staged.baked.code, staged.baked.step));
                book.taken.recycle_baked(staged.baked);
            }
        }
    }

    // `_apply_edit`: the tiles an edit dirtied rebake at the level each is held at, installed
    // or in flight; a commit's or undo's own tiles stage for the batch.
    pub(super) fn apply_edit(&mut self, edit: Edit) {
        let serial = edit.serial;
        let mut tiles = {
            let mut book = self.host.book();
            book.edits.counts[EDITS] += 1;
            book.edits.serial_taken = serial;
            std::mem::take(&mut book.tiles)
        };
        if edit.all {
            // Every batched, installed, requested and refused tile, in that order.
            self.host.book().state.dirty_all(serial, &mut tiles);
            self.staged_dirty(&tiles);
            self.host.book().tiles = tiles;
            return;
        }
        for (code, step, dirty) in &edit.levels {
            // The tiles held, batched, requested or refused at the level (a refused tile is held
            // too: the edit may have changed what refused it).
            self.host.book().state.dirty_level(*code, *step, dirty, serial, &mut tiles);
            self.staged_dirty(&tiles);
        }
        self.host.book().tiles = tiles;
        // A commit's or undo's own tiles (TERRAIN-07-E): bitwise the rebakes of the edit's
        // inputs, so they install with the batch instead of rebaking. A tile neither held nor
        // wanted installs where it is graded and a resident patch meets it (`TERRAIN-23`); any
        // other waits for its list.
        let epoch = edit.preview_epoch;
        for entry in edit.staged {
            let tile = entry.baked.tile;
            let token = self.host.book().state.stage_edit_tile(tile, entry.graded, serial);
            match token {
                Some(_) => self.staged_dirty(&[tile]),
                None => {
                    self.staged_dirty(&[]);
                    continue;
                }
            }
            let mut baked = entry.baked;
            baked.token = token.unwrap_or(0);
            let mut book = self.host.book();
            if baked.behind {
                book.variant_missing(Vector2i::new(tile.x, tile.z));
            }
            if let Some(old) = book.edits.staged.insert(
                tile,
                Staged {
                    baked,
                    adopted: entry.adopted,
                    preview_epoch: epoch,
                },
            ) {
                book.taken.recycle_baked(old.baked);
            }
        }
        let mut book = self.host.book();
        book.edits.release = book.edits.release.max(epoch);
    }

    // `_install_edit_batch`: the batch installs once every tile of it has its result, all in
    // this frame. Returns how many tiles it installed.
    pub(super) fn install_edit_batch(&mut self) -> i64 {
        let (batch, release, staged) = {
            let book = self.host.book();
            (
                book.state.requests.edit_batch.len(),
                book.edits.release,
                book.edits.staged.len(),
            )
        };
        if batch == 0 && release >= 0 {
            // A commit whose staged tiles no held tile takes: its preview ends with the edit (the
            // renderer's own entry point, which a trace records).
            self.host.book().edits.release = -1;
            self.release_preview(0, release);
        }
        if batch == 0 || staged < batch {
            return 0;
        }
        // A flagged tile's sets take the variant first (queued as it staged): the batch installs
        // whole once none lacks it.
        if self.host.book().batch_waits_variant() {
            self.host.book().taken.counts[VARIANT_WAITS] += 1;
            return 0;
        }
        // Adoption changes ownership only: the same preview slots remain drawn. The replaced
        // installed copies are freed before the batch's allocations are planned.
        let mut keys = std::mem::take(&mut self.host.book().edits.keys);
        keys.clear();
        keys.extend(self.host.book().edits.staged.keys().copied());
        for &tile in &keys {
            if self.host.book().adopt_staged(tile) {
                self.effects();
                self.host.book().edits.counts[ADOPTED] += 1;
            }
        }
        let mut reserved = Vec::new();
        self.host.book().reserve_batch(&mut reserved);
        self.effects();
        let reserved = reserved.as_slice();
        if reserved[0] == 2 {
            // Its level is exhausted at the device or budget cap (the explicit pool-exhaustion
            // contract): it leaves the batch without an upload.
            self.host.book().unstage(Tile {
                x: reserved[1] as i32,
                z: reserved[2] as i32,
            });
        }
        if reserved[0] != 0 {
            self.host.book().edits.keys = keys;
            return 0;
        }
        // Every allocation fits before any published texture is overwritten.
        keys.clear();
        keys.extend(self.host.book().edits.staged.keys().copied());
        let mut at = 1;
        let mut installed = Vec::with_capacity(keys.len());
        for &tile in &keys {
            let count = reserved[at] as usize;
            let copies = &reserved[at + 1..at + 1 + 4 * count];
            at += 1 + 4 * count;
            let staged = self.host.book().edits.staged.remove(&tile);
            if let Some(staged) = staged {
                self.host.book().upload_staged(staged.baked, copies);
                self.effects();
                installed.push(tile);
            }
        }
        let count = installed.len() as i64;
        let release = {
            let mut book = self.host.book();
            book.edits.last_batch = installed;
            book.state.requests.edit_batch.clear();
            book.state.requests.edit_levels.clear();
            book.edits.serial_batched = book.edits.serial_taken;
            book.edits.keys = keys;
            std::mem::replace(&mut book.edits.release, -1)
        };
        // The committed tiles replace the preview's in the same frame.
        if release >= 0 {
            let released = self.host.book().preview_tiles(-(1 << 62), release);
            for tile in released {
                self.drop_preview(tile);
            }
            self.host.book().drop_preview_waits(0, release);
        }
        count
    }

    // `_drop_preview`: the installed tile draws again.
    pub(super) fn drop_preview(&mut self, tile: Tile) {
        {
            let mut book = self.host.book();
            if let Some(baked) = book.edits.preview_waits.remove(&tile) {
                book.taken.recycle_baked(baked);
            }
            book.drop_preview(Vector2i::new(tile.x, tile.z));
        }
        self.effects();
    }

    // `_show_preview_tile`: a flagged tile shows once its sets hold the variant (the preview's
    // lag, T27 (d)); otherwise placed, what it draws changed, or it waits for its page.
    pub(super) fn show_preview_tile(&mut self, baked: Baked) {
        let tile = baked.tile;
        let placed = {
            let mut book = self.host.book();
            if let Some(old) = book.edits.preview_waits.remove(&tile) {
                book.taken.recycle_baked(old);
            }
            if baked.behind && book.variant_missing(Vector2i::new(tile.x, tile.z)) {
                book.taken.counts[VARIANT_WAITS] += 1;
                book.edits.preview_waits.insert(tile, baked);
                return;
            }
            let code = baked.code;
            let placed = book.place_preview_baked(baked);
            (placed, code)
        };
        self.effects();
        let (placed, code) = placed;
        // The preview's lag (T27 (d)): a page that is full draws the tile once a later pose
        // finds room (its block grows meanwhile). A page that cannot grow (its cap or the
        // budget) is the level's pool exhausted, as the install path records it.
        let mut book = self.host.book();
        match placed {
            1 => {
                book.edits.counts[PREVIEW_PAGE_WAITS] += 1;
                crate::debug_log!(
                    "terrain",
                    "graded level {code} page full at preview tile ({}, {})",
                    tile.x,
                    tile.z
                );
            }
            2 => {
                book.edits.counts[PREVIEW_EXHAUSTED] += 1;
                crate::debug_log!(
                    "terrain",
                    "graded pool {code} exhausted at preview tile ({}, {})",
                    tile.x,
                    tile.z
                );
            }
            _ => {}
        }
    }
}

impl GradedTileBook {
    // `_drop_preview_waits`: the waiting preview tiles of epochs `first` to `last` dropped.
    fn drop_preview_waits(&mut self, first: i64, last: i64) {
        let mut keys = std::mem::take(&mut self.edits.keys);
        keys.clear();
        keys.extend(
            self.edits
                .preview_waits
                .iter()
                .filter(|(_, baked)| baked.token >= first && baked.token <= last)
                .map(|(tile, _)| *tile),
        );
        for tile in &keys {
            if let Some(baked) = self.edits.preview_waits.remove(tile) {
                self.taken.recycle_baked(baked);
            }
        }
        keys.clear();
        self.edits.keys = keys;
    }
}

// The preview's steps a frame or a release takes (`show_preview_frame`, `release_preview_epochs`,
// `take_preview_waits`).
impl<H: Host> Run<H> {
    // One road preview frame: `removed` (flat x, z) drop back to their installed tile, then each
    // of `tiles` takes slots of its own.
    pub(super) fn preview_frame(&mut self, removed: &[i32], tiles: impl IntoIterator<Item = Baked>) {
        for tile in removed.as_chunks::<2>().0 {
            self.drop_preview(Tile { x: tile[0], z: tile[1] });
        }
        for baked in tiles {
            self.show_preview_tile(baked);
        }
    }

    // The preview tiles of epochs `first` to `last` dropped, and those waiting for their variant.
    pub(super) fn release_epochs(&mut self, first: i64, last: i64) {
        let tiles = self.host.book().preview_tiles(first, last);
        for tile in tiles {
            self.drop_preview(tile);
        }
        self.host.book().drop_preview_waits(first, last);
    }

    // The preview tiles waiting for their sets' variant that can show now do, in the order they
    // began waiting.
    pub(super) fn preview_waits(&mut self) {
        let mut keys = {
            let mut book = self.host.book();
            let mut keys = std::mem::take(&mut book.edits.keys);
            keys.clear();
            keys.extend(book.edits.preview_waits.keys().copied());
            keys
        };
        for &tile in &keys {
            let ready = {
                let mut book = self.host.book();
                if book.variant_missing(Vector2i::new(tile.x, tile.z)) {
                    None
                } else {
                    book.edits.preview_waits.remove(&tile)
                }
            };
            if let Some(baked) = ready {
                self.show_preview_tile(baked);
            }
        }
        keys.clear();
        self.host.book().edits.keys = keys;
    }
}

#[godot_api(secondary)]
impl GradedTileBook {
    /// An edit the simulation's poll handed over (`_apply_edit`): the tiles it dirtied rebake
    /// at their held level, its own tiles stage for the batch. `edit` as
    /// `poll_graded_tile_results` packs it. O(its dirty and staged tiles).
    #[func]
    pub fn apply_edit(book: Gd<GradedTileBook>, renderer: Gd<RefCounted>, edit: VarDictionary) {
        Run::unbudgeted(ScriptHost::new(book, renderer)).apply_edit(Edit::from_dict(&edit));
    }

    /// The frame's edit batch (`_install_edit_batch`): installed whole once every tile has its
    /// result and its sets' variant, the released preview tiles dropped in the same frame.
    /// Returns the tiles installed (0: none this frame). O(batch copies + the touched pages' free
    /// runs) plus the writes.
    #[func]
    pub fn install_edit_batch(book: Gd<GradedTileBook>, renderer: Gd<RefCounted>) -> i64 {
        Run::unbudgeted(ScriptHost::new(book, renderer)).install_edit_batch()
    }

    /// Stages a batched tile's result by hand, as a polled bake of it does (tests; `tile` as
    /// `poll_graded_tile_results` packs it).
    #[func]
    pub fn stage_edit_result(&mut self, tile: VarDictionary) {
        self.stage_baked(Baked::from_dict(tile));
    }

    /// A refused batched `tile` (by contract) leaves the batch without a result (`_failed`).
    #[func]
    pub fn edit_unstage(&mut self, tile: Vector2i) {
        self.unstage(tile_of(tile));
    }

    /// Dirties `tile` with the edit's `serial` (`_dirty_tile`; tests): a bake in flight with an
    /// older token is dropped and asked again, an installed tile joins the batch, a staged result
    /// of it is dropped.
    #[func]
    pub fn dirty_edit_tile(
        book: Gd<GradedTileBook>,
        renderer: Gd<RefCounted>,
        tile: Vector2i,
        serial: i64,
    ) {
        let mut run = Run::unbudgeted(ScriptHost::new(book, renderer));
        run.host.book().state.dirty_tile(tile_of(tile), serial);
        run.staged_dirty(&[tile_of(tile)]);
    }

    /// Sets the serial of the last edit taken (tests).
    #[func]
    pub fn set_edit_serial_taken(&mut self, serial: i64) {
        self.edits.serial_taken = serial;
    }

    /// One road preview frame's tiles (`show_preview`'s loops): `removed` (flat x, z) drop back to
    /// their installed tile, then each of `tiles` (packed as `poll_graded_tile_results` packs
    /// its tiles, the token the preview epoch) takes slots of its own. O(changed tiles) plus
    /// their placing and writes.
    #[func]
    pub fn show_preview_frame(
        book: Gd<GradedTileBook>,
        renderer: Gd<RefCounted>,
        removed: PackedInt32Array,
        tiles: VarArray,
    ) {
        let tiles = tiles.iter_shared().map(|tile| Baked::from_dict(tile.to::<VarDictionary>()));
        Run::unbudgeted(ScriptHost::new(book, renderer)).preview_frame(removed.as_slice(), tiles);
    }

    /// One preview tile (`_show_preview_tile`; tests).
    #[func]
    pub fn show_preview_tile(
        book: Gd<GradedTileBook>,
        renderer: Gd<RefCounted>,
        tile: VarDictionary,
    ) {
        Run::unbudgeted(ScriptHost::new(book, renderer)).show_preview_tile(Baked::from_dict(tile));
    }

    /// Drops the preview tiles of epochs `first` to `last` and those waiting for their variant
    /// (`release_preview`'s part before its cell and wall changes). O(preview tiles).
    #[func]
    pub fn release_preview_epochs(
        book: Gd<GradedTileBook>,
        renderer: Gd<RefCounted>,
        first: i64,
        last: i64,
    ) {
        Run::unbudgeted(ScriptHost::new(book, renderer)).release_epochs(first, last);
    }

    /// The preview tiles waiting for their sets' variant that can show now do (in the order they
    /// began waiting). O(waiting tiles).
    #[func]
    pub fn take_preview_waits(book: Gd<GradedTileBook>, renderer: Gd<RefCounted>) {
        Run::unbudgeted(ScriptHost::new(book, renderer)).preview_waits();
    }

    /// Whether a preview tile waits for its sets' variant.
    #[func]
    pub fn has_preview_waits(&self) -> bool {
        self.edits.preview_waits.len() > 0
    }

    /// The serial of the last graded edit whose tiles draw (T61 (2)): every edit taken once no
    /// batch waits, else those up to the last installed batch. O(1).
    #[func]
    pub fn installed_edit_serial(&self) -> i64 {
        if self.state.requests.edit_batch.len() == 0 && self.edits.release < 0 {
            self.edits.serial_taken
        } else {
            self.edits.serial_batched
        }
    }

    /// The highest preview epoch the batch releases when it installs (-1: none).
    #[func]
    pub fn edit_release(&self) -> i64 {
        self.edits.release
    }

    /// The batch's staged results.
    #[func]
    pub fn edit_staged_len(&self) -> i64 {
        self.edits.staged.len() as i64
    }

    /// The tiles the last batch installed, flat `(x, z)`.
    #[func]
    pub fn last_edit_batch(&self) -> PackedInt32Array {
        self.edits
            .last_batch
            .iter()
            .flat_map(|tile| [tile.x, tile.z])
            .collect()
    }

    /// The edit path's counts since the last read: edits taken, committed tiles that adopted the
    /// preview's slot, preview tiles a full page left to a later pose, preview tiles whose pool
    /// is exhausted.
    #[func]
    pub fn take_edit_counts(&mut self) -> PackedInt64Array {
        PackedInt64Array::from(self.edit_counts().as_slice())
    }
}
