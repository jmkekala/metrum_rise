//! Top-level edge geometry debug dump assembly.

use super::*;

#[derive(Clone, Copy, Debug)]
struct RoadCutFillSummary {
    sample_count: usize,
    max_fill_m: f32,
    max_cut_m: f32,
    max_grade: f32,
}

impl RoadCutFillSummary {
    fn empty() -> Self {
        Self {
            sample_count: 0,
            max_fill_m: 0.0,
            max_cut_m: 0.0,
            max_grade: 0.0,
        }
    }

    fn mode(self) -> &'static str {
        if self.max_fill_m <= 0.25 && self.max_cut_m <= 0.25 {
            "near-grade"
        } else if self.max_fill_m > self.max_cut_m * 1.5 {
            "fill-heavy"
        } else if self.max_cut_m > self.max_fill_m * 1.5 {
            "cut-heavy"
        } else {
            "mixed"
        }
    }
}

impl RoadSurfaceSystem {
    /// Builds compact per-road cut/fill debug lines for road geometry dumps.
    pub(crate) fn build_road_cut_fill_debug_lines(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        edge_ids: &[usize],
    ) -> Vec<String> {
        let mut requested_edge_ids = edge_ids.to_vec();
        requested_edge_ids.sort_unstable();
        requested_edge_ids.dedup();

        requested_edge_ids
            .into_iter()
            .filter(|&edge_idx| edge_idx < graph.edge_count() && !graph.edge(edge_idx).deleted)
            .map(|edge_idx| {
                let sections = self
                    .compiled_sections
                    .get(&edge_idx)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                let summary = self.road_cut_fill_summary(terrain, sections);
                format!(
                    "road_cut_fill edge={} samples={} max_fill={:.3} max_cut={:.3} max_grade={:.3} mode={}",
                    edge_idx,
                    summary.sample_count,
                    summary.max_fill_m,
                    summary.max_cut_m,
                    summary.max_grade,
                    summary.mode()
                )
            })
            .collect()
    }

