// SPDX-License-Identifier: GPL-2.0-only

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
    FinalProfile,
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
            // Finalization must preserve the source-fit contract too. A hard
            // design-grade cap on a supported hillside creates a cut platform
            // and steepens the return to the source profile outside the core.
            let max_sample_delta_m = sample_offsets
                .iter()
                .map(|&(dx, dz)| {
                    ((grade_x - candidate_grade_x) * dx + (grade_z - candidate_grade_z) * dz).abs()
                })
                .fold(0.0_f32, f32::max);
            if max_sample_delta_m <= JUNCTION_PROFILE_LIMIT_MAX_SAMPLE_DELTA_M {
                limited_grade_x = candidate_grade_x;
                limited_grade_z = candidate_grade_z;
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
    /// Exercises conservative source fitting in isolated profile/compiler regressions.
    /// Production edits use the shared finalizer below.
    #[cfg(test)]
    pub(in crate::simulation::network) fn solve_junction_endpoint_profiles_for_edges(
        &mut self,
        affected_nodes: &HashSet<u32>,
        adaptable_edges: &HashSet<usize>,
    ) -> HashSet<usize> {
        self.solve_indexed_endpoint_profiles(
            affected_nodes,
            adaptable_edges,
            adaptable_edges,
            JunctionProfileLimitMode::ConservativeSourceFit,
        )
    }

    /// Resolves final grade limits before applying one physical-profile transition per endpoint.
    /// The local RoadEditPlan and cold commit path share this finalization entry point.
    pub(crate) fn finalize_junction_endpoint_profiles_for_edges(
        &mut self,
        affected_nodes: &HashSet<u32>,
        adaptable_edges: &HashSet<usize>,
        authored_edges: &HashSet<usize>,
    ) -> HashSet<usize> {
        self.solve_indexed_endpoint_profiles(
            affected_nodes,
            adaptable_edges,
            authored_edges,
            JunctionProfileLimitMode::FinalProfile,
        )
    }

    fn solve_indexed_endpoint_profiles(
        &mut self,
        affected_nodes: &HashSet<u32>,
        adaptable_edges: &HashSet<usize>,
        authored_edges: &HashSet<usize>,
        limit_mode: JunctionProfileLimitMode,
    ) -> HashSet<usize> {
        if affected_nodes.is_empty() || adaptable_edges.is_empty() {
            return HashSet::new();
        }

        let mut reindex_ids = self.surface_edges_touching_nodes(affected_nodes);
        reindex_ids.sort_unstable();
        reindex_ids.dedup();
        for &edge_idx in &reindex_ids {
            self.remove_from_spatial_index(edge_idx);
        }

        let changed_edges = self.solve_junction_endpoint_profiles(
            affected_nodes,
            adaptable_edges,
            authored_edges,
            limit_mode,
        );

        for edge_idx in reindex_ids {
            self.add_to_spatial_index(edge_idx);
        }
        changed_edges
    }

    fn solve_junction_endpoint_profiles(
        &mut self,
        affected_nodes: &HashSet<u32>,
        adaptable_edges: &HashSet<usize>,
        authored_edges: &HashSet<usize>,
        limit_mode: JunctionProfileLimitMode,
    ) -> HashSet<usize> {
        let incidents_by_node = self.build_junction_profile_incidents(affected_nodes);
        let mut edge_solves: Vec<(
            usize,
            bool,
            JunctionEndpointProfilePlane,
            bool,
            bool,
            f32,
            Option<JunctionEndpointProfilePlane>,
        )> = Vec::new();

        let mut node_ids: Vec<u32> = incidents_by_node.keys().copied().collect();
        node_ids.sort_unstable();
        for node_id in node_ids {
            let incidents = &incidents_by_node[&node_id];
            if incidents.len() < 2 || self.junction_profile_incidents_form_pass_through(incidents) {
                continue;
            }
            let previous_plane = if matches!(limit_mode, JunctionProfileLimitMode::FinalProfile)
                && incidents
                    .iter()
                    .all(|incident| !authored_edges.contains(&incident.edge_idx))
            {
                // A remote split dirties the retained endpoint for topology/clips, but
                // does not author a new profile there. Apply only a change of plane to
                // its solved transition, not another blend toward that same plane.
                self.solved_junction_endpoint_profile_plane(node_id)
            } else {
                None
            };
            let stable_incidents = incidents
                .iter()
                .copied()
                .filter(|incident| !authored_edges.contains(&incident.edge_idx))
                .collect::<Vec<_>>();
            let stable_edges = stable_incidents
                .iter()
                .map(|incident| incident.edge_idx)
                .collect::<HashSet<_>>();
            let solve = if let Some(bend_solve) =
                self.solve_bend_profile_solve(node_id, incidents, limit_mode)
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
                let is_authority = solve
                    .authority_incidents
                    .contains(&(incident.edge_idx, incident.at_start));
                let adapt_control = adaptable_edges.contains(&incident.edge_idx)
                    && previous_plane.is_none()
                    && !(preserve_authority && is_authority);
                let materialize_supports = preserve_authority && !is_authority;
                // Capture bounds before applying any solve: inserting control supports can
                // change f32 endpoint tangents, which must not make later roads order-dependent.
                edge_solves.push((
                    incident.edge_idx,
                    incident.at_start,
                    solve.plane,
                    materialize_supports,
                    adapt_control,
                    self.junction_profile_crossing_core_m(incident.edge_idx, incident.at_start),
                    previous_plane,
                ));
            }
        }

        edge_solves.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut changed_edges = Vec::new();
        for (
            edge_idx,
            at_start,
            plane,
            materialize_supports,
            adapt_control,
            core_m,
            previous_plane,
        ) in edge_solves
        {
            if edge_idx >= self.edges.len() || self.edges[edge_idx].deleted {
                continue;
            }
            let edge = &self.edges[edge_idx];
            let opposite_node = self.get_valid_node(if at_start {
                edge.end_node
            } else {
                edge.start_node
            });
            let protect_opposite_endpoint = self
                .node_adjacency(opposite_node)
                .iter()
                .filter(|&&id| {
                    !self.edges[id].deleted && self.edges[id].primary_type == TransitType::Road
                })
                .take(2)
                .count()
                == 2;
            if !adapt_control {
                // Existing control geometry remains the grade authority. Its physical
                // crossing still needs the same solution as the other incident mouths.
                let edge = &self.edges[edge_idx];
                if edge.physical_geometry.len() >= 2
                    && edge.physical_geometry.iter().all(|point| {
                        (point.y - plane.height_at_xz(point.x, point.z)).abs() <= 0.0001
                    })
                {
                    continue;
                }
            }
            Self::apply_junction_profile_plane_to_edge(
                &mut self.edges[edge_idx],
                at_start,
                plane,
                materialize_supports,
                protect_opposite_endpoint,
                core_m,
                adapt_control,
                previous_plane,
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
        affected_nodes: &HashSet<u32>,
    ) -> HashMap<u32, Vec<JunctionProfileIncident>> {
        let mut incidents_by_node: HashMap<u32, Vec<JunctionProfileIncident>> = HashMap::new();

        let candidate_edge_ids = self.surface_edges_touching_nodes(affected_nodes);
        for edge_idx in candidate_edge_ids {
            let Some(edge) = self.edges.get(edge_idx) else {
                continue;
            };
            if edge.deleted
                || edge.primary_type != TransitType::Road
                || edge.geometry.len() < 2
                || edge.start_node as usize >= self.nodes.len()
                || edge.end_node as usize >= self.nodes.len()
            {
                continue;
            }

            let start_node = self.get_valid_node(edge.start_node);
            let end_node = self.get_valid_node(edge.end_node);
            for (node_id, at_start) in [(start_node, true), (end_node, false)] {
                if !affected_nodes.contains(&node_id) {
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

    /// Reconstructs crossfall from the final physical profiles at the shared crossing core.
    /// This reads the solved surface; it neither chooses a new grade target nor reblends heights.
    pub(crate) fn solved_junction_endpoint_profile_plane(
        &self,
        node_id: u32,
    ) -> Option<JunctionEndpointProfilePlane> {
        let origin = self.nodes.get(node_id as usize)?.pos;
        let (mut xx, mut xz, mut zz, mut xy, mut zy) = (0.0_f64, 0.0, 0.0, 0.0, 0.0);
        for &edge_idx in self.node_adjacency(node_id) {
            let edge = &self.edges[edge_idx];
            if edge.deleted || edge.primary_type != TransitType::Road {
                continue;
            }
            let points = if edge.physical_geometry.len() >= 2 {
                &edge.physical_geometry
            } else {
                &edge.geometry
            };
            let at_start = self.get_valid_node(edge.start_node) == node_id;
            let mut distance = 0.0;
            let mut sample = None;
            for index in 0..points.len().saturating_sub(1) {
                let (a, b) = if at_start {
                    (points[index], points[index + 1])
                } else {
                    (
                        points[points.len() - index - 1],
                        points[points.len() - index - 2],
                    )
                };
                let run = Self::edge_profile_point_distance_m(a, b);
                if run > f32::EPSILON && distance + run >= 1.0 {
                    sample = Some(a.lerp(b, (1.0 - distance) / run));
                    break;
                }
                distance += run;
            }
            let Some(sample) = sample else {
                continue;
            };
            let dx = f64::from(sample.x - origin.x);
            let dz = f64::from(sample.z - origin.z);
            let dy = f64::from(sample.y - origin.y);
            xx += dx * dx;
            xz += dx * dz;
            zz += dz * dz;
            xy += dx * dy;
            zy += dz * dy;
        }
        let det = xx * zz - xz * xz;
        if det.abs() <= f64::EPSILON * (xx * zz).max(1.0) {
            return self.junction_endpoint_profile_plane(node_id);
        }
        let plane = JunctionEndpointProfilePlane {
            origin,
            grade_x: ((xy * zz - zy * xz) / det) as f32,
            grade_z: ((xx * zy - xz * xy) / det) as f32,
        };
        // This reconstructs an already solved profile, not a speculative source fit.
        // Dropping its crossfall on a steep hill would make intersecting rails disagree.
        plane.grade().is_finite().then_some(plane)
    }

    /// Builds the canonical endpoint profile plane for a Bend/JunctionN node from incident edge mouths.
    pub(crate) fn junction_endpoint_profile_plane(
        &self,
        node_id: u32,
    ) -> Option<JunctionEndpointProfilePlane> {
        if self.nodes.get(node_id as usize)?.node_type != NodeType::Junction {
            return None;
        }
        let affected_nodes = HashSet::from([node_id]);
        let incidents_by_node = self.build_junction_profile_incidents(&affected_nodes);
        let incidents = incidents_by_node.get(&node_id)?;
        if self.junction_profile_incidents_form_pass_through(incidents) {
            return None;
        }
        self.solve_bend_profile_solve(
            node_id,
            incidents,
            JunctionProfileLimitMode::ConservativeSourceFit,
        )
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
        limit_mode: JunctionProfileLimitMode,
    ) -> Option<JunctionProfileSolve> {
        if !self.junction_profile_incidents_form_bend(incidents) {
            return None;
        }
        Some(JunctionProfileSolve {
            plane: self.solve_junction_profile_plane(node_id, incidents, limit_mode)?,
            authority_incidents: HashSet::new(),
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
        let corridors = Self::junction_profile_authority_corridors(&samples, limit_mode);
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
        limit_mode: JunctionProfileLimitMode,
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
                // Finalization uses prepared road profiles. Discarding a supported
                // steep through-road here lets a later, flatter branch steal authority.
                if !grade.is_finite()
                    || (!matches!(limit_mode, JunctionProfileLimitMode::FinalProfile)
                        && grade > JUNCTION_PROFILE_REJECT_MAX_GRADE)
                {
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
        protect_opposite_endpoint: bool,
        hard_zone_m: f32,
        adapt_control: bool,
        previous_plane: Option<JunctionEndpointProfilePlane>,
    ) {
        let total_length_m = Self::edge_profile_length_m(edge);
        if total_length_m <= JUNCTION_PROFILE_MIN_SAMPLE_M {
            return;
        }
        // A short link to another connected endpoint must not let the later solve overwrite
        // that endpoint's supports. A terminal has no competing profile and keeps the full blend.
        let available_length_m = if protect_opposite_endpoint {
            total_length_m * 0.5
        } else {
            total_length_m
        };
        let blend_end_m = (hard_zone_m + JUNCTION_PROFILE_BLEND_ZONE_M).min(available_length_m);
        if hard_zone_m < JUNCTION_PROFILE_MIN_SAMPLE_M {
            return;
        }
        let saved_control = (!adapt_control).then(|| edge.geometry.clone());
        let solve_sample_m = JUNCTION_PROFILE_SOLVE_SAMPLE_M.min(blend_end_m);
        let should_materialize_supports = materialize_supports
            || Self::endpoint_profile_support_delta_m(edge, at_start, plane, solve_sample_m)
                .is_some_and(|delta_m| delta_m > JUNCTION_PROFILE_SUPPORT_HEIGHT_EPS_M);
        if should_materialize_supports {
            Self::materialize_edge_endpoint_profile_supports(
                edge,
                at_start,
                hard_zone_m,
                blend_end_m,
            );
        }

        let mut physical_geometry = Self::physical_profile_on_control_alignment(edge);
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
            point.y = if distance_m <= hard_zone_m {
                target_y
            } else if let Some(previous) = previous_plane {
                point.y + (target_y - previous.height_at_xz(point.x, point.z)) * weight
            } else {
                point.y * (1.0 - weight) + target_y * weight
            };
        }
        edge.physical_geometry = physical_geometry;
        if let Some(control) = saved_control {
            edge.geometry = control;
        }
    }

    /// Samples the existing physical profile at control stations without importing solver pins.
    /// Both polylines follow the same authored XZ alignment; materialized support knots can differ.
    /// A monotonic station walk costs O(control points + physical points), with one output buffer.
    pub(in crate::simulation::network) fn physical_profile_on_control_alignment(
        edge: &Edge,
    ) -> Vec<Vector3> {
        let source = &edge.physical_geometry;
        if source.len() < 2 {
            return source.clone();
        }
        let mut result = Vec::with_capacity(edge.geometry.len());
        let mut source_index = 0;
        let mut source_station = 0.0;
        let mut target_station = 0.0;
        for (index, point) in edge.geometry.iter().enumerate() {
            if index > 0 {
                target_station +=
                    Self::edge_profile_point_distance_m(edge.geometry[index - 1], *point);
            }
            while source_index + 2 < source.len() {
                let run = Self::edge_profile_point_distance_m(
                    source[source_index],
                    source[source_index + 1],
                );
                if source_station + run >= target_station {
                    break;
                }
                source_station += run;
                source_index += 1;
            }
            let a = source[source_index];
            let b = source[source_index + 1];
            let run = Self::edge_profile_point_distance_m(a, b);
            let t = ((target_station - source_station) / run.max(f32::EPSILON)).clamp(0.0, 1.0);
            result.push(Vector3::new(point.x, a.y + (b.y - a.y) * t, point.z));
        }
        result
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
        hard_zone_m: f32,
        blend_end_m: f32,
    ) {
        let solve_sample_m = JUNCTION_PROFILE_SOLVE_SAMPLE_M.min(blend_end_m);
        Self::ensure_edge_endpoint_profile_support(edge, at_start, solve_sample_m);

        Self::ensure_edge_endpoint_profile_support(edge, at_start, hard_zone_m);
        let mut distance_m = hard_zone_m + JUNCTION_PROFILE_SUPPORT_STEP_M;
        while distance_m < blend_end_m - JUNCTION_PROFILE_SUPPORT_EPS_M {
            Self::ensure_edge_endpoint_profile_support(edge, at_start, distance_m);
            distance_m += JUNCTION_PROFILE_SUPPORT_STEP_M;
        }
        Self::ensure_edge_endpoint_profile_support(edge, at_start, blend_end_m);
    }

    /// End of the incident roadbed overlap, independent of node/span material ownership.
    /// Costs O(local incident profile points), with no city-wide lookup.
    pub(crate) fn junction_profile_crossing_core_m(&self, edge_idx: usize, at_start: bool) -> f32 {
        use crate::simulation::network::surface::RoadSurfaceSystem;
        let edge = &self.edges[edge_idx];
        let length = Self::edge_profile_length_m(edge);
        let mut core = Self::junction_profile_hard_zone_m(edge, length);
        let Some(direction) = Self::edge_endpoint_direction_xz(edge, at_start) else {
            return core;
        };
        let node = self.get_valid_node(if at_start {
            edge.start_node
        } else {
            edge.end_node
        });
        let width = RoadSurfaceSystem::visual_roadbed_half_width_m(edge);
        for &other_idx in self.node_adjacency(node) {
            let other = &self.edges[other_idx];
            if other_idx == edge_idx || other.deleted || other.primary_type != TransitType::Road {
                continue;
            }
            let other_start = self.get_valid_node(other.start_node) == node;
            let Some(other_direction) = Self::edge_endpoint_direction_xz(other, other_start) else {
                continue;
            };
            if Self::directions_are_pass_through(direction, other_direction) {
                continue;
            }
            let dot = direction.dot(other_direction).clamp(-1.0, 1.0);
            let sin = direction.cross(other_direction).abs();
            let other_width = RoadSurfaceSystem::visual_roadbed_half_width_m(other);
            // Extreme vertex of the intersection of two forward roadbed strips.
            // Unlike ownership, this does not reserve a whole extra roadbed width.
            let overlap = if sin <= f32::EPSILON {
                length
            } else if width + other_width * dot >= 0.0 {
                (other_width + width * dot) / sin
            } else {
                other_width * sin
            };
            core = core.max(overlap);
        }
        core.min(length * 0.5)
    }

    /// Minimum central pin for an endpoint without a wider crossing overlap.
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
