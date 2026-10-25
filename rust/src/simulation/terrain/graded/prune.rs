// SPDX-License-Identifier: GPL-2.0-only

//! The two-pass exact pruning of the tile bake (`TERRAIN-07`, `T18` (a)): the seams each bin of
//! a tile's lattice must evaluate, bins of `max(4 m, the level's spacing)`.
//!
//! Pass 1 lists in a bin the seams whose cone can bind the base there, over the bin's exact base
//! range, and the seams within the band. Every other seam's cone holds the base at the bin's
//! samples, so a sample whose first-pass ground is at least its base is final. A sample left
//! below its base (a cut's upper bound binds) can still be lifted by such a seam's lower bound:
//! pass 2 lists, in each bin holding such samples, the seams whose lower bound can rise above
//! the lowest of them. Dirty tile selection asks the same bins.

use super::grade::{
    CAP_M, Cones, ReachBounds, band_reach, base_reach, bind_reach, lift_reach, may_bind, may_lift,
};
use super::sample::{BaseWindow, Listed};
use super::seams::Seam;
use super::tile::{BakeStats, grown, meets};

// Smallest seam bin side over a tile's lattice, metres. A level whose spacing is wider bins at
// its spacing: its band and reaches grow with the spacing, so finer bins would each list nearly
// the same seams for one or two lattice points (`GL3`).
const BIN_M: f64 = 4.0;

/// The bin side at a level of lattice spacing `spacing_m`: `max(4 m, spacing)`. The bake and
/// the selection bin alike.
pub(super) fn bin_side(spacing_m: f64) -> f64 {
    spacing_m.max(BIN_M)
}
// Bins per side of a block; a block is tested before its bins.
const BLOCK: u32 = 4;

/// The seam bins over a lattice rectangle, rows along +z, with the exact base range of each bin
/// and of each block of 4 x 4 bins. Reused between tiles.
#[derive(Clone, Debug, Default)]
pub(super) struct BinGrid {
    rect: [f64; 4],
    side: f64,
    count: [u32; 2],
    blocks: [u32; 2],
    base: Vec<(f64, f64)>,
    block_base: Vec<(f64, f64)>,
    lattice_base: (f64, f64),
}

impl BinGrid {
    /// Bins of side `side` ([`bin_side`]) over the lattice rectangle `rect` with the base
    /// ranges from `base`, which holds the rectangle. `O(bins)`.
    pub(super) fn fill(&mut self, base: &BaseWindow, rect: [f64; 4], side: f64) {
        self.rect = rect;
        self.side = side;
        self.count =
            [0, 1].map(|axis| (((rect[axis + 2] - rect[axis]) / side).ceil() as u32).max(1));
        self.blocks = self.count.map(|n| n.div_ceil(BLOCK));
        self.base.clear();
        for bz in 0..self.count[1] {
            for bx in 0..self.count[0] {
                self.base.push(base.bilinear_range(self.bounds(bx, bz)));
            }
        }
        self.block_base.clear();
        self.block_base.resize(
            (self.blocks[0] * self.blocks[1]) as usize,
            (f64::INFINITY, f64::NEG_INFINITY),
        );
        let mut all = (f64::INFINITY, f64::NEG_INFINITY);
        for bz in 0..self.count[1] {
            for bx in 0..self.count[0] {
                let range = self.base[self.index(bx, bz)];
                let block =
                    &mut self.block_base[((bz / BLOCK) * self.blocks[0] + bx / BLOCK) as usize];
                *block = (block.0.min(range.0), block.1.max(range.1));
                all = (all.0.min(range.0), all.1.max(range.1));
            }
        }
        self.lattice_base = all;
    }

    pub(super) fn len(&self) -> usize {
        (self.count[0] * self.count[1]) as usize
    }

    pub(super) fn index(&self, bx: u32, bz: u32) -> usize {
        (bz * self.count[0] + bx) as usize
    }

