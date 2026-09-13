// SPDX-License-Identifier: GPL-2.0-only

//! Non-overlapping spacing search for curved parcel runs.

use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::parcels::geometry::{geometries_overlap, geometry_from_attachment};
use crate::simulation::zoning::parcels::{
    CURVE_RUN_SPACING_BINARY_STEPS, CURVE_RUN_SPACING_SEARCH_STEP_M, OVERLAP_EPSILON_M,
    ParcelGeometry,
};

pub(super) fn next_non_overlapping_run_geometry(
    graph: &RegionGraph,
    edge_idx: usize,
    side: i8,
    min_center_s: f32,
    max_center_s: f32,
    edge_len_m: f32,
    frontage_m: f32,
    depth_m: f32,
    previous_geometries: &[ParcelGeometry],
) -> Option<(f32, ParcelGeometry)> {
    next_non_overlapping_run_geometry_directed(
        graph,
        edge_idx,
        side,
        min_center_s,
        max_center_s,
        1.0,
        edge_len_m,
        frontage_m,
        depth_m,
        previous_geometries,
        None,
    )
}

pub(super) fn next_non_overlapping_run_geometry_directed(
    graph: &RegionGraph,
    edge_idx: usize,
    side: i8,
    min_center_s: f32,
    limit_s: f32,
    direction: f32,
    edge_len_m: f32,
    frontage_m: f32,
    depth_m: f32,
    previous_geometries: &[ParcelGeometry],
    blocking_geometry: Option<&ParcelGeometry>,
) -> Option<(f32, ParcelGeometry)> {
    next_non_overlapping_run_geometry_with(
        graph,
        edge_idx,
        side,
        min_center_s,
        limit_s,
        direction,
        edge_len_m,
        frontage_m,
        depth_m,
        |geometry| {
            geometry_overlaps_previous_or_blocking(geometry, previous_geometries, blocking_geometry)
        },
    )
}

/// Advances the shared run-spacing solver using a caller's local overlap query.
/// The accepted station obeys the same saved-attachment bounds and search tolerance as drag runs.
pub(crate) fn next_non_overlapping_run_geometry_with(
    graph: &RegionGraph,
    edge_idx: usize,
    side: i8,
    min_center_s: f32,
    limit_s: f32,
    direction: f32,
    edge_len_m: f32,
    frontage_m: f32,
    depth_m: f32,
    mut overlaps: impl FnMut(&ParcelGeometry) -> bool,
) -> Option<(f32, ParcelGeometry)> {
    let min_frontage_s = frontage_m * 0.5;
    let max_frontage_s = edge_len_m - min_frontage_s;
    let geometry_at = |station: f32| {
        let mut center_t = station / edge_len_m;
        // A legal endpoint can round outside the strict saved-attachment bounds. Move
        // its normalized representation one float inward, without relaxing those bounds.
        let attachment_s = center_t * edge_len_m;
        if attachment_s < min_frontage_s {
            center_t = center_t.next_up();
        } else if attachment_s > max_frontage_s {
            center_t = center_t.next_down();
        }
        geometry_from_attachment(graph, edge_idx, side, center_t, frontage_m, depth_m)
    };
    let mut low_s = min_center_s;
    let mut high_s = min_center_s;
    loop {
        if !directed_s_within_limit(high_s, limit_s, direction) {
            return None;
        }
        // Reject actual overhang before normalizing the attachment for persistence.
        if high_s < min_frontage_s || high_s > max_frontage_s {
            return None;
        }
        let geometry = geometry_at(high_s);
        if !overlaps(&geometry) {
            break;
        }
        low_s = high_s;
        // Probe the remaining end of the interval before giving up. A full search step can
        // overshoot the road end even when a tightly packed last lot still fits in the bracket.
        high_s = if direction >= 0.0 {
            (high_s + CURVE_RUN_SPACING_SEARCH_STEP_M)
                .min(limit_s)
                .min(max_frontage_s)
        } else {
            (high_s - CURVE_RUN_SPACING_SEARCH_STEP_M)
                .max(limit_s)
                .max(min_frontage_s)
        };
        if (high_s - low_s) * direction <= 0.0 {
            return None;
        }
    }

    if (high_s - min_center_s).abs() <= OVERLAP_EPSILON_M {
        return Some((high_s, geometry_at(high_s)));
    }

    for _ in 0..CURVE_RUN_SPACING_BINARY_STEPS {
        let mid_s = (low_s + high_s) * 0.5;
        let geometry = geometry_at(mid_s);
        if overlaps(&geometry) {
            low_s = mid_s;
        } else {
            high_s = mid_s;
        }
    }

    Some((high_s, geometry_at(high_s)))
}

pub(super) fn directed_s_within_limit(s_m: f32, limit_s: f32, direction: f32) -> bool {
    if direction >= 0.0 {
        s_m <= limit_s + OVERLAP_EPSILON_M
    } else {
        s_m >= limit_s - OVERLAP_EPSILON_M
    }
}

fn geometry_overlaps_previous_or_blocking(
    geometry: &ParcelGeometry,
    previous_geometries: &[ParcelGeometry],
    blocking_geometry: Option<&ParcelGeometry>,
) -> bool {
    blocking_geometry.is_some_and(|blocking| geometries_overlap(blocking, geometry))
        || previous_geometries
            .iter()
            .any(|previous| geometries_overlap(previous, geometry))
}
