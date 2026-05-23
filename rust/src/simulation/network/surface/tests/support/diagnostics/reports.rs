//! Canonical node pipeline diagnostic report helpers.

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
        super::node::boundary::NodeBoundaryExportError::EmptyOuterBoundary
    ) {
        let (bad_degree, bad_point_degree, first_edges) =
            exposed_boundary_degree_debug(arrangement);
        let exposed_edges = arrangement
            .edges()
            .iter()
            .filter(|edge| edge.exposed_boundary())
            .count();
        let region_summaries = arrangement
            .regions()
            .iter()
            .take(16)
            .map(|region| {
                (
                    region.owner(),
                    region.outer_loop().len(),
                    region.holes().len(),
                    region.boundary_edges().len(),
                    region.area_m2(),
                )
            })
            .collect::<Vec<_>>();
        return format!(
            "regions={} faces={} edges={} exposed_edges={} bad_xz_degrees={bad_degree:?} bad_point_degrees={bad_point_degree:?} first_edges={first_edges:?} first_regions={region_summaries:?}",
            arrangement.regions().len(),
            arrangement.faces().len(),
            arrangement.edges().len(),
            exposed_edges,
        );
    }
    if matches!(
        error,
        super::node::boundary::NodeBoundaryExportError::DegenerateOuterBoundaryLoop
    ) {
        let (bad_degree, _, first_edges) = exposed_boundary_degree_debug(arrangement);
        return format!(
            "exposed_edge_count={} bad_xz_degrees={bad_degree:?} first_edges={:?}",
            arrangement
                .edges()
                .iter()
                .filter(|edge| edge.exposed_boundary())
                .count(),
            first_edges
        );
    }
    if let super::node::boundary::NodeBoundaryExportError::MissingEarthworkBoundarySegmentSource {
        start_x_key,
        start_z_key,
        end_x_key,
        end_z_key,
        nearby_source_edges,
    } = error
    {
        let (bad_degree, bad_point_degree, first_edges) =
            exposed_boundary_degree_debug(arrangement);
        return format!(
            "missing_segment=(({start_x_key},{start_z_key}),({end_x_key},{end_z_key})) nearby_source_edges={nearby_source_edges:?} exposed_edge_count={} bad_xz_degrees={bad_degree:?} bad_point_degrees={bad_point_degree:?} first_edges={first_edges:?}",
            arrangement
                .edges()
                .iter()
                .filter(|edge| edge.exposed_boundary())
                .count(),
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
    let exposed_edges_at_key = arrangement
        .edges()
        .iter()
        .filter(|edge| edge.exposed_boundary())
        .filter_map(|edge| {
            let start = arrangement.vertices().get(edge.start().index())?;
            let end = arrangement.vertices().get(edge.end().index())?;
            super::segments::arrangement_key_lies_on_segment(key, start.key(), end.key()).then_some(
                (
                    edge.owner(),
                    (start.key().x_key(), start.height_mm(), start.key().z_key()),
                    (end.key().x_key(), end.height_mm(), end.key().z_key()),
                ),
            )
        })
        .take(12)
        .collect::<Vec<_>>();
    format!(
        "boundary_key={key:?} owner_pair_segments={owner_pair_segments:?} key_segments={key_segments:?} exposed_edges_at_key={exposed_edges_at_key:?}"
    )
}

fn exposed_boundary_degree_debug(
    arrangement: &NodeArrangement,
) -> (
    Vec<((i64, i64), usize)>,
    Vec<((i64, i64, i64), usize)>,
    Vec<((i64, i64, i64), (i64, i64, i64))>,
) {
    let mut degree = BTreeMap::<(i64, i64), usize>::new();
    let mut point_degree = BTreeMap::<(i64, i64, i64), usize>::new();
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
        *point_degree.entry(start_key).or_default() += 1;
        *point_degree.entry(end_key).or_default() += 1;
    }
    let bad_degree = degree
        .into_iter()
        .filter(|(_, count)| *count != 2)
        .take(24)
        .collect::<Vec<_>>();
    let bad_point_degree = point_degree
        .into_iter()
        .filter(|(_, count)| *count != 2)
        .take(24)
        .collect::<Vec<_>>();
    (
        bad_degree,
        bad_point_degree,
        exposed.into_iter().take(24).collect(),
    )
}
