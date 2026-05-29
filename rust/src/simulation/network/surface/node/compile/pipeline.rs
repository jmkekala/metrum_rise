//! Canonical node surface compile pipeline.

use super::*;
use std::time::Instant;

fn elapsed_ms(start: Option<Instant>) -> f64 {
    start
        .map(|start| start.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

impl RoadSurfaceSystem {
    pub(super) fn compile_canonical_node_surface_regions(
        &self,
        node_id: u32,
        kind: RoadSurfaceVisualNodePieceKind,
        mouths: &[OrderedIncidentPieceMouth],
    ) -> Option<super::super::NodeSurfaceRegionResult> {
        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);

        let input_start = road_debug.then(Instant::now);
        let input = match Self::build_node_arrangement_input_from_mouths(node_id, kind, mouths) {
            Ok(input) => input,
            Err(error) => {
                self.log_node_input_extraction_error(node_id, kind, &error);
                return None;
            }
        };
        let input_ms = elapsed_ms(input_start);

        let rails_start = road_debug.then(Instant::now);
        let (rails, rail_profile) =
            match Self::build_node_rail_contours_from_input_with_profile(&input, road_debug) {
                Ok((rails, profile)) => (rails, profile),
                Err(error) => {
                    self.log_node_validation_report(
                        &NodeValidationReport::from_rail_generation_error(node_id, kind, &error),
                    );
                    return None;
                }
            };
        let rails_ms = elapsed_ms(rails_start);

        let ownership_start = road_debug.then(Instant::now);
        let ownership = match Self::build_node_boolean_ownership_from_rails(&rails) {
            Ok(ownership) => ownership,
            Err(error) => {
                self.log_node_validation_report(
                    &NodeValidationReport::from_boolean_ownership_error(node_id, kind, &error),
                );
                return None;
            }
        };
        let ownership_ms = elapsed_ms(ownership_start);

        let ownership_diag_start = road_debug.then(Instant::now);
        if let Some(report) = NodeValidationReport::from_owned_region_arrangement_diagnostics(
            &ownership.owned_region_arrangement,
        ) {
            self.log_node_validation_report(&report);
            return None;
        }
        let ownership_diag_ms = elapsed_ms(ownership_diag_start);

        let heights_start = road_debug.then(Instant::now);
        let heights =
            match Self::build_node_height_solution_from_ownership(&input, &rails, &ownership) {
                Ok(heights) => heights,
                Err(error) => {
                    self.log_node_validation_report(
                        &NodeValidationReport::from_height_field_error(node_id, kind, &error),
                    );
                    return None;
                }
            };
        let heights_ms = elapsed_ms(heights_start);

        let arrangement_start = road_debug.then(Instant::now);
        let (mut arrangement, arrangement_profile) =
            match NodeArrangement::from_height_solution_with_profile(&heights, road_debug) {
                Ok((arrangement, profile)) => (arrangement, profile),
                Err(error) => {
                    self.log_node_validation_report(&NodeValidationReport::from_arrangement_error(
                        node_id, kind, &error,
                    ));
                    return None;
                }
            };
        let arrangement_ms = elapsed_ms(arrangement_start);

        let arrangement_diag_start = road_debug.then(Instant::now);
        if let Some(report) = NodeValidationReport::from_arrangement_diagnostics(&arrangement) {
            self.log_node_validation_report(&report);
            return None;
        }
        let arrangement_diag_ms = elapsed_ms(arrangement_diag_start);

        let triangulation_start = road_debug.then(Instant::now);
        let triangulation = match Self::build_node_triangulation_from_arrangement(&arrangement) {
            Ok(triangulation) => triangulation,
            Err(error) => {
                self.log_node_validation_report(&NodeValidationReport::from_triangulation_error(
                    node_id, kind, &error,
                ));
                return None;
            }
        };
        let triangulation_ms = elapsed_ms(triangulation_start);

        let triangulation_validation_start = road_debug.then(Instant::now);
        match Self::validate_node_triangulation_solution(&triangulation) {
            Ok(report) => self.log_node_validation_report(&report),
            Err(error) => {
                self.log_node_validation_report(&error.report);
                if error.report.has_blocking_diagnostics() {
                    return None;
                }
            }
        }
        let triangulation_validation_ms = elapsed_ms(triangulation_validation_start);

        let attach_start = road_debug.then(Instant::now);
        let attach_profile =
            match arrangement.attach_triangulation_with_profile(&triangulation, road_debug) {
                Ok(profile) => profile,
                Err(error) => {
                    self.log_node_validation_report(&NodeValidationReport::from_arrangement_error(
                        node_id, kind, &error,
                    ));
                    return None;
                }
            };
        let attach_ms = elapsed_ms(attach_start);

        let export_start = road_debug.then(Instant::now);
        match Self::node_surface_regions_from_arrangement(&arrangement, &ownership.footprint_shapes)
        {
            Ok(regions) => {
                let export_ms = elapsed_ms(export_start);
                if road_debug {
                    let total_ms = elapsed_ms(total_start);
                    if total_ms >= 50.0 {
                        crate::debug_log!(
                            "road",
                            "node_compile_pipeline_detail node={} kind={:?} mouths={} arrangement_vertices={} arrangement_regions={} road_polygons={} curb_polygons={} sidewalk_polygons={} raised_step_faces={} input_ms={:.3} rails_ms={:.3} ownership_ms={:.3} ownership_diag_ms={:.3} heights_ms={:.3} arrangement_ms={:.3} arrangement_diag_ms={:.3} triangulation_ms={:.3} triangulation_validation_ms={:.3} attach_ms={:.3} export_ms={:.3} total_ms={:.3}",
                            node_id,
                            kind,
                            mouths.len(),
                            arrangement.vertices().len(),
                            arrangement.regions().len(),
                            regions.road_surface_polygons.len(),
                            regions.curb_surface_polygons.len(),
                            regions.sidewalk_surface_polygons.len(),
                            regions.raised_step_faces.len(),
                            input_ms,
                            rails_ms,
                            ownership_ms,
                            ownership_diag_ms,
                            heights_ms,
                            arrangement_ms,
                            arrangement_diag_ms,
                            triangulation_ms,
                            triangulation_validation_ms,
                            attach_ms,
                            export_ms,
                            total_ms
                        );
                        crate::debug_log!(
                            "road",
                            "node_rail_build_detail node={} kind={:?} mouths={} contours={} constraints={} source_constraints={} validation_constraints={} height_carrier_sources={} height_carrier_points={} contact_pair_tests={} contact_pair_aabb_rejected={} contact_pair_kind_rejected={} contact_pair_processed={} contact_overlay_calls={} contact_constraints_emitted={} terminal_caps_ms={:.3} side_joins_ms={:.3} owners_ms={:.3} mouth_base_contours_ms={:.3} mouth_band_contours_ms={:.3} cap_height_carriers_ms={:.3} terminal_cap_contours_ms={:.3} side_join_contours_ms={:.3} boundary_constraints_ms={:.3} span_handoff_ms={:.3} contact_noding_first_ms={:.3} raised_step_contacts_first_ms={:.3} material_contacts_ms={:.3} raised_step_contacts_second_ms={:.3} contact_noding_second_ms={:.3} same_band_contacts_ms={:.3} contact_noding_third_ms={:.3} validation_source_constraints_ms={:.3} retain_constraints_ms={:.3} validate_endpoints_ms={:.3} source_carriers_ms={:.3} total_ms={:.3}",
                            node_id,
                            kind,
                            rail_profile.mouths,
                            rail_profile.contours,
                            rail_profile.constraints,
                            rail_profile.source_constraints,
                            rail_profile.validation_constraints,
                            rail_profile.height_carrier_sources,
                            rail_profile.height_carrier_points,
                            rail_profile.contact_pair_tests,
                            rail_profile.contact_pair_aabb_rejected,
                            rail_profile.contact_pair_kind_rejected,
                            rail_profile.contact_pair_processed,
                            rail_profile.contact_overlay_calls,
                            rail_profile.contact_constraints_emitted,
                            rail_profile.terminal_caps_ms,
                            rail_profile.side_joins_ms,
                            rail_profile.owners_ms,
                            rail_profile.mouth_base_contours_ms,
                            rail_profile.mouth_band_contours_ms,
                            rail_profile.cap_height_carriers_ms,
                            rail_profile.terminal_cap_contours_ms,
                            rail_profile.side_join_contours_ms,
                            rail_profile.boundary_constraints_ms,
                            rail_profile.span_handoff_ms,
                            rail_profile.contact_noding_first_ms,
                            rail_profile.raised_step_contacts_first_ms,
                            rail_profile.material_contacts_ms,
                            rail_profile.raised_step_contacts_second_ms,
                            rail_profile.contact_noding_second_ms,
                            rail_profile.same_band_contacts_ms,
                            rail_profile.contact_noding_third_ms,
                            rail_profile.validation_source_constraints_ms,
                            rail_profile.retain_constraints_ms,
                            rail_profile.validate_endpoints_ms,
                            rail_profile.source_carriers_ms,
                            rail_profile.total_ms
                        );
                        crate::debug_log!(
                            "road",
                            "node_arrangement_build_detail node={} kind={:?} height_regions={} pending_edges_before={} pending_edges_after={} vertices={} edges={} regions={} seam_constraints={} diagnostics={} pending_regions_ms={:.3} noding_ms={:.3} edge_support_ms={:.3} boundary_edges_ms={:.3} push_regions_ms={:.3} conflict_ms={:.3} total_ms={:.3}",
                            node_id,
                            kind,
                            arrangement_profile.height_regions,
                            arrangement_profile.pending_edges_before,
                            arrangement_profile.pending_edges_after,
                            arrangement_profile.vertices,
                            arrangement_profile.edges,
                            arrangement_profile.regions,
                            arrangement_profile.seam_constraints,
                            arrangement_profile.diagnostics,
                            arrangement_profile.pending_regions_ms,
                            arrangement_profile.noding_ms,
                            arrangement_profile.edge_support_ms,
                            arrangement_profile.boundary_edges_ms,
                            arrangement_profile.push_regions_ms,
                            arrangement_profile.conflict_ms,
                            arrangement_profile.total_ms
                        );
                        crate::debug_log!(
                            "road",
                            "node_attach_detail node={} kind={:?} regions={} source_vertices={} source_triangles={} vertex_insert_attempts={} arrangement_vertices_before={} arrangement_vertices_after={} vertices_inserted={} vertices_reused={} faces_pushed={} validation_ms={:.3} insert_vertices_ms={:.3} push_faces_ms={:.3} conflict_ms={:.3} total_ms={:.3}",
                            node_id,
                            kind,
                            attach_profile.regions,
                            attach_profile.source_vertices,
                            attach_profile.source_triangles,
                            attach_profile.vertex_insert_attempts,
                            attach_profile.arrangement_vertices_before,
                            attach_profile.arrangement_vertices_after,
                            attach_profile.vertices_inserted,
                            attach_profile.vertices_reused,
                            attach_profile.faces_pushed,
                            attach_profile.validation_ms,
                            attach_profile.insert_vertices_ms,
                            attach_profile.push_faces_ms,
                            attach_profile.conflict_ms,
                            attach_profile.total_ms
                        );
                    }
                }
                Some(regions)
            }
            Err(error) => {
                self.log_node_boundary_export_error(&arrangement, &error);
                None
            }
        }
    }
}
