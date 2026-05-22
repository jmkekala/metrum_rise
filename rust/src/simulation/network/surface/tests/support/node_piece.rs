//! Node-piece ownership and visible-top assertions for road-surface tests.

use super::*;

pub(in crate::simulation::network::surface::tests) fn assert_node_top_covers_footprint(
    piece: &RoadSurfaceVisualNodePiece,
) {
    let (missing_area_m2, extra_area_m2, budget_m2, missing_shapes, extra_shapes) =
        node_top_coverage_details_m2(piece);
    assert!(
        missing_area_m2 <= budget_m2 && extra_area_m2 <= budget_m2,
        "node top surfaces must exactly cover the canonical footprint; kind={:?} missing_area={missing_area_m2:.6} extra_area={extra_area_m2:.6} budget={budget_m2:.6} missing_shapes={missing_shapes:?} extra_shapes={extra_shapes:?}",
        piece.kind
    );
}

pub(in crate::simulation::network::surface::tests) fn assert_material_triangles_do_not_overlap(
    piece: &RoadSurfaceVisualNodePiece,
) {
    for non_road_region in piece
        .owned_regions
        .iter()
        .filter(|region| region.kind != RoadSurfaceBandKind::Carriageway)
    {
        for &non_road_triangle in &non_road_region.polygon.triangles_world {
            for road_region in piece
                .owned_regions
                .iter()
                .filter(|region| region.kind == RoadSurfaceBandKind::Carriageway)
            {
                for &road_triangle in &road_region.polygon.triangles_world {
                    let overlap_area_m2 =
                        triangle_overlap_area_m2(non_road_triangle, road_triangle);
                    let area_budget_m2 =
                        triangle_overlap_numeric_budget_m2(non_road_triangle, road_triangle);
                    assert!(
                        overlap_area_m2 <= area_budget_m2,
                        "node material triangles must not overlap beyond numeric dust; kind={:?} overlap_area={overlap_area_m2:.8} budget={area_budget_m2:.8} non_road_triangle={non_road_triangle:?} road_triangle={road_triangle:?}",
                        non_road_region.kind
                    );
                }
            }
        }
    }
}

pub(in crate::simulation::network::surface::tests) fn assert_terminal_mouth_handoff_surface_is_owned(
    piece: &RoadSurfaceVisualNodePiece,
    mouth: &super::IncidentMouthProfile,
    material: RoadSurfaceBandKind,
    start_boundary_index: usize,
    end_boundary_index: usize,
    label: &str,
) {
    let start = mouth.boundary_points_world[start_boundary_index];
    let end = mouth.boundary_points_world[end_boundary_index];
    let inward = mouth.inward_direction_xz.normalized();
    let sample = Vector2::new(
        (start.x + end.x) * 0.5 - inward.x * 0.1,
        (start.z + end.z) * 0.5 - inward.y * 0.1,
    );
    let polygons = match material {
        RoadSurfaceBandKind::CurbOrShoulder => &piece.curb_surface_polygons,
        RoadSurfaceBandKind::Sidewalk => &piece.sidewalk_surface_polygons,
        RoadSurfaceBandKind::Carriageway => &piece.road_surface_polygons,
        _ => &piece.sidewalk_surface_polygons,
    };
    assert!(
        point_inside_visual_polygons(polygons, sample),
        "terminal handoff surface must be owned by {material:?}; label={label} sample={sample:?}"
    );
}

pub(in crate::simulation::network::surface::tests) fn assert_terminal_band_interval_grid_is_owned(
    piece: &RoadSurfaceVisualNodePiece,
    endpoint: &super::IncidentMouthProfile,
    mouth: &super::IncidentMouthProfile,
    material: RoadSurfaceBandKind,
    start_boundary_index: usize,
    end_boundary_index: usize,
    label: &str,
) {
    let polygons = match material {
        RoadSurfaceBandKind::CurbOrShoulder => &piece.curb_surface_polygons,
        RoadSurfaceBandKind::Sidewalk => &piece.sidewalk_surface_polygons,
        RoadSurfaceBandKind::Carriageway => &piece.road_surface_polygons,
        _ => &piece.sidewalk_surface_polygons,
    };
    for longitudinal_t in [0.1_f32, 0.5, 0.9, 0.98] {
        for lateral_t in [0.05_f32, 0.5, 0.95] {
            let endpoint_start = endpoint.boundary_points_world[start_boundary_index];
            let endpoint_end = endpoint.boundary_points_world[end_boundary_index];
            let mouth_start = mouth.boundary_points_world[start_boundary_index];
            let mouth_end = mouth.boundary_points_world[end_boundary_index];
            let endpoint_sample = endpoint_start.lerp(endpoint_end, lateral_t);
            let mouth_sample = mouth_start.lerp(mouth_end, lateral_t);
            let sample_world = endpoint_sample.lerp(mouth_sample, longitudinal_t);
            let sample = Vector2::new(sample_world.x, sample_world.z);
            assert!(
                point_inside_visual_polygons(polygons, sample),
                "terminal band interval must be owned by {material:?}; label={label} longitudinal_t={longitudinal_t} lateral_t={lateral_t} sample={sample:?}"
            );
        }
    }
}

