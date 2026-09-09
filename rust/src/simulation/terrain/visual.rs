// SPDX-License-Identifier: GPL-2.0-only

//! Bounded visual-terrain edits over the resident sparse grid, without modifying source terrain.

use super::*;
use std::borrow::Cow;

/// Visual samples for terrain compilation, with an explicit unchanged source/grid dependency.
pub(crate) trait TerrainVisualSource: Sync {
    /// Authoritative source terrain and coordinate frame; does not include visual edits.
    fn terrain(&self) -> &TerrainSystem;
    /// Bilinear visual height, including any planned structural resets and writes.
    fn sample_visual_height_world(&self, world_x: f32, world_z: f32) -> f32;
    /// Resident visual revision, or none when planned samples must not enter resident caches.
    fn visual_cache_generation(&self) -> Option<u64>;
}

impl TerrainVisualSource for TerrainSystem {
    fn visual_cache_generation(&self) -> Option<u64> {
        Some(self.visual_generation())
    }

    fn terrain(&self) -> &TerrainSystem {
        self
    }

    fn sample_visual_height_world(&self, world_x: f32, world_z: f32) -> f32 {
        self.sample_visual_height_world(world_x, world_z)
    }
}

/// Copy-on-write visual samples using the existing terrain storage chunk layout.
/// Untouched chunks fall through to the pinned terrain; no city-wide map or sample copy occurs.
#[derive(Debug)]
pub(crate) struct TerrainVisualOverlay {
    data: SparseChunkGrid<f32>,
    // Explicit coverage distinguishes a reset-to-default chunk from an untouched sparse chunk.
    chunks: HashSet<(usize, usize)>,
}

impl TerrainVisualOverlay {
    /// Captures existing storage chunks intersecting a local transaction's reset footprint.
    pub(crate) fn capture_region(
        &mut self,
        terrain: &TerrainSystem,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) {
        let Some((x0, x1, z0, z1)) = terrain.grid_rect_for_world_bounds(min_x, min_z, max_x, max_z)
        else {
            return;
        };
        let size = self.data.chunk_size();
        for z in z0 / size..=z1 / size {
            for x in x0 / size..=x1 / size {
                self.capture_chunk(terrain, x, z);
            }
        }
    }

    /// Restores a bounded checkpoint through the terrain subsystem, preserving source heights.
    /// Called while the simulation lock excludes concurrent edits to captured storage chunks.
    pub(crate) fn restore(self, terrain: &mut TerrainSystem) {
        let size = self.data.chunk_size();
        if !self.chunks.is_empty() {
            terrain.visual_generation = terrain.visual_generation.wrapping_add(1);
        }
        for (x, z) in self.chunks {
            let (x0, x1, z0, z1) = (
                x * size,
                ((x + 1) * size - 1).min(terrain.width - 1),
                z * size,
                ((z + 1) * size - 1).min(terrain.height - 1),
            );
            terrain.data.copy_rect_from(&self.data, x0, x1, z0, z1);
            terrain.mark_render_patches_for_grid_rect(x0, x1, z0, z1);
        }
    }

    /// Starts an empty local edit against the supplied terrain layout.
    pub(crate) fn new(terrain: &TerrainSystem) -> Self {
        Self {
            data: terrain.data.empty_like(),
            chunks: HashSet::new(),
        }
    }

    fn capture_chunk(&mut self, terrain: &TerrainSystem, x: usize, z: usize) {
        if self.chunks.insert((x, z)) {
            let size = self.data.chunk_size();
            self.data.copy_rect_from(
                &terrain.data,
                x * size,
                (x + 1) * size - 1,
                z * size,
                (z + 1) * size - 1,
            );
        }
    }

    /// Resets the same inclusive grid rectangle as live structural earthwork publication.
    /// Work is O(touched storage chunks + copied samples), independent of city size.
    pub(crate) fn reset_region_from_source_world(
        &mut self,
        terrain: &TerrainSystem,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) {
        let Some((x0, x1, z0, z1)) = terrain.grid_rect_for_world_bounds(min_x, min_z, max_x, max_z)
        else {
            return;
        };
        let size = self.data.chunk_size();
        for z in z0 / size..=z1 / size {
            for x in x0 / size..=x1 / size {
                self.capture_chunk(terrain, x, z);
            }
        }
        self.data
            .copy_rect_from(&terrain.source_data, x0, x1, z0, z1);
    }

