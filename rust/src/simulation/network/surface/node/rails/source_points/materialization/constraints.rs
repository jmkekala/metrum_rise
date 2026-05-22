//! Height-carrier point materialization from source-backed rail constraints.

use super::super::super::super::backend::RoadVec3;
use super::super::super::contours::height_for_key_on_generated_edge;
use super::super::super::geometry::{road_point_from_key, road_point_key};
use super::super::super::topology::NodeRailPointKey;
use super::super::super::{NodeRailConstraint, NodeRailConstraintKind, NodeRailGenerationError};
use super::super::NodeRailHeightSourceKey;
use super::super::collection::{
    known_height, push_materialized_height_carrier_points,
    unambiguous_source_height_points_by_key_subset,
};
use std::collections::BTreeMap;

pub(in crate::simulation::network::surface::node::rails) fn push_source_constraint_height_carrier_points(
    points_by_source: &mut BTreeMap<NodeRailHeightSourceKey, Vec<RoadVec3>>,
    constraints: &[NodeRailConstraint],
) -> Result<(), NodeRailGenerationError> {
    let mut materialized = Vec::new();
    for constraint in constraints {
        if constraint.kind == NodeRailConstraintKind::RaisedStepContact {
            continue;
        }
        let (Some(owner), Some(source_band_index)) =
            (constraint.owner, constraint.source_band_index)
        else {
            continue;
        };
        let source = (
            owner.kind(),
            constraint.source_mouth_order_index,
            source_band_index,
        );
        let Some(source_points) = points_by_source.get(&source) else {
            continue;
        };
        let keys = constraint_point_keys(constraint);
        let source_heights_by_key =
            unambiguous_source_height_points_by_key_subset(source_points, &keys);
        materialized.extend(
            materialized_constraint_height_points(&keys, &source_heights_by_key)
                .into_iter()
                .map(|point| (source, point)),
        );
    }
    push_materialized_height_carrier_points(points_by_source, materialized)
}

fn materialized_constraint_height_points(
    keys: &[NodeRailPointKey],
    heights_by_key: &BTreeMap<NodeRailPointKey, f64>,
) -> Vec<RoadVec3> {
    let mut points = Vec::new();
    for (index, point) in keys.iter().copied().enumerate() {
        if known_height(heights_by_key, point).is_some() {
            continue;
        }
        let Some((start, start_height_m, end, end_height_m)) =
            surrounding_known_height_segment(keys, heights_by_key, index)
        else {
            continue;
        };
        let Some(height_m) =
            height_for_key_on_generated_edge(point, start, end, start_height_m, end_height_m)
        else {
            continue;
        };
        let point_xz = road_point_from_key(point);
        points.push(RoadVec3::new(point_xz.x, height_m, point_xz.y));
    }
    points
}

fn constraint_point_keys(constraint: &NodeRailConstraint) -> Vec<NodeRailPointKey> {
    constraint
        .points_xz
        .iter()
        .copied()
        .map(road_point_key)
        .collect()
}

fn surrounding_known_height_segment(
    keys: &[NodeRailPointKey],
    heights_by_key: &BTreeMap<NodeRailPointKey, f64>,
    index: usize,
) -> Option<(NodeRailPointKey, f64, NodeRailPointKey, f64)> {
    let (start, start_height_m) = (0..index).rev().find_map(|candidate_index| {
        let point = keys[candidate_index];
        known_height(heights_by_key, point).map(|height_m| (point, height_m))
    })?;
    let (end, end_height_m) = (index + 1..keys.len()).find_map(|candidate_index| {
        let point = keys[candidate_index];
        known_height(heights_by_key, point).map(|height_m| (point, height_m))
    })?;
    Some((start, start_height_m, end, end_height_m))
}

#[cfg(test)]
mod tests {
    use super::super::super::super::super::RoadSurfaceBandKind;
    use super::super::super::super::super::arrangement::NodeBandOwner;
    use super::super::super::super::super::backend::{RoadVec2, road_vec3_xz as xz};
    use super::super::super::super::super::keys::SurfaceHeightMmKey;
    use super::*;

    #[test]
    fn source_constraint_height_carrier_skips_ambiguous_key_without_suppressing_unrelated_points() {
        let owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let mut points_by_source = BTreeMap::new();
        points_by_source.insert(
            (RoadSurfaceBandKind::Carriageway, 2, 4),
            vec![RoadVec3::new(1.0, 0.5, 3.0), RoadVec3::new(1.0, 0.75, 3.0)],
        );
        points_by_source
            .get_mut(&(RoadSurfaceBandKind::Carriageway, 2, 4))
            .expect("source support was inserted")
            .extend([RoadVec3::new(3.0, 3.0, 3.0), RoadVec3::new(5.0, 5.0, 3.0)]);
        let constraints = [NodeRailConstraint {
            constraint_index: 0,
            kind: NodeRailConstraintKind::BandContour {
                kind: RoadSurfaceBandKind::Carriageway,
            },
            source_mouth_order_index: 2,
            source_band_index: Some(4),
            source_boundary_index: None,
            owner: Some(owner),
            opposite_owner: None,
            points_xz: vec![
                RoadVec2::new(1.0, 3.0),
                RoadVec2::new(2.0, 3.0),
                RoadVec2::new(3.0, 3.0),
                RoadVec2::new(4.0, 3.0),
                RoadVec2::new(5.0, 3.0),
            ],
        }];

        push_source_constraint_height_carrier_points(&mut points_by_source, &constraints)
            .expect("unambiguous source keys should still materialize");

        let points = points_by_source
            .get(&(RoadSurfaceBandKind::Carriageway, 2, 4))
            .expect("source support should remain present");
        assert!(points.iter().any(|point| road_point_key(xz(*point))
            == road_point_key(RoadVec2::new(4.0, 3.0))
            && SurfaceHeightMmKey::from_m_f64(point.y).as_i64() == 4000));
        assert!(!points.iter().any(|point| {
            road_point_key(xz(*point)) == road_point_key(RoadVec2::new(2.0, 3.0))
        }));
    }
}
