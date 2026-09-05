// SPDX-License-Identifier: GPL-2.0-only

//! Raised-step key and owner literal writers.

use super::*;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface::debug) fn append_node_band_owner_literal(
        dump: &mut String,
        owner: NodeBandOwner,
    ) {
        let _ = write!(
            dump,
            "{{\"kind\":\"{:?}\",\"owner_index\":{}}}",
            owner.kind(),
            owner.owner_index()
        );
    }

    pub(in crate::simulation::network::surface::debug) fn append_node_arrangement_segment_key_literal(
        dump: &mut String,
        start: NodeArrangementKey,
        end: NodeArrangementKey,
    ) {
        let _ = write!(
            dump,
            "{{\"start\":{{\"x_key\":{},\"z_key\":{},\"x_mm\":{},\"z_mm\":{}}},\"end\":{{\"x_key\":{},\"z_key\":{},\"x_mm\":{},\"z_mm\":{}}}}}",
            start.x_key(),
            start.z_key(),
            start.x_mm(),
            start.z_mm(),
            end.x_key(),
            end.z_key(),
            end.x_mm(),
            end.z_mm()
        );
    }

    pub(in crate::simulation::network::surface::debug) fn append_debug_top_boundary_edge_list_literal(
        dump: &mut String,
        edges: &[DebugTopBoundaryEdge],
    ) {
        dump.push('[');
        for (index, edge) in edges.iter().enumerate() {
            if index > 0 {
                dump.push_str(", ");
            }
            Self::append_debug_top_boundary_edge_literal(dump, *edge);
        }
        dump.push(']');
    }

    pub(in crate::simulation::network::surface::debug) fn append_debug_top_boundary_edge_literal(
        dump: &mut String,
        edge: DebugTopBoundaryEdge,
    ) {
        dump.push('{');
        dump.push_str("\"owner\":");
        Self::append_debug_boundary_owner_literal(dump, edge.owner);
        dump.push_str(",\"edge_world\":");
        Self::append_vector3_pair_precise_literal(dump, edge.start, edge.end);
        dump.push_str(",\"edge_key\":");
        Self::append_debug_render_edge_key_literal(dump, edge.key);
        dump.push_str(",\"xz_key\":");
        Self::append_debug_render_xz_edge_key_literal(dump, edge.xz_key);
        let _ = write!(dump, ",\"avg_y_m\":{:.6}", edge.avg_y_m);
        dump.push('}');
    }

    pub(in crate::simulation::network::surface::debug) fn append_debug_boundary_owner_literal(
        dump: &mut String,
        owner: DebugBoundaryOwner,
    ) {
        let _ = write!(
            dump,
            "{{\"region\":{},\"kind\":\"{:?}\",\"owner_index\":{}}}",
            owner.region_index, owner.kind, owner.owner_index
        );
    }
}
