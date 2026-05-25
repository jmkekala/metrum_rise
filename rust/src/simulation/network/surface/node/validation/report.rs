//! Validation report types and debug serialization.

use super::super::arrangement::{NodeBandHeightFieldId, NodeBandOwner};
use super::super::height::NodeHeightAuthoritySource;
use super::super::keys::SurfaceXzKey;
use super::super::ownership::NodeSourceSegmentAuthorizationCandidate;
use super::super::triangulation::NodeTriangulationSolution;
use super::super::{RoadSurfaceBandKind, RoadSurfaceVisualNodePieceKind};
use serde_json::{Value, json};
use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeValidationReport {
    pub(crate) node_id: u32,
    pub(crate) piece_kind: RoadSurfaceVisualNodePieceKind,
    pub(crate) region_count: usize,
    pub(crate) triangle_count: usize,
    pub(crate) exposed_edge_count: usize,
    pub(crate) diagnostics: Vec<NodeGeometryDiagnostic>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeValidationError {
    pub(crate) report: NodeValidationReport,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeGeometryDiagnostic {
    pub(crate) node_id: u32,
    pub(crate) piece_kind: RoadSurfaceVisualNodePieceKind,
    pub(crate) stage: NodeGeometryStage,
    pub(crate) backend: NodeGeometryBackend,
    pub(crate) kind: NodeGeometryDiagnosticKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NodeGeometryStage {
    ContourGeneration,
    BooleanOwnership,
    NodeGrade,
    HeightEvaluation,
    Validation,
    CdtTriangulation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NodeGeometryBackend {
    CavalierContours,
    IOverlay,
    HeightCarrier,
    CanonicalKeys,
    Parry2d,
    Spade,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NodeGeometryDiagnosticKind {
    RejectedResidual {
        residual: NodeRejectedResidualKind,
        shape_count: usize,
        area_m2: f32,
    },
    NonExplicitBoundaryVertex {
        region_index: usize,
        owner: RoadSurfaceBandKind,
        owner_index: usize,
        height_field_id: NodeBandHeightFieldId,
        x_key: i64,
        z_key: i64,
        x_mm: i64,
        z_mm: i64,
        min_boundary_distance_mm: i64,
    },
    HeightConflict {
        x_mm: i64,
        z_mm: i64,
        existing_height_mm: i64,
        incoming_height_mm: i64,
    },
    SourceHeightFieldConflict {
        mouth_order_index: usize,
        band_index: usize,
        source_kind: RoadSurfaceBandKind,
        height_field_id: NodeBandHeightFieldId,
        owner: Option<NodeBandOwner>,
        existing_authority: NodeHeightAuthoritySource,
        incoming_authority: NodeHeightAuthoritySource,
        x_mm: i64,
        z_mm: i64,
        existing_height_mm: i64,
        incoming_height_mm: i64,
    },
    SharedSourceHeightConflict {
        x_mm: i64,
        z_mm: i64,
        kind: RoadSurfaceBandKind,
        owner: NodeBandOwner,
        opposite_owner: Option<NodeBandOwner>,
        height_field_id: Option<NodeBandHeightFieldId>,
        incoming_owner: NodeBandOwner,
        incoming_height_field_id: Option<NodeBandHeightFieldId>,
        constraint_index: Option<usize>,
        existing_authority: Option<NodeHeightAuthoritySource>,
        incoming_authority: Option<NodeHeightAuthoritySource>,
        existing_height_mm: i64,
        incoming_height_mm: i64,
    },
    CrossRegionHeightConflict {
        edge_start_x_key: i64,
        edge_start_z_key: i64,
        edge_end_x_key: i64,
        edge_end_z_key: i64,
        edge_start_x_mm: i64,
        edge_start_z_mm: i64,
        edge_end_x_mm: i64,
        edge_end_z_mm: i64,
        conflict_x_key: i64,
        conflict_z_key: i64,
        conflict_x_mm: i64,
        conflict_z_mm: i64,
        existing_region_index: usize,
        existing_owner: RoadSurfaceBandKind,
        existing_owner_index: usize,
        existing_start_height_mm: i64,
        existing_end_height_mm: i64,
        existing_conflict_height_mm: i64,
        incoming_region_index: usize,
        incoming_owner: RoadSurfaceBandKind,
        incoming_owner_index: usize,
        incoming_start_height_mm: i64,
        incoming_end_height_mm: i64,
        incoming_conflict_height_mm: i64,
        matching_explicit_step_segments: Vec<NodeExplicitStepSegmentDiagnostic>,
        non_matching_explicit_step_segments: Vec<NodeExplicitStepSegmentDiagnostic>,
    },
    HeightFieldFailure {
        reason: &'static str,
        mouth_order_index: Option<usize>,
        band_index: Option<usize>,
        kind: Option<RoadSurfaceBandKind>,
        source_kind: Option<RoadSurfaceBandKind>,
        height_field_id: Option<NodeBandHeightFieldId>,
        owner: Option<NodeBandOwner>,
        point_x_key: Option<i64>,
        point_z_key: Option<i64>,
        point_x_mm: Option<i64>,
        point_z_mm: Option<i64>,
        axis: Option<&'static str>,
        raw_parameter: Option<f64>,
    },
    MissingGradeAuthority {
        region_index: usize,
        contour_index: usize,
        x_mm: i64,
        z_mm: i64,
        owner: RoadSurfaceBandKind,
        owner_index: usize,
        height_field_id: NodeBandHeightFieldId,
        height_mm: i64,
    },
    OpenBoundary {
        region_index: usize,
        owner: RoadSurfaceBandKind,
        owner_index: usize,
        height_field_id: NodeBandHeightFieldId,
        vertex_index: Option<usize>,
        x_key: Option<i64>,
        z_key: Option<i64>,
        x_mm: Option<i64>,
        z_mm: Option<i64>,
        start_x_key: Option<i64>,
        start_z_key: Option<i64>,
        end_x_key: Option<i64>,
        end_z_key: Option<i64>,
        start_x_mm: Option<i64>,
        start_z_mm: Option<i64>,
        end_x_mm: Option<i64>,
        end_z_mm: Option<i64>,
        degree: usize,
    },
    DuplicateExposedEdge {
        region_index: Option<usize>,
        regions: Vec<NodeBoundaryRegionDiagnostic>,
        start_x_key: i64,
        start_z_key: i64,
        end_x_key: i64,
        end_z_key: i64,
        start_x_mm: i64,
        start_z_mm: i64,
        end_x_mm: i64,
        end_z_mm: i64,
        count: usize,
    },
    InvalidConstraint {
        region_index: usize,
        constraint_index: Option<usize>,
        reason: NodeInvalidConstraintReason,
    },
    TriangleCoverageMismatch {
        region_index: usize,
        missing_area_m2: f32,
        extra_area_m2: f32,
    },
    TriangleOverlap {
        region_index: usize,
        overlap_area_m2: f32,
    },
    SeamConstraintFailure {
        region_index: usize,
        owner: RoadSurfaceBandKind,
        owner_index: usize,
        opposite_owner: RoadSurfaceBandKind,
        opposite_owner_index: usize,
        start_x_mm: i64,
        start_z_mm: i64,
        end_x_mm: i64,
        end_z_mm: i64,
        reason: NodeSeamConstraintFailureReason,
    },
    AmbiguousOwnedBoundaryEdge {
        region_index: usize,
        owner: RoadSurfaceBandKind,
        owner_index: usize,
        opposite_owners: Vec<(RoadSurfaceBandKind, usize)>,
        start_x_mm: i64,
        start_z_mm: i64,
        end_x_mm: i64,
        end_z_mm: i64,
    },
    UnmaterializedRaisedStepAuthority {
        region_index: usize,
        owner: RoadSurfaceBandKind,
        owner_index: usize,
        opposite_owner: RoadSurfaceBandKind,
        opposite_owner_index: usize,
        start_x_mm: i64,
        start_z_mm: i64,
        end_x_mm: i64,
        end_z_mm: i64,
        source_constraint_indices: Vec<usize>,
    },
    NonCanonicalOwnedRegionVertex {
        owner: NodeBandOwner,
        point_x_key: i64,
        point_z_key: i64,
        point_x_mm: i64,
        point_z_mm: i64,
        canonical_x_key: i64,
        canonical_z_key: i64,
        canonical_x_mm: i64,
        canonical_z_mm: i64,
    },
    AmbiguousCanonicalOwnedRegionVertex {
        owner: NodeBandOwner,
        point_x_key: i64,
        point_z_key: i64,
        point_x_mm: i64,
        point_z_mm: i64,
        candidates: Vec<NodeCanonicalPointDiagnostic>,
    },
    AmbiguousSourceSegmentAuthorizedOwnedRegionVertex {
        owner: NodeBandOwner,
        point_x_key: i64,
        point_z_key: i64,
        point_x_mm: i64,
        point_z_mm: i64,
        source_kind: RoadSurfaceBandKind,
        source_mouth_order_index: usize,
        source_band_index: usize,
        candidates: Vec<NodeSourceSegmentAuthorizationCandidate>,
    },
    BackendFailure {
        reason: &'static str,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NodeRejectedResidualKind {
    Asphalt,
    Band(RoadSurfaceBandKind),
    NonRoad,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NodeInvalidConstraintReason {
    Degenerate,
    OutOfRange,
    Crossing,
    Duplicate,
    CdtRejected,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NodeSeamConstraintFailureReason {
    Missing,
    Ambiguous,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NodeCanonicalPointDiagnostic {
    pub(crate) x_key: i64,
    pub(crate) z_key: i64,
    pub(crate) x_mm: i64,
    pub(crate) z_mm: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NodeBoundaryRegionDiagnostic {
    pub(crate) region_index: usize,
    pub(crate) owner: RoadSurfaceBandKind,
    pub(crate) owner_index: usize,
    pub(crate) height_field_id: NodeBandHeightFieldId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NodeExplicitStepSegmentDiagnostic {
    pub(crate) segment_index: usize,
    pub(crate) start_x_key: i64,
    pub(crate) start_z_key: i64,
    pub(crate) end_x_key: i64,
    pub(crate) end_z_key: i64,
    pub(crate) start_x_mm: i64,
    pub(crate) start_z_mm: i64,
    pub(crate) end_x_mm: i64,
    pub(crate) end_z_mm: i64,
    pub(crate) owner: RoadSurfaceBandKind,
    pub(crate) owner_index: usize,
    pub(crate) opposite_owner: RoadSurfaceBandKind,
    pub(crate) opposite_owner_index: usize,
    pub(crate) owners_match_regions: bool,
    pub(crate) edge_lies_on_segment: bool,
}

impl NodeCanonicalPointDiagnostic {
    pub(crate) fn from_key(point: (i64, i64)) -> Self {
        Self {
            x_key: point.0,
            z_key: point.1,
            x_mm: SurfaceXzKey::coordinate_key_to_mm(point.0),
            z_mm: SurfaceXzKey::coordinate_key_to_mm(point.1),
        }
    }
}

impl NodeValidationReport {
    pub(crate) fn debug_dump(&self) -> String {
        let mut dump = String::new();
        let _ = write!(
            dump,
            "{{\"node_id\":{},\"piece_kind\":\"{:?}\",\"region_count\":{},\"triangle_count\":{},\"exposed_edge_count\":{},\"diagnostics\":[",
            self.node_id,
            self.piece_kind,
            self.region_count,
            self.triangle_count,
            self.exposed_edge_count
        );
        for (index, diagnostic) in self.diagnostics.iter().enumerate() {
            if index > 0 {
                let _ = write!(dump, ",");
            }
            let _ = write!(dump, "{}", diagnostic.debug_record());
        }
        let _ = write!(dump, "]}}");
        dump
    }

    pub(crate) fn has_blocking_diagnostics(&self) -> bool {
        self.diagnostics.iter().any(|diagnostic| {
            // Parry crossing checks are diagnostic only once Spade accepted the constraints and
            // the overlay coverage checks passed. Missing coverage and ownership failures still
            // block export.
            !matches!(
                diagnostic.kind,
                NodeGeometryDiagnosticKind::InvalidConstraint {
                    reason: NodeInvalidConstraintReason::Crossing,
                    ..
                }
            )
        })
    }

    pub(super) fn single_diagnostic(diagnostic: NodeGeometryDiagnostic) -> Self {
        Self {
            node_id: diagnostic.node_id,
            piece_kind: diagnostic.piece_kind,
            region_count: 0,
            triangle_count: 0,
            exposed_edge_count: 0,
            diagnostics: vec![diagnostic],
        }
    }
}

impl NodeGeometryDiagnostic {
    pub(super) fn debug_record(&self) -> String {
        let mut record = json!({
            "node_id": self.node_id,
            "piece_kind": format!("{:?}", self.piece_kind),
            "stage": self.stage.as_str(),
            "backend": self.backend.as_str(),
            "kind": self.kind.as_str(),
        });
        if let (Some(record), Some(detail)) =
            (record.as_object_mut(), self.kind.detail_value().as_object())
        {
            record.extend(
                detail
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            );
        }
        serde_json::to_string(&record).unwrap_or_else(|_| {
            format!(
                "{{\"node_id\":{},\"piece_kind\":\"{:?}\",\"stage\":\"{}\",\"backend\":\"{}\",\"kind\":\"{}\"}}",
                self.node_id,
                self.piece_kind,
                self.stage.as_str(),
                self.backend.as_str(),
                self.kind.as_str()
            )
        })
    }
}

impl NodeGeometryStage {
    fn as_str(self) -> &'static str {
        match self {
            Self::ContourGeneration => "contour_generation",
            Self::BooleanOwnership => "boolean_ownership",
            Self::NodeGrade => "node_grade",
            Self::HeightEvaluation => "height_evaluation",
            Self::Validation => "validation",
            Self::CdtTriangulation => "cdt_triangulation",
        }
    }
}

impl NodeGeometryBackend {
    fn as_str(self) -> &'static str {
        match self {
            Self::CavalierContours => "cavalier_contours",
            Self::IOverlay => "i_overlay",
            Self::HeightCarrier => "height_carrier",
            Self::CanonicalKeys => "canonical_keys",
            Self::Parry2d => "parry2d",
            Self::Spade => "spade",
        }
    }
}

impl NodeGeometryDiagnosticKind {
    fn detail_value(&self) -> Value {
        match self {
            Self::RejectedResidual {
                residual,
                shape_count,
                area_m2,
            } => json!({
                "residual": residual.detail_value(),
                "shape_count": shape_count,
                "area_m2": area_m2,
            }),
            Self::NonExplicitBoundaryVertex {
                region_index,
                owner,
                owner_index,
                height_field_id,
                x_key,
                z_key,
                x_mm,
                z_mm,
                min_boundary_distance_mm,
            } => json!({
                "region_index": region_index,
                "owner": band_owner_parts(*owner, *owner_index),
                "height_field_id": height_field_id_value(*height_field_id),
                "x_key": x_key,
                "z_key": z_key,
                "x_mm": x_mm,
                "z_mm": z_mm,
                "min_boundary_distance_mm": min_boundary_distance_mm,
            }),
            Self::HeightConflict {
                x_mm,
                z_mm,
                existing_height_mm,
                incoming_height_mm,
            } => json!({
                "x_mm": x_mm,
                "z_mm": z_mm,
                "existing_height_mm": existing_height_mm,
                "incoming_height_mm": incoming_height_mm,
            }),
            Self::SourceHeightFieldConflict {
                mouth_order_index,
                band_index,
                source_kind,
                height_field_id,
                owner,
                existing_authority,
                incoming_authority,
                x_mm,
                z_mm,
                existing_height_mm,
                incoming_height_mm,
            } => json!({
                "mouth_order_index": mouth_order_index,
                "band_index": band_index,
                "source_kind": band_kind_value(*source_kind),
                "height_field_id": height_field_id_value(*height_field_id),
                "owner": optional_owner_value(*owner),
                "existing_authority": authority_value(*existing_authority),
                "incoming_authority": authority_value(*incoming_authority),
                "x_mm": x_mm,
                "z_mm": z_mm,
                "existing_height_mm": existing_height_mm,
                "incoming_height_mm": incoming_height_mm,
            }),
            Self::SharedSourceHeightConflict {
                x_mm,
                z_mm,
                kind,
                owner,
                opposite_owner,
                height_field_id,
                incoming_owner,
                incoming_height_field_id,
                constraint_index,
                existing_authority,
                incoming_authority,
                existing_height_mm,
                incoming_height_mm,
            } => json!({
                "x_mm": x_mm,
                "z_mm": z_mm,
                "surface_kind": band_kind_value(*kind),
                "owner": owner_value(*owner),
                "opposite_owner": optional_owner_value(*opposite_owner),
                "height_field_id": optional_height_field_id_value(*height_field_id),
                "incoming_owner": owner_value(*incoming_owner),
                "incoming_height_field_id": optional_height_field_id_value(*incoming_height_field_id),
                "constraint_index": constraint_index,
                "existing_authority": optional_authority_value(*existing_authority),
                "incoming_authority": optional_authority_value(*incoming_authority),
                "existing_height_mm": existing_height_mm,
                "incoming_height_mm": incoming_height_mm,
            }),
            Self::CrossRegionHeightConflict {
                edge_start_x_key,
                edge_start_z_key,
                edge_end_x_key,
                edge_end_z_key,
                edge_start_x_mm,
                edge_start_z_mm,
                edge_end_x_mm,
                edge_end_z_mm,
                conflict_x_key,
                conflict_z_key,
                conflict_x_mm,
                conflict_z_mm,
                existing_region_index,
                existing_owner,
                existing_owner_index,
                existing_start_height_mm,
                existing_end_height_mm,
                existing_conflict_height_mm,
                incoming_region_index,
                incoming_owner,
                incoming_owner_index,
                incoming_start_height_mm,
                incoming_end_height_mm,
                incoming_conflict_height_mm,
                matching_explicit_step_segments,
                non_matching_explicit_step_segments,
            } => json!({
                "edge_start_x_key": edge_start_x_key,
                "edge_start_z_key": edge_start_z_key,
                "edge_end_x_key": edge_end_x_key,
                "edge_end_z_key": edge_end_z_key,
                "edge_start_x_mm": edge_start_x_mm,
                "edge_start_z_mm": edge_start_z_mm,
                "edge_end_x_mm": edge_end_x_mm,
                "edge_end_z_mm": edge_end_z_mm,
                "conflict_x_key": conflict_x_key,
                "conflict_z_key": conflict_z_key,
                "conflict_x_mm": conflict_x_mm,
                "conflict_z_mm": conflict_z_mm,
                "existing_region_index": existing_region_index,
                "existing_owner": band_owner_parts(*existing_owner, *existing_owner_index),
                "existing_start_height_mm": existing_start_height_mm,
                "existing_end_height_mm": existing_end_height_mm,
                "existing_conflict_height_mm": existing_conflict_height_mm,
                "incoming_region_index": incoming_region_index,
                "incoming_owner": band_owner_parts(*incoming_owner, *incoming_owner_index),
                "incoming_start_height_mm": incoming_start_height_mm,
                "incoming_end_height_mm": incoming_end_height_mm,
                "incoming_conflict_height_mm": incoming_conflict_height_mm,
                "matching_explicit_step_segments": step_segments_value(matching_explicit_step_segments),
                "non_matching_explicit_step_segments": step_segments_value(non_matching_explicit_step_segments),
            }),
            Self::HeightFieldFailure {
                reason,
                mouth_order_index,
                band_index,
                kind,
                source_kind,
                height_field_id,
                owner,
                point_x_key,
                point_z_key,
                point_x_mm,
                point_z_mm,
                axis,
                raw_parameter,
            } => json!({
                "reason": reason,
                "mouth_order_index": mouth_order_index,
                "band_index": band_index,
                "region_kind": optional_band_kind_value(*kind),
                "source_kind": optional_band_kind_value(*source_kind),
                "height_field_id": optional_height_field_id_value(*height_field_id),
                "owner": optional_owner_value(*owner),
                "point_x_key": point_x_key,
                "point_z_key": point_z_key,
                "point_x_mm": point_x_mm,
                "point_z_mm": point_z_mm,
                "axis": axis,
                "raw_parameter": raw_parameter,
            }),
            Self::MissingGradeAuthority {
                region_index,
                contour_index,
                x_mm,
                z_mm,
                owner,
                owner_index,
                height_field_id,
                height_mm,
            } => json!({
                "region_index": region_index,
                "contour_index": contour_index,
                "x_mm": x_mm,
                "z_mm": z_mm,
                "owner": band_owner_parts(*owner, *owner_index),
                "height_field_id": height_field_id_value(*height_field_id),
                "height_mm": height_mm,
            }),
            Self::OpenBoundary {
                region_index,
                owner,
                owner_index,
                height_field_id,
                vertex_index,
                x_key,
                z_key,
                x_mm,
                z_mm,
                start_x_key,
                start_z_key,
                end_x_key,
                end_z_key,
                start_x_mm,
                start_z_mm,
                end_x_mm,
                end_z_mm,
                degree,
            } => json!({
                "region_index": region_index,
                "owner": band_owner_parts(*owner, *owner_index),
                "height_field_id": height_field_id_value(*height_field_id),
                "vertex_index": vertex_index,
                "x_key": x_key,
                "z_key": z_key,
                "x_mm": x_mm,
                "z_mm": z_mm,
                "start_x_key": start_x_key,
                "start_z_key": start_z_key,
                "end_x_key": end_x_key,
                "end_z_key": end_z_key,
                "start_x_mm": start_x_mm,
                "start_z_mm": start_z_mm,
                "end_x_mm": end_x_mm,
                "end_z_mm": end_z_mm,
                "degree": degree,
            }),
            Self::DuplicateExposedEdge {
                region_index,
                regions,
                start_x_key,
                start_z_key,
                end_x_key,
                end_z_key,
                start_x_mm,
                start_z_mm,
                end_x_mm,
                end_z_mm,
                count,
            } => json!({
                "region_index": region_index,
                "regions": regions
                    .iter()
                    .map(boundary_region_value)
                    .collect::<Vec<_>>(),
                "start_x_key": start_x_key,
                "start_z_key": start_z_key,
                "end_x_key": end_x_key,
                "end_z_key": end_z_key,
                "start_x_mm": start_x_mm,
                "start_z_mm": start_z_mm,
                "end_x_mm": end_x_mm,
                "end_z_mm": end_z_mm,
                "count": count,
            }),
            Self::InvalidConstraint {
                region_index,
                constraint_index,
                reason,
            } => json!({
                "region_index": region_index,
                "constraint_index": constraint_index,
                "reason": format!("{:?}", reason),
            }),
            Self::TriangleCoverageMismatch {
                region_index,
                missing_area_m2,
                extra_area_m2,
            } => json!({
                "region_index": region_index,
                "missing_area_m2": missing_area_m2,
                "extra_area_m2": extra_area_m2,
            }),
            Self::TriangleOverlap {
                region_index,
                overlap_area_m2,
            } => json!({
                "region_index": region_index,
                "overlap_area_m2": overlap_area_m2,
            }),
            Self::SeamConstraintFailure {
                region_index,
                owner,
                owner_index,
                opposite_owner,
                opposite_owner_index,
                start_x_mm,
                start_z_mm,
                end_x_mm,
                end_z_mm,
                reason,
            } => json!({
                "region_index": region_index,
                "owner": band_owner_parts(*owner, *owner_index),
                "opposite_owner": band_owner_parts(*opposite_owner, *opposite_owner_index),
                "start_x_mm": start_x_mm,
                "start_z_mm": start_z_mm,
                "end_x_mm": end_x_mm,
                "end_z_mm": end_z_mm,
                "reason": format!("{:?}", reason),
            }),
            Self::AmbiguousOwnedBoundaryEdge {
                region_index,
                owner,
                owner_index,
                opposite_owners,
                start_x_mm,
                start_z_mm,
                end_x_mm,
                end_z_mm,
            } => json!({
                "region_index": region_index,
                "owner": band_owner_parts(*owner, *owner_index),
                "opposite_owners": opposite_owners
                    .iter()
                    .map(|(kind, owner_index)| band_owner_parts(*kind, *owner_index))
                    .collect::<Vec<_>>(),
                "start_x_mm": start_x_mm,
                "start_z_mm": start_z_mm,
                "end_x_mm": end_x_mm,
                "end_z_mm": end_z_mm,
            }),
            Self::UnmaterializedRaisedStepAuthority {
                region_index,
                owner,
                owner_index,
                opposite_owner,
                opposite_owner_index,
                start_x_mm,
                start_z_mm,
                end_x_mm,
                end_z_mm,
                source_constraint_indices,
            } => json!({
                "region_index": region_index,
                "owner": band_owner_parts(*owner, *owner_index),
                "opposite_owner": band_owner_parts(*opposite_owner, *opposite_owner_index),
                "start_x_mm": start_x_mm,
                "start_z_mm": start_z_mm,
                "end_x_mm": end_x_mm,
                "end_z_mm": end_z_mm,
                "source_constraint_indices": source_constraint_indices,
            }),
            Self::NonCanonicalOwnedRegionVertex {
                owner,
                point_x_key,
                point_z_key,
                point_x_mm,
                point_z_mm,
                canonical_x_key,
                canonical_z_key,
                canonical_x_mm,
                canonical_z_mm,
            } => json!({
                "owner": owner_value(*owner),
                "point_x_key": point_x_key,
                "point_z_key": point_z_key,
                "point_x_mm": point_x_mm,
                "point_z_mm": point_z_mm,
                "canonical_x_key": canonical_x_key,
                "canonical_z_key": canonical_z_key,
                "canonical_x_mm": canonical_x_mm,
                "canonical_z_mm": canonical_z_mm,
            }),
            Self::AmbiguousCanonicalOwnedRegionVertex {
                owner,
                point_x_key,
                point_z_key,
                point_x_mm,
                point_z_mm,
                candidates,
            } => json!({
                "owner": owner_value(*owner),
                "point_x_key": point_x_key,
                "point_z_key": point_z_key,
                "point_x_mm": point_x_mm,
                "point_z_mm": point_z_mm,
                "candidates": candidates
                    .iter()
                    .map(canonical_point_value)
                    .collect::<Vec<_>>(),
            }),
            Self::AmbiguousSourceSegmentAuthorizedOwnedRegionVertex {
                owner,
                point_x_key,
                point_z_key,
                point_x_mm,
                point_z_mm,
                source_kind,
                source_mouth_order_index,
                source_band_index,
                candidates,
            } => json!({
                "owner": owner_value(*owner),
                "point_x_key": point_x_key,
                "point_z_key": point_z_key,
                "point_x_mm": point_x_mm,
                "point_z_mm": point_z_mm,
                "source": {
                    "kind": band_kind_value(*source_kind),
                    "mouth_order_index": source_mouth_order_index,
                    "band_index": source_band_index,
                },
                "candidates": candidates
                    .iter()
                    .map(source_segment_authorization_candidate_value)
                    .collect::<Vec<_>>(),
            }),
            Self::BackendFailure { reason } => json!({
                "reason": reason,
            }),
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            Self::RejectedResidual { .. } => "rejected_residual",
            Self::NonExplicitBoundaryVertex { .. } => "non_explicit_boundary_vertex",
            Self::HeightConflict { .. } | Self::CrossRegionHeightConflict { .. } => {
                "height_conflict"
            }
            Self::SourceHeightFieldConflict { .. } => "source_height_field_conflict",
            Self::SharedSourceHeightConflict { .. } => "shared_source_height_conflict",
            Self::HeightFieldFailure { .. } => "height_field_failure",
            Self::MissingGradeAuthority { .. } => "missing_grade_authority",
            Self::OpenBoundary { .. } => "open_boundary",
            Self::DuplicateExposedEdge { .. } => "duplicate_exposed_edge",
            Self::InvalidConstraint { .. } => "invalid_constraint",
            Self::TriangleCoverageMismatch { .. } => "triangle_coverage_mismatch",
            Self::TriangleOverlap { .. } => "triangle_overlap",
            Self::SeamConstraintFailure { .. } => "seam_constraint_failure",
            Self::AmbiguousOwnedBoundaryEdge { .. } => "ambiguous_owned_boundary_edge",
            Self::UnmaterializedRaisedStepAuthority { .. } => {
                "unmaterialized_raised_step_authority"
            }
            Self::NonCanonicalOwnedRegionVertex { .. } => "noncanonical_owned_region_vertex",
            Self::AmbiguousCanonicalOwnedRegionVertex { .. } => {
                "ambiguous_canonical_owned_region_vertex"
            }
            Self::AmbiguousSourceSegmentAuthorizedOwnedRegionVertex { .. } => {
                "ambiguous_source_segment_authorization"
            }
            Self::BackendFailure { .. } => "backend_failure",
        }
    }
}

impl NodeRejectedResidualKind {
    fn detail_value(&self) -> Value {
        match self {
            Self::Asphalt => json!({"type": "asphalt"}),
            Self::Band(kind) => json!({
                "type": "band",
                "kind": band_kind_value(*kind),
            }),
            Self::NonRoad => json!({"type": "non_road"}),
        }
    }
}

fn band_kind_value(kind: RoadSurfaceBandKind) -> Value {
    json!(format!("{:?}", kind))
}

fn optional_band_kind_value(kind: Option<RoadSurfaceBandKind>) -> Value {
    kind.map(band_kind_value).unwrap_or(Value::Null)
}

fn owner_value(owner: NodeBandOwner) -> Value {
    band_owner_parts(owner.kind(), owner.owner_index())
}

fn optional_owner_value(owner: Option<NodeBandOwner>) -> Value {
    owner.map(owner_value).unwrap_or(Value::Null)
}

fn band_owner_parts(kind: RoadSurfaceBandKind, owner_index: usize) -> Value {
    json!({
        "kind": band_kind_value(kind),
        "owner_index": owner_index,
    })
}

fn height_field_id_value(height_field_id: NodeBandHeightFieldId) -> Value {
    json!({
        "mouth_order_index": height_field_id.mouth_order_index(),
        "band_index": height_field_id.band_index(),
        "debug": format!("{:?}", height_field_id),
    })
}

fn optional_height_field_id_value(height_field_id: Option<NodeBandHeightFieldId>) -> Value {
    height_field_id
        .map(height_field_id_value)
        .unwrap_or(Value::Null)
}

fn authority_value(authority: NodeHeightAuthoritySource) -> Value {
    json!(format!("{:?}", authority))
}

fn optional_authority_value(authority: Option<NodeHeightAuthoritySource>) -> Value {
    authority.map(authority_value).unwrap_or(Value::Null)
}

fn canonical_point_value(point: &NodeCanonicalPointDiagnostic) -> Value {
    json!({
        "x_key": point.x_key,
        "z_key": point.z_key,
        "x_mm": point.x_mm,
        "z_mm": point.z_mm,
    })
}

fn boundary_region_value(region: &NodeBoundaryRegionDiagnostic) -> Value {
    json!({
        "region_index": region.region_index,
        "owner": band_owner_parts(region.owner, region.owner_index),
        "height_field_id": height_field_id_value(region.height_field_id),
    })
}

fn source_segment_authorization_candidate_value(
    candidate: &NodeSourceSegmentAuthorizationCandidate,
) -> Value {
    json!({
        "source": {
            "kind": band_kind_value(candidate.source_kind),
            "mouth_order_index": candidate.source_mouth_order_index,
            "band_index": candidate.source_band_index,
        },
        "canonical": canonical_point_value(&NodeCanonicalPointDiagnostic::from_key(
            candidate.canonical_point,
        )),
        "segment_start": canonical_point_value(&NodeCanonicalPointDiagnostic::from_key(
            candidate.segment_start,
        )),
        "segment_end": canonical_point_value(&NodeCanonicalPointDiagnostic::from_key(
            candidate.segment_end,
        )),
        "distance_key_units_sq": candidate.distance_key_units_sq,
        "dust_budget_key_units": candidate.dust_budget_key_units,
    })
}

fn step_segments_value(segments: &[NodeExplicitStepSegmentDiagnostic]) -> Value {
    json!(
        segments
            .iter()
            .map(|segment| {
                json!({
                    "segment_index": segment.segment_index,
                    "start_x_key": segment.start_x_key,
                    "start_z_key": segment.start_z_key,
                    "end_x_key": segment.end_x_key,
                    "end_z_key": segment.end_z_key,
                    "start_x_mm": segment.start_x_mm,
                    "start_z_mm": segment.start_z_mm,
                    "end_x_mm": segment.end_x_mm,
                    "end_z_mm": segment.end_z_mm,
                    "owner": band_owner_parts(segment.owner, segment.owner_index),
                    "opposite_owner": band_owner_parts(
                        segment.opposite_owner,
                        segment.opposite_owner_index,
                    ),
                    "owners_match_regions": segment.owners_match_regions,
                    "edge_lies_on_segment": segment.edge_lies_on_segment,
                })
            })
            .collect::<Vec<_>>()
    )
}

pub(super) fn push_validation_diagnostic(
    solution: &NodeTriangulationSolution,
    diagnostics: &mut Vec<NodeGeometryDiagnostic>,
    backend: NodeGeometryBackend,
    kind: NodeGeometryDiagnosticKind,
) {
    diagnostics.push(NodeGeometryDiagnostic {
        node_id: solution.node_id,
        piece_kind: solution.piece_kind,
        stage: NodeGeometryStage::Validation,
        backend,
        kind,
    });
}
