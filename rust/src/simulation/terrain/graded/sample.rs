// SPDX-License-Identifier: GPL-2.0-only

//! The pointwise graded-terrain function (`TERRAIN-07`) and the base heightmap it reads.
//!
//! Everything here is `f64`: the base is the bilinear interpolation of the `f32` source samples
//! scaled to metres, and a sample's value is a function of its position, its coverage and the
//! seams it is given, which the bake prunes exactly and the queries do not prune at all.

use super::grade::{CAP_M, Cones, INSIDE_REACH_M, cone, inside, inside_deep, settled};
use super::seams::{Seam, SeamKind};
use crate::config::HEIGHT_SCALE;
use crate::simulation::terrain::TerrainSystem;

/// No seam index.
pub(super) const NO_SEAM: u32 = u32::MAX;

/// The class of a point, in the spec's order: a road top triangle covers it, or it is ground.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Class {
    Road,
    Ground,
}

/// One evaluated point.
#[derive(Clone, Copy, Debug)]
pub(super) struct Value {
    // Graded height `g`, metres.
    pub(super) height: f64,
    // Distance to the nearest road seam clamped to the band, negative under a road.
    pub(super) sdf: f64,
    // The nearest road seam within the band (an index into the seams given), else `NO_SEAM`.
    pub(super) nearest: u32,
    // Whether the cones conflicted (`L > U`) at a ground point.
    pub(super) conflict: bool,
    // The cone bounds over the seams evaluated.
    pub(super) cones: Cones,
    // Distance to the nearest road seam within the band, `+inf` without one.
    pub(super) nearest_distance: f64,
    // Candidates evaluated before the early exit.
    pub(super) evaluated: u32,
}

/// One candidate seam of a sample: its index, a lower bound of its distance from the sample,
/// and the lowest and highest heights of it and every candidate after it, whose distance
/// bounds are at least its own. The evaluation stops at the first candidate past which no seam
/// can change the value or the nearest seam (`settled`); with distance bound `0` it never
/// stops.
#[derive(Clone, Copy, Debug)]
pub(super) struct Listed {
    pub(super) index: u32,
    pub(super) distance: f64,
    pub(super) heights: (f64, f64),
}

impl Listed {
    /// A candidate without bounds, for evaluations over every seam.
    pub(super) fn unbounded(index: u32) -> Self {
        Self {
            index,
            distance: 0.0,
            heights: (f64::NEG_INFINITY, f64::INFINITY),
        }
    }
}

/// Evaluates the graded terrain at the world point `(x, z)` with base `base`, `road` the
/// highest covering road top triangle's height (`None` uncovered), from the candidate seams
/// (into `seams`), until no later candidate can change the value or the nearest seam. Seams
/// beyond the cap are left out. Only road seams name the nearest seam, within `band` metres.
/// The value and the nearest-seam order are order-free, so the result does not depend on the
/// candidates' order. One lane of [`evaluate_lanes`]. `O(candidates)`.
pub(super) fn evaluate(
    x: f64,
    z: f64,
    base: f64,
    road: Option<f32>,
    seams: &[Seam],
    candidates: impl Iterator<Item = Listed>,
    band: f64,
) -> Value {
    let [value] = evaluate_lanes(&[x], z, &[base], &[road], seams, candidates, band);
    value
}

