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
    let min_frontage_s = frontage_m * 0.5;
    let max_frontage_s = edge_len_m - min_frontage_s;
    let mut low_s = min_center_s;
    let mut high_s = min_center_s;
    loop {
        if !directed_s_within_limit(high_s, limit_s, direction) {
            return None;
        }
        // Spacing on a curve may advance beyond the caller's initial station. Check the
        // normalized attachment that is actually persisted, including its f32 round trip.
        let attachment_s = (high_s / edge_len_m) * edge_len_m;
        if attachment_s < min_frontage_s || attachment_s > max_frontage_s {
            return None;
        }
        let geometry = geometry_from_attachment(
            graph,
            edge_idx,
            side,
            high_s / edge_len_m,
            frontage_m,
            depth_m,
        );
        if !geometry_overlaps_previous_or_blocking(
            &geometry,
            previous_geometries,
            blocking_geometry,
        ) {
            break;
        }
        low_s = high_s;
        high_s += direction * CURVE_RUN_SPACING_SEARCH_STEP_M;
    }

    if (high_s - min_center_s).abs() <= OVERLAP_EPSILON_M {
        return Some((
            high_s,
            geometry_from_attachment(
                graph,
                edge_idx,
                side,
                high_s / edge_len_m,
                frontage_m,
                depth_m,
            ),
        ));
    }

    for _ in 0..CURVE_RUN_SPACING_BINARY_STEPS {
        let mid_s = (low_s + high_s) * 0.5;
        let geometry = geometry_from_attachment(
            graph,
            edge_idx,
            side,
            mid_s / edge_len_m,
            frontage_m,
            depth_m,
        );
        if geometry_overlaps_previous_or_blocking(&geometry, previous_geometries, blocking_geometry)
        {
            low_s = mid_s;
        } else {
            high_s = mid_s;
        }
    }

    Some((
        high_s,
        geometry_from_attachment(
            graph,
            edge_idx,
            side,
            high_s / edge_len_m,
            frontage_m,
            depth_m,
        ),
    ))
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
