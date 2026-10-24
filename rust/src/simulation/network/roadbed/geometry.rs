// SPDX-License-Identifier: GPL-2.0-only

//! Generic triangle-grid, ray and constrained-triangulation helpers shared with terrain tiles,
//! engineered-ground queries and building-site paving.

use super::{RoadVec2, RoadVec3};
use spade::{ConstrainedDelaunayTriangulation, Point2, Triangulation};

type SurfaceCdt = ConstrainedDelaunayTriangulation<Point2<f64>>;

// Sampling tolerance of barycentric tests and ray hits.
const SAMPLE_EPSILON_M: f64 = 0.001;
// Grid cell padding; the legacy f32 millimetre widened to f64, kept bit for bit.
const GRID_EPSILON_M: f64 = 0.001_f32 as f64;
const QUERY_GRID_BASE_CELL_M: f64 = 4.0;
const QUERY_GRID_MAX_CELLS: usize = 256;

/// Immutable triangle grid owning its triangles, for compiled terrain tiles.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RoadSurfaceTriangleQueryIndex {
    grid: SurfaceTriangleGrid,
    triangles: Vec<[RoadVec3; 3]>,
}

// Bounded owner-local XZ grid listing caller-defined triangle ids per cell (`ROAD-44`, `ROAD-49`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct SurfaceTriangleGrid {
    bounds_xz: [f64; 4],
    cell_size_m: f64,
    width: usize,
    height: usize,
    cell_offsets: Vec<u32>,
    cell_items: Vec<u32>,
}

impl RoadSurfaceTriangleQueryIndex {
    /// Builds the bounded grid for a terrain tile without copying polygon wrappers.
    pub(crate) fn from_ground_triangles(
        triangles: impl IntoIterator<Item = [RoadVec3; 3]>,
    ) -> Self {
        let triangles: Vec<_> = triangles.into_iter().collect();
        Self {
            grid: SurfaceTriangleGrid::from_triangles(triangles.iter().copied(), |triangle_idx| {
                triangle_idx as u32
            }),
            triangles,
        }
    }

    /// Samples the highest ground triangle in the grid cell containing `point`.
    pub(crate) fn sample_ground_height(&self, point: RoadVec2) -> Option<f32> {
        let heights = self.grid.cell_items(point).iter();
        heights
            .filter_map(|&item| triangle_height_at_xz(&self.triangles[item as usize], point))
            .reduce(f32::max)
    }
}

impl SurfaceTriangleGrid {
    // Grids `triangles` in order; `item_id` names the triangle at each position in its cells.
    // Cells start at the base size and double until at most the maximum count remain.
    pub(super) fn from_triangles(
        triangles: impl Iterator<Item = [RoadVec3; 3]> + Clone,
        item_id: impl Fn(usize) -> u32,
    ) -> Self {
        if triangles.clone().next().is_none() {
            return Self::default();
        }
        let bounds_xz = bounds_xz(triangles.clone().flatten());
        let mut cell_size_m = QUERY_GRID_BASE_CELL_M;
        let (mut width, mut height) = query_grid_dimensions(bounds_xz, cell_size_m);
        while width.saturating_mul(height) > QUERY_GRID_MAX_CELLS {
            cell_size_m *= 2.0;
            (width, height) = query_grid_dimensions(bounds_xz, cell_size_m);
        }
        let mut grid = Self {
            bounds_xz,
            cell_size_m,
            width,
            height,
            ..Self::default()
        };
        let cell_count = width.saturating_mul(height);
        let mut counts = vec![0_u32; cell_count];
        for triangle in triangles.clone() {
            for cell_idx in grid.triangle_cells(triangle) {
                counts[cell_idx] += 1;
            }
        }

        grid.cell_offsets = Vec::with_capacity(cell_count + 1);
        grid.cell_offsets.push(0);
        for count in counts {
            let end = grid.cell_offsets.last().copied().unwrap_or(0) + count;
            grid.cell_offsets.push(end);
        }
        grid.cell_items = vec![0_u32; grid.cell_offsets.last().copied().unwrap_or(0) as usize];
        let mut cursors = grid.cell_offsets[..cell_count].to_vec();
        for (triangle_idx, triangle) in triangles.enumerate() {
            let item = item_id(triangle_idx);
            for cell_idx in grid.triangle_cells(triangle) {
                let cursor = &mut cursors[cell_idx];
                grid.cell_items[*cursor as usize] = item;
                *cursor += 1;
            }
        }
        grid
    }