pub(in crate::simulation::network::surface::tests) fn assert_terminal_band_interval_grid_is_not_duplicated_by_span(
    span_piece: &super::RoadSurfaceVisualSpanPiece,
    endpoint: &super::IncidentMouthProfile,
    mouth: &super::IncidentMouthProfile,
    start_boundary_index: usize,
    end_boundary_index: usize,
    label: &str,
) {
    for longitudinal_t in [0.1_f32, 0.5, 0.9, 0.98] {
        for lateral_t in [0.05_f32, 0.5, 0.95] {
            let endpoint_start = endpoint.boundary_points_world[start_boundary_index];
            let endpoint_end = endpoint.boundary_points_world[end_boundary_index];
            let mouth_start = mouth.boundary_points_world[start_boundary_index];
            let mouth_end = mouth.boundary_points_world[end_boundary_index];
            let endpoint_sample = endpoint_start.lerp(endpoint_end, lateral_t);
            let mouth_sample = mouth_start.lerp(mouth_end, lateral_t);
            let sample_world = endpoint_sample.lerp(mouth_sample, longitudinal_t);
            let sample = Vector2::new(sample_world.x, sample_world.z);
            let duplicated =
                point_inside_visual_polygons(&span_piece.road_surface_polygons, sample)
                    || point_inside_visual_polygons(&span_piece.curb_surface_polygons, sample)
                    || point_inside_visual_polygons(&span_piece.sidewalk_surface_polygons, sample);
            assert!(
                !duplicated,
                "terminal band interval must not be duplicated by span top surfaces; label={label} longitudinal_t={longitudinal_t} lateral_t={lateral_t} sample={sample:?}"
            );
        }
    }
}

pub(in crate::simulation::network::surface::tests) fn assert_top_mesh_centroids_inside_outer_boundary(
    piece: &RoadSurfaceVisualNodePiece,
) {
    for triangle in piece
        .road_surface_polygons
        .iter()
        .chain(piece.curb_surface_polygons.iter())
        .chain(piece.sidewalk_surface_polygons.iter())
        .flat_map(|polygon| polygon.triangles_world.iter().copied())
    {
        let centroid = triangle_centroid_xz(triangle);
        assert!(
            point_inside_visual_polygons(&piece.outer_boundary_loops, centroid),
            "node outer boundary must contain emitted top-surface triangle centroids; centroid={centroid:?}"
        );
    }
}

pub(in crate::simulation::network::surface::tests) fn assert_top_surface_triangles_face_up(
    piece: &RoadSurfaceVisualNodePiece,
) {
    for triangle in piece
        .road_surface_polygons
        .iter()
        .chain(piece.curb_surface_polygons.iter())
        .chain(piece.sidewalk_surface_polygons.iter())
        .flat_map(|polygon| polygon.triangles_world.iter().copied())
    {
        let double_area_xz = (triangle[1].x - triangle[0].x) * (triangle[2].z - triangle[0].z)
            - (triangle[1].z - triangle[0].z) * (triangle[2].x - triangle[0].x);
        assert!(
            double_area_xz >= -0.001,
            "node top-surface triangles must remain front-facing from above; kind={:?} triangle={triangle:?} double_area_xz={double_area_xz:.6}",
            piece.kind
        );
    }
}

