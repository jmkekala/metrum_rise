// SPDX-License-Identifier: GPL-2.0-only

//! Generation completeness and paint revisions on the existing cell spatial chunks.

use super::{CellBounds, CellKey, CellStore, DVec2, RegionGraph, chunk_at, chunks_for_bounds};

impl CellStore {
    /// Returns the packed overlay version and whether all empty geometry is current.
    pub(crate) fn chunk_state(&self, chunk: (i32, i32)) -> (u64, bool) {
        self.chunks
            .get(&chunk)
            .map_or((0, false), |entry| (entry.revision, entry.generated))
    }

    /// Returns a bounded batch of nearby dirty envelopes, or the full chunk for a cold cache.
    /// Warm batches never exceed one chunk's width/height. Ready lookup uses one hash query;
    /// dirty lookup uses at most nine more, with no allocation or city-wide traversal.
    pub(crate) fn chunk_generation_bounds(&self, chunk: (i32, i32)) -> Option<CellBounds> {
        let Some(entry) = self.chunks.get(&chunk) else {
            return Some(Self::chunk_bounds(chunk));
        };
        if entry.generated {
            return None;
        }
        let Some(local) = entry.dirty_bounds else {
            return Some(Self::chunk_bounds(chunk));
        };
        let mut joined = local;
        for x in chunk.0.saturating_sub(1)..=chunk.0.saturating_add(1) {
            for y in chunk.1.saturating_sub(1)..=chunk.1.saturating_add(1) {
                if let Some(dirty) = self
                    .chunks
                    .get(&(x, y))
                    .and_then(|entry| entry.dirty_bounds)
                {
                    joined.min = joined.min.min(dirty.min);
                    joined.max = joined.max.max(dirty.max);
                }
            }
        }
        Some(
            if (joined.max - joined.min).max_element() <= f64::from(RegionGraph::CHUNK_SIZE) {
                joined
            } else {
                local
            },
        )
    }

    /// Completes the requested chunk and warm neighbours whose entire dirty envelope was
    /// covered by the caller's generation bounds. Partly covered and cold neighbours stay dirty.
    pub(crate) fn complete_generated_chunk(&mut self, chunk: (i32, i32), bounds: CellBounds) {
        let entry = self.chunks.entry(chunk).or_default();
        entry.generated = true;
        entry.dirty_bounds = None;
        for neighbour in chunks_for_bounds(bounds) {
            if let Some(entry) = self.chunks.get_mut(&neighbour)
                && let Some(dirty) = entry.dirty_bounds
                && dirty.min.cmpge(bounds.min).all()
                && dirty.max.cmple(bounds.max).all()
            {
                entry.generated = true;
                entry.dirty_bounds = None;
            }
        }
    }

    /// Invalidates cached chunks within an edit's geometric influence, including paint that
    /// changes retained strip alignment. Warm chunks retain only their affected envelope;
    /// overlapping edits union in O(touched chunks) with constant storage per chunk.
    pub(crate) fn invalidate_generated_cells(&mut self, bounds: CellBounds) {
        if bounds.is_valid() {
            for chunk in chunks_for_bounds(bounds) {
                if let Some(entry) = self.chunks.get_mut(&chunk) {
                    let chunk_bounds = Self::chunk_bounds(chunk);
                    let dirty = CellBounds {
                        min: bounds.min.max(chunk_bounds.min),
                        max: bounds.max.min(chunk_bounds.max),
                    };
                    if entry.generated {
                        entry.dirty_bounds = Some(dirty);
                    } else if let Some(previous) = &mut entry.dirty_bounds {
                        previous.min = previous.min.min(dirty.min);
                        previous.max = previous.max.max(dirty.max);
                    }
                    entry.generated = false;
                }
            }
        }
    }

    /// Full-world authoring/load operations may invalidate every viewed chunk explicitly.
    pub(crate) fn invalidate_all_generated_cells(&mut self) {
        for entry in self.chunks.values_mut() {
            entry.generated = false;
            entry.dirty_bounds = None;
        }
    }

    /// Constant-time check before enumerating local road geometry for cache invalidation.
    pub(crate) fn has_cached_chunks(&self) -> bool {
        !self.chunks.is_empty()
    }

    /// Assigns each full cell to its centre's chunk so packed overlay uploads contain no duplicates.
    pub(crate) fn visit_chunk_cells(&self, chunk: (i32, i32), mut visit: impl FnMut(CellKey)) {
        self.visit_in_bounds(Self::chunk_bounds(chunk), |key| {
            if self.cell_chunk(key) == Some(chunk) {
                visit(key);
            }
        });
    }

    /// World envelope for one cell overlay/generation chunk.
    pub(crate) fn chunk_bounds(chunk: (i32, i32)) -> CellBounds {
        let size = f64::from(RegionGraph::CHUNK_SIZE);
        let min = DVec2::new(f64::from(chunk.0), f64::from(chunk.1)) * size;
        CellBounds {
            min,
            max: min + DVec2::splat(size),
        }
    }

    fn cell_chunk(&self, key: CellKey) -> Option<(i32, i32)> {
        let frame = self.frame(key.grid)?;
        Some(chunk_at(
            frame.world(f64::from(key.x) + 0.5, f64::from(key.y) + 0.5),
        ))
    }

    /// Stamps a changed full cell's single rendering owner after its authority epoch advances.
    pub(super) fn mark_cell_chunk_changed(&mut self, key: CellKey) {
        if let Some(chunk) = self.cell_chunk(key) {
            self.chunks.entry(chunk).or_default().revision = self.revision;
        }
    }
}
