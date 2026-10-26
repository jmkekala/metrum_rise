// SPDX-License-Identifier: GPL-2.0-only

//! Point and ray queries of the graded surface (`TERRAIN-07`), for roads and building sites; no
//! consumer reads them until `TERRAIN-07-F`. Every query evaluates the same pointwise function as
//! the bake, over every seam within the cap of the points it reads (no pruning, so no reach to
//! compute), and answers from the `GL0` lattice the surface renders with: a point between lattice points lies
//! on the `GL0` triangle under it, each lattice cell split along its `(i, j)`-`(i + 1, j + 1)`
//! diagonal. Allocation-free once the scratch is warm.

use super::coverage::covered_height_at;
use super::grade::{CAP_M, FILL_ARM_MAX_M, TIE_IN_SLOPE};
use super::portal::{PortalWindow, cuts};
use super::sample::{BaseGrid, BaseWindow, Listed, evaluate};
use super::seams::{Seam, SeamScratch, gather_road_seams};
use super::sites::{NO_SITE, SiteBuffer, SiteSource};
use super::tile::{Level, TILE_M, grown};
use crate::assets::SiteSurfaceMaterial;
use crate::simulation::network::roadbed::{PieceId, RoadbedSystem};
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::Vector3;

// `GL0` lattice spacing, metres.
const GL0_M: f64 = 0.5;
// Window growth beyond the cap: gathering more seams changes no value.
const WINDOW_PAD_M: f64 = 1.0;

/// What a point of the surface is, for consumers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GroundClass {
    /// Ungraded base terrain.
    Natural,
    /// Ground whose graded height differs from the base (`delta != 0`) outside footprints.
    Graded,
    /// Covered by a grounded road top triangle, or cut by a tunnel's portal opening
    /// (`TERRAIN-07-C`, `docs/terrain.md` T22 5 (a)): the terrain is discarded there.
    Road,
    /// Inside a building site's support footprint (the allocator's predicate), paved or not:
    /// the terrain is the pad, at its support height.
    Pad,
    /// Outside every footprint, on a site surface (the frontage paving and aprons): the first
    /// surface holding the point, in canonical site order and authored order.
    Paved(SiteSurfaceMaterial),
}

/// Reused buffers of the queries.
#[derive(Default)]
pub struct QueryScratch {
    seams: SeamScratch,
    window: Vec<Seam>,
    base: BaseWindow,
    pieces: Vec<PieceId>,
    sites: SiteBuffer,
    portals: PortalWindow,
}

/// The graded surface over a base heightmap, a roadbed and building sites: the read API every
/// consumer moves to. Point queries cost one gather of the seams and sites within the cap of the
/// point, then `O(S)` per lattice point read (three for a height, twelve for a normal).
#[derive(Clone, Copy)]
pub struct GradedSurface<'a> {
    /// The base heightmap (its source samples).
    pub terrain: &'a TerrainSystem,
    /// The roadbed whose grounded outer chains and top triangles grade the terrain.
    pub roadbed: &'a RoadbedSystem,
    /// The building sites whose pads and surfaces grade and paint it; ready
    /// ([`SiteSource::ready`]) while the surface is queried.
    pub sites: &'a dyn SiteSource,
}

