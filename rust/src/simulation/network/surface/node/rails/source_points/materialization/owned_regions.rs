// SPDX-License-Identifier: GPL-2.0-only

//! Height-carrier materialization from explicit node carrier provenance.

use super::super::super::contours::height_for_key_on_generated_edge;
use super::super::super::geometry::road_point_key;
use super::super::super::topology::NodeRailPointKey;
use super::super::super::{
    NodeGeneratedContourKind, NodeRailConstraint, NodeRailConstraintKind, NodeRailContourSet,
    NodeRailGenerationError, NodeRailHeightCarrierPaths,
};
use super::super::NodeRailHeightSourceKey;
use super::super::collection::{
    push_materialized_height_carrier_points, source_has_height_point, source_height_points_by_key,
};
use crate::simulation::network::surface::keys::SurfaceXzKey;
use crate::simulation::network::surface::node::backend::{RoadVec3, road_vec3_xz as xz};
use crate::simulation::network::surface::node::height::constrained_height_triangles_from_vertices;
use crate::simulation::network::surface::node::ownership::{
    NodeBooleanOwnership, NodeCarrierProvenanceOrigin, NodeSourceCarrierSegmentId,
};
use std::collections::BTreeMap;

pub(in crate::simulation::network::surface::node::rails) fn push_owned_region_height_carrier_points(
    points_by_source: &mut BTreeMap<NodeRailHeightSourceKey, Vec<RoadVec3>>,
    rails: &NodeRailContourSet,
    ownership: &NodeBooleanOwnership,
) -> Result<(), NodeRailGenerationError> {
    let mut materialized = Vec::new();
    for record in &ownership.carrier_provenance.records {
        let source = (
            record.source_kind,
            record.source_mouth_order_index,
            record.source_band_index,
        );
        let point = record.point.raw_tuple();
        if source_has_height_point(points_by_source, source, point) {
            continue;
        }
        let Some(height_m) =
            height_for_carrier_provenance(source, point, record.origin, points_by_source, rails)?
        else {
            continue;
        };
        let point_xz = SurfaceXzKey::from_raw_tuple(point).to_road_xz();
        materialized.push((source, RoadVec3::new(point_xz.x, height_m, point_xz.y)));
    }
    push_materialized_height_carrier_points(points_by_source, materialized)
}

fn height_for_carrier_provenance(
    source: NodeRailHeightSourceKey,
    point: NodeRailPointKey,
    origin: NodeCarrierProvenanceOrigin,
    points_by_source: &BTreeMap<NodeRailHeightSourceKey, Vec<RoadVec3>>,
    rails: &NodeRailContourSet,
) -> Result<Option<f64>, NodeRailGenerationError> {
    match origin {
        NodeCarrierProvenanceOrigin::SourceVertex => {
            let source_points = points_by_source
                .get(&source)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            Ok(source_height_points_by_key(source, source_points)?
                .get(&point)
                .copied())
        }
        NodeCarrierProvenanceOrigin::SourceSegment {
            source_segment_id,
            canonical_point,
            segment_start,
            segment_end,
            ..
        } => {
            let canonical_point = canonical_point.raw_tuple();
            let Some(height_m) = height_for_explicit_source_segment(
                source,
                canonical_point,
                segment_start.raw_tuple(),
                segment_end.raw_tuple(),
                points_by_source,
                rails,
                source_segment_id,
            )?
            else {
                return Err(NodeRailGenerationError::MissingCarrierProvenanceHeight {
                    kind: source.0,
                    mouth_order_index: source.1,
                    band_index: source.2,
                    point_x_key: point.0,
                    point_z_key: point.1,
                    source_segment_id,
                });
            };
            Ok(Some(height_m))
        }
        NodeCarrierProvenanceOrigin::SourceIntersection { .. }
        | NodeCarrierProvenanceOrigin::GeneratedCarrierVertex { .. } => Ok(None),
        NodeCarrierProvenanceOrigin::GeneratedCarrierSurface { contour_index, .. } => Ok(
            height_for_generated_carrier_surface(point, contour_index, rails),
        ),
    }
}

fn height_for_generated_carrier_surface(
    point: NodeRailPointKey,
    contour_index: usize,
    rails: &NodeRailContourSet,
) -> Option<f64> {
    let contour = rails.contours.get(contour_index)?;
    let triangles =
        constrained_height_triangles_from_vertices(contour.height_points_world.as_ref()?).ok()?;
    let point_xz = SurfaceXzKey::from_raw_tuple(point).to_road_xz();
    let mut heights = triangles
        .iter()
        .filter_map(|triangle| triangle.height_at(point_xz))
        .collect::<Vec<_>>();
    heights.sort_by_key(|height| {
        crate::simulation::network::surface::keys::SurfaceHeightMmKey::from_m_f64(*height)
    });
    heights.dedup_by_key(|height| {
        crate::simulation::network::surface::keys::SurfaceHeightMmKey::from_m_f64(*height)
    });
    (heights.len() == 1).then_some(heights[0])
}

