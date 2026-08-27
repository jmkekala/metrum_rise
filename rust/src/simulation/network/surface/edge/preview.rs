//! Temporary road preview compilation from conditioned edge input.

use super::super::backend::road_vec3_to_godot;
use super::super::{
    RoadSurfaceCompileReason, RoadSurfaceSection, RoadSurfaceSystem, RoadSurfaceVisualNodePiece,
    SAMPLE_EPSILON_M,
};
use super::input::{
    PREVIEW_CLEARANCE_M, PreparedRoadInput, ROAD_PROFILE_MAX_GRADE, RoadExtensionReprofile,
};
use crate::config;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::{EdgeClass, NodeType};
use crate::simulation::network::{TransitNetwork, build_surface_edge, topology};
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::zoning::ZoningSystem;
use godot::prelude::Vector3;
use std::collections::{HashMap, HashSet};

const PREVIEW_VALID_REASON: &str = "";
const PREVIEW_BRIDGE_CLEARANCE_REASON: &str = "bridge_clearance";
const PREVIEW_TUNNEL_CLEARANCE_REASON: &str = "tunnel_clearance";
const PREVIEW_SURFACE_GEOMETRY_REASON: &str = "surface_geometry_invalid";
const PREVIEW_TOO_SHORT_REASON: &str = "too_short";
const PREVIEW_MIN_ENDPOINT_SEGMENT_M: f32 = 2.0;
const PREVIEW_BRIDGE_GROUND_TOLERANCE_M: f32 = 0.05;

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