    // Cells, row by row, that a triangle's XZ bounds padded by the grid epsilon touch.
    fn triangle_cells(&self, triangle: [RoadVec3; 3]) -> impl Iterator<Item = usize> + use<> {
        let [min_x, min_z, max_x, max_z] = bounds_xz(triangle);
        let (x0, z0) = self.clamped_cell(min_x - GRID_EPSILON_M, min_z - GRID_EPSILON_M);
        let (x1, z1) = self.clamped_cell(max_x + GRID_EPSILON_M, max_z + GRID_EPSILON_M);
        let width = self.width;
        (z0..=z1).flat_map(move |z| (x0..=x1).map(move |x| z * width + x))
    }

    // Ids in cells touching an XZ rectangle, without allocating. An id may occur in several
    // cells. Cost follows the touched cells and their entries.
    pub(super) fn items_in_bounds(&self, bounds: [f64; 4]) -> impl Iterator<Item = u32> + '_ {
        let intersects = self.width > 0
            && self.height > 0
            && bounds[0] <= bounds[2]
            && bounds[1] <= bounds[3]
            && bounds[0] <= self.bounds_xz[2]
            && bounds[1] <= self.bounds_xz[3]
            && bounds[2] >= self.bounds_xz[0]
            && bounds[3] >= self.bounds_xz[1];
        let (x0, z0) = self.clamped_cell(bounds[0], bounds[1]);
        let (x1, z1) = self.clamped_cell(bounds[2], bounds[3]);
        (z0..=z1)
            .take(if intersects { z1 - z0 + 1 } else { 0 })
            .flat_map(move |z| (x0..=x1).flat_map(move |x| self.items_at(x, z).iter().copied()))
    }

    // Ids in the cells the ray `origin + t * dir` crosses for `t` in `[t0, t1]`, in ray order.
    pub(super) fn items_along_ray(
        &self,
        origin: RoadVec3,
        dir: RoadVec3,
        t0: f64,
        t1: f64,
        mut visit: impl FnMut(u32),
    ) {
        if self.width == 0 || self.height == 0 {
            return;
        }
        let [min_x, min_z, max_x, max_z] = self.bounds_xz;
        let Some((entry_t, exit_t)) = ray_xz_interval_for_bounds(
            origin,
            dir,
            min_x - GRID_EPSILON_M,
            min_z - GRID_EPSILON_M,
            max_x + GRID_EPSILON_M,
            max_z + GRID_EPSILON_M,
        ) else {
            return;
        };
        let (t0, t1) = (t0.max(entry_t), t1.min(exit_t));
        if t0 > t1 {
            return;
        }
        walk_ray_cells(
            RoadVec2::new(origin.x, origin.z),
            RoadVec2::new(dir.x, dir.z),
            (t0, t1),
            RoadVec2::new(min_x, min_z),
            self.cell_size_m,
            |x, z, _, _| {
                let x = x.clamp(0, self.width as i64 - 1) as usize;
                let z = z.clamp(0, self.height as i64 - 1) as usize;
                for &item in self.items_at(x, z) {
                    visit(item);
                }
                true
            },
        );
    }

    fn clamped_cell(&self, x: f64, z: f64) -> (usize, usize) {
        let cell = |value: f64, origin: f64, count: usize| {
            (((value - origin) / self.cell_size_m).floor() as isize)
                .clamp(0, count.saturating_sub(1) as isize) as usize
        };
        (
            cell(x, self.bounds_xz[0], self.width),
            cell(z, self.bounds_xz[1], self.height),
        )
    }

    fn items_at(&self, x: usize, z: usize) -> &[u32] {
        let cell_idx = z * self.width + x;
        &self.cell_items
            [self.cell_offsets[cell_idx] as usize..self.cell_offsets[cell_idx + 1] as usize]
    }

    // Triangle ids listed in the cell containing `point`; empty outside the grid.
    pub(super) fn cell_items(&self, point: RoadVec2) -> &[u32] {
        if self.width == 0
            || self.height == 0
            || point.x < self.bounds_xz[0] - GRID_EPSILON_M
            || point.y < self.bounds_xz[1] - GRID_EPSILON_M
            || point.x > self.bounds_xz[2] + GRID_EPSILON_M
            || point.y > self.bounds_xz[3] + GRID_EPSILON_M
        {
            return &[];
        }
        let (x, z) = self.clamped_cell(point.x, point.y);
        self.items_at(x, z)
    }
}