impl GradedSurface<'_> {
    /// The authored base `b(p)`, metres.
    pub fn base_height_at(&self, x: f64, z: f64, scratch: &mut QueryScratch) -> f64 {
        scratch.base.fill(self.terrain, [x, z, x, z]);
        scratch.base.at(x, z)
    }

    /// The graded ground the terrain renders: `GL0` values on the `GL0` triangle under the
    /// point; in a pad its support height. Under a road it is the hidden inside-rule surface,
    /// so callers test [`Self::mask_at`] first.
    pub fn ground_height_at(&self, x: f64, z: f64, scratch: &mut QueryScratch) -> f64 {
        let cell = Cell::at(x, z);
        self.prepare(cell.rect(0), scratch);
        let [w0, w1, w2] = cell.weights();
        let [v0, v1, v2] = cell.vertices();
        w0 * self.vertex(v0, scratch)
            + w1 * self.vertex(v1, scratch)
            + w2 * self.vertex(v2, scratch)
    }

    /// The visible surface: the road's height where a road covers the point, else the graded
    /// ground.
    pub fn surface_height_at(&self, x: f64, z: f64, scratch: &mut QueryScratch) -> f64 {
        match self.roadbed.height_at(x as f32, z as f32) {
            Some(height) => f64::from(height),
            None => self.ground_height_at(x, z, scratch),
        }
    }

    /// The rendered `GL0` normal: per lattice point the central difference of the graded
    /// height over its four neighbours, interpolated over the `GL0` triangle under the point and
    /// normalized.
    pub fn normal_at(&self, x: f64, z: f64, scratch: &mut QueryScratch) -> [f64; 3] {
        let cell = Cell::at(x, z);
        self.prepare(cell.rect(1), scratch);
        let mut normal = [0.0; 3];
        for (weight, [i, j]) in cell.weights().into_iter().zip(cell.vertices()) {
            let dx = self.vertex([i + 1, j], scratch) - self.vertex([i - 1, j], scratch);
            let dz = self.vertex([i, j + 1], scratch) - self.vertex([i, j - 1], scratch);
            let vertex = normalized([-dx / (2.0 * GL0_M), 1.0, -dz / (2.0 * GL0_M)]);
            for axis in 0..3 {
                normal[axis] += weight * vertex[axis];
            }
        }
        normalized(normal)
    }

    /// What the point is, in this order: road where a grounded road top triangle covers it or
    /// a portal opening cuts it (a portal deck within the reach of its row, the ground's
    /// rendered value at the point less than the clearance above it, T22 5 (a)), pad inside a
    /// site's support footprint, paved on a site surface, graded where the graded value differs
    /// from the base, else natural.
    pub fn mask_at(&self, x: f64, z: f64, scratch: &mut QueryScratch) -> GroundClass {
        if covered_height_at(self.roadbed, x, z, &mut scratch.pieces).is_some() {
            return GroundClass::Road;
        }
        self.prepare([x, z, x, z], scratch);
        self.prepare_portals([x, z, x, z], scratch);
        if self.opened(x, z, scratch) {
            return GroundClass::Road;
        }
        if scratch.sites.pad_at(x, z) != NO_SITE {
            return GroundClass::Pad;
        }
        if let Some(material) = scratch.sites.material_at(x, z) {
            return GroundClass::Paved(material);
        }
        let base = scratch.base.at(x, z);
        let value = self.evaluate(x, z, base, None, scratch);
        if (value - base) as f32 != 0.0 {
            GroundClass::Graded
        } else {
            GroundClass::Natural
        }
    }

    /// The nearer of the road's ray hit and the visible graded ground's: the base patches
    /// (bilinear) in tiles without a seam within the cap, the `GL0` triangles elsewhere, never
    /// ground a road covers or a portal opening cuts.
    pub fn raycast_surface(
        &self,
        origin: [f64; 3],
        direction: [f64; 3],
        scratch: &mut QueryScratch,
    ) -> Option<[f64; 3]> {
        let road = self
            .roadbed
            .raycast(to_vector(origin), to_vector(direction))
            .map(|hit| {
                let hit = [hit.x, hit.y, hit.z].map(f64::from);
                let length2 = dot(direction, direction);
                (0..3)
                    .map(|a| (hit[a] - origin[a]) * direction[a])
                    .sum::<f64>()
                    / length2
            });
        let limit = road.unwrap_or(f64::INFINITY);
        let ground = self.march_ground(origin, direction, limit, true, scratch);
        let t = match (road, ground) {
            (Some(road), Some(ground)) => road.min(ground),
            (road, ground) => road.or(ground)?,
        };
        Some(along(origin, direction, t))
    }

    /// The first hit of a ray on the authored base (bilinear), for terrain brushes.
    pub fn raycast_base(
        &self,
        origin: [f64; 3],
        direction: [f64; 3],
        scratch: &mut QueryScratch,
    ) -> Option<[f64; 3]> {
        self.march_ground(origin, direction, f64::INFINITY, false, scratch)
            .map(|t| along(origin, direction, t))
    }

    // Whether a portal opening cuts the uncovered point of the window and portal parts
    // prepared around it: a portal deck
    // within the reach there, and the ground's rendered value (the base plus its `f32` delta, as
    // the bake packs it) less than the clearance above it (T22 5 (a)).
    fn opened(&self, x: f64, z: f64, scratch: &mut QueryScratch) -> bool {
        let Some(deck) = scratch.portals.deck_at(x, z) else {
            return false;
        };
        let base = scratch.base.at(x, z);
        let delta = (self.evaluate(x, z, base, None, scratch) - base) as f32;
        cuts(deck, base + f64::from(delta))
    }

    // Gathers the seams, sites and base samples within the cap of `rect`: road seams in key
    // order, then the pad seams.
    fn prepare(&self, rect: [f64; 4], scratch: &mut QueryScratch) {
        let window = grown(rect, CAP_M + WINDOW_PAD_M);
        gather_road_seams(
            self.roadbed,
            window,
            &mut scratch.seams,
            &mut scratch.window,
        );
        scratch.sites.gather(self.sites, window);
        scratch.sites.push_pad_seams(&mut scratch.window);
        scratch.base.fill(self.terrain, window);
    }

    // Gathers the portal parts around `rect`, for the opening's test (T22 5 (a)).
    fn prepare_portals(&self, rect: [f64; 4], scratch: &mut QueryScratch) {
        let band = Level::Gl0.band_m(scratch.base.grid().cell_m);
        scratch
            .portals
            .gather(self.roadbed, rect, band, &mut scratch.pieces);
    }

    // The graded value at a point of the prepared window, over every gathered seam, with its
    // pad from the gathered sites.
    fn evaluate(
        &self,
        x: f64,
        z: f64,
        base: f64,
        road: Option<f32>,
        scratch: &QueryScratch,
    ) -> f64 {
        let seams = &scratch.window;
        let band = Level::Gl0.band_m(scratch.base.grid().cell_m);
        let all = (0..seams.len() as u32).map(Listed::unbounded);
        let site = scratch.sites.pad_at(x, z);
        let pad = (site != NO_SITE).then(|| scratch.sites.height(site));
        evaluate(x, z, base, road, pad, seams, all, band).height
    }

    // The rendered height of a `GL0` lattice point: the base plus the `f32` delta the bake
    // packs there.
    fn vertex(&self, [i, j]: [i64; 2], scratch: &mut QueryScratch) -> f64 {
        let (x, z) = (i as f64 * GL0_M, j as f64 * GL0_M);
        let road = covered_height_at(self.roadbed, x, z, &mut scratch.pieces);
        let base = scratch.base.at(x, z);
        let delta = (self.evaluate(x, z, base, road, scratch) - base) as f32;
        base + f64::from(delta)
    }

    // First `t` at which the ray meets the ground before `limit`: per 64 m tile, the base
    // cells where no seam lies within the cap (or always, for the authored base), else the
    // `GL0` cells below the tile's height bound, skipping hits a road covers.
    fn march_ground(
        &self,
        origin: [f64; 3],
        direction: [f64; 3],
        limit: f64,
        graded: bool,
        scratch: &mut QueryScratch,
    ) -> Option<f64> {
        let grid = BaseGrid::of(self.terrain);
        let world = [-grid.half[0], -grid.half[1], grid.half[0], grid.half[1]];
        let (entry, exit) = clip_xz(origin, direction, world)?;
        let (entry, exit) = (entry.max(0.0), exit.min(limit));
        if entry > exit {
            return None;
        }
        let mut hit = None;
        walk_cells(
            origin,
            direction,
            (entry, exit),
            [0.0, 0.0],
            TILE_M,
            |tx, tz, t0, t1| {
                let tile = [tx as f64 * TILE_M, tz as f64 * TILE_M];
                let rect = [tile[0], tile[1], tile[0] + TILE_M, tile[1] + TILE_M];
                if graded {
                    self.prepare(grown(rect, GL0_M), scratch);
                    self.prepare_portals(grown(rect, GL0_M), scratch);
                }
                hit = if !graded || scratch.window.is_empty() {
                    self.march_base(origin, direction, (t0, t1), &grid, scratch)
                } else {
                    self.march_graded(origin, direction, (t0, t1), rect, scratch)
                };
                hit.is_none()
            },
        );
        hit
    }

    // First hit on the bilinear base cells along `[t0, t1]`.
    fn march_base(
        &self,
        origin: [f64; 3],
        direction: [f64; 3],
        (t0, t1): (f64, f64),
        grid: &BaseGrid,
        scratch: &mut QueryScratch,
    ) -> Option<f64> {
        let corner = [-grid.half[0], -grid.half[1]];
        let mut hit = None;
        walk_cells(
            origin,
            direction,
            (t0, t1),
            corner,
            grid.cell_m,
            |cx, cz, a, b| {
                let x0 = corner[0] + cx as f64 * grid.cell_m;
                let z0 = corner[1] + cz as f64 * grid.cell_m;
                let (x1, z1) = (x0 + grid.cell_m, z0 + grid.cell_m);
                scratch.base.fill(self.terrain, [x0, z0, x1, z1]);
                let corners =
                    [(x0, z0), (x1, z0), (x0, z1), (x1, z1)].map(|(x, z)| scratch.base.at(x, z));
                hit = bilinear_hit(origin, direction, (a, b), [x0, z0], grid.cell_m, corners);
                hit.is_none()
            },
        );
        hit
    }

    // First visible hit on the `GL0` triangles of a tile along `[t0, t1]`.
    fn march_graded(
        &self,
        origin: [f64; 3],
        direction: [f64; 3],
        (t0, t1): (f64, f64),
        rect: [f64; 4],
        scratch: &mut QueryScratch,
    ) -> Option<f64> {
        // The ground outside footprints lies in `[min(b, H_lo), max(b, H_hi)]`; triangles that
        // straddle an edge rise at most the capped fill arm above a seam.
        let base_high = scratch.base.range(rect).1;
        let seam_high = scratch
            .window
            .iter()
            .map(|seam| seam.height_range().1)
            .fold(f64::NEG_INFINITY, f64::max);
        let high = base_high.max(seam_high + TIE_IN_SLOPE * FILL_ARM_MAX_M);
        let lowest =
            |a: f64, b: f64| (origin[1] + direction[1] * a).min(origin[1] + direction[1] * b);
        if lowest(t0, t1) > high {
            return None;
        }
        let mut hit = None;
        walk_cells(
            origin,
            direction,
            (t0, t1),
            [0.0, 0.0],
            GL0_M,
            |i, j, a, b| {
                if lowest(a, b) > high {
                    return true;
                }
                let [h00, h10, h01, h11] = [[i, j], [i + 1, j], [i, j + 1], [i + 1, j + 1]]
                    .map(|v| self.vertex(v, scratch));
                let (x0, z0) = (i as f64 * GL0_M, j as f64 * GL0_M);
                let (x1, z1) = (x0 + GL0_M, z0 + GL0_M);
                let triangles = [
                    [[x0, h00, z0], [x1, h10, z0], [x1, h11, z1]],
                    [[x0, h00, z0], [x1, h11, z1], [x0, h01, z1]],
                ];
                let t = triangles
                    .iter()
                    .filter_map(|triangle| ray_triangle(origin, direction, triangle))
                    .filter(|&t| t >= a && t <= b)
                    .fold(None, |best: Option<f64>, t| {
                        Some(best.map_or(t, |best| best.min(t)))
                    });
                if let Some(t) = t {
                    let [x, _, z] = along(origin, direction, t);
                    if covered_height_at(self.roadbed, x, z, &mut scratch.pieces).is_none()
                        && !self.opened(x, z, scratch)
                    {
                        hit = Some(t);
                    }
                }
                hit.is_none()
            },
        );
        hit
    }
}

