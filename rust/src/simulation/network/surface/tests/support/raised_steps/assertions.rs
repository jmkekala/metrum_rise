//! Raised-step vertical-face assertion helpers.

use super::*;

pub(in crate::simulation::network::surface::tests) fn assert_raised_step_face_lower_edge_covers(
    polygons: &[RoadSurfaceVisualPolygon],
    start: RoadVec3,
    end: RoadVec3,
    label: &str,
) {
    let start_key = test_xz_key(start);
    let end_key = test_xz_key(end);
    let expected_length = RoadVec2::new(end.x - start.x, end.z - start.z).length();
    let covered_length = polygons
        .iter()
        .filter_map(vertical_face_lower_edge_for_test)
        .filter(|edge| {
            test_xz_key_lies_on_segment_or_dust(edge[0], start, end, start_key, end_key)
                && test_xz_key_lies_on_segment_or_dust(edge[1], start, end, start_key, end_key)
        })
        .map(|edge| RoadVec2::new(edge[1].x - edge[0].x, edge[1].z - edge[0].z).length())
        .sum::<f64>();

    assert!(
        covered_length + 0.001 >= expected_length,
        "raised-step face lower edge must cover expected segment; label={label} start={start:?} end={end:?} covered={covered_length:.4} expected={expected_length:.4}"
    );
}

fn test_xz_key_lies_on_segment_or_dust(
    point: RoadVec3,
    start: RoadVec3,
    end: RoadVec3,
    start_key: (i64, i64),
    end_key: (i64, i64),
) -> bool {
    if test_xz_key_lies_on_segment(test_xz_key(point), start_key, end_key) {
        return true;
    }
    let segment = RoadVec2::new(end.x - start.x, end.z - start.z);
    let length_sq = segment.length_squared();
    if length_sq <= f64::EPSILON {
        return false;
    }
    let point_offset = RoadVec2::new(point.x - start.x, point.z - start.z);
    let t = point_offset.dot(segment) / length_sq;
    if !(-0.001..=1.001).contains(&t) {
        return false;
    }
    let projection = RoadVec2::new(start.x, start.z) + segment * t;
    let distance = (RoadVec2::new(point.x, point.z) - projection).length();
    distance <= 0.001
}

pub(in crate::simulation::network::surface::tests) fn assert_top_raised_step_owner_boundaries_have_vertical_faces(
    piece: &RoadSurfaceVisualNodePiece,
) {
    let top_edges = test_owned_top_boundary_edges(piece);
    let face_lower_keys = piece
        .raised_step_face_polygons
        .iter()
        .filter_map(vertical_face_lower_edge_for_test)
        .filter_map(|edge| TestRenderEdgeKey::normalized(edge[0], edge[1]).map(|key| key.xz()))
        .collect::<Vec<_>>();
    let mut edges_by_xz = BTreeMap::<TestRenderXzEdgeKey, Vec<TestTopBoundaryEdge>>::new();
    for edge in top_edges {
        edges_by_xz.entry(edge.xz_key).or_default().push(edge);
    }

    for edges in edges_by_xz.values() {
        for (left_index, left_edge) in edges.iter().enumerate() {
            for right_edge in edges.iter().skip(left_index + 1) {
                let (lower_edge, raised_edge) = if left_edge.avg_y_m <= right_edge.avg_y_m {
                    (*left_edge, *right_edge)
                } else {
                    (*right_edge, *left_edge)
                };
                if lower_edge.key == raised_edge.key
                    || lower_edge.avg_y_m >= raised_edge.avg_y_m
                    || !test_top_edges_form_raised_step(lower_edge, raised_edge)
                {
                    continue;
                }
                let matching_canonical_steps =
                    explicit_vertical_step_descriptions_for_xz_key(piece, lower_edge.xz_key);
                if matching_canonical_steps.is_empty() {
                    continue;
                }
                assert!(
                    face_lower_keys
                        .iter()
                        .copied()
                        .any(|face_key| face_key.contains(lower_edge.xz_key)),
                    "surviving raised-step owner boundary must emit an explicit vertical face; kind={:?} xz_key={:?} lower_owner={:?}[{}] lower={:?}->{:?} raised_owner={:?}[{}] raised={:?}->{:?} matching_canonical_steps={:?} face_lower_keys={:?}",
                    piece.kind,
                    lower_edge.xz_key,
                    lower_edge.kind,
                    lower_edge.owner_index,
                    lower_edge.start,
                    lower_edge.end,
                    raised_edge.kind,
                    raised_edge.owner_index,
                    raised_edge.start,
                    raised_edge.end,
                    matching_canonical_steps,
                    face_lower_keys
                );
            }
        }
    }
}

