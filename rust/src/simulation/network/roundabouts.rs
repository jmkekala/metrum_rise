// SPDX-License-Identifier: GPL-2.0-only

//! Roundabout rings (`ROAD-62`): the ring's geometry and its plan on the graph, on empty ground or
//! in place of a junction. A ring is a circle of one-way edges between ring nodes: each node
//! is an ordinary junction of ring-in, ring-out and at most one arm, a bare ring three pass-through
//! nodes (D8). Each ring edge holds the circle ([`RoundaboutRing`]), so a split re-samples its
//! halves. Every ring edge leaves and enters its nodes along a 1 m straight tangent lead: the two
//! ring arms of a node are then exactly straight there, which the junction layout needs (without
//! leads the island sector is two curves leaving one point, `CornerDoesNotFit`). The plan is a
//! pure, local product of the graph ([`RoundaboutLayout::prepare`]) applied to the graph alone
//! ([`RoundaboutLayout::apply`]); whether a ring fits (arms too close, a short approach) is the
//! junction layout's own answer on the applied graph, not a rule here. Lanes, terrain, dependents
//! and undo are the transaction's.

use super::graph::{Edge, RegionGraph, RoundaboutRing, SidewalkSides};
use super::lanes::pedestrian_junctions::segment_distance_xz;
use super::lanes::vehicle_junctions::node_whitelists_vehicle_turns;
use super::roadbed::visual_roadbed_half_width_m;
use super::types::{EdgeClass, NodeType, TransitFlags, TransitType};
use crate::config;
use crate::simulation::pathing::cost::CostCalculator;
use godot::prelude::{Vector2, Vector3};
use std::f32::consts::TAU;

mod joins;
pub(crate) use joins::{join_near_bare_node, ring_walk};

/// A ring's speed limit, 20 km/h.
pub(crate) const RING_SPEED_MS: f32 = 20.0 / 3.6;
/// The most lanes a ring has.
pub(crate) const RING_MAX_LANES: u8 = 2;
/// The largest diameter that replaces a junction (D14): every car and walker inside the circle
/// stays within the 30 m lane reattachment reach of a ring or arm lane.
pub(crate) const CONVERSION_MAX_DIAMETER_M: f32 = 60.0;
// The straight lead at each end of a ring edge (D7).
const LEAD_M: f32 = 1.0;
// The arc between two supports of a ring edge, at most.
const ARC_STEP_M: f32 = 1.0;
// The shortest arc between two ring nodes that holds both leads and a support. The layout refuses
// arms far earlier (15 m on one lane): this only keeps the plan from building a degenerate edge.
const MIN_ARC_M: f32 = 3.0 * LEAD_M;
// The bare ring's nodes (D8), the fewest without an edge of a half turn or more.
const BARE_RING_ANGLES_DEG: [f32; 3] = [90.0, 210.0, 330.0];
// The ring's largest radius, the reach of the island query.
const RING_MAX_RADIUS_M: f32 = 40.0;
const ISLAND_REFUSAL: &str = "roundabout_island";
// How far beyond the two roadbeds the clearance query looks.
const CLEARANCE_QUERY_PAD_M: f32 = 24.0;
// The sides of a refused ring's outline.
const OUTLINE_SIDES: usize = 64;
// Knots nearer than this to a cut merge into it.
const CUT_KNOT_MERGE_M: f32 = 0.01;

/// The diameters a ring of `lanes` lanes takes, `(min, max)`, between the outside asphalt kerbs
/// (D21): 30-60 m on one lane, 45-80 m on two. The floors keep up to five arms buildable at every
/// size; six need more, and the junction layout refuses them.
pub(crate) fn diameter_range_m(lanes: u8) -> (f32, f32) {
    if lanes >= RING_MAX_LANES {
        (45.0, 80.0)
    } else {
        (30.0, 60.0)
    }
}

/// The diameter a ring of `lanes` lanes starts at: 36 m on one lane, 50 m on two.
pub(crate) fn default_diameter_m(lanes: u8) -> f32 {
    if lanes >= RING_MAX_LANES { 50.0 } else { 36.0 }
}

/// The ring's sidewalk side in its edges' forward frame: the outer side (the ring has no walk on
/// the island). Forward is the sense of travel, so the outer side is the drivers' outer side:
/// right when driving on the right, as the island is on their left.
pub(crate) fn ring_sidewalk_side() -> SidewalkSides {
    if config::DRIVE_ON_LEFT {
        SidewalkSides::LEFT
    } else {
        SidewalkSides::RIGHT
    }
}

