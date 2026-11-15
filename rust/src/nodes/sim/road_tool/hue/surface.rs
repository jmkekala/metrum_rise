// SPDX-License-Identifier: GPL-2.0-only

//! The hue over the draft's drawn road pieces (`ROAD-112` round HU2): the top triangles and curb
//! faces of the preview pieces the draft's stroke runs over (carriageway, kerbs, sidewalks,
//! junction corners, end caps), so the hue covers exactly what the preview draws and follows its
//! camber and kerbs.
//!
//! Height rule: every point of the hue lies at least [`super::HUE_LIFT_M`] above both the road
//! surface under it and the drawn ground there (the caller's, read at `GL0` lattice points). The
//! top triangles are cut into fragments by a world grid of [`SURFACE_CELL_M`] cells (Sutherland-
//! Hodgman in plan; a shared edge is cut at bitwise the same points from both sides, so the
//! fragments tile the pieces with no crack). Each fragment lies on its triangle's plane `H`; its
//! deficit `d = max(0, max(G(l) - H(l)))` over the lattice points `l` of the `GL0` cells meeting
//! its bounds (the drawn ground `G` is linear on each `GL0` triangle, so every point of the
//! fragment is a convex combination of those points and `G - H <= d` everywhere on it). A vertex
//! rises by the largest deficit of the fragments that share its plan position, so the mesh stays
//! closed and each fragment rises by at least its own deficit: `hue >= max(H, G) + lift` at every
//! point, not only at the vertices. Curb faces take their corners' rises.
//!
//! Cost per pose: `O(top triangles' cells + fragments x 25 + vertices log vertices)`, one ground
//! query per lattice point of each cell holding a fragment; every buffer is reused.

use crate::simulation::network::roadbed::{PieceMesh, RoadVec3};
use crate::simulation::terrain::graded::{GradedSurface, QueryScratch};

/// The cell the hue's fill is cut into, metres: the terrain's 2 m cells.
pub(crate) const SURFACE_CELL_M: f64 = 2.0;
/// The drawn ground's lattice spacing, metres: the graded surface's `GL0` lattice, on whose
/// triangles the rendered ground is linear.
pub(crate) const GROUND_LATTICE_M: f64 = 0.5;
// The block one graded gather prepares for the drawn ground's reads, metres: four cells a side,
// so a gather lists only the seams near the pose's own cells, never a long draft's whole window.
const GATHER_BLOCK_M: f64 = 4.0 * SURFACE_CELL_M;
// Lattice points along one cell's side.
const CELL_POINTS: usize = (SURFACE_CELL_M / GROUND_LATTICE_M) as usize + 1;
// Plan positions closer than this share one rise (the same point cut from two fragments).
const KEY_M: f64 = 1.0e-4;
// The spacing of the stroke's samples that decide which pieces are the draft's, metres.
const SAMPLE_M: f64 = 1.0;

// One fragment: a convex polygon of `len` points from `start` in `points`, inside `cell`, on
// the plane `y = plane[0] + plane[1] x + plane[2] z`.
#[derive(Clone, Copy, Debug)]
struct Fragment {
    cell: [i64; 2],
    start: u32,
    len: u32,
    plane: [f64; 3],
    deficit: f64,
}

/// The draft's drawn road surface and the scratch that builds its hue; reused across poses.
#[derive(Debug, Default)]
pub(crate) struct PieceSurface {
    // The draft's pieces' top triangles and curb faces.
    tops: Vec<[RoadVec3; 3]>,
    curbs: Vec<[RoadVec3; 3]>,
    // Scratch: the polygon being clipped, the fragments' plan points, the fragments and the
    // rises by plan key.
    polygon: Vec<[f64; 2]>,
    clipped: Vec<[f64; 2]>,
    points: Vec<[f64; 2]>,
    fragments: Vec<Fragment>,
    rises: Vec<(u64, f64)>,
}

impl PieceSurface {
    pub(super) fn clear(&mut self) {
        self.tops.clear();
        self.curbs.clear();
    }

    pub(super) fn is_empty(&self) -> bool {
        self.tops.is_empty()
    }

