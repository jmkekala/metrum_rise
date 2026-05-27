//! Node ownership, height, and top-surface debug dump formatting.

use super::*;

impl RoadSurfaceSystem {
    pub(super) fn append_node_geometry_debug_dump(
        &self,
        dump: &mut String,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
        piece: &RoadSurfaceVisualNodePiece,
    ) {
        let surface_chunks = self
            .surface_node_chunks
            .get(&node_id)
            .cloned()
            .unwrap_or_default();
        let earthwork_chunks = self
            .earthwork_node_chunks
            .get(&node_id)
            .cloned()
            .unwrap_or_default();
        let incident_edges = self.debug_incident_edges_for_node(graph, node_id);
        let uses_visible_earthwork =
            self.node_piece_uses_visible_earthwork(graph, node_id, terrain);

        let _ = writeln!(dump, "    {{");
        let _ = writeln!(dump, "      \"node_id\": {node_id},");
        let _ = writeln!(dump, "      \"kind\": \"{:?}\",", piece.kind);
        dump.push_str("      \"incident_edges\": ");
        Self::append_usize_list_literal(dump, &incident_edges);
        dump.push_str(",\n");
        dump.push_str("      \"surface_chunks\": ");
        Self::append_chunk_key_list_literal(dump, &surface_chunks);
        dump.push_str(",\n");
        dump.push_str("      \"earthwork_chunks\": ");
        Self::append_chunk_key_list_literal(dump, &earthwork_chunks);
        dump.push_str(",\n");
        let _ = writeln!(
            dump,
            "      \"uses_visible_earthwork\": {},",
            uses_visible_earthwork
        );
        dump.push_str("      \"band_ownership\": ");
        Self::append_node_band_ownership_debug_literal(dump, piece);
        dump.push_str(",\n");
        dump.push_str("      \"height_owner\": ");
        Self::append_node_height_owner_debug_literal(dump, piece);
        dump.push_str(",\n");
        dump.push_str("      \"node_grade_authority\": ");
        Self::append_node_grade_authority_debug_literal(dump, piece);
        dump.push_str(",\n");
        dump.push_str("      \"node_top_surface_provenance\": ");
        Self::append_node_top_surface_provenance_debug_literal(dump, piece);
        dump.push_str(",\n");
        dump.push_str("      \"seam_constraints\": ");
        self.append_node_seam_constraints_debug_literal(dump, graph, node_id);
        dump.push_str(",\n");
        dump.push_str("      \"road_topology\": ");
        Self::append_polygon_collection_debug_literal(dump, terrain, &piece.road_surface_polygons);
        dump.push_str(",\n");
        dump.push_str("      \"curb_topology\": ");
        Self::append_polygon_collection_debug_literal(dump, terrain, &piece.curb_surface_polygons);
        dump.push_str(",\n");
        dump.push_str("      \"raised_step_face_topology\": ");
        Self::append_polygon_collection_debug_literal(
            dump,
            terrain,
            &piece.raised_step_face_polygons,
        );
        dump.push_str(",\n");
        dump.push_str("      \"raised_step_face_details\": ");
        Self::append_raised_step_face_details_debug_literal(dump, piece);
        dump.push_str(",\n");
        dump.push_str("      \"sidewalk_topology\": ");
        Self::append_polygon_collection_debug_literal(
            dump,
            terrain,
            &piece.sidewalk_surface_polygons,
        );
        dump.push_str(",\n");
        dump.push_str("      \"material_footprint_coverage\": ");
        Self::append_material_footprint_coverage_debug_literal(dump, piece);
        dump.push_str(",\n");
        dump.push_str("      \"outer_boundary_top_match\": ");
        self.append_outer_boundary_top_match_debug_literal(dump, terrain, piece);
        dump.push_str(",\n");
        dump.push_str("      \"mouth_seams\": ");
        self.append_mouth_seam_debug_literal(dump, graph, node_id, piece);
        dump.push_str(",\n");
        dump.push_str("      \"earthwork_face_sources\": ");
        Self::append_earthwork_face_sources_debug_literal(dump, &piece.render_earthwork_faces);
        dump.push_str(",\n");
        dump.push_str("      \"earthwork_face_top_match\": ");
        self.append_earthwork_face_top_match_debug_literal(dump, terrain, piece);
        dump.push('\n');
        let _ = write!(dump, "    }}");
    }

