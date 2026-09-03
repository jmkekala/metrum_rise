//! Road-seam source recovery, quality budgets, and deterministic simplification.

use super::*;

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
        if points.len() < 3 || edge_sources.len() != points.len() {
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

pub(super) fn mergeable_terrain_cdt_seam_source(
    first: TerrainCdtRoadBoundarySource,
    second: TerrainCdtRoadBoundarySource,
) -> Option<TerrainCdtRoadBoundarySource> {
    match (first, second) {
        (
            TerrainCdtRoadBoundarySource::SpanSupportBoundary {
                edge_idx: edge_idx_a,
                edge_class: edge_class_a,
                support_policy: support_policy_a,
                source_band_index: source_band_index_a,
                band_kind: band_kind_a,
                role: role_a,
                start_section_index: start_section_index_a,
                end_section_index: end_section_index_a,
                start_s_m: start_s_m_a,
                end_s_m: end_s_m_a,
            },
            TerrainCdtRoadBoundarySource::SpanSupportBoundary {
                edge_idx: edge_idx_b,
                edge_class: edge_class_b,
                support_policy: support_policy_b,
                source_band_index: source_band_index_b,
                band_kind: band_kind_b,
                role: role_b,
                start_section_index: start_section_index_b,
                end_section_index: end_section_index_b,
                start_s_m: start_s_m_b,
                end_s_m: end_s_m_b,
            },
        ) if edge_idx_a == edge_idx_b
            && edge_class_a == edge_class_b
            && support_policy_a == support_policy_b
            && source_band_index_a == source_band_index_b
            && band_kind_a == band_kind_b
            && role_a == role_b =>
        {
            Some(TerrainCdtRoadBoundarySource::SpanSupportBoundary {
                edge_idx: edge_idx_a,
                edge_class: edge_class_a,
                support_policy: support_policy_a,
                source_band_index: source_band_index_a,
                band_kind: band_kind_a,
                role: role_a,
                start_section_index: start_section_index_a.min(start_section_index_b),
                end_section_index: end_section_index_a.max(end_section_index_b),
                start_s_m: start_s_m_a.min(start_s_m_b),
                end_s_m: end_s_m_a.max(end_s_m_b),
            })
        }
        (
            TerrainCdtRoadBoundarySource::NodeFootprintBoundary {
                node_id: node_id_a,
                node_kind: node_kind_a,
                owner_kind: owner_kind_a,
                owner_index: owner_index_a,
                boundary_source: boundary_source_a,
            },
            TerrainCdtRoadBoundarySource::NodeFootprintBoundary {
                node_id: node_id_b,
                node_kind: node_kind_b,
                owner_kind: owner_kind_b,
                owner_index: owner_index_b,
                boundary_source: boundary_source_b,
            },
        ) if node_id_a == node_id_b
            && node_kind_a == node_kind_b
            && owner_kind_a == owner_kind_b
            && owner_index_a == owner_index_b
            && boundary_source_a == boundary_source_b =>
        {
            Some(first)
        }
        (
            TerrainCdtRoadBoundarySource::NodeSameMaterialBoundaryHandoff {
                node_id: node_id_a,
                node_kind: node_kind_a,
                owner_kind: owner_kind_a,
                owner_index_a: owner_index_a_a,
                owner_index_b: owner_index_b_a,
                boundary_source: boundary_source_a,
            },
            TerrainCdtRoadBoundarySource::NodeSameMaterialBoundaryHandoff {
                node_id: node_id_b,
                node_kind: node_kind_b,
                owner_kind: owner_kind_b,
                owner_index_a: owner_index_a_b,
                owner_index_b: owner_index_b_b,
                boundary_source: boundary_source_b,
            },
        ) if node_id_a == node_id_b
            && node_kind_a == node_kind_b
            && owner_kind_a == owner_kind_b
            && owner_index_a_a == owner_index_a_b
            && owner_index_b_a == owner_index_b_b
            && boundary_source_a == boundary_source_b =>
        {
            Some(first)
        }
        (
            TerrainCdtRoadBoundarySource::BuildingSiteBoundary {
                building_idx: building_idx_a,
                local_loop_index: local_loop_index_a,
                local_edge_index: local_edge_index_a,
            },
            TerrainCdtRoadBoundarySource::BuildingSiteBoundary {
                building_idx: building_idx_b,
                local_loop_index: local_loop_index_b,
                local_edge_index: local_edge_index_b,
            },
        ) if building_idx_a == building_idx_b && local_loop_index_a == local_loop_index_b => {
            Some(TerrainCdtRoadBoundarySource::BuildingSiteBoundary {
                building_idx: building_idx_a,
                local_loop_index: local_loop_index_a,
                local_edge_index: local_edge_index_a.min(local_edge_index_b),
            })
        }
        (
            TerrainCdtRoadBoundarySource::SyntheticTestBoundary {
                stable_piece_id: stable_piece_id_a,
                local_loop_index: local_loop_index_a,
                local_edge_index: local_edge_index_a,
            },
            TerrainCdtRoadBoundarySource::SyntheticTestBoundary {
                stable_piece_id: stable_piece_id_b,
                local_loop_index: local_loop_index_b,
                local_edge_index: local_edge_index_b,
            },
        ) if stable_piece_id_a == stable_piece_id_b && local_loop_index_a == local_loop_index_b => {
            Some(TerrainCdtRoadBoundarySource::SyntheticTestBoundary {
                stable_piece_id: stable_piece_id_a,
                local_loop_index: local_loop_index_a,
                local_edge_index: local_edge_index_a.min(local_edge_index_b),
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
    let mut source_edges = road_loop.source_edges.clone();
    if source_edges.is_empty() && !road_loop.vertices.is_empty() {
        source_edges = road_loop
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
            .collect();
    }
    source_edges
        .into_iter()
        .filter(|edge| !same_xz(edge.start, edge.end))
        .collect()
}

fn terrain_cdt_loop_edge_sources(
    points: &[TerrainCdtVertex],
    source_edges: &[TerrainCdtRoadLoopSourceEdge],
) -> Vec<Option<TerrainCdtRoadBoundarySource>> {
    if points.is_empty() {
        return Vec::new();
    }
    (0..points.len())
        .map(|index| {
            terrain_cdt_loop_segment_source(
                points[index],
                points[(index + 1) % points.len()],
                source_edges,
            )
        })
        .collect()
}

fn terrain_cdt_loop_segment_source(
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
    let Some(start_t) =
        source_sample_parameter_on_road_constraint(source_edge.start, source_edge.end, start)
    else {
        return false;
    };
    let Some(end_t) =
        source_sample_parameter_on_road_constraint(source_edge.start, source_edge.end, end)
    else {
        return false;
    };
    let source_length_m = edge_length_xz_m(source_edge.start, source_edge.end);
    (start_t - end_t).abs() * source_length_m > CDT_EPSILON_M
}

fn merge_terrain_cdt_boundary_source(
    target: &mut Option<TerrainCdtRoadBoundarySource>,
    candidate: TerrainCdtRoadBoundarySource,
) {
    if target.is_none_or(|current| terrain_cdt_boundary_source_cmp(candidate, current).is_lt()) {
        *target = Some(candidate);
    }
}