// The sense of travel around the ring as a sign of the XZ angle: decreasing when driving on the
// right, which keeps the island on the drivers' left.
fn travel_sense() -> f32 {
    if config::DRIVE_ON_LEFT { 1.0 } else { -1.0 }
}

impl RoundaboutRing {
    fn angle_of(&self, point: Vector3) -> f32 {
        (point.z - self.centre.y).atan2(point.x - self.centre.x)
    }

    // The point at `angle`, `radius_m` from the centre, at height `y`.
    fn at(&self, angle: f32, radius_m: f32, y: f32) -> Vector3 {
        let (sin, cos) = angle.sin_cos();
        Vector3::new(
            self.centre.x + cos * radius_m,
            y,
            self.centre.y + sin * radius_m,
        )
    }

    // The unit tangent at `angle` in the sense of travel.
    fn tangent(angle: f32) -> Vector3 {
        let (sin, cos) = angle.sin_cos();
        Vector3::new(-sin, 0.0, cos) * travel_sense()
    }

    /// The ring polyline from node position `from` to `to` in the sense of travel: both ends
    /// exactly, a straight tangent lead at each (shortened to a third of a very short arc), and
    /// between them supports at the crossings of the circle's tangents at most 1 m of arc apart,
    /// so the polyline's segments touch the circle and no chord cuts inside it. Heights run
    /// linearly from `from` to `to`: level on a whole ring. The one construction of a new ring
    /// and a split. `O(points)`.
    pub(crate) fn arc(&self, from: Vector3, to: Vector3) -> Vec<Vector3> {
        let (start, end) = (self.angle_of(from), self.angle_of(to));
        let radius_m = self.radius_m;
        let sense = travel_sense();
        let sweep = (sense * (end - start)).rem_euclid(TAU);
        let lead_m = LEAD_M.min(sweep * radius_m / 3.0);
        let lead_rad = lead_m / radius_m;
        let inner = sweep - 2.0 * lead_rad;
        let count = ((inner * radius_m / ARC_STEP_M).ceil() as usize).max(2);
        let delta = inner / count as f32;
        let support_radius_m = radius_m / (delta * 0.5).cos();
        let mut points = Vec::with_capacity(count + 4);
        points.push(from);
        points.push(from + Self::tangent(start) * lead_m);
        points.extend((0..count).map(|step| {
            let along = lead_rad + delta * (step as f32 + 0.5);
            let y = from.y + (to.y - from.y) * (along / sweep);
            self.at(start + sense * along, support_radius_m, y)
        }));
        points.push(to - Self::tangent(end) * lead_m);
        points.push(to);
        points
    }
}

/// The circle of a ring of `lanes` lanes and `diameter_m` at `centre` as one closed polyline, the
/// outline a refused ring shows. `O(1)`: 64 points.
pub(crate) fn circle_outline(centre: Vector3, diameter_m: f32, lanes: u8) -> Vec<Vector3> {
    let radius_m = ring_radius_m(diameter_m, lanes);
    let ring = RoundaboutRing {
        centre: Vector2::new(centre.x, centre.z),
        radius_m,
    };
    let step = TAU / OUTLINE_SIDES as f32;
    (0..=OUTLINE_SIDES)
        .map(|side| ring.at(side as f32 * step, radius_m, centre.y))
        .collect()
}

// The centreline radius of a ring between its outside kerbs (D21).
fn ring_radius_m(diameter_m: f32, lanes: u8) -> f32 {
    0.5 * (diameter_m - f32::from(lanes) * config::LANE_WIDTH)
}

/// Where a roundabout goes.
#[derive(Clone, Copy, Debug)]
pub(crate) enum RoundaboutSite {
    /// A bare ring of `lanes` lanes at `centre`, on empty ground.
    Ground {
        /// The ring's centre; its height is the ring's.
        centre: Vector3,
        /// The ring's lanes, 1 or 2.
        lanes: u8,
    },
    /// A ring in place of this junction node, at its position and height. Its lanes follow its
    /// arms: two when an arm enters with two or more, else one.
    Junction(u32),
}