    /// The plan bounds `[min_x, min_z, max_x, max_z]` of the pushed pieces, `None` without one.
    #[cfg(test)]
    pub(crate) fn bounds(&self) -> Option<[f64; 4]> {
        let mut bounds: Option<[f64; 4]> = None;
        for point in self.tops.iter().flatten() {
            let b = bounds.get_or_insert([point.x, point.z, point.x, point.z]);
            *b = [
                b[0].min(point.x),
                b[1].min(point.z),
                b[2].max(point.x),
                b[3].max(point.z),
            ];
        }
        bounds
    }

    /// Adds the pieces among `pieces` the draft's `path` runs over (a sample every
    /// [`SAMPLE_M`] along it inside one of the piece's top triangles): its spans, its end caps,
    /// its bends and the junctions it makes, never an existing road's span the edit only
    /// rebuilt. `O(samples x pieces' top triangles)` in the worst case, bounds first.
    pub(super) fn push_pieces<'a>(
        &mut self,
        path: &[godot::prelude::Vector3],
        pieces: impl Iterator<Item = &'a PieceMesh>,
    ) {
        for mesh in pieces {
            if !runs_over(mesh, path) {
                continue;
            }
            let top = mesh.top_triangle_count();
            for triangle in 0..top {
                self.tops.push(mesh.triangle_points(triangle));
            }
            for triangle in top..mesh.triangles().len() {
                self.curbs.push(mesh.triangle_points(triangle));
            }
        }
    }

    /// Appends the hue's triangles (each vertex `lift` above its rise) to `out`. `ground` reads
    /// the drawn ground at a lattice point. See the module notes for the rule.
    pub(super) fn build(
        &mut self,
        lift: f64,
        mut ground: impl FnMut(f64, f64) -> f64,
        out: &mut Vec<godot::prelude::Vector3>,
    ) {
        self.cut();
        self.deficits(&mut ground);
        self.rises.clear();
        for fragment in &self.fragments {
            let points = &self.points[fragment.start as usize..][..fragment.len as usize];
            self.rises
                .extend(points.iter().map(|&point| (key(point), fragment.deficit)));
        }
        self.rises.sort_unstable_by_key(|rise| rise.0);
        self.rises.dedup_by(|later, kept| {
            let same = later.0 == kept.0;
            if same {
                kept.1 = kept.1.max(later.1);
            }
            same
        });
        let rises = &self.rises;
        let rise = |point: [f64; 2]| {
            rises
                .binary_search_by(|probe| probe.0.cmp(&key(point)))
                .map_or(0.0, |index| rises[index].1)
        };
        let vertex = |point: [f64; 2], height: f64| {
            godot::prelude::Vector3::new(
                point[0] as f32,
                (height + rise(point) + lift) as f32,
                point[1] as f32,
            )
        };
        for fragment in &self.fragments {
            let points = &self.points[fragment.start as usize..][..fragment.len as usize];
            let at = |point: [f64; 2]| vertex(point, on_plane(fragment.plane, point));
            for pair in points[1..].windows(2) {
                out.extend([at(points[0]), at(pair[0]), at(pair[1])]);
            }
        }
        for face in &self.curbs {
            out.extend(face.map(|corner| vertex([corner.x, corner.z], corner.y)));
        }
    }

    // Cuts every top triangle into its fragments, in cell order. `O(cells the triangles meet)`.
    fn cut(&mut self) {
        self.points.clear();
        self.fragments.clear();
        for triangle in &self.tops {
            let Some(plane) = plane_of(triangle) else {
                continue;
            };
            let [min_x, min_z, max_x, max_z] = triangle.iter().fold(
                [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY],
                |b, p| [b[0].min(p.x), b[1].min(p.z), b[2].max(p.x), b[3].max(p.z)],
            );
            let cells = |min: f64, max: f64| {
                (min / SURFACE_CELL_M).floor() as i64..=(max / SURFACE_CELL_M).floor() as i64
            };
            for i in cells(min_x, max_x) {
                for j in cells(min_z, max_z) {
                    self.polygon.clear();
                    self.polygon.extend(triangle.iter().map(|p| [p.x, p.z]));
                    let (x0, z0) = (i as f64 * SURFACE_CELL_M, j as f64 * SURFACE_CELL_M);
                    for (axis, bound, above) in [
                        (0, x0, true),
                        (0, x0 + SURFACE_CELL_M, false),
                        (1, z0, true),
                        (1, z0 + SURFACE_CELL_M, false),
                    ] {
                        clip(&self.polygon, &mut self.clipped, axis, bound, above);
                        std::mem::swap(&mut self.polygon, &mut self.clipped);
                    }
                    if self.polygon.len() < 3 || plan_area(&self.polygon) <= 1.0e-9 {
                        continue;
                    }
                    self.fragments.push(Fragment {
                        cell: [i, j],
                        start: self.points.len() as u32,
                        len: self.polygon.len() as u32,
                        plane,
                        deficit: 0.0,
                    });
                    self.points.extend_from_slice(&self.polygon);
                }
            }
        }
        // Cell order, so each cell's lattice is read once; ties keep the triangles' order.
        self.fragments
            .sort_unstable_by_key(|fragment| (fragment.cell, fragment.start));
    }

    // Each fragment's deficit over the drawn ground: per cell its lattice read once, then per
    // fragment the lattice points of the `GL0` cells meeting its bounds.
    fn deficits(&mut self, ground: &mut impl FnMut(f64, f64) -> f64) {
        let mut lattice = [[0.0; CELL_POINTS]; CELL_POINTS];
        let mut cell = None;
        for fragment in &mut self.fragments {
            let [x0, z0] = fragment.cell.map(|c| c as f64 * SURFACE_CELL_M);
            if cell != Some(fragment.cell) {
                cell = Some(fragment.cell);
                for (i, row) in lattice.iter_mut().enumerate() {
                    for (j, value) in row.iter_mut().enumerate() {
                        let (x, z) = (
                            x0 + i as f64 * GROUND_LATTICE_M,
                            z0 + j as f64 * GROUND_LATTICE_M,
                        );
                        *value = ground(x, z);
                    }
                }
            }
            let points = &self.points[fragment.start as usize..][..fragment.len as usize];
            let [min_x, min_z, max_x, max_z] = points.iter().fold(
                [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY],
                |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])],
            );
            let span = |min: f64, max: f64, origin: f64| {
                let first = ((min - origin) / GROUND_LATTICE_M).floor().max(0.0) as usize;
                let last = ((max - origin) / GROUND_LATTICE_M).ceil() as usize;
                first.min(CELL_POINTS - 1)..=last.min(CELL_POINTS - 1)
            };
            let mut deficit = 0.0_f64;
            for i in span(min_x, max_x, x0) {
                for j in span(min_z, max_z, z0) {
                    let point = [
                        x0 + i as f64 * GROUND_LATTICE_M,
                        z0 + j as f64 * GROUND_LATTICE_M,
                    ];
                    deficit = deficit.max(lattice[i][j] - on_plane(fragment.plane, point));
                }
            }
            fragment.deficit = deficit;
        }
    }
}

