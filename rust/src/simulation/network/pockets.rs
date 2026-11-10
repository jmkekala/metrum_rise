// SPDX-License-Identifier: GPL-2.0-only

//! The turn pocket edit's plan (`ROAD-95`, PLAN-R5 2.5): a pure, local product of the graph that
//! gives a road end a pocket, resizes it, changes its roles or removes it. A pocket is the short
//! edge between the junction and a degree-two [`NodeType::LaneTransition`] node, split off its
//! base road `body + taper half` metres from the junction along the control polyline (D7); the
//! roadbed tapers the transition (D4). A removal joins the pair into one edge (D8), a resize
//! joins and splits again, moving the transition node and reusing the pair's edge id. The plan
//! is applied to the graph alone ([`PocketPlan::apply_to_graph`]); the dependents (buildings and
//! occupancy) move by their own call ([`migrate_pocket_dependents`]), journaled for undo.
//! `prepare` is `O(edge points + degree)`, `apply_to_graph` `O(points + the far node's rules)`.

use super::graph::{Edge, RegionGraph, RoadPocket};
use super::lanes::approach_offers_turns;
use super::lanes::vehicle_junctions::node_whitelists_vehicle_turns;
use super::roundabouts::ArmTrim;
use super::roadbed::{
    CROSSWALK_DEPTH_M, EdgeEnd, LEFT_POCKET_TAPER_M, NOSE_CLEARANCE_M, PocketNarrowing,
    medians_line_up,
};
use super::topology::{self, RoadSplitDependentsUndo, SplitHalves};
use super::types::NodeType;
use crate::simulation::buildings::allocator::{BuildingAllocator, EdgeOccupancy};
use godot::prelude::Vector3;

/// The shortest pocket body, from the junction node (D7).
pub(crate) const POCKET_BODY_MIN_M: f32 = 20.0;
/// The longest pocket body (D7).
pub(crate) const POCKET_BODY_MAX_M: f32 = 300.0;
/// The pocket body's step (D7).
pub(crate) const POCKET_BODY_STEP_M: f32 = 10.0;
/// The Road Properties window's default pocket body (D7).
pub(crate) const POCKET_BODY_DEFAULT_M: f32 = 60.0;
/// Half the roadbed's taper for a one-lane step (D4: `clamp(3 x 7 m, 12, 28) / 2`): the
/// transition's mouths stand this far from its node, so the split lies this far beyond the body.
pub(crate) const POCKET_TAPER_HALF_M: f32 = 10.5;
// What the base road keeps beyond the transition: both mouths of a taper and a metre.
const BASE_REMAINDER_MIN_M: f32 = 2.0 * POCKET_TAPER_HALF_M + 1.0;

/// One pocket edit at the end `end` of road `edge`: pocket lanes on the drivers' `left` and
/// `right` with a `body_m` metre body from the junction node; both roles off removes the pocket.
/// The end of a road joined to a pocket's transition node edits that pocket.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PocketRequest {
    /// The selected road.
    pub(crate) edge: usize,
    /// Its end the pocket is at, or whose transition leads to the pocket.
    pub(crate) end: EdgeEnd,
    /// A pocket lane on the drivers' left.
    pub(crate) left: bool,
    /// A pocket lane on the drivers' right.
    pub(crate) right: bool,
    /// The body length from the junction node, [`POCKET_BODY_MIN_M`] to [`POCKET_BODY_MAX_M`] in
    /// [`POCKET_BODY_STEP_M`] steps.
    pub(crate) body_m: f32,
}

/// A pocket as the graph holds it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PocketState {
    /// The pocket edge, between the junction and the transition.
    pub(crate) edge: usize,
    /// Its roles and the end at the junction.
    pub(crate) pocket: RoadPocket,
    /// The body from the junction node along the control polyline (the pocket edge's length
    /// less the taper half).
    pub(crate) body_m: f32,
    /// The junction the pocket lanes turn at.
    pub(crate) junction: u32,
}