impl RoundaboutSite {
    /// The ring's lanes here. `O(degree)` at a junction.
    pub(crate) fn lanes(self, graph: &RegionGraph) -> u8 {
        match self {
            Self::Ground { lanes, .. } => lanes,
            Self::Junction(node) => junction_lanes(graph, node),
        }
    }

    /// The diameters a ring takes here, `(min, max)`: [`diameter_range_m`] at its lanes, a
    /// replacement capped at [`CONVERSION_MAX_DIAMETER_M`]. `O(degree)` at a junction.
    pub(crate) fn diameter_range_m(self, graph: &RegionGraph) -> (f32, f32) {
        let (min_m, max_m) = diameter_range_m(self.lanes(graph));
        match self {
            Self::Ground { .. } => (min_m, max_m),
            Self::Junction(_) => (min_m, max_m.min(CONVERSION_MAX_DIAMETER_M)),
        }
    }
}

/// One roundabout edit.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RoundaboutRequest {
    /// Where it goes.
    pub(crate) site: RoundaboutSite,
    /// The diameter between the outside asphalt kerbs in metres, the sidewalk additional (D21).
    pub(crate) diameter_m: f32,
}

/// A roundabout's plan: the ring and the arms cut to it, the incident records only.
pub(crate) struct RoundaboutLayout {
    // The ring nodes in ring order; ring edge `i` leaves node `i` for the next one.
    points: Vec<Vector3>,
    ring: Vec<Edge>,
    // Per ring node, the arm cut to it and the arm's id. Empty on the ground.
    arms: Vec<(usize, Edge)>,
    old_node: Option<u32>,
    lanes: u8,
    circle: RoundaboutRing,
}

/// How a conversion shortened one arm, for its dependents (`ROAD-62`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct ArmTrim {
    /// The arm, keeping its id.
    pub(crate) edge: usize,
    /// Whether the arm lost its start, so that its stations shift.
    pub(crate) at_start: bool,
    /// The arm's length before the cut.
    pub(crate) old_length_m: f32,
    /// The arm's length after it.
    pub(crate) new_length_m: f32,
}

/// The ids a [`RoundaboutLayout::apply`] installed.
pub(crate) struct RoundaboutApplied {
    /// The ring nodes in ring order.
    pub(crate) nodes: Vec<u32>,
    /// The ring edges; edge `i` leaves node `i`.
    pub(crate) ring: Vec<usize>,
    /// The arms, each keeping its id, in the ring order of their nodes.
    pub(crate) arms: Vec<usize>,
}