    /// Applies the existing stamper's in-bounds, storage-chunk-grouped writes in original order.
    pub(crate) fn set_heights<U>(
        &mut self,
        terrain: &TerrainSystem,
        writes: &[U],
        cell: impl Fn(&U) -> (usize, usize, f32),
    ) {
        let size = self.data.chunk_size();
        let mut previous = None;
        for write in writes {
            let (x, z, _) = cell(write);
            let key = (x / size, z / size);
            if previous != Some(key) {
                self.capture_chunk(terrain, key.0, key.1);
                previous = Some(key);
            }
        }
        self.data.set_cells_grouped_by_chunk(writes, cell);
    }

    /// Removes unchanged coverage so ordinary grounded edits retain the live sampling fast path.
    pub(crate) fn discard_unchanged(&mut self, terrain: &TerrainSystem) {
        self.chunks
            .retain(|&(x, z)| !self.data.chunk_values_match(&terrain.data, x, z));
    }

    /// Exact post-adoption sample check, including structural grading outside patch interiors.
    pub(crate) fn matches_current(&self, terrain: &TerrainSystem) -> bool {
        self.chunks
            .iter()
            .all(|&(x, z)| self.data.chunk_values_match(&terrain.data, x, z))
    }

    /// Borrows the planned visual result over the same pinned terrain used to compile this edit.
    pub(crate) fn view<'a>(&'a self, terrain: &'a TerrainSystem) -> impl TerrainVisualSource + 'a {
        VisualView {
            terrain,
            overlay: self,
        }
    }

    /// Patches a captured texture, including its clamped border ring, from the final visual grid.
    pub(crate) fn patch_snapshot<'a>(
        &self,
        terrain: &TerrainSystem,
        patch: &'a TerrainPatchSnapshot,
    ) -> Cow<'a, TerrainPatchSnapshot> {
        if self.chunks.is_empty() {
            return Cow::Borrowed(patch);
        }
        let mut patch = patch.clone();
        // Use the captured patch's integer lattice, not a round-trip through f32 world origins.
        let start_x = patch.patch_x * terrain.render_patch_interval_cells;
        let start_z = patch.patch_z * terrain.render_patch_interval_cells;
        for z in 0..patch.texture_height {
            let grid_z = (start_z + z)
                .saturating_sub(patch.inner_offset_z)
                .min(terrain.height - 1);
            for x in 0..patch.texture_width {
                let grid_x = (start_x + x)
                    .saturating_sub(patch.inner_offset_x)
                    .min(terrain.width - 1);
                patch.height_data[z * patch.texture_width + x] =
                    self.grid(terrain, grid_x, grid_z).get(grid_x, grid_z);
            }
        }
        Cow::Owned(patch)
    }

    fn grid<'a>(
        &'a self,
        terrain: &'a TerrainSystem,
        x: usize,
        z: usize,
    ) -> &'a SparseChunkGrid<f32> {
        let size = self.data.chunk_size();
        if self.chunks.contains(&(x / size, z / size)) {
            &self.data
        } else {
            &terrain.data
        }
    }
}

struct VisualView<'a> {
    terrain: &'a TerrainSystem,
    overlay: &'a TerrainVisualOverlay,
}