    /// The bin column or row (`axis` 0 or 1) holding a coordinate, clamped to the rectangle.
    pub(super) fn of(&self, axis: usize, value: f64) -> u32 {
        (((value - self.rect[axis]) / self.side).floor().max(0.0) as u32).min(self.count[axis] - 1)
    }

    // A bin's rectangle; the last bin of an axis may reach beyond the lattice.
    fn bounds(&self, bx: u32, bz: u32) -> [f64; 4] {
        let x = self.rect[0] + f64::from(bx) * self.side;
        let z = self.rect[1] + f64::from(bz) * self.side;
        [x, z, x + self.side, z + self.side]
    }

    // The rectangle of the block holding bin `(bx, bz)`.
    fn block_bounds(&self, bx: u32, bz: u32) -> [f64; 4] {
        let side = self.side * f64::from(BLOCK);
        let x = self.rect[0] + f64::from(bx / BLOCK) * side;
        let z = self.rect[1] + f64::from(bz / BLOCK) * side;
        [x, z, x + side, z + side]
    }

    // The bins `[x0, x1, z0, z1]` meeting `area`, empty as `[1, 0, 1, 0]` when it misses the
    // rectangle.
    fn range(&self, area: [f64; 4]) -> [u32; 4] {
        if !meets(area, self.rect) {
            return [1, 0, 1, 0];
        }
        [
            self.of(0, area[0]),
            self.of(0, area[2]),
            self.of(1, area[1]),
            self.of(1, area[3]),
        ]
    }

    /// The bins pass 1 visits for `seam`: its box grown by the smaller of its Lipschitz base
    /// reach over the window (`slope`, `base`) and its bind reach over the lattice's base range,
    /// and at least the band. No bin outside lists the seam under the one-pass rule either.
    /// Returns the range and the reach. `O(cells under the seam)`.
    pub(super) fn first_range(
        &self,
        seam: &Seam,
        base: &BaseWindow,
        slope: f64,
        band: f64,
    ) -> ([u32; 4], f64) {
        let (heights, area) = (seam.height_range(), seam.box_xz());
        let reach = base_reach(&ReachBounds {
            seam: heights,
            base_under: base.range(area),
            slope,
        })
        .min(bind_reach(heights, self.lattice_base));
        (self.range(grown(area, reach.max(band_reach(band)))), reach)
    }

    /// Whether pass 1 lists `seam` (visited over `range`) in bin `(bx, bz)`, `distance` from its
    /// box: its cone may bind the bin's base, or it lies within the band. `O(1)`.
    fn first_lists(
        &self,
        seam: &Seam,
        range: [u32; 4],
        (bx, bz): (u32, u32),
        distance: f64,
        band: f64,
    ) -> bool {
        (range[0]..=range[1]).contains(&bx)
            && (range[2]..=range[3]).contains(&bz)
            && may_bind(
                seam.height_range(),
                self.base[self.index(bx, bz)],
                distance,
                band,
            )
    }