pub(in crate::simulation::network::surface::tests) fn assert_node_piece_uses_band_owned_regions(
    piece: &RoadSurfaceVisualNodePiece,
) {
    assert!(
        !piece.owned_regions.is_empty(),
        "node piece must keep explicit band-owned regions as its source of rendered top surfaces"
    );
    let carriageway_count = piece
        .owned_regions
        .iter()
        .filter(|region| region.kind == RoadSurfaceBandKind::Carriageway)
        .count();
    let non_road_count = piece
        .owned_regions
        .iter()
        .filter(|region| {
            region.kind != RoadSurfaceBandKind::Carriageway
                && region.kind != RoadSurfaceBandKind::CurbOrShoulder
        })
        .count();
    let curb_count = piece
        .owned_regions
        .iter()
        .filter(|region| region.kind == RoadSurfaceBandKind::CurbOrShoulder)
        .count();
    assert_eq!(
        carriageway_count,
        piece.road_surface_polygons.len(),
        "asphalt polygons must be derived from carriageway-owned node regions"
    );
    assert_eq!(
        curb_count,
        piece.curb_surface_polygons.len(),
        "curb polygons must be derived from curb/shoulder-owned node regions"
    );
    assert_eq!(
        non_road_count,
        piece.sidewalk_surface_polygons.len(),
        "sidewalk polygons must be derived from sidewalk-owned node regions"
    );
    assert!(
        piece
            .owned_regions
            .iter()
            .all(|region| RoadSurfaceSystem::polygon_has_area_xz(&region.polygon.points_world)),
        "owned node regions must be non-degenerate before triangulation"
    );
    assert_node_top_surface_sources_have_grade_authority(piece);
    assert_node_terrain_clip_sources_have_footprint_provenance(piece);
}

pub(in crate::simulation::network::surface::tests) fn assert_node_top_surface_sources_have_grade_authority(
    piece: &RoadSurfaceVisualNodePiece,
) {
    assert_eq!(
        piece.node_top_surface_sources.len(),
        piece.owned_regions.len(),
        "every emitted node top region must carry one provenance record"
    );
    assert!(
        !piece.node_grade_authorities.is_empty(),
        "node top provenance must reference a non-empty grade-authority table"
    );
    for source in &piece.node_top_surface_sources {
        assert!(
            !source.vertex_sources.is_empty(),
            "node top provenance must name polygon vertex sources"
        );
        assert!(
            !source.triangle_sources.is_empty(),
            "node top provenance must name emitted triangle sources"
        );
        for grade_authority_index in
            source
                .vertex_sources
                .iter()
                .map(|source| source.grade_authority_index)
                .chain(source.triangle_sources.iter().flat_map(|triangle| {
                    triangle.iter().map(|source| source.grade_authority_index)
                }))
        {
            assert!(
                grade_authority_index < piece.node_grade_authorities.len(),
                "node top provenance index {grade_authority_index} must reference an emitted grade-authority row"
            );
        }
    }
}

pub(in crate::simulation::network::surface::tests) fn assert_node_terrain_clip_sources_have_footprint_provenance(
    piece: &RoadSurfaceVisualNodePiece,
) {
    for edge in piece
        .terrain_clip_boundary_loops
        .iter()
        .flat_map(|boundary_loop| boundary_loop.source_edges.iter())
    {
        let RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
            node_id,
            kind,
            owner_kind,
            owner_index,
            boundary_source,
        } = edge.source
        else {
            panic!(
                "node terrain clip edge must carry node footprint provenance, got {:?}",
                edge.source
            );
        };
        assert_eq!(node_id, piece.node_id);
        assert_eq!(kind, piece.kind);
        assert!(
            piece
                .owned_regions
                .iter()
                .any(|region| region.kind == owner_kind && region.owner_index == owner_index),
            "node terrain clip edge owner must refer to a canonical owned top region"
        );
        let boundary_source =
            boundary_source.expect("node terrain clip edge must carry exact endpoint provenance");
        assert_node_footprint_boundary_vertex_source_is_valid(piece, boundary_source.start);
        assert_node_footprint_boundary_vertex_source_is_valid(piece, boundary_source.end);
    }
}

pub(in crate::simulation::network::surface::tests) fn assert_node_piece_has_curb_and_sidewalk_owners(
    piece: &RoadSurfaceVisualNodePiece,
) {
    assert!(
        piece
            .owned_regions
            .iter()
            .any(|region| region.kind == RoadSurfaceBandKind::CurbOrShoulder),
        "node non-road hardcut must expose explicit curb/shoulder owners"
    );
    assert!(
        piece
            .owned_regions
            .iter()
            .any(|region| region.kind == RoadSurfaceBandKind::Sidewalk),
        "node non-road hardcut must expose explicit sidewalk owners"
    );
}

