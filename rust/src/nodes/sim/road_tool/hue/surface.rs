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
//! Cost per pose (`ROAD-123`): `O(C + F log F + F x 25 + V + L log L + R)` for the `C` cells the
//! top triangles' boxes meet, the `F` fragments, the `V` output vertices, the `L` points of
//! lifted fragments (a vertex searches them only beside a lifted cell) and the drawn ground's
//! reads `R`: one per lattice point a fragment's bounds name in its cell, made 8 m block by block
//! (the graded reader prepares each block once and evaluates each lattice point once). Nothing
//! outside the draft's pieces is read; every buffer is reused, so a pose allocates nothing once
//! they have grown.

use crate::simulation::network::roadbed::{PieceMesh, RoadVec3};
use crate::simulation::terrain::graded::{GradedSurface, LatticeBlock, QueryScratch};

/// The cell the hue's fill is cut into, metres: the terrain's 2 m cells.
pub(crate) const SURFACE_CELL_M: f64 = 2.0;
/// The drawn ground's lattice spacing, metres: the graded surface's `GL0` lattice, on whose
/// triangles the rendered ground is linear.
pub(crate) const GROUND_LATTICE_M: f64 = 0.5;
// Cells a side of the graded surface's lattice block (`LatticeBlock`, 8 m): the drawn ground is
// read block by block, so a block's window is prepared once a pose.
const BLOCK_CELLS: i64 = 4;
// Lattice points along one cell's side.
const CELL_POINTS: usize = (SURFACE_CELL_M / GROUND_LATTICE_M) as usize + 1;
// Plan positions closer than this share one rise (the same point cut from two fragments).
const KEY_M: f64 = 1.0e-4;
// The spacing of the stroke's samples that decide which pieces are the draft's, metres.
const SAMPLE_M: f64 = 1.0;

// One fragment: a convex polygon of `len` points from `start` in `points`, inside `cell`, on
// the plane `y = plane[0] + plane[1] x + plane[2] z`; `span` the first and last lattice column
// and row of its cell's lattice (`CELL_POINTS` a side) that the `GL0` cells meeting its bounds
// name.
#[derive(Clone, Copy, Debug)]
struct Fragment {
    cell: [i64; 2],
    start: u32,
    len: u32,
    plane: [f64; 3],
    deficit: f64,
    span: [u8; 4],
}

// A triangle clipped by a cell's four sides: at most seven points.
const POLYGON_MAX: usize = 7;

// A convex polygon of `len` points, on the stack.
#[derive(Clone, Copy)]
struct Polygon {
    len: usize,
    points: [[f64; 2]; POLYGON_MAX],
}

impl Polygon {
    fn points(&self) -> &[[f64; 2]] {
        &self.points[..self.len]
    }
}

