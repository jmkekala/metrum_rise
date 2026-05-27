//! Canonical height-carrier vertex normalization.

use super::model::*;
use super::seams::quantize_source_height_m;
use super::source_edges::height_source_point_key;
use super::*;

pub(super) fn canonical_height_vertices(
    points: &[RoadVec3],
) -> Result<Vec<(RoadVec2, f64)>, HeightCarrierContourError> {
    let mut vertices = Vec::with_capacity(points.len());
    let mut heights_by_key = BTreeMap::<NodeHeightSourcePointKey, SurfaceHeightMmKey>::new();
    for point in points {
        let point_xz = quantize_road_vec2_to_overlay_grid(xz(*point));
        let key = height_source_point_key(point_xz);
        let height_key = SurfaceHeightMmKey::from_m_f64(point.y);
        if let Some(existing_height_key) = heights_by_key.get(&key) {
            if *existing_height_key != height_key {
                return Err(HeightCarrierContourError::ConflictingDuplicateHeightVertex);
            }
        } else {
            heights_by_key.insert(key, height_key);
        }
        let height_m = quantize_source_height_m(point.y);
        if vertices
            .last()
            .is_some_and(|(last_xz, _)| height_source_point_key(*last_xz) == key)
        {
            continue;
        }
        vertices.push((point_xz, height_m));
    }
    if vertices.len() > 1
        && height_source_point_key(vertices[0].0)
            == height_source_point_key(vertices.last().expect("height vertices are non-empty").0)
    {
        vertices.pop();
    }
    Ok(vertices)
}

pub(super) fn height_vertex_heights_from_vertices(
    points: &[RoadVec3],
) -> Result<BTreeMap<NodeHeightSourcePointKey, f64>, HeightCarrierContourError> {
    Ok(canonical_height_vertices(points)?
        .into_iter()
        .map(|(point_xz, height_m)| (height_source_point_key(point_xz), height_m))
        .collect())
}

pub(super) fn closed_height_contour_edges_from_vertices(
    points: &[RoadVec3],
) -> Result<Vec<NodeBandHeightContourEdge>, HeightCarrierContourError> {
    let vertices = canonical_height_vertices(points)?;
    Ok(height_contour_edges_from_canonical_vertices(
        &vertices, true,
    ))
}

pub(super) fn open_height_contour_edges_from_vertices(
    points: &[RoadVec3],
) -> Result<Vec<NodeBandHeightContourEdge>, HeightCarrierContourError> {
    let vertices = canonical_height_vertices(points)?;
    Ok(height_contour_edges_from_canonical_vertices(
        &vertices, false,
    ))
}

fn height_contour_edges_from_canonical_vertices(
    vertices: &[(RoadVec2, f64)],
    closed: bool,
) -> Vec<NodeBandHeightContourEdge> {
    let mut edges = Vec::new();
    if vertices.len() < 2 {
        return edges;
    }
    for segment in vertices.windows(2) {
        push_height_contour_edge(&mut edges, segment[0], segment[1]);
    }
    if closed {
        push_height_contour_edge(
            &mut edges,
            *vertices.last().expect("len checked"),
            vertices[0],
        );
    }
    edges
}

pub(super) fn push_height_contour_edge(
    edges: &mut Vec<NodeBandHeightContourEdge>,
    start: (RoadVec2, f64),
    end: (RoadVec2, f64),
) {
    let start_key = height_source_point_key(start.0);
    let end_key = height_source_point_key(end.0);
    if start_key == end_key {
        return;
    }
    let edge = NodeBandHeightContourEdge {
        start: start_key,
        end: end_key,
        start_height_mm: SurfaceHeightMmKey::from_m_f64(start.1).as_i64(),
        end_height_mm: SurfaceHeightMmKey::from_m_f64(end.1).as_i64(),
    };
    if !edges.iter().any(|existing| {
        existing.start == edge.start
            && existing.end == edge.end
            && existing.start_height_mm == edge.start_height_mm
            && existing.end_height_mm == edge.end_height_mm
    }) {
        edges.push(edge);
    }
}
