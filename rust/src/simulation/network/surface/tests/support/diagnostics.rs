//! Node pipeline diagnostic helpers for road-surface tests.

use super::*;

pub(in crate::simulation::network::surface::tests) fn canonical_junction_pipeline_report(
    surface: &RoadSurfaceSystem,
    graph: &RegionGraph,
    node_id: u32,
) -> String {
    canonical_node_pipeline_report(
        surface,
        graph,
        node_id,
        RoadSurfaceVisualNodePieceKind::JunctionN,
    )
}

pub(in crate::simulation::network::surface::tests) fn canonical_node_pipeline_report(
    surface: &RoadSurfaceSystem,
    graph: &RegionGraph,
    node_id: u32,
    piece_kind: RoadSurfaceVisualNodePieceKind,
) -> String {
    let valid = graph.get_valid_node(node_id);
    let incidents = surface.sorted_incident_surface_edges(graph, valid);
    let Some(mouths) = surface.build_ordered_piece_mouths(&incidents) else {
        return format!("node {node_id}: failed to build ordered mouths");
    };
    let input = match RoadSurfaceSystem::build_node_arrangement_input_from_mouths(
        node_id, piece_kind, &mouths,
    ) {
        Ok(input) => input,
        Err(error) => return format!("node {node_id}: input extraction failed: {error:?}"),
    };
    let rails = match RoadSurfaceSystem::build_node_rail_contours_from_input(&input) {
        Ok(rails) => rails,
        Err(error) => {
            return NodeValidationReport::from_rail_generation_error(node_id, piece_kind, &error)
                .debug_dump();
        }
    };
    let ownership = match RoadSurfaceSystem::build_node_boolean_ownership_from_rails(&rails) {
        Ok(ownership) => ownership,
        Err(error) => {
            return format!(
                "{} error={error:?}",
                NodeValidationReport::from_boolean_ownership_error(node_id, piece_kind, &error)
                    .debug_dump()
            );
        }
    };
    if let Some(report) = NodeValidationReport::from_owned_region_arrangement_diagnostics(
        &ownership.owned_region_arrangement,
    ) {
        return report.debug_dump();
    }
    let heights = match RoadSurfaceSystem::build_node_height_solution_from_ownership(
        &input, &rails, &ownership,
    ) {
        Ok(heights) => heights,
        Err(error) => {
            if let NodeHeightFieldError::SharedSourceHeightConflict {
                constraint_index: Some(constraint_index),
                ..
            } = &error
            {
                return format!(
                    "{} {}",
                    NodeValidationReport::from_height_field_error(node_id, piece_kind, &error,)
                        .debug_dump(),
                    source_rail_debug_for_height_conflict(
                        &input,
                        rails.constraints.get(*constraint_index)
                    )
                );
            }
            return NodeValidationReport::from_height_field_error(node_id, piece_kind, &error)
                .debug_dump();
        }
    };
    let mut arrangement = match NodeArrangement::from_height_solution(&heights) {
        Ok(arrangement) => arrangement,
        Err(error) => {
            if let NodeArrangementError::DuplicateVertexHeightConflict { key, .. } = &error {
                return format!(
                    "{} vertices_at_key={:?}",
                    NodeValidationReport::from_arrangement_error(node_id, piece_kind, &error,)
                        .debug_dump(),
                    height_solution_vertices_at_arrangement_key(&heights, *key)
                );
            }
            return NodeValidationReport::from_arrangement_error(node_id, piece_kind, &error)
                .debug_dump();
        }
    };
    if let Some(report) = NodeValidationReport::from_arrangement_diagnostics(&arrangement) {
        return report.debug_dump();
    }
    let triangulation =
        match RoadSurfaceSystem::build_node_triangulation_from_arrangement(&arrangement) {
            Ok(triangulation) => triangulation,
            Err(error) => {
                return NodeValidationReport::from_triangulation_error(node_id, piece_kind, &error)
                    .debug_dump();
            }
        };
    match RoadSurfaceSystem::validate_node_triangulation_solution(&triangulation) {
        Ok(report) => {
            if !report.diagnostics.is_empty() {
                return report.debug_dump();
            }
        }
        Err(error) => {
            if let Some(extra) =
                triangulation_height_conflict_debug(&heights, &ownership, &error.report)
            {
                return format!("{} {extra}", error.report.debug_dump());
            }
            if let Some(extra) =
                triangulation_duplicate_exposed_edge_debug(&triangulation, &error.report)
            {
                return format!("{} {extra}", error.report.debug_dump());
            }
            return error.report.debug_dump();
        }
    }
    if let Err(error) = arrangement.attach_triangulation(&triangulation) {
        return NodeValidationReport::from_arrangement_error(node_id, piece_kind, &error)
            .debug_dump();
    }
    if let Err(error) = RoadSurfaceSystem::node_surface_regions_from_arrangement(
        &arrangement,
        &ownership.footprint_shapes,
    ) {
        return format!(
            "boundary export failed: {error:?} {}",
            boundary_export_step_debug(&arrangement, &error)
        );
    }
    format!("canonical {piece_kind:?} pipeline reached boundary export")
}