// An existing pair: `first` runs into the transition, `second` out of it; `pocket` is the one
// that carries the pocket. `first_length_m` is `first`'s physical length, which the join's
// frontage offset reads.
#[derive(Clone, Copy, Debug)]
struct Pair {
    first: usize,
    second: usize,
    transition: u32,
    pocket: usize,
    first_length_m: f32,
}

// Where the base road is cut: its control segment, the transition's position, and the pocket in
// the base road's frame (its end is the junction's).
#[derive(Clone, Copy, Debug)]
struct Cut {
    segment: usize,
    point: Vector3,
    pocket: RoadPocket,
}

/// The planned pocket edit: the base road (the joined pair, or the selected road as it is) and,
/// unless the pocket is removed, where it is cut and which half takes the pocket.
#[derive(Clone)]
pub(crate) struct PocketPlan {
    pair: Option<Pair>,
    base: Edge,
    // The base road's id: the selected road's, or the pair's first.
    first: usize,
    junction: u32,
    cut: Option<Cut>,
}

/// What [`PocketPlan::apply_to_graph`] changed.
pub(crate) struct PocketApplied {
    /// The base road's id, kept by the half from its start.
    pub(crate) first: usize,
    /// The pair's second edge before the edit (joined into `first`), if there was a pair.
    pub(crate) joined: Option<usize>,
    /// The split, unless the pocket was removed.
    pub(crate) split: Option<SplitHalves>,
    /// The transition node: kept, moved, added, or left without arms by a removal.
    pub(crate) transition: Option<u32>,
    /// The pocket edge, unless the pocket was removed.
    pub(crate) pocket_edge: Option<usize>,
    // `first`'s physical length before the edit, for the join's frontage offset.
    first_length_m: f32,
}

impl PocketApplied {
    /// The edges whose records the edit replaced: the base road and the pair's or split's second
    /// edge.
    pub(crate) fn edges(&self) -> impl Iterator<Item = usize> + '_ {
        [
            Some(self.first),
            self.joined,
            self.split.as_ref().map(|split| split.new_edge_id),
        ]
        .into_iter()
        .flatten()
    }
}

fn node_at(edge: &Edge, end: EdgeEnd) -> u32 {
    match end {
        EdgeEnd::Start => edge.start_node,
        EdgeEnd::End => edge.end_node,
    }
}

fn opposite(end: EdgeEnd) -> EdgeEnd {
    match end {
        EdgeEnd::Start => EdgeEnd::End,
        EdgeEnd::End => EdgeEnd::Start,
    }
}

// The XZ length of a polyline: the control polyline's stations (`edge_profile_point_distance_m`).
fn xz_length_m(points: &[Vector3]) -> f32 {
    points
        .windows(2)
        .map(|pair| RegionGraph::edge_profile_point_distance_m(pair[0], pair[1]))
        .sum()
}

impl PocketState {
    /// The pocket at end `end` of road `edge`, or reached through the transition at that end;
    /// `None` without one. `O(degree)`.
    pub(crate) fn find(graph: &RegionGraph, edge: usize, end: EdgeEnd) -> Option<Self> {
        let pair = find_pair(graph, edge, end).ok().flatten()?;
        let pocket_edge = graph.edge(pair.pocket);
        let pocket = pocket_edge.pocket?;
        Some(Self {
            edge: pair.pocket,
            pocket,
            body_m: xz_length_m(&pocket_edge.geometry) - POCKET_TAPER_HALF_M,
            junction: graph.get_valid_node(node_at(pocket_edge, pocket.end)),
        })
    }
}