impl TerrainVisualSource for VisualView<'_> {
    fn visual_cache_generation(&self) -> Option<u64> {
        self.overlay
            .chunks
            .is_empty()
            .then_some(self.terrain.visual_generation())
    }

    fn terrain(&self) -> &TerrainSystem {
        self.terrain
    }

    fn sample_visual_height_world(&self, world_x: f32, world_z: f32) -> f32 {
        if self.overlay.chunks.is_empty() {
            return self.terrain.sample_visual_height_world(world_x, world_z);
        }
        let (x, z) = self.terrain.world_to_grid_coords(world_x, world_z);
        self.terrain
            .interpolate_height_cells(x, z, |x0, x1, z0, z1| {
                let size = self.overlay.data.chunk_size();
                if x0 / size == x1 / size && z0 / size == z1 / size {
                    self.overlay
                        .grid(self.terrain, x0, z0)
                        .get_bilinear_cells(x0, x1, z0, z1)
                } else {
                    [(x0, z0), (x1, z0), (x0, z1), (x1, z1)]
                        .map(|(x, z)| self.overlay.grid(self.terrain, x, z).get(x, z))
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_resets_and_writes_match_live_samples_and_bordered_patches() {
        let mut base = TerrainSystem::with_chunking(37, 29, 1.0, 8, 1.5);
        for z in 0..base.height {
            for x in 0..base.width {
                base.set_height(x, z, 1.5 + (x + z) as f32 * 0.01);
            }
        }
        base.set_visual_heights_at_grid_unmarked(
            &[(15, 8, 9.0), (25, 20, 3.0), (0, 0, 5.0)],
            |sample| *sample,
        );
        let before = base.clone(); // Independent test oracle; production never clones the map.
        let mut live = base.clone();
        let mut overlay = TerrainVisualOverlay::new(&base);
        for (rect, writes) in [
            (
                (8, 16, 4, 16),
                vec![(15, 8, 2.0), (16, 8, 4.0), (17, 8, 6.0), (22, 10, 7.0)],
            ),
            ((16, 24, 8, 16), vec![(16, 8, -0.1)]),
            ((0, 3, 0, 3), vec![(0, 1, 2.5)]),
        ] {
            let (min_x, min_z) = base.grid_to_world_coords(rect.0, rect.2);
            let (max_x, max_z) = base.grid_to_world_coords(rect.1, rect.3);
            overlay.reset_region_from_source_world(&base, min_x, min_z, max_x, max_z);
            live.reset_visual_region_from_source_world(min_x, min_z, max_x, max_z);
            overlay.set_heights(&base, &writes, |sample| *sample);
            live.set_visual_heights_at_grid_unmarked(&writes, |sample| *sample);
        }
        overlay.discard_unchanged(&base);
        let view = overlay.view(&base);
        // Includes interpolation across storage boundaries and clamping beyond the map edges.
        for z in -2..=(base.height as i32 * 2 + 2) {
            for x in -2..=(base.width as i32 * 2 + 2) {
                let (origin_x, origin_z) = base.grid_to_world_coords(0, 0);
                let (wx, wz) = (origin_x + x as f32 * 0.5, origin_z + z as f32 * 0.5);
                assert_eq!(
                    view.sample_visual_height_world(wx, wz),
                    live.sample_visual_height_world(wx, wz),
                    "({x}, {z})"
                );
                assert_eq!(
                    base.sample_visual_height_world(wx, wz),
                    before.sample_visual_height_world(wx, wz)
                );
                assert_eq!(
                    base.sample_height_world(wx, wz),
                    live.sample_height_world(wx, wz)
                );
            }
        }
        for z in 0..4 {
            for x in 0..6 {
                let Some(patch) = base.visual_patch_snapshot(x, z) else {
                    continue;
                };
                assert_eq!(
                    overlay.patch_snapshot(&base, &patch).as_ref(),
                    &live.visual_patch_snapshot(x, z).unwrap()
                );
            }
        }
        let (wx, wz) = base.grid_to_world_coords(17, 8);
        assert_eq!(
            view.sample_visual_height_world(wx, wz),
            base.get_height(17, 8),
            "later reset must erase earlier neighboring stamp"
        );
        let (wx, wz) = base.grid_to_world_coords(25, 20);
        assert_eq!(
            view.sample_visual_height_world(wx, wz),
            3.0,
            "unrelated existing stamp stays untouched"
        );
    }

    #[test]
    fn reset_to_sparse_default_does_not_fall_through_and_work_stays_local() {
        let mut base = TerrainSystem::with_chunking(1_000_000, 1_000_000, 1.0, 8, 1.5);
        base.set_visual_heights_at_grid_unmarked(
            &[(16, 16, 9.0), (999_000, 999_000, 7.0)],
            |sample| *sample,
        );
        let mut overlay = TerrainVisualOverlay::new(&base);
        let (min_x, min_z) = base.grid_to_world_coords(16, 16);
        let (max_x, max_z) = base.grid_to_world_coords(23, 23);
        overlay.reset_region_from_source_world(&base, min_x, min_z, max_x, max_z);
        overlay.discard_unchanged(&base);
        assert_eq!(overlay.chunks.len(), 1);
        assert_eq!(overlay.data.materialized_chunk_count(), 0);
        assert_eq!(
            overlay.view(&base).sample_visual_height_world(min_x, min_z),
            1.5
        );
        let (wx, wz) = base.grid_to_world_coords(999_000, 999_000);
        assert_eq!(overlay.view(&base).sample_visual_height_world(wx, wz), 7.0);
        // A no-op reset need not add per-sample overlay probes to ordinary road compilation.
        let mut clean = TerrainVisualOverlay::new(&base);
        let (min_x, min_z) = base.grid_to_world_coords(32, 32);
        let (max_x, max_z) = base.grid_to_world_coords(39, 39);
        clean.reset_region_from_source_world(&base, min_x, min_z, max_x, max_z);
        clean.discard_unchanged(&base);
        assert!(clean.chunks.is_empty());
    }
}
