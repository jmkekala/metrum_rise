// SPDX-License-Identifier: GPL-2.0-only

//! Portal rows in the graded terrain (`TERRAIN-07-C`, `docs/terrain.md` T22 5, closed by
//! `ROAD-61` round R8): no tie-in cone reaches past a tunnel's portal row, so the hill over the
//! bore stays natural and the approach's cut ends at the row, where the roadbed's headwall
//! stands.
//!
//! A portal row is the tunnel span's end row that a grounded piece shares; the grounded piece's
//! wall run across that mouth holds its vertices, the road on its left and the tunnel on its
//! right (`RoadbedSystem::for_each_tunnel_mouth`). A point lies past the row when it lies
//! strictly right of the line through the row's corners and within the cap of their segment; a
//! seam's cone is hidden from such a point when the seam's nearest point to it lies on that
//! line or left of it. Everything is derived from the grounded pieces a bake or query gathers
//! anyway; the roadbed reads no terrain height.
//!
//! Round R8c: each row also yields its headwall's two wings as [`SeamKind::Wing`] seams, their
//! tops from [`headwall_wing`], the roadbed's own function, so the ground the bake grades to a
//! wing meets the wall's top bitwise.

use super::grade::CAP_M;
use super::prune::box_distance;
use super::seams::{Seam, SeamKind, segment_at};
use crate::simulation::network::roadbed::{HEADWALL_WING_M, PieceId, RoadbedSystem, headwall_wing};

/// The bore clearance `H`, metres: the cover a bore keeps over its deck from portal to portal
/// (`ROAD-61`), the roadbed's headwall height above a portal row's corners.
pub const PORTAL_CLEARANCE_M: f64 = 5.0;
/// The cap of a headwall wing's cone (`ROAD-61` round R8c): the cap less the wing's length, so
/// a wing reaches no point farther than the cap from its row's corners, the reach every bake and
/// query gathers portal rows (and the pieces holding them) over.
pub(super) const WING_CAP_M: f64 = CAP_M - HEADWALL_WING_M;
// Bounds padding, as coverage pads: the margin the dirty-tile selection's `PIECE_PAD_M` assumes.
const PAD_M: f64 = 0.01;

// One portal row: its corner segment's key bits (the road on its left), its corners in XZ, its
// vertices' bits and its headwall's wings (round R8c).
#[derive(Clone, Debug, PartialEq)]
struct PortalRow {
    key: [u64; 6],
    a: [f64; 2],
    b: [f64; 2],
    vertices: Vec<[u64; 3]>,
    wings: [Seam; 2],
}

impl PortalRow {
    // `(p - a) x (b - a)` for the XZ point `p`: positive right of the corner segment, on the
    // tunnel's side. (`(b - a) x (p - a)` is positive on the left, the road's side.)
    fn side(&self, x: f64, z: f64) -> f64 {
        let [ax, az] = self.a;
        let [bx, bz] = self.b;
        (bz - az) * (x - ax) - (bx - ax) * (z - az)
    }

    // Whether the XZ point lies past the row: strictly on the tunnel's side of its line, within
    // the cap of its corner segment.
    fn past(&self, x: f64, z: f64) -> bool {
        self.side(x, z) > 0.0 && {
            let [ax, az] = self.a;
            let [bx, bz] = self.b;
            segment_at([ax, 0.0, az], [bx, 0.0, bz], x, z).distance <= CAP_M
        }
    }

    // Whether the XZ point projects onto the row's line strictly between its corners.
    fn between_corners(&self, x: f64, z: f64) -> bool {
        let [ax, az] = self.a;
        let [bx, bz] = self.b;
        let (dx, dz) = (bx - ax, bz - az);
        let t = (x - ax) * dx + (z - az) * dz;
        t > 0.0 && t < dx * dx + dz * dz
    }

    // Whether `seam` is one of this row's wings and the XZ point lies over its bore: past the
    // row, between its corners (round R8c). The hill over a bore stays natural: no wing grades
    // it, as no cone of the approach does.
    fn wing_over_bore(&self, seam: &Seam, x: f64, z: f64) -> bool {
        self.wings.contains(seam) && self.past(x, z) && self.between_corners(x, z)
    }
}

/// The portal rows within the cap of one lattice rectangle or query window, sorted by their
/// corner segments' bits and unique, so the window is a function of the geometry alone.
#[derive(Default)]
pub(super) struct PortalWindow {
    rows: Vec<PortalRow>,
}

fn bits(point: [f64; 3]) -> [u64; 3] {
    point.map(f64::to_bits)
}

