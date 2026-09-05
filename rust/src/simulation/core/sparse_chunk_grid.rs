//! Sparse chunk-backed grid storage for large authored worlds.
//!
//! The grid exposes dense `(x, y)` access but only allocates chunk payloads for
//! cells that differ from the configured default value. Dense materialization is
//! still available for save/load and renderer upload boundaries.

use std::collections::HashMap;
use std::sync::Arc;

/// Sparse chunk-backed 2D grid with fixed-size square chunks.
#[derive(Clone)]
pub(crate) struct SparseChunkGrid<T: Copy + PartialEq> {
    width: usize,
    height: usize,
    chunk_size: usize,
    default_value: T,
    chunks: HashMap<u64, Arc<Vec<T>>>,
}

impl<T: Copy + PartialEq> SparseChunkGrid<T> {
    /// Creates an empty sparse grid that returns `default_value` for untouched cells.
    pub(crate) fn new(width: usize, height: usize, chunk_size: usize, default_value: T) -> Self {
        Self {
            width,
            height,
            chunk_size: chunk_size.max(1),
            default_value,
            chunks: HashMap::new(),
        }
    }

    /// Returns the cell value at `(x, y)` or the default value when untouched or out of bounds.
    #[inline]
    pub(crate) fn get(&self, x: usize, y: usize) -> T {
        if x >= self.width || y >= self.height {
            return self.default_value;
        }

        let (chunk_x, chunk_y, local_idx) = self.cell_address(x, y);
        self.chunks
            .get(&Self::chunk_key(chunk_x, chunk_y))
            .map(|chunk| chunk[local_idx])
            .unwrap_or(self.default_value)
    }

    /// Returns four in-bounds cells used by bilinear interpolation.
    ///
    /// Adjacent interpolation cells normally share one sparse chunk, reducing four map probes to
    /// one. The uncommon storage-boundary case retains the ordinary per-cell lookup semantics.
    #[inline]
    pub(crate) fn get_bilinear_cells(&self, x0: usize, x1: usize, y0: usize, y1: usize) -> [T; 4] {
        debug_assert!(x0 < self.width && x1 < self.width);
        debug_assert!(y0 < self.height && y1 < self.height);
        let chunk_x0 = x0 / self.chunk_size;
        let chunk_y0 = y0 / self.chunk_size;
        if x1 / self.chunk_size == chunk_x0 && y1 / self.chunk_size == chunk_y0 {
            let Some(chunk) = self.chunks.get(&Self::chunk_key(chunk_x0, chunk_y0)) else {
                return [self.default_value; 4];
            };
            let local_x0 = x0 % self.chunk_size;
            let local_x1 = x1 % self.chunk_size;
            let local_y0 = y0 % self.chunk_size;
            let local_y1 = y1 % self.chunk_size;
            return [
                chunk[local_y0 * self.chunk_size + local_x0],
                chunk[local_y0 * self.chunk_size + local_x1],
                chunk[local_y1 * self.chunk_size + local_x0],
                chunk[local_y1 * self.chunk_size + local_x1],
            ];
        }

        [
            self.get(x0, y0),
            self.get(x1, y0),
            self.get(x0, y1),
            self.get(x1, y1),
        ]
    }

    /// Updates one cell and materializes its chunk only when needed.
    pub(crate) fn set(&mut self, x: usize, y: usize, value: T) {
        if x >= self.width || y >= self.height {
            return;
        }

        let (chunk_x, chunk_y, local_idx) = self.cell_address(x, y);
        let key = Self::chunk_key(chunk_x, chunk_y);

        if value == self.default_value {
            let remove_chunk = if let Some(chunk) = self.chunks.get_mut(&key) {
                let chunk = Arc::make_mut(chunk);
                chunk[local_idx] = value;
                chunk.iter().all(|cell| *cell == self.default_value)
            } else {
                false
            };
            if remove_chunk {
                self.chunks.remove(&key);
            }
            return;
        }

        let chunk_len = self.chunk_len();
        let default_value = self.default_value;
        let chunk = self
            .chunks
            .entry(key)
            .or_insert_with(|| Arc::new(vec![default_value; chunk_len]));
        let chunk = Arc::make_mut(chunk);
        chunk[local_idx] = value;
    }