/// The drawn graded ground (`GradedSurface::ground_height_near`, bitwise the point query) as
/// the hue's `drawn` reader (round HU3): one gather per [`GATHER_BLOCK_M`] block, which the
/// cell-ordered reads of [`PieceSurface::build`] meet in runs. Through the view's shared query
/// index a pose's read costs `O(blocks x local seams + lattice points x listed seams)`, about
/// 0.2 ms for a 290 m street (release probe, round HU3); allocation-free once the scratch is warm.
pub(crate) fn graded_ground<'a>(
    surface: &'a GradedSurface<'a>,
    scratch: &'a mut QueryScratch,
) -> impl FnMut(f64, f64) -> f64 + 'a {
    let mut gathered: Option<[f64; 2]> = None;
    move |x, z| {
        let inside = gathered.is_some_and(|[x0, z0]| {
            (x0..=x0 + GATHER_BLOCK_M).contains(&x) && (z0..=z0 + GATHER_BLOCK_M).contains(&z)
        });
        if !inside {
            let [x0, z0] = [x, z].map(|value| (value / GATHER_BLOCK_M).floor() * GATHER_BLOCK_M);
            let half = GATHER_BLOCK_M * 0.5;
            surface.gather_near(x0 + half, z0 + half, half, scratch);
            gathered = Some([x0, z0]);
        }
        surface.ground_height_near(x, z, scratch)
    }
}

