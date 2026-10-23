// SPDX-License-Identifier: GPL-2.0-only

//! Visible-surface height, band, ray, overlap and lane queries over published pieces.
//!
//! Every query maps XZ to pieces through the fixed query-cell index and then to the triangles
//! that render through the piece's grid: `O(1)` cell lookup plus the triangles of one grid cell
//! for a point, and the cells a ray or rectangle crosses otherwise. Nothing allocates except
//! [`RoadbedSystem::pieces_in_bounds`], which returns a sorted list.

use super::geometry::{
    ray_xz_interval_for_bounds, road_ray_triangle_intersection_t, triangle_height_at_xz,
    walk_ray_cells,
};
use super::index::{QUERY_CELL_M, query_cell_at};
use super::mesh::PieceMesh;
use super::{BandKind, RoadVec2, RoadVec3, RoadbedSystem};
use godot::prelude::Vector3;

// Point-in-bounds tolerance, matching the piece grids' padding.
const BOUNDS_EPSILON_M: f64 = 0.001;

/// Identity of one roadbed piece.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PieceId {
    /// The span piece of an edge.
    Span(usize),
    /// The junction, bend or terminal piece of a node.
    Node(u32),
}

/// Prepared owner-local surface lookup, reused for every point of one lane.
pub struct LaneSurfaceQuery<'a> {
    // The lane owner's span, or a junction lane's node piece.
    owner: Option<&'a PieceMesh>,
    carriageway_only: bool,
}

impl LaneSurfaceQuery<'_> {
    /// Visible height of the lane owner's surface at a world XZ point; carriageway-only for
    /// vehicle lanes. `None` off the owner's pieces.
    pub fn height_at(&self, x: f32, z: f32) -> Option<f32> {
        let point = RoadVec2::new(f64::from(x), f64::from(z));
        let carriageway_only = self.carriageway_only;
        self.owner
            .and_then(|mesh| top_at(mesh, point, |kind| accepts(carriageway_only, kind)))
            .map(|(height_m, _)| height_m)
    }
}

fn accepts(carriageway_only: bool, kind: BandKind) -> bool {
    !carriageway_only || kind == BandKind::Carriageway
}

// Highest accepted top triangle of one piece at `point`, with its band kind.
fn top_at(
    mesh: &PieceMesh,
    point: RoadVec2,
    accept: impl Fn(BandKind) -> bool,
) -> Option<(f32, BandKind)> {
    let mut best: Option<(f32, BandKind)> = None;
    for &item in mesh.grid().cell_items(point) {
        let triangle = item as usize;
        let Some(kind) = mesh.triangle_band(triangle).filter(|&kind| accept(kind)) else {
            continue;
        };
        if let Some(height_m) = triangle_height_at_xz(&mesh.triangle_points(triangle), point)
            && best.is_none_or(|(best_m, _)| height_m > best_m)
        {
            best = Some((height_m, kind));
        }
    }
    best
}

fn bounds_intersect(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3]
}