// The pair the request edits: the edge's own pocket at `end`, or the pocket of the other arm at
// a transition at `end` whose far end is that transition. `Ok(None)` for a plain road end; an
// error when the pair no longer has its shape (a third arm, a turned piece). `O(degree)`.
fn find_pair(graph: &RegionGraph, edge: usize, end: EdgeEnd) -> Result<Option<Pair>, &'static str> {
    let selected = graph.edge(edge);
    let pocket_id = match selected.pocket.filter(|pocket| pocket.end == end) {
        Some(_) => edge,
        None => {
            let node = graph.get_valid_node(node_at(selected, end));
            if graph.node(node).node_type != NodeType::LaneTransition {
                return Ok(None);
            }
            let other = graph.node_adjacency(node).iter().copied().find(|&other| {
                let arm = graph.edge(other);
                other != edge
                    && !arm.deleted
                    && arm.pocket.is_some_and(|pocket| {
                        graph.get_valid_node(node_at(arm, opposite(pocket.end))) == node
                    })
            });
            match other {
                Some(other) => other,
                None => return Ok(None),
            }
        }
    };
    let pocket_edge = graph.edge(pocket_id);
    let pocket = pocket_edge.pocket.expect("found by its pocket");
    let transition = graph.get_valid_node(node_at(pocket_edge, opposite(pocket.end)));
    if !graph.is_lane_transition(transition) {
        return Err("pocket_transition_changed");
    }
    let base = graph
        .node_adjacency(transition)
        .iter()
        .copied()
        .find(|&other| other != pocket_id && !graph.edge(other).deleted)
        .ok_or("pocket_transition_changed")?;
    let (first, second) = match pocket.end {
        EdgeEnd::End => (base, pocket_id),
        EdgeEnd::Start => (pocket_id, base),
    };
    if graph.edge(first).end_node != transition || graph.edge(second).start_node != transition {
        return Err("pocket_transition_changed");
    }
    Ok(Some(Pair {
        first,
        second,
        transition,
        pocket: pocket_id,
        first_length_m: graph.edge(first).physical_length,
    }))
}

// The base road with `pocket` set, when the road can carry its roles: the pocket's own checks
// ([`Edge::validate_pocket`]), then `pocket_median_gap` when the median would not run on past the
// transition. A left pocket's transition narrows a raised median and passes through
// (`validate_pocket`); a right pocket's is laid out and its piece carries a raised median between
// its mouths, while a painted one ends in chevrons either side of it (round R5b1a). Both halves
// are at least `MEDIAN_THROUGH_MIN_EDGE_M` long (`POCKET_BODY_MIN_M` plus the taper half,
// `BASE_REMAINDER_MIN_M`), so the pair's edge rule decides; it holds for every pocket
// `validate_pocket` accepts. `O(points)` for the clone.
fn pocket_piece(base: &Edge, pocket: RoadPocket) -> Result<Edge, &'static str> {
    let mut piece = base.clone();
    piece.set_pocket(Some(pocket));
    piece.validate_pocket()?;
    let piece_end = match pocket.end {
        EdgeEnd::Start => EdgeEnd::End,
        EdgeEnd::End => EdgeEnd::Start,
    };
    if let Some(median) = base.median
        && !medians_line_up((base, pocket.end), (&piece, piece_end), median.is_raised())
    {
        return Err("pocket_median_gap");
    }
    Ok(piece)
}

/// One road end as the Junction window's Turn pockets rows show it (`ROAD-95`, round R5c2):
/// the junction there, the road from it outwards (the pocket edge and its base road joined) and
/// whether the road can carry a pocket lane on each side, from the edit's own role checks.
pub(crate) struct PocketEndView {
    /// The node at this end: the pocket's junction, else the road's own end node.
    pub(crate) junction: u32,
    /// The road's physical polyline from the junction outwards.
    pub(crate) road: Vec<Vector3>,
    /// The widest of its edges, metres.
    pub(crate) width_m: f32,
    /// A left pocket's refusal on this road whatever its length (`Ok` when the road can carry
    /// one): [`Edge::validate_pocket`] and the median rule the edit applies.
    pub(crate) left: Result<(), &'static str>,
    /// The same for a right pocket.
    pub(crate) right: Result<(), &'static str>,
}

