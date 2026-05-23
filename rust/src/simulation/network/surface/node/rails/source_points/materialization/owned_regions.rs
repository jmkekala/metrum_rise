//! Height-carrier point materialization from boolean-owned rail regions.

use super::super::super::super::backend::{RoadVec3, overlay_point_to_road, road_vec3_xz as xz};
use super::super::super::super::keys::{SurfaceHeightMmKey, SurfaceXzKey};
use super::super::super::super::ownership::NodeBooleanOwnership;
use super::super::super::contours::height_for_key_on_generated_edge;
use super::super::super::geometry::road_point_key;
use super::super::super::topology::NodeRailPointKey;
use super::super::super::{
    NodeRailConstraint, NodeRailConstraintKind, NodeRailGenerationError, NodeRailHeightCarrierPaths,
};
use super::super::NodeRailHeightSourceKey;
use super::super::collection::{
    conflicting_height_carrier_point_error, known_height, push_materialized_height_carrier_points,
    source_has_height_point, source_height_points_by_key,
};
use std::collections::BTreeMap;

// Mirrors the ownership-stage duplicate-source cluster budget: sub-quarter-millimeter
// same-owner/source points are one canonical source cluster; broader same-mm collisions are not
// height authority.
const SOURCE_DUPLICATE_CLUSTER_MAX_SPAN_UNITS: i64 = 256;

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
    collect_candidate_height(
        source,
        point,
        &mut selected_height_m,
        same_mm_source_cluster_height(source, point, points_by_source)?,
    )?;
    for constraint in constraints {
        if !constraint_authorizes_height_source(constraint, source) {
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

fn same_mm_source_cluster_height(
    source: NodeRailHeightSourceKey,
    point: NodeRailPointKey,
    points_by_source: &BTreeMap<NodeRailHeightSourceKey, Vec<RoadVec3>>,
) -> Result<Option<f64>, NodeRailGenerationError> {
    let Some(source_points) = points_by_source.get(&source) else {
        return Ok(None);
    };
    let point_mm = point_mm_key(point);
    let candidates = source_points
        .iter()
        .filter_map(|source_point| {
            let source_point_key = road_point_key(xz(*source_point));
            (source_point_key != point && point_mm_key(source_point_key) == point_mm)
                .then_some((source_point_key, source_point.y))
        })
        .collect::<Vec<_>>();
    if candidates.len() < 2 || !same_mm_candidates_form_source_duplicate_cluster(&candidates) {
        return Ok(None);
    }
    let mut selected_height_m = None;
    for (_, height_m) in candidates {
        collect_candidate_height(source, point, &mut selected_height_m, Some(height_m))?;
    }
    Ok(selected_height_m)
}

fn same_mm_candidates_form_source_duplicate_cluster(
    candidates: &[(NodeRailPointKey, f64)],
) -> bool {
    let min_x = candidates
        .iter()
        .map(|(candidate, _)| candidate.0)
        .min()
        .unwrap_or_default();
    let max_x = candidates
        .iter()
        .map(|(candidate, _)| candidate.0)
        .max()
        .unwrap_or_default();
    let min_z = candidates
        .iter()
        .map(|(candidate, _)| candidate.1)
        .min()
        .unwrap_or_default();
    let max_z = candidates
        .iter()
        .map(|(candidate, _)| candidate.1)
        .max()
        .unwrap_or_default();
    max_x - min_x <= SOURCE_DUPLICATE_CLUSTER_MAX_SPAN_UNITS
        && max_z - min_z <= SOURCE_DUPLICATE_CLUSTER_MAX_SPAN_UNITS
}

fn point_mm_key(point: NodeRailPointKey) -> NodeRailPointKey {
    (
        SurfaceXzKey::coordinate_key_to_mm(point.0),
        SurfaceXzKey::coordinate_key_to_mm(point.1),
    )
}

fn constraint_authorizes_height_source(
    constraint: &NodeRailConstraint,
    source: NodeRailHeightSourceKey,
) -> bool {
    if constraint.kind == NodeRailConstraintKind::RaisedStepContact {
        return false;
    }
    if (
        constraint.source_mouth_order_index,
        constraint.source_band_index,
    ) != (source.1, Some(source.2))
    {
        return false;
    }
    [constraint.owner, constraint.opposite_owner]
        .into_iter()
        .flatten()
        .any(|owner| owner.kind() == source.0)
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

#[cfg(test)]
mod tests {
    use super::super::super::super::super::RoadSurfaceBandKind;
    use super::super::super::super::super::RoadSurfaceVisualNodePieceKind;
    use super::super::super::super::super::arrangement::NodeBandOwner;
    use super::super::super::super::super::backend::RoadVec2;
    use super::super::super::super::super::ownership::{
        NodeBooleanOwnedRegion, NodeOwnedRegionArrangement,
    };
    use super::super::super::super::super::rails::NodeGeneratedContourClaimPriority;
    use super::*;

    #[test]
    fn owned_region_materializes_height_from_opposite_owner_pair_constraint() {
        let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 2);
        let source = (RoadSurfaceBandKind::Sidewalk, 0, 5);
        let constraints = [NodeRailConstraint {
            constraint_index: 0,
            kind: NodeRailConstraintKind::BandBoundary {
                left_kind: RoadSurfaceBandKind::CurbOrShoulder,
                right_kind: RoadSurfaceBandKind::Sidewalk,
            },
            source_mouth_order_index: source.1,
            source_band_index: Some(source.2),
            source_boundary_index: None,
            owner: Some(curb),
            opposite_owner: Some(sidewalk),
            points_xz: vec![
                RoadVec2::new(0.0, 0.0),
                RoadVec2::new(5.0, 0.0),
                RoadVec2::new(10.0, 0.0),
            ],
        }];
        let mut points_by_source = BTreeMap::from([(
            source,
            vec![RoadVec3::new(0.0, 1.0, 0.0), RoadVec3::new(10.0, 3.0, 0.0)],
        )]);
        let ownership = test_ownership(NodeBooleanOwnedRegion {
            kind: RoadSurfaceBandKind::Sidewalk,
            owner: sidewalk,
            claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
            source_mouth_order_index: source.1,
            source_band_index: Some(source.2),
            shape: vec![vec![[5.0, 0.0], [5.0, 1.0], [6.0, 0.0]]],
            area_m2: 0.5,
            seam_constraints: Vec::new(),
        });

        push_owned_region_height_carrier_points(
            &mut points_by_source,
            &constraints,
            &BTreeMap::new(),
            &ownership,
        )
        .expect("opposite-owner source constraint must materialize final owned vertex height");

        let points = points_by_source
            .get(&source)
            .expect("source should remain present");
        assert!(points.iter().any(|point| {
            road_point_key(xz(*point)) == road_point_key(RoadVec2::new(5.0, 0.0))
                && SurfaceHeightMmKey::from_m_f64(point.y) == SurfaceHeightMmKey::from_m_f64(2.0)
        }));
    }

    #[test]
    fn owned_region_materializes_height_from_same_mm_source_cluster() {
        let sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 2);
        let source = (RoadSurfaceBandKind::Sidewalk, 1, 5);
        let mut points_by_source = BTreeMap::from([(
            source,
            vec![
                RoadVec3::new(1.0, 4.0, 0.0),
                RoadVec3::new(1.0001, 4.0, 0.0),
            ],
        )]);
        let ownership = test_ownership(NodeBooleanOwnedRegion {
            kind: RoadSurfaceBandKind::Sidewalk,
            owner: sidewalk,
            claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
            source_mouth_order_index: source.1,
            source_band_index: Some(source.2),
            shape: vec![vec![[1.00005, 0.0], [1.00005, 1.0], [2.0, 0.0]]],
            area_m2: 0.5,
            seam_constraints: Vec::new(),
        });

        push_owned_region_height_carrier_points(
            &mut points_by_source,
            &[],
            &BTreeMap::new(),
            &ownership,
        )
        .expect("same-mm source cluster must materialize the final owned vertex height");

        let points = points_by_source
            .get(&source)
            .expect("source should remain present");
        assert!(points.iter().any(|point| {
            road_point_key(xz(*point)) == road_point_key(RoadVec2::new(1.00005, 0.0))
                && SurfaceHeightMmKey::from_m_f64(point.y) == SurfaceHeightMmKey::from_m_f64(4.0)
        }));
    }

    fn test_ownership(region: NodeBooleanOwnedRegion) -> NodeBooleanOwnership {
        let owned_regions = vec![region];
        NodeBooleanOwnership {
            node_id: 1,
            piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
            footprint_shapes: Vec::new(),
            asphalt_shapes: Vec::new(),
            non_road_shapes: Vec::new(),
            owned_region_arrangement: NodeOwnedRegionArrangement::from_owned_regions(
                1,
                RoadSurfaceVisualNodePieceKind::JunctionN,
                &owned_regions,
                &Vec::new(),
                &[],
            ),
            owned_regions,
        }
    }
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
    if let Some(index) = points.iter().position(|candidate| *candidate == point) {
        if let Some(height_m) = known_height(heights_by_key, point) {
            return Some(height_m);
        }
        if let Some((start, start_height_m, end, end_height_m)) =
            surrounding_known_constraint_height_segment(&points, heights_by_key, index)
        {
            return height_for_key_on_generated_edge(
                point,
                start,
                end,
                start_height_m,
                end_height_m,
            );
        }
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

fn surrounding_known_constraint_height_segment(
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
