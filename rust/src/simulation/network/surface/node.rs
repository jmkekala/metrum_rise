//! Explicit visual node-piece construction and incident-edge classification.

use self::{
    arrangement::{
        NodeArrangement, NodeArrangementKey, NodeBandOwner, NodeExplicitVerticalStepSegment,
    },
    boundary::{
        ArrangementBoundaryPointKey, ArrangementSegmentParameter, NodeBoundaryExportError,
        NodeFootprintBoundaryExportSources, arrangement_boundary_point_to_world,
        boundary_points_numeric_area_budget_m2,
        node_earthwork_boundary_segments_from_footprint_loops,
    },
    input::NodeInputExtractionError,
    validation::NodeValidationReport,
};
use super::{
    CURB_STEP_HEIGHT_M, CompiledNodeKind, IncidentEdgeSide, IncidentMouthProfile,
    IncidentSurfaceEdge, OrderedIncidentPieceMouth, RoadSurfaceBandKind,
    RoadSurfaceEarthworkBoundarySegment, RoadSurfaceEarthworkFaceSource,
    RoadSurfaceEarthworkRenderFace, RoadSurfaceSystem, RoadSurfaceTerrainClipLoop,
    RoadSurfaceVisualNodePieceKind, RoadSurfaceVisualPolygon, SAMPLE_EPSILON_M,
    band_semantics::ordered_raised_step_kinds,
};
pub(super) use super::{
    IncidentMouthBand, NODE_OVERLAY_MIN_AREA_M2, NODE_OVERLAY_NUMERIC_DUST_WIDTH_M,
    NodeOverlayContour, NodeOverlayPoint, NodeOverlayShape, NodeOverlayShapes,
    RoadSurfaceVisualNodeCompileInput, SurfaceCdt,
    backend::{self, RoadVec2, RoadVec3},
    band_semantics, indices, keys, paths, segments,
};
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::terrain::TerrainSystem;
use std::collections::BTreeSet;

// Node-piece classification threshold.
const PASS_THROUGH_DOT_THRESHOLD: f32 = 0.98;

struct NodeExportTopHeightContext {
    carriageway_height_keys: BTreeSet<(arrangement::NodeArrangementKey, i64)>,
    flat_carriageway_height_mm: Option<i64>,
    explicit_step_lower_edges: Vec<NodeExportExplicitStepLowerEdge>,
}

#[derive(Clone, Copy)]
struct NodeExportExplicitStepLowerEdge {
    raised_owner: NodeBandOwner,
    step_start: arrangement::NodeArrangementKey,
    step_end: arrangement::NodeArrangementKey,
    edge_start: arrangement::NodeArrangementKey,
    edge_end: arrangement::NodeArrangementKey,
    edge_start_height_mm: i64,
    edge_end_height_mm: i64,
}

impl NodeExportTopHeightContext {
    fn from_arrangement(
        arrangement: &NodeArrangement,
        explicit_vertical_step_segments: &[NodeExplicitVerticalStepSegment],
    ) -> Self {
        let carriageway_height_keys = arrangement
            .vertices()
            .iter()
            .filter(|vertex| {
                vertex
                    .owners()
                    .iter()
                    .any(|owner| owner.kind() == RoadSurfaceBandKind::Carriageway)
            })
            .map(|vertex| (vertex.key(), vertex.height_mm()))
            .collect::<BTreeSet<_>>();
        Self {
            flat_carriageway_height_mm: flat_carriageway_height_mm(&carriageway_height_keys),
            carriageway_height_keys,
            explicit_step_lower_edges: explicit_step_lower_edges(
                arrangement,
                explicit_vertical_step_segments,
            ),
        }
    }

    fn flat_raised_top_height_mm(&self, owner: NodeBandOwner) -> Option<i64> {
        if !matches!(
            owner.kind(),
            RoadSurfaceBandKind::CurbOrShoulder | RoadSurfaceBandKind::Sidewalk
        ) {
            return None;
        }
        self.flat_carriageway_height_mm
            .map(|height_mm| height_mm + curb_step_height_mm())
    }

    fn raised_owner_vertex_matches_explicit_step_lower_height(
        &self,
        owner: NodeBandOwner,
        key: arrangement::NodeArrangementKey,
        height_mm: i64,
    ) -> bool {
        self.explicit_step_lower_edges.iter().any(|edge| {
            edge.raised_owner == owner
                && key.lies_on_segment(edge.step_start, edge.step_end)
                && edge.lower_height_mm_at(key) == Some(height_mm)
        })
    }
}

fn flat_carriageway_height_mm(
    carriageway_height_keys: &BTreeSet<(arrangement::NodeArrangementKey, i64)>,
) -> Option<i64> {
    let mut heights = carriageway_height_keys
        .iter()
        .map(|(_, height_mm)| *height_mm);
    let first = heights.next()?;
    heights.all(|height_mm| height_mm == first).then_some(first)
}

fn curb_step_height_mm() -> i64 {
    (f64::from(CURB_STEP_HEIGHT_M) * 1000.0).round() as i64
}

impl NodeExportExplicitStepLowerEdge {
    fn lower_height_mm_at(self, key: arrangement::NodeArrangementKey) -> Option<i64> {
        if !key.lies_on_segment(self.edge_start, self.edge_end) {
            return None;
        }
        let parameter = segments::overlay_segment_parameter(
            segments::arrangement_key(key),
            segments::arrangement_key(self.edge_start),
            segments::arrangement_key(self.edge_end),
        )
        .or_else(|| {
            segments::exact_line_parameter(
                segments::arrangement_key(key),
                segments::arrangement_key(self.edge_start),
                segments::arrangement_key(self.edge_end),
            )
        })?;
        Some(segments::interpolate_height_i64(
            self.edge_start_height_mm,
            self.edge_end_height_mm,
            parameter,
        ))
    }
}