impl PocketEndView {
    /// End `end` of road `edge` (or the pocket reached through its transition there), `None`
    /// for an edge that is not live. `O(edge points + degree)`, allocating: a UI query.
    pub(crate) fn of(graph: &RegionGraph, edge: usize, end: EdgeEnd) -> Option<Self> {
        let selected = graph.get_edge(edge).filter(|edge| !edge.deleted)?;
        let (base, end, width_m) = match find_pair(graph, edge, end).ok().flatten() {
            Some(pair) => {
                let pocket = graph.edge(pair.pocket);
                let width_m = pocket
                    .width
                    .max(graph.edge(pair.first).width)
                    .max(graph.edge(pair.second).width);
                (joined_base(graph, &pair), pocket.pocket?.end, width_m)
            }
            None => (selected.clone(), end, selected.width),
        };
        let role = |left, right| pocket_piece(&base, RoadPocket { end, left, right }).map(|_| ());
        let mut road = base.physical_geometry.clone();
        if end == EdgeEnd::End {
            road.reverse();
        }
        Some(Self {
            junction: graph.get_valid_node(node_at(&base, end)),
            road,
            width_m,
            left: role(true, false),
            right: role(false, true),
        })
    }
}

// The pair joined into one record with `first`'s id: the base piece's lanes and attributes,
// both polylines end to end. `O(points)`.
fn joined_base(graph: &RegionGraph, pair: &Pair) -> Edge {
    let (a, b) = (graph.edge(pair.first), graph.edge(pair.second));
    let base_id = if pair.pocket == pair.first {
        pair.second
    } else {
        pair.first
    };
    let mut base = graph.edge(base_id).clone();
    base.start_node = a.start_node;
    base.end_node = b.end_node;
    base.start_clip = a.start_clip;
    base.end_clip = b.end_clip;
    base.geometry = a.geometry.clone();
    base.geometry.extend_from_slice(&b.geometry[1..]);
    base.physical_geometry = a.physical_geometry.clone();
    base.physical_geometry
        .extend_from_slice(&b.physical_geometry[1..]);
    base.pocket = None;
    let (cost, length) = crate::simulation::pathing::cost::CostCalculator::calculate_costs(&base);
    base.base_cost = cost;
    base.physical_length = length;
    base
}

