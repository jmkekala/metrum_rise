//! Source height-carrier point collection for generated node rails.

use super::super::RoadSurfaceBandKind;
use super::super::backend::{RoadVec3, overlay_point_to_road, road_vec3_xz as xz};
use super::super::input::NodeInputBandInterval;
use super::super::keys::SurfaceHeightMmKey;
use super::super::ownership::NodeBooleanOwnership;
use super::contours::{height_for_key_on_generated_edge, subdivided_world_chord};
use super::geometry::{road_point_from_key, road_point_key};
use super::topology::NodeRailPointKey;
use super::{
    NodeGeneratedContour, NodeGeneratedContourKind, NodeRailConstraint, NodeRailGenerationError,
    NodeRailHeightCarrierPaths,
};
use std::collections::BTreeMap;

pub(super) fn interval_height_carrier_paths(
    interval: &NodeInputBandInterval,
) -> NodeRailHeightCarrierPaths {
    if interval.start_path_world.len() > 2
        && source_height_path_is_endpoint_chord(
            &interval.end_path_world,
            interval.mouth_end_world,
            interval.endpoint_end_world,
        )
    {
        return NodeRailHeightCarrierPaths {
            start_path_world: interval.start_path_world.clone(),
            end_path_world: subdivided_world_chord(
                interval.mouth_end_world,
                interval.endpoint_end_world,
                interval.start_path_world.len(),
            ),
        };
    }
    if interval.end_path_world.len() > 2
        && source_height_path_is_endpoint_chord(
            &interval.start_path_world,
            interval.mouth_start_world,
            interval.endpoint_start_world,
        )
    {
        return NodeRailHeightCarrierPaths {
            start_path_world: subdivided_world_chord(
                interval.mouth_start_world,
                interval.endpoint_start_world,
                interval.end_path_world.len(),
            ),
            end_path_world: interval.end_path_world.clone(),
        };
    }
    NodeRailHeightCarrierPaths {
        start_path_world: interval.start_path_world.clone(),
        end_path_world: interval.end_path_world.clone(),
    }
}

pub(super) fn interval_height_carrier_points(
    interval: &NodeInputBandInterval,
    paths: &NodeRailHeightCarrierPaths,
) -> Vec<RoadVec3> {
    [
        interval.endpoint_start_world,
        interval.endpoint_end_world,
        interval.mouth_end_world,
        interval.mouth_start_world,
    ]
    .into_iter()
    .chain(paths.start_path_world.iter().copied())
    .chain(paths.end_path_world.iter().copied())
    .collect()
}

