// SPDX-License-Identifier: GPL-2.0-only

//! Building sites as grading and material clients (`TERRAIN-07-C`): the sites a window reads,
//! in a canonical order, their `Pad` seams, the `Pad` class and support height of the lattice
//! points their footprints hold, the material class of the first surface holding a point, and
//! the nearest material edge within the band.
//!
//! The terrain is the pad (`T11`): inside a support footprint, by the allocator's own
//! predicate, the graded height is the support height exactly. Sites are ordered by their
//! content (footprint, height, surfaces), never by building index, which a swap-remove far away
//! renumbers: the first surface holding a point, in that order and then in authored order, sets
//! its class.

use super::seams::{Seam, SeamAt, SeamKind, segment_at};
use crate::assets::SiteSurfaceMaterial;
use crate::simulation::buildings::allocator::site_footprint_contains;
use godot::prelude::Vector2;
use std::cmp::Ordering;

/// No site or edge index.
pub(super) const NO_SITE: u32 = u32::MAX;
// Bounds padding around a footprint: the allocator's predicate accepts points up to 1 mm
// outside an edge (`SITE_POINT_EPS_M`), and f32 rounding of the corners.
const FOOTPRINT_PAD_M: f64 = 0.01;
// Bounds padding around a surface or an edge's band: rounding only lists more points, each
// then tested exactly.
const SURFACE_PAD_M: f64 = 1.0e-6;
// Distance from a polygon's sides beyond which a lattice point's side is the even-odd rule's
// along its row, so the point needs no predicate test: far above the allocator predicate's 1 mm
// tolerance and the f32 rounding of its crossings.
const SPAN_MARGIN_M: f64 = 0.01;

/// What the graded terrain reads of building sites: the sites whose bounds meet a rectangle,
/// through a spatial index (the allocator's 512 m site chunks), never a scan of every site.
pub trait SiteSource: Sync {
    /// Whether rectangle queries can be answered without scanning every site: false while
    /// the source's index is being rebuilt. Fixed while the source is borrowed.
    fn ready(&self) -> bool;

    /// Appends to `out` ([`SiteBuffer::push_site`], [`SiteBuffer::push_surface`]) every site
    /// whose bounds (lot, footprint and surfaces) meet `rect` (`[min_x, min_z, max_x, max_z]`),
    /// possibly more. Only called while [`Self::ready`].
    fn sites_in_bounds(&self, rect: [f64; 4], out: &mut SiteBuffer);
}

/// A world without building sites, for road-only fixtures.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoSites;

impl SiteSource for NoSites {
    fn ready(&self) -> bool {
        true
    }

    fn sites_in_bounds(&self, _: [f64; 4], _: &mut SiteBuffer) {}
}

// One site of a buffer: its footprint corners and surfaces (ranges into the buffer's points and
// surfaces) and its support height.
#[derive(Clone, Copy, Debug)]
struct SiteRecord {
    footprint: (u32, u32),
    height: f32,
    surfaces: (u32, u32),
}

// One surface polygon: its material, corners and XZ bounds.
#[derive(Clone, Copy, Debug)]
struct SurfaceRecord {
    material: SiteSurfaceMaterial,
    points: (u32, u32),
    bounds: [f64; 4],
}

/// The building sites of one window, filled by a [`SiteSource`] and put in canonical order by
/// the bake or query that reads them. Reused between windows, so gathering allocates nothing
/// once warm.
#[derive(Clone, Debug, Default)]
pub struct SiteBuffer {
    sites: Vec<SiteRecord>,
    points: Vec<Vector2>,
    surfaces: Vec<SurfaceRecord>,
    // Site positions in canonical order.
    order: Vec<u32>,
    // Material edges, oriented with the surface's interior on the left, sorted by key and
    // unique: `[ax, az, bx, bz]`.
    edges: Vec<[f64; 4]>,
    edge_keys: Vec<([u64; 4], usize)>,
    ring: Vec<[f64; 2]>,
}

impl SiteBuffer {
    /// Adds one site: its support footprint corners (world XZ) and flat support height. Its
    /// surfaces follow with [`Self::push_surface`].
    pub fn push_site(&mut self, footprint: &[Vector2], support_height_m: f32) {
        let start = self.points.len() as u32;
        self.points.extend_from_slice(footprint);
        let surfaces = self.surfaces.len() as u32;
        self.sites.push(SiteRecord {
            footprint: (start, self.points.len() as u32),
            height: support_height_m,
            surfaces: (surfaces, surfaces),
        });
    }

