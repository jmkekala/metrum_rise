//! Raised-step vertical-face helpers for road-surface tests.

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

#[derive(Clone, Copy, Debug)]
pub(in crate::simulation::network::surface::tests) struct TestTopBoundaryEdge {
    pub(in crate::simulation::network::surface::tests) kind: RoadSurfaceBandKind,
    pub(in crate::simulation::network::surface::tests) owner_index: usize,
    pub(in crate::simulation::network::surface::tests) start: Vector3,
    pub(in crate::simulation::network::surface::tests) end: Vector3,
    pub(in crate::simulation::network::surface::tests) key: TestRenderEdgeKey,
    pub(in crate::simulation::network::surface::tests) xz_key: TestRenderXzEdgeKey,
    pub(in crate::simulation::network::surface::tests) avg_y_m: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(in crate::simulation::network::surface::tests) struct TestRenderVertexKey {
    pub(in crate::simulation::network::surface::tests) x_key: i64,
    pub(in crate::simulation::network::surface::tests) y_mm: i64,
    pub(in crate::simulation::network::surface::tests) z_key: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(in crate::simulation::network::surface::tests) struct TestRenderEdgeKey {
    pub(in crate::simulation::network::surface::tests) start: TestRenderVertexKey,
    pub(in crate::simulation::network::surface::tests) end: TestRenderVertexKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(in crate::simulation::network::surface::tests) struct TestRenderXzVertexKey {
    pub(in crate::simulation::network::surface::tests) x_key: i64,
    pub(in crate::simulation::network::surface::tests) z_key: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(in crate::simulation::network::surface::tests) struct TestRenderXzEdgeKey {
    pub(in crate::simulation::network::surface::tests) start: TestRenderXzVertexKey,
    pub(in crate::simulation::network::surface::tests) end: TestRenderXzVertexKey,
}

impl TestRenderVertexKey {
    fn from_point(point: Vector3) -> Self {
        let (x_key, z_key) = test_xz_key(point);
        Self {
            x_key,
            y_mm: (point.y * 1000.0).round() as i64,
            z_key,
        }
    }

    fn xz(self) -> TestRenderXzVertexKey {
        TestRenderXzVertexKey {
            x_key: self.x_key,
            z_key: self.z_key,
        }
    }
}

impl TestRenderXzVertexKey {
    fn from_arrangement_key(key: super::arrangement::NodeArrangementKey) -> Self {
        Self {
            x_key: key.x_key(),
            z_key: key.z_key(),
        }
    }
}

impl TestRenderEdgeKey {
    fn normalized(start: Vector3, end: Vector3) -> Option<Self> {
        let start = TestRenderVertexKey::from_point(start);
        let end = TestRenderVertexKey::from_point(end);
        if start == end {
            return None;
        }
        Some(if start <= end {
            Self { start, end }
        } else {
            Self {
                start: end,
                end: start,
            }
        })
    }

    fn xz(self) -> TestRenderXzEdgeKey {
        let start = self.start.xz();
        let end = self.end.xz();
        if start <= end {
            TestRenderXzEdgeKey { start, end }
        } else {
            TestRenderXzEdgeKey {
                start: end,
                end: start,
            }
        }
    }
}

impl TestRenderXzEdgeKey {
    fn normalized_from_arrangement_keys(
        start: super::arrangement::NodeArrangementKey,
        end: super::arrangement::NodeArrangementKey,
    ) -> Option<Self> {
        let start = TestRenderXzVertexKey::from_arrangement_key(start);
        let end = TestRenderXzVertexKey::from_arrangement_key(end);
        if start == end {
            return None;
        }
        Some(if start <= end {
            Self { start, end }
        } else {
            Self {
                start: end,
                end: start,
            }
        })
    }

    fn contains(self, edge: Self) -> bool {
        test_render_xz_vertex_key_lies_on_segment(edge.start, self.start, self.end)
            && test_render_xz_vertex_key_lies_on_segment(edge.end, self.start, self.end)
    }
}

pub(in crate::simulation::network::surface::tests) fn test_render_xz_vertex_key_lies_on_segment(
    point: TestRenderXzVertexKey,
    start: TestRenderXzVertexKey,
    end: TestRenderXzVertexKey,
) -> bool {
    let dx = i128::from(end.x_key - start.x_key);
    let dz = i128::from(end.z_key - start.z_key);
    let px = i128::from(point.x_key - start.x_key);
    let pz = i128::from(point.z_key - start.z_key);
    dx * pz - dz * px == 0
        && point.x_key >= start.x_key.min(end.x_key)
        && point.x_key <= start.x_key.max(end.x_key)
        && point.z_key >= start.z_key.min(end.z_key)
        && point.z_key <= start.z_key.max(end.z_key)
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

pub(in crate::simulation::network::surface::tests) fn explicit_vertical_step_descriptions_for_xz_key(
    piece: &RoadSurfaceVisualNodePiece,
    xz_key: TestRenderXzEdgeKey,
) -> Vec<String> {
    piece
        .explicit_vertical_step_segments
        .iter()
        .enumerate()
        .filter_map(|(step_index, segment)| {
            TestRenderXzEdgeKey::normalized_from_arrangement_keys(segment.start(), segment.end())
                .filter(|step_key| step_key.contains(xz_key))
                .map(|_| {
                    format!(
                        "#{step_index} {:?}<->{:?} {:?}->{:?}",
                        segment.owner(),
                        segment.opposite_owner(),
                        segment.start(),
                        segment.end()
                    )
                })
        })
        .collect()
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

pub(in crate::simulation::network::surface::tests) fn explicit_vertical_step_has_visible_top_support(
    segment: super::arrangement::NodeExplicitVerticalStepSegment,
    top_edges_by_xz: &BTreeMap<TestRenderXzEdgeKey, Vec<TestTopBoundaryEdge>>,
) -> bool {
    let Some(xz_key) =
        TestRenderXzEdgeKey::normalized_from_arrangement_keys(segment.start(), segment.end())
    else {
        return false;
    };
    let Some(edges) = top_edges_by_xz.get(&xz_key) else {
        return false;
    };
    edges.iter().any(|lower_edge| {
        edges.iter().any(|raised_edge| {
            lower_edge.avg_y_m < raised_edge.avg_y_m
                && test_top_edges_form_raised_step(*lower_edge, *raised_edge)
        })
    })
}

pub(in crate::simulation::network::surface::tests) fn test_owners_form_raised_step(
    lower_kind: RoadSurfaceBandKind,
    raised_kind: RoadSurfaceBandKind,
) -> bool {
    ordered_raised_step_kinds(lower_kind, raised_kind) == Some((lower_kind, raised_kind))
}

pub(in crate::simulation::network::surface::tests) fn test_top_edges_form_raised_step(
    lower_edge: TestTopBoundaryEdge,
    raised_edge: TestTopBoundaryEdge,
) -> bool {
    test_owners_form_raised_step(lower_edge.kind, raised_edge.kind)
}

pub(in crate::simulation::network::surface::tests) fn explicit_vertical_step_segment_len_squared_m2(
    segment: super::arrangement::NodeExplicitVerticalStepSegment,
) -> f32 {
    let dx = (segment.end().x_key() - segment.start().x_key()) as f64
        / super::backend::ROAD_OVERLAY_COORDINATE_SCALE;
    let dz = (segment.end().z_key() - segment.start().z_key()) as f64
        / super::backend::ROAD_OVERLAY_COORDINATE_SCALE;
    (dx * dx + dz * dz) as f32
}

pub(in crate::simulation::network::surface::tests) fn test_owned_top_boundary_edges(
    piece: &RoadSurfaceVisualNodePiece,
) -> Vec<TestTopBoundaryEdge> {
    let mut boundary_edges = Vec::new();
    for region in &piece.owned_regions {
        let mut edge_counts = BTreeMap::<TestRenderEdgeKey, (usize, Vector3, Vector3)>::new();
        if region.polygon.triangles_world.is_empty() {
            let points = &region.polygon.points_world;
            if points.len() >= 2 {
                for index in 0..points.len() {
                    if let Some(key) = TestRenderEdgeKey::normalized(
                        points[index],
                        points[(index + 1) % points.len()],
                    ) {
                        edge_counts
                            .entry(key)
                            .and_modify(|entry| entry.0 += 1)
                            .or_insert((1, points[index], points[(index + 1) % points.len()]));
                    }
                }
            }
        } else {
            for triangle in &region.polygon.triangles_world {
                for edge_index in 0..3 {
                    if let Some(key) = TestRenderEdgeKey::normalized(
                        triangle[edge_index],
                        triangle[(edge_index + 1) % 3],
                    ) {
                        edge_counts
                            .entry(key)
                            .and_modify(|entry| entry.0 += 1)
                            .or_insert((1, triangle[edge_index], triangle[(edge_index + 1) % 3]));
                    }
                }
            }
        }
        for (key, (count, start, end)) in edge_counts {
            if count == 1 {
                boundary_edges.push(TestTopBoundaryEdge {
                    kind: region.kind,
                    owner_index: region.owner_index,
                    start,
                    end,
                    key,
                    xz_key: key.xz(),
                    avg_y_m: (start.y + end.y) * 0.5,
                });
            }
        }
    }
    boundary_edges
}

pub(in crate::simulation::network::surface::tests) fn vertical_face_lower_edge_for_test(
    polygon: &RoadSurfaceVisualPolygon,
) -> Option<[Vector3; 2]> {
    let [first_edge, second_edge] = vertical_face_side_edges_for_test(polygon)?;
    let first_avg_y = (first_edge[0].y + first_edge[1].y) * 0.5;
    let second_avg_y = (second_edge[0].y + second_edge[1].y) * 0.5;
    Some(if first_avg_y <= second_avg_y {
        first_edge
    } else {
        second_edge
    })
}

pub(in crate::simulation::network::surface::tests) fn vertical_face_side_edges_for_test(
    polygon: &RoadSurfaceVisualPolygon,
) -> Option<[[Vector3; 2]; 2]> {
    let [a, b, c, d] = polygon.points_world.as_slice() else {
        return None;
    };
    Some([[*a, *d], [*b, *c]])
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

pub(in crate::simulation::network::surface::tests) fn test_lower_owner_from_vertical_face_source(
    source: super::RoadSurfaceVerticalFaceSource,
) -> Option<NodeBandOwner> {
    let segment = source.segment();
    let owner = segment.owner();
    let opposite_owner = segment.opposite_owner();
    let (lower_kind, _) = ordered_raised_step_kinds(owner.kind(), opposite_owner.kind())?;
    Some(if owner.kind() == lower_kind {
        owner
    } else {
        opposite_owner
    })
}

pub(in crate::simulation::network::surface::tests) fn vertical_face_owner_edge_for_test(
    face: &RoadSurfaceVisualPolygon,
    top_edges: &[TestTopBoundaryEdge],
    owner: NodeBandOwner,
) -> Option<[Vector3; 2]> {
    let [first_edge, second_edge] = vertical_face_side_edges_for_test(face)?;
    [first_edge, second_edge].into_iter().find(|edge| {
        let Some(edge_key) = TestRenderEdgeKey::normalized(edge[0], edge[1]).map(|key| key.xz())
        else {
            return false;
        };
        top_edges.iter().any(|top_edge| {
            top_edge.xz_key == edge_key
                && top_edge.kind == owner.kind()
                && top_edge.owner_index == owner.owner_index()
        })
    })
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

pub(in crate::simulation::network::surface::tests) fn vertical_face_visible_direction_for_test(
    polygon: &RoadSurfaceVisualPolygon,
) -> Option<Vector3> {
    let [upper_start, lower_start, lower_end, _upper_end] = polygon.points_world.as_slice() else {
        return None;
    };
    let normal = (*lower_start - *upper_start).cross(*lower_end - *upper_start);
    (normal.length_squared() > 1e-8).then(|| -normal.normalized())
}

pub(in crate::simulation::network::surface::tests) fn vertical_face_upper_edge_for_test(
    polygon: &RoadSurfaceVisualPolygon,
) -> Option<[Vector3; 2]> {
    let [a, b, c, d] = polygon.points_world.as_slice() else {
        return None;
    };
    let first_edge = [*a, *d];
    let second_edge = [*b, *c];
    let first_avg_y = (first_edge[0].y + first_edge[1].y) * 0.5;
    let second_avg_y = (second_edge[0].y + second_edge[1].y) * 0.5;
    Some(if first_avg_y >= second_avg_y {
        first_edge
    } else {
        second_edge
    })
}

pub(in crate::simulation::network::surface::tests) fn polygon_boundary_overlaps_edge_at_height_for_test(
    polygon: &RoadSurfaceVisualPolygon,
    edge: [Vector3; 2],
) -> bool {
    if !polygon.triangles_world.is_empty() {
        let mut triangle_edges = BTreeMap::<TestRenderEdgeKey, (usize, [Vector3; 2])>::new();
        for triangle in &polygon.triangles_world {
            for edge_index in 0..3 {
                let start = triangle[edge_index];
                let end = triangle[(edge_index + 1) % 3];
                let Some(key) = TestRenderEdgeKey::normalized(start, end) else {
                    continue;
                };
                triangle_edges
                    .entry(key)
                    .and_modify(|entry| entry.0 += 1)
                    .or_insert((1, [start, end]));
            }
        }
        return triangle_edges
            .into_values()
            .filter_map(|(count, boundary_edge)| (count == 1).then_some(boundary_edge))
            .any(|boundary_edge| test_boundary_edge_contains_edge_at_height(boundary_edge, edge));
    }

    let points = &polygon.points_world;
    if points.len() < 2 {
        return false;
    }
    (0..points.len()).any(|index| {
        let start = points[index];
        let end = points[(index + 1) % points.len()];
        test_boundary_edge_contains_edge_at_height([start, end], edge)
    })
}

pub(in crate::simulation::network::surface::tests) fn test_boundary_edge_contains_edge_at_height(
    boundary_edge: [Vector3; 2],
    edge: [Vector3; 2],
) -> bool {
    let boundary_start = TestRenderVertexKey::from_point(boundary_edge[0]);
    let boundary_end = TestRenderVertexKey::from_point(boundary_edge[1]);
    let edge_start = TestRenderVertexKey::from_point(edge[0]);
    let edge_end = TestRenderVertexKey::from_point(edge[1]);
    if !test_xz_segments_overlap_with_length(
        (boundary_start.x_key, boundary_start.z_key),
        (boundary_end.x_key, boundary_end.z_key),
        (edge_start.x_key, edge_start.z_key),
        (edge_end.x_key, edge_end.z_key),
    ) {
        return false;
    }
    let Some((start_numerator, start_denominator)) =
        test_boundary_segment_parameter_xz(edge_start, boundary_start, boundary_end)
    else {
        return false;
    };
    let Some((end_numerator, end_denominator)) =
        test_boundary_segment_parameter_xz(edge_end, boundary_start, boundary_end)
    else {
        return false;
    };
    if start_numerator < 0
        || start_numerator > start_denominator
        || end_numerator < 0
        || end_numerator > end_denominator
    {
        return false;
    }
    (test_interpolated_height_mm(
        boundary_start,
        boundary_end,
        start_numerator,
        start_denominator,
    ) - edge_start.y_mm)
        .abs()
        <= 1
        && (test_interpolated_height_mm(
            boundary_start,
            boundary_end,
            end_numerator,
            end_denominator,
        ) - edge_end.y_mm)
            .abs()
            <= 1
}

pub(in crate::simulation::network::surface::tests) fn test_boundary_segment_parameter_xz(
    point: TestRenderVertexKey,
    start: TestRenderVertexKey,
    end: TestRenderVertexKey,
) -> Option<(i128, i128)> {
    let dx = end.x_key - start.x_key;
    let dz = end.z_key - start.z_key;
    let px = point.x_key - start.x_key;
    let pz = point.z_key - start.z_key;
    let length_squared = i128::from(dx) * i128::from(dx) + i128::from(dz) * i128::from(dz);
    if length_squared == 0 || i128::from(dx) * i128::from(pz) - i128::from(dz) * i128::from(px) != 0
    {
        return None;
    }
    Some((
        i128::from(px) * i128::from(dx) + i128::from(pz) * i128::from(dz),
        length_squared,
    ))
}

pub(in crate::simulation::network::surface::tests) fn test_interpolated_height_mm(
    start: TestRenderVertexKey,
    end: TestRenderVertexKey,
    numerator: i128,
    denominator: i128,
) -> i64 {
    let value =
        i128::from(start.y_mm) * denominator + i128::from(end.y_mm - start.y_mm) * numerator;
    if value >= 0 {
        ((value + denominator / 2) / denominator) as i64
    } else {
        -(((-value + denominator / 2) / denominator) as i64)
    }
}

pub(in crate::simulation::network::surface::tests) fn test_xz_segments_overlap_with_length(
    a_start: (i64, i64),
    a_end: (i64, i64),
    b_start: (i64, i64),
    b_end: (i64, i64),
) -> bool {
    if a_start == a_end || b_start == b_end {
        return false;
    }
    let a_dx = i128::from(a_end.0 - a_start.0);
    let a_dz = i128::from(a_end.1 - a_start.1);
    let b_dx = i128::from(b_end.0 - b_start.0);
    let b_dz = i128::from(b_end.1 - b_start.1);
    if a_dx * b_dz - a_dz * b_dx != 0 {
        return false;
    }
    if !test_xz_key_lies_on_segment(a_start, b_start, b_end)
        && !test_xz_key_lies_on_segment(a_end, b_start, b_end)
        && !test_xz_key_lies_on_segment(b_start, a_start, a_end)
        && !test_xz_key_lies_on_segment(b_end, a_start, a_end)
    {
        return false;
    }
    let use_x = (a_end.0 - a_start.0).abs() >= (a_end.1 - a_start.1).abs();
    let coordinate = |key: (i64, i64)| {
        if use_x { key.0 } else { key.1 }
    };
    let a0 = coordinate(a_start);
    let a1 = coordinate(a_end);
    let b0 = coordinate(b_start);
    let b1 = coordinate(b_end);
    a0.min(a1).max(b0.min(b1)) < a0.max(a1).min(b0.max(b1))
}

pub(in crate::simulation::network::surface::tests) fn polygon_centroid_for_test(
    polygon: &RoadSurfaceVisualPolygon,
) -> Option<Vector3> {
    let mut sum = Vector3::ZERO;
    let mut count = 0usize;
    for point in &polygon.points_world {
        sum += Vector3::new(point.x, 0.0, point.z);
        count += 1;
    }
    (count > 0).then_some(sum / count as f32)
}
