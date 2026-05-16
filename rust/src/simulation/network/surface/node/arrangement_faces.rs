//! Arrangement face boundary intervals used by node vertical face export.

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct ArrangementFaceBoundaryInterval {
    owner: NodeBandOwner,
    start: ArrangementSegmentParameter,
    end: ArrangementSegmentParameter,
    edge_start: ArrangementBoundaryPointKey,
    edge_end: ArrangementBoundaryPointKey,
}

pub(super) fn arrangement_owner_face_boundary_intervals_for_segment(
    arrangement: &NodeArrangement,
    owner: NodeBandOwner,
    segment_key: (NodeArrangementKey, NodeArrangementKey),
) -> Vec<ArrangementFaceBoundaryInterval> {
    let mut intervals = Vec::new();
    for face in arrangement
        .faces()
        .iter()
        .filter(|face| face.owner() == owner)
        .filter(|face| {
            RoadSurfaceSystem::arrangement_face_visual_triangle(arrangement, face).is_some()
        })
    {
        let vertices = face.vertices();
        for index in 0..vertices.len() {
            let Some(edge_start) =
                arrangement_vertex_boundary_point_key(arrangement, vertices[index])
            else {
                continue;
            };
            let Some(edge_end) = arrangement_vertex_boundary_point_key(
                arrangement,
                vertices[(index + 1) % vertices.len()],
            ) else {
                continue;
            };
            if let Some((start, end)) =
                arrangement_face_boundary_overlap_interval(segment_key, edge_start, edge_end)
            {
                intervals.push(ArrangementFaceBoundaryInterval {
                    owner: face.owner(),
                    start,
                    end,
                    edge_start,
                    edge_end,
                });
            }
        }
    }
    intervals.sort();
    intervals.dedup();
    intervals
}

fn arrangement_vertex_boundary_point_key(
    arrangement: &NodeArrangement,
    vertex_id: super::arrangement::NodeArrangementVertexId,
) -> Option<ArrangementBoundaryPointKey> {
    let vertex = arrangement.vertices().get(vertex_id.index())?;
    Some(arrangement_key_boundary_point(
        vertex.key(),
        vertex.height_mm(),
    ))
}

pub(super) fn arrangement_shared_face_boundary_intervals(
    lower_intervals: &[ArrangementFaceBoundaryInterval],
    raised_intervals: &[ArrangementFaceBoundaryInterval],
) -> Vec<(
    ArrangementFaceBoundaryInterval,
    ArrangementFaceBoundaryInterval,
    ArrangementSegmentParameter,
    ArrangementSegmentParameter,
)> {
    let mut shared = Vec::new();
    for lower in lower_intervals {
        for raised in raised_intervals {
            let start = lower.start.max(raised.start);
            let end = lower.end.min(raised.end);
            if end > start {
                shared.push((*lower, *raised, start, end));
            }
        }
    }
    shared.sort_by(|a, b| a.2.cmp(&b.2).then(a.3.cmp(&b.3)));
    shared
}

fn arrangement_face_boundary_overlap_interval(
    segment_key: (NodeArrangementKey, NodeArrangementKey),
    edge_start: ArrangementBoundaryPointKey,
    edge_end: ArrangementBoundaryPointKey,
) -> Option<(ArrangementSegmentParameter, ArrangementSegmentParameter)> {
    let segment_start = arrangement_key_boundary_point(segment_key.0, 0);
    let segment_end = arrangement_key_boundary_point(segment_key.1, 0);
    let edge_start_t = boundary_segment_parameter_xz(edge_start, segment_start, segment_end)?;
    let edge_end_t = boundary_segment_parameter_xz(edge_end, segment_start, segment_end)?;
    let start = edge_start_t
        .min(edge_end_t)
        .max(ArrangementSegmentParameter::zero());
    let end = edge_start_t
        .max(edge_end_t)
        .min(ArrangementSegmentParameter::one());
    (end > start).then_some((start, end))
}

pub(super) fn arrangement_face_boundary_interval_point_at(
    segment_key: (NodeArrangementKey, NodeArrangementKey),
    interval: ArrangementFaceBoundaryInterval,
    parameter: ArrangementSegmentParameter,
) -> Option<Vector3> {
    let segment_start = arrangement_key_boundary_point(segment_key.0, 0);
    let segment_end = arrangement_key_boundary_point(segment_key.1, 0);
    let segment_point = interpolated_segment_point_key(segment_start, segment_end, parameter);
    let edge_t =
        boundary_segment_parameter_xz(segment_point, interval.edge_start, interval.edge_end)?;
    let edge_point = interpolated_segment_point_key(interval.edge_start, interval.edge_end, edge_t);
    let y_mm = interpolated_segment_height_mm(interval.edge_start, interval.edge_end, edge_t);
    Some(arrangement_boundary_point_to_world(
        ArrangementBoundaryPointKey {
            x_key: edge_point.x_key,
            z_key: edge_point.z_key,
            y_mm,
        },
    ))
}