pub(in crate::simulation::network::surface::tests) fn assert_compiled_bend_piece<'a>(
    surface: &'a RoadSurfaceSystem,
    graph: &RegionGraph,
    bend: u32,
) -> &'a RoadSurfaceVisualNodePiece {
    let piece = surface
        .compiled_visual_node_pieces()
        .get(&bend)
        .unwrap_or_else(|| {
            panic!(
                "bend should compile through canonical owned regions: {}",
                canonical_node_pipeline_report(
                    surface,
                    graph,
                    bend,
                    RoadSurfaceVisualNodePieceKind::Bend
                )
            )
        });
    assert_eq!(piece.kind, RoadSurfaceVisualNodePieceKind::Bend);
    assert_node_piece_uses_band_owned_regions(piece);
    assert_node_piece_has_curb_and_sidewalk_owners(piece);
    assert_material_triangles_do_not_overlap(piece);
    assert!(!piece.outer_boundary_loops.is_empty());
    assert!(
        !piece.render_earthwork_faces.is_empty(),
        "bend piece must emit terrain skirt faces from its canonical outer boundary"
    );
    assert!(!piece.road_surface_polygons.is_empty());
    assert!(!piece.curb_surface_polygons.is_empty());
    assert!(!piece.raised_step_face_polygons.is_empty());
    assert!(!piece.sidewalk_surface_polygons.is_empty());
    assert_top_mesh_centroids_inside_outer_boundary(piece);
    assert_top_surface_triangles_face_up(piece);
    assert_raised_step_faces_have_top_support(piece);
    assert_raised_step_faces_visible_from_lower_owner(piece);
    assert_top_raised_step_owner_boundaries_have_vertical_faces(piece);
    assert_outer_boundary_vertices_match_visible_top(piece);
    assert_node_top_covers_footprint(piece);
    assert_node_earthwork_faces_have_footprint_provenance(piece);
    assert_earthwork_faces_stay_outside_top_footprint(piece);
    piece
}

pub(in crate::simulation::network::surface::tests) fn assert_compiled_junction_piece<'a>(
    surface: &'a RoadSurfaceSystem,
    graph: &RegionGraph,
    junction: u32,
) -> &'a RoadSurfaceVisualNodePiece {
    let piece = surface
        .compiled_visual_node_pieces()
        .get(&junction)
        .unwrap_or_else(|| {
            panic!(
                "junction should compile through canonical owned regions: {}",
                canonical_junction_pipeline_report(surface, graph, junction)
            )
        });
    assert_eq!(piece.kind, RoadSurfaceVisualNodePieceKind::JunctionN);
    assert_node_piece_uses_band_owned_regions(piece);
    assert_node_piece_has_curb_and_sidewalk_owners(piece);
    assert_material_triangles_do_not_overlap(piece);
    assert!(!piece.outer_boundary_loops.is_empty());
    assert!(
        !piece.render_earthwork_faces.is_empty(),
        "junction piece must emit terrain skirt faces from its canonical outer boundary"
    );
    assert!(!piece.road_surface_polygons.is_empty());
    assert!(!piece.curb_surface_polygons.is_empty());
    assert!(!piece.raised_step_face_polygons.is_empty());
    assert!(!piece.sidewalk_surface_polygons.is_empty());
    assert_top_mesh_centroids_inside_outer_boundary(piece);
    assert_top_surface_triangles_face_up(piece);
    assert_raised_step_faces_have_top_support(piece);
    assert_raised_step_faces_visible_from_lower_owner(piece);
    assert_top_raised_step_owner_boundaries_have_vertical_faces(piece);
    assert_outer_boundary_vertices_match_visible_top(piece);
    assert_node_top_covers_footprint(piece);
    assert_node_earthwork_faces_have_footprint_provenance(piece);
    assert_earthwork_faces_stay_outside_top_footprint(piece);
    piece
}