impl RoundaboutLayout {
    /// The plan of `request` on `graph`, or a stable refusal key, nothing touched:
    /// `invalid_roundabout_dimensions`, `invalid_roundabout_junction`,
    /// `roundabout_requires_three_to_six_arms`, `roundabout_has_lane_restrictions`,
    /// `roundabout_requires_ordinary_surface_arms`, `roundabout_arm_has_pocket`,
    /// `roundabout_arm_too_short`, `roundabout_replacement_too_large`,
    /// `roundabout_arms_too_close` (nodes too close to build an edge between),
    /// `roundabout_overlaps_road`. `O(arm points + ring points + clearance)`, with
    /// [`Self::validate_clearance`]'s bound; 3 to 6 arms.
    pub(crate) fn prepare(
        graph: &RegionGraph,
        request: RoundaboutRequest,
    ) -> Result<Self, &'static str> {
        let diameter_m = request.diameter_m;
        if !diameter_m.is_finite() {
            return Err("invalid_roundabout_dimensions");
        }
        let (centre, lanes, node) = match request.site {
            RoundaboutSite::Ground { centre, lanes } => {
                if !centre.is_finite() || !(1..=RING_MAX_LANES).contains(&lanes) {
                    return Err("invalid_roundabout_dimensions");
                }
                (centre, lanes, None)
            }
            RoundaboutSite::Junction(node) => {
                check_junction(graph, node)?;
                if diameter_m > CONVERSION_MAX_DIAMETER_M {
                    return Err("roundabout_replacement_too_large");
                }
                let centre = graph.node(node).pos;
                (centre, junction_lanes(graph, node), Some(node))
            }
        };
        let (min_m, max_m) = diameter_range_m(lanes);
        if !(min_m..=max_m).contains(&diameter_m) {
            return Err("invalid_roundabout_dimensions");
        }
        let ring = RoundaboutRing {
            centre: Vector2::new(centre.x, centre.z),
            radius_m: ring_radius_m(diameter_m, lanes),
        };
        let mut entries: Vec<RingEntry> = match node {
            Some(node) => cut_arms(graph, node, ring, centre.y)?,
            None => BARE_RING_ANGLES_DEG
                .into_iter()
                .map(|degrees| {
                    let angle = degrees.to_radians();
                    let point = ring.at(angle, ring.radius_m, centre.y);
                    RingEntry {
                        angle,
                        point,
                        arm: None,
                    }
                })
                .collect(),
        };
        // Arms in angle order, the lower id first at one angle.
        entries.sort_by(|a, b| {
            a.angle
                .total_cmp(&b.angle)
                .then_with(|| a.arm_id().cmp(&b.arm_id()))
        });
        let count = entries.len();
        for i in 0..count {
            let gap = (entries[ring_next(i, count)].angle - entries[i].angle).abs();
            if gap.min(TAU - gap) * ring.radius_m < MIN_ARC_M {
                return Err("roundabout_arms_too_close");
            }
        }
        let ring_edges = (0..count)
            .map(|i| {
                ring_edge(
                    ring,
                    entries[i].point,
                    entries[ring_next(i, count)].point,
                    lanes,
                )
            })
            .collect();
        let points = entries.iter().map(|entry| entry.point).collect();
        let arms = entries.into_iter().filter_map(|entry| entry.arm).collect();
        let layout = Self {
            points,
            ring: ring_edges,
            arms,
            old_node: node,
            lanes,
            circle: ring,
        };
        layout.validate_clearance(graph)?;
        Ok(layout)
    }

    // The ring's corridor keeps clear of every road but the arms it replaces,
    // `roundabout_overlaps_road`: bridges and tunnels included, as the check has no heights. The
    // R-tree bounds the search: `O(ring log E + ring points x nearby points)`.
    fn validate_clearance(&self, graph: &RegionGraph) -> Result<(), &'static str> {
        for edge in &self.ring {
            let half_m = visual_roadbed_half_width_m(edge);
            let (mut min, mut max) = (edge.geometry[0], edge.geometry[0]);
            for point in &edge.geometry {
                min = min.coord_min(*point);
                max = max.coord_max(*point);
            }
            let halo = Vector3::new(1.0, 0.0, 1.0) * (half_m + CLEARANCE_QUERY_PAD_M);
            for id in graph.get_edges_near_aabb(min - halo, max + halo) {
                let other = graph.edge(id);
                if other.deleted || self.arms.iter().any(|(arm, _)| *arm == id) {
                    continue;
                }
                let gap_m = half_m + visual_roadbed_half_width_m(other);
                let touches = edge.geometry.windows(2).any(|a| {
                    other
                        .physical_geometry
                        .windows(2)
                        .any(|b| segment_distance_xz(a[0], a[1], b[0], b[1]) < gap_m)
                });
                if touches {
                    return Err("roundabout_overlaps_road");
                }
            }
        }
        Ok(())
    }

    /// The ring's edges, for the corridor guards of the transaction.
    pub(crate) fn ring_edges(&self) -> &[Edge] {
        &self.ring
    }

    /// The ring's lanes.
    pub(crate) fn lanes(&self) -> u8 {
        self.lanes
    }

    /// The ring's circle.
    pub(crate) fn circle(&self) -> RoundaboutRing {
        self.circle
    }

    /// The junction the ring replaces, if any.
    pub(crate) fn old_node(&self) -> Option<u32> {
        self.old_node
    }

    /// The ring's centreline as one closed polyline. `O(ring points)`.
    pub(crate) fn outline(&self) -> Vec<Vector3> {
        let mut points: Vec<Vector3> = self
            .ring
            .iter()
            .flat_map(|edge| edge.geometry[..edge.geometry.len() - 1].iter().copied())
            .collect();
        points.extend(points.first().copied());
        points
    }

    /// Each arm's id with the point it is cut at, in the ring order of their nodes.
    pub(crate) fn arm_cuts(&self) -> impl Iterator<Item = (usize, Vector3)> + '_ {
        self.arms.iter().map(|(id, arm)| {
            let at_start = Some(arm.start_node) == self.old_node;
            let points = &arm.geometry;
            (*id, points[if at_start { 0 } else { points.len() - 1 }])
        })
    }

    /// What the cuts take off each arm of `graph`, read before [`Self::apply`].
    pub(crate) fn arm_trims(&self, graph: &RegionGraph) -> Vec<ArmTrim> {
        let trim = |(id, arm): &(usize, Edge)| ArmTrim {
            edge: *id,
            at_start: Some(arm.start_node) == self.old_node,
            old_length_m: graph.edge(*id).physical_length,
            new_length_m: arm.physical_length,
        };
        self.arms.iter().map(trim).collect()
    }

    /// Installs the plan on `graph`, graph records only: the ring nodes and edges, each arm cut
    /// and moved to its ring node under its own id, the old node retired with its crossing
    /// overrides. The caller lays out the ring nodes ([`RoundaboutApplied::nodes`]), solves the
    /// arms' profiles and owns lanes, dependents, terrain and undo. `O(ring points + arm points)`.
    pub(crate) fn apply(&self, graph: &mut RegionGraph) -> RoundaboutApplied {
        let nodes: Vec<u32> = self
            .points
            .iter()
            .map(|&point| graph.add_node(point, NodeType::Junction))
            .collect();
        let mut arms = Vec::with_capacity(self.arms.len());
        for ((id, arm), &node) in self.arms.iter().zip(&nodes) {
            let (id, mut arm) = (*id, arm.clone());
            let old = self.old_node.expect("arms replace a junction");
            graph.remove_from_spatial_index(id);
            if arm.start_node == old {
                arm.start_node = node;
            } else {
                arm.end_node = node;
            }
            graph.edges[id] = arm;
            graph.add_to_spatial_index(id);
            graph.adjacency[node as usize].push(id);
            arms.push(id);
        }
        if let Some(old) = self.old_node {
            graph.adjacency[old as usize].clear();
            graph.nodes[old as usize].crosswalk_overrides.clear();
            graph.remove_node_from_spatial_index(old, graph.node(old).pos);
        }
        let mut ring = Vec::with_capacity(self.ring.len());
        for (i, edge) in self.ring.iter().enumerate() {
            let mut edge = edge.clone();
            edge.start_node = nodes[i];
            edge.end_node = nodes[ring_next(i, nodes.len())];
            ring.push(graph.add_edge(edge));
        }
        RoundaboutApplied { nodes, ring, arms }
    }
}