    /// Adds a surface polygon (world XZ) to the last site, after its earlier surfaces.
    pub fn push_surface(&mut self, material: SiteSurfaceMaterial, polygon: &[Vector2]) {
        let start = self.points.len() as u32;
        self.points.extend_from_slice(polygon);
        self.surfaces.push(SurfaceRecord {
            material,
            points: (start, self.points.len() as u32),
            bounds: bounds_of(polygon),
        });
        if let Some(site) = self.sites.last_mut() {
            site.surfaces.1 = self.surfaces.len() as u32;
        }
    }

    pub(super) fn clear(&mut self) {
        self.sites.clear();
        self.points.clear();
        self.surfaces.clear();
        self.order.clear();
        self.edges.clear();
    }

    /// Whether no site was gathered.
    pub(super) fn is_empty(&self) -> bool {
        self.sites.is_empty()
    }

    /// Gathers the sites meeting `rect` from a ready `source` in canonical order, with their
    /// edges. `O(sites log sites + edges log edges)`.
    pub(super) fn gather(&mut self, source: &(impl SiteSource + ?Sized), rect: [f64; 4]) {
        self.gather_unordered(source, rect);
        self.canonicalize();
    }

    /// Gathers the sites meeting `rect` from a ready `source` in the source's order, without the
    /// canonical order and the material edges: only [`Self::for_each_pad_seam`] may read them,
    /// whose seams do not depend on the order (the selection's floor). `O(sites)`.
    pub(super) fn gather_unordered(&mut self, source: &(impl SiteSource + ?Sized), rect: [f64; 4]) {
        debug_assert!(
            source.ready(),
            "graded terrain sites read from an unready source"
        );
        self.clear();
        source.sites_in_bounds(rect, self);
    }

    // Orders the sites by content and builds the material edge list.
    fn canonicalize(&mut self) {
        self.order.clear();
        self.order.extend(0..self.sites.len() as u32);
        let Self {
            sites,
            points,
            surfaces,
            order,
            ..
        } = self;
        order.sort_unstable_by(|&a, &b| {
            compare_sites(points, surfaces, &sites[a as usize], &sites[b as usize])
        });
        self.edge_keys.clear();
        for surface in 0..self.surfaces.len() {
            self.oriented_ring(self.surfaces[surface].points);
            let n = self.ring.len();
            if n < 2 {
                continue;
            }
            for i in 0..n {
                let ([ax, az], [bx, bz]) = (self.ring[i], self.ring[(i + 1) % n]);
                self.edge_keys
                    .push(([ax, az, bx, bz].map(f64::to_bits), surface));
            }
        }
        self.edge_keys.sort_unstable_by_key(|&(key, _)| key);
        self.edge_keys.dedup_by_key(|(key, _)| *key);
        self.edges.clear();
        self.edges.extend(
            self.edge_keys
                .iter()
                .map(|(key, _)| key.map(f64::from_bits)),
        );
    }

    // The f64 corners of `points`, oriented with positive signed area (the material partition's
    // convention), into `self.ring`.
    fn oriented_ring(&mut self, (start, end): (u32, u32)) {
        self.ring.clear();
        self.ring.extend(
            self.points[start as usize..end as usize]
                .iter()
                .map(|p| [f64::from(p.x), f64::from(p.y)]),
        );
        if signed_area(&self.ring) < 0.0 {
            self.ring.reverse();
        }
    }

    // The site at canonical position `rank`.
    fn ranked(&self, rank: usize) -> &SiteRecord {
        &self.sites[self.order[rank] as usize]
    }

    fn footprint(&self, site: &SiteRecord) -> &[Vector2] {
        &self.points[site.footprint.0 as usize..site.footprint.1 as usize]
    }

    /// The support height of the site at canonical position `rank`, metres.
    pub(super) fn height(&self, rank: u32) -> f64 {
        f64::from(self.ranked(rank as usize).height)
    }

    /// The material edges, sorted by key: `[ax, az, bx, bz]`, interior on the left.
    pub(super) fn edges(&self) -> &[[f64; 4]] {
        &self.edges
    }