pub(in crate::simulation::network::surface::tests) fn assert_no_unfaced_cross_material_height_boundaries(
    piece: &RoadSurfaceVisualNodePiece,
) {
    let top_edges = test_owned_top_boundary_edges(piece);
    let face_lower_edges = piece
        .raised_step_face_polygons
        .iter()
        .filter_map(vertical_face_lower_edge_for_test)
        .collect::<Vec<_>>();

    for (left_index, left_edge) in top_edges.iter().enumerate() {
        for right_edge in top_edges.iter().skip(left_index + 1) {
            if left_edge.kind == right_edge.kind {
                continue;
            }
            let (lower_edge, raised_edge) = if left_edge.avg_y_m <= right_edge.avg_y_m {
                (*left_edge, *right_edge)
            } else {
                (*right_edge, *left_edge)
            };
            if lower_edge.avg_y_m + f64::from(SAMPLE_EPSILON_M) >= raised_edge.avg_y_m {
                continue;
            }
            let Some(overlap) = test_top_boundary_overlap_interval(lower_edge, raised_edge) else {
                continue;
            };
            assert!(
                test_top_edges_form_raised_step(lower_edge, raised_edge),
                "cross-material top boundary has a height jump without an adjacent raised-step owner pair; kind={:?} overlap={:?} lower_owner={:?}[{}] lower={:?}->{:?} raised_owner={:?}[{}] raised={:?}->{:?}",
                piece.kind,
                overlap,
                lower_edge.kind,
                lower_edge.owner_index,
                lower_edge.start,
                lower_edge.end,
                raised_edge.kind,
                raised_edge.owner_index,
                raised_edge.start,
                raised_edge.end
            );
            assert!(
                test_raised_step_faces_cover_overlap_interval(
                    lower_edge,
                    overlap,
                    &face_lower_edges
                ),
                "cross-material raised top boundary must emit explicit vertical face coverage; kind={:?} overlap={:?} lower_owner={:?}[{}] lower={:?}->{:?} raised_owner={:?}[{}] raised={:?}->{:?} matching_canonical_steps={:?} face_lower_edges={:?}",
                piece.kind,
                overlap,
                lower_edge.kind,
                lower_edge.owner_index,
                lower_edge.start,
                lower_edge.end,
                raised_edge.kind,
                raised_edge.owner_index,
                raised_edge.start,
                raised_edge.end,
                explicit_vertical_step_descriptions_for_xz_key(piece, lower_edge.xz_key),
                face_lower_edges
            );
        }
    }
}

pub(in crate::simulation::network::surface::tests) fn assert_surface_no_unfaced_cross_material_height_boundaries(
    surface: &RoadSurfaceSystem,
) {
    let mut top_edges = Vec::new();
    let mut face_lower_edges = Vec::new();
    for span_piece in surface.compiled_visual_span_pieces().values() {
        for region in &span_piece.span_owned_regions {
            top_edges.extend(test_polygon_top_boundary_edges(
                region.owner.kind,
                region.owner.source_band_index,
                &region.polygon,
            ));
        }
        face_lower_edges.extend(
            span_piece
                .raised_step_face_polygons
                .iter()
                .filter_map(vertical_face_lower_edge_for_test),
        );
    }
    for node_piece in surface.compiled_visual_node_pieces().values() {
        top_edges.extend(test_owned_top_boundary_edges(node_piece));
        face_lower_edges.extend(
            node_piece
                .raised_step_face_polygons
                .iter()
                .filter_map(vertical_face_lower_edge_for_test),
        );
    }
    for (left_index, left_edge) in top_edges.iter().enumerate() {
        for right_edge in top_edges.iter().skip(left_index + 1) {
            if left_edge.kind == right_edge.kind {
                continue;
            }
            let (lower_edge, raised_edge) = if left_edge.avg_y_m <= right_edge.avg_y_m {
                (*left_edge, *right_edge)
            } else {
                (*right_edge, *left_edge)
            };
            if lower_edge.avg_y_m + f64::from(SAMPLE_EPSILON_M) >= raised_edge.avg_y_m {
                continue;
            }
            let Some(overlap) = test_top_boundary_overlap_interval(lower_edge, raised_edge) else {
                continue;
            };
            if !test_top_edges_form_raised_step(lower_edge, raised_edge) {
                continue;
            }
            assert!(
                test_raised_step_faces_cover_overlap_interval(
                    lower_edge,
                    overlap,
                    &face_lower_edges
                ),
                "surface cross-material raised top boundary must emit explicit vertical face coverage; overlap={:?} lower_owner={:?}[{}] lower={:?}->{:?} raised_owner={:?}[{}] raised={:?}->{:?} face_lower_edges={:?}",
                overlap,
                lower_edge.kind,
                lower_edge.owner_index,
                lower_edge.start,
                lower_edge.end,
                raised_edge.kind,
                raised_edge.owner_index,
                raised_edge.start,
                raised_edge.end,
                face_lower_edges
            );
        }
    }
}

