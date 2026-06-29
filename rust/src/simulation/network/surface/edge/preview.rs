//! Temporary road preview compilation from conditioned edge input.

use super::super::backend::road_vec3_to_godot;
use super::super::{
    RoadSurfaceSection, RoadSurfaceSystem, RoadSurfaceVisualNodePiece, SAMPLE_EPSILON_M,
};
use super::input::{
    PREVIEW_CLEARANCE_M, PreparedRoadInput, ROAD_PROFILE_MAX_GRADE, RoadExtensionReprofile,
};
use crate::config;
use crate::simulation::network::build_surface_edge;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::{EdgeClass, NodeType};
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::Vector3;
use std::collections::{HashMap, HashSet};

// Preview validation thresholds.
const PREVIEW_MAX_GRADE: f32 = ROAD_PROFILE_MAX_GRADE + 0.001;
const PREVIEW_VALID_REASON: &str = "";
const PREVIEW_TOO_STEEP_REASON: &str = "too_steep";
const PREVIEW_BRIDGE_CLEARANCE_REASON: &str = "bridge_clearance";
const PREVIEW_TUNNEL_CLEARANCE_REASON: &str = "tunnel_clearance";
const PREVIEW_SURFACE_GEOMETRY_REASON: &str = "surface_geometry_invalid";

/// Machine-readable road-preview validation state shared by the UI and commit guard.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadPreviewValidation {
    /// True when the preview can be placed as a road under the current road-profile rules.
    pub is_valid: bool,
    /// Stable reason key for invalid previews, or an empty string when valid.
    pub invalid_reason: &'static str,
    /// Largest compiled centerline grade seen in the preview.
    pub max_grade: f32,
    /// Player-facing centerline grade limit for standard roads.
    pub allowed_grade: f32,
    /// Start station of the steepest invalid span, in meters along the road.
    pub offending_span_start_m: f32,
    /// End station of the steepest invalid span, in meters along the road.
    pub offending_span_end_m: f32,
    /// Horizontal length of the steepest span, in metres.
    pub offending_span_run_m: f32,
    /// Signed height delta across the steepest span, in metres.
    pub offending_span_height_delta_m: f32,
    /// Road center height at the start of the steepest span.
    pub offending_span_start_height_m: f32,
    /// Road center height at the end of the steepest span.
    pub offending_span_end_height_m: f32,
    /// Source terrain height at the start of the steepest span.
    pub offending_span_start_terrain_height_m: f32,
    /// Source terrain height at the end of the steepest span.
    pub offending_span_end_terrain_height_m: f32,
    /// Road-minus-terrain delta at the start of the steepest span.
    pub offending_span_start_support_delta_m: f32,
    /// Road-minus-terrain delta at the end of the steepest span.
    pub offending_span_end_support_delta_m: f32,
    /// Existing graph node snapped to the start endpoint, or -1 when the start is open terrain.
    pub start_endpoint_snapped_node_id: i32,
    /// Existing graph node snapped to the end endpoint, or -1 when the end is open terrain.
    pub end_endpoint_snapped_node_id: i32,
    /// Road height at the start endpoint after profile preparation and endpoint snapping.
    pub start_endpoint_height_m: f32,
    /// Road height at the end endpoint after profile preparation and endpoint snapping.
    pub end_endpoint_height_m: f32,
    /// Source terrain height below the start endpoint.
    pub start_endpoint_terrain_height_m: f32,
    /// Source terrain height below the end endpoint.
    pub end_endpoint_terrain_height_m: f32,
    /// Start endpoint road-minus-terrain delta; positive means fill.
    pub start_endpoint_support_delta_m: f32,
    /// End endpoint road-minus-terrain delta; positive means fill.
    pub end_endpoint_support_delta_m: f32,
    /// Measured bridge/tunnel clearance at the sampled validation point.
    pub clearance_m: f32,
    /// Required bridge/tunnel clearance at the sampled validation point.
    pub required_clearance_m: f32,
}

