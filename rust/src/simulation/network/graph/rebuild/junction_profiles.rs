//! Deterministic Bend and JunctionN endpoint-profile solving and support materialization.

use super::super::data::{Edge, RegionGraph};
use crate::simulation::network::types::{NodeType, TransitType};
use godot::prelude::{Vector2, Vector3};
use std::collections::{HashMap, HashSet};

const JUNCTION_PROFILE_HARD_ZONE_MIN_M: f32 = 1.0;
const JUNCTION_PROFILE_HARD_ZONE_MAX_M: f32 = 2.0;
pub(crate) const JUNCTION_PROFILE_BLEND_ZONE_M: f32 = 32.0;
pub(super) const JUNCTION_PROFILE_SOLVE_SAMPLE_M: f32 = 12.0;
const JUNCTION_PROFILE_MIN_SAMPLE_M: f32 = 1.0;
pub(super) const JUNCTION_PROFILE_MOUTH_MAX_GRADE: f32 = 0.16;
const JUNCTION_PROFILE_LIMIT_MAX_SAMPLE_DELTA_M: f32 = 0.5;
const JUNCTION_PROFILE_REJECT_MAX_GRADE: f32 = 0.5;
const JUNCTION_PROFILE_PLANE_DET_EPS: f32 = 1.0e-5;
const JUNCTION_PROFILE_SUPPORT_EPS_M: f32 = 0.01;
const JUNCTION_PROFILE_SUPPORT_HEIGHT_EPS_M: f32 = 0.05;
pub(super) const JUNCTION_PROFILE_SUPPORT_STEP_M: f32 = 2.0;
const JUNCTION_PROFILE_AUTHORITY_CORRIDOR_DOT: f32 = -0.86;
const JUNCTION_PROFILE_AUTHORITY_GRADE_SCORE_SCALE: f32 = 2.0;

#[derive(Clone, Copy)]
struct JunctionProfileIncident {
    edge_idx: usize,
    at_start: bool,
}

#[derive(Clone, Copy)]
struct JunctionProfileIncidentSample {
    incident: JunctionProfileIncident,
    direction_xz: Vector2,
    dx: f32,
    dz: f32,
    dy: f32,
    stable: bool,
}

struct JunctionProfileCorridorCandidate {
    a: usize,
    b: usize,
    score: f32,
    grade: f32,
}

struct JunctionProfileSolve {
    plane: JunctionEndpointProfilePlane,
    authority_incidents: HashSet<(usize, bool)>,
}

#[derive(Clone, Copy)]
enum JunctionProfileLimitMode {
    ConservativeSourceFit,
    RegradePlatform,
}

/// Node-local profile plane used to make incident Bend/JunctionN mouth rails height-compatible.
#[derive(Clone, Copy)]
pub(crate) struct JunctionEndpointProfilePlane {
    pub(super) origin: Vector3,
    pub(super) grade_x: f32,
    pub(super) grade_z: f32,
}

impl JunctionEndpointProfilePlane {
    /// Evaluates the solved endpoint profile height at an arbitrary world XZ coordinate.
    pub(crate) fn height_at_xz(&self, x: f32, z: f32) -> f32 {
        self.origin.y + self.grade_x * (x - self.origin.x) + self.grade_z * (z - self.origin.z)
    }

    pub(super) fn grade(&self) -> f32 {
        self.grade_x.hypot(self.grade_z)
    }

    fn grade_limited(
        origin: Vector3,
        grade_x: f32,
        grade_z: f32,
        sample_offsets: &[(f32, f32)],
        limit_mode: JunctionProfileLimitMode,
    ) -> Option<Self> {
        let grade = grade_x.hypot(grade_z);
        if !grade.is_finite()
            || matches!(limit_mode, JunctionProfileLimitMode::ConservativeSourceFit)
                && grade > JUNCTION_PROFILE_REJECT_MAX_GRADE
        {
            return None;
        }
        let mut limited_grade_x = grade_x;
        let mut limited_grade_z = grade_z;
        if grade > JUNCTION_PROFILE_MOUTH_MAX_GRADE {
            let scale = JUNCTION_PROFILE_MOUTH_MAX_GRADE / grade;
            let candidate_grade_x = grade_x * scale;
            let candidate_grade_z = grade_z * scale;
            match limit_mode {
                JunctionProfileLimitMode::ConservativeSourceFit => {
                    let max_sample_delta_m = sample_offsets
                        .iter()
                        .map(|&(dx, dz)| {
                            ((grade_x - candidate_grade_x) * dx
                                + (grade_z - candidate_grade_z) * dz)
                                .abs()
                        })
                        .fold(0.0_f32, f32::max);
                    if max_sample_delta_m <= JUNCTION_PROFILE_LIMIT_MAX_SAMPLE_DELTA_M {
                        limited_grade_x = candidate_grade_x;
                        limited_grade_z = candidate_grade_z;
                    }
                }
                JunctionProfileLimitMode::RegradePlatform => {
                    limited_grade_x = candidate_grade_x;
                    limited_grade_z = candidate_grade_z;
                }
            }
        };
        Some(Self {
            origin,
            grade_x: limited_grade_x,
            grade_z: limited_grade_z,
        })
    }
}