// The `GL0` lattice cell holding a point and the point's place in it.
struct Cell {
    index: [i64; 2],
    fraction: [f64; 2],
}

impl Cell {
    fn at(x: f64, z: f64) -> Self {
        let index = [x, z].map(|value| (value / GL0_M).floor() as i64);
        let fraction = [
            (x - index[0] as f64 * GL0_M) / GL0_M,
            (z - index[1] as f64 * GL0_M) / GL0_M,
        ];
        Self { index, fraction }
    }

    // The cell grown by `ring` lattice steps.
    fn rect(&self, ring: i64) -> [f64; 4] {
        let [i, j] = self.index;
        [
            (i - ring) as f64 * GL0_M,
            (j - ring) as f64 * GL0_M,
            (i + 1 + ring) as f64 * GL0_M,
            (j + 1 + ring) as f64 * GL0_M,
        ]
    }

    // The triangle under the point: below the diagonal (`fx >= fz`) `(i, j)`, `(i + 1, j)`,
    // `(i + 1, j + 1)`; above it `(i, j)`, `(i, j + 1)`, `(i + 1, j + 1)`.
    fn vertices(&self) -> [[i64; 2]; 3] {
        let [i, j] = self.index;
        if self.fraction[0] >= self.fraction[1] {
            [[i, j], [i + 1, j], [i + 1, j + 1]]
        } else {
            [[i, j], [i, j + 1], [i + 1, j + 1]]
        }
    }

