//! Canonical boundary edge extraction and overlap checks for node export.

use super::*;

pub(super) fn normalized_arrangement_boundary_segment_key(
    start: Vector3,
    end: Vector3,
) -> (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey) {
    let start = ArrangementBoundaryPointKey::from_world(start);
    let end = ArrangementBoundaryPointKey::from_world(end);
    if start <= end {
        (start, end)
    } else {
        (end, start)
    }
}

pub(super) fn node_owned_region_matches_owner(
    region: &NodeOwnedRegion,
    owner: NodeBandOwner,
) -> bool {
    region.kind == owner.kind() && region.owner_index == owner.owner_index()
}

pub(super) fn visual_polygon_boundary_overlaps_edge_at_height(
    polygon: &RoadSurfaceVisualPolygon,
    edge: [Vector3; 2],
) -> bool {
    if !polygon.triangles_world.is_empty() {
        let mut triangle_edges = BTreeMap::<
            (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
            (usize, [Vector3; 2]),
        >::new();
        for triangle in &polygon.triangles_world {
            for edge_index in 0..3 {
                let start = triangle[edge_index];
                let end = triangle[(edge_index + 1) % 3];
                if ArrangementBoundaryPointKey::from_world(start).xz_key()
                    == ArrangementBoundaryPointKey::from_world(end).xz_key()
                {
                    continue;
                }
                triangle_edges
                    .entry(normalized_arrangement_boundary_segment_key(start, end))
                    .and_modify(|entry| entry.0 += 1)
                    .or_insert((1, [start, end]));
            }
        }
        return triangle_edges
            .into_values()
            .filter_map(|(count, boundary_edge)| (count == 1).then_some(boundary_edge))
            .any(|boundary_edge| boundary_edge_contains_edge_at_height(boundary_edge, edge));
    }

    let points = &polygon.points_world;
    if points.len() < 2 {
        return false;
    }
    (0..points.len()).any(|index| {
        let start = points[index];
        let end = points[(index + 1) % points.len()];
        boundary_edge_contains_edge_at_height([start, end], edge)
    })
}

pub(super) fn visual_polygon_boundary_overlaps_edge_xz(
    polygon: &RoadSurfaceVisualPolygon,
    edge: [Vector3; 2],
) -> bool {
    if !polygon.triangles_world.is_empty() {
        let mut triangle_edges = BTreeMap::<
            (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
            (usize, [Vector3; 2]),
        >::new();
        for triangle in &polygon.triangles_world {
            for edge_index in 0..3 {
                let start = triangle[edge_index];
                let end = triangle[(edge_index + 1) % 3];
                if ArrangementBoundaryPointKey::from_world(start).xz_key()
                    == ArrangementBoundaryPointKey::from_world(end).xz_key()
                {
                    continue;
                }
                triangle_edges
                    .entry(normalized_arrangement_boundary_segment_key(start, end))
                    .and_modify(|entry| entry.0 += 1)
                    .or_insert((1, [start, end]));
            }
        }
        return triangle_edges
            .into_values()
            .filter_map(|(count, boundary_edge)| (count == 1).then_some(boundary_edge))
            .any(|boundary_edge| boundary_edge_contains_edge_xz(boundary_edge, edge));
    }

    let points = &polygon.points_world;
    if points.len() < 2 {
        return false;
    }
    (0..points.len()).any(|index| {
        let start = points[index];
        let end = points[(index + 1) % points.len()];
        boundary_edge_contains_edge_xz([start, end], edge)
    })
}

pub(super) fn boundary_edge_contains_edge_xz(
    boundary_edge: [Vector3; 2],
    edge: [Vector3; 2],
) -> bool {
    let boundary_start = ArrangementBoundaryPointKey::from_world(boundary_edge[0]);
    let boundary_end = ArrangementBoundaryPointKey::from_world(boundary_edge[1]);
    let edge_start = ArrangementBoundaryPointKey::from_world(edge[0]);
    let edge_end = ArrangementBoundaryPointKey::from_world(edge[1]);
    if !arrangement_segments_exact_overlap_with_length(
        boundary_start.xz_key(),
        boundary_end.xz_key(),
        edge_start.xz_key(),
        edge_end.xz_key(),
    ) {
        return false;
    }
    let Some(start_parameter) =
        boundary_segment_parameter_xz(edge_start, boundary_start, boundary_end)
    else {
        return false;
    };
    let Some(end_parameter) = boundary_segment_parameter_xz(edge_end, boundary_start, boundary_end)
    else {
        return false;
    };
    start_parameter >= ArrangementSegmentParameter::zero()
        && start_parameter <= ArrangementSegmentParameter::one()
        && end_parameter >= ArrangementSegmentParameter::zero()
        && end_parameter <= ArrangementSegmentParameter::one()
}

pub(super) fn boundary_edge_contains_edge_at_height(
    boundary_edge: [Vector3; 2],
    edge: [Vector3; 2],
) -> bool {
    let boundary_start = ArrangementBoundaryPointKey::from_world(boundary_edge[0]);
    let boundary_end = ArrangementBoundaryPointKey::from_world(boundary_edge[1]);
    let edge_start = ArrangementBoundaryPointKey::from_world(edge[0]);
    let edge_end = ArrangementBoundaryPointKey::from_world(edge[1]);
    if !arrangement_segments_exact_overlap_with_length(
        boundary_start.xz_key(),
        boundary_end.xz_key(),
        edge_start.xz_key(),
        edge_end.xz_key(),
    ) {
        return false;
    }
    let Some(start_parameter) =
        boundary_segment_parameter_xz(edge_start, boundary_start, boundary_end)
    else {
        return false;
    };
    let Some(end_parameter) = boundary_segment_parameter_xz(edge_end, boundary_start, boundary_end)
    else {
        return false;
    };
    if start_parameter < ArrangementSegmentParameter::zero()
        || start_parameter > ArrangementSegmentParameter::one()
        || end_parameter < ArrangementSegmentParameter::zero()
        || end_parameter > ArrangementSegmentParameter::one()
    {
        return false;
    }
    (interpolated_segment_height_mm(boundary_start, boundary_end, start_parameter)
        - edge_start.y_mm)
        .abs()
        <= 1
        && (interpolated_segment_height_mm(boundary_start, boundary_end, end_parameter)
            - edge_end.y_mm)
            .abs()
            <= 1
}

pub(super) fn arrangement_segments_exact_overlap_with_length(
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
    if a_dx * b_dz - a_dz * b_dx != 0 {
        return false;
    }
    if !arrangement_key_lies_exactly_on_segment(a_start, b_start, b_end)
        && !arrangement_key_lies_exactly_on_segment(a_end, b_start, b_end)
        && !arrangement_key_lies_exactly_on_segment(b_start, a_start, a_end)
        && !arrangement_key_lies_exactly_on_segment(b_end, a_start, a_end)
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
    a0.min(a1).max(b0.min(b1)) < a0.max(a1).min(b0.max(b1))
}
