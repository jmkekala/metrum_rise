// SPDX-License-Identifier: GPL-2.0-only

//! Deterministic Bend and JunctionN endpoint-profile solving and support materialization.

use super::super::data::{Edge, RegionGraph};
use crate::simulation::network::grade::GradeScratch;
use crate::simulation::network::roadbed::{
    CURB_STEP_HEIGHT_M, EdgeEnd, RoadVec2, directions_pass_through, edges_pass_through,
    incident_direction, terminal_plane,
};
use crate::simulation::network::types::{EdgeClass, NodeType, TransitType};
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::{Vector2, Vector3};
use std::cell::RefCell;
use std::collections::HashSet;

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
const JUNCTION_PROFILE_AUTHORITY_CORRIDOR_DOT: f64 = -0.86;

// One edge end at a node, with its direction away from the node: the node layout's own
// (`roadbed::incident_direction`, towards the first control point beyond 1 mm), in `f64`.
#[derive(Clone, Copy)]
struct JunctionProfileIncident {
    edge_idx: usize,
    at_start: bool,
    direction: RoadVec2,
}

// The incidents of one node, each in incident order: those whose plane it solves (its roads, or
// its footpaths where only footpaths meet), and the footpaths attached to a road node's sidewalk
// ring (D5), which are pinned to the plane at sidewalk height.
#[derive(Clone, Copy)]
struct NodeProfileIncidents<'a> {
    plane: &'a [JunctionProfileIncident],
    attached: &'a [JunctionProfileIncident],
}

// The incidents of the nodes one solve reads (`build_junction_profile_incidents`), grouped by
// node in node order.
#[derive(Default)]
struct ProfileIncidents {
    // The live road and footpath edges touching the nodes.
    candidates: Vec<usize>,
    // Each incident with its node, whether it is attached and its direction angle, sorted.
    keyed: Vec<(u32, bool, f64, JunctionProfileIncident)>,
    // The incidents in that order.
    incidents: Vec<JunctionProfileIncident>,
    // Per node with an incident, in node order: its id and where its plane incidents start,
    // its attached ones start and they end in `incidents`.
    nodes: Vec<(u32, usize, usize, usize)>,
}

impl ProfileIncidents {
    fn node(&self, index: usize) -> (u32, NodeProfileIncidents<'_>) {
        let (node, plane, attached, end) = self.nodes[index];
        let incidents = NodeProfileIncidents {
            plane: &self.incidents[plane..attached],
            attached: &self.incidents[attached..end],
        };
        (node, incidents)
    }
}

// The junction profile solve's buffers, reused between solves so that a solve allocates nothing
// once they have grown to its neighbourhood (`ROAD-68-K17`): the solve runs on every preview
// pose, and the node checks solve single nodes again.
#[derive(Default)]
struct ProfileSolveScratch {
    // The edges the solve derives, sorted.
    edges: Vec<usize>,
    // Their ends' nodes, sorted and unique.
    pin_nodes: Vec<u32>,
    incidents: ProfileIncidents,
    samples: Vec<JunctionProfileIncidentSample>,
    // Per derived edge, by its index in `edges`, the pins of its start and end: the node's
    // plane, and whether that end takes vertical-curve supports.
    pins: Vec<[Option<(JunctionEndpointProfilePlane, bool)>; 2]>,
    // The road ends with attached footpaths, as indices into `incidents.nodes`.
    road_ends: Vec<usize>,
    // The derived edges whose physical profile changed.
    changed: Vec<usize>,
    profile: ProfileBuffers,
}

// The buffers of one derived profile (`pinned_physical_profile_into`).
#[derive(Default)]
struct ProfileBuffers {
    // The profile being derived.
    points: Vec<Vector3>,
    // Each point's distance from the end being pinned.
    distances: Vec<f32>,
    grade: GradeScratch,
}

thread_local! {
    // One per thread: pool workers keep theirs between solves.
    static PROFILE_SCRATCH: RefCell<ProfileSolveScratch> = RefCell::default();
}

impl ProfileSolveScratch {
    // Runs `solve` with this thread's scratch. Taken for the call, not borrowed, as
    // `GradeScratch` is, so a solve nested on the same thread starts from empty buffers instead
    // of failing a borrow.
    fn with<R>(solve: impl FnOnce(&mut Self) -> R) -> R {
        let mut scratch = PROFILE_SCRATCH.take();
        let result = solve(&mut scratch);
        PROFILE_SCRATCH.set(scratch);
        result
    }
}

#[derive(Clone, Copy)]
struct JunctionProfileIncidentSample {
    incident: JunctionProfileIncident,
    dx: f32,
    dz: f32,
    dy: f32,
}

// A corridor's ranking keys, compared lexicographically (`ROAD-68` "Heights").
struct JunctionProfileCorridorCandidate {
    a: usize,
    b: usize,
    fits: bool,
    dot: f64,
    grade: f64,
}

