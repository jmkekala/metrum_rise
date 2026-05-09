//! Debug extraction helpers for compiled road-surface state.

use super::{
    IncidentEdgeSide, IncidentMouthProfile, NodeOverlayContour, NodeOverlayPoint, NodeOverlayShape,
    NodeOverlayShapes, RoadSurfaceBandKind, RoadSurfaceDebugData, RoadSurfaceEarthworkFaceKind,
    RoadSurfaceSection, RoadSurfaceSystem, RoadSurfaceVisualNodePiece, RoadSurfaceVisualPolygon,
    SAMPLE_EPSILON_M, SurfaceChunkKey, backend,
};
use crate::config;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::{Vector2, Vector3};
use i_overlay::core::overlay_rule::OverlayRule;
use std::fmt::Write as _;

const DEBUG_MAX_PROBLEM_SAMPLES: usize = 12;
const DEBUG_VERTEX_MATCH_TOLERANCE_M: f32 = 0.004;
const DEBUG_VERTEX_NEAR_TOLERANCE_M: f32 = 0.002;

#[derive(Clone, Copy)]
struct DebugTopVertex {
    material: &'static str,
    point: Vector3,
}

#[derive(Clone, Copy)]
struct DebugClosestTopVertex {
    material: &'static str,
    point: Vector3,
    xz_error_m: f32,
    y_delta_m: f32,
}

#[derive(Default)]
struct DebugMatchStats {
    total: usize,
    problem_count: usize,
    max_xz_error_m: f32,
    max_y_error_m: f32,
}

#[derive(Default)]
struct DebugCoverageStats {
    footprint_area_m2: f32,
    top_area_m2: f32,
    missing_area_m2: f32,
    extra_area_m2: f32,
    area_budget_m2: f32,
    missing_shape_count: usize,
    extra_shape_count: usize,
}

impl RoadSurfaceSystem {
    pub(crate) fn build_debug_line_data(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
    ) -> RoadSurfaceDebugData {
        let mut data = RoadSurfaceDebugData::default();

        let mut edge_indices: Vec<usize> = self.compiled_sections.keys().copied().collect();
        edge_indices.retain(|edge_idx| self.compiled_visual_span_pieces.contains_key(edge_idx));
        edge_indices.sort_unstable();
        for edge_idx in edge_indices {
            if edge_idx >= graph.edge_count() {
                continue;
            }
            let Some(sections) = self.compiled_sections.get(&edge_idx) else {
                continue;
            };
            for section in sections {
                let profile = self.section_profile_world_points(section, 0.18);
                if let (Some(first), Some(last)) = (profile.first(), profile.last()) {
                    data.section_lines.push(*first);
                    data.section_lines.push(*last);
                }
            }

            for pair in sections.windows(2) {
                let profile_a = self.section_profile_world_points(&pair[0], 0.12);
                let profile_b = self.section_profile_world_points(&pair[1], 0.12);
                if profile_a.len() < 2 || profile_a.len() != profile_b.len() {
                    continue;
                }
                for index in 0..profile_a.len() {
                    data.band_lines.push(profile_a[index]);
                    data.band_lines.push(profile_b[index]);
                }
            }

            let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                continue;
            };
            for boundary_loop in &piece.outer_boundary_loops {
                let points: Vec<Vector3> = boundary_loop
                    .points_world
                    .iter()
                    .map(|point| *point + Vector3::UP * 0.22)
                    .collect();
                if points.len() < 2 {
                    continue;
                }
                for index in 0..points.len() {
                    data.piece_boundary_lines.push(points[index]);
                    data.piece_boundary_lines
                        .push(points[(index + 1) % points.len()]);
                }
            }
        }

        let mut node_ids: Vec<u32> = self.compiled_visual_node_pieces.keys().copied().collect();
        node_ids.sort_unstable();
        for node_id in node_ids {
            let Some(piece) = self.compiled_visual_node_pieces.get(&node_id) else {
                continue;
            };
            for boundary_loop in &piece.outer_boundary_loops {
                let points: Vec<Vector3> = boundary_loop
                    .points_world
                    .iter()
                    .map(|point| *point + Vector3::UP * 0.24)
                    .collect();
                if points.len() < 2 {
                    continue;
                }
                for index in 0..points.len() {
                    data.piece_boundary_lines.push(points[index]);
                    data.piece_boundary_lines
                        .push(points[(index + 1) % points.len()]);
                }
            }
        }