/// How a stroke through `points` meets the rings near it (D9): it may not end on an island
/// (`roundabout_island`) or, `grounded`, run through one (`roundabout_crossed`, or
/// `roundabout_island` for a chord between two points of the ring). A bridge or tunnel passes over
/// or under. An end within the join tolerance inside the circle is a join: the stroke's end moves
/// onto the ring. The island's refusal first, so the visiting order does not matter.
/// `O(log E + edges within the largest ring's radius of the stroke + ring edges x points)`.
pub(crate) fn stroke_ring_refusal(
    graph: &RegionGraph,
    points: &[Vector3],
    grounded: bool,
) -> Option<&'static str> {
    let (mut min, mut max) = (points[0], points[0]);
    for point in points {
        min = min.coord_min(*point);
        max = max.coord_max(*point);
    }
    let reach = Vector3::new(1.0, 0.0, 1.0) * RING_MAX_RADIUS_M;
    let mut refusal = None;
    graph.visit_edges_near_aabb(min - reach, max + reach, |id| {
        let edge = graph.edge(id);
        let Some(ring) = edge.roundabout.filter(|_| !edge.deleted) else {
            return;
        };
        match stroke_verdict(ring, points, grounded) {
            Some(ISLAND_REFUSAL) => refusal = Some(ISLAND_REFUSAL),
            found => refusal = refusal.or(found),
        }
    });
    refusal
}

// [`stroke_ring_refusal`] on one ring. `O(points)`.
fn stroke_verdict(
    ring: RoundaboutRing,
    points: &[Vector3],
    grounded: bool,
) -> Option<&'static str> {
    let centre = Vector3::new(ring.centre.x, 0.0, ring.centre.y);
    let from_centre = |point: Vector3| (point.x - centre.x).hypot(point.z - centre.z);
    let ends = [points[0], points[points.len() - 1]].map(from_centre);
    let island_m = ring.radius_m - config::INTERSECTION_TOLERANCE;
    if ends.iter().any(|&end| end < island_m) {
        return Some(ISLAND_REFUSAL);
    }
    let through = grounded
        && points
            .windows(2)
            .any(|pair| segment_distance_xz(centre, centre, pair[0], pair[1]) < island_m);
    if !through {
        return None;
    }
    let outside_m = ring.radius_m + config::INTERSECTION_TOLERANCE;
    Some(if ends.iter().any(|&end| end > outside_m) {
        "roundabout_crossed"
    } else {
        ISLAND_REFUSAL
    })
}