/// Temporary preview compile output for one road-tool stroke.
#[derive(Clone, Debug, PartialEq)]
pub struct PreviewRoadSurfaceResult {
    /// Edge class inferred from the preview stroke before temporary compilation.
    pub edge_class: EdgeClass,
    /// Prepared physical centerline points after the same vertical-profile solve used by committed
    /// placement.
    pub prepared_points: Vec<Vector3>,
    /// Compiled section cache for the temporary preview edge.
    pub compiled_sections: Vec<RoadSurfaceSection>,
    /// Explicit visual node pieces for the temporary preview edge endpoints.
    pub compiled_visual_node_pieces: Vec<RoadSurfaceVisualNodePiece>,
    /// Triangulated top-surface preview mesh vertices from the solved section geometry.
    pub surface_vertices: Vec<Vector3>,
    /// Detailed validity state and machine-readable invalid reason.
    pub validation: RoadPreviewValidation,
    /// Preview validity after grade and bridge / tunnel clearance checks.
    pub is_valid: bool,
}

impl RoadSurfaceSystem {
    /// Compiles one temporary road preview using the same point conditioning and section compiler
    /// as committed placement while keeping preview cache lifetime transient.
    pub fn compile_preview_surface(
        &self,
        raw_points: &[Vector3],
        fwd_lanes: u8,
        bkw_lanes: u8,
        terrain: &TerrainSystem,
    ) -> PreviewRoadSurfaceResult {
        let (prepared_points, edge_class) = Self::prepare_road_input_points(raw_points, terrain);

        if prepared_points.len() < 2 {
            return PreviewRoadSurfaceResult {
                edge_class,
                prepared_points,
                compiled_sections: Vec::new(),
                compiled_visual_node_pieces: Vec::new(),
                surface_vertices: Vec::new(),
                validation: RoadPreviewValidation::valid(0.0),
                is_valid: true,
            };
        }

        let mut graph = RegionGraph::new();
        let start_node = graph.add_node(prepared_points[0], NodeType::Junction);
        let end_node = graph.add_node(*prepared_points.last().unwrap(), NodeType::Junction);
        let edge_idx = graph.add_edge(build_surface_edge(
            start_node,
            end_node,
            prepared_points.clone(),
            fwd_lanes,
            bkw_lanes,
            edge_class,
        ));

        let mut preview_surface = RoadSurfaceSystem::new(self.chunk_span_m);
        preview_surface.node_validation_logging_enabled = false;
        preview_surface.compile_dirty(&graph, terrain);

        let compiled_sections = preview_surface
            .compiled_sections()
            .get(&edge_idx)
            .cloned()
            .unwrap_or_default();
        let compiled_visual_node_pieces = [start_node, end_node]
            .into_iter()
            .filter_map(|node_id| {
                preview_surface
                    .compiled_visual_node_pieces()
                    .get(&node_id)
                    .cloned()
            })
            .collect();
        let surface_vertices = self.build_preview_surface_vertices(&compiled_sections);
        let validation = Self::preview_surface_validation(
            edge_class,
            &prepared_points,
            &compiled_sections,
            terrain,
        );
        let is_valid = validation.is_valid;

        PreviewRoadSurfaceResult {
            edge_class,
            prepared_points,
            compiled_sections,
            compiled_visual_node_pieces,
            surface_vertices,
            validation,
            is_valid,
        }
    }

    /// Compiles the lightweight editor preview mesh without building transient node pieces or
    /// chunk caches that are only needed by committed road-surface exports.
    pub fn compile_preview_surface_mesh_only(
        &self,
        raw_points: &[Vector3],
        fwd_lanes: u8,
        bkw_lanes: u8,
        terrain: &TerrainSystem,
    ) -> PreviewRoadSurfaceResult {
        let (prepared_points, edge_class) = Self::prepare_road_input_points(raw_points, terrain);
        self.compile_preview_surface_mesh_only_from_prepared(
            prepared_points,
            edge_class,
            fwd_lanes,
            bkw_lanes,
            terrain,
        )
    }

    /// Compiles the lightweight editor preview while preserving existing visible road-surface
    /// heights for snapped input endpoints.
    pub(crate) fn compile_preview_surface_mesh_only_with_existing_surface(
        &self,
        raw_points: &[Vector3],
        fwd_lanes: u8,
        bkw_lanes: u8,
        terrain: &TerrainSystem,
        existing_graph: &RegionGraph,
        existing_surface: &RoadSurfaceSystem,
    ) -> PreviewRoadSurfaceResult {
        let prepared_input = Self::prepare_road_input_with_extension_to_visible_surface(
            raw_points,
            terrain,
            existing_graph,
            existing_surface,
        );
        let mut preview = self.compile_preview_surface_mesh_only_from_prepared(
            prepared_input.points.clone(),
            prepared_input.class,
            fwd_lanes,
            bkw_lanes,
            terrain,
        );
        let validation = self.validate_prepared_road_input_against_graph(
            &prepared_input,
            fwd_lanes,
            bkw_lanes,
            terrain,
            existing_graph,
            preview.validation.clone(),
        );
        preview.is_valid = validation.is_valid;
        preview.validation = validation;
        preview
    }

