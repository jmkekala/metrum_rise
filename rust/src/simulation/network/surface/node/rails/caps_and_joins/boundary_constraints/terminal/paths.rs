// SPDX-License-Identifier: GPL-2.0-only

//! Shared terminal-cap path and side-edge extraction helpers.

use super::super::*;

pub(super) fn terminal_cap_material_boundary_side_edges(
    cap_band: &NodeTerminalCapBand,
) -> Vec<(RoadVec3, RoadVec3)> {
    let Some(inner_start_world) = cap_band.inner_path_world.first().copied() else {
        return Vec::new();
    };
    let Some(inner_end_world) = cap_band.inner_path_world.last().copied() else {
        return Vec::new();
    };
    let Some(outer_start_world) = cap_band.outer_path_world.first().copied() else {
        return Vec::new();
    };
    let Some(outer_end_world) = cap_band.outer_path_world.last().copied() else {
        return Vec::new();
    };
    [
        (inner_start_world, outer_start_world),
        (inner_end_world, outer_end_world),
    ]
    .into_iter()
    .filter(|(start, end)| road_point_key(xz(*start)) != road_point_key(xz(*end)))
    .collect()
}

pub(super) fn terminal_cap_side_footprint_edge_for_role(
    cap_band: &NodeTerminalCapBand,
    role: TerminalCapBandRole,
) -> Option<(RoadVec3, RoadVec3)> {
    match role {
        TerminalCapBandRole::LeftSide => terminal_cap_start_side_edge(cap_band),
        TerminalCapBandRole::RightSide => terminal_cap_end_side_edge(cap_band),
        TerminalCapBandRole::LeftCorner
        | TerminalCapBandRole::EndBand
        | TerminalCapBandRole::RightCorner => None,
    }
}

pub(super) fn terminal_cap_corner_material_edge_for_role(
    cap_band: &NodeTerminalCapBand,
    role: TerminalCapBandRole,
) -> Option<(RoadVec3, RoadVec3)> {
    match role {
        TerminalCapBandRole::LeftCorner => terminal_cap_end_side_edge(cap_band),
        TerminalCapBandRole::RightCorner => terminal_cap_start_side_edge(cap_band),
        TerminalCapBandRole::LeftSide
        | TerminalCapBandRole::EndBand
        | TerminalCapBandRole::RightSide => None,
    }
}

pub(super) fn terminal_cap_corner_footprint_edge_for_role(
    cap_band: &NodeTerminalCapBand,
    role: TerminalCapBandRole,
) -> Option<(RoadVec3, RoadVec3)> {
    match role {
        TerminalCapBandRole::LeftCorner => terminal_cap_start_side_edge(cap_band),
        TerminalCapBandRole::RightCorner => terminal_cap_end_side_edge(cap_band),
        TerminalCapBandRole::LeftSide
        | TerminalCapBandRole::EndBand
        | TerminalCapBandRole::RightSide => None,
    }
}

pub(super) fn terminal_cap_band_inner_contour_path(
    cap_band: &NodeTerminalCapBand,
) -> Option<Vec<RoadVec2>> {
    let points = cap_band
        .inner_path_world
        .iter()
        .copied()
        .map(xz)
        .collect::<Vec<_>>();
    clean_generated_constraint_path(points)
}

pub(super) fn terminal_cap_band_outer_contour_path(
    cap_band: &NodeTerminalCapBand,
) -> Option<Vec<RoadVec2>> {
    let points = cap_band
        .outer_path_world
        .iter()
        .copied()
        .map(xz)
        .collect::<Vec<_>>();
    clean_generated_constraint_path(points)
}

fn terminal_cap_start_side_edge(cap_band: &NodeTerminalCapBand) -> Option<(RoadVec3, RoadVec3)> {
    let inner_start_world = cap_band.inner_path_world.first().copied()?;
    let outer_start_world = cap_band.outer_path_world.first().copied()?;
    (road_point_key(xz(inner_start_world)) != road_point_key(xz(outer_start_world)))
        .then_some((inner_start_world, outer_start_world))
}

fn terminal_cap_end_side_edge(cap_band: &NodeTerminalCapBand) -> Option<(RoadVec3, RoadVec3)> {
    let inner_end_world = cap_band.inner_path_world.last().copied()?;
    let outer_end_world = cap_band.outer_path_world.last().copied()?;
    (road_point_key(xz(inner_end_world)) != road_point_key(xz(outer_end_world)))
        .then_some((inner_end_world, outer_end_world))
}
