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

pub(super) fn validate_canonical_height_vertices(
    points: &[RoadVec3],
) -> Result<(), HeightCarrierContourError> {
    canonical_height_vertices(points).map(|_| ())
}

pub(super) fn height_vertex_heights_from_vertices(
    points: &[RoadVec3],
) -> Result<BTreeMap<NodeHeightSourcePointKey, f64>, HeightCarrierContourError> {
    Ok(canonical_height_vertices(points)?
        .into_iter()
        .map(|(point_xz, height_m)| (height_source_point_key(point_xz), height_m))
        .collect())
}
