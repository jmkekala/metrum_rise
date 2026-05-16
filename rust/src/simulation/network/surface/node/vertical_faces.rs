//! Raised-step vertical face construction and support checks.

use super::arrangement_faces::*;
use super::boundary_edges::*;
use super::*;

impl RoadSurfaceSystem {
    pub(super) fn raised_step_face_polygons_from_arrangement(
        arrangement: &NodeArrangement,
        explicit_vertical_step_segments: &[NodeExplicitVerticalStepSegment],
    ) -> Vec<(RoadSurfaceVisualPolygon, RoadSurfaceVerticalFaceSource)> {
        let mut emitted = BTreeSet::new();
        let mut faces = Vec::new();
        for (step_index, segment) in explicit_vertical_step_segments.iter().copied().enumerate() {
            let Some((lower_owner, raised_owner)) =
                canonical_vertical_step_lower_and_raised_owners(segment)
            else {
                continue;
            };
            let segment_key = (segment.start(), segment.end());
            let lower_intervals = arrangement_owner_face_boundary_intervals_for_segment(
                arrangement,
                lower_owner,
                segment_key,
            );
            let raised_intervals = arrangement_owner_face_boundary_intervals_for_segment(
                arrangement,
                raised_owner,
                segment_key,
            );
            Self::push_arrangement_vertical_step_faces_from_intervals(
                arrangement,
                lower_owner,
                segment_key,
                segment_key,
                &lower_intervals,
                &raised_intervals,
                step_index,
                segment,
                &mut emitted,
                &mut faces,
            );
            if let Some((dedup_key, face)) = Self::arrangement_vertical_step_face_from_segment(
                arrangement,
                lower_owner,
                raised_owner,
                segment_key,
            ) {
                if emitted.insert(dedup_key) {
                    faces.push((
                        face,
                        RoadSurfaceVerticalFaceSource::CanonicalStep {
                            explicit_vertical_step_index: step_index,
                            segment,
                        },
                    ));
                }
            }
        }
        faces
    }

