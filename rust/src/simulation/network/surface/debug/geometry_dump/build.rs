//! Top-level edge geometry debug dump assembly.

use super::*;

impl RoadSurfaceSystem {
    pub(crate) fn build_edge_geometry_debug_dump(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        edge_ids: &[usize],
    ) -> String {
        let mut sorted_edge_ids = edge_ids.to_vec();
        sorted_edge_ids.sort_unstable();
        sorted_edge_ids.dedup();

        let debug_node_ids = self.debug_node_ids_for_edges(graph, &sorted_edge_ids);

        let mut dump = String::new();
        let _ = writeln!(dump, "ROAD_GEOMETRY_DUMP_BEGIN");
        let _ = writeln!(dump, "{{");
        let _ = writeln!(dump, "  \"edge_ids\": {:?},", sorted_edge_ids);
        let _ = writeln!(dump, "  \"node_compile_status\": [");
        let mut first_status = true;
        for &node_id in &debug_node_ids {
            if !first_status {
                let _ = writeln!(dump, ",");
            }
            first_status = false;
            self.append_node_compile_status_debug_dump(&mut dump, graph, terrain, node_id);
        }
        let _ = writeln!(dump);
        let _ = writeln!(dump, "  ],");
        let _ = writeln!(dump, "  \"edges\": [");

        let mut first_edge = true;
        for &edge_idx in &sorted_edge_ids {
            if edge_idx >= graph.edge_count() {
                continue;
            }
            let edge = graph.edge(edge_idx);
            if edge.deleted {
                continue;
            }

            if !first_edge {
                let _ = writeln!(dump, ",");
            }
            first_edge = false;
            self.append_edge_geometry_debug_dump(&mut dump, graph, terrain, edge_idx, edge);
        }

        let _ = writeln!(dump);
        let _ = writeln!(dump, "  ],");
        let _ = writeln!(dump, "  \"nodes\": [");

        let mut first_node = true;
        for node_id in debug_node_ids {
            let Some(piece) = self.compiled_visual_node_pieces.get(&node_id) else {
                continue;
            };

            if !first_node {
                let _ = writeln!(dump, ",");
            }
            first_node = false;
            self.append_node_geometry_debug_dump(&mut dump, graph, terrain, node_id, piece);
        }

        let _ = writeln!(dump);
        let _ = writeln!(dump, "  ]");
        let _ = writeln!(dump, "}}");
        let _ = write!(dump, "ROAD_GEOMETRY_DUMP_END");
        dump
    }

    pub(in crate::simulation::network::surface::debug) fn debug_node_ids_for_edges(
        &self,
        graph: &RegionGraph,
        edge_ids: &[usize],
    ) -> Vec<u32> {
        let mut node_ids = Vec::with_capacity(edge_ids.len().saturating_mul(2));
        for &edge_idx in edge_ids {
            if edge_idx >= graph.edge_count() {
                continue;
            }
            let edge = graph.edge(edge_idx);
            if edge.deleted {
                continue;
            }
            node_ids.push(graph.get_valid_node(edge.start_node));
            node_ids.push(graph.get_valid_node(edge.end_node));
        }
        node_ids.sort_unstable();
        node_ids.dedup();
        node_ids
    }

    pub(in crate::simulation::network::surface::debug) fn append_node_compile_status_debug_dump(
        &self,
        dump: &mut String,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
    ) {
        let valid = graph.get_valid_node(node_id);
        let incidents = self.sorted_incident_surface_edges(graph, valid);
        let kind = self.classify_visual_node_kind(&incidents);
        let compiled_piece = self.compiled_visual_node_pieces.get(&node_id);
        let compiled = compiled_piece.is_some();
        let uses_visible_earthwork =
            compiled && self.node_piece_uses_visible_earthwork(graph, node_id, terrain);

        let _ = writeln!(dump, "    {{");
        let _ = writeln!(dump, "      \"node_id\": {node_id},");
        let _ = writeln!(dump, "      \"kind\": \"{:?}\",", kind);
        dump.push_str("      \"incident_edges\": ");
        Self::append_usize_list_literal(dump, &self.debug_incident_edges_for_node(graph, node_id));
        dump.push_str(",\n");
        let _ = writeln!(dump, "      \"compiled\": {compiled},");
        let _ = writeln!(
            dump,
            "      \"uses_visible_earthwork\": {}",
            uses_visible_earthwork
        );
        let _ = write!(dump, "    }}");
    }

