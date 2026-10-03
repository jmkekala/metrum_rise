// SPDX-License-Identifier: GPL-2.0-only

//! Public road-surface contracts, module wiring, and shared numeric constants.
//!
//! The sibling modules own the concrete edge, span, node, overlay, query,
//! earthwork, geometry, cache, system, and debug implementations. This file
//! keeps only the public contracts and stage re-exports that cross those owners.

use spade::{ConstrainedDelaunayTriangulation, Point2};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

mod backend;
mod band_semantics;
mod cache;
mod debug;
mod earthwork;
mod edge;
mod geometry;
mod incident;
mod indices;
mod keys;
mod node;
mod overlay;
mod paths;
mod query;
mod segments;
mod span;
mod system;
mod terrain_clip;

pub use backend::{RoadVec2, RoadVec3};
pub use cache::{RoadEarthworkChunkCacheEntry, RoadSurfaceChunkCacheEntry};
pub(crate) use edge::RoadPreviewVisualMesh;
pub use edge::{PreviewRoadSurfaceResult, RoadPreviewValidation};
pub use node::RoadSurfaceVisualNodePiece;
pub(crate) use query::{ray_xz_interval_for_bounds, road_ray_triangle_intersection_t};
pub use span::{RoadSurfaceVisualSpanPiece, SpanQuad};
pub use system::RoadSurfaceSystem;

pub(crate) use cache::ChunkCacheKind;
pub(crate) use cache::RoadSurfaceTopologyUndo;
pub(crate) use earthwork::RoadEarthworkPlan;
pub(crate) use earthwork::{
    RoadSurfaceEarthworkBoundarySegment, RoadSurfaceEarthworkFaceKind,
    RoadSurfaceEarthworkFaceSource, RoadSurfaceEarthworkRenderFace,
    RoadSurfaceEarthworkSupportPolicy,
};
pub(crate) use edge::PreparedRoadInput;
pub(crate) use edge::{CURB_STEP_HEIGHT_M, RoadExtensionReprofile};
pub(crate) use incident::{
    CompiledNodeKind, IncidentEdgeSide, IncidentMouthBand, IncidentMouthProfile,
    IncidentSurfaceEdge, OrderedIncidentPieceMouth, RoadSurfaceVisualNodeCompileInput,
};
pub(crate) use node::{
    NodeCanonicalTopologyCache, NodeFootprintBoundaryDirectSource,
    NodeFootprintBoundarySegmentSource, NodeFootprintBoundaryVertexSource, NodeOwnedRegion,
    NodeTopSurfacePolygonSource, NodeVisualCompileResult, RoadSurfaceVerticalFaceSource,
    rounded_sidewalk_corner_path_xz,
};
pub(crate) use node::{arrangement, height};
#[cfg(test)]
pub(crate) use node::{input, ownership, rails, terminal, triangulation, validation};
pub(crate) use query::{PlannedRoadSurfaceQuery, RoadSurfaceView};
pub(crate) use span::{
    RoadSurfaceSpanBandOwner, RoadSurfaceSpanOwnedRegion, RoadSurfaceSpanRegionRole,
};
pub(crate) use system::{RoadPreviewTopologyReuse, RoadSurfaceCompileReason};
pub(crate) use terrain_clip::{
    RoadSurfaceTerrainClipContourRole, RoadSurfaceTerrainClipEdgeKind,
    RoadSurfaceTerrainClipExport, RoadSurfaceTerrainClipExportError, RoadSurfaceTerrainClipLoop,
    RoadSurfaceTerrainClipLoopTopology, RoadSurfaceTerrainClipSourceEdge,
    terrain_clip_edge_kind_for_band,
};

// Shared geometric tolerances used across surface compilation, overlay solving, and queries.
const SAMPLE_EPSILON_M: f32 = 0.001;
const WORLD_POINT_DEDUP_DISTANCE_M: f32 = 1.0e-4;
const WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2: f32 =
    WORLD_POINT_DEDUP_DISTANCE_M * WORLD_POINT_DEDUP_DISTANCE_M;
