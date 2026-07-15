//! Godot-Rust bridge helpers for network-related geometry formatting.
use crate::config::HEIGHT_SCALE;
use crate::debug_log;
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
use std::time::Instant;

const GHOST_HEIGHT_SAMPLE_STEP_M: f32 = 8.0;

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
    let road_debug = crate::debug::category_enabled("road");
    let total_start = road_debug.then(Instant::now);
    let compile_ms = 0.0;

    let mut vertices = Vec::new();
    let mut colors = Vec::new();
    let guide_color = Color::from_rgba(1.0, 1.0, 1.0, 0.30);
    let graph = &core.region_graph;
    let road_surface = &core.transit_network.road_surface;
    let terrain = &core.heightmap;
    let mut height_samples = 0usize;

    let outward_start = road_debug.then(Instant::now);
    let mut edge_count = 0usize;
    for edge in graph
        .edges()
        .iter()
        .filter(|edge| !edge.deleted && edge.physical_geometry.len() >= 2)
    {
        edge_count += 1;
        let geom = &edge.physical_geometry;
        let end_index = geom.len() - 1;
        if let Some(start_tangent) = endpoint_tangent_xz(geom[0], geom[1]) {
            append_outward_ghost_guide(
                geom[0],
                start_tangent,
                graph,
                road_surface,
                terrain,
                guide_color,
                &mut vertices,
                &mut colors,
                &mut height_samples,
            );
        }

        if let Some(end_tangent) = endpoint_tangent_xz(geom[end_index], geom[end_index - 1]) {
            append_outward_ghost_guide(
                geom[end_index],
                end_tangent,
                graph,
                road_surface,
                terrain,
                guide_color,
                &mut vertices,
                &mut colors,
                &mut height_samples,
            );
        }
    }
    let outward_ms = outward_start
        .map(|start| start.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0);

    let offset_start = road_debug.then(Instant::now);
    for edge in graph
        .edges()
        .iter()
        .filter(|edge| !edge.deleted && edge.physical_geometry.len() >= 2)
    {
        for offset_index in 1..=GHOST_MAX_OFFSETS {
            let alpha = GHOST_OFFSET_ALPHAS[offset_index - 1];
            let color = Color::from_rgba(1.0, 1.0, 1.0, alpha);
            let offset = offset_index as f32 * GHOST_GRID_SPACING_M;
            append_offset_ghost_curve(
                &edge.physical_geometry,
                offset,
                graph,
                road_surface,
                terrain,
                color,
                &mut vertices,
                &mut colors,
                &mut height_samples,
            );
            append_offset_ghost_curve(
                &edge.physical_geometry,
                -offset,
                graph,
                road_surface,
                terrain,
                color,
                &mut vertices,
                &mut colors,
                &mut height_samples,
            );
        }
    }
    let offset_ms = offset_start
        .map(|start| start.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0);

    let vertex_count = vertices.len();
    let color_count = colors.len();
    let dict_start = road_debug.then(Instant::now);
    let mut dict = VarDictionary::new();
    dict.set("vertices", PackedVector3Array::from_iter(vertices));
    dict.set("colors", PackedColorArray::from_iter(colors));
    let dict_ms = dict_start
        .map(|start| start.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0);
    if road_debug {
        debug_log!(
            "road",
            "ghost_lines_rust edges={} vertices={} colors={} compile_ms={:.3} outward_ms={:.3} offset_ms={:.3} dict_ms={:.3} height_samples={} total_ms={:.3}",
            edge_count,
            vertex_count,
            color_count,
            compile_ms,
            outward_ms,
            offset_ms,
            dict_ms,
            height_samples,
            total_start
                .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                .unwrap_or(0.0)
        );
    }
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

fn append_outward_ghost_guide(
    anchor: Vector3,
    tangent: Vector2,
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
    terrain: &TerrainSystem,
    color: Color,
    vertices: &mut Vec<Vector3>,
    colors: &mut Vec<Color>,
    height_samples: &mut usize,
) {
    let perp = Vector2::new(-tangent.y, tangent.x);
    let anchor_xz = Vector2::new(anchor.x, anchor.z);
    let end_xz = anchor_xz + tangent * GHOST_OUTWARD_EXTEND_M;
    push_ghost_surface_line(
        anchor_xz,
        end_xz,
        GHOST_LINE_LIFT_M,
        graph,
        road_surface,
        terrain,
        color,
        vertices,
        colors,
        height_samples,
    );

    let mut dist = GHOST_TICK_INTERVAL_M;
    while dist <= GHOST_OUTWARD_EXTEND_M {
        let tick_center = anchor_xz + tangent * dist;
        push_ghost_surface_line(
            Vector2::new(
                tick_center.x - perp.x * GHOST_TICK_HALF_M,
                tick_center.y - perp.y * GHOST_TICK_HALF_M,
            ),
            Vector2::new(
                tick_center.x + perp.x * GHOST_TICK_HALF_M,
                tick_center.y + perp.y * GHOST_TICK_HALF_M,
            ),
            GHOST_TICK_LIFT_M,
            graph,
            road_surface,
            terrain,
            color,
            vertices,
            colors,
            height_samples,
        );
        dist += GHOST_TICK_INTERVAL_M;
    }
}

