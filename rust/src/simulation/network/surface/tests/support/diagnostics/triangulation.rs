//! Triangulation diagnostic extraction helpers.

use super::*;
use crate::simulation::network::surface::keys::SurfaceXzKey;

pub(in crate::simulation::network::surface::tests) fn triangulation_height_conflict_debug(
    heights: &super::height::NodeHeightSolution,
    ownership: &super::ownership::NodeBooleanOwnership,
    report: &NodeValidationReport,
) -> Option<String> {
    report.diagnostics.iter().find_map(|diagnostic| {
        if let NodeGeometryDiagnosticKind::HeightConflict { x_mm, z_mm, .. } = diagnostic.kind {
            let surface_key = SurfaceXzKey::from_raw_keys(x_mm, z_mm);
            Some(format!(
                "surface_vertices={:?}",
                height_solution_vertices_at_surface_key(heights, surface_key),
            ))
        } else if let NodeGeometryDiagnosticKind::CrossRegionHeightConflict {
            edge_start_x_key,
            edge_start_z_key,
            edge_end_x_key,
            edge_end_z_key,
            ..
        } = diagnostic.kind
        {
            let start_key = arrangement_key_from_overlay_keys(edge_start_x_key, edge_start_z_key);
            let end_key = arrangement_key_from_overlay_keys(edge_end_x_key, edge_end_z_key);
            Some(format!(
                "start_vertices={:?} end_vertices={:?} ownership={:?}",
                height_solution_vertices_at_arrangement_key(heights, start_key),
                height_solution_vertices_at_arrangement_key(heights, end_key),
                owned_region_claims_for_height_conflict(ownership, diagnostic)
            ))
        } else {
            None
        }
    })
}

pub(in crate::simulation::network::surface::tests) fn triangulation_duplicate_exposed_edge_debug(
    triangulation: &crate::simulation::network::surface::triangulation::NodeTriangulationSolution,
    report: &NodeValidationReport,
) -> Option<String> {
    report.diagnostics.iter().find_map(|diagnostic| {
        if let NodeGeometryDiagnosticKind::DuplicateExposedEdge {
            start_x_mm,
            start_z_mm,
            end_x_mm,
            end_z_mm,
            ..
        } = diagnostic.kind
        {
            Some(format!(
                "duplicate_edge_regions={:?}",
                triangulation_regions_for_exposed_edge(
                    triangulation,
                    (start_x_mm, start_z_mm),
                    (end_x_mm, end_z_mm),
                )
            ))
        } else {
            None
        }
    })
}

pub(in crate::simulation::network::surface::tests) fn triangulation_open_boundary_debug(
    triangulation: &crate::simulation::network::surface::triangulation::NodeTriangulationSolution,
    report: &NodeValidationReport,
) -> Option<String> {
    let region_indices = report
        .diagnostics
        .iter()
        .filter_map(|diagnostic| {
            if let NodeGeometryDiagnosticKind::OpenBoundary { region_index, .. } = diagnostic.kind {
                Some(region_index)
            } else {
                None
            }
        })
        .collect::<BTreeSet<_>>();
    if region_indices.is_empty() {
        return None;
    }
    let open_regions = region_indices
        .into_iter()
        .filter_map(|region_index| {
            let region = triangulation.regions.get(region_index)?;
            let mut degree = BTreeMap::<(i64, i64), usize>::new();
            let mut incident_edges = BTreeMap::<(i64, i64), Vec<((i64, i64), (i64, i64))>>::new();
            for constraint in &region.boundary_constraints {
                let Some(start) = region.vertices.get(constraint[0]) else {
                    continue;
                };
                let Some(end) = region.vertices.get(constraint[1]) else {
                    continue;
                };
                let start_key = road_vec3_raw_xz(start.point_world);
                let end_key = road_vec3_raw_xz(end.point_world);
                if start_key == end_key {
                    continue;
                }
                *degree.entry(start_key).or_default() += 1;
                *degree.entry(end_key).or_default() += 1;
                incident_edges
                    .entry(start_key)
                    .or_default()
                    .push((start_key, end_key));
                incident_edges
                    .entry(end_key)
                    .or_default()
                    .push((start_key, end_key));
            }
            let bad_points = degree
                .into_iter()
                .filter(|(_, degree)| *degree != 2)
                .take(12)
                .map(|(point, degree)| {
                    (
                        point,
                        raw_xz_to_mm(point),
                        degree,
                        incident_edges.remove(&point).unwrap_or_default(),
                    )
                })
                .collect::<Vec<_>>();
            Some((
                region_index,
                region.owner,
                region.height_field_id,
                region.vertices.len(),
                region.boundary_constraints.len(),
                region.triangles.len(),
                bad_points,
            ))
        })
        .collect::<Vec<_>>();
    Some(format!("open_boundary_debug={open_regions:?}"))
}

