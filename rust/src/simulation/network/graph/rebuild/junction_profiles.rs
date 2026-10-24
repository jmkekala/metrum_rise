// SPDX-License-Identifier: GPL-2.0-only

//! Deterministic Bend and JunctionN endpoint-profile solving and support materialization.

use super::super::data::{Edge, RegionGraph};
use crate::simulation::network::roadbed::{CURB_STEP_HEIGHT_M, terminal_plane};
use crate::simulation::network::types::{NodeType, TransitType};
use godot::prelude::{Vector2, Vector3};
use std::collections::{HashMap, HashSet};

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

// The incidents of one node: those whose plane it solves (its roads, or its footpaths where only
// footpaths meet), and the footpaths attached to a road node's sidewalk ring (D5), which are
// pinned to the plane at sidewalk height.
#[derive(Default)]
struct NodeProfileIncidents {
    plane: Vec<JunctionProfileIncident>,
    attached: Vec<JunctionProfileIncident>,
}

#[derive(Clone, Copy)]
struct JunctionProfileIncidentSample {
    incident: JunctionProfileIncident,
    direction_xz: Vector2,
    dx: f32,
    dz: f32,
    dy: f32,
}

struct JunctionProfileCorridorCandidate {
    a: usize,
    b: usize,
    fits: bool,
    score: f32,
    grade: f32,
}

// One pinned edge end: its node's plane, the mouth station the plane holds up to, and how the
// pin eases back to the control grade.
#[derive(Clone, Copy)]
pub(super) struct EndpointPin {
    pub(super) plane: JunctionEndpointProfilePlane,
    pub(super) hard_zone_m: f32,
    // A non-authority arm of a junction with an authority corridor always takes supports.
    pub(super) force_supports: bool,
    // The far node joins another profile, so this pin may use only half the edge.
    pub(super) half_edge: bool,
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
#[derive(Clone, Copy, Debug)]
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

    /// A terminal's plane (`ROAD-68-C`): through the road end and its mouth centreline point,
    /// with the arm's end grade along that run and no crossfall; `None` when they coincide in
    /// plan.
    pub(crate) fn terminal(end: Vector3, mouth: Vector3) -> Option<Self> {
        let (dx, dz) = (mouth.x - end.x, mouth.z - end.z);
        let run_m = dx.hypot(dz);
        if run_m <= f32::EPSILON {
            return None;
        }
        let grade = (mouth.y - end.y) / run_m;
        Some(Self {
            origin: end,
            grade_x: grade * dx / run_m,
            grade_z: grade * dz / run_m,
        })
    }

    pub(super) fn grade(&self) -> f32 {
        self.grade_x.hypot(self.grade_z)
    }

