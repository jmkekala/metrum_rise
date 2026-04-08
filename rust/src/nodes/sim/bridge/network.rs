//! Godot-Rust bridge helpers for network-related geometry formatting.
use crate::nodes::sim::core::SimCore;
use godot::prelude::*;

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