    pub(crate) fn build_edge_geometry_debug_dump(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        edge_ids: &[usize],
    ) -> String {
        let mut requested_edge_ids = edge_ids.to_vec();
        requested_edge_ids.sort_unstable();
        requested_edge_ids.dedup();

        let debug_node_ids = self.debug_node_ids_for_edges(graph, &requested_edge_ids);
        let debug_edge_ids =
            self.debug_edge_ids_for_edges_and_nodes(graph, &requested_edge_ids, &debug_node_ids);

        let mut dump = String::new();
        let _ = writeln!(dump, "ROAD_GEOMETRY_DUMP_BEGIN");
        let _ = writeln!(dump, "{{");
        let _ = writeln!(dump, "  \"requested_edge_ids\": {:?},", requested_edge_ids);
        let _ = writeln!(dump, "  \"edge_ids\": {:?},", debug_edge_ids);
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
        for &edge_idx in &debug_edge_ids {
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

    fn debug_edge_ids_for_edges_and_nodes(
        &self,
        graph: &RegionGraph,
        requested_edge_ids: &[usize],
        debug_node_ids: &[u32],
    ) -> Vec<usize> {
        let mut edge_ids = requested_edge_ids.to_vec();
        for &node_id in debug_node_ids {
            if node_id as usize >= graph.node_adjacency_count() {
                continue;
            }
            for &edge_idx in graph.node_adjacency(node_id) {
                if edge_idx < graph.edge_count() && !graph.edge(edge_idx).deleted {
                    edge_ids.push(edge_idx);
                }
            }
        }
        edge_ids.retain(|&edge_idx| edge_idx < graph.edge_count() && !graph.edge(edge_idx).deleted);
        edge_ids.sort_unstable();
        edge_ids.dedup();
        edge_ids
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
        let failure = (!compiled)
            .then(|| {
                self.visual_node_compile_input(graph, node_id)
                    .and_then(|input| {
                        self.canonical_node_compile_failure_debug_dump(
                            node_id,
                            input.kind,
                            &input.mouths,
                        )
                    })
            })
            .flatten();

        let _ = writeln!(dump, "    {{");
        let _ = writeln!(dump, "      \"node_id\": {node_id},");
        let _ = writeln!(dump, "      \"kind\": \"{:?}\",", kind);
        dump.push_str("      \"incident_edges\": ");
        Self::append_usize_list_literal(dump, &self.debug_incident_edges_for_node(graph, node_id));
        dump.push_str(",\n");
        let _ = writeln!(dump, "      \"compiled\": {compiled},");
        if let Some(failure) = failure {
            let _ = writeln!(
                dump,
                "      \"uses_visible_earthwork\": {},",
                uses_visible_earthwork
            );
            let _ = writeln!(dump, "      \"failure\": {failure}");
        } else {
            let _ = writeln!(
                dump,
                "      \"uses_visible_earthwork\": {}",
                uses_visible_earthwork
            );
        }
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
        dump.push_str("      \"road_cut_fill_summary\": ");
        self.append_road_cut_fill_summary_debug_literal(dump, terrain, sections);
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
            dump.push_str("      \"span_final_top_regions\": ");
            Self::append_span_final_top_regions_debug_literal(dump, piece);
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
            dump.push_str("      \"span_final_top_regions\": [],\n");
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

    fn append_road_cut_fill_summary_debug_literal(
        &self,
        dump: &mut String,
        terrain: &TerrainSystem,
        sections: &[RoadSurfaceSection],
    ) {
        let summary = self.road_cut_fill_summary(terrain, sections);
        let _ = write!(
            dump,
            "{{\"sample_count\":{},\"max_fill_m\":{:.3},\"max_cut_m\":{:.3},\"max_grade\":{:.3},\"mode\":\"{}\"}}",
            summary.sample_count,
            summary.max_fill_m,
            summary.max_cut_m,
            summary.max_grade,
            summary.mode()
        );
    }

    fn road_cut_fill_summary(
        &self,
        terrain: &TerrainSystem,
        sections: &[RoadSurfaceSection],
    ) -> RoadCutFillSummary {
        if sections.is_empty() {
            return RoadCutFillSummary::empty();
        }

        let mut summary = RoadCutFillSummary::empty();
        for section in sections {
            let center = backend::RoadVec3::new(
                section.center_xz.x,
                f64::from(section.center_height_m),
                section.center_xz.y,
            );
            Self::accumulate_road_cut_fill_sample(&mut summary, terrain, center);

            if let (Some(first_band), Some(last_band)) =
                (section.bands.first(), section.bands.last())
            {
                let left_road = self.section_boundary_world_point(
                    section,
                    first_band.lateral_start_m,
                    first_band.height_start_m,
                );
                let right_road = self.section_boundary_world_point(
                    section,
                    last_band.lateral_end_m,
                    last_band.height_end_m,
                );
                Self::accumulate_road_cut_fill_sample(&mut summary, terrain, left_road);
                Self::accumulate_road_cut_fill_sample(&mut summary, terrain, right_road);
            }
        }

        for pair in sections.windows(2) {
            let run = (pair[1].s_m - pair[0].s_m).abs();
            if run <= SAMPLE_EPSILON_M {
                continue;
            }
            let grade = (pair[1].center_height_m - pair[0].center_height_m).abs() / run;
            summary.max_grade = summary.max_grade.max(grade);
        }

        summary
    }

    fn accumulate_road_cut_fill_sample(
        summary: &mut RoadCutFillSummary,
        terrain: &TerrainSystem,
        point: backend::RoadVec3,
    ) {
        let source_y_m =
            terrain.sample_height_world(point.x as f32, point.z as f32) * config::HEIGHT_SCALE;
        let delta_m = point.y as f32 - source_y_m;
        if delta_m >= 0.0 {
            summary.max_fill_m = summary.max_fill_m.max(delta_m);
        } else {
            summary.max_cut_m = summary.max_cut_m.max(-delta_m);
        }
        summary.sample_count += 1;
    }
}