// Shared overlay/geometry area floor: one 1 mm quantized square keeps closure slivers visible.
const NODE_OVERLAY_MIN_AREA_M2: f32 = 1.0e-6;
// Self-checks that compare two backend results allow a small fixed-grid residual budget.
const NODE_OVERLAY_NUMERIC_AREA_EPS_M2: f32 = NODE_OVERLAY_MIN_AREA_M2 * 16.0;
const NODE_OVERLAY_NUMERIC_DUST_WIDTH_M: f32 = WORLD_POINT_DEDUP_DISTANCE_M;
const NODE_OVERLAY_NUMERIC_AREA_CAP_M2: f32 = 1.0e-3;
// Avoid Rayon setup overhead for the small edge/node sets common in single-edit rebuilds.
const PARALLEL_SURFACE_COMPILE_MIN_ITEMS: usize = 16;
// Span earthwork and render geometry are heavy enough to amortize scheduling at two edges.
const PARALLEL_SPAN_COMPILE_MIN_ITEMS: usize = 2;
// Node pieces are much heavier than edge/span pieces; parallelize as soon as two dirty nodes exist.
const PARALLEL_NODE_COMPILE_MIN_ITEMS: usize = 2;

type SurfaceCdt = ConstrainedDelaunayTriangulation<Point2<f64>>;
type NodeOverlayPoint = [f64; 2];
type NodeOverlayPointKey = (i64, i64);
type NodeOverlayContour = Vec<NodeOverlayPoint>;
type NodeOverlayShape = Vec<NodeOverlayContour>;
type NodeOverlayShapes = Vec<NodeOverlayShape>;

/// Chunk key used by the road-surface and earthwork caches.
pub type SurfaceChunkKey = (i32, i32);

/// Ordered lateral surface-band kinds supported by the compiled roadbed.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RoadSurfaceBandKind {
    /// Main drivable carriageway surface.
    Carriageway,
    /// Curb or shoulder transition surface adjacent to the carriageway.
    CurbOrShoulder,
    /// Walkable sidewalk surface.
    Sidewalk,
    /// Dedicated pedestrian corridor that is not a roadside sidewalk band.
    Footpath,
    /// Reserved central median or separator.
    Median,
    /// Reserved parking band.
    Parking,
    /// Reserved bicycle band.
    CycleTrack,
    /// Reserved tram corridor.
    TramReservation,
}

/// One ordered lateral band inside a compiled roadbed section.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadSurfaceBand {
    /// Surface-band classification.
    pub kind: RoadSurfaceBandKind,
    /// Inclusive lateral start offset from the section centerline in world metres.
    pub lateral_start_m: f32,
    /// Inclusive lateral end offset from the section centerline in world metres.
    pub lateral_end_m: f32,
    /// Height in world metres at `lateral_start_m`.
    pub height_start_m: f32,
    /// Height in world metres at `lateral_end_m`.
    pub height_end_m: f32,
}

/// One sampled cross-section along an edge in the compiled roadbed model.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadSurfaceSection {
    /// Owning edge id.
    pub edge_idx: usize,
    /// Longitudinal distance from the edge start in world metres.
    pub s_m: f32,
    /// Section center point in world-space XZ metres.
    pub center_xz: RoadVec2,
    /// Solved center height in world metres.
    pub center_height_m: f32,
    /// Unit tangent vector in XZ.
    pub tangent_xz: RoadVec2,
    /// Unit lateral axis in XZ.
    pub lateral_xz: RoadVec2,
    /// Ordered lateral bands for this section.
    pub bands: Vec<RoadSurfaceBand>,
}

/// Piece classification for explicit visual node ownership during the graph/visual split.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoadSurfaceVisualNodePieceKind {
    /// One incident surface edge ends here and requires a terminal visual piece.
    Terminal,
    /// Two non-pass-through incident edges require one explicit bend visual piece.
    Bend,
    /// Three or more incident edges require an explicit multi-mouth junction visual piece.
    JunctionN,
}

impl RoadSurfaceVisualNodePieceKind {
    pub(crate) fn sort_key(self) -> u8 {
        match self {
            Self::Terminal => 0,
            Self::Bend => 1,
            Self::JunctionN => 2,
        }
    }
}

/// One explicit polygon owned by the visual road carrier.
#[derive(Clone, Debug)]
pub struct RoadSurfaceVisualPolygon {
    /// Ordered world-space polygon points.
    pub points_world: Vec<RoadVec3>,
    // Deterministic triangles covering the polygon, read through `triangles()`.
    triangulation: PolygonTriangulation,
}

// Triangles stored by reference to the polygon's own points instead of as copies; `ROAD-45`.
// Construction picks the first form that reproduces the triangles bit for bit.
#[derive(Clone, Debug, PartialEq)]
enum PolygonTriangulation {
    // Exactly the fan `(0, i, i + 1)` over every point.
    Fan,
    // Index triples into the points.
    Indexed(Box<[[u16; 3]]>),
    // Some vertex is not one of the points, or the points overflow `u16`.
    Explicit(Box<[[RoadVec3; 3]]>),
}

