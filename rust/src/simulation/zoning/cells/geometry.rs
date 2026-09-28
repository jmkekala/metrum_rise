// SPDX-License-Identifier: GPL-2.0-only

//! Canonical square lattice geometry and exact positive-area convex footprint predicates.

use glam::DVec2;
use serde::{Deserialize, Serialize};

// Match the road overlay's micrometre coordinate grid. Integer SAT uses i128 products so
// city coordinates never lose their small separating gaps to floating point cancellation.
const COORDINATE_SCALE: f64 = 1_000_000.0;
const DIRECTION_SCALE: f64 = 1_000_000_000_000.0;
const MAX_COORDINATE_M: f64 = 1_000_000_000.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
struct FrameKey {
    axis: [i64; 2],
    phase: [i64; 2],
    cell_micrometres: i64,
}

/// Canonical orientation and phase shared by aligned road-side cell groups.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub(crate) struct GridFrame {
    key: FrameKey,
}

/// Derived basis and metric phase reused while visiting one block; never persistent authority.
pub(super) struct FrameGeometry {
    axis: DVec2,
    normal: DVec2,
    cell: f64,
    phase: DVec2,
}

/// World XZ query envelope; exact footprint tests follow spatial candidate lookup.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CellBounds {
    /// Inclusive minimum world coordinates.
    pub(crate) min: DVec2,
    /// Inclusive maximum world coordinates.
    pub(crate) max: DVec2,
}

impl CellBounds {
    /// Computes the enclosing world envelope without allocating geometry.
    pub(crate) fn from_points(points: impl IntoIterator<Item = DVec2>) -> Self {
        let mut bounds = Self {
            min: DVec2::splat(f64::INFINITY),
            max: DVec2::splat(f64::NEG_INFINITY),
        };
        for point in points {
            bounds.min = bounds.min.min(point);
            bounds.max = bounds.max.max(point);
        }
        bounds
    }

    /// Expands all four sides by the requested nonnegative world distance.
    pub(crate) fn expanded(self, margin: f64) -> Self {
        Self {
            min: self.min - DVec2::splat(margin),
            max: self.max + DVec2::splat(margin),
        }
    }

    /// Includes boundary contact; callers distinguish it from interior overlap when necessary.
    pub(crate) fn intersects(self, other: Self) -> bool {
        self.min.cmple(other.max).all() && self.max.cmpge(other.min).all()
    }

    /// Checks finite ordered bounds before enumerating query chunks.
    pub(crate) fn is_valid(self) -> bool {
        valid_point(self.min) && valid_point(self.max) && self.min.cmple(self.max).all()
    }
}

impl GridFrame {
    /// Validates serialized integer state before it can enter the frame registry.
    pub(crate) fn is_valid(self) -> bool {
        let key = self.key;
        if !(1..=DIRECTION_SCALE as i64).contains(&key.axis[0])
            || !(0..DIRECTION_SCALE as i64).contains(&key.axis[1])
            || !(COORDINATE_SCALE as i64..=1_000 * COORDINATE_SCALE as i64)
                .contains(&key.cell_micrometres)
            || key
                .phase
                .iter()
                .any(|&value| value < 0 || value >= key.cell_micrometres)
        {
            return false;
        }
        // Rounding a unit vector componentwise moves its length by at most sqrt(0.5).
        (DVec2::new(key.axis[0] as f64, key.axis[1] as f64).length() - DIRECTION_SCALE).abs()
            <= 0.708
    }

    /// Normalizes quarter-turn-equivalent bases and removes whole-cell translations.
    /// Inputs beyond the supported numeric range or with a degenerate basis are rejected.
    pub(crate) fn new(origin: DVec2, direction: DVec2, cell_m: f64) -> Option<Self> {
        if !valid_point(origin)
            || !direction.is_finite()
            || !direction.length_squared().is_finite()
            || direction.length_squared() < 1e-20
            || !cell_m.is_finite()
            || !(1.0..=1_000.0).contains(&cell_m)
        {
            return None;
        }
        let mut axis = direction.normalize();
        // A square lattice is invariant under quarter turns. Keeping the basis in the
        // first quadrant makes reversed and perpendicular roads identify the same frame.
        for _ in 0..4 {
            if axis.x > 0.0 && axis.y >= 0.0 {
                break;
            }
            axis = DVec2::new(-axis.y, axis.x);
        }
        let mut axis_key = [
            (axis.x * DIRECTION_SCALE).round() as i64,
            (axis.y * DIRECTION_SCALE).round() as i64,
        ];
        if axis_key[0] == 0 {
            axis_key = [axis_key[1], 0];
        }
        let axis = DVec2::new(axis_key[0] as f64, axis_key[1] as f64).normalize();
        let normal = DVec2::new(-axis.y, axis.x);
        let cell = (cell_m * COORDINATE_SCALE).round() as i64;
        let phase = [axis.dot(origin), normal.dot(origin)]
            .map(|value| ((value * COORDINATE_SCALE).round() as i64).rem_euclid(cell));
        Some(Self {
            key: FrameKey {
                axis: axis_key,
                phase,
                cell_micrometres: cell,
            },
        })
    }