    /// Appends the `Pad` seams of every site, sorted by key. `O(corners log corners)`.
    pub(super) fn push_pad_seams(&mut self, out: &mut Vec<Seam>) {
        let start = out.len();
        self.for_each_pad_seam(|seam| out.push(seam));
        // Sorted by key, the gathered order is gone: seams with equal keys are equal.
        out[start..].sort_unstable_by_key(Seam::key);
    }

    /// Visits the `Pad` seams of every site in the buffer's order: each footprint side at the
    /// support height, oriented counter-clockwise (positive signed area in XZ), each site's
    /// sides from its first corner. A footprint of fewer than three corners holds no point and
    /// gives none. `O(corners)`.
    pub(super) fn for_each_pad_seam(&mut self, mut visit: impl FnMut(Seam)) {
        for index in 0..self.sites.len() {
            let site = self.sites[index];
            let height = f64::from(site.height);
            if site.footprint.1 - site.footprint.0 < 3 {
                continue;
            }
            self.oriented_ring(site.footprint);
            let n = self.ring.len();
            for i in 0..n {
                let ([ax, az], [bx, bz]) = (self.ring[i], self.ring[(i + 1) % n]);
                visit(Seam {
                    a: [ax, height, az],
                    b: [bx, height, bz],
                    kind: SeamKind::Pad,
                });
            }
        }
    }

    /// The canonical position of the first site whose support footprint holds the world point
    /// `(x, z)` by the allocator's predicate, else [`NO_SITE`]. `O(sites x corners)`.
    pub(super) fn pad_at(&self, x: f64, z: f64) -> u32 {
        let point = Vector2::new(x as f32, z as f32);
        (0..self.sites.len())
            .find(|&rank| {
                let site = self.ranked(rank);
                let [min_x, min_z, max_x, max_z] = bounds_of(self.footprint(site));
                x >= min_x - FOOTPRINT_PAD_M
                    && x <= max_x + FOOTPRINT_PAD_M
                    && z >= min_z - FOOTPRINT_PAD_M
                    && z <= max_z + FOOTPRINT_PAD_M
                    && site_footprint_contains(self.footprint(site), point)
            })
            .map_or(NO_SITE, |rank| rank as u32)
    }

    /// The material of the first surface holding the world point `(x, z)` (closed), sites in
    /// canonical order and each site's surfaces in authored order; `None` for none.
    /// `O(surfaces x corners)`.
    pub(super) fn material_at(&mut self, x: f64, z: f64) -> Option<SiteSurfaceMaterial> {
        for rank in 0..self.sites.len() {
            let site = *self.ranked(rank);
            for surface in site.surfaces.0..site.surfaces.1 {
                let record = self.surfaces[surface as usize];
                if inside_box(record.bounds, x, z, SURFACE_PAD_M) {
                    self.oriented_ring(record.points);
                    if polygon_holds(&self.ring, x, z) {
                        return Some(record.material);
                    }
                }
            }
        }
        None
    }

    /// Rasterizes the `Pad` class over the lattice `xs` x `zs` (row-major by `z`): `pads[k]`
    /// is the canonical position of the first site whose footprint holds point `k`, else
    /// [`NO_SITE`]. Pruned, each footprint's rows take the even-odd rule away from its sides
    /// ([`for_each_span_point`]) and the allocator's predicate within `SPAN_MARGIN_M` of them;
    /// unpruned, the predicate at every point. `O(points in the footprints + points near their
    /// sides x corners)`.
    pub(super) fn rasterize_pads(
        &mut self,
        xs: &[f64],
        zs: &[f64],
        prune: bool,
        pads: &mut Vec<u32>,
    ) {
        pads.clear();
        pads.resize(xs.len() * zs.len(), NO_SITE);
        for rank in 0..self.sites.len() {
            let site = *self.ranked(rank);
            if site.footprint.1 - site.footprint.0 < 3 {
                continue;
            }
            self.oriented_ring(site.footprint);
            let (ring, footprint) = (&self.ring, self.footprint(&site));
            for_each_span_point(xs, zs, ring, FOOTPRINT_PAD_M, prune, |k, x, z, sure| {
                if pads[k] == NO_SITE
                    && (sure
                        || site_footprint_contains(footprint, Vector2::new(x as f32, z as f32)))
                {
                    pads[k] = rank as u32;
                }
            });
        }
    }