#[derive(Clone, Debug, Eq, PartialEq)]
enum SurfaceValidationFailure {
    MissingSpans(Vec<usize>),
    MissingNodes(Vec<u32>),
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
        preview_surface.compile_dirty_with_reason(
            &graph,
            terrain,
            RoadSurfaceCompileReason::PreviewWorker,
        );

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
        self.compile_preview_surface_mesh_only_with_existing_surface_snap(
            raw_points,
            fwd_lanes,
            bkw_lanes,
            terrain,
            existing_graph,
            existing_surface,
            true,
        )
    }

    pub(crate) fn compile_preview_surface_mesh_only_with_existing_surface_snap(
        &self,
        raw_points: &[Vector3],
        fwd_lanes: u8,
        bkw_lanes: u8,
        terrain: &TerrainSystem,
        existing_graph: &RegionGraph,
        existing_surface: &RoadSurfaceSystem,
        snap_to_existing_roads: bool,
    ) -> PreviewRoadSurfaceResult {
        let prepared_input = Self::prepare_road_input_for_tool(
            raw_points,
            terrain,
            existing_graph,
            existing_surface,
            snap_to_existing_roads,
        );
        let mut preview = self.compile_preview_surface_mesh_only_from_prepared(
            prepared_input.points.clone(),
            prepared_input.class,
            fwd_lanes,
            bkw_lanes,
            terrain,
        );
        let validation = self.validate_prepared_road_input_against_graph_with_compile_reason(
            &prepared_input,
            fwd_lanes,
            bkw_lanes,
            terrain,
            existing_graph,
            preview.validation,
            RoadSurfaceCompileReason::PreviewWorker,
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

    pub(crate) fn validate_prepared_road_input_against_graph_with_compile_reason(
        &self,
        prepared_input: &PreparedRoadInput,
        fwd_lanes: u8,
        bkw_lanes: u8,
        terrain: &TerrainSystem,
        existing_graph: &RegionGraph,
        new_edge_validation: RoadPreviewValidation,
        compile_reason: RoadSurfaceCompileReason,
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
                prepared_input.endpoint_snap_enabled,
                compile_reason,
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
            prepared_input.endpoint_snap_enabled,
            compile_reason,
        )
    }

    /// Validates a road-tool candidate using only prepared-profile samples and local endpoint
    /// graph checks. This path is intended for synchronous editor feedback, so it must not compile
    /// temporary node surfaces or preview meshes.
    pub(crate) fn validate_prepared_road_candidate_fast(
        &self,
        prepared_input: &PreparedRoadInput,
        _fwd_lanes: u8,
        _bkw_lanes: u8,
        terrain: &TerrainSystem,
        existing_graph: &RegionGraph,
    ) -> RoadPreviewValidation {
        let mut validation = Self::fast_prepared_profile_validation(
            prepared_input.class,
            &prepared_input.validation_points,
            terrain,
        );
        Self::record_preview_endpoint_height_debug(
            &mut validation,
            &prepared_input.points,
            terrain,
        );
        Self::record_preview_endpoint_snap_debug_with_extension(
            &mut validation,
            &prepared_input.points,
            existing_graph,
            prepared_input.extension.as_ref(),
            prepared_input.endpoint_snap_enabled,
        );
        if !validation.is_valid || prepared_input.points.len() < 2 {
            return validation;
        }

        self.validate_candidate_endpoint_geometry_fast(
            &prepared_input.points,
            existing_graph,
            prepared_input.extension.as_ref(),
            prepared_input.endpoint_snap_enabled,
            validation,
        )
    }

    fn fast_prepared_profile_validation(
        edge_class: EdgeClass,
        prepared_points: &[Vector3],
        terrain: &TerrainSystem,
    ) -> RoadPreviewValidation {
        let mut validation = RoadPreviewValidation::valid(0.0);
        Self::record_preview_endpoint_height_debug(&mut validation, prepared_points, terrain);
        if prepared_points.len() < 2 {
            return validation.with_invalid_reason(PREVIEW_TOO_SHORT_REASON);
        }

        let mut station_m = 0.0;
        for pair in prepared_points.windows(2) {
            let run = (pair[1].x - pair[0].x).hypot(pair[1].z - pair[0].z);
            if run <= SAMPLE_EPSILON_M {
                continue;
            }
            let next_station_m = station_m + run;
            let grade = (pair[1].y - pair[0].y).abs() / run;
            if grade > validation.max_grade {
                validation.max_grade = grade;
                validation.offending_span_start_m = station_m;
                validation.offending_span_end_m = next_station_m;
                validation.offending_span_run_m = run;
                validation.offending_span_height_delta_m = pair[1].y - pair[0].y;
                validation.offending_span_start_height_m = pair[0].y;
                validation.offending_span_end_height_m = pair[1].y;
                validation.offending_span_start_terrain_height_m =
                    Self::preview_terrain_height_m(terrain, pair[0].x, pair[0].z);
                validation.offending_span_end_terrain_height_m =
                    Self::preview_terrain_height_m(terrain, pair[1].x, pair[1].z);
                validation.offending_span_start_support_delta_m =
                    pair[0].y - validation.offending_span_start_terrain_height_m;
                validation.offending_span_end_support_delta_m =
                    pair[1].y - validation.offending_span_end_terrain_height_m;
            }
            station_m = next_station_m;
        }

        if station_m < PREVIEW_MIN_ENDPOINT_SEGMENT_M {
            return validation.with_invalid_reason(PREVIEW_TOO_SHORT_REASON);
        }
        if edge_class == EdgeClass::Bridge
            && Self::bridge_profile_is_ground_transition(prepared_points, terrain)
        {
            validation.clearance_m = prepared_points
                .iter()
                .map(|point| point.y - Self::preview_terrain_height_m(terrain, point.x, point.z))
                .fold(f32::INFINITY, f32::min);
            validation.required_clearance_m = 0.0;
            if validation.clearance_m < -PREVIEW_BRIDGE_GROUND_TOLERANCE_M {
                return validation.with_invalid_reason(PREVIEW_BRIDGE_CLEARANCE_REASON);
            }
        } else if prepared_points.len() > 2 {
            let mid = prepared_points[prepared_points.len() / 2];
            let terrain_h = Self::preview_terrain_height_m(terrain, mid.x, mid.z);
            match edge_class {
                EdgeClass::Bridge => {
                    validation.clearance_m = mid.y - terrain_h;
                    validation.required_clearance_m = PREVIEW_CLEARANCE_M;
                    if validation.clearance_m < PREVIEW_CLEARANCE_M {
                        return validation.with_invalid_reason(PREVIEW_BRIDGE_CLEARANCE_REASON);
                    }
                }
                EdgeClass::Tunnel => {
                    validation.clearance_m = terrain_h - mid.y;
                    validation.required_clearance_m = PREVIEW_CLEARANCE_M;
                    if validation.clearance_m < PREVIEW_CLEARANCE_M {
                        return validation.with_invalid_reason(PREVIEW_TUNNEL_CLEARANCE_REASON);
                    }
                }
                EdgeClass::Standard => {}
            }
        }

        validation
    }

    fn validate_candidate_endpoint_geometry_fast(
        &self,
        prepared_points: &[Vector3],
        existing_graph: &RegionGraph,
        extension: Option<&RoadExtensionReprofile>,
        endpoint_snap_enabled: bool,
        validation: RoadPreviewValidation,
    ) -> RoadPreviewValidation {
        let Some(first) = prepared_points.first().copied() else {
            return validation.with_invalid_reason(PREVIEW_TOO_SHORT_REASON);
        };
        let Some(last) = prepared_points.last().copied() else {
            return validation.with_invalid_reason(PREVIEW_TOO_SHORT_REASON);
        };

        let start_existing = Self::validation_endpoint_existing_node(
            first,
            existing_graph,
            extension,
            endpoint_snap_enabled,
        );
        let end_existing = Self::validation_endpoint_existing_node(
            last,
            existing_graph,
            extension,
            endpoint_snap_enabled,
        );
        if start_existing.is_some() && start_existing == end_existing {
            return validation.with_invalid_reason(PREVIEW_SURFACE_GEOMETRY_REASON);
        }

        validation
    }

    /// Returns the visible roadbed half-width for one not-yet-committed candidate.
    pub(crate) fn preview_candidate_roadbed_half_width_m(
        prepared_points: &[Vector3],
        edge_class: EdgeClass,
        fwd_lanes: u8,
        bkw_lanes: u8,
    ) -> f32 {
        let points = match (prepared_points.first(), prepared_points.last()) {
            (Some(first), Some(last)) => vec![*first, *last],
            _ => Vec::new(),
        };
        let edge = build_surface_edge(0, 1, points, fwd_lanes, bkw_lanes, edge_class);
        Self::visual_roadbed_half_width_m(&edge)
    }

    fn record_preview_endpoint_snap_debug_with_extension(
        validation: &mut RoadPreviewValidation,
        prepared_points: &[Vector3],
        existing_graph: &RegionGraph,
        extension: Option<&RoadExtensionReprofile>,
        endpoint_snap_enabled: bool,
    ) {
        let (Some(start), Some(end)) = (prepared_points.first(), prepared_points.last()) else {
            return;
        };
        validation.start_endpoint_snapped_node_id = Self::validation_endpoint_existing_node(
            *start,
            existing_graph,
            extension,
            endpoint_snap_enabled,
        )
        .map(Self::debug_node_id)
        .unwrap_or(-1);
        validation.end_endpoint_snapped_node_id = Self::validation_endpoint_existing_node(
            *end,
            existing_graph,
            extension,
            endpoint_snap_enabled,
        )
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
        if edge_class == EdgeClass::Bridge
            && Self::bridge_profile_is_ground_transition(prepared_points, terrain)
        {
            validation.clearance_m = compiled_sections
                .iter()
                .map(|section| {
                    section.center_height_m
                        - Self::preview_terrain_height_m(
                            terrain,
                            section.center_xz.x as f32,
                            section.center_xz.y as f32,
                        )
                })
                .fold(f32::INFINITY, f32::min);
            validation.required_clearance_m = 0.0;
            if validation.clearance_m < -PREVIEW_BRIDGE_GROUND_TOLERANCE_M {
                validation.is_valid = false;
                validation.invalid_reason = PREVIEW_BRIDGE_CLEARANCE_REASON;
                return validation;
            }
        } else if prepared_points.len() > 2 {
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

    fn bridge_profile_is_ground_transition(
        prepared_points: &[Vector3],
        terrain: &TerrainSystem,
    ) -> bool {
        let (Some(first), Some(last)) = (prepared_points.first(), prepared_points.last()) else {
            return false;
        };
        let first_clearance = first.y - Self::preview_terrain_height_m(terrain, first.x, first.z);
        let last_clearance = last.y - Self::preview_terrain_height_m(terrain, last.x, last.z);
        (first_clearance.abs() <= PREVIEW_CLEARANCE_M && last_clearance > PREVIEW_CLEARANCE_M)
            || (last_clearance.abs() <= PREVIEW_CLEARANCE_M
                && first_clearance > PREVIEW_CLEARANCE_M)
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
            true,
            RoadSurfaceCompileReason::CommitValidator,
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
        endpoint_snap_enabled: bool,
        compile_reason: RoadSurfaceCompileReason,
    ) -> RoadPreviewValidation {
        let mut validation = validation;
        Self::record_preview_endpoint_snap_debug_with_extension(
            &mut validation,
            prepared_points,
            existing_graph,
            extension,
            endpoint_snap_enabled,
        );
        if !validation.is_valid || prepared_points.len() < 2 {
            return validation;
        }

        let Some((validation_graph, new_edge_idx, required_edge_ids, required_node_ids)) = self
            .build_surface_validation_graph(
                prepared_points,
                edge_class,
                fwd_lanes,
                bkw_lanes,
                existing_graph,
                extension,
                endpoint_snap_enabled,
            )
        else {
            return validation.with_invalid_reason(PREVIEW_SURFACE_GEOMETRY_REASON);
        };
        if new_edge_idx >= validation_graph.edge_count()
            || validation_graph.edge(new_edge_idx).deleted
            || required_edge_ids.is_empty()
        {
            return validation.with_invalid_reason(PREVIEW_SURFACE_GEOMETRY_REASON);
        }

        let mut validation_surface = RoadSurfaceSystem::new(self.chunk_span_m);
        validation_surface.node_validation_logging_enabled = false;
        validation_surface.compile_dirty_with_reason(&validation_graph, terrain, compile_reason);

        if let Some(failure) = Self::explicit_surface_validation_failure(
            &validation_surface,
            &required_edge_ids,
            &required_node_ids,
        ) {
            match failure {
                SurfaceValidationFailure::MissingSpans(missing_required_edge_ids) => {
                    Self::log_surface_validation_missing_spans(
                        &validation_graph,
                        &validation_surface,
                        new_edge_idx,
                        &required_edge_ids,
                        &missing_required_edge_ids,
                        &required_node_ids,
                        compile_reason,
                    );
                }
                SurfaceValidationFailure::MissingNodes(missing_required_node_ids) => {
                    Self::log_surface_validation_missing_nodes(
                        &validation_graph,
                        &validation_surface,
                        new_edge_idx,
                        &required_edge_ids,
                        &missing_required_node_ids,
                        compile_reason,
                    );
                }
            }
            return validation.with_invalid_reason(PREVIEW_SURFACE_GEOMETRY_REASON);
        }

        let missing_required_edge_ids = required_edge_ids
            .iter()
            .copied()
            .filter(|edge_idx| {
                *edge_idx >= validation_graph.edge_count()
                    || validation_graph.edge(*edge_idx).deleted
                    || !validation_surface
                        .compiled_visual_span_pieces()
                        .contains_key(edge_idx)
            })
            .collect::<Vec<_>>();
        if !missing_required_edge_ids.is_empty() {
            Self::log_surface_validation_missing_spans(
                &validation_graph,
                &validation_surface,
                new_edge_idx,
                &required_edge_ids,
                &missing_required_edge_ids,
                &required_node_ids,
                compile_reason,
            );
            return validation.with_invalid_reason(PREVIEW_SURFACE_GEOMETRY_REASON);
        }

        let mut missing_required_node_ids = Vec::new();
        if edge_class == EdgeClass::Standard {
            for &node_id in &required_node_ids {
                let node_id = validation_graph.get_valid_node(node_id);
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
                            missing_required_node_ids.push(node_id);
                        }
                    }
                }
            }
        }
        if !missing_required_node_ids.is_empty() {
            missing_required_node_ids.sort_unstable();
            missing_required_node_ids.dedup();
            Self::log_surface_validation_missing_nodes(
                &validation_graph,
                &validation_surface,
                new_edge_idx,
                &required_edge_ids,
                &missing_required_node_ids,
                compile_reason,
            );
            return validation.with_invalid_reason(PREVIEW_SURFACE_GEOMETRY_REASON);
        }

        validation
    }

    fn explicit_surface_validation_failure(
        validation_surface: &RoadSurfaceSystem,
        required_edge_ids: &[usize],
        required_node_ids: &[u32],
    ) -> Option<SurfaceValidationFailure> {
        if !validation_surface.last_failed_span_ids.is_empty() {
            return Some(SurfaceValidationFailure::MissingSpans(
                Self::required_or_all_failed_span_ids(
                    &validation_surface.last_failed_span_ids,
                    required_edge_ids,
                ),
            ));
        }
        if !validation_surface.last_failed_node_ids.is_empty() {
            return Some(SurfaceValidationFailure::MissingNodes(
                Self::required_or_all_failed_node_ids(
                    &validation_surface.last_failed_node_ids,
                    required_node_ids,
                ),
            ));
        }
        None
    }

    fn required_or_all_failed_span_ids(
        failed_span_ids: &[usize],
        required_edge_ids: &[usize],
    ) -> Vec<usize> {
        let required_failures = failed_span_ids
            .iter()
            .copied()
            .filter(|edge_idx| required_edge_ids.binary_search(edge_idx).is_ok())
            .collect::<Vec<_>>();
        if required_failures.is_empty() {
            failed_span_ids.to_vec()
        } else {
            required_failures
        }
    }

    fn required_or_all_failed_node_ids(
        failed_node_ids: &[u32],
        required_node_ids: &[u32],
    ) -> Vec<u32> {
        let required_failures = failed_node_ids
            .iter()
            .copied()
            .filter(|node_id| required_node_ids.binary_search(node_id).is_ok())
            .collect::<Vec<_>>();
        if required_failures.is_empty() {
            failed_node_ids.to_vec()
        } else {
            required_failures
        }
    }

    fn log_surface_validation_missing_spans(
        validation_graph: &RegionGraph,
        validation_surface: &RoadSurfaceSystem,
        new_edge_idx: usize,
        required_edge_ids: &[usize],
        missing_edge_ids: &[usize],
        required_node_ids: &[u32],
        compile_reason: RoadSurfaceCompileReason,
    ) {
        if !crate::debug::category_enabled("road") {
            return;
        }
        let failure_label = validation_surface
            .last_compile_failure_label()
            .unwrap_or("none");
        let failed_span_ids = if validation_surface.last_failed_span_ids.is_empty() {
            missing_edge_ids
        } else {
            validation_surface.last_failed_span_ids.as_slice()
        };
        crate::debug_log!(
            "road",
            "road_candidate_surface_geometry_failed cause=missing_spans compile_reason={} new_edge={} graph_edges={} graph_nodes={} required_edges={:?} missing_edges={:?} compiler_failed_spans={:?} required_nodes={:?} surface_failure={}",
            compile_reason.as_str(),
            new_edge_idx,
            validation_graph.edge_count(),
            validation_graph.node_count(),
            required_edge_ids,
            missing_edge_ids,
            failed_span_ids,
            required_node_ids,
            failure_label
        );
        for &edge_idx in failed_span_ids.iter().take(8) {
            Self::log_surface_validation_edge_detail(validation_graph, edge_idx, new_edge_idx);
        }
    }

    fn log_surface_validation_missing_nodes(
        validation_graph: &RegionGraph,
        validation_surface: &RoadSurfaceSystem,
        new_edge_idx: usize,
        required_edge_ids: &[usize],
        missing_node_ids: &[u32],
        compile_reason: RoadSurfaceCompileReason,
    ) {
        if !crate::debug::category_enabled("road") {
            return;
        }
        let failure_label = validation_surface
            .last_compile_failure_label()
            .unwrap_or("none");
        let failed_node_ids = if validation_surface.last_failed_node_ids.is_empty() {
            missing_node_ids
        } else {
            validation_surface.last_failed_node_ids.as_slice()
        };
        crate::debug_log!(
            "road",
            "road_candidate_surface_geometry_failed cause=missing_nodes compile_reason={} new_edge={} graph_edges={} graph_nodes={} required_edges={:?} missing_nodes={:?} compiler_failed_nodes={:?} surface_failure={}",
            compile_reason.as_str(),
            new_edge_idx,
            validation_graph.edge_count(),
            validation_graph.node_count(),
            required_edge_ids,
            missing_node_ids,
            failed_node_ids,
            failure_label
        );
        for &node_id in failed_node_ids.iter().take(8) {
            Self::log_surface_validation_node_detail(validation_graph, node_id);
        }
    }

    fn log_surface_validation_edge_detail(
        validation_graph: &RegionGraph,
        edge_idx: usize,
        new_edge_idx: usize,
    ) {
        if edge_idx >= validation_graph.edge_count() {
            crate::debug_log!(
                "road",
                "road_candidate_surface_failed_edge edge={} new_edge={} missing_from_graph=true",
                edge_idx,
                new_edge_idx
            );
            return;
        }
        let edge = validation_graph.edge(edge_idx);
        let start_node = validation_graph.get_valid_node(edge.start_node);
        let end_node = validation_graph.get_valid_node(edge.end_node);
        let start_pos = validation_graph.node(start_node).pos;
        let end_pos = validation_graph.node(end_node).pos;
        let first = edge.geometry.first().copied().unwrap_or(start_pos);
        let last = edge.geometry.last().copied().unwrap_or(end_pos);
        crate::debug_log!(
            "road",
            "road_candidate_surface_failed_edge edge={} new_edge={} deleted={} class={:?} lanes=({},{}) length={:.3} start_clip={:.3} end_clip={:.3} nodes=({},{}) node_pos=({:.3},{:.3},{:.3})->({:.3},{:.3},{:.3}) geometry_points={} geometry_first=({:.3},{:.3},{:.3}) geometry_last=({:.3},{:.3},{:.3})",
            edge_idx,
            new_edge_idx,
            edge.deleted,
            edge.class,
            edge.fwd_lane_count(),
            edge.bkw_lane_count(),
            edge.physical_length,
            edge.start_clip,
            edge.end_clip,
            start_node,
            end_node,
            start_pos.x,
            start_pos.y,
            start_pos.z,
            end_pos.x,
            end_pos.y,
            end_pos.z,
            edge.geometry.len(),
            first.x,
            first.y,
            first.z,
            last.x,
            last.y,
            last.z
        );
    }

    fn log_surface_validation_node_detail(validation_graph: &RegionGraph, node_id: u32) {
        let node_id = validation_graph.get_valid_node(node_id);
        if node_id as usize >= validation_graph.node_adjacency_count() {
            crate::debug_log!(
                "road",
                "road_candidate_surface_failed_node node={} missing_adjacency=true",
                node_id
            );
            return;
        }
        let node_pos = validation_graph.node(node_id).pos;
        let incident_edges = validation_graph
            .node_adjacency(node_id)
            .iter()
            .copied()
            .filter(|edge_idx| {
                *edge_idx < validation_graph.edge_count()
                    && Self::is_surface_edge(validation_graph.edge(*edge_idx))
            })
            .collect::<Vec<_>>();
        crate::debug_log!(
            "road",
            "road_candidate_surface_failed_node node={} pos=({:.3},{:.3},{:.3}) incident_edges={:?}",
            node_id,
            node_pos.x,
            node_pos.y,
            node_pos.z,
            incident_edges
        );
        for edge_idx in incident_edges.iter().copied().take(8) {
            Self::log_surface_validation_edge_detail(validation_graph, edge_idx, usize::MAX);
        }
    }

    #[cfg(test)]
    pub(crate) fn build_surface_validation_graph_for_test(
        &self,
        prepared_points: &[Vector3],
        edge_class: EdgeClass,
        fwd_lanes: u8,
        bkw_lanes: u8,
        existing_graph: &RegionGraph,
    ) -> Option<(RegionGraph, usize, Vec<usize>, Vec<u32>)> {
        self.build_surface_validation_graph(
            prepared_points,
            edge_class,
            fwd_lanes,
            bkw_lanes,
            existing_graph,
            None,
            true,
        )
    }

    fn build_surface_validation_graph(
        &self,
        prepared_points: &[Vector3],
        edge_class: EdgeClass,
        fwd_lanes: u8,
        bkw_lanes: u8,
        existing_graph: &RegionGraph,
        extension: Option<&RoadExtensionReprofile>,
        endpoint_snap_enabled: bool,
    ) -> Option<(RegionGraph, usize, Vec<usize>, Vec<u32>)> {
        if prepared_points.len() < 2 {
            return None;
        }

        let mut validation_graph = RegionGraph::new();
        let mut node_map = HashMap::new();
        let mut copied_edges = HashSet::new();
        let start_existing = Self::validation_endpoint_existing_node(
            prepared_points[0],
            existing_graph,
            extension,
            endpoint_snap_enabled,
        );
        let end_existing = Self::validation_endpoint_existing_node(
            *prepared_points.last().unwrap(),
            existing_graph,
            extension,
            endpoint_snap_enabled,
        );

        let existing_edge_ids = Self::validation_candidate_existing_edge_ids(
            prepared_points,
            fwd_lanes,
            bkw_lanes,
            edge_class,
            existing_graph,
            extension,
            start_existing,
            end_existing,
        );
        for edge_idx in existing_edge_ids {
            Self::copy_validation_graph_edge(
                &mut validation_graph,
                existing_graph,
                &mut node_map,
                &mut copied_edges,
                edge_idx,
                extension,
            );
        }

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
        let topology_node_count_before = validation_graph.node_count();

        let mut points = prepared_points.to_vec();
        points[0] = validation_graph.node(start_node).pos;
        let last_idx = points.len() - 1;
        points[last_idx] = validation_graph.node(end_node).pos;
        let new_edge_idx = validation_graph.add_edge(build_surface_edge(
            start_node, end_node, points, fwd_lanes, bkw_lanes, edge_class,
        ));
        self.process_validation_graph_intersections(&mut validation_graph, new_edge_idx);

        let required_edge_ids = validation_graph
            .edges()
            .iter()
            .enumerate()
            .skip(new_edge_idx)
            .filter_map(|(edge_idx, edge)| {
                (!edge.deleted && Self::is_surface_edge(edge)).then_some(edge_idx)
            })
            .collect::<Vec<_>>();
        let mut required_node_ids = vec![start_node, end_node];
        required_node_ids.extend(
            (topology_node_count_before..validation_graph.node_count())
                .filter_map(|node_id| u32::try_from(node_id).ok()),
        );
        for node_id in &mut required_node_ids {
            *node_id = validation_graph.get_valid_node(*node_id);
        }
        required_node_ids.sort_unstable();
        required_node_ids.dedup();

        Some((
            validation_graph,
            new_edge_idx,
            required_edge_ids,
            required_node_ids,
        ))
    }

    fn validation_candidate_existing_edge_ids(
        prepared_points: &[Vector3],
        fwd_lanes: u8,
        bkw_lanes: u8,
        edge_class: EdgeClass,
        existing_graph: &RegionGraph,
        extension: Option<&RoadExtensionReprofile>,
        start_existing: Option<u32>,
        end_existing: Option<u32>,
    ) -> Vec<usize> {
        let mut min_x = f32::MAX;
        let mut max_x = f32::MIN;
        let mut min_z = f32::MAX;
        let mut max_z = f32::MIN;
        for point in prepared_points {
            min_x = min_x.min(point.x);
            max_x = max_x.max(point.x);
            min_z = min_z.min(point.z);
            max_z = max_z.max(point.z);
        }

        let pad = config::SNAP_TOLERANCE
            + config::INTERSECTION_TOLERANCE
            + Self::preview_candidate_roadbed_half_width_m(
                prepared_points,
                edge_class,
                fwd_lanes,
                bkw_lanes,
            );
        let mut edge_ids = existing_graph.get_edges_near_aabb(
            Vector3::new(min_x - pad, 0.0, min_z - pad),
            Vector3::new(max_x + pad, 0.0, max_z + pad),
        );
        if let Some(extension) = extension {
            edge_ids.push(extension.existing_edge_idx);
        }
        for existing_node in [start_existing, end_existing].into_iter().flatten() {
            Self::push_incident_validation_edges(existing_graph, existing_node, &mut edge_ids);
        }

        edge_ids.sort_unstable();
        edge_ids.dedup();

        let mut incident_nodes = Vec::new();
        for &edge_idx in &edge_ids {
            if edge_idx >= existing_graph.edge_count() {
                continue;
            }
            let edge = existing_graph.edge(edge_idx);
            if edge.deleted || !Self::is_surface_edge(edge) {
                continue;
            }
            incident_nodes.push(existing_graph.get_valid_node(edge.start_node));
            incident_nodes.push(existing_graph.get_valid_node(edge.end_node));
        }
        incident_nodes.sort_unstable();
        incident_nodes.dedup();
        for node_id in incident_nodes {
            Self::push_incident_validation_edges(existing_graph, node_id, &mut edge_ids);
        }

        edge_ids.sort_unstable();
        edge_ids.dedup();
        edge_ids
    }

    fn push_incident_validation_edges(
        existing_graph: &RegionGraph,
        node_id: u32,
        edge_ids: &mut Vec<usize>,
    ) {
        let valid = existing_graph.get_valid_node(node_id);
        if valid as usize >= existing_graph.node_adjacency_count() {
            return;
        }
        edge_ids.extend(existing_graph.node_adjacency(valid).iter().copied());
    }

    fn process_validation_graph_intersections(
        &self,
        validation_graph: &mut RegionGraph,
        new_edge_idx: usize,
    ) {
        let mut network = TransitNetwork::new_with_surface_chunk_span(self.chunk_span_m);
        let world_config = WorldConfig::default();
        let mut zoning = ZoningSystem::new(&world_config);
        let mut allocator = BuildingAllocator::new();
        topology::process_intersections(
            &mut network,
            validation_graph,
            new_edge_idx,
            &mut zoning,
            &mut allocator,
        );
        Self::cleanup_validation_duplicate_edges(validation_graph);

        let affected_nodes = (0..validation_graph.node_count())
            .map(|node_id| validation_graph.get_valid_node(node_id as u32))
            .collect::<HashSet<_>>();
        let mut affected_edges = validation_graph
            .edges()
            .iter()
            .enumerate()
            .filter_map(|(edge_idx, edge)| (!edge.deleted).then_some(edge_idx))
            .collect::<HashSet<_>>();
        let profile_changed_edges = validation_graph
            .solve_junction_endpoint_profiles_for_edges(&affected_nodes, &affected_edges);
        affected_edges.extend(profile_changed_edges);
        let regrade_changed_edges = validation_graph
            .regrade_junction_endpoint_profiles_for_nodes(&affected_nodes, &affected_edges);
        affected_edges.extend(regrade_changed_edges);
        validation_graph.rebuild_intersection_clips();
    }

    fn cleanup_validation_duplicate_edges(graph: &mut RegionGraph) {
        let mut seen = HashSet::new();
        let mut to_remove = Vec::new();

        for (edge_idx, edge) in graph.edges().iter().enumerate() {
            if edge.deleted {
                continue;
            }
            let pair = if edge.start_node < edge.end_node {
                (edge.start_node, edge.end_node)
            } else {
                (edge.end_node, edge.start_node)
            };

            if seen.contains(&pair) || edge.start_node == edge.end_node {
                to_remove.push(edge_idx);
            } else {
                seen.insert(pair);
            }
        }

        for edge_idx in to_remove {
            graph.edge_mut(edge_idx).deleted = true;
        }
        graph.rebuild_adjacency_list();
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

    fn copy_validation_graph_edge(
        validation_graph: &mut RegionGraph,
        existing_graph: &RegionGraph,
        node_map: &mut HashMap<u32, u32>,
        copied_edges: &mut HashSet<usize>,
        edge_idx: usize,
        extension: Option<&RoadExtensionReprofile>,
    ) {
        if !copied_edges.insert(edge_idx) || edge_idx >= existing_graph.edge_count() {
            return;
        }
        let edge = existing_graph.edge(edge_idx);
        if edge.deleted || !Self::is_surface_edge(edge) {
            return;
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
            return;
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

    fn validation_endpoint_existing_node(
        point: Vector3,
        existing_graph: &RegionGraph,
        extension: Option<&RoadExtensionReprofile>,
        endpoint_snap_enabled: bool,
    ) -> Option<u32> {
        if let Some(extension) = extension {
            let node_pos = existing_graph.node(extension.snapped_node_id).pos;
            let dx = point.x - node_pos.x;
            let dz = point.z - node_pos.z;
            if dx.hypot(dz) < config::SNAP_TOLERANCE {
                return Some(extension.snapped_node_id);
            }
        }
        if !endpoint_snap_enabled {
            return None;
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

    /// Replaces a valid result with one stable machine-readable rejection reason.
    pub(crate) fn with_invalid_reason(mut self, invalid_reason: &'static str) -> Self {
        self.is_valid = false;
        self.invalid_reason = invalid_reason;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_surface_validation_failure_reports_latched_node_stage() {
        let mut validation_surface = RoadSurfaceSystem::new(16.0);
        validation_surface.last_failed_node_ids = vec![4, 12];

        let failure = RoadSurfaceSystem::explicit_surface_validation_failure(
            &validation_surface,
            &[7, 8],
            &[4, 9],
        );

        assert_eq!(
            failure,
            Some(SurfaceValidationFailure::MissingNodes(vec![4]))
        );
    }

    #[test]
    fn explicit_surface_validation_failure_keeps_context_failure_when_required_set_misses() {
        let mut validation_surface = RoadSurfaceSystem::new(16.0);
        validation_surface.last_failed_node_ids = vec![12];

        let failure = RoadSurfaceSystem::explicit_surface_validation_failure(
            &validation_surface,
            &[7, 8],
            &[4, 9],
        );

        assert_eq!(
            failure,
            Some(SurfaceValidationFailure::MissingNodes(vec![12]))
        );
    }
}
