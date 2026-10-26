// SPDX-License-Identifier: GPL-2.0-only

//! Dirty tile selection (`TERRAIN-07`): the tiles a road change, a site change or a base edit
//! can alter, from the changed seams' own reaches, so cost follows the edit and never the city.

use super::grade::{CAP_M, TIE_IN_SLOPE, lift_reach};
use super::portal::{PORTAL_REACH_M, PortalWindow};
use super::prune::{BinGrid, bin_side, box_distance};
use super::sample::{BaseGrid, BaseWindow};
use super::seams::{Seam, road_seam};
use super::sites::{SiteBuffer, SiteSource, box_seam};
use super::tile::{Level, TileKey, grown, lattice, meets, window};
use crate::simulation::buildings::allocator::SiteChange;
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
    bins: BinGrid,
    xs: Vec<f64>,
    zs: Vec<f64>,
    sites: SiteBuffer,
    // The portal opening's parts around the candidate's lattice in the two roadbeds.
    portals: PortalWindow,
    other_portals: PortalWindow,
    // The candidate's lattice rectangle.
    rect: [f64; 4],
}

thread_local! {
    static CANDIDATE: RefCell<CandidateScratch> = RefCell::default();
}

/// The tiles at `level` whose bake a road change from `before` to `after` can alter, sorted
/// and unique, into `out`:
/// - every tile where a changed segment may be listed by either pass of the bake's pruning:
///   pass 1 by the bake's own bin test (its cone may bind a bin's base, or it is a road seam
///   within the band), pass 2 where its lower bound may rise above the tile's floor, the lowest
///   `h + k D` over the window's seams before and after the change, the sites' pad seams
///   among them (`D` the distance from a seam's box to the lattice), which lies under every
///   first-pass ground below the base; so a removed seam counts;
/// - every tile whose lattice meets a changed piece and whose covering top triangles differ
///   between the two roadbeds (coverage and the road surface under the deep inside rule), so a
///   split trunk's far half, rebuilt bit for bit, marks nothing;
/// - every tile whose lattice lies within the larger of the portal reach and the band of a
///   changed piece with a portal row ([`TerrainChanges::portals`]) and whose portal parts
///   ([`PortalWindow`]: the rows, decks and bore seams the portal opening reads, `TERRAIN-07-C`)
///   differ between the two roadbeds. The windows hold only those pieces' parts, so an edit that
///   replaces no portal piece reads none.
///
/// A superset is safe: rebaking an unaffected tile reproduces its bits. `O(changed segments x
/// tiles within the cap of each)` plus one window per candidate tile, in parallel.
#[allow(clippy::too_many_arguments)]
pub fn dirty_tiles_for_road_changes(
    terrain: &TerrainSystem,
    before: &RoadbedSystem,
    after: &RoadbedSystem,
    sites: &impl SiteSource,
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
    let portal_reach = PORTAL_REACH_M.max(band);
    for bounds in &changes.portals {
        push_tiles(
            grown(*bounds, apron + portal_reach + PIECE_PAD_M),
            &mut scratch.candidates,
        );
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
    let (pieces, portals) = (&changes.pieces, &changes.portals);
    crate::thread_pools::install(|| {
        scratch
            .candidates
            .par_iter()
            .map(|&tile| {
                CANDIDATE.with_borrow_mut(|candidate| {
                    reached(
                        terrain, after, sites, changed, tile, level, &grid, candidate,
                    ) || covering_triangles_changed(before, after, pieces, candidate)
                        || portals_changed(before, after, portals, band, candidate)
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

/// The tiles at `level` whose bake the site changes `changes` can alter (`TERRAIN-07-C`), sorted
/// and unique, into `out`; `sites` holds the sites after the changes, `roadbed` the roads:
/// - every tile whose lattice lies within the band of a change's bounds: the `Pad` class, the
///   material class and the nearest material edge change only there;
/// - every tile where a changed site's pad seams may be listed by either pass of the bake's
///   pruning, as for a changed road segment, each change standing for its pad seams as one seam
///   spanning its bounds at its support height ([`box_seam`]): the passes' tests read a seam's
///   box and height range only, and every pad seam's box lies in the bounds, so a tile one of
///   them may change passes too.
///
/// A placement passes the new site, a removal the old one, a rebuild both. A superset is safe.
/// `O(changes x tiles within the cap of each)` plus one window per candidate tile, in
/// parallel.
#[allow(clippy::too_many_arguments)]
pub fn dirty_tiles_for_site_changes(
    terrain: &TerrainSystem,
    roadbed: &RoadbedSystem,
    sites: &impl SiteSource,
    changes: &[SiteChange],
    level: Level,
    scratch: &mut SelectScratch,
    out: &mut Vec<TileKey>,
) {
    out.clear();
    let grid = BaseGrid::of(terrain);
    let band = level.band_m(grid.cell_m);
    let apron = level.spacing_m(grid.cell_m);
    scratch.changed.clear();
    scratch.changed.extend(changes.iter().map(|change| {
        box_seam(
            change.bounds.map(f64::from),
            f64::from(change.support_height_m),
        )
    }));
    scratch.candidates.clear();
    for seam in &scratch.changed {
        push_tiles(
            grown(seam.box_xz(), CAP_M.max(band) + apron),
            &mut scratch.candidates,
        );
    }
    sort_unique(&mut scratch.candidates);
    let changed = &scratch.changed;
    crate::thread_pools::install(|| {
        scratch
            .candidates
            .par_iter()
            .map(|&tile| {
                CANDIDATE.with_borrow_mut(|candidate| {
                    lattice(terrain, tile, level, &mut candidate.xs, &mut candidate.zs);
                    let (xs, zs) = (&candidate.xs, &candidate.zs);
                    let rect = grown(
                        [xs[0], zs[0], xs[xs.len() - 1], zs[zs.len() - 1]],
                        band + PIECE_PAD_M,
                    );
                    changed.iter().any(|seam| meets(seam.box_xz(), rect))
                        || reached(terrain, roadbed, sites, changed, tile, level, &grid, candidate)
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

// Whether the portal parts the bake of the candidate's lattice (left in `candidate` by
// `reached`) reads differ between the roadbeds, where a changed portal piece (`portals`, its
// bounds) lies within the larger of the portal reach and the band of it: the opening reads
// nothing else of the roadbed beyond the coverage, the seams and the finished values, which the
// other tests follow.
fn portals_changed(
    before: &RoadbedSystem,
    after: &RoadbedSystem,
    portals: &[[f64; 4]],
    band: f64,
    candidate: &mut CandidateScratch,
) -> bool {
    let rect = candidate.rect;
    let near = grown(rect, PORTAL_REACH_M.max(band) + PIECE_PAD_M);
    if !portals.iter().any(|bounds| meets(*bounds, near)) {
        return false;
    }
    candidate
        .portals
        .gather(before, rect, band, &mut candidate.pieces);
    candidate
        .other_portals
        .gather(after, rect, band, &mut candidate.pieces);
    !candidate.portals.same(&candidate.other_portals)
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

// Whether a changed seam may change a value of the tile's lattice: the bake's pass 1 lists it
// in a bin of the tile, or its lower bound may rise above the tile's floor, the lowest
// `h_lo + k D` over the window's seams before and after the change (`roadbed`'s and the sites'
// pad seams, which hold one side of the change, and the changed ones): at a sample whose
// first-pass ground lies below the base that ground is at least the upper bound `U >= h + k d`
// of some seam there, so pass 2 lists no seam whose lower bound stays below the floor. A site
// source that cannot answer leaves the floor at `-inf`, which reaches every changed seam.
// `O(window seams + changed seams x blocks)`.
#[allow(clippy::too_many_arguments)]
fn reached(
    terrain: &TerrainSystem,
    roadbed: &RoadbedSystem,
    sites: &impl SiteSource,
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
    let lowest =
        |seam: &Seam| seam.height_range().0 + TIE_IN_SLOPE * box_distance(seam.box_xz(), rect);
    let mut floor = f64::INFINITY;
    roadbed.for_each_grounded_seam_in_bounds(window, &mut scratch.pieces, |[a, b]| {
        floor = floor.min(lowest(&road_seam(a, b)));
    });
    if sites.ready() {
        // The floor is a minimum, so the sites need no canonical order.
        scratch.sites.gather_unordered(sites, window);
        scratch
            .sites
            .for_each_pad_seam(|seam| floor = floor.min(lowest(&seam)));
    } else {
        floor = f64::NEG_INFINITY;
    }
    for seam in changed.iter().filter(in_window) {
        floor = floor.min(lowest(seam));
    }
    scratch.base.fill(terrain, window);
    scratch
        .bins
        .fill(&scratch.base, rect, bin_side(level.spacing_m(grid.cell_m)));
    let slope = scratch.base.slope();
    let (base, bins) = (&scratch.base, &scratch.bins);
    changed.iter().filter(in_window).any(|seam| {
        if box_distance(seam.box_xz(), rect) <= lift_reach(seam.height_range().1, floor) {
            return true;
        }
        let (range, _) = bins.first_range(seam, base, slope, band);
        bins.first_bins(seam, range, band, |_, _| true)
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