    /// Rasterizes the material class over the lattice into `classes` (0 none, 1 asphalt,
    /// 2 concrete), the first surface holding each point. `O(points in the boxes x corners)`.
    pub(super) fn rasterize_classes(
        &mut self,
        xs: &[f64],
        zs: &[f64],
        prune: bool,
        classes: &mut Vec<u8>,
    ) {
        classes.clear();
        classes.resize(xs.len() * zs.len(), 0);
        for rank in 0..self.sites.len() {
            let site = *self.ranked(rank);
            for surface in site.surfaces.0..site.surfaces.1 {
                let record = self.surfaces[surface as usize];
                self.oriented_ring(record.points);
                let (ring, class) = (&self.ring, material_class(record.material));
                for_each_span_point(xs, zs, ring, SURFACE_PAD_M, prune, |k, x, z, sure| {
                    if classes[k] == 0 && (sure || polygon_holds(ring, x, z)) {
                        classes[k] = class;
                    }
                });
            }
        }
    }

    /// The nearest material edge of every lattice point within `band`, by the seams' total
    /// order (smaller distance, then the larger distance to the edge's line, then the smaller
    /// key): `nearest[k]` its index in [`Self::edges`], else [`NO_SITE`]. Pruned, each edge
    /// visits the points of each row its band can reach; unpruned, every point.
    /// `O(points within the edges' bands)`.
    pub(super) fn nearest_edges(
        &self,
        xs: &[f64],
        zs: &[f64],
        band: f64,
        prune: bool,
        nearest: &mut Vec<(u32, f64)>,
    ) {
        nearest.clear();
        nearest.resize(xs.len() * zs.len(), (NO_SITE, f64::INFINITY));
        let width = xs.len();
        for (index, edge) in self.edges.iter().enumerate() {
            let index = index as u32;
            let (a, b) = edge_points(edge);
            let mut visit = |k: usize, x: f64, z: f64| {
                let at = segment_at(a, b, x, z);
                if at.distance >= band {
                    return;
                }
                let (best, best_distance) = nearest[k];
                let nearer = best == NO_SITE
                    || at.distance < best_distance
                    || (at.distance == best_distance
                        && at.nearer_than(index, &self.edge_at(best, x, z), best));
                if nearer {
                    nearest[k] = (index, at.distance);
                }
            };
            if !prune {
                for (j, &z) in zs.iter().enumerate() {
                    for (i, &x) in xs.iter().enumerate() {
                        visit(j * width + i, x, z);
                    }
                }
                continue;
            }
            let (z0, z1) = (
                edge[1].min(edge[3]) - band - SURFACE_PAD_M,
                edge[1].max(edge[3]) + band + SURFACE_PAD_M,
            );
            let (j0, j1) = (
                zs.partition_point(|&z| z < z0),
                zs.partition_point(|&z| z <= z1),
            );
            for (j, &z) in zs.iter().enumerate().take(j1).skip(j0) {
                let Some((x0, x1)) = band_span(edge, band, z) else {
                    continue;
                };
                let (i0, i1) = (
                    xs.partition_point(|&x| x < x0),
                    xs.partition_point(|&x| x <= x1),
                );
                for (i, &x) in xs.iter().enumerate().take(i1).skip(i0) {
                    visit(j * width + i, x, z);
                }
            }
        }
    }

    fn edge_at(&self, index: u32, x: f64, z: f64) -> SeamAt {
        let (a, b) = edge_points(&self.edges[index as usize]);
        segment_at(a, b, x, z)
    }

    /// The sites' inputs for adoption, in canonical order: per site the footprint's corner count
    /// and bits, the support height's bits, the surface count and per surface its material, corner
    /// count and bits. Exact, compared as words.
    pub(super) fn inputs(&self, out: &mut Vec<u32>) {
        out.clear();
        let bits = |points: &[Vector2], out: &mut Vec<u32>| {
            out.push(points.len() as u32);
            out.extend(points.iter().flat_map(|p| [p.x.to_bits(), p.y.to_bits()]));
        };
        for rank in 0..self.sites.len() {
            let site = self.ranked(rank);
            bits(self.footprint(site), out);
            out.push(site.height.to_bits());
            out.push(site.surfaces.1 - site.surfaces.0);
            for surface in &self.surfaces[site.surfaces.0 as usize..site.surfaces.1 as usize] {
                out.push(u32::from(material_class(surface.material)));
                bits(
                    &self.points[surface.points.0 as usize..surface.points.1 as usize],
                    out,
                );
            }
        }
    }
}

