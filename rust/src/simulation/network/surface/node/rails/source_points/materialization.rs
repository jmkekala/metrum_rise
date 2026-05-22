//! Height-carrier point materialization from solved rail constraints and owned regions.

use super::super::super::backend::{RoadVec3, overlay_point_to_road, road_vec3_xz as xz};
use super::super::super::keys::SurfaceHeightMmKey;
use super::super::super::ownership::NodeBooleanOwnership;
use super::super::contours::height_for_key_on_generated_edge;
use super::super::geometry::{road_point_from_key, road_point_key};
use super::super::topology::NodeRailPointKey;
use super::super::{
    NodeRailConstraint, NodeRailConstraintKind, NodeRailGenerationError, NodeRailHeightCarrierPaths,
};
use super::NodeRailHeightSourceKey;
use super::collection::{
    conflicting_height_carrier_point_error, known_height, push_materialized_height_carrier_points,
    source_has_height_point, source_height_points_by_key,
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

pub(in crate::simulation::network::surface::node::rails) fn push_owned_region_height_carrier_points(
    points_by_source: &mut BTreeMap<NodeRailHeightSourceKey, Vec<RoadVec3>>,
    constraints: &[NodeRailConstraint],
    paths_by_source: &BTreeMap<NodeRailHeightSourceKey, NodeRailHeightCarrierPaths>,
    ownership: &NodeBooleanOwnership,
) -> Result<(), NodeRailGenerationError> {
    let mut materialized = Vec::new();
    for region in &ownership.owned_regions {
        let Some(source_band_index) = region.source_band_index else {
            continue;
        };
        let source = (
            region.kind,
            region.source_mouth_order_index,
            source_band_index,
        );
        for point_xz in region
            .shape
            .iter()
            .flat_map(|contour| contour.iter().copied())
            .map(overlay_point_to_road)
        {
            let point = road_point_key(point_xz);
            if source_has_height_point(points_by_source, source, point) {
                continue;
            }
            let Some(height_m) = height_for_source_key(
                source,
                point,
                constraints,
                paths_by_source,
                points_by_source,
            )?
            else {
                continue;
            };
            materialized.push((source, RoadVec3::new(point_xz.x, height_m, point_xz.y)));
        }
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
            surrounding_known_height_segment(&keys, heights_by_key, index)
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

fn height_for_source_key(
    source: NodeRailHeightSourceKey,
    point: NodeRailPointKey,
    constraints: &[NodeRailConstraint],
    paths_by_source: &BTreeMap<NodeRailHeightSourceKey, NodeRailHeightCarrierPaths>,
    points_by_source: &BTreeMap<NodeRailHeightSourceKey, Vec<RoadVec3>>,
) -> Result<Option<f64>, NodeRailGenerationError> {
    let mut selected_height_m = None;
    if let Some(paths) = paths_by_source.get(&source) {
        let mut contour_world =
            Vec::with_capacity(paths.start_path_world.len() + paths.end_path_world.len());
        contour_world.extend(paths.start_path_world.iter().copied());
        contour_world.extend(paths.end_path_world.iter().rev().copied());
        collect_candidate_height(
            source,
            point,
            &mut selected_height_m,
            height_on_world_points_key(point, &contour_world, true),
        )?;
    }
    let source_heights_by_key = match points_by_source.get(&source) {
        Some(points) => source_height_points_by_key(source, points)?,
        None => BTreeMap::new(),
    };
    for constraint in constraints {
        if constraint.kind == NodeRailConstraintKind::RaisedStepContact {
            continue;
        }
        if (
            constraint.owner.map(|owner| owner.kind()),
            constraint.source_mouth_order_index,
            constraint.source_band_index,
        ) != (Some(source.0), source.1, Some(source.2))
        {
            continue;
        }
        collect_candidate_height(
            source,
            point,
            &mut selected_height_m,
            height_on_constraint_key(point, constraint, &source_heights_by_key),
        )?;
    }
    Ok(selected_height_m)
}

fn collect_candidate_height(
    source: NodeRailHeightSourceKey,
    point: NodeRailPointKey,
    selected_height_m: &mut Option<f64>,
    candidate: Option<f64>,
) -> Result<(), NodeRailGenerationError> {
    let Some(candidate_height_m) = candidate else {
        return Ok(());
    };
    if let Some(selected_height_m) = selected_height_m {
        if SurfaceHeightMmKey::from_m_f64(*selected_height_m)
            != SurfaceHeightMmKey::from_m_f64(candidate_height_m)
        {
            return Err(conflicting_height_carrier_point_error(
                source,
                point,
                *selected_height_m,
                candidate_height_m,
            ));
        }
    } else {
        *selected_height_m = Some(candidate_height_m);
    }
    Ok(())
}

fn height_on_world_points_key(
    point: NodeRailPointKey,
    points_world: &[RoadVec3],
    closed: bool,
) -> Option<f64> {
    if points_world.is_empty() {
        return None;
    }
    for source_point in points_world {
        if road_point_key(xz(*source_point)) == point {
            return Some(source_point.y);
        }
    }
    for segment in points_world.windows(2) {
        if let Some(height_m) = height_on_world_segment_key(point, segment[0], segment[1]) {
            return Some(height_m);
        }
    }
    if closed && points_world.len() > 2 {
        let last = points_world.last().copied()?;
        return height_on_world_segment_key(point, last, points_world[0]);
    }
    None
}

fn height_on_world_segment_key(
    point: NodeRailPointKey,
    start: RoadVec3,
    end: RoadVec3,
) -> Option<f64> {
    height_for_key_on_generated_edge(
        point,
        road_point_key(xz(start)),
        road_point_key(xz(end)),
        start.y,
        end.y,
    )
}

fn height_on_constraint_key(
    point: NodeRailPointKey,
    constraint: &NodeRailConstraint,
    heights_by_key: &BTreeMap<NodeRailPointKey, f64>,
) -> Option<f64> {
    let points = constraint
        .points_xz
        .iter()
        .copied()
        .map(road_point_key)
        .collect::<Vec<_>>();
    if points.contains(&point) {
        return known_height(heights_by_key, point);
    }
    for segment in points.windows(2) {
        let start = segment[0];
        let end = segment[1];
        let (Some(start_height_m), Some(end_height_m)) = (
            known_height(heights_by_key, start),
            known_height(heights_by_key, end),
        ) else {
            continue;
        };
        if let Some(height_m) =
            height_for_key_on_generated_edge(point, start, end, start_height_m, end_height_m)
        {
            return Some(height_m);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::super::super::super::RoadSurfaceBandKind;
    use super::super::super::super::arrangement::NodeBandOwner;
    use super::super::super::super::backend::RoadVec2;
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