    /// Applies cells grouped by storage chunk while making each shared chunk mutable once.
    ///
    /// The caller must keep equal chunk keys contiguous. Repeated groups remain correct but lose
    /// the batching benefit. Coordinates must be in bounds.
    pub(crate) fn set_cells_grouped_by_chunk<U>(
        &mut self,
        cells: &[U],
        mut cell_value: impl FnMut(&U) -> (usize, usize, T),
    ) {
        let mut group_start = 0;
        while group_start < cells.len() {
            let (first_x, first_y, _) = cell_value(&cells[group_start]);
            debug_assert!(first_x < self.width && first_y < self.height);
            let first_chunk_x = first_x / self.chunk_size;
            let first_chunk_y = first_y / self.chunk_size;
            let key = Self::chunk_key(first_chunk_x, first_chunk_y);
            let mut group_end = group_start + 1;
            while group_end < cells.len() {
                let (x, y, _) = cell_value(&cells[group_end]);
                debug_assert!(x < self.width && y < self.height);
                if x / self.chunk_size != first_chunk_x || y / self.chunk_size != first_chunk_y {
                    break;
                }
                group_end += 1;
            }

            let group = &cells[group_start..group_end];
            let group_has_non_default = group
                .iter()
                .any(|cell| cell_value(cell).2 != self.default_value);
            if self.chunks.contains_key(&key) || group_has_non_default {
                let default_value = self.default_value;
                let chunk_len = self.chunk_len();
                let mut wrote_default = false;
                let remove_chunk = {
                    let chunk = self
                        .chunks
                        .entry(key)
                        .or_insert_with(|| Arc::new(vec![default_value; chunk_len]));
                    let chunk = Arc::make_mut(chunk);
                    for cell in group {
                        let (x, y, value) = cell_value(cell);
                        wrote_default |= value == default_value;
                        let local_x = x - first_chunk_x * self.chunk_size;
                        let local_y = y - first_chunk_y * self.chunk_size;
                        chunk[local_y * self.chunk_size + local_x] = value;
                    }
                    wrote_default && chunk.iter().all(|cell| *cell == default_value)
                };
                if remove_chunk {
                    self.chunks.remove(&key);
                }
            }
            group_start = group_end;
        }
    }

    /// Returns the fixed storage-chunk width and height in cells.
    pub(crate) fn chunk_size(&self) -> usize {
        self.chunk_size
    }

