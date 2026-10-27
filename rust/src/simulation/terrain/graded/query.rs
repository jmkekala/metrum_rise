// SPDX-License-Identifier: GPL-2.0-only

//! Point and ray queries of the graded surface (`TERRAIN-07`), for roads and building sites, the
//! read API the consumers moved to in `TERRAIN-07-F`. Every query evaluates the same pointwise
//! function as the bake and answers from the `GL0` lattice the surface renders with: a point
//! between lattice points lies on the `GL0` triangle under it, each lattice cell split along its
//! `(i, j)`-`(i + 1, j + 1)` diagonal.
//!
//! Exact pruning over the query's own window (`docs/terrain.md` T19 (a)): a query reads the
//! lattice points of a small rectangle (its `GL0` cell, one ring more for a normal, the cells
//! within a reach for the gather variant), and evaluates only the seams the bake's first pass
//! would list in a bin of that rectangle: those whose cone may bind its exact base range, and the
//! road seams within the band. Every other seam within the cap holds the base at every point of
//! the rectangle and is not a nearest seam, so it changes no value unless a cut's upper bound holds
//! a point below its base; such a point then takes the cones of the seams whose lower bound may
//! rise above its first-pass value (the bake's second pass, per point). The value is bitwise the
//! evaluation over every seam within the cap. Allocation-free once the scratch is warm.

use super::coverage::{
    covered_height_in, gather as gather_coverage, gather_from_pieces, mark_covered_cells,
};
use super::grade::{CAP_M, continues_ground, may_bind, may_lift};
use super::portal::{PortalWindow, cuts};
use super::prune::box_distance;
use super::query_index::TerrainQueryIndex;
use super::sample::{BaseGrid, BaseWindow, Listed, Value, evaluate};
use super::seams::{Seam, road_seam};
use super::sites::{NO_SITE, SiteBuffer, SiteSource};
use super::tile::{Level, TILE_M, grown, meets};
use crate::assets::SiteSurfaceMaterial;
use crate::simulation::network::roadbed::{
    GroundedPieceScratch, PieceId, RoadVec3, RoadbedSystem, WallSegment,
};
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

/// Reused buffers of the queries, and what the last query or gather prepared.
#[derive(Default)]
pub struct QueryScratch {
    // The rectangle prepared: every lattice point the prepared queries read lies in it.
    rect: Option<[f64; 4]>,
    // The rectangle the portal parts were gathered for.
    portal_rect: Option<[f64; 4]>,
    // Generation of the persistent inputs copied into this scratch.
    index_generation: u64,
    // The seams the first pass lists over `rect`: road seams in key order, unique, then the pad
    // seams in key order.
    window: Vec<Seam>,
    // The other seams whose box comes within the cap of `rect`, with their box's distance from
    // it: only a point below its base after the first pass reads them, so they are gathered
    // when the first such point needs them ([`GradedSurface::gather_rest`]).
    rest: Vec<(Seam, f64)>,
    // Whether `rest` holds the prepared rectangle's other seams.
    rest_ready: bool,
    // The prepared rectangle's exact base range, which sorted its seams.
    base_range: (f64, f64),
    base: BaseWindow,
    // Grounded pieces and top triangles gathered once for the prepared window's coverage reads.
    grounded: GroundedPieceScratch,
    coverage: Vec<[RoadVec3; 3]>,
    pieces: Vec<PieceId>,
    wall_segments: Vec<WallSegment>,
    walls: Vec<Seam>,
    sites: SiteBuffer,
    portals: PortalWindow,
    // Deterministically sampled gather-path comparison in debug and test builds.
    #[cfg(debug_assertions)]
    reference: Option<Box<QueryScratch>>,
    // Test reference: every gathered seam listed, as before the pruning.
    #[cfg(test)]
    pub(super) unpruned: bool,
    // Test counter: points whose value the second pass changed.
    #[cfg(test)]
    pub(super) lifts: std::cell::Cell<u32>,
}

#[cfg(test)]
impl QueryScratch {
    // The seams the last preparation listed, and those it gathered within the cap.
    pub(super) fn listed(&mut self, surface: &GradedSurface<'_>) -> (usize, usize) {
        surface.gather_rest(self);
        (self.window.len(), self.window.len() + self.rest.len())
    }
}

