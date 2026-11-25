// SPDX-License-Identifier: GPL-3.0-or-later

//! The base terrain's memory budget (`TERRAIN-41` B1, `docs/terrain.md` "Base terrain under a
//! memory budget"): how many patches may stay built, and the keep and release rectangles about the
//! wanted square that follow from it.
//!
//! - A **place** is the texels one terrain patch and its water patch ask the device for, from
//!   sizes the game itself chooses: the height texture (R32F, the payload's border of
//!   [`TERRAIN_RENDER_PATCH_BORDER_TEXELS`] a side), the land cover (R8 at the vegetation's 8 m
//!   texel, one texel of border a side) and the water depth (R32F, [`WATER_RENDER_PATCH_BORDER_TEXELS`]
//!   a side). Detail steps do not enter: a patch's textures are the same at every step.
//! - For a budget `B` and the extras `M` the scripts report (spare textures, water meshes, pooled
//!   resources), `N = floor((B - M) / P)` places.
//! - The **keep rectangle** is the wanted square widened by `K` patches, `K` the largest value for
//!   which the keep rectangle widened by two more patches holds at most `N` patches. Its unbuilt
//!   patches are filled (built hidden) once nothing else waits.
//! - The **release rectangle** is the keep rectangle widened by those two patches: built patches
//!   outside it are released, farthest first. The two patches are the hysteresis: a patch built at
//!   the keep rectangle's edge is released only after the camera moved two patches further, so a
//!   camera that wobbles across a patch boundary builds and frees nothing.

use super::PatchGrid;
use crate::simulation::terrain::TERRAIN_RENDER_PATCH_BORDER_TEXELS;
use crate::simulation::water::WATER_RENDER_PATCH_BORDER_TEXELS;

/// The vegetation's land cover texel, metres (`vegetation_api`'s `get_vegetation_land_cover`).
const LAND_COVER_TEXEL_M: f64 = 8.0;
/// Patches of hysteresis between the keep and the release rectangle.
pub(crate) const RELEASE_RING: i32 = 2;

/// The bytes of one place on `grid` by kind: `[height, land cover, water depth]` (an edge patch
/// is charged the full place; the debug view's split, `TERRAIN-41` B2).
pub(crate) fn place_kinds(grid: &PatchGrid) -> [u64; 3] {
    let samples = grid.interval.max(1) as u64 + 1;
    let height = samples + 2 * TERRAIN_RENDER_PATCH_BORDER_TEXELS as u64;
    let cover = (grid.span() / LAND_COVER_TEXEL_M).ceil() as u64 + 2;
    let depth = samples + 2 * WATER_RENDER_PATCH_BORDER_TEXELS as u64;
    [height * height * 4, cover * cover, depth * depth * 4]
}

/// The bytes of one place on `grid`: `[terrain patch, water patch]`.
pub(crate) fn place_bytes(grid: &PatchGrid) -> [u64; 2] {
    let [height, cover, depth] = place_kinds(grid);
    [height + cover, depth]
}

/// What a budget allows on a grid about a wanted square.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Keep {
    /// Places the budget holds (`N`); `u64::MAX` without a budget.
    pub(crate) places: u64,
    /// Patches the keep rectangle widens the wanted square by (`K`).
    pub(crate) ring: i32,
    /// The wanted square and its release ring hold more patches than the budget's places (a
    /// budget below the view): the square is built anyway.
    pub(crate) overrun: bool,
}

/// `N` places for a budget of `budget` bytes (none: unlimited) less `extras` bytes. `O(1)`.
pub(crate) fn places(grid: &PatchGrid, budget: Option<u64>, extras: u64) -> u64 {
    let [terrain, water] = place_bytes(grid);
    budget.map_or(u64::MAX, |budget| {
        budget.saturating_sub(extras) / (terrain + water)
    })
}

/// `K` for `places` about `square`: the largest ring whose rectangle widened by
/// [`RELEASE_RING`] holds at most `places` patches; the ring that covers the grid when every
/// patch fits. `O(K)` rectangle areas.
pub(crate) fn keep(grid: &PatchGrid, square: [i32; 4], places: u64) -> Keep {
    let area = |rect: [i32; 4]| {
        ((rect[2] - rect[0] + 1).max(0) as u64) * ((rect[3] - rect[1] + 1).max(0) as u64)
    };
    let whole = [0, 0, grid.cols - 1, grid.rows - 1];
    if area(grid.widen(square, RELEASE_RING)) > places {
        return Keep {
            places,
            ring: 0,
            overrun: true,
        };
    }
    let mut ring = 0;
    while grid.widen(square, ring) != whole
        && area(grid.widen(square, ring + 1 + RELEASE_RING)) <= places
    {
        ring += 1;
    }
    Keep {
        places,
        ring,
        overrun: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The 60 km world's layout: 117 x 117 patches of 64 cells of 8 m.
    const WIDE: PatchGrid = PatchGrid {
        cols: 117,
        rows: 117,
        interval: 64,
        cell: 8.0,
        world: [59904.0, 59904.0],
    };

    #[test]
    fn a_place_is_the_planned_texels() {
        // 73 x 73 R32F heights, 66 x 66 R8 land cover, 67 x 67 R32F water depth.
        assert_eq!(place_bytes(&WIDE), [21_316 + 4_356, 17_956]);
        assert_eq!(places(&WIDE, Some(512_000_000), 0), 11_735);
        assert_eq!(places(&WIDE, Some(512_000_000), 512_000_001), 0);
        assert_eq!(places(&WIDE, None, 0), u64::MAX);
    }

    #[test]
    fn the_ring_is_the_largest_whose_release_rectangle_fits() {
        let square = [40, 40, 92, 92];
        for places in [0, 3_249, 3_250, 5_000, 5_867, 11_735, 13_689, u64::MAX] {
            let kept = keep(&WIDE, square, places);
            let release = |ring: i32| {
                let rect = WIDE.widen(square, ring + RELEASE_RING);
                ((rect[2] - rect[0] + 1) * (rect[3] - rect[1] + 1)) as u64
            };
            if kept.overrun {
                assert!(release(0) > places && kept.ring == 0, "{places}");
                continue;
            }
            assert!(release(kept.ring) <= places, "{places}");
            let whole = WIDE.widen(square, kept.ring) == [0, 0, 116, 116];
            assert!(whole || release(kept.ring + 1) > places, "{places}");
        }
        assert!(keep(&WIDE, square, 3_248).overrun);
        assert_eq!(
            WIDE.widen(square, keep(&WIDE, square, u64::MAX).ring),
            [0, 0, 116, 116]
        );
    }
}
