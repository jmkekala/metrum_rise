//! Height source-edge support and exact handoff helpers.

use super::model::*;
use super::seams::*;
use super::*;

pub(super) fn height_edges_from_vertices(points: &[RoadVec3]) -> Vec<NodeBandHeightEdge> {
    let mut vertices = Vec::with_capacity(points.len());
    for point in points {
        let point_xz = quantize_road_vec2_to_overlay_grid(xz(*point));
        let key = height_source_point_key(point_xz);
        if vertices
            .last()
            .is_some_and(|(last_xz, _)| height_source_point_key(*last_xz) == key)
        {
            continue;
        }
        vertices.push((point_xz, quantize_source_height_m(point.y)));
    }
    if vertices.len() > 1
        && height_source_point_key(vertices[0].0)
            == height_source_point_key(vertices.last().expect("height vertices are non-empty").0)
    {
        vertices.pop();
    }
    if vertices.len() < 2 {
        return Vec::new();
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
    edges
}

pub(super) fn terminal_edge_height_at(
    point_xz: RoadVec2,
    edges: &[NodeBandHeightEdge],
) -> Option<f64> {
    let point = height_source_point_key(point_xz);
    for edge in edges {
        let start = height_source_point_key(edge.start_xz);
        let end = height_source_point_key(edge.end_xz);
        if !height_key_has_source_edge_support(point, start, end) {
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
        return Some(edge.start_height_m + (edge.end_height_m - edge.start_height_m) * t);
    }
    None
}

pub(super) fn height_key_has_source_edge_support(
    point: NodeHeightSourcePointKey,
    start: NodeHeightSourcePointKey,
    end: NodeHeightSourcePointKey,
) -> bool {
    // Exact segment membership is the normal contract. The cell-intersection branch is limited to
    // WORLD_POINT_DEDUP_DISTANCE_M so independently quantized copies of the same handoff vertex can
    // agree with their source rail without becoming a general near-edge fallback.
    raw_tuple_key_lies_exactly_on_segment(point, start, end)
        || raw_tuple_quantization_cell_intersects_segment(
            point,
            start,
            end,
            HEIGHT_SOURCE_EDGE_DEDUP_DRIFT_UNITS,
        )
}

pub(super) fn height_source_point_key(point: RoadVec2) -> NodeHeightSourcePointKey {
    SurfaceXzKey::from_road_xz(point).raw_tuple()
}
