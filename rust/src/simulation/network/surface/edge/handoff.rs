//! Visual span/node handoff distances and material-conflict ownership growth.

use super::super::{CompiledNodeKind, IncidentEdgeSide, RoadSurfaceSystem, SAMPLE_EPSILON_M};
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::EdgeClass;

// Visual span/node ownership handoff guards.
const VISUAL_NODE_HANDOFF_PADDING_M: f32 = 1.0;
pub(in crate::simulation::network::surface::edge) const VISUAL_MIN_SPAN_LENGTH_M: f32 = 0.5;
const VISUAL_CONFLICT_PASS_THROUGH_DOT_THRESHOLD: f32 = 0.98;
const VISUAL_CONFLICT_SIN_EPSILON: f32 = 1.0e-3;
const VISUAL_CONFLICT_MAX_HALFWIDTH_FACTOR: f32 = 5.0;

/// Shared visual-mouth decisions for one compiled edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::simulation::network::surface) struct EdgeMouthPolicy {
    /// Span-owned section bounds after node-mouth material conflict has been reserved.
    pub(in crate::simulation::network::surface) ownership_range: Option<(f32, f32)>,
    /// Profile-blend bounds used by junction and bend grade smoothing.
    pub(in crate::simulation::network::surface) profile_range: Option<(f32, f32)>,
}

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn visual_roadbed_half_width_m(edge: &Edge) -> f32 {
        Self::visual_profile_half_widths_for_edge(edge).0
    }

    pub(in crate::simulation::network::surface) fn visual_carriageway_half_width_m(
        edge: &Edge,
    ) -> f32 {
        Self::visual_profile_half_widths_for_edge(edge).1
    }

    pub(in crate::simulation::network::surface) fn visual_node_handoff_limit_m(edge: &Edge) -> f32 {
        Self::visual_roadbed_half_width_m(edge) + VISUAL_NODE_HANDOFF_PADDING_M
    }

    fn visual_terminal_handoff_m(edge: &Edge, total_length_m: f32) -> f32 {
        Self::visual_node_handoff_limit_m(edge).clamp(0.0, total_length_m)
    }

    /// Returns the shared visual mouth policy for one edge.
    ///
    /// The ownership range controls which longitudinal section range belongs to the edge span
    /// instead of the node piece. The profile range controls grade blending near junction and bend
    /// mouths, and may intentionally differ from ownership for sparse grounded bends.
    pub(in crate::simulation::network::surface) fn visual_edge_mouth_policy_for_edge(
        &self,
        graph: &RegionGraph,
        edge_idx: usize,
        edge: &Edge,
        total_length_m: f32,
        start_kind: Option<CompiledNodeKind>,
        end_kind: Option<CompiledNodeKind>,
        has_start_profile: bool,
        has_end_profile: bool,
    ) -> EdgeMouthPolicy {
        let ownership_range = self.visual_surface_ownership_range_for_edge(
            graph,
            edge_idx,
            edge,
            total_length_m,
            start_kind,
            end_kind,
        );
        let profile_range = Self::visual_profile_blend_range_for_edge(
            edge,
            total_length_m,
            ownership_range,
            start_kind,
            end_kind,
            has_start_profile,
            has_end_profile,
        );
        EdgeMouthPolicy {
            ownership_range,
            profile_range,
        }
    }

    fn visual_surface_ownership_range_for_edge(
        &self,
        graph: &RegionGraph,
        edge_idx: usize,
        edge: &Edge,
        total_length_m: f32,
        start_kind: Option<CompiledNodeKind>,
        end_kind: Option<CompiledNodeKind>,
    ) -> Option<(f32, f32)> {
        if total_length_m <= SAMPLE_EPSILON_M {
            return None;
        }

        let mut start_handoff = self.visual_node_handoff_distance_for_edge(
            graph,
            edge_idx,
            edge,
            total_length_m,
            start_kind,
            true,
        );
        let mut end_handoff = self.visual_node_handoff_distance_for_edge(
            graph,
            edge_idx,
            edge,
            total_length_m,
            end_kind,
            false,
        );
        let max_handoff_total = (total_length_m - VISUAL_MIN_SPAN_LENGTH_M).max(0.0);
        let handoff_total = start_handoff + end_handoff;
        if handoff_total > max_handoff_total && handoff_total > SAMPLE_EPSILON_M {
            let scale = max_handoff_total / handoff_total;
            start_handoff *= scale;
            end_handoff *= scale;
        }

        let start_s = start_handoff.clamp(0.0, total_length_m);
        let end_s = (total_length_m - end_handoff).clamp(0.0, total_length_m);
        (end_s - start_s > SAMPLE_EPSILON_M).then_some((start_s, end_s))
    }

    fn visual_profile_blend_range_for_edge(
        edge: &Edge,
        total_length_m: f32,
        ownership_range: Option<(f32, f32)>,
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
        } else if let Some((start_handoff_m, _)) = ownership_range {
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
        } else if let Some((_, end_handoff_m)) = ownership_range {
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

    fn visual_node_handoff_distance_for_edge(
        &self,
        graph: &RegionGraph,
        edge_idx: usize,
        edge: &Edge,
        total_length_m: f32,
        kind: Option<CompiledNodeKind>,
        at_start: bool,
    ) -> f32 {
        match kind {
            Some(CompiledNodeKind::Terminal) if edge.class == EdgeClass::Standard => {
                Self::visual_terminal_handoff_m(edge, total_length_m)
            }
            Some(CompiledNodeKind::Terminal) => 0.0,
            Some(CompiledNodeKind::Bend | CompiledNodeKind::JunctionN) => self
                .visual_material_conflict_handoff_m(
                    graph,
                    edge_idx,
                    edge,
                    total_length_m,
                    at_start,
                ),
            _ => 0.0,
        }
    }

    fn visual_material_conflict_handoff_m(
        &self,
        graph: &RegionGraph,
        edge_idx: usize,
        edge: &Edge,
        total_length_m: f32,
        at_start: bool,
    ) -> f32 {
        let side = if at_start {
            IncidentEdgeSide::Start
        } else {
            IncidentEdgeSide::End
        };
        let node_id = if at_start {
            graph.get_valid_node(edge.start_node)
        } else {
            graph.get_valid_node(edge.end_node)
        };
        let mut required_handoff = if at_start {
            Self::visual_start_handoff_m(edge, total_length_m)
        } else {
            Self::visual_end_handoff_m(edge, total_length_m)
        };

        let incidents = self.sorted_incident_surface_edges_from_graph_geometry(graph, node_id);
        let Some(current) = incidents
            .iter()
            .find(|incident| incident.edge_idx == edge_idx && incident.side == side)
        else {
            return required_handoff.clamp(0.0, total_length_m);
        };
        let roadbed_half_width_m = Self::visual_roadbed_half_width_m(edge);
        let carriageway_half_width_m = Self::visual_carriageway_half_width_m(edge);

        for other in &incidents {
            if other.edge_idx == edge_idx && other.side == side {
                continue;
            }
            let other_edge = graph.edge(other.edge_idx);
            let dot = current
                .direction_xz
                .dot(other.direction_xz)
                .clamp(-1.0, 1.0) as f32;
            if dot <= -VISUAL_CONFLICT_PASS_THROUGH_DOT_THRESHOLD {
                continue;
            }

            let sin_theta = (current.direction_xz.x * other.direction_xz.y
                - current.direction_xz.y * other.direction_xz.x)
                .abs() as f32;
            let other_roadbed_half_width_m = Self::visual_roadbed_half_width_m(other_edge);
            let pair_required = if sin_theta <= VISUAL_CONFLICT_SIN_EPSILON {
                total_length_m
            } else {
                let other_carriageway_half_width_m =
                    Self::visual_carriageway_half_width_m(other_edge);
                [
                    roadbed_half_width_m + other_roadbed_half_width_m,
                    roadbed_half_width_m + other_carriageway_half_width_m,
                    carriageway_half_width_m + other_roadbed_half_width_m,
                ]
                .into_iter()
                .map(|width_m| width_m / sin_theta)
                .fold(0.0, f32::max)
            };
            let widths_differ = (roadbed_half_width_m - other_roadbed_half_width_m).abs() > 0.1;
            let max_pair_handoff_m = if widths_differ {
                total_length_m
            } else {
                (roadbed_half_width_m.max(other_roadbed_half_width_m)
                    * VISUAL_CONFLICT_MAX_HALFWIDTH_FACTOR)
                    .max(Self::visual_node_handoff_limit_m(edge))
            };
            if pair_required.is_finite() {
                required_handoff = required_handoff.max(pair_required.min(max_pair_handoff_m));
            }
        }

        required_handoff.clamp(0.0, total_length_m)
    }

    fn visual_start_handoff_m(edge: &Edge, total_length_m: f32) -> f32 {
        edge.start_clip
            .max(Self::visual_node_handoff_limit_m(edge))
            .clamp(0.0, total_length_m)
    }

    fn visual_end_handoff_m(edge: &Edge, total_length_m: f32) -> f32 {
        edge.end_clip
            .max(Self::visual_node_handoff_limit_m(edge))
            .clamp(0.0, total_length_m)
    }
}
