//! Footprint boundary loop export from the final boolean road-owned footprint.

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
    pub(super) fn footprint_boundary_point_loops_from_footprint_shapes(
        footprint_shapes: &NodeOverlayShapes,
        boundary_export_sources: &mut NodeFootprintBoundaryExportSources,
    ) -> Result<Vec<Vec<NodeFootprintBoundaryPoint>>, NodeBoundaryExportError> {
        let mut loops = Vec::new();
        let mut emitted_loop_identities = BTreeSet::<Vec<ArrangementBoundaryPointKey>>::new();
        for shape in footprint_shapes {
            for contour in shape {
                let points = footprint_boundary_xz_point_loop_from_contour(contour);
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
            boundary_export_sources.has_exact_final_owned_footprint_boundary_support_at_xz_key(
                current_point_key.xz_key(),
            ) || RoadSurfaceSystem::signed_polygon_area_xz(&local_points).abs()
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
        let heighted_points =
            resolve_footprint_boundary_point_heights(split_points, boundary_export_sources)?;
        for split_points in post_height_footprint_boundary_point_loops(heighted_points) {
            if !emitted_loop_identities
                .insert(footprint_boundary_point_loop_identity(&split_points))
            {
                continue;
            }
            for index in 0..split_points.len() {
                let point = split_points[index];
                let previous_key = split_points[if index == 0 {
                    split_points.len() - 1
                } else {
                    index - 1
                }]
                .xz_key();
                let next_key = split_points[(index + 1) % split_points.len()].xz_key();
                boundary_export_sources.boundary_height_mm_at_contour_key(
                    point.xz_key(),
                    previous_key,
                    next_key,
                )?;
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
    }
    Ok(())
}

fn footprint_boundary_xz_point_loop_from_contour(
    contour: &[[f64; 2]],
) -> Vec<NodeFootprintBoundaryPoint> {
    contour
        .iter()
        .map(|point| {
            let key = NodeArrangementKey::from_point(RoadVec2::new(point[0], point[1]));
            NodeFootprintBoundaryPoint::new(ArrangementBoundaryPointKey {
                x_key: key.x_key(),
                z_key: key.z_key(),
                y_mm: 0,
            })
        })
        .collect()
}

fn resolve_footprint_boundary_point_heights(
    points: Vec<NodeFootprintBoundaryPoint>,
    boundary_export_sources: &NodeFootprintBoundaryExportSources,
) -> Result<Vec<NodeFootprintBoundaryPoint>, NodeBoundaryExportError> {
    let mut resolved = Vec::with_capacity(points.len());
    for index in 0..points.len() {
        let point = points[index];
        let key = point.xz_key();
        let previous_key = points[if index == 0 {
            points.len() - 1
        } else {
            index - 1
        }]
        .xz_key();
        let next_key = points[(index + 1) % points.len()].xz_key();
        let Some(height_mm) = boundary_export_sources.boundary_height_mm_at_contour_key(
            key,
            previous_key,
            next_key,
        )?
        else {
            return Err(NodeBoundaryExportError::MissingFootprintBoundaryHeight {
                x_key: key.x_key(),
                z_key: key.z_key(),
            });
        };
        resolved.push(NodeFootprintBoundaryPoint::new(
            ArrangementBoundaryPointKey {
                x_key: key.x_key(),
                z_key: key.z_key(),
                y_mm: height_mm,
            },
        ));
    }
    Ok(canonicalize_footprint_boundary_point_loop(resolved))
}

fn post_height_footprint_boundary_point_loops(
    points: Vec<NodeFootprintBoundaryPoint>,
) -> Vec<Vec<NodeFootprintBoundaryPoint>> {
    same_winding_boundary_point_loops_from_loop(&points)
        .into_iter()
        .flat_map(|points| {
            let mut points = canonicalize_footprint_boundary_point_loop(points);
            remove_subbudget_same_xz_footprint_boundary_vertices(&mut points);
            points = canonicalize_footprint_boundary_point_loop(points);
            same_winding_boundary_point_loops_from_loop(&points)
        })
        .filter_map(|mut points| {
            remove_subbudget_same_xz_footprint_boundary_vertices(&mut points);
            let points = canonicalize_footprint_boundary_point_loop(points);
            if points.len() < 3 {
                return None;
            }
            if signed_footprint_boundary_point_loop_area_xz(&points).abs()
                <= footprint_boundary_point_loop_numeric_area_budget_m2(&points)
            {
                return None;
            }
            Some(points)
        })
        .collect()
}

fn remove_subbudget_same_xz_footprint_boundary_vertices(
    points: &mut Vec<NodeFootprintBoundaryPoint>,
) {
    loop {
        if points.len() < 4 {
            return;
        }
        let mut removed = false;
        for index in 0..points.len() {
            let previous = if index == 0 {
                points.len() - 1
            } else {
                index - 1
            };
            let next = if index + 1 == points.len() {
                0
            } else {
                index + 1
            };
            if points[previous].xz_key() != points[index].xz_key()
                && points[index].xz_key() != points[next].xz_key()
                && points[previous].xz_key() != points[next].xz_key()
            {
                continue;
            }
            let local_points = [
                points[previous].point_world(),
                points[index].point_world(),
                points[next].point_world(),
            ];
            if RoadSurfaceSystem::signed_polygon_area_xz(&local_points).abs()
                > boundary_points_numeric_area_budget_m2(&local_points)
            {
                continue;
            }
            points.remove(index);
            removed = true;
            break;
        }
        if !removed {
            return;
        }
    }
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
