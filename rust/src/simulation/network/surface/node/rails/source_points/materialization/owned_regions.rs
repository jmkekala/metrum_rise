//! Height-carrier materialization from explicit node carrier provenance.

use super::super::super::contours::height_for_key_on_generated_edge;
use super::super::super::geometry::road_point_key;
use super::super::super::topology::NodeRailPointKey;
use super::super::super::{NodeRailGenerationError, NodeRailHeightCarrierPaths};
use super::super::NodeRailHeightSourceKey;
use super::super::collection::{
    push_materialized_height_carrier_points, source_has_height_point, source_height_points_by_key,
};
use crate::simulation::network::surface::keys::SurfaceXzKey;
use crate::simulation::network::surface::node::backend::{RoadVec3, road_vec3_xz as xz};
use crate::simulation::network::surface::node::ownership::{
    NodeBooleanOwnership, NodeCarrierProvenanceOrigin,
};
use std::collections::BTreeMap;

pub(in crate::simulation::network::surface::node::rails) fn push_owned_region_height_carrier_points(
    points_by_source: &mut BTreeMap<NodeRailHeightSourceKey, Vec<RoadVec3>>,
    _constraints: &[super::super::super::NodeRailConstraint],
    paths_by_source: &BTreeMap<NodeRailHeightSourceKey, NodeRailHeightCarrierPaths>,
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
        let Some(height_m) = height_for_carrier_provenance(
            source,
            point,
            record.origin,
            points_by_source,
            paths_by_source,
        )?
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
    paths_by_source: &BTreeMap<NodeRailHeightSourceKey, NodeRailHeightCarrierPaths>,
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
            canonical_point,
            segment_start,
            segment_end,
            ..
        } => height_for_explicit_source_segment(
            source,
            canonical_point.raw_tuple(),
            segment_start.raw_tuple(),
            segment_end.raw_tuple(),
            points_by_source,
            paths_by_source,
        ),
        NodeCarrierProvenanceOrigin::SourceSurface => Ok(None),
    }
}

fn height_for_explicit_source_segment(
    source: NodeRailHeightSourceKey,
    point: NodeRailPointKey,
    segment_start: NodeRailPointKey,
    segment_end: NodeRailPointKey,
    points_by_source: &BTreeMap<NodeRailHeightSourceKey, Vec<RoadVec3>>,
    paths_by_source: &BTreeMap<NodeRailHeightSourceKey, NodeRailHeightCarrierPaths>,
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

    let Some(paths) = paths_by_source.get(&source) else {
        return Ok(None);
    };
    Ok(height_for_segment_from_source_paths(
        point,
        segment_start,
        segment_end,
        paths,
    ))
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