    /// Copies an inclusive rectangular region from a layout-compatible grid.
    ///
    /// Shared chunks are already identical and cost only one pointer comparison. Diverged
    /// chunks are copied row-wise and compacted back to sparse storage when they become default.
    pub(crate) fn copy_rect_from(
        &mut self,
        source: &Self,
        min_x: usize,
        max_x: usize,
        min_y: usize,
        max_y: usize,
    ) {
        debug_assert_eq!(self.width, source.width);
        debug_assert_eq!(self.height, source.height);
        debug_assert_eq!(self.chunk_size, source.chunk_size);
        if self.width == 0 || self.height == 0 || min_x > max_x || min_y > max_y {
            return;
        }

        let min_x = min_x.min(self.width - 1);
        let max_x = max_x.min(self.width - 1);
        let min_y = min_y.min(self.height - 1);
        let max_y = max_y.min(self.height - 1);
        let min_chunk_x = min_x / self.chunk_size;
        let max_chunk_x = max_x / self.chunk_size;
        let min_chunk_y = min_y / self.chunk_size;
        let max_chunk_y = max_y / self.chunk_size;

        for chunk_y in min_chunk_y..=max_chunk_y {
            for chunk_x in min_chunk_x..=max_chunk_x {
                let key = Self::chunk_key(chunk_x, chunk_y);
                let source_chunk = source.chunks.get(&key).cloned();
                let chunks_are_shared = self
                    .chunks
                    .get(&key)
                    .zip(source_chunk.as_ref())
                    .is_some_and(|(target, source)| Arc::ptr_eq(target, source));
                if chunks_are_shared || (source_chunk.is_none() && !self.chunks.contains_key(&key))
                {
                    continue;
                }

                let chunk_origin_x = chunk_x * self.chunk_size;
                let chunk_origin_y = chunk_y * self.chunk_size;
                let local_min_x = min_x.saturating_sub(chunk_origin_x);
                let local_max_x = max_x
                    .saturating_sub(chunk_origin_x)
                    .min(self.chunk_size - 1);
                let local_min_y = min_y.saturating_sub(chunk_origin_y);
                let local_max_y = max_y
                    .saturating_sub(chunk_origin_y)
                    .min(self.chunk_size - 1);
                let covers_whole_chunk = local_min_x == 0
                    && local_max_x == self.chunk_size - 1
                    && local_min_y == 0
                    && local_max_y == self.chunk_size - 1;
                if covers_whole_chunk {
                    if let Some(source_chunk) = source_chunk {
                        self.chunks.insert(key, source_chunk);
                    } else {
                        self.chunks.remove(&key);
                    }
                    continue;
                }

                let chunk_len = self.chunk_len();
                let default_value = self.default_value;
                let target = self
                    .chunks
                    .entry(key)
                    .or_insert_with(|| Arc::new(vec![default_value; chunk_len]));
                let target = Arc::make_mut(target);
                for local_y in local_min_y..=local_max_y {
                    let row_start = local_y * self.chunk_size + local_min_x;
                    let row_end = local_y * self.chunk_size + local_max_x + 1;
                    if let Some(source_chunk) = source_chunk.as_ref() {
                        target[row_start..row_end]
                            .copy_from_slice(&source_chunk[row_start..row_end]);
                    } else {
                        target[row_start..row_end].fill(default_value);
                    }
                }
                if target.iter().all(|cell| *cell == default_value) {
                    self.chunks.remove(&key);
                }
            }
        }
    }

    /// Copies an inclusive rectangular region into an existing row-major buffer.
    ///
    /// The destination offsets locate the region's upper-left cell. Chunk rows are copied as
    /// slices, so extracting a render patch performs one hash lookup per touched chunk instead of
    /// one lookup per terrain sample.
    pub(crate) fn copy_rect_into(
        &self,
        min_x: usize,
        max_x: usize,
        min_y: usize,
        max_y: usize,
        target: &mut [T],
        target_stride: usize,
        target_x: usize,
        target_y: usize,
    ) {
        debug_assert!(min_x <= max_x && max_x < self.width);
        debug_assert!(min_y <= max_y && max_y < self.height);
        debug_assert!(target_x + max_x - min_x < target_stride);
        debug_assert!((target_y + max_y - min_y + 1) * target_stride <= target.len());

        let min_chunk_x = min_x / self.chunk_size;
        let max_chunk_x = max_x / self.chunk_size;
        let min_chunk_y = min_y / self.chunk_size;
        let max_chunk_y = max_y / self.chunk_size;

        for chunk_y in min_chunk_y..=max_chunk_y {
            let chunk_origin_y = chunk_y * self.chunk_size;
            let copy_min_y = min_y.max(chunk_origin_y);
            let copy_max_y = max_y.min(chunk_origin_y + self.chunk_size - 1);
            for chunk_x in min_chunk_x..=max_chunk_x {
                let chunk_origin_x = chunk_x * self.chunk_size;
                let copy_min_x = min_x.max(chunk_origin_x);
                let copy_max_x = max_x.min(chunk_origin_x + self.chunk_size - 1);
                let copy_width = copy_max_x - copy_min_x + 1;
                let source_chunk = self.chunks.get(&Self::chunk_key(chunk_x, chunk_y));

                for source_y in copy_min_y..=copy_max_y {
                    let destination_start =
                        (target_y + source_y - min_y) * target_stride + target_x + copy_min_x
                            - min_x;
                    let destination_end = destination_start + copy_width;
                    if let Some(source_chunk) = source_chunk {
                        let source_start = (source_y - chunk_origin_y) * self.chunk_size
                            + copy_min_x
                            - chunk_origin_x;
                        target[destination_start..destination_end].copy_from_slice(
                            &source_chunk[source_start..source_start + copy_width],
                        );
                    } else {
                        target[destination_start..destination_end].fill(self.default_value);
                    }
                }
            }
        }
    }

