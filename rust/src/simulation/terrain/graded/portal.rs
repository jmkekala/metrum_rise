// SPDX-License-Identifier: GPL-2.0-only

//! The portal opening (`TERRAIN-07-C`, `docs/terrain.md` T22 5 (a)): near a tunnel's portal row
//! the graded terrain over the deck is cut wherever it stands less than the bore clearance `H`
//! above it, so the opening ends where the terrain becomes the bore's crown. Everything is
//! derived from the roadbed's portal rows, deck triangles and bore seams (the tunnel's outer
//! chains) and the finished graded samples; the roadbed reads no terrain height.
//!
//! The cut is a post-pass on final values: a sample's ground, its coverage and its nearest road
//! seam are the same bits pruned and unpruned, so the cut is too.

use super::coverage::{raise_covered, triangle_bits, triangle_box};
use super::grade::{CAP_M, TIE_IN_SLOPE};
use super::prune::box_distance;
use super::sample::NO_SEAM;
use super::seams::{Seam, SeamKind, segment_at};
use super::sites::NO_SITE;
use super::tile::{grown, meets};
use crate::simulation::network::roadbed::{PieceId, PortalPart, RoadVec3, RoadbedSystem};

/// The bore clearance `H`: near a portal row, ground standing less than this above the deck is
/// cut (`docs/terrain.md` T22 5 (a)), metres.
pub const PORTAL_CLEARANCE_M: f64 = 5.0;
/// The portal reach `R = H / k`: within it of a portal row, ground graded at the tie-in slope
/// from the mouth can stay below the bore's crown, so deck coverage counts only there, metres.
pub const PORTAL_REACH_M: f64 = PORTAL_CLEARANCE_M / TIE_IN_SLOPE;
// Bounds padding: wider than the roadbed's sampling tolerance, so every triangle whose
// predicate can accept a point of the rectangle is kept (as coverage pads).
const PAD_M: f64 = 0.01;

/// What the portal opening reads around one lattice rectangle or point: the portal rows within
/// the reach, the portal spans' top triangles meeting the rectangle within the reach of a row,
/// and their bore seams within the band of the rectangle and the reach and band of a row. Each
/// list sorted by its bits and unique, so the window is a function of the geometry alone.
#[derive(Default)]
pub(super) struct PortalWindow {
    // Each row's corner segment `[a, b]` as key bits, and its vertices' bits from `row_starts`.
    rows: Vec<[u64; 6]>,
    row_starts: Vec<u32>,
    row_vertices: Vec<[u64; 3]>,
    triangles: Vec<[u64; 9]>,
    bores: Vec<Seam>,
    // Gather buffers: rows as visited, `(corner key, first, end)` into `gathered_vertices`.
    gathered: Vec<([u64; 6], u32, u32)>,
    gathered_vertices: Vec<[u64; 3]>,
}

fn bits(point: RoadVec3) -> [u64; 3] {
    [point.x, point.y, point.z].map(f64::to_bits)
}

fn triangle_points(bits: &[u64; 9]) -> [RoadVec3; 3] {
    let at = |k: usize| {
        RoadVec3::new(
            f64::from_bits(bits[k]),
            f64::from_bits(bits[k + 1]),
            f64::from_bits(bits[k + 2]),
        )
    };
    [at(0), at(3), at(6)]
}

// The corner segment `[a, b]` of a row key, world `[x, height, z]`.
fn row_segment(key: &[u64; 6]) -> ([f64; 3], [f64; 3]) {
    let v = key.map(f64::from_bits);
    ([v[0], v[1], v[2]], [v[3], v[4], v[5]])
}

fn row_box(key: &[u64; 6]) -> [f64; 4] {
    let (a, b) = row_segment(key);
    [
        a[0].min(b[0]),
        a[2].min(b[2]),
        a[0].max(b[0]),
        a[2].max(b[2]),
    ]
}

