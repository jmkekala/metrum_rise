// SPDX-License-Identifier: GPL-2.0-only

//! Small index helpers shared by surface compilation stages.

use super::keys::SurfaceXzKey;

const SURFACE_SEGMENT_TILE_KEYS: i64 = 8_000_000;

/// Inclusive fixed-point XZ bounds used for conservative segment candidate pruning.
#[derive(Clone, Copy, Debug)]
pub(super) struct SurfaceKeyBounds {
    min_x: i64,
    min_z: i64,
    max_x: i64,
    max_z: i64,
}

/// World-aligned tile occupied by one conservative fixed-point segment bound.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct SurfaceKeyTile {
    x: i64,
    z: i64,
}

pub(super) fn normalized_vertex_edge(a: usize, b: usize) -> [usize; 2] {
    if a < b { [a, b] } else { [b, a] }
}

impl SurfaceKeyBounds {
    /// Builds inclusive bounds for a fixed-point segment.
    pub(super) fn from_segment(start: SurfaceXzKey, end: SurfaceXzKey) -> Self {
        Self {
            min_x: start.x_key().min(end.x_key()),
            min_z: start.z_key().min(end.z_key()),
            max_x: start.x_key().max(end.x_key()),
            max_z: start.z_key().max(end.z_key()),
        }
    }

    /// Returns whether two inclusive bounds can contain an overlap.
    pub(super) fn overlaps(self, other: Self) -> bool {
        self.min_x <= other.max_x
            && other.min_x <= self.max_x
            && self.min_z <= other.max_z
            && other.min_z <= self.max_z
    }

    /// Expands every side by a non-negative fixed-point tolerance.
    pub(super) fn expanded(self, padding_keys: i64) -> Self {
        debug_assert!(padding_keys >= 0);
        Self {
            min_x: self.min_x.saturating_sub(padding_keys),
            min_z: self.min_z.saturating_sub(padding_keys),
            max_x: self.max_x.saturating_add(padding_keys),
            max_z: self.max_z.saturating_add(padding_keys),
        }
    }
}

impl SurfaceKeyTile {
    /// Visits every world-aligned tile touched by the conservative bounds.
    pub(super) fn for_each_in_bounds(bounds: SurfaceKeyBounds, mut visit: impl FnMut(Self)) {
        let min_tile_x = bounds.min_x.div_euclid(SURFACE_SEGMENT_TILE_KEYS);
        let max_tile_x = bounds.max_x.div_euclid(SURFACE_SEGMENT_TILE_KEYS);
        let min_tile_z = bounds.min_z.div_euclid(SURFACE_SEGMENT_TILE_KEYS);
        let max_tile_z = bounds.max_z.div_euclid(SURFACE_SEGMENT_TILE_KEYS);
        for x in min_tile_x..=max_tile_x {
            for z in min_tile_z..=max_tile_z {
                visit(Self { x, z });
            }
        }
    }
}