    fn push_arrangement_vertical_step_faces_from_intervals(
        arrangement: &NodeArrangement,
        lower_owner: NodeBandOwner,
        lower_segment_key: (NodeArrangementKey, NodeArrangementKey),
        raised_segment_key: (NodeArrangementKey, NodeArrangementKey),
        lower_intervals: &[ArrangementFaceBoundaryInterval],
        raised_intervals: &[ArrangementFaceBoundaryInterval],
        step_index: usize,
        segment: NodeExplicitVerticalStepSegment,
        emitted: &mut BTreeSet<(
            (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
            (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
        )>,
        faces: &mut Vec<(RoadSurfaceVisualPolygon, RoadSurfaceVerticalFaceSource)>,
    ) {
        for (lower_interval, raised_interval, start_t, end_t) in
            arrangement_shared_face_boundary_intervals(lower_intervals, raised_intervals)
        {
            let Some(lower_start) = arrangement_face_boundary_interval_point_at(
                lower_segment_key,
                lower_interval,
                start_t,
            ) else {
                continue;
            };
            let Some(lower_end) = arrangement_face_boundary_interval_point_at(
                lower_segment_key,
                lower_interval,
                end_t,
            ) else {
                continue;
            };
            let Some(raised_start) = arrangement_face_boundary_interval_point_at(
                raised_segment_key,
                raised_interval,
                start_t,
            ) else {
                continue;
            };
            let Some(raised_end) = arrangement_face_boundary_interval_point_at(
                raised_segment_key,
                raised_interval,
                end_t,
            ) else {
                continue;
            };
            let Some((dedup_key, face)) = Self::arrangement_vertical_step_face_polygon(
                arrangement,
                lower_owner,
                lower_segment_key,
                lower_start,
                lower_end,
                raised_start,
                raised_end,
            ) else {
                continue;
            };
            if !emitted.insert(dedup_key) {
                continue;
            }
            faces.push((
                face,
                RoadSurfaceVerticalFaceSource::CanonicalStep {
                    explicit_vertical_step_index: step_index,
                    segment,
                },
            ));
        }
    }

    fn arrangement_vertical_step_face_from_segment(
        arrangement: &NodeArrangement,
        lower_owner: NodeBandOwner,
        raised_owner: NodeBandOwner,
        segment_key: (NodeArrangementKey, NodeArrangementKey),
    ) -> Option<(
        (
            (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
            (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
        ),
        RoadSurfaceVisualPolygon,
    )> {
        let lower_start = arrangement_owner_boundary_point_at_key(
            arrangement,
            lower_owner,
            segment_key.0,
            false,
        )?;
        let lower_end = arrangement_owner_boundary_point_at_key(
            arrangement,
            lower_owner,
            segment_key.1,
            false,
        )?;
        let raised_start = arrangement_owner_boundary_point_at_key(
            arrangement,
            raised_owner,
            segment_key.0,
            true,
        )?;
        let raised_end = arrangement_owner_boundary_point_at_key(
            arrangement,
            raised_owner,
            segment_key.1,
            true,
        )?;
        Self::arrangement_vertical_step_face_polygon(
            arrangement,
            lower_owner,
            segment_key,
            lower_start,
            lower_end,
            raised_start,
            raised_end,
        )
    }

    fn arrangement_vertical_step_face_polygon(
        arrangement: &NodeArrangement,
        lower_owner: NodeBandOwner,
        segment_key: (NodeArrangementKey, NodeArrangementKey),
        lower_start: Vector3,
        lower_end: Vector3,
        raised_start: Vector3,
        raised_end: Vector3,
    ) -> Option<(
        (
            (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
            (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
        ),
        RoadSurfaceVisualPolygon,
    )> {
        let lower_span_xz = Vector2::new(lower_end.x - lower_start.x, lower_end.z - lower_start.z);
        let raised_span_xz =
            Vector2::new(raised_end.x - raised_start.x, raised_end.z - raised_start.z);
        if lower_span_xz.length_squared() <= VERTICAL_STEP_MIN_SPAN_M * VERTICAL_STEP_MIN_SPAN_M
            || raised_span_xz.length_squared()
                <= VERTICAL_STEP_MIN_SPAN_M * VERTICAL_STEP_MIN_SPAN_M
        {
            return None;
        }
        if (raised_start.y - lower_start.y <= SAMPLE_EPSILON_M)
            && (raised_end.y - lower_end.y <= SAMPLE_EPSILON_M)
        {
            return None;
        }
        let dedup_key = vertical_face_dedup_key(lower_start, lower_end, raised_start, raised_end);
        let mut points = [raised_start, lower_start, lower_end, raised_end];
        if let Some(visible_dot) = arrangement_vertical_face_visible_dot_to_owner(
            arrangement,
            lower_owner,
            segment_key,
            points,
        ) {
            if visible_dot <= 0.0 {
                points = [points[3], points[2], points[1], points[0]];
            }
        } else {
            let lower_owner_direction = arrangement_owner_direction_for_segment(
                arrangement,
                lower_owner,
                segment_key,
                lower_start,
                lower_end,
            )
            .unwrap_or_else(|| {
                let edge_direction = lower_end - lower_start;
                Vector3::new(-edge_direction.z, 0.0, edge_direction.x)
            });
            let face_normal = (points[1] - points[0]).cross(points[2] - points[0]);
            if face_normal.dot(lower_owner_direction) > 0.0 {
                points = [points[3], points[2], points[1], points[0]];
            }
        }
        Self::make_vertical_quad_polygon(points).map(|face| (dedup_key, face))
    }

    pub(super) fn sort_raised_step_faces(
        faces: &mut [(RoadSurfaceVisualPolygon, RoadSurfaceVerticalFaceSource)],
    ) {
        faces.sort_by(
            |(left_polygon, left_source), (right_polygon, right_source)| {
                Self::visual_polygon_ordering(left_polygon, right_polygon)
                    .then(left_source.sort_key().cmp(&right_source.sort_key()))
            },
        );
    }
}

fn vertical_face_dedup_key(
    lower_start: Vector3,
    lower_end: Vector3,
    upper_start: Vector3,
    upper_end: Vector3,
) -> (
    (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
    (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
) {
    (
        normalized_arrangement_boundary_segment_key(lower_start, lower_end),
        normalized_arrangement_boundary_segment_key(upper_start, upper_end),
    )
}

pub(super) fn canonical_vertical_step_lower_and_raised_owners(
    segment: NodeExplicitVerticalStepSegment,
) -> Option<(NodeBandOwner, NodeBandOwner)> {
    let owner = segment.owner();
    let opposite_owner = segment.opposite_owner();
    let (lower_kind, _) = ordered_raised_step_kinds(owner.kind(), opposite_owner.kind())?;
    if owner.kind() == lower_kind {
        Some((owner, opposite_owner))
    } else {
        Some((opposite_owner, owner))
    }
}

pub(super) fn dedup_raised_step_faces(
    faces: &mut Vec<(RoadSurfaceVisualPolygon, RoadSurfaceVerticalFaceSource)>,
) {
    let mut emitted = BTreeSet::new();
    faces.retain(|(polygon, _)| {
        let Some(key) = raised_step_face_span_key(polygon) else {
            return true;
        };
        emitted.insert(key)
    });
}

pub(super) fn append_canonical_raised_step_faces_from_owned_region_boundaries(
    faces: &mut Vec<(RoadSurfaceVisualPolygon, RoadSurfaceVerticalFaceSource)>,
    owned_regions: &[NodeOwnedRegion],
    explicit_vertical_step_segments: &[NodeExplicitVerticalStepSegment],
) {
    for (step_index, segment) in explicit_vertical_step_segments.iter().copied().enumerate() {
        let Some((lower_owner, raised_owner)) =
            canonical_vertical_step_lower_and_raised_owners(segment)
        else {
            continue;
        };
        for lower_edge in owned_region_boundary_edges_for_owner(owned_regions, lower_owner) {
            if !world_edge_lies_on_explicit_vertical_step_segment(lower_edge, segment) {
                continue;
            }
            for raised_edge in owned_region_boundary_edges_for_owner(owned_regions, raised_owner) {
                let Some(raised_edge) = clip_edge_to_reference_xz(raised_edge, lower_edge) else {
                    continue;
                };
                if (raised_edge[0].y - lower_edge[0].y <= SAMPLE_EPSILON_M)
                    && (raised_edge[1].y - lower_edge[1].y <= SAMPLE_EPSILON_M)
                {
                    continue;
                }
                let Some(face) = RoadSurfaceSystem::make_vertical_quad_polygon([
                    raised_edge[0],
                    lower_edge[0],
                    lower_edge[1],
                    raised_edge[1],
                ]) else {
                    continue;
                };
                faces.push((
                    face,
                    RoadSurfaceVerticalFaceSource::CanonicalStep {
                        explicit_vertical_step_index: step_index,
                        segment,
                    },
                ));
            }
        }
    }
}

pub(super) fn append_final_owned_raised_step_faces_from_shared_top_boundaries(
    faces: &mut Vec<(RoadSurfaceVisualPolygon, RoadSurfaceVerticalFaceSource)>,
    owned_regions: &[NodeOwnedRegion],
) {
    let mut edges_by_xz = BTreeMap::<
        (NodeArrangementKey, NodeArrangementKey),
        Vec<(NodeBandOwner, [Vector3; 2])>,
    >::new();
    for region in owned_regions {
        let owner = NodeBandOwner::new(region.kind, region.owner_index);
        for edge in owned_region_boundary_edges(region) {
            let start = ArrangementBoundaryPointKey::from_world(edge[0]).xz_key();
            let end = ArrangementBoundaryPointKey::from_world(edge[1]).xz_key();
            if start == end {
                continue;
            }
            let key = if start <= end {
                (start, end)
            } else {
                (end, start)
            };
            edges_by_xz.entry(key).or_default().push((owner, edge));
        }
    }

    for (key, edges) in edges_by_xz {
        for (left_index, (left_owner, left_edge)) in edges.iter().copied().enumerate() {
            for (right_owner, right_edge) in edges.iter().copied().skip(left_index + 1) {
                let Some(segment) =
                    NodeExplicitVerticalStepSegment::new(key.0, key.1, left_owner, right_owner)
                else {
                    continue;
                };
                let Some((lower_owner, raised_owner)) =
                    canonical_vertical_step_lower_and_raised_owners(segment)
                else {
                    continue;
                };
                let (lower_edge, raised_edge) =
                    if left_owner == lower_owner && right_owner == raised_owner {
                        (left_edge, right_edge)
                    } else if right_owner == lower_owner && left_owner == raised_owner {
                        (right_edge, left_edge)
                    } else {
                        continue;
                    };
                let Some(raised_edge) = clip_edge_to_reference_xz(raised_edge, lower_edge) else {
                    continue;
                };
                if (raised_edge[0].y - lower_edge[0].y <= SAMPLE_EPSILON_M)
                    && (raised_edge[1].y - lower_edge[1].y <= SAMPLE_EPSILON_M)
                {
                    continue;
                }
                let Some(face) = RoadSurfaceSystem::make_vertical_quad_polygon([
                    raised_edge[0],
                    lower_edge[0],
                    lower_edge[1],
                    raised_edge[1],
                ]) else {
                    continue;
                };
                faces.push((
                    face,
                    RoadSurfaceVerticalFaceSource::FinalOwnedBoundary { segment },
                ));
            }
        }
    }
}

pub(super) fn retain_raised_step_faces_with_top_support(
    faces: &mut Vec<(RoadSurfaceVisualPolygon, RoadSurfaceVerticalFaceSource)>,
    owned_regions: &[NodeOwnedRegion],
) {
    faces.retain(|(polygon, source)| {
        let Some((lower_owner, raised_owner)) =
            canonical_vertical_step_lower_and_raised_owners(source.segment())
        else {
            return false;
        };
        let Some((lower_edge, upper_edge)) = vertical_face_support_edges(polygon) else {
            return false;
        };
        owned_region_has_top_boundary_edge(owned_regions, lower_owner, lower_edge)
            && owned_region_has_top_boundary_edge(owned_regions, raised_owner, upper_edge)
    });
}

pub(super) fn orient_raised_step_faces_to_lower_owner_support(
    faces: &mut Vec<(RoadSurfaceVisualPolygon, RoadSurfaceVerticalFaceSource)>,
    owned_regions: &[NodeOwnedRegion],
) {
    for (polygon, source) in faces {
        let Some((lower_owner, _)) =
            canonical_vertical_step_lower_and_raised_owners(source.segment())
        else {
            continue;
        };
        let Some(lower_edge) =
            vertical_face_support_edge_for_owner(polygon, owned_regions, lower_owner)
        else {
            continue;
        };
        let Some(visible_dot) = vertical_face_visible_dot_to_supported_owner(
            polygon,
            lower_edge,
            owned_regions,
            lower_owner,
        ) else {
            continue;
        };
        if visible_dot > 0.0 {
            continue;
        }
        let Some(points) = reversed_vertical_face_points(polygon) else {
            continue;
        };
        if let Some(oriented) = RoadSurfaceSystem::make_vertical_quad_polygon(points) {
            *polygon = oriented;
        }
    }
}

fn owned_region_has_top_boundary_edge(
    owned_regions: &[NodeOwnedRegion],
    owner: NodeBandOwner,
    edge: [Vector3; 2],
) -> bool {
    owned_regions
        .iter()
        .filter(|region| node_owned_region_matches_owner(region, owner))
        .any(|region| visual_polygon_boundary_overlaps_edge_at_height(&region.polygon, edge))
}

fn vertical_face_support_edge_for_owner(
    polygon: &RoadSurfaceVisualPolygon,
    owned_regions: &[NodeOwnedRegion],
    owner: NodeBandOwner,
) -> Option<[Vector3; 2]> {
    vertical_face_side_edges(polygon).and_then(|edges| {
        edges
            .into_iter()
            .find(|edge| owned_region_has_top_boundary_edge_xz(owned_regions, owner, *edge))
    })
}

fn owned_region_has_top_boundary_edge_xz(
    owned_regions: &[NodeOwnedRegion],
    owner: NodeBandOwner,
    edge: [Vector3; 2],
) -> bool {
    owned_regions
        .iter()
        .filter(|region| node_owned_region_matches_owner(region, owner))
        .any(|region| visual_polygon_boundary_overlaps_edge_xz(&region.polygon, edge))
}

fn vertical_face_visible_dot_to_supported_owner(
    polygon: &RoadSurfaceVisualPolygon,
    lower_edge: [Vector3; 2],
    owned_regions: &[NodeOwnedRegion],
    owner: NodeBandOwner,
) -> Option<f32> {
    let visible_direction = vertical_face_visible_direction(polygon)?;
    let midpoint = (lower_edge[0] + lower_edge[1]) * 0.5;
    let mut best_dot: Option<f32> = None;
    for region in owned_regions
        .iter()
        .filter(|region| node_owned_region_matches_owner(region, owner))
    {
        if !visual_polygon_boundary_overlaps_edge_xz(&region.polygon, lower_edge) {
            continue;
        }
        let Some(centroid) = visual_polygon_centroid(&region.polygon) else {
            continue;
        };
        let owner_direction = Vector3::new(centroid.x - midpoint.x, 0.0, centroid.z - midpoint.z);
        if owner_direction.length_squared() <= 1e-8 {
            continue;
        }
        let dot = visible_direction.dot(owner_direction.normalized());
        best_dot = Some(best_dot.map_or(dot, |current| current.max(dot)));
    }
    best_dot
}

fn vertical_face_visible_direction(polygon: &RoadSurfaceVisualPolygon) -> Option<Vector3> {
    let [upper_start, lower_start, lower_end, _upper_end] = polygon.points_world.as_slice() else {
        return None;
    };
    let normal = (*lower_start - *upper_start).cross(*lower_end - *upper_start);
    let visible_direction = Vector3::new(-normal.x, 0.0, -normal.z);
    if visible_direction.length_squared() <= 1e-8 {
        return None;
    }
    Some(visible_direction.normalized())
}

fn visual_polygon_centroid(polygon: &RoadSurfaceVisualPolygon) -> Option<Vector3> {
    let mut sum = Vector3::ZERO;
    let mut count = 0usize;
    for point in &polygon.points_world {
        sum += Vector3::new(point.x, 0.0, point.z);
        count += 1;
    }
    (count > 0).then_some(sum / count as f32)
}

fn reversed_vertical_face_points(polygon: &RoadSurfaceVisualPolygon) -> Option<[Vector3; 4]> {
    let [a, b, c, d] = polygon.points_world.as_slice() else {
        return None;
    };
    Some([*d, *c, *b, *a])
}

pub(super) fn vertical_face_side_edges(
    polygon: &RoadSurfaceVisualPolygon,
) -> Option<[[Vector3; 2]; 2]> {
    let [a, b, c, d] = polygon.points_world.as_slice() else {
        return None;
    };
    Some([[*a, *d], [*b, *c]])
}

fn vertical_face_support_edges(
    polygon: &RoadSurfaceVisualPolygon,
) -> Option<([Vector3; 2], [Vector3; 2])> {
    let [first_edge, second_edge] = vertical_face_side_edges(polygon)?;
    let first_avg_y = (first_edge[0].y + first_edge[1].y) * 0.5;
    let second_avg_y = (second_edge[0].y + second_edge[1].y) * 0.5;
    if first_avg_y <= second_avg_y {
        Some((first_edge, second_edge))
    } else {
        Some((second_edge, first_edge))
    }
}

fn raised_step_face_span_key(
    polygon: &RoadSurfaceVisualPolygon,
) -> Option<(
    (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
    (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
)> {
    if polygon.points_world.len() != 4 {
        return None;
    }
    let mut span_edges = Vec::new();
    for index in 0..polygon.points_world.len() {
        let start = polygon.points_world[index];
        let end = polygon.points_world[(index + 1) % polygon.points_world.len()];
        if ArrangementBoundaryPointKey::from_world(start).xz_key()
            != ArrangementBoundaryPointKey::from_world(end).xz_key()
        {
            span_edges.push((start, end, (start.y + end.y) * 0.5));
        }
    }
    if span_edges.len() != 2 {
        return None;
    }
    span_edges.sort_by(|a, b| a.2.total_cmp(&b.2));
    Some(vertical_face_dedup_key(
        span_edges[0].0,
        span_edges[0].1,
        span_edges[1].0,
        span_edges[1].1,
    ))
}