impl PortalWindow {
    /// Gathers the portal parts around the lattice rectangle `rect` at a level of band `band`:
    /// the rows within the reach of it, the top triangles meeting it within the reach of a row,
    /// and the bore seams within the band of it and the reach and band of a row. Pieces are found
    /// through the roadbed's query cells over `rect` grown by the larger of the reach and the
    /// band. `O(P log P + T log T + S log S)` for the pieces, triangles and segments met.
    pub(super) fn gather(
        &mut self,
        roadbed: &RoadbedSystem,
        rect: [f64; 4],
        band: f64,
        pieces: &mut Vec<PieceId>,
    ) {
        self.rows.clear();
        self.row_starts.clear();
        self.row_vertices.clear();
        self.triangles.clear();
        self.bores.clear();
        self.gathered.clear();
        self.gathered_vertices.clear();
        let window = grown(rect, PORTAL_REACH_M.max(band) + PAD_M);
        let lattice = grown(rect, PAD_M);
        let Self {
            triangles,
            bores,
            gathered,
            gathered_vertices,
            ..
        } = self;
        roadbed.for_each_portal_part_in_bounds(window, pieces, |part| match part {
            PortalPart::Row(vertices) => {
                let (Some(&a), Some(&b)) = (vertices.first(), vertices.last()) else {
                    return;
                };
                let [ax, ay, az] = bits(a);
                let [bx, by, bz] = bits(b);
                let key = [ax, ay, az, bx, by, bz];
                if box_distance(row_box(&key), rect) <= PORTAL_REACH_M + PAD_M {
                    let first = gathered_vertices.len() as u32;
                    gathered_vertices.extend(vertices.iter().map(|&point| bits(point)));
                    gathered.push((key, first, gathered_vertices.len() as u32));
                }
            }
            PortalPart::Top(triangle) => {
                if meets(triangle_box(&triangle), lattice) {
                    triangles.push(triangle_bits(&triangle));
                }
            }
            PortalPart::Bore([a, b]) => {
                let seam = Seam {
                    a: a.to_array(),
                    b: b.to_array(),
                    kind: SeamKind::Bore,
                };
                if box_distance(seam.box_xz(), rect) <= band + PAD_M {
                    bores.push(seam);
                }
            }
        });
        self.gathered.sort_unstable();
        self.gathered.dedup_by_key(|(key, ..)| *key);
        for &(key, first, end) in &self.gathered {
            self.rows.push(key);
            self.row_starts.push(self.row_vertices.len() as u32);
            self.row_vertices
                .extend_from_slice(&self.gathered_vertices[first as usize..end as usize]);
        }
        self.row_starts.push(self.row_vertices.len() as u32);
        // Only parts within the reach of a row can open or name anything.
        let rows = &self.rows;
        let near_row = |area: [f64; 4], reach: f64| {
            rows.iter()
                .any(|row| box_distance(row_box(row), area) <= reach + PAD_M)
        };
        self.triangles
            .retain(|t| near_row(triangle_box(&triangle_points(t)), PORTAL_REACH_M));
        self.triangles.sort_unstable();
        self.triangles.dedup();
        self.bores
            .retain(|seam| near_row(seam.box_xz(), PORTAL_REACH_M + band));
        self.bores.sort_unstable_by_key(Seam::key);
        self.bores.dedup_by_key(|seam| seam.key());
    }

    /// Whether nothing of a portal was gathered.
    pub(super) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The bore seams, in key order.
    pub(super) fn bores(&self) -> &[Seam] {
        &self.bores
    }

    /// Whether the XZ point lies within the portal reach of a portal row (the row's corner
    /// segment). `O(rows)`.
    pub(super) fn reaches(&self, x: f64, z: f64) -> bool {
        self.rows.iter().any(|row| {
            let (a, b) = row_segment(row);
            segment_at(a, b, x, z).distance <= PORTAL_REACH_M
        })
    }

    /// Whether a wall seam lies on a portal row: both its ends are vertices of one row, bitwise
    /// (a grounded piece's wall run across a tunnel's mouth takes the row's own vertices).
    pub(super) fn opens(&self, wall: &Seam) -> bool {
        let ends = [wall.a, wall.b].map(|p| p.map(f64::to_bits));
        self.row_starts.windows(2).any(|range| {
            let row = &self.row_vertices[range[0] as usize..range[1] as usize];
            ends.iter().all(|end| row.contains(end))
        })
    }

