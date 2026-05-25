//! Source height-carrier point collection and conflict detection.

use super::super::super::RoadSurfaceBandKind;
use super::super::super::backend::{RoadVec3, road_vec3_xz as xz};
use super::super::super::keys::SurfaceHeightMmKey;
use super::super::NodeRailGenerationError;
use super::super::geometry::road_point_key;
use super::super::topology::NodeRailPointKey;
use super::NodeRailHeightSourceKey;
use std::collections::BTreeMap;

pub(in crate::simulation::network::surface::node::rails) fn push_band_height_carrier_points(
    points_by_source: &mut BTreeMap<NodeRailHeightSourceKey, Vec<RoadVec3>>,
    mouth_order_index: usize,
    source_band_index: usize,
    kind: RoadSurfaceBandKind,
    points_world: impl IntoIterator<Item = RoadVec3>,
) -> Result<(), NodeRailGenerationError> {
    let points = points_by_source
        .entry((kind, mouth_order_index, source_band_index))
        .or_default();
    for point in points_world {
        let point_key = road_point_key(xz(point));
        if let Some(existing) = points
            .iter()
            .find(|existing| road_point_key(xz(**existing)) == point_key)
        {
            if SurfaceHeightMmKey::from_m_f64(existing.y) == SurfaceHeightMmKey::from_m_f64(point.y)
            {
                continue;
            }
        }
        points.push(point);
    }
    Ok(())
}

pub(super) fn push_materialized_height_carrier_points(
    points_by_source: &mut BTreeMap<NodeRailHeightSourceKey, Vec<RoadVec3>>,
    materialized: Vec<(NodeRailHeightSourceKey, RoadVec3)>,
) -> Result<(), NodeRailGenerationError> {
    for ((kind, mouth_order_index, source_band_index), point) in materialized {
        push_band_height_carrier_points(
            points_by_source,
            mouth_order_index,
            source_band_index,
            kind,
            [point],
        )?;
    }
    Ok(())
}

pub(super) fn source_height_points_by_key(
    source: NodeRailHeightSourceKey,
    points: &[RoadVec3],
) -> Result<BTreeMap<NodeRailPointKey, f64>, NodeRailGenerationError> {
    source_height_points_by_matching_key(source, points, |_| true)
}

fn source_height_points_by_matching_key(
    source: NodeRailHeightSourceKey,
    points: &[RoadVec3],
    mut include_key: impl FnMut(NodeRailPointKey) -> bool,
) -> Result<BTreeMap<NodeRailPointKey, f64>, NodeRailGenerationError> {
    let mut heights_by_key = BTreeMap::<NodeRailPointKey, f64>::new();
    for point in points {
        let key = road_point_key(xz(*point));
        if !include_key(key) {
            continue;
        }
        match heights_by_key.get_mut(&key) {
            Some(existing_height_m)
                if SurfaceHeightMmKey::from_m_f64(*existing_height_m)
                    == SurfaceHeightMmKey::from_m_f64(point.y) => {}
            Some(existing_height_m) => {
                return Err(conflicting_height_carrier_point_error(
                    source,
                    key,
                    *existing_height_m,
                    point.y,
                ));
            }
            None => {
                heights_by_key.insert(key, point.y);
            }
        }
    }
    Ok(heights_by_key)
}

pub(super) fn source_has_height_point(
    points_by_source: &BTreeMap<NodeRailHeightSourceKey, Vec<RoadVec3>>,
    source: NodeRailHeightSourceKey,
    point: NodeRailPointKey,
) -> bool {
    points_by_source.get(&source).is_some_and(|points| {
        points
            .iter()
            .any(|source_point| road_point_key(xz(*source_point)) == point)
    })
}

pub(super) fn conflicting_height_carrier_point_error(
    source: NodeRailHeightSourceKey,
    point: NodeRailPointKey,
    existing_height_m: f64,
    incoming_height_m: f64,
) -> NodeRailGenerationError {
    NodeRailGenerationError::ConflictingHeightCarrierPoint {
        kind: source.0,
        mouth_order_index: source.1,
        band_index: source.2,
        point_x_key: point.0,
        point_z_key: point.1,
        existing_height_mm: SurfaceHeightMmKey::from_m_f64(existing_height_m).as_i64(),
        incoming_height_mm: SurfaceHeightMmKey::from_m_f64(incoming_height_m).as_i64(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::super::RoadSurfaceBandKind;
    use super::*;

    #[test]
    fn band_height_carrier_preserves_conflicting_duplicate_key_for_height_validation() {
        let mut points_by_source = BTreeMap::new();

        push_band_height_carrier_points(
            &mut points_by_source,
            2,
            4,
            RoadSurfaceBandKind::Sidewalk,
            [RoadVec3::new(1.0, 0.5, 3.0), RoadVec3::new(1.0, 0.75, 3.0)],
        )
        .expect("raw duplicate support must be preserved for height-stage rejection");

        let points = points_by_source
            .get(&(RoadSurfaceBandKind::Sidewalk, 2, 4))
            .expect("source support should be recorded");
        assert_eq!(points.len(), 2);
    }

    #[test]
    fn source_height_points_by_key_rejects_conflict_instead_of_dropping_key() {
        let source = (RoadSurfaceBandKind::Carriageway, 1, 0);
        let points = [RoadVec3::new(2.0, 1.0, 0.0), RoadVec3::new(2.0, 1.25, 0.0)];

        let error = source_height_points_by_key(source, &points)
            .expect_err("conflicting source support must not become missing support");

        assert!(matches!(
            error,
            NodeRailGenerationError::ConflictingHeightCarrierPoint {
                kind: RoadSurfaceBandKind::Carriageway,
                mouth_order_index: 1,
                band_index: 0,
                existing_height_mm: 1000,
                incoming_height_mm: 1250,
                ..
            }
        ));
    }
}
