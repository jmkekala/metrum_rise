//! Godot-Rust bridge helpers for network-related geometry formatting.
use crate::config::HEIGHT_SCALE;
use crate::debug_log;
use crate::nodes::sim::core::SimCore;
use godot::prelude::*;
use std::time::Instant;

const GHOST_GRID_SPACING_M: f32 = 80.0;
const GHOST_MAX_OFFSETS: usize = 3;
const GHOST_OUTWARD_EXTEND_M: f32 = 200.0;
const GHOST_TICK_INTERVAL_M: f32 = 20.0;
const GHOST_TICK_HALF_M: f32 = 1.5;
const GHOST_LINE_LIFT_M: f32 = 0.06;
const GHOST_TICK_LIFT_M: f32 = 0.07;
const GHOST_OFFSET_ALPHAS: [f32; GHOST_MAX_OFFSETS] = [0.30, 0.12, 0.04];

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
        let t0 = (geom[0] - geom[1]).normalized();
        out.push(geom[0].x);
        out.push(geom[0].z);
        out.push(t0.x);
        out.push(t0.z);
        // End endpoint — tangent points outward
        let t1 = (geom[n - 1] - geom[n - 2]).normalized();
        out.push(geom[n - 1].x);
        out.push(geom[n - 1].z);
        out.push(t1.x);
        out.push(t1.z);
    }
    out
}

/// Returns complete ghost-guide line vertices and colors for direct Godot upload.
pub fn get_road_ghost_line_data(core: &mut SimCore) -> VarDictionary {
    let road_debug = crate::debug::category_enabled("road");
    let total_start = road_debug.then(Instant::now);
    let compile_start = road_debug.then(Instant::now);
    core.transit_network
        .road_surface
        .compile_dirty(&core.region_graph, &core.heightmap);
    let compile_ms = compile_start
        .map(|start| start.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0);

    let mut vertices = Vec::new();
    let mut colors = Vec::new();
    let guide_color = Color::from_rgba(1.0, 1.0, 1.0, 0.30);

    let outward_start = road_debug.then(Instant::now);
    let mut edge_count = 0usize;
    for edge in core
        .region_graph
        .edges()
        .iter()
        .filter(|edge| !edge.deleted && edge.physical_geometry.len() >= 2)
    {
        edge_count += 1;
        let geom = &edge.physical_geometry;
        let end_index = geom.len() - 1;
        let start_tangent = (geom[0] - geom[1]).normalized();
        append_outward_ghost_guide(
            core,
            Vector2::new(geom[0].x, geom[0].z),
            Vector2::new(start_tangent.x, start_tangent.z),
            guide_color,
            &mut vertices,
            &mut colors,
        );

        let end_tangent = (geom[end_index] - geom[end_index - 1]).normalized();
        append_outward_ghost_guide(
            core,
            Vector2::new(geom[end_index].x, geom[end_index].z),
            Vector2::new(end_tangent.x, end_tangent.z),
            guide_color,
            &mut vertices,
            &mut colors,
        );
    }
    let outward_ms = outward_start
        .map(|start| start.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0);

    let offset_start = road_debug.then(Instant::now);
    for edge in core
        .region_graph
        .edges()
        .iter()
        .filter(|edge| !edge.deleted && edge.physical_geometry.len() >= 2)
    {
        let points: Vec<Vector2> = edge
            .physical_geometry
            .iter()
            .map(|point| Vector2::new(point.x, point.z))
            .collect();
        for offset_index in 1..=GHOST_MAX_OFFSETS {
            let alpha = GHOST_OFFSET_ALPHAS[offset_index - 1];
            let color = Color::from_rgba(1.0, 1.0, 1.0, alpha);
            let offset = offset_index as f32 * GHOST_GRID_SPACING_M;
            append_offset_ghost_curve(core, &points, offset, color, &mut vertices, &mut colors);
            append_offset_ghost_curve(core, &points, -offset, color, &mut vertices, &mut colors);
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
            vertex_count,
            total_start
                .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                .unwrap_or(0.0)
        );
    }
    dict
}