    fn compile_preview_surface_mesh_only_from_prepared(
        &self,
        prepared_points: Vec<Vector3>,
        edge_class: EdgeClass,
        fwd_lanes: u8,
        bkw_lanes: u8,
        terrain: &TerrainSystem,
    ) -> PreviewRoadSurfaceResult {
        if prepared_points.len() < 2 {
            return PreviewRoadSurfaceResult {
                edge_class,
                prepared_points,
                compiled_sections: Vec::new(),
                compiled_visual_node_pieces: Vec::new(),
                surface_vertices: Vec::new(),
                validation: RoadPreviewValidation::valid(0.0),
                is_valid: true,
            };
        }

        let mut graph = RegionGraph::new();
        let start_node = graph.add_node(prepared_points[0], NodeType::Junction);
        let end_node = graph.add_node(*prepared_points.last().unwrap(), NodeType::Junction);
        let edge_idx = graph.add_edge(build_surface_edge(
            start_node,
            end_node,
            prepared_points.clone(),
            fwd_lanes,
            bkw_lanes,
            edge_class,
        ));

        let compiled_sections = self.compile_edge_sections(&graph, edge_idx);
        let surface_vertices = self.build_preview_surface_vertices(&compiled_sections);
        let validation = Self::preview_surface_validation(
            edge_class,
            &prepared_points,
            &compiled_sections,
            terrain,
        );
        let is_valid = validation.is_valid;

        PreviewRoadSurfaceResult {
            edge_class,
            prepared_points,
            compiled_sections,
            compiled_visual_node_pieces: Vec::new(),
            surface_vertices,
            validation,
            is_valid,
        }
    }

    /// Validates already prepared road geometry without building a preview mesh.
    pub(crate) fn validate_prepared_road_surface(
        &self,
        prepared_points: &[Vector3],
        edge_class: EdgeClass,
        fwd_lanes: u8,
        bkw_lanes: u8,
        terrain: &TerrainSystem,
    ) -> RoadPreviewValidation {
        if prepared_points.len() < 2 {
            return RoadPreviewValidation::valid(0.0);
        }

        let mut graph = RegionGraph::new();
        let start_node = graph.add_node(prepared_points[0], NodeType::Junction);
        let end_node = graph.add_node(*prepared_points.last().unwrap(), NodeType::Junction);
        let edge_idx = graph.add_edge(build_surface_edge(
            start_node,
            end_node,
            prepared_points.to_vec(),
            fwd_lanes,
            bkw_lanes,
            edge_class,
        ));

        let compiled_sections = self.compile_edge_sections(&graph, edge_idx);
        Self::preview_surface_validation(edge_class, prepared_points, &compiled_sections, terrain)
    }

    /// Validates already prepared road geometry against the nearby committed graph topology.
    #[cfg(test)]
    pub(crate) fn validate_prepared_road_surface_against_graph(
        &self,
        prepared_points: &[Vector3],
        edge_class: EdgeClass,
        fwd_lanes: u8,
        bkw_lanes: u8,
        terrain: &TerrainSystem,
        existing_graph: &RegionGraph,
    ) -> RoadPreviewValidation {
        let validation = self.validate_prepared_road_surface(
            prepared_points,
            edge_class,
            fwd_lanes,
            bkw_lanes,
            terrain,
        );
        self.validate_prepared_surface_geometry_against_graph(
            prepared_points,
            edge_class,
            fwd_lanes,
            bkw_lanes,
            terrain,
            existing_graph,
            validation,
        )
    }

