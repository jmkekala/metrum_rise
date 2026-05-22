//! Raised-step vertical-face assertion helpers.

use super::*;

pub(in crate::simulation::network::surface::tests) fn assert_raised_step_face_lower_edge_covers(
    polygons: &[RoadSurfaceVisualPolygon],
    start: Vector3,
    end: Vector3,
    label: &str,
) {
    let start_key = test_xz_key(start);
    let end_key = test_xz_key(end);
    let expected_length = Vector2::new(end.x - start.x, end.z - start.z).length();
    let covered_length = polygons
        .iter()
        .filter_map(vertical_face_lower_edge_for_test)
        .filter(|edge| {
            test_xz_key_lies_on_segment(test_xz_key(edge[0]), start_key, end_key)
                && test_xz_key_lies_on_segment(test_xz_key(edge[1]), start_key, end_key)
        })
        .map(|edge| Vector2::new(edge[1].x - edge[0].x, edge[1].z - edge[0].z).length())
        .sum::<f32>();

    assert!(
        covered_length + 0.001 >= expected_length,
        "raised-step face lower edge must cover expected segment; label={label} start={start:?} end={end:?} covered={covered_length:.4} expected={expected_length:.4}"
    );
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
            <= SAMPLE_EPSILON_M * SAMPLE_EPSILON_M
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
            Vector3::new(visible_direction.x, 0.0, visible_direction.z).normalized();
        let Some(lower_edge) = vertical_face_owner_edge_for_test(face, &top_edges, lower_owner)
        else {
            continue;
        };
        let midpoint = (lower_edge[0] + lower_edge[1]) * 0.5;
        let mut best_dot: Option<f32> = None;

        for region in piece.owned_regions.iter().filter(|region| {
            region.kind == lower_owner.kind() && region.owner_index == lower_owner.owner_index()
        }) {
            let Some(centroid) = polygon_centroid_for_test(&region.polygon) else {
                continue;
            };
            let owner_direction =
                Vector3::new(centroid.x - midpoint.x, 0.0, centroid.z - midpoint.z);
            if owner_direction.length_squared() <= 1e-8 {
                continue;
            }
            let dot = visible_direction.dot(owner_direction.normalized());
            best_dot = Some(best_dot.map_or(dot, |current| current.max(dot)));
        }

        if let Some(dot) = best_dot {
            assert!(
                dot > -0.25,
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
    for face in &piece.raised_step_face_polygons {
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
        let lower_matches = piece
            .owned_regions
            .iter()
            .filter(|region| {
                polygon_boundary_overlaps_edge_at_height_for_test(&region.polygon, lower_edge)
            })
            .collect::<Vec<_>>();
        let upper_matches = piece
            .owned_regions
            .iter()
            .filter(|region| {
                polygon_boundary_overlaps_edge_at_height_for_test(&region.polygon, upper_edge)
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
        assert!(
            lower_matches.iter().any(|lower_match| {
                upper_matches.iter().any(|upper_match| {
                    test_owners_form_raised_step(lower_match.kind, upper_match.kind)
                })
            }),
            "raised-step face support edges must belong to an explicit raised-step owner pair; lower_edge={lower_edge:?} upper_edge={upper_edge:?} face={:?}",
            face.points_world
        );
    }
}
