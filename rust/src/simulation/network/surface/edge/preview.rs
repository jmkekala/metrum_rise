//! Temporary road preview compilation from conditioned edge input.

use super::super::{
    RoadSurfaceSection, RoadSurfaceSystem, RoadSurfaceVisualNodePiece, SAMPLE_EPSILON_M,
};
use super::input::PREVIEW_CLEARANCE_M;
use crate::config;
use crate::simulation::network::build_surface_edge;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::{EdgeClass, NodeType};
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::Vector3;

// Preview validation thresholds.
const PREVIEW_MAX_GRADE: f32 = 0.41;

/// Temporary preview compile output for one road-tool stroke.
#[derive(Clone, Debug, PartialEq)]
pub struct PreviewRoadSurfaceResult {
    /// Edge class inferred from the preview stroke before temporary compilation.
    pub edge_class: EdgeClass,
    /// Prepared centerline points after the same grounding, simplification, and smoothing rules
    /// used by committed placement.
    pub prepared_points: Vec<Vector3>,
    /// Compiled section cache for the temporary preview edge.
    pub compiled_sections: Vec<RoadSurfaceSection>,
    /// Explicit visual node pieces for the temporary preview edge endpoints.
    pub compiled_visual_node_pieces: Vec<RoadSurfaceVisualNodePiece>,
    /// Triangulated top-surface preview mesh vertices from the solved section geometry.
    pub surface_vertices: Vec<Vector3>,
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
        let (conditioned_points, edge_class) =
            Self::classify_and_ground_road_points(raw_points, terrain);
        let mut prepared_points = Self::simplify_road_input_points(&conditioned_points);
        Self::taubin_smooth_road_heights(&mut prepared_points);

        if prepared_points.len() < 2 {
            return PreviewRoadSurfaceResult {
                edge_class,
                prepared_points,
                compiled_sections: Vec::new(),
                compiled_visual_node_pieces: Vec::new(),
                surface_vertices: Vec::new(),
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
        let is_valid = Self::preview_surface_is_valid(
            edge_class,
            &prepared_points,
            &compiled_sections,
            terrain,
        );

        PreviewRoadSurfaceResult {
            edge_class,
            prepared_points,
            compiled_sections,
            compiled_visual_node_pieces,
            surface_vertices,
            is_valid,
        }
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
                vertices.extend_from_slice(&[a0, b0, a1, a1, b0, b1]);
            }
        }

        vertices
    }

    fn preview_surface_is_valid(
        edge_class: EdgeClass,
        prepared_points: &[Vector3],
        compiled_sections: &[RoadSurfaceSection],
        terrain: &TerrainSystem,
    ) -> bool {
        for pair in compiled_sections.windows(2) {
            let run = (pair[1].s_m - pair[0].s_m).abs();
            if run <= SAMPLE_EPSILON_M {
                continue;
            }
            let grade = (pair[1].center_height_m - pair[0].center_height_m).abs() / run;
            if grade > PREVIEW_MAX_GRADE {
                return false;
            }
        }

        if prepared_points.len() > 2 {
            if let Some(mid_section) = compiled_sections.get(compiled_sections.len() / 2) {
                let terrain_h = terrain
                    .sample_height_world(mid_section.center_xz.x, mid_section.center_xz.y)
                    * config::HEIGHT_SCALE;
                match edge_class {
                    EdgeClass::Bridge => {
                        if mid_section.center_height_m < terrain_h + PREVIEW_CLEARANCE_M {
                            return false;
                        }
                    }
                    EdgeClass::Tunnel => {
                        if mid_section.center_height_m > terrain_h - PREVIEW_CLEARANCE_M {
                            return false;
                        }
                    }
                    EdgeClass::Standard => {}
                }
            }
        }

        true
    }
}
