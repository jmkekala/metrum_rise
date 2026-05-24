//! Edge centerline section sampling and longitudinal height selection.

use super::super::{CompiledNodeKind, RoadSurfaceSection, RoadSurfaceSystem, SAMPLE_EPSILON_M};
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::EdgeClass;
use godot::prelude::{Vector2, Vector3};

// Longitudinal section sampling cadence by road-edge class.
const STANDARD_SECTION_STEP_M: f32 = 8.0;
const BRIDGE_SECTION_STEP_M: f32 = 12.0;
const TUNNEL_SECTION_STEP_M: f32 = 10.0;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn compile_edge_sections(
        &self,
        graph: &RegionGraph,
        edge_idx: usize,
    ) -> Vec<RoadSurfaceSection> {
        let edge = graph.edge(edge_idx);
        let points = self.edge_points(edge);
        if points.is_empty() {
            return Vec::new();
        }
        if points.len() == 1 {
            let center = points[0];
            let center_height_m = self.solve_section_height(center);
            let tangent_xz = Vector2::RIGHT;
            let lateral_xz = Vector2::new(-tangent_xz.y, tangent_xz.x);
            return vec![RoadSurfaceSection {
                edge_idx,
                s_m: 0.0,
                center_xz: Vector2::new(center.x, center.z),
                center_height_m,
                tangent_xz,
                lateral_xz,
                bands: self.build_lateral_bands(edge, center, lateral_xz, None),
            }];
        }

        let cumulative = self.build_cumulative_distances(points);
        let start_kind = self.classify_surface_node_kind_from_graph_geometry(
            graph,
            graph.get_valid_node(edge.start_node),
        );
        let end_kind = self.classify_surface_node_kind_from_graph_geometry(
            graph,
            graph.get_valid_node(edge.end_node),
        );
        let handoff_range = self.visual_surface_handoff_range_for_edge(
            graph,
            edge_idx,
            edge,
            *cumulative.last().unwrap_or(&0.0),
            start_kind,
            end_kind,
        );
        let start_profile_plane = matches!(
            start_kind,
            Some(CompiledNodeKind::Bend | CompiledNodeKind::JunctionN)
        )
        .then(|| graph.junction_endpoint_profile_plane(graph.get_valid_node(edge.start_node)))
        .flatten();
        let end_profile_plane = matches!(
            end_kind,
            Some(CompiledNodeKind::Bend | CompiledNodeKind::JunctionN)
        )
        .then(|| graph.junction_endpoint_profile_plane(graph.get_valid_node(edge.end_node)))
        .flatten();
        let sample_distances = self.build_section_sample_distances(
            graph,
            edge_idx,
            edge,
            &cumulative,
            start_kind,
            end_kind,
        );
        sample_distances
            .into_iter()
            .map(|s_m| {
                let (center, tangent_xz) = self.sample_polyline(points, &cumulative, s_m);
                let lateral_xz = Vector2::new(-tangent_xz.y, tangent_xz.x).normalized();
                let profile_plane =
                    handoff_range.and_then(|(start_handoff_s_m, end_handoff_s_m)| {
                        if s_m <= start_handoff_s_m + SAMPLE_EPSILON_M {
                            start_profile_plane
                        } else if s_m >= end_handoff_s_m - SAMPLE_EPSILON_M {
                            end_profile_plane
                        } else {
                            None
                        }
                    });
                let center_height_m = profile_plane.map_or_else(
                    || self.solve_section_height(center),
                    |plane| plane.height_at_xz(center.x, center.z),
                );
                RoadSurfaceSection {
                    edge_idx,
                    s_m,
                    center_xz: Vector2::new(center.x, center.z),
                    center_height_m,
                    tangent_xz,
                    lateral_xz,
                    bands: self.build_lateral_bands(edge, center, lateral_xz, profile_plane),
                }
            })
            .collect()
    }

    fn solve_section_height(&self, center: Vector3) -> f32 {
        center.y
    }

    fn build_section_sample_distances(
        &self,
        graph: &RegionGraph,
        edge_idx: usize,
        edge: &Edge,
        cumulative: &[f32],
        start_kind: Option<CompiledNodeKind>,
        end_kind: Option<CompiledNodeKind>,
    ) -> Vec<f32> {
        let Some(&total_length) = cumulative.last() else {
            return vec![0.0];
        };
        if total_length <= SAMPLE_EPSILON_M {
            return vec![0.0];
        }

        let mut samples = vec![0.0, total_length];
        samples.push(edge.start_clip.clamp(0.0, total_length));
        samples.push((total_length - edge.end_clip).clamp(0.0, total_length));
        if let Some((start_throat, end_throat)) = self.visual_surface_handoff_range_for_edge(
            graph,
            edge_idx,
            edge,
            total_length,
            start_kind,
            end_kind,
        ) {
            samples.push(start_throat);
            samples.push(end_throat);
        }

        for &distance in cumulative {
            samples.push(distance);
        }

        let step_m = self.section_step_for_class(edge.class);
        for segment in cumulative.windows(2) {
            let start_s = segment[0];
            let end_s = segment[1];
            let mut sample_s = start_s + step_m;
            while sample_s < end_s - SAMPLE_EPSILON_M {
                samples.push(sample_s);
                sample_s += step_m;
            }
        }

        samples.sort_by(f32::total_cmp);
        samples.dedup_by(|a, b| (*a - *b).abs() <= SAMPLE_EPSILON_M);
        samples
    }

    fn section_step_for_class(&self, class: EdgeClass) -> f32 {
        match class {
            EdgeClass::Standard => STANDARD_SECTION_STEP_M,
            EdgeClass::Bridge => BRIDGE_SECTION_STEP_M,
            EdgeClass::Tunnel => TUNNEL_SECTION_STEP_M,
        }
    }
}