impl PocketPlan {
    /// Plans `request` on `graph` without changing it: finds the pair (the road's own pocket or
    /// one reached through its transition), joins it, and cuts the base road for the requested
    /// pocket. The error is the refusal key: `invalid_road`, the pocket's own checks
    /// ([`Edge::validate_pocket`]: `pocket_requires_surface_road`, `pocket_lane_limit`,
    /// `pocket_requires_wide_median`, `invalid_pocket_dimensions`),
    /// `pocket_median_gap` (the median would not line up across the transition),
    /// `pocket_junction_whitelisted` (D9), `pocket_movement_missing` (the junction offers no such
    /// turn), `pocket_road_too_short` (the base road keeps less than two taper halves and a metre
    /// beyond the transition), `pocket_unchanged`, `pocket_transition_changed`. The compile's own refusals come later.
    /// `O(edge points + degree)`.
    pub(crate) fn prepare(
        graph: &RegionGraph,
        request: PocketRequest,
    ) -> Result<Self, &'static str> {
        let PocketRequest {
            edge,
            end,
            left,
            right,
            body_m,
        } = request;
        if graph.get_edge(edge).is_none_or(|edge| edge.deleted) {
            return Err("invalid_road");
        }
        let pair = find_pair(graph, edge, end)?;
        let current = pair.and_then(|pair| graph.edge(pair.pocket).pocket);
        let (base, first, end) = match &pair {
            Some(pair) => (
                joined_base(graph, pair),
                pair.first,
                current.expect("a pair has a pocket").end,
            ),
            None => (graph.edge(edge).clone(), edge, end),
        };
        let junction = graph.get_valid_node(node_at(&base, end));
        let plan = |cut| {
            Ok(Self {
                pair,
                base: base.clone(),
                first,
                junction,
                cut,
            })
        };
        if node_whitelists_vehicle_turns(graph, junction as usize) {
            return Err("pocket_junction_whitelisted");
        }
        if !left && !right {
            return match pair {
                Some(_) => plan(None),
                None => Err("pocket_unchanged"),
            };
        }
        let pocket = RoadPocket { end, left, right };
        let piece = pocket_piece(&base, pocket)?;
        if !body_m.is_finite()
            || !(POCKET_BODY_MIN_M..=POCKET_BODY_MAX_M).contains(&body_m)
            || body_m % POCKET_BODY_STEP_M != 0.0
        {
            return Err("invalid_pocket_dimensions");
        }
        // The approach at the junction: the pocket edge of a pair, else the selected road; both
        // run in the base road's direction there.
        let approach = pair.map_or(edge, |pair| pair.pocket);
        let (offers_left, offers_right) = approach_offers_turns(graph, approach, junction);
        if (left && !offers_left) || (right && !offers_right) {
            return Err("pocket_movement_missing");
        }
        let total_m = xz_length_m(&base.geometry);
        let span_m = body_m + POCKET_TAPER_HALF_M;
        if total_m - span_m < BASE_REMAINDER_MIN_M || !narrowing_fits(&base, &piece, end, span_m) {
            return Err("pocket_road_too_short");
        }
        if let Some(state) = pair.and_then(|_| PocketState::find(graph, edge, request.end))
            && state.pocket == pocket
            && (state.body_m - body_m).abs() < 1e-3
        {
            return Err("pocket_unchanged");
        }
        let station_m = match end {
            EdgeEnd::Start => span_m,
            EdgeEnd::End => total_m - span_m,
        };
        plan(Some(cut_at(&base, station_m, pocket)))
    }

    /// The planned pocket (its end and roles), unless the pocket is removed. `O(1)`.
    pub(crate) fn pocket(&self) -> Option<RoadPocket> {
        self.cut.map(|cut| cut.pocket)
    }

    /// The pair's edges, or the selected road: what the edit replaces.
    pub(crate) fn source_edges(&self) -> Vec<usize> {
        match self.pair {
            Some(pair) => vec![pair.first, pair.second],
            None => vec![self.first],
        }
    }

    /// The nodes the edit lays out again: the junction, the transition (when it exists) and the
    /// base road's far end.
    pub(crate) fn nodes(&self) -> Vec<u32> {
        let far = node_at(
            &self.base,
            if self.junction == self.base.start_node {
                EdgeEnd::End
            } else {
                EdgeEnd::Start
            },
        );
        let mut nodes = vec![self.junction, far];
        nodes.extend(self.pair.map(|pair| pair.transition));
        nodes
    }

    /// Installs the plan on `graph`, graph records only: joins the pair into the base road's id
    /// (the far node's rules and crossing overrides move back to it), then, unless the pocket is
    /// removed, cuts it at the planned station with the transition node moved there (or added),
    /// the second half in the pair's old slot, and gives the junction's half the pocket. The
    /// caller lays out the nodes ([`Self::nodes`]) and moves the dependents. `O(points + far
    /// node rules)`.
    pub(crate) fn apply_to_graph(&self, graph: &mut RegionGraph) -> PocketApplied {
        let first_length_m = self
            .pair
            .map_or(graph.edge(self.first).physical_length, |pair| {
                pair.first_length_m
            });
        if let Some(pair) = self.pair {
            join_pair(graph, &pair, self.base.clone());
        }
        let mut applied = PocketApplied {
            first: self.first,
            joined: self.pair.map(|pair| pair.second),
            split: None,
            transition: self.pair.map(|pair| pair.transition),
            pocket_edge: None,
            first_length_m,
        };
        let Some(cut) = self.cut else {
            if let Some(transition) = applied.transition {
                // An orphan, not a hidden node; not a border role (decision 5 of round R5a1).
                graph.nodes[transition as usize].node_type = NodeType::Junction;
            }
            return applied;
        };
        let transition = match applied.transition {
            Some(node) => {
                graph.remove_node_from_spatial_index(node, graph.node(node).pos);
                graph.set_node_pos(node, cut.point);
                graph.add_node_to_spatial_index(node);
                node
            }
            None => graph.add_node(cut.point, NodeType::LaneTransition),
        };
        graph.nodes[transition as usize].node_type = NodeType::LaneTransition;
        let split =
            topology::split_edge_graph(graph, self.first, cut.segment, transition, applied.joined)
                .expect("the cut lies at least a taper from both ends");
        let pocket_edge = match cut.pocket.end {
            EdgeEnd::Start => self.first,
            EdgeEnd::End => split.new_edge_id,
        };
        graph.set_edge_pocket(pocket_edge, Some(cut.pocket));
        applied.transition = Some(transition);
        applied.split = Some(split);
        applied.pocket_edge = Some(pocket_edge);
        applied
    }
}

