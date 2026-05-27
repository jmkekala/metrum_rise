//! Debug extraction helpers for compiled road-surface state.

use super::{
    IncidentEdgeSide, IncidentMouthProfile, NodeFootprintBoundarySegmentSource,
    NodeFootprintBoundaryVertexSource, NodeOverlayContour, NodeOverlayPoint, NodeOverlayShape,
    NodeOverlayShapes, NodeTopSurfacePolygonSource, RoadSurfaceBandKind,
    RoadSurfaceEarthworkFaceKind, RoadSurfaceEarthworkFaceSource, RoadSurfaceEarthworkRenderFace,
    RoadSurfaceEarthworkSupportPolicy, RoadSurfaceSection, RoadSurfaceSpanBandOwner,
    RoadSurfaceSpanOwnedRegion, RoadSurfaceSpanRegionRole, RoadSurfaceSystem,
    RoadSurfaceVerticalFaceSource, RoadSurfaceVisualNodePiece, RoadSurfaceVisualPolygon,
    RoadSurfaceVisualSpanPiece, SAMPLE_EPSILON_M, SurfaceChunkKey,
    arrangement::{NodeArrangementKey, NodeBandOwner, NodeExplicitVerticalStepSegment},
    backend,
    band_semantics::ordered_raised_step_kinds,
    height::{NodeGradeCarrierDecision, NodeGradeVertexAuthority, NodeHeightAuthoritySource},
    keys::{SurfaceHeightMmKey, SurfaceXzKey},
};
use crate::config;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::{Vector2, Vector3};
use i_overlay::core::overlay_rule::OverlayRule;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

mod coverage;
mod geometry_dump;
mod line_data;
mod node_dump;
mod support;
#[cfg(test)]
mod tests;
mod vertical_steps;

#[derive(Default)]
pub(crate) struct RoadSurfaceDebugData {
    pub(crate) section_lines: Vec<Vector3>,
    pub(crate) band_lines: Vec<Vector3>,
    pub(crate) piece_boundary_lines: Vec<Vector3>,
    pub(crate) earthwork_chunk_lines: Vec<Vector3>,
}

const DEBUG_MAX_PROBLEM_SAMPLES: usize = 12;
const DEBUG_VERTEX_MATCH_TOLERANCE_M: f32 = 0.004;
const DEBUG_VERTEX_NEAR_TOLERANCE_M: f32 = 0.002;

#[cfg(test)]
#[derive(Clone, Copy)]
struct DebugTopVertex {
    material: &'static str,
    point: backend::RoadVec3,
}

#[derive(Clone, Copy)]
struct DebugClosestTopVertex {
    material: &'static str,
    point: backend::RoadVec3,
    xz_error_m: f32,
    y_delta_m: f32,
}

#[derive(Clone, Copy)]
struct DebugMouthTopAnchor {
    point_index: usize,
    band_index: usize,
    role: &'static str,
    material: &'static str,
    point: backend::RoadVec3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct DebugRenderVertexKey {
    x_key: i64,
    y_mm: i64,
    z_key: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct DebugRenderXzVertexKey {
    x_key: i64,
    z_key: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct DebugRenderEdgeKey {
    start: DebugRenderVertexKey,
    end: DebugRenderVertexKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct DebugRenderXzEdgeKey {
    start: DebugRenderXzVertexKey,
    end: DebugRenderXzVertexKey,
}

#[derive(Clone, Copy)]
struct DebugBoundaryOwner {
    region_index: usize,
    kind: RoadSurfaceBandKind,
    owner_index: usize,
}

#[derive(Clone, Copy)]
struct DebugTopBoundaryEdge {
    owner: DebugBoundaryOwner,
    start: backend::RoadVec3,
    end: backend::RoadVec3,
    key: DebugRenderEdgeKey,
    xz_key: DebugRenderXzEdgeKey,
    avg_y_m: f32,
}

#[derive(Clone, Copy)]
struct DebugVerticalFaceSpanEdges {
    lower_start: backend::RoadVec3,
    lower_end: backend::RoadVec3,
    upper_start: backend::RoadVec3,
    upper_end: backend::RoadVec3,
}

#[derive(Clone, Copy)]
struct DebugExpectedVerticalStep {
    lower: DebugTopBoundaryEdge,
    upper: DebugTopBoundaryEdge,
}

struct DebugCanonicalVerticalStep {
    explicit_vertical_step_index: usize,
    segment: NodeExplicitVerticalStepSegment,
    lower_owner: NodeBandOwner,
    raised_owner: NodeBandOwner,
    lower_top_matches: Vec<DebugTopBoundaryEdge>,
    raised_top_matches: Vec<DebugTopBoundaryEdge>,
}

#[derive(Default)]
struct DebugMatchStats {
    total: usize,
    problem_count: usize,
    max_xz_error_m: f32,
    max_y_error_m: f32,
}

#[derive(Default)]
struct DebugCoverageStats {
    footprint_area_m2: f32,
    top_area_m2: f32,
    missing_area_m2: f32,
    extra_area_m2: f32,
    area_budget_m2: f32,
    missing_shape_count: usize,
    extra_shape_count: usize,
}

impl DebugRenderVertexKey {
    fn from_point(point: backend::RoadVec3) -> Self {
        let xz_key = SurfaceXzKey::from_world_xz(point);
        Self {
            x_key: xz_key.x_key(),
            y_mm: SurfaceHeightMmKey::from_m_f64(point.y).as_i64(),
            z_key: xz_key.z_key(),
        }
    }

    fn xz(self) -> DebugRenderXzVertexKey {
        DebugRenderXzVertexKey {
            x_key: self.x_key,
            z_key: self.z_key,
        }
    }
}

impl DebugRenderEdgeKey {
    fn normalized(start: backend::RoadVec3, end: backend::RoadVec3) -> Option<Self> {
        let start = DebugRenderVertexKey::from_point(start);
        let end = DebugRenderVertexKey::from_point(end);
        if start == end {
            return None;
        }
        Some(if start <= end {
            Self { start, end }
        } else {
            Self {
                start: end,
                end: start,
            }
        })
    }

    fn xz(self) -> DebugRenderXzEdgeKey {
        DebugRenderXzEdgeKey::normalized(self.start.xz(), self.end.xz())
    }
}

impl DebugRenderXzEdgeKey {
    fn normalized(start: DebugRenderXzVertexKey, end: DebugRenderXzVertexKey) -> Self {
        if start <= end {
            Self { start, end }
        } else {
            Self {
                start: end,
                end: start,
            }
        }
    }

    fn from_arrangement_segment(start: NodeArrangementKey, end: NodeArrangementKey) -> Self {
        Self::normalized(
            DebugRenderXzVertexKey {
                x_key: start.x_key(),
                z_key: start.z_key(),
            },
            DebugRenderXzVertexKey {
                x_key: end.x_key(),
                z_key: end.z_key(),
            },
        )
    }
}