fn explicit_step_lower_edges(
    arrangement: &NodeArrangement,
    explicit_vertical_step_segments: &[NodeExplicitVerticalStepSegment],
) -> Vec<NodeExportExplicitStepLowerEdge> {
    let mut lower_edges = Vec::new();
    for segment in explicit_vertical_step_segments.iter().copied() {
        let owners = [segment.owner(), segment.opposite_owner()];
        let Some(lower_owner) = owners
            .iter()
            .copied()
            .find(|owner| segment.owner_matches_height_side(*owner, true))
        else {
            continue;
        };
        if lower_owner.kind() != RoadSurfaceBandKind::Carriageway {
            continue;
        }
        let Some(raised_owner) = owners
            .iter()
            .copied()
            .find(|owner| segment.owner_matches_height_side(*owner, false))
        else {
            continue;
        };
        lower_edges.extend(
            arrangement
                .edges()
                .iter()
                .filter(|edge| edge.owner() == lower_owner)
                .filter_map(|edge| {
                    let start = arrangement.vertices().get(edge.start().index())?;
                    let end = arrangement.vertices().get(edge.end().index())?;
                    Some(NodeExportExplicitStepLowerEdge {
                        raised_owner,
                        step_start: segment.start(),
                        step_end: segment.end(),
                        edge_start: start.key(),
                        edge_end: end.key(),
                        edge_start_height_mm: start.height_mm(),
                        edge_end_height_mm: end.height_mm(),
                    })
                }),
        );
    }
    lower_edges
}

fn node_export_top_height_m(
    owner: NodeBandOwner,
    source_kind: RoadSurfaceBandKind,
    key: arrangement::NodeArrangementKey,
    height_m: f64,
    height_mm: i64,
    context: &NodeExportTopHeightContext,
) -> f64 {
    let height_mm = node_export_top_height_mm(owner, source_kind, key, height_mm, context);
    if height_mm == keys::SurfaceHeightMmKey::from_m_f64(height_m).as_i64() {
        height_m
    } else {
        height_mm as f64 / keys::SURFACE_MM_PER_M
    }
}

fn node_export_top_height_mm(
    owner: NodeBandOwner,
    source_kind: RoadSurfaceBandKind,
    key: arrangement::NodeArrangementKey,
    height_mm: i64,
    context: &NodeExportTopHeightContext,
) -> i64 {
    if let Some(flat_raised_top_height_mm) = context.flat_raised_top_height_mm(owner) {
        return flat_raised_top_height_mm;
    }
    if node_export_top_height_needs_curb_lift(owner, source_kind, key, height_mm, context) {
        height_mm + curb_step_height_mm()
    } else {
        height_mm
    }
}

fn node_export_top_height_needs_curb_lift(
    owner: NodeBandOwner,
    source_kind: RoadSurfaceBandKind,
    key: arrangement::NodeArrangementKey,
    height_mm: i64,
    context: &NodeExportTopHeightContext,
) -> bool {
    if !matches!(
        owner.kind(),
        RoadSurfaceBandKind::CurbOrShoulder | RoadSurfaceBandKind::Sidewalk
    ) {
        return false;
    }
    source_kind == RoadSurfaceBandKind::Carriageway
        || context.raised_owner_vertex_matches_explicit_step_lower_height(owner, key, height_mm)
}

fn node_export_top_source_kind(
    owner: NodeBandOwner,
    fallback_source_kind: RoadSurfaceBandKind,
    vertex_key: arrangement::NodeArrangementKey,
    vertex_height_mm: i64,
    context: &NodeExportTopHeightContext,
) -> RoadSurfaceBandKind {
    if matches!(
        owner.kind(),
        RoadSurfaceBandKind::CurbOrShoulder | RoadSurfaceBandKind::Sidewalk
    ) && (context
        .carriageway_height_keys
        .contains(&(vertex_key, vertex_height_mm))
        || context.raised_owner_vertex_matches_explicit_step_lower_height(
            owner,
            vertex_key,
            vertex_height_mm,
        ))
    {
        RoadSurfaceBandKind::Carriageway
    } else {
        fallback_source_kind
    }
}

pub(crate) mod arrangement;
mod arrangement_faces;
pub(crate) mod boundary;
mod boundary_edges;
mod compile;
mod export;
pub(crate) mod height;
mod incident;
pub(crate) mod input;
pub(crate) mod joins;
pub(crate) mod ownership;
mod piece;
pub(crate) mod rails;
pub(crate) mod terminal;
#[cfg(test)]
mod tests;
pub(crate) mod triangulation;
pub(crate) mod validation;
mod vertical_faces;

pub use piece::RoadSurfaceVisualNodePiece;
pub(crate) use piece::{
    NodeBooleanDebugSnapshot, NodeCornerTrimDebug, NodeCornerTrimSideJoinIntersectionDebug,
    NodeEarthworkOwnerSource, NodeFootprintBoundaryDirectSource,
    NodeFootprintBoundarySegmentSource, NodeFootprintBoundaryVertexSource, NodeOwnedRegion,
    NodePostBooleanOwnedRegionDebug, NodeSideJoinContourDebug, NodeSideJoinGapDebug,
    NodeSideJoinMaterialTrimDebug, NodeSurfaceRegionResult, NodeTopSurfacePolygonSource,
    NodeTopSurfaceVertexSource, RoadSurfaceVerticalFaceSource,
};
pub(in crate::simulation::network::surface::node) use vertical_faces::RoadSurfaceRaisedStepFace;
