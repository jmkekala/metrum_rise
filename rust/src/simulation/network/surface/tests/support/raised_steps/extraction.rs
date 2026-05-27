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

pub(in crate::simulation::network::surface::tests) fn test_owned_top_boundary_edges(
    piece: &RoadSurfaceVisualNodePiece,
) -> Vec<TestTopBoundaryEdge> {
    let mut boundary_edges = Vec::new();
    for region in &piece.owned_regions {
        let mut edge_counts = BTreeMap::<TestRenderEdgeKey, (usize, RoadVec3, RoadVec3)>::new();
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
) -> Option<[RoadVec3; 2]> {
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
