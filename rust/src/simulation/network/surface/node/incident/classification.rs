//! Incident edge collection and visual node kind classification.

use super::*;

impl RoadSurfaceSystem {
    pub(super) fn normalized_angle_ccw(direction_xz: RoadVec2) -> f32 {
        let angle = direction_xz.y.atan2(direction_xz.x) as f32;
        if angle < 0.0 {
            angle + std::f32::consts::TAU
        } else {
            angle
        }
    }

    #[cfg(test)]
    pub(in crate::simulation::network::surface) fn left_normal_xz(
        direction_xz: RoadVec2,
    ) -> RoadVec2 {
        RoadVec2::new(-direction_xz.y, direction_xz.x)
    }

    pub(in crate::simulation::network::surface) fn classify_visual_node_kind(
        &self,
        incidents: &[IncidentSurfaceEdge],
    ) -> CompiledNodeKind {
        match incidents.len() {
            0 | 1 => CompiledNodeKind::Terminal,
            2 => {
                let a = incidents[0];
                let b = incidents[1];
                let straight =
                    a.direction_xz.dot(b.direction_xz) <= f64::from(-PASS_THROUGH_DOT_THRESHOLD);
                if !straight {
                    return CompiledNodeKind::Bend;
                }
                CompiledNodeKind::PassThrough
            }
            _ => CompiledNodeKind::JunctionN,
        }
    }

    pub(in crate::simulation::network::surface) fn classify_surface_node_kind_from_graph_geometry(
        &self,
        graph: &RegionGraph,
        node_id: u32,
    ) -> Option<CompiledNodeKind> {
        let incidents = self.sorted_incident_surface_edges_from_graph_geometry(graph, node_id);
        (!incidents.is_empty()).then(|| self.classify_visual_node_kind(&incidents))
    }

    pub(in crate::simulation::network::surface) fn sorted_incident_surface_edges_from_graph_geometry(
        &self,
        graph: &RegionGraph,
        node_id: u32,
    ) -> Vec<IncidentSurfaceEdge> {
        let mut incidents = self.collect_incident_surface_edges_from_graph_geometry(graph, node_id);
        incidents.sort_by(Self::incident_surface_edge_direction_ordering);
        incidents
    }

    fn collect_incident_surface_edges_from_graph_geometry(
        &self,
        graph: &RegionGraph,
        node_id: u32,
    ) -> Vec<IncidentSurfaceEdge> {
        self.collect_incident_surface_edges_with_direction(
            graph,
            node_id,
            |surface, _, edge, side| surface.incident_direction_from_edge_geometry(edge, side),
        )
    }

    fn incident_direction_from_edge_geometry(
        &self,
        edge: &Edge,
        side: IncidentEdgeSide,
    ) -> Option<RoadVec2> {
        let points = self.edge_points(edge);
        if points.len() < 2 {
            return None;
        }

        match side {
            IncidentEdgeSide::Start => {
                let endpoint = points[0];
                points.iter().skip(1).find_map(|point| {
                    let direction = RoadVec2::new(
                        f64::from(point.x - endpoint.x),
                        f64::from(point.z - endpoint.z),
                    );
                    (direction.length_squared() > f64::from(SAMPLE_EPSILON_M * SAMPLE_EPSILON_M))
                        .then(|| direction.normalize())
                })
            }
            IncidentEdgeSide::End => {
                let endpoint = *points.last()?;
                points.iter().rev().skip(1).find_map(|point| {
                    let direction = RoadVec2::new(
                        f64::from(point.x - endpoint.x),
                        f64::from(point.z - endpoint.z),
                    );
                    (direction.length_squared() > f64::from(SAMPLE_EPSILON_M * SAMPLE_EPSILON_M))
                        .then(|| direction.normalize())
                })
            }
        }
    }

    pub(in crate::simulation::network::surface) fn sorted_incident_surface_edges(
        &self,
        graph: &RegionGraph,
        node_id: u32,
    ) -> Vec<IncidentSurfaceEdge> {
        let mut incidents = self.collect_incident_surface_edges(graph, node_id);
        incidents.sort_by(Self::incident_surface_edge_direction_ordering);
        incidents
    }

    fn collect_incident_surface_edges(
        &self,
        graph: &RegionGraph,
        node_id: u32,
    ) -> Vec<IncidentSurfaceEdge> {
        self.collect_incident_surface_edges_with_direction(
            graph,
            node_id,
            |surface, edge_idx, _, side| {
                surface.incident_direction_from_compiled_mouth(edge_idx, side)
            },
        )
    }

    fn collect_incident_surface_edges_with_direction(
        &self,
        graph: &RegionGraph,
        node_id: u32,
        mut direction_for: impl FnMut(&Self, usize, &Edge, IncidentEdgeSide) -> Option<RoadVec2>,
    ) -> Vec<IncidentSurfaceEdge> {
        if node_id as usize >= graph.node_adjacency_count() {
            return Vec::new();
        }

        let mut incidents = Vec::new();
        for &edge_idx in graph.node_adjacency(node_id) {
            if edge_idx >= graph.edge_count() {
                continue;
            }
            let edge = graph.edge(edge_idx);
            if !Self::is_surface_edge(edge) {
                continue;
            }

            let side = if graph.get_valid_node(edge.start_node) == node_id {
                Some(IncidentEdgeSide::Start)
            } else if graph.get_valid_node(edge.end_node) == node_id {
                Some(IncidentEdgeSide::End)
            } else {
                None
            };
            let Some(side) = side else {
                continue;
            };
            let Some(direction_xz) = direction_for(self, edge_idx, edge, side) else {
                continue;
            };
            incidents.push(IncidentSurfaceEdge {
                edge_idx,
                side,
                direction_xz,
            });
        }

        incidents.sort_by(|a, b| a.edge_idx.cmp(&b.edge_idx).then(a.side.cmp(&b.side)));
        incidents
    }

    fn incident_direction_from_compiled_mouth(
        &self,
        edge_idx: usize,
        side: IncidentEdgeSide,
    ) -> Option<RoadVec2> {
        let piece = self.compiled_visual_span_pieces.get(&edge_idx)?;
        match side {
            IncidentEdgeSide::Start => piece
                .start_mouth_profile
                .as_ref()
                .map(|mouth| mouth.inward_direction_xz),
            IncidentEdgeSide::End => piece
                .end_mouth_profile
                .as_ref()
                .map(|mouth| mouth.inward_direction_xz),
        }
    }

    fn incident_surface_edge_direction_ordering(
        a: &IncidentSurfaceEdge,
        b: &IncidentSurfaceEdge,
    ) -> std::cmp::Ordering {
        incident_direction_ordering(
            Self::normalized_angle_ccw(a.direction_xz),
            a.edge_idx,
            a.side,
            Self::normalized_angle_ccw(b.direction_xz),
            b.edge_idx,
            b.side,
        )
    }
}

pub(super) fn incident_direction_ordering(
    left_angle_ccw: f32,
    left_edge_idx: usize,
    left_side: IncidentEdgeSide,
    right_angle_ccw: f32,
    right_edge_idx: usize,
    right_side: IncidentEdgeSide,
) -> std::cmp::Ordering {
    left_angle_ccw
        .total_cmp(&right_angle_ccw)
        .then(left_edge_idx.cmp(&right_edge_idx))
        .then(left_side.cmp(&right_side))
}
