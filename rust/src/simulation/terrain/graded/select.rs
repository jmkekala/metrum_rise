// SPDX-License-Identifier: GPL-2.0-only

//! Dirty tile selection (`TERRAIN-07`): the tiles a road change or a base edit can alter, from
//! the changed seams' own reaches, so cost follows the edit and never the city.

use super::grade::{CAP_M, ReachBounds, reach};
use super::sample::{BaseGrid, BaseWindow};
use super::seams::{Seam, road_seam};
use super::tile::{Level, TileKey, grown, lattice, meets, window};
use crate::simulation::network::roadbed::{PieceId, RoadbedSystem, TerrainChanges};
use crate::simulation::terrain::TerrainSystem;
use rayon::prelude::*;
use std::cell::RefCell;

// Padding around a changed piece: coverage reads triangles this far beyond a lattice.
const PIECE_PAD_M: f64 = 0.01;

/// Reused buffers of the selection.
#[derive(Default)]
pub struct SelectScratch {
    changed: Vec<Seam>,
    candidates: Vec<TileKey>,
    dirty: Vec<bool>,
}

#[derive(Default)]
struct CandidateScratch {
    pieces: Vec<PieceId>,
    base: BaseWindow,
    xs: Vec<f64>,
    zs: Vec<f64>,
}

thread_local! {
    static CANDIDATE: RefCell<CandidateScratch> = RefCell::default();
}

/// The tiles at `level` whose bake a road change can alter, sorted and unique, into `out`:
/// every tile whose lattice meets a changed piece (coverage and the road heights there), and
/// every tile within a changed segment's band or its reach over that tile, the reach computed
/// as the tile's bake computes it but with the window's seam heights widened by every changed
/// segment, so it bounds the reach before and after the change and a removed seam counts. A
/// superset is safe: rebaking an unaffected tile reproduces its bits. `O(changed segments x
/// tiles within the cap of each)` plus one window per candidate tile, in parallel.
pub fn dirty_tiles_for_road_changes(
    terrain: &TerrainSystem,
    roadbed: &RoadbedSystem,
    changes: &TerrainChanges,
    level: Level,
    scratch: &mut SelectScratch,
    out: &mut Vec<TileKey>,
) {
    out.clear();
    let grid = BaseGrid::of(terrain);
    let band = level.band_m(grid.cell_m);
    // Lattice points lie at most one spacing (`GL3`: one step) beyond their tile.
    let apron = level.spacing_m(grid.cell_m);
    for bounds in &changes.pieces {
        push_tiles(grown(*bounds, apron + PIECE_PAD_M), out);
    }
    scratch.changed.clear();
    scratch
        .changed
        .extend(changes.segments.iter().map(|[a, b]| road_seam(*a, *b)));
    scratch.candidates.clear();
    for seam in &scratch.changed {
        push_tiles(
            grown(seam.box_xz(), CAP_M.max(band) + apron),
            &mut scratch.candidates,
        );
    }
    sort_unique(out);
    sort_unique(&mut scratch.candidates);
    scratch
        .candidates
        .retain(|tile| out.binary_search(tile).is_err());
    let changed = &scratch.changed;
    crate::thread_pools::install(|| {
        scratch
            .candidates
            .par_iter()
            .map(|&tile| {
                CANDIDATE.with_borrow_mut(|candidate| {
                    reached(terrain, roadbed, changed, tile, level, &grid, candidate)
                })
            })
            .collect_into_vec(&mut scratch.dirty);
    });
    out.extend(
        scratch
            .candidates
            .iter()
            .zip(&scratch.dirty)
            .filter_map(|(&tile, &dirty)| dirty.then_some(tile)),
    );
    sort_unique(out);
}

/// The tiles at `level` whose values a base edit of the samples `[x0, x1] x [z0, z1]` (grid
/// indices, inclusive) can alter: the graded value at a point reads the base only there, so
/// these are the tiles whose lattice meets the changed samples' bilinear support. Sorted.
pub fn dirty_tiles_for_base_change(
    terrain: &TerrainSystem,
    samples: [usize; 4],
    level: Level,
    out: &mut Vec<TileKey>,
) {
    out.clear();
    let grid = BaseGrid::of(terrain);
    let [x0, x1, z0, z1] = samples.map(|index| index as f64);
    let world = |axis: usize, index: f64| index * grid.cell_m - grid.half[axis];
    let support = [
        world(0, x0 - 1.0),
        world(1, z0 - 1.0),
        world(0, x1 + 1.0),
        world(1, z1 + 1.0),
    ];
    push_tiles(grown(support, level.spacing_m(grid.cell_m)), out);
    sort_unique(out);
}

// Whether a changed seam can reach the tile's lattice: its box grown by the band or by its
// reach over the tile's window, with the window's seam heights including every changed seam.
fn reached(
    terrain: &TerrainSystem,
    roadbed: &RoadbedSystem,
    changed: &[Seam],
    tile: TileKey,
    level: Level,
    grid: &BaseGrid,
    scratch: &mut CandidateScratch,
) -> bool {
    let band = level.band_m(grid.cell_m);
    lattice(terrain, tile, level, &mut scratch.xs, &mut scratch.zs);
    let rect = [
        scratch.xs[0],
        scratch.zs[0],
        scratch.xs[scratch.xs.len() - 1],
        scratch.zs[scratch.zs.len() - 1],
    ];
    let window = window(rect, band);
    let in_window = |seam: &&Seam| meets(seam.box_xz(), window);
    if !changed.iter().any(|seam| in_window(&seam)) {
        return false;
    }
    let mut heights = (f64::INFINITY, f64::NEG_INFINITY);
    let mut widen = |(lo, hi): (f64, f64)| heights = (heights.0.min(lo), heights.1.max(hi));
    roadbed.for_each_grounded_seam_in_bounds(window, &mut scratch.pieces, |[a, b]| {
        widen((a.y.min(b.y), a.y.max(b.y)));
    });
    for seam in changed.iter().filter(in_window) {
        widen(seam.height_range());
    }
    scratch.base.fill(terrain, window);
    let slope = scratch.base.slope();
    changed.iter().filter(in_window).any(|seam| {
        let reach = reach(&ReachBounds {
            seam: seam.height_range(),
            base_under: scratch.base.range(seam.box_xz()),
            window_seams: heights,
            slope,
        });
        meets(grown(seam.box_xz(), reach.max(band)), rect)
    })
}

// Appends every tile meeting `rect`.
fn push_tiles(rect: [f64; 4], out: &mut Vec<TileKey>) {
    let (first, last) = (TileKey::at(rect[0], rect[1]), TileKey::at(rect[2], rect[3]));
    for x in first.x..=last.x {
        for z in first.z..=last.z {
            out.push(TileKey { x, z });
        }
    }
}

fn sort_unique(tiles: &mut Vec<TileKey>) {
    tiles.sort_unstable();
    tiles.dedup();
}
