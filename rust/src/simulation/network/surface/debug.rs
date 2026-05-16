//! Debug extraction helpers for compiled road-surface state.

use super::{
    IncidentEdgeSide, IncidentMouthProfile, NodeOverlayContour, NodeOverlayPoint, NodeOverlayShape,
    NodeOverlayShapes, RoadSurfaceBandKind, RoadSurfaceEarthworkFaceKind,
    RoadSurfaceEarthworkFaceSource, RoadSurfaceSection, RoadSurfaceSpanBandOwner,
    RoadSurfaceSpanOwnedRegion, RoadSurfaceSpanRegionRole, RoadSurfaceSystem,
    RoadSurfaceVerticalFaceSource, RoadSurfaceVisualNodePiece, RoadSurfaceVisualPolygon,
    RoadSurfaceVisualSpanPiece, SAMPLE_EPSILON_M, SurfaceChunkKey,
    arrangement::{NodeArrangementKey, NodeBandOwner, NodeExplicitVerticalStepSegment},
    backend,
    band_semantics::ordered_raised_step_kinds,
    height::NodeHeightAuthoritySource,
    keys::{SurfaceHeightMmKey, SurfaceXzKey},
    node_grade::{NodeGradeCarrierDecision, NodeGradeVertexAuthority},
};
use crate::config;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::{Vector2, Vector3};
use i_overlay::core::overlay_rule::OverlayRule;
use std::collections::BTreeMap;
use std::fmt::Write as _;

#[derive(Default)]
pub(crate) struct RoadSurfaceDebugData {
    pub(crate) section_lines: Vec<Vector3>,
    pub(crate) band_lines: Vec<Vector3>,
    pub(crate) piece_boundary_lines: Vec<Vector3>,
    pub(crate) earthwork_chunk_lines: Vec<Vector3>,
}

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