/// [`evaluate`] at `N` points `(xs[lane], z)` of one lattice row at once, with bases `bases`
/// and covering road heights `roads`, over one candidate list. Each lane takes the candidates
/// until its own early exit, and later ones no longer touch it, so every lane equals
/// `evaluate` at its point bitwise, its cones and count included. Each candidate's seam is
/// evaluated for all lanes at once ([`Seam::at_lanes`]), with the cone bounds updated by
/// selects, so the arithmetic compiles to vector instructions. `O(candidates x N)`.
pub(super) fn evaluate_lanes<const N: usize>(
    xs: &[f64; N],
    z: f64,
    bases: &[f64; N],
    roads: &[Option<f32>; N],
    seams: &[Seam],
    candidates: impl Iterator<Item = Listed>,
    band: f64,
) -> [Value; N] {
    let (mut lower, mut upper) = ([Cones::NONE.lower; N], [Cones::NONE.upper; N]);
    // Per lane the nearest road seam within the band: index, distance and height.
    let mut nearest = [None::<(u32, f64, f64)>; N];
    let (mut live, mut evaluated) = ([true; N], [0u32; N]);
    // A ground lane's base; a covered lane's value reads only the nearest road seam.
    let ground: [Option<f64>; N] =
        std::array::from_fn(|lane| roads[lane].is_none().then_some(bases[lane]));
    for Listed {
        index,
        distance,
        heights,
    } in candidates
    {
        let mut any = false;
        for lane in 0..N {
            if !live[lane] {
                continue;
            }
            let cones = Cones {
                lower: lower[lane],
                upper: upper[lane],
            };
            let nearest_distance = nearest[lane].map_or(f64::INFINITY, |(_, d, _)| d);
            live[lane] = !settled(
                &cones,
                ground[lane],
                heights,
                distance,
                nearest_distance,
                band,
            );
            evaluated[lane] += u32::from(live[lane]);
            any |= live[lane];
        }
        if !any {
            break;
        }
        let seam = &seams[index as usize];
        let (distance, height) = seam.at_lanes(xs, z);
        for lane in 0..N {
            let take = live[lane] && distance[lane] <= CAP_M;
            let (cone_lower, cone_upper) = cone(height[lane], distance[lane]);
            lower[lane] = if take {
                lower[lane].max(cone_lower)
            } else {
                lower[lane]
            };
            upper[lane] = if take {
                upper[lane].min(cone_upper)
            } else {
                upper[lane]
            };
        }
        if seam.kind != SeamKind::Road {
            continue;
        }
        for lane in 0..N {
            if live[lane]
                && distance[lane] <= CAP_M
                && distance[lane] < band
                && nearest[lane].is_none_or(|(best, best_distance, _)| {
                    nearer(
                        seams,
                        (xs[lane], z),
                        (index, distance[lane]),
                        (best, best_distance),
                    )
                })
            {
                nearest[lane] = Some((index, distance[lane], height[lane]));
            }
        }
    }
    std::array::from_fn(|lane| {
        let cones = Cones {
            lower: lower[lane],
            upper: upper[lane],
        };
        value(
            bases[lane],
            roads[lane],
            cones,
            nearest[lane],
            evaluated[lane],
            band,
        )
    })
}

// The nearest-seam order ([`SeamAt::nearer_than`]) between a candidate and the best so far, as
// `(index, distance)`: by distance, and on a tie by the line distances from the scalar seam
// evaluation, whose distances are the same bits.
fn nearer(
    seams: &[Seam],
    (x, z): (f64, f64),
    (index, distance): (u32, f64),
    (best, best_distance): (u32, f64),
) -> bool {
    if distance != best_distance {
        return distance < best_distance;
    }
    let at = seams[index as usize].at(x, z);
    at.nearer_than(index, &seams[best as usize].at(x, z), best)
}

// A sample's value from its evaluated cones and nearest road seam within the band (`(index,
// distance, height)`): the inside rule where a road covers it, the overlap rule elsewhere.
fn value(
    base: f64,
    road: Option<f32>,
    cones: Cones,
    nearest: Option<(u32, f64, f64)>,
    evaluated: u32,
    band: f64,
) -> Value {
    let class = if road.is_some() {
        Class::Road
    } else {
        Class::Ground
    };
    let height = match (class, road) {
        (Class::Road, Some(road)) => match nearest.filter(|&(_, d, _)| d < INSIDE_REACH_M) {
            Some((_, d, h)) => inside(base, h, d),
            None => inside_deep(base, f64::from(road)),
        },
        _ => cones.ground(base),
    };
    let distance = nearest.map_or(band, |(_, d, _)| d);
    Value {
        height,
        sdf: if class == Class::Road {
            -distance
        } else {
            distance
        },
        nearest: nearest.map_or(NO_SEAM, |(index, ..)| index),
        conflict: class == Class::Ground && cones.lower > cones.upper,
        cones,
        nearest_distance: nearest.map_or(f64::INFINITY, |(_, d, _)| d),
        evaluated,
    }
}

