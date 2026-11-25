// SPDX-License-Identifier: GPL-3.0-or-later

//! The backdrop's cells (`TERRAIN-41` round D2, `docs/terrain.md` "The backdrop drawn"): every
//! patch of the map that the terrain does not draw as a patch draws one backdrop cell, and a patch
//! the terrain draws draws none. So in every frame each patch of the map draws exactly one of the
//! two: a patch inside the wanted square until its unit makes it resident, and every patch outside
//! it (never drawn as a patch there), so a view that reaches past the square to the far plane
//! shows ground, not sky.
//!
//! **The hand-over rule, on the renderer's side** (`TERRAIN-41` D3): a patch is drawn as a patch
//! when it is resident *and* its node draws something. A resident patch can draw nothing: the
//! graded tile book collapses its PlaneMesh while its list of graded tiles is not known or its
//! graded set is not ready (T53 (a), "nothing until then"). So a cell goes only when both hold and
//! comes back when either stops: [`Cells::set_resident`] takes the terrain layer's residency
//! changes (`follow_terrain`, in the pass in which the unit ran) and [`Cells::set_blank`] the
//! book's draw-nothing changes (taken in both passes), and [`Cells::flush`] sends the changed
//! blocks before the frame draws (both of the stream node's passes end with it). The book marks
//! a patch that turns resident without a known list as drawing nothing in the same pass, so its
//! first frame draws its cell, not a PlaneMesh it would collapse a frame later. One `MultiMesh` and one instance a block of
//! [`BLOCK_PATCHES`] x [`BLOCK_PATCHES`] patches, holding that block's cells in a dense list (a
//! swap-remove keeps it dense), drawn with the block's material; the engine culls a block by its
//! bounds like any instance.
//!
//! Bounds: at most one instance a patch of the map (1,296 on the shipped 18,432 m map, 13,689 at
//! 59,904 m) in at most one `MultiMesh` a block (9 and 64). A residency change is `O(1)`; a frame
//! sends `O(blocks changed x BLOCK_PATCHES^2)` rows (each changed block's list again, at most 256
//! rows of 16 floats); a frame without a change sends nothing.

use super::BLOCK_PATCHES;
use crate::nodes::sim::render::stream::backend::{Handle, RenderBackend, Scenario, Shadow};
use crate::nodes::sim::render::stream::layers::terrain::{PatchGrid, PatchKey};
use crate::nodes::sim::render::vegetation_product::godot_math::Aabb;

/// Floats a cell's row: the instance's 3 x 4 transform (a translation to the patch's corner), then
/// its custom lane: the patch's column and row in its block and its size, metres (the last patch
/// of a map that is not a whole number of patches is shorter).
pub(crate) const STRIDE: usize = 16;
/// Cells a block holds at most.
pub(crate) const CAPACITY: usize = BLOCK_PATCHES * BLOCK_PATCHES;
// A patch drawn as a patch: no cell.
const NOT_DRAWN: u16 = u16::MAX;
// A patch's state bits: resident (the terrain layer's), and blank (resident but drawing nothing,
// the graded book's). A cell draws unless the patch is resident and not blank.
const RESIDENT: u8 = 1;
const BLANK: u8 = 2;

/// The cells of the map's patches not drawn as patches, by block.
#[derive(Default)]
pub(crate) struct Cells {
    grid: Option<PatchGrid>,
    blocks_x: usize,
    // Per patch of the map (row-major): its cell's slot in its block's list, or `NOT_DRAWN`.
    slots: Vec<u16>,
    // Per patch of the map: `RESIDENT` and `BLANK`.
    states: Vec<u8>,
    // Per block: the patches drawing a cell, as `local z x BLOCK_PATCHES + local x`, dense.
    lists: Vec<Vec<u16>>,
    // Per block: its `MultiMesh` and instance, made at its first draw; and whether it draws.
    objects: Vec<Option<[Handle; 2]>>,
    shown: Vec<bool>,
    // Blocks whose list or readiness changed since the last flush.
    dirty: Vec<bool>,
    dirty_blocks: Vec<u32>,
    // The rows of the block being sent (reused).
    rows: Vec<f32>,
    drawn: usize,
}