#[derive(Clone, Copy)]
struct DebugMouthTopAnchor {
    point_index: usize,
    band_index: usize,
    role: &'static str,
    material: &'static str,
    point: Vector3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct DebugRenderVertexKey {
    x_key: i64,
    y_mm: i64,
    z_key: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct DebugRenderXzVertexKey {
    x_key: i64,
    z_key: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct DebugRenderEdgeKey {
    start: DebugRenderVertexKey,
    end: DebugRenderVertexKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct DebugRenderXzEdgeKey {
    start: DebugRenderXzVertexKey,
    end: DebugRenderXzVertexKey,
}

#[derive(Clone, Copy)]
struct DebugBoundaryOwner {
    region_index: usize,
    kind: RoadSurfaceBandKind,
    owner_index: usize,
}

#[derive(Clone, Copy)]
struct DebugTopBoundaryEdge {
    owner: DebugBoundaryOwner,
    start: Vector3,
    end: Vector3,
    key: DebugRenderEdgeKey,
    xz_key: DebugRenderXzEdgeKey,
    avg_y_m: f32,
}

#[derive(Clone, Copy)]
struct DebugVerticalFaceSpanEdges {
    lower_start: Vector3,
    lower_end: Vector3,
    upper_start: Vector3,
    upper_end: Vector3,
}

#[derive(Clone, Copy)]
struct DebugExpectedVerticalStep {
    lower: DebugTopBoundaryEdge,
    upper: DebugTopBoundaryEdge,
}

struct DebugCanonicalVerticalStep {
    explicit_vertical_step_index: usize,
    segment: NodeExplicitVerticalStepSegment,
    lower_owner: NodeBandOwner,
    raised_owner: NodeBandOwner,
    lower_top_matches: Vec<DebugTopBoundaryEdge>,
    raised_top_matches: Vec<DebugTopBoundaryEdge>,
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

impl DebugRenderVertexKey {
    fn from_point(point: Vector3) -> Self {
        let xz_key = SurfaceXzKey::from_godot_world_xz(point);
        Self {
            x_key: xz_key.x_key(),
            y_mm: SurfaceHeightMmKey::from_m_f32(point.y).as_i64(),
            z_key: xz_key.z_key(),
        }
    }

    fn xz(self) -> DebugRenderXzVertexKey {
        DebugRenderXzVertexKey {
            x_key: self.x_key,
            z_key: self.z_key,
        }
    }
}

impl DebugRenderEdgeKey {
    fn normalized(start: Vector3, end: Vector3) -> Option<Self> {
        let start = DebugRenderVertexKey::from_point(start);
        let end = DebugRenderVertexKey::from_point(end);
        if start == end {
            return None;
        }
        Some(if start <= end {
            Self { start, end }
        } else {
            Self {
                start: end,
                end: start,
            }
        })
    }

    fn xz(self) -> DebugRenderXzEdgeKey {
        DebugRenderXzEdgeKey::normalized(self.start.xz(), self.end.xz())
    }
}

impl DebugRenderXzEdgeKey {
    fn normalized(start: DebugRenderXzVertexKey, end: DebugRenderXzVertexKey) -> Self {
        if start <= end {
            Self { start, end }
        } else {
            Self {
                start: end,
                end: start,
            }
        }
    }

    fn from_arrangement_segment(start: NodeArrangementKey, end: NodeArrangementKey) -> Self {
        Self::normalized(
            DebugRenderXzVertexKey {
                x_key: start.x_key(),
                z_key: start.z_key(),
            },
            DebugRenderXzVertexKey {
                x_key: end.x_key(),
                z_key: end.z_key(),
            },
        )
    }
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

    fn append_node_compile_status_debug_dump(
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

    fn append_span_ownership_debug_literal(dump: &mut String, piece: &RoadSurfaceVisualSpanPiece) {
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

    fn append_span_earthwork_support_debug_literal(
        dump: &mut String,
        piece: &RoadSurfaceVisualSpanPiece,
    ) {
        dump.push('{');
        let _ = write!(
            dump,
            "\"support_region_count\":{},\"edge_class\":\"{:?}\",\"support_policy\":\"{}\"",
            piece.span_earthwork_support_regions.len(),
            piece.edge_class,
            super::RoadSurfaceEarthworkSupportPolicy::from_edge_class(piece.edge_class)
                .debug_name()
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

    fn append_span_raised_step_sources_debug_literal(
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

    fn append_span_terrain_clip_source_edges_debug_literal(
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

    fn append_earthwork_face_sources_debug_literal(
        dump: &mut String,
        faces: &[super::RoadSurfaceEarthworkRenderFace],
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

    fn append_earthwork_face_source_debug_literal(
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
            } => {
                let _ = write!(
                    dump,
                    "{{\"source_kind\":\"node_footprint_boundary\",\"node_id\":{},\"node_kind\":\"{:?}\",\"owner_kind\":\"{:?}\",\"owner_index\":{}}}",
                    node_id, kind, owner_kind, owner_index
                );
            }
        }
    }

    fn append_span_projection_diagnostics_debug_literal(
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

    fn append_span_band_owner_debug_literal(dump: &mut String, owner: RoadSurfaceSpanBandOwner) {
        let _ = write!(
            dump,
            "{{\"source_band_index\":{},\"kind\":\"{:?}\"}}",
            owner.source_band_index, owner.kind
        );
    }

    fn span_region_projection_matches_from_regions(
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

    fn debug_polygon_height_range(polygon: &RoadSurfaceVisualPolygon) -> Option<(f32, f32)> {
        let mut min_y = f32::INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for point in &polygon.points_world {
            min_y = min_y.min(point.y);
            max_y = max_y.max(point.y);
        }
        min_y.is_finite().then_some((min_y, max_y))
    }

    fn span_region_role_debug_name(role: RoadSurfaceSpanRegionRole) -> &'static str {
        match role {
            RoadSurfaceSpanRegionRole::Asphalt => "asphalt",
            RoadSurfaceSpanRegionRole::CurbOrShoulder => "curb_or_shoulder",
            RoadSurfaceSpanRegionRole::NonRoad => "non_road",
        }
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
        dump.push_str("      \"node_grade_authority\": ");
        Self::append_node_grade_authority_debug_literal(dump, piece);
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

    fn append_node_grade_authority_debug_literal(
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

    fn append_node_grade_authority_record_debug_literal(
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

    fn append_node_height_authority_debug_literal(
        dump: &mut String,
        authority: Option<NodeHeightAuthoritySource>,
    ) {
        if let Some(authority) = authority {
            let _ = write!(dump, "\"{:?}\"", authority);
        } else {
            dump.push_str("null");
        }
    }

    fn node_grade_decision_debug_name(decision: NodeGradeCarrierDecision) -> &'static str {
        match decision {
            NodeGradeCarrierDecision::SourceCarrier { .. } => "source_carrier",
            NodeGradeCarrierDecision::SameOwnerCanonicalVertex => "same_owner_canonical_vertex",
            NodeGradeCarrierDecision::SameMaterialSharedEdge => "same_material_shared_edge",
            NodeGradeCarrierDecision::SameMaterialVertex => "same_material_vertex",
            NodeGradeCarrierDecision::SameMaterialSeam => "same_material_seam",
            NodeGradeCarrierDecision::ExplicitMaterialSeam => "explicit_material_seam",
            NodeGradeCarrierDecision::ExplicitMaterialSeamAdoption => {
                "explicit_material_seam_adoption"
            }
        }
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

    fn append_raised_step_face_details_debug_literal(
        dump: &mut String,
        piece: &RoadSurfaceVisualNodePiece,
    ) {
        let top_edges = Self::debug_owned_top_boundary_edges(piece);
        let mut top_edges_by_key: BTreeMap<DebugRenderEdgeKey, Vec<DebugTopBoundaryEdge>> =
            BTreeMap::new();
        for edge in &top_edges {
            top_edges_by_key.entry(edge.key).or_default().push(*edge);
        }
        let expected_steps = Self::debug_expected_raised_steps(&top_edges);
        let canonical_steps = Self::debug_canonical_raised_steps(piece, &top_edges);

        let face_span_edges: Vec<Option<DebugVerticalFaceSpanEdges>> = piece
            .raised_step_face_polygons
            .iter()
            .map(Self::debug_vertical_face_span_edges)
            .collect();
        let mut face_expected_matches = vec![Vec::new(); face_span_edges.len()];
        let mut expected_face_matches = vec![Vec::new(); expected_steps.len()];
        let mut face_canonical_matches = vec![Vec::new(); face_span_edges.len()];
        let mut canonical_face_matches = vec![Vec::new(); canonical_steps.len()];
        let canonical_step_indices_by_source: BTreeMap<
            (usize, NodeExplicitVerticalStepSegment),
            usize,
        > = canonical_steps
            .iter()
            .enumerate()
            .map(|(step_index, step)| {
                (
                    (step.explicit_vertical_step_index, step.segment),
                    step_index,
                )
            })
            .collect();
        for (face_index, source) in piece.raised_step_face_sources.iter().copied().enumerate() {
            if face_index >= face_canonical_matches.len() {
                continue;
            }
            if let RoadSurfaceVerticalFaceSource::CanonicalStep {
                explicit_vertical_step_index,
                segment,
            } = source
            {
                if let Some(&canonical_step_index) =
                    canonical_step_indices_by_source.get(&(explicit_vertical_step_index, segment))
                {
                    face_canonical_matches[face_index].push(canonical_step_index);
                    canonical_face_matches[canonical_step_index].push(face_index);
                }
            }
        }

        for (face_index, span_edges) in face_span_edges.iter().enumerate() {
            let Some(span_edges) = span_edges else {
                continue;
            };
            let Some(lower_key) =
                DebugRenderEdgeKey::normalized(span_edges.lower_start, span_edges.lower_end)
            else {
                continue;
            };
            let Some(upper_key) =
                DebugRenderEdgeKey::normalized(span_edges.upper_start, span_edges.upper_end)
            else {
                continue;
            };
            for (step_index, step) in expected_steps.iter().enumerate() {
                if step.lower.key == lower_key && step.upper.key == upper_key {
                    face_expected_matches[face_index].push(step_index);
                    expected_face_matches[step_index].push(face_index);
                }
            }
        }

        let mut face_problem_count = 0usize;
        for (face_index, span_edges) in face_span_edges.iter().enumerate() {
            let Some(span_edges) = span_edges else {
                face_problem_count += 1;
                continue;
            };
            let lower_key =
                DebugRenderEdgeKey::normalized(span_edges.lower_start, span_edges.lower_end);
            let upper_key =
                DebugRenderEdgeKey::normalized(span_edges.upper_start, span_edges.upper_end);
            let lower_matches = lower_key
                .and_then(|key| top_edges_by_key.get(&key))
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let upper_matches = upper_key
                .and_then(|key| top_edges_by_key.get(&key))
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let matches_raised_step_owner_pair =
                Self::debug_top_matches_form_raised_step_owner_pair(lower_matches, upper_matches);
            let visible_dot = Self::debug_polygon_winding_normal(
                &piece.raised_step_face_polygons[face_index].points_world,
            )
            .map(|normal| -normal)
            .and_then(|direction| {
                Self::debug_visible_dot_to_lower_raised_step_owner(
                    piece,
                    (span_edges.lower_start + span_edges.lower_end) * 0.5,
                    direction,
                    lower_matches,
                    upper_matches,
                )
            });
            let visible_from_lower_owner = visible_dot.is_some_and(|dot| dot > 0.0);
            let face_problem = !matches_raised_step_owner_pair
                || (face_expected_matches[face_index].is_empty()
                    && face_canonical_matches[face_index].is_empty())
                || !visible_from_lower_owner;
            if face_problem {
                face_problem_count += 1;
            }
        }
        let missing_required_face_count = expected_face_matches
            .iter()
            .filter(|matches| matches.is_empty())
            .count();
        let final_required_problem_count = face_problem_count + missing_required_face_count;
        let non_exposed_source_constraint_count = canonical_face_matches
            .iter()
            .filter(|matches| matches.is_empty())
            .count();
        let canonical_problem_count = canonical_steps
            .iter()
            .zip(&canonical_face_matches)
            .filter(|(step, matches)| {
                !matches.is_empty()
                    && !Self::debug_canonical_step_visible_from_lower_owner(
                        piece,
                        step,
                        matches,
                        &face_span_edges,
                        &top_edges_by_key,
                    )
                    .unwrap_or(false)
            })
            .count();
        dump.push('{');
        let _ = write!(
            dump,
            "\"face_count\":{},\"emitted_face_count\":{},\"top_boundary_edge_count\":{},\"expected_raised_step_count\":{},\"final_required_face_count\":{},\"missing_required_face_count\":{},\"face_problem_count\":{},\"final_required_problem_count\":{},\"canonical_raised_step_count\":{},\"source_constraint_count\":{},\"non_exposed_source_constraint_count\":{},\"canonical_raised_step_problem_count\":{},\"problem_count\":{}",
            piece.raised_step_face_polygons.len(),
            piece.raised_step_face_polygons.len(),
            top_edges.len(),
            expected_steps.len(),
            expected_steps.len(),
            missing_required_face_count,
            face_problem_count,
            final_required_problem_count,
            canonical_steps.len(),
            canonical_steps.len(),
            non_exposed_source_constraint_count,
            canonical_problem_count,
            final_required_problem_count
        );
        dump.push_str(",\"faces\":[");
        for (face_index, polygon) in piece.raised_step_face_polygons.iter().enumerate() {
            if face_index > 0 {
                dump.push_str(", ");
            }
            Self::append_raised_step_face_detail_literal(
                dump,
                piece,
                face_index,
                polygon,
                piece.raised_step_face_sources.get(face_index).copied(),
                face_span_edges[face_index],
                &top_edges_by_key,
                &face_expected_matches[face_index],
                &face_canonical_matches[face_index],
            );
        }
        dump.push_str("],\"expected_raised_steps\":[");
        for (step_index, step) in expected_steps.iter().enumerate() {
            if step_index > 0 {
                dump.push_str(", ");
            }
            Self::append_expected_vertical_step_literal(
                dump,
                step_index,
                *step,
                &expected_face_matches[step_index],
            );
        }
        dump.push_str("],\"canonical_raised_steps\":[");
        for (step_index, step) in canonical_steps.iter().enumerate() {
            if step_index > 0 {
                dump.push_str(", ");
            }
            Self::append_canonical_vertical_step_literal(
                dump,
                piece,
                step_index,
                step,
                &canonical_face_matches[step_index],
                &face_span_edges,
                &top_edges_by_key,
            );
        }
        dump.push_str("]}");
    }

    fn append_raised_step_face_detail_literal(
        dump: &mut String,
        piece: &RoadSurfaceVisualNodePiece,
        face_index: usize,
        polygon: &RoadSurfaceVisualPolygon,
        source: Option<RoadSurfaceVerticalFaceSource>,
        span_edges: Option<DebugVerticalFaceSpanEdges>,
        top_edges_by_key: &BTreeMap<DebugRenderEdgeKey, Vec<DebugTopBoundaryEdge>>,
        expected_step_matches: &[usize],
        canonical_step_matches: &[usize],
    ) {
        let normal = Self::debug_polygon_winding_normal(&polygon.points_world);
        let visible_direction = normal.map(|normal| -normal);

        dump.push('{');
        let _ = write!(
            dump,
            "\"face\":{},\"polygon_vertex_count\":{},\"triangle_count\":{}",
            face_index,
            polygon.points_world.len(),
            polygon.triangles_world.len()
        );
        dump.push_str(",\"points_world\":");
        Self::append_vector3_precise_list_literal(dump, &polygon.points_world);
        dump.push_str(",\"winding_normal\":");
        Self::append_optional_vector3_precise_literal(dump, normal);
        dump.push_str(",\"godot_cull_back_visible_direction\":");
        Self::append_optional_vector3_precise_literal(dump, visible_direction);
        Self::append_raised_step_face_source_literal(dump, source);

        let Some(span_edges) = span_edges else {
            dump.push_str(",\"status\":\"non_vertical_quad_span\"}");
            return;
        };

        let lower_key =
            DebugRenderEdgeKey::normalized(span_edges.lower_start, span_edges.lower_end);
        let upper_key =
            DebugRenderEdgeKey::normalized(span_edges.upper_start, span_edges.upper_end);
        dump.push_str(",\"lower_edge_world\":");
        Self::append_vector3_pair_precise_literal(
            dump,
            span_edges.lower_start,
            span_edges.lower_end,
        );
        dump.push_str(",\"upper_edge_world\":");
        Self::append_vector3_pair_precise_literal(
            dump,
            span_edges.upper_start,
            span_edges.upper_end,
        );
        dump.push_str(",\"lower_edge_key\":");
        Self::append_optional_debug_render_edge_key_literal(dump, lower_key);
        dump.push_str(",\"upper_edge_key\":");
        Self::append_optional_debug_render_edge_key_literal(dump, upper_key);

        let lower_matches = lower_key.and_then(|key| top_edges_by_key.get(&key));
        let upper_matches = upper_key.and_then(|key| top_edges_by_key.get(&key));
        dump.push_str(",\"lower_top_matches\":");
        Self::append_debug_top_boundary_edge_list_literal(
            dump,
            lower_matches.map(Vec::as_slice).unwrap_or(&[]),
        );
        dump.push_str(",\"upper_top_matches\":");
        Self::append_debug_top_boundary_edge_list_literal(
            dump,
            upper_matches.map(Vec::as_slice).unwrap_or(&[]),
        );
        dump.push_str(",\"matching_expected_step_indices\":");
        Self::append_usize_list_literal(dump, expected_step_matches);
        dump.push_str(",\"matching_canonical_step_indices\":");
        Self::append_usize_list_literal(dump, canonical_step_matches);

        let lower_midpoint = (span_edges.lower_start + span_edges.lower_end) * 0.5;
        let visible_dot = visible_direction.and_then(|direction| {
            Self::debug_visible_dot_to_lower_raised_step_owner(
                piece,
                lower_midpoint,
                direction,
                lower_matches.map(Vec::as_slice).unwrap_or(&[]),
                upper_matches.map(Vec::as_slice).unwrap_or(&[]),
            )
        });
        dump.push_str(",\"visible_dot_lower_owner\":");
        Self::append_optional_f32_precise_literal(dump, visible_dot);
        dump.push_str(",\"visible_from_lower_owner\":");
        if let Some(dot) = visible_dot {
            let _ = write!(dump, "{}", dot > 0.0);
        } else {
            dump.push_str("null");
        }

        let matches_raised_step_owner_pair = Self::debug_top_matches_form_raised_step_owner_pair(
            lower_matches.map(Vec::as_slice).unwrap_or(&[]),
            upper_matches.map(Vec::as_slice).unwrap_or(&[]),
        );
        let face_problem = !matches_raised_step_owner_pair
            || (expected_step_matches.is_empty() && canonical_step_matches.is_empty())
            || visible_dot.is_none_or(|dot| dot <= 0.0);
        let _ = write!(
            dump,
            ",\"matches_raised_step_owner_pair\":{},\"problem\":{}",
            matches_raised_step_owner_pair, face_problem
        );
        dump.push('}');
    }

    fn append_raised_step_face_source_literal(
        dump: &mut String,
        source: Option<RoadSurfaceVerticalFaceSource>,
    ) {
        dump.push_str(",\"source_kind\":");
        match source {
            Some(RoadSurfaceVerticalFaceSource::CanonicalStep { .. }) => {
                dump.push_str("\"canonical_step\"");
            }
            Some(RoadSurfaceVerticalFaceSource::FinalOwnedBoundary { .. }) => {
                dump.push_str("\"final_owned_boundary\"");
            }
            None => dump.push_str("null"),
        }
        dump.push_str(",\"source_explicit_vertical_step_index\":");
        if let Some(source_index) =
            source.and_then(RoadSurfaceVerticalFaceSource::explicit_vertical_step_index)
        {
            let _ = write!(dump, "{source_index}");
        } else {
            dump.push_str("null");
        }
        dump.push_str(",\"source_owner_pair\":");
        if let Some(source) = source {
            let segment = source.segment();
            dump.push_str("{\"owner\":");
            Self::append_node_band_owner_literal(dump, segment.owner());
            dump.push_str(",\"opposite_owner\":");
            Self::append_node_band_owner_literal(dump, segment.opposite_owner());
            dump.push('}');
        } else {
            dump.push_str("null");
        }
        dump.push_str(",\"source_canonical_edge_key\":");
        if let Some(source) = source {
            let segment = source.segment();
            Self::append_node_arrangement_segment_key_literal(dump, segment.start(), segment.end());
        } else {
            dump.push_str("null");
        }
    }

    fn append_expected_vertical_step_literal(
        dump: &mut String,
        step_index: usize,
        step: DebugExpectedVerticalStep,
        face_matches: &[usize],
    ) {
        dump.push('{');
        let _ = write!(dump, "\"step\":{},\"lower\":", step_index);
        Self::append_debug_top_boundary_edge_literal(dump, step.lower);
        dump.push_str(",\"upper\":");
        Self::append_debug_top_boundary_edge_literal(dump, step.upper);
        dump.push_str(",\"matching_face_indices\":");
        Self::append_usize_list_literal(dump, face_matches);
        let _ = write!(dump, ",\"problem\":{}", face_matches.is_empty());
        dump.push('}');
    }

    fn append_canonical_vertical_step_literal(
        dump: &mut String,
        piece: &RoadSurfaceVisualNodePiece,
        step_index: usize,
        step: &DebugCanonicalVerticalStep,
        face_matches: &[usize],
        face_span_edges: &[Option<DebugVerticalFaceSpanEdges>],
        top_edges_by_key: &BTreeMap<DebugRenderEdgeKey, Vec<DebugTopBoundaryEdge>>,
    ) {
        let visible_dot = Self::debug_canonical_step_visible_dot_from_lower_owner(
            piece,
            step,
            face_matches,
            face_span_edges,
            top_edges_by_key,
        );
        let visible_from_lower_owner = visible_dot.map(|dot| dot > 0.0);
        let materialized = !face_matches.is_empty();
        let problem = materialized && visible_from_lower_owner != Some(true);

        dump.push('{');
        let _ = write!(
            dump,
            "\"step\":{},\"explicit_vertical_step_index\":{},\"owner_pair\":{{\"owner\":",
            step_index, step.explicit_vertical_step_index
        );
        Self::append_node_band_owner_literal(dump, step.segment.owner());
        dump.push_str(",\"opposite_owner\":");
        Self::append_node_band_owner_literal(dump, step.segment.opposite_owner());
        dump.push_str("},\"lower_owner\":");
        Self::append_node_band_owner_literal(dump, step.lower_owner);
        dump.push_str(",\"raised_owner\":");
        Self::append_node_band_owner_literal(dump, step.raised_owner);
        dump.push_str(",\"canonical_edge_key\":");
        Self::append_node_arrangement_segment_key_literal(
            dump,
            step.segment.start(),
            step.segment.end(),
        );
        dump.push_str(",\"height_delta_m\":");
        Self::append_optional_f32_precise_literal(
            dump,
            Self::debug_canonical_step_height_delta(step),
        );
        dump.push_str(",\"lower_top_matches\":");
        Self::append_debug_top_boundary_edge_list_literal(dump, &step.lower_top_matches);
        dump.push_str(",\"raised_top_matches\":");
        Self::append_debug_top_boundary_edge_list_literal(dump, &step.raised_top_matches);
        dump.push_str(",\"matching_face_indices\":");
        Self::append_usize_list_literal(dump, face_matches);
        dump.push_str(",\"materialization_status\":");
        if materialized {
            dump.push_str("\"materialized\"");
        } else {
            dump.push_str("\"not_exposed_after_boolean_ownership\"");
        }
        dump.push_str(",\"visible_dot_lower_owner\":");
        Self::append_optional_f32_precise_literal(dump, visible_dot);
        dump.push_str(",\"visible_from_lower_owner\":");
        if let Some(visible) = visible_from_lower_owner {
            let _ = write!(dump, "{visible}");
        } else {
            dump.push_str("null");
        }
        let _ = write!(dump, ",\"problem\":{problem}");
        dump.push('}');
    }

    fn debug_canonical_raised_steps(
        piece: &RoadSurfaceVisualNodePiece,
        top_edges: &[DebugTopBoundaryEdge],
    ) -> Vec<DebugCanonicalVerticalStep> {
        let mut steps = Vec::new();
        for (explicit_vertical_step_index, segment) in piece
            .explicit_vertical_step_segments
            .iter()
            .copied()
            .enumerate()
        {
            let Some((lower_owner, raised_owner)) =
                Self::debug_canonical_step_lower_and_raised_owners(segment)
            else {
                continue;
            };
            let xz_key =
                DebugRenderXzEdgeKey::from_arrangement_segment(segment.start(), segment.end());
            let lower_top_matches = top_edges
                .iter()
                .copied()
                .filter(|edge| {
                    edge.xz_key == xz_key
                        && Self::debug_boundary_owner_matches_band(edge.owner, lower_owner)
                })
                .collect();
            let raised_top_matches = top_edges
                .iter()
                .copied()
                .filter(|edge| {
                    edge.xz_key == xz_key
                        && Self::debug_boundary_owner_matches_band(edge.owner, raised_owner)
                })
                .collect();
            steps.push(DebugCanonicalVerticalStep {
                explicit_vertical_step_index,
                segment,
                lower_owner,
                raised_owner,
                lower_top_matches,
                raised_top_matches,
            });
        }
        steps.sort_by(|a, b| {
            a.explicit_vertical_step_index
                .cmp(&b.explicit_vertical_step_index)
                .then(a.segment.start().cmp(&b.segment.start()))
                .then(a.segment.end().cmp(&b.segment.end()))
                .then(a.lower_owner.cmp(&b.lower_owner))
                .then(a.raised_owner.cmp(&b.raised_owner))
        });
        steps
    }

    fn debug_canonical_step_visible_from_lower_owner(
        piece: &RoadSurfaceVisualNodePiece,
        step: &DebugCanonicalVerticalStep,
        face_matches: &[usize],
        face_span_edges: &[Option<DebugVerticalFaceSpanEdges>],
        top_edges_by_key: &BTreeMap<DebugRenderEdgeKey, Vec<DebugTopBoundaryEdge>>,
    ) -> Option<bool> {
        Self::debug_canonical_step_visible_dot_from_lower_owner(
            piece,
            step,
            face_matches,
            face_span_edges,
            top_edges_by_key,
        )
        .map(|dot| dot > 0.0)
    }

    fn debug_canonical_step_visible_dot_from_lower_owner(
        piece: &RoadSurfaceVisualNodePiece,
        step: &DebugCanonicalVerticalStep,
        face_matches: &[usize],
        face_span_edges: &[Option<DebugVerticalFaceSpanEdges>],
        top_edges_by_key: &BTreeMap<DebugRenderEdgeKey, Vec<DebugTopBoundaryEdge>>,
    ) -> Option<f32> {
        let mut best: Option<f32> = None;
        for &face_index in face_matches {
            let Some(span_edges) = face_span_edges.get(face_index).copied().flatten() else {
                continue;
            };
            let Some(visible_direction) = piece
                .raised_step_face_polygons
                .get(face_index)
                .and_then(|polygon| Self::debug_polygon_winding_normal(&polygon.points_world))
                .map(|normal| -normal)
            else {
                continue;
            };
            let lower_key =
                DebugRenderEdgeKey::normalized(span_edges.lower_start, span_edges.lower_end);
            let lower_matches = lower_key
                .and_then(|key| top_edges_by_key.get(&key))
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let Some(dot) = Self::debug_visible_dot_to_lower_owner(
                piece,
                (span_edges.lower_start + span_edges.lower_end) * 0.5,
                visible_direction,
                lower_matches,
                step.lower_owner,
            ) else {
                continue;
            };
            best = Some(best.map_or(dot, |current| current.max(dot)));
        }
        best
    }

    fn debug_canonical_step_height_delta(step: &DebugCanonicalVerticalStep) -> Option<f32> {
        let lower = step.lower_top_matches.first()?;
        let raised = step.raised_top_matches.first()?;
        Some(raised.avg_y_m - lower.avg_y_m)
    }

    fn debug_canonical_step_lower_and_raised_owners(
        segment: NodeExplicitVerticalStepSegment,
    ) -> Option<(NodeBandOwner, NodeBandOwner)> {
        let owner = segment.owner();
        let opposite_owner = segment.opposite_owner();
        let (lower_kind, _) = ordered_raised_step_kinds(owner.kind(), opposite_owner.kind())?;
        if owner.kind() == lower_kind {
            Some((owner, opposite_owner))
        } else {
            Some((opposite_owner, owner))
        }
    }

    fn debug_owner_pair_forms_raised_step(
        lower_owner: DebugBoundaryOwner,
        raised_owner: DebugBoundaryOwner,
    ) -> bool {
        ordered_raised_step_kinds(lower_owner.kind, raised_owner.kind)
            == Some((lower_owner.kind, raised_owner.kind))
    }

    fn debug_top_matches_form_raised_step_owner_pair(
        lower_matches: &[DebugTopBoundaryEdge],
        upper_matches: &[DebugTopBoundaryEdge],
    ) -> bool {
        lower_matches.iter().any(|lower| {
            upper_matches
                .iter()
                .any(|upper| Self::debug_owner_pair_forms_raised_step(lower.owner, upper.owner))
        })
    }

    fn debug_boundary_owner_matches_band(
        owner: DebugBoundaryOwner,
        band_owner: NodeBandOwner,
    ) -> bool {
        owner.kind == band_owner.kind() && owner.owner_index == band_owner.owner_index()
    }

    fn append_node_band_owner_literal(dump: &mut String, owner: NodeBandOwner) {
        let _ = write!(
            dump,
            "{{\"kind\":\"{:?}\",\"owner_index\":{}}}",
            owner.kind(),
            owner.owner_index()
        );
    }

    fn append_node_arrangement_segment_key_literal(
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

    fn append_debug_top_boundary_edge_list_literal(
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

    fn append_debug_top_boundary_edge_literal(dump: &mut String, edge: DebugTopBoundaryEdge) {
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

    fn append_debug_boundary_owner_literal(dump: &mut String, owner: DebugBoundaryOwner) {
        let _ = write!(
            dump,
            "{{\"region\":{},\"kind\":\"{:?}\",\"owner_index\":{}}}",
            owner.region_index, owner.kind, owner.owner_index
        );
    }

    fn debug_owned_top_boundary_edges(
        piece: &RoadSurfaceVisualNodePiece,
    ) -> Vec<DebugTopBoundaryEdge> {
        let mut boundary_edges = Vec::new();
        for (region_index, region) in piece.owned_regions.iter().enumerate() {
            let owner = DebugBoundaryOwner {
                region_index,
                kind: region.kind,
                owner_index: region.owner_index,
            };
            let mut edge_counts: BTreeMap<DebugRenderEdgeKey, (usize, Vector3, Vector3)> =
                BTreeMap::new();
            if region.polygon.triangles_world.is_empty() {
                let points = &region.polygon.points_world;
                if points.len() >= 2 {
                    for index in 0..points.len() {
                        Self::record_debug_top_boundary_edge_count(
                            &mut edge_counts,
                            points[index],
                            points[(index + 1) % points.len()],
                        );
                    }
                }
            } else {
                for triangle in &region.polygon.triangles_world {
                    for edge_index in 0..3 {
                        Self::record_debug_top_boundary_edge_count(
                            &mut edge_counts,
                            triangle[edge_index],
                            triangle[(edge_index + 1) % 3],
                        );
                    }
                }
            }
            for (key, (count, start, end)) in edge_counts {
                if count != 1 {
                    continue;
                }
                boundary_edges.push(DebugTopBoundaryEdge {
                    owner,
                    start,
                    end,
                    key,
                    xz_key: key.xz(),
                    avg_y_m: (start.y + end.y) * 0.5,
                });
            }
        }
        boundary_edges.sort_by(|a, b| {
            a.key
                .cmp(&b.key)
                .then(a.owner.region_index.cmp(&b.owner.region_index))
                .then(a.owner.kind.cmp(&b.owner.kind))
                .then(a.owner.owner_index.cmp(&b.owner.owner_index))
        });
        boundary_edges
    }

    fn record_debug_top_boundary_edge_count(
        edge_counts: &mut BTreeMap<DebugRenderEdgeKey, (usize, Vector3, Vector3)>,
        start: Vector3,
        end: Vector3,
    ) {
        let Some(key) = DebugRenderEdgeKey::normalized(start, end) else {
            return;
        };
        edge_counts
            .entry(key)
            .and_modify(|entry| entry.0 += 1)
            .or_insert((1, start, end));
    }

    fn debug_expected_raised_steps(
        top_edges: &[DebugTopBoundaryEdge],
    ) -> Vec<DebugExpectedVerticalStep> {
        let mut edges_by_xz: BTreeMap<DebugRenderXzEdgeKey, Vec<DebugTopBoundaryEdge>> =
            BTreeMap::new();
        for edge in top_edges {
            edges_by_xz.entry(edge.xz_key).or_default().push(*edge);
        }

        let mut steps = Vec::new();
        for edges in edges_by_xz.values() {
            for (left_index, left_edge) in edges.iter().enumerate() {
                for right_edge in edges.iter().skip(left_index + 1) {
                    if left_edge.key == right_edge.key {
                        continue;
                    }
                    let (lower, upper) = if left_edge.avg_y_m <= right_edge.avg_y_m {
                        (*left_edge, *right_edge)
                    } else {
                        (*right_edge, *left_edge)
                    };
                    if !Self::debug_owner_pair_forms_raised_step(lower.owner, upper.owner) {
                        continue;
                    }
                    steps.push(DebugExpectedVerticalStep { lower, upper });
                }
            }
        }

        steps.sort_by(|a, b| {
            a.lower
                .key
                .cmp(&b.lower.key)
                .then(a.upper.key.cmp(&b.upper.key))
                .then(a.lower.owner.region_index.cmp(&b.lower.owner.region_index))
                .then(a.upper.owner.region_index.cmp(&b.upper.owner.region_index))
        });
        steps
    }

    fn debug_vertical_face_span_edges(
        polygon: &RoadSurfaceVisualPolygon,
    ) -> Option<DebugVerticalFaceSpanEdges> {
        if polygon.points_world.len() < 4 {
            return None;
        }
        let mut span_edges = Vec::new();
        for index in 0..polygon.points_world.len() {
            let start = polygon.points_world[index];
            let end = polygon.points_world[(index + 1) % polygon.points_world.len()];
            let start_key = DebugRenderVertexKey::from_point(start).xz();
            let end_key = DebugRenderVertexKey::from_point(end).xz();
            if start_key != end_key {
                span_edges.push((start, end, (start.y + end.y) * 0.5));
            }
        }
        if span_edges.len() != 2 {
            return None;
        }
        span_edges.sort_by(|a, b| a.2.total_cmp(&b.2));
        Some(DebugVerticalFaceSpanEdges {
            lower_start: span_edges[0].0,
            lower_end: span_edges[0].1,
            upper_start: span_edges[1].0,
            upper_end: span_edges[1].1,
        })
    }

    fn debug_polygon_winding_normal(points: &[Vector3]) -> Option<Vector3> {
        if points.len() < 3 {
            return None;
        }
        for index in 1..points.len().saturating_sub(1) {
            let normal = (points[index] - points[0]).cross(points[index + 1] - points[0]);
            if normal.length_squared() > 1e-8 {
                return Some(normal.normalized());
            }
        }
        None
    }

    fn debug_visible_dot_to_lower_raised_step_owner(
        piece: &RoadSurfaceVisualNodePiece,
        face_midpoint: Vector3,
        visible_direction: Vector3,
        lower_matches: &[DebugTopBoundaryEdge],
        upper_matches: &[DebugTopBoundaryEdge],
    ) -> Option<f32> {
        let visible_xz = Vector3::new(visible_direction.x, 0.0, visible_direction.z);
        if visible_xz.length_squared() <= 1e-8 {
            return None;
        }
        let visible_xz = visible_xz.normalized();
        let mut best: Option<f32> = None;
        for edge in lower_matches.iter().filter(|lower| {
            upper_matches
                .iter()
                .any(|upper| Self::debug_owner_pair_forms_raised_step(lower.owner, upper.owner))
        }) {
            let Some(centroid) = Self::debug_owned_region_centroid(piece, edge.owner.region_index)
            else {
                continue;
            };
            let owner_direction = Vector3::new(
                centroid.x - face_midpoint.x,
                0.0,
                centroid.z - face_midpoint.z,
            );
            if owner_direction.length_squared() <= 1e-8 {
                continue;
            }
            let dot = visible_xz.dot(owner_direction.normalized());
            best = Some(best.map_or(dot, |current| current.max(dot)));
        }
        best
    }

    fn debug_visible_dot_to_lower_owner(
        piece: &RoadSurfaceVisualNodePiece,
        face_midpoint: Vector3,
        visible_direction: Vector3,
        lower_matches: &[DebugTopBoundaryEdge],
        lower_owner: NodeBandOwner,
    ) -> Option<f32> {
        let visible_xz = Vector3::new(visible_direction.x, 0.0, visible_direction.z);
        if visible_xz.length_squared() <= 1e-8 {
            return None;
        }
        let visible_xz = visible_xz.normalized();
        let mut best: Option<f32> = None;
        for edge in lower_matches
            .iter()
            .filter(|edge| Self::debug_boundary_owner_matches_band(edge.owner, lower_owner))
        {
            let Some(centroid) = Self::debug_owned_region_centroid(piece, edge.owner.region_index)
            else {
                continue;
            };
            let owner_direction = Vector3::new(
                centroid.x - face_midpoint.x,
                0.0,
                centroid.z - face_midpoint.z,
            );
            if owner_direction.length_squared() <= 1e-8 {
                continue;
            }
            let dot = visible_xz.dot(owner_direction.normalized());
            best = Some(best.map_or(dot, |current| current.max(dot)));
        }
        best
    }

    fn debug_owned_region_centroid(
        piece: &RoadSurfaceVisualNodePiece,
        region_index: usize,
    ) -> Option<Vector3> {
        let region = piece.owned_regions.get(region_index)?;
        let mut sum = Vector3::ZERO;
        let mut count = 0usize;
        if region.polygon.points_world.is_empty() {
            for point in region
                .polygon
                .triangles_world
                .iter()
                .flat_map(|triangle| triangle.iter().copied())
            {
                sum += point;
                count += 1;
            }
        } else {
            for point in &region.polygon.points_world {
                sum += *point;
                count += 1;
            }
        }
        (count > 0).then_some(sum * (1.0 / count as f32))
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
            for anchor in Self::mouth_top_match_anchors(mouth) {
                let Some(closest) = Self::closest_debug_top_support_for_material(
                    anchor.point,
                    anchor.material,
                    piece,
                ) else {
                    continue;
                };
                Self::update_debug_match_stats(&mut stats, closest);
                if Self::debug_match_is_problem(closest)
                    && samples.len() < DEBUG_MAX_PROBLEM_SAMPLES
                {
                    let mut sample = String::new();
                    let _ = write!(
                        sample,
                        "{{\"point\":{},\"band\":{},\"role\":\"{}\",\"expected_material\":\"{}\",\"mouth\":",
                        anchor.point_index, anchor.band_index, anchor.role, anchor.material
                    );
                    Self::append_vector3_literal(&mut sample, anchor.point);
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
        let mut span_support_source_count = 0_usize;
        let mut node_footprint_source_count = 0_usize;
        for (face_index, face) in piece.render_earthwork_faces.iter().enumerate() {
            match face.kind {
                RoadSurfaceEarthworkFaceKind::Slope => slope_count += 1,
                RoadSurfaceEarthworkFaceKind::RetainingWall => retaining_wall_count += 1,
            }
            match face.source {
                RoadSurfaceEarthworkFaceSource::SpanSupportBoundary { .. } => {
                    span_support_source_count += 1
                }
                RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary { .. } => {
                    node_footprint_source_count += 1
                }
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
                    "{{\"face\":{},\"kind\":\"{:?}\",\"source\":",
                    face_index, face.kind
                );
                Self::append_earthwork_face_source_debug_literal(&mut sample, face.source);
                sample.push_str(",\"inner_start\":");
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
            "\"face_count\":{},\"slope_count\":{},\"retaining_wall_count\":{},\"span_support_source_count\":{},\"node_footprint_source_count\":{},\"missing_source_count\":0,",
            piece.render_earthwork_faces.len(),
            slope_count,
            retaining_wall_count,
            span_support_source_count,
            node_footprint_source_count
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

    fn mouth_top_match_anchors(mouth: &IncidentMouthProfile) -> Vec<DebugMouthTopAnchor> {
        let mut anchors = Vec::new();
        if mouth.bands.is_empty() || mouth.boundary_points_world.len() != mouth.bands.len() + 1 {
            return anchors;
        }

        for point_index in 0..mouth.boundary_points_world.len() {
            if Self::mouth_boundary_point_is_outer_footprint(mouth, point_index) {
                let (band_index, point) = if point_index == 0 {
                    (0, mouth.bands[0].start_point_world)
                } else {
                    let band_index = mouth.bands.len() - 1;
                    (band_index, mouth.bands[band_index].end_point_world)
                };
                anchors.push(DebugMouthTopAnchor {
                    point_index,
                    band_index,
                    role: "outer_footprint",
                    material: Self::debug_material_for_band_kind(mouth.bands[band_index].kind),
                    point,
                });
                continue;
            }

            if !Self::mouth_boundary_point_is_material_seam(mouth, point_index) {
                continue;
            }

            let before_index = point_index - 1;
            let after_index = point_index;
            anchors.push(DebugMouthTopAnchor {
                point_index,
                band_index: before_index,
                role: "material_seam_before",
                material: Self::debug_material_for_band_kind(mouth.bands[before_index].kind),
                point: mouth.bands[before_index].end_point_world,
            });
            anchors.push(DebugMouthTopAnchor {
                point_index,
                band_index: after_index,
                role: "material_seam_after",
                material: Self::debug_material_for_band_kind(mouth.bands[after_index].kind),
                point: mouth.bands[after_index].start_point_world,
            });
        }
        anchors
    }

    fn debug_material_for_band_kind(kind: RoadSurfaceBandKind) -> &'static str {
        match kind {
            RoadSurfaceBandKind::Carriageway => "road",
            RoadSurfaceBandKind::CurbOrShoulder => "curb",
            _ => "sidewalk",
        }
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

    fn closest_debug_top_support_for_material(
        point: Vector3,
        material: &'static str,
        piece: &RoadSurfaceVisualNodePiece,
    ) -> Option<DebugClosestTopVertex> {
        let polygons = match material {
            "road" => &piece.road_surface_polygons,
            "curb" => &piece.curb_surface_polygons,
            _ => &piece.sidewalk_surface_polygons,
        };
        let mut best = None;
        for polygon in polygons {
            for &candidate in &polygon.points_world {
                Self::update_closest_debug_top_support(&mut best, point, material, candidate);
            }
            for index in 0..polygon.points_world.len() {
                let start = polygon.points_world[index];
                let end = polygon.points_world[(index + 1) % polygon.points_world.len()];
                Self::update_closest_debug_top_segment_support(
                    &mut best, point, material, start, end,
                );
            }
            for triangle in &polygon.triangles_world {
                for &candidate in triangle {
                    Self::update_closest_debug_top_support(&mut best, point, material, candidate);
                }
                for index in 0..3 {
                    Self::update_closest_debug_top_segment_support(
                        &mut best,
                        point,
                        material,
                        triangle[index],
                        triangle[(index + 1) % 3],
                    );
                }
            }
        }
        best
    }

    fn update_closest_debug_top_support(
        best: &mut Option<DebugClosestTopVertex>,
        point: Vector3,
        material: &'static str,
        candidate: Vector3,
    ) {
        let xz_error_m = Vector2::new(candidate.x - point.x, candidate.z - point.z).length();
        let candidate = DebugClosestTopVertex {
            material,
            point: candidate,
            xz_error_m,
            y_delta_m: point.y - candidate.y,
        };
        Self::retain_closest_debug_top_support(best, candidate);
    }

    fn update_closest_debug_top_segment_support(
        best: &mut Option<DebugClosestTopVertex>,
        point: Vector3,
        material: &'static str,
        start: Vector3,
        end: Vector3,
    ) {
        let segment_xz = Vector2::new(end.x - start.x, end.z - start.z);
        let len_squared = segment_xz.length_squared();
        if len_squared <= SAMPLE_EPSILON_M * SAMPLE_EPSILON_M {
            return;
        }
        let to_point_xz = Vector2::new(point.x - start.x, point.z - start.z);
        let t = (to_point_xz.dot(segment_xz) / len_squared).clamp(0.0, 1.0);
        let candidate = start.lerp(end, t);
        Self::update_closest_debug_top_support(best, point, material, candidate);
    }

    fn retain_closest_debug_top_support(
        best: &mut Option<DebugClosestTopVertex>,
        candidate: DebugClosestTopVertex,
    ) {
        let replace = best.is_none_or(|current| {
            candidate
                .xz_error_m
                .total_cmp(&current.xz_error_m)
                .then(
                    candidate
                        .y_delta_m
                        .abs()
                        .total_cmp(&current.y_delta_m.abs()),
                )
                .is_lt()
        });
        if replace {
            *best = Some(candidate);
        }
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

    fn append_vector3_precise_list_literal(dump: &mut String, points: &[Vector3]) {
        dump.push('[');
        for (index, point) in points.iter().enumerate() {
            if index > 0 {
                dump.push_str(", ");
            }
            Self::append_vector3_precise_literal(dump, *point);
        }
        dump.push(']');
    }

    fn append_vector3_pair_precise_literal(dump: &mut String, start: Vector3, end: Vector3) {
        dump.push('[');
        Self::append_vector3_precise_literal(dump, start);
        dump.push_str(", ");
        Self::append_vector3_precise_literal(dump, end);
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

    fn append_debug_render_edge_key_literal(dump: &mut String, key: DebugRenderEdgeKey) {
        dump.push('{');
        dump.push_str("\"start\":");
        Self::append_debug_render_vertex_key_literal(dump, key.start);
        dump.push_str(",\"end\":");
        Self::append_debug_render_vertex_key_literal(dump, key.end);
        dump.push('}');
    }

    fn append_optional_debug_render_edge_key_literal(
        dump: &mut String,
        key: Option<DebugRenderEdgeKey>,
    ) {
        if let Some(key) = key {
            Self::append_debug_render_edge_key_literal(dump, key);
        } else {
            dump.push_str("null");
        }
    }

    fn append_debug_render_vertex_key_literal(dump: &mut String, key: DebugRenderVertexKey) {
        let _ = write!(
            dump,
            "{{\"x_key\":{},\"y_mm\":{},\"z_key\":{}}}",
            key.x_key, key.y_mm, key.z_key
        );
    }

    fn append_debug_render_xz_edge_key_literal(dump: &mut String, key: DebugRenderXzEdgeKey) {
        dump.push('{');
        dump.push_str("\"start\":");
        Self::append_debug_render_xz_vertex_key_literal(dump, key.start);
        dump.push_str(",\"end\":");
        Self::append_debug_render_xz_vertex_key_literal(dump, key.end);
        dump.push('}');
    }

    fn append_debug_render_xz_vertex_key_literal(dump: &mut String, key: DebugRenderXzVertexKey) {
        let _ = write!(dump, "{{\"x_key\":{},\"z_key\":{}}}", key.x_key, key.z_key);
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

    fn append_vector3_precise_literal(dump: &mut String, point: Vector3) {
        let _ = write!(dump, "[{:.6}, {:.6}, {:.6}]", point.x, point.y, point.z);
    }

    fn append_optional_vector3_precise_literal(dump: &mut String, point: Option<Vector3>) {
        if let Some(point) = point {
            Self::append_vector3_precise_literal(dump, point);
        } else {
            dump.push_str("null");
        }
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

    fn append_optional_f32_precise_literal(dump: &mut String, value: Option<f32>) {
        if let Some(value) = value {
            let _ = write!(dump, "{value:.6}");
        } else {
            dump.push_str("null");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::surface::{
        IncidentMouthBand, NodeOwnedRegion, RoadSurfaceVisualNodePieceKind,
    };

    fn polygon(points_world: Vec<Vector3>) -> RoadSurfaceVisualPolygon {
        RoadSurfaceVisualPolygon {
            points_world,
            triangles_world: Vec::new(),
        }
    }

    fn empty_node_piece() -> RoadSurfaceVisualNodePiece {
        RoadSurfaceVisualNodePiece {
            node_id: 0,
            kind: RoadSurfaceVisualNodePieceKind::Terminal,
            outer_boundary_loops: Vec::new(),
            terrain_clip_boundary_loops: Vec::new(),
            road_surface_polygons: Vec::new(),
            curb_surface_polygons: Vec::new(),
            raised_step_face_polygons: Vec::new(),
            raised_step_face_sources: Vec::new(),
            sidewalk_surface_polygons: Vec::new(),
            explicit_vertical_step_segments: Vec::new(),
            node_grade_authorities: Vec::new(),
            owned_regions: Vec::new(),
            earthwork_surface_polygons: Vec::new(),
            earthwork_outer_boundary_loops: Vec::new(),
            render_earthwork_faces: Vec::new(),
        }
    }

    #[test]
    fn mouth_seam_debug_matches_vertical_step_anchors_by_material() {
        let curb_anchor = Vector3::new(0.0, 0.12, 0.0);
        let road_anchor = Vector3::new(0.0, 0.0, 0.0);
        let mouth = IncidentMouthProfile {
            inward_direction_xz: Vector2::RIGHT,
            boundary_points_world: vec![
                Vector3::new(-1.0, 0.12, 0.0),
                curb_anchor,
                Vector3::new(1.0, 0.0, 0.0),
            ],
            bands: vec![
                IncidentMouthBand {
                    kind: RoadSurfaceBandKind::CurbOrShoulder,
                    start_point_world: Vector3::new(-1.0, 0.12, 0.0),
                    end_point_world: curb_anchor,
                },
                IncidentMouthBand {
                    kind: RoadSurfaceBandKind::Carriageway,
                    start_point_world: road_anchor,
                    end_point_world: Vector3::new(1.0, 0.0, 0.0),
                },
            ],
        };

        let mut piece = empty_node_piece();
        piece.road_surface_polygons.push(polygon(vec![
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(1.0, 0.0, 0.0),
            Vector3::new(0.0, 0.0, -1.0),
        ]));
        piece.curb_surface_polygons.push(polygon(vec![
            Vector3::new(-1.0, 0.12, -1.0),
            Vector3::new(1.0, 0.12, 1.0),
            Vector3::new(-1.0, 0.12, 1.0),
        ]));

        let material_blind = RoadSurfaceSystem::closest_debug_top_vertex(
            curb_anchor,
            &RoadSurfaceSystem::debug_top_vertices(&piece),
        )
        .expect("test piece should expose a top vertex");
        assert_eq!(material_blind.material, "road");
        assert!((material_blind.y_delta_m - 0.12).abs() <= f32::EPSILON);

        let anchors = RoadSurfaceSystem::mouth_top_match_anchors(&mouth);
        let curb_seam_anchor = anchors
            .iter()
            .find(|anchor| anchor.role == "material_seam_before")
            .expect("curb side of asphalt-curb mouth seam should be checked");
        assert_eq!(curb_seam_anchor.material, "curb");
        let curb_match = RoadSurfaceSystem::closest_debug_top_support_for_material(
            curb_seam_anchor.point,
            curb_seam_anchor.material,
            &piece,
        )
        .expect("curb material should support its seam anchor");
        assert_eq!(curb_match.material, "curb");
        assert!(curb_match.xz_error_m <= f32::EPSILON);
        assert!(curb_match.y_delta_m.abs() <= f32::EPSILON);

        let road_seam_anchor = anchors
            .iter()
            .find(|anchor| anchor.role == "material_seam_after")
            .expect("road side of asphalt-curb mouth seam should be checked");
        assert_eq!(road_seam_anchor.material, "road");
        let road_match = RoadSurfaceSystem::closest_debug_top_support_for_material(
            road_seam_anchor.point,
            road_seam_anchor.material,
            &piece,
        )
        .expect("road material should support its seam anchor");
        assert_eq!(road_match.material, "road");
        assert!(road_match.xz_error_m <= f32::EPSILON);
        assert!(road_match.y_delta_m.abs() <= f32::EPSILON);
    }

    #[test]
    fn raised_step_face_debug_reports_exact_top_edge_closure() {
        let mut piece = empty_node_piece();
        piece.owned_regions.push(NodeOwnedRegion {
            kind: RoadSurfaceBandKind::Carriageway,
            owner_index: 7,
            polygon: polygon(vec![
                Vector3::new(-1.0, 0.0, -1.0),
                Vector3::new(1.0, 0.0, -1.0),
                Vector3::new(1.0, 0.0, 0.0),
                Vector3::new(-1.0, 0.0, 0.0),
            ]),
        });
        piece.owned_regions.push(NodeOwnedRegion {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
            owner_index: 11,
            polygon: polygon(vec![
                Vector3::new(-1.0, 0.12, 0.0),
                Vector3::new(1.0, 0.12, 0.0),
                Vector3::new(1.0, 0.12, 1.0),
                Vector3::new(-1.0, 0.12, 1.0),
            ]),
        });
        piece.raised_step_face_polygons.push(polygon(vec![
            Vector3::new(-1.0, 0.12, 0.0),
            Vector3::new(-1.0, 0.0, 0.0),
            Vector3::new(1.0, 0.0, 0.0),
            Vector3::new(1.0, 0.12, 0.0),
        ]));

        let mut dump = String::new();
        RoadSurfaceSystem::append_raised_step_face_details_debug_literal(&mut dump, &piece);

        assert!(dump.contains("\"face_count\":1"));
        assert!(dump.contains("\"expected_raised_step_count\":1"));
        assert!(dump.contains("\"problem_count\":0"));
        assert!(dump.contains("\"matches_raised_step_owner_pair\":true"));
        assert!(dump.contains("\"visible_from_lower_owner\":true"));
    }

    #[test]
    fn raised_step_face_debug_reports_generic_curb_sidewalk_steps() {
        let mut piece = empty_node_piece();
        let canonical_segment = NodeExplicitVerticalStepSegment::new(
            NodeArrangementKey::from_point(backend::RoadVec2::new(-1.0, 0.0)),
            NodeArrangementKey::from_point(backend::RoadVec2::new(1.0, 0.0)),
            NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 7),
            NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 11),
        )
        .expect("curb-sidewalk step should be non-degenerate");
        piece
            .explicit_vertical_step_segments
            .push(canonical_segment);
        piece.owned_regions.push(NodeOwnedRegion {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
            owner_index: 7,
            polygon: polygon(vec![
                Vector3::new(-1.0, 0.12, -1.0),
                Vector3::new(1.0, 0.12, -1.0),
                Vector3::new(1.0, 0.12, 0.0),
                Vector3::new(-1.0, 0.12, 0.0),
            ]),
        });
        piece.owned_regions.push(NodeOwnedRegion {
            kind: RoadSurfaceBandKind::Sidewalk,
            owner_index: 11,
            polygon: polygon(vec![
                Vector3::new(-1.0, 0.18, 0.0),
                Vector3::new(1.0, 0.18, 0.0),
                Vector3::new(1.0, 0.18, 1.0),
                Vector3::new(-1.0, 0.18, 1.0),
            ]),
        });
        piece.raised_step_face_polygons.push(polygon(vec![
            Vector3::new(-1.0, 0.18, 0.0),
            Vector3::new(-1.0, 0.12, 0.0),
            Vector3::new(1.0, 0.12, 0.0),
            Vector3::new(1.0, 0.18, 0.0),
        ]));
        piece
            .raised_step_face_sources
            .push(RoadSurfaceVerticalFaceSource::CanonicalStep {
                explicit_vertical_step_index: 0,
                segment: canonical_segment,
            });

        let mut dump = String::new();
        RoadSurfaceSystem::append_raised_step_face_details_debug_literal(&mut dump, &piece);

        assert!(dump.contains("\"canonical_raised_step_count\":1"));
        assert!(dump.contains("\"canonical_raised_step_problem_count\":0"));
        assert!(dump.contains("\"raised_owner\":{\"kind\":\"Sidewalk\",\"owner_index\":11}"));
        assert!(dump.contains("\"matching_face_indices\":[0]"));
        assert!(dump.contains("\"matches_raised_step_owner_pair\":true"));
        assert!(dump.contains("\"visible_from_lower_owner\":true"));
    }

    #[test]
    fn raised_step_face_debug_matches_canonical_step_by_source_identity() {
        let mut piece = empty_node_piece();
        let canonical_segment = NodeExplicitVerticalStepSegment::new(
            NodeArrangementKey::from_point(backend::RoadVec2::new(-1.0, 0.0)),
            NodeArrangementKey::from_point(backend::RoadVec2::new(1.0, 0.0)),
            NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 7),
            NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 11),
        )
        .expect("test step should be non-degenerate");
        piece
            .explicit_vertical_step_segments
            .push(canonical_segment);

        let rendered_end_x = 1.000002;
        piece.owned_regions.push(NodeOwnedRegion {
            kind: RoadSurfaceBandKind::Carriageway,
            owner_index: 7,
            polygon: polygon(vec![
                Vector3::new(-1.0, 0.0, -1.0),
                Vector3::new(rendered_end_x, 0.0, -1.0),
                Vector3::new(rendered_end_x, 0.0, 0.0),
                Vector3::new(-1.0, 0.0, 0.0),
            ]),
        });
        piece.owned_regions.push(NodeOwnedRegion {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
            owner_index: 11,
            polygon: polygon(vec![
                Vector3::new(-1.0, 0.12, 0.0),
                Vector3::new(rendered_end_x, 0.12, 0.0),
                Vector3::new(rendered_end_x, 0.12, 1.0),
                Vector3::new(-1.0, 0.12, 1.0),
            ]),
        });
        piece.raised_step_face_polygons.push(polygon(vec![
            Vector3::new(-1.0, 0.12, 0.0),
            Vector3::new(-1.0, 0.0, 0.0),
            Vector3::new(rendered_end_x, 0.0, 0.0),
            Vector3::new(rendered_end_x, 0.12, 0.0),
        ]));
        piece
            .raised_step_face_sources
            .push(RoadSurfaceVerticalFaceSource::CanonicalStep {
                explicit_vertical_step_index: 0,
                segment: canonical_segment,
            });

        let mut dump = String::new();
        RoadSurfaceSystem::append_raised_step_face_details_debug_literal(&mut dump, &piece);

        assert!(dump.contains("\"canonical_raised_step_count\":1"));
        assert!(dump.contains("\"canonical_raised_step_problem_count\":0"));
        assert!(dump.contains("\"source_kind\":\"canonical_step\""));
        assert!(dump.contains("\"source_explicit_vertical_step_index\":0"));
        assert!(dump.contains("\"matching_canonical_step_indices\":[0]"));
        assert!(dump.contains("\"matching_face_indices\":[0]"));
    }

    #[test]
    fn raised_step_face_debug_reports_final_owned_boundary_source() {
        let mut piece = empty_node_piece();
        let boundary_segment = NodeExplicitVerticalStepSegment::new(
            NodeArrangementKey::from_point(backend::RoadVec2::new(-1.0, 0.0)),
            NodeArrangementKey::from_point(backend::RoadVec2::new(1.0, 0.0)),
            NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 7),
            NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 11),
        )
        .expect("test boundary step should be non-degenerate");
        piece.owned_regions.push(NodeOwnedRegion {
            kind: RoadSurfaceBandKind::Carriageway,
            owner_index: 7,
            polygon: polygon(vec![
                Vector3::new(-1.0, 0.0, -1.0),
                Vector3::new(1.0, 0.0, -1.0),
                Vector3::new(1.0, 0.0, 0.0),
                Vector3::new(-1.0, 0.0, 0.0),
            ]),
        });
        piece.owned_regions.push(NodeOwnedRegion {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
            owner_index: 11,
            polygon: polygon(vec![
                Vector3::new(-1.0, 0.12, 0.0),
                Vector3::new(1.0, 0.12, 0.0),
                Vector3::new(1.0, 0.12, 1.0),
                Vector3::new(-1.0, 0.12, 1.0),
            ]),
        });
        piece.raised_step_face_polygons.push(polygon(vec![
            Vector3::new(-1.0, 0.12, 0.0),
            Vector3::new(-1.0, 0.0, 0.0),
            Vector3::new(1.0, 0.0, 0.0),
            Vector3::new(1.0, 0.12, 0.0),
        ]));
        piece
            .raised_step_face_sources
            .push(RoadSurfaceVerticalFaceSource::FinalOwnedBoundary {
                segment: boundary_segment,
            });

        let mut dump = String::new();
        RoadSurfaceSystem::append_raised_step_face_details_debug_literal(&mut dump, &piece);

        assert!(dump.contains("\"face_count\":1"));
        assert!(dump.contains("\"problem_count\":0"));
        assert!(dump.contains("\"source_kind\":\"final_owned_boundary\""));
        assert!(dump.contains("\"source_explicit_vertical_step_index\":null"));
        assert!(dump.contains("\"source_owner_pair\":{\"owner\":{\"kind\":\"Carriageway\",\"owner_index\":7},\"opposite_owner\":{\"kind\":\"CurbOrShoulder\",\"owner_index\":11}}"));
        assert!(dump.contains("\"source_canonical_edge_key\":{\"start\""));
        assert!(!dump.contains("18446744073709551615"));
    }

    #[test]
    fn raised_step_face_debug_matches_canonical_step_by_original_source_index() {
        let mut piece = empty_node_piece();
        let filtered_segment = NodeExplicitVerticalStepSegment::new(
            NodeArrangementKey::from_point(backend::RoadVec2::new(-2.0, 0.0)),
            NodeArrangementKey::from_point(backend::RoadVec2::new(-1.5, 0.0)),
            NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 1),
            NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2),
        )
        .expect("same-height owner segment should still be non-degenerate");
        let canonical_segment = NodeExplicitVerticalStepSegment::new(
            NodeArrangementKey::from_point(backend::RoadVec2::new(-1.0, 0.0)),
            NodeArrangementKey::from_point(backend::RoadVec2::new(1.0, 0.0)),
            NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 7),
            NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 11),
        )
        .expect("test step should be non-degenerate");
        piece.explicit_vertical_step_segments.push(filtered_segment);
        piece
            .explicit_vertical_step_segments
            .push(canonical_segment);
        piece.owned_regions.push(NodeOwnedRegion {
            kind: RoadSurfaceBandKind::Carriageway,
            owner_index: 7,
            polygon: polygon(vec![
                Vector3::new(-1.0, 0.0, -1.0),
                Vector3::new(1.0, 0.0, -1.0),
                Vector3::new(1.0, 0.0, 0.0),
                Vector3::new(-1.0, 0.0, 0.0),
            ]),
        });
        piece.owned_regions.push(NodeOwnedRegion {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
            owner_index: 11,
            polygon: polygon(vec![
                Vector3::new(-1.0, 0.12, 0.0),
                Vector3::new(1.0, 0.12, 0.0),
                Vector3::new(1.0, 0.12, 1.0),
                Vector3::new(-1.0, 0.12, 1.0),
            ]),
        });
        piece.raised_step_face_polygons.push(polygon(vec![
            Vector3::new(-1.0, 0.12, 0.0),
            Vector3::new(-1.0, 0.0, 0.0),
            Vector3::new(1.0, 0.0, 0.0),
            Vector3::new(1.0, 0.12, 0.0),
        ]));
        piece
            .raised_step_face_sources
            .push(RoadSurfaceVerticalFaceSource::CanonicalStep {
                explicit_vertical_step_index: 1,
                segment: canonical_segment,
            });

        let mut dump = String::new();
        RoadSurfaceSystem::append_raised_step_face_details_debug_literal(&mut dump, &piece);

        assert!(dump.contains("\"source_constraint_count\":1"));
        assert!(dump.contains("\"canonical_raised_step_problem_count\":0"));
        assert!(dump.contains("\"problem_count\":0"));
        assert!(dump.contains("\"matching_canonical_step_indices\":[0]"));
        assert!(dump.contains("\"explicit_vertical_step_index\":1"));
        assert!(dump.contains("\"materialization_status\":\"materialized\""));
    }

    #[test]
    fn raised_step_debug_reports_non_exposed_source_constraints_without_failing_final_faces() {
        let mut piece = empty_node_piece();
        let canonical_segment = NodeExplicitVerticalStepSegment::new(
            NodeArrangementKey::from_point(backend::RoadVec2::new(-1.0, 0.0)),
            NodeArrangementKey::from_point(backend::RoadVec2::new(1.0, 0.0)),
            NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 7),
            NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 11),
        )
        .expect("test step should be non-degenerate");
        piece
            .explicit_vertical_step_segments
            .push(canonical_segment);

        let mut dump = String::new();
        RoadSurfaceSystem::append_raised_step_face_details_debug_literal(&mut dump, &piece);

        assert!(dump.contains("\"source_constraint_count\":1"));
        assert!(dump.contains("\"final_required_face_count\":0"));
        assert!(dump.contains("\"non_exposed_source_constraint_count\":1"));
        assert!(dump.contains("\"canonical_raised_step_problem_count\":0"));
        assert!(dump.contains("\"problem_count\":0"));
        assert!(
            dump.contains("\"materialization_status\":\"not_exposed_after_boolean_ownership\"")
        );
    }
}