pub(in crate::simulation::network::surface::tests) fn triangulation_regions_for_exposed_edge(
    triangulation: &crate::simulation::network::surface::triangulation::NodeTriangulationSolution,
    start_mm: (i64, i64),
    end_mm: (i64, i64),
) -> Vec<String> {
    let expected = normalized_test_mm_edge_key(start_mm, end_mm);
    let mut matches = Vec::new();
    for (region_index, region) in triangulation.regions.iter().enumerate() {
        let mut edge_counts = BTreeMap::<((i64, i64), (i64, i64)), usize>::new();
        for triangle in &region.triangles {
            for edge_index in 0..3 {
                let start = &region.vertices[triangle.vertices[edge_index]];
                let end = &region.vertices[triangle.vertices[(edge_index + 1) % 3]];
                *edge_counts
                    .entry(normalized_test_world_mm_edge_key(
                        start.point_world.x as f32,
                        start.point_world.z as f32,
                        end.point_world.x as f32,
                        end.point_world.z as f32,
                    ))
                    .or_default() += 1;
            }
        }
        if let Some(count) = edge_counts.get(&expected).copied() {
            matches.push(format!(
                "region={} owner={:?} height_field={:?} local_count={}",
                region_index, region.owner, region.height_field_id, count
            ));
        }
    }
    matches
}

fn road_vec3_raw_xz(point: super::backend::RoadVec3) -> (i64, i64) {
    let key = SurfaceXzKey::from_world_xz(point);
    (key.x_key(), key.z_key())
}

fn raw_xz_to_mm(point: (i64, i64)) -> (i64, i64) {
    (
        SurfaceXzKey::coordinate_key_to_mm(point.0),
        SurfaceXzKey::coordinate_key_to_mm(point.1),
    )
}

pub(in crate::simulation::network::surface::tests) fn normalized_test_world_mm_edge_key(
    start_x: f32,
    start_z: f32,
    end_x: f32,
    end_z: f32,
) -> ((i64, i64), (i64, i64)) {
    normalized_test_mm_edge_key(
        (
            (start_x * 1000.0).round() as i64,
            (start_z * 1000.0).round() as i64,
        ),
        (
            (end_x * 1000.0).round() as i64,
            (end_z * 1000.0).round() as i64,
        ),
    )
}

pub(in crate::simulation::network::surface::tests) fn normalized_test_mm_edge_key(
    start: (i64, i64),
    end: (i64, i64),
) -> ((i64, i64), (i64, i64)) {
    if start <= end {
        (start, end)
    } else {
        (end, start)
    }
}

pub(in crate::simulation::network::surface::tests) fn owned_region_claims_for_height_conflict(
    ownership: &super::ownership::NodeBooleanOwnership,
    diagnostic: &super::validation::NodeGeometryDiagnostic,
) -> Vec<String> {
    let NodeGeometryDiagnosticKind::CrossRegionHeightConflict {
        existing_region_index,
        incoming_region_index,
        ..
    } = diagnostic.kind
    else {
        return Vec::new();
    };
    [existing_region_index, incoming_region_index]
        .into_iter()
        .filter_map(|region_index| {
            ownership.owned_regions.get(region_index).map(|region| {
                format!(
                    "region={} kind={:?} owner={:?} claim={:?} source_mouth={} source_band={:?} area={:.6}",
                    region_index,
                    region.kind,
                    region.owner,
                    region.claim_priority,
                    region.source_mouth_order_index,
                    region.source_band_index,
                    region.area_m2
                )
            })
        })
        .collect()
}