/// Returns the nearest ghost-guide snap point, with height resolved in Rust.
pub fn get_road_ghost_snap(
    core: &mut SimCore,
    pos: Vector3,
    max_dist_m: f32,
    altitude_offset_m: f32,
) -> Option<Vector3> {
    core.transit_network
        .road_surface
        .compile_dirty(&core.region_graph, &core.heightmap);

    let query = Vector2::new(pos.x, pos.z);
    let mut best_dist = max_dist_m;
    let mut best_point = None;

    for edge in core
        .region_graph
        .edges()
        .iter()
        .filter(|edge| !edge.deleted && edge.physical_geometry.len() >= 2)
    {
        let geom = &edge.physical_geometry;
        let end_index = geom.len() - 1;
        let start_tangent = (geom[0] - geom[1]).normalized();
        update_best_outward_ghost_snap(
            query,
            Vector2::new(geom[0].x, geom[0].z),
            Vector2::new(start_tangent.x, start_tangent.z),
            &mut best_dist,
            &mut best_point,
        );

        let end_tangent = (geom[end_index] - geom[end_index - 1]).normalized();
        update_best_outward_ghost_snap(
            query,
            Vector2::new(geom[end_index].x, geom[end_index].z),
            Vector2::new(end_tangent.x, end_tangent.z),
            &mut best_dist,
            &mut best_point,
        );
    }

    for edge in core
        .region_graph
        .edges()
        .iter()
        .filter(|edge| !edge.deleted && edge.physical_geometry.len() >= 2)
    {
        let points: Vec<Vector2> = edge
            .physical_geometry
            .iter()
            .map(|point| Vector2::new(point.x, point.z))
            .collect();
        for offset_index in 1..=GHOST_MAX_OFFSETS {
            let offset = offset_index as f32 * GHOST_GRID_SPACING_M;
            update_best_offset_ghost_snap(query, &points, offset, &mut best_dist, &mut best_point);
            update_best_offset_ghost_snap(query, &points, -offset, &mut best_dist, &mut best_point);
        }
    }

    best_point.map(|point| {
        Vector3::new(
            point.x,
            ghost_surface_height_m(core, point) + altitude_offset_m,
            point.y,
        )
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
    core: &SimCore,
    anchor: Vector2,
    tangent: Vector2,
    color: Color,
    vertices: &mut Vec<Vector3>,
    colors: &mut Vec<Color>,
) {
    let perp = Vector2::new(-tangent.y, tangent.x);
    let end = anchor + tangent * GHOST_OUTWARD_EXTEND_M;
    push_ghost_line(
        core,
        anchor,
        end,
        GHOST_LINE_LIFT_M,
        color,
        vertices,
        colors,
    );

    let mut dist = GHOST_TICK_INTERVAL_M;
    while dist <= GHOST_OUTWARD_EXTEND_M {
        let tick_center = anchor + tangent * dist;
        push_ghost_line(
            core,
            tick_center - perp * GHOST_TICK_HALF_M,
            tick_center + perp * GHOST_TICK_HALF_M,
            GHOST_TICK_LIFT_M,
            color,
            vertices,
            colors,
        );
        dist += GHOST_TICK_INTERVAL_M;
    }
}

fn append_offset_ghost_curve(
    core: &SimCore,
    points: &[Vector2],
    offset_m: f32,
    color: Color,
    vertices: &mut Vec<Vector3>,
    colors: &mut Vec<Color>,
) {
    if points.len() < 2 {
        return;
    }

    let mut offset_segments = Vec::with_capacity(points.len().saturating_sub(1));
    for segment in points.windows(2) {
        let a = segment[0];
        let b = segment[1];
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
        push_ghost_line(core, a, b, GHOST_LINE_LIFT_M, color, vertices, colors);
    }
}

fn update_best_outward_ghost_snap(
    query: Vector2,
    anchor: Vector2,
    tangent: Vector2,
    best_dist: &mut f32,
    best_point: &mut Option<Vector2>,
) {
    let to_query = query - anchor;
    let along = to_query.dot(tangent);
    if along < 0.0 {
        return;
    }
    let closest = anchor + tangent * along;
    update_best_ghost_point(query, closest, best_dist, best_point);
}

fn update_best_offset_ghost_snap(
    query: Vector2,
    points: &[Vector2],
    offset_m: f32,
    best_dist: &mut f32,
    best_point: &mut Option<Vector2>,
) {
    for segment in points.windows(2) {
        let a = segment[0];
        let b = segment[1];
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
        let offset_seg = offset_b - offset_a;
        let along =
            ((query - offset_a).dot(offset_seg) / offset_seg.length_squared()).clamp(0.0, 1.0);
        update_best_ghost_point(query, offset_a + offset_seg * along, best_dist, best_point);
    }
}

fn update_best_ghost_point(
    query: Vector2,
    candidate: Vector2,
    best_dist: &mut f32,
    best_point: &mut Option<Vector2>,
) {
    let distance = (query - candidate).length();
    if distance < *best_dist {
        *best_dist = distance;
        *best_point = Some(candidate);
    }
}

fn push_ghost_line(
    core: &SimCore,
    start: Vector2,
    end: Vector2,
    lift_m: f32,
    color: Color,
    vertices: &mut Vec<Vector3>,
    colors: &mut Vec<Color>,
) {
    vertices.push(Vector3::new(
        start.x,
        ghost_surface_height_m(core, start) + lift_m,
        start.y,
    ));
    vertices.push(Vector3::new(
        end.x,
        ghost_surface_height_m(core, end) + lift_m,
        end.y,
    ));
    colors.push(color);
    colors.push(color);
}

fn ghost_surface_height_m(core: &SimCore, pos: Vector2) -> f32 {
    core.transit_network
        .road_surface
        .sample_visible_surface_height(&core.region_graph, &core.heightmap, pos.x, pos.y)
        .unwrap_or_else(|| core.heightmap.sample_visual_height_world(pos.x, pos.y) * HEIGHT_SCALE)
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