    /// Validates prepared road input, including any terminal-extension corridor reprofile.
    pub(crate) fn validate_prepared_road_input_against_graph(
        &self,
        prepared_input: &PreparedRoadInput,
        fwd_lanes: u8,
        bkw_lanes: u8,
        terrain: &TerrainSystem,
        existing_graph: &RegionGraph,
        new_edge_validation: RoadPreviewValidation,
    ) -> RoadPreviewValidation {
        if prepared_input.validation_points == prepared_input.points {
            return self.validate_prepared_surface_geometry_against_graph_with_extension(
                &prepared_input.points,
                prepared_input.class,
                fwd_lanes,
                bkw_lanes,
                terrain,
                existing_graph,
                new_edge_validation,
                prepared_input.extension.as_ref(),
            );
        }

        let corridor_validation = self.validate_prepared_road_surface(
            &prepared_input.validation_points,
            prepared_input.class,
            fwd_lanes,
            bkw_lanes,
            terrain,
        );
        if !corridor_validation.is_valid {
            return corridor_validation;
        }

        self.validate_prepared_surface_geometry_against_graph_with_extension(
            &prepared_input.points,
            prepared_input.class,
            fwd_lanes,
            bkw_lanes,
            terrain,
            existing_graph,
            new_edge_validation,
            prepared_input.extension.as_ref(),
        )
    }

    fn record_preview_endpoint_snap_debug_with_extension(
        validation: &mut RoadPreviewValidation,
        prepared_points: &[Vector3],
        existing_graph: &RegionGraph,
        extension: Option<&RoadExtensionReprofile>,
    ) {
        let (Some(start), Some(end)) = (prepared_points.first(), prepared_points.last()) else {
            return;
        };
        validation.start_endpoint_snapped_node_id =
            Self::validation_endpoint_existing_node(*start, existing_graph, extension)
                .map(Self::debug_node_id)
                .unwrap_or(-1);
        validation.end_endpoint_snapped_node_id =
            Self::validation_endpoint_existing_node(*end, existing_graph, extension)
                .map(Self::debug_node_id)
                .unwrap_or(-1);
    }

    fn debug_node_id(node_id: u32) -> i32 {
        node_id.min(i32::MAX as u32) as i32
    }

    fn build_preview_surface_vertices(&self, sections: &[RoadSurfaceSection]) -> Vec<Vector3> {
        if sections.len() < 2 {
            return Vec::new();
        }

        let mut vertices = Vec::new();
        for pair in sections.windows(2) {
            let profile_a = self.section_profile_world_points(&pair[0]);
            let profile_b = self.section_profile_world_points(&pair[1]);
            if profile_a.len() < 2 || profile_a.len() != profile_b.len() {
                continue;
            }

            for index in 0..profile_a.len() - 1 {
                let a0 = profile_a[index];
                let a1 = profile_a[index + 1];
                let b0 = profile_b[index];
                let b1 = profile_b[index + 1];
                vertices.extend_from_slice(&[
                    road_vec3_to_godot(a0),
                    road_vec3_to_godot(b0),
                    road_vec3_to_godot(a1),
                    road_vec3_to_godot(a1),
                    road_vec3_to_godot(b0),
                    road_vec3_to_godot(b1),
                ]);
            }
        }

        vertices
    }