const ROAD_SURFACE_QUERY_GRID_BASE_CELL_M: f64 = 4.0;
const ROAD_SURFACE_QUERY_GRID_MAX_CELLS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq)]
struct RoadSurfaceIndexedTriangle {
    triangle: [RoadVec3; 3],
    carriageway: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
/// Immutable owner-local triangle grid shared by road carriers and compiled terrain tiles.
pub(crate) struct RoadSurfaceTriangleQueryIndex {
    grid: SurfaceTriangleGrid,
    triangles: Vec<RoadSurfaceIndexedTriangle>,
}

/// Bounded owner-local XZ grid listing caller-defined triangle ids per cell.
///
/// Owners that can rebuild their triangles keep only this grid; `ROAD-44`.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SurfaceTriangleGrid {
    bounds_xz: [f64; 4],
    cell_size_m: f64,
    width: usize,
    height: usize,
    cell_offsets: Vec<u32>,
    cell_items: Vec<u32>,
}

#[derive(Clone)]
struct RoadSurfaceTerrainLoopGradingCacheEntry {
    terrain_source_generation: u64,
    terrain_visual_generation: u64,
    render_step_bits: u32,
    points_world: Arc<Vec<RoadVec3>>,
    influence_bounds: Option<(f32, f32, f32, f32)>,
    patch_margins: Arc<BTreeMap<(usize, usize), f32>>,
}

#[derive(Default)]
struct RoadSurfaceTerrainGradingCache {
    span_loops: HashMap<usize, Vec<Option<RoadSurfaceTerrainLoopGradingCacheEntry>>>,
    node_loops: HashMap<u32, Vec<Option<RoadSurfaceTerrainLoopGradingCacheEntry>>>,
}

/// Prepared owner-local surface lookup reused for every point of one lane.
pub(crate) struct RoadLaneSurfaceQuery<'a> {
    node_indices: [Option<&'a RoadSurfaceTriangleQueryIndex>; 2],
    node_count: usize,
    span: Option<&'a RoadSurfaceVisualSpanPiece>,
    carriageway_only: bool,
}

impl RoadSurfaceVisualPolygon {
    /// Builds a polygon from its deterministic boundary and triangulation.
    pub(crate) fn from_parts(
        points_world: Vec<RoadVec3>,
        triangles_world: &[[RoadVec3; 3]],
    ) -> Self {
        let same_bits = |a: RoadVec3, b: RoadVec3| {
            a.to_array().map(f64::to_bits) == b.to_array().map(f64::to_bits)
        };
        let index_of = |point: RoadVec3| {
            let index = points_world.iter().position(|&own| same_bits(own, point))?;
            u16::try_from(index).ok()
        };
        let indices: Option<Vec<_>> = triangles_world
            .iter()
            .map(|triangle| {
                Some([
                    index_of(triangle[0])?,
                    index_of(triangle[1])?,
                    index_of(triangle[2])?,
                ])
            })
            .collect();
        let triangulation = match indices {
            Some(indices) => PolygonTriangulation::from_indices(points_world.len(), indices),
            None => PolygonTriangulation::Explicit(triangles_world.into()),
        };
        Self {
            points_world,
            triangulation,
        }
    }

    /// Builds a polygon whose triangles are index triples into `points_world`.
    pub(crate) fn from_indexed_parts(
        points_world: Vec<RoadVec3>,
        triangles: &[[usize; 3]],
    ) -> Self {
        let indices: Option<Vec<_>> = triangles
            .iter()
            .map(|triangle| {
                let [a, b, c] = triangle.map(|index| u16::try_from(index).ok());
                Some([a?, b?, c?])
            })
            .collect();
        let triangulation = match indices {
            Some(indices) => PolygonTriangulation::from_indices(points_world.len(), indices),
            None => PolygonTriangulation::Explicit(
                triangles
                    .iter()
                    .map(|triangle| triangle.map(|index| points_world[index]))
                    .collect(),
            ),
        };
        Self {
            points_world,
            triangulation,
        }
    }

    /// Number of triangles covering the polygon.
    pub fn triangle_count(&self) -> usize {
        match &self.triangulation {
            PolygonTriangulation::Fan => self.points_world.len() - 2,
            PolygonTriangulation::Indexed(indices) => indices.len(),
            PolygonTriangulation::Explicit(triangles) => triangles.len(),
        }
    }

