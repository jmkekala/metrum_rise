// SPDX-License-Identifier: GPL-2.0-only

//! Same-road parcel-run projection.

mod spacing;

use super::projection::{
    ProjectedRoadPoint, project_buildable_road_point_at, project_point_to_edge,
};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::parcels::{ParcelGeometry, ParcelPlacementError};
use godot::prelude::Vector2;
use spacing::{
    directed_s_within_limit, next_non_overlapping_run_geometry,
    next_non_overlapping_run_geometry_directed,
};

const RUN_PHASE_SEARCH_LIMIT: usize = 24;
const RUN_PHASE_EPSILON_M: f32 = 0.001;

/// Candidate layouts for one same-edge drag run.
pub(crate) struct ParcelRunProjection {
    /// Deterministic layout variants for the current drag span.
    pub(crate) layouts: Vec<Vec<ParcelGeometry>>,
    /// Direction from drag start toward drag end along the owning edge.
    pub(crate) endpoint_direction: f32,
}

pub(crate) fn project_parcel_run_layouts_at(
    graph: &RegionGraph,
    start_pos: Vector2,
    end_pos: Vector2,
    frontage_m: f32,
    depth_m: f32,
    gap_m: f32,
) -> Result<ParcelRunProjection, ParcelPlacementError> {
    let start = project_buildable_road_point_at(graph, start_pos, frontage_m, depth_m)?;
    project_parcel_run_layouts_from_projected_start(
        graph, start, end_pos, frontage_m, depth_m, gap_m,
    )
}

pub(crate) fn project_parcel_run_from_existing(
    graph: &RegionGraph,
    existing: &ParcelGeometry,
    end_pos: Vector2,
    frontage_m: f32,
    depth_m: f32,
    gap_m: f32,
) -> Result<Vec<ParcelGeometry>, ParcelPlacementError> {
    let edge_idx = existing.edge_idx;
    if edge_idx >= graph.edge_count() {
        return Err(ParcelPlacementError::NoRoadAttachment);
    }
    let edge = graph.edge(edge_idx);
    if edge.deleted
        || edge.no_building_spawn
        || edge.physical_length <= frontage_m
        || edge.physical_geometry.len() < 2
    {
        return Err(ParcelPlacementError::NoRoadAttachment);
    }
    let s_m = existing.frontage_center_t.clamp(0.0, 1.0) * edge.physical_length;
    if s_m < existing.frontage_m * 0.5 || s_m > edge.physical_length - existing.frontage_m * 0.5 {
        return Err(ParcelPlacementError::FrontageOutOfBounds);
    }
    let Some(end) = project_point_to_edge(graph, edge_idx, end_pos) else {
        return Err(ParcelPlacementError::NoRoadAttachment);
    };

    let spacing_m = frontage_m + gap_m;
    if spacing_m <= 0.0 || !spacing_m.is_finite() {
        return Err(ParcelPlacementError::InvalidGap);
    }

    let direction = if end.s_m >= s_m { 1.0 } else { -1.0 };
    let first_offset_m = existing.frontage_m * 0.5 + gap_m + frontage_m * 0.5;
    let limit_s = end.s_m;
    let mut center_s = s_m + direction * first_offset_m;
    let mut geometries = Vec::new();

    while directed_s_within_limit(center_s, limit_s, direction) {
        if center_s < frontage_m * 0.5 || center_s > edge.physical_length - frontage_m * 0.5 {
            break;
        }
        let Some((accepted_s, geometry)) = next_non_overlapping_run_geometry_directed(
            graph,
            edge_idx,
            if existing.side >= 0 { 1 } else { -1 },
            center_s,
            limit_s,
            direction,
            edge.physical_length,
            frontage_m,
            depth_m,
            &geometries,
            Some(existing),
        ) else {
            break;
        };
        geometries.push(geometry);
        center_s = accepted_s + direction * spacing_m;
    }

    if geometries.is_empty() {
        return Err(ParcelPlacementError::NoRoadAttachment);
    }
    Ok(geometries)
}

fn project_parcel_run_layouts_from_projected_start(
    graph: &RegionGraph,
    start: ProjectedRoadPoint,
    end_pos: Vector2,
    frontage_m: f32,
    depth_m: f32,
    gap_m: f32,
) -> Result<ParcelRunProjection, ParcelPlacementError> {
    let Some(end) = project_point_to_edge(graph, start.edge_idx, end_pos) else {
        return Err(ParcelPlacementError::NoRoadAttachment);
    };

    let spacing_m = frontage_m + gap_m;
    if spacing_m <= 0.0 || !spacing_m.is_finite() {
        return Err(ParcelPlacementError::InvalidGap);
    }

    let min_s = start.s_m.min(end.s_m);
    let max_s = start.s_m.max(end.s_m);
    let endpoint_direction = if end.s_m >= start.s_m { 1.0 } else { -1.0 };
    let span_m = max_s - min_s;
    let mut phases = Vec::new();
    push_unique_phase(&mut phases, 0.0);
    push_unique_phase(&mut phases, span_m.rem_euclid(spacing_m));

    let phase_steps = (spacing_m.ceil() as usize).clamp(1, RUN_PHASE_SEARCH_LIMIT);
    let phase_step_m = spacing_m / phase_steps as f32;
    for step in 1..phase_steps {
        push_unique_phase(&mut phases, phase_step_m * step as f32);
    }

    let mut layouts = Vec::with_capacity(phases.len());
    for phase_m in phases {
        if phase_m > span_m + RUN_PHASE_EPSILON_M {
            continue;
        }
        let layout = project_parcel_run_layout_from_phase(
            graph,
            start.edge_idx,
            start.side,
            min_s + phase_m,
            max_s,
            start.edge_len_m,
            frontage_m,
            depth_m,
            spacing_m,
        );
        if !layout.is_empty() {
            layouts.push(layout);
        }
    }

    if layouts.is_empty() {
        return Err(ParcelPlacementError::NoRoadAttachment);
    }

    Ok(ParcelRunProjection {
        layouts,
        endpoint_direction,
    })
}

fn project_parcel_run_layout_from_phase(
    graph: &RegionGraph,
    edge_idx: usize,
    side: i8,
    first_center_s: f32,
    max_s: f32,
    edge_len_m: f32,
    frontage_m: f32,
    depth_m: f32,
    spacing_m: f32,
) -> Vec<ParcelGeometry> {
    let mut center_s = first_center_s;
    let mut geometries = Vec::new();
    while center_s <= max_s + 0.001 {
        if center_s < frontage_m * 0.5 || center_s > edge_len_m - frontage_m * 0.5 {
            break;
        }
        let Some((accepted_s, geometry)) = next_non_overlapping_run_geometry(
            graph,
            edge_idx,
            side,
            center_s,
            max_s,
            edge_len_m,
            frontage_m,
            depth_m,
            &geometries,
        ) else {
            break;
        };
        geometries.push(geometry);
        center_s = accepted_s + spacing_m;
    }
    geometries
}

fn push_unique_phase(phases: &mut Vec<f32>, phase_m: f32) {
    if phase_m < RUN_PHASE_EPSILON_M {
        if phases
            .iter()
            .any(|existing| existing.abs() < RUN_PHASE_EPSILON_M)
        {
            return;
        }
        phases.push(0.0);
        return;
    }
    if phases
        .iter()
        .any(|existing| (*existing - phase_m).abs() < RUN_PHASE_EPSILON_M)
    {
        return;
    }
    phases.push(phase_m);
}
