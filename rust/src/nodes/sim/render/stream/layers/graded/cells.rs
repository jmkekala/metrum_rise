// SPDX-License-Identifier: GPL-2.0-only

//! The graded renderer's cell sets and walls in Rust (`RENDER-32-J`, `TERRAIN-13` part 3), ported
//! from `terrain_graded_tiles.gd` by record and replay with their order of effects: per resident
//! patch one set of cell `MultiMesh`es (the base cells, `GL0`-`GL3` and the behind-the-road
//! variant's four), each instance a tile's place, slot and stitching codes; the per-tile updates
//! (`_update_cell_tiles`), the whole set's build (`_begin_build`, `_build_tiles`), the wall faces
//! (`_rebuild_patch_walls`) and the marks that queue them. The CPU buffers are Rust's; the
//! GDScript publishes them and keeps the Godot objects (nodes, materials, their parameters),
//! applying the [`Effect`]s in order.
//!
//! The marks the other parts of the state push ([`Effect::Bind`], [`Effect::Installed`] and the
//! rest) are applied here where the GDScript applied them ([`Graded::settle`]), so every queue
//! keeps the GDScript's insertion order and every output its place.
//!
//! Bounds: a tile's update `O(1)` (its four sides each find one patch); a set's build `O(the
//! patch's tiles)`, budgeted per tile; walls `O(faces of the patch's tiles)`; a mark `O(the
//! patches meeting a tile)`; a bind `O(the block's sets x levels)`. No allocation once the sets
//! are prewarmed, but a set's first behind-the-road variant (the build path, as before).

use super::ordered::OrderedMap;
use super::state::{Effect, Entry, Graded, PatchKey, TILE_M, Tile};

/// Cell meshes of a set: `GL0`-`GL3`, the base cells, the variant's `GL0`-`GL3`.
pub(crate) const CELL_MESHES: usize = 9;
pub(crate) const CELL_BASE: usize = 4;
pub(crate) const CELL_BEHIND: usize = 5;
pub(crate) const CELL_PLAIN: usize = 5;
// Three transform rows followed by custom data, as `MultiMesh.TRANSFORM_3D` packs them.
const CELL_INSTANCE_FLOATS: usize = 16;
const PATCH_TILE_ROW: i32 = 32;
/// Wall face instances a set's wall `MultiMesh` starts with.
pub(crate) const MAX_WALLS_PER_PATCH: u32 = 256;
const WALL_FACE_FLOATS: usize = 8;
/// Floats a wall face's instance takes in [`Cells::wall_floats`]: its three basis columns.
pub(crate) const WALL_INSTANCE_FLOATS: usize = 9;

/// What a resident patch draws (T53 (a)).
pub(crate) const DRAW_PLANE: u8 = 0;
pub(crate) const DRAW_SET: u8 = 1;
pub(crate) const DRAW_NONE: u8 = 2;

/// A resident patch entry as `terrain.gd` keeps it: rectangle (`real_t`), LOD step, generation.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Patch {
    pub rect: [f32; 4],
    pub step: i32,
    pub generation: i64,
}

/// A cell instance's custom data (place, slot, own and side codes), as a `Color`.
type Custom = [f32; 4];

/// One patch's cell set (the GDScript's `patch_cells` entry, its Godot objects aside).
#[derive(Clone, Debug)]
pub(crate) struct CellSet {
    pub key: PatchKey,
    pub step: i32,
    pub tiles: Vec<Tile>,
    pub first: Tile,
    pub last: Tile,
    // Tiles its build has written (-1: not begun).
    pub built: i32,
    pub shown: bool,
    // Whether it drew complete in its patch's residency.
    pub complete: bool,
    // Whether it holds the behind-the-road variant's meshes.
    pub variant: bool,
    // The page its materials bind per level (-1: none).
    pub pages: [i64; 4],
    // The base cells' mesh (lattice code, at most 6), as the GDScript last set it.
    pub base_mesh: Option<u8>,
    pub counts: [u32; CELL_MESHES],
    // Per tile place its (mesh, instance); (-1, -1) while not written.
    pub at: Vec<(i8, i32)>,
    // Per mesh the custom data and tile of each instance, and its complete CPU buffer.
    pub custom: Vec<Vec<Custom>>,
    pub tile_at: Vec<Vec<Tile>>,
    pub buffers: Vec<Vec<f32>>,
    pub dirty: [bool; CELL_MESHES],
    pub wall_capacity: u32,
    // Its grass mask revision (grass renders the patch's mask again when it moves).
    pub mask_revision: i64,
}

impl CellSet {
    fn meshes(&self) -> usize {
        if self.variant { CELL_MESHES } else { CELL_PLAIN }
    }

    pub(crate) fn is_built(&self) -> bool {
        self.built >= 0 && self.built as usize == self.tiles.len()
    }

    pub(crate) fn place(&self, tile: Tile) -> usize {
        ((tile.x - self.first.x) + (self.last.x - self.first.x + 1) * (tile.z - self.first.z)) as usize
    }

    fn holds(&self, tile: Tile) -> bool {
        tile.x >= self.first.x && tile.z >= self.first.z && tile.x <= self.last.x && tile.z <= self.last.z
    }

    // Instances in the graded meshes (every one but the base cells').
    pub(crate) fn graded_cells(&self) -> u32 {
        (0..CELL_MESHES).filter(|&index| index != CELL_BASE).map(|index| self.counts[index]).sum()
    }
}