    /// Returns a full row-major dense snapshot of the sparse grid.
    pub(crate) fn clone_dense(&self) -> Vec<T> {
        let mut dense = vec![self.default_value; self.width * self.height];
        let chunk_size = self.chunk_size;

        for (&key, chunk) in &self.chunks {
            let (chunk_x, chunk_y) = Self::decode_chunk_key(key);
            let origin_x = chunk_x * chunk_size;
            let origin_y = chunk_y * chunk_size;
            let copy_w = (self.width.saturating_sub(origin_x)).min(chunk_size);
            let copy_h = (self.height.saturating_sub(origin_y)).min(chunk_size);

            for local_y in 0..copy_h {
                let src_start = local_y * chunk_size;
                let dst_start = (origin_y + local_y) * self.width + origin_x;
                let dst_end = dst_start + copy_w;
                dense[dst_start..dst_end].copy_from_slice(&chunk[src_start..src_start + copy_w]);
            }
        }

        dense
    }

    /// Replaces the sparse contents from a dense row-major buffer.
    pub(crate) fn replace_from_dense(&mut self, dense: &[T]) -> Result<(), String> {
        if dense.len() != self.width * self.height {
            return Err(format!(
                "dense slice length mismatch: got {}, expected {}",
                dense.len(),
                self.width * self.height
            ));
        }

        self.chunks.clear();
        let chunk_cols = self.width.div_ceil(self.chunk_size);
        let chunk_rows = self.height.div_ceil(self.chunk_size);
        let chunk_len = self.chunk_len();

        for chunk_y in 0..chunk_rows {
            for chunk_x in 0..chunk_cols {
                let origin_x = chunk_x * self.chunk_size;
                let origin_y = chunk_y * self.chunk_size;
                let copy_w = (self.width - origin_x).min(self.chunk_size);
                let copy_h = (self.height - origin_y).min(self.chunk_size);
                let mut chunk = vec![self.default_value; chunk_len];
                let mut touched = false;

                for local_y in 0..copy_h {
                    let src_start = (origin_y + local_y) * self.width + origin_x;
                    let src_end = src_start + copy_w;
                    let row = &dense[src_start..src_end];
                    let dst_start = local_y * self.chunk_size;
                    chunk[dst_start..dst_start + copy_w].copy_from_slice(row);
                    if !touched && row.iter().any(|cell| *cell != self.default_value) {
                        touched = true;
                    }
                }

                if touched {
                    self.chunks
                        .insert(Self::chunk_key(chunk_x, chunk_y), Arc::new(chunk));
                }
            }
        }

        Ok(())
    }

    /// Returns the number of materialized chunks currently stored.
    #[cfg(test)]
    pub(crate) fn materialized_chunk_count(&self) -> usize {
        self.chunks.len()
    }

    fn chunk_len(&self) -> usize {
        self.chunk_size * self.chunk_size
    }

    fn cell_address(&self, x: usize, y: usize) -> (usize, usize, usize) {
        let chunk_x = x / self.chunk_size;
        let chunk_y = y / self.chunk_size;
        let local_x = x % self.chunk_size;
        let local_y = y % self.chunk_size;
        (chunk_x, chunk_y, local_y * self.chunk_size + local_x)
    }

    fn chunk_key(chunk_x: usize, chunk_y: usize) -> u64 {
        ((chunk_y as u64) << 32) | chunk_x as u64
    }

