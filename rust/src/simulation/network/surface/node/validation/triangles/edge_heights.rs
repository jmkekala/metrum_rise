//! Cross-region triangle edge height validation.

use super::super::super::NODE_OVERLAY_NUMERIC_DUST_WIDTH_M;
use super::super::super::arrangement::{
    NodeBandOwner, NodeExplicitVerticalStepSegment, owners_form_explicit_vertical_step_pair,
    source_authorities_form_side_join_asphalt_sidewalk_split,
};
use super::super::super::band_semantics::{
    raised_step_kinds_can_contact, raised_step_requires_exact_constraint_span,
};
use super::super::super::height::{
    NodeGradeCarrierDecision, NodeGradeVertexAuthority, NodeHeightCarrierProvenanceKey,
};
use super::super::super::keys::{SURFACE_XZ_KEY_SCALE, SurfaceSegmentParameter};
use super::super::super::segments;
use super::super::super::triangulation::{NodeTriangulatedRegion, NodeTriangulationSolution};
use super::super::report::{
    NodeExplicitStepSegmentDiagnostic, NodeGeometryBackend, NodeGeometryDiagnostic,
    NodeGeometryDiagnosticKind, push_validation_diagnostic,
};
use super::super::{
    NodeValidationEdgeKey, NodeValidationPointKey, point_key_from_world, quantize_m,
};
use super::{edge_indices_valid, triangle_edges, triangle_indices_valid};
use std::collections::BTreeMap;

mod diagnostics;
mod index;
mod steps;

use diagnostics::push_triangle_edge_height_conflict;
use steps::cross_region_edges_form_explicit_vertical_step;