/// The base heightmap's lattice: where its samples lie in world metres.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct BaseGrid {
    pub(super) half: [f64; 2],
    pub(super) cell_m: f64,
    size: [usize; 2],
}

impl BaseGrid {
    pub(super) fn of(terrain: &TerrainSystem) -> Self {
        let cell_m = f64::from(terrain.cell_size_m());
        let size = [terrain.width, terrain.height];
        Self {
            half: size.map(|n| n.saturating_sub(1) as f64 * cell_m * 0.5),
            cell_m,
            size,
        }
    }

    // Clamped fractional grid coordinate of a world coordinate on `axis`.
    fn grid(&self, axis: usize, world_m: f64) -> f64 {
        ((world_m + self.half[axis]) / self.cell_m).clamp(0.0, (self.size[axis] - 1) as f64)
    }

    // Sample index range `[first, last]` on `axis` whose cells meet `[min_m, max_m]`.
    fn range(&self, axis: usize, min_m: f64, max_m: f64) -> (usize, usize) {
        let first = self.grid(axis, min_m).floor() as usize;
        let last = (self.grid(axis, max_m).ceil() as usize + 1).min(self.size[axis] - 1);
        (first, last)
    }
}

/// A world coordinate's place on one base axis: the samples on either side and the fractions
/// towards each.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct AxisStop {
    first: usize,
    second: usize,
    fraction: f64,
    rest: f64,
}

/// The base samples, in metres, of the cells under a window: what one bake or query reads of
/// the heightmap, copied once so a sample costs no chunk lookup. Reused between windows.
#[derive(Clone, Debug, Default)]
pub(super) struct BaseWindow {
    grid: BaseGrid,
    first: [usize; 2],
    count: [usize; 2],
    metres: Vec<f64>,
}

impl BaseWindow {
    /// Copies the samples of the cells meeting `rect = [min_x, min_z, max_x, max_z]`.
    /// `O(cells)`.
    pub(super) fn fill(&mut self, terrain: &TerrainSystem, rect: [f64; 4]) {
        self.grid = BaseGrid::of(terrain);
        let (x0, x1) = self.grid.range(0, rect[0], rect[2]);
        let (z0, z1) = self.grid.range(1, rect[1], rect[3]);
        self.first = [x0, z0];
        self.count = [x1 - x0 + 1, z1 - z0 + 1];
        self.metres.clear();
        let scale = f64::from(HEIGHT_SCALE);
        for gz in z0..=z1 {
            for gx in x0..=x1 {
                self.metres
                    .push(f64::from(terrain.get_height(gx, gz)) * scale);
            }
        }
    }

    pub(super) fn grid(&self) -> &BaseGrid {
        &self.grid
    }

    fn sample(&self, gx: usize, gz: usize) -> f64 {
        debug_assert!(gx >= self.first[0] && gx - self.first[0] < self.count[0]);
        debug_assert!(gz >= self.first[1] && gz - self.first[1] < self.count[1]);
        self.metres[(gz - self.first[1]) * self.count[0] + gx - self.first[0]]
    }

    /// The base `b(p)` at a world point inside the window: bilinear in `f64`, exactly the
    /// sample on a lattice point.
    pub(super) fn at(&self, x: f64, z: f64) -> f64 {
        self.at_stops(&self.stop(0, x), &self.stop(1, z))
    }