// The outcome of one node's plane solve.
enum NodeSolve {
    // Nothing to pin: fewer than two plane incidents, or a pass-through pair without attached
    // footpaths, whose spans share one section.
    Unpinned,
    Solved(JunctionProfileSolve),
    // An arm too short to sample, or samples that fix no plane: `check_node` rejects the node
    // (`NodeError::NoPlane`) before an edit commits.
    NoPlane,
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
    // The authority corridor's two incidents, as `(edge, at_start)`; `None` for a plane fitted
    // to every sample.
    authority: Option<[(usize, bool); 2]>,
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
        samples: &[JunctionProfileIncidentSample],
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
            let max_sample_delta_m = samples
                .iter()
                .map(|sample| {
                    ((grade_x - candidate_grade_x) * sample.dx
                        + (grade_z - candidate_grade_z) * sample.dz)
                        .abs()
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
        let mut changed_edges = HashSet::new();
        self.solve_indexed_endpoint_profiles(
            affected_nodes,
            JunctionProfileLimitMode::ConservativeSourceFit,
            None,
            &mut changed_edges,
        );
        changed_edges
    }

    /// Derives the physical profile of every road edge touching `affected_nodes` from its control
    /// polyline and the pins at both of its ends: each Bend and JunctionN end's solved plane up
    /// to its mouth station (the clip, which must be current), eased back to the control grade.
    /// Control polylines are authored and never rewritten here, and the solve has no edit roles,
    /// so profiles depend only on the final graph, not on the edits that produced it (`ROAD-68`
    /// invariant 6). The blends of standard roads keep within the source-delta band of the
    /// source `terrain` (`ROAD-68` open question 39); `None` only for a graph without a world (a
    /// unit fixture), whose blends have no band. The local RoadEditPlan and cold commit path
    /// share this entry point. Inserts the edges whose physical profile changed into
    /// `changed_edges`; `O(touching edges * points + local degree)`. The solve runs in this
    /// thread's buffers, so it allocates nothing once they have grown to its neighbourhood but
    /// the profiles it changes and the R-tree's removals (`ROAD-68-K17`).
    pub(crate) fn finalize_junction_endpoint_profiles(
        &mut self,
        affected_nodes: &HashSet<u32>,
        terrain: Option<&TerrainSystem>,
        changed_edges: &mut HashSet<usize>,
    ) {
        self.solve_indexed_endpoint_profiles(
            affected_nodes,
            JunctionProfileLimitMode::FinalProfile,
            terrain,
            changed_edges,
        );
    }

    fn solve_indexed_endpoint_profiles(
        &mut self,
        affected_nodes: &HashSet<u32>,
        limit_mode: JunctionProfileLimitMode,
        terrain: Option<&TerrainSystem>,
        changed_edges: &mut HashSet<usize>,
    ) {
        if affected_nodes.is_empty() {
            return;
        }
        ProfileSolveScratch::with(|scratch| {
            self.surface_edges_touching_nodes_into(
                affected_nodes.iter().copied(),
                |node| affected_nodes.contains(&node),
                &mut scratch.edges,
            );
            for &edge_idx in &scratch.edges {
                self.remove_from_spatial_index(edge_idx);
            }

            self.solve_junction_endpoint_profiles(limit_mode, terrain, scratch);

            for &edge_idx in &scratch.edges {
                self.add_to_spatial_index(edge_idx);
            }
            changed_edges.extend(scratch.changed.iter().copied());
        });
    }

    // Derives `scratch.edges`, leaving the changed ones in `scratch.changed`.
    fn solve_junction_endpoint_profiles(
        &mut self,
        limit_mode: JunctionProfileLimitMode,
        terrain: Option<&TerrainSystem>,
        scratch: &mut ProfileSolveScratch,
    ) {
        let ProfileSolveScratch {
            edges,
            pin_nodes,
            incidents,
            samples,
            pins,
            road_ends,
            changed,
            profile,
        } = scratch;
        // Both ends of every derived edge: its far end's pin must be reproduced exactly, and the
        // far node's plane is the same function of its arms' control polylines as before.
        pin_nodes.clear();
        pin_nodes.extend(edges.iter().flat_map(|&edge_idx| {
            let edge = &self.edges[edge_idx];
            [edge.start_node, edge.end_node].map(|node| self.get_valid_node(node))
        }));
        pin_nodes.sort_unstable();
        pin_nodes.dedup();
        self.build_junction_profile_incidents(pin_nodes, incidents);
        pins.clear();
        pins.resize(edges.len(), [None; 2]);
        // Road ends with attached footpaths: pinned once their roads' profiles are derived.
        road_ends.clear();
        for index in 0..incidents.nodes.len() {
            let (node_id, node) = incidents.node(index);
            if node.plane.len() == 1 && !node.attached.is_empty() {
                road_ends.push(index);
            }
            // A node without a plane keeps no pins; `check_node` rejects it before any edit
            // that produces it commits.
            let NodeSolve::Solved(solve) =
                self.node_profile_solve(node_id, node, limit_mode, samples)
            else {
                continue;
            };
            // A junction with an authority corridor eases every other arm onto its plane, attached
            // footpaths included: they never belong to the corridor.
            let preserve_authority = solve.authority.is_some() && node.plane.len() > 2;
            for incident in node.plane {
                let key = (incident.edge_idx, incident.at_start);
                let supports = preserve_authority
                    && !solve
                        .authority
                        .is_some_and(|authority| authority.contains(&key));
                Self::set_pin(edges, pins, incident, (solve.plane, supports));
            }
            // Attached footpaths join the sidewalk ring: the plane at sidewalk height.
            for incident in node.attached {
                Self::set_pin(
                    edges,
                    pins,
                    incident,
                    (solve.plane.raised(), preserve_authority),
                );
            }
        }

        // A road end's plane runs through its road's derived profile (`roadbed::terminal_plane`),
        // so the footpaths attached there are derived after every other edge.
        let at_road_end = |edge_idx: usize| {
            road_ends.iter().any(|&index| {
                incidents
                    .node(index)
                    .1
                    .attached
                    .iter()
                    .any(|incident| incident.edge_idx == edge_idx)
            })
        };
        changed.clear();
        for late in [false, true] {
            if late {
                for &index in road_ends.iter() {
                    let (node_id, node) = incidents.node(index);
                    // `None` only where the road end's layout fails, which `check_node` rejects.
                    let Some(plane) = terminal_plane(self, node_id) else {
                        continue;
                    };
                    // A road end has no authority corridor, so its footpaths take supports only
                    // where their grade leaves the plane, as every arm without one does.
                    for incident in node.attached {
                        Self::set_pin(edges, pins, incident, (plane.raised(), false));
                    }
                }
            }
            for (index, &edge_idx) in edges.iter().enumerate() {
                if at_road_end(edge_idx) == late {
                    self.derive_pinned_profile(edge_idx, pins[index], terrain, profile, changed);
                }
            }
        }
        for &edge_idx in changed.iter() {
            let (cost, length) = crate::simulation::pathing::cost::CostCalculator::calculate_costs(
                &self.edges[edge_idx],
            );
            self.edges[edge_idx].base_cost = cost;
            self.edges[edge_idx].physical_length = length;
        }
    }

    // Sets the pin of `incident`'s edge end where the solve derives that edge (`edges`, sorted);
    // the pins of the far nodes' other arms are never read.
    fn set_pin(
        edges: &[usize],
        pins: &mut [[Option<(JunctionEndpointProfilePlane, bool)>; 2]],
        incident: &JunctionProfileIncident,
        pin: (JunctionEndpointProfilePlane, bool),
    ) {
        if let Ok(index) = edges.binary_search(&incident.edge_idx) {
            pins[index][usize::from(!incident.at_start)] = Some(pin);
        }
    }

    // Derives the physical profile of `edge_idx` from its control polyline and its end pins (start,
    // end), recording it in `changed` when it differs from the edge's current one.
    fn derive_pinned_profile(
        &mut self,
        edge_idx: usize,
        pins: [Option<(JunctionEndpointProfilePlane, bool)>; 2],
        terrain: Option<&TerrainSystem>,
        profile: &mut ProfileBuffers,
        changed: &mut Vec<usize>,
    ) {
        let pin = |at_start: bool| {
            pins[usize::from(!at_start)].map(|(plane, force_supports)| EndpointPin {
                plane,
                hard_zone_m: self.junction_profile_hard_zone_m(edge_idx, at_start),
                force_supports,
                half_edge: self.edge_end_is_shared(edge_idx, !at_start),
            })
        };
        let pins = [pin(true), pin(false)];
        Self::pinned_physical_profile_into(&self.edges[edge_idx], pins, terrain, profile);
        let physical_geometry = &mut self.edges[edge_idx].physical_geometry;
        if profile.points != *physical_geometry {
            // Into the edge's own buffer, which allocates only where the profile outgrows it.
            physical_geometry.clone_from(&profile.points);
            changed.push(edge_idx);
        }
    }

    // Whether the node at one end of an edge has a second live road or footpath edge, pass-through
    // nodes included, so a pin at the other end may use only half the edge and never overwrites
    // that end's supports.
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

    // The direction of `edge` away from its start or end node, as the node layout takes it; an
    // edge end without one (every control point within 1 mm) is no arm of its node there.
    fn profile_incident(
        edge_idx: usize,
        edge: &Edge,
        at_start: bool,
    ) -> Option<JunctionProfileIncident> {
        let end = if at_start {
            EdgeEnd::Start
        } else {
            EdgeEnd::End
        };
        let is_profile_edge = !edge.deleted
            && matches!(edge.primary_type, TransitType::Road | TransitType::Foot)
            && edge.geometry.len() >= 2;
        is_profile_edge
            .then(|| incident_direction(edge, end))
            .flatten()
            .map(|direction| JunctionProfileIncident {
                edge_idx,
                at_start,
                direction,
            })
    }

    // Whether `node_id` has a road arm: its plane is then its roads', and its footpaths are
    // attached to them (D5). The incident rule of `build_junction_profile_incidents`.
    fn has_road_arm(&self, node_id: u32) -> bool {
        self.node_adjacency(node_id).iter().any(|&id| {
            let edge = &self.edges[id];
            edge.primary_type == TransitType::Road
                && [edge.start_node, edge.end_node]
                    .into_iter()
                    .zip([true, false])
                    .any(|(node, at_start)| {
                        self.get_valid_node(node) == node_id
                            && Self::profile_incident(id, edge, at_start).is_some()
                    })
        })
    }

    // The incidents of the bend and junction nodes among `nodes` (sorted and unique) into
    // `incidents`, which allocates only to grow. O(incidents * log(incidents)).
    fn build_junction_profile_incidents(&self, nodes: &[u32], incidents: &mut ProfileIncidents) {
        let ProfileIncidents {
            candidates,
            keyed,
            incidents: list,
            nodes: runs,
        } = incidents;
        let affected = |node: u32| nodes.binary_search(&node).is_ok();
        self.surface_edges_touching_nodes_into(nodes.iter().copied(), affected, candidates);
        keyed.clear();
        for &edge_idx in candidates.iter() {
            let Some(edge) = self.edges.get(edge_idx) else {
                continue;
            };
            if edge.start_node as usize >= self.nodes.len()
                || edge.end_node as usize >= self.nodes.len()
            {
                continue;
            }

            let start_node = self.get_valid_node(edge.start_node);
            let end_node = self.get_valid_node(edge.end_node);
            for (node_id, at_start) in [(start_node, true), (end_node, false)] {
                if !affected(node_id)
                    || self.nodes[node_id as usize].node_type != NodeType::Junction
                {
                    continue;
                }
                let Some(incident) = Self::profile_incident(edge_idx, edge, at_start) else {
                    continue;
                };
                let attached = edge.primary_type == TransitType::Foot && self.has_road_arm(node_id);
                // Incident order is the direction angle from world +x.
                let angle = incident.direction.y.atan2(incident.direction.x);
                keyed.push((node_id, attached, angle, incident));
            }
        }

        // By node, its plane incidents before its attached ones, each in incident order; edge
        // ids, which follow creation order, break only exactly equal directions, which layouts
        // reject (`DuplicateDirection`). Every key is distinct, so the order is total.
        keyed.sort_unstable_by(
            |(node_a, attached_a, angle_a, a), (node_b, attached_b, angle_b, b)| {
                node_a
                    .cmp(node_b)
                    .then(attached_a.cmp(attached_b))
                    .then(angle_a.total_cmp(angle_b))
                    .then(a.edge_idx.cmp(&b.edge_idx))
                    .then(a.at_start.cmp(&b.at_start))
            },
        );
        list.clear();
        list.extend(keyed.iter().map(|&(_, _, _, incident)| incident));
        runs.clear();
        for (index, &(node, attached, _, _)) in keyed.iter().enumerate() {
            match runs.last_mut() {
                Some(run) if run.0 == node => {
                    if !attached {
                        run.2 = index + 1;
                    }
                    run.3 = index + 1;
                }
                _ => {
                    let plane_end = if attached { index } else { index + 1 };
                    runs.push((node, index, plane_end, index + 1));
                }
            }
        }
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
        self.junction_profile_node_solve(node_id, JunctionProfileLimitMode::ConservativeSourceFit)
    }

    /// Whether the finalizer finds no plane at bend or junction `node_id`, which then keeps no
    /// pins: an arm too short to sample, or samples that fix no plane. `check_node` rejects such
    /// a node, so no edit that produces one commits.
    pub(crate) fn junction_profile_solve_fails(&self, node_id: u32) -> bool {
        matches!(
            self.single_node_solve(node_id, JunctionProfileLimitMode::FinalProfile),
            Some(NodeSolve::NoPlane)
        )
    }

    fn junction_profile_node_solve(
        &self,
        node_id: u32,
        limit_mode: JunctionProfileLimitMode,
    ) -> Option<JunctionEndpointProfilePlane> {
        match self.single_node_solve(node_id, limit_mode)? {
            NodeSolve::Solved(solve) => Some(solve.plane),
            NodeSolve::Unpinned | NodeSolve::NoPlane => None,
        }
    }

    // The plane solve of bend or junction `node_id` alone, in this thread's buffers; `None` for
    // another node, or one without incidents.
    fn single_node_solve(
        &self,
        node_id: u32,
        limit_mode: JunctionProfileLimitMode,
    ) -> Option<NodeSolve> {
        if self.nodes.get(node_id as usize)?.node_type != NodeType::Junction {
            return None;
        }
        ProfileSolveScratch::with(|scratch| {
            self.build_junction_profile_incidents(&[node_id], &mut scratch.incidents);
            (!scratch.incidents.nodes.is_empty()).then(|| {
                let (_, node) = scratch.incidents.node(0);
                self.node_profile_solve(node_id, node, limit_mode, &mut scratch.samples)
            })
        })
    }

    // The plane solve of one node, its plane incidents in incident order. Every incident takes
    // its sample, so an arm too short to sample fails the node rather than dropping out of its
    // plane. A bend fits the plane through its two samples; a junction, or a pass-through pair
    // with attached footpaths, takes its authority corridor and else the least-squares plane of
    // every sample. The samples go into `samples`.
    fn node_profile_solve(
        &self,
        node_id: u32,
        node: NodeProfileIncidents,
        limit_mode: JunctionProfileLimitMode,
        samples: &mut Vec<JunctionProfileIncidentSample>,
    ) -> NodeSolve {
        let incidents = node.plane;
        let straight = match incidents[..] {
            [a, b] => directions_pass_through(a.direction, b.direction),
            _ => false,
        };
        // A straight pair of one profile shares its span sections; of two profiles it has a
        // layout and takes its corridor plane, as a pass-through pair with attached footpaths
        // does (`edges_pass_through`).
        let pass_through = match incidents[..] {
            [a, b] => edges_pass_through(
                (&self.edges[a.edge_idx], a.direction),
                (&self.edges[b.edge_idx], b.direction),
            ),
            _ => false,
        };
        if incidents.len() < 2 || (pass_through && node.attached.is_empty()) {
            return NodeSolve::Unpinned;
        }
        let Some(origin) = self.nodes.get(node_id as usize).map(|node| node.pos) else {
            return NodeSolve::NoPlane;
        };
        if !self.junction_profile_incident_samples(origin, incidents, samples) {
            return NodeSolve::NoPlane;
        }
        let bend = incidents.len() == 2 && !straight;
        let authority = if bend {
            None
        } else {
            Self::junction_profile_authority_solve(origin, samples, limit_mode)
        };
        authority
            .or_else(|| {
                Self::junction_profile_plane(origin, samples, limit_mode).map(|plane| {
                    JunctionProfileSolve {
                        plane,
                        authority: None,
                    }
                })
            })
            .map_or(NodeSolve::NoPlane, NodeSolve::Solved)
    }

    fn junction_profile_authority_solve(
        origin: Vector3,
        samples: &[JunctionProfileIncidentSample],
        limit_mode: JunctionProfileLimitMode,
    ) -> Option<JunctionProfileSolve> {
        let best = Self::junction_profile_authority_corridor(samples, limit_mode)?;
        let (a, b) = (samples[best.a], samples[best.b]);
        let gradient = Self::junction_profile_corridor_gradient(a, b)?;
        let plane = JunctionEndpointProfilePlane::grade_limited(
            origin,
            gradient.x,
            gradient.y,
            &[a, b],
            limit_mode,
        )?;
        Some(JunctionProfileSolve {
            plane,
            authority: Some(
                [a, b].map(|sample| (sample.incident.edge_idx, sample.incident.at_start)),
            ),
        })
    }

    // Each incident's control-profile point 12 m along it, or at its mouth station where that
    // lies farther (the clip, current before the solve), at most half its edge: the plane then
    // meets the control profile no nearer than where the pin ends, so a wide mouth on a hill's
    // knee does not carry the near grade past it (`ROAD-68` open question 31). Into `samples`, in
    // incident order; false when an incident's edge is too short for a sample at least 1 m from
    // the node.
    fn junction_profile_incident_samples(
        &self,
        origin: Vector3,
        incidents: &[JunctionProfileIncident],
        samples: &mut Vec<JunctionProfileIncidentSample>,
    ) -> bool {
        samples.clear();
        for incident in incidents {
            let Some(sample) = self.junction_profile_incident_sample(origin, incident) else {
                return false;
            };
            samples.push(sample);
        }
        true
    }

    fn junction_profile_incident_sample(
        &self,
        origin: Vector3,
        incident: &JunctionProfileIncident,
    ) -> Option<JunctionProfileIncidentSample> {
        let edge = self.edges.get(incident.edge_idx)?;
        let total_length_m = Self::edge_profile_length_m(edge);
        let sample_distance_m = JUNCTION_PROFILE_SOLVE_SAMPLE_M
            .max(self.junction_profile_hard_zone_m(incident.edge_idx, incident.at_start))
            .min(total_length_m * 0.5);
        if sample_distance_m < JUNCTION_PROFILE_MIN_SAMPLE_M {
            return None;
        }
        let sample =
            Self::sample_edge_geometry_from_endpoint(edge, incident.at_start, sample_distance_m)?;
        let (dx, dz) = (sample.x - origin.x, sample.z - origin.z);
        (dx * dx + dz * dz > JUNCTION_PROFILE_MIN_SAMPLE_M * JUNCTION_PROFILE_MIN_SAMPLE_M)
            .then_some(JunctionProfileIncidentSample {
                incident: *incident,
                dx,
                dz,
                dy: sample.y - origin.y,
            })
    }

    // The best corridor (an opposite enough arm pair), in `f64` by one lexicographic rule
    // (`ROAD-68` "Heights"): a corridor whose plane every other arm's sample meets within the
    // support tolerance first, so a fall-line road crossed by a contour road keeps its grade;
    // then the straighter pair (lower dot of the arm directions); then the flatter; then incident
    // order (direction angle from world +x). Geometry alone ranks them: which road was built first
    // must not matter (`ROAD-68` invariant 6). The pair breaks every tie, so the rule is a total
    // order and the best is unique. O(samples^3).
    fn junction_profile_authority_corridor(
        samples: &[JunctionProfileIncidentSample],
        limit_mode: JunctionProfileLimitMode,
    ) -> Option<JunctionProfileCorridorCandidate> {
        let rank = |a: &JunctionProfileCorridorCandidate, b: &JunctionProfileCorridorCandidate| {
            b.fits
                .cmp(&a.fits)
                .then(a.dot.total_cmp(&b.dot))
                .then(a.grade.total_cmp(&b.grade))
                .then(a.a.cmp(&b.a))
                .then(a.b.cmp(&b.b))
        };
        let mut best = None;
        for a in 0..samples.len() {
            for b in a + 1..samples.len() {
                let dot = samples[a]
                    .incident
                    .direction
                    .dot(samples[b].incident.direction)
                    .clamp(-1.0, 1.0);
                if dot > JUNCTION_PROFILE_AUTHORITY_CORRIDOR_DOT {
                    continue;
                }
                let Some(gradient) =
                    Self::junction_profile_corridor_gradient(samples[a], samples[b])
                else {
                    continue;
                };
                let (grade_x, grade_z) = (f64::from(gradient.x), f64::from(gradient.y));
                let grade = grade_x.hypot(grade_z);
                // Finalization uses prepared road profiles. Discarding a supported
                // steep through-road here lets a later, flatter branch steal authority.
                if !grade.is_finite()
                    || (!matches!(limit_mode, JunctionProfileLimitMode::FinalProfile)
                        && grade > f64::from(JUNCTION_PROFILE_REJECT_MAX_GRADE))
                {
                    continue;
                }
                let fits = samples.iter().enumerate().all(|(index, sample)| {
                    let residual_m = f64::from(sample.dy)
                        - grade_x * f64::from(sample.dx)
                        - grade_z * f64::from(sample.dz);
                    index == a
                        || index == b
                        || residual_m.abs() <= f64::from(JUNCTION_PROFILE_SUPPORT_HEIGHT_EPS_M)
                });
                let candidate = JunctionProfileCorridorCandidate {
                    a,
                    b,
                    fits,
                    dot,
                    grade,
                };
                if best
                    .as_ref()
                    .is_none_or(|best| rank(&candidate, best).is_lt())
                {
                    best = Some(candidate);
                }
            }
        }
        best
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

    // The least-squares plane through the node and every sample.
    fn junction_profile_plane(
        origin: Vector3,
        samples: &[JunctionProfileIncidentSample],
        limit_mode: JunctionProfileLimitMode,
    ) -> Option<JunctionEndpointProfilePlane> {
        let (mut xx, mut xz, mut zz, mut xy, mut zy) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for sample in samples {
            let (dx, dz, dy) = (sample.dx, sample.dz, sample.dy);
            xx += dx * dx;
            xz += dx * dz;
            zz += dz * dz;
            xy += dx * dy;
            zy += dz * dy;
        }
        let det = xx * zz - xz * xz;
        if det.abs() <= JUNCTION_PROFILE_PLANE_DET_EPS {
            return None;
        }
        let grade_x = (xy * zz - zy * xz) / det;
        let grade_z = (xx * zy - xz * xy) / det;
        JunctionEndpointProfilePlane::grade_limited(origin, grade_x, grade_z, samples, limit_mode)
    }

    // `pinned_physical_profile_into` in fresh buffers, for the profile regressions.
    #[cfg(test)]
    pub(super) fn pinned_physical_profile(
        edge: &Edge,
        pins: [Option<EndpointPin>; 2],
        terrain: Option<&TerrainSystem>,
    ) -> Vec<Vector3> {
        let mut profile = ProfileBuffers::default();
        Self::pinned_physical_profile_into(edge, pins, terrain, &mut profile);
        profile.points
    }

    // The physical profile of `edge`, derived from its control polyline alone: a knot at each
    // pinned end's mouth, so heights lie on its plane exactly up to there, plus that end's
    // vertical-curve supports where it needs them, with heights eased back to the control grade
    // over the blend zone, and built within the grade and, on a standard road, the source band of
    // `terrain` (`build_within_grade`). Start end first; `O(points * inserted knots)`. Into
    // `profile.points`, in `profile`'s buffers, which allocate only to grow.
    fn pinned_physical_profile_into(
        edge: &Edge,
        pins: [Option<EndpointPin>; 2],
        terrain: Option<&TerrainSystem>,
        profile: &mut ProfileBuffers,
    ) {
        let ProfileBuffers {
            points,
            distances,
            grade,
        } = profile;
        points.clear();
        points.extend_from_slice(&edge.geometry);
        let total_length_m = Self::edge_profile_length_m(edge);
        if total_length_m <= JUNCTION_PROFILE_MIN_SAMPLE_M {
            return;
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
            Self::ensure_profile_support(points, distances, at_start, pin.hard_zone_m);
            if pin.force_supports
                || Self::endpoint_profile_support_delta_m(edge, at_start, pin.plane, solve_sample_m)
                    .is_some_and(|delta_m| delta_m > JUNCTION_PROFILE_SUPPORT_HEIGHT_EPS_M)
            {
                Self::materialize_endpoint_profile_supports(
                    points,
                    distances,
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
            Self::profile_endpoint_distances_into(points, at_start, distances);
            for (point, &distance_m) in points.iter_mut().zip(distances.iter()) {
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
        // Bridges and tunnels leave the ground by design; only a standard road keeps the band.
        let terrain = terrain.filter(|_| edge.class == EdgeClass::Standard);
        Self::build_within_grade(points, &blends, terrain, grade);
    }

    // Builds the blends of a derived profile within 33%, or else within the 1:3 maximum, and on a
    // standard road inside the source-delta band of `terrain` (`ROAD-68` open questions 37 and
    // 39): a blend that eases an arm onto a flatter plane can climb faster than either, and one
    // that follows a plane fitted to the arms' heights a mouth station out can leave the ground the
    // stroke solve cut or filled the arm to. Only the points each pinned end blends (past its
    // mouth knot, up to its blend end) move, to the heights nearest their blended ones
    // (`grade::construct_within_grade`); the plane stretches and the control polyline beyond the
    // blends are held, so a steep control (a terrain sync, a saved legacy profile) is not this
    // solve's to rebuild, and a held point off the ground widens its neighbours' band as a
    // stroke's pins do (`grade::source_bounds`). A profile within 33% and the band is left as it
    // is. Where the held points, the band and the 1:3 maximum leave no profile, the blends are
    // built inside the band at the least grade that allows them, which placement refuses as
    // steeper than the maximum (`road_too_steep`). O(points); that least grade O(points^2). The
    // construction runs in the solve's grade scratch, so it allocates nothing once that has
    // grown to the profile (`ROAD-68-K16`, `ROAD-68-K17`).
    fn build_within_grade(
        points: &mut [Vector3],
        blends: &[Option<(EndpointPin, f32)>; 2],
        terrain: Option<&TerrainSystem>,
        scratch: &mut GradeScratch,
    ) {
        use crate::simulation::network::grade::{
            ROAD_GRADE_BUILT, SourcePoint, construct_within_bounds, source_bounds, source_ground_m,
            steepest_grade, within_source_band,
        };
        if points.len() < 3 || blends.iter().all(Option::is_none) {
            return;
        }
        // Allocation-free first: a profile within 33% whose every point lies within 5 m of the
        // ground lies inside every band below, held points off it widening only.
        let steep = steepest_grade(points).is_some_and(|steepest| steepest.grade > ROAD_GRADE_BUILT);
        if !steep
            && terrain.is_none_or(|terrain| {
                points
                    .iter()
                    .all(|&point| within_source_band(terrain, point))
            })
        {
            return;
        }
        let GradeScratch {
            runs,
            sources,
            bounds,
            envelope,
            heights,
            backward,
        } = scratch;
        runs.clear();
        runs.extend(
            points
                .windows(2)
                .map(|pair| Self::edge_profile_point_distance_m(pair[0], pair[1])),
        );
        // The stretch of each end its blend moves, measured as the blend above measures it:
        // past the mouth knot, which `ensure_profile_support` reuses within its 1 cm, up to
        // the blend's end. Distances from the start accumulate forwards, from the end in
        // `heights`, as `profile_endpoint_distances_into` sums them.
        let blended_m = blends.map(|blend| {
            blend.map(|(pin, blend_end_m)| {
                (
                    pin.hard_zone_m + JUNCTION_PROFILE_SUPPORT_EPS_M,
                    blend_end_m,
                )
            })
        });
        Self::profile_endpoint_distances_into(points, false, heights);
        let mut from_start = 0.0;
        // Each point's bounds: a held point at its height, a blended one inside the band,
        // or anywhere on a road without one. On a road with a band they are its
        // `source_bounds`, which `sources` hands each point's ground and held height.
        bounds.clear();
        sources.clear();
        for (index, point) in points.iter().enumerate() {
            if index > 0 {
                from_start += runs[index - 1];
            }
            let from = [from_start, heights[index]];
            let blended = |end: usize| {
                blended_m[end]
                    .is_some_and(|(held_m, end_m)| from[end] > held_m && from[end] < end_m)
            };
            let held = index == 0 || index + 1 == points.len() || !(blended(0) || blended(1));
            match terrain {
                Some(terrain) => sources.push(SourcePoint {
                    ground_m: source_ground_m(terrain, point.x, point.z),
                    held_m: held.then_some(point.y),
                }),
                None => bounds.push(if held {
                    (point.y, point.y)
                } else {
                    (f32::NEG_INFINITY, f32::INFINITY)
                }),
            }
        }
        if terrain.is_some() {
            source_bounds(sources, runs, bounds);
        }
        if !steep
            && points
                .iter()
                .zip(&*bounds)
                .all(|(point, &(low, high))| (low..=high).contains(&point.y))
        {
            return;
        }
        heights.clear();
        heights.extend(points.iter().map(|point| point.y));
        construct_within_bounds(heights, bounds, runs, envelope, backward);
        for (point, &height) in points.iter_mut().zip(&*heights) {
            point.y = height;
        }
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
        distances: &mut Vec<f32>,
        at_start: bool,
        hard_zone_m: f32,
        blend_end_m: f32,
    ) {
        let solve_sample_m = JUNCTION_PROFILE_SOLVE_SAMPLE_M.min(blend_end_m);
        Self::ensure_profile_support(points, distances, at_start, solve_sample_m);

        Self::ensure_profile_support(points, distances, at_start, hard_zone_m);
        let mut distance_m = hard_zone_m + JUNCTION_PROFILE_SUPPORT_STEP_M;
        while distance_m < blend_end_m - JUNCTION_PROFILE_SUPPORT_EPS_M {
            Self::ensure_profile_support(points, distances, at_start, distance_m);
            distance_m += JUNCTION_PROFILE_SUPPORT_STEP_M;
        }
        Self::ensure_profile_support(points, distances, at_start, blend_end_m);
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
    // `JUNCTION_PROFILE_SUPPORT_EPS_M` of it; `distances` is its buffer of the points' distances
    // from that end.
    fn ensure_profile_support(
        points: &mut Vec<Vector3>,
        distances: &mut Vec<f32>,
        at_start: bool,
        distance_m: f32,
    ) {
        if !distance_m.is_finite() || distance_m <= JUNCTION_PROFILE_SUPPORT_EPS_M {
            return;
        }
        if points.len() < 2 {
            return;
        }
        Self::profile_endpoint_distances_into(points, at_start, distances);
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

    // The control polyline's point `distance_m` from one end. The distances accumulate segment
    // by segment as `profile_endpoint_distances_into` sums them, without a buffer; O(points).
    pub(super) fn sample_edge_geometry_from_endpoint(
        edge: &Edge,
        at_start: bool,
        distance_m: f32,
    ) -> Option<Vector3> {
        let points = &edge.geometry;
        if points.is_empty() {
            return None;
        }
        if points.len() == 1 {
            return Some(points[0]);
        }

        let mut start_d = 0.0;
        if at_start {
            for index in 0..points.len() - 1 {
                let end_d =
                    start_d + Self::edge_profile_point_distance_m(points[index], points[index + 1]);
                if distance_m > end_d && index + 2 < points.len() {
                    start_d = end_d;
                    continue;
                }
                let segment_m = (end_d - start_d).max(f32::EPSILON);
                let t = ((distance_m - start_d) / segment_m).clamp(0.0, 1.0);
                return Some(points[index].lerp(points[index + 1], t));
            }
        } else {
            for index in (1..points.len()).rev() {
                let end_d =
                    start_d + Self::edge_profile_point_distance_m(points[index], points[index - 1]);
                if distance_m > end_d && index > 1 {
                    start_d = end_d;
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

    // Each point's horizontal distance along `points` from one end, into `distances`, which
    // allocates only to grow.
    fn profile_endpoint_distances_into(
        points: &[Vector3],
        at_start: bool,
        distances: &mut Vec<f32>,
    ) {
        distances.clear();
        distances.resize(points.len(), 0.0);
        if points.len() < 2 {
            return;
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
    }
}