impl RegionGraph {
    /// Adapts newly authored edge endpoints to existing Bend/JunctionN grade/profile anchors.
    ///
    /// The node compiler consumes the resulting edge profiles as source authority; it still
    /// rejects any contradictory mouth heights that remain after this edit-stage solve.
    pub(in crate::simulation::network) fn solve_junction_endpoint_profiles_for_edges(
        &mut self,
        affected_nodes: &HashSet<u32>,
        adaptable_edges: &HashSet<usize>,
    ) -> HashSet<usize> {
        if affected_nodes.is_empty() || adaptable_edges.is_empty() {
            return HashSet::new();
        }

        let valid_node_ids: Vec<u32> = (0..self.nodes.len())
            .map(|i| self.get_valid_node(i as u32))
            .collect();
        let mut reindex_ids = adaptable_edges
            .iter()
            .copied()
            .filter(|&edge_idx| edge_idx < self.edges.len() && !self.edges[edge_idx].deleted)
            .collect::<Vec<_>>();
        reindex_ids.sort_unstable();
        reindex_ids.dedup();
        for &edge_idx in &reindex_ids {
            self.remove_from_spatial_index(edge_idx);
        }

        let changed_edges = self.solve_junction_endpoint_profiles(
            &valid_node_ids,
            affected_nodes,
            adaptable_edges,
            JunctionProfileLimitMode::ConservativeSourceFit,
        );

        for edge_idx in reindex_ids {
            self.add_to_spatial_index(edge_idx);
        }
        changed_edges
    }

    /// Regrades affected over-limit Bend/JunctionN mouths through source profile geometry.
    ///
    /// This is the stronger road-edit path: only nodes whose conservative profile still exceeds
    /// the grade cap are selected, then adaptable non-authority mouths receive real support
    /// vertices so later section compiles sample the solved profile.
    pub(in crate::simulation::network) fn regrade_junction_endpoint_profiles_for_nodes(
        &mut self,
        affected_nodes: &HashSet<u32>,
        adaptable_edges: &HashSet<usize>,
    ) -> HashSet<usize> {
        if affected_nodes.is_empty() || adaptable_edges.is_empty() {
            return HashSet::new();
        }

        let valid_node_ids: Vec<u32> = (0..self.nodes.len())
            .map(|i| self.get_valid_node(i as u32))
            .collect();
        let regrade_nodes =
            self.junction_profile_regrade_nodes(&valid_node_ids, affected_nodes, adaptable_edges);
        if regrade_nodes.is_empty() {
            return HashSet::new();
        }

        let mut reindex_ids = self.surface_edges_touching_nodes(&regrade_nodes);
        reindex_ids.retain(|edge_idx| adaptable_edges.contains(edge_idx));
        reindex_ids.sort_unstable();
        reindex_ids.dedup();
        if reindex_ids.is_empty() {
            return HashSet::new();
        }

        for &edge_idx in &reindex_ids {
            self.remove_from_spatial_index(edge_idx);
        }

        let effective_adaptable_edges = reindex_ids.iter().copied().collect::<HashSet<_>>();
        let changed_edges = self.solve_junction_endpoint_profiles(
            &valid_node_ids,
            &regrade_nodes,
            &effective_adaptable_edges,
            JunctionProfileLimitMode::RegradePlatform,
        );

        for edge_idx in reindex_ids {
            self.add_to_spatial_index(edge_idx);
        }
        changed_edges
    }