impl Cells {
    /// Starts over on a map's patch grid (none: no map): nothing is resident yet, so every patch
    /// of the map wants its cell. Frees nothing ([`Self::free_objects`] first). `O(patches)`,
    /// once a world.
    pub(crate) fn reset(&mut self, grid: Option<PatchGrid>) {
        let rows = std::mem::take(&mut self.rows);
        *self = Self {
            rows,
            ..Self::default()
        };
        let Some(grid) = grid.filter(|grid| grid.cols > 0 && grid.rows > 0) else {
            return;
        };
        let [cols, map_rows] = [grid.cols as usize, grid.rows as usize];
        let blocks_x = cols.div_ceil(BLOCK_PATCHES);
        let blocks = blocks_x * map_rows.div_ceil(BLOCK_PATCHES);
        self.grid = Some(grid);
        self.blocks_x = blocks_x;
        self.slots = vec![NOT_DRAWN; cols * map_rows];
        self.states = vec![0; cols * map_rows];
        self.lists = (0..blocks).map(|_| Vec::with_capacity(CAPACITY)).collect();
        self.objects = vec![None; blocks];
        self.shown = vec![false; blocks];
        self.dirty = vec![false; blocks];
        self.dirty_blocks = Vec::with_capacity(blocks);
        for index in 0..cols * map_rows {
            self.set_drawn(index, true);
        }
    }

    /// Frees every block's objects (they are made again at the next flush that draws).
    pub(crate) fn free_objects(&mut self, backend: &mut impl RenderBackend) {
        for (block, objects) in self.objects.iter_mut().enumerate() {
            if let Some([multimesh, instance]) = objects.take() {
                backend.free(instance);
                backend.free(multimesh);
            }
            self.shown[block] = false;
        }
        self.touch_all();
    }

    /// Patch `key` became resident or left: its cell goes when it is resident and not blank,
    /// and comes otherwise. `O(1)`.
    pub(crate) fn set_resident(&mut self, key: PatchKey, resident: bool) {
        self.set_state(key, RESIDENT, resident);
    }

    /// Resident patch `key` draws nothing (`blank`: its cell comes) or draws again (its cell goes
    /// while it is resident). A patch that is not resident keeps the bit and draws its cell.
    /// `O(1)`.
    pub(crate) fn set_blank(&mut self, key: PatchKey, blank: bool) {
        self.set_state(key, BLANK, blank);
    }

    /// Whether patch `key` is drawn as a patch: resident and not blank. `O(1)`.
    pub(crate) fn patch_draws(&self, key: PatchKey) -> bool {
        self.index(key)
            .is_some_and(|index| self.states[index] & (RESIDENT | BLANK) == RESIDENT)
    }

    fn set_state(&mut self, key: PatchKey, bit: u8, on: bool) {
        let Some(index) = self.index(key) else {
            return;
        };
        if on {
            self.states[index] |= bit;
        } else {
            self.states[index] &= !bit;
        }
        let drawn_as_patch = self.states[index] & (RESIDENT | BLANK) == RESIDENT;
        self.set_drawn(index, !drawn_as_patch);
    }

    /// Marks block `block` for the next flush (its readiness may have changed). `O(1)`.
    pub(crate) fn touch(&mut self, block: usize) {
        if block < self.dirty.len() && !self.dirty[block] {
            self.dirty[block] = true;
            self.dirty_blocks.push(block as u32);
        }
    }

    /// Marks every block for the next flush. `O(blocks)`.
    pub(crate) fn touch_all(&mut self) {
        for block in 0..self.dirty.len() {
            self.touch(block);
        }
    }

    /// The map's patch grid.
    pub(crate) fn grid(&self) -> Option<PatchGrid> {
        self.grid
    }

    /// Cells listed (patches not drawn as patches), drawn or not.
    pub(crate) fn listed(&self) -> usize {
        self.drawn
    }

    /// Cells drawn: listed in a block that draws. `O(blocks)`.
    pub(crate) fn shown_count(&self) -> usize {
        self.lists
            .iter()
            .zip(&self.shown)
            .filter(|(_, shown)| **shown)
            .map(|(list, _)| list.len())
            .sum()
    }

    /// Blocks that draw. `O(blocks)`.
    pub(crate) fn shown_blocks(&self) -> usize {
        self.shown.iter().filter(|shown| **shown).count()
    }

    /// Whether patch `key` draws a cell this frame (as last flushed). `O(1)`.
    pub(crate) fn is_shown(&self, key: PatchKey) -> bool {
        self.index(key).is_some_and(|index| {
            self.slots[index] != NOT_DRAWN && self.shown[self.place(index).0]
        })
    }

