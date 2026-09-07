// SPDX-License-Identifier: GPL-2.0-only

//! Godot-Rust bridge helpers for network-related geometry formatting.
use crate::config::HEIGHT_SCALE;
use crate::nodes::sim::core::SimCore;
use crate::nodes::sim::road_tool::{
    GHOST_GRID_SPACING_M, GHOST_LINE_LIFT_M, GHOST_MAX_OFFSETS, GHOST_OFFSET_ALPHAS,
    GHOST_OUTWARD_EXTEND_M, GHOST_TICK_HALF_M, GHOST_TICK_INTERVAL_M, GHOST_TICK_LIFT_M,
    RoadGhostSnapIndex, endpoint_tangent_xz,
};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::surface::RoadSurfaceSystem;
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::*;
mod ghost_lines;
pub(crate) use ghost_lines::RoadGhostLineCache;

/// Returns ghost guide data for the road-tool overlay.
pub fn get_road_ghost_guides(core: &SimCore) -> PackedFloat32Array {
    let graph = &core.region_graph;
    let mut out = PackedFloat32Array::new();
    for edge in graph
        .edges()
        .iter()
        .filter(|e| !e.deleted && e.physical_geometry.len() >= 2)
    {
        let geom = &edge.physical_geometry;
        let n = geom.len();
        // Start endpoint — tangent points outward
        if let Some(t0) = endpoint_tangent_xz(geom[0], geom[1]) {
            out.push(geom[0].x);
            out.push(geom[0].z);
            out.push(t0.x);
            out.push(t0.y);
        }
        // End endpoint — tangent points outward
        if let Some(t1) = endpoint_tangent_xz(geom[n - 1], geom[n - 2]) {
            out.push(geom[n - 1].x);
            out.push(geom[n - 1].z);
            out.push(t1.x);
            out.push(t1.y);
        }
    }
    out
}

/// Returns complete ghost-guide line vertices and colors for direct Godot upload.
pub fn get_road_ghost_line_data(core: &mut SimCore) -> VarDictionary {
    if !core
        .transit_network
        .road_surface
        .published_generation_matches_source()
        || core.cached_road_mesh_generation != core.road_tool_surface_generation
    {
        return VarDictionary::new();
    }
    core.refresh_road_ghost_lines();
    let mut dict = VarDictionary::new();
    dict.set("generation", core.road_tool_surface_generation as i64);
    dict.set("rebuilt_edges", core.road_ghost_lines.rebuilt_edges as i64);
    dict.set(
        "vertices",
        PackedVector3Array::from_iter(core.road_ghost_lines.vertices.iter().copied()),
    );
    dict.set(
        "colors",
        PackedColorArray::from_iter(core.road_ghost_lines.colors.iter().copied()),
    );
    dict
}

/// Returns the nearest ghost-guide snap point, with height resolved in Rust.
pub fn get_road_ghost_snap(
    core: &SimCore,
    pos: Vector3,
    max_dist_m: f32,
    altitude_offset_m: f32,
) -> Option<Vector3> {
    let ghost_snap_index = RoadGhostSnapIndex::from_graph(&core.region_graph);
    get_road_ghost_snap_from_parts(
        &core.region_graph,
        &core.transit_network.road_surface,
        &core.heightmap,
        &ghost_snap_index,
        pos,
        max_dist_m,
        altitude_offset_m,
    )
}

/// Returns the nearest ghost-guide snap point from immutable road-query data.
pub(crate) fn get_road_ghost_snap_from_parts(
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
    terrain: &TerrainSystem,
    ghost_snap_index: &RoadGhostSnapIndex,
    pos: Vector3,
    max_dist_m: f32,
    altitude_offset_m: f32,
) -> Option<Vector3> {
    let query = Vector2::new(pos.x, pos.z);
    ghost_snap_index
        .nearest_point(query, max_dist_m)
        .map(|point| {
            let height_m = road_surface
                .sample_visible_surface_height(graph, terrain, point.x, point.y)
                .unwrap_or_else(|| {
                    terrain.sample_visual_height_world(point.x, point.y) * HEIGHT_SCALE
                        + altitude_offset_m
                });
            Vector3::new(point.x, height_m, point.y)
        })
}

/// Returns the full physical geometry of every non-deleted road edge.
pub fn get_road_edge_polylines(core: &SimCore) -> PackedFloat32Array {
    let mut out = PackedFloat32Array::new();
    for edge in core.region_graph.edges().iter().filter(|e| !e.deleted) {
        let geom = &edge.physical_geometry;
        out.push(geom.len() as f32);
        for p in geom {
            out.push(p.x);
            out.push(p.z);
        }
    }
    out
}

/// Returns the road tangent direction closest to `pos` within `max_dist` metres.
pub fn get_road_tangent_at(core: &SimCore, pos: Vector3, max_dist: f32) -> Vector2 {
    let graph = &core.region_graph;
    let mut best_dist_sq = max_dist * max_dist;
    let mut best_tangent = Vector2::new(0.0, 1.0); // fallback: world +Z (north)

    for edge in graph
        .edges()
        .iter()
        .filter(|e| !e.deleted && e.physical_geometry.len() >= 2)
    {
        let geom = &edge.physical_geometry;
        for seg in geom.windows(2) {
            let a = seg[0];
            let b = seg[1];
            let abx = b.x - a.x;
            let abz = b.z - a.z;
            let len_sq = abx * abx + abz * abz;
            if len_sq < 1e-6 {
                continue;
            }
            let t = ((pos.x - a.x) * abx + (pos.z - a.z) * abz) / len_sq;
            let t = t.clamp(0.0, 1.0);
            let cx = a.x + t * abx;
            let cz = a.z + t * abz;
            let dx = pos.x - cx;
            let dz = pos.z - cz;
            let dist_sq = dx * dx + dz * dz;
            if dist_sq < best_dist_sq {
                best_dist_sq = dist_sq;
                let inv_len = 1.0 / len_sq.sqrt();
                best_tangent = Vector2::new(abx * inv_len, abz * inv_len);
            }
        }
    }
    best_tangent
}