    fn junction_profile_regrade_nodes(
        &self,
        valid_node_ids: &[u32],
        affected_nodes: &HashSet<u32>,
        adaptable_edges: &HashSet<usize>,
    ) -> HashSet<u32> {
        let incidents_by_node =
            self.build_junction_profile_incidents(valid_node_ids, Some(affected_nodes));
        incidents_by_node
            .iter()
            .filter_map(|(&node_id, incidents)| {
                if incidents.len() < 2
                    || self.junction_profile_incidents_form_pass_through(incidents)
                {
                    return None;
                }
                if !incidents
                    .iter()
                    .any(|incident| adaptable_edges.contains(&incident.edge_idx))
                {
                    return None;
                }

                let stable_edges = incidents
                    .iter()
                    .filter_map(|incident| {
                        (!adaptable_edges.contains(&incident.edge_idx)).then_some(incident.edge_idx)
                    })
                    .collect::<HashSet<_>>();
                if let Some(bend_solve) = self.solve_bend_profile_solve(node_id, incidents) {
                    return self
                        .bend_profile_requires_regrade(incidents, adaptable_edges, bend_solve.plane)
                        .then_some(node_id);
                }
                let conservative = self.solve_junction_profile_solve(
                    node_id,
                    incidents,
                    &stable_edges,
                    JunctionProfileLimitMode::ConservativeSourceFit,
                );
                if conservative.as_ref().is_some_and(|solve| {
                    solve.plane.grade() > JUNCTION_PROFILE_MOUTH_MAX_GRADE + 1.0e-4
                }) || conservative.is_none()
                    && self
                        .solve_junction_profile_solve(
                            node_id,
                            incidents,
                            &stable_edges,
                            JunctionProfileLimitMode::RegradePlatform,
                        )
                        .is_some()
                {
                    Some(node_id)
                } else {
                    None
                }
            })
            .collect()
    }

    fn solve_junction_endpoint_profiles(
        &mut self,
        valid_node_ids: &[u32],
        affected_nodes: &HashSet<u32>,
        adaptable_edges: &HashSet<usize>,
        limit_mode: JunctionProfileLimitMode,
    ) -> HashSet<usize> {
        let incidents_by_node =
            self.build_junction_profile_incidents(valid_node_ids, Some(affected_nodes));
        let mut edge_solves: Vec<(usize, bool, JunctionEndpointProfilePlane, bool)> = Vec::new();

        let mut node_ids: Vec<u32> = incidents_by_node.keys().copied().collect();
        node_ids.sort_unstable();
        for node_id in node_ids {
            let incidents = &incidents_by_node[&node_id];
            if incidents.len() < 2 || self.junction_profile_incidents_form_pass_through(incidents) {
                continue;
            }
            let stable_incidents = incidents
                .iter()
                .copied()
                .filter(|incident| !adaptable_edges.contains(&incident.edge_idx))
                .collect::<Vec<_>>();
            let stable_edges = stable_incidents
                .iter()
                .map(|incident| incident.edge_idx)
                .collect::<HashSet<_>>();
            let solve = if let Some(bend_solve) = self.solve_bend_profile_solve(node_id, incidents)
            {
                Some(bend_solve)
            } else if stable_incidents.len() >= 2 {
                self.solve_junction_profile_solve(
                    node_id,
                    &stable_incidents,
                    &stable_edges,
                    limit_mode,
                )
                .or_else(|| {
                    self.solve_junction_profile_solve(node_id, incidents, &stable_edges, limit_mode)
                })
            } else {
                self.solve_junction_profile_solve(node_id, incidents, &stable_edges, limit_mode)
            };
            let Some(solve) = solve else {
                continue;
            };
            let preserve_authority = solve.authority_incidents.len() >= 2
                && solve.authority_incidents.len() < incidents.len();
            for incident in incidents {
                if !adaptable_edges.contains(&incident.edge_idx) {
                    continue;
                }
                let is_authority = solve
                    .authority_incidents
                    .contains(&(incident.edge_idx, incident.at_start));
                if preserve_authority && is_authority {
                    continue;
                }
                let materialize_supports =
                    matches!(limit_mode, JunctionProfileLimitMode::RegradePlatform)
                        || (preserve_authority && !is_authority);
                edge_solves.push((
                    incident.edge_idx,
                    incident.at_start,
                    solve.plane,
                    materialize_supports,
                ));
            }
        }

        edge_solves.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut changed_edges = Vec::new();
        for (edge_idx, at_start, plane, materialize_supports) in edge_solves {
            if edge_idx >= self.edges.len() || self.edges[edge_idx].deleted {
                continue;
            }
            Self::apply_junction_profile_plane_to_edge(
                &mut self.edges[edge_idx],
                at_start,
                plane,
                materialize_supports,
            );
            changed_edges.push(edge_idx);
        }
        changed_edges.sort_unstable();
        changed_edges.dedup();
        for &edge_idx in &changed_edges {
            let (cost, length) = crate::simulation::pathing::cost::CostCalculator::calculate_costs(
                &self.edges[edge_idx],
            );
            self.edges[edge_idx].base_cost = cost;
            self.edges[edge_idx].physical_length = length;
        }
        changed_edges.into_iter().collect()
    }