fn query_grid_dimensions(bounds_xz: [f64; 4], cell_size_m: f64) -> (usize, usize) {
    let width = (((bounds_xz[2] - bounds_xz[0]) / cell_size_m).floor() as usize + 1).max(1);
    let height = (((bounds_xz[3] - bounds_xz[1]) / cell_size_m).floor() as usize + 1).max(1);
    (width, height)
}

/// XZ bounds `[min_x, min_z, max_x, max_z]` of `points` by `f64::min` and `f64::max`; inverted
/// infinities when there are none.
pub(super) fn bounds_xz(points: impl IntoIterator<Item = RoadVec3>) -> [f64; 4] {
    let (mut min, mut max) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for point in points {
        min = [min[0].min(point.x), min[1].min(point.z)];
        max = [max[0].max(point.x), max[1].max(point.z)];
    }
    [min[0], min[1], max[0], max[1]]
}

/// Visits the cells of a uniform XZ grid that the ray `origin + t * dir` crosses for `t` in
/// `[t0, t1]`, in ray order, with each cell's parameter interval, until `visit` returns false.
/// Cell `(x, z)` spans `grid_origin + (x, z) * cell_m`. `O(cells crossed)`.
pub(super) fn walk_ray_cells(
    origin: RoadVec2,
    dir: RoadVec2,
    (t0, t1): (f64, f64),
    grid_origin: RoadVec2,
    cell_m: f64,
    mut visit: impl FnMut(i64, i64, f64, f64) -> bool,
) {
    let start = origin + dir * t0;
    let mut cell_x = ((start.x - grid_origin.x) / cell_m).floor() as i64;
    let mut cell_z = ((start.y - grid_origin.y) / cell_m).floor() as i64;
    let axis = |direction: f64, origin: f64, grid_origin: f64, cell: i64| {
        if direction == 0.0 {
            return (0, f64::INFINITY, f64::INFINITY);
        }
        let step = if direction > 0.0 { 1 } else { -1 };
        let boundary = grid_origin + (cell + i64::from(direction > 0.0)) as f64 * cell_m;
        (
            step,
            (boundary - origin) / direction,
            cell_m / direction.abs(),
        )
    };
    let (step_x, mut next_x, delta_x) = axis(dir.x, origin.x, grid_origin.x, cell_x);
    let (step_z, mut next_z, delta_z) = axis(dir.y, origin.y, grid_origin.y, cell_z);
    let mut t = t0;
    loop {
        let exit = next_x.min(next_z).min(t1);
        if !visit(cell_x, cell_z, t, exit) || exit >= t1 {
            return;
        }
        if next_x < next_z {
            cell_x += step_x;
            t = next_x;
            next_x += delta_x;
        } else {
            cell_z += step_z;
            t = next_z;
            next_z += delta_z;
        }
    }
}

