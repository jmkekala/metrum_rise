//! Canonical node surface compile pipeline.

use super::*;
use std::time::Instant;

fn elapsed_ms(start: Option<Instant>) -> f64 {
    start
        .map(|start| start.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

fn detailed_road_geometry_debug_enabled() -> bool {
    ["METRUM_DEBUG_ROAD_GEOMETRY_DUMP", "METRUM_DEBUG_ROAD_PROBE"]
        .iter()
        .any(|key| {
            std::env::var(key)
                .map(|value| !value.is_empty() && value != "0")
                .unwrap_or(false)
        })
}

fn node_corner_trims_apply_to_footprint(rails: &rails::NodeRailContourSet) -> bool {
    if rails.corner_trims.is_empty() {
        return false;
    }
    match rails.piece_kind {
        RoadSurfaceVisualNodePieceKind::Bend => true,
        RoadSurfaceVisualNodePieceKind::JunctionN => rails.corner_trims.iter().any(|trim| {
            rails.side_join_gaps.iter().any(|gap| {
                gap.from_mouth_order_index == trim.source_mouth_order_index
                    && gap.role == joins::NodeInputSideJoinGapRole::Exterior
            })
        }),
        RoadSurfaceVisualNodePieceKind::Terminal => false,
    }
}

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn canonical_node_compile_failure_debug_dump(
        &self,
        node_id: u32,
        kind: RoadSurfaceVisualNodePieceKind,
        mouths: &[OrderedIncidentPieceMouth],
    ) -> Option<String> {
        let input = match Self::build_node_arrangement_input_from_mouths(node_id, kind, mouths) {
            Ok(input) => input,
            Err(error) => {
                let error = serde_json::to_string(&format!("{error:?}"))
                    .unwrap_or_else(|_| "\"input_extraction_failed\"".to_string());
                return Some(format!(
                    "{{\"failed_stage\":\"input_extraction\",\"error\":{error}}}"
                ));
            }
        };
        let (rails, _) = match Self::build_node_rail_contours_from_input_with_profile(&input, false)
        {
            Ok(result) => result,
            Err(error) => {
                return Some(Self::node_compile_failure_debug_dump(
                    "rail_generation",
                    NodeValidationReport::from_rail_generation_error(node_id, kind, &error),
                ));
            }
        };
        let ownership = match Self::build_node_boolean_ownership_from_rails(&rails) {
            Ok(ownership) => ownership,
            Err(error) => {
                return Some(Self::node_compile_failure_debug_dump(
                    "boolean_ownership",
                    NodeValidationReport::from_boolean_ownership_error(node_id, kind, &error),
                ));
            }
        };
        if let Some(report) = NodeValidationReport::from_owned_region_arrangement_diagnostics(
            &ownership.owned_region_arrangement,
        ) {
            return Some(Self::node_compile_failure_debug_dump(
                "owned_region_arrangement",
                report,
            ));
        }
        let heights =
            match Self::build_node_height_solution_from_ownership(&input, &rails, &ownership) {
                Ok(heights) => heights,
                Err(error) => {
                    return Some(Self::node_compile_failure_debug_dump(
                        "height_solution",
                        NodeValidationReport::from_height_field_error(node_id, kind, &error)
                            .with_height_failure_context(&rails, &ownership),
                    ));
                }
            };
        let (mut arrangement, _, precomputed_explicit_steps) =
            match NodeArrangement::from_height_solution_with_profile(&heights, false) {
                Ok(result) => result,
                Err(error) => {
                    return Some(Self::node_compile_failure_debug_dump(
                        "arrangement",
                        NodeValidationReport::from_arrangement_error(node_id, kind, &error),
                    ));
                }
            };
        if let Some(report) = NodeValidationReport::from_arrangement_diagnostics(&arrangement) {
            return Some(Self::node_compile_failure_debug_dump(
                "arrangement_diagnostics",
                report,
            ));
        }
        let triangulation = match precomputed_explicit_steps.as_deref() {
            Some(explicit_steps) => {
                Self::build_node_triangulation_from_arrangement_with_explicit_steps(
                    &arrangement,
                    explicit_steps,
                )
            }
            None => Self::build_node_triangulation_from_arrangement(&arrangement),
        };
        let triangulation = match triangulation {
            Ok(triangulation) => triangulation,
            Err(error) => {
                return Some(Self::node_compile_failure_debug_dump(
                    "triangulation",
                    NodeValidationReport::from_triangulation_error(node_id, kind, &error),
                ));
            }
        };
        match Self::validate_node_triangulation_solution(&triangulation) {
            Ok(_) => {}
            Err(error) => {
                if error.report.has_blocking_diagnostics() {
                    return Some(Self::node_compile_failure_debug_dump(
                        "triangulation_validation",
                        error.report,
                    ));
                }
            }
        }
        if let Err(error) = arrangement.attach_triangulation_with_profile(&triangulation, false) {
            return Some(Self::node_compile_failure_debug_dump(
                "attach_triangulation",
                NodeValidationReport::from_arrangement_error(node_id, kind, &error),
            ));
        }
        match Self::node_surface_regions_from_arrangement_with_profile(
            &arrangement,
            &ownership.footprint_shapes,
            false,
        ) {
            Ok(_) => None,
            Err(error) => Some(Self::node_compile_failure_debug_dump(
                "boundary_export",
                Self::node_boundary_export_report(&arrangement, &error),
            )),
        }
    }

    fn node_compile_failure_debug_dump(
        failed_stage: &'static str,
        report: NodeValidationReport,
    ) -> String {
        format!(
            "{{\"failed_stage\":\"{failed_stage}\",\"validation_report\":{}}}",
            report.debug_dump()
        )
    }

    pub(super) fn compile_canonical_node_surface_regions_with_topology_cache(
        &self,
        node_id: u32,
        kind: RoadSurfaceVisualNodePieceKind,
        mouths: &[OrderedIncidentPieceMouth],
        previous_topology: Option<&super::NodeCanonicalTopologyCache>,
    ) -> Option<super::NodeCanonicalSurfaceCompileResult> {
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
        let (rails, rail_profile, reuse_status, rail_topology) =
            match rails::NodeRailContourSet::from_input_with_profile_and_topology_reuse(
                &input,
                road_debug,
                previous_topology.map(|topology| &topology.rail_topology),
            ) {
                Ok((rails, profile, reuse_status, topology)) => {
                    (rails, profile, reuse_status, topology)
                }
                Err(error) => {
                    self.log_node_validation_report(
                        &NodeValidationReport::from_rail_generation_error(node_id, kind, &error),
                    );
                    return None;
                }
            };
        let rails_ms = elapsed_ms(rails_start);

        let ownership_start = road_debug.then(Instant::now);
        let (ownership, ownership_incremental, ownership_reuse_stats) = if reuse_status
            .ownership_reuse_safe
        {
            let Some(previous_topology) = previous_topology else {
                return None;
            };
            let Some(previous_ownership) = previous_topology.ownership.as_ref() else {
                return None;
            };
            let ownership = if previous_ownership.node_id == input.node_id
                && previous_ownership.piece_kind == input.piece_kind
            {
                Arc::clone(previous_ownership)
            } else {
                Arc::new(
                    previous_ownership.clone_with_node_identity(input.node_id, input.piece_kind),
                )
            };
            (
                ownership,
                Arc::clone(&previous_topology.ownership_incremental),
                ownership::NodeOwnershipReuseStats::default(),
            )
        } else {
            match Self::build_node_boolean_ownership_from_rails_with_incremental_reuse(
                &rails,
                previous_topology.map(|topology| topology.ownership_incremental.as_ref()),
            ) {
                Ok((ownership, incremental, stats)) => {
                    (Arc::new(ownership), Arc::new(incremental), stats)
                }
                Err(error) => {
                    self.log_node_validation_report(
                        &NodeValidationReport::from_boolean_ownership_error(node_id, kind, &error),
                    );
                    return None;
                }
            }
        };
        let ownership_ms = elapsed_ms(ownership_start);
        let boolean_debug = detailed_road_geometry_debug_enabled().then(|| {
            NodeBooleanDebugSnapshot::from_rails_and_ownership(
                &rails,
                &ownership,
                node_corner_trims_apply_to_footprint(&rails),
            )
        });

        let ownership_diag_start = road_debug.then(Instant::now);
        if let Some(report) = NodeValidationReport::from_owned_region_arrangement_diagnostics(
            &ownership.owned_region_arrangement,
        ) {
            self.log_node_validation_report(&report);
            return None;
        }
        let ownership_diag_ms = elapsed_ms(ownership_diag_start);

        let cached_arrangement = reuse_status
            .arrangement_reuse_safe
            .then(|| previous_topology.and_then(|topology| topology.arrangement.as_ref()))
            .flatten()
            .cloned();
        let (
            arrangement,
            explicit_vertical_step_segments,
            arrangement_profile,
            heights_ms,
            arrangement_ms,
            arrangement_diag_ms,
            triangulation_ms,
            triangulation_validation_ms,
            attach_profile,
            attach_ms,
        ) = if let Some(arrangement) = cached_arrangement {
            let explicit_vertical_step_segments = previous_topology
                .and_then(|topology| topology.base_explicit_vertical_step_segments.as_ref())
                .cloned()
                .unwrap_or_else(|| Arc::new(arrangement.explicit_vertical_step_segments()));
            (
                arrangement,
                explicit_vertical_step_segments,
                arrangement::NodeArrangementBuildProfile::default(),
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
                arrangement::NodeArrangementAttachProfile::default(),
                0.0,
            )
        } else {
            let heights_start = road_debug.then(Instant::now);
            let heights =
                match Self::build_node_height_solution_from_ownership(&input, &rails, &ownership) {
                    Ok(heights) => heights,
                    Err(error) => {
                        let report =
                            NodeValidationReport::from_height_field_error(node_id, kind, &error)
                                .with_height_failure_context(&rails, &ownership);
                        self.log_node_validation_report(&report);
                        return None;
                    }
                };
            let heights_ms = elapsed_ms(heights_start);

            let arrangement_start = road_debug.then(Instant::now);
            let (mut arrangement, arrangement_profile, precomputed_explicit_steps) =
                match NodeArrangement::from_height_solution_with_profile(&heights, road_debug) {
                    Ok(result) => result,
                    Err(error) => {
                        self.log_node_validation_report(
                            &NodeValidationReport::from_arrangement_error(node_id, kind, &error),
                        );
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
            let triangulation = match precomputed_explicit_steps.as_deref() {
                Some(explicit_steps) => {
                    Self::build_node_triangulation_from_arrangement_with_explicit_steps(
                        &arrangement,
                        explicit_steps,
                    )
                }
                None => Self::build_node_triangulation_from_arrangement(&arrangement),
            };
            let triangulation = match triangulation {
                Ok(triangulation) => triangulation,
                Err(error) => {
                    self.log_node_validation_report(
                        &NodeValidationReport::from_triangulation_error(node_id, kind, &error),
                    );
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
                        self.log_node_validation_report(
                            &NodeValidationReport::from_arrangement_error(node_id, kind, &error),
                        );
                        return None;
                    }
                };
            let attach_ms = elapsed_ms(attach_start);
            let explicit_vertical_step_segments =
                Arc::new(triangulation.explicit_vertical_step_segments);
            (
                Arc::new(arrangement),
                explicit_vertical_step_segments,
                arrangement_profile,
                heights_ms,
                arrangement_ms,
                arrangement_diag_ms,
                triangulation_ms,
                triangulation_validation_ms,
                attach_profile,
                attach_ms,
            )
        };

        let export_start = road_debug.then(Instant::now);
        match Self::node_surface_regions_from_arrangement_with_profile_and_incremental_reuse_for_identity(
            node_id,
            kind,
            &arrangement,
            &ownership.footprint_shapes,
            &explicit_vertical_step_segments,
            road_debug,
            previous_topology.map(|topology| topology.export_incremental.as_ref()),
        ) {
            Ok((mut regions, export_profile, export_incremental, export_reuse_stats)) => {
                regions.boolean_debug = boolean_debug;
                let export_ms = elapsed_ms(export_start);
                if road_debug {
                    let total_ms = elapsed_ms(total_start);
                    Self::log_node_surface_smoothness_detail(node_id, kind, &regions);
                    let previous_ownership_hits = ownership_reuse_stats.cleanup_previous_hits
                        + ownership_reuse_stats.final_boundary_previous_hits
                        + ownership_reuse_stats.final_assembly_previous_hits
                        + ownership_reuse_stats.seam_extraction_previous_hits
                        + ownership_reuse_stats.edge_seam_previous_hits;
                    let previous_export_hits = export_reuse_stats.previous_hits();
                    if total_ms >= 50.0
                        || (kind == RoadSurfaceVisualNodePieceKind::JunctionN
                            && mouths.len() >= 4)
                        || previous_ownership_hits > 0
                        || previous_export_hits > 0
                        || reuse_status.rail_topology_reused
                        || reuse_status.ownership_reuse_safe
                    {
                        crate::debug_log!(
                            "road",
                            "node_compile_pipeline_detail node={} kind={:?} mouths={} rail_topology_reused={} ownership_reused={} ownership_cleanup_cache_hits={} ownership_cleanup_previous_hits={} ownership_cleanup_cache_misses={} ownership_final_boundary_cache_hits={} ownership_final_boundary_previous_hits={} ownership_final_boundary_cache_misses={} ownership_final_assembly_cache_hits={} ownership_final_assembly_previous_hits={} ownership_final_assembly_cache_misses={} ownership_seam_extraction_cache_hits={} ownership_seam_extraction_previous_hits={} ownership_seam_extraction_cache_misses={} ownership_edge_seam_cache_hits={} ownership_edge_seam_previous_hits={} ownership_edge_seam_cache_misses={} arrangement_vertices={} arrangement_regions={} road_polygons={} curb_polygons={} sidewalk_polygons={} raised_step_faces={} input_ms={:.3} rails_ms={:.3} ownership_ms={:.3} ownership_diag_ms={:.3} heights_ms={:.3} arrangement_ms={:.3} arrangement_diag_ms={:.3} triangulation_ms={:.3} triangulation_validation_ms={:.3} attach_ms={:.3} export_ms={:.3} total_ms={:.3}",
                            node_id,
                            kind,
                            mouths.len(),
                            reuse_status.rail_topology_reused,
                            reuse_status.ownership_reuse_safe,
                            ownership_reuse_stats.cleanup_cache_hits,
                            ownership_reuse_stats.cleanup_previous_hits,
                            ownership_reuse_stats.cleanup_cache_misses,
                            ownership_reuse_stats.final_boundary_cache_hits,
                            ownership_reuse_stats.final_boundary_previous_hits,
                            ownership_reuse_stats.final_boundary_cache_misses,
                            ownership_reuse_stats.final_assembly_cache_hits,
                            ownership_reuse_stats.final_assembly_previous_hits,
                            ownership_reuse_stats.final_assembly_cache_misses,
                            ownership_reuse_stats.seam_extraction_cache_hits,
                            ownership_reuse_stats.seam_extraction_previous_hits,
                            ownership_reuse_stats.seam_extraction_cache_misses,
                            ownership_reuse_stats.edge_seam_cache_hits,
                            ownership_reuse_stats.edge_seam_previous_hits,
                            ownership_reuse_stats.edge_seam_cache_misses,
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
                            "node_rail_build_detail node={} kind={:?} mouths={} contours={} constraints={} source_constraints={} validation_constraints={} height_carrier_sources={} height_carrier_points={} contact_pair_tests={} contact_candidate_pairs={} contact_pair_aabb_rejected={} contact_pair_kind_rejected={} contact_authority_rejected={} contact_pair_processed={} contact_same_material_candidate_pairs={} contact_raised_step_candidate_pairs={} contact_same_authority_skipped={} contact_overlay_calls={} same_material_overlay_calls={} same_material_pair_cache_hits={} raised_step_pair_cache_previous_hits={} raised_step_pair_cache_misses={} source_target_group_cache_hits={} source_contact_cache_hits={} source_contact_cache_misses={} source_pair_cache_hits={} source_pair_cache_misses={} contact_noding_pair_cache_hits={} contact_noding_pair_cache_misses={} contact_noding_component_cache_hits={} contact_noding_component_cache_misses={} retained_authority_cache_hits={} retained_authority_current_hits={} retained_authority_previous_hits={} retained_authority_cache_misses={} retained_decision_cache_hits={} retained_decision_current_hits={} retained_decision_previous_hits={} retained_decision_cache_misses={} contact_constraints_emitted={} same_material_height_split_candidates={} same_material_height_split_appended={} same_material_height_split_duplicates={} terminal_caps_ms={:.3} side_joins_ms={:.3} owners_ms={:.3} mouth_base_contours_ms={:.3} mouth_band_contours_ms={:.3} cap_height_carriers_ms={:.3} terminal_cap_contours_ms={:.3} side_join_contours_ms={:.3} boundary_constraints_ms={:.3} span_handoff_ms={:.3} contact_noding_first_ms={:.3} raised_step_contacts_first_ms={:.3} material_contacts_ms={:.3} raised_step_contacts_second_ms={:.3} contact_noding_second_ms={:.3} same_band_contacts_ms={:.3} contact_noding_third_ms={:.3} validation_source_constraints_ms={:.3} retain_constraints_ms={:.3} validate_endpoints_ms={:.3} source_carriers_ms={:.3} total_ms={:.3}",
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
                            rail_profile.contact_candidate_pairs,
                            rail_profile.contact_pair_aabb_rejected,
                            rail_profile.contact_pair_kind_rejected,
                            rail_profile.contact_authority_rejected,
                            rail_profile.contact_pair_processed,
                            rail_profile.contact_same_material_candidate_pairs,
                            rail_profile.contact_raised_step_candidate_pairs,
                            rail_profile.contact_same_authority_skipped,
                            rail_profile.contact_overlay_calls,
                            rail_profile.same_material_overlay_calls,
                            rail_profile.same_material_pair_cache_hits,
                            rail_profile.raised_step_pair_cache_previous_hits,
                            rail_profile.raised_step_pair_cache_misses,
                            rail_profile.source_target_group_cache_hits,
                            rail_profile.source_contact_cache_hits,
                            rail_profile.source_contact_cache_misses,
                            rail_profile.source_pair_cache_hits,
                            rail_profile.source_pair_cache_misses,
                            rail_profile.contact_noding_pair_cache_hits,
                            rail_profile.contact_noding_pair_cache_misses,
                            rail_profile.contact_noding_component_cache_hits,
                            rail_profile.contact_noding_component_cache_misses,
                            rail_profile.retained_authority_cache_hits,
                            rail_profile.retained_authority_current_hits,
                            rail_profile.retained_authority_previous_hits,
                            rail_profile.retained_authority_cache_misses,
                            rail_profile.retained_decision_cache_hits,
                            rail_profile.retained_decision_current_hits,
                            rail_profile.retained_decision_previous_hits,
                            rail_profile.retained_decision_cache_misses,
                            rail_profile.contact_constraints_emitted,
                            rail_profile.same_material_height_split_candidates,
                            rail_profile.same_material_height_split_appended,
                            rail_profile.same_material_height_split_duplicates,
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
                        crate::debug_log!(
                            "road",
                            "node_export_detail node={} kind={:?} arrangement_faces={} owned_regions={} footprint_loops={} earthwork_segments={} terrain_clip_loops={} raised_step_faces={} explicit_step_previous_hits={} explicit_step_misses={} explicit_step_pair_previous_hits={} explicit_step_pair_misses={} height_split_previous_hits={} height_split_misses={} top_edge_cache_hits={} top_edge_previous_hits={} top_edge_cache_misses={} raised_step_cache_hits={} raised_step_previous_hits={} raised_step_cache_misses={} explicit_step_topology_ms={:.3} height_split_validation_ms={:.3} authority_ms={:.3} face_export_ms={:.3} boundary_sources_ms={:.3} raised_step_faces_ms={:.3} material_partition_ms={:.3} footprint_boundary_ms={:.3} earthwork_boundary_ms={:.3} outer_boundary_ms={:.3} terrain_clip_ms={:.3} sorting_ms={:.3} total_ms={:.3}",
                            node_id,
                            kind,
                            export_profile.arrangement_faces,
                            export_profile.owned_regions,
                            export_profile.footprint_loops,
                            export_profile.earthwork_segments,
                            export_profile.terrain_clip_loops,
                            export_profile.raised_step_faces,
                            export_reuse_stats.explicit_step_previous_hits,
                            export_reuse_stats.explicit_step_misses,
                            export_reuse_stats.explicit_step_pair_previous_hits,
                            export_reuse_stats.explicit_step_pair_misses,
                            export_reuse_stats.height_split_previous_hits,
                            export_reuse_stats.height_split_misses,
                            export_reuse_stats.top_edge_cache_hits,
                            export_reuse_stats.top_edge_previous_hits,
                            export_reuse_stats.top_edge_cache_misses,
                            export_reuse_stats.raised_step_cache_hits,
                            export_reuse_stats.raised_step_previous_hits,
                            export_reuse_stats.raised_step_cache_misses,
                            export_profile.explicit_step_topology_ms,
                            export_profile.height_split_validation_ms,
                            export_profile.authority_ms,
                            export_profile.face_export_ms,
                            export_profile.boundary_sources_ms,
                            export_profile.raised_step_faces_ms,
                            export_profile.material_partition_ms,
                            export_profile.footprint_boundary_ms,
                            export_profile.earthwork_boundary_ms,
                            export_profile.outer_boundary_ms,
                            export_profile.terrain_clip_ms,
                            export_profile.sorting_ms,
                            export_profile.total_ms
                        );
                    }
                }
                let retain_whole_topology = self.retain_complete_node_topology_for_replay
                    || kind == RoadSurfaceVisualNodePieceKind::JunctionN;
                Some(super::NodeCanonicalSurfaceCompileResult {
                    regions,
                    topology_cache: Some(super::NodeCanonicalTopologyCache {
                        rail_topology: if retain_whole_topology {
                            rail_topology
                        } else {
                            rail_topology.into_incremental_only()
                        },
                        ownership: retain_whole_topology.then_some(ownership),
                        arrangement: retain_whole_topology.then(|| Arc::clone(&arrangement)),
                        base_explicit_vertical_step_segments: retain_whole_topology
                            .then(|| Arc::clone(&explicit_vertical_step_segments)),
                        ownership_incremental,
                        export_incremental: Arc::new(export_incremental),
                    }),
                    rail_topology_reused: reuse_status.rail_topology_reused,
                    ownership_reused: reuse_status.ownership_reuse_safe,
                    #[cfg(test)]
                    export_reuse_stats,
                })
            }
            Err(error) => {
                self.log_node_boundary_export_error(&arrangement, &error);
                None
            }
        }
    }
}
