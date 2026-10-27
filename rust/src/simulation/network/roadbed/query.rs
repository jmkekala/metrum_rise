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
use super::mesh::{PieceMesh, abutment_bottom_height};
use super::{BandKind, RoadVec2, RoadVec3, RoadbedSystem};
use godot::prelude::Vector3;

// Ray-in-extent tolerance, matching the piece grids' padding.
const BOUNDS_EPSILON_M: f64 = 0.001;

/// Identity of one roadbed piece.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PieceId {
    /// The span piece of an edge.
    Span(usize),
    /// The junction, bend or terminal piece of a node.
    Node(u32),
}

/// One part of a tunnel span with a portal row ([`RoadbedSystem::for_each_portal_part_in_bounds`],
/// `TERRAIN-07-C`, `docs/terrain.md` T22 5 (a)).
#[derive(Clone, Copy, Debug)]
pub enum PortalPart<'a> {
    /// A portal row's vertices, corner to corner ([`PieceMesh::portal_rows`]).
    Row(&'a [RoadVec3]),
    /// A top triangle: the deck the graded terrain opens near a portal row.
    Top([RoadVec3; 3]),
    /// An outer-chain segment, the deck on its left: a bore seam, along which the opening's
    /// sides stand.
    Bore([RoadVec3; 2]),
}

/// One wall-run segment ([`RoadbedSystem::wall_seams_in_bounds`], `TERRAIN-07-C`), with the
/// abutment face's bottom edge below its ends where its run crosses a bridge's mouth
/// (`TERRAIN-07-D`, the abutment band).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WallSegment {
    /// The segment's two points, the grounded road on its left.
    pub points: [RoadVec3; 2],
    /// The height of the abutment face's bottom edge below each point (the skirt depth below
    /// the row's corners, straight between them); `None` where the structure is a tunnel.
    pub abutment_bottom: Option<[f64; 2]>,
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
        let accepts = |kind| !self.carriageway_only || kind == BandKind::Carriageway;
        self.owner
            .and_then(|mesh| top_at(mesh, point, accepts))
            .map(|(height_m, _)| height_m)
    }
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

