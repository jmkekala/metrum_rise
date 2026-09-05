// SPDX-License-Identifier: GPL-2.0-only

//! Patch rails, constraint noding, and deterministic segment geometry.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::super::*;

pub(in crate::simulation::terrain::cdt) fn push_patch_boundary_constraints(
    patch: TerrainCdtPatch,
    vertices: &[TerrainCdtVertex],
    constraint_set: &mut BTreeSet<[usize; 2]>,
) {
    let mut left = Vec::new();
    let mut top = Vec::new();
    let mut right = Vec::new();
    let mut bottom = Vec::new();

    for (index, vertex) in vertices.iter().copied().enumerate() {
        if same_coord(vertex.x, patch.min_x)
            && vertex.z >= patch.min_z - CDT_EPSILON_M
            && vertex.z <= patch.max_z + CDT_EPSILON_M
        {
            left.push((quantized_coord(vertex.z), index));
        }
        if same_coord(vertex.z, patch.max_z)
            && vertex.x >= patch.min_x - CDT_EPSILON_M
            && vertex.x <= patch.max_x + CDT_EPSILON_M
        {
            top.push((quantized_coord(vertex.x), index));
        }
        if same_coord(vertex.x, patch.max_x)
            && vertex.z >= patch.min_z - CDT_EPSILON_M
            && vertex.z <= patch.max_z + CDT_EPSILON_M
        {
            right.push((-quantized_coord(vertex.z), index));
        }
        if same_coord(vertex.z, patch.min_z)
            && vertex.x >= patch.min_x - CDT_EPSILON_M
            && vertex.x <= patch.max_x + CDT_EPSILON_M
        {
            bottom.push((-quantized_coord(vertex.x), index));
        }
    }

    push_sorted_boundary_side(&mut left, constraint_set);
    push_sorted_boundary_side(&mut top, constraint_set);
    push_sorted_boundary_side(&mut right, constraint_set);
    push_sorted_boundary_side(&mut bottom, constraint_set);
}

fn push_sorted_boundary_side(
    side: &mut Vec<(i64, usize)>,
    constraint_set: &mut BTreeSet<[usize; 2]>,
) {
    side.sort_unstable();
    side.dedup_by_key(|(_, index)| *index);
    for pair in side.windows(2) {
        insert_constraint([pair[0].1, pair[1].1], constraint_set);
    }
}

pub(in crate::simulation::terrain::cdt) fn insert_constraint(
    edge: [usize; 2],
    constraint_set: &mut BTreeSet<[usize; 2]>,
) {
    let edge = normalize_edge_array(edge[0], edge[1]);
    if edge[0] != edge[1] {
        constraint_set.insert(edge);
    }
}