    /// Visits the bins of `range` where pass 1 lists `seam` (with the distance from its box), in
    /// row order, until `visit` returns true; returns whether it did. A block whose base range and distance fail the test
    /// skips its bins, which would fail it too (`may_bind` only loosens with a wider range and
    /// a smaller distance). `O(blocks in range + bins of the blocks that pass)`.
    pub(super) fn first_bins(
        &self,
        seam: &Seam,
        range: [u32; 4],
        band: f64,
        mut visit: impl FnMut(usize, f64) -> bool,
    ) -> bool {
        if range[0] > range[1] {
            return false;
        }
        let (heights, area) = (seam.height_range(), seam.box_xz());
        for cz in range[2] / BLOCK..=range[3] / BLOCK {
            for cx in range[0] / BLOCK..=range[1] / BLOCK {
                let block = self.block_base[(cz * self.blocks[0] + cx) as usize];
                let corner = (cx * BLOCK, cz * BLOCK);
                let distance = box_distance(area, self.block_bounds(corner.0, corner.1));
                if !may_bind(heights, block, distance, band) {
                    continue;
                }
                for bz in (cz * BLOCK).max(range[2])..=(cz * BLOCK + BLOCK - 1).min(range[3]) {
                    for bx in (cx * BLOCK).max(range[0])..=(cx * BLOCK + BLOCK - 1).min(range[1]) {
                        let distance = box_distance(area, self.bounds(bx, bz));
                        if self.first_lists(seam, range, (bx, bz), distance, band)
                            && visit(self.index(bx, bz), distance)
                        {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
}

// Distance between two XZ boxes, `0` where they meet: at most the distance between any two of
// their points.
pub(super) fn box_distance(a: [f64; 4], b: [f64; 4]) -> f64 {
    let dx = (a[0] - b[2]).max(b[0] - a[2]).max(0.0);
    let dz = (a[1] - b[3]).max(b[1] - a[3]).max(0.0);
    (dx * dx + dz * dz).sqrt()
}

/// Seam lists per bin (CSR): each bin's candidates by ascending distance from the bin (then
/// window order), with the height bounds of each candidate and those after it, for the
/// sample's early exit.
#[derive(Default)]
pub(super) struct BinLists {
    starts: Vec<u32>,
    items: Vec<Listed>,
    cursor: Vec<u32>,
}

impl BinLists {
    /// From `(bin, candidate)` pairs, each candidate holding its own seam's heights, by a
    /// counting sort, then each bin's candidates by distance with the suffix height bounds.
    /// `O(pairs log (pairs per bin) + bins)`.
    pub(super) fn build(&mut self, bins: usize, pairs: &[(u32, Listed)]) {
        self.starts.clear();
        self.starts.resize(bins + 1, 0);
        for &(bin, _) in pairs {
            self.starts[bin as usize + 1] += 1;
        }
        for bin in 0..bins {
            self.starts[bin + 1] += self.starts[bin];
        }
        self.cursor.clear();
        self.cursor.extend_from_slice(&self.starts[..bins]);
        self.items.clear();
        self.items.resize(pairs.len(), Listed::unbounded(0));
        for &(bin, listed) in pairs {
            let cursor = &mut self.cursor[bin as usize];
            self.items[*cursor as usize] = listed;
            *cursor += 1;
        }
        for bin in 0..bins {
            let list = &mut self.items[self.starts[bin] as usize..self.starts[bin + 1] as usize];
            list.sort_unstable_by(|a, b| {
                a.distance
                    .total_cmp(&b.distance)
                    .then(a.index.cmp(&b.index))
            });
            let mut heights = (f64::INFINITY, f64::NEG_INFINITY);
            for listed in list.iter_mut().rev() {
                heights = (
                    heights.0.min(listed.heights.0),
                    heights.1.max(listed.heights.1),
                );
                listed.heights = heights;
            }
        }
    }

    pub(super) fn of(&self, bin: usize) -> &[Listed] {
        &self.items[self.starts[bin] as usize..self.starts[bin + 1] as usize]
    }
}

/// A ground sample whose first-pass ground lies below its base (a cut's upper bound holds it
/// there): what pass 2 needs to finish it.
#[derive(Clone, Copy)]
pub(super) struct Lifted {
    pub(super) sample: u32,
    pub(super) bin: u32,
    pub(super) x: f64,
    pub(super) z: f64,
    pub(super) base: f64,
    pub(super) cones: Cones,
    // Within one lattice spacing of a road seam, and within the band.
    pub(super) near_seam: bool,
    pub(super) in_band: bool,
}

/// Pass 1: every bin lists the seams `BinGrid::first_bins` names, as `(bin, seam)` pairs in seam
/// order; each seam's visited range goes to `ranges`. Unpruned, every bin lists every seam.
/// `O(seams x blocks in each range + bins of the blocks that pass)`.
#[allow(clippy::too_many_arguments)]
pub(super) fn list_first_pass(
    seams: &[Seam],
    bins: &BinGrid,
    base: &BaseWindow,
    band: f64,
    prune: bool,
    ranges: &mut Vec<[u32; 4]>,
    pairs: &mut Vec<(u32, Listed)>,
    stats: &mut BakeStats,
) {
    let slope = if prune { base.slope() } else { 0.0 };
    ranges.clear();
    pairs.clear();
    for (index, seam) in seams.iter().enumerate() {
        let listed = |distance: f64| Listed {
            index: index as u32,
            distance,
            heights: seam.height_range(),
        };
        if !prune {
            // The reference: every seam everywhere, with no distance bound to stop at.
            let all = [0, bins.count[0] - 1, 0, bins.count[1] - 1];
            ranges.push(all);
            pairs.extend((0..bins.len() as u32).map(|bin| (bin, listed(0.0))));
            continue;
        }
        let (range, reach) = bins.first_range(seam, base, slope, band);
        stats.reach_sum_m += reach;
        stats.reach_max_m = stats.reach_max_m.max(reach);
        ranges.push(range);
        bins.first_bins(seam, range, band, |bin, distance| {
            pairs.push((bin as u32, listed(distance)));
            false
        });
    }
}

/// Pass 2: each bin holding samples whose first-pass ground lies below the base lists the seams
/// pass 1 did not list there whose lower bound may rise above the lowest such ground, `floor`
/// (`may_lift`), visiting each seam's box grown by its lift reach over the lowest floor.
/// `O(seams x bins in each box)`.
pub(super) fn list_second_pass(
    seams: &[Seam],
    bins: &BinGrid,
    band: f64,
    floor: &[f64],
    ranges: &[[u32; 4]],
    pairs: &mut Vec<(u32, Listed)>,
) {
    let lowest = floor.iter().copied().fold(f64::INFINITY, f64::min);
    pairs.clear();
    for (index, seam) in seams.iter().enumerate() {
        let (heights, area) = (seam.height_range(), seam.box_xz());
        let reach = lift_reach(heights.1, lowest);
        if reach < 0.0 {
            continue;
        }
        let range = bins.range(grown(area, reach));
        if range[0] > range[1] {
            continue;
        }
        for bz in range[2]..=range[3] {
            for bx in range[0]..=range[1] {
                let bin = bins.index(bx, bz);
                if floor[bin] == f64::INFINITY {
                    continue;
                }
                let distance = box_distance(area, bins.bounds(bx, bz));
                if may_lift(heights.1, floor[bin], distance)
                    && !bins.first_lists(seam, ranges[index], (bx, bz), distance, band)
                {
                    pairs.push((
                        bin as u32,
                        Listed {
                            index: index as u32,
                            distance,
                            heights,
                        },
                    ));
                }
            }
        }
    }
}

/// Finishes the samples pass 1 left below their base: the first pass's cones plus pass 2's
/// seams within the cap. Every seam left out has a lower bound below the first-pass ground and
/// an upper bound above the base, so `g = max(L, min(b, U))` equals its value over every seam
/// within the cap, bitwise. Writes their deltas; returns whether any is graded.
/// `O(samples x their lists)`.
pub(super) fn finish_second_pass(
    seams: &[Seam],
    lists: &BinLists,
    lifted: &[Lifted],
    texels: &mut [[f32; 4]],
    stats: &mut BakeStats,
) -> bool {
    let mut graded = false;
    for sample in lifted {
        let mut cones = sample.cones;
        let list = lists.of(sample.bin as usize);
        stats.second_pass_candidates += list.len() as u64;
        for listed in list {
            let at = seams[listed.index as usize].at(sample.x, sample.z);
            if at.distance <= CAP_M {
                cones.add(at.height, at.distance);
            }
        }
        let delta = (cones.ground(sample.base) - sample.base) as f32;
        texels[sample.sample as usize][0] = delta;
        graded |= delta != 0.0 || sample.in_band;
        if cones.lower > cones.upper {
            stats.conflicts += 1;
            stats.seam_conflicts += u32::from(sample.near_seam);
        }
    }
    stats.second_pass_samples = lifted.len() as u32;
    stats.candidates += stats.second_pass_candidates;
    graded
}
