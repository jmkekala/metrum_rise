// SPDX-License-Identifier: GPL-2.0-only

//! Footprint boundary loop export from the final boolean road-owned footprint.

use super::super::{
    NODE_OVERLAY_MIN_AREA_M2, NODE_OVERLAY_NUMERIC_DUST_WIDTH_M, NodeOverlayShapes,
    NodeOwnedRegion, SAMPLE_EPSILON_M,
    arrangement::NodeArrangementKey,
    backend::{ROAD_OVERLAY_COORDINATE_SCALE, RoadVec2, RoadVec3},
    boundary::{
        ArrangementBoundaryPointKey, NodeBoundaryExportError, NodeFootprintBoundaryExportSources,
        NodeFootprintBoundaryPoint, remove_subbudget_unsupported_numeric_boundary_vertices,
        same_winding_boundary_point_loops_from_loop,
    },
};
use crate::simulation::network::surface::RoadSurfaceSystem;
use std::collections::BTreeSet;

#[derive(Clone, Copy)]
struct FootprintTopTriangleSupport {
    triangle: [RoadVec3; 3],
    min_x: f64,
    max_x: f64,
    min_z: f64,
    max_z: f64,
}

impl RoadSurfaceSystem {
    pub(super) fn footprint_boundary_point_loops_from_footprint_shapes(
        footprint_shapes: &NodeOverlayShapes,
        top_regions: &[NodeOwnedRegion],
        boundary_export_sources: &mut NodeFootprintBoundaryExportSources,
    ) -> Result<Vec<Vec<NodeFootprintBoundaryPoint>>, NodeBoundaryExportError> {
        let mut loops = Vec::new();
        let mut emitted_loop_identities = BTreeSet::<Vec<ArrangementBoundaryPointKey>>::new();
        let top_supports = footprint_top_triangle_supports_from_regions(top_regions);
        for shape in footprint_shapes {
            for contour in shape {
                let points = footprint_boundary_xz_point_loop_from_contour(contour);
                push_valid_footprint_boundary_point_loops(
                    points,
                    &top_supports,
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
    top_supports: &[FootprintTopTriangleSupport],
    boundary_export_sources: &mut NodeFootprintBoundaryExportSources,
    emitted_loop_identities: &mut BTreeSet<Vec<ArrangementBoundaryPointKey>>,
    loops: &mut Vec<Vec<NodeFootprintBoundaryPoint>>,
) -> Result<(), NodeBoundaryExportError> {
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
        for split_points in
            post_height_footprint_boundary_point_loops(heighted_points, top_supports)
        {
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
        let point_key = ArrangementBoundaryPointKey {
            x_key: key.x_key(),
            z_key: key.z_key(),
            y_mm: height_mm,
        };
        let point_key = boundary_export_sources
            .canonical_final_height_endpoint_dust_point(point_key)
            .unwrap_or(point_key);
        resolved.push(NodeFootprintBoundaryPoint::new(point_key));
    }
    Ok(canonicalize_footprint_boundary_point_loop(resolved))
}

fn post_height_footprint_boundary_point_loops(
    points: Vec<NodeFootprintBoundaryPoint>,
    top_supports: &[FootprintTopTriangleSupport],
) -> Vec<Vec<NodeFootprintBoundaryPoint>> {
    same_winding_boundary_point_loops_from_loop(&points)
        .into_iter()
        .flat_map(|points| {
            let mut points = canonicalize_footprint_boundary_point_loop(points);
            remove_subbudget_numeric_footprint_boundary_vertices(&mut points, top_supports);
            points = canonicalize_footprint_boundary_point_loop(points);
            same_winding_boundary_point_loops_from_loop(&points)
        })
        .filter_map(|mut points| {
            remove_subbudget_numeric_footprint_boundary_vertices(&mut points, top_supports);
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

fn remove_subbudget_numeric_footprint_boundary_vertices(
    points: &mut Vec<NodeFootprintBoundaryPoint>,
    top_supports: &[FootprintTopTriangleSupport],
) {
    remove_subbudget_same_xz_footprint_boundary_vertices(points);
    remove_subbudget_unsupported_numeric_boundary_vertices(points, |point_key, local_points| {
        footprint_boundary_point_has_visible_top_support(point_key, top_supports)
            || RoadSurfaceSystem::signed_polygon_area_xz(&local_points).abs()
                > footprint_boundary_points_numeric_area_budget_m2(&local_points)
    });
    remove_subbudget_same_xz_footprint_boundary_vertices(points);
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
                footprint_boundary_point_world(points[previous]),
                footprint_boundary_point_world(points[index]),
                footprint_boundary_point_world(points[next]),
            ];
            if RoadSurfaceSystem::signed_polygon_area_xz(&local_points).abs()
                > footprint_boundary_points_numeric_area_budget_m2(&local_points)
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

fn footprint_boundary_point_has_visible_top_support(
    point_key: ArrangementBoundaryPointKey,
    top_supports: &[FootprintTopTriangleSupport],
) -> bool {
    let point = footprint_boundary_point_world(NodeFootprintBoundaryPoint::new(point_key));
    top_supports
        .iter()
        .copied()
        .any(|support| support.supports_boundary_point(point))
}

fn footprint_top_triangle_supports_from_regions(
    top_regions: &[NodeOwnedRegion],
) -> Vec<FootprintTopTriangleSupport> {
    top_regions
        .iter()
        .flat_map(|region| region.polygon.triangles_world.iter().copied())
        .map(FootprintTopTriangleSupport::new)
        .collect()
}

impl FootprintTopTriangleSupport {
    fn new(triangle: [RoadVec3; 3]) -> Self {
        let min_x = triangle
            .iter()
            .map(|point| point.x)
            .fold(f64::INFINITY, f64::min);
        let max_x = triangle
            .iter()
            .map(|point| point.x)
            .fold(f64::NEG_INFINITY, f64::max);
        let min_z = triangle
            .iter()
            .map(|point| point.z)
            .fold(f64::INFINITY, f64::min);
        let max_z = triangle
            .iter()
            .map(|point| point.z)
            .fold(f64::NEG_INFINITY, f64::max);
        Self {
            triangle,
            min_x,
            max_x,
            min_z,
            max_z,
        }
    }

    fn supports_boundary_point(self, point: RoadVec3) -> bool {
        let tolerance = visible_top_match_tolerance_m();
        if point.x < self.min_x - tolerance
            || point.x > self.max_x + tolerance
            || point.z < self.min_z - tolerance
            || point.z > self.max_z + tolerance
        {
            return false;
        }
        let Some((wa, wb, wc)) = RoadSurfaceSystem::triangle_barycentric_weights_xz(
            self.triangle,
            RoadVec2::new(point.x, point.z),
        ) else {
            return false;
        };
        let height = self.triangle[0].y * f64::from(wa)
            + self.triangle[1].y * f64::from(wb)
            + self.triangle[2].y * f64::from(wc);
        (height - point.y).abs() <= tolerance
    }
}

fn visible_top_match_tolerance_m() -> f64 {
    f64::from(SAMPLE_EPSILON_M) * 2.0
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
) -> Vec<RoadVec3> {
    points
        .iter()
        .copied()
        .map(footprint_boundary_point_world)
        .collect()
}

fn footprint_boundary_point_world(point: NodeFootprintBoundaryPoint) -> RoadVec3 {
    RoadVec3::new(
        point.point_key.x_key as f64 / ROAD_OVERLAY_COORDINATE_SCALE,
        point.point_key.y_mm as f64 / 1000.0,
        point.point_key.z_key as f64 / ROAD_OVERLAY_COORDINATE_SCALE,
    )
}

fn signed_footprint_boundary_point_loop_area_xz(points: &[NodeFootprintBoundaryPoint]) -> f32 {
    RoadSurfaceSystem::signed_polygon_area_xz(&footprint_boundary_point_loop_world_points(points))
}

fn footprint_boundary_point_loop_numeric_area_budget_m2(
    points: &[NodeFootprintBoundaryPoint],
) -> f32 {
    footprint_boundary_points_numeric_area_budget_m2(&footprint_boundary_point_loop_world_points(
        points,
    ))
}

fn footprint_boundary_points_numeric_area_budget_m2(points: &[RoadVec3]) -> f32 {
    if points.len() < 2 {
        return NODE_OVERLAY_MIN_AREA_M2;
    }
    let perimeter_m = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(start, end)| RoadVec2::new(start.x - end.x, start.z - end.z).length())
        .sum::<f64>() as f32;
    // This is the uncertainty of a complete boundary, not one boolean-operation residual.
    // Capping its accumulated area makes an unchanged sub-resolution seam become a real
    // terrain hole merely because it is longer or joins another seam at the junction.
    perimeter_m * NODE_OVERLAY_NUMERIC_DUST_WIDTH_M + points.len() as f32 * NODE_OVERLAY_MIN_AREA_M2
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::surface::RoadSurfaceVisualNodePieceKind;

    fn export_loop(contour: &[[f64; 2]]) -> Result<usize, NodeBoundaryExportError> {
        let mut sources = NodeFootprintBoundaryExportSources::from_owned_regions(
            45,
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &[],
            &[],
            &[],
            &[],
        )?;
        let mut identities = BTreeSet::new();
        let mut loops = Vec::new();
        push_valid_footprint_boundary_point_loops(
            footprint_boundary_xz_point_loop_from_contour(contour),
            &[],
            &mut sources,
            &mut identities,
            &mut loops,
        )?;
        Ok(loops.len())
    }

    #[test]
    fn logged_junction_seam_does_not_become_a_terrain_hole() {
        // Saved junction 45: 29.6 m of boundary enclosing 0.0013775 m² of overlay dust.
        // Its opposing curb heights differ by 92 mm and cannot share a terrain vertex.
        let contour = vec![
            [2817.34375, -8223.773438],
            [2820.83585, -8223.538413],
            [2820.625401, -8220.41146],
            [2820.332087, -8216.051711],
            [2820.245228, -8214.760651],
            [2820.332129, -8216.051708],
            [2820.500081, -8218.547104],
            [2820.66802, -8221.0425],
            [2820.835992, -8223.538582],
            [2821.003859, -8226.033296],
            [2820.835862, -8223.538591],
        ];
        for reversed in [false, true] {
            let mut candidate = contour.clone();
            if reversed {
                candidate.reverse();
            }
            for _ in 0..candidate.len() {
                assert_eq!(
                    export_loop(&candidate).expect("dust boundary must be omitted"),
                    0
                );
                candidate.rotate_left(1);
            }
        }
    }

    #[test]
    fn resolved_hole_is_not_discarded_by_boundary_uncertainty() {
        // A real hole of comparable area must reach height/source validation.
        let contour = [[0.0, 0.0], [0.04, 0.0], [0.04, 0.04], [0.0, 0.04]];
        assert!(export_loop(&contour).is_err());
    }
}