    fn preview_surface_validation(
        edge_class: EdgeClass,
        prepared_points: &[Vector3],
        compiled_sections: &[RoadSurfaceSection],
        terrain: &TerrainSystem,
    ) -> RoadPreviewValidation {
        let mut validation = RoadPreviewValidation::valid(0.0);
        Self::record_preview_endpoint_height_debug(&mut validation, prepared_points, terrain);
        for pair in compiled_sections.windows(2) {
            let run = (pair[1].s_m - pair[0].s_m).abs();
            if run <= SAMPLE_EPSILON_M {
                continue;
            }
            let grade = (pair[1].center_height_m - pair[0].center_height_m).abs() / run;
            if grade > validation.max_grade {
                validation.max_grade = grade;
                validation.offending_span_start_m = pair[0].s_m;
                validation.offending_span_end_m = pair[1].s_m;
                validation.offending_span_run_m = run;
                validation.offending_span_height_delta_m =
                    pair[1].center_height_m - pair[0].center_height_m;
                validation.offending_span_start_height_m = pair[0].center_height_m;
                validation.offending_span_end_height_m = pair[1].center_height_m;
                validation.offending_span_start_terrain_height_m = Self::preview_terrain_height_m(
                    terrain,
                    pair[0].center_xz.x as f32,
                    pair[0].center_xz.y as f32,
                );
                validation.offending_span_end_terrain_height_m = Self::preview_terrain_height_m(
                    terrain,
                    pair[1].center_xz.x as f32,
                    pair[1].center_xz.y as f32,
                );
                validation.offending_span_start_support_delta_m =
                    pair[0].center_height_m - validation.offending_span_start_terrain_height_m;
                validation.offending_span_end_support_delta_m =
                    pair[1].center_height_m - validation.offending_span_end_terrain_height_m;
            }
        }
        if validation.max_grade > PREVIEW_MAX_GRADE {
            validation.is_valid = false;
            validation.invalid_reason = PREVIEW_TOO_STEEP_REASON;
            return validation;
        }

        if prepared_points.len() > 2 {
            if let Some(mid_section) = compiled_sections.get(compiled_sections.len() / 2) {
                let terrain_h = Self::preview_terrain_height_m(
                    terrain,
                    mid_section.center_xz.x as f32,
                    mid_section.center_xz.y as f32,
                );
                match edge_class {
                    EdgeClass::Bridge => {
                        validation.clearance_m = mid_section.center_height_m - terrain_h;
                        validation.required_clearance_m = PREVIEW_CLEARANCE_M;
                        if validation.clearance_m < PREVIEW_CLEARANCE_M {
                            validation.is_valid = false;
                            validation.invalid_reason = PREVIEW_BRIDGE_CLEARANCE_REASON;
                            return validation;
                        }
                    }
                    EdgeClass::Tunnel => {
                        validation.clearance_m = terrain_h - mid_section.center_height_m;
                        validation.required_clearance_m = PREVIEW_CLEARANCE_M;
                        if validation.clearance_m < PREVIEW_CLEARANCE_M {
                            validation.is_valid = false;
                            validation.invalid_reason = PREVIEW_TUNNEL_CLEARANCE_REASON;
                            return validation;
                        }
                    }
                    EdgeClass::Standard => {}
                }
            }
        }

        validation
    }

    fn preview_terrain_height_m(terrain: &TerrainSystem, x: f32, z: f32) -> f32 {
        terrain.sample_height_world(x, z) * config::HEIGHT_SCALE
    }

    fn record_preview_endpoint_height_debug(
        validation: &mut RoadPreviewValidation,
        prepared_points: &[Vector3],
        terrain: &TerrainSystem,
    ) {
        let (Some(start), Some(end)) = (prepared_points.first(), prepared_points.last()) else {
            return;
        };
        validation.start_endpoint_height_m = start.y;
        validation.end_endpoint_height_m = end.y;
        validation.start_endpoint_terrain_height_m =
            Self::preview_terrain_height_m(terrain, start.x, start.z);
        validation.end_endpoint_terrain_height_m =
            Self::preview_terrain_height_m(terrain, end.x, end.z);
        validation.start_endpoint_support_delta_m =
            start.y - validation.start_endpoint_terrain_height_m;
        validation.end_endpoint_support_delta_m = end.y - validation.end_endpoint_terrain_height_m;
    }

    #[cfg(test)]
    fn validate_prepared_surface_geometry_against_graph(
        &self,
        prepared_points: &[Vector3],
        edge_class: EdgeClass,
        fwd_lanes: u8,
        bkw_lanes: u8,
        terrain: &TerrainSystem,
        existing_graph: &RegionGraph,
        validation: RoadPreviewValidation,
    ) -> RoadPreviewValidation {
        self.validate_prepared_surface_geometry_against_graph_with_extension(
            prepared_points,
            edge_class,
            fwd_lanes,
            bkw_lanes,
            terrain,
            existing_graph,
            validation,
            None,
        )
    }

