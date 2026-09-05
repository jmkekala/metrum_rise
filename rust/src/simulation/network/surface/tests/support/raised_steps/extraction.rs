// SPDX-License-Identifier: GPL-2.0-only

//! Raised-step top-boundary and source extraction helpers.

use super::*;

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
            top_edges_have_positive_height_delta(*lower_edge, *raised_edge)
                && test_top_edges_form_raised_step(*lower_edge, *raised_edge)
        })
    })
}

fn top_edges_have_positive_height_delta(
    lower_edge: TestTopBoundaryEdge,
    raised_edge: TestTopBoundaryEdge,
) -> bool {
    let lower_points = [lower_edge.key.start, lower_edge.key.end];
    let raised_points = [raised_edge.key.start, raised_edge.key.end];
    lower_points.iter().any(|lower| {
        raised_points.iter().any(|raised| {
            lower.x_key == raised.x_key && lower.z_key == raised.z_key && lower.y_mm < raised.y_mm
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

pub(in crate::simulation::network::surface::tests) fn test_owned_top_boundary_edges(
    piece: &RoadSurfaceVisualNodePiece,
) -> Vec<TestTopBoundaryEdge> {
    let mut top_edges = Vec::new();
    for region in &piece.owned_regions {
        let mut edge_counts = BTreeMap::<TestRenderEdgeKey, (usize, RoadVec3, RoadVec3)>::new();
        for (key, start, end) in test_polygon_top_edge_candidates(&region.polygon) {
            edge_counts
                .entry(key)
                .and_modify(|entry| entry.0 += 1)
                .or_insert((1, start, end));
        }
        top_edges.extend(
            edge_counts
                .into_iter()
                .filter_map(|(key, (count, start, end))| {
                    (count == 1).then_some(TestTopBoundaryEdge {
                        kind: region.kind,
                        owner_index: region.owner_index,
                        start,
                        end,
                        key,
                        xz_key: key.xz(),
                        avg_y_m: (start.y + end.y) * 0.5,
                    })
                }),
        );
    }
    top_edges
}

pub(in crate::simulation::network::surface::tests) fn test_polygon_top_boundary_edges(
    kind: RoadSurfaceBandKind,
    owner_index: usize,
    polygon: &RoadSurfaceVisualPolygon,
) -> Vec<TestTopBoundaryEdge> {
    let mut edge_counts = BTreeMap::<TestRenderEdgeKey, (usize, RoadVec3, RoadVec3)>::new();
    for (key, start, end) in test_polygon_top_edge_candidates(polygon) {
        edge_counts
            .entry(key)
            .and_modify(|entry| entry.0 += 1)
            .or_insert((1, start, end));
    }
    edge_counts
        .into_iter()
        .filter_map(|(key, (count, start, end))| {
            (count == 1).then_some(TestTopBoundaryEdge {
                kind,
                owner_index,
                start,
                end,
                key,
                xz_key: key.xz(),
                avg_y_m: (start.y + end.y) * 0.5,
            })
        })
        .collect()
}

fn test_polygon_top_edge_candidates(
    polygon: &RoadSurfaceVisualPolygon,
) -> Vec<(TestRenderEdgeKey, RoadVec3, RoadVec3)> {
    let mut edges = Vec::new();
    if polygon.triangles_world.is_empty() {
        let points = &polygon.points_world;
        if points.len() >= 2 {
            for index in 0..points.len() {
                if let Some(key) =
                    TestRenderEdgeKey::normalized(points[index], points[(index + 1) % points.len()])
                {
                    edges.push((key, points[index], points[(index + 1) % points.len()]));
                }
            }
        }
    } else {
        for triangle in &polygon.triangles_world {
            for edge_index in 0..3 {
                if let Some(key) = TestRenderEdgeKey::normalized(
                    triangle[edge_index],
                    triangle[(edge_index + 1) % 3],
                ) {
                    edges.push((key, triangle[edge_index], triangle[(edge_index + 1) % 3]));
                }
            }
        }
    }
    edges
}

pub(in crate::simulation::network::surface::tests) fn vertical_face_lower_edge_for_test(
    polygon: &RoadSurfaceVisualPolygon,
) -> Option<[RoadVec3; 2]> {
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
) -> Option<[[RoadVec3; 2]; 2]> {
    let [a, b, c, d] = polygon.points_world.as_slice() else {
        return None;
    };
    Some([[*a, *d], [*b, *c]])
}

pub(in crate::simulation::network::surface::tests) fn test_lower_owner_from_vertical_face_source(
    source: super::RoadSurfaceVerticalFaceSource,
) -> Option<NodeBandOwner> {
    test_lower_and_raised_owners_from_vertical_face_source(source).map(|(lower, _)| lower)
}

pub(in crate::simulation::network::surface::tests) fn test_lower_and_raised_owners_from_vertical_face_source(
    source: super::RoadSurfaceVerticalFaceSource,
) -> Option<(NodeBandOwner, NodeBandOwner)> {
    source.lower_and_raised_owners()
}

pub(in crate::simulation::network::surface::tests) fn vertical_face_owner_edge_for_test(
    face: &RoadSurfaceVisualPolygon,
    top_edges: &[TestTopBoundaryEdge],
    owner: NodeBandOwner,
) -> Option<[RoadVec3; 2]> {
    let [first_edge, second_edge] = vertical_face_side_edges_for_test(face)?;
    [first_edge, second_edge].into_iter().find(|edge| {
        top_edges.iter().any(|top_edge| {
            top_edge.kind == owner.kind()
                && top_edge.owner_index == owner.owner_index()
                && test_boundary_edge_contains_edge_at_height([top_edge.start, top_edge.end], *edge)
        })
    })
}

pub(in crate::simulation::network::surface::tests) fn vertical_face_visible_direction_for_test(
    polygon: &RoadSurfaceVisualPolygon,
) -> Option<RoadVec3> {
    let [upper_start, lower_start, lower_end, _upper_end] = polygon.points_world.as_slice() else {
        return None;
    };
    let normal = (*lower_start - *upper_start).cross(*lower_end - *upper_start);
    (normal.length_squared() > 1e-8).then(|| -normal.normalize())
}

pub(in crate::simulation::network::surface::tests) fn vertical_face_upper_edge_for_test(
    polygon: &RoadSurfaceVisualPolygon,
) -> Option<[RoadVec3; 2]> {
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
