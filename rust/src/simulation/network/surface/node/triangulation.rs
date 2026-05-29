//! Spade-backed triangulation for canonical node-owned height regions.

use super::arrangement::{
    NodeArrangement, NodeArrangementVertex, NodeArrangementVertexId, NodeBandHeightFieldId,
    NodeBandOwner, NodeExplicitVerticalStepSegment, NodeOwnedRegion,
};
use super::backend::{RoadVec2, RoadVec3};
use super::height::{NodeGradeCarrierDecision, NodeGradeVertexAuthority};
use super::indices::normalized_vertex_edge;
use super::keys::{SurfaceHeightMmKey, SurfaceXzKey};
use super::{
    NODE_OVERLAY_MIN_AREA_M2, NodeOverlayContour, NodeOverlayPoint, NodeOverlayShape,
    NodeOverlayShapes, RoadSurfaceBandKind, RoadSurfaceSystem, RoadSurfaceVisualNodePieceKind,
    SurfaceCdt,
};
use i_overlay::core::overlay_rule::OverlayRule;
use spade::{Point2, Triangulation};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeTriangulationSolution {
    pub(crate) node_id: u32,
    pub(crate) piece_kind: RoadSurfaceVisualNodePieceKind,
    pub(crate) regions: Vec<NodeTriangulatedRegion>,
    pub(crate) explicit_vertical_step_segments: Vec<NodeExplicitVerticalStepSegment>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeTriangulatedRegion {
    pub(crate) kind: RoadSurfaceBandKind,
    pub(crate) owner: NodeBandOwner,
    pub(crate) height_field_id: NodeBandHeightFieldId,
    pub(crate) vertices: Vec<NodeTriangulatedVertex>,
    pub(crate) boundary_constraints: Vec<[usize; 2]>,
    pub(crate) triangles: Vec<NodeTriangulatedTriangle>,
    pub(crate) area_m2: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeTriangulatedVertex {
    pub(crate) point_world: RoadVec3,
    pub(crate) height_field_id: NodeBandHeightFieldId,
    pub(crate) grade_authority: NodeGradeVertexAuthority,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct NodeTriangulatedTriangle {
    pub(crate) vertices: [usize; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NodeTriangulationError {
    EmptyHeightSolution {
        node_id: u32,
    },
    EmptyRegionShape {
        node_id: u32,
        region_index: usize,
    },
    DegenerateRegionContour {
        node_id: u32,
        region_index: usize,
        contour_index: usize,
        vertex_count: usize,
    },
    DuplicateVertexHeightConflict {
        node_id: u32,
        region_index: usize,
        x_mm: i64,
        z_mm: i64,
        existing_height_mm: i64,
        incoming_height_mm: i64,
    },
    InvalidConstraint {
        node_id: u32,
        region_index: usize,
        constraint_count: usize,
    },
    CdtBuildFailed {
        node_id: u32,
        region_index: usize,
    },
    EmptyTriangulation {
        node_id: u32,
        region_index: usize,
    },
    BooleanOperationFailed {
        node_id: u32,
        region_index: usize,
        stage: &'static str,
    },
    TriangleCoverageMismatch {
        node_id: u32,
        region_index: usize,
        missing_area_m2: f32,
        extra_area_m2: f32,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeTriangulationPointKey {
    x_mm: i64,
    z_mm: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NodeTriangulationHeightKey(i64);

mod build;
mod coverage;
mod regions;
mod vertices;

fn quantize_m(value: f64) -> i64 {
    SurfaceHeightMmKey::from_m_f64(value).as_i64()
}

impl NodeTriangulationPointKey {
    fn from_arrangement_vertex(vertex: &NodeArrangementVertex) -> Self {
        let key = SurfaceXzKey::from_road_xz(vertex.point_xz());
        Self {
            x_mm: key.x_key(),
            z_mm: key.z_key(),
        }
    }

    fn from_world(point: RoadVec3) -> Self {
        let key = SurfaceXzKey::from_world_xz(point);
        Self {
            x_mm: key.x_key(),
            z_mm: key.z_key(),
        }
    }

    fn road_xz(self) -> RoadVec2 {
        SurfaceXzKey::from_raw_keys(self.x_mm, self.z_mm).to_road_xz()
    }
}

impl NodeTriangulationHeightKey {
    fn from_arrangement_vertex(vertex: &NodeArrangementVertex) -> Self {
        Self(quantize_m(vertex.height_m()))
    }
}

#[cfg(test)]
mod tests;
