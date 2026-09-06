// SPDX-License-Identifier: GPL-2.0-only

//! Road-loop constraint insertion and source-vertex splitting.

use std::collections::BTreeMap;

use super::super::*;

#[derive(Clone, Copy)]
pub(in crate::simulation::terrain::cdt) struct TerrainCdtRoadLoopConstraintIdentity {
    pub(in crate::simulation::terrain::cdt) stable_piece_id: u64,
    pub(in crate::simulation::terrain::cdt) local_loop_index: u32,
}

pub(in crate::simulation::terrain::cdt) fn push_road_loop_constraints(
    indices: &[usize],
    vertices: &[TerrainCdtVertex],
    patch: TerrainCdtPatch,
    identity: TerrainCdtRoadLoopConstraintIdentity,
    edge_sources: &[Option<TerrainCdtRoadBoundarySource>],
    road_constraint_edges: &mut Vec<[usize; 2]>,
    road_constraint_sources: &mut BTreeMap<[usize; 2], TerrainCdtRoadConstraintSource>,
) -> Result<(), TerrainCdtError> {
    for index in 0..indices.len() {
        let edge = normalize_edge_array(indices[index], indices[(index + 1) % indices.len()]);
        if edge[0] == edge[1] {
            continue;
        }
        if !edge_lies_on_patch_boundary(vertices[edge[0]], vertices[edge[1]], patch) {
            let Some(boundary_source) = edge_sources.get(index).copied().flatten() else {
                return Err(TerrainCdtError::MissingRoadBoundarySource);
            };
            road_constraint_edges.push(edge);
            road_constraint_sources
                .entry(edge)
                .or_insert(TerrainCdtRoadConstraintSource {
                    stable_piece_id: identity.stable_piece_id,
                    local_loop_index: identity.local_loop_index,
                    local_edge_index: u32::try_from(index).unwrap_or(u32::MAX),
                    boundary_source,
                });
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TerrainCdtSourceVertexSplit {
    t: f64,
    vertex: TerrainCdtVertex,
}

pub(in crate::simulation::terrain::cdt) fn road_loop_contains_source_edge_vertices(
    points: &[TerrainCdtVertex],
    source_edges: &[TerrainCdtRoadLoopSourceEdge],
) -> bool {
    if source_edges.is_empty() {
        return true;
    }
    let mut point_keys = points
        .iter()
        .copied()
        .map(terrain_cdt_vertex_key)
        .collect::<Vec<_>>();
    point_keys.sort_unstable();
    point_keys.dedup();
    source_edges.iter().all(|edge| {
        [edge.start, edge.end].into_iter().all(|vertex| {
            point_keys
                .binary_search(&terrain_cdt_vertex_key(vertex))
                .is_ok()
        })
    })
}

pub(in crate::simulation::terrain::cdt) fn split_road_loop_segments_at_source_vertices(
    points: Vec<TerrainCdtVertex>,
    source_edges: &[TerrainCdtRoadLoopSourceEdge],
) -> Result<Vec<TerrainCdtVertex>, TerrainCdtError> {
    if points.len() < 2 || source_edges.is_empty() {
        return Ok(points);
    }

    let mut split_points = Vec::with_capacity(points.len() + source_edges.len());
    for index in 0..points.len() {
        let start = points[index];
        let end = points[(index + 1) % points.len()];
        let segment_length_m = edge_length_xz_m(start, end);
        if split_points
            .last()
            .is_none_or(|last: &TerrainCdtVertex| !same_xz(*last, start))
        {
            split_points.push(start);
        }

        let mut splits = source_edges
            .iter()
            .flat_map(|edge| [edge.start, edge.end])
            .filter(|candidate| !same_xz(*candidate, start) && !same_xz(*candidate, end))
            .filter_map(|candidate| {
                source_sample_parameter_on_road_constraint(start, end, candidate).and_then(|t| {
                    (t * segment_length_m > CDT_EPSILON_M
                        && (1.0 - t) * segment_length_m > CDT_EPSILON_M)
                        .then_some(TerrainCdtSourceVertexSplit {
                            t,
                            vertex: candidate,
                        })
                })
            })
            .collect::<Vec<_>>();
        sort_dedup_source_vertex_splits(&mut splits)?;
        for split in splits {
            if let Some(last) = split_points.last()
                && same_xz(*last, split.vertex)
            {
                if !same_height(last.height_m, split.vertex.height_m) {
                    return Err(TerrainCdtError::ConflictingRoadBoundaryHeight);
                }
                continue;
            }
            split_points.push(split.vertex);
        }
    }

    simplified_road_loop(split_points)
}

fn sort_dedup_source_vertex_splits(
    splits: &mut Vec<TerrainCdtSourceVertexSplit>,
) -> Result<(), TerrainCdtError> {
    splits.sort_by(|a, b| {
        a.t.total_cmp(&b.t)
            .then_with(|| terrain_cdt_vertex_key(a.vertex).cmp(&terrain_cdt_vertex_key(b.vertex)))
    });

    let mut deduped = Vec::with_capacity(splits.len());
    for split in splits.iter().copied() {
        if let Some(last) = deduped.last_mut() {
            let last: &mut TerrainCdtSourceVertexSplit = last;
            if same_xz(split.vertex, last.vertex) {
                if !same_height(split.vertex.height_m, last.vertex.height_m) {
                    return Err(TerrainCdtError::ConflictingRoadBoundaryHeight);
                }
                continue;
            }
        }
        deduped.push(split);
    }
    *splits = deduped;
    Ok(())
}
