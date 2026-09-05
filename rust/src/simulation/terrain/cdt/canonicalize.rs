// SPDX-License-Identifier: GPL-2.0-only

//! Canonical input ordering, clipping, vertex insertion, and stage assembly.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::*;

pub(super) struct CanonicalTerrainCdtInput {
    pub(super) vertices: Vec<TerrainCdtVertex>,
    pub(super) constraints: Vec<[usize; 2]>,
    pub(super) road_constraint_edges: Vec<[usize; 2]>,
    pub(super) road_constraint_sources: BTreeMap<[usize; 2], TerrainCdtRoadConstraintSource>,
    pub(super) road_loops: Vec<CanonicalTerrainCdtRoadLoop>,
    pub(super) accepted_seam_edges: usize,
    pub(super) merged_subbudget_seam_edges: usize,
    pub(super) retaining_wall_required_seam_edges: usize,
    pub(super) internal_road_constraint_edges: usize,
    pub(super) invalid_constraint_edges: usize,
    pub(super) retaining_wall_required_sources: Vec<TerrainCdtRoadBoundarySource>,
    pub(super) blocking_degenerate_seam_edges: usize,
    pub(super) seam_quality_samples: Vec<TerrainCdtSeamQualitySample>,
    pub(super) tie_in_widened_source_samples: usize,
    pub(super) tie_in_widened_max_y_delta_m: f32,
    pub(super) tie_in_widened_max_slope_ratio: f32,
    pub(super) tie_in_widened_samples: Vec<TerrainCdtTieInSample>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct CanonicalTerrainCdtRoadLoop {
    pub(super) footprint_group_id: u64,
    pub(super) is_hole: bool,
    pub(super) vertices: Vec<TerrainCdtVertex>,
    pub(super) edge_sources: Vec<Option<TerrainCdtRoadBoundarySource>>,
    pub(super) min_x: f64,
    pub(super) min_z: f64,
    pub(super) max_x: f64,
    pub(super) max_z: f64,
    pub(super) min_height_m: f32,
    pub(super) max_height_m: f32,
    pub(super) sourced_edges: Vec<CanonicalTerrainCdtSourcedEdge>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct CanonicalTerrainCdtSourcedEdge {
    pub(super) start_x: f64,
    pub(super) start_z: f64,
    pub(super) start_height_m: f32,
    pub(super) delta_x: f64,
    pub(super) delta_z: f64,
    pub(super) delta_height_m: f32,
    pub(super) length_squared_m: f64,
    pub(super) min_x: f64,
    pub(super) min_z: f64,
    pub(super) max_x: f64,
    pub(super) max_z: f64,
    pub(super) source: TerrainCdtRoadBoundarySource,
}

pub(super) fn canonicalize_input(
    mut input: TerrainCdtInput,
) -> Result<CanonicalTerrainCdtInput, TerrainCdtError> {
    let expected_vertex_count = 4usize
        .saturating_add(input.source_samples.len())
        .saturating_add(input.tie_in_guide_samples.len())
        .saturating_add(input.tie_in_guide_constraints.len().saturating_mul(2));
    let mut vertices = Vec::with_capacity(expected_vertex_count);
    // Iteration order is never observed; canonical input ordering determines stable vertex IDs.
    let mut vertex_lookup = HashMap::with_capacity(expected_vertex_count);
    let mut road_vertex_heights = BTreeMap::new();
    let mut constraint_set = BTreeSet::new();
    let mut road_constraint_edges = Vec::new();
    let mut road_constraint_sources = BTreeMap::new();
    let mut road_loops = Vec::new();
    let mut source_sample_vertex_indices = Vec::new();
    let mut accepted_seam_edges = 0usize;
    let mut merged_subbudget_seam_edges = 0usize;
    let mut retaining_wall_required_seam_edges = 0usize;
    let mut retaining_wall_required_sources = Vec::new();
    let mut blocking_degenerate_seam_edges = 0usize;
    let mut invalid_constraint_edges = 0usize;
    let mut seam_quality_samples = Vec::new();
    let mut tie_in_widened_source_samples = 0usize;
    let mut tie_in_widened_max_y_delta_m = 0.0_f32;
    let mut tie_in_widened_max_slope_ratio = 0.0_f32;
    let mut tie_in_widened_samples = Vec::new();

    let patch_corners = input.patch.corners_cw();
    for &vertex in &patch_corners {
        insert_vertex(vertex, &mut vertices, &mut vertex_lookup);
    }

    input.road_loops.sort_by_key(|road_loop| {
        (
            road_loop.footprint_group_id,
            road_loop.is_hole,
            road_loop.stable_piece_id,
            road_loop.local_loop_index,
        )
    });
    for road_loop in input.road_loops {
        let original_source_edges = normalized_road_loop_source_edges(&road_loop);
        let original_points = simplified_road_loop(road_loop.vertices)?;
        if original_points.len() < 3
            || signed_area(&original_points).abs() <= CDT_EPSILON_M * CDT_EPSILON_M
        {
            continue;
        }
        for points in clip_loop_to_patch_components(&original_points, input.patch) {
            let points =
                split_road_loop_segments_at_source_vertices(points, &original_source_edges)?;
            if points.len() < 3 || signed_area(&points).abs() <= CDT_EPSILON_M * CDT_EPSILON_M {
                continue;
            }
            let seam_quality =
                harden_terrain_cdt_road_loop_seams(points, &original_source_edges, input.patch);
            let points = seam_quality.points;
            let edge_sources = seam_quality.edge_sources;
            accepted_seam_edges += seam_quality.accepted_seam_edges;
            merged_subbudget_seam_edges += seam_quality.merged_subbudget_seam_edges;
            retaining_wall_required_seam_edges += seam_quality.retaining_wall_required_seam_edges;
            blocking_degenerate_seam_edges += seam_quality.blocking_degenerate_seam_edges;
            append_seam_quality_samples(&mut seam_quality_samples, seam_quality.samples);
            if points.len() < 3 || signed_area(&points).abs() <= CDT_EPSILON_M * CDT_EPSILON_M {
                continue;
            }
            let site_owned_loop = road_loop_edge_sources_are_building_site_only(&edge_sources);
            let loop_indices = points
                .iter()
                .map(|&vertex| {
                    insert_road_vertex(
                        vertex,
                        site_owned_loop,
                        &mut vertices,
                        &mut vertex_lookup,
                        &mut road_vertex_heights,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            push_road_loop_constraints(
                &loop_indices,
                &vertices,
                input.patch,
                TerrainCdtRoadLoopConstraintIdentity {
                    stable_piece_id: road_loop.stable_piece_id,
                    local_loop_index: road_loop.local_loop_index,
                },
                &edge_sources,
                &mut road_constraint_edges,
                &mut road_constraint_sources,
            )?;
            let loop_bounds = terrain_cdt_loop_bounds(&points);
            let (min_height_m, max_height_m) = points.iter().fold(
                (f32::INFINITY, f32::NEG_INFINITY),
                |(min_height_m, max_height_m), point| {
                    (
                        min_height_m.min(point.height_m),
                        max_height_m.max(point.height_m),
                    )
                },
            );
            let sourced_edges = canonical_terrain_cdt_sourced_edges(&points, &edge_sources);
            road_loops.push(CanonicalTerrainCdtRoadLoop {
                footprint_group_id: road_loop.footprint_group_id,
                is_hole: road_loop.is_hole,
                min_x: loop_bounds.min_x,
                min_z: loop_bounds.min_z,
                max_x: loop_bounds.max_x,
                max_z: loop_bounds.max_z,
                min_height_m,
                max_height_m,
                sourced_edges,
                vertices: points,
                edge_sources,
            });
        }
    }

    input
        .tie_in_guide_samples
        .sort_by_cached_key(|sample| terrain_cdt_vertex_key(sample.vertex));
    for sample in input.tie_in_guide_samples {
        let vertex = sample.vertex;
        if !tie_in_guide_vertex_is_valid(vertex, input.patch, &road_loops) {
            continue;
        }
        insert_vertex(vertex, &mut vertices, &mut vertex_lookup);
    }

    input
        .tie_in_guide_constraints
        .sort_by_cached_key(|constraint| {
            (
                terrain_cdt_vertex_key(constraint.start),
                terrain_cdt_vertex_key(constraint.end),
            )
        });
    for constraint in input.tie_in_guide_constraints {
        if !tie_in_guide_vertex_is_valid(constraint.start, input.patch, &road_loops)
            || !tie_in_guide_vertex_is_valid(constraint.end, input.patch, &road_loops)
            || same_xz(constraint.start, constraint.end)
        {
            continue;
        }
        let start = insert_vertex(constraint.start, &mut vertices, &mut vertex_lookup);
        let end = insert_vertex(constraint.end, &mut vertices, &mut vertex_lookup);
        insert_constraint([start, end], &mut constraint_set);
    }

    input
        .source_samples
        .sort_by_cached_key(|sample| terrain_cdt_vertex_key(*sample));
    for sample in input.source_samples {
        if !patch_contains(sample, input.patch) {
            continue;
        }
        if point_inside_any_road_footprint(sample, &road_loops) {
            continue;
        }
        if let Some(tie_in_sample) =
            widening_tie_in_sample_against_any_road_loop(sample, &road_loops)
        {
            tie_in_widened_source_samples += 1;
            tie_in_widened_max_y_delta_m =
                tie_in_widened_max_y_delta_m.max(tie_in_sample.height_delta_m);
            tie_in_widened_max_slope_ratio =
                tie_in_widened_max_slope_ratio.max(tie_in_sample.slope_ratio);
            if tie_in_sample.height_delta_m >= MIN_RETAINING_WALL_TIE_IN_HEIGHT_DELTA_M
                && terrain_cdt_boundary_source_requires_widened_sample_retaining_wall(
                    tie_in_sample.seam_source,
                )
            {
                retaining_wall_required_sources.push(tie_in_sample.seam_source);
            }
            insert_tie_in_widened_sample(&mut tie_in_widened_samples, tie_in_sample);
            continue;
        }
        let previous_vertex_count = vertices.len();
        let vertex_index = insert_vertex(sample, &mut vertices, &mut vertex_lookup);
        if vertices.len() > previous_vertex_count {
            source_sample_vertex_indices.push(vertex_index);
        }
    }

    invalid_constraint_edges += node_road_constraint_edges(
        &mut vertices,
        &mut vertex_lookup,
        input.patch,
        &source_sample_vertex_indices,
        &mut road_constraint_edges,
        &mut road_constraint_sources,
    );
    let internal_road_constraint_edges = retain_exposed_road_constraint_edges(
        &vertices,
        &road_loops,
        &mut road_constraint_edges,
        &mut road_constraint_sources,
    );
    push_patch_boundary_constraints(input.patch, &vertices, &mut constraint_set);
    for edge in &road_constraint_edges {
        insert_constraint(*edge, &mut constraint_set);
    }
    sort_dedup_terrain_cdt_boundary_sources(&mut retaining_wall_required_sources);

    Ok(CanonicalTerrainCdtInput {
        vertices,
        constraints: constraint_set.into_iter().collect(),
        road_constraint_edges,
        road_constraint_sources,
        road_loops,
        accepted_seam_edges,
        merged_subbudget_seam_edges,
        retaining_wall_required_seam_edges,
        internal_road_constraint_edges,
        invalid_constraint_edges,
        retaining_wall_required_sources,
        blocking_degenerate_seam_edges,
        seam_quality_samples,
        tie_in_widened_source_samples,
        tie_in_widened_max_y_delta_m,
        tie_in_widened_max_slope_ratio,
        tie_in_widened_samples,
    })
}

fn canonical_terrain_cdt_sourced_edges(
    vertices: &[TerrainCdtVertex],
    edge_sources: &[Option<TerrainCdtRoadBoundarySource>],
) -> Vec<CanonicalTerrainCdtSourcedEdge> {
    if vertices.len() < 2 {
        return Vec::new();
    }
    edge_sources
        .iter()
        .copied()
        .enumerate()
        .filter_map(|(index, source)| {
            let source = source?;
            let start = vertices[index];
            let end = vertices[(index + 1) % vertices.len()];
            let delta_x = end.x - start.x;
            let delta_z = end.z - start.z;
            Some(CanonicalTerrainCdtSourcedEdge {
                start_x: start.x,
                start_z: start.z,
                start_height_m: start.height_m,
                delta_x,
                delta_z,
                delta_height_m: end.height_m - start.height_m,
                length_squared_m: delta_x * delta_x + delta_z * delta_z,
                min_x: start.x.min(end.x),
                min_z: start.z.min(end.z),
                max_x: start.x.max(end.x),
                max_z: start.z.max(end.z),
                source,
            })
        })
        .collect()
}

fn tie_in_guide_vertex_is_valid(
    vertex: TerrainCdtVertex,
    patch: TerrainCdtPatch,
    road_loops: &[CanonicalTerrainCdtRoadLoop],
) -> bool {
    patch_contains(vertex, patch)
        && !point_inside_any_road_footprint(vertex, road_loops)
        && widening_tie_in_sample_against_any_road_loop(vertex, road_loops).is_none()
}

#[derive(Clone, Copy)]
pub(super) struct TerrainCdtRoadVertexHeight {
    pub(super) height_m: f32,
    pub(super) site_owned_only: bool,
}

pub(super) fn insert_vertex(
    vertex: TerrainCdtVertex,
    vertices: &mut Vec<TerrainCdtVertex>,
    vertex_lookup: &mut HashMap<(i64, i64), usize>,
) -> usize {
    let key = terrain_cdt_vertex_xz_key(vertex);
    if let Some(index) = vertex_lookup.get(&key) {
        return *index;
    }
    let index = vertices.len();
    vertices.push(vertex);
    vertex_lookup.insert(key, index);
    index
}

fn insert_road_vertex(
    vertex: TerrainCdtVertex,
    site_owned_only: bool,
    vertices: &mut Vec<TerrainCdtVertex>,
    vertex_lookup: &mut HashMap<(i64, i64), usize>,
    road_vertex_heights: &mut BTreeMap<(i64, i64), TerrainCdtRoadVertexHeight>,
) -> Result<usize, TerrainCdtError> {
    let key = terrain_cdt_vertex_xz_key(vertex);
    let candidate = TerrainCdtRoadVertexHeight {
        height_m: vertex.height_m,
        site_owned_only,
    };
    if let Some(existing) = road_vertex_heights.get(&key).copied() {
        let merged = merge_road_vertex_height(existing, candidate)
            .ok_or(TerrainCdtError::ConflictingRoadBoundaryHeight)?;
        let index = vertex_lookup
            .get(&key)
            .copied()
            .ok_or(TerrainCdtError::ConflictingRoadBoundaryHeight)?;
        vertices[index].height_m = merged.height_m;
        road_vertex_heights.insert(key, merged);
        return Ok(index);
    }

    let index = if let Some(index) = vertex_lookup.get(&key).copied() {
        // Exact road ownership replaces an unsourced patch-corner height at the same X/Z.
        vertices[index].height_m = vertex.height_m;
        index
    } else {
        let index = vertices.len();
        vertices.push(vertex);
        vertex_lookup.insert(key, index);
        index
    };
    road_vertex_heights.insert(key, candidate);
    Ok(index)
}

pub(super) fn merge_road_vertex_height(
    existing: TerrainCdtRoadVertexHeight,
    candidate: TerrainCdtRoadVertexHeight,
) -> Option<TerrainCdtRoadVertexHeight> {
    if same_height(existing.height_m, candidate.height_m) {
        return Some(TerrainCdtRoadVertexHeight {
            height_m: existing.height_m,
            site_owned_only: existing.site_owned_only && candidate.site_owned_only,
        });
    }
    match (existing.site_owned_only, candidate.site_owned_only) {
        (true, false) => Some(candidate),
        (false, true) => Some(existing),
        _ => None,
    }
}

pub(super) fn road_loop_edge_sources_are_building_site_only(
    edge_sources: &[Option<TerrainCdtRoadBoundarySource>],
) -> bool {
    !edge_sources.is_empty()
        && edge_sources.iter().all(|source| {
            matches!(
                source,
                Some(TerrainCdtRoadBoundarySource::BuildingSiteBoundary { .. })
            )
        })
}