    pub(in crate::simulation::network::surface::debug) fn append_edge_geometry_debug_dump(
        &self,
        dump: &mut String,
        _graph: &RegionGraph,
        terrain: &TerrainSystem,
        edge_idx: usize,
        edge: &Edge,
    ) {
        let sections = self
            .compiled_sections
            .get(&edge_idx)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let surface_chunks = self
            .surface_span_chunks
            .get(&edge_idx)
            .cloned()
            .unwrap_or_default();
        let earthwork_chunks = self
            .earthwork_span_chunks
            .get(&edge_idx)
            .cloned()
            .unwrap_or_default();

        let _ = writeln!(dump, "    {{");
        let _ = writeln!(dump, "      \"edge_idx\": {edge_idx},");
        let _ = writeln!(dump, "      \"start_node\": {},", edge.start_node);
        let _ = writeln!(dump, "      \"end_node\": {},", edge.end_node);
        let _ = writeln!(dump, "      \"class\": \"{:?}\",", edge.class);
        let _ = writeln!(dump, "      \"primary_type\": \"{:?}\",", edge.primary_type);
        let _ = writeln!(dump, "      \"width_m\": {:.3},", edge.width);
        let _ = writeln!(dump, "      \"fwd_lanes\": {},", edge.fwd_lanes);
        let _ = writeln!(dump, "      \"bkw_lanes\": {},", edge.bkw_lanes);
        let _ = writeln!(
            dump,
            "      \"physical_length_m\": {:.3},",
            edge.physical_length
        );
        let _ = writeln!(dump, "      \"start_clip_m\": {:.3},", edge.start_clip);
        let _ = writeln!(dump, "      \"end_clip_m\": {:.3},", edge.end_clip);
        dump.push_str("      \"surface_chunks\": ");
        Self::append_chunk_key_list_literal(dump, &surface_chunks);
        dump.push_str(",\n");
        dump.push_str("      \"earthwork_chunks\": ");
        Self::append_chunk_key_list_literal(dump, &earthwork_chunks);
        dump.push_str(",\n");
        dump.push_str("      \"geometry_world\": ");
        Self::append_vector3_list_literal(dump, &edge.geometry);
        dump.push_str(",\n");
        dump.push_str("      \"geometry_world_precise\": ");
        Self::append_vector3_precise_list_literal(dump, &edge.geometry);
        dump.push_str(",\n");
        dump.push_str("      \"physical_geometry_world\": ");
        Self::append_vector3_list_literal(dump, &edge.physical_geometry);
        dump.push_str(",\n");
        dump.push_str("      \"physical_geometry_world_precise\": ");
        Self::append_vector3_precise_list_literal(dump, &edge.physical_geometry);
        dump.push_str(",\n");
        let _ = writeln!(dump, "      \"sections\": [");

        for (section_index, section) in sections.iter().enumerate() {
            if section_index > 0 {
                let _ = writeln!(dump, ",");
            }
            self.append_section_geometry_debug_dump(dump, terrain, section);
        }

        let _ = writeln!(dump);
        let _ = writeln!(dump, "      ],");
        if let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) {
            dump.push_str("      \"span_ownership\": ");
            Self::append_span_ownership_debug_literal(dump, piece);
            dump.push_str(",\n");
            dump.push_str("      \"span_earthwork_support\": ");
            Self::append_span_earthwork_support_debug_literal(dump, piece);
            dump.push_str(",\n");
            dump.push_str("      \"span_earthwork_face_sources\": ");
            Self::append_earthwork_face_sources_debug_literal(dump, &piece.render_earthwork_faces);
            dump.push_str(",\n");
            dump.push_str("      \"span_raised_step_face_sources\": ");
            Self::append_span_raised_step_sources_debug_literal(dump, piece);
            dump.push_str(",\n");
            dump.push_str("      \"terrain_clip_source_edges\": ");
            Self::append_span_terrain_clip_source_edges_debug_literal(dump, piece);
            dump.push_str(",\n");
            dump.push_str("      \"span_projection_diagnostics\": ");
            Self::append_span_projection_diagnostics_debug_literal(dump, piece);
            dump.push('\n');
        } else {
            dump.push_str("      \"span_ownership\": {\"owned_region_count\":0,\"regions\":[]},\n");
            dump.push_str(
                "      \"span_earthwork_support\": {\"support_region_count\":0,\"regions\":[]},\n",
            );
            dump.push_str("      \"span_earthwork_face_sources\": [],\n");
            dump.push_str("      \"span_raised_step_face_sources\": [],\n");
            dump.push_str("      \"terrain_clip_source_edges\": [],\n");
            dump.push_str(
                "      \"span_projection_diagnostics\": {\"span_piece_compiled\":false}\n",
            );
        }
        let _ = write!(dump, "    }}");
    }
}
