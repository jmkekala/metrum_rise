//! Sparse chunk-backed grid storage for large authored worlds.
//!
//! The grid exposes dense `(x, y)` access but only allocates chunk payloads for
//! cells that differ from the configured default value. Dense materialization is
//! still available for save/load and renderer upload boundaries.

use std::collections::HashMap;

/// Sparse chunk-backed 2D grid with fixed-size square chunks.
#[derive(Clone)]
pub(crate) struct SparseChunkGrid<T: Copy + PartialEq> {
    width: usize,
    height: usize,
    chunk_size: usize,
    default_value: T,
    chunks: HashMap<u64, Vec<T>>,
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

    /// Updates one cell and materializes its chunk only when needed.
    pub(crate) fn set(&mut self, x: usize, y: usize, value: T) {
        if x >= self.width || y >= self.height {
            return;
        }

        let (chunk_x, chunk_y, local_idx) = self.cell_address(x, y);
        let key = Self::chunk_key(chunk_x, chunk_y);

        if value == self.default_value {
            let remove_chunk = if let Some(chunk) = self.chunks.get_mut(&key) {
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
            .or_insert_with(|| vec![default_value; chunk_len]);
        chunk[local_idx] = value;
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
                    self.chunks.insert(Self::chunk_key(chunk_x, chunk_y), chunk);
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
}