    /// Where a world coordinate on `axis` lies between base samples, for [`Self::at_stops`]: a
    /// lattice reuses one per column and one per row.
    pub(super) fn stop(&self, axis: usize, value: f64) -> AxisStop {
        let g = self.grid.grid(axis, value);
        let first = g.floor() as usize;
        let fraction = g - first as f64;
        AxisStop {
            first,
            second: (first + 1).min(self.grid.size[axis] - 1),
            fraction,
            rest: 1.0 - fraction,
        }
    }

    /// [`Self::at`] from the point's stops: the same operations, the same bits.
    pub(super) fn at_stops(&self, x: &AxisStop, z: &AxisStop) -> f64 {
        let h0 =
            self.sample(x.first, z.first) * x.rest + self.sample(x.second, z.first) * x.fraction;
        let h1 =
            self.sample(x.first, z.second) * x.rest + self.sample(x.second, z.second) * x.fraction;
        h0 * z.rest + h1 * z.fraction
    }

    /// Lowest and highest base over the cells meeting `rect` inside the window, which bound the
    /// bilinear base there: a seam reaching beyond the window is bounded over its part inside,
    /// where its nearest points to the window's lattice lie. `O(cells)`.
    pub(super) fn range(&self, rect: [f64; 4]) -> (f64, f64) {
        let clip = |axis: usize, (first, last): (usize, usize)| {
            let end = self.first[axis] + self.count[axis] - 1;
            (
                first.clamp(self.first[axis], end),
                last.clamp(self.first[axis], end),
            )
        };
        let (x0, x1) = clip(0, self.grid.range(0, rect[0], rect[2]));
        let (z0, z1) = clip(1, self.grid.range(1, rect[1], rect[3]));
        let mut range = (f64::INFINITY, f64::NEG_INFINITY);
        for gz in z0..=z1 {
            for gx in x0..=x1 {
                let h = self.sample(gx, gz);
                range = (range.0.min(h), range.1.max(h));
            }
        }
        range
    }

    /// The exact range of the bilinear base over `rect` inside the window. On each piece of
    /// `rect` within one cell the base is linear along either axis, so it is extreme at a corner
    /// of the piece: the range is over the corners of `rect`, the crossings of its sides with the
    /// cell lines and the lattice points inside it. `O(points)`: `(side / cell + 2)^2` at most,
    /// 9 for a 4 m bin on cells of 2 m or more.
    pub(super) fn bilinear_range(&self, rect: [f64; 4]) -> (f64, f64) {
        let grid = &self.grid;
        let stops = |axis: usize, lo: f64, hi: f64| {
            let first = grid.grid(axis, lo).floor() as i64 + 1;
            let last = grid.grid(axis, hi).ceil() as i64 - 1;
            std::iter::once(lo)
                .chain((first..=last).map(move |i| i as f64 * grid.cell_m - grid.half[axis]))
                .chain(std::iter::once(hi))
        };
        let mut range = (f64::INFINITY, f64::NEG_INFINITY);
        for z in stops(1, rect[1], rect[3]) {
            for x in stops(0, rect[0], rect[2]) {
                let h = self.at(x, z);
                range = (range.0.min(h), range.1.max(h));
            }
        }
        range
    }

    /// A Lipschitz bound of the bilinear base over the whole window: the largest cell gradient
    /// bound `|(max |dx|, max |dz|)| / cell`. Beyond the world edge the base is constant.
    /// `O(cells)`.
    pub(super) fn slope(&self) -> f64 {
        let mut slope: f64 = 0.0;
        for j in 0..self.count[1].saturating_sub(1) {
            for i in 0..self.count[0].saturating_sub(1) {
                let (gx, gz) = (self.first[0] + i, self.first[1] + j);
                let [h00, h10, h01, h11] = [(gx, gz), (gx + 1, gz), (gx, gz + 1), (gx + 1, gz + 1)]
                    .map(|(x, z)| self.sample(x, z));
                let dx = (h10 - h00).abs().max((h11 - h01).abs());
                let dz = (h01 - h00).abs().max((h11 - h10).abs());
                slope = slope.max((dx * dx + dz * dz).sqrt() / self.grid.cell_m);
            }
        }
        slope
    }
}