pub(in crate::simulation::network::surface::tests) fn assert_outer_boundary_vertices_match_visible_top(
    piece: &RoadSurfaceVisualNodePiece,
) {
    let top_polygons = piece
        .road_surface_polygons
        .iter()
        .chain(piece.curb_surface_polygons.iter())
        .chain(piece.sidewalk_surface_polygons.iter())
        .collect::<Vec<_>>();
    let top_vertices = visible_top_vertices(piece);
    assert!(
        !top_vertices.is_empty(),
        "node piece must emit visible top vertices before boundary matching can be checked"
    );
    for boundary_point in piece
        .outer_boundary_loops
        .iter()
        .flat_map(|polygon| polygon.points_world.iter())
    {
        let overlay_match_tolerance_m = SAMPLE_EPSILON_M * 2.0;
        let mut sampled_visible_top = false;
        let mut sampled_matching_height = false;
        for polygon in &top_polygons {
            for &triangle in &polygon.triangles_world {
                let Some((wa, wb, wc)) = RoadSurfaceSystem::triangle_barycentric_weights_xz(
                    triangle,
                    Vector2::new(boundary_point.x, boundary_point.z),
                ) else {
                    continue;
                };
                sampled_visible_top = true;
                let height = triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc;
                if (height - boundary_point.y).abs() <= overlay_match_tolerance_m {
                    sampled_matching_height = true;
                    break;
                }
            }
            if sampled_matching_height {
                break;
            }
        }
        if sampled_visible_top {
            assert!(
                sampled_matching_height,
                "node outer boundary must use a visible top-surface height at covered boundary points; boundary={boundary_point:?}"
            );
            continue;
        }

        let Some(closest) = top_vertices.iter().min_by(|a, b| {
            let da = Vector2::new(a.x - boundary_point.x, a.z - boundary_point.z).length_squared();
            let db = Vector2::new(b.x - boundary_point.x, b.z - boundary_point.z).length_squared();
            da.total_cmp(&db)
        }) else {
            panic!("node piece emitted no top vertices");
        };
        let xz_error =
            Vector2::new(closest.x - boundary_point.x, closest.z - boundary_point.z).length();
        if xz_error <= overlay_match_tolerance_m {
            let matching_height = top_vertices.iter().any(|candidate| {
                Vector2::new(
                    candidate.x - boundary_point.x,
                    candidate.z - boundary_point.z,
                )
                .length()
                    <= overlay_match_tolerance_m
                    && (candidate.y - boundary_point.y).abs() <= overlay_match_tolerance_m
            });
            assert!(
                matching_height,
                "node outer boundary must use the colocated visible top height; boundary={boundary_point:?} closest={closest:?} xz_error={xz_error:.4}"
            );
            continue;
        }

        if let Some(height) = top_polygons.iter().find_map(|polygon| {
            polygon.triangles_world.iter().find_map(|&triangle| {
                RoadSurfaceSystem::triangle_barycentric_weights_xz(
                    triangle,
                    Vector2::new(boundary_point.x, boundary_point.z),
                )
                .map(|(wa, wb, wc)| triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc)
            })
        }) {
            assert!(
                (height - boundary_point.y).abs() <= overlay_match_tolerance_m,
                "node outer boundary must use the visible top-surface height at covered boundary points; boundary={boundary_point:?} sampled_height={height:.4}"
            );
        } else {
            panic!(
                "node outer boundary vertex must be covered by visible top geometry; boundary={boundary_point:?} closest={closest:?} xz_error={xz_error:.4}"
            );
        }
    }
}

pub(in crate::simulation::network::surface::tests) fn assert_outer_boundary_vertices_use_visible_top_boundary_support(
    piece: &RoadSurfaceVisualNodePiece,
) {
    let top_polygons = piece
        .road_surface_polygons
        .iter()
        .chain(piece.curb_surface_polygons.iter())
        .chain(piece.sidewalk_surface_polygons.iter())
        .collect::<Vec<_>>();
    for boundary_point in piece
        .outer_boundary_loops
        .iter()
        .flat_map(|polygon| polygon.points_world.iter())
    {
        let Some(closest) = top_polygons
            .iter()
            .flat_map(|polygon| {
                polygon
                    .points_world
                    .windows(2)
                    .map(|segment| {
                        closest_point_on_segment_xz(*boundary_point, segment[0], segment[1])
                    })
                    .chain((!polygon.points_world.is_empty()).then(|| {
                        let last = *polygon.points_world.last().unwrap();
                        closest_point_on_segment_xz(*boundary_point, last, polygon.points_world[0])
                    }))
                    .chain(polygon.triangles_world.iter().flat_map(|triangle| {
                        (0..3).map(|index| {
                            closest_point_on_segment_xz(
                                *boundary_point,
                                triangle[index],
                                triangle[(index + 1) % 3],
                            )
                        })
                    }))
            })
            .min_by(|a, b| {
                let da =
                    Vector2::new(a.x - boundary_point.x, a.z - boundary_point.z).length_squared();
                let db =
                    Vector2::new(b.x - boundary_point.x, b.z - boundary_point.z).length_squared();
                da.total_cmp(&db).then(
                    (a.y - boundary_point.y)
                        .abs()
                        .total_cmp(&(b.y - boundary_point.y).abs()),
                )
            })
        else {
            panic!("node piece emitted no top boundary support");
        };
        let xz_error =
            Vector2::new(closest.x - boundary_point.x, closest.z - boundary_point.z).length();
        let y_error = (closest.y - boundary_point.y).abs();
        assert!(
            xz_error <= SAMPLE_EPSILON_M * 2.0 && y_error <= SAMPLE_EPSILON_M * 2.0,
            "node outer boundary vertices must lie on canonical visible top boundary support; boundary={boundary_point:?} closest={closest:?} xz_error={xz_error:.4} y_error={y_error:.4}"
        );
    }
}