fn height_for_explicit_source_segment(
    source: NodeRailHeightSourceKey,
    point: NodeRailPointKey,
    segment_start: NodeRailPointKey,
    segment_end: NodeRailPointKey,
    points_by_source: &BTreeMap<NodeRailHeightSourceKey, Vec<RoadVec3>>,
    rails: &NodeRailContourSet,
    source_segment_id: NodeSourceCarrierSegmentId,
) -> Result<Option<f64>, NodeRailGenerationError> {
    let source_points = points_by_source
        .get(&source)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let heights_by_key = source_height_points_by_key(source, source_points)?;
    if let (Some(start_height_m), Some(end_height_m)) = (
        heights_by_key.get(&segment_start).copied(),
        heights_by_key.get(&segment_end).copied(),
    ) {
        return Ok(height_for_key_on_generated_edge(
            point,
            segment_start,
            segment_end,
            start_height_m,
            end_height_m,
        ));
    }

    if let Some(paths_for_source) = rails.height_carrier_paths_by_source.get(&source)
        && let Some(height_m) = paths_for_source.iter().find_map(|paths| {
            height_for_segment_from_source_paths(point, segment_start, segment_end, paths)
        })
    {
        return Ok(Some(height_m));
    }
    if let Some(height_m) = height_for_segment_from_source_constraints(
        point,
        segment_start,
        segment_end,
        rails,
        source_segment_id,
        &heights_by_key,
    ) {
        return Ok(Some(height_m));
    }
    if let Some(height_m) = height_for_segment_from_generated_contours(
        point,
        segment_start,
        segment_end,
        rails,
        source_segment_id,
    ) {
        return Ok(Some(height_m));
    }
    Ok(None)
}

fn height_for_segment_from_source_constraints(
    point: NodeRailPointKey,
    segment_start: NodeRailPointKey,
    segment_end: NodeRailPointKey,
    rails: &NodeRailContourSet,
    source_segment_id: NodeSourceCarrierSegmentId,
    heights_by_key: &BTreeMap<NodeRailPointKey, f64>,
) -> Option<f64> {
    rails
        .constraints
        .iter()
        .filter(|constraint| constraint_matches_source_segment(source_segment_id, constraint))
        .filter_map(|constraint| {
            let keys = constraint_point_keys(constraint);
            let segment_index = keys.windows(2).position(|segment| {
                (segment[0] == segment_start && segment[1] == segment_end)
                    || (segment[0] == segment_end && segment[1] == segment_start)
            })?;
            height_for_path_segment_point(point, &keys, heights_by_key, segment_index)
        })
        .next()
}

fn constraint_matches_source_segment(
    source_segment_id: NodeSourceCarrierSegmentId,
    constraint: &NodeRailConstraint,
) -> bool {
    constraint.kind != NodeRailConstraintKind::RaisedStepContact
        && constraint.source_mouth_order_index == source_segment_id.source_mouth_order_index
        && constraint.source_band_index == Some(source_segment_id.source_band_index)
        && [constraint.owner, constraint.opposite_owner].contains(&Some(source_segment_id.owner))
        && source_segment_id.owner.kind() == source_segment_id.source_kind
}

fn constraint_point_keys(constraint: &NodeRailConstraint) -> Vec<NodeRailPointKey> {
    constraint
        .points_xz
        .iter()
        .copied()
        .map(road_point_key)
        .collect()
}

fn height_for_path_segment_point(
    point: NodeRailPointKey,
    keys: &[NodeRailPointKey],
    heights_by_key: &BTreeMap<NodeRailPointKey, f64>,
    segment_index: usize,
) -> Option<f64> {
    let (start, start_height_m) = (0..=segment_index).rev().find_map(|candidate_index| {
        let point = keys[candidate_index];
        heights_by_key
            .get(&point)
            .copied()
            .map(|height| (point, height))
    })?;
    let (end, end_height_m) = (segment_index + 1..keys.len()).find_map(|candidate_index| {
        let point = keys[candidate_index];
        heights_by_key
            .get(&point)
            .copied()
            .map(|height| (point, height))
    })?;
    height_for_key_on_generated_edge(point, start, end, start_height_m, end_height_m)
}

fn height_for_segment_from_generated_contours(
    point: NodeRailPointKey,
    segment_start: NodeRailPointKey,
    segment_end: NodeRailPointKey,
    rails: &NodeRailContourSet,
    source_segment_id: NodeSourceCarrierSegmentId,
) -> Option<f64> {
    rails
        .contours
        .iter()
        .filter(|contour| {
            contour.owner == Some(source_segment_id.owner)
                && contour.source_mouth_order_index == source_segment_id.source_mouth_order_index
                && contour.source_band_index == Some(source_segment_id.source_band_index)
                && matches!(
                    contour.kind,
                    NodeGeneratedContourKind::Band { kind }
                        if kind == source_segment_id.source_kind
                )
        })
        .filter_map(|contour| {
            height_for_segment_from_world_path(
                point,
                segment_start,
                segment_end,
                contour.height_points_world.as_ref()?,
                true,
            )
        })
        .next()
}