/// Whether a live ring edge lies within the join tolerance of `points`' bounds, so the stroke may
/// join a ring. One R-tree visit, `O(log E + edges near)`.
pub(crate) fn ring_near_stroke(graph: &RegionGraph, points: &[Vector3]) -> bool {
    let (mut min, mut max) = (points[0], points[0]);
    for point in points {
        min = min.coord_min(*point);
        max = max.coord_max(*point);
    }
    let reach = Vector3::new(1.0, 0.0, 1.0) * config::INTERSECTION_TOLERANCE;
    let mut near = false;
    graph.visit_edges_near_aabb(min - reach, max + reach, |id| {
        let edge = graph.edge(id);
        near |= edge.roundabout.is_some() && !edge.deleted;
    });
    near
}

/// The ring through `node`, if a live ring edge touches it. `O(degree)`.
pub(crate) fn ring_at(graph: &RegionGraph, node: u32) -> Option<RoundaboutRing> {
    graph.node_adjacency(node).iter().find_map(|&id| {
        let edge = graph.edge(id);
        edge.roundabout.filter(|_| !edge.deleted)
    })
}

/// Whether road `edge` brings lanes to `node` from outside a ring through the node: a ring entry,
/// where the road's traffic gives way and the B5 + D2 pole and the triangle row stand. A ring edge
/// is not one, nor is an arm that only leaves the ring. `O(degree)`.
pub(crate) fn enters_ring(graph: &RegionGraph, edge: usize, node: u32) -> bool {
    let road = graph.edge(edge);
    let lanes = if road.end_node == node {
        road.fwd_lanes
    } else {
        road.bkw_lanes
    };
    lanes > 0 && road.roundabout.is_none() && ring_at(graph, graph.get_valid_node(node)).is_some()
}

/// Whether a ring edge joins nodes `a` and `b`: a road between them would double it, and two roads
/// between the same two nodes are refused anywhere. `O(degree)`.
pub(crate) fn ring_neighbours(graph: &RegionGraph, a: u32, b: u32) -> bool {
    let a = graph.get_valid_node(a);
    let b = graph.get_valid_node(b);
    graph.node_adjacency(a).iter().any(|&id| {
        let edge = graph.edge(id);
        !edge.deleted
            && edge.roundabout.is_some()
            && [edge.start_node, edge.end_node].map(|end| graph.get_valid_node(end)).contains(&b)
    })
}

/// Whether ring edge `id` of a loaded graph is one a roundabout edit could have built: one-way
/// with 1 or 2 lanes in the ring's sense, the ring's speed and sidewalk side, no zoning, median or
/// pocket, on the surface, a real circle, and with its ring closed at both ends, the same circle
/// arriving at its start and leaving its end. The error is the stable reason. `O(degree)`.
pub(crate) fn validate_ring_edge(graph: &RegionGraph, id: usize) -> Result<(), &'static str> {
    let edge = graph.edge(id);
    let Some(ring) = edge.roundabout else {
        return Ok(());
    };
    if !(1..=RING_MAX_LANES).contains(&edge.fwd_lanes)
        || edge.bkw_lanes != 0
        || edge.primary_type != TransitType::Road
        || edge.class != EdgeClass::Standard
        || edge.median.is_some()
        || edge.pocket.is_some()
        || !edge.no_building_spawn
        || edge.speed_limit != RING_SPEED_MS
        || edge.sidewalks != ring_sidewalk_side()
    {
        return Err("ring_edge_section");
    }
    if !ring.centre.is_finite() || !ring.radius_m.is_finite() || ring.radius_m <= 0.0 {
        return Err("ring_edge_circle");
    }
    let ring_edges = |node: u32, leaving: bool| {
        let arms = graph
            .node_adjacency(node)
            .iter()
            .map(|&arm| graph.edge(arm));
        arms.filter(|arm| arm.roundabout == Some(ring))
            .filter(|arm| {
                (if leaving {
                    arm.start_node
                } else {
                    arm.end_node
                }) == node
            })
            .count()
    };
    if ring_edges(edge.start_node, false) != 1 || ring_edges(edge.end_node, true) != 1 {
        return Err("ring_edge_not_closed");
    }
    Ok(())
}