    /// Deterministic triangles covering the polygon in world space, in construction order.
    pub fn triangles(
        &self,
    ) -> impl ExactSizeIterator<Item = [RoadVec3; 3]> + DoubleEndedIterator + Clone + '_ {
        (0..self.triangle_count()).map(move |index| self.triangle(index))
    }

    fn triangle(&self, index: usize) -> [RoadVec3; 3] {
        let points = &self.points_world;
        match &self.triangulation {
            PolygonTriangulation::Fan => [points[0], points[index + 1], points[index + 2]],
            PolygonTriangulation::Indexed(indices) => {
                indices[index].map(|point| points[usize::from(point)])
            }
            PolygonTriangulation::Explicit(triangles) => triangles[index],
        }
    }
}

// Same points and same triangles, whichever form stores them.
impl PartialEq for RoadSurfaceVisualPolygon {
    fn eq(&self, other: &Self) -> bool {
        self.points_world == other.points_world
            && (self.triangulation == other.triangulation || self.triangles().eq(other.triangles()))
    }
}

impl PolygonTriangulation {
    fn from_indices(point_count: usize, indices: Vec<[u16; 3]>) -> Self {
        let fan = point_count >= 3
            && indices.len() == point_count - 2
            && indices.iter().enumerate().all(|(index, &triangle)| {
                let next = index as u16 + 1;
                triangle == [0, next, next + 1]
            });
        if fan {
            Self::Fan
        } else {
            Self::Indexed(indices.into_boxed_slice())
        }
    }
}

impl RoadSurfaceTriangleQueryIndex {
    /// Builds the existing bounded grid for a terrain tile without copying polygon wrappers.
    pub(crate) fn from_ground_triangles(
        triangles: impl IntoIterator<Item = [RoadVec3; 3]>,
    ) -> Self {
        Self::from_indexed_triangles(
            triangles
                .into_iter()
                .map(|triangle| RoadSurfaceIndexedTriangle {
                    triangle,
                    carriageway: false,
                })
                .collect(),
        )
    }

    fn from_surface_polygons(
        road: &[RoadSurfaceVisualPolygon],
        curb: &[RoadSurfaceVisualPolygon],
        sidewalk: &[RoadSurfaceVisualPolygon],
    ) -> Self {
        let mut triangles = Vec::new();
        for (polygons, carriageway) in [(road, true), (curb, false), (sidewalk, false)] {
            triangles.extend(polygons.iter().flat_map(|polygon| {
                polygon
                    .triangles()
                    .map(move |triangle| RoadSurfaceIndexedTriangle {
                        triangle,
                        carriageway,
                    })
            }));
        }
        Self::from_indexed_triangles(triangles)
    }

    fn from_indexed_triangles(triangles: Vec<RoadSurfaceIndexedTriangle>) -> Self {
        Self {
            grid: SurfaceTriangleGrid::from_triangles(
                triangles.iter().map(|indexed| indexed.triangle),
                |triangle_idx| triangle_idx as u32,
            ),
            triangles,
        }
    }

    fn cell_triangle_indices(&self, point: RoadVec2) -> &[u32] {
        self.grid.cell_items(point)
    }
}