pub(super) fn push_band_height_carrier_points(
    points_by_source: &mut BTreeMap<(RoadSurfaceBandKind, usize, usize), Vec<RoadVec3>>,
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

pub(super) fn push_generated_contour_height_carrier_points(
    points_by_source: &mut BTreeMap<(RoadSurfaceBandKind, usize, usize), Vec<RoadVec3>>,
    contours: &[NodeGeneratedContour],
) -> Result<(), NodeRailGenerationError> {
    for contour in contours {
        let (NodeGeneratedContourKind::Band { kind }, Some(source_band_index), Some(points_world)) = (
            contour.kind,
            contour.source_band_index,
            contour.height_points_world.as_deref(),
        ) else {
            continue;
        };
        push_band_height_carrier_points(
            points_by_source,
            contour.source_mouth_order_index,
            source_band_index,
            kind,
            points_world.iter().copied(),
        )?;
    }
    Ok(())
}

pub(super) fn push_source_constraint_height_carrier_points(
    points_by_source: &mut BTreeMap<(RoadSurfaceBandKind, usize, usize), Vec<RoadVec3>>,
    constraints: &[NodeRailConstraint],
) -> Result<(), NodeRailGenerationError> {
    let mut materialized = Vec::new();
    for constraint in constraints {
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
        let Ok(source_heights_by_key) = source_height_points_by_key(source, source_points) else {
            continue;
        };
        materialized.extend(
            materialized_constraint_height_points(constraint, &source_heights_by_key)
                .into_iter()
                .map(|point| (source, point)),
        );
    }
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

pub(super) fn push_owned_region_height_carrier_points(
    points_by_source: &mut BTreeMap<(RoadSurfaceBandKind, usize, usize), Vec<RoadVec3>>,
    contours: &[NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
    paths_by_source: &BTreeMap<(RoadSurfaceBandKind, usize, usize), NodeRailHeightCarrierPaths>,
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
                contours,
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

fn source_height_path_is_endpoint_chord(
    path_world: &[RoadVec3],
    mouth_world: RoadVec3,
    endpoint_world: RoadVec3,
) -> bool {
    path_world.len() == 2
        && source_height_points_match(path_world[0], mouth_world)
        && source_height_points_match(path_world[1], endpoint_world)
}

fn source_height_points_match(a: RoadVec3, b: RoadVec3) -> bool {
    road_point_key(xz(a)) == road_point_key(xz(b))
        && SurfaceHeightMmKey::from_m_f64(a.y) == SurfaceHeightMmKey::from_m_f64(b.y)
}

fn source_height_points_by_key(
    source: (RoadSurfaceBandKind, usize, usize),
    points: &[RoadVec3],
) -> Result<BTreeMap<NodeRailPointKey, f64>, NodeRailGenerationError> {
    let mut heights_by_key = BTreeMap::<NodeRailPointKey, f64>::new();
    for point in points {
        let key = road_point_key(xz(*point));
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

fn materialized_constraint_height_points(
    constraint: &NodeRailConstraint,
    heights_by_key: &BTreeMap<NodeRailPointKey, f64>,
) -> Vec<RoadVec3> {
    let keys = constraint
        .points_xz
        .iter()
        .copied()
        .map(road_point_key)
        .collect::<Vec<_>>();
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

fn known_height(
    heights_by_key: &BTreeMap<NodeRailPointKey, f64>,
    point: NodeRailPointKey,
) -> Option<f64> {
    heights_by_key.get(&point).copied()
}

fn source_has_height_point(
    points_by_source: &BTreeMap<(RoadSurfaceBandKind, usize, usize), Vec<RoadVec3>>,
    source: (RoadSurfaceBandKind, usize, usize),
    point: NodeRailPointKey,
) -> bool {
    points_by_source.get(&source).is_some_and(|points| {
        points
            .iter()
            .any(|source_point| road_point_key(xz(*source_point)) == point)
    })
}

fn height_for_source_key(
    source: (RoadSurfaceBandKind, usize, usize),
    point: NodeRailPointKey,
    contours: &[NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
    paths_by_source: &BTreeMap<(RoadSurfaceBandKind, usize, usize), NodeRailHeightCarrierPaths>,
    points_by_source: &BTreeMap<(RoadSurfaceBandKind, usize, usize), Vec<RoadVec3>>,
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
    for contour in contours {
        let (NodeGeneratedContourKind::Band { kind }, Some(source_band_index), Some(points_world)) = (
            contour.kind,
            contour.source_band_index,
            contour.height_points_world.as_deref(),
        ) else {
            continue;
        };
        if (kind, contour.source_mouth_order_index, source_band_index) != source {
            continue;
        }
        collect_candidate_height(
            source,
            point,
            &mut selected_height_m,
            height_on_world_points_key(point, points_world, true),
        )?;
    }
    let source_heights_by_key = points_by_source
        .get(&source)
        .map(|points| source_height_points_by_key(source, points))
        .transpose()?
        .unwrap_or_default();
    for constraint in constraints {
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
    source: (RoadSurfaceBandKind, usize, usize),
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
        return height_on_world_segment_key(
            point,
            *points_world.last().expect("checked non-empty"),
            points_world[0],
        );
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

fn conflicting_height_carrier_point_error(
    source: (RoadSurfaceBandKind, usize, usize),
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