pub(super) fn arrangement_key_boundary_point(
    key: NodeArrangementKey,
    y_mm: i64,
) -> ArrangementBoundaryPointKey {
    ArrangementBoundaryPointKey {
        x_key: key.x_key(),
        z_key: key.z_key(),
        y_mm,
    }
}

pub(super) fn arrangement_owner_boundary_point_at_key(
    arrangement: &NodeArrangement,
    owner: NodeBandOwner,
    key: NodeArrangementKey,
    prefer_highest: bool,
) -> Option<Vector3> {
    let mut candidates = arrangement
        .vertices()
        .iter()
        .filter(|vertex| vertex.owners().contains(&owner))
        .filter(|vertex| vertex.key() == key)
        .collect::<Vec<_>>();
    candidates.sort_by_key(|vertex| {
        let height_key = if prefer_highest {
            -vertex.height_mm()
        } else {
            vertex.height_mm()
        };
        (height_key, vertex.key())
    });
    candidates.first().map(|vertex| {
        arrangement_boundary_point_to_world(arrangement_key_boundary_point(
            vertex.key(),
            vertex.height_mm(),
        ))
    })
}

pub(super) fn arrangement_owner_direction_for_segment(
    arrangement: &NodeArrangement,
    owner: NodeBandOwner,
    segment_key: (NodeArrangementKey, NodeArrangementKey),
    start: Vector3,
    end: Vector3,
) -> Option<Vector3> {
    let midpoint = (start + end) * 0.5;
    let mut best = None;
    for face in arrangement.faces() {
        if face.owner() != owner
            || !arrangement_face_boundary_overlaps_segment(arrangement, face, segment_key)
        {
            continue;
        }
        let Some(centroid) = arrangement_face_centroid(arrangement, face) else {
            continue;
        };
        let direction = Vector3::new(centroid.x - midpoint.x, 0.0, centroid.z - midpoint.z);
        let distance_squared = direction.length_squared();
        if distance_squared <= SAMPLE_EPSILON_M * SAMPLE_EPSILON_M {
            continue;
        }
        if best.is_none_or(|(best_distance_squared, _)| distance_squared < best_distance_squared) {
            best = Some((distance_squared, direction));
        }
    }
    if let Some((_, direction)) = best {
        return Some(direction);
    }

    for region in arrangement.regions() {
        if region.owner() != owner
            || !arrangement_region_boundary_overlaps_segment(arrangement, region, segment_key)
        {
            continue;
        }
        let Some(centroid) = arrangement_region_centroid(arrangement, region) else {
            continue;
        };
        let direction = Vector3::new(centroid.x - midpoint.x, 0.0, centroid.z - midpoint.z);
        let distance_squared = direction.length_squared();
        if distance_squared <= SAMPLE_EPSILON_M * SAMPLE_EPSILON_M {
            continue;
        }
        if best.is_none_or(|(best_distance_squared, _)| distance_squared < best_distance_squared) {
            best = Some((distance_squared, direction));
        }
    }
    best.map(|(_, direction)| direction)
}

