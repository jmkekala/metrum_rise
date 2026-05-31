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
    NODE_OVERLAY_MIN_AREA_M2, NODE_OVERLAY_NUMERIC_DUST_WIDTH_M, NodeOverlayContour,
    NodeOverlayPoint, NodeOverlayShape, NodeOverlayShapes, RoadSurfaceBandKind, RoadSurfaceSystem,
    RoadSurfaceVisualNodePieceKind, SurfaceCdt,
};
use crate::simulation::network::surface::keys::SURFACE_XZ_KEY_SCALE;
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

const NODE_TRIANGULATION_CARRIAGEWAY_GUIDE_SPACING_M: f64 = 12.0;
const NODE_TRIANGULATION_GUIDE_MIN_HEIGHT_DELTA_M: f64 = 0.01;
const NODE_TRIANGULATION_GUIDE_PLANE_MAX_RESIDUAL_M: f64 = 0.005;
const NODE_TRIANGULATION_MAX_GUIDE_SEGMENTS_PER_EDGE: usize = 64;

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

    fn distance_key_units_sq(self, other: Self) -> i128 {
        let dx = i128::from(self.x_mm - other.x_mm);
        let dz = i128::from(self.z_mm - other.z_mm);
        dx * dx + dz * dz
    }
}

impl NodeTriangulationHeightKey {
    fn from_arrangement_vertex(vertex: &NodeArrangementVertex) -> Self {
        Self(quantize_m(vertex.height_m()))
    }
}

fn node_triangulation_dust_key_units() -> i64 {
    (f64::from(NODE_OVERLAY_NUMERIC_DUST_WIDTH_M) * SURFACE_XZ_KEY_SCALE).round() as i64
}

fn same_authority_numeric_dust_vertex(
    point_key: NodeTriangulationPointKey,
    height_key: NodeTriangulationHeightKey,
    grade_authority: NodeGradeVertexAuthority,
    vertices: &[NodeTriangulatedVertex],
    vertex_lookup: &BTreeMap<NodeTriangulationPointKey, (usize, NodeTriangulationHeightKey)>,
) -> Option<usize> {
    let dust_key_units = node_triangulation_dust_key_units();
    if dust_key_units <= 0 {
        return None;
    }
    let dust_key_units_sq = i128::from(dust_key_units) * i128::from(dust_key_units);
    let range_start = NodeTriangulationPointKey {
        x_mm: point_key.x_mm - dust_key_units,
        z_mm: i64::MIN,
    };
    let range_end = NodeTriangulationPointKey {
        x_mm: point_key.x_mm + dust_key_units,
        z_mm: i64::MAX,
    };
    vertex_lookup
        .range(range_start..=range_end)
        .filter_map(|(candidate_key, (candidate_index, candidate_height_key))| {
            if *candidate_height_key != height_key
                || point_key.distance_key_units_sq(*candidate_key) > dust_key_units_sq
            {
                return None;
            }
            let candidate = vertices.get(*candidate_index)?;
            same_height_authority_for_numeric_dust(candidate.grade_authority, grade_authority)
                .then_some((
                    point_key.distance_key_units_sq(*candidate_key),
                    *candidate_key,
                    *candidate_index,
                ))
        })
        .min_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)))
        .map(|(_, _, index)| index)
}

fn same_height_authority_for_numeric_dust(
    existing: NodeGradeVertexAuthority,
    incoming: NodeGradeVertexAuthority,
) -> bool {
    existing.owner == incoming.owner
        && existing.height_field_id == incoming.height_field_id
        && existing.height_key == incoming.height_key
        && existing.source_provenance == incoming.source_provenance
}

#[cfg(test)]
mod tests;