/// The cells' half of the state.
#[derive(Debug, Default)]
pub(crate) struct Cells {
    pub(crate) sets: Vec<CellSet>,
    // Patch key -> its set, in the GDScript's `patch_cells` order; the free sets.
    pub(crate) by_patch: OrderedMap<PatchKey, u32>,
    pub(crate) pool: Vec<u32>,
    pub(crate) per_patch: usize,
    identity: Vec<f32>,
    // Per level code block -> the page its patches' sets bind, as the binds set it in order (the
    // GDScript's `_block_page`).
    pub(crate) bound: [std::collections::HashMap<(i32, i32), u32>; 4],
    pub(crate) dirty_patches: OrderedMap<PatchKey, ()>,
    pub(crate) dirty_walls: OrderedMap<PatchKey, ()>,
    pub(crate) dirty_tiles: OrderedMap<Tile, ()>,
    pub(crate) dirty_shown: OrderedMap<PatchKey, ()>,
    pub(crate) builds: OrderedMap<PatchKey, ()>,
    pub(crate) builds_urgent: OrderedMap<PatchKey, ()>,
    pub(crate) variants_wanted: OrderedMap<PatchKey, ()>,
    pub(crate) patch_steps: std::collections::HashMap<PatchKey, i32>,
    // The last mask revision given, and how many were (`mask_revision_bumps`).
    pub(crate) mask_revision: i64,
    pub(crate) mask_bumps: u64,
    // The tile whose walls the call in progress stages (`_entry_walls`).
    pub(crate) staged_walls: Option<(Tile, Vec<f32>)>,
    // Published buffers and wall faces of the effects not yet taken.
    pub(crate) publish_floats: Vec<f32>,
    pub(crate) wall_floats: Vec<f32>,
    // Those of the effects the GDScript took last (it reads them as it applies them).
    pub(crate) taken_publish: Vec<f32>,
    pub(crate) taken_walls: Vec<f32>,
    // Per-frame counters: tiles updated, flips, instances written, moved.
    pub(crate) frame: [u64; 4],
    // Scratch.
    touched: OrderedMap<PatchKey, ()>,
    tile_scratch: Vec<Tile>,
    key_scratch: Vec<PatchKey>,
    meeting: Vec<PatchKey>,
    patch_scratch: Vec<PatchKey>,
    erased: Vec<Tile>,
}

/// A copy of what a tile shows, for `_shown_changed`.
#[derive(Clone, Debug, Default)]
pub(crate) struct Shown {
    pub cell_code: Option<u8>,
    pub walls: Vec<f32>,
}

/// `base_code`: the lattice code of the base cells and `GL3` at a patch LOD step.
pub(crate) fn base_code(step: i32) -> u8 {
    (3.0 + (f64::from(step.max(1)).ln() / 2f64.ln()).round()) as u8
}

impl Graded {
    /// Setup's prewarm: `count` sets of `per_patch` tile places, all free (the last taken first).
    pub(crate) fn cells_setup(&mut self, per_patch: usize, count: usize) {
        let cells = &mut self.cells;
        cells.per_patch = per_patch;
        cells.identity = vec![0.0; per_patch * CELL_INSTANCE_FLOATS];
        for instance in 0..per_patch {
            let offset = instance * CELL_INSTANCE_FLOATS;
            cells.identity[offset] = 1.0;
            cells.identity[offset + 5] = 1.0;
            cells.identity[offset + 10] = 1.0;
        }
        cells.sets.clear();
        cells.by_patch.clear();
        cells.pool.clear();
        for _ in 0..count {
            let set = self.new_set();
            self.cells.pool.push(set);
        }
        for code in 0..4 {
            self.cells.bound[code].clear();
        }
        for code in 0..self.caps.len().min(2) {
            if let Some(&page) = self.block_page[code].get(&(0, 0)) {
                self.cells.bound[code].insert((0, 0), page);
            }
        }
    }

    fn new_set(&mut self) -> u32 {
        let cells = &mut self.cells;
        let id = cells.sets.len() as u32;
        let mut set = CellSet {
            key: (0, 0),
            step: -1,
            tiles: Vec::new(),
            first: Tile { x: 0, z: 0 },
            last: Tile { x: -1, z: -1 },
            built: -1,
            shown: false,
            complete: false,
            variant: false,
            pages: [-1; 4],
            base_mesh: None,
            counts: [0; CELL_MESHES],
            at: vec![(-1, -1); cells.per_patch],
            custom: Vec::new(),
            tile_at: Vec::new(),
            buffers: Vec::new(),
            dirty: [false; CELL_MESHES],
            wall_capacity: MAX_WALLS_PER_PATCH,
            mask_revision: 0,
        };
        Self::add_records(&mut set, &cells.identity, cells.per_patch, CELL_PLAIN);
        cells.sets.push(set);
        id
    }

    // `_add_cell_records`: `meshes` more meshes' records and complete CPU buffers.
    fn add_records(set: &mut CellSet, identity: &[f32], per_patch: usize, meshes: usize) {
        for _ in 0..meshes {
            set.custom.push(vec![[0.0; 4]; per_patch]);
            set.tile_at.push(vec![Tile { x: 0, z: 0 }; per_patch]);
            set.buffers.push(identity.to_vec());
        }
    }

    /// The live resident dictionary's mutation (`patch` none: erased).
    pub(crate) fn set_patch(&mut self, key: PatchKey, patch: Option<Patch>) {
        match patch {
            Some(patch) => {
                self.resident.insert(key, patch);
            }
            None => {
                self.resident.remove(&key);
            }
        }
    }

    // --- Pages and what a tile shows. ---

    // `_patch_page`: the page patch `key`'s sets bind at level `code` (-1: none yet).
    fn patch_page(&self, key: PatchKey, code: u8) -> i64 {
        let block = self.block_of(key, code);
        self.cells.bound[usize::from(code)].get(&block).map_or(-1, |&page| i64::from(page))
    }

    fn has_copy(&self, entry: &Entry, key: PatchKey) -> bool {
        let page = self.patch_page(key, entry.level);
        page >= 0 && entry.copy_in(page as u32).is_some()
    }

    /// `_shown_in`: what `tile` draws in patch `key`'s cells: its preview tile where the page of
    /// the patch's block at its level holds a copy, else its installed tile there, else none.
    pub(crate) fn shown_in(&self, tile: Tile, key: PatchKey) -> Option<&Entry> {
        if let Some(entry) = self.preview.get(&tile)
            && self.has_copy(entry, key)
        {
            return Some(entry);
        }
        self.installed.get(&tile).filter(|entry| self.has_copy(entry, key))
    }

    // `_shown`: its preview tile while the preview shows one, else its installed tile.
    fn shown(&self, tile: Tile) -> Option<&Entry> {
        self.preview.get(&tile).or_else(|| self.installed.get(&tile))
    }

    /// What `tile` shows now, for `shown_changed`.
    pub(crate) fn shown_copy(&self, tile: Tile, out: &mut Shown) {
        let entry = self.shown(tile);
        out.cell_code = entry.map(|entry| entry.cell_code);
        out.walls.clear();
        if let Some(entry) = entry {
            out.walls.extend_from_slice(&entry.walls);
        }
    }

    /// `_shown_changed`: what `tile` draws changed from `before`.
    pub(crate) fn shown_changed(&mut self, tile: Tile, before: &Shown) {
        let after = self.shown(tile);
        let sides = before.cell_code.is_none() || after.is_none_or(|entry| Some(entry.cell_code) != before.cell_code);
        let walls_changed = after.map_or(&[][..], |entry| &entry.walls[..]) != &before.walls[..];
        self.cell_changed(tile, sides);
        if walls_changed {
            self.walls_changed(tile);
        }
        self.await_cells(tile);
    }