pub(in crate::simulation::network::surface::tests) fn boundary_export_step_debug(
    arrangement: &NodeArrangement,
    error: &super::node::boundary::NodeBoundaryExportError,
) -> String {
    if matches!(
        error,
        super::node::boundary::NodeBoundaryExportError::DegenerateOuterBoundaryLoop
    ) {
        let mut degree = BTreeMap::<(i64, i64), usize>::new();
        let mut exposed = Vec::new();
        for edge in arrangement
            .edges()
            .iter()
            .filter(|edge| edge.exposed_boundary())
        {
            let Some(start) = arrangement.vertices().get(edge.start().index()) else {
                continue;
            };
            let Some(end) = arrangement.vertices().get(edge.end().index()) else {
                continue;
            };
            let start_key = (start.key().x_key(), start.key().z_key(), start.height_mm());
            let end_key = (end.key().x_key(), end.key().z_key(), end.height_mm());
            exposed.push((start_key, end_key));
            *degree
                .entry((start.key().x_key(), start.key().z_key()))
                .or_default() += 1;
            *degree
                .entry((end.key().x_key(), end.key().z_key()))
                .or_default() += 1;
        }
        let bad_degree = degree
            .into_iter()
            .filter(|(_, count)| *count != 2)
            .take(24)
            .collect::<Vec<_>>();
        return format!(
            "exposed_edge_count={} bad_xz_degrees={bad_degree:?} first_edges={:?}",
            exposed.len(),
            exposed.into_iter().take(24).collect::<Vec<_>>()
        );
    }
    let super::node::boundary::NodeBoundaryExportError::ConflictingFootprintBoundaryHeight {
        x_key,
        z_key,
        existing_owner_kind,
        existing_owner_index,
        incoming_owner_kind,
        incoming_owner_index,
        ..
    } = error
    else {
        return String::new();
    };
    let key = NodeArrangementKey::from_point(super::backend::RoadVec2::new(
        *x_key as f64 / super::backend::ROAD_OVERLAY_COORDINATE_SCALE,
        *z_key as f64 / super::backend::ROAD_OVERLAY_COORDINATE_SCALE,
    ));
    let existing_owner = NodeBandOwner::new(*existing_owner_kind, *existing_owner_index);
    let incoming_owner = NodeBandOwner::new(*incoming_owner_kind, *incoming_owner_index);
    let step_segments = arrangement.explicit_vertical_step_segments();
    let owner_pair_segments = step_segments
        .iter()
        .filter(|segment| {
            (segment.owner() == existing_owner && segment.opposite_owner() == incoming_owner)
                || (segment.owner() == incoming_owner && segment.opposite_owner() == existing_owner)
        })
        .copied()
        .collect::<Vec<_>>();
    let key_segments = owner_pair_segments
        .iter()
        .filter(|segment| {
            super::segments::arrangement_key_lies_on_segment(key, segment.start(), segment.end())
        })
        .copied()
        .collect::<Vec<_>>();
    format!(
        "boundary_key={key:?} owner_pair_segments={owner_pair_segments:?} key_segments={key_segments:?}"
    )
}

pub(in crate::simulation::network::surface::tests) fn assert_junction_rejected_with_canonical_height_diagnostic(
    surface: &RoadSurfaceSystem,
    graph: &RegionGraph,
    node_id: u32,
    label: &str,
) {
    assert!(
        !surface.compiled_visual_node_pieces().contains_key(&node_id),
        "{label} unexpectedly compiled after same-XZ height disagreement"
    );
    let report = canonical_junction_pipeline_report(surface, graph, node_id);
    let accepted_height_rejection = report.contains("shared_source_height_conflict")
        || report.contains("source_height_field_conflict")
        || report.contains("vertex_outside_height_field")
        || report.contains("\"height_conflict\"")
        || report.contains("missing_raised_step_vertical_face")
        || report.contains("MissingRaisedStepVerticalFace");
    assert!(
        accepted_height_rejection,
        "{label} must reject with a canonical height diagnostic: {report}"
    );
}

