//! Source-provenance debug literal writers.

use super::*;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface::debug) fn append_span_raised_step_sources_debug_literal(
        dump: &mut String,
        piece: &RoadSurfaceVisualSpanPiece,
    ) {
        dump.push('[');
        for (source_index, source) in piece.span_raised_step_sources.iter().copied().enumerate() {
            if source_index > 0 {
                dump.push_str(", ");
            }
            let _ = write!(dump, "{{\"face_index\":{},\"lower_owner\":", source_index);
            Self::append_span_band_owner_debug_literal(dump, source.lower_owner);
            dump.push_str(",\"raised_owner\":");
            Self::append_span_band_owner_debug_literal(dump, source.raised_owner);
            let _ = write!(
                dump,
                ",\"start_section_index\":{},\"end_section_index\":{},\"start_s_m\":{:.3},\"end_s_m\":{:.3},\"start_lower_world\":",
                source.start_section_index,
                source.end_section_index,
                source.start_s_m,
                source.end_s_m,
            );
            Self::append_vector3_precise_literal(dump, source.start_lower_world);
            dump.push_str(",\"start_raised_world\":");
            Self::append_vector3_precise_literal(dump, source.start_raised_world);
            dump.push_str(",\"end_lower_world\":");
            Self::append_vector3_precise_literal(dump, source.end_lower_world);
            dump.push_str(",\"end_raised_world\":");
            Self::append_vector3_precise_literal(dump, source.end_raised_world);
            dump.push('}');
        }
        dump.push(']');
    }

    pub(in crate::simulation::network::surface::debug) fn append_span_terrain_clip_source_edges_debug_literal(
        dump: &mut String,
        piece: &RoadSurfaceVisualSpanPiece,
    ) {
        dump.push('[');
        let mut first_edge = true;
        for (loop_index, boundary_loop) in piece.terrain_clip_boundary_loops.iter().enumerate() {
            for (edge_index, edge) in boundary_loop.source_edges.iter().enumerate() {
                if !first_edge {
                    dump.push_str(", ");
                }
                first_edge = false;
                let _ = write!(
                    dump,
                    "{{\"loop_index\":{},\"edge_index\":{},\"kind\":\"{:?}\",\"start\":",
                    loop_index, edge_index, edge.kind
                );
                Self::append_vector3_precise_literal(dump, edge.start);
                dump.push_str(",\"end\":");
                Self::append_vector3_precise_literal(dump, edge.end);
                dump.push('}');
            }
        }
        dump.push(']');
    }

    pub(in crate::simulation::network::surface::debug) fn append_earthwork_face_sources_debug_literal(
        dump: &mut String,
        faces: &[RoadSurfaceEarthworkRenderFace],
    ) {
        dump.push('[');
        for (face_index, face) in faces.iter().enumerate() {
            if face_index > 0 {
                dump.push_str(", ");
            }
            let outer_end = face.polygon.points_world.get(2).copied();
            let outer_start = face.polygon.points_world.get(3).copied();
            let _ = write!(
                dump,
                "{{\"face_index\":{},\"kind\":\"{:?}\",\"source\":",
                face_index, face.kind
            );
            Self::append_earthwork_face_source_debug_literal(dump, face.source);
            dump.push_str(",\"inner_start\":");
            Self::append_vector3_precise_literal(dump, face.inner_start);
            dump.push_str(",\"inner_end\":");
            Self::append_vector3_precise_literal(dump, face.inner_end);
            dump.push_str(",\"outer_start\":");
            if let Some(outer_start) = outer_start {
                Self::append_vector3_precise_literal(dump, outer_start);
            } else {
                dump.push_str("null");
            }
            dump.push_str(",\"outer_end\":");
            if let Some(outer_end) = outer_end {
                Self::append_vector3_precise_literal(dump, outer_end);
            } else {
                dump.push_str("null");
            }
            dump.push('}');
        }
        dump.push(']');
    }

    pub(in crate::simulation::network::surface::debug) fn append_earthwork_face_source_debug_literal(
        dump: &mut String,
        source: RoadSurfaceEarthworkFaceSource,
    ) {
        match source {
            RoadSurfaceEarthworkFaceSource::SpanSupportBoundary {
                edge_idx,
                edge_class,
                support_policy,
                owner,
                role,
                start_section_index,
                end_section_index,
                start_s_m,
                end_s_m,
            } => {
                let _ = write!(
                    dump,
                    "{{\"source_kind\":\"span_support_boundary\",\"edge_idx\":{},\"edge_class\":\"{:?}\",\"support_policy\":\"{}\",\"owner\":",
                    edge_idx,
                    edge_class,
                    support_policy.debug_name()
                );
                Self::append_span_band_owner_debug_literal(dump, owner);
                let _ = write!(
                    dump,
                    ",\"role\":\"{}\",\"start_section_index\":{},\"end_section_index\":{},\"start_s_m\":{:.3},\"end_s_m\":{:.3}}}",
                    Self::span_region_role_debug_name(role),
                    start_section_index,
                    end_section_index,
                    start_s_m,
                    end_s_m
                );
            }
            RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
                node_id,
                kind,
                owner_kind,
                owner_index,
                boundary_source,
            } => {
                let _ = write!(
                    dump,
                    "{{\"source_kind\":\"node_footprint_boundary\",\"node_id\":{},\"node_kind\":\"{:?}\",\"owner_kind\":\"{:?}\",\"owner_index\":{},\"boundary_source\":",
                    node_id, kind, owner_kind, owner_index
                );
                Self::append_node_footprint_boundary_segment_source_debug_literal(
                    dump,
                    boundary_source,
                );
                dump.push('}');
            }
            RoadSurfaceEarthworkFaceSource::NodeSameMaterialBoundaryHandoff {
                node_id,
                kind,
                owner_kind,
                owner_index_a,
                owner_index_b,
                boundary_source,
            } => {
                let _ = write!(
                    dump,
                    "{{\"source_kind\":\"node_same_material_boundary_handoff\",\"node_id\":{},\"node_kind\":\"{:?}\",\"owner_kind\":\"{:?}\",\"owner_index_a\":{},\"owner_index_b\":{},\"boundary_source\":",
                    node_id, kind, owner_kind, owner_index_a, owner_index_b
                );
                Self::append_node_footprint_boundary_segment_source_debug_literal(
                    dump,
                    boundary_source,
                );
                dump.push('}');
            }
        }
    }

    pub(in crate::simulation::network::surface::debug) fn append_node_footprint_boundary_segment_source_debug_literal(
        dump: &mut String,
        source: Option<NodeFootprintBoundarySegmentSource>,
    ) {
        let Some(source) = source else {
            dump.push_str("null");
            return;
        };
        dump.push_str("{\"start\":");
        Self::append_node_footprint_boundary_vertex_source_debug_literal(dump, source.start);
        dump.push_str(",\"end\":");
        Self::append_node_footprint_boundary_vertex_source_debug_literal(dump, source.end);
        dump.push('}');
    }

    pub(in crate::simulation::network::surface::debug) fn append_node_footprint_boundary_vertex_source_debug_literal(
        dump: &mut String,
        source: NodeFootprintBoundaryVertexSource,
    ) {
        match source {
            NodeFootprintBoundaryVertexSource::Direct(direct) => {
                let _ = write!(
                    dump,
                    "{{\"source_kind\":\"direct_top_vertex\",\"top_surface_source_index\":{},\"grade_authority_index\":{}}}",
                    direct.top_surface_source_index, direct.grade_authority_index
                );
            }
            NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint { x_key, z_key, y_mm } => {
                let _ = write!(
                    dump,
                    "{{\"source_kind\":\"canonical_boundary_point\",\"x_key\":{},\"z_key\":{},\"y_mm\":{}}}",
                    x_key, z_key, y_mm
                );
            }
            NodeFootprintBoundaryVertexSource::BoundaryInterpolation {
                owning_segment_start,
                owning_segment_end,
                height_mm,
            } => {
                let _ = write!(
                    dump,
                    "{{\"source_kind\":\"boundary_interpolation\",\"height_mm\":{},\"owning_segment_start\":{{\"top_surface_source_index\":{},\"grade_authority_index\":{}}},\"owning_segment_end\":{{\"top_surface_source_index\":{},\"grade_authority_index\":{}}}}}",
                    height_mm,
                    owning_segment_start.top_surface_source_index,
                    owning_segment_start.grade_authority_index,
                    owning_segment_end.top_surface_source_index,
                    owning_segment_end.grade_authority_index
                );
            }
        }
    }

    pub(in crate::simulation::network::surface::debug) fn append_node_boundary_key_debug_literal(
        dump: &mut String,
        point: backend::RoadVec3,
    ) {
        let key = NodeArrangementKey::from_point(backend::road_vec3_xz(point));
        let _ = write!(dump, "[{},{}]", key.x_key(), key.z_key());
    }
}