    // `_cell_code`: a cell's lattice code in patch `key`.
    fn cell_code_in(&self, tile: Tile, step: i32, key: PatchKey) -> u8 {
        self.shown_in(tile, key).map_or_else(|| base_code(step), |entry| entry.cell_code)
    }

    // `_cell_mesh`: the cell mesh a tile draws in, in patch `key`.
    pub(crate) fn cell_mesh(&self, tile: Tile, key: PatchKey) -> usize {
        self.shown_in(tile, key).map_or(CELL_BASE, |entry| {
            usize::from(entry.level) + if entry.behind { CELL_BEHIND } else { 0 }
        })
    }

    // The patches whose rectangle meets the rectangle at `position` of `size` (`_patches_meeting`,
    // `real_t` math), z-major, into `out`.
    fn patches_meeting(&self, position: [f32; 2], size: [f32; 2], out: &mut Vec<PatchKey>) {
        out.clear();
        let span = self.shape.patch_span;
        let min = self.shape.world_min;
        let first = [0, 1].map(|axis| ((position[axis] - min[axis]) / span).floor() as i32);
        let last = [0, 1].map(|axis| ((position[axis] + size[axis] - min[axis]) / span).floor() as i32);
        for z in first[1]..=last[1] {
            for x in first[0]..=last[0] {
                out.push((x, z));
            }
        }
    }

    fn tile_patches(&mut self, tile: Tile) -> Vec<PatchKey> {
        let mut keys = std::mem::take(&mut self.cells.key_scratch);
        self.patches_meeting([tile.x as f32 * TILE_M, tile.z as f32 * TILE_M], [TILE_M, TILE_M], &mut keys);
        keys
    }

    // `_side_code`: the code a cell's side stitches to.
    fn side_code(&self, own: u8, neighbour: Tile, side_m: f64, patch: &Patch, axis: usize, own_key: PatchKey) -> u8 {
        let lo = f64::from(patch.rect[axis]);
        let hi = f64::from(patch.rect[axis] + patch.rect[axis + 2]);
        if side_m > lo && side_m < hi {
            return own.max(self.cell_code_in(neighbour, patch.step, own_key));
        }
        if side_m != lo && side_m != hi {
            return own;
        }
        let mut probe = [(neighbour.x as f32 + 0.5) * TILE_M, (neighbour.z as f32 + 0.5) * TILE_M];
        probe[axis] = if side_m == lo { (lo - 0.5) as f32 } else { (hi + 0.5) as f32 };
        let span = self.shape.patch_span;
        let min = self.shape.world_min;
        let key = (((probe[0] - min[0]) / span).floor() as i32, ((probe[1] - min[1]) / span).floor() as i32);
        if let Some(other) = self.resident.get(&key) {
            let shown = self
                .cells
                .by_patch
                .get(&key)
                .is_some_and(|&set| self.cells.sets[set as usize].shown);
            if shown {
                return own.max(self.cell_code_in(neighbour, other.step, key));
            }
            return own.max(base_code(other.step));
        }
        own
    }

    // `_cell_custom`: a tile's instance custom data in patch `key`'s cells.
    pub(crate) fn cell_custom(&self, tile: Tile, first: Tile, patch: &Patch, key: PatchKey) -> Custom {
        let own = self.cell_code_in(tile, patch.step, key);
        let origin = [tile.x as f32 * TILE_M, tile.z as f32 * TILE_M];
        let tile_m = f64::from(TILE_M);
        let mut codes = u32::from(own);
        let side = |neighbour: Tile, side_m: f64, axis: usize| u32::from(self.side_code(own, neighbour, side_m, patch, axis, key));
        codes |= side(Tile { x: tile.x - 1, z: tile.z }, f64::from(origin[0]), 0) << 3;
        codes |= side(Tile { x: tile.x + 1, z: tile.z }, f64::from(origin[0]) + tile_m, 0) << 6;
        codes |= side(Tile { x: tile.x, z: tile.z - 1 }, f64::from(origin[1]), 1) << 9;
        codes |= side(Tile { x: tile.x, z: tile.z + 1 }, f64::from(origin[1]) + tile_m, 1) << 12;
        let slot = self.shown_in(tile, key).map_or(-1, |entry| {
            let page = self.patch_page(key, entry.level) as u32;
            entry.copy_in(page).map_or(-1, |copy| copy.slot as i64)
        });
        let place = (tile.x - first.x) + PATCH_TILE_ROW * (tile.z - first.z);
        [place as f32, slot as f32, (codes & 511) as f32, (codes >> 9) as f32]
    }

    // --- Marks. ---

