// ========================================================================
//  MANIFEST
// ========================================================================
//  script_name: sections.rs
//  script_path: rust/src/simulation/network/surface/edge/sections.rs
//  module_name: sections
//  version: 0.1.0
//  description: Samples cross-section stations along an edge centerline
//  kind: module
//  spec: none
//  internal_dependencies: [graph, backend]
//  external_dependencies: []
//  features: [section-sampling, road-surface, width-taper, profile-blend]
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-24
// ========================================================================

//! Edge centerline section sampling and longitudinal height selection.

use super::super::backend::{RoadVec2, RoadVec3, godot_vec3_to_road};
use super::super::{
    CompiledNodeKind, IncidentEdgeSide, RoadSurfaceSection, RoadSurfaceSystem, SAMPLE_EPSILON_M,
};
use super::{EdgeMouthPolicy, EdgeProfilePlaneBlend};
use crate::config;
use crate::simulation::network::graph::rebuild::{
    JUNCTION_PROFILE_BLEND_ZONE_M, JunctionEndpointProfilePlane,
};
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{EdgeClass, TransitFlags, TransitType};

// ========================================================================
// SAMPLING CADENCE
// ========================================================================

// Longitudinal section sampling cadence by road-edge class.
const STANDARD_SECTION_STEP_M: f32 = 8.0;
const BRIDGE_SECTION_STEP_M: f32 = 12.0;
const TUNNEL_SECTION_STEP_M: f32 = 10.0;
const PROFILE_TRANSITION_SECTION_STEP_M: f32 = 2.0;
const PROTECTED_SECTION_SAMPLE_CLEARANCE_M: f32 = 0.125;
const WIDTH_CHANGE_TAPER_MIN_M: f32 = 12.0;
const WIDTH_CHANGE_TAPER_MAX_M: f32 = 28.0;
const WIDTH_CHANGE_TAPER_WIDTH_FACTOR: f32 = 3.0;
const WIDTH_CHANGE_TAPER_SECTION_STEP_M: f32 = 2.0;
const WIDTH_CHANGE_TAPER_MAX_EDGE_FRACTION: f32 = 0.8;
// Endpoint directions are measured away from the shared node. Straight width changes are near -1,
// normal bends stay below this value, and overlapping same-side edges are rejected.
const WIDTH_CHANGE_CORRIDOR_DOT_MAX: f64 = 0.25;
const WIDTH_CHANGE_MIN_DELTA_M: f32 = 0.25;

#[derive(Clone, Copy, Debug)]
struct EdgeWidthTaper {
    target_carriageway_half_width_m: f32,
    length_m: f32,
}

#[derive(Clone, Copy, Debug, Default)]
struct EdgeWidthTapers {
    start: Option<EdgeWidthTaper>,
    end: Option<EdgeWidthTaper>,
}

