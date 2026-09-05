// SPDX-License-Identifier: GPL-2.0-only

//! Deterministic constrained triangulation for road-touched terrain patches.
//!
//! The public data contract is kept in [`model`]. The remaining modules form a one-way
//! pipeline from canonical input through constrained triangulation, face ownership, and
//! diagnostics. No stage depends on Godot types.

#![cfg_attr(not(test), allow(dead_code))]

mod builder;
mod canonicalize;
mod constraints;
mod diagnostics;
mod face_classification;
mod loop_clip;
mod model;
mod seam_quality;

pub(crate) use builder::build_road_touched_terrain_patch;
pub(crate) use model::*;

use canonicalize::*;
use constraints::*;
use diagnostics::*;
use face_classification::*;
use loop_clip::*;
use seam_quality::*;

/// Clips one authoritative road loop and its source-owned edge provenance to a rectangle.
///
/// A concave loop can intersect the rectangle as multiple disconnected components.
pub(crate) fn clip_terrain_cdt_road_loop_to_patch(
    road_loop: &TerrainCdtRoadLoop,
    patch: TerrainCdtPatch,
) -> Vec<TerrainCdtRoadLoop> {
    let clipped_source_edges = normalized_road_loop_source_edges(road_loop)
        .into_iter()
        .filter_map(|edge| {
            clip_segment_to_patch(edge.start, edge.end, patch).map(|(start, end)| {
                TerrainCdtRoadLoopSourceEdge {
                    start,
                    end,
                    source: edge.source,
                }
            })
        })
        .collect::<Vec<_>>();
    clip_loop_to_patch_components(&road_loop.vertices, patch)
        .into_iter()
        .map(|vertices| {
            let source_edges = clipped_source_edges
                .iter()
                .copied()
                .filter(|edge| component_contains_source_edge(&vertices, *edge))
                .collect();
            TerrainCdtRoadLoop::new_with_source_edges_and_topology(
                road_loop.stable_piece_id,
                road_loop.footprint_group_id,
                road_loop.local_loop_index,
                road_loop.is_hole,
                vertices,
                source_edges,
            )
        })
        .collect()
}

fn component_contains_source_edge(
    component: &[TerrainCdtVertex],
    source_edge: TerrainCdtRoadLoopSourceEdge,
) -> bool {
    (0..component.len()).any(|index| {
        let start = component[index];
        let end = component[(index + 1) % component.len()];
        segments_have_metric_collinear_overlap(start, end, source_edge.start, source_edge.end)
    })
}

fn segments_have_metric_collinear_overlap(
    first_start: TerrainCdtVertex,
    first_end: TerrainCdtVertex,
    second_start: TerrainCdtVertex,
    second_end: TerrainCdtVertex,
) -> bool {
    let first_dx = first_end.x - first_start.x;
    let first_dz = first_end.z - first_start.z;
    let second_dx = second_end.x - second_start.x;
    let second_dz = second_end.z - second_start.z;
    let first_length_m = first_dx.hypot(first_dz);
    let second_length_m = second_dx.hypot(second_dz);
    if first_length_m <= CDT_EPSILON_M || second_length_m <= CDT_EPSILON_M {
        return false;
    }

    if cross_xz(first_dx, first_dz, second_dx, second_dz).abs()
        > CDT_EPSILON_M * first_length_m.max(second_length_m)
    {
        return false;
    }
    let start_delta_x = second_start.x - first_start.x;
    let start_delta_z = second_start.z - first_start.z;
    if cross_xz(start_delta_x, start_delta_z, first_dx, first_dz).abs()
        > CDT_EPSILON_M * first_length_m
    {
        return false;
    }

    let first_t0 = segment_parameter(first_start, first_end, second_start.x, second_start.z);
    let first_t1 = segment_parameter(first_start, first_end, second_end.x, second_end.z);
    let overlap_start = first_t0.min(first_t1).max(0.0);
    let overlap_end = first_t0.max(first_t1).min(1.0);
    (overlap_end - overlap_start).max(0.0) * first_length_m > CDT_EPSILON_M
}

/// Clips one terrain-CDT guide segment to a rectangular local patch.
pub(crate) fn clip_terrain_cdt_segment_to_patch(
    start: TerrainCdtVertex,
    end: TerrainCdtVertex,
    patch: TerrainCdtPatch,
) -> Option<(TerrainCdtVertex, TerrainCdtVertex)> {
    clip_segment_to_patch(start, end, patch)
}

#[cfg(test)]
mod tests;
