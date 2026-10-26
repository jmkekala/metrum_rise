// SPDX-License-Identifier: GPL-2.0-only

//! Seams (`TERRAIN-07`): the segments the graded terrain ties in to, their canonical keys, and
//! one seam's nearest point, distance, height and side at a world point.

use crate::simulation::network::roadbed::{PieceId, RoadVec3, RoadbedSystem};

/// What a seam bounds. Every kind ties in at the slope ratio
/// [`super::grade::TIE_IN_SLOPE`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SeamKind {
    /// One grounded outer-chain segment of the roadbed, road on its left.
    Road,
    /// One side of a building site's support footprint at its support height, oriented
    /// counter-clockwise (positive signed area in XZ), the pad on its left (`TERRAIN-07-C`).
    Pad,
    /// One segment of a wall run across a structure mouth (`TERRAIN-07-C`, `docs/terrain.md`
    /// T22 4 (a)), the grounded road on its left: no cone, never a sample's nearest seam; the
    /// bake builds wall faces along it from its height to the graded ground.
    Wall,
}

/// One seam segment in world metres `[x, height, z]`, its height linear along it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seam {
    /// Start point.
    pub a: [f64; 3],
    /// End point.
    pub b: [f64; 3],
    /// What the seam bounds.
    pub kind: SeamKind,
}

impl Seam {
    /// Canonical key: the bits of `a` then `b`. Never a piece, edge or building id, so a split
    /// that renumbers pieces leaves the keys of unchanged geometry equal.
    pub fn key(&self) -> [u64; 6] {
        let ([ax, ay, az], [bx, by, bz]) = (self.a, self.b);
        [ax, ay, az, bx, by, bz].map(f64::to_bits)
    }

    /// XZ bounds `[min_x, min_z, max_x, max_z]`.
    pub fn box_xz(&self) -> [f64; 4] {
        [
            self.a[0].min(self.b[0]),
            self.a[2].min(self.b[2]),
            self.a[0].max(self.b[0]),
            self.a[2].max(self.b[2]),
        ]
    }

    /// Lowest and highest height along the seam.
    pub(super) fn height_range(&self) -> (f64, f64) {
        (self.a[1].min(self.b[1]), self.a[1].max(self.b[1]))
    }

    /// The seam at the world XZ point `(x, z)`, in `f64` with exact clamps: the nearest point is
    /// `a` or `b` itself where the projection parameter leaves `[0, 1]`.
    pub(super) fn at(&self, x: f64, z: f64) -> SeamAt {
        segment_at(self.a, self.b, x, z)
    }

    /// [`Self::at`]'s distance and height at `N` points `(xs[l], z)` of one lattice row, the
    /// same operations per lane in the same order, so each lane equals `at` bitwise. The clamps
    /// are selects rather than branches, so the lanes compile to vector instructions. `O(N)`.
    pub(super) fn at_lanes<const N: usize>(&self, xs: &[f64; N], z: f64) -> ([f64; N], [f64; N]) {
        let [ax, ay, az] = self.a;
        let [bx, by, bz] = self.b;
        let (dx, dz) = (bx - ax, bz - az);
        let len2 = dx * dx + dz * dz;
        let (mut distance, mut height) = ([0.0; N], [ay; N]);
        if len2 > 0.0 {
            for lane in 0..N {
                let x = xs[lane];
                let t = ((x - ax) * dx + (z - az) * dz) / len2;
                let (mid_x, mid_z, mid_h) = (ax + dx * t, az + dz * t, ay + (by - ay) * t);
                let (qx, qz, h) = if t <= 0.0 {
                    (ax, az, ay)
                } else if t >= 1.0 {
                    (bx, bz, by)
                } else {
                    (mid_x, mid_z, mid_h)
                };
                let (ex, ez) = (x - qx, z - qz);
                distance[lane] = (ex * ex + ez * ez).sqrt();
                height[lane] = h;
            }
        } else {
            for lane in 0..N {
                let (ex, ez) = (xs[lane] - ax, z - az);
                distance[lane] = (ex * ex + ez * ez).sqrt();
            }
        }
        (distance, height)
    }
}