/// The draft's drawn road surface and the scratch that builds its hue; reused across poses.
#[derive(Debug, Default)]
pub(crate) struct PieceSurface {
    // The draft's pieces' top triangles and curb faces.
    tops: Vec<[RoadVec3; 3]>,
    curbs: Vec<[RoadVec3; 3]>,
    // Scratch: the fragments' plan points, the fragments and the
    // rises by plan key.
    points: Vec<[f64; 2]>,
    fragments: Vec<Fragment>,
    // The fragments' sort: compact keys, and the fragments before it.
    sorted: Vec<(u64, u32)>,
    unsorted: Vec<Fragment>,
    // The cells holding fragments, in cell order; their read order by 8 m block (key, cell
    // slot); each cell's drawn ground at its lattice points.
    cells: Vec<[i64; 2]>,
    masks: Vec<u32>,
    blocks: Vec<(u64, u32)>,
    // Per cell: whether a lifted fragment lies in it or a neighbour (only then can its points
    // share a key with a rise).
    near_lift: Vec<bool>,
    lattices: Vec<[[f64; CELL_POINTS]; CELL_POINTS]>,
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
        // Only the fragments the ground lifts name a rise: a point no lifted fragment shares
        // rises by nothing, the largest deficit of its fragments (all zero), and a lift-free pose
        // sorts and searches nothing (`ROAD-123`).
        self.rises.clear();
        for fragment in self
            .fragments
            .iter()
            .filter(|fragment| fragment.deficit > 0.0)
        {
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
        // Two points share a key only within 0.1 mm, so a point's rise can come only from a
        // lifted fragment in its own cell or a neighbour; elsewhere no search is made.
        self.near_lift.clear();
        self.near_lift.resize(self.cells.len(), false);
        let mut lifted = None;
        for fragment in self
            .fragments
            .iter()
            .filter(|fragment| fragment.deficit > 0.0)
        {
            if lifted == Some(fragment.cell) {
                continue;
            }
            lifted = Some(fragment.cell);
            let [i, j] = fragment.cell;
            for di in -1..=1 {
                for dj in -1..=1 {
                    let near = cell_key([i + di, j + dj]);
                    if let Ok(slot) = self.cells.binary_search_by_key(&near, |&c| cell_key(c)) {
                        self.near_lift[slot] = true;
                    }
                }
            }
        }
        let rises = &self.rises;
        let rise = |point: [f64; 2]| {
            if rises.is_empty() {
                return 0.0;
            }
            rises
                .binary_search_by(|probe| probe.0.cmp(&key(point)))
                .map_or(0.0, |index| rises[index].1)
        };
        let vertex = |point: [f64; 2], height: f64, near: bool| {
            let rise = if near { rise(point) } else { 0.0 };
            godot::prelude::Vector3::new(
                point[0] as f32,
                (height + rise + lift) as f32,
                point[1] as f32,
            )
        };
        let mut slot = 0;
        for (index, fragment) in self.fragments.iter().enumerate() {
            if index > 0 && self.cells[slot] != fragment.cell {
                slot += 1;
            }
            let near = self.near_lift[slot];
            let points = &self.points[fragment.start as usize..][..fragment.len as usize];
            let at = |point: [f64; 2]| vertex(point, on_plane(fragment.plane, point), near);
            for pair in points[1..].windows(2) {
                out.extend([at(points[0]), at(pair[0]), at(pair[1])]);
            }
        }
        for face in &self.curbs {
            out.extend(face.map(|corner| vertex([corner.x, corner.z], corner.y, true)));
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
                    let mut polygon = Polygon {
                        len: 3,
                        points: [[0.0; 2]; POLYGON_MAX],
                    };
                    for (point, corner) in polygon.points.iter_mut().zip(triangle) {
                        *point = [corner.x, corner.z];
                    }
                    let (x0, z0) = (i as f64 * SURFACE_CELL_M, j as f64 * SURFACE_CELL_M);
                    for (axis, bound, above) in [
                        (0, x0, true),
                        (0, x0 + SURFACE_CELL_M, false),
                        (1, z0, true),
                        (1, z0 + SURFACE_CELL_M, false),
                    ] {
                        // A side that keeps every point clips nothing: the polygon as it is.
                        let kept = |point: &[f64; 2]| {
                            if above {
                                point[axis] >= bound
                            } else {
                                point[axis] <= bound
                            }
                        };
                        if polygon.points().iter().all(kept) {
                            continue;
                        }
                        polygon = clip(&polygon, axis, bound, above);
                    }
                    let points = polygon.points();
                    if points.len() < 3 || plan_area(points) <= 1.0e-9 {
                        continue;
                    }
                    self.fragments.push(Fragment {
                        cell: [i, j],
                        start: self.points.len() as u32,
                        len: points.len() as u32,
                        plane,
                        deficit: 0.0,
                        span: lattice_span(points, [x0, z0]),
                    });
                    self.points.extend_from_slice(points);
                }
            }
        }
        // Cell order, so each cell's lattice is read once; ties keep the triangles' order
        // (`start` grows with the fragments' order). Sorted as compact keys, then moved once.
        self.sorted.clear();
        self.sorted.extend(
            (self.fragments.iter().enumerate())
                .map(|(index, fragment)| (cell_key(fragment.cell), index as u32)),
        );
        self.sorted.sort_unstable();
        self.unsorted.clear();
        self.unsorted.append(&mut self.fragments);
        self.fragments.extend(
            self.sorted
                .iter()
                .map(|&(_, index)| self.unsorted[index as usize]),
        );
    }

    // Each fragment's deficit over the drawn ground: per cell its lattice read once, then per
    // fragment the lattice points of the `GL0` cells meeting its bounds. Cells are visited block
    // by block (`ROAD-123`): in cell order a draft running along `z` met each block once per
    // 2 m column, and each visit prepared the block's graded window again.
    fn deficits(&mut self, ground: &mut impl FnMut(f64, f64) -> f64) {
        // The cells holding fragments (in cell order) and the lattice points their fragments'
        // bounds name (a bit each); the points read by 8 m block; then the fragments' deficits
        // in their own order.
        self.cells.clear();
        self.masks.clear();
        for fragment in &self.fragments {
            if self.cells.last() != Some(&fragment.cell) {
                self.cells.push(fragment.cell);
                self.masks.push(0);
            }
            let (columns, rows) = spans(fragment);
            let mask = self.masks.last_mut().expect("the fragment's cell");
            for i in columns {
                for j in rows.clone() {
                    *mask |= 1 << (i * CELL_POINTS + j);
                }
            }
        }
        self.blocks.clear();
        self.blocks
            .extend((self.cells.iter().enumerate()).map(|(slot, cell)| {
                (
                    cell_key(cell.map(|c| c.div_euclid(BLOCK_CELLS))),
                    slot as u32,
                )
            }));
        self.blocks.sort_unstable();
        self.lattices
            .resize(self.cells.len(), [[0.0; CELL_POINTS]; CELL_POINTS]);
        for &(_, slot) in &self.blocks {
            let [x0, z0] = self.cells[slot as usize].map(|c| c as f64 * SURFACE_CELL_M);
            let mask = self.masks[slot as usize];
            for (i, row) in self.lattices[slot as usize].iter_mut().enumerate() {
                for (j, value) in row.iter_mut().enumerate() {
                    if mask & (1 << (i * CELL_POINTS + j)) != 0 {
                        *value = ground(
                            x0 + i as f64 * GROUND_LATTICE_M,
                            z0 + j as f64 * GROUND_LATTICE_M,
                        );
                    }
                }
            }
        }
        let mut slot = 0;
        for (index, fragment) in self.fragments.iter_mut().enumerate() {
            if index > 0 && self.cells[slot] != fragment.cell {
                slot += 1;
            }
            let lattice = &self.lattices[slot];
            let [x0, z0] = fragment.cell.map(|c| c as f64 * SURFACE_CELL_M);
            let (columns, rows) = spans(fragment);
            let mut deficit = 0.0_f64;
            for i in columns {
                for j in rows.clone() {
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

// The first and last lattice column and row (of the cell at `[x0, z0]`) of the `GL0` cells
// meeting a fragment's bounds.
fn lattice_span(points: &[[f64; 2]], [x0, z0]: [f64; 2]) -> [u8; 4] {
    let [min_x, min_z, max_x, max_z] = points.iter().fold(
        [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY],
        |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])],
    );
    let span = |min: f64, max: f64, origin: f64| {
        let first = ((min - origin) / GROUND_LATTICE_M).floor().max(0.0) as usize;
        let last = ((max - origin) / GROUND_LATTICE_M).ceil() as usize;
        [
            first.min(CELL_POINTS - 1) as u8,
            last.min(CELL_POINTS - 1) as u8,
        ]
    };
    let ([first_i, last_i], [first_j, last_j]) = (span(min_x, max_x, x0), span(min_z, max_z, z0));
    [first_i, last_i, first_j, last_j]
}

// A fragment's lattice columns and rows ([`lattice_span`]).
fn spans(
    fragment: &Fragment,
) -> (
    std::ops::RangeInclusive<usize>,
    std::ops::RangeInclusive<usize>,
) {
    let [first_i, last_i, first_j, last_j] = fragment.span.map(usize::from);
    (first_i..=last_i, first_j..=last_j)
}

/// The drawn graded ground (`GradedSurface::ground_height_near`, bitwise the point query) as
/// the hue's `drawn` reader (round HU3), read at the `GL0` lattice points through
/// `GradedSurface::ground_at_lattice` (`ROAD-123`): the block-ordered reads of
/// [`PieceSurface::build`] prepare each 8 m block of the footprint once, raise its coverage
/// once and evaluate each lattice point once, so a pose's read costs
/// `O(blocks x (local pieces and seams + coverage triangles) + lattice points x listed seams)`
/// for the blocks and points under the draft's pieces, never its whole window.
/// Allocation-free once the scratch is warm (the block is a fixed-size array).
pub(crate) fn graded_ground<'a>(
    surface: &'a GradedSurface<'a>,
    scratch: &'a mut QueryScratch,
) -> impl FnMut(f64, f64) -> f64 + 'a {
    let mut block = LatticeBlock::default();
    move |x, z| {
        let [i, j] = [x, z].map(|value| (value / GROUND_LATTICE_M).round() as i64);
        if i as f64 * GROUND_LATTICE_M == x && j as f64 * GROUND_LATTICE_M == z {
            surface.ground_at_lattice([i, j], &mut block, scratch)
        } else {
            surface.ground_height_at(x, z, scratch)
        }
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
// above it when `above`), returned. A crossing is computed from its edge's ends in a fixed
// order, so an edge two polygons share is cut at bitwise the same point from both.
fn clip(input: &Polygon, axis: usize, bound: f64, above: bool) -> Polygon {
    let mut out = Polygon {
        len: 0,
        points: [[0.0; 2]; POLYGON_MAX],
    };
    let mut push = |point: [f64; 2]| {
        out.points[out.len] = point;
        out.len += 1;
    };
    let kept = |point: [f64; 2]| {
        if above {
            point[axis] >= bound
        } else {
            point[axis] <= bound
        }
    };
    let input = input.points();
    let Some(&last) = input.last() else {
        return out;
    };
    let mut previous = last;
    for &current in input {
        if kept(current) != kept(previous) {
            let (a, b) = if (previous[0], previous[1]) <= (current[0], current[1]) {
                (previous, current)
            } else {
                (current, previous)
            };
            let t = (bound - a[axis]) / (b[axis] - a[axis]);
            let mut crossing = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            crossing[axis] = bound;
            push(crossing);
        }
        if kept(current) {
            push(current);
        }
        previous = current;
    }
    out
}

// An order-preserving key of a cell (or block) index pair: `(i, j)` in lexicographic order, for
// indices within `i32` (world coordinates over 2 m).
fn cell_key([i, j]: [i64; 2]) -> u64 {
    let half = |index: i64| u64::from(index as i32 as u32 ^ 0x8000_0000);
    (half(i) << 32) | half(j)
}

// A plan position's key, quantized to `KEY_M`.
fn key(point: [f64; 2]) -> u64 {
    let [x, z] = point.map(|value| (value / KEY_M).round() as i64 as i32 as u32);
    (u64::from(x) << 32) | u64::from(z)
}