// Whether `path` runs over one of `mesh`'s top triangles: a sample every `SAMPLE_M` along it
// (each point and the points between) inside one, in plan.
fn runs_over(mesh: &PieceMesh, path: &[godot::prelude::Vector3]) -> bool {
    let [min_x, min_z, max_x, max_z] = mesh.bounds_xz();
    let holds = |x: f64, z: f64| {
        (min_x..=max_x).contains(&x)
            && (min_z..=max_z).contains(&z)
            && (0..mesh.top_triangle_count())
                .any(|triangle| plan_holds(&mesh.triangle_points(triangle), x, z))
    };
    path.windows(2).any(|pair| {
        let (a, b) = (pair[0], pair[1]);
        let length = f64::from((b.x - a.x).hypot(b.z - a.z));
        let steps = (length / SAMPLE_M).ceil().max(1.0) as usize;
        (0..=steps).any(|step| {
            let t = step as f32 / steps as f32;
            holds(
                f64::from(a.x + (b.x - a.x) * t),
                f64::from(a.z + (b.z - a.z) * t),
            )
        })
    }) || path
        .first()
        .is_some_and(|p| path.len() == 1 && holds(f64::from(p.x), f64::from(p.z)))
}

// Whether plan point `(x, z)` lies in triangle `t` (edges included), either winding.
fn plan_holds(t: &[RoadVec3; 3], x: f64, z: f64) -> bool {
    let side = |a: RoadVec3, b: RoadVec3| (b.x - a.x) * (z - a.z) - (b.z - a.z) * (x - a.x);
    let (d0, d1, d2) = (side(t[0], t[1]), side(t[1], t[2]), side(t[2], t[0]));
    let negative = d0 < 0.0 || d1 < 0.0 || d2 < 0.0;
    let positive = d0 > 0.0 || d1 > 0.0 || d2 > 0.0;
    !(negative && positive)
}

// The plane `y = a + b x + c z` through a triangle, `None` for one without plan area (a face).
fn plane_of([p0, p1, p2]: &[RoadVec3; 3]) -> Option<[f64; 3]> {
    let normal = (*p1 - *p0).cross(*p2 - *p0);
    if normal.y.abs() <= 1.0e-9 {
        return None;
    }
    let (b, c) = (-normal.x / normal.y, -normal.z / normal.y);
    Some([p0.y - b * p0.x - c * p0.z, b, c])
}

fn on_plane(plane: [f64; 3], point: [f64; 2]) -> f64 {
    plane[0] + plane[1] * point[0] + plane[2] * point[1]
}

fn plan_area(polygon: &[[f64; 2]]) -> f64 {
    let first = polygon[0];
    polygon[1..]
        .windows(2)
        .map(|pair| {
            let (a, b) = (pair[0], pair[1]);
            ((a[0] - first[0]) * (b[1] - first[1]) - (a[1] - first[1]) * (b[0] - first[0])) * 0.5
        })
        .sum::<f64>()
        .abs()
}

// One Sutherland-Hodgman step: the part of `input` on the kept side of `axis = bound` (at or
// above it when `above`) into `out`. A crossing is computed from its edge's ends in a fixed
// order, so an edge two polygons share is cut at bitwise the same point from both.
fn clip(input: &[[f64; 2]], out: &mut Vec<[f64; 2]>, axis: usize, bound: f64, above: bool) {
    out.clear();
    let kept = |point: [f64; 2]| {
        if above {
            point[axis] >= bound
        } else {
            point[axis] <= bound
        }
    };
    for (index, &current) in input.iter().enumerate() {
        let previous = input[(index + input.len() - 1) % input.len()];
        if kept(current) != kept(previous) {
            let (a, b) = if (previous[0], previous[1]) <= (current[0], current[1]) {
                (previous, current)
            } else {
                (current, previous)
            };
            let t = (bound - a[axis]) / (b[axis] - a[axis]);
            let mut crossing = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            crossing[axis] = bound;
            out.push(crossing);
        }
        if kept(current) {
            out.push(current);
        }
    }
}

// A plan position's key, quantized to `KEY_M`.
fn key(point: [f64; 2]) -> u64 {
    let [x, z] = point.map(|value| (value / KEY_M).round() as i64 as i32 as u32);
    (u64::from(x) << 32) | u64::from(z)
}