// Spade accepts a constrained graph but does not node crossing or T-touching
// constraints for us. i_overlay owns roadbed area union; this patch-local pass
// only canonicalizes the final CDT constraint graph. Determinism comes from
// sorted original road loops, quantized XZ vertex lookup, and BTreeSet edge
// emission. Complexity is O(E^2 + E*S) with bbox rejection over one dirty
// terrain patch's roadbed constraints and source samples, outside the per-tick
// simulation hot path.
#[derive(Clone, Copy, Debug, PartialEq)]
struct TerrainCdtRoadConstraintSplit {
    t: f64,
    vertex_index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TerrainCdtSourceSampleConstraintHit {
    edge_index: usize,
    t: f64,
    height_m: f32,
}

pub(in crate::simulation::terrain::cdt) fn node_road_constraint_edges(
    vertices: &mut Vec<TerrainCdtVertex>,
    vertex_lookup: &mut HashMap<(i64, i64), usize>,
    patch: TerrainCdtPatch,
    source_sample_vertex_indices: &[usize],
    road_constraint_edges: &mut Vec<[usize; 2]>,
    road_constraint_sources: &mut BTreeMap<[usize; 2], TerrainCdtRoadConstraintSource>,
) -> usize {
    if road_constraint_edges.len() < 2 {
        return 0;
    }

    let original_edges = road_constraint_edges.clone();
    let original_sources = road_constraint_sources.clone();
    let mut constraint_vertex_site_owned_only =
        terrain_cdt_constraint_vertex_site_ownership(&original_edges, &original_sources);
    let mut invalid_constraint_edges = 0usize;
    let mut split_points = original_edges
        .iter()
        .map(|edge| {
            vec![
                TerrainCdtRoadConstraintSplit {
                    t: 0.0,
                    vertex_index: edge[0],
                },
                TerrainCdtRoadConstraintSplit {
                    t: 1.0,
                    vertex_index: edge[1],
                },
            ]
        })
        .collect::<Vec<_>>();

    for first_index in 0..original_edges.len() {
        for second_index in first_index + 1..original_edges.len() {
            let first_edge = original_edges[first_index];
            let second_edge = original_edges[second_index];
            if first_edge == second_edge {
                continue;
            }
            let first_start = vertices[first_edge[0]];
            let first_end = vertices[first_edge[1]];
            let second_start = vertices[second_edge[0]];
            let second_end = vertices[second_edge[1]];
            if !segment_bounds_overlap(first_start, first_end, second_start, second_end) {
                continue;
            }

            for intersection in
                segment_intersections(first_start, first_end, second_start, second_end)
                    .into_iter()
                    .flatten()
            {
                let first_t =
                    segment_parameter(first_start, first_end, intersection.x, intersection.z);
                let second_t =
                    segment_parameter(second_start, second_end, intersection.x, intersection.z);
                if !unit_interval_contains(first_t) || !unit_interval_contains(second_t) {
                    continue;
                }
                let first_height =
                    interpolated_segment_height(first_start, first_end, clamp_unit(first_t));
                let second_height =
                    interpolated_segment_height(second_start, second_end, clamp_unit(second_t));
                let first_source = original_sources
                    .get(&first_edge)
                    .map(|source| source.boundary_source);
                let second_source = original_sources
                    .get(&second_edge)
                    .map(|source| source.boundary_source);
                let Some(intersection_height) = shared_road_constraint_height_for_sources(
                    first_height,
                    second_height,
                    first_source,
                    second_source,
                ) else {
                    invalid_constraint_edges += 1;
                    continue;
                };
                let site_owned_only = first_source
                    .is_some_and(terrain_cdt_boundary_source_is_building_site)
                    && second_source.is_some_and(terrain_cdt_boundary_source_is_building_site);
                let Some(vertex_index) = insert_road_constraint_vertex(
                    TerrainCdtVertex::new(intersection.x, intersection_height, intersection.z),
                    site_owned_only,
                    vertices,
                    vertex_lookup,
                    &mut constraint_vertex_site_owned_only,
                ) else {
                    invalid_constraint_edges += 1;
                    continue;
                };
                split_points[first_index].push(TerrainCdtRoadConstraintSplit {
                    t: clamp_unit(first_t),
                    vertex_index,
                });
                split_points[second_index].push(TerrainCdtRoadConstraintSplit {
                    t: clamp_unit(second_t),
                    vertex_index,
                });
            }
        }
    }

    split_road_constraints_at_source_samples(
        &original_edges,
        vertices,
        source_sample_vertex_indices,
        &mut split_points,
    );

    let mut noded_edges = BTreeSet::new();
    road_constraint_sources.clear();

    for (edge, splits) in original_edges.iter().copied().zip(split_points.iter_mut()) {
        sort_dedup_constraint_splits(splits);
        let source = original_sources.get(&edge).copied();
        for pair in splits.windows(2) {
            let noded_edge = normalize_edge_array(pair[0].vertex_index, pair[1].vertex_index);
            if noded_edge[0] == noded_edge[1]
                || edge_lies_on_patch_boundary(
                    vertices[noded_edge[0]],
                    vertices[noded_edge[1]],
                    patch,
                )
            {
                continue;
            }
            noded_edges.insert(noded_edge);
            if let Some(source) = source {
                road_constraint_sources
                    .entry(noded_edge)
                    .and_modify(|existing| {
                        if terrain_cdt_road_constraint_source_cmp(source, *existing).is_lt() {
                            *existing = source;
                        }
                    })
                    .or_insert(source);
            }
        }
    }

    *road_constraint_edges = noded_edges.into_iter().collect();
    invalid_constraint_edges
}

fn terrain_cdt_constraint_vertex_site_ownership(
    edges: &[[usize; 2]],
    sources: &BTreeMap<[usize; 2], TerrainCdtRoadConstraintSource>,
) -> BTreeMap<usize, bool> {
    let mut site_owned_only_by_vertex = BTreeMap::new();
    for edge in edges {
        let site_owned_edge = sources
            .get(edge)
            .map(|source| source.boundary_source)
            .is_some_and(terrain_cdt_boundary_source_is_building_site);
        for vertex_index in edge {
            site_owned_only_by_vertex
                .entry(*vertex_index)
                .and_modify(|site_owned_only| *site_owned_only &= site_owned_edge)
                .or_insert(site_owned_edge);
        }
    }
    site_owned_only_by_vertex
}

pub(in crate::simulation::terrain::cdt) fn retain_exposed_road_constraint_edges(
    vertices: &[TerrainCdtVertex],
    road_loops: &[CanonicalTerrainCdtRoadLoop],
    road_constraint_edges: &mut Vec<[usize; 2]>,
    road_constraint_sources: &mut BTreeMap<[usize; 2], TerrainCdtRoadConstraintSource>,
) -> usize {
    let mut internal_edges = Vec::new();
    road_constraint_edges.retain(|edge| {
        let exposed =
            road_constraint_edge_exposes_terrain(vertices[edge[0]], vertices[edge[1]], road_loops);
        if !exposed {
            internal_edges.push(*edge);
        }
        exposed
    });
    for edge in &internal_edges {
        road_constraint_sources.remove(edge);
    }
    internal_edges.len()
}

fn road_constraint_edge_exposes_terrain(
    start: TerrainCdtVertex,
    end: TerrainCdtVertex,
    road_loops: &[CanonicalTerrainCdtRoadLoop],
) -> bool {
    let dx = end.x - start.x;
    let dz = end.z - start.z;
    let length = dx.hypot(dz);
    if length <= CDT_EPSILON_M {
        return false;
    }
    let mid_x = (start.x + end.x) * 0.5;
    let mid_z = (start.z + end.z) * 0.5;
    let probe_distance = MIN_SOURCE_OWNED_SEAM_EDGE_LENGTH_M.min(length * 0.25);
    let nx = -dz / length * probe_distance;
    let nz = dx / length * probe_distance;
    let height_m = (start.height_m + end.height_m) * 0.5;
    let left_probe = TerrainCdtVertex::new(mid_x + nx, height_m, mid_z + nz);
    let right_probe = TerrainCdtVertex::new(mid_x - nx, height_m, mid_z - nz);
    road_exterior_support_point(left_probe, road_loops)
        || road_exterior_support_point(right_probe, road_loops)
}

fn split_road_constraints_at_source_samples(
    original_edges: &[[usize; 2]],
    vertices: &mut [TerrainCdtVertex],
    source_sample_vertex_indices: &[usize],
    split_points: &mut [Vec<TerrainCdtRoadConstraintSplit>],
) {
    for &vertex_index in source_sample_vertex_indices {
        let Some(vertex) = vertices.get(vertex_index).copied() else {
            continue;
        };
        let mut hits = Vec::new();
        for (edge_index, edge) in original_edges.iter().copied().enumerate() {
            if vertex_index == edge[0] || vertex_index == edge[1] {
                continue;
            }
            let start = vertices[edge[0]];
            let end = vertices[edge[1]];
            if !point_bounds_overlap_segment(vertex, start, end) {
                continue;
            }
            let Some(t) = source_sample_parameter_on_road_constraint(start, end, vertex) else {
                continue;
            };
            hits.push(TerrainCdtSourceSampleConstraintHit {
                edge_index,
                t,
                height_m: interpolated_segment_height(start, end, t),
            });
        }

        if hits.is_empty() {
            continue;
        }
        let height_m = hits[0].height_m;
        if !hits.iter().all(|hit| same_height(hit.height_m, height_m)) {
            continue;
        }
        vertices[vertex_index].height_m = height_m;
        for hit in hits {
            split_points[hit.edge_index].push(TerrainCdtRoadConstraintSplit {
                t: hit.t,
                vertex_index,
            });
        }
    }
}

pub(in crate::simulation::terrain::cdt) fn source_sample_parameter_on_road_constraint(
    start: TerrainCdtVertex,
    end: TerrainCdtVertex,
    sample: TerrainCdtVertex,
) -> Option<f64> {
    if !point_bounds_overlap_segment(sample, start, end) {
        return None;
    }
    if same_xz(sample, start) {
        return Some(0.0);
    }
    if same_xz(sample, end) {
        return Some(1.0);
    }
    let t = segment_parameter(start, end, sample.x, sample.z);
    if !unit_interval_contains(t) {
        return None;
    }
    let t = clamp_unit(t);
    let closest = interpolate_vertex(start, end, t);
    same_xz(closest, sample).then_some(t)
}

pub(in crate::simulation::terrain::cdt) fn point_bounds_overlap_segment(
    point: TerrainCdtVertex,
    start: TerrainCdtVertex,
    end: TerrainCdtVertex,
) -> bool {
    point.x >= start.x.min(end.x) - CDT_EPSILON_M
        && point.x <= start.x.max(end.x) + CDT_EPSILON_M
        && point.z >= start.z.min(end.z) - CDT_EPSILON_M
        && point.z <= start.z.max(end.z) + CDT_EPSILON_M
}

fn insert_road_constraint_vertex(
    vertex: TerrainCdtVertex,
    site_owned_only: bool,
    vertices: &mut Vec<TerrainCdtVertex>,
    vertex_lookup: &mut HashMap<(i64, i64), usize>,
    constraint_vertex_site_owned_only: &mut BTreeMap<usize, bool>,
) -> Option<usize> {
    let key = terrain_cdt_vertex_xz_key(vertex);
    if let Some(index) = vertex_lookup.get(&key) {
        let existing_site_owned_only = constraint_vertex_site_owned_only.get(index).copied();
        let Some(existing_site_owned_only) = existing_site_owned_only else {
            vertices[*index].height_m = vertex.height_m;
            constraint_vertex_site_owned_only.insert(*index, site_owned_only);
            return Some(*index);
        };
        let merged = merge_road_vertex_height(
            TerrainCdtRoadVertexHeight {
                height_m: vertices[*index].height_m,
                site_owned_only: existing_site_owned_only,
            },
            TerrainCdtRoadVertexHeight {
                height_m: vertex.height_m,
                site_owned_only,
            },
        )?;
        vertices[*index].height_m = merged.height_m;
        constraint_vertex_site_owned_only.insert(*index, merged.site_owned_only);
        return Some(*index);
    }
    let index = vertices.len();
    vertices.push(vertex);
    vertex_lookup.insert(key, index);
    constraint_vertex_site_owned_only.insert(index, site_owned_only);
    Some(index)
}

fn shared_road_constraint_height_for_sources(
    first_height_m: f32,
    second_height_m: f32,
    first_source: Option<TerrainCdtRoadBoundarySource>,
    second_source: Option<TerrainCdtRoadBoundarySource>,
) -> Option<f32> {
    if let Some(height_m) = shared_road_constraint_height(first_height_m, second_height_m) {
        return Some(height_m);
    }
    match (
        first_source.is_some_and(terrain_cdt_boundary_source_is_building_site),
        second_source.is_some_and(terrain_cdt_boundary_source_is_building_site),
    ) {
        (true, false) => Some(second_height_m),
        (false, true) => Some(first_height_m),
        _ => None,
    }
}

fn terrain_cdt_boundary_source_is_building_site(source: TerrainCdtRoadBoundarySource) -> bool {
    matches!(
        source,
        TerrainCdtRoadBoundarySource::BuildingSiteBoundary { .. }
    )
}

fn sort_dedup_constraint_splits(splits: &mut Vec<TerrainCdtRoadConstraintSplit>) {
    splits.sort_by(|a, b| {
        a.t.total_cmp(&b.t)
            .then_with(|| a.vertex_index.cmp(&b.vertex_index))
    });
    let mut deduped = Vec::with_capacity(splits.len());
    for split in splits.iter().copied() {
        if let Some(last) = deduped.last_mut() {
            let last: &mut TerrainCdtRoadConstraintSplit = last;
            if (split.t - last.t).abs() <= CDT_EPSILON_M || split.vertex_index == last.vertex_index
            {
                if split.vertex_index < last.vertex_index {
                    *last = split;
                }
                continue;
            }
        }
        deduped.push(split);
    }
    *splits = deduped;
}

pub(in crate::simulation::terrain::cdt) fn segment_intersections(
    first_start: TerrainCdtVertex,
    first_end: TerrainCdtVertex,
    second_start: TerrainCdtVertex,
    second_end: TerrainCdtVertex,
) -> [Option<TerrainCdtVertex>; 2] {
    if !segment_bounds_overlap(first_start, first_end, second_start, second_end) {
        return [None, None];
    }
    let first_dx = first_end.x - first_start.x;
    let first_dz = first_end.z - first_start.z;
    let second_dx = second_end.x - second_start.x;
    let second_dz = second_end.z - second_start.z;
    let first_len_sq = first_dx * first_dx + first_dz * first_dz;
    let second_len_sq = second_dx * second_dx + second_dz * second_dz;
    if first_len_sq <= CDT_EPSILON_M * CDT_EPSILON_M
        || second_len_sq <= CDT_EPSILON_M * CDT_EPSILON_M
    {
        return [None, None];
    }

    let cross = cross_xz(first_dx, first_dz, second_dx, second_dz);
    let start_delta_x = second_start.x - first_start.x;
    let start_delta_z = second_start.z - first_start.z;
    if cross.abs() > CDT_EPSILON_M * first_len_sq.sqrt().max(second_len_sq.sqrt()) {
        let first_t = cross_xz(start_delta_x, start_delta_z, second_dx, second_dz) / cross;
        let second_t = cross_xz(start_delta_x, start_delta_z, first_dx, first_dz) / cross;
        if unit_interval_contains(first_t) && unit_interval_contains(second_t) {
            return [
                Some(TerrainCdtVertex::new(
                    first_start.x + first_dx * clamp_unit(first_t),
                    0.0,
                    first_start.z + first_dz * clamp_unit(first_t),
                )),
                None,
            ];
        }
        return [None, None];
    }

    if cross_xz(start_delta_x, start_delta_z, first_dx, first_dz).abs()
        > CDT_EPSILON_M * first_len_sq.sqrt()
    {
        return [None, None];
    }

    let first_t0 = segment_parameter(first_start, first_end, second_start.x, second_start.z);
    let first_t1 = segment_parameter(first_start, first_end, second_end.x, second_end.z);
    let overlap_start = first_t0.min(first_t1).max(0.0);
    let overlap_end = first_t0.max(first_t1).min(1.0);
    if overlap_start > overlap_end + CDT_EPSILON_M {
        return [None, None];
    }

    let first = TerrainCdtVertex::new(
        first_start.x + first_dx * clamp_unit(overlap_start),
        0.0,
        first_start.z + first_dz * clamp_unit(overlap_start),
    );
    let second = ((overlap_end - overlap_start).abs() > CDT_EPSILON_M).then(|| {
        TerrainCdtVertex::new(
            first_start.x + first_dx * clamp_unit(overlap_end),
            0.0,
            first_start.z + first_dz * clamp_unit(overlap_end),
        )
    });
    [Some(first), second]
}

pub(in crate::simulation::terrain::cdt) fn segment_bounds_overlap(
    first_start: TerrainCdtVertex,
    first_end: TerrainCdtVertex,
    second_start: TerrainCdtVertex,
    second_end: TerrainCdtVertex,
) -> bool {
    first_start.x.min(first_end.x) <= second_start.x.max(second_end.x) + CDT_EPSILON_M
        && second_start.x.min(second_end.x) <= first_start.x.max(first_end.x) + CDT_EPSILON_M
        && first_start.z.min(first_end.z) <= second_start.z.max(second_end.z) + CDT_EPSILON_M
        && second_start.z.min(second_end.z) <= first_start.z.max(first_end.z) + CDT_EPSILON_M
}

pub(in crate::simulation::terrain::cdt) fn segment_parameter(
    start: TerrainCdtVertex,
    end: TerrainCdtVertex,
    x: f64,
    z: f64,
) -> f64 {
    let dx = end.x - start.x;
    let dz = end.z - start.z;
    let length_squared = dx * dx + dz * dz;
    if length_squared <= CDT_EPSILON_M * CDT_EPSILON_M {
        return 0.0;
    }
    ((x - start.x) * dx + (z - start.z) * dz) / length_squared
}

pub(in crate::simulation::terrain::cdt) fn interpolated_segment_height(
    start: TerrainCdtVertex,
    end: TerrainCdtVertex,
    t: f64,
) -> f32 {
    (f64::from(start.height_m) + f64::from(end.height_m - start.height_m) * t) as f32
}

pub(in crate::simulation::terrain::cdt) fn unit_interval_contains(value: f64) -> bool {
    (-CDT_EPSILON_M..=1.0 + CDT_EPSILON_M).contains(&value)
}

pub(in crate::simulation::terrain::cdt) fn clamp_unit(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

pub(in crate::simulation::terrain::cdt) fn cross_xz(ax: f64, az: f64, bx: f64, bz: f64) -> f64 {
    ax * bz - az * bx
}

pub(in crate::simulation::terrain::cdt) fn edge_lies_on_patch_boundary(
    a: TerrainCdtVertex,
    b: TerrainCdtVertex,
    patch: TerrainCdtPatch,
) -> bool {
    (same_coord(a.x, patch.min_x) && same_coord(b.x, patch.min_x))
        || (same_coord(a.x, patch.max_x) && same_coord(b.x, patch.max_x))
        || (same_coord(a.z, patch.min_z) && same_coord(b.z, patch.min_z))
        || (same_coord(a.z, patch.max_z) && same_coord(b.z, patch.max_z))
}

pub(in crate::simulation::terrain::cdt) fn edge_length_xz_m(
    a: TerrainCdtVertex,
    b: TerrainCdtVertex,
) -> f64 {
    let dx = b.x - a.x;
    let dz = b.z - a.z;
    (dx * dx + dz * dz).sqrt()
}