impl SurfaceTriangleGrid {
    /// Grids `triangles` in order; `item_id` names the triangle at each position in its cells.
    pub(crate) fn from_triangles(
        triangles: impl Iterator<Item = [RoadVec3; 3]> + Clone,
        item_id: impl Fn(usize) -> u32,
    ) -> Self {
        let mut bounds_xz = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        let mut empty = true;
        for triangle in triangles.clone() {
            empty = false;
            for point in triangle {
                bounds_xz[0] = bounds_xz[0].min(point.x);
                bounds_xz[1] = bounds_xz[1].min(point.z);
                bounds_xz[2] = bounds_xz[2].max(point.x);
                bounds_xz[3] = bounds_xz[3].max(point.z);
            }
        }
        if empty {
            return Self::default();
        }

        let mut cell_size_m = ROAD_SURFACE_QUERY_GRID_BASE_CELL_M;
        let (mut width, mut height) = query_grid_dimensions(bounds_xz, cell_size_m);
        while width.saturating_mul(height) > ROAD_SURFACE_QUERY_GRID_MAX_CELLS {
            cell_size_m *= 2.0;
            (width, height) = query_grid_dimensions(bounds_xz, cell_size_m);
        }
        let cell_count = width.saturating_mul(height);
        let mut counts = vec![0_u32; cell_count];
        for triangle in triangles.clone() {
            let (min_x, min_z, max_x, max_z) =
                query_grid_triangle_cell_bounds(triangle, bounds_xz, cell_size_m, width, height);
            for z in min_z..=max_z {
                for x in min_x..=max_x {
                    counts[z * width + x] += 1;
                }
            }
        }

        let mut cell_offsets = Vec::with_capacity(cell_count + 1);
        cell_offsets.push(0);
        for count in counts {
            cell_offsets.push(cell_offsets.last().copied().unwrap_or(0) + count);
        }
        let mut cell_items = vec![0_u32; cell_offsets.last().copied().unwrap_or(0) as usize];
        let mut cursors = cell_offsets[..cell_count].to_vec();
        for (triangle_idx, triangle) in triangles.enumerate() {
            let (min_x, min_z, max_x, max_z) =
                query_grid_triangle_cell_bounds(triangle, bounds_xz, cell_size_m, width, height);
            let item = item_id(triangle_idx);
            for z in min_z..=max_z {
                for x in min_x..=max_x {
                    let cell_idx = z * width + x;
                    let cursor = &mut cursors[cell_idx];
                    cell_items[*cursor as usize] = item;
                    *cursor += 1;
                }
            }
        }

        Self {
            bounds_xz,
            cell_size_m,
            width,
            height,
            cell_offsets,
            cell_items,
        }
    }

    /// Triangle ids listed in the cell containing `point`; empty outside the grid.
    pub(crate) fn cell_items(&self, point: RoadVec2) -> &[u32] {
        if self.width == 0
            || self.height == 0
            || point.x < self.bounds_xz[0] - f64::from(SAMPLE_EPSILON_M)
            || point.y < self.bounds_xz[1] - f64::from(SAMPLE_EPSILON_M)
            || point.x > self.bounds_xz[2] + f64::from(SAMPLE_EPSILON_M)
            || point.y > self.bounds_xz[3] + f64::from(SAMPLE_EPSILON_M)
        {
            return &[];
        }
        let x = (((point.x - self.bounds_xz[0]) / self.cell_size_m).floor() as isize)
            .clamp(0, self.width as isize - 1) as usize;
        let z = (((point.y - self.bounds_xz[1]) / self.cell_size_m).floor() as isize)
            .clamp(0, self.height as isize - 1) as usize;
        let cell_idx = z * self.width + x;
        let start = self.cell_offsets[cell_idx] as usize;
        let end = self.cell_offsets[cell_idx + 1] as usize;
        &self.cell_items[start..end]
    }
}

fn query_grid_dimensions(bounds_xz: [f64; 4], cell_size_m: f64) -> (usize, usize) {
    let width = (((bounds_xz[2] - bounds_xz[0]) / cell_size_m).floor() as usize + 1).max(1);
    let height = (((bounds_xz[3] - bounds_xz[1]) / cell_size_m).floor() as usize + 1).max(1);
    (width, height)
}

fn query_grid_triangle_cell_bounds(
    triangle: [RoadVec3; 3],
    bounds_xz: [f64; 4],
    cell_size_m: f64,
    width: usize,
    height: usize,
) -> (usize, usize, usize, usize) {
    let epsilon = f64::from(SAMPLE_EPSILON_M);
    let min_world_x = triangle
        .iter()
        .map(|point| point.x)
        .fold(f64::INFINITY, f64::min)
        - epsilon;
    let min_world_z = triangle
        .iter()
        .map(|point| point.z)
        .fold(f64::INFINITY, f64::min)
        - epsilon;
    let max_world_x = triangle
        .iter()
        .map(|point| point.x)
        .fold(f64::NEG_INFINITY, f64::max)
        + epsilon;
    let max_world_z = triangle
        .iter()
        .map(|point| point.z)
        .fold(f64::NEG_INFINITY, f64::max)
        + epsilon;
    let cell_x = |world_x: f64| {
        (((world_x - bounds_xz[0]) / cell_size_m).floor() as isize).clamp(0, width as isize - 1)
            as usize
    };
    let cell_z = |world_z: f64| {
        (((world_z - bounds_xz[1]) / cell_size_m).floor() as isize).clamp(0, height as isize - 1)
            as usize
    };
    (
        cell_x(min_world_x),
        cell_z(min_world_z),
        cell_x(max_world_x),
        cell_z(max_world_z),
    )
}

#[cfg(test)]
mod tests;