    /// Returns the square side length in world metres.
    pub(crate) fn cell_m(self) -> f64 {
        self.key.cell_micrometres as f64 / COORDINATE_SCALE
    }

    /// Rephases an existing basis without independently normalizing its direction again.
    pub(super) fn at_origin(mut self, origin: DVec2) -> Self {
        let (u, v) = self.axes();
        self.key.phase = [u.dot(origin), v.dot(origin)].map(|value| {
            ((value * COORDINATE_SCALE).round() as i64).rem_euclid(self.key.cell_micrometres)
        });
        self
    }

    /// Compares phases only after two sources have explicitly selected the same basis.
    pub(super) fn phase_matches(self, other: Self, precision_m: f64) -> bool {
        if self.key.axis != other.key.axis
            || self.key.cell_micrometres != other.key.cell_micrometres
        {
            return false;
        }
        let delta = self.local(other.world(0.0, 0.0));
        ((delta - delta.round()).abs() * self.cell_m()).max_element() <= precision_m
    }

    /// Returns the canonical orthonormal basis, independent of the road's directed side.
    pub(crate) fn axes(self) -> (DVec2, DVec2) {
        let axis = DVec2::new(self.key.axis[0] as f64, self.key.axis[1] as f64).normalize();
        (axis, DVec2::new(-axis.y, axis.x))
    }

    /// Converts fractional lattice coordinates into world metres before vertex normalization.
    pub(crate) fn world(self, x: f64, y: f64) -> DVec2 {
        let (axis, normal) = self.axes();
        axis * (self.key.phase[0] as f64 / COORDINATE_SCALE + x * self.cell_m())
            + normal * (self.key.phase[1] as f64 / COORDINATE_SCALE + y * self.cell_m())
    }

    /// Converts world metres into fractional coordinates of this lattice.
    pub(crate) fn local(self, point: DVec2) -> DVec2 {
        let (axis, normal) = self.axes();
        (DVec2::new(axis.dot(point), normal.dot(point))
            - DVec2::new(self.key.phase[0] as f64, self.key.phase[1] as f64) / COORDINATE_SCALE)
            / self.cell_m()
    }

    /// Returns the canonical counterclockwise footprint used by both legality and rendering.
    pub(crate) fn corners(self, x: i32, y: i32) -> [DVec2; 4] {
        self.rectangle_corners(f64::from(x), f64::from(y), 1.0, 1.0)
    }

    /// Reuses one normalized basis for a cell, block or lot while evaluating each vertex
    /// independently in lattice coordinates, preserving shared-boundary rounding.
    pub(super) fn rectangle_corners(self, x: f64, y: f64, width: f64, depth: f64) -> [DVec2; 4] {
        self.geometry().rectangle_corners(x, y, width, depth)
    }

    /// Prepares invariant floating-point terms once for a batch of canonical cell footprints.
    pub(super) fn geometry(self) -> FrameGeometry {
        let (axis, normal) = self.axes();
        FrameGeometry {
            axis,
            normal,
            cell: self.cell_m(),
            phase: DVec2::new(self.key.phase[0] as f64, self.key.phase[1] as f64)
                / COORDINATE_SCALE,
        }
    }

    /// Uses closed boundaries; `CellStore` resolves shared-edge picks by stable cell address.
    pub(crate) fn contains(self, x: i32, y: i32, point: DVec2) -> bool {
        convex_contains(&self.corners(x, y), point)
    }
}

impl FrameGeometry {
    /// Evaluates each vertex independently, preserving the canonical shared-boundary arithmetic.
    pub(super) fn rectangle_corners(&self, x: f64, y: f64, width: f64, depth: f64) -> [DVec2; 4] {
        let left = self.phase.x + x * self.cell;
        let right = self.phase.x + (x + width) * self.cell;
        let front = self.phase.y + y * self.cell;
        let back = self.phase.y + (y + depth) * self.cell;
        [
            canonical_point(self.axis * left + self.normal * front),
            canonical_point(self.axis * right + self.normal * front),
            canonical_point(self.axis * right + self.normal * back),
            canonical_point(self.axis * left + self.normal * back),
        ]
    }
}

/// Rejects coordinates outside the finite range supported by canonical integer predicates.
pub(super) fn valid_point(point: DVec2) -> bool {
    point.is_finite() && point.abs().cmple(DVec2::splat(MAX_COORDINATE_M)).all()
}

/// Rounds a world coordinate onto the shared micrometre representation.
pub(super) fn canonical_point(point: DVec2) -> DVec2 {
    (point * COORDINATE_SCALE).round() / COORDINATE_SCALE
}

fn integer_point(point: DVec2) -> [i64; 2] {
    [point.x, point.y].map(|value| (value * COORDINATE_SCALE).round() as i64)
}