    fn build_junction_profile_incidents(
        &self,
        valid_node_ids: &[u32],
        affected_nodes: Option<&HashSet<u32>>,
    ) -> HashMap<u32, Vec<JunctionProfileIncident>> {
        let mut incidents_by_node: HashMap<u32, Vec<JunctionProfileIncident>> = HashMap::new();

        let candidate_edge_ids = affected_nodes
            .map(|affected| self.surface_edges_touching_nodes(affected))
            .unwrap_or_else(|| (0..self.edges.len()).collect());
        for edge_idx in candidate_edge_ids {
            let Some(edge) = self.edges.get(edge_idx) else {
                continue;
            };
            if edge.deleted
                || edge.primary_type != TransitType::Road
                || edge.geometry.len() < 2
                || edge.start_node as usize >= valid_node_ids.len()
                || edge.end_node as usize >= valid_node_ids.len()
            {
                continue;
            }

            let start_node = valid_node_ids[edge.start_node as usize];
            let end_node = valid_node_ids[edge.end_node as usize];
            for (node_id, at_start) in [(start_node, true), (end_node, false)] {
                if affected_nodes.is_some_and(|affected| !affected.contains(&node_id)) {
                    continue;
                }
                if self.nodes[node_id as usize].node_type != NodeType::Junction {
                    continue;
                }
                incidents_by_node
                    .entry(node_id)
                    .or_default()
                    .push(JunctionProfileIncident { edge_idx, at_start });
            }
        }

        for (&node_id, incidents) in incidents_by_node.iter_mut() {
            incidents.sort_by(|a, b| {
                self.junction_profile_incident_angle(node_id, *a)
                    .total_cmp(&self.junction_profile_incident_angle(node_id, *b))
                    .then(a.edge_idx.cmp(&b.edge_idx))
                    .then(a.at_start.cmp(&b.at_start))
            });
        }
        incidents_by_node
    }

    fn junction_profile_incident_angle(
        &self,
        node_id: u32,
        incident: JunctionProfileIncident,
    ) -> f32 {
        let Some(edge) = self.edges.get(incident.edge_idx) else {
            return 0.0;
        };
        let Some(origin) = self.nodes.get(node_id as usize).map(|node| node.pos) else {
            return 0.0;
        };
        let away = if incident.at_start {
            edge.geometry.get(1).copied()
        } else {
            edge.geometry
                .len()
                .checked_sub(2)
                .and_then(|index| edge.geometry.get(index).copied())
        };
        let Some(away) = away else {
            return 0.0;
        };
        (away.z - origin.z).atan2(away.x - origin.x)
    }

    /// Builds the canonical endpoint profile plane for a Bend/JunctionN node from incident edge mouths.
    pub(crate) fn junction_endpoint_profile_plane(
        &self,
        node_id: u32,
    ) -> Option<JunctionEndpointProfilePlane> {
        if self.nodes.get(node_id as usize)?.node_type != NodeType::Junction {
            return None;
        }
        let valid_node_ids: Vec<u32> = (0..self.nodes.len())
            .map(|i| self.get_valid_node(i as u32))
            .collect();
        let affected_nodes = HashSet::from([node_id]);
        let incidents_by_node =
            self.build_junction_profile_incidents(&valid_node_ids, Some(&affected_nodes));
        let incidents = incidents_by_node.get(&node_id)?;
        if self.junction_profile_incidents_form_pass_through(incidents) {
            return None;
        }
        self.solve_bend_profile_solve(node_id, incidents)
            .or_else(|| {
                self.solve_junction_profile_solve(
                    node_id,
                    incidents,
                    &HashSet::new(),
                    JunctionProfileLimitMode::ConservativeSourceFit,
                )
            })
            .map(|solve| solve.plane)
    }

    fn solve_bend_profile_solve(
        &self,
        node_id: u32,
        incidents: &[JunctionProfileIncident],
    ) -> Option<JunctionProfileSolve> {
        if !self.junction_profile_incidents_form_bend(incidents) {
            return None;
        }
        Some(JunctionProfileSolve {
            plane: JunctionEndpointProfilePlane {
                origin: self.nodes.get(node_id as usize)?.pos,
                grade_x: 0.0,
                grade_z: 0.0,
            },
            authority_incidents: HashSet::new(),
        })
    }

    fn bend_profile_requires_regrade(
        &self,
        incidents: &[JunctionProfileIncident],
        adaptable_edges: &HashSet<usize>,
        plane: JunctionEndpointProfilePlane,
    ) -> bool {
        incidents
            .iter()
            .filter(|incident| adaptable_edges.contains(&incident.edge_idx))
            .any(|incident| {
                let Some(edge) = self.edges.get(incident.edge_idx) else {
                    return false;
                };
                let total_length_m = Self::edge_profile_length_m(edge);
                if total_length_m <= JUNCTION_PROFILE_MIN_SAMPLE_M {
                    return false;
                }
                let hard_zone_m = Self::junction_profile_hard_zone_m(edge, total_length_m);
                let blend_end_m = (hard_zone_m + JUNCTION_PROFILE_BLEND_ZONE_M).min(total_length_m);
                if hard_zone_m < JUNCTION_PROFILE_MIN_SAMPLE_M {
                    return false;
                }
                let solve_sample_m = JUNCTION_PROFILE_SOLVE_SAMPLE_M.min(blend_end_m);
                Self::endpoint_profile_support_delta_m(
                    edge,
                    incident.at_start,
                    plane,
                    solve_sample_m,
                )
                .is_some_and(|delta_m| delta_m > JUNCTION_PROFILE_SUPPORT_HEIGHT_EPS_M)
            })
    }