// The ring node after `i` of `count`, in the sense of travel, ring nodes being in ascending angle.
fn ring_next(i: usize, count: usize) -> usize {
    if config::DRIVE_ON_LEFT {
        (i + 1) % count
    } else {
        (i + count - 1) % count
    }
}

// A ring node of the plan: its angle on the circle, its point and the arm cut to it.
struct RingEntry {
    angle: f32,
    point: Vector3,
    arm: Option<(usize, Edge)>,
}

impl RingEntry {
    fn arm_id(&self) -> usize {
        self.arm.as_ref().map_or(usize::MAX, |(id, _)| *id)
    }
}

// Whether `node` may become a roundabout, its arms aside. `O(degree + the node's lane connections)`.
fn check_junction(graph: &RegionGraph, node: u32) -> Result<(), &'static str> {
    if node as usize >= graph.node_count() || graph.get_valid_node(node) != node {
        return Err("invalid_roundabout_junction");
    }
    if !(3..=6).contains(&graph.live_node_connection_count(node)) {
        return Err("roundabout_requires_three_to_six_arms");
    }
    if node_whitelists_vehicle_turns(graph, node as usize) {
        return Err("roundabout_has_lane_restrictions");
    }
    Ok(())
}

// The ring's lanes at `node`: two when an arm enters with two or more, else one. `O(degree)`.
fn junction_lanes(graph: &RegionGraph, node: u32) -> u8 {
    let entering = |&id: &usize| {
        let edge = graph.edge(id);
        match edge.deleted {
            true => 0,
            false if edge.end_node == node => edge.fwd_lanes,
            false => edge.bkw_lanes,
        }
    };
    let widest = graph.node_adjacency(node).iter().map(entering).max();
    widest.unwrap_or(1).clamp(1, RING_MAX_LANES)
}

// The live arms of `node` cut to `ring`, each with its angle and ring node at height `y`.
// `O(arm points)`.
fn cut_arms(
    graph: &RegionGraph,
    node: u32,
    ring: RoundaboutRing,
    y: f32,
) -> Result<Vec<RingEntry>, &'static str> {
    let mut ids: Vec<usize> = graph.node_adjacency(node).to_vec();
    ids.sort_unstable();
    ids.dedup();
    let mut entries = Vec::with_capacity(ids.len());
    for id in ids {
        let edge = graph.edge(id);
        if edge.deleted {
            continue;
        }
        if edge.roundabout.is_some()
            || edge.class != EdgeClass::Standard
            || edge.primary_type != TransitType::Road
            || edge.allowed_types & TransitFlags::CAR == 0
            || edge.start_node == edge.end_node
        {
            return Err("roundabout_requires_ordinary_surface_arms");
        }
        if edge.pocket.is_some() {
            return Err("roundabout_arm_has_pocket");
        }
        let at_start = edge.start_node == node;
        let far = if at_start {
            edge.end_node
        } else {
            edge.start_node
        };
        let (geometry, point) =
            cut_polyline(&edge.geometry, at_start, ring, y).ok_or_else(|| {
                if graph.is_lane_transition(far) {
                    "roundabout_arm_has_pocket"
                } else {
                    "roundabout_arm_too_short"
                }
            })?;
        if comes_back(&geometry, at_start, ring) {
            return Err("roundabout_overlaps_road");
        }
        let physical_geometry = edge_physical_cut(edge, at_start, ring, point);
        let mut arm = edge.clone();
        arm.geometry = geometry;
        arm.physical_geometry = physical_geometry.ok_or("roundabout_arm_too_short")?;
        // Its far end keeps its clip: that node is not touched.
        if at_start {
            arm.start_clip = 0.0;
        } else {
            arm.end_clip = 0.0;
        }
        price_edge(&mut arm);
        let angle = (point.z - ring.centre.y).atan2(point.x - ring.centre.x);
        entries.push(RingEntry {
            angle: angle.rem_euclid(TAU),
            point,
            arm: Some((id, arm)),
        });
    }
    if !(3..=6).contains(&entries.len()) {
        return Err("roundabout_requires_three_to_six_arms");
    }
    Ok(entries)
}