fn append_offset_ghost_curve(
    points: &[Vector3],
    offset_m: f32,
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
    terrain: &TerrainSystem,
    color: Color,
    vertices: &mut Vec<Vector3>,
    colors: &mut Vec<Color>,
    height_samples: &mut usize,
) {
    if points.len() < 2 {
        return;
    }

    let mut offset_segments = Vec::with_capacity(points.len().saturating_sub(1));
    for segment in points.windows(2) {
        let a = Vector2::new(segment[0].x, segment[0].z);
        let b = Vector2::new(segment[1].x, segment[1].z);
        let seg = b - a;
        if seg.length_squared() < 0.01 {
            continue;
        }
        let seg_norm = seg.normalized();
        let perp = Vector2::new(-seg_norm.y, seg_norm.x);
        let offset_a = a + perp * offset_m;
        let offset_b = b + perp * offset_m;
        if (offset_b - offset_a).dot(seg_norm) < 0.0 {
            continue;
        }
        offset_segments.push((offset_a, offset_b));
    }

    let mut skip_next = false;
    for index in 0..offset_segments.len() {
        if skip_next {
            skip_next = false;
            continue;
        }
        let (a, b) = offset_segments[index];
        if let Some((next_a, next_b)) = offset_segments.get(index + 1).copied() {
            if segments_cross_2d(a, b, next_a, next_b) {
                skip_next = true;
                continue;
            }
        }
        push_ghost_surface_line(
            a,
            b,
            GHOST_LINE_LIFT_M,
            graph,
            road_surface,
            terrain,
            color,
            vertices,
            colors,
            height_samples,
        );
    }
}

fn push_ghost_surface_line(
    start: Vector2,
    end: Vector2,
    lift_m: f32,
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
    terrain: &TerrainSystem,
    color: Color,
    vertices: &mut Vec<Vector3>,
    colors: &mut Vec<Color>,
    height_samples: &mut usize,
) {
    let delta = end - start;
    let length = delta.length();
    if length <= 0.01 {
        return;
    }

    let segment_count = (length / GHOST_HEIGHT_SAMPLE_STEP_M).ceil().max(1.0) as usize;
    let mut prev = ghost_surface_point_m(graph, road_surface, terrain, start, lift_m);
    *height_samples += 1;
    for step in 1..=segment_count {
        let t = step as f32 / segment_count as f32;
        let pos = start + delta * t;
        let next = ghost_surface_point_m(graph, road_surface, terrain, pos, lift_m);
        *height_samples += 1;
        vertices.push(prev);
        vertices.push(next);
        colors.push(color);
        colors.push(color);
        prev = next;
    }
}

fn ghost_surface_point_m(
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
    terrain: &TerrainSystem,
    pos: Vector2,
    lift_m: f32,
) -> Vector3 {
    Vector3::new(
        pos.x,
        ghost_surface_height_m(graph, road_surface, terrain, pos) + lift_m,
        pos.y,
    )
}

fn ghost_surface_height_m(
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
    terrain: &TerrainSystem,
    pos: Vector2,
) -> f32 {
    road_surface
        .sample_visible_surface_height(graph, terrain, pos.x, pos.y)
        .unwrap_or_else(|| terrain.sample_visual_height_world(pos.x, pos.y) * HEIGHT_SCALE)
}

fn segments_cross_2d(a1: Vector2, b1: Vector2, a2: Vector2, b2: Vector2) -> bool {
    let d1 = b1 - a1;
    let d2 = b2 - a2;
    let denom = d1.x * d2.y - d1.y * d2.x;
    if denom.abs() < 1e-6 {
        return false;
    }
    let t = ((a2.x - a1.x) * d2.y - (a2.y - a1.y) * d2.x) / denom;
    let u = ((a2.x - a1.x) * d1.y - (a2.y - a1.y) * d1.x) / denom;
    t > 0.0 && t < 1.0 && u > 0.0 && u < 1.0
}