        let mut chunks: Vec<SurfaceChunkKey> = self.earthwork_chunk_cache.keys().copied().collect();
        chunks.sort_unstable();
        for chunk in chunks {
            let (min, max) = self.chunk_bounds(chunk);
            let corners = [
                Vector3::new(
                    min.x,
                    terrain.sample_visual_height_world(min.x, min.z) * config::HEIGHT_SCALE + 0.35,
                    min.z,
                ),
                Vector3::new(
                    max.x,
                    terrain.sample_visual_height_world(max.x, min.z) * config::HEIGHT_SCALE + 0.35,
                    min.z,
                ),
                Vector3::new(
                    max.x,
                    terrain.sample_visual_height_world(max.x, max.z) * config::HEIGHT_SCALE + 0.35,
                    max.z,
                ),
                Vector3::new(
                    min.x,
                    terrain.sample_visual_height_world(min.x, max.z) * config::HEIGHT_SCALE + 0.35,
                    max.z,
                ),
            ];
            for index in 0..corners.len() {
                data.earthwork_chunk_lines.push(corners[index]);
                data.earthwork_chunk_lines
                    .push(corners[(index + 1) % corners.len()]);
            }
        }

        data
    }

    pub(crate) fn build_edge_geometry_debug_dump(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        edge_ids: &[usize],
    ) -> String {
        let mut sorted_edge_ids = edge_ids.to_vec();
        sorted_edge_ids.sort_unstable();
        sorted_edge_ids.dedup();

        let mut dump = String::new();
        let _ = writeln!(dump, "ROAD_GEOMETRY_DUMP_BEGIN");
        let _ = writeln!(dump, "{{");
        let _ = writeln!(dump, "  \"edge_ids\": {:?},", sorted_edge_ids);
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
        for node_id in self.debug_node_ids_for_edges(graph, &sorted_edge_ids) {
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

    fn debug_node_ids_for_edges(&self, graph: &RegionGraph, edge_ids: &[usize]) -> Vec<u32> {
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

    fn append_edge_geometry_debug_dump(
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
        let _ = writeln!(dump, "      ]");
        let _ = write!(dump, "    }}");
    }

    fn append_section_geometry_debug_dump(
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
            let left_outer =
                self.earthwork_transition_point(left_road, section.lateral_xz * -1.0, terrain);
            let right_outer =
                self.earthwork_transition_point(right_road, section.lateral_xz, terrain);

            dump.push_str("          \"left_road_edge\": ");
            Self::append_surface_sample_literal(dump, terrain, left_road);
            dump.push_str(",\n");
            dump.push_str("          \"right_road_edge\": ");
            Self::append_surface_sample_literal(dump, terrain, right_road);
            dump.push_str(",\n");
            dump.push_str("          \"left_outer_margin\": ");
            Self::append_surface_sample_literal(dump, terrain, left_outer);
            dump.push_str(",\n");
            dump.push_str("          \"right_outer_margin\": ");
            Self::append_surface_sample_literal(dump, terrain, right_outer);
            dump.push_str(",\n");
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

    fn append_node_geometry_debug_dump(
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
        dump.push_str("      \"seam_constraints\": ");
        self.append_node_seam_constraints_debug_literal(dump, graph, node_id);
        dump.push_str(",\n");
        dump.push_str("      \"road_topology\": ");
        Self::append_polygon_collection_debug_literal(dump, terrain, &piece.road_surface_polygons);
        dump.push_str(",\n");
        dump.push_str("      \"curb_topology\": ");
        Self::append_polygon_collection_debug_literal(dump, terrain, &piece.curb_surface_polygons);
        dump.push_str(",\n");
        dump.push_str("      \"curb_vertical_face_topology\": ");
        Self::append_polygon_collection_debug_literal(
            dump,
            terrain,
            &piece.curb_vertical_face_polygons,
        );
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
        dump.push_str("      \"earthwork_face_top_match\": ");
        self.append_earthwork_face_top_match_debug_literal(dump, terrain, piece);
        dump.push('\n');
        let _ = write!(dump, "    }}");
    }

    fn append_node_band_ownership_debug_literal(
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

    fn append_node_height_owner_debug_literal(
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
                y_min = y_min.min(point.y);
                y_max = y_max.max(point.y);
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

    fn append_node_seam_constraints_debug_literal(
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

    fn debug_incident_edges_for_node(&self, graph: &RegionGraph, node_id: u32) -> Vec<usize> {
        if node_id as usize >= graph.node_adjacency_count() {
            return Vec::new();
        }
        let mut incident_edges = graph.node_adjacency(node_id).to_vec();
        incident_edges.sort_unstable();
        incident_edges.dedup();
        incident_edges
    }

    fn append_polygon_collection_debug_literal(
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
            y_min = y_min.min(point.y);
            y_max = y_max.max(point.y);
            let source_y_m = terrain.sample_height_world(point.x, point.z) * config::HEIGHT_SCALE;
            let visual_y_m =
                terrain.sample_visual_height_world(point.x, point.z) * config::HEIGHT_SCALE;
            source_delta_min = source_delta_min.min(point.y - source_y_m);
            source_delta_max = source_delta_max.max(point.y - source_y_m);
            visual_delta_min = visual_delta_min.min(point.y - visual_y_m);
            visual_delta_max = visual_delta_max.max(point.y - visual_y_m);
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
                let y_delta = (end.y - start.y).abs();
                max_triangle_y_delta_m = max_triangle_y_delta_m.max(y_delta);
                let xz_distance = Vector2::new(end.x - start.x, end.z - start.z).length();
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

    fn append_material_footprint_coverage_debug_literal(
        dump: &mut String,
        piece: &RoadSurfaceVisualNodePiece,
    ) {
        let footprint_contours =
            Self::debug_overlay_contours_from_polygons(&piece.outer_boundary_loops);
        let Some(mut footprint_shapes) = Self::overlay_union_contours(&footprint_contours) else {
            dump.push_str("{\"status\":\"overlay_failed\"}");
            return;
        };
        Self::sort_overlay_shapes(&mut footprint_shapes);

        let top_contours = Self::debug_overlay_contours_from_top_polygons(
            piece
                .road_surface_polygons
                .iter()
                .chain(piece.curb_surface_polygons.iter())
                .chain(piece.sidewalk_surface_polygons.iter()),
        );
        let Some(mut top_shapes) = Self::overlay_union_contours(&top_contours) else {
            dump.push_str("{\"status\":\"overlay_failed\"}");
            return;
        };
        Self::sort_overlay_shapes(&mut top_shapes);

        let Some(mut missing_shapes) =
            Self::overlay_binary_shapes(&footprint_shapes, &top_shapes, OverlayRule::Difference)
        else {
            dump.push_str("{\"status\":\"overlay_failed\"}");
            return;
        };
        Self::sort_overlay_shapes(&mut missing_shapes);

        let Some(mut extra_shapes) =
            Self::overlay_binary_shapes(&top_shapes, &footprint_shapes, OverlayRule::Difference)
        else {
            dump.push_str("{\"status\":\"overlay_failed\"}");
            return;
        };
        Self::sort_overlay_shapes(&mut extra_shapes);

        let stats = DebugCoverageStats {
            footprint_area_m2: Self::debug_overlay_area_m2(&footprint_shapes),
            top_area_m2: Self::debug_overlay_area_m2(&top_shapes),
            missing_area_m2: Self::debug_overlay_area_m2(&missing_shapes),
            extra_area_m2: Self::debug_overlay_area_m2(&extra_shapes),
            area_budget_m2: Self::overlay_numeric_area_budget_for_shapes(&footprint_shapes)
                .max(Self::overlay_numeric_area_budget_for_shapes(&top_shapes)),
            missing_shape_count: missing_shapes.len(),
            extra_shape_count: extra_shapes.len(),
        };

        dump.push('{');
        let problem = stats.missing_area_m2 > stats.area_budget_m2
            || stats.extra_area_m2 > stats.area_budget_m2;
        let _ = write!(
            dump,
            "\"status\":\"ok\",\"problem\":{},\"footprint_area_m2\":{:.6},\"top_area_m2\":{:.6},\"missing_area_m2\":{:.6},\"extra_area_m2\":{:.6},\"area_budget_m2\":{:.6},\"missing_shape_count\":{},\"extra_shape_count\":{}",
            problem,
            stats.footprint_area_m2,
            stats.top_area_m2,
            stats.missing_area_m2,
            stats.extra_area_m2,
            stats.area_budget_m2,
            stats.missing_shape_count,
            stats.extra_shape_count
        );
        dump.push_str(",\"missing_samples\":[");
        Self::append_overlay_shape_samples(dump, &missing_shapes);
        dump.push_str("],\"extra_samples\":[");
        Self::append_overlay_shape_samples(dump, &extra_shapes);
        dump.push_str("]}");
    }

    fn append_outer_boundary_top_match_debug_literal(
        &self,
        dump: &mut String,
        terrain: &TerrainSystem,
        piece: &RoadSurfaceVisualNodePiece,
    ) {
        let top_vertices = Self::debug_top_vertices(piece);
        let mut stats = DebugMatchStats::default();
        let mut samples = Vec::new();
        for (loop_index, boundary_loop) in piece.outer_boundary_loops.iter().enumerate() {
            for (vertex_index, &point) in boundary_loop.points_world.iter().enumerate() {
                let Some(closest) = Self::closest_debug_top_vertex(point, &top_vertices) else {
                    continue;
                };
                Self::update_debug_match_stats(&mut stats, closest);
                if Self::debug_match_is_problem(closest)
                    && samples.len() < DEBUG_MAX_PROBLEM_SAMPLES
                {
                    let source_y_m =
                        terrain.sample_height_world(point.x, point.z) * config::HEIGHT_SCALE;
                    let visual_y_m =
                        terrain.sample_visual_height_world(point.x, point.z) * config::HEIGHT_SCALE;
                    let mut sample = String::new();
                    let _ = write!(
                        sample,
                        "{{\"loop\":{},\"vertex\":{},\"boundary\":",
                        loop_index, vertex_index
                    );
                    Self::append_vector3_literal(&mut sample, point);
                    sample.push_str(",\"closest_material\":\"");
                    sample.push_str(closest.material);
                    sample.push_str("\",\"closest\":");
                    Self::append_vector3_literal(&mut sample, closest.point);
                    let _ = write!(
                        sample,
                        ",\"xz_error_m\":{:.4},\"y_delta_m\":{:.4},\"source_terrain_y_m\":{:.3},\"visual_terrain_y_m\":{:.3}}}",
                        closest.xz_error_m, closest.y_delta_m, source_y_m, visual_y_m
                    );
                    samples.push(sample);
                }
            }
        }
        Self::append_match_stats_with_samples_literal(dump, &stats, &samples);
    }

    fn append_mouth_seam_debug_literal(
        &self,
        dump: &mut String,
        graph: &RegionGraph,
        node_id: u32,
        piece: &RoadSurfaceVisualNodePiece,
    ) {
        let top_vertices = Self::debug_top_vertices(piece);
        dump.push('[');
        let incident_edges = self.debug_incident_edges_for_node(graph, node_id);
        let mut first = true;
        for edge_idx in incident_edges {
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

            if !first {
                dump.push_str(", ");
            }
            first = false;

            let mut stats = DebugMatchStats::default();
            let mut samples = Vec::new();
            for (point_index, &point) in mouth.boundary_points_world.iter().enumerate() {
                if !Self::mouth_boundary_point_requires_top_match(mouth, point_index) {
                    continue;
                }
                let Some(closest) = Self::closest_debug_top_vertex(point, &top_vertices) else {
                    continue;
                };
                Self::update_debug_match_stats(&mut stats, closest);
                if Self::debug_match_is_problem(closest)
                    && samples.len() < DEBUG_MAX_PROBLEM_SAMPLES
                {
                    let mut sample = String::new();
                    let _ = write!(sample, "{{\"point\":{},\"mouth\":", point_index);
                    Self::append_vector3_literal(&mut sample, point);
                    sample.push_str(",\"closest_material\":\"");
                    sample.push_str(closest.material);
                    sample.push_str("\",\"closest\":");
                    Self::append_vector3_literal(&mut sample, closest.point);
                    let _ = write!(
                        sample,
                        ",\"xz_error_m\":{:.4},\"y_delta_m\":{:.4}}}",
                        closest.xz_error_m, closest.y_delta_m
                    );
                    samples.push(sample);
                }
            }

            let _ = write!(
                dump,
                "{{\"edge_idx\":{},\"side\":\"{:?}\",\"mouth_vertex_count\":{},",
                edge_idx,
                side,
                mouth.boundary_points_world.len()
            );
            Self::append_match_stats_fields(dump, &stats);
            dump.push_str(",\"samples\":[");
            Self::append_raw_json_samples(dump, &samples);
            dump.push_str("]}");
        }
        dump.push(']');
    }

    fn append_earthwork_face_top_match_debug_literal(
        &self,
        dump: &mut String,
        terrain: &TerrainSystem,
        piece: &RoadSurfaceVisualNodePiece,
    ) {
        let top_vertices = Self::debug_top_vertices(piece);
        let mut stats = DebugMatchStats::default();
        let mut samples = Vec::new();
        let mut slope_count = 0_usize;
        let mut retaining_wall_count = 0_usize;
        for (face_index, face) in piece.render_earthwork_faces.iter().enumerate() {
            match face.kind {
                RoadSurfaceEarthworkFaceKind::Slope => slope_count += 1,
                RoadSurfaceEarthworkFaceKind::RetainingWall => retaining_wall_count += 1,
            }
            let inner_start = face.inner_start;
            let inner_end = face.inner_end;
            let mut face_problem = false;
            for point in [inner_start, inner_end] {
                let Some(closest) = Self::closest_debug_top_vertex(point, &top_vertices) else {
                    continue;
                };
                Self::update_debug_match_stats(&mut stats, closest);
                face_problem |= Self::debug_match_is_problem(closest);
            }
            if face_problem && samples.len() < DEBUG_MAX_PROBLEM_SAMPLES {
                let outer_end = face.polygon.points_world[2];
                let outer_start = face.polygon.points_world[3];
                let mut sample = String::new();
                let _ = write!(
                    sample,
                    "{{\"face\":{},\"kind\":\"{:?}\",\"inner_start\":",
                    face_index, face.kind
                );
                Self::append_surface_sample_literal(&mut sample, terrain, inner_start);
                sample.push_str(",\"inner_end\":");
                Self::append_surface_sample_literal(&mut sample, terrain, inner_end);
                sample.push_str(",\"outer_end\":");
                Self::append_surface_sample_literal(&mut sample, terrain, outer_end);
                sample.push_str(",\"outer_start\":");
                Self::append_surface_sample_literal(&mut sample, terrain, outer_start);
                sample.push('}');
                samples.push(sample);
            }
        }

        dump.push('{');
        let _ = write!(
            dump,
            "\"face_count\":{},\"slope_count\":{},\"retaining_wall_count\":{},",
            piece.render_earthwork_faces.len(),
            slope_count,
            retaining_wall_count
        );
        Self::append_match_stats_fields(dump, &stats);
        dump.push_str(",\"samples\":[");
        Self::append_raw_json_samples(dump, &samples);
        dump.push_str("]}");
    }

    fn debug_band_kind_order() -> [RoadSurfaceBandKind; 8] {
        [
            RoadSurfaceBandKind::Carriageway,
            RoadSurfaceBandKind::CurbOrShoulder,
            RoadSurfaceBandKind::Sidewalk,
            RoadSurfaceBandKind::Footpath,
            RoadSurfaceBandKind::Median,
            RoadSurfaceBandKind::Parking,
            RoadSurfaceBandKind::CycleTrack,
            RoadSurfaceBandKind::TramReservation,
        ]
    }

    fn mouth_boundary_point_requires_top_match(
        mouth: &IncidentMouthProfile,
        point_index: usize,
    ) -> bool {
        Self::mouth_boundary_point_is_outer_footprint(mouth, point_index)
            || Self::mouth_boundary_point_is_material_seam(mouth, point_index)
    }

    fn mouth_boundary_point_is_outer_footprint(
        mouth: &IncidentMouthProfile,
        point_index: usize,
    ) -> bool {
        point_index == 0 || point_index + 1 == mouth.boundary_points_world.len()
    }

    fn mouth_boundary_point_is_material_seam(
        mouth: &IncidentMouthProfile,
        point_index: usize,
    ) -> bool {
        if point_index == 0 || point_index >= mouth.boundary_points_world.len().saturating_sub(1) {
            return false;
        }
        let Some(before) = mouth.bands.get(point_index - 1) else {
            return false;
        };
        let Some(after) = mouth.bands.get(point_index) else {
            return false;
        };
        before.kind != after.kind
    }

    fn debug_top_vertices(piece: &RoadSurfaceVisualNodePiece) -> Vec<DebugTopVertex> {
        let mut vertices = Vec::new();
        for polygon in &piece.road_surface_polygons {
            vertices.extend(
                polygon
                    .points_world
                    .iter()
                    .copied()
                    .map(|point| DebugTopVertex {
                        material: "road",
                        point,
                    }),
            );
            vertices.extend(polygon.triangles_world.iter().flat_map(|triangle| {
                triangle.iter().copied().map(|point| DebugTopVertex {
                    material: "road",
                    point,
                })
            }));
        }
        for polygon in &piece.curb_surface_polygons {
            vertices.extend(
                polygon
                    .points_world
                    .iter()
                    .copied()
                    .map(|point| DebugTopVertex {
                        material: "curb",
                        point,
                    }),
            );
            vertices.extend(polygon.triangles_world.iter().flat_map(|triangle| {
                triangle.iter().copied().map(|point| DebugTopVertex {
                    material: "curb",
                    point,
                })
            }));
        }
        for polygon in &piece.sidewalk_surface_polygons {
            vertices.extend(
                polygon
                    .points_world
                    .iter()
                    .copied()
                    .map(|point| DebugTopVertex {
                        material: "sidewalk",
                        point,
                    }),
            );
            vertices.extend(polygon.triangles_world.iter().flat_map(|triangle| {
                triangle.iter().copied().map(|point| DebugTopVertex {
                    material: "sidewalk",
                    point,
                })
            }));
        }
        vertices
    }

    fn debug_overlay_contours_from_polygons(
        polygons: &[RoadSurfaceVisualPolygon],
    ) -> Vec<NodeOverlayContour> {
        polygons
            .iter()
            .filter_map(|polygon| {
                Self::debug_overlay_contour_from_world_points(&polygon.points_world)
            })
            .collect()
    }

    fn debug_overlay_contours_from_top_polygons<'a>(
        polygons: impl IntoIterator<Item = &'a RoadSurfaceVisualPolygon>,
    ) -> Vec<NodeOverlayContour> {
        let mut contours = Vec::new();
        for polygon in polygons {
            if polygon.triangles_world.is_empty() {
                if let Some(contour) =
                    Self::debug_overlay_contour_from_world_points(&polygon.points_world)
                {
                    contours.push(contour);
                }
                continue;
            }
            for triangle in &polygon.triangles_world {
                if let Some(contour) = Self::debug_overlay_contour_from_world_points(triangle) {
                    contours.push(contour);
                }
            }
        }
        contours
    }

    fn debug_overlay_contour_from_world_points(points: &[Vector3]) -> Option<NodeOverlayContour> {
        let mut contour = Vec::with_capacity(points.len());
        for point in points {
            let point = backend::road_vec2_to_overlay_point(backend::godot_vec3_xz_to_road(*point));
            if contour.last().is_none_or(|last| *last != point) {
                contour.push(point);
            }
        }
        if contour.len() >= 2 && contour.first() == contour.last() {
            contour.pop();
        }
        (contour.len() >= 3).then_some(contour)
    }

    fn debug_overlay_area_m2(shapes: &NodeOverlayShapes) -> f32 {
        shapes.iter().map(Self::overlay_shape_area_m2).sum()
    }

    fn append_overlay_shape_samples(dump: &mut String, shapes: &[NodeOverlayShape]) {
        for (index, shape) in shapes.iter().take(DEBUG_MAX_PROBLEM_SAMPLES).enumerate() {
            if index > 0 {
                dump.push_str(", ");
            }
            let area_m2 = Self::overlay_shape_area_m2(shape);
            let (centroid_x, centroid_z) = Self::overlay_shape_centroid_xz(shape);
            let (min_x, min_z, max_x, max_z) = Self::overlay_shape_bounds_xz(shape);
            let _ = write!(
                dump,
                "{{\"area_m2\":{:.6},\"centroid\":[{:.3}, {:.3}],\"bounds\":[[{:.3}, {:.3}], [{:.3}, {:.3}]]}}",
                area_m2, centroid_x, centroid_z, min_x, min_z, max_x, max_z
            );
        }
    }

    fn overlay_shape_centroid_xz(shape: &NodeOverlayShape) -> (f64, f64) {
        let mut weighted_x = 0.0;
        let mut weighted_z = 0.0;
        let mut total_weight = 0.0;
        for contour in shape {
            let area = Self::debug_overlay_contour_signed_area_m2(contour);
            let weight = area.abs();
            let (x, z) = Self::overlay_contour_average_xz(contour);
            weighted_x += x * weight;
            weighted_z += z * weight;
            total_weight += weight;
        }
        if total_weight <= f64::EPSILON {
            return Self::overlay_contour_average_xz(
                shape.first().map(Vec::as_slice).unwrap_or(&[]),
            );
        }
        (weighted_x / total_weight, weighted_z / total_weight)
    }

    fn overlay_contour_average_xz(contour: &[NodeOverlayPoint]) -> (f64, f64) {
        if contour.is_empty() {
            return (0.0, 0.0);
        }
        let mut x = 0.0;
        let mut z = 0.0;
        for point in contour {
            x += point[0];
            z += point[1];
        }
        (x / contour.len() as f64, z / contour.len() as f64)
    }

    fn debug_overlay_contour_signed_area_m2(contour: &NodeOverlayContour) -> f64 {
        if contour.len() < 3 {
            return 0.0;
        }
        let mut signed_area = 0.0;
        for index in 0..contour.len() {
            let current = contour[index];
            let next = contour[(index + 1) % contour.len()];
            signed_area += current[0] * next[1] - next[0] * current[1];
        }
        signed_area * 0.5
    }

    fn overlay_shape_bounds_xz(shape: &NodeOverlayShape) -> (f64, f64, f64, f64) {
        let mut min_x = f64::INFINITY;
        let mut min_z = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_z = f64::NEG_INFINITY;
        for point in shape.iter().flat_map(|contour| contour.iter()) {
            min_x = min_x.min(point[0]);
            min_z = min_z.min(point[1]);
            max_x = max_x.max(point[0]);
            max_z = max_z.max(point[1]);
        }
        if !min_x.is_finite() {
            return (0.0, 0.0, 0.0, 0.0);
        }
        (min_x, min_z, max_x, max_z)
    }

    fn closest_debug_top_vertex(
        point: Vector3,
        top_vertices: &[DebugTopVertex],
    ) -> Option<DebugClosestTopVertex> {
        top_vertices
            .iter()
            .map(|vertex| {
                let xz_error_m =
                    Vector2::new(vertex.point.x - point.x, vertex.point.z - point.z).length();
                DebugClosestTopVertex {
                    material: vertex.material,
                    point: vertex.point,
                    xz_error_m,
                    y_delta_m: point.y - vertex.point.y,
                }
            })
            .min_by(|a, b| {
                a.xz_error_m
                    .total_cmp(&b.xz_error_m)
                    .then(a.y_delta_m.abs().total_cmp(&b.y_delta_m.abs()))
            })
    }

    fn update_debug_match_stats(stats: &mut DebugMatchStats, closest: DebugClosestTopVertex) {
        stats.total += 1;
        stats.max_xz_error_m = stats.max_xz_error_m.max(closest.xz_error_m);
        if closest.xz_error_m <= DEBUG_VERTEX_NEAR_TOLERANCE_M {
            stats.max_y_error_m = stats.max_y_error_m.max(closest.y_delta_m.abs());
        }
        if Self::debug_match_is_problem(closest) {
            stats.problem_count += 1;
        }
    }

    fn debug_match_is_problem(closest: DebugClosestTopVertex) -> bool {
        closest.xz_error_m > DEBUG_VERTEX_NEAR_TOLERANCE_M
            || (closest.xz_error_m <= DEBUG_VERTEX_NEAR_TOLERANCE_M
                && closest.y_delta_m.abs() > DEBUG_VERTEX_MATCH_TOLERANCE_M)
    }

    fn append_match_stats_with_samples_literal(
        dump: &mut String,
        stats: &DebugMatchStats,
        samples: &[String],
    ) {
        dump.push('{');
        Self::append_match_stats_fields(dump, stats);
        dump.push_str(",\"samples\":[");
        Self::append_raw_json_samples(dump, samples);
        dump.push_str("]}");
    }

    fn append_match_stats_fields(dump: &mut String, stats: &DebugMatchStats) {
        let _ = write!(
            dump,
            "\"tested_vertices\":{},\"problem_count\":{},\"max_xz_error_m\":{:.4},\"max_y_error_m\":{:.4}",
            stats.total, stats.problem_count, stats.max_xz_error_m, stats.max_y_error_m
        );
    }

    fn append_raw_json_samples(dump: &mut String, samples: &[String]) {
        for (index, sample) in samples.iter().enumerate() {
            if index > 0 {
                dump.push_str(", ");
            }
            dump.push_str(sample);
        }
    }

    fn append_surface_sample_literal(dump: &mut String, terrain: &TerrainSystem, point: Vector3) {
        let source_y_m = terrain.sample_height_world(point.x, point.z) * config::HEIGHT_SCALE;
        let visual_y_m =
            terrain.sample_visual_height_world(point.x, point.z) * config::HEIGHT_SCALE;
        dump.push('{');
        dump.push_str("\"world\":");
        Self::append_vector3_literal(dump, point);
        let _ = write!(
            dump,
            ",\"source_terrain_y_m\":{:.3},\"visual_terrain_y_m\":{:.3}",
            source_y_m, visual_y_m
        );
        dump.push('}');
    }

    fn append_vector3_list_literal(dump: &mut String, points: &[Vector3]) {
        dump.push('[');
        for (index, point) in points.iter().enumerate() {
            if index > 0 {
                dump.push_str(", ");
            }
            Self::append_vector3_literal(dump, *point);
        }
        dump.push(']');
    }

    fn append_usize_list_literal(dump: &mut String, values: &[usize]) {
        dump.push('[');
        for (index, value) in values.iter().enumerate() {
            if index > 0 {
                dump.push_str(", ");
            }
            let _ = write!(dump, "{value}");
        }
        dump.push(']');
    }

    fn append_chunk_key_list_literal(dump: &mut String, chunks: &[SurfaceChunkKey]) {
        dump.push('[');
        for (index, chunk) in chunks.iter().enumerate() {
            if index > 0 {
                dump.push_str(", ");
            }
            let _ = write!(dump, "[{}, {}]", chunk.0, chunk.1);
        }
        dump.push(']');
    }

    fn append_vector3_literal(dump: &mut String, point: Vector3) {
        let _ = write!(dump, "[{:.3}, {:.3}, {:.3}]", point.x, point.y, point.z);
    }

    fn append_vector2_literal(dump: &mut String, point: Vector2) {
        let _ = write!(dump, "[{:.3}, {:.3}]", point.x, point.y);
    }

    fn append_optional_f32_literal(dump: &mut String, value: Option<f32>) {
        if let Some(value) = value {
            let _ = write!(dump, "{value:.3}");
        } else {
            dump.push_str("null");
        }
    }
}