pub(in crate::simulation::network::surface::tests) fn triangulation_height_conflict_debug(
    heights: &super::height::NodeHeightSolution,
    ownership: &super::ownership::NodeBooleanOwnership,
    report: &NodeValidationReport,
) -> Option<String> {
    report.diagnostics.iter().find_map(|diagnostic| {
        if let NodeGeometryDiagnosticKind::CrossRegionHeightConflict {
            edge_start_x_key,
            edge_start_z_key,
            edge_end_x_key,
            edge_end_z_key,
            ..
        } = diagnostic.kind
        {
            let start_key = arrangement_key_from_overlay_keys(edge_start_x_key, edge_start_z_key);
            let end_key = arrangement_key_from_overlay_keys(edge_end_x_key, edge_end_z_key);
            Some(format!(
                "start_vertices={:?} end_vertices={:?} ownership={:?}",
                height_solution_vertices_at_arrangement_key(heights, start_key),
                height_solution_vertices_at_arrangement_key(heights, end_key),
                owned_region_claims_for_height_conflict(ownership, diagnostic)
            ))
        } else {
            None
        }
    })
}

pub(in crate::simulation::network::surface::tests) fn triangulation_duplicate_exposed_edge_debug(
    triangulation: &super::triangulation::NodeTriangulationSolution,
    report: &NodeValidationReport,
) -> Option<String> {
    report.diagnostics.iter().find_map(|diagnostic| {
        if let NodeGeometryDiagnosticKind::DuplicateExposedEdge {
            start_x_mm,
            start_z_mm,
            end_x_mm,
            end_z_mm,
            ..
        } = diagnostic.kind
        {
            Some(format!(
                "duplicate_edge_regions={:?}",
                triangulation_regions_for_exposed_edge(
                    triangulation,
                    (start_x_mm, start_z_mm),
                    (end_x_mm, end_z_mm),
                )
            ))
        } else {
            None
        }
    })
}

pub(in crate::simulation::network::surface::tests) fn triangulation_regions_for_exposed_edge(
    triangulation: &super::triangulation::NodeTriangulationSolution,
    start_mm: (i64, i64),
    end_mm: (i64, i64),
) -> Vec<String> {
    let expected = normalized_test_mm_edge_key(start_mm, end_mm);
    let mut matches = Vec::new();
    for (region_index, region) in triangulation.regions.iter().enumerate() {
        let mut edge_counts = BTreeMap::<((i64, i64), (i64, i64)), usize>::new();
        for triangle in &region.triangles {
            for edge_index in 0..3 {
                let start = &region.vertices[triangle.vertices[edge_index]];
                let end = &region.vertices[triangle.vertices[(edge_index + 1) % 3]];
                *edge_counts
                    .entry(normalized_test_world_mm_edge_key(
                        start.point_world.x as f32,
                        start.point_world.z as f32,
                        end.point_world.x as f32,
                        end.point_world.z as f32,
                    ))
                    .or_default() += 1;
            }
        }
        if let Some(count) = edge_counts.get(&expected).copied() {
            matches.push(format!(
                "region={} owner={:?} height_field={:?} local_count={}",
                region_index, region.owner, region.height_field_id, count
            ));
        }
    }
    matches
}

pub(in crate::simulation::network::surface::tests) fn normalized_test_world_mm_edge_key(
    start_x: f32,
    start_z: f32,
    end_x: f32,
    end_z: f32,
) -> ((i64, i64), (i64, i64)) {
    normalized_test_mm_edge_key(
        (
            (start_x * 1000.0).round() as i64,
            (start_z * 1000.0).round() as i64,
        ),
        (
            (end_x * 1000.0).round() as i64,
            (end_z * 1000.0).round() as i64,
        ),
    )
}

pub(in crate::simulation::network::surface::tests) fn normalized_test_mm_edge_key(
    start: (i64, i64),
    end: (i64, i64),
) -> ((i64, i64), (i64, i64)) {
    if start <= end {
        (start, end)
    } else {
        (end, start)
    }
}

pub(in crate::simulation::network::surface::tests) fn owned_region_claims_for_height_conflict(
    ownership: &super::ownership::NodeBooleanOwnership,
    diagnostic: &super::validation::NodeGeometryDiagnostic,
) -> Vec<String> {
    let NodeGeometryDiagnosticKind::CrossRegionHeightConflict {
        existing_region_index,
        incoming_region_index,
        ..
    } = diagnostic.kind
    else {
        return Vec::new();
    };
    [existing_region_index, incoming_region_index]
        .into_iter()
        .filter_map(|region_index| {
            ownership.owned_regions.get(region_index).map(|region| {
                format!(
                    "region={} kind={:?} owner={:?} claim={:?} source_mouth={} source_band={:?} area={:.6}",
                    region_index,
                    region.kind,
                    region.owner,
                    region.claim_priority,
                    region.source_mouth_order_index,
                    region.source_band_index,
                    region.area_m2
                )
            })
        })
        .collect()
}

pub(in crate::simulation::network::surface::tests) fn arrangement_key_from_overlay_keys(
    x_key: i64,
    z_key: i64,
) -> NodeArrangementKey {
    NodeArrangementKey::from_point(super::backend::RoadVec2::new(
        x_key as f64 / super::backend::ROAD_OVERLAY_COORDINATE_SCALE,
        z_key as f64 / super::backend::ROAD_OVERLAY_COORDINATE_SCALE,
    ))
}