    fn junction_profile_incidents_form_bend(&self, incidents: &[JunctionProfileIncident]) -> bool {
        let [_, _] = incidents else {
            return false;
        };
        !self.junction_profile_incidents_form_pass_through(incidents)
    }

    fn junction_profile_incidents_form_pass_through(
        &self,
        incidents: &[JunctionProfileIncident],
    ) -> bool {
        let [a, b] = incidents else {
            return false;
        };
        let Some(a_direction) = self
            .edges
            .get(a.edge_idx)
            .and_then(|edge| Self::edge_endpoint_direction_xz(edge, a.at_start))
        else {
            return false;
        };
        let Some(b_direction) = self
            .edges
            .get(b.edge_idx)
            .and_then(|edge| Self::edge_endpoint_direction_xz(edge, b.at_start))
        else {
            return false;
        };

        Self::directions_are_pass_through(a_direction, b_direction)
    }

    fn solve_junction_profile_solve(
        &self,
        node_id: u32,
        incidents: &[JunctionProfileIncident],
        stable_edges: &HashSet<usize>,
        limit_mode: JunctionProfileLimitMode,
    ) -> Option<JunctionProfileSolve> {
        self.solve_junction_profile_authority_solve(node_id, incidents, stable_edges, limit_mode)
            .or_else(|| {
                self.solve_junction_profile_plane(node_id, incidents, limit_mode)
                    .map(|plane| JunctionProfileSolve {
                        plane,
                        authority_incidents: HashSet::new(),
                    })
            })
    }

    fn solve_junction_profile_authority_solve(
        &self,
        node_id: u32,
        incidents: &[JunctionProfileIncident],
        stable_edges: &HashSet<usize>,
        limit_mode: JunctionProfileLimitMode,
    ) -> Option<JunctionProfileSolve> {
        let samples = self.junction_profile_incident_samples(node_id, incidents, stable_edges);
        if samples.len() < 2 {
            return None;
        }
        let corridors = Self::junction_profile_authority_corridors(&samples);
        let best_corridor = corridors.first()?;
        let authority_incidents = [best_corridor.a, best_corridor.b]
            .into_iter()
            .map(|index| {
                (
                    samples[index].incident.edge_idx,
                    samples[index].incident.at_start,
                )
            })
            .collect::<HashSet<_>>();
        let plane = self.solve_junction_profile_corridor_plane(
            node_id,
            samples[best_corridor.a],
            samples[best_corridor.b],
            limit_mode,
        )?;
        Some(JunctionProfileSolve {
            plane,
            authority_incidents,
        })
    }

    fn junction_profile_incident_samples(
        &self,
        node_id: u32,
        incidents: &[JunctionProfileIncident],
        stable_edges: &HashSet<usize>,
    ) -> Vec<JunctionProfileIncidentSample> {
        let Some(origin) = self.nodes.get(node_id as usize).map(|node| node.pos) else {
            return Vec::new();
        };
        let mut samples = Vec::with_capacity(incidents.len());
        for incident in incidents {
            let Some(edge) = self.edges.get(incident.edge_idx) else {
                continue;
            };
            let Some(direction_xz) = Self::edge_endpoint_direction_xz(edge, incident.at_start)
            else {
                continue;
            };
            let total_length_m = Self::edge_profile_length_m(edge);
            if total_length_m <= JUNCTION_PROFILE_MIN_SAMPLE_M {
                continue;
            }
            let sample_distance_m = JUNCTION_PROFILE_SOLVE_SAMPLE_M.min(total_length_m * 0.5);
            if sample_distance_m < JUNCTION_PROFILE_MIN_SAMPLE_M {
                continue;
            }
            let Some(sample) = Self::sample_edge_geometry_from_endpoint(
                edge,
                incident.at_start,
                sample_distance_m,
            ) else {
                continue;
            };
            let dx = sample.x - origin.x;
            let dz = sample.z - origin.z;
            if dx * dx + dz * dz <= JUNCTION_PROFILE_MIN_SAMPLE_M * JUNCTION_PROFILE_MIN_SAMPLE_M {
                continue;
            }
            samples.push(JunctionProfileIncidentSample {
                incident: *incident,
                direction_xz,
                dx,
                dz,
                dy: sample.y - origin.y,
                stable: stable_edges.contains(&incident.edge_idx),
            });
        }
        samples
    }