fn test_top_boundary_overlap_interval(
    lower_edge: TestTopBoundaryEdge,
    raised_edge: TestTopBoundaryEdge,
) -> Option<(i128, i128, i128)> {
    if !test_xz_segments_overlap_with_length(
        (lower_edge.xz_key.start.x_key, lower_edge.xz_key.start.z_key),
        (lower_edge.xz_key.end.x_key, lower_edge.xz_key.end.z_key),
        (
            raised_edge.xz_key.start.x_key,
            raised_edge.xz_key.start.z_key,
        ),
        (raised_edge.xz_key.end.x_key, raised_edge.xz_key.end.z_key),
    ) {
        return None;
    }
    let lower_start = TestRenderVertexKey::from_point(lower_edge.start);
    let lower_end = TestRenderVertexKey::from_point(lower_edge.end);
    let raised_start = TestRenderVertexKey::from_point(raised_edge.start);
    let raised_end = TestRenderVertexKey::from_point(raised_edge.end);
    let (raised_start_numerator, denominator) =
        test_boundary_segment_parameter_xz(raised_start, lower_start, lower_end)?;
    let (raised_end_numerator, raised_denominator) =
        test_boundary_segment_parameter_xz(raised_end, lower_start, lower_end)?;
    if denominator != raised_denominator || denominator <= 0 {
        return None;
    }
    let overlap_start = raised_start_numerator.min(raised_end_numerator).max(0);
    let overlap_end = raised_start_numerator
        .max(raised_end_numerator)
        .min(denominator);
    (overlap_end > overlap_start).then_some((overlap_start, overlap_end, denominator))
}

fn test_raised_step_faces_cover_overlap_interval(
    lower_edge: TestTopBoundaryEdge,
    overlap: (i128, i128, i128),
    face_lower_edges: &[[RoadVec3; 2]],
) -> bool {
    let (required_start, required_end, denominator) = overlap;
    let lower_start = TestRenderVertexKey::from_point(lower_edge.start);
    let lower_end = TestRenderVertexKey::from_point(lower_edge.end);
    let mut intervals = Vec::<(i128, i128)>::new();
    for face_edge in face_lower_edges {
        let face_start = TestRenderVertexKey::from_point(face_edge[0]);
        let face_end = TestRenderVertexKey::from_point(face_edge[1]);
        let Some((face_start_numerator, face_denominator)) =
            test_boundary_segment_parameter_xz(face_start, lower_start, lower_end)
        else {
            continue;
        };
        let Some((face_end_numerator, face_end_denominator)) =
            test_boundary_segment_parameter_xz(face_end, lower_start, lower_end)
        else {
            continue;
        };
        if face_denominator != denominator || face_end_denominator != denominator {
            continue;
        }
        let start = face_start_numerator
            .min(face_end_numerator)
            .max(required_start);
        let end = face_start_numerator
            .max(face_end_numerator)
            .min(required_end);
        if end > start {
            intervals.push((start, end));
        }
    }
    intervals.sort_unstable();
    let mut cursor = required_start;
    let tolerance = (denominator / 1_000_000).max(1);
    for (start, end) in intervals {
        if start > cursor + tolerance {
            return false;
        }
        cursor = cursor.max(end);
        if cursor + tolerance >= required_end {
            return true;
        }
    }
    cursor + tolerance >= required_end
}

pub(in crate::simulation::network::surface::tests) fn assert_canonical_explicit_vertical_steps_have_faces(
    piece: &RoadSurfaceVisualNodePiece,
) {
    let top_edges = test_owned_top_boundary_edges(piece);
    let mut top_edges_by_xz = BTreeMap::<TestRenderXzEdgeKey, Vec<TestTopBoundaryEdge>>::new();
    for edge in top_edges {
        top_edges_by_xz.entry(edge.xz_key).or_default().push(edge);
    }
    let face_source_segments = piece
        .raised_step_face_sources
        .iter()
        .map(|source| source.segment())
        .collect::<BTreeSet<_>>();

    for (step_index, segment) in piece.explicit_vertical_step_segments.iter().enumerate() {
        let owner = segment.owner();
        let opposite_owner = segment.opposite_owner();
        let owner_pair_requires_face =
            test_owners_form_raised_step(owner.kind(), opposite_owner.kind());
        if !owner_pair_requires_face {
            continue;
        }
        if explicit_vertical_step_segment_len_squared_m2(*segment)
            <= f64::from(SAMPLE_EPSILON_M) * f64::from(SAMPLE_EPSILON_M)
        {
            continue;
        }
        if !explicit_vertical_step_has_visible_top_support(*segment, &top_edges_by_xz) {
            continue;
        }

        assert!(
            face_source_segments.contains(segment),
            "canonical explicit vertical step must be consumed by a rendered vertical face; kind={:?} step_index={} segment={:?}",
            piece.kind,
            step_index,
            segment
        );
    }
}

