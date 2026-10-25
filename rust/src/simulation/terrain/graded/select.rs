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
    other_pieces: Vec<PieceId>,
    triangles: Vec<[[u64; 3]; 3]>,
    other_triangles: Vec<[[u64; 3]; 3]>,
    base: BaseWindow,
    xs: Vec<f64>,
    zs: Vec<f64>,
    // The candidate's lattice rectangle.
    rect: [f64; 4],
}

thread_local! {
    static CANDIDATE: RefCell<CandidateScratch> = RefCell::default();
}

/// The tiles at `level` whose bake a road change from `before` to `after` can alter, sorted
/// and unique, into `out`:
/// - every tile within a changed segment's band or its reach over that tile, the reach computed
///   as the tile's bake computes it but with the window's lowest seam height taken over every
///   changed segment too, so it bounds the reach before and after the change and a removed seam
///   counts;
/// - every tile whose lattice meets a changed piece and whose covering top triangles differ
///   between the two roadbeds (coverage and the road surface under the deep inside rule), so a
///   split trunk's far half, rebuilt bit for bit, marks nothing.
///
/// A superset is safe: rebaking an unaffected tile reproduces its bits. `O(changed segments x
/// tiles within the cap of each)` plus one window per candidate tile, in parallel.
pub fn dirty_tiles_for_road_changes(
    terrain: &TerrainSystem,
    before: &RoadbedSystem,
    after: &RoadbedSystem,
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
    scratch.candidates.clear();
    for bounds in &changes.pieces {
        push_tiles(grown(*bounds, apron + PIECE_PAD_M), &mut scratch.candidates);
    }
    scratch.changed.clear();
    scratch
        .changed
        .extend(changes.segments.iter().map(|[a, b]| road_seam(*a, *b)));
    for seam in &scratch.changed {
        push_tiles(
            grown(seam.box_xz(), CAP_M.max(band) + apron),
            &mut scratch.candidates,
        );
    }
    sort_unique(&mut scratch.candidates);
    let changed = &scratch.changed;
    let pieces = &changes.pieces;
    crate::thread_pools::install(|| {
        scratch
            .candidates
            .par_iter()
            .map(|&tile| {
                CANDIDATE.with_borrow_mut(|candidate| {
                    reached(terrain, after, changed, tile, level, &grid, candidate)
                        || covering_triangles_changed(before, after, pieces, candidate)
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
}

// Whether the grounded top triangles that can cover a lattice point of the candidate's lattice
// (left in `candidate` by `reached`) differ between the roadbeds, where a changed piece meets
// it. Pieces published in both with the same mesh are skipped; the others' triangles meeting
// the lattice are compared as sorted bits, whatever their pieces and grids.
fn covering_triangles_changed(
    before: &RoadbedSystem,
    after: &RoadbedSystem,
    pieces: &[[f64; 4]],
    candidate: &mut CandidateScratch,
) -> bool {
    let rect = grown(candidate.rect, PIECE_PAD_M);
    if !pieces.iter().any(|bounds| meets(*bounds, rect)) {
        return false;
    }
    before.grounded_pieces_in_bounds(rect, &mut candidate.pieces);
    after.grounded_pieces_in_bounds(rect, &mut candidate.other_pieces);
    let shared = |id: &PieceId, other: &RoadbedSystem| {
        let (a, b) = (before.piece_mesh(*id), other.piece_mesh(*id));
        matches!((a, b), (Some(a), Some(b)) if std::ptr::eq(a, b))
    };
    for (roadbed, ids, bits) in [
        (before, &candidate.pieces, &mut candidate.triangles),
        (
            after,
            &candidate.other_pieces,
            &mut candidate.other_triangles,
        ),
    ] {
        bits.clear();
        for id in ids.iter().filter(|id| !shared(id, after)) {
            let Some(mesh) = roadbed.piece_mesh(*id) else {
                continue;
            };
            mesh.any_top_triangle_in_bounds(rect, |triangle| {
                let points = mesh.triangle_points(triangle);
                if meets(triangle_box(&points), rect) {
                    bits.push(points.map(|p| [p.x, p.y, p.z].map(f64::to_bits)));
                }
                false
            });
        }
        bits.sort_unstable();
        bits.dedup();
    }
    candidate.triangles != candidate.other_triangles
}

fn triangle_box(points: &[crate::simulation::network::roadbed::RoadVec3; 3]) -> [f64; 4] {
    let [a, b, c] = points;
    [
        a.x.min(b.x).min(c.x),
        a.z.min(b.z).min(c.z),
        a.x.max(b.x).max(c.x),
        a.z.max(b.z).max(c.z),
    ]
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
// reach over the tile's window, with the window's lowest seam height including every changed
// seam.
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
    scratch.rect = rect;
    let window = window(rect, band);
    let in_window = |seam: &&Seam| meets(seam.box_xz(), window);
    if !changed.iter().any(|seam| in_window(&seam)) {
        return false;
    }
    let mut window_low = f64::INFINITY;
    roadbed.for_each_grounded_seam_in_bounds(window, &mut scratch.pieces, |[a, b]| {
        window_low = window_low.min(a.y).min(b.y);
    });
    for seam in changed.iter().filter(in_window) {
        window_low = window_low.min(seam.height_range().0);
    }
    scratch.base.fill(terrain, window);
    let slope = scratch.base.slope();
    changed.iter().filter(in_window).any(|seam| {
        let reach = reach(&ReachBounds {
            seam: seam.height_range(),
            base_under: scratch.base.range(seam.box_xz()),
            window_low,
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