    fn decode_chunk_key(key: u64) -> (usize, usize) {
        ((key & 0xFFFF_FFFF) as usize, (key >> 32) as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::SparseChunkGrid;

    #[test]
    fn untouched_cells_read_back_default_without_materializing_everything() {
        let mut grid = SparseChunkGrid::new(10, 10, 4, 0.0f32);
        grid.set(2, 3, 5.0);

        assert_eq!(grid.get(0, 0), 0.0);
        assert_eq!(grid.get(2, 3), 5.0);
        assert_eq!(grid.materialized_chunk_count(), 1);
    }

    #[test]
    fn dense_round_trip_only_keeps_non_default_chunks() {
        let dense = vec![
            0u16, 0, 0, 0, //
            0, 7, 0, 0, //
            0, 0, 0, 0, //
            0, 0, 9, 0,
        ];
        let mut grid = SparseChunkGrid::new(4, 4, 2, 0u16);
        grid.replace_from_dense(&dense)
            .expect("dense slice should match grid size");

        assert_eq!(grid.clone_dense(), dense);
        assert_eq!(grid.materialized_chunk_count(), 2);
    }

    #[test]
    fn rectangular_copy_restores_only_the_selected_cells() {
        let mut source = SparseChunkGrid::new(8, 8, 4, 0u16);
        source.set(2, 2, 7);
        source.set(6, 6, 9);
        let mut visual = source.clone();
        visual.set(2, 2, 3);
        visual.set(3, 3, 5);
        visual.set(6, 6, 4);

        visual.copy_rect_from(&source, 1, 4, 1, 4);

        assert_eq!(visual.get(2, 2), 7);
        assert_eq!(visual.get(3, 3), 0);
        assert_eq!(visual.get(6, 6), 4);
    }

    #[test]
    fn rectangular_snapshot_copies_chunk_rows_and_default_gaps() {
        let mut grid = SparseChunkGrid::new(9, 7, 4, 3u16);
        grid.set(2, 1, 7);
        grid.set(4, 3, 9);
        grid.set(7, 5, 11);
        let mut snapshot = vec![0u16; 9 * 7];

        grid.copy_rect_into(1, 7, 1, 5, &mut snapshot, 9, 1, 1);

        for y in 1..=5 {
            for x in 1..=7 {
                assert_eq!(snapshot[y * 9 + x], grid.get(x, y));
            }
        }
        assert!(snapshot[0..9].iter().all(|sample| *sample == 0));
        assert!(snapshot.iter().step_by(9).all(|sample| *sample == 0));
    }

    #[test]
    fn bilinear_cells_match_individual_reads_across_chunk_boundaries() {
        let mut grid = SparseChunkGrid::new(9, 9, 4, 3u16);
        grid.set(3, 3, 7);
        grid.set(4, 3, 8);
        grid.set(3, 4, 9);
        grid.set(4, 4, 10);

        assert_eq!(grid.get_bilinear_cells(2, 3, 2, 3), [3, 3, 3, 7]);
        assert_eq!(grid.get_bilinear_cells(3, 4, 3, 4), [7, 8, 9, 10]);
        assert_eq!(grid.get_bilinear_cells(8, 8, 8, 8), [3; 4]);
    }

    #[test]
    fn grouped_cell_writes_preserve_sparse_default_compaction() {
        let mut grid = SparseChunkGrid::new(8, 8, 4, 0u16);
        let writes = [(1, 1, 7), (2, 2, 8), (4, 1, 9), (5, 2, 10)];
        grid.set_cells_grouped_by_chunk(&writes, |write| *write);

        assert_eq!(grid.get(1, 1), 7);
        assert_eq!(grid.get(5, 2), 10);
        assert_eq!(grid.materialized_chunk_count(), 2);

        let clears = [(1, 1, 0), (2, 2, 0), (4, 1, 0), (5, 2, 0)];
        grid.set_cells_grouped_by_chunk(&clears, |write| *write);
        assert_eq!(grid.materialized_chunk_count(), 0);
    }
}