pub(super) fn arrangement_vertical_face_visible_dot_to_owner(
    arrangement: &NodeArrangement,
    owner: NodeBandOwner,
    segment_key: (NodeArrangementKey, NodeArrangementKey),
    points: [Vector3; 4],
) -> Option<f32> {
    let [upper_start, lower_start, lower_end, _upper_end] = points;
    let normal = (lower_start - upper_start).cross(lower_end - upper_start);
    if normal.length_squared() <= 1e-8 {
        return None;
    }
    let visible_direction = Vector3::new(-normal.x, 0.0, -normal.z);
    if visible_direction.length_squared() <= 1e-8 {
        return None;
    }
    let visible_direction = visible_direction.normalized();
    let midpoint = (lower_start + lower_end) * 0.5;
    let mut best_dot: Option<f32> = None;
    for face in arrangement.faces() {
        if face.owner() != owner
            || !arrangement_face_boundary_overlaps_segment(arrangement, face, segment_key)
        {
            continue;
        }
        let Some(centroid) = arrangement_face_centroid(arrangement, face) else {
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

fn arrangement_face_boundary_overlaps_segment(
    arrangement: &NodeArrangement,
    face: &NodeArrangementFace,
    segment_key: (NodeArrangementKey, NodeArrangementKey),
) -> bool {
    let Some(triangle) = RoadSurfaceSystem::arrangement_face_visual_triangle(arrangement, face)
    else {
        return false;
    };
    for index in 0..triangle.len() {
        let start =
            NodeArrangementKey::from_point(super::backend::godot_vec3_xz_to_road(triangle[index]));
        let end = NodeArrangementKey::from_point(super::backend::godot_vec3_xz_to_road(
            triangle[(index + 1) % triangle.len()],
        ));
        if arrangement_segments_overlap_with_length(start, end, segment_key.0, segment_key.1) {
            return true;
        }
    }
    false
}

fn arrangement_face_centroid(
    arrangement: &NodeArrangement,
    face: &NodeArrangementFace,
) -> Option<Vector3> {
    let triangle = RoadSurfaceSystem::arrangement_face_visual_triangle(arrangement, face)?;
    let mut sum = Vector3::ZERO;
    for point in triangle {
        sum += Vector3::new(point.x, 0.0, point.z);
    }
    Some(sum / triangle.len() as f32)
}

fn arrangement_region_boundary_overlaps_segment(
    arrangement: &NodeArrangement,
    region: &super::arrangement::NodeOwnedRegion,
    segment_key: (NodeArrangementKey, NodeArrangementKey),
) -> bool {
    region.boundary_edges().iter().any(|edge_id| {
        let Some(edge) = arrangement.edges().get(edge_id.index()) else {
            return false;
        };
        let Some(edge_start) = arrangement
            .vertices()
            .get(edge.start().index())
            .map(|vertex| NodeArrangementKey::from_point(vertex.point_xz()))
        else {
            return false;
        };
        let Some(edge_end) = arrangement
            .vertices()
            .get(edge.end().index())
            .map(|vertex| NodeArrangementKey::from_point(vertex.point_xz()))
        else {
            return false;
        };
        arrangement_segments_overlap_with_length(edge_start, edge_end, segment_key.0, segment_key.1)
    })
}

fn arrangement_segments_overlap_with_length(
    a_start: NodeArrangementKey,
    a_end: NodeArrangementKey,
    b_start: NodeArrangementKey,
    b_end: NodeArrangementKey,
) -> bool {
    if a_start == a_end || b_start == b_end {
        return false;
    }
    let a_dx = i128::from(a_end.x_key() - a_start.x_key());
    let a_dz = i128::from(a_end.z_key() - a_start.z_key());
    let b_dx = i128::from(b_end.x_key() - b_start.x_key());
    let b_dz = i128::from(b_end.z_key() - b_start.z_key());
    let cross = a_dx * b_dz - a_dz * b_dx;
    let collinearity_bound = surface_overlay_grid_collinearity_error_bound(a_dx, a_dz)
        .max(surface_overlay_grid_collinearity_error_bound(b_dx, b_dz));
    if cross != 0 && cross.abs() > collinearity_bound {
        return false;
    }
    if !arrangement_key_lies_on_segment(a_start, b_start, b_end)
        && !arrangement_key_lies_on_segment(a_end, b_start, b_end)
        && !arrangement_key_lies_on_segment(b_start, a_start, a_end)
        && !arrangement_key_lies_on_segment(b_end, a_start, a_end)
    {
        return false;
    }
    let use_x = (a_end.x_key() - a_start.x_key()).abs() >= (a_end.z_key() - a_start.z_key()).abs();
    let coordinate = |key: NodeArrangementKey| {
        if use_x { key.x_key() } else { key.z_key() }
    };
    let a0 = coordinate(a_start);
    let a1 = coordinate(a_end);
    let b0 = coordinate(b_start);
    let b1 = coordinate(b_end);
    let a_min = a0.min(a1);
    let a_max = a0.max(a1);
    let b_min = b0.min(b1);
    let b_max = b0.max(b1);
    a_min.max(b_min) < a_max.min(b_max)
}

fn arrangement_region_centroid(
    arrangement: &NodeArrangement,
    region: &super::arrangement::NodeOwnedRegion,
) -> Option<Vector3> {
    let mut sum = Vector3::ZERO;
    let mut count = 0usize;
    for vertex_id in region.outer_loop() {
        let Some(point) = RoadSurfaceSystem::arrangement_vertex_world(arrangement, *vertex_id)
        else {
            continue;
        };
        sum += Vector3::new(point.x, 0.0, point.z);
        count += 1;
    }
    (count > 0).then_some(sum / count as f32)
}

pub(super) fn visible_top_boundary_height_mm_at_key(
    top_polygons: &[&RoadSurfaceVisualPolygon],
    key: NodeArrangementKey,
) -> Option<i64> {
    let mut heights = Vec::new();
    for polygon in top_polygons {
        append_boundary_loop_heights_at_key(&mut heights, &polygon.points_world, key);
        for triangle in &polygon.triangles_world {
            append_boundary_loop_heights_at_key(&mut heights, triangle, key);
        }
    }
    heights.sort_unstable();
    heights.dedup();
    heights.into_iter().max()
}

fn append_boundary_loop_heights_at_key(
    heights: &mut Vec<i64>,
    points: &[Vector3],
    key: NodeArrangementKey,
) {
    if points.len() < 2 {
        return;
    }
    let point = arrangement_key_boundary_point(key, 0);
    for index in 0..points.len() {
        let start = ArrangementBoundaryPointKey::from_world(points[index]);
        let end = ArrangementBoundaryPointKey::from_world(points[(index + 1) % points.len()]);
        if !arrangement_key_lies_on_segment(key, start.xz_key(), end.xz_key()) {
            continue;
        }
        let Some(parameter) = boundary_segment_parameter_xz(point, start, end) else {
            continue;
        };
        heights.push(interpolated_segment_height_mm(start, end, parameter));
    }
}
