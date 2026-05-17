//! Incident edge classification and mouth path collection for node pieces.

use super::*;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn build_ordered_piece_mouths(
        &self,
        incidents: &[IncidentSurfaceEdge],
    ) -> Option<Vec<OrderedIncidentPieceMouth>> {
        let mut mouths = Vec::with_capacity(incidents.len());
        for &incident in incidents {
            let profile = self.build_incident_mouth_profile(incident)?;
            let endpoint_profile = self.build_incident_endpoint_profile(incident)?;
            let (
                boundary_paths_world,
                band_start_paths_world,
                band_end_paths_world,
                uses_sampled_band_domain_paths,
            ) = self.build_incident_mouth_paths(incident, &profile, &endpoint_profile);
            mouths.push(OrderedIncidentPieceMouth {
                profile,
                endpoint_profile,
                boundary_paths_world,
                band_start_paths_world,
                band_end_paths_world,
                uses_sampled_band_domain_paths,
                direction_angle_ccw: Self::normalized_angle_ccw(incident.direction_xz),
                direction_xz: incident.direction_xz,
                edge_idx: incident.edge_idx,
                side: incident.side,
            });
        }
        mouths.sort_by(|a, b| {
            incident_direction_ordering(
                a.direction_angle_ccw,
                a.edge_idx,
                a.side,
                b.direction_angle_ccw,
                b.edge_idx,
                b.side,
            )
        });
        Some(mouths)
    }

    pub(super) fn incident_edge_visual_handoff_is_overconstrained(
        &self,
        graph: &RegionGraph,
        incident: IncidentSurfaceEdge,
    ) -> bool {
        if incident.edge_idx >= graph.edge_count() {
            return true;
        }
        let edge = graph.edge(incident.edge_idx);
        let Some(piece) = self.compiled_visual_span_pieces.get(&incident.edge_idx) else {
            return true;
        };
        let Some(sections) = self.compiled_sections.get(&incident.edge_idx) else {
            return true;
        };
        let Some(total_length_m) = sections.last().map(|section| section.s_m) else {
            return true;
        };
        if total_length_m <= SAMPLE_EPSILON_M {
            return true;
        }
        let has_current_mouth_profile = match incident.side {
            IncidentEdgeSide::Start => piece.start_mouth_profile.is_some(),
            IncidentEdgeSide::End => piece.end_mouth_profile.is_some(),
        };
        if !has_current_mouth_profile {
            return true;
        }

        let start_kind = self.classify_surface_node_kind_from_graph_geometry(
            graph,
            graph.get_valid_node(edge.start_node),
        );
        let end_kind = self.classify_surface_node_kind_from_graph_geometry(
            graph,
            graph.get_valid_node(edge.end_node),
        );
        let Some((start_handoff_s_m, end_handoff_s_m)) = self
            .visual_surface_handoff_range_for_edge(
                graph,
                incident.edge_idx,
                edge,
                total_length_m,
                start_kind,
                end_kind,
            )
        else {
            return true;
        };
        let actual_handoff_m = match incident.side {
            IncidentEdgeSide::Start => start_handoff_s_m,
            IncidentEdgeSide::End => total_length_m - end_handoff_s_m,
        };
        let opposite_handoff_m = match incident.side {
            IncidentEdgeSide::Start => total_length_m - end_handoff_s_m,
            IncidentEdgeSide::End => start_handoff_s_m,
        };
        let span_remaining_m = end_handoff_s_m - start_handoff_s_m;
        span_remaining_m <= VISUAL_MIN_SPAN_LENGTH_M + SAMPLE_EPSILON_M
            && actual_handoff_m > opposite_handoff_m * VISUAL_DOMINANT_HANDOFF_REJECTION_RATIO
    }

    fn build_incident_mouth_paths(
        &self,
        incident: IncidentSurfaceEdge,
        profile: &IncidentMouthProfile,
        endpoint_profile: &IncidentMouthProfile,
    ) -> (
        Vec<Vec<Vector3>>,
        Vec<Vec<Vector3>>,
        Vec<Vec<Vector3>>,
        bool,
    ) {
        let Some(sections) = self.compiled_sections.get(&incident.edge_idx) else {
            return (Vec::new(), Vec::new(), Vec::new(), false);
        };
        let Some(mouth_index) = sections.iter().enumerate().find_map(|(index, section)| {
            let candidate = Self::build_mouth_profile_from_section(section, incident.side)?;
            incident_mouth_profiles_match(&candidate, profile).then_some(index)
        }) else {
            return (Vec::new(), Vec::new(), Vec::new(), false);
        };

        let section_indices: Vec<usize> = match incident.side {
            IncidentEdgeSide::Start => (0..=mouth_index).rev().collect(),
            IncidentEdgeSide::End => (mouth_index..sections.len()).collect(),
        };
        let mut profile_path = Vec::with_capacity(section_indices.len());
        for section_index in section_indices {
            let Some(path_profile) =
                Self::build_mouth_profile_from_section(&sections[section_index], incident.side)
            else {
                return (Vec::new(), Vec::new(), Vec::new(), false);
            };
            if !incident_mouth_profiles_have_same_shape(profile, &path_profile) {
                return (Vec::new(), Vec::new(), Vec::new(), false);
            }
            profile_path.push(path_profile);
        }

        let Some(last_profile) = profile_path.last() else {
            return (Vec::new(), Vec::new(), Vec::new(), false);
        };
        if !incident_mouth_profiles_match(last_profile, endpoint_profile) {
            return (Vec::new(), Vec::new(), Vec::new(), false);
        }

        let uses_sampled_band_domain_paths =
            incident_profile_path_has_non_collinear_center(&profile_path);
        let boundary_paths_world = (0..profile.boundary_points_world.len())
            .map(|boundary_index| {
                incident_world_path(
                    profile_path
                        .iter()
                        .map(|path_profile| path_profile.boundary_points_world[boundary_index]),
                )
            })
            .collect();
        let band_start_paths_world = (0..profile.bands.len())
            .map(|band_index| {
                incident_world_path(
                    profile_path
                        .iter()
                        .map(|path_profile| path_profile.bands[band_index].start_point_world),
                )
            })
            .collect();
        let band_end_paths_world = (0..profile.bands.len())
            .map(|band_index| {
                incident_world_path(
                    profile_path
                        .iter()
                        .map(|path_profile| path_profile.bands[band_index].end_point_world),
                )
            })
            .collect();
        (
            boundary_paths_world,
            band_start_paths_world,
            band_end_paths_world,
            uses_sampled_band_domain_paths,
        )
    }

    fn build_incident_mouth_profile(
        &self,
        incident: IncidentSurfaceEdge,
    ) -> Option<IncidentMouthProfile> {
        let piece = self.compiled_visual_span_pieces.get(&incident.edge_idx)?;
        match incident.side {
            IncidentEdgeSide::Start => piece.start_mouth_profile.clone(),
            IncidentEdgeSide::End => piece.end_mouth_profile.clone(),
        }
    }

    fn build_incident_endpoint_profile(
        &self,
        incident: IncidentSurfaceEdge,
    ) -> Option<IncidentMouthProfile> {
        let sections = self.compiled_sections.get(&incident.edge_idx)?;
        let section = match incident.side {
            IncidentEdgeSide::Start => sections.first()?,
            IncidentEdgeSide::End => sections.last()?,
        };
        Self::build_mouth_profile_from_section(section, incident.side)
    }

    fn normalized_angle_ccw(direction_xz: Vector2) -> f32 {
        let angle = direction_xz.y.atan2(direction_xz.x);
        if angle < 0.0 {
            angle + std::f32::consts::TAU
        } else {
            angle
        }
    }

    #[cfg(test)]
    pub(in crate::simulation::network::surface) fn left_normal_xz(
        direction_xz: Vector2,
    ) -> Vector2 {
        Vector2::new(-direction_xz.y, direction_xz.x)
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
                let straight = a.direction_xz.dot(b.direction_xz) <= -PASS_THROUGH_DOT_THRESHOLD;
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
    ) -> Option<Vector2> {
        let points = self.edge_points(edge);
        if points.len() < 2 {
            return None;
        }

        match side {
            IncidentEdgeSide::Start => {
                let endpoint = points[0];
                points.iter().skip(1).find_map(|point| {
                    let direction = Vector2::new(point.x - endpoint.x, point.z - endpoint.z);
                    (direction.length_squared() > SAMPLE_EPSILON_M * SAMPLE_EPSILON_M)
                        .then(|| direction.normalized())
                })
            }
            IncidentEdgeSide::End => {
                let endpoint = *points.last()?;
                points.iter().rev().skip(1).find_map(|point| {
                    let direction = Vector2::new(point.x - endpoint.x, point.z - endpoint.z);
                    (direction.length_squared() > SAMPLE_EPSILON_M * SAMPLE_EPSILON_M)
                        .then(|| direction.normalized())
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
        mut direction_for: impl FnMut(&Self, usize, &Edge, IncidentEdgeSide) -> Option<Vector2>,
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
    ) -> Option<Vector2> {
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

fn incident_direction_ordering(
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

fn incident_mouth_profiles_match(
    left: &IncidentMouthProfile,
    right: &IncidentMouthProfile,
) -> bool {
    incident_mouth_profiles_have_same_shape(left, right)
        && left
            .boundary_points_world
            .iter()
            .zip(&right.boundary_points_world)
            .all(|(left, right)| {
                ArrangementBoundaryPointKey::from_world(*left)
                    == ArrangementBoundaryPointKey::from_world(*right)
            })
        && left.bands.iter().zip(&right.bands).all(|(left, right)| {
            ArrangementBoundaryPointKey::from_world(left.start_point_world)
                == ArrangementBoundaryPointKey::from_world(right.start_point_world)
                && ArrangementBoundaryPointKey::from_world(left.end_point_world)
                    == ArrangementBoundaryPointKey::from_world(right.end_point_world)
        })
}

fn incident_mouth_profiles_have_same_shape(
    left: &IncidentMouthProfile,
    right: &IncidentMouthProfile,
) -> bool {
    left.boundary_points_world.len() == right.boundary_points_world.len()
        && left.bands.len() == right.bands.len()
        && left
            .bands
            .iter()
            .zip(&right.bands)
            .all(|(left, right)| left.kind == right.kind)
}

fn incident_world_path(points: impl IntoIterator<Item = Vector3>) -> Vec<Vector3> {
    points.into_iter().collect()
}

fn incident_profile_path_has_non_collinear_center(path: &[IncidentMouthProfile]) -> bool {
    if path.len() <= 2 {
        return false;
    }
    let Some(start) = path.first().and_then(incident_profile_center_key) else {
        return false;
    };
    let Some(end) = path.last().and_then(incident_profile_center_key) else {
        return false;
    };
    if start == end {
        return false;
    }
    let dx = i128::from(end.x_key() - start.x_key());
    let dz = i128::from(end.z_key() - start.z_key());
    path[1..path.len() - 1].iter().any(|profile| {
        let Some(key) = incident_profile_center_key(profile) else {
            return false;
        };
        let px = i128::from(key.x_key() - start.x_key());
        let pz = i128::from(key.z_key() - start.z_key());
        px * dz - pz * dx != 0
    })
}

fn incident_profile_center_key(profile: &IncidentMouthProfile) -> Option<NodeArrangementKey> {
    let first = profile.boundary_points_world.first()?;
    let last = profile.boundary_points_world.last()?;
    let center = (*first + *last) * 0.5;
    Some(NodeArrangementKey::from_point(
        super::backend::godot_vec3_xz_to_road(center),
    ))
}
