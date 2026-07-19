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
        !same_xz(start, end)
            && source_sample_parameter_on_road_constraint(source_edge.start, source_edge.end, start)
                .is_some()
            && source_sample_parameter_on_road_constraint(source_edge.start, source_edge.end, end)
                .is_some()
    })
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