/// The graded surface over a base heightmap, a roadbed and building sites: the read API every
/// consumer moves to. A point query walks the grounded chain segments and the sites within the
/// cap of its window (`O(P + V / 8)` for the pieces met and their runs of chain segments, plus
/// the sites of the 64 m site cells and the corners of the runs and sites that may bind), keeps
/// the `K` seams the pruning lists, `O(K log K)`, then costs `O(K)` per lattice point read (three
/// for a height, twelve for a normal), plus `O(V)` once per window when a point a cut holds below
/// its base needs the other seams. The `_near` variants share one gather between the points
/// within a reach of a centre.
#[derive(Clone, Copy)]
pub struct GradedSurface<'a> {
    /// The base heightmap (its source samples).
    pub terrain: &'a TerrainSystem,
    /// The roadbed whose grounded outer chains and top triangles grade the terrain.
    pub roadbed: &'a RoadbedSystem,
    /// The building sites whose pads and surfaces grade and paint it; ready
    /// ([`SiteSource::ready`]) while the surface is queried.
    pub sites: &'a dyn SiteSource,
    /// Unsaved per-64 m candidate inputs. Production queries always provide it; isolated
    /// fixtures may omit it to exercise the gather reference.
    pub(crate) query_index: Option<&'a TerrainQueryIndex>,
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
        let answer = self.ground_height_at_unchecked(x, z, scratch);
        self.debug_reference(x, z, answer, scratch, |surface, reference| {
            surface.ground_height_at_unchecked(x, z, reference)
        });
        answer
    }

    fn ground_height_at_unchecked(&self, x: f64, z: f64, scratch: &mut QueryScratch) -> f64 {
        let cell = Cell::at(x, z);
        self.prepare(cell.rect(0), scratch);
        if self.quiet(scratch) {
            let [tx, tz] = [x, z].map(|value| (value / TILE_M).floor() * TILE_M);
            self.prepare(grown([tx, tz, tx + TILE_M, tz + TILE_M], GL0_M), scratch);
            if self.quiet(scratch) {
                return scratch.base.at(x, z);
            }
        }
        self.ground_prepared(&cell, scratch)
    }

    /// The visible surface: the road's height where a road covers the point, else the graded
    /// ground.
    pub fn surface_height_at(&self, x: f64, z: f64, scratch: &mut QueryScratch) -> f64 {
        match self.roadbed.height_at(x as f32, z as f32) {
            Some(height) => f64::from(height),
            None => self.ground_height_at(x, z, scratch),
        }
    }

    /// Visible height and rendered normal at one point, prepared as one graded query window.
    pub fn surface_height_and_normal_at(
        &self,
        x: f64,
        z: f64,
        scratch: &mut QueryScratch,
    ) -> (f64, [f64; 3]) {
        if let Some((height, normal)) = self.roadbed.height_and_normal_at(x as f32, z as f32) {
            return (f64::from(height), normal.to_array().map(f64::from));
        }
        let cell = Cell::at(x, z);
        // A seam anywhere in a rendered tile switches that tile from its bilinear base mesh to
        // the GL0 lattice. Prepare the whole tile once so the height makes the same choice as
        // `surface_height_at`, with one GL0 ring for the centre normal's differences.
        let [tx, tz] = [x, z].map(|value| (value / TILE_M).floor() * TILE_M);
        self.prepare(grown([tx, tz, tx + TILE_M, tz + TILE_M], GL0_M), scratch);
        let height = if self.quiet(scratch) {
            scratch.base.at(x, z)
        } else {
            self.ground_prepared(&cell, scratch)
        };
        (height, self.normal_prepared(&cell, scratch))
    }

    /// Prepares the `_near` queries ([`Self::ground_height_near`], [`Self::surface_height_near`])
    /// for every point within `reach` metres of `(x, z)` on each axis: one gather for clustered
    /// points, such as a vehicle's contacts. The window is wider than one point's, so it lists
    /// more seams; every value stays exact.
    pub fn gather_near(&self, x: f64, z: f64, reach: f64, scratch: &mut QueryScratch) {
        if self.query_index.is_some_and(TerrainQueryIndex::ready) {
            scratch.rect = None;
            scratch.portal_rect = None;
            return;
        }
        // A seam anywhere in a rendered tile switches that tile from bilinear base to its `GL0`
        // lattice. Gather the complete tiles the points may occupy so `_near` makes the same
        // tile-level choice as an independent point query.
        let reach = reach.max(0.0);
        let min_x = ((x - reach) / TILE_M).floor() * TILE_M;
        let min_z = ((z - reach) / TILE_M).floor() * TILE_M;
        let max_x = ((x + reach) / TILE_M).floor() * TILE_M + TILE_M;
        let max_z = ((z + reach) / TILE_M).floor() * TILE_M + TILE_M;
        let rect = grown([min_x, min_z, max_x, max_z], GL0_M);
        self.prepare(rect, scratch);
        self.prepare_portals(rect, scratch);
    }

    /// [`Self::ground_height_at`] from the last [`Self::gather_near`], bitwise. A point whose
    /// cell lies outside the gathered window is prepared alone, so the answer is exact either way.
    pub fn ground_height_near(&self, x: f64, z: f64, scratch: &mut QueryScratch) -> f64 {
        let answer = self.ground_height_near_unchecked(x, z, scratch);
        self.debug_reference(x, z, answer, scratch, |surface, reference| {
            surface.ground_height_at_unchecked(x, z, reference)
        });
        answer
    }

    fn ground_height_near_unchecked(&self, x: f64, z: f64, scratch: &mut QueryScratch) -> f64 {
        let cell = Cell::at(x, z);
        if !scratch.rect.is_some_and(|rect| holds(rect, cell.rect(0))) {
            self.prepare(cell.rect(0), scratch);
        }
        let [tx, tz] = [x, z].map(|value| (value / TILE_M).floor() * TILE_M);
        let tile = grown([tx, tz, tx + TILE_M, tz + TILE_M], GL0_M);
        if self.quiet(scratch) || self.quiet_for(tile, x, z, scratch) {
            return scratch.base.at(x, z);
        }
        self.ground_prepared(&cell, scratch)
    }

    /// [`Self::mask_at`] from the last [`Self::gather_near`], equal to it; a point outside the
    /// gathered window is prepared alone.
    pub fn mask_near(&self, x: f64, z: f64, scratch: &mut QueryScratch) -> GroundClass {
        let answer = self.mask_near_unchecked(x, z, scratch);
        self.debug_reference(x, z, answer, scratch, |surface, reference| {
            surface.mask_at_unchecked(x, z, reference)
        });
        answer
    }

    fn mask_near_unchecked(&self, x: f64, z: f64, scratch: &mut QueryScratch) -> GroundClass {
        let point = [x, z, x, z];
        if !scratch.rect.is_some_and(|rect| holds(rect, point)) {
            self.prepare(point, scratch);
        }
        if covered_height_in(&scratch.coverage, x, z).is_some() {
            return GroundClass::Road;
        }
        if !scratch.portal_rect.is_some_and(|rect| holds(rect, point)) {
            self.prepare_portals(point, scratch);
        }
        self.mask_prepared(x, z, scratch)
    }

    /// [`Self::surface_height_at`] from the last [`Self::gather_near`], bitwise.
    pub fn surface_height_near(&self, x: f64, z: f64, scratch: &mut QueryScratch) -> f64 {
        match self.roadbed.height_at(x as f32, z as f32) {
            Some(height) => f64::from(height),
            None => self.ground_height_near(x, z, scratch),
        }
    }

    // The graded ground on the `GL0` triangle under the point of `cell`, from the prepared
    // window.
    fn ground_prepared(&self, cell: &Cell, scratch: &mut QueryScratch) -> f64 {
        let [w0, w1, w2] = cell.weights();
        let [v0, v1, v2] = cell.vertices();
        w0 * self.vertex(v0, scratch)
            + w1 * self.vertex(v1, scratch)
            + w2 * self.vertex(v2, scratch)
    }

    /// The rendered `GL0` normal: per lattice point the central difference of the graded
    /// height over its four neighbours, interpolated over the `GL0` triangle under the point and
    /// normalized.
    pub fn normal_at(&self, x: f64, z: f64, scratch: &mut QueryScratch) -> [f64; 3] {
        let answer = self.normal_at_unchecked(x, z, scratch);
        self.debug_reference(
            x,
            z,
            answer.map(f64::to_bits),
            scratch,
            |surface, reference| surface.normal_at_unchecked(x, z, reference).map(f64::to_bits),
        );
        answer
    }

    fn normal_at_unchecked(&self, x: f64, z: f64, scratch: &mut QueryScratch) -> [f64; 3] {
        let cell = Cell::at(x, z);
        self.prepare(cell.rect(1), scratch);
        self.normal_prepared(&cell, scratch)
    }

    fn normal_prepared(&self, cell: &Cell, scratch: &mut QueryScratch) -> [f64; 3] {
        let cell_m = scratch.base.grid().cell_m;
        let mut normal = [0.0; 3];
        for (weight, [i, j]) in cell.weights().into_iter().zip(cell.vertices()) {
            let (px, pz) = (i as f64 * GL0_M, j as f64 * GL0_M);
            let base = &scratch.base;
            let base_dx = (base.at(px + cell_m, pz) - base.at(px - cell_m, pz)) / (2.0 * cell_m);
            let base_dz = (base.at(px, pz + cell_m) - base.at(px, pz - cell_m)) / (2.0 * cell_m);
            let dx = base_dx
                + (self.delta([i + 1, j], scratch) - self.delta([i - 1, j], scratch))
                    / (2.0 * GL0_M);
            let dz = base_dz
                + (self.delta([i, j + 1], scratch) - self.delta([i, j - 1], scratch))
                    / (2.0 * GL0_M);
            let vertex = normalized([-dx, 1.0, -dz]);
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
        let answer = self.mask_at_unchecked(x, z, scratch);
        self.debug_reference(x, z, answer, scratch, |surface, reference| {
            surface.mask_at_unchecked(x, z, reference)
        });
        answer
    }

    fn mask_at_unchecked(&self, x: f64, z: f64, scratch: &mut QueryScratch) -> GroundClass {
        self.prepare([x, z, x, z], scratch);
        if covered_height_in(&scratch.coverage, x, z).is_some() {
            return GroundClass::Road;
        }
        self.prepare_portals([x, z, x, z], scratch);
        self.mask_prepared(x, z, scratch)
    }

    #[cfg(debug_assertions)]
    fn debug_reference<T: std::fmt::Debug + PartialEq>(
        &self,
        x: f64,
        z: f64,
        answer: T,
        scratch: &mut QueryScratch,
        query: impl FnOnce(&GradedSurface<'_>, &mut QueryScratch) -> T,
    ) {
        let sample = x.to_bits().wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ z.to_bits().rotate_left(23);
        if self.query_index.is_none() || sample & 0xff != 0 {
            return;
        }
        let mut reference = scratch.reference.take().unwrap_or_default();
        let surface = GradedSurface {
            terrain: self.terrain,
            roadbed: self.roadbed,
            sites: self.sites,
            query_index: None,
        };
        let expected = query(&surface, &mut reference);
        scratch.reference = Some(reference);
        assert_eq!(answer, expected, "indexed graded query differs at ({x}, {z})");
    }

    #[cfg(not(debug_assertions))]
    fn debug_reference<T>(
        &self,
        _: f64,
        _: f64,
        _: T,
        _: &mut QueryScratch,
        _: impl FnOnce(&GradedSurface<'_>, &mut QueryScratch) -> T,
    ) {
    }

    /// The area form of [`Self::mask_at`]'s `Road` (by coverage) and `Pad` classes, for water's
    /// cells (`docs/terrain.md` T32 (a)): `out[j * (xs.len() - 1) + i]` is whether a grounded
    /// road top triangle or a building site's support footprint meets the closed cell
    /// `[xs[i], xs[i + 1]] x [zs[j], zs[j + 1]]` (`xs`, `zs` sorted). Exact: the coverage
    /// predicate's closed triangles, each footprint's closed polygon, exact orientations. Portal
    /// openings are not counted, and a footprint is not grown by the allocator predicate's 1 mm
    /// tolerance. The site index must be ready. `O(T + S + C)` for the triangles and sites met
    /// and the cells in their boxes.
    pub fn mark_cells_met_by_roads_and_pads(
        &self,
        xs: &[f64],
        zs: &[f64],
        out: &mut Vec<bool>,
        scratch: &mut QueryScratch,
    ) {
        out.clear();
        let (Some(&min_x), Some(&max_x), Some(&min_z), Some(&max_z)) =
            (xs.first(), xs.last(), zs.first(), zs.last())
        else {
            return;
        };
        out.resize(xs.len().saturating_sub(1) * zs.len().saturating_sub(1), false);
        mark_covered_cells(self.roadbed, xs, zs, &mut scratch.pieces, out);
        // The gather replaces the window's sites, so no window stays prepared.
        scratch.rect = None;
        scratch.sites.gather_unordered(self.sites, [min_x, min_z, max_x, max_z]);
        scratch.sites.mark_footprint_cells(xs, zs, out);
    }

    // [`Self::mask_at`] at an uncovered point of the prepared window and portal parts.
    fn mask_prepared(&self, x: f64, z: f64, scratch: &mut QueryScratch) -> GroundClass {
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

    // Prepares the queries of the lattice points in `rect`: the base samples one base cell around
    // it, the sites within the cap, and the seams within the cap the bake's first pass lists over
    // the rectangle's exact base range (their cone may bind that base, or a road seam lies within
    // the band): road seams in key order, then the pad seams. The runs of chain segments and the
    // sites whose bounds fail that test hold no listed seam and are skipped; the other seams
    // within the cap wait for [`Self::gather_rest`]. `O(P + V / 8 + K log K)` plus the sites, for
    // the `K` seams of the runs and sites that may bind.
    fn prepare(&self, rect: [f64; 4], scratch: &mut QueryScratch) {
        if let Some(index) = self.query_index.filter(|index| index.ready()) {
            let tile =
                super::tile::TileKey::at((rect[0] + rect[2]) * 0.5, (rect[1] + rect[3]) * 0.5);
            if scratch.rect == Some(rect) && scratch.index_generation == index.generation() {
                return;
            }
            let cell_m = BaseGrid::of(self.terrain).cell_m;
            scratch.base.fill(self.terrain, grown(rect, cell_m));
            scratch.window.clear();
            scratch.rest.clear();
            let window = grown(rect, CAP_M + WINDOW_PAD_M);
            let band = Level::Gl0.band_m(cell_m);
            let base = scratch.base.bilinear_range(rect);
            let unpruned = Self::unpruned(scratch);
            let binds = |seam: &Seam, band: f64| {
                unpruned
                    || may_bind(
                        seam.height_range(),
                        base,
                        box_distance(seam.box_xz(), rect),
                        band,
                    )
            };
            if let Some(inputs) = index.get(tile) {
                scratch.coverage.clear();
                gather_from_pieces(
                    self.roadbed,
                    rect,
                    &inputs.coverage_pieces,
                    &mut scratch.coverage,
                );
                scratch.sites.copy_inputs_in_bounds_from(&inputs.sites, window);
                scratch.walls.clear();
                scratch.walls.extend(
                    inputs
                        .walls
                        .iter()
                        .copied()
                        .filter(|seam| meets(seam.box_xz(), window)),
                );
                for seam in &inputs.road_seams {
                    if !meets(seam.box_xz(), window) {
                        continue;
                    }
                    if binds(seam, band) {
                        scratch.window.push(*seam);
                    } else {
                        let distance = box_distance(seam.box_xz(), rect);
                        if distance <= CAP_M + WINDOW_PAD_M {
                            scratch.rest.push((*seam, distance));
                        }
                    }
                }
                let roads = scratch.window.len();
                scratch.sites.for_each_pad_seam_where(
                    |bounds, _| meets(bounds, window),
                    |seam| {
                        if binds(&seam, 0.0) {
                            scratch.window.push(seam);
                        } else {
                            let distance = box_distance(seam.box_xz(), rect);
                            if distance <= CAP_M + WINDOW_PAD_M {
                                scratch.rest.push((seam, distance));
                            }
                        }
                    },
                );
                scratch.window[roads..].sort_unstable_by_key(Seam::key);
            } else {
                scratch.coverage.clear();
                scratch.sites.clear();
                scratch.walls.clear();
            }
            scratch.rect = Some(rect);
            scratch.index_generation = index.generation();
            scratch.rest_ready = true;
            scratch.base_range = base;
            return;
        }
        let window = grown(rect, CAP_M + WINDOW_PAD_M);
        let cell_m = BaseGrid::of(self.terrain).cell_m;
        scratch.base.fill(self.terrain, grown(rect, cell_m));
        gather_coverage(
            self.roadbed,
            rect,
            &mut scratch.grounded,
            &mut scratch.coverage,
        );
        let band = Level::Gl0.band_m(cell_m);
        let base = scratch.base.bilinear_range(rect);
        let unpruned = Self::unpruned(scratch);
        let QueryScratch {
            window: listed,
            rest,
            sites,
            ..
        } = scratch;
        listed.clear();
        rest.clear();
        let binds = |bounds: [f64; 4], heights: (f64, f64), band: f64| {
            unpruned || may_bind(heights, base, box_distance(bounds, rect), band)
        };
        self.roadbed.for_each_grounded_seam_in_bounds_where_indexed(
            window,
            &mut scratch.grounded,
            |bounds, heights| binds(bounds, heights, band),
            |[a, b]| {
                let seam = road_seam(a, b);
                if binds(seam.box_xz(), seam.height_range(), band) {
                    listed.push(seam);
                }
            },
        );
        // A segment two pieces share comes twice; key order is the nearest seam's tie order.
        listed.sort_unstable_by_key(Seam::key);
        listed.dedup_by_key(|seam| seam.key());
        let roads = listed.len();
        // The queries read the sites in any order (`SiteBuffer::gather_unordered`).
        sites.gather_unordered(self.sites, window);
        // No sample names a pad seam, so it takes no band.
        sites.for_each_pad_seam_where(
            |bounds, height| binds(bounds, (height, height), 0.0),
            |seam| {
                if binds(seam.box_xz(), seam.height_range(), 0.0) {
                    listed.push(seam);
                }
            },
        );
        listed[roads..].sort_unstable_by_key(Seam::key);
        self.roadbed
            .grounded_pieces_in_bounds(window, &mut scratch.pieces);
        self.roadbed.wall_seams_in_bounds(
            window,
            &scratch.pieces,
            &mut scratch.wall_segments,
        );
        scratch.walls.clear();
        scratch.walls.extend(scratch.wall_segments.iter().map(|wall| {
            let [a, b] = wall.points;
            Seam {
                kind: super::seams::SeamKind::Wall,
                ..road_seam(a, b)
            }
        }));
        scratch.rect = Some(rect);
        scratch.rest_ready = false;
        scratch.base_range = base;
    }

    // Gathers the prepared rectangle's other seams within the cap into `rest`, once per
    // preparation: the road and pad seams of the window the first pass does not list, with their
    // box's distance from the rectangle. `O(P + V)` plus the sites' corners.
    fn gather_rest(&self, scratch: &mut QueryScratch) {
        let Some(rect) = scratch.rect.filter(|_| !scratch.rest_ready) else {
            return;
        };
        let window = grown(rect, CAP_M + WINDOW_PAD_M);
        let band = Level::Gl0.band_m(scratch.base.grid().cell_m);
        let (base, unpruned) = (scratch.base_range, Self::unpruned(scratch));
        let QueryScratch {
            rest,
            sites,
            ..
        } = scratch;
        rest.clear();
        let mut sort = |seam: Seam, band: f64| {
            let distance = box_distance(seam.box_xz(), rect);
            if !unpruned
                && !may_bind(seam.height_range(), base, distance, band)
                && distance <= CAP_M + WINDOW_PAD_M
            {
                rest.push((seam, distance));
            }
        };
        self.roadbed.for_each_grounded_seam_in_bounds_where_indexed(
            window,
            &mut scratch.grounded,
            |_, _| true,
            |[a, b]| sort(road_seam(a, b), band),
        );
        sites.for_each_pad_seam(|seam| sort(seam, 0.0));
        scratch.rest_ready = true;
    }

    // Whether the test reference lists every gathered seam.
    #[cfg(test)]
    fn unpruned(scratch: &QueryScratch) -> bool {
        scratch.unpruned
    }

    #[cfg(not(test))]
    fn unpruned(_: &QueryScratch) -> bool {
        false
    }

    // Whether no seam comes within the cap of the prepared rectangle, listed or not.
    fn quiet(&self, scratch: &mut QueryScratch) -> bool {
        self.gather_rest(scratch);
        scratch.window.is_empty() && scratch.rest.is_empty()
    }

    // Whether the seams already gathered for a wider `_near` window leave this point's local
    // rectangle quiet. This preserves the bilinear-base shortcut of [`Self::ground_height_at`]
    // without another piece/site gather for the common all-quiet case.
    fn quiet_for(&self, rect: [f64; 4], x: f64, z: f64, scratch: &mut QueryScratch) -> bool {
        self.gather_rest(scratch);
        if scratch.sites.pad_at(x, z) != NO_SITE {
            return false;
        }
        let window = grown(rect, CAP_M + WINDOW_PAD_M);
        scratch
            .window
            .iter()
            .chain(scratch.rest.iter().map(|(seam, _)| seam))
            .all(|seam| box_distance(seam.box_xz(), window) > 0.0)
    }

    // Gathers the portal parts around `rect`, for the opening's test (T22 5 (a)).
    fn prepare_portals(&self, rect: [f64; 4], scratch: &mut QueryScratch) {
        let band = Level::Gl0.band_m(scratch.base.grid().cell_m);
        scratch
            .portals
            .gather(self.roadbed, rect, band, &mut scratch.pieces);
        scratch.portal_rect = Some(rect);
    }

    // The graded value at a point of the prepared window, with its pad from the gathered sites:
    // over the listed seams, then, for a ground point a cut's upper bound holds below its base, the
    // cones of the other seams whose lower bound may rise above that first-pass value. Their
    // upper bounds lie above the base, so `min(b, U)` keeps its operand and the overlap rule
    // returns the evaluation over every seam within the cap, bitwise.
    fn evaluate(
        &self,
        x: f64,
        z: f64,
        base: f64,
        road: Option<f32>,
        scratch: &mut QueryScratch,
    ) -> f64 {
        let (mut first, mut pad) = self.value(x, z, base, road, scratch);
        let band = Level::Gl0.band_m(scratch.base.grid().cell_m);
        let continued = road.is_some()
            && continues_ground(&scratch.walls, x, z, first.nearest_distance, band);
        if continued {
            (first, pad) = self.value(x, z, base, None, scratch);
        }
        if (road.is_some() && !continued) || pad.is_some() || first.height >= base {
            return first.height;
        }
        self.gather_rest(scratch);
        let mut cones = first.cones;
        for (seam, distance) in &scratch.rest {
            if may_lift(seam.height_range().1, first.height, *distance) {
                let at = seam.at(x, z);
                if at.distance <= CAP_M {
                    cones.add(at.height, at.distance);
                }
            }
        }
        let height = cones.ground(base);
        #[cfg(test)]
        if height != first.height {
            scratch.lifts.set(scratch.lifts.get() + 1);
        }
        height
    }

    // The first-pass value at one point and whether a site pad holds it.
    fn value(
        &self,
        x: f64,
        z: f64,
        base: f64,
        road: Option<f32>,
        scratch: &QueryScratch,
    ) -> (Value, Option<f64>) {
        let seams = &scratch.window;
        let band = Level::Gl0.band_m(scratch.base.grid().cell_m);
        let all = (0..seams.len() as u32).map(Listed::unbounded);
        let site = scratch.sites.pad_at(x, z);
        let pad = (site != NO_SITE).then(|| scratch.sites.height(site));
        (evaluate(x, z, base, road, pad, seams, all, band), pad)
    }

    // The rendered height of a `GL0` lattice point: the base plus the `f32` delta the bake
    // packs there.
    fn vertex(&self, [i, j]: [i64; 2], scratch: &mut QueryScratch) -> f64 {
        let (x, z) = (i as f64 * GL0_M, j as f64 * GL0_M);
        scratch.base.at(x, z) + self.delta([i, j], scratch)
    }

    // The `f32` delta the bake packs at one `GL0` lattice point.
    fn delta(&self, [i, j]: [i64; 2], scratch: &mut QueryScratch) -> f64 {
        let (x, z) = (i as f64 * GL0_M, j as f64 * GL0_M);
        let road = covered_height_in(&scratch.coverage, x, z);
        let base = scratch.base.at(x, z);
        f64::from((self.evaluate(x, z, base, road, scratch) - base) as f32)
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
                hit = if !graded || self.quiet(scratch) {
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
            .chain(scratch.rest.iter().map(|(seam, _)| seam))
            .map(|seam| seam.height_range().1)
            .fold(f64::NEG_INFINITY, f64::max);
        let high = base_high.max(seam_high);
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
                    if covered_height_in(&scratch.coverage, x, z).is_none()
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

// Whether `outer` holds `inner`, both `[min_x, min_z, max_x, max_z]`.
fn holds(outer: [f64; 4], inner: [f64; 4]) -> bool {
    outer[0] <= inner[0] && outer[1] <= inner[1] && inner[2] <= outer[2] && inner[3] <= outer[3]
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
