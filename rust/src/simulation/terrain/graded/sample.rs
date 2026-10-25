// SPDX-License-Identifier: GPL-2.0-only

//! The pointwise graded-terrain function (`TERRAIN-07`) and the base heightmap it reads.
//!
//! Everything here is `f64`: the base is the bilinear interpolation of the `f32` source samples
//! scaled to metres, and a sample's value is a function of its position, its coverage and the
//! seams it is given, which the bake prunes exactly and the queries do not prune at all.

use super::grade::{CAP_M, Cones, INSIDE_REACH_M, inside, inside_deep};
use super::seams::{Seam, SeamAt, SeamKind};
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
    // Distance to the nearest road seam within the band, `+inf` without one.
    pub(super) nearest_distance: f64,
}

/// Evaluates the graded terrain at the world point `(x, z)` with base `base`, `road` the
/// highest covering road top triangle's height (`None` uncovered), from the candidate seams
/// (indices into `seams`, ascending: window order is key order). Seams beyond the cap are left
/// out here. Only road seams name the nearest seam, within `band` metres. `O(candidates)`.
pub(super) fn evaluate(
    x: f64,
    z: f64,
    base: f64,
    road: Option<f32>,
    seams: &[Seam],
    candidates: impl Iterator<Item = u32>,
    band: f64,
) -> Value {
    let mut cones = Cones::NONE;
    let mut nearest: Option<(u32, SeamAt)> = None;
    for index in candidates {
        let seam = &seams[index as usize];
        let at = seam.at(x, z);
        if at.distance > CAP_M {
            continue;
        }
        cones.add(at.height, at.distance);
        if seam.kind == SeamKind::Road
            && at.distance < band
            && nearest.is_none_or(|(best, best_at)| at.nearer_than(index, &best_at, best))
        {
            nearest = Some((index, at));
        }
    }
    let class = if road.is_some() {
        Class::Road
    } else {
        Class::Ground
    };
    let height = match (class, road) {
        (Class::Road, Some(road)) => match nearest.filter(|(_, at)| at.distance < INSIDE_REACH_M) {
            Some((_, at)) => inside(base, at.height, at.distance),
            None => inside_deep(base, f64::from(road)),
        },
        _ => cones.ground(base),
    };
    let distance = nearest.map_or(band, |(_, at)| at.distance);
    Value {
        height,
        sdf: if class == Class::Road {
            -distance
        } else {
            distance
        },
        nearest: nearest.map_or(NO_SEAM, |(index, _)| index),
        conflict: class == Class::Ground && cones.lower > cones.upper,
        nearest_distance: nearest.map_or(f64::INFINITY, |(_, at)| at.distance),
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
        let (gx, gz) = (self.grid.grid(0, x), self.grid.grid(1, z));
        let (x0, z0) = (gx.floor() as usize, gz.floor() as usize);
        let (fx, fz) = (gx - x0 as f64, gz - z0 as f64);
        let x1 = (x0 + 1).min(self.grid.size[0] - 1);
        let z1 = (z0 + 1).min(self.grid.size[1] - 1);
        let h0 = self.sample(x0, z0) * (1.0 - fx) + self.sample(x1, z0) * fx;
        let h1 = self.sample(x0, z1) * (1.0 - fx) + self.sample(x1, z1) * fx;
        h0 * (1.0 - fz) + h1 * fz
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
