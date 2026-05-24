//! Visible-top support matching helpers.

use super::*;

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

pub(in crate::simulation::network::surface::tests) fn assert_outer_boundary_vertices_match_visible_top(
    piece: &RoadSurfaceVisualNodePiece,
) {
    let (missing_area_m2, extra_area_m2, budget_m2, missing_shapes, extra_shapes) =
        node_top_coverage_details_m2(piece);
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
    for boundary_loop in &piece.outer_boundary_loops {
        for boundary_index in 0..boundary_loop.points_world.len() {
            let boundary_point = &boundary_loop.points_world[boundary_index];
            let previous_boundary = boundary_loop.points_world[if boundary_index == 0 {
                boundary_loop.points_world.len() - 1
            } else {
                boundary_index - 1
            }];
            let next_boundary =
                boundary_loop.points_world[(boundary_index + 1) % boundary_loop.points_world.len()];
            let local_area_m2 = RoadSurfaceSystem::signed_polygon_area_xz(&[
                previous_boundary,
                *boundary_point,
                next_boundary,
            ])
            .abs();
            let overlay_match_tolerance_m = SAMPLE_EPSILON_M * 2.0;
            let mut sampled_visible_top = false;
            let mut sampled_matching_height = false;
            let mut sampled_heights = Vec::new();
            for polygon in &top_polygons {
                for &triangle in &polygon.triangles_world {
                    let Some((wa, wb, wc)) = RoadSurfaceSystem::triangle_barycentric_weights_xz(
                        triangle,
                        Vector2::new(boundary_point.x, boundary_point.z),
                    ) else {
                        continue;
                    };
                    if wa < 0.0 || wb < 0.0 || wc < 0.0 {
                        continue;
                    }
                    sampled_visible_top = true;
                    let height = triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc;
                    sampled_heights.push(height);
                    if (height - boundary_point.y).abs() <= overlay_match_tolerance_m {
                        sampled_matching_height = true;
                        break;
                    }
                }
                if sampled_matching_height {
                    break;
                }
            }
            if sampled_visible_top && sampled_matching_height {
                continue;
            }

            if let Some(closest_boundary) =
                closest_visible_top_boundary_point(&top_polygons, *boundary_point)
            {
                let xz_error = Vector2::new(
                    closest_boundary.x - boundary_point.x,
                    closest_boundary.z - boundary_point.z,
                )
                .length();
                let y_error = (closest_boundary.y - boundary_point.y).abs();
                if xz_error <= overlay_match_tolerance_m && y_error <= overlay_match_tolerance_m {
                    continue;
                }
            }

            let Some(closest) = top_vertices.iter().min_by(|a, b| {
                let da =
                    Vector2::new(a.x - boundary_point.x, a.z - boundary_point.z).length_squared();
                let db =
                    Vector2::new(b.x - boundary_point.x, b.z - boundary_point.z).length_squared();
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
                    let (wa, wb, wc) = RoadSurfaceSystem::triangle_barycentric_weights_xz(
                        triangle,
                        Vector2::new(boundary_point.x, boundary_point.z),
                    )?;
                    (wa >= 0.0 && wb >= 0.0 && wc >= 0.0)
                        .then_some(triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc)
                })
            }) {
                assert!(
                    (height - boundary_point.y).abs() <= overlay_match_tolerance_m,
                    "node outer boundary must use the visible top-surface height at covered boundary points; boundary={boundary_point:?} sampled_height={height:.4} sampled_heights={sampled_heights:?}"
                );
            } else {
                panic!(
                    "node outer boundary vertex must be covered by visible top geometry; boundary={boundary_point:?} previous={previous_boundary:?} next={next_boundary:?} local_area_m2={local_area_m2:.8} closest={closest:?} xz_error={xz_error:.4} missing_area={missing_area_m2:.8} extra_area={extra_area_m2:.8} budget={budget_m2:.8} missing_shapes={missing_shapes:?} extra_shapes={extra_shapes:?}"
                );
            }
        }
    }
}

fn closest_visible_top_boundary_point(
    top_polygons: &[&RoadSurfaceVisualPolygon],
    boundary_point: Vector3,
) -> Option<Vector3> {
    top_polygons
        .iter()
        .flat_map(|polygon| {
            polygon
                .points_world
                .windows(2)
                .map(|segment| closest_point_on_segment_xz(boundary_point, segment[0], segment[1]))
                .chain((!polygon.points_world.is_empty()).then(|| {
                    let last = *polygon.points_world.last().unwrap();
                    closest_point_on_segment_xz(boundary_point, last, polygon.points_world[0])
                }))
                .chain(polygon.triangles_world.iter().flat_map(|triangle| {
                    (0..3).map(|index| {
                        closest_point_on_segment_xz(
                            boundary_point,
                            triangle[index],
                            triangle[(index + 1) % 3],
                        )
                    })
                }))
        })
        .min_by(|a, b| {
            let da = Vector2::new(a.x - boundary_point.x, a.z - boundary_point.z).length_squared();
            let db = Vector2::new(b.x - boundary_point.x, b.z - boundary_point.z).length_squared();
            da.total_cmp(&db).then(
                (a.y - boundary_point.y)
                    .abs()
                    .total_cmp(&(b.y - boundary_point.y).abs()),
            )
        })
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