const SHARED_HEIGHT_RAISED_STEP_EDGE_DUST_MM: i64 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct HeightedTriangleEdge {
    region_index: usize,
    start_height_mm: i64,
    end_height_mm: i64,
    start_source_provenance: Option<NodeHeightCarrierProvenanceKey>,
    end_source_provenance: Option<NodeHeightCarrierProvenanceKey>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HeightedOwnedCoverageEdge {
    edge: NodeValidationEdgeKey,
    heighted_edge: HeightedTriangleEdge,
}

#[derive(Default)]
struct ValidationTriangleEdgeIndex {
    by_edge: BTreeMap<NodeValidationEdgeKey, Vec<HeightedTriangleEdge>>,
    by_owner_coverage: BTreeMap<NodeBandOwner, Vec<HeightedOwnedCoverageEdge>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct HeightedEdgeCoverageInterval {
    start: SurfaceSegmentParameter,
    end: SurfaceSegmentParameter,
}

pub(super) fn validate_cross_region_triangle_edge_heights(
    solution: &NodeTriangulationSolution,
    diagnostics: &mut Vec<NodeGeometryDiagnostic>,
) {
    let edge_index = ValidationTriangleEdgeIndex::from_solution(solution);

    for (edge_key, heighted_edges) in &edge_index.by_edge {
        let mut heighted_edges = heighted_edges.clone();
        heighted_edges.sort_unstable();
        heighted_edges.dedup();
        'edge: for left_index in 0..heighted_edges.len() {
            for right_index in left_index + 1..heighted_edges.len() {
                let left = heighted_edges[left_index];
                let right = heighted_edges[right_index];
                if left.region_index == right.region_index
                    || (left.start_height_mm == right.start_height_mm
                        && left.end_height_mm == right.end_height_mm)
                    || cross_region_edges_have_distinct_source_provenance_at_conflict(
                        solution, left, right,
                    )
                    || cross_region_edges_form_source_authorized_side_join_asphalt_sidewalk_split(
                        solution, *edge_key, left, right,
                    )
                    || cross_region_edges_form_shared_height_raised_step_dust_match(
                        solution, left, right,
                    )
                    || cross_region_edges_form_explicit_vertical_step(
                        solution,
                        &edge_index,
                        *edge_key,
                        left,
                        right,
                    )
                {
                    continue;
                }
                push_triangle_edge_height_conflict(solution, diagnostics, *edge_key, left, right);
                break 'edge;
            }
        }
    }
}

fn cross_region_edges_form_shared_height_raised_step_dust_match(
    solution: &NodeTriangulationSolution,
    left: HeightedTriangleEdge,
    right: HeightedTriangleEdge,
) -> bool {
    let Some(left_region) = solution.regions.get(left.region_index) else {
        return false;
    };
    let Some(right_region) = solution.regions.get(right.region_index) else {
        return false;
    };
    if left_region.owner == right_region.owner
        || !shared_height_raised_step_pair(left_region.owner, right_region.owner)
    {
        return false;
    }

    let mut has_conflict = false;
    for (left_height, right_height) in [
        (left.start_height_mm, right.start_height_mm),
        (left.end_height_mm, right.end_height_mm),
    ] {
        let delta_mm = (left_height - right_height).abs();
        if delta_mm == 0 {
            continue;
        }
        has_conflict = true;
        if delta_mm > SHARED_HEIGHT_RAISED_STEP_EDGE_DUST_MM {
            return false;
        }
    }
    has_conflict
}

fn shared_height_raised_step_pair(owner: NodeBandOwner, opposite_owner: NodeBandOwner) -> bool {
    raised_step_kinds_can_contact(owner.kind(), opposite_owner.kind())
        && !raised_step_requires_exact_constraint_span(owner.kind(), opposite_owner.kind())
}

fn cross_region_edges_have_distinct_source_provenance_at_conflict(
    solution: &NodeTriangulationSolution,
    left: HeightedTriangleEdge,
    right: HeightedTriangleEdge,
) -> bool {
    let Some(left_region) = solution.regions.get(left.region_index) else {
        return false;
    };
    let Some(right_region) = solution.regions.get(right.region_index) else {
        return false;
    };
    if left_region.owner == right_region.owner || left_region.kind != right_region.kind {
        return false;
    }

    let mut has_conflict = false;
    for (left_height, right_height, left_provenance, right_provenance) in [
        (
            left.start_height_mm,
            right.start_height_mm,
            left.start_source_provenance,
            right.start_source_provenance,
        ),
        (
            left.end_height_mm,
            right.end_height_mm,
            left.end_source_provenance,
            right.end_source_provenance,
        ),
    ] {
        if left_height == right_height {
            continue;
        }
        has_conflict = true;
        let (Some(left_provenance), Some(right_provenance)) = (left_provenance, right_provenance)
        else {
            return false;
        };
        if left_provenance == right_provenance {
            return false;
        }
    }
    has_conflict
}

fn cross_region_edges_form_source_authorized_side_join_asphalt_sidewalk_split(
    solution: &NodeTriangulationSolution,
    edge: NodeValidationEdgeKey,
    left: HeightedTriangleEdge,
    right: HeightedTriangleEdge,
) -> bool {
    let Some(left_region) = solution.regions.get(left.region_index) else {
        return false;
    };
    let Some(right_region) = solution.regions.get(right.region_index) else {
        return false;
    };
    if left_region.owner == right_region.owner || left_region.kind == right_region.kind {
        return false;
    }

    let mut has_conflict = false;
    for (point, left_height, right_height, left_provenance, right_provenance) in [
        (
            edge.start,
            left.start_height_mm,
            right.start_height_mm,
            left.start_source_provenance,
            right.start_source_provenance,
        ),
        (
            edge.end,
            left.end_height_mm,
            right.end_height_mm,
            left.end_source_provenance,
            right.end_source_provenance,
        ),
    ] {
        if left_height == right_height {
            continue;
        }
        has_conflict = true;
        if !source_authorized_side_join_asphalt_sidewalk_endpoint(
            point,
            left_region,
            left_height,
            left_provenance,
            right_region,
            right_height,
            right_provenance,
        ) {
            return false;
        }
    }
    has_conflict
}

fn source_authorized_side_join_asphalt_sidewalk_endpoint(
    point: NodeValidationPointKey,
    left_region: &NodeTriangulatedRegion,
    left_height_mm: i64,
    left_provenance: Option<NodeHeightCarrierProvenanceKey>,
    right_region: &NodeTriangulatedRegion,
    right_height_mm: i64,
    right_provenance: Option<NodeHeightCarrierProvenanceKey>,
) -> bool {
    let Some(left_provenance) = left_provenance else {
        return false;
    };
    let Some(right_provenance) = right_provenance else {
        return false;
    };
    let left_authority =
        validation_edge_endpoint_authority(point, left_region, left_height_mm, left_provenance);
    let right_authority =
        validation_edge_endpoint_authority(point, right_region, right_height_mm, right_provenance);
    source_authorities_form_side_join_asphalt_sidewalk_split(left_authority, right_authority)
}

fn validation_edge_endpoint_authority(
    point: NodeValidationPointKey,
    region: &NodeTriangulatedRegion,
    height_mm: i64,
    source_provenance: NodeHeightCarrierProvenanceKey,
) -> NodeGradeVertexAuthority {
    let point_xz = point.surface_key().to_road_xz();
    let height_m = height_mm as f64 / 1000.0;
    NodeGradeVertexAuthority::new_with_source_provenance(
        point_xz,
        height_m,
        region.owner,
        region.height_field_id,
        NodeGradeCarrierDecision::SourceCarrier { authority: None },
        Some(source_provenance),
    )
}