// ========================================================================
// COMPILING SECTIONS
// ========================================================================

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
                bands: self.build_lateral_bands(edge, center, lateral_xz, None, None),
            }];
        }

        let cumulative = self.build_cumulative_distances(&points);
        let total_length_m = *cumulative.last().unwrap_or(&0.0);
        let (start_kind, start_pass_through_tangent) =
            self.section_endpoint_kind_and_tangent(graph, edge_idx, edge, true);
        let (end_kind, end_pass_through_tangent) =
            self.section_endpoint_kind_and_tangent(graph, edge_idx, edge, false);
        let start_profile_plane = Self::node_kind_uses_endpoint_profile(start_kind)
            .then(|| graph.junction_endpoint_profile_plane(graph.get_valid_node(edge.start_node)))
            .flatten();
        let end_profile_plane = Self::node_kind_uses_endpoint_profile(end_kind)
            .then(|| graph.junction_endpoint_profile_plane(graph.get_valid_node(edge.end_node)))
            .flatten();
        let mouth_policy = self.visual_edge_mouth_policy_for_edge(
            graph,
            edge_idx,
            edge,
            total_length_m,
            start_kind,
            end_kind,
            start_profile_plane.is_some(),
            end_profile_plane.is_some(),
        );
        let width_tapers =
            self.edge_width_tapers_for_width_change_node(graph, edge_idx, edge, total_length_m);
        let sample_distances = self.build_section_sample_distances(
            edge,
            &cumulative,
            mouth_policy,
            start_profile_plane.is_some(),
            end_profile_plane.is_some(),
            width_tapers,
        );
        sample_distances
            .into_iter()
            .map(|s_m| {
                let (center, sampled_tangent_xz) = self.sample_polyline(&points, &cumulative, s_m);
                let tangent_xz = if s_m <= SAMPLE_EPSILON_M {
                    start_pass_through_tangent.unwrap_or(sampled_tangent_xz)
                } else if total_length_m - s_m <= SAMPLE_EPSILON_M {
                    end_pass_through_tangent.unwrap_or(sampled_tangent_xz)
                } else {
                    sampled_tangent_xz
                };
                let lateral_xz = RoadVec2::new(-tangent_xz.y, tangent_xz.x).normalize();
                let profile_blend = mouth_policy.profile_range.and_then(|profile| {
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
                    bands: self.build_lateral_bands(
                        edge,
                        center,
                        lateral_xz,
                        profile_blend,
                        Some(Self::section_carriageway_half_width(
                            edge,
                            total_length_m,
                            s_m,
                            width_tapers,
                        )),
                    ),
                }
            })
            .collect()
    }

    fn section_endpoint_kind_and_tangent(
        &self,
        graph: &RegionGraph,
        edge_idx: usize,
        edge: &Edge,
        at_start: bool,
    ) -> (Option<CompiledNodeKind>, Option<RoadVec2>) {
        let node_id = graph.get_valid_node(if at_start {
            edge.start_node
        } else {
            edge.end_node
        });
        let incidents = self.sorted_incident_surface_edges_from_graph_geometry(graph, node_id);
        if incidents.is_empty() {
            return (None, None);
        }
        let kind = self.classify_visual_node_kind(&incidents);
        if kind != CompiledNodeKind::PassThrough {
            return (Some(kind), None);
        }

        let [first, second] = incidents.as_slice() else {
            return (Some(kind), None);
        };
        let side = if at_start {
            IncidentEdgeSide::Start
        } else {
            IncidentEdgeSide::End
        };
        let Some(current) = incidents
            .iter()
            .find(|incident| incident.edge_idx == edge_idx && incident.side == side)
        else {
            return (Some(kind), None);
        };
        let axis = first.direction_xz - second.direction_xz;
        if axis.length_squared() <= f64::from(SAMPLE_EPSILON_M * SAMPLE_EPSILON_M) {
            return (Some(kind), None);
        }
        let mut tangent_xz = axis.normalize();
        let forward_xz = if at_start {
            current.direction_xz
        } else {
            -current.direction_xz
        };
        if tangent_xz.dot(forward_xz) < 0.0 {
            tangent_xz = -tangent_xz;
        }
        (Some(kind), Some(tangent_xz))
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

    fn smootherstep(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
    }

    fn edge_width_tapers_for_width_change_node(
        &self,
        graph: &RegionGraph,
        edge_idx: usize,
        edge: &Edge,
        total_length_m: f32,
    ) -> EdgeWidthTapers {
        if !Self::edge_supports_width_change_taper(edge) || total_length_m <= SAMPLE_EPSILON_M {
            return EdgeWidthTapers::default();
        }

        EdgeWidthTapers {
            start: self.edge_endpoint_width_taper(
                graph,
                edge_idx,
                edge,
                graph.get_valid_node(edge.start_node),
                total_length_m,
            ),
            end: self.edge_endpoint_width_taper(
                graph,
                edge_idx,
                edge,
                graph.get_valid_node(edge.end_node),
                total_length_m,
            ),
        }
    }

    pub(in crate::simulation::network::surface::edge) fn visual_endpoint_profile_half_widths_for_edge(
        &self,
        graph: &RegionGraph,
        edge_idx: usize,
        edge: &Edge,
        total_length_m: f32,
        at_start: bool,
    ) -> (f32, f32) {
        let width_tapers =
            self.edge_width_tapers_for_width_change_node(graph, edge_idx, edge, total_length_m);
        let s_m = if at_start { 0.0 } else { total_length_m };
        Self::visual_profile_half_widths_for_edge_with_carriageway_override(
            edge,
            Some(Self::section_carriageway_half_width(
                edge,
                total_length_m,
                s_m,
                width_tapers,
            )),
        )
    }

    fn edge_endpoint_width_taper(
        &self,
        graph: &RegionGraph,
        edge_idx: usize,
        edge: &Edge,
        node_id: u32,
        total_length_m: f32,
    ) -> Option<EdgeWidthTaper> {
        let current_direction =
            self.edge_endpoint_direction_away_from_node(graph, edge, node_id)?;
        let mut road_incident_count = 0usize;
        let mut other_edge_idx = None;
        for &incident_edge_idx in graph.node_adjacency(node_id) {
            if incident_edge_idx >= graph.edge_count() {
                continue;
            }
            let incident_edge = graph.edge(incident_edge_idx);
            if !Self::edge_supports_width_change_taper(incident_edge)
                || !Self::edge_is_incident_to_node(graph, incident_edge, node_id)
            {
                continue;
            }
            road_incident_count += 1;
            if incident_edge_idx != edge_idx {
                other_edge_idx = Some(incident_edge_idx);
            }
        }
        if road_incident_count != 2 {
            return None;
        }

        let other_edge = graph.edge(other_edge_idx?);
        let other_direction =
            self.edge_endpoint_direction_away_from_node(graph, other_edge, node_id)?;
        if current_direction.dot(other_direction) > WIDTH_CHANGE_CORRIDOR_DOT_MAX {
            return None;
        }

        let full_half_width_m = Self::base_carriageway_half_width(edge);
        let target_half_width_m = Self::base_carriageway_half_width(other_edge);
        if full_half_width_m <= target_half_width_m + WIDTH_CHANGE_MIN_DELTA_M * 0.5 {
            return None;
        }

        let width_delta_m = (full_half_width_m - target_half_width_m) * 2.0;
        let requested_length_m = (width_delta_m * WIDTH_CHANGE_TAPER_WIDTH_FACTOR)
            .clamp(WIDTH_CHANGE_TAPER_MIN_M, WIDTH_CHANGE_TAPER_MAX_M);
        let max_length_m = total_length_m * WIDTH_CHANGE_TAPER_MAX_EDGE_FRACTION;
        let length_m = requested_length_m.min(max_length_m);
        (length_m > WIDTH_CHANGE_TAPER_SECTION_STEP_M).then_some(EdgeWidthTaper {
            target_carriageway_half_width_m: target_half_width_m,
            length_m,
        })
    }

    fn edge_supports_width_change_taper(edge: &Edge) -> bool {
        !edge.deleted
            && edge.class == EdgeClass::Standard
            && edge.primary_type == TransitType::Road
            && edge.allowed_types & TransitFlags::CAR != 0
    }

    fn edge_is_incident_to_node(graph: &RegionGraph, edge: &Edge, node_id: u32) -> bool {
        graph.get_valid_node(edge.start_node) == node_id
            || graph.get_valid_node(edge.end_node) == node_id
    }

    fn edge_endpoint_direction_away_from_node(
        &self,
        graph: &RegionGraph,
        edge: &Edge,
        node_id: u32,
    ) -> Option<RoadVec2> {
        let points = self.edge_points(edge);
        if points.len() < 2 {
            return None;
        }
        let (from, to) = if graph.get_valid_node(edge.start_node) == node_id {
            (points[0], points[1])
        } else if graph.get_valid_node(edge.end_node) == node_id {
            let last_idx = points.len() - 1;
            (points[last_idx], points[last_idx - 1])
        } else {
            return None;
        };

        let delta_x = f64::from(to.x - from.x);
        let delta_z = f64::from(to.z - from.z);
        let length = (delta_x * delta_x + delta_z * delta_z).sqrt();
        (length > f64::from(SAMPLE_EPSILON_M))
            .then(|| RoadVec2::new(delta_x / length, delta_z / length))
    }

    fn section_carriageway_half_width(
        edge: &Edge,
        total_length_m: f32,
        s_m: f32,
        width_tapers: EdgeWidthTapers,
    ) -> f32 {
        let base_half_width_m = Self::base_carriageway_half_width(edge);
        let mut half_width_m = base_half_width_m;
        if let Some(taper) = width_tapers.start {
            half_width_m = half_width_m.min(Self::tapered_carriageway_half_width(
                base_half_width_m,
                taper,
                s_m,
            ));
        }
        if let Some(taper) = width_tapers.end {
            half_width_m = half_width_m.min(Self::tapered_carriageway_half_width(
                base_half_width_m,
                taper,
                total_length_m - s_m,
            ));
        }
        half_width_m.max(config::LANE_WIDTH * 0.5)
    }

    fn tapered_carriageway_half_width(
        base_half_width_m: f32,
        taper: EdgeWidthTaper,
        distance_from_node_m: f32,
    ) -> f32 {
        let blend =
            1.0 - Self::smootherstep((distance_from_node_m / taper.length_m).clamp(0.0, 1.0));
        base_half_width_m + (taper.target_carriageway_half_width_m - base_half_width_m) * blend
    }

    fn base_carriageway_half_width(edge: &Edge) -> f32 {
        edge.width.max(config::LANE_WIDTH) * 0.5
    }

    fn build_section_sample_distances(
        &self,
        edge: &Edge,
        cumulative: &[f32],
        mouth_policy: EdgeMouthPolicy,
        has_start_profile: bool,
        has_end_profile: bool,
        width_tapers: EdgeWidthTapers,
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
        if let Some((start_throat, end_throat)) = mouth_policy.ownership_range {
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

        if let Some((start_profile_s_m, end_profile_s_m)) = mouth_policy.profile_range {
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
        Self::push_width_taper_section_samples(
            &mut samples,
            &mut protected_samples,
            width_tapers,
            total_length,
        );

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

    fn push_width_taper_section_samples(
        samples: &mut Vec<f32>,
        protected_samples: &mut Vec<f32>,
        width_tapers: EdgeWidthTapers,
        total_length_m: f32,
    ) {
        if let Some(taper) = width_tapers.start {
            Self::push_protected_section_sample(
                samples,
                protected_samples,
                taper.length_m,
                total_length_m,
            );
            Self::push_width_taper_transition_section_samples(
                samples,
                protected_samples,
                0.0,
                taper.length_m,
                total_length_m,
            );
        }
        if let Some(taper) = width_tapers.end {
            let taper_start_m = total_length_m - taper.length_m;
            Self::push_protected_section_sample(
                samples,
                protected_samples,
                taper_start_m,
                total_length_m,
            );
            Self::push_width_taper_transition_section_samples(
                samples,
                protected_samples,
                taper_start_m,
                total_length_m,
                total_length_m,
            );
        }
    }

    fn push_width_taper_transition_section_samples(
        samples: &mut Vec<f32>,
        protected_samples: &[f32],
        start_m: f32,
        end_m: f32,
        total_length_m: f32,
    ) {
        let start_m = start_m.clamp(0.0, total_length_m);
        let end_m = end_m.clamp(0.0, total_length_m);
        if end_m <= start_m + WIDTH_CHANGE_TAPER_SECTION_STEP_M {
            return;
        }

        let mut sample_m = start_m + WIDTH_CHANGE_TAPER_SECTION_STEP_M;
        while sample_m < end_m - SAMPLE_EPSILON_M {
            if !Self::is_near_protected_section_sample(sample_m, protected_samples) {
                samples.push(sample_m);
            }
            sample_m += WIDTH_CHANGE_TAPER_SECTION_STEP_M;
        }
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

// ========================================================================
// TESTS
// ========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::types::{NodeType, VehicleFrontageAccess};
    use godot::prelude::Vector3;

    fn test_edge(start_node: u32, end_node: u32, points: Vec<Vector3>, width: f32) -> Edge {
        let length = points
            .windows(2)
            .map(|segment| segment[0].distance_to(segment[1]))
            .sum();
        Edge {
            start_node,
            end_node,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width,
            lanes: crate::simulation::network::graph::LaneLayout::from_counts(((width / config::LANE_WIDTH).round() as u8).max(1), 0),
            speed_limit: 50.0,
            base_cost: 0.0,
            physical_length: length,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: points.clone(),
            physical_geometry: points,
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
            frontage_class: Default::default(),
        }
    }

    fn carriageway_half_width(section: &RoadSurfaceSection) -> f32 {
        section
            .bands
            .iter()
            .filter(|band| {
                band.kind == crate::simulation::network::surface::RoadSurfaceBandKind::Carriageway
            })
            .map(|band| band.lateral_start_m.abs().max(band.lateral_end_m.abs()))
            .fold(0.0, f32::max)
    }

    #[test]
    fn angled_width_change_sections_taper_wider_endpoint() {
        let surface = RoadSurfaceSystem::new(RegionGraph::CHUNK_SIZE);
        let mut graph = RegionGraph::new();
        let n0 = graph.add_node(Vector3::new(-25.0, 0.0, 15.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(25.0, 0.0, 0.0), NodeType::Junction);

        graph.add_edge(test_edge(
            n0,
            n1,
            vec![Vector3::new(-25.0, 0.0, 15.0), Vector3::ZERO],
            7.0,
        ));
        let wide_edge_idx = graph.add_edge(test_edge(
            n1,
            n2,
            vec![Vector3::ZERO, Vector3::new(25.0, 0.0, 0.0)],
            14.0,
        ));
        graph.rebuild_adjacency_list();

        let sections = surface.compile_edge_sections(&graph, wide_edge_idx);
        let first = sections.first().expect("wide edge should have sections");
        let after_taper = sections
            .iter()
            .find(|section| section.s_m >= 20.0)
            .expect("wide edge should sample the taper end");

        assert!(
            carriageway_half_width(first) <= 3.75,
            "wide angled endpoint should start at narrow carriageway width; first_half={:.2}",
            carriageway_half_width(first)
        );
        assert!(
            carriageway_half_width(after_taper) >= 6.5,
            "wide angled endpoint should return to full carriageway width after taper; after_half={:.2}",
            carriageway_half_width(after_taper)
        );
    }

    #[test]
    fn shallow_pass_through_handoff_uses_one_shared_cross_section_axis() {
        let surface = RoadSurfaceSystem::new(RegionGraph::CHUNK_SIZE);
        let mut graph = RegionGraph::new();
        let west = graph.add_node(Vector3::new(-40.0, 0.0, 0.0), NodeType::Junction);
        let center = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let east = graph.add_node(Vector3::new(40.0, 0.0, 4.0), NodeType::Junction);

        let bridge_idx = graph.add_edge(test_edge(
            west,
            center,
            vec![Vector3::new(-40.0, 0.0, 0.0), Vector3::ZERO],
            7.0,
        ));
        graph.edge_mut(bridge_idx).class = EdgeClass::Bridge;
        let approach_idx = graph.add_edge(test_edge(
            center,
            east,
            vec![Vector3::ZERO, Vector3::new(40.0, 0.0, 4.0)],
            7.0,
        ));
        graph.rebuild_adjacency_list();

        assert_eq!(
            surface.classify_surface_node_kind_from_graph_geometry(&graph, center),
            Some(CompiledNodeKind::PassThrough),
            "test setup must exercise the shallow pass-through path"
        );

        let bridge_sections = surface.compile_edge_sections(&graph, bridge_idx);
        let approach_sections = surface.compile_edge_sections(&graph, approach_idx);
        let bridge_mouth = bridge_sections
            .last()
            .expect("bridge must reach the handoff");
        let approach_mouth = approach_sections
            .first()
            .expect("approach must start at the handoff");
        let lateral_dot = bridge_mouth.lateral_xz.dot(approach_mouth.lateral_xz).abs();

        assert!(
            (1.0 - lateral_dot) <= 1.0e-9,
            "pass-through span mouths must use the same cross-section axis; abs_dot={lateral_dot:.12}"
        );
    }
}
