// SPDX-License-Identifier: GPL-2.0-only

//! Road-seam source recovery, quality budgets, and deterministic simplification.

use super::*;
use rstar::primitives::{GeomWithData, Line};
use rstar::{AABB, RTree};

#[derive(Clone, Debug, PartialEq)]
pub(super) struct TerrainCdtLoopSeamQuality {
    pub(super) points: Vec<TerrainCdtVertex>,
    pub(super) edge_sources: Vec<Option<TerrainCdtRoadBoundarySource>>,
    pub(super) accepted_seam_edges: usize,
    pub(super) merged_subbudget_seam_edges: usize,
    pub(super) retaining_wall_required_seam_edges: usize,
    pub(super) blocking_degenerate_seam_edges: usize,
    pub(super) samples: Vec<TerrainCdtSeamQualitySample>,
}

pub(super) fn harden_terrain_cdt_road_loop_seams(
    mut points: Vec<TerrainCdtVertex>,
    source_edges: &[TerrainCdtRoadLoopSourceEdge],
    patch: TerrainCdtPatch,
) -> TerrainCdtLoopSeamQuality {
    let mut edge_sources = terrain_cdt_loop_edge_sources(&points, source_edges);
    let mut merged_subbudget_seam_edges = 0usize;
    let mut samples = Vec::new();

    loop {
        // Three vertices are the minimum valid loop. The removal helper deliberately keeps that
        // minimum, so searching for another eligible merge here would retry the same no-op forever.
        if points.len() <= 3 || edge_sources.len() != points.len() {
            break;
        }
        let Some(merge) = next_mergeable_subbudget_seam_vertex(&points, &edge_sources) else {
            break;
        };
        let start = points[merge.previous_index];
        let end = points[merge.next_index];
        let length_m = edge_length_xz_m(start, end) as f32;
        let height_delta_m = (end.height_m - start.height_m).abs();
        insert_seam_quality_sample(
            &mut samples,
            TerrainCdtSeamQualitySample {
                kind: TerrainCdtSeamQualityKind::MergedSubbudgetSeamEdge,
                start,
                end,
                source: merge.source,
                length_m,
                height_delta_m,
            },
        );
        remove_loop_vertex_and_merge_source(
            &mut points,
            &mut edge_sources,
            merge.vertex_index,
            merge.source,
        );
        merged_subbudget_seam_edges += 1;
    }

    let mut accepted_seam_edges = 0usize;
    let mut retaining_wall_required_seam_edges = 0usize;
    let mut blocking_degenerate_seam_edges = 0usize;
    for index in 0..points.len() {
        let Some(source) = edge_sources.get(index).copied().flatten() else {
            continue;
        };
        let start = points[index];
        let end = points[(index + 1) % points.len()];
        if edge_lies_on_patch_boundary(start, end, patch) {
            continue;
        }
        let length_m = edge_length_xz_m(start, end);
        let height_delta_m = (end.height_m - start.height_m).abs();
        if length_m + CDT_EPSILON_M < MIN_SOURCE_OWNED_SEAM_EDGE_LENGTH_M {
            if terrain_cdt_boundary_source_allows_retaining_wall(source)
                && height_delta_m > MIN_TIE_IN_HEIGHT_DELTA_M
                && (length_m <= CDT_EPSILON_M
                    || height_delta_m / length_m as f32 > MAX_TERRAIN_TIE_IN_SLOPE_RATIO)
            {
                retaining_wall_required_seam_edges += 1;
                insert_seam_quality_sample(
                    &mut samples,
                    TerrainCdtSeamQualitySample {
                        kind: TerrainCdtSeamQualityKind::RetainingWallRequired,
                        start,
                        end,
                        source,
                        length_m: length_m as f32,
                        height_delta_m,
                    },
                );
            } else if length_m <= CDT_EPSILON_M {
                blocking_degenerate_seam_edges += 1;
                insert_seam_quality_sample(
                    &mut samples,
                    TerrainCdtSeamQualitySample {
                        kind: TerrainCdtSeamQualityKind::BlockingDegenerateSeam,
                        start,
                        end,
                        source,
                        length_m: length_m as f32,
                        height_delta_m,
                    },
                );
            } else {
                accepted_seam_edges += 1;
            }
        } else {
            accepted_seam_edges += 1;
        }
    }

    TerrainCdtLoopSeamQuality {
        points,
        edge_sources,
        accepted_seam_edges,
        merged_subbudget_seam_edges,
        retaining_wall_required_seam_edges,
        blocking_degenerate_seam_edges,
        samples,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TerrainCdtSeamMerge {
    previous_index: usize,
    vertex_index: usize,
    next_index: usize,
    source: TerrainCdtRoadBoundarySource,
}

fn next_mergeable_subbudget_seam_vertex(
    points: &[TerrainCdtVertex],
    edge_sources: &[Option<TerrainCdtRoadBoundarySource>],
) -> Option<TerrainCdtSeamMerge> {
    for vertex_index in 0..points.len() {
        let previous_index = if vertex_index == 0 {
            points.len() - 1
        } else {
            vertex_index - 1
        };
        let next_index = (vertex_index + 1) % points.len();
        let previous_len_m = edge_length_xz_m(points[previous_index], points[vertex_index]);
        let next_len_m = edge_length_xz_m(points[vertex_index], points[next_index]);
        if previous_len_m >= MIN_SOURCE_OWNED_SEAM_EDGE_LENGTH_M
            && next_len_m >= MIN_SOURCE_OWNED_SEAM_EDGE_LENGTH_M
        {
            continue;
        }
        let Some(previous_source) = edge_sources.get(previous_index).copied().flatten() else {
            continue;
        };
        let Some(next_source) = edge_sources.get(vertex_index).copied().flatten() else {
            continue;
        };
        let Some(source) = mergeable_terrain_cdt_seam_source(previous_source, next_source) else {
            continue;
        };
        if !source_owned_vertex_can_be_removed(
            points[previous_index],
            points[vertex_index],
            points[next_index],
        ) {
            continue;
        }
        return Some(TerrainCdtSeamMerge {
            previous_index,
            vertex_index,
            next_index,
            source,
        });
    }
    None
}

fn remove_loop_vertex_and_merge_source(
    points: &mut Vec<TerrainCdtVertex>,
    edge_sources: &mut Vec<Option<TerrainCdtRoadBoundarySource>>,
    vertex_index: usize,
    source: TerrainCdtRoadBoundarySource,
) {
    if points.len() <= 3 || points.len() != edge_sources.len() {
        return;
    }
    points.remove(vertex_index);
    if vertex_index == 0 {
        let last = edge_sources.len() - 1;
        edge_sources[last] = Some(source);
        edge_sources.remove(0);
    } else {
        edge_sources[vertex_index - 1] = Some(source);
        edge_sources.remove(vertex_index);
    }
}

fn source_owned_vertex_can_be_removed(
    previous: TerrainCdtVertex,
    vertex: TerrainCdtVertex,
    next: TerrainCdtVertex,
) -> bool {
    let merged_len_m = edge_length_xz_m(previous, next);
    if merged_len_m <= CDT_EPSILON_M {
        return false;
    }
    let cross = cross_xz(
        vertex.x - previous.x,
        vertex.z - previous.z,
        next.x - previous.x,
        next.z - previous.z,
    )
    .abs();
    if cross > CDT_EPSILON_M * merged_len_m.max(1.0) {
        return false;
    }
    if !point_bounds_overlap_segment(vertex, previous, next) {
        return false;
    }
    let t = clamp_unit(segment_parameter(previous, next, vertex.x, vertex.z));
    same_height(
        interpolated_segment_height(previous, next, t),
        vertex.height_m,
    )
}

// Two consecutive seam edges merge when they come from one road chain or one site loop; the
// merged edge keeps the lower segment index.
pub(super) fn mergeable_terrain_cdt_seam_source(
    first: TerrainCdtRoadBoundarySource,
    second: TerrainCdtRoadBoundarySource,
) -> Option<TerrainCdtRoadBoundarySource> {
    use TerrainCdtRoadBoundarySource as Source;
    match (first, second) {
        (
            Source::RoadBoundary {
                piece,
                chain,
                segment: segment_a,
                support,
            },
            Source::RoadBoundary {
                piece: piece_b,
                chain: chain_b,
                segment: segment_b,
                support: support_b,
            },
        ) if piece == piece_b && chain == chain_b && support == support_b => {
            Some(Source::RoadBoundary {
                piece,
                chain,
                segment: segment_a.min(segment_b),
                support,
            })
        }
        (
            Source::BuildingSiteBoundary {
                building_idx,
                local_loop_index,
                local_edge_index: edge_a,
            },
            Source::BuildingSiteBoundary {
                building_idx: building_b,
                local_loop_index: loop_b,
                local_edge_index: edge_b,
            },
        ) if building_idx == building_b && local_loop_index == loop_b => {
            Some(Source::BuildingSiteBoundary {
                building_idx,
                local_loop_index,
                local_edge_index: edge_a.min(edge_b),
            })
        }
        #[cfg(test)]
        (
            Source::SyntheticTestBoundary {
                stable_piece_id,
                local_loop_index,
                local_edge_index: edge_a,
            },
            Source::SyntheticTestBoundary {
                stable_piece_id: piece_b,
                local_loop_index: loop_b,
                local_edge_index: edge_b,
            },
        ) if stable_piece_id == piece_b && local_loop_index == loop_b => {
            Some(Source::SyntheticTestBoundary {
                stable_piece_id,
                local_loop_index,
                local_edge_index: edge_a.min(edge_b),
            })
        }
        _ => None,
    }
}

fn insert_seam_quality_sample(
    samples: &mut Vec<TerrainCdtSeamQualitySample>,
    sample: TerrainCdtSeamQualitySample,
) {
    if samples.len() >= MAX_SEAM_QUALITY_SAMPLES {
        return;
    }
    samples.push(sample);
}

pub(super) fn append_seam_quality_samples(
    target: &mut Vec<TerrainCdtSeamQualitySample>,
    samples: Vec<TerrainCdtSeamQualitySample>,
) {
    for sample in samples {
        insert_seam_quality_sample(target, sample);
    }
}

pub(super) fn normalized_road_loop_source_edges(
    road_loop: &TerrainCdtRoadLoop,
) -> Vec<TerrainCdtRoadLoopSourceEdge> {
    let source_edges = road_loop.source_edges.clone();
    // Tests may pass bare loops; production loops always carry their sources.
    #[cfg(test)]
    let source_edges = if source_edges.is_empty() && !road_loop.vertices.is_empty() {
        road_loop
            .vertices
            .iter()
            .copied()
            .enumerate()
            .map(|(index, start)| TerrainCdtRoadLoopSourceEdge {
                start,
                end: road_loop.vertices[(index + 1) % road_loop.vertices.len()],
                source: TerrainCdtRoadBoundarySource::SyntheticTestBoundary {
                    stable_piece_id: road_loop.stable_piece_id,
                    local_loop_index: road_loop.local_loop_index,
                    local_edge_index: u32::try_from(index).unwrap_or(u32::MAX),
                },
            })
            .collect()
    } else {
        source_edges
    };
    source_edges
        .into_iter()
        .filter(|edge| !same_xz(edge.start, edge.end))
        .collect()
}

pub(super) fn terrain_cdt_loop_edge_sources(
    points: &[TerrainCdtVertex],
    source_edges: &[TerrainCdtRoadLoopSourceEdge],
) -> Vec<Option<TerrainCdtRoadBoundarySource>> {
    terrain_cdt_loop_edge_sources_in_bounds(points, source_edges, None)
}

pub(super) fn terrain_cdt_loop_edge_sources_in_bounds(
    points: &[TerrainCdtVertex],
    source_edges: &[TerrainCdtRoadLoopSourceEdge],
    query_bounds: Option<TerrainCdtLoopBounds>,
) -> Vec<Option<TerrainCdtRoadBoundarySource>> {
    if points.is_empty() {
        return Vec::new();
    }
    let relevant = |start: TerrainCdtVertex, end: TerrainCdtVertex| {
        query_bounds.is_none_or(|bounds| {
            start.x.max(end.x) + CDT_EPSILON_M >= bounds.min_x
                && start.x.min(end.x) - CDT_EPSILON_M <= bounds.max_x
                && start.z.max(end.z) + CDT_EPSILON_M >= bounds.min_z
                && start.z.min(end.z) - CDT_EPSILON_M <= bounds.max_z
        })
    };
    // One bounded loop-local index, using the same R-tree library as road queries.
    // O(S log S + P log S + overlapping candidates), instead of P*S source tests.
    // Source selection is a semantic minimum, independent of tree traversal order.
    let index = RTree::bulk_load(
        source_edges
            .iter()
            .enumerate()
            .filter(|(_, edge)| relevant(edge.start, edge.end))
            .map(|(id, edge)| {
                GeomWithData::new(
                    Line::new([edge.start.x, edge.start.z], [edge.end.x, edge.end.z]),
                    id,
                )
            })
            .collect(),
    );
    (0..points.len())
        .map(|point_index| {
            let start = points[point_index];
            let end = points[(point_index + 1) % points.len()];
            if same_xz(start, end) || !relevant(start, end) {
                return None;
            }
            let bounds = AABB::from_corners(
                [
                    start.x.min(end.x) - CDT_EPSILON_M,
                    start.z.min(end.z) - CDT_EPSILON_M,
                ],
                [
                    start.x.max(end.x) + CDT_EPSILON_M,
                    start.z.max(end.z) + CDT_EPSILON_M,
                ],
            );
            let mut source = None;
            for candidate in index.locate_in_envelope_intersecting(&bounds) {
                let edge = source_edges[candidate.data];
                if terrain_cdt_segment_lies_on_source_edge(start, end, edge) {
                    merge_terrain_cdt_boundary_source(&mut source, edge.source);
                }
            }
            source
        })
        .collect()
}

#[cfg(test)]
pub(super) fn terrain_cdt_loop_segment_source(
    start: TerrainCdtVertex,
    end: TerrainCdtVertex,
    source_edges: &[TerrainCdtRoadLoopSourceEdge],
) -> Option<TerrainCdtRoadBoundarySource> {
    if same_xz(start, end) {
        return None;
    }
    let mut source = None;
    for &source_edge in source_edges {
        if !terrain_cdt_segment_lies_on_source_edge(start, end, source_edge) {
            continue;
        }
        merge_terrain_cdt_boundary_source(&mut source, source_edge.source);
    }
    source
}

fn terrain_cdt_segment_lies_on_source_edge(
    start: TerrainCdtVertex,
    end: TerrainCdtVertex,
    source_edge: TerrainCdtRoadLoopSourceEdge,
) -> bool {
    if !segment_bounds_overlap(start, end, source_edge.start, source_edge.end) {
        return false;
    }
    if source_sample_parameter_on_road_constraint(source_edge.start, source_edge.end, start)
        .is_none()
    {
        return false;
    }
    if source_sample_parameter_on_road_constraint(source_edge.start, source_edge.end, end).is_none()
    {
        return false;
    }
    // Distinct 1 mm identity cells can be separated by less than 1 mm across a rounding
    // boundary. Both endpoints lying on the source is sufficient; seam hardening owns the
    // separate decision to merge or retain such a short sourced edge.
    true
}

fn merge_terrain_cdt_boundary_source(
    target: &mut Option<TerrainCdtRoadBoundarySource>,
    candidate: TerrainCdtRoadBoundarySource,
) {
    if target.is_none_or(|current| terrain_cdt_boundary_source_cmp(candidate, current).is_lt()) {
        *target = Some(candidate);
    }
}
