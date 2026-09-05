// SPDX-License-Identifier: GPL-2.0-only

//! Validation logging for canonical node compilation.

use super::*;
use crate::simulation::network::surface::keys::SurfaceXzKey;
use crate::simulation::network::surface::node::validation::NodeGeometryDiagnosticKind;

impl RoadSurfaceSystem {
    pub(super) fn log_node_input_extraction_error(
        &self,
        node_id: u32,
        kind: RoadSurfaceVisualNodePieceKind,
        error: &NodeInputExtractionError,
    ) {
        if !self.node_validation_logging_enabled {
            return;
        }
        crate::debug_log!(
            "road",
            "node_canonical_input_failed node={} piece={:?} error={:?}",
            node_id,
            kind,
            error
        );
    }

    pub(super) fn log_node_validation_report(&self, report: &NodeValidationReport) {
        if !self.node_validation_logging_enabled || report.diagnostics.is_empty() {
            return;
        }
        crate::debug_log!("road", "node_canonical_validation {}", report.debug_dump());
    }

    pub(super) fn log_node_boundary_export_error(
        &self,
        arrangement: &NodeArrangement,
        error: &NodeBoundaryExportError,
    ) {
        if !self.node_validation_logging_enabled {
            return;
        }
        let report = Self::node_boundary_export_report(arrangement, error);
        self.log_node_validation_report(&report);
    }

    pub(in crate::simulation::network::surface) fn node_boundary_export_report(
        arrangement: &NodeArrangement,
        error: &NodeBoundaryExportError,
    ) -> NodeValidationReport {
        match error {
            NodeBoundaryExportError::MissingFootprintBoundaryHeight { x_key, z_key } => {
                let _ = (*x_key, *z_key);
                NodeValidationReport::from_boundary_export_error(
                    arrangement.node_id(),
                    arrangement.piece_kind(),
                    "missing_footprint_boundary_height",
                )
            }
            NodeBoundaryExportError::ConflictingFootprintBoundaryHeight {
                x_key,
                z_key,
                existing_y_mm,
                incoming_y_mm,
                existing_owner_kind,
                existing_owner_index,
                existing_source,
                incoming_owner_kind,
                incoming_owner_index,
                incoming_source,
            } => NodeValidationReport::from_boundary_export_diagnostic(
                arrangement.node_id(),
                arrangement.piece_kind(),
                NodeGeometryDiagnosticKind::FootprintBoundaryHeightConflict {
                    x_key: *x_key,
                    z_key: *z_key,
                    x_mm: SurfaceXzKey::coordinate_key_to_mm(*x_key),
                    z_mm: SurfaceXzKey::coordinate_key_to_mm(*z_key),
                    existing_y_mm: *existing_y_mm,
                    incoming_y_mm: *incoming_y_mm,
                    existing_owner_kind: *existing_owner_kind,
                    existing_owner_index: *existing_owner_index,
                    existing_source: *existing_source,
                    incoming_owner_kind: *incoming_owner_kind,
                    incoming_owner_index: *incoming_owner_index,
                    incoming_source: *incoming_source,
                },
            ),
            NodeBoundaryExportError::ConflictingFootprintBoundarySplitHeight {
                x_key,
                z_key,
                existing_y_mm,
                incoming_y_mm,
            } => {
                let _ = (*x_key, *z_key, *existing_y_mm, *incoming_y_mm);
                NodeValidationReport::from_boundary_export_error(
                    arrangement.node_id(),
                    arrangement.piece_kind(),
                    "conflicting_footprint_boundary_split_height",
                )
            }
            NodeBoundaryExportError::AmbiguousEarthworkBoundarySegmentSource {
                start_x_key,
                start_z_key,
                start_y_mm,
                end_x_key,
                end_z_key,
                end_y_mm,
                existing_height_field_id,
                incoming_height_field_id,
                existing_source,
                incoming_source,
            } => {
                let _ = (
                    *start_x_key,
                    *start_z_key,
                    *start_y_mm,
                    *end_x_key,
                    *end_z_key,
                    *end_y_mm,
                    *existing_height_field_id,
                    *incoming_height_field_id,
                    *existing_source,
                    *incoming_source,
                );
                NodeValidationReport::from_boundary_export_error(
                    arrangement.node_id(),
                    arrangement.piece_kind(),
                    "ambiguous_earthwork_boundary_segment_source",
                )
            }
            NodeBoundaryExportError::AmbiguousFootprintBoundaryPointSource {
                x_key,
                z_key,
                y_mm,
                existing_owner_kind,
                existing_owner_index,
                existing_source,
                incoming_owner_kind,
                incoming_owner_index,
                incoming_source,
            } => {
                let _ = (
                    *x_key,
                    *z_key,
                    *y_mm,
                    *existing_owner_kind,
                    *existing_owner_index,
                    *existing_source,
                    *incoming_owner_kind,
                    *incoming_owner_index,
                    *incoming_source,
                );
                NodeValidationReport::from_boundary_export_error(
                    arrangement.node_id(),
                    arrangement.piece_kind(),
                    "ambiguous_footprint_boundary_point_source",
                )
            }
            NodeBoundaryExportError::EmptyOuterBoundary => {
                NodeValidationReport::from_boundary_export_error(
                    arrangement.node_id(),
                    arrangement.piece_kind(),
                    "empty_outer_boundary",
                )
            }
            NodeBoundaryExportError::DegenerateOuterBoundaryLoop => {
                NodeValidationReport::from_boundary_export_error(
                    arrangement.node_id(),
                    arrangement.piece_kind(),
                    "degenerate_outer_boundary_loop",
                )
            }
            NodeBoundaryExportError::MissingEarthworkBoundarySource => {
                NodeValidationReport::from_boundary_export_error(
                    arrangement.node_id(),
                    arrangement.piece_kind(),
                    "missing_earthwork_boundary_source",
                )
            }
            NodeBoundaryExportError::MissingEarthworkBoundarySegmentSource {
                start_x_key,
                start_z_key,
                end_x_key,
                end_z_key,
                nearby_source_edges,
            } => {
                let _ = (
                    *start_x_key,
                    *start_z_key,
                    *end_x_key,
                    *end_z_key,
                    nearby_source_edges,
                );
                NodeValidationReport::from_boundary_export_error(
                    arrangement.node_id(),
                    arrangement.piece_kind(),
                    "missing_earthwork_boundary_segment_source",
                )
            }
            NodeBoundaryExportError::MissingNodeTopSurfaceGradeAuthority => {
                NodeValidationReport::from_boundary_export_error(
                    arrangement.node_id(),
                    arrangement.piece_kind(),
                    "missing_node_top_surface_grade_authority",
                )
            }
        }
    }
}