    pub(super) fn append_node_band_ownership_debug_literal(
        dump: &mut String,
        piece: &RoadSurfaceVisualNodePiece,
    ) {
        dump.push('{');
        let _ = write!(dump, "\"owned_region_count\":{}", piece.owned_regions.len());
        for kind in Self::debug_band_kind_order() {
            let count = piece
                .owned_regions
                .iter()
                .filter(|region| region.kind == kind)
                .count();
            let _ = write!(dump, ",\"{:?}\":{}", kind, count);
        }
        dump.push('}');
    }

    pub(super) fn append_node_height_owner_debug_literal(
        dump: &mut String,
        piece: &RoadSurfaceVisualNodePiece,
    ) {
        dump.push('[');
        for (region_index, region) in piece.owned_regions.iter().enumerate() {
            if region_index > 0 {
                dump.push_str(", ");
            }
            let mut y_min = f32::INFINITY;
            let mut y_max = f32::NEG_INFINITY;
            for point in region.polygon.points_world.iter().copied().chain(
                region
                    .polygon
                    .triangles_world
                    .iter()
                    .flat_map(|triangle| triangle.iter().copied()),
            ) {
                y_min = y_min.min(point.y as f32);
                y_max = y_max.max(point.y as f32);
            }
            let _ = write!(
                dump,
                "{{\"region\":{},\"kind\":\"{:?}\",\"owner_index\":{},\"polygon_vertex_count\":{},\"triangle_count\":{},\"y_min_m\":{:.3},\"y_max_m\":{:.3}}}",
                region_index,
                region.kind,
                region.owner_index,
                region.polygon.points_world.len(),
                region.polygon.triangles_world.len(),
                y_min,
                y_max
            );
        }
        dump.push(']');
    }

    pub(super) fn append_node_grade_authority_debug_literal(
        dump: &mut String,
        piece: &RoadSurfaceVisualNodePiece,
    ) {
        dump.push('[');
        for (index, authority) in piece.node_grade_authorities.iter().enumerate() {
            if index > 0 {
                dump.push_str(", ");
            }
            Self::append_node_grade_authority_record_debug_literal(dump, *authority);
        }
        dump.push(']');
    }

    pub(super) fn append_node_grade_authority_record_debug_literal(
        dump: &mut String,
        authority: NodeGradeVertexAuthority,
    ) {
        let _ = write!(
            dump,
            "{{\"x_key\":{},\"z_key\":{},\"x_mm\":{},\"z_mm\":{},\"owner_kind\":\"{:?}\",\"owner_index\":{},\"height_field_id\":\"{:?}\",\"height_mm\":{},\"decision\":\"{}\"",
            authority.key.x_key(),
            authority.key.z_key(),
            authority.key.x_mm(),
            authority.key.z_mm(),
            authority.owner.kind(),
            authority.owner.owner_index(),
            authority.height_field_id,
            authority.height_key.as_i64(),
            Self::node_grade_decision_debug_name(authority.decision),
        );
        if let NodeGradeCarrierDecision::SourceCarrier { authority } = authority.decision {
            dump.push_str(",\"source_authority\":");
            Self::append_node_height_authority_debug_literal(dump, authority);
        }
        dump.push('}');
    }

    pub(super) fn append_node_height_authority_debug_literal(
        dump: &mut String,
        authority: Option<NodeHeightAuthoritySource>,
    ) {
        if let Some(authority) = authority {
            let _ = write!(dump, "\"{:?}\"", authority);
        } else {
            dump.push_str("null");
        }
    }