    fn junction_profile_authority_corridors(
        samples: &[JunctionProfileIncidentSample],
    ) -> Vec<JunctionProfileCorridorCandidate> {
        let mut candidates = Vec::new();
        for a in 0..samples.len() {
            for b in a + 1..samples.len() {
                let dot = samples[a]
                    .direction_xz
                    .dot(samples[b].direction_xz)
                    .clamp(-1.0, 1.0);
                if dot > JUNCTION_PROFILE_AUTHORITY_CORRIDOR_DOT {
                    continue;
                }
                let Some(grade) = Self::junction_profile_corridor_grade(samples[a], samples[b])
                else {
                    continue;
                };
                if grade > JUNCTION_PROFILE_REJECT_MAX_GRADE {
                    continue;
                }
                let stable_count = usize::from(samples[a].stable) + usize::from(samples[b].stable);
                let score = stable_count as f32 * 4.0 + (-dot)
                    - grade * JUNCTION_PROFILE_AUTHORITY_GRADE_SCORE_SCALE;
                candidates.push(JunctionProfileCorridorCandidate { a, b, score, grade });
            }
        }
        candidates.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then(a.grade.total_cmp(&b.grade))
                .then(a.a.cmp(&b.a))
                .then(a.b.cmp(&b.b))
        });
        candidates
    }

    fn junction_profile_corridor_grade(
        a: JunctionProfileIncidentSample,
        b: JunctionProfileIncidentSample,
    ) -> Option<f32> {
        let axis = Vector2::new(a.dx, a.dz);
        let axis_len = axis.length();
        if axis_len <= f32::EPSILON {
            return None;
        }
        let axis = axis / axis_len;
        let at = a.dx * axis.x + a.dz * axis.y;
        let bt = b.dx * axis.x + b.dz * axis.y;
        let denom = at * at + bt * bt;
        if denom <= f32::EPSILON {
            return None;
        }
        Some(((at * a.dy + bt * b.dy) / denom).abs())
    }

    fn solve_junction_profile_corridor_plane(
        &self,
        node_id: u32,
        a: JunctionProfileIncidentSample,
        b: JunctionProfileIncidentSample,
        limit_mode: JunctionProfileLimitMode,
    ) -> Option<JunctionEndpointProfilePlane> {
        let origin = self.nodes.get(node_id as usize)?.pos;
        let axis = Vector2::new(a.dx, a.dz);
        let axis_len = axis.length();
        if axis_len <= f32::EPSILON {
            return None;
        }
        let axis = axis / axis_len;
        let at = a.dx * axis.x + a.dz * axis.y;
        let bt = b.dx * axis.x + b.dz * axis.y;
        let denom = at * at + bt * bt;
        if denom <= f32::EPSILON {
            return None;
        }
        let grade = (at * a.dy + bt * b.dy) / denom;
        JunctionEndpointProfilePlane::grade_limited(
            origin,
            grade * axis.x,
            grade * axis.y,
            &[(a.dx, a.dz), (b.dx, b.dz)],
            limit_mode,
        )
    }

    fn solve_junction_profile_plane(
        &self,
        node_id: u32,
        incidents: &[JunctionProfileIncident],
        limit_mode: JunctionProfileLimitMode,
    ) -> Option<JunctionEndpointProfilePlane> {
        let origin = self.nodes.get(node_id as usize)?.pos;
        let mut xx = 0.0;
        let mut xz = 0.0;
        let mut zz = 0.0;
        let mut xy = 0.0;
        let mut zy = 0.0;
        let mut sample_count = 0;
        let mut sample_offsets = Vec::with_capacity(incidents.len());

        for incident in incidents {
            let edge = self.edges.get(incident.edge_idx)?;
            let total_length_m = Self::edge_profile_length_m(edge);
            if total_length_m <= JUNCTION_PROFILE_MIN_SAMPLE_M {
                continue;
            }
            let sample_distance_m = JUNCTION_PROFILE_SOLVE_SAMPLE_M.min(total_length_m * 0.5);
            if sample_distance_m < JUNCTION_PROFILE_MIN_SAMPLE_M {
                continue;
            }
            let Some(sample) = Self::sample_edge_geometry_from_endpoint(
                edge,
                incident.at_start,
                sample_distance_m,
            ) else {
                continue;
            };
            let dx = sample.x - origin.x;
            let dz = sample.z - origin.z;
            let dy = sample.y - origin.y;
            if dx * dx + dz * dz <= JUNCTION_PROFILE_MIN_SAMPLE_M * JUNCTION_PROFILE_MIN_SAMPLE_M {
                continue;
            }
            xx += dx * dx;
            xz += dx * dz;
            zz += dz * dz;
            xy += dx * dy;
            zy += dz * dy;
            sample_count += 1;
            sample_offsets.push((dx, dz));
        }

        if sample_count < 2 {
            return None;
        }
        let det = xx * zz - xz * xz;
        if det.abs() <= JUNCTION_PROFILE_PLANE_DET_EPS {
            return None;
        }

        let grade_x = (xy * zz - zy * xz) / det;
        let grade_z = (xx * zy - xz * xy) / det;
        JunctionEndpointProfilePlane::grade_limited(
            origin,
            grade_x,
            grade_z,
            &sample_offsets,
            limit_mode,
        )
    }

    pub(super) fn apply_junction_profile_plane_to_edge(
        edge: &mut Edge,
        at_start: bool,
        plane: JunctionEndpointProfilePlane,
        materialize_supports: bool,
    ) {
        let total_length_m = Self::edge_profile_length_m(edge);
        if total_length_m <= JUNCTION_PROFILE_MIN_SAMPLE_M {
            return;
        }
        let hard_zone_m = Self::junction_profile_hard_zone_m(edge, total_length_m);
        let blend_end_m =
            (hard_zone_m + JUNCTION_PROFILE_BLEND_ZONE_M).min(total_length_m.max(hard_zone_m));
        if hard_zone_m < JUNCTION_PROFILE_MIN_SAMPLE_M {
            return;
        }
        let solve_sample_m = JUNCTION_PROFILE_SOLVE_SAMPLE_M.min(blend_end_m);
        let should_materialize_supports = materialize_supports
            || Self::endpoint_profile_support_delta_m(edge, at_start, plane, solve_sample_m)
                .is_some_and(|delta_m| delta_m > JUNCTION_PROFILE_SUPPORT_HEIGHT_EPS_M);
        if should_materialize_supports {
            Self::materialize_edge_endpoint_profile_supports(edge, at_start, blend_end_m);
        }

        let mut physical_geometry = edge.geometry.clone();
        let solve_sample_m = should_materialize_supports.then_some(solve_sample_m);
        let distances = Self::edge_endpoint_distances(edge, at_start);
        for (point, distance_m) in edge.geometry.iter_mut().zip(distances.iter().copied()) {
            if distance_m > blend_end_m {
                continue;
            }
            let target_y = plane.origin.y
                + plane.grade_x * (point.x - plane.origin.x)
                + plane.grade_z * (point.z - plane.origin.z);
            let weight = if distance_m <= hard_zone_m
                || solve_sample_m.is_some_and(|sample_m| {
                    (distance_m - sample_m).abs() <= JUNCTION_PROFILE_SUPPORT_EPS_M
                })
                || blend_end_m <= hard_zone_m
            {
                1.0
            } else {
                let t = ((distance_m - hard_zone_m) / (blend_end_m - hard_zone_m)).clamp(0.0, 1.0);
                1.0 - Self::smootherstep(t)
            };
            point.y = point.y * (1.0 - weight) + target_y * weight;
        }
        for (point, distance_m) in physical_geometry.iter_mut().zip(distances.iter().copied()) {
            if distance_m > blend_end_m {
                continue;
            }
            let target_y = plane.origin.y
                + plane.grade_x * (point.x - plane.origin.x)
                + plane.grade_z * (point.z - plane.origin.z);
            let weight = if distance_m <= hard_zone_m || blend_end_m <= hard_zone_m {
                1.0
            } else {
                let t = ((distance_m - hard_zone_m) / (blend_end_m - hard_zone_m)).clamp(0.0, 1.0);
                1.0 - Self::smootherstep(t)
            };
            point.y = point.y * (1.0 - weight) + target_y * weight;
        }
        edge.physical_geometry = physical_geometry;
    }

    fn endpoint_profile_support_delta_m(
        edge: &Edge,
        at_start: bool,
        plane: JunctionEndpointProfilePlane,
        distance_m: f32,
    ) -> Option<f32> {
        let sample = Self::sample_edge_geometry_from_endpoint(edge, at_start, distance_m)?;
        Some((sample.y - plane.height_at_xz(sample.x, sample.z)).abs())
    }

    fn materialize_edge_endpoint_profile_supports(
        edge: &mut Edge,
        at_start: bool,
        blend_end_m: f32,
    ) {
        let solve_sample_m = JUNCTION_PROFILE_SOLVE_SAMPLE_M.min(blend_end_m);
        Self::ensure_edge_endpoint_profile_support(edge, at_start, solve_sample_m);

        let mut distance_m = solve_sample_m + JUNCTION_PROFILE_SUPPORT_STEP_M;
        while distance_m < blend_end_m - JUNCTION_PROFILE_SUPPORT_EPS_M {
            Self::ensure_edge_endpoint_profile_support(edge, at_start, distance_m);
            distance_m += JUNCTION_PROFILE_SUPPORT_STEP_M;
        }
        Self::ensure_edge_endpoint_profile_support(edge, at_start, blend_end_m);
    }

    /// Returns the short endpoint distance that stays exactly on the JunctionN profile plane.
    pub(crate) fn junction_profile_hard_zone_m(edge: &Edge, total_length_m: f32) -> f32 {
        (edge.width * 0.25)
            .clamp(
                JUNCTION_PROFILE_HARD_ZONE_MIN_M,
                JUNCTION_PROFILE_HARD_ZONE_MAX_M,
            )
            .min(total_length_m * 0.25)
    }

    fn smootherstep(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
    }

    fn ensure_edge_endpoint_profile_support(edge: &mut Edge, at_start: bool, distance_m: f32) {
        if !distance_m.is_finite() || distance_m <= JUNCTION_PROFILE_SUPPORT_EPS_M {
            return;
        }
        if edge.geometry.len() < 2 {
            return;
        }
        let distances = Self::edge_endpoint_distances(edge, at_start);
        let Some(&total_length_m) = distances.iter().max_by(|a, b| a.total_cmp(b)) else {
            return;
        };
        if distance_m >= total_length_m - JUNCTION_PROFILE_SUPPORT_EPS_M {
            return;
        }
        if distances
            .iter()
            .any(|existing| (*existing - distance_m).abs() <= JUNCTION_PROFILE_SUPPORT_EPS_M)
        {
            return;
        }

        if at_start {
            for index in 0..edge.geometry.len() - 1 {
                let start_d = distances[index];
                let end_d = distances[index + 1];
                if distance_m < start_d - JUNCTION_PROFILE_SUPPORT_EPS_M
                    || distance_m > end_d + JUNCTION_PROFILE_SUPPORT_EPS_M
                {
                    continue;
                }
                let segment_m = (end_d - start_d).max(f32::EPSILON);
                let t = ((distance_m - start_d) / segment_m).clamp(0.0, 1.0);
                let point = edge.geometry[index].lerp(edge.geometry[index + 1], t);
                edge.geometry.insert(index + 1, point);
                return;
            }
        } else {
            for index in (1..edge.geometry.len()).rev() {
                let start_d = distances[index];
                let end_d = distances[index - 1];
                if distance_m < start_d - JUNCTION_PROFILE_SUPPORT_EPS_M
                    || distance_m > end_d + JUNCTION_PROFILE_SUPPORT_EPS_M
                {
                    continue;
                }
                let segment_m = (end_d - start_d).max(f32::EPSILON);
                let t = ((distance_m - start_d) / segment_m).clamp(0.0, 1.0);
                let point = edge.geometry[index].lerp(edge.geometry[index - 1], t);
                edge.geometry.insert(index, point);
                return;
            }
        }
    }

    pub(super) fn sample_edge_geometry_from_endpoint(
        edge: &Edge,
        at_start: bool,
        distance_m: f32,
    ) -> Option<Vector3> {
        let distances = Self::edge_endpoint_distances(edge, at_start);
        let points = &edge.geometry;
        if points.is_empty() {
            return None;
        }
        if points.len() == 1 {
            return Some(points[0]);
        }

        if at_start {
            for index in 0..points.len() - 1 {
                let start_d = distances[index];
                let end_d = distances[index + 1];
                if distance_m > end_d && index + 2 < points.len() {
                    continue;
                }
                let segment_m = (end_d - start_d).max(f32::EPSILON);
                let t = ((distance_m - start_d) / segment_m).clamp(0.0, 1.0);
                return Some(points[index].lerp(points[index + 1], t));
            }
        } else {
            for index in (1..points.len()).rev() {
                let start_d = distances[index];
                let end_d = distances[index - 1];
                if distance_m > end_d && index > 1 {
                    continue;
                }
                let segment_m = (end_d - start_d).max(f32::EPSILON);
                let t = ((distance_m - start_d) / segment_m).clamp(0.0, 1.0);
                return Some(points[index].lerp(points[index - 1], t));
            }
        }

        if at_start {
            points.last().copied()
        } else {
            points.first().copied()
        }
    }

    fn edge_endpoint_distances(edge: &Edge, at_start: bool) -> Vec<f32> {
        let mut distances = vec![0.0; edge.geometry.len()];
        if edge.geometry.len() < 2 {
            return distances;
        }

        if at_start {
            for index in 1..edge.geometry.len() {
                distances[index] = distances[index - 1]
                    + Self::edge_profile_point_distance_m(
                        edge.geometry[index - 1],
                        edge.geometry[index],
                    );
            }
        } else {
            for index in (0..edge.geometry.len() - 1).rev() {
                distances[index] = distances[index + 1]
                    + Self::edge_profile_point_distance_m(
                        edge.geometry[index + 1],
                        edge.geometry[index],
                    );
            }
        }
        distances
    }
}