pub(in crate::simulation::network::surface::tests) fn assert_raised_step_faces_visible_from_lower_owner(
    piece: &RoadSurfaceVisualNodePiece,
) {
    let top_edges = test_owned_top_boundary_edges(piece);
    for (face, source) in piece
        .raised_step_face_polygons
        .iter()
        .zip(piece.raised_step_face_sources.iter())
    {
        let Some(lower_owner) = test_lower_owner_from_vertical_face_source(*source) else {
            continue;
        };
        let Some(visible_direction) = vertical_face_visible_direction_for_test(face) else {
            continue;
        };
        let visible_direction =
            RoadVec3::new(visible_direction.x, 0.0, visible_direction.z).normalize();
        let Some(lower_edge) = vertical_face_owner_edge_for_test(face, &top_edges, lower_owner)
        else {
            continue;
        };
        let midpoint = (lower_edge[0] + lower_edge[1]) * 0.5;
        let mut best_dot: Option<f64> = None;

        for region in piece.owned_regions.iter().filter(|region| {
            region.kind == lower_owner.kind() && region.owner_index == lower_owner.owner_index()
        }) {
            let Some(centroid) = polygon_centroid_for_test(&region.polygon) else {
                continue;
            };
            let owner_direction =
                RoadVec3::new(centroid.x - midpoint.x, 0.0, centroid.z - midpoint.z);
            if owner_direction.length_squared() <= 1e-8 {
                continue;
            }
            let dot = visible_direction.dot(owner_direction.normalize());
            best_dot = Some(best_dot.map_or(dot, |current| current.max(dot)));
        }

        if let Some(dot) = best_dot {
            assert!(
                dot > 0.0,
                "raised-step face must be visible from its lower owner; kind={:?} face={:?} visible_direction={visible_direction:?} dot={dot:.6}",
                piece.kind,
                face.points_world
            );
        }
    }
}

pub(in crate::simulation::network::surface::tests) fn assert_raised_step_faces_have_top_support(
    piece: &RoadSurfaceVisualNodePiece,
) {
    let top_edges = test_owned_top_boundary_edges(piece);
    for (face, source) in piece
        .raised_step_face_polygons
        .iter()
        .zip(piece.raised_step_face_sources.iter())
    {
        let Some(lower_edge) = vertical_face_lower_edge_for_test(face) else {
            panic!(
                "raised-step face must expose a non-degenerate lower edge; face={:?}",
                face.points_world
            );
        };
        let Some(upper_edge) = vertical_face_upper_edge_for_test(face) else {
            panic!(
                "raised-step face must expose a non-degenerate upper edge; face={:?}",
                face.points_world
            );
        };
        let Some((lower_owner, raised_owner)) =
            test_lower_and_raised_owners_from_vertical_face_source(*source)
        else {
            panic!(
                "raised-step face must carry explicit raised-step provenance; source={source:?}"
            );
        };
        let lower_matches = top_edges
            .iter()
            .filter(|edge| {
                edge.kind == lower_owner.kind()
                    && edge.owner_index == lower_owner.owner_index()
                    && test_boundary_edge_contains_edge_at_height(
                        [edge.start, edge.end],
                        lower_edge,
                    )
            })
            .collect::<Vec<_>>();
        let upper_matches = top_edges
            .iter()
            .filter(|edge| {
                edge.kind == raised_owner.kind()
                    && edge.owner_index == raised_owner.owner_index()
                    && test_boundary_edge_contains_edge_at_height(
                        [edge.start, edge.end],
                        upper_edge,
                    )
            })
            .collect::<Vec<_>>();
        assert!(
            !lower_matches.is_empty(),
            "raised-step face lower edge must be backed by a top owner; lower_edge={lower_edge:?} face={:?}",
            face.points_world
        );
        assert!(
            !upper_matches.is_empty(),
            "raised-step face upper edge must be backed by a top owner; upper_edge={upper_edge:?} face={:?}",
            face.points_world
        );
    }
}