fn height_for_segment_from_source_paths(
    point: NodeRailPointKey,
    segment_start: NodeRailPointKey,
    segment_end: NodeRailPointKey,
    paths: &NodeRailHeightCarrierPaths,
) -> Option<f64> {
    let mut contour_world =
        Vec::with_capacity(paths.start_path_world.len() + paths.end_path_world.len());
    contour_world.extend(paths.start_path_world.iter().copied());
    contour_world.extend(paths.end_path_world.iter().rev().copied());
    height_for_segment_from_world_path(point, segment_start, segment_end, &contour_world, true)
}

fn height_for_segment_from_world_path(
    point: NodeRailPointKey,
    segment_start: NodeRailPointKey,
    segment_end: NodeRailPointKey,
    path_world: &[RoadVec3],
    closed: bool,
) -> Option<f64> {
    for segment in path_world.windows(2) {
        if segment_matches(segment_start, segment_end, segment[0], segment[1]) {
            return height_on_world_segment_key(point, segment[0], segment[1]);
        }
    }
    if closed
        && path_world.len() > 2
        && let (Some(start), Some(end)) = (path_world.last().copied(), path_world.first().copied())
        && segment_matches(segment_start, segment_end, start, end)
    {
        return height_on_world_segment_key(point, start, end);
    }
    None
}

fn segment_matches(
    segment_start: NodeRailPointKey,
    segment_end: NodeRailPointKey,
    start: RoadVec3,
    end: RoadVec3,
) -> bool {
    let start_key = road_point_key(xz(start));
    let end_key = road_point_key(xz(end));
    (start_key == segment_start && end_key == segment_end)
        || (start_key == segment_end && end_key == segment_start)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::surface::node::arrangement::NodeBandOwner;
    use crate::simulation::network::surface::node::backend::RoadVec2;
    use crate::simulation::network::surface::node::ownership::{
        NodeOwnedRegionArrangementKey, NodeSourceCarrierRegistry,
    };
    use crate::simulation::network::surface::{
        RoadSurfaceBandKind, RoadSurfaceVisualNodePieceKind,
    };

    #[test]
    fn source_constraint_height_materializes_only_from_recorded_source_segment() {
        let owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 7);
        let source_segment_id = NodeSourceCarrierSegmentId {
            owner,
            source_kind: RoadSurfaceBandKind::Carriageway,
            source_mouth_order_index: 3,
            source_band_index: 1,
            segment_start: NodeOwnedRegionArrangementKey::from_point(RoadVec2::new(2.0, 0.0)),
            segment_end: NodeOwnedRegionArrangementKey::from_point(RoadVec2::new(3.0, 0.0)),
        };
        let rails = NodeRailContourSet {
            node_id: 1,
            piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
            contours: Vec::new(),
            corner_trims: Vec::new(),
            side_join_gaps: Vec::new(),
            constraints: vec![NodeRailConstraint {
                constraint_index: 0,
                kind: NodeRailConstraintKind::BandContour {
                    kind: RoadSurfaceBandKind::Carriageway,
                },
                source_mouth_order_index: 3,
                source_band_index: Some(1),
                source_boundary_index: None,
                owner: Some(owner),
                opposite_owner: None,
                points_xz: vec![
                    RoadVec2::new(0.0, 0.0),
                    RoadVec2::new(2.0, 0.0),
                    RoadVec2::new(3.0, 0.0),
                    RoadVec2::new(5.0, 0.0),
                ],
            }],
            height_carrier_paths_by_source: BTreeMap::new(),
            height_carrier_points_by_source: BTreeMap::new(),
            source_carriers: NodeSourceCarrierRegistry::default(),
        };
        let mut heights_by_key = BTreeMap::new();
        heights_by_key.insert(road_point_key(RoadVec2::new(0.0, 0.0)), 10.0);
        heights_by_key.insert(road_point_key(RoadVec2::new(5.0, 0.0)), 15.0);

        let height_m = height_for_segment_from_source_constraints(
            road_point_key(RoadVec2::new(2.0, 0.0)),
            road_point_key(RoadVec2::new(2.0, 0.0)),
            road_point_key(RoadVec2::new(3.0, 0.0)),
            &rails,
            source_segment_id,
            &heights_by_key,
        )
        .expect("recorded source segment should materialize from its explicit source constraint");

        assert_eq!(height_m, 12.0);

        let wrong_source_segment_id = NodeSourceCarrierSegmentId {
            source_band_index: 2,
            ..source_segment_id
        };
        assert_eq!(
            height_for_segment_from_source_constraints(
                road_point_key(RoadVec2::new(2.0, 0.0)),
                road_point_key(RoadVec2::new(2.0, 0.0)),
                road_point_key(RoadVec2::new(3.0, 0.0)),
                &rails,
                wrong_source_segment_id,
                &heights_by_key,
            ),
            None
        );
    }
}