    pub(super) fn node_grade_decision_debug_name(
        decision: NodeGradeCarrierDecision,
    ) -> &'static str {
        match decision {
            NodeGradeCarrierDecision::SourceCarrier { .. } => "source_carrier",
            NodeGradeCarrierDecision::SameOwnerCanonicalVertex => "same_owner_canonical_vertex",
            NodeGradeCarrierDecision::SameMaterialSharedEdge => "same_material_shared_edge",
            NodeGradeCarrierDecision::SameMaterialVertex => "same_material_vertex",
            NodeGradeCarrierDecision::SameMaterialSeam => "same_material_seam",
            NodeGradeCarrierDecision::ExplicitMaterialSeam => "explicit_material_seam",
        }
    }

    pub(super) fn append_node_top_surface_provenance_debug_literal(
        dump: &mut String,
        piece: &RoadSurfaceVisualNodePiece,
    ) {
        dump.push('[');
        for (source_index, source) in piece.node_top_surface_sources.iter().enumerate() {
            if source_index > 0 {
                dump.push_str(", ");
            }
            let missing_source_count = Self::node_top_surface_missing_source_count(
                source,
                piece.node_grade_authorities.len(),
            );
            let _ = write!(
                dump,
                "{{\"region\":{},\"kind\":\"{:?}\",\"owner_index\":{},\"height_field_id\":\"{:?}\",\"polygon_vertex_count\":{},\"triangle_count\":{},\"missing_source_count\":{},\"grade_authority_indices\":",
                source_index,
                source.kind,
                source.owner_index,
                source.height_field_id,
                source.vertex_sources.len(),
                source.triangle_sources.len(),
                missing_source_count,
            );
            Self::append_node_top_surface_source_indices_debug_literal(dump, source);
            dump.push('}');
        }
        dump.push(']');
    }

    pub(super) fn append_node_top_surface_source_indices_debug_literal(
        dump: &mut String,
        source: &NodeTopSurfacePolygonSource,
    ) {
        let mut indices = BTreeSet::new();
        indices.extend(
            source
                .vertex_sources
                .iter()
                .map(|source| source.grade_authority_index),
        );
        indices.extend(
            source
                .triangle_sources
                .iter()
                .flat_map(|triangle| triangle.iter().map(|source| source.grade_authority_index)),
        );
        dump.push('[');
        for (index, grade_authority_index) in indices.into_iter().enumerate() {
            if index > 0 {
                dump.push(',');
            }
            let _ = write!(dump, "{grade_authority_index}");
        }
        dump.push(']');
    }

    pub(super) fn node_top_surface_missing_source_count(
        source: &NodeTopSurfacePolygonSource,
        authority_count: usize,
    ) -> usize {
        source
            .vertex_sources
            .iter()
            .map(|source| source.grade_authority_index)
            .chain(
                source.triangle_sources.iter().flat_map(|triangle| {
                    triangle.iter().map(|source| source.grade_authority_index)
                }),
            )
            .filter(|index| *index >= authority_count)
            .count()
    }

    pub(super) fn append_node_seam_constraints_debug_literal(
        &self,
        dump: &mut String,
        graph: &RegionGraph,
        node_id: u32,
    ) {
        let mut mouth_count = 0usize;
        let mut span_handoff_vertices = 0usize;
        let mut material_seam_vertices = 0usize;
        let mut outer_footprint_vertices = 0usize;
        for edge_idx in self.debug_incident_edges_for_node(graph, node_id) {
            if edge_idx >= graph.edge_count() {
                continue;
            }
            let edge = graph.edge(edge_idx);
            if edge.deleted {
                continue;
            }
            let start_node = graph.get_valid_node(edge.start_node);
            let side = if start_node == node_id {
                IncidentEdgeSide::Start
            } else {
                IncidentEdgeSide::End
            };
            let Some(span_piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                continue;
            };
            let mouth = match side {
                IncidentEdgeSide::Start => span_piece.start_mouth_profile.as_ref(),
                IncidentEdgeSide::End => span_piece.end_mouth_profile.as_ref(),
            };
            let Some(mouth) = mouth else {
                continue;
            };
            mouth_count += 1;
            for point_index in 0..mouth.boundary_points_world.len() {
                if Self::mouth_boundary_point_is_outer_footprint(mouth, point_index) {
                    outer_footprint_vertices += 1;
                    span_handoff_vertices += 1;
                } else if Self::mouth_boundary_point_is_material_seam(mouth, point_index) {
                    material_seam_vertices += 1;
                    span_handoff_vertices += 1;
                }
            }
        }
        dump.push('{');
        let _ = write!(
            dump,
            "\"mouth_count\":{},\"span_handoff_vertices\":{},\"material_seam_vertices\":{},\"outer_footprint_vertices\":{}",
            mouth_count, span_handoff_vertices, material_seam_vertices, outer_footprint_vertices
        );
        dump.push('}');
    }

    pub(super) fn debug_incident_edges_for_node(
        &self,
        graph: &RegionGraph,
        node_id: u32,
    ) -> Vec<usize> {
        if node_id as usize >= graph.node_adjacency_count() {
            return Vec::new();
        }
        let mut incident_edges = graph.node_adjacency(node_id).to_vec();
        incident_edges.sort_unstable();
        incident_edges.dedup();
        incident_edges
    }

    pub(super) fn append_polygon_collection_debug_literal(
        dump: &mut String,
        terrain: &TerrainSystem,
        polygons: &[RoadSurfaceVisualPolygon],
    ) {
        let polygon_count = polygons.len();
        let triangle_count: usize = polygons
            .iter()
            .map(|polygon| polygon.triangles_world.len())
            .sum();
        let vertex_count: usize = polygons
            .iter()
            .map(|polygon| polygon.points_world.len())
            .sum();

        let mut y_min = f32::INFINITY;
        let mut y_max = f32::NEG_INFINITY;
        let mut source_delta_min = f32::INFINITY;
        let mut source_delta_max = f32::NEG_INFINITY;
        let mut visual_delta_min = f32::INFINITY;
        let mut visual_delta_max = f32::NEG_INFINITY;
        for point in polygons
            .iter()
            .flat_map(|polygon| polygon.points_world.iter().copied())
        {
            let point_y = point.y as f32;
            y_min = y_min.min(point_y);
            y_max = y_max.max(point_y);
            let source_y_m =
                terrain.sample_height_world(point.x as f32, point.z as f32) * config::HEIGHT_SCALE;
            let visual_y_m = terrain.sample_visual_height_world(point.x as f32, point.z as f32)
                * config::HEIGHT_SCALE;
            source_delta_min = source_delta_min.min(point_y - source_y_m);
            source_delta_max = source_delta_max.max(point_y - source_y_m);
            visual_delta_min = visual_delta_min.min(point_y - visual_y_m);
            visual_delta_max = visual_delta_max.max(point_y - visual_y_m);
        }

        let mut max_triangle_y_delta_m = 0.0_f32;
        let mut max_triangle_slope_ratio = 0.0_f32;
        for triangle in polygons
            .iter()
            .flat_map(|polygon| polygon.triangles_world.iter().copied())
        {
            for edge_index in 0..3 {
                let start = triangle[edge_index];
                let end = triangle[(edge_index + 1) % 3];
                let y_delta = (end.y - start.y).abs() as f32;
                max_triangle_y_delta_m = max_triangle_y_delta_m.max(y_delta);
                let xz_distance =
                    backend::road_vec3_xz(end).distance(backend::road_vec3_xz(start)) as f32;
                if xz_distance > SAMPLE_EPSILON_M {
                    max_triangle_slope_ratio = max_triangle_slope_ratio.max(y_delta / xz_distance);
                }
            }
        }

        dump.push('{');
        let _ = write!(
            dump,
            "\"polygon_count\":{},\"triangle_count\":{},\"vertex_count\":{},",
            polygon_count, triangle_count, vertex_count
        );
        dump.push_str("\"y_min_m\":");
        Self::append_optional_f32_literal(dump, (vertex_count > 0).then_some(y_min));
        dump.push_str(",\"y_max_m\":");
        Self::append_optional_f32_literal(dump, (vertex_count > 0).then_some(y_max));
        dump.push_str(",\"source_delta_min_m\":");
        Self::append_optional_f32_literal(dump, (vertex_count > 0).then_some(source_delta_min));
        dump.push_str(",\"source_delta_max_m\":");
        Self::append_optional_f32_literal(dump, (vertex_count > 0).then_some(source_delta_max));
        dump.push_str(",\"visual_delta_min_m\":");
        Self::append_optional_f32_literal(dump, (vertex_count > 0).then_some(visual_delta_min));
        dump.push_str(",\"visual_delta_max_m\":");
        Self::append_optional_f32_literal(dump, (vertex_count > 0).then_some(visual_delta_max));
        let _ = write!(
            dump,
            ",\"max_triangle_y_delta_m\":{:.3},\"max_triangle_slope_ratio\":{:.3}",
            max_triangle_y_delta_m, max_triangle_slope_ratio
        );
        dump.push('}');
    }
}
