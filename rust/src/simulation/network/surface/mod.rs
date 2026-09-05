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
pub use edge::{PreviewRoadSurfaceResult, RoadPreviewValidation};
pub use node::RoadSurfaceVisualNodePiece;
pub use span::RoadSurfaceVisualSpanPiece;
pub use system::RoadSurfaceSystem;

pub(crate) use cache::ChunkCacheKind;
pub(crate) use cache::RoadSurfaceTopologyUndo;
pub(crate) use earthwork::{
    RoadSurfaceEarthworkBoundarySegment, RoadSurfaceEarthworkFaceKind,
    RoadSurfaceEarthworkFaceSource, RoadSurfaceEarthworkRenderFace,
    RoadSurfaceEarthworkSupportPolicy,
};
#[cfg(test)]
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
#[derive(Clone, Debug, PartialEq)]
pub struct RoadSurfaceVisualPolygon {
    /// Ordered world-space polygon points.
    pub points_world: Vec<RoadVec3>,
    /// Deterministic cached triangles covering the polygon in world space.
    pub triangles_world: Vec<[RoadVec3; 3]>,
}

const ROAD_SURFACE_QUERY_GRID_BASE_CELL_M: f64 = 4.0;
const ROAD_SURFACE_QUERY_GRID_MAX_CELLS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq)]
struct RoadSurfaceIndexedTriangle {
    triangle: [RoadVec3; 3],
    carriageway: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct RoadSurfaceTriangleQueryIndex {
    bounds_xz: [f64; 4],
    cell_size_m: f64,
    width: usize,
    height: usize,
    triangles: Vec<RoadSurfaceIndexedTriangle>,
    cell_offsets: Vec<u32>,
    cell_triangle_indices: Vec<u32>,
}

#[derive(Clone)]
struct RoadSurfaceTerrainLoopGradingCacheEntry {
    terrain_source_generation: u64,
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
    span_index: Option<&'a RoadSurfaceTriangleQueryIndex>,
    carriageway_only: bool,
}

impl RoadSurfaceVisualPolygon {
    /// Builds a polygon from its deterministic boundary and triangulation.
    pub(crate) fn from_parts(
        points_world: Vec<RoadVec3>,
        triangles_world: Vec<[RoadVec3; 3]>,
    ) -> Self {
        Self {
            points_world,
            triangles_world,
        }
    }
}

impl RoadSurfaceTriangleQueryIndex {
    fn from_surface_polygons(
        road: &[RoadSurfaceVisualPolygon],
        curb: &[RoadSurfaceVisualPolygon],
        sidewalk: &[RoadSurfaceVisualPolygon],
    ) -> Self {
        let mut triangles = Vec::new();
        for (polygons, carriageway) in [(road, true), (curb, false), (sidewalk, false)] {
            triangles.extend(polygons.iter().flat_map(|polygon| {
                polygon
                    .triangles_world
                    .iter()
                    .copied()
                    .map(move |triangle| RoadSurfaceIndexedTriangle {
                        triangle,
                        carriageway,
                    })
            }));
        }
        if triangles.is_empty() {
            return Self::default();
        }

        let mut bounds_xz = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for indexed in &triangles {
            for point in indexed.triangle {
                bounds_xz[0] = bounds_xz[0].min(point.x);
                bounds_xz[1] = bounds_xz[1].min(point.z);
                bounds_xz[2] = bounds_xz[2].max(point.x);
                bounds_xz[3] = bounds_xz[3].max(point.z);
            }
        }

        let mut cell_size_m = ROAD_SURFACE_QUERY_GRID_BASE_CELL_M;
        let (mut width, mut height) = query_grid_dimensions(bounds_xz, cell_size_m);
        while width.saturating_mul(height) > ROAD_SURFACE_QUERY_GRID_MAX_CELLS {
            cell_size_m *= 2.0;
            (width, height) = query_grid_dimensions(bounds_xz, cell_size_m);
        }
        let cell_count = width.saturating_mul(height);
        let mut counts = vec![0_u32; cell_count];
        for indexed in &triangles {
            let (min_x, min_z, max_x, max_z) = query_grid_triangle_cell_bounds(
                indexed.triangle,
                bounds_xz,
                cell_size_m,
                width,
                height,
            );
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
        let mut cell_triangle_indices =
            vec![0_u32; cell_offsets.last().copied().unwrap_or(0) as usize];
        let mut cursors = cell_offsets[..cell_count].to_vec();
        for (triangle_idx, indexed) in triangles.iter().enumerate() {
            let (min_x, min_z, max_x, max_z) = query_grid_triangle_cell_bounds(
                indexed.triangle,
                bounds_xz,
                cell_size_m,
                width,
                height,
            );
            for z in min_z..=max_z {
                for x in min_x..=max_x {
                    let cell_idx = z * width + x;
                    let cursor = &mut cursors[cell_idx];
                    cell_triangle_indices[*cursor as usize] = triangle_idx as u32;
                    *cursor += 1;
                }
            }
        }

        Self {
            bounds_xz,
            cell_size_m,
            width,
            height,
            triangles,
            cell_offsets,
            cell_triangle_indices,
        }
    }

    fn cell_triangle_indices(&self, point: RoadVec2) -> &[u32] {
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
        &self.cell_triangle_indices[start..end]
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