    /// `_cell_changed`: `tile`'s instance changes, with `sides` its four neighbours' too.
    pub(crate) fn cell_changed(&mut self, tile: Tile, sides: bool) {
        let dirty = &mut self.cells.dirty_tiles;
        dirty.insert(tile, ());
        if !sides {
            return;
        }
        for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            dirty.insert(Tile { x: tile.x + dx, z: tile.z + dz }, ());
        }
    }

    /// `_walls_changed`: the patches that may draw one of `tile`'s faces rewrite their walls.
    pub(crate) fn walls_changed(&mut self, tile: Tile) {
        let keys = self.tile_patches(tile);
        for &key in &keys {
            self.cells.dirty_walls.insert(key, ());
        }
        self.cells.key_scratch = keys;
    }

    // A set's mask revision advances (grass renders its mask again; it reads the revision).
    fn bump_mask(&mut self, set: u32) {
        self.cells.mask_revision += 1;
        self.cells.mask_bumps += 1;
        self.cells.sets[set as usize].mask_revision = self.cells.mask_revision;
    }

    /// `_mask_changed`: every resident patch drawing `tile` renders its mask again.
    pub(crate) fn mask_changed(&mut self, tile: Tile) {
        let keys = self.tile_patches(tile);
        for &key in &keys {
            if let Some(&set) = self.cells.by_patch.get(&key)
                && self.resident.contains_key(&key)
            {
                self.bump_mask(set);
            }
        }
        self.cells.key_scratch = keys;
    }

    /// `_await_cells`: the resident patches meeting `tile` start or stop drawing their set, or
    /// build one first (urgent).
    pub(crate) fn await_cells(&mut self, tile: Tile) {
        let keys = self.tile_patches(tile);
        for &key in &keys {
            if !self.resident.contains_key(&key) {
                continue;
            }
            let built = self
                .cells
                .by_patch
                .get(&key)
                .is_some_and(|&set| self.cells.sets[set as usize].is_built());
            if built {
                self.cells.dirty_shown.insert(key, ());
            } else {
                self.cells.builds_urgent.insert(key, ());
            }
        }
        self.cells.key_scratch = keys;
    }

    // `_dirty_shown` for every patch meeting `tile`.
    pub(crate) fn tile_shown_dirty(&mut self, tile: Tile) {
        let keys = self.tile_patches(tile);
        for &key in &keys {
            self.cells.dirty_shown.insert(key, ());
        }
        self.cells.key_scratch = keys;
    }

    /// `_border_changed`: patch `key`'s residency or step changed: the tiles just outside its
    /// sides on tile lines stitch across them.
    pub(crate) fn border_changed(&mut self, key: PatchKey) {
        let span = self.shape.patch_span;
        let min = self.shape.world_min;
        let origin = [min[0] + key.0 as f32 * span, min[1] + key.1 as f32 * span];
        let size = [0, 1].map(|axis| (-min[axis] - origin[axis]).min(span));
        let end = [origin[0] + size[0], origin[1] + size[1]];
        let first = [0, 1].map(|axis| (origin[axis] / TILE_M).floor() as i32 - 1);
        let last = [0, 1].map(|axis| (end[axis] / TILE_M).ceil() as i32);
        let tile_m = f64::from(TILE_M);
        for axis in 0..2 {
            let across = 1 - axis;
            for side in [f64::from(origin[axis]), f64::from(end[axis])] {
                if side.rem_euclid(tile_m) != 0.0 {
                    continue;
                }
                let column = (side / tile_m) as i32 - i32::from(side == f64::from(origin[axis]));
                for row in first[across]..=last[across] {
                    let mut tile = [0; 2];
                    tile[axis] = column;
                    tile[across] = row;
                    self.cells.dirty_tiles.insert(Tile { x: tile[0], z: tile[1] }, ());
                }
            }
        }
    }

    // --- Settling the other parts' marks. ---

    /// Applies the effects the other parts pushed since the last call, in order, as the
    /// GDScript's `_apply_effects` did: a bind rebinds its block's sets, an exhausted, refused or
    /// evicted tile and an entry change mark the cells they touch. Everything else passes on.
    pub(crate) fn settle(&mut self) {
        if self.effects.is_empty() {
            return;
        }
        let mut raw = std::mem::take(&mut self.effects);
        self.cells.erased.clear();
        for effect in raw.drain(..) {
            match effect {
                Effect::Bind { code, block, rewrite, page } => {
                    match page {
                        Some(page) => self.cells.bound[usize::from(code)].insert(block, page),
                        None => self.cells.bound[usize::from(code)].remove(&block),
                    };
                    self.bind_block(code, block, rewrite);
                }
                Effect::Exhausted { tile, .. } => {
                    self.tile_shown_dirty(tile);
                    self.out.push(effect);
                }
                Effect::Evicted { tile, walls } => {
                    // The GDScript read its mirror: an entry change of this call erased it.
                    let walls = walls && !self.cells.erased.contains(&tile);
                    self.cell_changed(tile, true);
                    if walls {
                        self.walls_changed(tile);
                    }
                    self.out.push(effect);
                }
                Effect::Installed { tile, preview } => {
                    let staged = self.cells.staged_walls.take();
                    let entries = self.entries_mut(preview);
                    match entries.get_mut(&tile) {
                        Some(entry) => {
                            if let Some((at, walls)) = &staged
                                && *at == tile
                            {
                                entry.walls.clone_from(walls);
                            }
                        }
                        None if !preview => self.cells.erased.push(tile),
                        None => {}
                    }
                    self.cells.staged_walls = staged;
                }
                Effect::MaskChanged { tile } => self.mask_changed(tile),
                Effect::CellChanged { tile, sides } => self.cell_changed(tile, sides),
                Effect::AwaitCells { tile } => self.await_cells(tile),
                other => self.out.push(other),
            }
        }
        // Nothing above pushes to the raw effects; keep the allocation.
        debug_assert!(self.effects.is_empty());
        self.effects = raw;
    }

    // `_bind_block`: block `block`'s sets at level `code` bind its page; with `rewrite` each of
    // their tiles' instances is rewritten.
    fn bind_block(&mut self, code: u8, block: (i32, i32), rewrite: bool) {
        let mut keys = std::mem::take(&mut self.cells.key_scratch);
        keys.clear();
        if code < 2 {
            keys.extend(self.cells.by_patch.keys().copied());
        } else {
            let side = self.shape.block_patches;
            for z in 0..side {
                for x in 0..side {
                    let key = (block.0 * side + x, block.1 * side + z);
                    if self.cells.by_patch.contains_key(&key) {
                        keys.push(key);
                    }
                }
            }
        }
        for &key in &keys {
            let set = *self.cells.by_patch.get(&key).expect("listed sets exist");
            if self.bind_page(set, key) && rewrite {
                let tiles = std::mem::take(&mut self.cells.sets[set as usize].tiles);
                for &tile in &tiles {
                    self.cells.dirty_tiles.insert(tile, ());
                }
                self.cells.sets[set as usize].tiles = tiles;
                self.cells.dirty_shown.insert(key, ());
            }
        }
        self.cells.key_scratch = keys;
    }

    // `_bind_page`: a set's graded materials of each level take the page its patch's block uses
    // there. Returns whether a binding changed.
    fn bind_page(&mut self, set: u32, key: PatchKey) -> bool {
        let mut changed = false;
        for code in 0..self.caps.len().min(4) as u8 {
            let page = self.patch_page(key, code);
            let bound = &mut self.cells.sets[set as usize].pages[usize::from(code)];
            if *bound == page {
                continue;
            }
            *bound = page;
            changed = true;
            self.out.push(Effect::CellPages { set, code, page });
        }
        if changed {
            self.bump_mask(set);
        }
        changed
    }

    // --- Sets. ---

    // `_acquire_cells`: a pooled set for patch `key` (a new one if the pool is empty).
    fn acquire(&mut self, key: PatchKey) -> u32 {
        let set = match self.cells.pool.pop() {
            Some(set) => set,
            None => self.new_set(),
        };
        self.out.push(Effect::CellAcquired { set, key });
        set
    }

    /// `_begin_build`: patch `key`'s set (taken from the pool if it has none) started for the
    /// patch as it stands, no tile written. Returns the set.
    pub(crate) fn begin_build(&mut self, key: PatchKey) -> u32 {
        let patch = *self.resident.get(&key).expect("a resident patch builds");
        let set = match self.cells.by_patch.get(&key) {
            Some(&set) => set,
            None => {
                let set = self.acquire(key);
                self.cells.by_patch.insert(key, set);
                set
            }
        };
        self.cells.sets[set as usize].key = key;
        self.bind_page(set, key);
        self.out.push(Effect::CellPark { set, key: Some(key) });
        self.bump_mask(set);
        let mut tiles = std::mem::take(&mut self.cells.sets[set as usize].tiles);
        tiles.clear();
        Self::rect_tiles(patch.rect, &mut tiles);
        let cell_set = &mut self.cells.sets[set as usize];
        cell_set.step = patch.step;
        cell_set.first = tiles.first().copied().unwrap_or(Tile { x: 0, z: 0 });
        cell_set.last = tiles.last().copied().unwrap_or(Tile { x: -1, z: -1 });
        cell_set.tiles = tiles;
        cell_set.built = 0;
        cell_set.complete = false;
        cell_set.counts = [0; CELL_MESHES];
        cell_set.at.fill((-1, -1));
        self.out.push(Effect::CellBegin { set, key, step: patch.step });
        if !self.cells.sets[set as usize].variant && self.patch_flagged(set) {
            self.add_variant(set);
        }
        let code = base_code(patch.step).min(6);
        let cell_set = &mut self.cells.sets[set as usize];
        match cell_set.base_mesh {
            None => {
                cell_set.base_mesh = Some(code);
                self.out.push(Effect::CellBaseMesh { set, code, reset: false });
            }
            Some(held) if held != code => {
                cell_set.base_mesh = Some(code);
                cell_set.dirty[CELL_BASE] = true;
                self.out.push(Effect::CellBaseMesh { set, code, reset: true });
            }
            Some(_) => {}
        }
        self.out.push(Effect::CellCountsCleared { set });
        if self.cells.sets[set as usize].shown {
            self.set_meshes(set, 0);
        }
        set
    }

    // `_patch_tiles`: the tiles whose interior meets a rectangle.
    fn rect_tiles(rect: [f32; 4], out: &mut Vec<Tile>) {
        let end = [rect[0] + rect[2], rect[1] + rect[3]];
        let tile_m = f64::from(TILE_M);
        let first = [0, 1].map(|axis| (f64::from(rect[axis]) / tile_m).floor() as i32);
        let last = [0, 1].map(|axis| (f64::from(end[axis]) / tile_m).ceil() as i32 - 1);
        for z in first[1]..=last[1] {
            for x in first[0]..=last[0] {
                out.push(Tile { x, z });
            }
        }
    }

    // `_set_cell_meshes`: a drawing set's meshes and instance uniforms, from mesh `first` on.
    fn set_meshes(&mut self, set: u32, first: usize) {
        if first <= CELL_BASE {
            let cell_set = &mut self.cells.sets[set as usize];
            cell_set.base_mesh = Some(base_code(cell_set.step).min(6));
        }
        self.out.push(Effect::CellMeshes { set, first: first as u8 });
    }

    /// `_build_tiles`: writes the set's next tiles, each as it stands; with a budget it stops
    /// once `over` says the frame's install budget is spent and at least `minimum` tiles were
    /// written. The last tile completes the set (its counts and walls). Returns whether it is
    /// built.
    pub(crate) fn build_tiles(&mut self, set: u32, budget: Option<(&mut dyn FnMut() -> bool, u32)>) -> bool {
        let key = self.cells.sets[set as usize].key;
        let patch = *self.resident.get(&key).expect("a resident patch builds");
        let (mut over, minimum) = match budget {
            Some((over, minimum)) => (Some(over), minimum),
            None => (None, 0),
        };
        let mut built = self.cells.sets[set as usize].built as usize;
        let mut written = 0;
        let first = self.cells.sets[set as usize].first;
        while built < self.cells.sets[set as usize].tiles.len() {
            if let Some(over) = over.as_mut()
                && written >= minimum
                && over()
            {
                self.flush(set);
                self.cells.sets[set as usize].built = built as i32;
                return false;
            }
            let tile = self.cells.sets[set as usize].tiles[built];
            let mesh = self.cell_mesh(tile, key);
            let custom = self.cell_custom(tile, first, &patch, key);
            let cell_set = &mut self.cells.sets[set as usize];
            let index = cell_set.counts[mesh] as usize;
            Self::write_cell(cell_set, mesh, index, custom);
            cell_set.custom[mesh][index] = custom;
            cell_set.tile_at[mesh][index] = tile;
            let place = cell_set.place(tile);
            cell_set.at[place] = (mesh as i8, index as i32);
            cell_set.counts[mesh] = index as u32 + 1;
            built += 1;
            written += 1;
        }
        self.cells.sets[set as usize].built = built as i32;
        self.flush(set);
        let cell_set = &self.cells.sets[set as usize];
        for mesh in 0..cell_set.meshes() {
            let count = cell_set.counts[mesh];
            self.out.push(Effect::CellCount { set, mesh: mesh as u8, count });
        }
        self.rebuild_walls(set);
        self.out.push(Effect::CellBuilt { set });
        true
    }

    // `_write_cell`: the authoritative CPU buffer takes an instance's custom data.
    fn write_cell(set: &mut CellSet, mesh: usize, index: usize, custom: Custom) {
        let offset = index * CELL_INSTANCE_FLOATS + 12;
        set.buffers[mesh][offset..offset + 4].copy_from_slice(&custom);
        set.dirty[mesh] = true;
    }

    // `_flush_cell_buffers`: one complete publication per changed mesh.
    fn flush(&mut self, set: u32) {
        let cells = &mut self.cells;
        let cell_set = &mut cells.sets[set as usize];
        for mesh in 0..cell_set.meshes() {
            if !cell_set.dirty[mesh] {
                continue;
            }
            cell_set.dirty[mesh] = false;
            let at = cells.publish_floats.len() as u32;
            cells.publish_floats.extend_from_slice(&cell_set.buffers[mesh]);
            self.out.push(Effect::CellPublish { set, mesh: mesh as u8, at });
        }
    }

    // `_rebuild_patch_walls`: the wall faces of the set's shown tiles, each drawn by the patch
    // holding its midpoint, as instances whose basis carries the face.
    fn rebuild_walls(&mut self, set: u32) {
        let key = self.cells.sets[set as usize].key;
        let rect = self.resident.get(&key).expect("a resident patch's walls").rect;
        let centre = [rect[0] + rect[2] * 0.5, rect[1] + rect[3] * 0.5];
        let mut tiles = std::mem::take(&mut self.cells.tile_scratch);
        tiles.clear();
        Self::rect_tiles(rect, &mut tiles);
        let wanted: usize = tiles
            .iter()
            .filter_map(|&tile| self.shown_in(tile, key))
            .map(|entry| entry.walls.len() / WALL_FACE_FLOATS)
            .sum();
        let mut capacity = self.cells.sets[set as usize].wall_capacity;
        if wanted > capacity as usize {
            capacity = (wanted as u32).next_power_of_two();
            self.cells.sets[set as usize].wall_capacity = capacity;
        }
        let start = self.cells.wall_floats.len() as u32;
        let mut floats = std::mem::take(&mut self.cells.wall_floats);
        let (mut count, mut truncated) = (0u32, 0u32);
        for &tile in &tiles {
            let Some(entry) = self.shown_in(tile, key) else {
                continue;
            };
            let tile_origin = [tile.x as f32 * TILE_M, tile.z as f32 * TILE_M];
            let origin = [tile_origin[0] - centre[0], tile_origin[1] - centre[1]];
            for face in entry.walls.as_chunks::<WALL_FACE_FLOATS>().0 {
                let [a_x, a_z, b_x, b_z, bottom_a, bottom_b, top_a, top_b] =
                    [0, 1, 2, 3, 4, 5, 6, 7].map(|at| f64::from(face[at]));
                // GDScript sums the floats as doubles, then makes a real_t vector of them.
                let half = [(a_x + b_x) as f32 * 0.5, (a_z + b_z) as f32 * 0.5];
                let middle = [tile_origin[0] + half[0], tile_origin[1] + half[1]];
                if middle[0] < rect[0]
                    || middle[1] < rect[1]
                    || middle[0] >= rect[0] + rect[2]
                    || middle[1] >= rect[1] + rect[3]
                {
                    continue;
                }
                if count == capacity {
                    truncated += 1;
                    continue;
                }
                let (origin_x, origin_z) = (f64::from(origin[0]), f64::from(origin[1]));
                floats.extend_from_slice(&[
                    (origin_x + a_x) as f32,
                    bottom_a as f32,
                    (origin_z + a_z) as f32,
                    (origin_x + b_x) as f32,
                    bottom_b as f32,
                    (origin_z + b_z) as f32,
                    top_a as f32,
                    top_b as f32,
                    0.0,
                ]);
                count += 1;
            }
        }
        self.cells.wall_floats = floats;
        self.cells.tile_scratch = tiles;
        self.out.push(Effect::CellWalls { set, start, count, capacity, truncated });
    }

    /// `_rebuild_patch_cells`: patch `key`'s set built whole now (a drawing set whose patch
    /// changed step; the tests' reference). A drawing set's base cells take its material again.
    pub(crate) fn rebuild_set(&mut self, key: PatchKey) {
        let set = self.begin_build(key);
        self.build_tiles(set, None);
        if self.cells.sets[set as usize].shown {
            self.out.push(Effect::CellRebuilt { set, key });
        }
        self.cells.dirty_shown.insert(key, ());
    }

    /// `_update_cell_tiles`: the queued tiles' instances rewritten in every set holding them (the
    /// sets rewritten whole this frame excepted); a touched set may start or stop drawing.
    fn update_cell_tiles(&mut self) {
        let mut touched = std::mem::take(&mut self.cells.touched);
        touched.clear();
        let mut tiles = std::mem::take(&mut self.cells.tile_scratch);
        tiles.clear();
        tiles.extend(self.cells.dirty_tiles.keys().copied());
        let mut keys = std::mem::take(&mut self.cells.meeting);
        for &tile in &tiles {
            self.mask_changed(tile);
            keys.clear();
            self.patches_meeting([tile.x as f32 * TILE_M, tile.z as f32 * TILE_M], [TILE_M, TILE_M], &mut keys);
            for &key in &keys {
                if self.cells.dirty_patches.contains_key(&key) || !self.resident.contains_key(&key) {
                    continue;
                }
                let Some(&set) = self.cells.by_patch.get(&key) else {
                    continue;
                };
                if self.update_tile_cell(set, tile) {
                    touched.insert(key, ());
                }
            }
        }
        self.cells.dirty_tiles.clear();
        for &key in touched.keys() {
            let set = *self.cells.by_patch.get(&key).expect("touched sets exist");
            self.flush(set);
            self.cells.dirty_shown.insert(key, ());
        }
        self.cells.touched = touched;
        self.cells.tile_scratch = tiles;
        self.cells.meeting = keys;
    }

    // `_update_tile_cell`: one tile's instance in a set, rewritten in place while its mesh is
    // unchanged, else moved by a swap-remove and an append. Returns whether anything changed.
    fn update_tile_cell(&mut self, set: u32, tile: Tile) -> bool {
        let cell_set = &self.cells.sets[set as usize];
        if !cell_set.holds(tile) {
            return false;
        }
        self.cells.frame[0] += 1;
        let place = cell_set.place(tile);
        let (at_mesh, at_index) = cell_set.at[place];
        if at_mesh < 0 {
            // Not written yet: its build writes it as it stands then.
            return false;
        }
        let (key, first) = (cell_set.key, cell_set.first);
        let patch = *self.resident.get(&key).expect("a resident patch's set");
        let mesh = self.cell_mesh(tile, key);
        let custom = self.cell_custom(tile, first, &patch, key);
        let (at_mesh, at_index) = (at_mesh as usize, at_index as usize);
        let cell_set = &mut self.cells.sets[set as usize];
        if at_mesh == mesh {
            if cell_set.custom[at_mesh][at_index] == custom {
                return false;
            }
            cell_set.custom[at_mesh][at_index] = custom;
            Self::write_cell(cell_set, at_mesh, at_index, custom);
            self.cells.frame[2] += 1;
            return true;
        }
        let end = cell_set.counts[at_mesh] as usize - 1;
        if at_index != end {
            let moved = cell_set.tile_at[at_mesh][end];
            cell_set.custom[at_mesh][at_index] = cell_set.custom[at_mesh][end];
            cell_set.tile_at[at_mesh][at_index] = moved;
            let moved_place = cell_set.place(moved);
            cell_set.at[moved_place] = (at_mesh as i8, at_index as i32);
            let data = cell_set.custom[at_mesh][at_index];
            Self::write_cell(cell_set, at_mesh, at_index, data);
        }
        cell_set.counts[at_mesh] = end as u32;
        self.out.push(Effect::CellCount { set, mesh: at_mesh as u8, count: end as u32 });
        let cell_set = &mut self.cells.sets[set as usize];
        let index = cell_set.counts[mesh] as usize;
        cell_set.custom[mesh][index] = custom;
        cell_set.tile_at[mesh][index] = tile;
        cell_set.at[place] = (mesh as i8, index as i32);
        Self::write_cell(cell_set, mesh, index, custom);
        cell_set.counts[mesh] = index as u32 + 1;
        self.out.push(Effect::CellCount { set, mesh: mesh as u8, count: index as u32 + 1 });
        self.cells.frame[2] += 1;
        self.cells.frame[3] += 1;
        true
    }

    /// `_apply_cell_changes`: a patch-wide change rewrites the patch's set; every other change
    /// its own tiles' instances in the sets drawing them; then the sets that may start or stop
    /// drawing, and the cells across their sides.
    pub(crate) fn apply_cell_changes(&mut self) {
        self.settle();
        let mut keys = std::mem::take(&mut self.cells.patch_scratch);
        keys.clear();
        keys.extend(self.cells.dirty_patches.keys().copied());
        for &key in &keys {
            if self.resident.contains_key(&key) {
                self.rebuild_set(key);
            }
            self.cells.dirty_walls.remove(&key);
        }
        if self.cells.dirty_tiles.len() > 0 {
            self.update_cell_tiles();
        }
        self.cells.dirty_patches.clear();
        keys.clear();
        keys.extend(self.cells.dirty_shown.keys().copied());
        for &key in &keys {
            self.apply_shown(key);
        }
        self.cells.dirty_shown.clear();
        if self.cells.dirty_tiles.len() > 0 {
            self.update_cell_tiles();
            self.cells.dirty_shown.clear();
        }
        self.cells.patch_scratch = keys;
    }

    /// `_apply_wall_changes`.
    pub(crate) fn apply_wall_changes(&mut self) {
        self.settle();
        let mut keys = std::mem::take(&mut self.cells.patch_scratch);
        keys.clear();
        keys.extend(self.cells.dirty_walls.keys().copied());
        for &key in &keys {
            if !self.resident.contains_key(&key) {
                continue;
            }
            if let Some(&set) = self.cells.by_patch.get(&key)
                && self.cells.sets[set as usize].is_built()
            {
                self.rebuild_walls(set);
            }
        }
        self.cells.dirty_walls.clear();
        self.cells.patch_scratch = keys;
    }

    // --- The behind-the-road variant. ---

    /// `_patch_flagged`: whether a tile shown or installed in the set's patch is flagged behind
    /// the road.
    fn patch_flagged(&self, set: u32) -> bool {
        self.cells.sets[set as usize].tiles.iter().any(|tile| {
            self.preview.get(tile).is_some_and(|entry| entry.behind)
                || self.installed.get(tile).is_some_and(|entry| entry.behind)
        })
    }

    /// `_variant_missing`: whether a set of the patches meeting `tile` lacks the variant; each
    /// such set is queued for it.
    pub(crate) fn variant_missing(&mut self, tile: Tile) -> bool {
        let keys = self.tile_patches(tile);
        let mut missing = false;
        for &key in &keys {
            let Some(&set) = self.cells.by_patch.get(&key) else {
                continue;
            };
            let cell_set = &self.cells.sets[set as usize];
            if cell_set.variant || !cell_set.holds(tile) {
                continue;
            }
            self.cells.variants_wanted.insert(key, ());
            missing = true;
        }
        self.cells.key_scratch = keys;
        missing
    }

    /// `_add_variant`: the set takes the behind-the-road variant's four meshes (the build path).
    pub(crate) fn add_variant(&mut self, set: u32) {
        let cells = &mut self.cells;
        let cell_set = &mut cells.sets[set as usize];
        Self::add_records(cell_set, &cells.identity, cells.per_patch, CELL_MESHES - CELL_PLAIN);
        cell_set.variant = true;
        let (pages, shown) = (cell_set.pages, cell_set.shown);
        self.out.push(Effect::CellVariant { set, pages, shown });
        if shown {
            self.set_meshes(set, CELL_BEHIND);
        }
        self.bump_mask(set);
    }

    /// `_add_wanted_variants`' step: the first queued set takes the variant if it lacks it.
    /// Returns whether one was added; `None` when none is queued.
    pub(crate) fn add_next_variant(&mut self) -> Option<bool> {
        let key = *self.cells.variants_wanted.keys().next()?;
        self.cells.variants_wanted.remove(&key);
        let Some(&set) = self.cells.by_patch.get(&key) else {
            return Some(false);
        };
        if self.cells.sets[set as usize].variant {
            return Some(false);
        }
        self.add_variant(set);
        Some(true)
    }

    // --- Residency. ---

    /// `_patch_changed`'s cell parts around the book's: the cells across its sides follow its
    /// step; a resident patch's set follows it (`node_moved`: a drawing set's patch node changed).
    pub(crate) fn patch_changed_cells(&mut self, key: PatchKey, rect: Option<[f32; 4]>, node_moved: bool) {
        self.settle();
        let step = self.resident.get(&key).map_or(-1, |patch| patch.step);
        if step != self.cells.patch_steps.get(&key).copied().unwrap_or(-1) {
            self.border_changed(key);
            if step < 0 {
                self.cells.patch_steps.remove(&key);
            } else {
                self.cells.patch_steps.insert(key, step);
            }
        }
        let present = rect.is_some();
        self.patch_changed(key, rect);
        self.settle();
        if !present {
            return;
        }
        self.cells.dirty_shown.insert(key, ());
        let Some(&set) = self.cells.by_patch.get(&key) else {
            self.cells.builds.insert(key, ());
            return;
        };
        if node_moved {
            self.hide(set);
            self.cells.dirty_shown.insert(key, ());
        }
        self.out.push(Effect::CellPark { set, key: Some(key) });
        let cell_set = &mut self.cells.sets[set as usize];
        if cell_set.step != step {
            if cell_set.shown {
                self.cells.dirty_patches.insert(key, ());
            } else {
                cell_set.built = -1;
                self.cells.builds.insert(key, ());
            }
        }
    }

    // `_hide_cells`.
    pub(crate) fn hide(&mut self, set: u32) {
        self.cells.sets[set as usize].shown = false;
        self.out.push(Effect::CellHide { set });
    }

    /// `release_patch`'s part: patch `key`'s set returns to the pool. Returns it, if it had one.
    pub(crate) fn release_set(&mut self, key: PatchKey) -> Option<u32> {
        self.settle();
        let cells = &mut self.cells;
        let set = cells.by_patch.remove(&key);
        cells.builds.remove(&key);
        cells.builds_urgent.remove(&key);
        cells.dirty_shown.remove(&key);
        let set = set?;
        let cell_set = &mut cells.sets[set as usize];
        cell_set.shown = false;
        cell_set.built = -1;
        cells.pool.push(set);
        Some(set)
    }

    /// `clear`'s cell part (after every set was released).
    pub(crate) fn clear_cells(&mut self) {
        let cells = &mut self.cells;
        cells.dirty_walls.clear();
        cells.dirty_tiles.clear();
        cells.dirty_patches.clear();
        cells.patch_steps.clear();
        cells.builds.clear();
        cells.builds_urgent.clear();
        cells.variants_wanted.clear();
        cells.dirty_shown.clear();
        cells.staged_walls = None;
    }

    /// `_take_list`'s cell part: a patch without a set builds one; what it draws follows its list.
    pub(crate) fn list_arrived(&mut self, key: PatchKey) {
        self.settle();
        if !self.cells.by_patch.contains_key(&key) {
            self.cells.builds.insert(key, ());
        }
        self.cells.dirty_shown.insert(key, ());
    }

    /// Whether queued cell work will change what is drawn (`has_pending_work`'s part): urgent
    /// builds, variants wanted, a queued build whose patch lists a tile.
    pub(crate) fn cells_pending(&self) -> bool {
        let cells = &self.cells;
        cells.builds_urgent.len() > 0
            || cells.variants_wanted.len() > 0
            || cells
                .builds
                .keys()
                .any(|key| self.lists.get(key).is_some_and(|(_, tiles)| !tiles.is_empty()))
    }

    /// The next queued build: urgent ones first. `None` when none is queued.
    pub(crate) fn next_build(&self) -> Option<(PatchKey, bool)> {
        if let Some(&key) = self.cells.builds_urgent.keys().next() {
            return Some((key, true));
        }
        self.cells.builds.keys().next().map(|&key| (key, false))
    }

    /// A build is done (or its patch left): it leaves both queues; a built set may start drawing.
    pub(crate) fn build_finished(&mut self, key: PatchKey, built: bool) {
        self.cells.builds.remove(&key);
        self.cells.builds_urgent.remove(&key);
        if built {
            self.cells.dirty_shown.insert(key, ());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::state::Shape;
    use super::*;

    #[test]
    fn base_code_follows_the_step() {
        assert_eq!([0, 1, 2, 4, 8].map(base_code), [3, 3, 4, 5, 6]);
    }

    // One resident 510 m patch at the world's lower corner (64 tiles), the GDScript renderer's
    // level shapes, its prewarmed sets.
    fn world() -> Graded {
        let mut graded = Graded::default();
        let shape = Shape {
            world_min: [-510.0, -510.0],
            patch_span: 510.0,
            patch_grid: [2, 2],
            block_patches: 2,
            layer_limit: 2048,
        };
        graded.setup(shape, &[(131, 64, 3000), (67, 192, 2000), (35, 96, 1000), (35, 256, 1000)]);
        graded.cells_setup(81, 4);
        let patch = Patch { rect: [-510.0, -510.0, 510.0, 510.0], step: 1, generation: 0 };
        graded.set_patch((0, 0), Some(patch));
        graded
    }

    fn install(graded: &mut Graded, x: i32, z: i32, code: u8) {
        let mut scratch = Vec::new();
        assert!(graded.upload(Tile { x, z }, (code, 0, 0, false), 1, None, &mut scratch));
        graded.settle();
    }

    // The set's instances per mesh, sorted: what the GPU draws whatever their order.
    fn record(graded: &Graded, set: u32) -> Vec<Vec<[u32; 4]>> {
        let set = &graded.cells.sets[set as usize];
        (0..CELL_PLAIN)
            .map(|mesh| {
                let mut rows: Vec<[u32; 4]> = set.custom[mesh][..set.counts[mesh] as usize]
                    .iter()
                    .map(|custom| custom.map(f32::to_bits))
                    .collect();
                rows.sort_unstable();
                rows
            })
            .collect()
    }

    #[test]
    fn per_tile_updates_equal_a_whole_rewrite() {
        let mut graded = world();
        graded.take_list((0, 0), 0, vec![Tile { x: -1, z: -1 }, Tile { x: -2, z: -1 }]);
        install(&mut graded, -1, -1, 0);
        let set = graded.begin_build((0, 0));
        assert!(graded.build_tiles(set, None));
        let cells = &graded.cells.sets[set as usize];
        assert_eq!((cells.counts[0], cells.counts[CELL_BASE]), (1, 63));
        // A listed tile not installed yet: the patch draws nothing; once it shows, its set.
        graded.apply_cell_changes();
        assert_eq!(graded.draw_state((0, 0)), DRAW_NONE);
        // A second tile and a level change arrive as per-tile updates (with the neighbours' sides).
        install(&mut graded, -2, -1, 1);
        install(&mut graded, -1, -1, 1);
        graded.apply_cell_changes();
        assert!(graded.cells.sets[set as usize].shown, "the set draws once its listed tiles show");
        assert_eq!(graded.draw_state((0, 0)), DRAW_SET);
        let updated = record(&graded, set);
        assert_eq!(updated[1].len(), 2);
        graded.rebuild_set((0, 0));
        assert_eq!(record(&graded, set), updated, "the per-tile updates equal the patch's rewrite");
        assert_eq!(graded.draw_violations(), (0, 0));
        // Every instance's custom data is its place, slot and codes; a GL1 side meeting the base
        // cells (code 3 at step 1) stitches to the coarser code, a side on the border of a patch
        // not resident to its own.
        let cells = &graded.cells.sets[set as usize];
        let (mesh, index) = cells.at[cells.place(Tile { x: -1, z: -1 })];
        let custom = cells.custom[mesh as usize][index as usize];
        assert_eq!((mesh, custom[0]), (1, (7 + 32 * 7) as f32));
        assert!(custom[1] >= 0.0);
        assert_eq!(custom[2] as u32 & 7, 1);
        assert_eq!(custom[3] as u32 & 7, 3, "the -z side meets the base cells");
        assert_eq!((custom[2] as u32 >> 6) & 7, 1, "the +x side is the border of a patch not resident");
    }
}