/// The class word of a material: 1 asphalt, 2 concrete (0 is no surface).
pub(super) fn material_class(material: SiteSurfaceMaterial) -> u8 {
    match material {
        SiteSurfaceMaterial::Asphalt => 1,
        SiteSurfaceMaterial::Concrete => 2,
    }
}

// An edge's endpoints as seam points at height 0.
fn edge_points(edge: &[f64; 4]) -> ([f64; 3], [f64; 3]) {
    ([edge[0], 0.0, edge[1]], [edge[2], 0.0, edge[3]])
}

// The content order of two sites: footprint corners, support height, then surfaces (material
// and corners), by bits. Total over content, so only identical sites tie.
fn compare_sites(
    points: &[Vector2],
    surfaces: &[SurfaceRecord],
    a: &SiteRecord,
    b: &SiteRecord,
) -> Ordering {
    let corners = |(start, end): (u32, u32)| {
        points[start as usize..end as usize]
            .iter()
            .flat_map(|p| [p.x.to_bits(), p.y.to_bits()])
    };
    let surface_list = |site: &SiteRecord| {
        surfaces[site.surfaces.0 as usize..site.surfaces.1 as usize]
            .iter()
            .flat_map(|surface| {
                std::iter::once(u32::from(material_class(surface.material)))
                    .chain(std::iter::once(surface.points.1 - surface.points.0))
                    .chain(corners(surface.points))
            })
    };
    (a.footprint.1 - a.footprint.0)
        .cmp(&(b.footprint.1 - b.footprint.0))
        .then_with(|| corners(a.footprint).cmp(corners(b.footprint)))
        .then_with(|| a.height.to_bits().cmp(&b.height.to_bits()))
        .then_with(|| surface_list(a).cmp(surface_list(b)))
}

// Visits the lattice points of `xs` x `zs` (row-major by `z`, as `(k, x, z, sure)`) the
// polygon `ring` may hold, its box grown by `pad`: `sure` where the point lies inside by the
// even-odd rule along its row and farther than `SPAN_MARGIN_M` from every side, where the
// predicate the caller would test agrees; unsure (to be tested) within that margin of a side;
// outside points farther than it are skipped. A row within the margin of a corner's z visits
// every point of the box unsure, so every side a point there can be near crosses the row
// transversally, and a point is within the margin of a crossing side only if it is within
// `margin / |sin|` of the crossing in x. Unpruned, every point is visited unsure.
// `O(points in the box)` with `O(crossings)` per point.
fn for_each_span_point(
    xs: &[f64],
    zs: &[f64],
    ring: &[[f64; 2]],
    pad: f64,
    prune: bool,
    mut visit: impl FnMut(usize, f64, f64, bool),
) {
    let width = xs.len();
    let area = ring.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |[a, b, c, d], p| [a.min(p[0]), b.min(p[1]), c.max(p[0]), d.max(p[1])],
    );
    if !prune {
        for (j, &z) in zs.iter().enumerate() {
            for (i, &x) in xs.iter().enumerate() {
                visit(j * width + i, x, z, false);
            }
        }
        return;
    }
    let pad = pad.max(SPAN_MARGIN_M);
    let (i0, i1) = (
        xs.partition_point(|&x| x < area[0] - pad),
        xs.partition_point(|&x| x <= area[2] + pad),
    );
    let (j0, j1) = (
        zs.partition_point(|&z| z < area[1] - pad),
        zs.partition_point(|&z| z <= area[3] + pad),
    );
    // Per crossing side: its x on the row and the x distance within which a point may lie within
    // the margin of it. At most a few per row for the footprints and surfaces of sites.
    let mut crossings = [(0.0f64, 0.0f64); 16];
    let n = ring.len();
    for (j, &z) in zs.iter().enumerate().take(j1).skip(j0) {
        let risky = ring.iter().any(|p| (p[1] - z).abs() <= SPAN_MARGIN_M);
        let mut count = 0;
        if !risky {
            for e in 0..n {
                let (a, b) = (ring[e], ring[(e + 1) % n]);
                if (a[1] < z) != (b[1] < z) {
                    if count == crossings.len() {
                        count = usize::MAX;
                        break;
                    }
                    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
                    let x = a[0] + dx * ((z - a[1]) / dz);
                    crossings[count] = (x, SPAN_MARGIN_M * dx.hypot(dz) / dz.abs());
                    count += 1;
                }
            }
        }
        for (i, &x) in xs.iter().enumerate().take(i1).skip(i0) {
            let k = j * width + i;
            if risky || count == usize::MAX {
                visit(k, x, z, false);
                continue;
            }
            let near = crossings[..count]
                .iter()
                .any(|&(cx, margin)| (x - cx).abs() <= margin);
            if near {
                visit(k, x, z, false);
                continue;
            }
            let left = crossings[..count].iter().filter(|&&(cx, _)| cx < x).count();
            if left % 2 == 1 {
                visit(k, x, z, true);
            }
        }
    }
}