/// Tests convex interiors exactly on the canonical coordinate grid. Boundary contact is legal.
pub(crate) fn interiors_overlap(a: &[DVec2], b: &[DVec2]) -> bool {
    if a.len() < 3 || b.len() < 3 || !a.iter().chain(b).copied().all(valid_point) {
        return false;
    }
    for polygon in [a, b] {
        let area: i128 = (0..polygon.len())
            .map(|index| {
                let p = integer_point(polygon[index]);
                let q = integer_point(polygon[(index + 1) % polygon.len()]);
                i128::from(p[0]) * i128::from(q[1]) - i128::from(p[1]) * i128::from(q[0])
            })
            .sum();
        if area == 0 {
            return false;
        }
    }
    for polygon in [a, b] {
        for index in 0..polygon.len() {
            let p = integer_point(polygon[index]);
            let q = integer_point(polygon[(index + 1) % polygon.len()]);
            let axis = [
                i128::from(p[1]) - i128::from(q[1]),
                i128::from(q[0]) - i128::from(p[0]),
            ];
            if axis == [0, 0] {
                continue;
            }
            let project = |vertices: &[DVec2]| {
                vertices
                    .iter()
                    .fold((i128::MAX, i128::MIN), |(min, max), &point| {
                        let point = integer_point(point);
                        let value = i128::from(point[0]) * axis[0] + i128::from(point[1]) * axis[1];
                        (min.min(value), max.max(value))
                    })
            };
            let (amin, amax) = project(a);
            let (bmin, bmax) = project(b);
            if amax <= bmin || bmax <= amin {
                return false;
            }
        }
    }
    true
}

// A road boundary and a lattice frontage are normalized independently. The sum of two
// vertex-rounding errors and one phase-rounding error is below four micrometres; direction
// normalization contributes at most 2 / DIRECTION_SCALE times the world coordinate magnitude.
// This uncertainty applies only to road contact, never to cell/cell ownership.
/// Bounds independent road/lattice coordinate and direction rounding over this world extent.
pub(super) fn road_contact_precision(bounds: CellBounds) -> f64 {
    4.0 / COORDINATE_SCALE
        + 2.0 * bounds.min.abs().max(bounds.max.abs()).max_element() / DIRECTION_SCALE
}

/// Includes the two rounded f32 stages used by road subdivision and surface construction.
/// This representation bound applies to road contact only, never cell/cell or site ownership.
pub(super) fn road_source_precision(bounds: CellBounds) -> f64 {
    let scale = bounds
        .min
        .abs()
        .max(bounds.max.abs())
        .max_element()
        .max((bounds.max - bounds.min).length());
    road_contact_precision(bounds) + 2.0 * f64::from(f32::EPSILON) * scale
}

/// Returns the certain interior for compiled-road contact tests, accounting only for the
/// road representation/coordinate rounding bound. Cell ownership uses the full rectangle.
pub(crate) fn road_contact_interior(corners: &[DVec2; 4]) -> [DVec2; 4] {
    let precision = road_source_precision(CellBounds::from_points(*corners));
    let u = (corners[1] - corners[0]).normalize_or_zero() * precision;
    let v = (corners[3] - corners[0]).normalize_or_zero() * precision;
    [
        corners[0] + u + v,
        corners[1] - u + v,
        corners[2] - u - v,
        corners[3] + u - v,
    ]
}

fn convex_contains(polygon: &[DVec2; 4], point: DVec2) -> bool {
    if !valid_point(point) {
        return false;
    }
    let point = integer_point(point);
    (0..4).all(|index| {
        let a = integer_point(polygon[index]);
        let b = integer_point(polygon[(index + 1) % 4]);
        (i128::from(b[0]) - i128::from(a[0])) * (i128::from(point[1]) - i128::from(a[1]))
            - (i128::from(b[1]) - i128::from(a[1])) * (i128::from(point[0]) - i128::from(a[0]))
            >= 0
    })
}

/// Tests the complete pointer segment against a square, including boundary contact.
pub(super) fn segment_intersects_cell(corners: &[DVec2; 4], start: DVec2, end: DVec2) -> bool {
    if convex_contains(corners, start) || convex_contains(corners, end) {
        return true;
    }
    let cross = |a: [i64; 2], b: [i64; 2], c: [i64; 2]| {
        (i128::from(b[0]) - i128::from(a[0])) * (i128::from(c[1]) - i128::from(a[1]))
            - (i128::from(b[1]) - i128::from(a[1])) * (i128::from(c[0]) - i128::from(a[0]))
    };
    let a = integer_point(start);
    let b = integer_point(end);
    (0..4).any(|index| {
        let c = integer_point(corners[index]);
        let d = integer_point(corners[(index + 1) % 4]);
        let first = [cross(a, b, c), cross(a, b, d)];
        let second = [cross(c, d, a), cross(c, d, b)];
        let straddles = |v: [i128; 2]| (v[0] <= 0 && v[1] >= 0) || (v[1] <= 0 && v[0] >= 0);
        // Collinear lines can be disjoint; retain the segment envelope test as well.
        straddles(first)
            && straddles(second)
            && (0..2).all(|axis| {
                a[axis].min(b[axis]) <= c[axis].max(d[axis])
                    && c[axis].min(d[axis]) <= a[axis].max(b[axis])
            })
    })
}