// `points`, which run from the node out when `at_start` and in otherwise, cut at the circle, the
// cut at height `y`, in the edge's own direction, with the cut point.
fn cut_polyline(
    points: &[Vector3],
    at_start: bool,
    ring: RoundaboutRing,
    y: f32,
) -> Option<(Vec<Vector3>, Vector3)> {
    let outward: Vec<Vector3> = if at_start {
        points.to_vec()
    } else {
        points.iter().rev().copied().collect()
    };
    let (segment, mut point) = circle_cut(&outward, ring)?;
    point.y = y;
    Some((kept_from(&outward, segment, point, at_start)?, point))
}

// The physical profile cut at the control polyline's `point`, in the edge's own direction.
fn edge_physical_cut(
    edge: &Edge,
    at_start: bool,
    ring: RoundaboutRing,
    point: Vector3,
) -> Option<Vec<Vector3>> {
    let outward: Vec<Vector3> = if at_start {
        edge.physical_geometry.clone()
    } else {
        edge.physical_geometry.iter().rev().copied().collect()
    };
    let (segment, _) = circle_cut(&outward, ring)?;
    kept_from(&outward, segment, point, at_start)
}

// `point` and what lies beyond `segment` of `outward`, in the edge's own direction. `None` when
// nothing does.
fn kept_from(
    outward: &[Vector3],
    segment: usize,
    point: Vector3,
    at_start: bool,
) -> Option<Vec<Vector3>> {
    let mut kept = vec![point];
    kept.extend(
        outward[segment + 1..]
            .iter()
            .filter(|knot| knot.distance_to(point) > CUT_KNOT_MERGE_M),
    );
    if kept.len() < 2 {
        return None;
    }
    if !at_start {
        kept.reverse();
    }
    Some(kept)
}

// Whether an arm cut to `ring` enters the circle again beyond its first segment, which leaves it.
fn comes_back(geometry: &[Vector3], at_start: bool, ring: RoundaboutRing) -> bool {
    let centre = Vector3::new(ring.centre.x, 0.0, ring.centre.y);
    let beyond = if at_start {
        &geometry[1..]
    } else {
        &geometry[..geometry.len() - 1]
    };
    beyond
        .windows(2)
        .any(|pair| segment_distance_xz(centre, centre, pair[0], pair[1]) < ring.radius_m)
}

// Where `points`, running out from the circle's centre, first leave `ring`'s circle: the segment
// and the point. `None` when they never do.
fn circle_cut(points: &[Vector3], ring: RoundaboutRing) -> Option<(usize, Vector3)> {
    let radius_m = ring.radius_m;
    for (i, pair) in points.windows(2).enumerate() {
        let a = Vector2::new(pair[0].x, pair[0].z) - ring.centre;
        let b = Vector2::new(pair[1].x, pair[1].z) - ring.centre;
        if b.length_squared() < radius_m * radius_m {
            continue;
        }
        let d = b - a;
        let det =
            a.dot(d).powi(2) - d.length_squared() * (a.length_squared() - radius_m * radius_m);
        if det < 0.0 || d.length_squared() < 1e-8 {
            continue;
        }
        let t = (-a.dot(d) + det.sqrt()) / d.length_squared();
        if (0.0..=1.0).contains(&t) {
            return Some((i, pair[0].lerp(pair[1], t)));
        }
    }
    None
}

// A ring edge from `from` to `to`, nodes unset (`apply` sets them).
fn ring_edge(ring: RoundaboutRing, from: Vector3, to: Vector3, lanes: u8) -> Edge {
    let points = ring.arc(from, to);
    let mut edge = Edge {
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR,
        class: EdgeClass::Standard,
        width: Edge::cross_section_width_m(lanes, 0, None),
        fwd_lanes: lanes,
        bkw_lanes: 0,
        speed_limit: RING_SPEED_MS,
        geometry: points.clone(),
        physical_geometry: points,
        no_building_spawn: true,
        roundabout: Some(ring),
        ..Edge::default()
    };
    edge.set_sidewalks(ring_sidewalk_side());
    price_edge(&mut edge);
    edge
}

// `edge`'s routing cost and length from its geometry.
fn price_edge(edge: &mut Edge) {
    (edge.base_cost, edge.physical_length) = CostCalculator::calculate_costs(edge);
}

#[cfg(test)]
mod tests;
