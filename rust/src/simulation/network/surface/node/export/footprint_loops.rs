//! Footprint boundary loop export from final owned top-surface footprints.

use super::super::{
    NodeOverlayShapes,
    arrangement::NodeArrangementKey,
    backend::RoadVec2,
    boundary::{
        ArrangementBoundaryPointKey, NodeBoundaryExportError, NodeFootprintBoundaryExportSources,
        NodeFootprintBoundaryPoint, boundary_points_numeric_area_budget_m2,
        remove_subbudget_unsupported_numeric_boundary_vertices,
        same_winding_boundary_point_loops_from_loop,
    },
};
use crate::simulation::network::surface::RoadSurfaceSystem;
use godot::prelude::Vector3;
use std::collections::BTreeSet;

impl RoadSurfaceSystem {
    pub(super) fn footprint_shapes_from_owned_regions(
        owned_regions: &[super::NodeOwnedRegion],
    ) -> Result<NodeOverlayShapes, NodeBoundaryExportError> {
        let contours = owned_regions
            .iter()
            .map(|region| {
                region
                    .polygon
                    .points_world
                    .iter()
                    .map(|point| [f64::from(point.x), f64::from(point.z)])
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let shapes = Self::overlay_union_contours(&contours)
            .ok_or(NodeBoundaryExportError::EmptyOuterBoundary)?;
        (!shapes.is_empty())
            .then_some(shapes)
            .ok_or(NodeBoundaryExportError::EmptyOuterBoundary)
    }

    pub(super) fn footprint_boundary_point_loops_from_footprint_shapes(
        footprint_shapes: &NodeOverlayShapes,
        boundary_export_sources: &mut NodeFootprintBoundaryExportSources,
    ) -> Result<Vec<Vec<NodeFootprintBoundaryPoint>>, NodeBoundaryExportError> {
        let mut loops = Vec::new();
        let mut emitted_loop_identities = BTreeSet::<Vec<ArrangementBoundaryPointKey>>::new();
        for shape in footprint_shapes {
            for contour in shape {
                let mut points = Vec::with_capacity(contour.len());
                for point in contour {
                    let key = NodeArrangementKey::from_point(RoadVec2::new(point[0], point[1]));
                    let Some(height_mm) = boundary_export_sources.boundary_height_mm_at_key(key)?
                    else {
                        return Err(NodeBoundaryExportError::MissingFootprintBoundaryHeight {
                            x_key: key.x_key(),
                            z_key: key.z_key(),
                        });
                    };
                    points.push(NodeFootprintBoundaryPoint::new(
                        ArrangementBoundaryPointKey {
                            x_key: key.x_key(),
                            z_key: key.z_key(),
                            y_mm: height_mm,
                        },
                    ));
                }
                push_valid_footprint_boundary_point_loops(
                    points,
                    boundary_export_sources,
                    &mut emitted_loop_identities,
                    &mut loops,
                )?;
            }
        }
        (!loops.is_empty())
            .then_some(loops)
            .ok_or(NodeBoundaryExportError::EmptyOuterBoundary)
    }
}

fn push_valid_footprint_boundary_point_loops(
    points: Vec<NodeFootprintBoundaryPoint>,
    boundary_export_sources: &mut NodeFootprintBoundaryExportSources,
    emitted_loop_identities: &mut BTreeSet<Vec<ArrangementBoundaryPointKey>>,
    loops: &mut Vec<Vec<NodeFootprintBoundaryPoint>>,
) -> Result<(), NodeBoundaryExportError> {
    let mut points = canonicalize_footprint_boundary_point_loop(points);
    remove_subbudget_unsupported_numeric_boundary_vertices(
        &mut points,
        |current_point_key, local_points| {
            boundary_export_sources
                .has_exact_final_owned_footprint_boundary_support_at_point(current_point_key)
                || RoadSurfaceSystem::signed_polygon_area_xz(&local_points).abs()
                    > boundary_points_numeric_area_budget_m2(&local_points)
        },
    );
    let points = canonicalize_footprint_boundary_point_loop(points);
    if points.len() < 3 {
        return Ok(());
    }
    if signed_footprint_boundary_point_loop_area_xz(&points).abs()
        <= footprint_boundary_point_loop_numeric_area_budget_m2(&points)
    {
        return Ok(());
    }
    for split_points in same_winding_boundary_point_loops_from_loop(&points) {
        if signed_footprint_boundary_point_loop_area_xz(&split_points).abs()
            <= footprint_boundary_point_loop_numeric_area_budget_m2(&split_points)
        {
            continue;
        }
        if !emitted_loop_identities.insert(footprint_boundary_point_loop_identity(&split_points)) {
            continue;
        }
        for point in &split_points {
            boundary_export_sources.reject_boundary_vertex_height_conflict(point.xz_key())?;
            if !boundary_export_sources
                .has_exact_final_owned_footprint_boundary_support_at_point(point.point_key)
            {
                return Err(NodeBoundaryExportError::MissingFootprintBoundaryHeight {
                    x_key: point.point_key.x_key,
                    z_key: point.point_key.z_key,
                });
            }
        }
        loops.push(split_points);
    }
    Ok(())
}

fn canonicalize_footprint_boundary_point_loop(
    mut points: Vec<NodeFootprintBoundaryPoint>,
) -> Vec<NodeFootprintBoundaryPoint> {
    points.dedup_by(|a, b| a.point_key == b.point_key);
    if points.len() >= 2
        && points.first().map(|point| point.point_key) == points.last().map(|point| point.point_key)
    {
        points.pop();
    }
    points
}

fn footprint_boundary_point_loop_identity(
    points: &[NodeFootprintBoundaryPoint],
) -> Vec<ArrangementBoundaryPointKey> {
    let keys = points
        .iter()
        .map(|point| point.point_key)
        .collect::<Vec<_>>();
    let forward = canonical_footprint_boundary_loop_rotation(&keys);
    let mut reversed = keys;
    reversed.reverse();
    let reversed = canonical_footprint_boundary_loop_rotation(&reversed);
    forward.min(reversed)
}

fn canonical_footprint_boundary_loop_rotation(
    keys: &[ArrangementBoundaryPointKey],
) -> Vec<ArrangementBoundaryPointKey> {
    if keys.is_empty() {
        return Vec::new();
    }
    let start_index = keys
        .iter()
        .enumerate()
        .min_by_key(|(_, key)| **key)
        .map(|(index, _)| index)
        .unwrap_or(0);
    keys[start_index..]
        .iter()
        .chain(&keys[..start_index])
        .copied()
        .collect()
}

fn footprint_boundary_point_loop_world_points(
    points: &[NodeFootprintBoundaryPoint],
) -> Vec<Vector3> {
    points.iter().map(|point| point.point_world()).collect()
}

fn signed_footprint_boundary_point_loop_area_xz(points: &[NodeFootprintBoundaryPoint]) -> f32 {
    RoadSurfaceSystem::signed_polygon_area_xz(&footprint_boundary_point_loop_world_points(points))
}

fn footprint_boundary_point_loop_numeric_area_budget_m2(
    points: &[NodeFootprintBoundaryPoint],
) -> f32 {
    boundary_points_numeric_area_budget_m2(&footprint_boundary_point_loop_world_points(points))
}
