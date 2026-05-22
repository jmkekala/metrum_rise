//! Edge, span, and source-provenance geometry dump formatting.

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

    pub(super) fn debug_node_ids_for_edges(
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

    pub(super) fn append_node_compile_status_debug_dump(
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

    pub(super) fn append_edge_geometry_debug_dump(
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
        dump.push_str("      \"physical_geometry_world\": ");
        Self::append_vector3_list_literal(dump, &edge.physical_geometry);
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

    pub(super) fn append_section_geometry_debug_dump(
        &self,
        dump: &mut String,
        terrain: &TerrainSystem,
        section: &RoadSurfaceSection,
    ) {
        let center_world = Vector3::new(
            section.center_xz.x,
            section.center_height_m,
            section.center_xz.y,
        );
        let source_center_y_m = terrain
            .sample_height_world(section.center_xz.x, section.center_xz.y)
            * config::HEIGHT_SCALE;
        let visual_center_y_m = terrain
            .sample_visual_height_world(section.center_xz.x, section.center_xz.y)
            * config::HEIGHT_SCALE;

        let _ = writeln!(dump, "        {{");
        let _ = writeln!(dump, "          \"s_m\": {:.3},", section.s_m);
        dump.push_str("          \"center_world\": ");
        Self::append_vector3_literal(dump, center_world);
        dump.push_str(",\n");
        dump.push_str("          \"tangent_xz\": ");
        Self::append_vector2_literal(dump, section.tangent_xz);
        dump.push_str(",\n");
        dump.push_str("          \"lateral_xz\": ");
        Self::append_vector2_literal(dump, section.lateral_xz);
        dump.push_str(",\n");
        let _ = writeln!(
            dump,
            "          \"source_center_y_m\": {:.3},",
            source_center_y_m
        );
        let _ = writeln!(
            dump,
            "          \"visual_center_y_m\": {:.3},",
            visual_center_y_m
        );

        if let (Some(first_band), Some(last_band)) = (section.bands.first(), section.bands.last()) {
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
            dump.push_str("          \"left_road_edge\": ");
            Self::append_surface_sample_literal(dump, terrain, left_road);
            dump.push_str(",\n");
            dump.push_str("          \"right_road_edge\": ");
            Self::append_surface_sample_literal(dump, terrain, right_road);
            dump.push_str(",\n");
            if let (Some(left_outer), Some(right_outer)) = (
                self.earthwork_transition_point(left_road, section.lateral_xz * -1.0, terrain),
                self.earthwork_transition_point(right_road, section.lateral_xz, terrain),
            ) {
                dump.push_str("          \"left_outer_margin\": ");
                Self::append_surface_sample_literal(dump, terrain, left_outer);
                dump.push_str(",\n");
                dump.push_str("          \"right_outer_margin\": ");
                Self::append_surface_sample_literal(dump, terrain, right_outer);
                dump.push_str(",\n");
            }
        }

        let _ = writeln!(dump, "          \"bands\": [");
        for (band_index, band) in section.bands.iter().enumerate() {
            if band_index > 0 {
                let _ = writeln!(dump, ",");
            }
            let _ = write!(
                dump,
                "            {{\"kind\":\"{:?}\",\"lateral_start_m\":{:.3},\"lateral_end_m\":{:.3},\"height_start_m\":{:.3},\"height_end_m\":{:.3}}}",
                band.kind,
                band.lateral_start_m,
                band.lateral_end_m,
                band.height_start_m,
                band.height_end_m
            );
        }
        let _ = writeln!(dump);
        let _ = writeln!(dump, "          ]");
        let _ = write!(dump, "        }}");
    }

    pub(super) fn append_span_ownership_debug_literal(
        dump: &mut String,
        piece: &RoadSurfaceVisualSpanPiece,
    ) {
        dump.push('{');
        let _ = write!(
            dump,
            "\"owned_region_count\":{}",
            piece.span_owned_regions.len()
        );
        for role in [
            RoadSurfaceSpanRegionRole::Asphalt,
            RoadSurfaceSpanRegionRole::CurbOrShoulder,
            RoadSurfaceSpanRegionRole::NonRoad,
        ] {
            let count = piece
                .span_owned_regions
                .iter()
                .filter(|region| region.role == role)
                .count();
            let _ = write!(
                dump,
                ",\"{}\":{}",
                Self::span_region_role_debug_name(role),
                count
            );
        }
        for kind in Self::debug_band_kind_order() {
            let count = piece
                .span_owned_regions
                .iter()
                .filter(|region| region.owner.kind == kind)
                .count();
            let _ = write!(dump, ",\"band_{:?}\":{}", kind, count);
        }
        dump.push_str(",\"regions\":[");
        for (region_index, region) in piece.span_owned_regions.iter().enumerate() {
            if region_index > 0 {
                dump.push_str(", ");
            }
            let _ = write!(
                dump,
                "{{\"edge_idx\":{},\"role\":\"{}\",\"source_band_index\":{},\"band_kind\":\"{:?}\",\"start_section_index\":{},\"end_section_index\":{},\"start_s_m\":{:.3},\"end_s_m\":{:.3},\"point_count\":{},\"height_min_m\":",
                region.edge_idx,
                Self::span_region_role_debug_name(region.role),
                region.owner.source_band_index,
                region.owner.kind,
                region.start_section_index,
                region.end_section_index,
                region.start_s_m,
                region.end_s_m,
                region.polygon.points_world.len(),
            );
            Self::append_optional_f32_precise_literal(
                dump,
                Self::debug_polygon_height_range(&region.polygon).map(|(min_y, _)| min_y),
            );
            dump.push_str(",\"height_max_m\":");
            Self::append_optional_f32_precise_literal(
                dump,
                Self::debug_polygon_height_range(&region.polygon).map(|(_, max_y)| max_y),
            );
            dump.push('}');
        }
        dump.push_str("]}");
    }

    pub(super) fn append_span_earthwork_support_debug_literal(
        dump: &mut String,
        piece: &RoadSurfaceVisualSpanPiece,
    ) {
        dump.push('{');
        let _ = write!(
            dump,
            "\"support_region_count\":{},\"edge_class\":\"{:?}\",\"support_policy\":\"{}\"",
            piece.span_earthwork_support_regions.len(),
            piece.edge_class,
            RoadSurfaceEarthworkSupportPolicy::from_edge_class(piece.edge_class).debug_name()
        );
        for role in [
            RoadSurfaceSpanRegionRole::Asphalt,
            RoadSurfaceSpanRegionRole::CurbOrShoulder,
            RoadSurfaceSpanRegionRole::NonRoad,
        ] {
            let count = piece
                .span_earthwork_support_regions
                .iter()
                .filter(|region| region.role == role)
                .count();
            let _ = write!(
                dump,
                ",\"{}\":{}",
                Self::span_region_role_debug_name(role),
                count
            );
        }
        for kind in Self::debug_band_kind_order() {
            let count = piece
                .span_earthwork_support_regions
                .iter()
                .filter(|region| region.owner.kind == kind)
                .count();
            let _ = write!(dump, ",\"band_{:?}\":{}", kind, count);
        }
        dump.push_str(",\"regions\":[");
        for (region_index, region) in piece.span_earthwork_support_regions.iter().enumerate() {
            if region_index > 0 {
                dump.push_str(", ");
            }
            let _ = write!(
                dump,
                "{{\"edge_idx\":{},\"role\":\"{}\",\"source_band_index\":{},\"band_kind\":\"{:?}\",\"start_section_index\":{},\"end_section_index\":{},\"start_s_m\":{:.3},\"end_s_m\":{:.3},\"point_count\":{},\"height_min_m\":",
                region.edge_idx,
                Self::span_region_role_debug_name(region.role),
                region.owner.source_band_index,
                region.owner.kind,
                region.start_section_index,
                region.end_section_index,
                region.start_s_m,
                region.end_s_m,
                region.polygon.points_world.len(),
            );
            Self::append_optional_f32_precise_literal(
                dump,
                Self::debug_polygon_height_range(&region.polygon).map(|(min_y, _)| min_y),
            );
            dump.push_str(",\"height_max_m\":");
            Self::append_optional_f32_precise_literal(
                dump,
                Self::debug_polygon_height_range(&region.polygon).map(|(_, max_y)| max_y),
            );
            dump.push('}');
        }
        dump.push_str("]}");
    }

    pub(super) fn append_span_raised_step_sources_debug_literal(
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

    pub(super) fn append_span_terrain_clip_source_edges_debug_literal(
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

    pub(super) fn append_earthwork_face_sources_debug_literal(
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

    pub(super) fn append_earthwork_face_source_debug_literal(
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
        }
    }

    pub(super) fn append_node_footprint_boundary_segment_source_debug_literal(
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

    pub(super) fn append_node_footprint_boundary_vertex_source_debug_literal(
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

    pub(super) fn append_node_boundary_key_debug_literal(dump: &mut String, point: Vector3) {
        let key = NodeArrangementKey::from_point(backend::godot_vec3_xz_to_road(point));
        let _ = write!(dump, "[{},{}]", key.x_key(), key.z_key());
    }

    pub(super) fn append_span_projection_diagnostics_debug_literal(
        dump: &mut String,
        piece: &RoadSurfaceVisualSpanPiece,
    ) {
        let road_projection_matches = Self::span_region_projection_matches_from_regions(
            &piece.span_owned_regions,
            RoadSurfaceSpanRegionRole::Asphalt,
            &piece.road_surface_polygons,
        );
        let curb_projection_matches = Self::span_region_projection_matches_from_regions(
            &piece.span_owned_regions,
            RoadSurfaceSpanRegionRole::CurbOrShoulder,
            &piece.curb_surface_polygons,
        );
        let sidewalk_projection_matches = Self::span_region_projection_matches_from_regions(
            &piece.span_owned_regions,
            RoadSurfaceSpanRegionRole::NonRoad,
            &piece.sidewalk_surface_polygons,
        );
        let raised_step_source_count_matches =
            piece.raised_step_face_polygons.len() == piece.span_raised_step_sources.len();
        let sourced_earthwork_face_count = piece.render_earthwork_faces.len();
        let _ = write!(
            dump,
            "{{\"span_piece_compiled\":true,\"road_projection_matches\":{},\"curb_projection_matches\":{},\"sidewalk_projection_matches\":{},\"raised_step_source_count_matches\":{},\"terrain_clip_loop_count\":{},\"terrain_clip_source_edge_count\":{},\"earthwork_support_region_count\":{},\"sourced_earthwork_face_count\":{},\"missing_earthwork_face_source_count\":0}}",
            road_projection_matches,
            curb_projection_matches,
            sidewalk_projection_matches,
            raised_step_source_count_matches,
            piece.terrain_clip_boundary_loops.len(),
            piece
                .terrain_clip_boundary_loops
                .iter()
                .map(|boundary_loop| boundary_loop.source_edges.len())
                .sum::<usize>(),
            piece.span_earthwork_support_regions.len(),
            sourced_earthwork_face_count
        );
    }

    pub(super) fn append_span_band_owner_debug_literal(
        dump: &mut String,
        owner: RoadSurfaceSpanBandOwner,
    ) {
        let _ = write!(
            dump,
            "{{\"source_band_index\":{},\"kind\":\"{:?}\"}}",
            owner.source_band_index, owner.kind
        );
    }

    pub(super) fn span_region_projection_matches_from_regions(
        regions: &[RoadSurfaceSpanOwnedRegion],
        role: RoadSurfaceSpanRegionRole,
        projected: &[RoadSurfaceVisualPolygon],
    ) -> bool {
        let mut expected: Vec<RoadSurfaceVisualPolygon> = regions
            .iter()
            .filter(|region| region.role == role)
            .map(|region| region.polygon.clone())
            .collect();
        let mut actual = projected.to_vec();
        Self::sort_visual_polygons(&mut expected);
        Self::sort_visual_polygons(&mut actual);
        expected == actual
    }

    pub(super) fn debug_polygon_height_range(
        polygon: &RoadSurfaceVisualPolygon,
    ) -> Option<(f32, f32)> {
        let mut min_y = f32::INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for point in &polygon.points_world {
            min_y = min_y.min(point.y);
            max_y = max_y.max(point.y);
        }
        min_y.is_finite().then_some((min_y, max_y))
    }

    pub(super) fn span_region_role_debug_name(role: RoadSurfaceSpanRegionRole) -> &'static str {
        match role {
            RoadSurfaceSpanRegionRole::Asphalt => "asphalt",
            RoadSurfaceSpanRegionRole::CurbOrShoulder => "curb_or_shoulder",
            RoadSurfaceSpanRegionRole::NonRoad => "non_road",
        }
    }
}