/// The segment from `a` to `b` (world `[x, height, z]`) at the world XZ point `(x, z)`, in
/// `f64` with exact clamps: the nearest point is `a` or `b` itself where the projection
/// parameter leaves `[0, 1]`. Seams and material edges share it, and with it the nearest order.
pub(super) fn segment_at(a: [f64; 3], b: [f64; 3], x: f64, z: f64) -> SeamAt {
    let [ax, ay, az] = a;
    let [bx, by, bz] = b;
    let (dx, dz) = (bx - ax, bz - az);
    let len2 = dx * dx + dz * dz;
    let (qx, qz, h, cross) = if len2 > 0.0 {
        let t = ((x - ax) * dx + (z - az) * dz) / len2;
        // `|(b - a) x (p - a)|`: over the length, the distance to the seam's line.
        let cross = (dx * (z - az) - dz * (x - ax)).abs();
        if t <= 0.0 {
            (ax, az, ay, cross)
        } else if t >= 1.0 {
            (bx, bz, by, cross)
        } else {
            (ax + dx * t, az + dz * t, ay + (by - ay) * t, cross)
        }
    } else {
        (ax, az, ay, 0.0)
    };
    let (ex, ez) = (x - qx, z - qz);
    SeamAt {
        distance: (ex * ex + ez * ez).sqrt(),
        height: h,
        cross,
        len2,
    }
}

/// One seam evaluated at a point.
#[derive(Clone, Copy, Debug)]
pub(super) struct SeamAt {
    // Distance from the point to the nearest point of the seam, in XZ.
    pub(super) distance: f64,
    // Seam height at the nearest point.
    pub(super) height: f64,
    // `|(b - a) x (p - a)|` and the squared length, for the line distance, which only a tie in
    // distance reads.
    cross: f64,
    len2: f64,
}

impl SeamAt {
    // Distance from the point to the seam's line (0 for a seam of zero length): the
    // pseudo-normal tie-break at a shared vertex.
    fn line_distance(&self) -> f64 {
        if self.len2 > 0.0 {
            self.cross / self.len2.sqrt()
        } else {
            0.0
        }
    }

    /// The nearest-seam total order with the seam index (window order is key order): smaller
    /// distance, then the larger distance to the seam's line, then the smaller key. Exact
    /// comparisons, no tolerance.
    pub(super) fn nearer_than(&self, index: u32, other: &SeamAt, other_index: u32) -> bool {
        if self.distance != other.distance {
            return self.distance < other.distance;
        }
        let (line, other_line) = (self.line_distance(), other.line_distance());
        if line != other_line {
            return line > other_line;
        }
        index < other_index
    }
}

/// The road seams whose XZ bounds touch `rect`, sorted by key: the grounded outer-chain
/// segments of the roadbed ([`RoadbedSystem::grounded_seams_in_bounds`]). `O(P log P + S log S)`
/// for the pieces and segments met; the scratch buffers are reused.
pub(super) fn gather_road_seams(
    roadbed: &RoadbedSystem,
    rect: [f64; 4],
    scratch: &mut SeamScratch,
    out: &mut Vec<Seam>,
) {
    roadbed.grounded_seams_in_bounds(rect, &mut scratch.pieces, &mut scratch.segments);
    out.clear();
    out.extend(scratch.segments.iter().map(|[a, b]| road_seam(*a, *b)));
}

/// A road seam from two roadbed points.
pub(super) fn road_seam(a: RoadVec3, b: RoadVec3) -> Seam {
    Seam {
        a: a.to_array(),
        b: b.to_array(),
        kind: SeamKind::Road,
    }
}

/// The wall seams whose XZ bounds touch `rect`, sorted by key: the wall-run segments of the
/// grounded pieces the preceding [`gather_road_seams`] over a rectangle holding `rect` found
/// ([`RoadbedSystem::wall_seams_in_bounds`]). `O(V + S log S)` for the pieces' wall-run
/// vertices and the segments kept.
pub(super) fn gather_wall_seams(
    roadbed: &RoadbedSystem,
    rect: [f64; 4],
    scratch: &mut SeamScratch,
    out: &mut Vec<Seam>,
) {
    roadbed.wall_seams_in_bounds(rect, &scratch.pieces, &mut scratch.segments);
    out.clear();
    out.extend(scratch.segments.iter().map(|[a, b]| Seam {
        kind: SeamKind::Wall,
        ..road_seam(*a, *b)
    }));
}

/// Reused buffers of [`gather_road_seams`] and [`gather_wall_seams`].
#[derive(Default)]
pub(super) struct SeamScratch {
    pub(super) pieces: Vec<PieceId>,
    segments: Vec<[RoadVec3; 2]>,
}