    /// The deck under each lattice point of `xs` x `zs` within the portal reach of a row: the
    /// highest covering portal-span top triangle's height by the roadbed's predicate (road
    /// coverage's raster), `-inf` elsewhere; `out` is left empty without any triangle.
    /// `O(T + K x rows)` for the triangles and the `K` lattice points in their boxes.
    pub(super) fn rasterize(&self, xs: &[f64], zs: &[f64], out: &mut Vec<f32>) {
        out.clear();
        if self.triangles.is_empty() {
            return;
        }
        out.resize(xs.len() * zs.len(), f32::NEG_INFINITY);
        for triangle in &self.triangles {
            raise_covered(&triangle_points(triangle), xs, zs, out);
        }
        let width = xs.len();
        for (k, cell) in out.iter_mut().enumerate() {
            if *cell > f32::NEG_INFINITY && !self.reaches(xs[k % width], zs[k / width]) {
                *cell = f32::NEG_INFINITY;
            }
        }
    }

    /// [`Self::rasterize`] at one world point: the deck there within the reach, else `None`.
    pub(super) fn deck_at(&self, x: f64, z: f64) -> Option<f32> {
        let mut best = f32::NEG_INFINITY;
        for triangle in &self.triangles {
            if let Some(height) =
                RoadbedSystem::top_triangle_height_at(&triangle_points(triangle), x, z)
            {
                best = best.max(height + 0.0);
            }
        }
        (best > f32::NEG_INFINITY && self.reaches(x, z)).then_some(best)
    }

    /// Appends the tile inputs: the portal triangles' bits after the covering ones, and the
    /// portal rows' and bore seams' keys after the wall seams'.
    pub(super) fn inputs(&self, triangles: &mut Vec<[u64; 9]>, seams: &mut Vec<[u64; 6]>) {
        triangles.extend_from_slice(&self.triangles);
        seams.extend_from_slice(&self.rows);
        seams.extend(self.bores.iter().map(Seam::key));
    }

    /// Whether two windows hold the same parts, bitwise.
    pub(super) fn same(&self, other: &Self) -> bool {
        self.rows == other.rows
            && self.row_vertices == other.row_vertices
            && self.row_starts == other.row_starts
            && self.triangles == other.triangles
            && self.bores.len() == other.bores.len()
            && self
                .bores
                .iter()
                .zip(&other.bores)
                .all(|(a, b)| a.key() == b.key())
    }
}

/// Whether a lattice point's ground is cut: covered by a portal deck `deck` within the reach and
/// by no road, with its rendered ground `ground` less than the clearance above the deck.
pub(super) fn cuts(deck: f32, ground: f64) -> bool {
    deck > f32::NEG_INFINITY && ground < f64::from(deck) + PORTAL_CLEARANCE_M
}

/// One finished tile's lattice and rasters, as the opening reads them.
pub(super) struct Finished<'a> {
    pub(super) xs: &'a [f64],
    pub(super) zs: &'a [f64],
    // Road coverage per lattice point (`-inf` uncovered), the site holding it (`NO_SITE`, or
    // empty without sites) and the portal deck ([`PortalWindow::rasterize`], empty without one).
    pub(super) road: &'a [f32],
    pub(super) pads: &'a [u32],
    pub(super) decks: &'a [f32],
    pub(super) band: f64,
}