    /// Sends the blocks changed since the last flush: a block draws when `material` gives it a
    /// material (its textures exist) and it lists a cell; its rows, count and bounds (`heights`,
    /// the ground's lowest and highest metres) are sent again when it draws. `mesh` is the cell's
    /// mesh (a catalogue index). `O(changed blocks x CAPACITY)`; nothing when nothing changed.
    pub(crate) fn flush(
        &mut self,
        backend: &mut impl RenderBackend,
        mesh: Option<u32>,
        material: impl Fn(usize) -> Option<Handle>,
        heights: [f32; 2],
    ) {
        for at in 0..self.dirty_blocks.len() {
            let block = self.dirty_blocks[at] as usize;
            self.dirty[block] = false;
            let material = mesh.and_then(|_| material(block));
            let count = self.lists[block].len();
            let mut show = material.is_some() && count > 0;
            if show {
                let objects = match self.objects[block] {
                    Some(objects) => Some(objects),
                    None => mesh.and_then(|mesh| {
                        let multimesh = backend.multimesh_create(CAPACITY as u32, false, true);
                        backend.multimesh_set_mesh(multimesh, mesh);
                        let instance = backend.instance_create(multimesh, Scenario::Main)?;
                        // Ground receives shadows and casts none, as the terrain patches.
                        backend.instance_set_shadow(instance, Shadow::Off);
                        backend.instance_set_visible(instance, false);
                        Some([multimesh, instance])
                    }),
                };
                self.objects[block] = objects;
                match objects {
                    Some([multimesh, instance]) => {
                        let aabb = self.write_rows(block, heights);
                        backend.instance_set_material_handle(instance, material);
                        backend.multimesh_set_buffer(multimesh, &self.rows, STRIDE, CAPACITY as u32);
                        backend.multimesh_set_visible_instances(multimesh, count as u32);
                        backend.multimesh_set_custom_aabb(multimesh, aabb);
                    }
                    None => show = false,
                }
            }
            if show != self.shown[block] {
                if let Some([_, instance]) = self.objects[block] {
                    backend.instance_set_visible(instance, show);
                }
                self.shown[block] = show;
            }
        }
        self.dirty_blocks.clear();
    }

    // The patch index of `key`, inside the map.
    fn index(&self, key: PatchKey) -> Option<usize> {
        let grid = self.grid?;
        (key[0] >= 0 && key[1] >= 0 && key[0] < grid.cols && key[1] < grid.rows)
            .then(|| (key[1] * grid.cols + key[0]) as usize)
    }

    // A patch's block and its place in it.
    fn place(&self, index: usize) -> (usize, u16) {
        let cols = self.grid.map_or(1, |grid| grid.cols as usize);
        let [x, z] = [index % cols, index / cols];
        let block = (z / BLOCK_PATCHES) * self.blocks_x + x / BLOCK_PATCHES;
        let local = (z % BLOCK_PATCHES) * BLOCK_PATCHES + x % BLOCK_PATCHES;
        (block, local as u16)
    }

    // The patch key of block `block`'s place `local`.
    fn key(&self, block: usize, local: u16) -> PatchKey {
        let local = local as usize;
        [
            ((block % self.blocks_x) * BLOCK_PATCHES + local % BLOCK_PATCHES) as i32,
            ((block / self.blocks_x) * BLOCK_PATCHES + local / BLOCK_PATCHES) as i32,
        ]
    }

    fn set_drawn(&mut self, index: usize, drawn: bool) {
        let slot = self.slots[index];
        if drawn == (slot != NOT_DRAWN) {
            return;
        }
        let (block, local) = self.place(index);
        let list = &mut self.lists[block];
        if drawn {
            self.slots[index] = list.len() as u16;
            list.push(local);
            self.drawn += 1;
        } else {
            list.swap_remove(slot as usize);
            if let Some(&moved) = list.get(slot as usize) {
                let key = self.key(block, moved);
                let moved = self.index(key).expect("a listed cell lies in the map");
                self.slots[moved] = slot;
            }
            self.slots[index] = NOT_DRAWN;
            self.drawn -= 1;
        }
        self.touch(block);
    }

    // Writes block `block`'s rows into the scratch rows and returns their bounds. `O(cells)`.
    fn write_rows(&mut self, block: usize, heights: [f32; 2]) -> Aabb {
        let grid = self.grid.expect("a block draws on a grid");
        self.rows.clear();
        let mut low = [f32::INFINITY; 2];
        let mut high = [f32::NEG_INFINITY; 2];
        for &local in &self.lists[block] {
            let key = [
                ((block % self.blocks_x) * BLOCK_PATCHES + local as usize % BLOCK_PATCHES) as i32,
                ((block / self.blocks_x) * BLOCK_PATCHES + local as usize / BLOCK_PATCHES) as i32,
            ];
            let [x, z, width, depth] = grid.rect(key);
            let place = [(local as usize % BLOCK_PATCHES) as f32, (local as usize / BLOCK_PATCHES) as f32];
            self.rows.extend([
                1.0, 0.0, 0.0, x, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, z, place[0], place[1], width, depth,
            ]);
            low = [low[0].min(x), low[1].min(z)];
            high = [high[0].max(x + width), high[1].max(z + depth)];
        }
        Aabb {
            position: [low[0], heights[0], low[1]],
            size: [high[0] - low[0], heights[1] - heights[0], high[1] - low[1]],
        }
    }
}

#[cfg(test)]
mod tests;