// Whether a left pocket's median narrowing (`ROAD-95`, D3) ends before the pocket edge's nose at
// the junction, on a pocket edge `span_m` long: `LEFT_POCKET_TAPER_M` from the transition, and
// beyond the junction's mouth (the base road's clip there, which the pocket keeps: its mouth
// profile is the base road's), a crossing strip, the nose clearance and the narrowed median's
// nose radius. True for every other pocket, a left pocket beside a kerb pocket included: its
// transition's piece narrows the median (`PocketNarrowing::on_span`). `O(1)`.
fn narrowing_fits(base: &Edge, piece: &Edge, end: EdgeEnd, span_m: f32) -> bool {
    if PocketNarrowing::on_span(piece).is_none() {
        return true;
    }
    let clip_m = match end {
        EdgeEnd::Start => base.start_clip,
        EdgeEnd::End => base.end_clip,
    };
    let nose_m = clip_m
        + CROSSWALK_DEPTH_M as f32
        + NOSE_CLEARANCE_M
        + 0.5 * piece.median.map_or(0.0, |median| median.width_m);
    span_m - LEFT_POCKET_TAPER_M >= nose_m
}

// The cut of `base` at `station_m` along its control polyline: the segment, the point there (its
// height on the physical profile, so the halves keep the solved profile) and the pocket.
fn cut_at(base: &Edge, station_m: f32, pocket: RoadPocket) -> Cut {
    let points = &base.geometry;
    let mut walked_m = 0.0;
    let mut segment = points.len() - 2;
    let mut fraction = 1.0;
    for (index, pair) in points.windows(2).enumerate() {
        let run_m = RegionGraph::edge_profile_point_distance_m(pair[0], pair[1]);
        if walked_m + run_m >= station_m && run_m > 0.0 {
            segment = index;
            fraction = (station_m - walked_m) / run_m;
            break;
        }
        walked_m += run_m;
    }
    let mut point = points[segment].lerp(points[segment + 1], fraction);
    point.y = RegionGraph::profile_height_at(&base.physical_geometry, point.x, point.z);
    Cut {
        segment,
        point,
        pocket,
    }
}

