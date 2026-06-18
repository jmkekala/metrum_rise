//! Edge centerline section sampling and longitudinal height selection.

use super::super::backend::{RoadVec2, RoadVec3, godot_vec3_to_road};
use super::super::{CompiledNodeKind, RoadSurfaceSection, RoadSurfaceSystem, SAMPLE_EPSILON_M};
use super::EdgeProfilePlaneBlend;
use crate::simulation::network::graph::rebuild::{
    JUNCTION_PROFILE_BLEND_ZONE_M, JunctionEndpointProfilePlane,
};
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::EdgeClass;

// Longitudinal section sampling cadence by road-edge class.
const STANDARD_SECTION_STEP_M: f32 = 8.0;
const BRIDGE_SECTION_STEP_M: f32 = 12.0;
const TUNNEL_SECTION_STEP_M: f32 = 10.0;
const PROFILE_TRANSITION_SECTION_STEP_M: f32 = 2.0;
const PROTECTED_SECTION_SAMPLE_CLEARANCE_M: f32 = 0.125;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn compile_edge_sections(
        &self,
        graph: &RegionGraph,
        edge_idx: usize,
    ) -> Vec<RoadSurfaceSection> {
        let edge = graph.edge(edge_idx);
        let points: Vec<RoadVec3> = self
            .edge_points(edge)
            .iter()
            .copied()
            .map(godot_vec3_to_road)
            .collect();
        if points.is_empty() {
            return Vec::new();
        }
        if points.len() == 1 {
            let center = points[0];
            let center_height_m = self.solve_section_height(center);
            let tangent_xz = RoadVec2::X;
            let lateral_xz = RoadVec2::new(-tangent_xz.y, tangent_xz.x);
            return vec![RoadSurfaceSection {
                edge_idx,
                s_m: 0.0,
                center_xz: RoadVec2::new(center.x, center.z),
                center_height_m,
                tangent_xz,
                lateral_xz,
                bands: self.build_lateral_bands(edge, center, lateral_xz, None),
            }];
        }

        let cumulative = self.build_cumulative_distances(&points);
        let start_kind = self.classify_surface_node_kind_from_graph_geometry(
            graph,
            graph.get_valid_node(edge.start_node),
        );
        let end_kind = self.classify_surface_node_kind_from_graph_geometry(
            graph,
            graph.get_valid_node(edge.end_node),
        );
        let total_length_m = *cumulative.last().unwrap_or(&0.0);
        let handoff_range = self.visual_surface_handoff_range_for_edge(
            graph,
            edge_idx,
            edge,
            total_length_m,
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
        let profile_range = Self::edge_profile_blend_range_for_edge(
            edge,
            total_length_m,
            handoff_range,
            start_kind,
            end_kind,
            start_profile_plane.is_some(),
            end_profile_plane.is_some(),
        );
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
                let (center, tangent_xz) = self.sample_polyline(&points, &cumulative, s_m);
                let lateral_xz = RoadVec2::new(-tangent_xz.y, tangent_xz.x).normalize();
                let profile_blend = profile_range.and_then(|profile| {
                    Self::edge_profile_blend_for_section(
                        s_m,
                        profile,
                        start_profile_plane,
                        end_profile_plane,
                    )
                });
                let center_height_m = profile_blend.map_or_else(
                    || self.solve_section_height(center),
                    |blend| {
                        blend.height_at_xz(
                            center.x as f32,
                            center.z as f32,
                            self.solve_section_height(center),
                        )
                    },
                );
                RoadSurfaceSection {
                    edge_idx,
                    s_m,
                    center_xz: RoadVec2::new(center.x, center.z),
                    center_height_m,
                    tangent_xz,
                    lateral_xz,
                    bands: self.build_lateral_bands(edge, center, lateral_xz, profile_blend),
                }
            })
            .collect()
    }

    fn solve_section_height(&self, center: RoadVec3) -> f32 {
        center.y as f32
    }

    fn edge_profile_blend_for_section(
        s_m: f32,
        profile_range: (f32, f32),
        start_profile_plane: Option<JunctionEndpointProfilePlane>,
        end_profile_plane: Option<JunctionEndpointProfilePlane>,
    ) -> Option<EdgeProfilePlaneBlend> {
        let (start_profile_s_m, end_profile_s_m) = profile_range;
        let span_m = (end_profile_s_m - start_profile_s_m).max(0.0);
        let fade_m = JUNCTION_PROFILE_BLEND_ZONE_M.min(span_m * 0.5);

        let start_blend = start_profile_plane.and_then(|plane| {
            let weight = if s_m <= start_profile_s_m + SAMPLE_EPSILON_M {
                1.0
            } else if fade_m > SAMPLE_EPSILON_M && s_m < start_profile_s_m + fade_m {
                let t = ((s_m - start_profile_s_m) / fade_m).clamp(0.0, 1.0);
                1.0 - Self::smootherstep(t)
            } else {
                0.0
            };
            EdgeProfilePlaneBlend::new(plane, weight)
        });

        let end_blend = end_profile_plane.and_then(|plane| {
            let weight = if s_m >= end_profile_s_m - SAMPLE_EPSILON_M {
                1.0
            } else if fade_m > SAMPLE_EPSILON_M && s_m > end_profile_s_m - fade_m {
                let t = ((end_profile_s_m - s_m) / fade_m).clamp(0.0, 1.0);
                1.0 - Self::smootherstep(t)
            } else {
                0.0
            };
            EdgeProfilePlaneBlend::new(plane, weight)
        });

        match (start_blend, end_blend) {
            (Some(start), Some(end)) if start.weight >= end.weight => Some(start),
            (Some(_), Some(end)) => Some(end),
            (Some(start), None) => Some(start),
            (None, Some(end)) => Some(end),
            (None, None) => None,
        }
    }

    fn edge_profile_blend_range_for_edge(
        edge: &Edge,
        total_length_m: f32,
        handoff_range: Option<(f32, f32)>,
        start_kind: Option<CompiledNodeKind>,
        end_kind: Option<CompiledNodeKind>,
        has_start_profile: bool,
        has_end_profile: bool,
    ) -> Option<(f32, f32)> {
        if total_length_m <= SAMPLE_EPSILON_M || (!has_start_profile && !has_end_profile) {
            return None;
        }

        let hard_zone_m = RegionGraph::junction_profile_hard_zone_m(edge, total_length_m);
        let start_s_m = if !has_start_profile {
            0.0
        } else if start_kind == Some(CompiledNodeKind::Bend)
            && Self::standard_bend_uses_short_profile_pin(edge)
        {
            hard_zone_m
        } else if let Some((start_handoff_m, _)) = handoff_range {
            start_handoff_m
        } else {
            hard_zone_m
        };
        let end_s_m = if !has_end_profile {
            total_length_m
        } else if end_kind == Some(CompiledNodeKind::Bend)
            && Self::standard_bend_uses_short_profile_pin(edge)
        {
            (total_length_m - hard_zone_m).max(0.0)
        } else if let Some((_, end_handoff_m)) = handoff_range {
            end_handoff_m
        } else {
            (total_length_m - hard_zone_m).max(0.0)
        };
        (end_s_m - start_s_m > SAMPLE_EPSILON_M).then_some((start_s_m, end_s_m))
    }

    fn standard_bend_uses_short_profile_pin(edge: &Edge) -> bool {
        if edge.class != EdgeClass::Standard {
            return false;
        }
        let points = if edge.physical_geometry.is_empty() {
            &edge.geometry
        } else {
            &edge.physical_geometry
        };
        points.len() <= 2
    }

    fn smootherstep(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
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

        let mut samples = Vec::new();
        let mut protected_samples = Vec::new();
        Self::push_protected_section_sample(
            &mut samples,
            &mut protected_samples,
            0.0,
            total_length,
        );
        Self::push_protected_section_sample(
            &mut samples,
            &mut protected_samples,
            total_length,
            total_length,
        );
        Self::push_protected_section_sample(
            &mut samples,
            &mut protected_samples,
            edge.start_clip,
            total_length,
        );
        Self::push_protected_section_sample(
            &mut samples,
            &mut protected_samples,
            total_length - edge.end_clip,
            total_length,
        );
        let handoff_range = self.visual_surface_handoff_range_for_edge(
            graph,
            edge_idx,
            edge,
            total_length,
            start_kind,
            end_kind,
        );
        if let Some((start_throat, end_throat)) = handoff_range {
            Self::push_protected_section_sample(
                &mut samples,
                &mut protected_samples,
                start_throat,
                total_length,
            );
            Self::push_protected_section_sample(
                &mut samples,
                &mut protected_samples,
                end_throat,
                total_length,
            );
        }

        let has_start_profile = Self::node_kind_uses_endpoint_profile(start_kind)
            && graph
                .junction_endpoint_profile_plane(graph.get_valid_node(edge.start_node))
                .is_some();
        let has_end_profile = Self::node_kind_uses_endpoint_profile(end_kind)
            && graph
                .junction_endpoint_profile_plane(graph.get_valid_node(edge.end_node))
                .is_some();
        if let Some((start_profile_s_m, end_profile_s_m)) = Self::edge_profile_blend_range_for_edge(
            edge,
            total_length,
            handoff_range,
            start_kind,
            end_kind,
            has_start_profile,
            has_end_profile,
        ) {
            Self::push_protected_section_sample(
                &mut samples,
                &mut protected_samples,
                start_profile_s_m,
                total_length,
            );
            Self::push_protected_section_sample(
                &mut samples,
                &mut protected_samples,
                end_profile_s_m,
                total_length,
            );
            let profile_fade_m = JUNCTION_PROFILE_BLEND_ZONE_M
                .min((end_profile_s_m - start_profile_s_m).max(0.0) * 0.5);
            if profile_fade_m > SAMPLE_EPSILON_M {
                if has_start_profile {
                    Self::push_profile_transition_section_samples(
                        &mut samples,
                        &protected_samples,
                        start_profile_s_m,
                        start_profile_s_m + profile_fade_m,
                        total_length,
                    );
                }
                if has_end_profile {
                    Self::push_profile_transition_section_samples(
                        &mut samples,
                        &protected_samples,
                        end_profile_s_m - profile_fade_m,
                        end_profile_s_m,
                        total_length,
                    );
                }
            }
        }

        for &distance in cumulative {
            if !Self::is_near_protected_section_sample(distance, &protected_samples) {
                samples.push(distance);
            }
        }

        let step_m = self.section_step_for_class(edge.class);
        for segment in cumulative.windows(2) {
            let start_s = segment[0];
            let end_s = segment[1];
            let mut sample_s = start_s + step_m;
            while sample_s < end_s - SAMPLE_EPSILON_M {
                if !Self::is_near_protected_section_sample(sample_s, &protected_samples) {
                    samples.push(sample_s);
                }
                sample_s += step_m;
            }
        }

        samples.sort_by(f32::total_cmp);
        samples.dedup_by(|a, b| (*a - *b).abs() <= SAMPLE_EPSILON_M);
        samples
    }

    fn node_kind_uses_endpoint_profile(kind: Option<CompiledNodeKind>) -> bool {
        matches!(
            kind,
            Some(CompiledNodeKind::Bend | CompiledNodeKind::JunctionN)
        )
    }

    fn push_profile_transition_section_samples(
        samples: &mut Vec<f32>,
        protected_samples: &[f32],
        start_m: f32,
        end_m: f32,
        total_length_m: f32,
    ) {
        let start_m = start_m.clamp(0.0, total_length_m);
        let end_m = end_m.clamp(0.0, total_length_m);
        if end_m <= start_m + PROFILE_TRANSITION_SECTION_STEP_M {
            return;
        }

        let mut sample_m = start_m + PROFILE_TRANSITION_SECTION_STEP_M;
        while sample_m < end_m - SAMPLE_EPSILON_M {
            if !Self::is_near_protected_section_sample(sample_m, protected_samples) {
                samples.push(sample_m);
            }
            sample_m += PROFILE_TRANSITION_SECTION_STEP_M;
        }
    }

    fn push_protected_section_sample(
        samples: &mut Vec<f32>,
        protected_samples: &mut Vec<f32>,
        distance_m: f32,
        total_length_m: f32,
    ) {
        let clamped = distance_m.clamp(0.0, total_length_m);
        samples.push(clamped);
        protected_samples.push(clamped);
    }

    fn is_near_protected_section_sample(distance_m: f32, protected_samples: &[f32]) -> bool {
        protected_samples.iter().any(|&protected_m| {
            let delta_m = (distance_m - protected_m).abs();
            delta_m > SAMPLE_EPSILON_M && delta_m <= PROTECTED_SECTION_SAMPLE_CLEARANCE_M
        })
    }

    fn section_step_for_class(&self, class: EdgeClass) -> f32 {
        match class {
            EdgeClass::Standard => STANDARD_SECTION_STEP_M,
            EdgeClass::Bridge => BRIDGE_SECTION_STEP_M,
            EdgeClass::Tunnel => TUNNEL_SECTION_STEP_M,
        }
    }
}