pub(in crate::simulation::network::surface::tests) fn closest_point_on_segment_xz(
    point: Vector3,
    start: Vector3,
    end: Vector3,
) -> Vector3 {
    let segment = Vector2::new(end.x - start.x, end.z - start.z);
    let len_squared = segment.length_squared();
    if len_squared <= SAMPLE_EPSILON_M * SAMPLE_EPSILON_M {
        return start;
    }
    let to_point = Vector2::new(point.x - start.x, point.z - start.z);
    let t = (to_point.dot(segment) / len_squared).clamp(0.0, 1.0);
    start.lerp(end, t)
}

pub(in crate::simulation::network::surface::tests) fn visible_top_vertices(
    piece: &RoadSurfaceVisualNodePiece,
) -> Vec<Vector3> {
    piece
        .road_surface_polygons
        .iter()
        .chain(piece.curb_surface_polygons.iter())
        .chain(piece.sidewalk_surface_polygons.iter())
        .flat_map(|polygon| {
            polygon.points_world.iter().copied().chain(
                polygon
                    .triangles_world
                    .iter()
                    .flat_map(|triangle| triangle.iter().copied()),
            )
        })
        .collect()
}

pub(in crate::simulation::network::surface::tests) fn assert_material_top_supports_point(
    polygons: &[RoadSurfaceVisualPolygon],
    point: Vector3,
    label: &str,
) {
    assert!(
        polygons
            .iter()
            .any(|polygon| polygon_supports_top_point(polygon, point)),
        "material top surface must support anchor point; label={label} point={point:?}"
    );
}

pub(in crate::simulation::network::surface::tests) fn polygon_supports_top_point(
    polygon: &RoadSurfaceVisualPolygon,
    point: Vector3,
) -> bool {
    polygon_vertices_support_top_point(&polygon.points_world, point)
        || polygon_edges_support_top_point(&polygon.points_world, point)
        || polygon.triangles_world.iter().any(|triangle| {
            triangle
                .iter()
                .any(|&candidate| top_points_match(candidate, point))
                || triangle_edges_support_top_point(*triangle, point)
        })
}

pub(in crate::simulation::network::surface::tests) fn polygon_vertices_support_top_point(
    vertices: &[Vector3],
    point: Vector3,
) -> bool {
    vertices
        .iter()
        .copied()
        .any(|candidate| top_points_match(candidate, point))
}

pub(in crate::simulation::network::surface::tests) fn polygon_edges_support_top_point(
    vertices: &[Vector3],
    point: Vector3,
) -> bool {
    if vertices.len() < 2 {
        return false;
    }
    (0..vertices.len()).any(|index| {
        segment_supports_top_point(
            point,
            vertices[index],
            vertices[(index + 1) % vertices.len()],
        )
    })
}

pub(in crate::simulation::network::surface::tests) fn triangle_edges_support_top_point(
    triangle: [Vector3; 3],
    point: Vector3,
) -> bool {
    (0..3)
        .any(|index| segment_supports_top_point(point, triangle[index], triangle[(index + 1) % 3]))
}

pub(in crate::simulation::network::surface::tests) fn segment_supports_top_point(
    point: Vector3,
    start: Vector3,
    end: Vector3,
) -> bool {
    let segment = Vector2::new(end.x - start.x, end.z - start.z);
    let len_squared = segment.length_squared();
    if len_squared <= SAMPLE_EPSILON_M * SAMPLE_EPSILON_M {
        return false;
    }
    let to_point = Vector2::new(point.x - start.x, point.z - start.z);
    let t = (to_point.dot(segment) / len_squared).clamp(0.0, 1.0);
    let candidate = start.lerp(end, t);
    top_points_match(candidate, point)
}

pub(in crate::simulation::network::surface::tests) fn top_points_match(
    candidate: Vector3,
    point: Vector3,
) -> bool {
    test_xz_key(candidate) == test_xz_key(point) && (candidate.y - point.y).abs() <= 0.004
}
