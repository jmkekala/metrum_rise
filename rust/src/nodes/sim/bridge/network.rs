// SPDX-License-Identifier: GPL-2.0-only

//! Godot-Rust bridge helpers for network-related geometry formatting.
use crate::nodes::sim::core::SimCore;
use godot::prelude::*;

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
