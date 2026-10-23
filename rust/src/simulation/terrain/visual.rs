// SPDX-License-Identifier: GPL-2.0-only

//! Visual-terrain sources and bounded captures of resident visual samples.

use super::*;

/// Visual samples for terrain compilation, with an explicit unchanged source/grid dependency.
pub(crate) trait TerrainVisualSource: Sync {
    /// Authoritative source terrain and coordinate frame; does not include visual edits.
    fn terrain(&self) -> &TerrainSystem;
    /// Bilinear visual height.
    fn sample_visual_height_world(&self, world_x: f32, world_z: f32) -> f32;
}

impl TerrainVisualSource for TerrainSystem {
    fn terrain(&self) -> &TerrainSystem {
        self
    }

    fn sample_visual_height_world(&self, world_x: f32, world_z: f32) -> f32 {
        self.sample_visual_height_world(world_x, world_z)
    }
}

/// Captured visual samples using the existing terrain storage chunk layout, for dependency
/// checks; no city-wide map or sample copy occurs.
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

    /// Exact post-adoption sample check, including structural grading outside patch interiors.
    pub(crate) fn matches_current(&self, terrain: &TerrainSystem) -> bool {
        self.chunks
            .iter()
            .all(|&(x, z)| self.data.chunk_values_match(&terrain.data, x, z))
    }
}
