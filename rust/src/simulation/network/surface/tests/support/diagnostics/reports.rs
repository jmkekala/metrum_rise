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
        return format!(
            "{} {}",
            report.debug_dump(),
            owned_region_arrangement_diagnostic_debug(&ownership, &rails)
        );
    }
    let heights = match RoadSurfaceSystem::build_node_height_solution_from_ownership(
        &input, &rails, &ownership,
    ) {
        Ok(heights) => heights,
        Err(error) => {
            if let NodeHeightFieldError::SharedSourceHeightConflict { .. } = &error {
                let mut debug = ownership_debug_for_height_conflict(&error, &rails, &ownership);
                if let NodeHeightFieldError::SharedSourceHeightConflict {
                    constraint_index: Some(constraint_index),
                    ..
                } = &error
                {
                    debug = format!(
                        "{debug} {}",
                        source_rail_debug_for_height_conflict(
                            &input,
                            rails.constraints.get(*constraint_index)
                        )
                    );
                }
                return format!(
                    "{} {}",
                    NodeValidationReport::from_height_field_error(node_id, piece_kind, &error,)
                        .debug_dump(),
                    debug,
                );
            }
            if let NodeHeightFieldError::MissingOwnedRegionCarrierSupport { .. } = &error {
                return format!(
                    "{} {}",
                    NodeValidationReport::from_height_field_error(node_id, piece_kind, &error)
                        .debug_dump(),
                    owned_region_height_support_debug(&ownership, &rails, &error)
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
            if let Some(extra) = triangulation_open_boundary_debug(&triangulation, &error.report) {
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

fn owned_region_height_support_debug(
    ownership: &ownership::NodeBooleanOwnership,
    rails: &rails::NodeRailContourSet,
    error: &NodeHeightFieldError,
) -> String {
    let NodeHeightFieldError::MissingOwnedRegionCarrierSupport {
        mouth_order_index,
        band_index,
        source_kind,
        owner,
        point_x_mm,
        point_z_mm,
        ..
    } = error
    else {
        return String::new();
    };
    let target_mm = (*point_x_mm, *point_z_mm);
    let matching_regions = ownership
        .owned_regions
        .iter()
        .enumerate()
        .filter(|(_, region)| {
            region.owner == *owner
                && region.kind == *source_kind
                && region.source_mouth_order_index == *mouth_order_index
                && region.source_band_index == Some(*band_index)
        })
        .map(|(region_index, region)| {
            let nearby_shape_points = region
                .shape
                .iter()
                .flat_map(|contour| contour.iter().copied())
                .map(|point| road_vec2_mm(backend::RoadVec2::new(point[0], point[1])))
                .filter(|point| point_near_mm(*point, target_mm, 2500))
                .collect::<Vec<_>>();
            let nearby_seams = region
                .seam_constraints
                .iter()
                .filter(|constraint| {
                    point_near_segment_bbox_mm(
                        target_mm,
                        road_vec2_mm(constraint.start_xz),
                        road_vec2_mm(constraint.end_xz),
                        2500,
                    )
                })
                .map(|constraint| {
                    (
                        constraint.constraint_index,
                        constraint.seam_source,
                        constraint.owner,
                        constraint.opposite_owner,
                        road_vec2_mm(constraint.start_xz),
                        road_vec2_mm(constraint.end_xz),
                        constraint.is_material_transition,
                        constraint.constrains_shared_height,
                    )
                })
                .collect::<Vec<_>>();
            (
                region_index,
                region.claim_priority,
                region.area_m2,
                nearby_shape_points,
                nearby_seams,
            )
        })
        .collect::<Vec<_>>();
    let matching_contours = rails
        .contours
        .iter()
        .enumerate()
        .filter(|(_, contour)| {
            contour.owner == Some(*owner)
                && contour.source_mouth_order_index == *mouth_order_index
                && contour.source_band_index == Some(*band_index)
                && matches!(
                    contour.kind,
                    rails::NodeGeneratedContourKind::Band { kind } if kind == *source_kind
                )
        })
        .filter(|(_, contour)| {
            contour
                .points_xz
                .iter()
                .copied()
                .map(road_vec2_mm)
                .any(|point| point_near_mm(point, target_mm, 2500))
        })
        .map(|(contour_index, contour)| {
            (
                contour_index,
                contour.purpose,
                contour.claim_priority,
                contour
                    .points_xz
                    .iter()
                    .copied()
                    .map(road_vec2_mm)
                    .collect::<Vec<_>>(),
                contour.height_points_world.as_ref().map(|points| {
                    points
                        .iter()
                        .map(|point| road_vec2_mm(backend::RoadVec2::new(point.x, point.z)))
                        .collect::<Vec<_>>()
                }),
            )
        })
        .collect::<Vec<_>>();
    format!(
        "height_support_debug target_mm={target_mm:?} matching_regions={matching_regions:?} matching_contours={matching_contours:?}"
    )
}

fn point_near_mm(point: (i64, i64), target: (i64, i64), tolerance_mm: i64) -> bool {
    (point.0 - target.0).abs() <= tolerance_mm && (point.1 - target.1).abs() <= tolerance_mm
}

fn point_near_segment_bbox_mm(
    point: (i64, i64),
    start: (i64, i64),
    end: (i64, i64),
    tolerance_mm: i64,
) -> bool {
    point.0 >= start.0.min(end.0) - tolerance_mm
        && point.0 <= start.0.max(end.0) + tolerance_mm
        && point.1 >= start.1.min(end.1) - tolerance_mm
        && point.1 <= start.1.max(end.1) + tolerance_mm
}

fn owned_region_arrangement_diagnostic_debug(
    ownership: &ownership::NodeBooleanOwnership,
    rails: &rails::NodeRailContourSet,
) -> String {
    let diagnostics = ownership
        .owned_region_arrangement
        .diagnostics()
        .iter()
        .take(4)
        .map(|diagnostic| match diagnostic {
            ownership::NodeOwnedRegionArrangementDiagnostic::MissingSeamConstraint {
                region_index,
                owner,
                opposite_owner,
                start,
                end,
            }
            | ownership::NodeOwnedRegionArrangementDiagnostic::UnmaterializedRaisedStepAuthority {
                region_index,
                owner,
                opposite_owner,
                start,
                end,
                ..
            }
            | ownership::NodeOwnedRegionArrangementDiagnostic::AmbiguousSeamConstraint {
                region_index,
                owner,
                opposite_owner,
                start,
                end,
            } => {
                let region_seams = ownership
                    .owned_regions
                    .get(*region_index)
                    .map(|region| {
                        region
                            .seam_constraints
                            .iter()
                            .map(|constraint| {
                                let rail = rails
                                    .constraints
                                    .iter()
                                    .find(|rail| rail.constraint_index == constraint.constraint_index);
                                let rail_kind = rail.map(|rail| rail.kind);
                                let rail_owner = rail.and_then(|rail| rail.owner);
                                let rail_opposite_owner = rail.and_then(|rail| rail.opposite_owner);
                                (
                                    constraint.constraint_index,
                                    (rail_kind, rail_owner, rail_opposite_owner),
                                    constraint.seam_source,
                                    (constraint.owner, constraint.opposite_owner),
                                    (
                                        super::segments::road_xz_key(constraint.start_xz).raw_tuple(),
                                        super::segments::road_xz_key(constraint.end_xz).raw_tuple(),
                                    ),
                                    (
                                        road_vec2_mm(constraint.start_xz),
                                        road_vec2_mm(constraint.end_xz),
                                    ),
                                    constraint.is_material_transition,
                                    constraint.constrains_shared_height,
                                )
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let nearby_regions = ownership
                    .owned_regions
                    .iter()
                    .enumerate()
                    .filter(|(_, region)| {
                        region.owner == *owner
                            || region.owner == *opposite_owner
                            || region.kind == RoadSurfaceBandKind::CurbOrShoulder
                    })
                    .filter_map(|(candidate_index, region)| {
                        let nearby_points = region
                            .shape
                            .iter()
                            .flat_map(|contour| contour.iter().copied())
                            .map(|point| road_vec2_mm(backend::RoadVec2::new(point[0], point[1])))
                            .filter(|point| {
                                point_near_segment_bbox_mm(
                                    *point,
                                    (start.x_mm(), start.z_mm()),
                                    (end.x_mm(), end.z_mm()),
                                    1000,
                                )
                            })
                            .collect::<Vec<_>>();
                        if nearby_points.is_empty() {
                            return None;
                        }
                        Some((
                            candidate_index,
                            region.kind,
                            region.owner,
                            region.claim_priority,
                            (region.area_m2 * 1_000_000.0).round() as i64,
                            nearby_points,
                        ))
                    })
                    .take(16)
                    .collect::<Vec<_>>();
                let nearby_rails = rails
                    .constraints
                    .iter()
                    .filter(|constraint| {
                        rail_constraint_matches_owner_kinds(constraint, *owner, *opposite_owner)
                            && rail_constraint_bbox_near_segment_mm(
                                constraint,
                                (start.x_mm(), start.z_mm()),
                                (end.x_mm(), end.z_mm()),
                                1500,
                            )
                    })
                    .take(16)
                    .map(|constraint| {
                        (
                            constraint.constraint_index,
                            constraint.kind,
                            constraint.owner,
                            constraint.opposite_owner,
                            constraint
                                .points_xz
                                .iter()
                                .map(|point| {
                                    (super::segments::road_xz_key(*point).raw_tuple(), road_vec2_mm(*point))
                                })
                                .collect::<Vec<_>>(),
                        )
                    })
                    .collect::<Vec<_>>();
                format!(
                    "owned_diag region={region_index} owner={owner:?} opposite={opposite_owner:?} start_key={start:?} end_key={end:?} start_mm=({}, {}) end_mm=({}, {}) region_seams={region_seams:?} nearby_regions={nearby_regions:?} nearby_rails={nearby_rails:?}",
                    start.x_mm(),
                    start.z_mm(),
                    end.x_mm(),
                    end.z_mm(),
                )
            }
            ownership::NodeOwnedRegionArrangementDiagnostic::AmbiguousOwnedBoundaryEdge {
                region_index,
                owner,
                opposite_owners,
                start,
                end,
            } => format!(
                "owned_diag ambiguous_boundary region={region_index} owner={owner:?} opposites={opposite_owners:?} start_mm=({}, {}) end_mm=({}, {})",
                start.x_mm(),
                start.z_mm(),
                end.x_mm(),
                end.z_mm(),
            ),
        })
        .collect::<Vec<_>>();
    format!("owned_arrangement_debug={diagnostics:?}")
}

fn rail_constraint_matches_owner_kinds(
    constraint: &rails::NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    [constraint.owner, constraint.opposite_owner]
        .into_iter()
        .flatten()
        .any(|constraint_owner| {
            constraint_owner.kind() == owner.kind()
                || constraint_owner.kind() == opposite_owner.kind()
        })
        || matches!(
            constraint.kind,
            rails::NodeRailConstraintKind::RaisedStepContact
                | rails::NodeRailConstraintKind::BandBoundary { .. }
                | rails::NodeRailConstraintKind::BandContour { .. }
                | rails::NodeRailConstraintKind::FootprintSeam { .. }
        )
}

fn rail_constraint_bbox_near_segment_mm(
    constraint: &rails::NodeRailConstraint,
    start_mm: (i64, i64),
    end_mm: (i64, i64),
    tolerance_mm: i64,
) -> bool {
    if constraint.points_xz.is_empty() {
        return false;
    }
    let segment_min_x = start_mm.0.min(end_mm.0) - tolerance_mm;
    let segment_max_x = start_mm.0.max(end_mm.0) + tolerance_mm;
    let segment_min_z = start_mm.1.min(end_mm.1) - tolerance_mm;
    let segment_max_z = start_mm.1.max(end_mm.1) + tolerance_mm;
    let mut rail_min_x = i64::MAX;
    let mut rail_max_x = i64::MIN;
    let mut rail_min_z = i64::MAX;
    let mut rail_max_z = i64::MIN;
    for point in &constraint.points_xz {
        let (x_mm, z_mm) = road_vec2_mm(*point);
        rail_min_x = rail_min_x.min(x_mm);
        rail_max_x = rail_max_x.max(x_mm);
        rail_min_z = rail_min_z.min(z_mm);
        rail_max_z = rail_max_z.max(z_mm);
    }
    rail_min_x <= segment_max_x
        && rail_max_x >= segment_min_x
        && rail_min_z <= segment_max_z
        && rail_max_z >= segment_min_z
}

fn road_vec2_mm(point: backend::RoadVec2) -> (i64, i64) {
    (
        (point.x * 1000.0).round() as i64,
        (point.y * 1000.0).round() as i64,
    )
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
    if let super::node::boundary::NodeBoundaryExportError::AmbiguousEarthworkBoundarySegmentSource {
        start_x_key,
        start_z_key,
        start_y_mm,
        end_x_key,
        end_z_key,
        end_y_mm,
        existing_height_field_id,
        incoming_height_field_id,
        ..
    } = error
    {
        let start_key = NodeArrangementKey::from_point(super::backend::RoadVec2::new(
            *start_x_key as f64 / super::backend::ROAD_OVERLAY_COORDINATE_SCALE,
            *start_z_key as f64 / super::backend::ROAD_OVERLAY_COORDINATE_SCALE,
        ));
        let end_key = NodeArrangementKey::from_point(super::backend::RoadVec2::new(
            *end_x_key as f64 / super::backend::ROAD_OVERLAY_COORDINATE_SCALE,
            *end_z_key as f64 / super::backend::ROAD_OVERLAY_COORDINATE_SCALE,
        ));
        let covering_exposed_edges = arrangement
            .edges()
            .iter()
            .filter(|edge| edge.exposed_boundary())
            .filter_map(|edge| {
                let start = arrangement.vertices().get(edge.start().index())?;
                let end = arrangement.vertices().get(edge.end().index())?;
                (super::segments::arrangement_key_lies_on_segment(
                    start_key,
                    start.key(),
                    end.key(),
                ) && super::segments::arrangement_key_lies_on_segment(
                    end_key,
                    start.key(),
                    end.key(),
                ))
                .then_some((
                    edge.owner(),
                    edge.height_field_id(),
                    (start.key().x_key(), start.height_mm(), start.key().z_key()),
                    (end.key().x_key(), end.height_mm(), end.key().z_key()),
                ))
            })
            .take(16)
            .collect::<Vec<_>>();
        return format!(
            "ambiguous_segment=(({start_x_key},{start_y_mm},{start_z_key}),({end_x_key},{end_y_mm},{end_z_key})) existing_height_field_id={existing_height_field_id:?} incoming_height_field_id={incoming_height_field_id:?} covering_exposed_edges={covering_exposed_edges:?}"
        );
    }
    if let super::node::boundary::NodeBoundaryExportError::MissingFootprintBoundaryHeight {
        x_key,
        z_key,
    } = error
    {
        let key = NodeArrangementKey::from_point(super::backend::RoadVec2::new(
            *x_key as f64 / super::backend::ROAD_OVERLAY_COORDINATE_SCALE,
            *z_key as f64 / super::backend::ROAD_OVERLAY_COORDINATE_SCALE,
        ));
        let vertices_at_key = arrangement
            .vertices()
            .iter()
            .filter(|vertex| vertex.key() == key)
            .map(|vertex| {
                (
                    vertex.height_mm(),
                    vertex.owners(),
                    vertex.grade_authority(),
                )
            })
            .collect::<Vec<_>>();
        let exposed_edges_at_key = arrangement
            .edges()
            .iter()
            .filter(|edge| edge.exposed_boundary())
            .filter_map(|edge| {
                let start = arrangement.vertices().get(edge.start().index())?;
                let end = arrangement.vertices().get(edge.end().index())?;
                super::segments::arrangement_key_lies_on_segment(key, start.key(), end.key())
                    .then_some((
                        edge.owner(),
                        (start.key().x_key(), start.height_mm(), start.key().z_key()),
                        (end.key().x_key(), end.height_mm(), end.key().z_key()),
                        edge.height_field_id(),
                    ))
            })
            .take(16)
            .collect::<Vec<_>>();
        let nearby_exposed_edges = arrangement
            .edges()
            .iter()
            .filter(|edge| edge.exposed_boundary())
            .filter_map(|edge| {
                let start = arrangement.vertices().get(edge.start().index())?;
                let end = arrangement.vertices().get(edge.end().index())?;
                point_near_segment_bbox_mm(
                    (*x_key / 1000, *z_key / 1000),
                    (start.key().x_key() / 1000, start.key().z_key() / 1000),
                    (end.key().x_key() / 1000, end.key().z_key() / 1000),
                    2,
                )
                .then_some((
                    edge.owner(),
                    (start.key().x_key(), start.height_mm(), start.key().z_key()),
                    (end.key().x_key(), end.height_mm(), end.key().z_key()),
                    edge.height_field_id(),
                ))
            })
            .take(16)
            .collect::<Vec<_>>();
        return format!(
            "boundary_key={key:?} vertices_at_key={vertices_at_key:?} exposed_edges_at_key={exposed_edges_at_key:?} nearby_exposed_edges={nearby_exposed_edges:?}"
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
    let vertices_at_key = arrangement
        .vertices()
        .iter()
        .filter(|vertex| vertex.key() == key)
        .map(|vertex| {
            (
                vertex.height_mm(),
                vertex.owners(),
                vertex.height_field_id(),
                vertex.grade_authority(),
            )
        })
        .collect::<Vec<_>>();
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
        "boundary_key={key:?} vertices_at_key={vertices_at_key:?} owner_pair_segments={owner_pair_segments:?} key_segments={key_segments:?} exposed_edges_at_key={exposed_edges_at_key:?}"
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