fn triangle_near_disc(triangle: [RoadVec3; 3], point: RoadVec2, radius: f64) -> bool {
    if triangle_height_at_xz(&triangle, point).is_some() {
        return true;
    }
    let points = triangle.map(|p| RoadVec2::new(p.x, p.z));
    (0..3).any(|i| {
        let a = points[i];
        let ab = points[(i + 1) % 3] - a;
        let t = if ab.length_squared() > 0.0 {
            ((point - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
        } else {
            0.0
        };
        point.distance_squared(a + ab * t) <= radius * radius
    })
}

impl RoadbedSystem {
    /// Height of the highest visible road triangle at a world XZ point.
    pub fn height_at(&self, x: f32, z: f32) -> Option<f32> {
        self.top_at(x, z, |_| true).map(|(height_m, _)| height_m)
    }

    /// Height of the highest visible carriageway triangle at a world XZ point.
    pub fn carriageway_height_at(&self, x: f32, z: f32) -> Option<f32> {
        self.top_at(x, z, |kind| kind == BandKind::Carriageway)
            .map(|(height_m, _)| height_m)
    }

    /// Band kind of the highest visible road triangle at a world XZ point.
    pub fn band_at(&self, x: f32, z: f32) -> Option<BandKind> {
        self.top_at(x, z, |_| true).map(|(_, kind)| kind)
    }

    // Highest accepted triangle over the pieces of the point's query cell; ties keep the first
    // in piece order, so the answer does not depend on publication history.
    fn top_at(&self, x: f32, z: f32, accept: impl Fn(BandKind) -> bool) -> Option<(f32, BandKind)> {
        let point = RoadVec2::new(f64::from(x), f64::from(z));
        let mut best: Option<(f32, BandKind)> = None;
        for indexed in self.index.cell(query_cell_at(point.x, point.y)) {
            let bounds = indexed.mesh.bounds_xz();
            if point.x < bounds[0] - BOUNDS_EPSILON_M
                || point.y < bounds[1] - BOUNDS_EPSILON_M
                || point.x > bounds[2] + BOUNDS_EPSILON_M
                || point.y > bounds[3] + BOUNDS_EPSILON_M
            {
                continue;
            }
            if let Some(top) = top_at(&indexed.mesh, point, &accept)
                && best.is_none_or(|(best_m, _)| top.0 > best_m)
            {
                best = Some(top);
            }
        }
        best
    }

    /// True when a visible triangle of an accepted band kind lies within `radius_m` of `center`.
    pub fn any_band_near_disc(
        &self,
        center: RoadVec2,
        radius_m: f64,
        kinds: impl Fn(BandKind) -> bool,
    ) -> bool {
        let bounds = [
            center.x - radius_m,
            center.y - radius_m,
            center.x + radius_m,
            center.y + radius_m,
        ];
        self.any_piece_in_bounds(bounds, |_, mesh| {
            mesh.grid().items_in_bounds(bounds).any(|item| {
                mesh.triangle_band(item as usize).is_some_and(&kinds)
                    && triangle_near_disc(mesh.triangle_points(item as usize), center, radius_m)
            })
        })
    }

    /// First hit of a world-space ray against the visible road surface.
    pub fn raycast(&self, origin: Vector3, direction: Vector3) -> Option<Vector3> {
        if direction.length_squared() <= f32::EPSILON {
            return None;
        }
        let extent = self.index.extent_xz()?;
        let origin = RoadVec3::new(
            f64::from(origin.x),
            f64::from(origin.y),
            f64::from(origin.z),
        );
        let direction = RoadVec3::new(
            f64::from(direction.x),
            f64::from(direction.y),
            f64::from(direction.z),
        );
        let (entry_t, exit_t) = ray_xz_interval_for_bounds(
            origin,
            direction,
            extent[0] - BOUNDS_EPSILON_M,
            extent[1] - BOUNDS_EPSILON_M,
            extent[2] + BOUNDS_EPSILON_M,
            extent[3] + BOUNDS_EPSILON_M,
        )?;
        let mut best: Option<(f64, RoadVec3)> = None;
        if direction.x == 0.0 && direction.z == 0.0 {
            // A vertical ray stays in one cell; its parameter is unbounded there.
            let cell = query_cell_at(origin.x, origin.z);
            self.raycast_cell(cell, origin, direction, (0.0, f64::INFINITY), &mut best);
        } else {
            walk_ray_cells(
                RoadVec2::new(origin.x, origin.z),
                RoadVec2::new(direction.x, direction.z),
                (entry_t.max(0.0), exit_t),
                RoadVec2::ZERO,
                QUERY_CELL_M,
                |x, z, t0, t1| {
                    self.raycast_cell((x as i32, z as i32), origin, direction, (t0, t1), &mut best);
                    // A hit before the next cell cannot be beaten by any later cell.
                    best.is_none_or(|(best_t, _)| best_t > t1)
                },
            );
        }
        best.map(|(_, hit)| Vector3::new(hit.x as f32, hit.y as f32, hit.z as f32))
    }

    // Tests the triangles of the cell's pieces along the ray's `t` interval inside the cell.
    fn raycast_cell(
        &self,
        cell: (i32, i32),
        origin: RoadVec3,
        direction: RoadVec3,
        (t0, t1): (f64, f64),
        best: &mut Option<(f64, RoadVec3)>,
    ) {
        for indexed in self.index.cell(cell) {
            indexed
                .mesh
                .grid()
                .items_along_ray(origin, direction, t0, t1, |item| {
                    let triangle = indexed.mesh.triangle_points(item as usize);
                    if let Some(t) = road_ray_triangle_intersection_t(triangle, origin, direction)
                        && best.is_none_or(|(best_t, _)| t < best_t)
                    {
                        *best = Some((t, origin + direction * t));
                    }
                });
        }
    }

    /// Prepares the surface lookup for one lane owned by `edge_id` (its span) or, for a junction
    /// lane, `node_id` (its node piece).
    pub fn lane_query(
        &self,
        edge_id: usize,
        node_id: usize,
        carriageway_only: bool,
    ) -> LaneSurfaceQuery<'_> {
        let span = self.span_piece(edge_id).map(|span| span.mesh());
        let node = || {
            u32::try_from(node_id)
                .ok()
                .and_then(|node| self.node_piece(node))
                .map(|piece| piece.mesh())
        };
        LaneSurfaceQuery {
            owner: span.or_else(node),
            carriageway_only,
        }
    }

    /// Visits visible top triangles, with their band kind, whose piece grid cells touch the XZ
    /// bounds `[min_x, min_z, max_x, max_z]`, until `visit` returns true; returns whether it did.
    /// A triangle may be visited more than once.
    pub fn any_top_triangle_in_bounds(
        &self,
        bounds_xz: [f64; 4],
        mut visit: impl FnMut([RoadVec3; 3], BandKind) -> bool,
    ) -> bool {
        self.any_piece_in_bounds(bounds_xz, |_, mesh| {
            mesh.grid().items_in_bounds(bounds_xz).any(|item| {
                let triangle = item as usize;
                mesh.triangle_band(triangle)
                    .is_some_and(|kind| visit(mesh.triangle_points(triangle), kind))
            })
        })
    }

    /// Pieces whose bounds touch the XZ bounds `[min_x, min_z, max_x, max_z]`, each with the
    /// generation that built it, in ascending piece order.
    pub fn pieces_in_bounds(
        &self,
        bounds_xz: [f64; 4],
    ) -> impl Iterator<Item = (PieceId, u64)> + '_ {
        let mut pieces = Vec::new();
        self.any_piece_in_bounds(bounds_xz, |id, _| {
            pieces.push(id);
            false
        });
        pieces.sort_unstable();
        pieces.dedup();
        pieces.into_iter().filter_map(|id| {
            self.pieces
                .get(&id)
                .map(|published| (id, published.build_generation))
        })
    }

    // Visits pieces whose bounds touch `bounds` until `visit` returns true. Walks the query
    // cells of the bounds clipped to the published extent, or every piece in order when that
    // is fewer; a piece may be visited once per cell it shares with the bounds.
    fn any_piece_in_bounds(
        &self,
        bounds: [f64; 4],
        mut visit: impl FnMut(PieceId, &PieceMesh) -> bool,
    ) -> bool {
        let Some(extent) = self.index.extent_xz() else {
            return false;
        };
        let clipped = [
            bounds[0].max(extent[0]),
            bounds[1].max(extent[1]),
            bounds[2].min(extent[2]),
            bounds[3].min(extent[3]),
        ];
        if clipped[0] > clipped[2] || clipped[1] > clipped[3] {
            return false;
        }
        let first = query_cell_at(clipped[0], clipped[1]);
        let last = query_cell_at(clipped[2], clipped[3]);
        let cells = (i64::from(last.0) - i64::from(first.0) + 1)
            * (i64::from(last.1) - i64::from(first.1) + 1);
        if cells as u64 > self.pieces.len() as u64 {
            return self.pieces.iter().any(|(&id, published)| {
                let mesh = published.mesh();
                bounds_intersect(mesh.bounds_xz(), bounds) && visit(id, mesh)
            });
        }
        for x in first.0..=last.0 {
            for z in first.1..=last.1 {
                for indexed in self.index.cell((x, z)) {
                    if bounds_intersect(indexed.mesh.bounds_xz(), bounds)
                        && visit(indexed.id, &indexed.mesh)
                    {
                        return true;
                    }
                }
            }
        }
        false
    }
}