// The x range of lattice row `z` within which a point can lie within `band` of the edge: the
// x range of the edge's part within `band` of the row in z, grown by `band`, and padded. A
// point of the row within `band` of the edge has its nearest point there, at most `band` away
// in x. `None` where no part of the edge comes that close in z.
fn band_span(edge: &[f64; 4], band: f64, z: f64) -> Option<(f64, f64)> {
    let [ax, az, bx, bz] = *edge;
    let (lo, hi) = (z - band, z + band);
    let (t0, t1) = if az == bz {
        if az < lo || az > hi {
            return None;
        }
        (0.0, 1.0)
    } else {
        let (ta, tb) = ((lo - az) / (bz - az), (hi - az) / (bz - az));
        (ta.min(tb).max(0.0), ta.max(tb).min(1.0))
    };
    if t0 > t1 {
        return None;
    }
    let (xa, xb) = (ax + (bx - ax) * t0, ax + (bx - ax) * t1);
    let pad = band + SURFACE_PAD_M + 1.0e-9 * (ax.abs().max(bx.abs()));
    Some((xa.min(xb) - pad, xa.max(xb) + pad))
}

fn inside_box(area: [f64; 4], x: f64, z: f64, pad: f64) -> bool {
    x >= area[0] - pad && x <= area[2] + pad && z >= area[1] - pad && z <= area[3] + pad
}

fn bounds_of(points: &[Vector2]) -> [f64; 4] {
    points.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |[min_x, min_z, max_x, max_z], p| {
            let (x, z) = (f64::from(p.x), f64::from(p.y));
            [min_x.min(x), min_z.min(z), max_x.max(x), max_z.max(z)]
        },
    )
}

// Twice the signed area in XZ, `sum(a.x b.z - b.x a.z)`: positive is the orientation the
// material partition keeps.
fn signed_area(ring: &[[f64; 2]]) -> f64 {
    (0..ring.len())
        .map(|i| {
            let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum()
}

// `(b - a) x (p - a)` in XZ, the material partition's side test: exact in sign for corners and
// points representable in `f32` (each difference and product is exact in `f64`).
fn cross(a: [f64; 2], b: [f64; 2], x: f64, z: f64) -> f64 {
    (b[0] - a[0]) * (z - a[1]) - (b[1] - a[1]) * (x - a[0])
}

/// Whether the closed polygon `ring` holds the XZ point `(x, z)`: on a side counts as inside,
/// as the material partition keeps a point with `cross >= 0` in a region. Elsewhere the
/// crossing number of the ray towards `+x`, each crossing decided by the side's `cross` sign
/// alone (no division), so the test is exact wherever `cross` is. `O(corners)`.
pub(super) fn polygon_holds(ring: &[[f64; 2]], x: f64, z: f64) -> bool {
    let n = ring.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        let side = cross(a, b, x, z);
        if side == 0.0
            && x >= a[0].min(b[0])
            && x <= a[0].max(b[0])
            && z >= a[1].min(b[1])
            && z <= a[1].max(b[1])
        {
            return true;
        }
        // A side crossing the ray's line (half-open in z), right of the point: for an upward
        // side the crossing lies at larger x exactly when the point is left of it.
        if (a[1] <= z) != (b[1] <= z) && ((b[1] > a[1]) == (side > 0.0)) {
            inside = !inside;
        }
    }
    inside
}

/// A seam whose XZ box is `bounds` (`[min_x, min_z, max_x, max_z]`) and whose height is
/// `height` along it: from one corner of the box to the other. The selection's tests read a
/// seam's box and height range only, so this stands for every seam lying in the box at that
/// height (a changed site's pad seams), each of whose tests it passes too.
pub(super) fn box_seam(bounds: [f64; 4], height: f64) -> Seam {
    Seam {
        a: [bounds[0], height, bounds[1]],
        b: [bounds[2], height, bounds[3]],
        kind: SeamKind::Pad,
    }
}