pub(in crate::simulation::network::surface::tests) fn source_rail_debug_for_height_conflict(
    input: &super::input::NodeArrangementInput,
    constraint: Option<&super::rails::NodeRailConstraint>,
) -> String {
    let Some(constraint) = constraint else {
        return "rail_constraint=<missing>".to_string();
    };
    let mut parts = vec![format!("rail_constraint={constraint:?}")];
    let Some(boundary_index) = constraint.source_boundary_index else {
        return parts.join(" ");
    };
    let Some(mouth) = input
        .mouths
        .iter()
        .find(|mouth| mouth.order_index == constraint.source_mouth_order_index)
    else {
        parts.push("mouth=<missing>".to_string());
        return parts.join(" ");
    };
    if let Some(boundary_rail) = mouth.boundary_rails.get(boundary_index) {
        parts.push(format!(
            "boundary_path={}",
            world_path_debug(&boundary_rail.path_world)
        ));
    }
    if let Some(left_band) = boundary_index
        .checked_sub(1)
        .and_then(|index| mouth.band_intervals.get(index))
    {
        parts.push(format!(
            "left_band={:?} start_path={} end_path={}",
            left_band.band_kind,
            world_path_debug(&left_band.start_path_world),
            world_path_debug(&left_band.end_path_world)
        ));
    }
    if let Some(right_band) = mouth.band_intervals.get(boundary_index) {
        parts.push(format!(
            "right_band={:?} start_path={} end_path={}",
            right_band.band_kind,
            world_path_debug(&right_band.start_path_world),
            world_path_debug(&right_band.end_path_world)
        ));
    }
    parts.join(" ")
}

pub(in crate::simulation::network::surface::tests) fn world_path_debug(
    path: &[super::backend::RoadVec3],
) -> String {
    let points = path
        .iter()
        .map(|point| format!("({:.3},{:.3},{:.3})", point.x, point.y, point.z))
        .collect::<Vec<_>>();
    format!("[{}]", points.join(","))
}

pub(in crate::simulation::network::surface::tests) fn height_solution_vertices_at_arrangement_key(
    heights: &super::height::NodeHeightSolution,
    key: NodeArrangementKey,
) -> Vec<String> {
    let mut matches = Vec::new();
    for (region_index, region) in heights.regions.iter().enumerate() {
        for vertex in region.shape.iter().flat_map(|contour| contour.iter()) {
            if NodeArrangementKey::from_point(vertex.point_xz) != key {
                continue;
            }
            let touching_seams = region
                .seam_constraints
                .iter()
                .filter(|constraint| {
                    let start = NodeArrangementKey::from_point(constraint.start_xz);
                    let end = NodeArrangementKey::from_point(constraint.end_xz);
                    start == key || end == key
                })
                .map(|constraint| {
                    format!(
                        "#{} {:?} owner={:?} opposite={:?} shared={} material={}",
                        constraint.constraint_index,
                        constraint.seam_source,
                        constraint.owner,
                        constraint.opposite_owner,
                        constraint.constrains_shared_height,
                        constraint.is_material_transition
                    )
                })
                .collect::<Vec<_>>();
            matches.push(format!(
                "region={} kind={:?} owner={:?} field={:?} height={:.3} seams={:?}",
                region_index,
                region.kind,
                region.owner,
                vertex.height_field_id,
                vertex.height_m,
                touching_seams
            ));
        }
    }
    matches
}

pub(in crate::simulation::network::surface::tests) fn assert_debug_dump_mouth_seams_are_clean(
    dump: &str,
) {
    let json_start = dump
        .find('{')
        .expect("road geometry dump should contain a JSON object");
    let json_end = dump
        .rfind('}')
        .expect("road geometry dump should contain a JSON object");
    let json: serde_json::Value = serde_json::from_str(&dump[json_start..=json_end])
        .expect("road geometry dump JSON should parse");
    let nodes = json["nodes"]
        .as_array()
        .expect("road geometry dump should include nodes");
    let mut checked = 0usize;
    for node in nodes {
        let node_id = node["node_id"].as_u64().unwrap_or_default();
        let mouth_seams = node["mouth_seams"]
            .as_array()
            .expect("node debug dump should include mouth seams");
        for seam in mouth_seams {
            checked += 1;
            let problem_count = seam["problem_count"]
                .as_u64()
                .expect("mouth seam debug should include a problem count");
            assert_eq!(
                problem_count, 0,
                "mouth seam debug must be clean; node_id={node_id} seam={seam}"
            );
        }
    }
    assert!(
        checked > 0,
        "road geometry dump should include mouth seam checks"
    );
}
