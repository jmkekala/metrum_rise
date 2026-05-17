//! Exact source handoff and canonical contour-edge support helpers.

use super::model::*;
use super::vertices::canonical_height_vertices;
use super::*;

const AUTHORIZED_CONTOUR_SUPPORT_DRIFT_UNITS: i128 =
    (WORLD_POINT_DEDUP_DISTANCE_M as f64 * HEIGHT_SOURCE_KEY_SCALE + 0.5) as i128;

pub(super) fn height_edges_from_vertices(
    points: &[RoadVec3],
) -> Result<Vec<NodeBandHeightEdge>, HeightCarrierContourError> {
    let vertices = canonical_height_vertices(points)?;
    if vertices.len() < 2 {
        return Ok(Vec::new());
    }

    let mut edges = Vec::with_capacity(vertices.len());
    for index in 0..vertices.len() {
        let (start_xz, start_height_m) = vertices[index];
        let (end_xz, end_height_m) = vertices[(index + 1) % vertices.len()];
        if height_source_point_key(start_xz) == height_source_point_key(end_xz) {
            continue;
        }
        edges.push(NodeBandHeightEdge {
            start_xz,
            end_xz,
            start_height_m,
            end_height_m,
        });
    }
    Ok(edges)
}

pub(super) fn terminal_edge_height_at(
    point_xz: RoadVec2,
    edges: &[NodeBandHeightEdge],
    support_keys: &BTreeSet<NodeHeightSourcePointKey>,
) -> Result<Option<f64>, NodeContourEdgeHeightConflict> {
    let point = height_source_point_key(point_xz);
    if !support_keys.contains(&point) {
        return Ok(None);
    }
    terminal_edge_height_at_with(
        point_xz,
        edges,
        height_key_has_authorized_contour_edge_support,
    )
}

pub(super) fn terminal_edge_height_at_exact(
    point_xz: RoadVec2,
    edges: &[NodeBandHeightEdge],
) -> Result<Option<f64>, NodeContourEdgeHeightConflict> {
    terminal_edge_height_at_with(point_xz, edges, height_key_has_source_edge_support)
}

fn terminal_edge_height_at_with(
    point_xz: RoadVec2,
    edges: &[NodeBandHeightEdge],
    has_support: impl Fn(
        NodeHeightSourcePointKey,
        NodeHeightSourcePointKey,
        NodeHeightSourcePointKey,
    ) -> bool,
) -> Result<Option<f64>, NodeContourEdgeHeightConflict> {
    let point = height_source_point_key(point_xz);
    let mut selected_height = None;
    let mut selected_height_mm = None;
    for edge in edges {
        let start = height_source_point_key(edge.start_xz);
        let end = height_source_point_key(edge.end_xz);
        if !has_support(point, start, end) {
            continue;
        }
        let dx = end.0 - start.0;
        let dz = end.1 - start.1;
        let denominator = if dx.abs() >= dz.abs() { dx } else { dz };
        if denominator == 0 {
            continue;
        }
        let numerator = if dx.abs() >= dz.abs() {
            point.0 - start.0
        } else {
            point.1 - start.1
        };
        let t = numerator as f64 / denominator as f64;
        if !(0.0..=1.0).contains(&t) {
            continue;
        }
        let height_m = edge.start_height_m + (edge.end_height_m - edge.start_height_m) * t;
        let height_mm = quantize_m(height_m);
        if let Some(existing_height_mm) = selected_height_mm {
            if existing_height_mm != height_mm {
                return Err(NodeContourEdgeHeightConflict {
                    existing_height_mm,
                    incoming_height_mm: height_mm,
                });
            }
            continue;
        }
        selected_height = Some(height_m);
        selected_height_mm = Some(height_mm);
    }
    Ok(selected_height)
}

pub(super) fn height_key_has_source_edge_support(
    point: NodeHeightSourcePointKey,
    start: NodeHeightSourcePointKey,
    end: NodeHeightSourcePointKey,
) -> bool {
    raw_tuple_key_lies_exactly_on_segment(point, start, end)
}