// Visits the segments of `chains` (a piece's outer chains or wall runs) whose XZ bounds touch
// `bounds_xz`, in chain order.
fn for_each_segment_in_bounds(
    mesh: &PieceMesh,
    chains: &[super::OuterChain],
    bounds_xz: [f64; 4],
    visit: &mut impl FnMut([RoadVec3; 2]),
) {
    for chain in chains {
        for pair in chain.vertices().windows(2) {
            let [a, b] = [pair[0], pair[1]].map(|v| mesh.positions()[v as usize]);
            let segment = [a.x.min(b.x), a.z.min(b.z), a.x.max(b.x), a.z.max(b.z)];
            if bounds_intersect(segment, bounds_xz) {
                visit([a, b]);
            }
        }
    }
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

    /// Visits each piece with a carriageway triangle at a world XZ point, in piece order, with
    /// the height of its highest one there: what [`Self::carriageway_height_at`] takes the
    /// maximum of, for callers that must leave some pieces out (`ROAD-68` open question 16).
    pub(crate) fn for_each_carriageway_top_at(
        &self,
        x: f32,
        z: f32,
        mut visit: impl FnMut(PieceId, f32),
    ) {
        let point = RoadVec2::new(f64::from(x), f64::from(z));
        for indexed in self.index.cell(query_cell_at(point.x, point.y)) {
            if let Some((height_m, _)) =
                top_at(&indexed.mesh, point, |kind| kind == BandKind::Carriageway)
            {
                visit(indexed.id, height_m);
            }
        }
    }

    /// Band kind of the highest visible road triangle at a world XZ point.
    #[cfg(test)]
    pub(crate) fn band_at(&self, x: f32, z: f32) -> Option<BandKind> {
        self.top_at(x, z, |_| true).map(|(_, kind)| kind)
    }

    // Highest accepted triangle over the pieces of the point's query cell; ties keep the first
    // in piece order, so the answer does not depend on publication history. A piece grid holds
    // only points within its padded top-triangle bounds, so pieces away from the point cost one
    // bounds test.
    fn top_at(&self, x: f32, z: f32, accept: impl Fn(BandKind) -> bool) -> Option<(f32, BandKind)> {
        let point = RoadVec2::new(f64::from(x), f64::from(z));
        let mut best: Option<(f32, BandKind)> = None;
        for indexed in self.index.cell(query_cell_at(point.x, point.y)) {
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
        self.any_top_triangle_in_bounds(bounds, |triangle, kind| {
            kinds(kind) && triangle_near_disc(triangle, center, radius_m)
        })
    }

    /// First hit of a world-space ray against the visible road surface.
    pub fn raycast(&self, origin: Vector3, direction: Vector3) -> Option<Vector3> {
        if direction.length_squared() <= f32::EPSILON {
            return None;
        }
        let extent = self.index.extent_xz()?;
        let origin = RoadVec3::from_array(origin.to_array().map(f64::from));
        let direction = RoadVec3::from_array(direction.to_array().map(f64::from));
        let (entry_t, exit_t) = ray_xz_interval_for_bounds(
            origin,
            direction,
            extent[0] - BOUNDS_EPSILON_M,
            extent[1] - BOUNDS_EPSILON_M,
            extent[2] + BOUNDS_EPSILON_M,
            extent[3] + BOUNDS_EPSILON_M,
        )?;
        let mut best: Option<(f64, RoadVec3)> = None;
        // A vertical ray visits its one cell over an unbounded parameter.
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
        self.any_top_triangle_of_pieces_in_bounds(bounds_xz, |mesh, triangle| {
            mesh.triangle_band(triangle)
                .is_some_and(|kind| visit(mesh.triangle_points(triangle), kind))
        })
    }

    /// [`Self::any_top_triangle_in_bounds`] without the band kind, which costs a search per
    /// triangle visited.
    pub fn any_top_triangle_points_in_bounds(
        &self,
        bounds_xz: [f64; 4],
        mut visit: impl FnMut([RoadVec3; 3]) -> bool,
    ) -> bool {
        self.any_top_triangle_of_pieces_in_bounds(bounds_xz, |mesh, triangle| {
            visit(mesh.triangle_points(triangle))
        })
    }

    // Visits the top triangles of the pieces' grid cells touching `bounds_xz`, as in
    // `any_top_triangle_in_bounds`.
    fn any_top_triangle_of_pieces_in_bounds(
        &self,
        bounds_xz: [f64; 4],
        mut visit: impl FnMut(&PieceMesh, usize) -> bool,
    ) -> bool {
        self.any_piece_in_bounds(bounds_xz, |_, mesh| {
            mesh.any_top_triangle_in_bounds(bounds_xz, |triangle| visit(mesh, triangle))
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

    /// The grounded outer-chain segments of the pieces whose bounds touch the XZ bounds
    /// `[min_x, min_z, max_x, max_z]`, each segment whose own XZ bounds touch them, sorted and
    /// deduplicated by key, the bits of its start then its end (`TERRAIN-07-A`): the seams the
    /// graded terrain grades against. Bridge and tunnel spans publish none; a one-vertex chain
    /// has no segment. `pieces` is the caller's scratch. `O(P log P + V + S log S)` for the `P`
    /// pieces met, their `V` chain vertices and the `S` segments kept.
    pub fn grounded_seams_in_bounds(
        &self,
        bounds_xz: [f64; 4],
        pieces: &mut Vec<PieceId>,
        out: &mut Vec<[RoadVec3; 2]>,
    ) {
        out.clear();
        self.for_each_grounded_seam_in_bounds(bounds_xz, pieces, |segment| out.push(segment));
        let key = |[a, b]: &[RoadVec3; 2]| [a.x, a.y, a.z, b.x, b.y, b.z].map(f64::to_bits);
        out.sort_unstable_by_key(key);
        out.dedup_by_key(|segment| key(segment));
    }

    /// [`Self::grounded_seams_in_bounds`] unsorted, in piece and chain order: a segment two
    /// pieces share is visited twice. `O(P log P + V / 8 + S)` for the `S` segments in the runs
    /// of eight that meet the bounds.
    pub fn for_each_grounded_seam_in_bounds(
        &self,
        bounds_xz: [f64; 4],
        pieces: &mut Vec<PieceId>,
        visit: impl FnMut([RoadVec3; 2]),
    ) {
        self.for_each_grounded_seam_in_bounds_where(bounds_xz, pieces, |_, _| true, visit);
    }

    /// [`Self::for_each_grounded_seam_in_bounds`] over the runs of consecutive chain segments
    /// that `keep` accepts by their XZ bounds `[min_x, min_z, max_x, max_z]` and height range
    /// `(lowest, highest)`, which hold each segment's own (`TERRAIN-07-F`, `docs/terrain.md`
    /// T37 (b)): a test that a segment's bounds can only fail where its run's fail skips no
    /// segment that passes it. `O(P log P + V / 8 + S)` for the `S` segments of the runs kept.
    pub fn for_each_grounded_seam_in_bounds_where(
        &self,
        bounds_xz: [f64; 4],
        pieces: &mut Vec<PieceId>,
        mut keep: impl FnMut([f64; 4], (f64, f64)) -> bool,
        mut visit: impl FnMut([RoadVec3; 2]),
    ) {
        self.grounded_pieces_in_bounds(bounds_xz, pieces);
        for id in pieces.iter() {
            let mesh = self.pieces[id].mesh();
            for block in mesh.seam_blocks() {
                if !bounds_intersect(block.bounds_xz, bounds_xz)
                    || !keep(block.bounds_xz, block.heights)
                {
                    continue;
                }
                let chain = mesh.outer_chains()[block.chain as usize].vertices();
                let (first, last) = (block.vertices.0 as usize, block.vertices.1 as usize);
                for pair in chain[first..=last].windows(2) {
                    let [a, b] = [pair[0], pair[1]].map(|v| mesh.positions()[v as usize]);
                    let segment = [a.x.min(b.x), a.z.min(b.z), a.x.max(b.x), a.z.max(b.z)];
                    if bounds_intersect(segment, bounds_xz) {
                        visit([a, b]);
                    }
                }
            }
        }
    }

    /// The wall-run segments ([`PieceMesh::wall_runs`], `TERRAIN-07-C`) of `pieces`, the
    /// grounded pieces [`Self::grounded_pieces_in_bounds`] found for the same bounds, whose own
    /// XZ bounds touch them, sorted and deduplicated by key like
    /// [`Self::grounded_seams_in_bounds`]: the structure mouths' wall seams. `O(V + S log S)`
    /// for the pieces' wall-run vertices `V` and the `S` segments kept. A run across a
    /// bridge's mouth ([`PieceMesh::abutment_runs`]) gives its segments the abutment face's
    /// bottom edge below their ends.
    pub fn wall_seams_in_bounds(
        &self,
        bounds_xz: [f64; 4],
        pieces: &[PieceId],
        out: &mut Vec<WallSegment>,
    ) {
        out.clear();
        for id in pieces {
            let mesh = self.pieces[id].mesh();
            for (run, &abutment) in mesh.wall_runs().iter().zip(mesh.abutment_runs()) {
                // A run spans its mouth row from corner to corner.
                let vertices = run.vertices();
                let corners = abutment.then(|| {
                    [vertices[0], vertices[vertices.len() - 1]]
                        .map(|v| mesh.positions()[v as usize])
                });
                let chains = std::slice::from_ref(run);
                for_each_segment_in_bounds(mesh, chains, bounds_xz, &mut |points| {
                    out.push(WallSegment {
                        points,
                        abutment_bottom: corners
                            .map(|corners| points.map(|p| abutment_bottom_height(corners, p))),
                    })
                });
            }
        }
        let key = |s: &WallSegment| {
            let [a, b] = s.points;
            [a.x, a.y, a.z, b.x, b.y, b.z].map(f64::to_bits)
        };
        out.sort_unstable_by_key(key);
        out.dedup_by_key(|segment| key(segment));
    }

    /// Visits the top triangles of the grounded pieces whose bounds touch the XZ bounds, whose
    /// piece grid cells touch them, in ascending piece order (`TERRAIN-07-A`): the footprint's
    /// coverage, tested with [`Self::top_triangle_height_at`]. A piece inside the bounds has
    /// every grid cell in them, so its top triangles are visited once each, in mesh order;
    /// another piece's triangle may be visited more than once. `pieces` is the caller's scratch.
    pub fn for_each_grounded_top_triangle_in_bounds(
        &self,
        bounds_xz: [f64; 4],
        pieces: &mut Vec<PieceId>,
        mut visit: impl FnMut([RoadVec3; 3]),
    ) {
        self.grounded_pieces_in_bounds(bounds_xz, pieces);
        for id in pieces.iter() {
            let mesh = self.pieces[id].mesh();
            let piece = mesh.bounds_xz();
            if bounds_xz[0] <= piece[0]
                && bounds_xz[1] <= piece[1]
                && piece[2] <= bounds_xz[2]
                && piece[3] <= bounds_xz[3]
            {
                for triangle in 0..mesh.top_triangle_count() {
                    visit(mesh.triangle_points(triangle));
                }
                continue;
            }
            mesh.any_top_triangle_in_bounds(bounds_xz, |triangle| {
                visit(mesh.triangle_points(triangle));
                false
            });
        }
    }

    /// Visits the parts of the tunnel spans with a portal row ([`PieceMesh::portal_rows`],
    /// `TERRAIN-07-C`) whose bounds touch the XZ bounds, piece by piece in ascending piece order:
    /// every portal row, the top triangles whose piece grid cells touch the bounds (a triangle
    /// may come more than once), and the outer-chain segments whose own bounds touch them. No
    /// terrain height is read. `pieces` is the caller's scratch. `O(P log P + T + V)` for the
    /// pieces met, the triangles visited and their chain vertices.
    pub fn for_each_portal_part_in_bounds(
        &self,
        bounds_xz: [f64; 4],
        pieces: &mut Vec<PieceId>,
        mut visit: impl FnMut(PortalPart<'_>),
    ) {
        self.portal_pieces_in_bounds(bounds_xz, pieces);
        for id in pieces.iter() {
            let mesh = self.pieces[id].mesh();
            mesh.portal_rows()
                .for_each(|row| visit(PortalPart::Row(row)));
            mesh.any_top_triangle_in_bounds(bounds_xz, |triangle| {
                visit(PortalPart::Top(mesh.triangle_points(triangle)));
                false
            });
            for_each_segment_in_bounds(mesh, mesh.outer_chains(), bounds_xz, &mut |segment| {
                visit(PortalPart::Bore(segment))
            });
        }
    }

    /// The tunnel spans with a portal row ([`PieceMesh::portal_rows`], `TERRAIN-07-C`) whose
    /// bounds touch the XZ bounds, sorted and unique, into `pieces`. `O(P log P)` for the pieces
    /// met.
    pub fn portal_pieces_in_bounds(&self, bounds_xz: [f64; 4], pieces: &mut Vec<PieceId>) {
        pieces.clear();
        self.any_piece_in_bounds(bounds_xz, |id, mesh| {
            if mesh.portal_rows().next().is_some() {
                pieces.push(id);
            }
            false
        });
        pieces.sort_unstable();
        pieces.dedup();
    }

    /// The point-in-triangle predicate and height [`Self::height_at`] takes the highest of:
    /// the triangle's height at the XZ point, `None` outside it. Exact, with no tolerance: the
    /// point's side of each edge is an exact orientation test on the `f64` positions, and the
    /// closed triangle holds the point, so every triangle meeting at an edge or vertex holds a
    /// point on it, whatever the junction's orientation. The graded terrain's coverage is this
    /// predicate (`TERRAIN-07`); its raster prepares each triangle once and ends each lattice
    /// row's scan with its held run (`CoverageTriangle`, the same predicate bit for bit).
    pub fn top_triangle_height_at(triangle: &[RoadVec3; 3], x: f64, z: f64) -> Option<f32> {
        triangle_height_at_xz(triangle, RoadVec2::new(x, z))
    }

    /// The grounded pieces (standard spans, and node pieces where a standard edge meets) whose
    /// bounds touch the XZ bounds, sorted and unique, into `pieces` (`TERRAIN-07-A`).
    pub fn grounded_pieces_in_bounds(&self, bounds: [f64; 4], pieces: &mut Vec<PieceId>) {
        pieces.clear();
        self.any_piece_in_bounds(bounds, |id, _| {
            pieces.push(id);
            false
        });
        pieces.sort_unstable();
        pieces.dedup();
        pieces.retain(|id| self.pieces[id].grounded());
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