    // Barycentric weights of the point for `vertices`.
    fn weights(&self) -> [f64; 3] {
        let [fx, fz] = self.fraction;
        if fx >= fz {
            [1.0 - fx, fx - fz, fz]
        } else {
            [1.0 - fz, fz - fx, fx]
        }
    }
}

fn normalized(v: [f64; 3]) -> [f64; 3] {
    let length = dot(v, v).sqrt();
    v.map(|c| c / length)
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn along(origin: [f64; 3], direction: [f64; 3], t: f64) -> [f64; 3] {
    [0, 1, 2].map(|axis| origin[axis] + direction[axis] * t)
}

fn to_vector(v: [f64; 3]) -> Vector3 {
    Vector3::new(v[0] as f32, v[1] as f32, v[2] as f32)
}

// The ray's `t` interval inside an XZ rectangle; `None` when it misses. A ray without XZ motion
// is inside for every `t` when its origin is.
fn clip_xz(origin: [f64; 3], direction: [f64; 3], rect: [f64; 4]) -> Option<(f64, f64)> {
    let (mut t0, mut t1) = (f64::NEG_INFINITY, f64::INFINITY);
    for (axis, (lo, hi)) in [(0, (rect[0], rect[2])), (2, (rect[1], rect[3]))] {
        if direction[axis] == 0.0 {
            if origin[axis] < lo || origin[axis] > hi {
                return None;
            }
            continue;
        }
        let (a, b) = (
            (lo - origin[axis]) / direction[axis],
            (hi - origin[axis]) / direction[axis],
        );
        t0 = t0.max(a.min(b));
        t1 = t1.min(a.max(b));
    }
    if direction[0] == 0.0 && direction[2] == 0.0 {
        // A vertical ray: bounded by where it passes the world's height range, which the cell
        // tests find themselves; give it a finite span.
        let span = 1.0e6 / direction[1].abs().max(f64::MIN_POSITIVE);
        return Some((0.0, span));
    }
    (t0 <= t1).then_some((t0, t1))
}

// Visits the square cells of `size` (anchored at `corner`) the ray's XZ path crosses for `t`
// in `[t0, t1]`, in order, with the `t` interval inside each, until `visit` returns false.
fn walk_cells(
    origin: [f64; 3],
    direction: [f64; 3],
    (t0, t1): (f64, f64),
    corner: [f64; 2],
    size: f64,
    mut visit: impl FnMut(i64, i64, f64, f64) -> bool,
) {
    let start = [origin[0] + direction[0] * t0, origin[2] + direction[2] * t0];
    let mut cell = [0, 1].map(|axis| ((start[axis] - corner[axis]) / size).floor() as i64);
    let dir = [direction[0], direction[2]];
    let step = dir.map(|d| if d > 0.0 { 1 } else { -1 });
    let next = |axis: usize, cell: [i64; 2]| {
        if dir[axis] == 0.0 {
            return f64::INFINITY;
        }
        let edge = corner[axis] + (cell[axis] + i64::from(dir[axis] > 0.0)) as f64 * size;
        let o = if axis == 0 { origin[0] } else { origin[2] };
        (edge - o) / dir[axis]
    };
    let mut t = t0;
    // A ray crossing the whole world crosses at most this many cells.
    for _ in 0..1_000_000 {
        let exit = next(0, cell).min(next(1, cell)).min(t1);
        if !visit(cell[0], cell[1], t, exit) || exit >= t1 {
            return;
        }
        let axis = usize::from(next(1, cell) < next(0, cell));
        cell[axis] += step[axis];
        t = exit;
    }
}

// First `t` in `[t0, t1]` where the ray reaches the bilinear patch of a base cell with lower
// corner `corner`, side `size` and corner heights `[h00, h10, h01, h11]`.
fn bilinear_hit(
    origin: [f64; 3],
    direction: [f64; 3],
    (t0, t1): (f64, f64),
    corner: [f64; 2],
    size: f64,
    [h00, h10, h01, h11]: [f64; 4],
) -> Option<f64> {
    // u(t) = u0 + du t, v(t) = v0 + dv t; f(t) = height(u, v) - y(t) = a t^2 + b t + c.
    let (u0, du) = ((origin[0] - corner[0]) / size, direction[0] / size);
    let (v0, dv) = ((origin[2] - corner[1]) / size, direction[2] / size);
    let (ex, ez, exz) = (h10 - h00, h01 - h00, h11 - h10 - h01 + h00);
    let a = exz * du * dv;
    let b = ex * du + ez * dv + exz * (u0 * dv + v0 * du) - direction[1];
    let c = h00 + ex * u0 + ez * v0 + exz * u0 * v0 - origin[1];
    let f = |t: f64| (a * t + b) * t + c;
    if f(t0) >= 0.0 {
        return Some(t0);
    }
    let mut roots = [f64::INFINITY; 2];
    if a == 0.0 {
        if b != 0.0 {
            roots[0] = -c / b;
        }
    } else {
        let disc = b * b - 4.0 * a * c;
        if disc >= 0.0 {
            let q = -0.5 * (b + b.signum() * disc.sqrt());
            roots = [q / a, if q != 0.0 { c / q } else { f64::INFINITY }];
        }
    }
    roots
        .into_iter()
        .filter(|&t| t >= t0 && t <= t1)
        .fold(None, |best: Option<f64>, t| {
            Some(best.map_or(t, |best| best.min(t)))
        })
}

// Möller-Trumbore in `f64`: `t` of the ray's hit on a triangle (either side), if any.
fn ray_triangle(
    origin: [f64; 3],
    direction: [f64; 3],
    [p0, p1, p2]: &[[f64; 3]; 3],
) -> Option<f64> {
    let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let cross = |a: [f64; 3], b: [f64; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let (e1, e2) = (sub(*p1, *p0), sub(*p2, *p0));
    let p = cross(direction, e2);
    let det = dot(e1, p);
    if det == 0.0 {
        return None;
    }
    let s = sub(origin, *p0);
    let u = dot(s, p) / det;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = cross(s, e1);
    let v = dot(direction, q) / det;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    Some(dot(e2, q) / det)
}