// Height of `triangle` at `point` by barycentric weights in XZ, within the sampling tolerance.
pub(super) fn triangle_height_at_xz(triangle: &[RoadVec3; 3], point: RoadVec2) -> Option<f32> {
    let min_x = triangle[0].x.min(triangle[1].x).min(triangle[2].x) - SAMPLE_EPSILON_M;
    let max_x = triangle[0].x.max(triangle[1].x).max(triangle[2].x) + SAMPLE_EPSILON_M;
    let min_z = triangle[0].z.min(triangle[1].z).min(triangle[2].z) - SAMPLE_EPSILON_M;
    let max_z = triangle[0].z.max(triangle[1].z).max(triangle[2].z) + SAMPLE_EPSILON_M;
    if point.x < min_x || point.x > max_x || point.y < min_z || point.y > max_z {
        return None;
    }

    let area = (triangle[1].x - triangle[0].x) * (triangle[2].z - triangle[0].z)
        - (triangle[1].z - triangle[0].z) * (triangle[2].x - triangle[0].x);
    if area.abs() <= SAMPLE_EPSILON_M {
        return None;
    }

    let numerator_w0 = (triangle[1].x - point.x) * (triangle[2].z - point.y)
        - (triangle[1].z - point.y) * (triangle[2].x - point.x);
    let numerator_w1 = (triangle[2].x - point.x) * (triangle[0].z - point.y)
        - (triangle[2].z - point.y) * (triangle[0].x - point.x);
    let numerator_w2 = area - numerator_w0 - numerator_w1;
    let tolerance = -SAMPLE_EPSILON_M * area;
    let outside = if area > 0.0 {
        numerator_w0 < tolerance || numerator_w1 < tolerance || numerator_w2 < tolerance
    } else {
        numerator_w0 > tolerance || numerator_w1 > tolerance || numerator_w2 > tolerance
    };
    if outside {
        return None;
    }
    let inverse_area = area.recip();
    let (wa, wb, wc) = (
        numerator_w0 * inverse_area,
        numerator_w1 * inverse_area,
        numerator_w2 * inverse_area,
    );
    Some((triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc) as f32)
}

/// Intersects a forward ray with an XZ rectangle; shared by road and engineered-ground queries.
pub(crate) fn ray_xz_interval_for_bounds(
    ray_origin: RoadVec3,
    ray_dir: RoadVec3,
    min_x: f64,
    min_z: f64,
    max_x: f64,
    max_z: f64,
) -> Option<(f64, f64)> {
    let mut entry_t = f64::NEG_INFINITY;
    let mut exit_t = f64::INFINITY;
    let axes = [
        (ray_origin.x, ray_dir.x, min_x..=max_x),
        (ray_origin.z, ray_dir.z, min_z..=max_z),
    ];
    for (origin, direction, range) in axes {
        // A parallel axis keeps the interval if the ray runs inside its slab.
        if direction.abs() <= f64::EPSILON {
            if !range.contains(&origin) {
                return None;
            }
            continue;
        }
        let t0 = (range.start() - origin) / direction;
        let t1 = (range.end() - origin) / direction;
        entry_t = entry_t.max(t0.min(t1));
        exit_t = exit_t.min(t0.max(t1));
    }
    (exit_t >= entry_t && exit_t >= 0.0).then_some((entry_t, exit_t))
}

/// Two-sided triangle hit parameter, shared by the visible road and terrain carriers.
pub(crate) fn road_ray_triangle_intersection_t(
    triangle: [RoadVec3; 3],
    ray_origin: RoadVec3,
    ray_dir: RoadVec3,
) -> Option<f64> {
    let edge_ab = triangle[1] - triangle[0];
    let edge_ac = triangle[2] - triangle[0];
    let pvec = ray_dir.cross(edge_ac);
    let det = edge_ab.dot(pvec);
    if det.abs() <= SAMPLE_EPSILON_M {
        return None;
    }

    let inv_det = 1.0 / det;
    let tvec = ray_origin - triangle[0];
    let u = tvec.dot(pvec) * inv_det;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }

    let qvec = tvec.cross(edge_ab);
    let v = ray_dir.dot(qvec) * inv_det;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }

    let t = edge_ac.dot(qvec) * inv_det;
    (t >= 0.0).then_some(t)
}