fn height_key_has_authorized_contour_edge_support(
    point: NodeHeightSourcePointKey,
    start: NodeHeightSourcePointKey,
    end: NodeHeightSourcePointKey,
) -> bool {
    height_key_has_source_edge_support(point, start, end)
        || authorized_support_cell_intersects_segment(point, start, end)
}

pub(super) fn height_source_point_key(point: RoadVec2) -> NodeHeightSourcePointKey {
    SurfaceXzKey::from_road_xz(point).raw_tuple()
}

fn authorized_support_cell_intersects_segment(
    point: NodeHeightSourcePointKey,
    start: NodeHeightSourcePointKey,
    end: NodeHeightSourcePointKey,
) -> bool {
    if start == end {
        return false;
    }
    let drift_x2 = AUTHORIZED_CONTOUR_SUPPORT_DRIFT_UNITS * 2;
    let min_x2 = i128::from(point.0) * 2 - drift_x2;
    let max_x2 = i128::from(point.0) * 2 + drift_x2;
    let min_z2 = i128::from(point.1) * 2 - drift_x2;
    let max_z2 = i128::from(point.1) * 2 + drift_x2;
    let segment_start = doubled_key(start);
    let segment_end = doubled_key(end);
    if doubled_point_inside_axis_aligned_box(segment_start, min_x2, max_x2, min_z2, max_z2)
        || doubled_point_inside_axis_aligned_box(segment_end, min_x2, max_x2, min_z2, max_z2)
    {
        return true;
    }
    let lower_left = (min_x2, min_z2);
    let lower_right = (max_x2, min_z2);
    let upper_right = (max_x2, max_z2);
    let upper_left = (min_x2, max_z2);
    [
        (lower_left, lower_right),
        (lower_right, upper_right),
        (upper_right, upper_left),
        (upper_left, lower_left),
    ]
    .into_iter()
    .any(|(edge_start, edge_end)| {
        doubled_segments_intersect(segment_start, segment_end, edge_start, edge_end)
    })
}

fn doubled_key(point: NodeHeightSourcePointKey) -> (i128, i128) {
    (i128::from(point.0) * 2, i128::from(point.1) * 2)
}

fn doubled_point_inside_axis_aligned_box(
    point: (i128, i128),
    min_x: i128,
    max_x: i128,
    min_z: i128,
    max_z: i128,
) -> bool {
    point.0 >= min_x && point.0 <= max_x && point.1 >= min_z && point.1 <= max_z
}

fn doubled_segments_intersect(
    a: (i128, i128),
    b: (i128, i128),
    c: (i128, i128),
    d: (i128, i128),
) -> bool {
    let ab_c = doubled_triangle_area2(a, b, c);
    let ab_d = doubled_triangle_area2(a, b, d);
    let cd_a = doubled_triangle_area2(c, d, a);
    let cd_b = doubled_triangle_area2(c, d, b);
    if ab_c == 0 && doubled_point_on_segment(c, a, b) {
        return true;
    }
    if ab_d == 0 && doubled_point_on_segment(d, a, b) {
        return true;
    }
    if cd_a == 0 && doubled_point_on_segment(a, c, d) {
        return true;
    }
    if cd_b == 0 && doubled_point_on_segment(b, c, d) {
        return true;
    }
    (ab_c > 0) != (ab_d > 0) && (cd_a > 0) != (cd_b > 0)
}

fn doubled_triangle_area2(a: (i128, i128), b: (i128, i128), c: (i128, i128)) -> i128 {
    let ab_x = b.0 - a.0;
    let ab_z = b.1 - a.1;
    let ac_x = c.0 - a.0;
    let ac_z = c.1 - a.1;
    ab_x * ac_z - ab_z * ac_x
}

fn doubled_point_on_segment(point: (i128, i128), start: (i128, i128), end: (i128, i128)) -> bool {
    point.0 >= start.0.min(end.0)
        && point.0 <= start.0.max(end.0)
        && point.1 >= start.1.min(end.1)
        && point.1 <= start.1.max(end.1)
}