impl PortalWindow {
    /// Gathers the portal rows of the grounded pieces `pieces` (gathered for a window holding
    /// `rect` grown by the cap) whose corner segment lies within the cap of the lattice
    /// rectangle `rect`. `O(runs + R log R + V)` for the pieces' wall runs, the rows kept and
    /// their vertices.
    pub(super) fn gather(&mut self, roadbed: &RoadbedSystem, rect: [f64; 4], pieces: &[PieceId]) {
        self.rows.clear();
        let rows = &mut self.rows;
        roadbed.for_each_tunnel_mouth(pieces, |positions, run| {
            let (Some(&first), Some(&last)) = (run.first(), run.last()) else {
                return;
            };
            let (a, b) = (positions[first as usize], positions[last as usize]);
            let area = [a.x.min(b.x), a.z.min(b.z), a.x.max(b.x), a.z.max(b.z)];
            if box_distance(area, rect) > CAP_M + PAD_M {
                return;
            }
            let [ax, ay, az] = bits(a.to_array());
            let [bx, by, bz] = bits(b.to_array());
            let wing = |corner, other| {
                let [top, tip] = headwall_wing(corner, other);
                Seam {
                    a: top.to_array(),
                    b: tip.to_array(),
                    kind: SeamKind::Wing,
                }
            };
            rows.push(PortalRow {
                key: [ax, ay, az, bx, by, bz],
                a: [a.x, a.z],
                b: [b.x, b.z],
                vertices: run
                    .iter()
                    .map(|&v| bits(positions[v as usize].to_array()))
                    .collect(),
                wings: [wing(a, b), wing(b, a)],
            });
        });
        self.rows.sort_unstable_by_key(|row| row.key);
        self.rows.dedup_by(|p, q| p.key == q.key);
    }

    /// The window itself when it holds a row, for the evaluations that hide cones past rows.
    pub(super) fn active(&self) -> Option<&Self> {
        (!self.rows.is_empty()).then_some(self)
    }

    /// Appends the rows' headwall wings ([`SeamKind::Wing`]) to `seams`, two per row in row
    /// order. `O(rows)`.
    pub(super) fn push_wings(&self, seams: &mut Vec<Seam>) {
        seams.extend(self.rows.iter().flat_map(|row| row.wings));
    }

    /// Whether the XZ point lies past some row of the window. `O(rows)`.
    pub(super) fn past(&self, x: f64, z: f64) -> bool {
        self.rows.iter().any(|row| row.past(x, z))
    }

    /// Whether `seam`'s cone is hidden from the XZ point `(x, z)`, which lies past some row
    /// ([`Self::past`]): its nearest point to the point, `Seam::at`'s exact clamp, lies on the
    /// road's side of the line of a row the point lies past, or on that line. A headwall's wing
    /// (round R8c) grades both sides of its plane beside the bore and is hidden only over its own
    /// row's bore, between the row's corners. `O(rows)`.
    pub(super) fn hides(&self, seam: &Seam, x: f64, z: f64) -> bool {
        if seam.kind == SeamKind::Wing {
            return self.rows.iter().any(|row| row.wing_over_bore(seam, x, z));
        }
        let (qx, qz) = nearest_point(seam, x, z);
        self.rows
            .iter()
            .any(|row| row.past(x, z) && row.side(qx, qz) <= 0.0)
    }

    /// Whether `seam`'s cone is hidden from a covered point `(x, z)` that continues the ground
    /// across a structure mouth (`grade::continues_ground`) within `band` of a portal row's
    /// corner segment: the point takes the ground the row's far side has, so the hill over the
    /// bore meets the row at its own height and the step down to the deck lies over the
    /// approach, where the mask discards it. Its nearest point lies on the road's side of that
    /// row's line, or on it; a headwall's wing when the point lies between that row's corners,
    /// where it continues the hill over the bore, which no wing grades. `O(rows)`.
    pub(super) fn hides_at_mouth(&self, seam: &Seam, x: f64, z: f64, band: f64) -> bool {
        if seam.kind == SeamKind::Wing {
            return self.rows.iter().any(|row| {
                let ([ax, az], [bx, bz]) = (row.a, row.b);
                row.wings.contains(seam)
                    && row.between_corners(x, z)
                    && segment_at([ax, 0.0, az], [bx, 0.0, bz], x, z).distance < band
            });
        }
        let (qx, qz) = nearest_point(seam, x, z);
        self.rows.iter().any(|row| {
            let ([ax, az], [bx, bz]) = (row.a, row.b);
            segment_at([ax, 0.0, az], [bx, 0.0, bz], x, z).distance < band
                && row.side(qx, qz) <= 0.0
        })
    }

    /// Whether a wall seam lies on a portal row: both its ends are vertices of one row, bitwise
    /// (a grounded piece's wall run across a tunnel's mouth takes the row's own vertices).
    pub(super) fn opens(&self, wall: &Seam) -> bool {
        let ends = [wall.a, wall.b].map(|p| p.map(f64::to_bits));
        self.rows
            .iter()
            .any(|row| ends.iter().all(|end| row.vertices.contains(end)))
    }

    /// Appends the tile inputs: each row's corner segment key, after the wall seams'.
    pub(super) fn inputs(&self, seams: &mut Vec<[u64; 6]>) {
        seams.extend(self.rows.iter().map(|row| row.key));
    }

    /// Whether two windows hold the same rows, bitwise.
    pub(super) fn same(&self, other: &Self) -> bool {
        self.rows == other.rows
    }
}

// The nearest point of `seam` to the XZ point `(x, z)`, by `Seam::at`'s exact clamp.
fn nearest_point(seam: &Seam, x: f64, z: f64) -> (f64, f64) {
    let [ax, _, az] = seam.a;
    let [bx, _, bz] = seam.b;
    let (dx, dz) = (bx - ax, bz - az);
    let len2 = dx * dx + dz * dz;
    if len2 > 0.0 {
        let t = ((x - ax) * dx + (z - az) * dz) / len2;
        if t <= 0.0 {
            (ax, az)
        } else if t >= 1.0 {
            (bx, bz)
        } else {
            (ax + dx * t, az + dz * t)
        }
    } else {
        (ax, az)
    }
}