/// Triangulates an outer boundary and holes for road and building-site polygons.
pub(crate) fn triangulate_constrained_rings_xz(
    rings: &[impl AsRef<[RoadVec3]>],
) -> Option<Vec<[RoadVec3; 3]>> {
    if rings.is_empty() || rings.iter().any(|ring| ring.as_ref().len() < 3) {
        return None;
    }
    let points_world: Vec<_> = rings
        .iter()
        .flat_map(|ring| ring.as_ref().iter().copied())
        .collect();

    let mut constraints = Vec::with_capacity(points_world.len());
    let mut offset = 0;
    for ring in rings {
        let len = ring.as_ref().len();
        constraints.extend((0..len).map(|i| [offset + i, offset + (i + 1) % len]));
        offset += len;
    }
    let vertices = points_world
        .iter()
        .map(|point| Point2::new(point.x, point.z))
        .collect();
    let mut invalid_constraints = 0usize;
    let cdt = SurfaceCdt::try_bulk_load_cdt(vertices, constraints, |_| {
        invalid_constraints += 1;
    })
    .ok()?;
    if invalid_constraints > 0 {
        return None;
    }
    // Ground must close even sub-centimetre residuals: the road renderer's skinny-triangle
    // rejection is inappropriate for a terrain cutout. CDT constraints make the interior
    // centroid test exact; no boundary-distance halo is used for hole inclusion.
    let contains = |point: RoadVec2| {
        polygon_contains_point_xz(rings[0].as_ref(), point)
            && !rings[1..]
                .iter()
                .any(|ring| polygon_contains_point_xz(ring.as_ref(), point))
    };
    let triangles: Vec<_> = cdt
        .inner_faces()
        .map(|face| {
            face.vertices()
                .map(|vertex| points_world[vertex.fix().index()])
        })
        .filter(|triangle| {
            let centroid = RoadVec2::new(
                (triangle[0].x + triangle[1].x + triangle[2].x) / 3.0,
                (triangle[0].z + triangle[1].z + triangle[2].z) / 3.0,
            );
            triangle_double_area_xz_m2(*triangle) > 0.0 && contains(centroid)
        })
        .collect();
    (!triangles.is_empty()).then_some(triangles)
}

// Even-odd crossing test in XZ over a ring of at least three points; points on the boundary are
// not specially included.
fn polygon_contains_point_xz(points_world: &[RoadVec3], point: RoadVec2) -> bool {
    let mut inside = false;
    for index in 0..points_world.len() {
        let start = points_world[index];
        let end = points_world[(index + 1) % points_world.len()];
        if (start.z > point.y) != (end.z > point.y) {
            let edge_x_at_point_z =
                (end.x - start.x) * (point.y - start.z) / (end.z - start.z) + start.x;
            if point.x < edge_x_at_point_z {
                inside = !inside;
            }
        }
    }
    inside
}

fn triangle_double_area_xz_m2(triangle: [RoadVec3; 3]) -> f64 {
    let [a, b, c] = triangle;
    ((b.x - a.x) * (c.z - a.z) - (b.z - a.z) * (c.x - a.x)).abs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ground_height_comes_from_highest_triangle_regardless_of_winding() {
        let lower = [
            RoadVec3::ZERO,
            RoadVec3::new(4.0, 0.0, 0.0),
            RoadVec3::new(0.0, 0.0, 4.0),
        ];
        for grade in [-0.4, 0.4] {
            let upper = lower.map(|p| RoadVec3::new(p.x, 5.0 + grade * p.x + 0.2 * p.z, p.z));
            for triangle in [upper, [upper[2], upper[1], upper[0]]] {
                let index = RoadSurfaceTriangleQueryIndex::from_ground_triangles([lower, triangle]);
                let point = RoadVec2::new(1.0, 1.0);
                let height = index.sample_ground_height(point).unwrap();
                assert!((height - (5.2 + grade as f32)).abs() < 1e-6);
                assert!(
                    index
                        .sample_ground_height(RoadVec2::new(10.0, 10.0))
                        .is_none()
                );
            }
        }
    }

    #[test]
    fn ground_infill_preserves_holes_and_subcentimetre_residuals() {
        let ring = |min: f64, max: f64| {
            vec![
                RoadVec3::new(min, 0.0, min),
                RoadVec3::new(max, 0.0, min),
                RoadVec3::new(max, 0.0, max),
                RoadVec3::new(min, 0.0, max),
            ]
        };
        for (rings, expected_area) in [
            (vec![ring(0.0, 4.0), ring(1.0, 3.0)], 12.0),
            (
                vec![vec![
                    RoadVec3::new(0.0, 0.0, 0.0),
                    RoadVec3::new(0.005, 0.0, 0.0),
                    RoadVec3::new(0.005, 0.0, 10.0),
                    RoadVec3::new(0.0, 0.0, 10.0),
                ]],
                0.05,
            ),
        ] {
            let triangles = triangulate_constrained_rings_xz(&rings).unwrap();
            let area: f64 = triangles
                .iter()
                .map(|triangle| triangle_double_area_xz_m2(*triangle) * 0.5)
                .sum();
            assert!(
                (area - expected_area).abs() < 1.0e-12,
                "infill area {area} != {expected_area}"
            );
        }
    }
}