    fn validate_prepared_surface_geometry_against_graph_with_extension(
        &self,
        prepared_points: &[Vector3],
        edge_class: EdgeClass,
        fwd_lanes: u8,
        bkw_lanes: u8,
        terrain: &TerrainSystem,
        existing_graph: &RegionGraph,
        validation: RoadPreviewValidation,
        extension: Option<&RoadExtensionReprofile>,
    ) -> RoadPreviewValidation {
        let mut validation = validation;
        Self::record_preview_endpoint_snap_debug_with_extension(
            &mut validation,
            prepared_points,
            existing_graph,
            extension,
        );
        if !validation.is_valid || prepared_points.len() < 2 {
            return validation;
        }

        let Some((validation_graph, new_edge_idx, endpoint_nodes)) =
            Self::build_surface_validation_graph(
                prepared_points,
                edge_class,
                fwd_lanes,
                bkw_lanes,
                existing_graph,
                extension,
            )
        else {
            return validation.with_invalid_reason(PREVIEW_SURFACE_GEOMETRY_REASON);
        };

        let mut validation_surface = RoadSurfaceSystem::new(self.chunk_span_m);
        validation_surface.node_validation_logging_enabled = false;
        validation_surface.compile_dirty(&validation_graph, terrain);

        if !validation_surface
            .compiled_visual_span_pieces()
            .contains_key(&new_edge_idx)
        {
            return validation.with_invalid_reason(PREVIEW_SURFACE_GEOMETRY_REASON);
        }

        for node_id in endpoint_nodes {
            let Some(kind) = validation_surface
                .classify_surface_node_kind_from_graph_geometry(&validation_graph, node_id)
            else {
                continue;
            };
            match kind {
                super::super::CompiledNodeKind::PassThrough => {}
                super::super::CompiledNodeKind::Terminal
                | super::super::CompiledNodeKind::Bend
                | super::super::CompiledNodeKind::JunctionN => {
                    if !validation_surface
                        .compiled_visual_node_pieces()
                        .contains_key(&node_id)
                    {
                        return validation.with_invalid_reason(PREVIEW_SURFACE_GEOMETRY_REASON);
                    }
                }
            }
        }

        validation
    }

    fn build_surface_validation_graph(
        prepared_points: &[Vector3],
        edge_class: EdgeClass,
        fwd_lanes: u8,
        bkw_lanes: u8,
        existing_graph: &RegionGraph,
        extension: Option<&RoadExtensionReprofile>,
    ) -> Option<(RegionGraph, usize, [u32; 2])> {
        if prepared_points.len() < 2 {
            return None;
        }

        let mut validation_graph = RegionGraph::new();
        let mut node_map = HashMap::new();
        let mut copied_edges = HashSet::new();
        let start_existing =
            Self::validation_endpoint_existing_node(prepared_points[0], existing_graph, extension);
        let end_existing = Self::validation_endpoint_existing_node(
            *prepared_points.last().unwrap(),
            existing_graph,
            extension,
        );

        let start_node = Self::validation_graph_endpoint_node(
            &mut validation_graph,
            existing_graph,
            &mut node_map,
            start_existing,
            prepared_points[0],
            extension,
        );
        let end_node = Self::validation_graph_endpoint_node(
            &mut validation_graph,
            existing_graph,
            &mut node_map,
            end_existing,
            *prepared_points.last().unwrap(),
            extension,
        );
        if start_node == end_node {
            return None;
        }

        for existing_node in [start_existing, end_existing].into_iter().flatten() {
            Self::copy_validation_graph_incident_edges(
                &mut validation_graph,
                existing_graph,
                &mut node_map,
                &mut copied_edges,
                existing_node,
                extension,
            );
        }

        let mut points = prepared_points.to_vec();
        points[0] = validation_graph.node(start_node).pos;
        let last_idx = points.len() - 1;
        points[last_idx] = validation_graph.node(end_node).pos;
        let new_edge_idx = validation_graph.add_edge(build_surface_edge(
            start_node, end_node, points, fwd_lanes, bkw_lanes, edge_class,
        ));

        Some((validation_graph, new_edge_idx, [start_node, end_node]))
    }

    fn validation_graph_endpoint_node(
        validation_graph: &mut RegionGraph,
        existing_graph: &RegionGraph,
        node_map: &mut HashMap<u32, u32>,
        existing_node: Option<u32>,
        fallback_pos: Vector3,
        extension: Option<&RoadExtensionReprofile>,
    ) -> u32 {
        match existing_node {
            Some(node_id) => Self::copy_validation_graph_node(
                validation_graph,
                existing_graph,
                node_map,
                node_id,
                extension,
            ),
            None => validation_graph.add_node(fallback_pos, NodeType::Junction),
        }
    }