/// Opens the portals over one finished tile (`docs/terrain.md` T22 5 (a)) in `texels` and
/// `nearest` (indices into `seams`, then `seams.len() + b` for bore seam `b`):
/// - a lattice point with a portal deck `t` and no road coverage whose rendered ground `g` (the
///   base `base(i, j)` plus its packed delta) stands below `t + H` is cut: its delta is kept,
///   its `sdf` is minus its distance to the nearer of its nearest road seam within the band and
///   its nearest bore seam within the band, clamped to the band, and it names that seam;
/// - a ground point (no coverage, no pad) not cut whose nearest bore seam within the band has it
///   on its right (outside the tunnel), within the reach of a portal row and with `g` below the
///   seam's height there plus `H`, takes that seam into its nearest seam the same way, with a
///   positive `sdf`; so the exact test closes the cut's sides on the bore seams, and a point
///   beside a crown never names one (taken pending the user's review, 2026-10-26).
///
/// Every order is the nearest-seam order, bore seams after the window's seams. Returns the cut
/// and named samples. `O(rows x deck points + bore seams x points in their band)`.
pub(super) fn open(
    window: &PortalWindow,
    seams: &[Seam],
    lattice: &Finished<'_>,
    base: impl Fn(usize, usize) -> f64,
    near: &mut Vec<(u32, f64)>,
    texels: &mut [[f32; 4]],
    nearest: &mut [u32],
) -> (u32, u32) {
    let Finished {
        xs,
        zs,
        road,
        pads,
        decks,
        band,
    } = *lattice;
    let width = xs.len();
    // Per lattice point its nearest bore seam within the band, by the nearest-seam order.
    near.clear();
    near.resize(texels.len(), (NO_SEAM, f64::INFINITY));
    for (index, bore) in window.bores.iter().enumerate() {
        let [min_x, min_z, max_x, max_z] = grown(bore.box_xz(), band);
        let (i0, i1) = (
            xs.partition_point(|&x| x < min_x),
            xs.partition_point(|&x| x <= max_x),
        );
        let (j0, j1) = (
            zs.partition_point(|&z| z < min_z),
            zs.partition_point(|&z| z <= max_z),
        );
        for j in j0..j1 {
            for i in i0..i1 {
                let at = bore.at(xs[i], zs[j]);
                if !(at.distance < band && at.distance <= CAP_M) {
                    continue;
                }
                let best = &mut near[j * width + i];
                let nearer = best.0 == NO_SEAM
                    || at.distance < best.1
                    || (at.distance == best.1
                        && at.nearer_than(
                            index as u32,
                            &window.bores[best.0 as usize].at(xs[i], zs[j]),
                            best.0,
                        ));
                if nearer {
                    *best = (index as u32, at.distance);
                }
            }
        }
    }
    let bore_base = seams.len() as u32;
    // The nearer of the point's road seam and bore seam `b`, as `(index, distance)`.
    let nearer = |x: f64, z: f64, road_seam: u32, bore: u32| {
        let bore_at = window.bores[bore as usize].at(x, z);
        if road_seam == NO_SEAM {
            return (bore_base + bore, bore_at.distance);
        }
        let road_at = seams[road_seam as usize].at(x, z);
        if bore_at.nearer_than(bore_base + bore, &road_at, road_seam) {
            (bore_base + bore, bore_at.distance)
        } else {
            (road_seam, road_at.distance)
        }
    };
    let (mut cut, mut named) = (0, 0);
    for (k, texel) in texels.iter_mut().enumerate() {
        let (i, j) = (k % width, k / width);
        let (x, z) = (xs[i], zs[j]);
        let ground = base(i, j) + f64::from(texel[0]);
        let covered = road[k] > f32::NEG_INFINITY;
        let deck = decks.get(k).copied().unwrap_or(f32::NEG_INFINITY);
        let (bore, _) = near[k];
        if !covered && cuts(deck, ground) {
            let road_seam = nearest[k];
            let (index, distance) = if bore != NO_SEAM {
                nearer(x, z, road_seam, bore)
            } else if road_seam != NO_SEAM {
                (road_seam, seams[road_seam as usize].at(x, z).distance)
            } else {
                (NO_SEAM, band)
            };
            texel[1] = -(distance.min(band)) as f32;
            nearest[k] = index;
            cut += 1;
            continue;
        }
        let in_pad = pads.get(k).is_some_and(|&site| site != NO_SITE);
        if covered || in_pad || bore == NO_SEAM {
            continue;
        }
        let seam = &window.bores[bore as usize];
        let at = seam.at(x, z);
        let [ax, _, az] = seam.a;
        let [bx, _, bz] = seam.b;
        // `(b - a) x (p - a)` below zero: the point is right of the seam, outside the tunnel.
        let right = (bx - ax) * (z - az) - (bz - az) * (x - ax) < 0.0;
        if right && window.reaches(x, z) && ground < at.height + PORTAL_CLEARANCE_M {
            let (index, distance) = nearer(x, z, nearest[k], bore);
            if index != nearest[k] {
                texel[1] = distance as f32;
                nearest[k] = index;
                named += 1;
            }
        }
    }
    (cut, named)
}
