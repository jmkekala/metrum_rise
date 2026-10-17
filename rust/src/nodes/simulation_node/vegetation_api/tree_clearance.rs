// SPDX-License-Identifier: GPL-2.0-only

//! Walkway and lamp setbacks for wild and brushed trees over existing surface and road chunk
//! indices.

use super::*;
use crate::simulation::network::surface::{RoadSurfaceBandKind, RoadVec2};

// Trunk-to-edge gap beside footpaths and sidewalks. Crowns (up to about 5 m across the
// broadleaf models) may still overhang a little, as street-side trees do; the gap keeps
// trunks and the bulk of the canopy off the path without opening a wide corridor.
const WALKWAY_CLEAR_RADIUS_M: f64 = 3.0;
// Trunk-to-pole gap, so a crown does not swallow a lamp head.
const LAMP_CLEAR_RADIUS_M: f32 = 4.0;
// Poles stand inside a sidewalk or at most this far beyond a road band's outer edge
// (street_lights.rs VERGE_OFFSET_M, PATH_VERGE_OFFSET_M).
const POLE_BEYOND_SURFACE_M: f64 = 0.75;

/// Whether a tree trunk keeps clear of walkways and lamp poles. Generated and brushed trees
/// both follow it; a yard planting's trees stand where its asset put them.
pub(super) fn clear(core: &SimCore, x: f32, z: f32) -> bool {
    let surface = &core.transit_network.road_surface;
    let point = RoadVec2::new(f64::from(x), f64::from(z));
    let near = |radius, kinds: fn(RoadSurfaceBandKind) -> bool| {
        surface.surface_near_disc(&core.region_graph, &core.heightmap, point, radius, kinds)
    };
    if near(WALKWAY_CLEAR_RADIUS_M, |kind| {
        matches!(
            kind,
            RoadSurfaceBandKind::Footpath | RoadSurfaceBandKind::Sidewalk
        )
    }) {
        return false;
    }
    // No surface within reach means no pole within reach: most forest candidates stop here.
    !near(
        f64::from(LAMP_CLEAR_RADIUS_M) + POLE_BEYOND_SURFACE_M,
        |_| true,
    ) || lamps_clear(core, Vector2::new(x, z))
}

fn lamps_clear(core: &SimCore, point: Vector2) -> bool {
    let surface = &core.transit_network.road_surface;
    let span = surface.chunk_span_m();
    let (ox, oz) = surface.chunk_origin_m();
    let origin = Vector2::new(ox, oz);
    let first = ((point - Vector2::splat(LAMP_CLEAR_RADIUS_M) - origin) / span).floor();
    let last = ((point + Vector2::splat(LAMP_CLEAR_RADIUS_M) - origin) / span).floor();
    // The exact poles, already routed into road mesh chunks (chunk-local XZ) by
    // push_street_light and rebuilt before publication. Only candidates beside a road get
    // here; each scans the poles of its one to four 512 m chunks, bounded by street density
    // per chunk rather than city size, without allocation.
    for cx in first.x as i32..=last.x as i32 {
        for cz in first.y as i32..=last.y as i32 {
            let Some(mesh) = core.cached_road_mesh_chunks.get(&(cx, cz)) else {
                continue;
            };
            let local = point - origin - Vector2::new(cx as f32, cz as f32) * span;
            if mesh.street_lights.iter().any(|light| {
                local.distance_squared_to(Vector2::new(light.position.x, light.position.z))
                    < LAMP_CLEAR_RADIUS_M * LAMP_CLEAR_RADIUS_M
            }) {
                return false;
            }
        }
    }
    true
}