// Joins `pair` into `first`'s slot as `base`; `second`'s slot is deleted, its geometry kept for
// the dependents' candidates, and the far node's rules and crossing overrides move back to
// `first`. O(points + far node rules).
fn join_pair(graph: &mut RegionGraph, pair: &Pair, base: Edge) {
    let (first, second, transition) = (pair.first, pair.second, pair.transition);
    let far = graph.edge(second).end_node;
    graph.remove_from_spatial_index(first);
    graph.remove_from_spatial_index(second);
    graph.adjacency[transition as usize].retain(|&edge| edge != first && edge != second);
    // Each node lists its arms in ascending order, as a load rebuilds them and the road tool's
    // replica restores them.
    let far_arms = &mut graph.adjacency[far as usize];
    far_arms.retain(|&edge| edge != second);
    let at = far_arms.partition_point(|&edge| edge < first);
    far_arms.insert(at, first);
    graph.edges[first] = base;
    graph.edges[second].deleted = true;
    graph.add_to_spatial_index(first);
    graph.move_node_rules_to_edge(far, second, first);
    let overrides = &mut graph.nodes[far as usize].crosswalk_overrides;
    if let Some(marked) = overrides.remove(&second) {
        overrides.insert(first, marked);
    }
}

/// The inverse journal of one pocket edit's dependents, or a roundabout's on its trimmed arms: the
/// join's, the split's and the trims' entries in the order they were applied.
pub(crate) struct PocketDependentsUndo {
    entries: Vec<DependentsEntry>,
}

enum DependentsEntry {
    Split(RoadSplitDependentsUndo),
    Join {
        rows: Vec<(usize, usize, usize, f32)>,
        occupancy: [(usize, Option<EdgeOccupancy>); 2],
    },
    Trim {
        rows: Vec<(usize, usize, usize, f32)>,
        edge: usize,
        occupancy: Option<EdgeOccupancy>,
    },
}

impl PocketDependentsUndo {
    /// Puts the buildings and occupancy back as they were before the edit, entries in reverse.
    /// O(rows + occupancy cells).
    pub(crate) fn restore(self, allocator: &mut BuildingAllocator) {
        for entry in self.entries.into_iter().rev() {
            match entry {
                DependentsEntry::Split(split) => split.restore(allocator),
                DependentsEntry::Join { rows, occupancy } => {
                    allocator.restore_split_frontages(&rows);
                    for (edge, cells) in occupancy {
                        match cells {
                            Some(cells) => allocator.edge_occupancy.insert(edge, cells),
                            None => allocator.edge_occupancy.remove(&edge),
                        };
                    }
                }
                DependentsEntry::Trim {
                    rows,
                    edge,
                    occupancy,
                } => {
                    allocator.restore_split_frontages(&rows);
                    match occupancy {
                        Some(cells) => allocator.edge_occupancy.insert(edge, cells),
                        None => allocator.edge_occupancy.remove(&edge),
                    };
                }
            }
        }
    }
}

/// Moves the dependents of an applied pocket edit, journaled: a join's second edge's buildings
/// and occupancy move onto the first behind its cells (`join_split_dependents`), then a split's
/// partition at the new station as a stroke's split does (`topology::migrate_split_dependents`).
/// `O(the pair's buildings + occupancy cells)`, plus the chunk candidates' test.
pub(crate) fn migrate_pocket_dependents(
    graph: &RegionGraph,
    applied: &PocketApplied,
    zoning: &crate::simulation::zoning::ZoningSystem,
    allocator: &mut BuildingAllocator,
) -> PocketDependentsUndo {
    let cell_m = zoning.config.zone_cell_m;
    let mut entries = Vec::new();
    if let Some(second) = applied.joined {
        entries.push(join_split_dependents(
            graph,
            [applied.first, second],
            applied.first_length_m,
            cell_m,
            allocator,
        ));
    }
    if let Some(split) = &applied.split {
        let mut journal = Some(Vec::new());
        topology::migrate_split_dependents(
            graph,
            applied.first,
            split.new_edge_id,
            (split.first_length_m / cell_m).floor() as usize,
            split.first_length_m,
            split.second_length_m,
            zoning,
            allocator,
            &mut journal,
        );
        entries.extend(journal.into_iter().flatten().map(DependentsEntry::Split));
    }
    PocketDependentsUndo { entries }
}