    // The same plane at sidewalk height: where attached footpaths are pinned (D5).
    fn raised(self) -> Self {
        Self {
            origin: self.origin + Vector3::new(0.0, CURB_STEP_HEIGHT_M, 0.0),
            ..self
        }
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
    pub(in crate::simulation::network) fn solve_junction_endpoint_profiles_for_nodes(
        &mut self,
        affected_nodes: &HashSet<u32>,
    ) -> HashSet<usize> {
        self.solve_indexed_endpoint_profiles(
            affected_nodes,
            JunctionProfileLimitMode::ConservativeSourceFit,
        )
    }

    /// Derives the physical profile of every road edge touching `affected_nodes` from its control
    /// polyline and the pins at both of its ends: each Bend and JunctionN end's solved plane up
    /// to its mouth station (the clip, which must be current), eased back to the control grade.
    /// Control polylines are authored and never rewritten here, and the solve has no edit roles,
    /// so profiles depend only on the final graph, not on the edits that produced it (`ROAD-68`
    /// invariant 6). The local RoadEditPlan and cold commit path share this entry point. Returns
    /// the edges whose physical profile changed; `O(touching edges * points + local degree)`.
    pub(crate) fn finalize_junction_endpoint_profiles(
        &mut self,
        affected_nodes: &HashSet<u32>,
    ) -> HashSet<usize> {
        self.solve_indexed_endpoint_profiles(affected_nodes, JunctionProfileLimitMode::FinalProfile)
    }

    fn solve_indexed_endpoint_profiles(
        &mut self,
        affected_nodes: &HashSet<u32>,
        limit_mode: JunctionProfileLimitMode,
    ) -> HashSet<usize> {
        if affected_nodes.is_empty() {
            return HashSet::new();
        }

        let edges = self.surface_edges_touching_nodes(affected_nodes);
        for &edge_idx in &edges {
            self.remove_from_spatial_index(edge_idx);
        }

        let changed_edges = self.solve_junction_endpoint_profiles(&edges, limit_mode);

        for &edge_idx in &edges {
            self.add_to_spatial_index(edge_idx);
        }
        changed_edges
    }

    fn solve_junction_endpoint_profiles(
        &mut self,
        edges: &[usize],
        limit_mode: JunctionProfileLimitMode,
    ) -> HashSet<usize> {
        // Both ends of every derived edge: its far end's pin must be reproduced exactly, and the
        // far node's plane is the same function of its arms' control polylines as before.
        let pin_nodes: HashSet<u32> = edges
            .iter()
            .flat_map(|&edge_idx| {
                let edge = &self.edges[edge_idx];
                [edge.start_node, edge.end_node].map(|node| self.get_valid_node(node))
            })
            .collect();
        let incidents_by_node = self.build_junction_profile_incidents(&pin_nodes);
        let mut node_ids: Vec<u32> = incidents_by_node.keys().copied().collect();
        node_ids.sort_unstable();
        // Per pinned end: its node's plane, and whether it must take vertical-curve supports.
        let mut pins: HashMap<(usize, bool), (JunctionEndpointProfilePlane, bool)> = HashMap::new();
        // Road ends with attached footpaths: pinned once their roads' profiles are derived.
        let mut road_ends = Vec::new();
        for node_id in node_ids {
            let NodeProfileIncidents {
                plane: incidents,
                attached,
            } = &incidents_by_node[&node_id];
            if incidents.len() == 1 && !attached.is_empty() {
                road_ends.push(node_id);
            }
            if incidents.len() < 2
                || (attached.is_empty()
                    && self.junction_profile_incidents_form_pass_through(incidents))
            {
                continue;
            }
            let Some(solve) = self
                .solve_bend_profile_solve(node_id, incidents, limit_mode)
                .or_else(|| self.solve_junction_profile_solve(node_id, incidents, limit_mode))
            else {
                continue;
            };
            // A junction with an authority corridor eases every other arm onto its plane.
            let preserve_authority = solve.authority_incidents.len() >= 2
                && solve.authority_incidents.len() < incidents.len();
            for incident in incidents {
                let key = (incident.edge_idx, incident.at_start);
                let supports = preserve_authority && !solve.authority_incidents.contains(&key);
                pins.insert(key, (solve.plane, supports));
            }
            // Attached footpaths join the sidewalk ring: the plane at sidewalk height.
            for incident in attached {
                let key = (incident.edge_idx, incident.at_start);
                pins.insert(key, (solve.plane.raised(), preserve_authority));
            }
        }

        // A road end's plane runs through its road's derived profile (`roadbed::terminal_plane`),
        // so the footpaths attached there are derived after every other edge.
        let at_road_end = |edge_idx: usize| {
            road_ends.iter().any(|node| {
                incidents_by_node[node]
                    .attached
                    .iter()
                    .any(|incident| incident.edge_idx == edge_idx)
            })
        };
        let (late, early): (Vec<usize>, Vec<usize>) =
            edges.iter().partition(|&&edge_idx| at_road_end(edge_idx));
        let mut changed_edges = Vec::new();
        for (phase, edges) in [early, late].iter().enumerate() {
            if phase == 1 {
                for node_id in &road_ends {
                    let Some(plane) = terminal_plane(self, *node_id) else {
                        continue;
                    };
                    for incident in &incidents_by_node[node_id].attached {
                        pins.insert(
                            (incident.edge_idx, incident.at_start),
                            (plane.raised(), false),
                        );
                    }
                }
            }
            self.derive_pinned_profiles(edges, &pins, &mut changed_edges);
        }
        for &edge_idx in &changed_edges {
            let (cost, length) = crate::simulation::pathing::cost::CostCalculator::calculate_costs(
                &self.edges[edge_idx],
            );
            self.edges[edge_idx].base_cost = cost;
            self.edges[edge_idx].physical_length = length;
        }
        changed_edges.into_iter().collect()
    }

    // Derives the physical profile of each of `edges` from its control polyline and its end pins.
    fn derive_pinned_profiles(
        &mut self,
        edges: &[usize],
        pins: &HashMap<(usize, bool), (JunctionEndpointProfilePlane, bool)>,
        changed_edges: &mut Vec<usize>,
    ) {
        for &edge_idx in edges {
            let pin = |at_start: bool| {
                pins.get(&(edge_idx, at_start))
                    .map(|&(plane, force_supports)| EndpointPin {
                        plane,
                        hard_zone_m: self.junction_profile_hard_zone_m(edge_idx, at_start),
                        force_supports,
                        half_edge: self.edge_end_is_shared(edge_idx, !at_start),
                    })
            };
            let pins = [pin(true), pin(false)];
            let physical_geometry = Self::pinned_physical_profile(&self.edges[edge_idx], pins);
            if physical_geometry != self.edges[edge_idx].physical_geometry {
                self.edges[edge_idx].physical_geometry = physical_geometry;
                changed_edges.push(edge_idx);
            }
        }
    }

    // Whether the node at one end of an edge joins another road or footpath profile there, so a
    // pin at the other end may use only half the edge and never overwrites that end's supports.
    fn edge_end_is_shared(&self, edge_idx: usize, at_start: bool) -> bool {
        let edge = &self.edges[edge_idx];
        let node = self.get_valid_node(if at_start {
            edge.start_node
        } else {
            edge.end_node
        });
        self.node_adjacency(node)
            .iter()
            .filter(|&&id| {
                let edge = &self.edges[id];
                !edge.deleted && matches!(edge.primary_type, TransitType::Road | TransitType::Foot)
            })
            .take(2)
            .count()
            == 2
    }

    // Whether `node_id` has a live road edge: its plane is then its roads', and its footpaths
    // are attached to them (D5).
    fn has_road_arm(&self, node_id: u32) -> bool {
        self.node_adjacency(node_id).iter().any(|&id| {
            let edge = &self.edges[id];
            !edge.deleted && edge.primary_type == TransitType::Road
        })
    }

    fn build_junction_profile_incidents(
        &self,
        affected_nodes: &HashSet<u32>,
    ) -> HashMap<u32, NodeProfileIncidents> {
        let mut incidents_by_node: HashMap<u32, NodeProfileIncidents> = HashMap::new();

        let candidate_edge_ids = self.surface_edges_touching_nodes(affected_nodes);
        for edge_idx in candidate_edge_ids {
            let Some(edge) = self.edges.get(edge_idx) else {
                continue;
            };
            if edge.deleted
                || !matches!(edge.primary_type, TransitType::Road | TransitType::Foot)
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
                let incidents = incidents_by_node.entry(node_id).or_default();
                let incident = JunctionProfileIncident { edge_idx, at_start };
                if edge.primary_type == TransitType::Foot && self.has_road_arm(node_id) {
                    incidents.attached.push(incident);
                } else {
                    incidents.plane.push(incident);
                }
            }
        }

        for (&node_id, incidents) in incidents_by_node.iter_mut() {
            for list in [&mut incidents.plane, &mut incidents.attached] {
                list.sort_by(|a, b| {
                    self.junction_profile_incident_angle(node_id, *a)
                        .total_cmp(&self.junction_profile_incident_angle(node_id, *b))
                        .then(a.edge_idx.cmp(&b.edge_idx))
                        .then(a.at_start.cmp(&b.at_start))
                });
            }
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
        // A node's plane is its roads', or its footpaths' where only footpaths meet.
        let plane_type = if self.has_road_arm(node_id) {
            TransitType::Road
        } else {
            TransitType::Foot
        };
        let (mut xx, mut xz, mut zz, mut xy, mut zy) = (0.0_f64, 0.0, 0.0, 0.0, 0.0);
        for &edge_idx in self.node_adjacency(node_id) {
            let edge = &self.edges[edge_idx];
            if edge.deleted || edge.primary_type != plane_type {
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
        let NodeProfileIncidents {
            plane: incidents,
            attached,
        } = incidents_by_node.get(&node_id)?;
        if attached.is_empty() && self.junction_profile_incidents_form_pass_through(incidents) {
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
        limit_mode: JunctionProfileLimitMode,
    ) -> Option<JunctionProfileSolve> {
        self.solve_junction_profile_authority_solve(node_id, incidents, limit_mode)
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
        limit_mode: JunctionProfileLimitMode,
    ) -> Option<JunctionProfileSolve> {
        let samples = self.junction_profile_incident_samples(node_id, incidents);
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
                let Some(gradient) =
                    Self::junction_profile_corridor_gradient(samples[a], samples[b])
                else {
                    continue;
                };
                let grade = gradient.length();
                // Finalization uses prepared road profiles. Discarding a supported
                // steep through-road here lets a later, flatter branch steal authority.
                if !grade.is_finite()
                    || (!matches!(limit_mode, JunctionProfileLimitMode::FinalProfile)
                        && grade > JUNCTION_PROFILE_REJECT_MAX_GRADE)
                {
                    continue;
                }
                // Geometry alone ranks corridors; which road was built first must not matter
                // (`ROAD-68` invariant 6, open question 16). A corridor whose plane every other
                // arm already meets within the support tolerance comes first, so a fall-line
                // road crossed by a contour road keeps its grade. Then the straightest and
                // flattest pair wins, with ties by incident order: the angle of each arm's first
                // control segment from world +x. Edge ids would follow creation order, so they
                // break only exactly equal angles, which layouts (`DuplicateDirection`) and
                // placement (25 degree arms) reject before a node has a piece.
                let fits = samples.iter().enumerate().all(|(index, sample)| {
                    index == a
                        || index == b
                        || (sample.dy - gradient.x * sample.dx - gradient.y * sample.dz).abs()
                            <= JUNCTION_PROFILE_SUPPORT_HEIGHT_EPS_M
                });
                let score = -dot - grade * JUNCTION_PROFILE_AUTHORITY_GRADE_SCORE_SCALE;
                candidates.push(JunctionProfileCorridorCandidate {
                    a,
                    b,
                    fits,
                    score,
                    grade,
                });
            }
        }
        candidates.sort_by(|a, b| {
            b.fits
                .cmp(&a.fits)
                .then(b.score.total_cmp(&a.score))
                .then(a.grade.total_cmp(&b.grade))
                .then(a.a.cmp(&b.a))
                .then(a.b.cmp(&b.b))
        });
        candidates
    }

    // The corridor's gradient in XZ: its grade along the first sample's axis, no crossfall.
    fn junction_profile_corridor_gradient(
        a: JunctionProfileIncidentSample,
        b: JunctionProfileIncidentSample,
    ) -> Option<Vector2> {
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
        Some(axis * ((at * a.dy + bt * b.dy) / denom))
    }

    fn solve_junction_profile_corridor_plane(
        &self,
        node_id: u32,
        a: JunctionProfileIncidentSample,
        b: JunctionProfileIncidentSample,
        limit_mode: JunctionProfileLimitMode,
    ) -> Option<JunctionEndpointProfilePlane> {
        let origin = self.nodes.get(node_id as usize)?.pos;
        let gradient = Self::junction_profile_corridor_gradient(a, b)?;
        JunctionEndpointProfilePlane::grade_limited(
            origin,
            gradient.x,
            gradient.y,
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

    // The physical profile of `edge`, derived from its control polyline alone: a knot at each
    // pinned end's mouth, so heights lie on its plane exactly up to there, plus that end's
    // vertical-curve supports where it needs them, with heights eased back to the control grade
    // over the blend zone. Start end first; `O(points * inserted knots)`.
    pub(super) fn pinned_physical_profile(
        edge: &Edge,
        pins: [Option<EndpointPin>; 2],
    ) -> Vec<Vector3> {
        let mut points = edge.geometry.clone();
        let total_length_m = Self::edge_profile_length_m(edge);
        if total_length_m <= JUNCTION_PROFILE_MIN_SAMPLE_M {
            return points;
        }
        let mut blends = [None; 2];
        for (blend, (pin, at_start)) in blends.iter_mut().zip(pins.into_iter().zip([true, false])) {
            let Some(pin) = pin.filter(|pin| pin.hard_zone_m >= JUNCTION_PROFILE_MIN_SAMPLE_M)
            else {
                continue;
            };
            let available_length_m = if pin.half_edge {
                total_length_m * 0.5
            } else {
                total_length_m
            };
            let blend_end_m =
                (pin.hard_zone_m + JUNCTION_PROFILE_BLEND_ZONE_M).min(available_length_m);
            let solve_sample_m = JUNCTION_PROFILE_SOLVE_SAMPLE_M.min(blend_end_m);
            Self::ensure_profile_support(&mut points, at_start, pin.hard_zone_m);
            if pin.force_supports
                || Self::endpoint_profile_support_delta_m(edge, at_start, pin.plane, solve_sample_m)
                    .is_some_and(|delta_m| delta_m > JUNCTION_PROFILE_SUPPORT_HEIGHT_EPS_M)
            {
                Self::materialize_endpoint_profile_supports(
                    &mut points,
                    at_start,
                    pin.hard_zone_m,
                    blend_end_m,
                );
            }
            *blend = Some((pin, blend_end_m));
        }
        for (blend, at_start) in blends.into_iter().zip([true, false]) {
            let Some((pin, blend_end_m)) = blend else {
                continue;
            };
            let distances = Self::profile_endpoint_distances(&points, at_start);
            for (point, distance_m) in points.iter_mut().zip(distances) {
                if distance_m > blend_end_m {
                    continue;
                }
                let target_y = pin.plane.height_at_xz(point.x, point.z);
                point.y = if distance_m <= pin.hard_zone_m || blend_end_m <= pin.hard_zone_m {
                    target_y
                } else {
                    let t = ((distance_m - pin.hard_zone_m) / (blend_end_m - pin.hard_zone_m))
                        .clamp(0.0, 1.0);
                    let weight = 1.0 - Self::smootherstep(t);
                    point.y * (1.0 - weight) + target_y * weight
                };
            }
        }
        points
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

    fn materialize_endpoint_profile_supports(
        points: &mut Vec<Vector3>,
        at_start: bool,
        hard_zone_m: f32,
        blend_end_m: f32,
    ) {
        let solve_sample_m = JUNCTION_PROFILE_SOLVE_SAMPLE_M.min(blend_end_m);
        Self::ensure_profile_support(points, at_start, solve_sample_m);

        Self::ensure_profile_support(points, at_start, hard_zone_m);
        let mut distance_m = hard_zone_m + JUNCTION_PROFILE_SUPPORT_STEP_M;
        while distance_m < blend_end_m - JUNCTION_PROFILE_SUPPORT_EPS_M {
            Self::ensure_profile_support(points, at_start, distance_m);
            distance_m += JUNCTION_PROFILE_SUPPORT_STEP_M;
        }
        Self::ensure_profile_support(points, at_start, blend_end_m);
    }

    /// Hard zone of an endpoint: the arm's mouth station in its node's junction layout, which
    /// the graph clip holds (`ROAD-68-C`). Clips must be current before the solve.
    pub(crate) fn junction_profile_hard_zone_m(&self, edge_idx: usize, at_start: bool) -> f32 {
        let edge = &self.edges[edge_idx];
        if at_start {
            edge.start_clip
        } else {
            edge.end_clip
        }
    }

    fn smootherstep(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
    }

    // Inserts a knot `distance_m` along `points` from one end unless one lies within
    // `JUNCTION_PROFILE_SUPPORT_EPS_M` of it.
    fn ensure_profile_support(points: &mut Vec<Vector3>, at_start: bool, distance_m: f32) {
        if !distance_m.is_finite() || distance_m <= JUNCTION_PROFILE_SUPPORT_EPS_M {
            return;
        }
        if points.len() < 2 {
            return;
        }
        let distances = Self::profile_endpoint_distances(points, at_start);
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
            for index in 0..points.len() - 1 {
                let start_d = distances[index];
                let end_d = distances[index + 1];
                if distance_m < start_d - JUNCTION_PROFILE_SUPPORT_EPS_M
                    || distance_m > end_d + JUNCTION_PROFILE_SUPPORT_EPS_M
                {
                    continue;
                }
                let segment_m = (end_d - start_d).max(f32::EPSILON);
                let t = ((distance_m - start_d) / segment_m).clamp(0.0, 1.0);
                let point = points[index].lerp(points[index + 1], t);
                points.insert(index + 1, point);
                return;
            }
        } else {
            for index in (1..points.len()).rev() {
                let start_d = distances[index];
                let end_d = distances[index - 1];
                if distance_m < start_d - JUNCTION_PROFILE_SUPPORT_EPS_M
                    || distance_m > end_d + JUNCTION_PROFILE_SUPPORT_EPS_M
                {
                    continue;
                }
                let segment_m = (end_d - start_d).max(f32::EPSILON);
                let t = ((distance_m - start_d) / segment_m).clamp(0.0, 1.0);
                let point = points[index].lerp(points[index - 1], t);
                points.insert(index, point);
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
        Self::profile_endpoint_distances(&edge.geometry, at_start)
    }

    fn profile_endpoint_distances(points: &[Vector3], at_start: bool) -> Vec<f32> {
        let mut distances = vec![0.0; points.len()];
        if points.len() < 2 {
            return distances;
        }

        if at_start {
            for index in 1..points.len() {
                distances[index] = distances[index - 1]
                    + Self::edge_profile_point_distance_m(points[index - 1], points[index]);
            }
        } else {
            for index in (0..points.len() - 1).rev() {
                distances[index] = distances[index + 1]
                    + Self::edge_profile_point_distance_m(points[index + 1], points[index]);
            }
        }
        distances
    }
}