    fn copy_validation_graph_node(
        validation_graph: &mut RegionGraph,
        existing_graph: &RegionGraph,
        node_map: &mut HashMap<u32, u32>,
        node_id: u32,
        extension: Option<&RoadExtensionReprofile>,
    ) -> u32 {
        let valid = existing_graph.get_valid_node(node_id);
        if let Some(&mapped) = node_map.get(&valid) {
            return mapped;
        }

        let node = existing_graph.node(valid);
        let pos = extension
            .filter(|extension| existing_graph.get_valid_node(extension.snapped_node_id) == valid)
            .map_or(node.pos, |extension| extension.snapped_node_pos);
        let mapped = validation_graph.add_node(pos, node.node_type);
        node_map.insert(valid, mapped);
        mapped
    }

    fn copy_validation_graph_incident_edges(
        validation_graph: &mut RegionGraph,
        existing_graph: &RegionGraph,
        node_map: &mut HashMap<u32, u32>,
        copied_edges: &mut HashSet<usize>,
        node_id: u32,
        extension: Option<&RoadExtensionReprofile>,
    ) {
        let valid = existing_graph.get_valid_node(node_id);
        if valid as usize >= existing_graph.node_adjacency_count() {
            return;
        }

        for &edge_idx in existing_graph.node_adjacency(valid) {
            if !copied_edges.insert(edge_idx) || edge_idx >= existing_graph.edge_count() {
                continue;
            }
            let edge = existing_graph.edge(edge_idx);
            if !Self::is_surface_edge(edge) {
                continue;
            }

            let start_node = Self::copy_validation_graph_node(
                validation_graph,
                existing_graph,
                node_map,
                edge.start_node,
                extension,
            );
            let end_node = Self::copy_validation_graph_node(
                validation_graph,
                existing_graph,
                node_map,
                edge.end_node,
                extension,
            );
            if start_node == end_node {
                continue;
            }

            let mut copied_edge = edge.clone();
            copied_edge.start_node = start_node;
            copied_edge.end_node = end_node;
            if let Some(extension) = extension
                && edge_idx == extension.existing_edge_idx
            {
                copied_edge.geometry = extension.existing_points.clone();
                copied_edge.physical_geometry = extension.existing_points.clone();
                copied_edge.physical_length =
                    validation_graph.calculate_length(&copied_edge.physical_geometry);
            }
            validation_graph.add_edge(copied_edge);
        }
    }

    fn validation_endpoint_existing_node(
        point: Vector3,
        existing_graph: &RegionGraph,
        extension: Option<&RoadExtensionReprofile>,
    ) -> Option<u32> {
        if let Some(extension) = extension {
            let node_pos = existing_graph.node(extension.snapped_node_id).pos;
            let dx = point.x - node_pos.x;
            let dz = point.z - node_pos.z;
            if dx.hypot(dz) < config::SNAP_TOLERANCE {
                return Some(extension.snapped_node_id);
            }
        }
        existing_graph.find_node_within(point, config::SNAP_TOLERANCE)
    }
}

impl RoadPreviewValidation {
    fn valid(max_grade: f32) -> Self {
        Self {
            is_valid: true,
            invalid_reason: PREVIEW_VALID_REASON,
            max_grade,
            allowed_grade: ROAD_PROFILE_MAX_GRADE,
            offending_span_start_m: 0.0,
            offending_span_end_m: 0.0,
            offending_span_run_m: 0.0,
            offending_span_height_delta_m: 0.0,
            offending_span_start_height_m: 0.0,
            offending_span_end_height_m: 0.0,
            offending_span_start_terrain_height_m: 0.0,
            offending_span_end_terrain_height_m: 0.0,
            offending_span_start_support_delta_m: 0.0,
            offending_span_end_support_delta_m: 0.0,
            start_endpoint_snapped_node_id: -1,
            end_endpoint_snapped_node_id: -1,
            start_endpoint_height_m: 0.0,
            end_endpoint_height_m: 0.0,
            start_endpoint_terrain_height_m: 0.0,
            end_endpoint_terrain_height_m: 0.0,
            start_endpoint_support_delta_m: 0.0,
            end_endpoint_support_delta_m: 0.0,
            clearance_m: 0.0,
            required_clearance_m: 0.0,
        }
    }

    fn with_invalid_reason(mut self, invalid_reason: &'static str) -> Self {
        self.is_valid = false;
        self.invalid_reason = invalid_reason;
        self
    }
}
