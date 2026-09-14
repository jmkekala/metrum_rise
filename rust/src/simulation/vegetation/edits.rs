// SPDX-License-Identifier: GPL-2.0-only

//! Sparse player edits over the generator's cell identities, with independent patch revisions.

use std::collections::{HashMap, HashSet};

/// Grid owning a cell; coordinates are meaningful only together with their layer's spacing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum VegetationLayer {
    /// Long-range tree grid, also used by the player brush.
    Canopy,
    /// Short-range decorative grid.
    Understory,
}

/// Identity of the single generated candidate in one layer's cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VegetationCell {
    /// Grid whose spacing scales these coordinates.
    pub layer: VegetationLayer,
    /// Signed world-grid column.
    pub x: i32,
    /// Signed world-grid row.
    pub z: i32,
}

/// One authoritative player placement, in world metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AuthoredPlant {
    /// World east-west position in metres.
    pub x: f32,
    /// World north-south position in metres.
    pub z: f32,
    /// Rotation around the vertical axis in radians.
    pub yaw: f32,
    /// Uniform instance scale.
    pub scale: f32,
    /// Renderer ordinal: conifer, broadleaf, bush or rock (0 through 3).
    pub species: u8,
}

// One block covers 16x16 cells of a single layer, so a patch query tests a handful of blocks
// instead of one lookup per cell. Every insertion maintains it, a save load included, so the
// index cannot go stale. It is deliberately never pruned: it is a conservative superset, and a
// stale block only costs the per-cell lookups that were unconditional before.
const CELL_BLOCK_SHIFT: i32 = 4;

fn cell_block(cell: VegetationCell) -> (VegetationLayer, i32, i32) {
    (
        cell.layer,
        cell.x >> CELL_BLOCK_SHIFT,
        cell.z >> CELL_BLOCK_SHIFT,
    )
}

#[derive(Clone, Debug, Default, PartialEq)]
struct CellEdit {
    generated_removed: bool,
    added: Vec<AuthoredPlant>,
}

/// Player edits keyed by generator cell, using O(edits) storage rather than O(world).
///
/// A patch visits O(K) bounded candidates with one expected O(1) lookup per visited cell,
/// plus O(A) output work for its authored plants. No second spatial index is needed.
#[derive(Clone, Debug, Default)]
pub struct VegetationEdits {
    cells: HashMap<VegetationCell, CellEdit>,
    patch_generations: HashMap<i64, u64>,
    blocks: HashSet<(VegetationLayer, i32, i32)>,
}

impl VegetationEdits {
    /// Returns the revision of one packed render-patch key, or zero when untouched.
    pub fn patch_generation(&self, key: i64) -> u64 {
        self.patch_generations.get(&key).copied().unwrap_or(0)
    }

    /// Whether any edited cell of one layer falls in an inclusive cell range, in O(blocks).
    ///
    /// A false answer lets a query skip the per-cell lookups entirely, which is the whole
    /// per-cell cost of the untouched patches that make up nearly every query.
    pub(crate) fn any_in_cell_range(
        &self,
        layer: VegetationLayer,
        x: (i32, i32),
        z: (i32, i32),
    ) -> bool {
        if self.cells.is_empty() {
            return false;
        }
        ((z.0 >> CELL_BLOCK_SHIFT)..=(z.1 >> CELL_BLOCK_SHIFT)).any(|bz| {
            ((x.0 >> CELL_BLOCK_SHIFT)..=(x.1 >> CELL_BLOCK_SHIFT))
                .any(|bx| self.blocks.contains(&(layer, bx, bz)))
        })
    }

    /// Borrows a cell delta with one expected O(1) lookup, without allocating.
    pub(crate) fn cell(&self, cell: VegetationCell) -> (bool, &[AuthoredPlant]) {
        self.cells.get(&cell).map_or((false, &[]), |edit| {
            (edit.generated_removed, edit.added.as_slice())
        })
    }

    /// Reserves batch capacity before committing edits and touched patch revisions.
    pub(crate) fn reserve(&mut self, cells: usize, patches: usize) {
        self.cells.reserve(cells);
        self.patch_generations.reserve(patches);
    }

    /// Advances one vegetation revision without changing any terrain payload.
    pub(crate) fn bump_patch(&mut self, key: i64) {
        let generation = self.patch_generations.entry(key).or_default();
        *generation = generation.wrapping_add(1);
    }

    /// Sets a generated-cell tombstone, pruning a restored cell with no additions.
    pub(crate) fn set_removed(&mut self, cell: VegetationCell, removed: bool) {
        if removed {
            self.cells.entry(cell).or_default().generated_removed = true;
            self.blocks.insert(cell_block(cell));
        } else if let Some(edit) = self.cells.get_mut(&cell) {
            edit.generated_removed = false;
            if edit.added.is_empty() {
                self.cells.remove(&cell);
            }
        }
    }

    // Persistent allocation happens only when committing an actual authored edit, never
    // while evaluating generator cells. The mandatory per-cell Vec retains insertion order.
    /// Commits one validated authored placement in its owning cell, preserving order.
    pub(crate) fn add(&mut self, cell: VegetationCell, plant: AuthoredPlant) {
        self.cells.entry(cell).or_default().added.push(plant);
        self.blocks.insert(cell_block(cell));
    }

    /// Removes matching authored plants and advances the patch keys returned by the predicate.
    pub(crate) fn remove_added(
        &mut self,
        cell: VegetationCell,
        mut remove: impl FnMut(&AuthoredPlant) -> Option<i64>,
    ) -> usize {
        let Some(edit) = self.cells.get_mut(&cell) else {
            return 0;
        };
        let before = edit.added.len();
        edit.added.retain(|plant| {
            if let Some(key) = remove(plant) {
                let generation = self.patch_generations.entry(key).or_default();
                *generation = generation.wrapping_add(1);
                false
            } else {
                true
            }
        });
        let removed = before - edit.added.len();
        if !edit.generated_removed && edit.added.is_empty() {
            self.cells.remove(&cell);
        }
        removed
    }

    // Sorting is confined to save time; query and edit paths never scan the whole store.
    /// Returns save-order keys in O(E log E); never used by local queries or editing.
    pub(crate) fn sorted_cells(&self) -> Vec<VegetationCell> {
        let mut cells: Vec<_> = self.cells.keys().copied().collect();
        cells.sort_unstable_by_key(|cell| (cell.layer, cell.z, cell.x));
        cells
    }

    #[cfg(test)]
    /// Returns the number of edited cells for sparse-store regressions.
    pub(crate) fn len(&self) -> usize {
        self.cells.len()
    }
}

/// Packs signed render-patch coordinates without losing either coordinate's bits.
pub(crate) fn pack_patch_key(x: i32, z: i32) -> i64 {
    (i64::from(x) << 32) | i64::from(z as u32)
}