/// Moves the dependents of arms a roundabout cut (`ROAD-62`), journaled for its undo: an arm that
/// lost its start drops the whole cells before the cut, as the second half of a split does, and
/// every building takes its frontage along the shorter arm. The edit took the land under the
/// ring and its island first (`ZONE-10`, decision D-ring), demolishing the buildings on it, so a
/// cell dropped holds no building.
/// `O(the arms' buildings + occupancy cells)`, plus the chunk candidates' test.
pub(crate) fn migrate_arm_trims(
    graph: &RegionGraph,
    trims: &[ArmTrim],
    zoning: &crate::simulation::zoning::ZoningSystem,
    allocator: &mut BuildingAllocator,
) -> PocketDependentsUndo {
    let cell_m = zoning.config.zone_cell_m;
    let entries = trims.iter().map(|trim| {
        let dropped_m = trim.old_length_m - trim.new_length_m;
        let dropped = if trim.at_start {
            (dropped_m / cell_m).floor() as usize
        } else {
            0
        };
        // The land a cut drops lies on the ring's disc, whose zoned land the edit took first:
        // no building fronts the dropped part of an arm. A cell lot's building keeps no cell
        // index (`cell_x` 0), so its frontage station is read. Debug builds only: O(B).
        debug_assert!(
            !trim.at_start
                || allocator.buildings.iter().all(|building| {
                    building.edge_idx != trim.edge
                        || building.frontage_t * trim.old_length_m >= dropped_m
                }),
            "a building fronts the cut of arm {}",
            trim.edge
        );
        let mut rows = Vec::new();
        allocator.migrate_trim_frontages(
            graph,
            trim.edge,
            dropped,
            trim.new_length_m,
            cell_m,
            &mut rows,
        );
        let occupancy = allocator.edge_occupancy.get(&trim.edge).cloned();
        if let Some(cells) = allocator.edge_occupancy.get_mut(&trim.edge) {
            let kept = if trim.at_start {
                dropped.min(cells.cells_long)..cells.cells_long
            } else {
                0..cells.cells_long.min((trim.new_length_m / cell_m).ceil() as usize)
            };

            cells.cells_long = kept.len();
            cells.left = cells.left[kept.clone()].to_vec();
            cells.right = cells.right[kept].to_vec();
        }
        DependentsEntry::Trim {
            rows,
            edge: trim.edge,
            occupancy,
        }
    });
    PocketDependentsUndo {
        entries: entries.collect(),
    }
}

// `migrate_split_dependents`' inverse: `second`'s buildings front `first` behind its cells
// (its occupancy's length, else its length before the join in whole cells), and the occupancy
// rows are concatenated. O(the pair's buildings + cells).
fn join_split_dependents(
    graph: &RegionGraph,
    [first, second]: [usize; 2],
    first_length_m: f32,
    cell_m: f32,
    allocator: &mut BuildingAllocator,
) -> DependentsEntry {
    let occupancy =
        [first, second].map(|edge| (edge, allocator.edge_occupancy.get(&edge).cloned()));
    let offset_cells = occupancy[0]
        .1
        .as_ref()
        .map_or((first_length_m / cell_m).floor() as usize, |occ| {
            occ.cells_long
        });
    let mut rows = Vec::new();
    allocator.migrate_join_frontages(
        graph,
        [first, second],
        offset_cells,
        graph.edge(first).physical_length,
        cell_m,
        &mut rows,
    );
    if let Some(tail) = allocator.edge_occupancy.remove(&second) {
        // A first edge without occupancy takes free columns up to the offset.
        let head = allocator
            .edge_occupancy
            .entry(first)
            .or_insert_with(|| EdgeOccupancy {
                cells_long: offset_cells,
                left: vec![false; offset_cells],
                right: vec![false; offset_cells],
            });
        head.cells_long += tail.cells_long;
        head.left.extend_from_slice(&tail.left);
        head.right.extend_from_slice(&tail.right);
    }
    DependentsEntry::Join { rows, occupancy }
}
