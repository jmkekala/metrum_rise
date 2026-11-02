// SPDX-License-Identifier: GPL-2.0-only

//! Spatial interaction and querying utilities for the road network.
//!
//! Provides functions for snapping world positions to nodes and edges,
//! finding the closest entities, and geometrical intersections.

use super::graph::RegionGraph;
use super::roadbed::PieceId;
use super::types::EdgeClass;
use godot::prelude::*;

const EDGE_SNAP_ENDPOINT_MARGIN_M: f32 = 0.25;

/// Stable target within one road-tool graph generation; edges slide, nodes stay fixed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NetworkSnapTarget {
    /// Canonical live graph node.
    Node(u32),
    /// Live edge whose centerline receives the current pointer projection.
    Edge(usize),
}

/// The structure a road-tool cursor's ray hit (`ROAD-99` follow-up B1): the cursor snaps to a
/// structure only where its ray hit that structure's deck, never to a buried tunnel bore, and a
/// ray that hit the ground beside or under a bridge stays on the ground's level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StructureHit {
    /// No ray: every structure may take the snap and the level choice alone decides.
    Any,
    /// The ray hit the ground, a grounded piece or a tunnel's: no structure takes the snap.
    Ground,
    /// The ray hit this bridge span's deck: it and its nodes may take the snap.
    Span(usize),
    /// The ray hit the piece of this node, which only structures meet: it and its bridge arms
    /// may take the snap.
    Node(u32),
}

impl StructureHit {
    /// The structure of the roadbed piece a ray hit first in `graph` (`None`: no piece). A
    /// bridge's span, or a node piece no standard road meets; anything else is the ground.
    /// `O(degree)`.
    pub(crate) fn of_piece(graph: &RegionGraph, piece: Option<PieceId>) -> Self {
        match piece {
            Some(PieceId::Span(edge))
                if graph
                    .edges()
                    .get(edge)
                    .is_some_and(|edge| !edge.deleted && edge.class == EdgeClass::Bridge) =>
            {
                Self::Span(edge)
            }
            Some(PieceId::Node(node))
                if (node as usize) < graph.node_count() && !graph.grounded_node(node) =>
            {
                Self::Node(graph.get_valid_node(node))
            }
            _ => Self::Ground,
        }
    }

    /// Whether `target` may take the cursor's snap: always on the ground's level (a standard
    /// edge, a grounded node); a structure's only as this hit allows. `O(degree)`.
    pub(crate) fn admits(self, graph: &RegionGraph, target: NetworkSnapTarget) -> bool {
        match target {
            NetworkSnapTarget::Node(node) => {
                graph.grounded_node(node) || self.allows_node(graph, node)
            }
            NetworkSnapTarget::Edge(edge) => {
                graph.edge(edge).class == EdgeClass::Standard || self.allows_edge(graph, edge)
            }
        }
    }

    fn allows_node(self, graph: &RegionGraph, node: u32) -> bool {
        match self {
            Self::Any => true,
            Self::Ground => false,
            Self::Span(edge) => {
                let edge = graph.edge(edge);
                [edge.start_node, edge.end_node]
                    .into_iter()
                    .any(|end| graph.get_valid_node(end) == node)
            }
            Self::Node(hit) => hit == node,
        }
    }

    fn allows_edge(self, graph: &RegionGraph, edge_idx: usize) -> bool {
        match self {
            Self::Any => true,
            Self::Ground => false,
            Self::Span(hit) => hit == edge_idx,
            Self::Node(hit) => {
                let edge = graph.edge(edge_idx);
                edge.class == EdgeClass::Bridge
                    && [edge.start_node, edge.end_node]
                        .into_iter()
                        .any(|end| graph.get_valid_node(end) == hit)
            }
        }
    }
}

/// Current pointer projection and the network entity responsible for the snap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NetworkSnap {
    /// Exact network position: the target's graph height (`ROAD-99`), a node's own or an edge's
    /// profile at the projection.
    pub(crate) position: Vector3,
    /// Target retained until the pointer leaves its release radius or the graph changes.
    pub(crate) target: NetworkSnapTarget,
}

/// Finds the closest network point using only XZ distance for snap eligibility and scoring.
pub(crate) fn get_closest_point_xz(
    graph: &RegionGraph,
    world_pos: Vector3,
    max_dist: f32,
) -> Option<Vector3> {
    get_closest_network_snap_xz(graph, world_pos, max_dist).map(|snap| snap.position)
}

/// Reprojects one retained target, with no allocation or resident-network scan.
/// Edge work is O(its polyline segments); endpoint acquisition keeps existing node priority.
pub(crate) fn retained_network_snap_xz(
    graph: &RegionGraph,
    world_pos: Vector3,
    target: NetworkSnapTarget,
    node_acquire_dist: f32,
    release_dist: f32,
) -> Option<NetworkSnap> {
    if release_dist <= 0.0 {
        return None;
    }
    match target {
        NetworkSnapTarget::Node(node_id) => {
            if node_id as usize >= graph.node_count() || !graph.is_live_canonical_node(node_id) {
                return None;
            }
            let position = graph.node(node_id).pos;
            (snap_distance(position, world_pos) <= release_dist)
                .then_some(NetworkSnap { position, target })
        }
        NetworkSnapTarget::Edge(edge_id) => {
            let edge = graph.edges().get(edge_id).filter(|edge| !edge.deleted)?;
            let mut endpoint = None;
            let mut endpoint_distance = node_acquire_dist;
            for node_id in [edge.start_node, edge.end_node] {
                let node_id = graph.get_valid_node(node_id);
                let position = graph.node(node_id).pos;
                let distance = snap_distance(position, world_pos);
                if distance < endpoint_distance
                    || (distance == endpoint_distance
                        && endpoint.is_some_and(|snap: NetworkSnap| {
                            matches!(snap.target, NetworkSnapTarget::Node(best) if node_id < best)
                        }))
                {
                    endpoint_distance = distance;
                    endpoint = Some(NetworkSnap {
                        position,
                        target: NetworkSnapTarget::Node(node_id),
                    });
                }
            }
            if endpoint.is_some() {
                return endpoint;
            }
            let mut closest = None;
            let mut min_distance = release_dist;
            for (index, segment) in edge.geometry.windows(2).enumerate() {
                let Some(position) = get_edge_snap_point(
                    world_pos,
                    segment[0],
                    segment[1],
                    index == 0,
                    index + 2 == edge.geometry.len(),
                ) else {
                    continue;
                };
                let distance = snap_distance(position, world_pos);
                if distance <= min_distance {
                    min_distance = distance;
                    closest = Some(NetworkSnap { position, target });
                }
            }
            closest.map(|snap| at_graph_height(graph, snap))
        }
    }
}

/// Acquires a cursor snap using the existing node grid and edge R-tree.
///
/// The ground (standard edges and grounded nodes) and the structures (bridges, tunnels and the
/// nodes only they meet) are separate levels (`ROAD-99`): a road under a bridge lies within the
/// same XZ radius as the deck. Each level takes its snap as one network did before, nodes first;
/// where both have one, the snap whose graph height stands nearer the cursor's (`world_pos.y`,
/// the level its ray hit) wins, the ground on a tie. With one level near the cursor this is that
/// level's snap. The snap reports its graph height: a node's, an edge's profile at the
/// projection. Same candidates and bound as one level; one profile read per level.
pub(crate) fn get_closest_network_snap_xz(
    graph: &RegionGraph,
    world_pos: Vector3,
    max_dist: f32,
) -> Option<NetworkSnap> {
    get_closest_network_snap_at_hit(graph, world_pos, max_dist, StructureHit::Any)
}

/// [`get_closest_network_snap_xz`] for a cursor whose ray hit `hit` (`ROAD-99` follow-up B1): the
/// structures' level takes only the structure the ray hit, so a cursor on the ground beside or
/// under a bridge, or on a hill over a tunnel, never snaps to it. The ground's level is as
/// before, bitwise. Adds an `O(degree)` test per structure candidate.
pub(crate) fn get_closest_network_snap_at_hit(
    graph: &RegionGraph,
    world_pos: Vector3,
    max_dist: f32,
    hit: StructureHit,
) -> Option<NetworkSnap> {
    // Per level, ground first: the best snap, its score and the nearest node's distance.
    let mut closest_pos: [Option<NetworkSnap>; 2] = [None; 2];
    let mut min_score = [f32::MAX; 2];

    // 1. Check nodes first (Higher priority/Sticky)
    let node_snap_dist = max_dist * 2.5;
    let mut closest_node_dist = [f32::MAX; 2];

    let min = Vector3::new(
        world_pos.x - node_snap_dist,
        0.0,
        world_pos.z - node_snap_dist,
    );
    let max = Vector3::new(
        world_pos.x + node_snap_dist,
        0.0,
        world_pos.z + node_snap_dist,
    );
    graph.visit_nodes_near_aabb(min, max, |node_id| {
        if !graph.is_live_canonical_node(node_id) {
            return;
        }
        let node = graph.node(node_id);
        let d = snap_distance(node.pos, world_pos);
        if d < node_snap_dist {
            let level = usize::from(!graph.grounded_node(node_id));
            if level == 1 && !hit.allows_node(graph, node_id) {
                return;
            }
            let score = d * 0.4; // Nodes are 2.5x more "attractive" than segments
            if score < min_score[level]
                || (score == min_score[level]
                    && closest_pos[level].is_some_and(|snap: NetworkSnap| {
                        matches!(snap.target, NetworkSnapTarget::Node(best) if node_id < best)
                    }))
            {
                min_score[level] = score;
                closest_pos[level] = Some(NetworkSnap {
                    position: node.pos,
                    target: NetworkSnapTarget::Node(node_id),
                });
            }
            if d < closest_node_dist[level] {
                closest_node_dist[level] = d;
            }
        }
    });

    // ABSOLUTE NODE PRIORITY:
    // If the cursor is close to an existing intersection node (within the standard snap tolerance),
    // absolutely lock to it and skip that level's edges. This prevents the segment distance calculator from
    // mathematically overriding the node with a tiny fractional margin point (e.g. 0.1m away on an attached edge),
    // which results in overlapping collision fragments and chaotic geometric splitting.
    let locked = closest_node_dist.map(|distance| distance <= max_dist);

    // 2. Check nearby edges using Spatial Index. The R-tree's order is unspecified, so the
    // candidates ascend by edge id and the strict comparison below keeps the lowest id among
    // edges at exactly the same distance (`ROAD-68` open question 37).
    if !(locked[0] && locked[1]) {
        let mut nearby_edges = graph.get_edges_near_point(world_pos, max_dist.max(20.0));
        nearby_edges.sort_unstable();

        for edge_idx in nearby_edges {
            let edge = graph.edge(edge_idx);
            let level = usize::from(edge.class != EdgeClass::Standard);
            if edge.deleted || locked[level] || (level == 1 && !hit.allows_edge(graph, edge_idx)) {
                continue;
            }

            let half_width = edge.width * 0.5;
            let edge_snap_dist = f32::max(max_dist, half_width + 1.0);

            for (index, segment) in edge.geometry.windows(2).enumerate() {
                let Some(pos) = get_edge_snap_point(
                    world_pos,
                    segment[0],
                    segment[1],
                    index == 0,
                    index + 2 == edge.geometry.len(),
                ) else {
                    continue;
                };
                let d_perp = snap_distance(pos, world_pos);

                if d_perp < edge_snap_dist {
                    let score = d_perp;
                    if score < min_score[level] {
                        min_score[level] = score;
                        closest_pos[level] = Some(NetworkSnap {
                            position: pos,
                            target: NetworkSnapTarget::Edge(edge_idx),
                        });
                    }
                }
            }
        }
    }
    match closest_pos.map(|snap| snap.map(|snap| at_graph_height(graph, snap))) {
        [Some(ground), Some(structure)]
            if (structure.position.y - world_pos.y).abs()
                < (ground.position.y - world_pos.y).abs() =>
        {
            Some(structure)
        }
        [ground, structure] => ground.or(structure),
    }
}

// `snap` at its target's graph height (`ROAD-99`): a node's own, an edge's profile at the
// projection (its physical profile when present). `O(profile points)`.
fn at_graph_height(graph: &RegionGraph, mut snap: NetworkSnap) -> NetworkSnap {
    if let NetworkSnapTarget::Edge(edge_idx) = snap.target {
        let edge = graph.edge(edge_idx);
        let profile = if edge.physical_geometry.len() >= 2 {
            &edge.physical_geometry
        } else {
            &edge.geometry
        };
        snap.position.y = RegionGraph::profile_height_at(profile, snap.position.x, snap.position.z);
    }
    snap
}

fn snap_distance(a: Vector3, b: Vector3) -> f32 {
    let dx = a.x - b.x;
    let dz = a.z - b.z;
    (dx * dx + dz * dz).sqrt()
}

/// Returns the nearest live edge under the pointer in XZ, within an inclusive radius.
/// Equal-distance edges choose the lowest ID. Work follows visited R-tree entries and
/// candidate physical-profile segments, without allocating a candidate buffer.
pub(crate) fn get_closest_edge_xz(
    graph: &RegionGraph,
    world_x: f32,
    world_z: f32,
    max_dist: f32,
) -> Option<usize> {
    let pointer = Vector2::new(world_x, world_z);
    let min = Vector3::new(world_x - max_dist, 0.0, world_z - max_dist);
    let max = Vector3::new(world_x + max_dist, 0.0, world_z + max_dist);
    let mut best_dist = f32::MAX;
    let mut best_edge = None;
    graph.visit_edges_near_aabb(min, max, |edge_id| {
        let edge = graph.edge(edge_id);
        if edge.deleted {
            return;
        }
        for span in edge.physical_geometry.windows(2) {
            let a = Vector2::new(span[0].x, span[0].z);
            let b = Vector2::new(span[1].x, span[1].z);
            let segment = b - a;
            let offset = pointer - a;
            let length_sq = segment.length_squared();
            let distance_sq = if length_sq == 0.0 {
                offset.length_squared()
            } else {
                let t = ((offset.x * segment.x + offset.y * segment.y) / length_sq).clamp(0.0, 1.0);
                let projection = Vector2::new(a.x + t * segment.x, a.y + t * segment.y);
                (pointer - projection).length_squared()
            };
            if distance_sq < best_dist
                || (distance_sq == best_dist && best_edge.is_some_and(|best| edge_id < best))
            {
                best_dist = distance_sq;
                best_edge = Some(edge_id);
            }
        }
    });
    if best_dist <= max_dist * max_dist {
        best_edge
    } else {
        None
    }
}

/// Finds the closest live canonical node using 3D distance and lowest-ID ties.
pub fn get_closest_node(graph: &RegionGraph, world_pos: Vector3, max_dist: f32) -> Option<u32> {
    closest_node(graph, world_pos, max_dist, false)
}

/// Finds the closest live canonical editor node using XZ distance and lowest-ID ties.
/// Radius is exclusive; work follows the existing node grid and local incident edges.
pub(crate) fn get_closest_node_xz(
    graph: &RegionGraph,
    world_pos: Vector3,
    max_dist: f32,
) -> Option<u32> {
    closest_node(graph, world_pos, max_dist.abs(), true)
}

fn closest_node(
    graph: &RegionGraph,
    world_pos: Vector3,
    max_dist: f32,
    xz_only: bool,
) -> Option<u32> {
    let mut closest = None;
    let mut min_dist_sq = max_dist * max_dist;
    let min = Vector3::new(world_pos.x - max_dist, 0.0, world_pos.z - max_dist);
    let max = Vector3::new(world_pos.x + max_dist, 0.0, world_pos.z + max_dist);
    graph.visit_nodes_near_aabb(min, max, |node_id| {
        let position = graph.node(node_id).pos;
        let d_sq = if xz_only {
            let dx = position.x - world_pos.x;
            let dz = position.z - world_pos.z;
            dx * dx + dz * dz
        } else {
            position.distance_squared_to(world_pos)
        };
        if (d_sq < min_dist_sq
            || (d_sq == min_dist_sq && closest.is_some_and(|best| node_id < best)))
            && graph.is_live_canonical_node(node_id)
        {
            min_dist_sq = d_sq;
            closest = Some(node_id);
        }
    });
    closest
}

fn get_edge_snap_point(
    p: Vector3,
    a: Vector3,
    b: Vector3,
    start_endpoint: bool,
    end_endpoint: bool,
) -> Option<Vector3> {
    let dx = b.x - a.x;
    let dz = b.z - a.z;
    let length_sq = dx * dx + dz * dz;
    if length_sq <= 0.000001 {
        return None;
    }

    let seg_len = length_sq.sqrt();
    let end_margin = (EDGE_SNAP_ENDPOINT_MARGIN_M / seg_len).min(0.49);
    // Interior polyline knots are not graph endpoints: clamping each side creates 0.5 m jumps.
    let t = (((p.x - a.x) * dx + (p.z - a.z) * dz) / length_sq).clamp(
        if start_endpoint { end_margin } else { 0.0 },
        if end_endpoint { 1.0 - end_margin } else { 1.0 },
    );
    Some(a + (b - a) * t)
}

/// Finds the intersection point of two 2D segments in the XZ plane.
///
/// Returns `Some((t_a, t_b))` if they intersect, where `t` is the distance along the segment in `[0, 1]`.
pub fn find_intersection_2d(
    p1: Vector3,
    p2: Vector3,
    p3: Vector3,
    p4: Vector3,
) -> Option<(f32, f32)> {
    fn cross_xz(a: Vector3, b: Vector3) -> f32 {
        a.x * b.z - a.z * b.x
    }

    let r = p2 - p1;
    let s = p4 - p3;
    let denom = cross_xz(r, s);
    if denom.abs() < 0.0001 {
        return None;
    }

    let qp = p3 - p1;
    let t = cross_xz(qp, s) / denom;
    let u = cross_xz(qp, r) / denom;

    const PARAM_EPSILON: f32 = 0.00001;
    if (-PARAM_EPSILON..=1.0 + PARAM_EPSILON).contains(&t)
        && (-PARAM_EPSILON..=1.0 + PARAM_EPSILON).contains(&u)
    {
        Some((t.clamp(0.0, 1.0), u.clamp(0.0, 1.0)))
    } else {
        None
    }
}

/// How a road drawn from a snapped start point should leave it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum StartDirection {
    /// Out of a dead end, continuing its road: unit XZ direction, signed.
    Continue(Vector3),
    /// Along the road through the start (an edge, or a degree-two node): unit XZ, either sense.
    Along(Vector3),
}

// Unit XZ direction of the edge's centreline leaving `node` into the edge, skipping
// sub-`MIN_RUN_M` segments. O(points of the edge).
fn direction_into_edge(graph: &RegionGraph, edge_id: usize, node: u32) -> Option<Vector3> {
    const MIN_RUN_M: f32 = 0.05;
    let edge = graph.edge(edge_id);
    let points = if edge.physical_geometry.len() >= 2 {
        &edge.physical_geometry
    } else {
        &edge.geometry
    };
    let first = if edge.start_node == node {
        *points.first()?
    } else {
        *points.last()?
    };
    let flat = |p: Vector3| Vector3::new(p.x - first.x, 0.0, p.z - first.z);
    let away = |p: &Vector3| (flat(*p).length() >= MIN_RUN_M).then(|| flat(*p).normalized());
    if edge.start_node == node {
        points.iter().find_map(away)
    } else {
        points.iter().rev().find_map(away)
    }
}

/// The direction a road started on `target` should take: continuing a dead end, or along the road
/// through the start. None at a junction of three or more arms. O(points of the touched edges).
pub(crate) fn road_start_direction(
    graph: &RegionGraph,
    target: NetworkSnapTarget,
    position: Vector3,
) -> Option<StartDirection> {
    match target {
        NetworkSnapTarget::Node(node) => {
            let mut arms = graph
                .node_adjacency(node)
                .iter()
                .copied()
                .filter(|&edge| !graph.edge(edge).deleted);
            let first = arms.next()?;
            let into = direction_into_edge(graph, first, node)?;
            match (arms.next(), arms.next()) {
                (None, _) => Some(StartDirection::Continue(-into)),
                (Some(_), None) => Some(StartDirection::Along(into)),
                _ => None,
            }
        }
        NetworkSnapTarget::Edge(edge_id) => {
            let edge = graph.edge(edge_id);
            let points = if edge.physical_geometry.len() >= 2 {
                &edge.physical_geometry
            } else {
                &edge.geometry
            };
            points
                .windows(2)
                .filter_map(|pair| {
                    let d = Vector3::new(pair[1].x - pair[0].x, 0.0, pair[1].z - pair[0].z);
                    let len_sq = d.length_squared();
                    (len_sq > 1e-6).then(|| {
                        let p = Vector3::new(position.x - pair[0].x, 0.0, position.z - pair[0].z);
                        let t = (p.dot(d) / len_sq).clamp(0.0, 1.0);
                        ((p - d * t).length_squared(), d / len_sq.sqrt())
                    })
                })
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, dir)| StartDirection::Along(dir))
        }
    }
}

/// Rotates the stroke `start -> cursor` onto the nearest 45-degree multiple from `reference`
/// (unit XZ) when it lies within `tolerance_rad` of it, keeping its horizontal length. Returns the
/// snapped XZ position (y from `cursor`; the caller samples the surface), or None when the stroke
/// is free or too short to have a direction.
pub(crate) fn snap_stroke_angle(
    start: Vector3,
    cursor: Vector3,
    reference: Vector3,
    tolerance_rad: f32,
) -> Option<Vector3> {
    let d = Vector3::new(cursor.x - start.x, 0.0, cursor.z - start.z);
    let length = d.length();
    if length < 1.0 || reference.length_squared() < 1e-6 {
        return None;
    }
    let reference = Vector3::new(reference.x, 0.0, reference.z).normalized();
    let base = reference.z.atan2(reference.x);
    let relative = d.z.atan2(d.x) - base;
    let step_rad = std::f32::consts::FRAC_PI_4;
    let snapped = (relative / step_rad).round() * step_rad;
    let off = (relative - snapped).rem_euclid(std::f32::consts::TAU);
    if off.min(std::f32::consts::TAU - off) > tolerance_rad {
        return None;
    }
    let angle = base + snapped;
    Some(Vector3::new(
        start.x + angle.cos() * length,
        cursor.y,
        start.z + angle.sin() * length,
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        NetworkSnapTarget, StructureHit, find_intersection_2d, get_closest_network_snap_at_hit,
        get_closest_network_snap_xz, get_closest_point_xz, get_edge_snap_point,
        retained_network_snap_xz,
    };
    use crate::simulation::network::graph::{Edge, RegionGraph};
    use crate::simulation::network::types::{EdgeClass, NodeType};
    use godot::prelude::Vector3;

    #[test]
    fn hover_query_ignores_deleted_roads() {
        let mut graph = RegionGraph::new();
        for x in [0.0, 2.0] {
            let start = graph.add_node(Vector3::new(x, 0.0, 0.0), NodeType::Junction);
            let end = graph.add_node(Vector3::new(x, 0.0, 20.0), NodeType::Junction);
            graph.add_edge(super::super::build_surface_edge(
                start,
                end,
                vec![graph.node(start).pos, graph.node(end).pos],
                1,
                1,
                EdgeClass::Standard,
                None,
            ));
        }
        assert_eq!(super::get_closest_edge_xz(&graph, 0.1, 10.0, 3.0), Some(0));
        graph.remove_from_spatial_index(0);
        graph.edge_mut(0).deleted = true;
        assert_eq!(super::get_closest_edge_xz(&graph, 0.1, 10.0, 3.0), Some(1));
        graph.remove_from_spatial_index(1);
        graph.edge_mut(1).deleted = true;
        assert_eq!(super::get_closest_edge_xz(&graph, 0.1, 10.0, 3.0), None);
    }

    #[test]
    fn hover_query_preserves_radius_ties_and_repeated_profile_points() {
        let mut graph = RegionGraph::new();
        for x in [2.0, -2.0] {
            let start = graph.add_node(Vector3::new(x, 50.0, -20.0), NodeType::Junction);
            let end = graph.add_node(Vector3::new(x, 80.0, 20.0), NodeType::Junction);
            graph.add_edge(super::super::build_surface_edge(
                start,
                end,
                vec![
                    graph.node(start).pos,
                    graph.node(start).pos,
                    graph.node(end).pos,
                ],
                1,
                1,
                EdgeClass::Standard,
                None,
            ));
        }
        for _ in 0..2 {
            assert_eq!(super::get_closest_edge_xz(&graph, 0.0, 0.0, 2.0), Some(0));
            assert_eq!(super::get_closest_edge_xz(&graph, 0.0, 0.0, 1.99), None);
            assert_eq!(super::get_closest_edge_xz(&graph, 0.0, -21.0, 3.0), Some(0));
            assert_eq!(super::get_closest_edge_xz(&graph, 0.0, 50.0, 5.0), None);
            // R-tree update history cannot choose a different equal-distance road.
            for id in [0, 1] {
                graph.remove_from_spatial_index(id);
            }
            for id in [1, 0] {
                graph.add_to_spatial_index(id);
            }
        }
    }

    // `ROAD-68` open question 37 (b): on an exact tie the cursor snap takes the lower edge id,
    // whichever order the R-tree holds the edges in.
    #[test]
    fn cursor_snap_ties_take_the_lowest_edge_id_in_any_tree_order() {
        let mut graph = RegionGraph::new();
        for x in [-2.0, 2.0] {
            let start = graph.add_node(Vector3::new(x, 0.0, -20.0), NodeType::Junction);
            let end = graph.add_node(Vector3::new(x, 0.0, 20.0), NodeType::Junction);
            let points = vec![graph.node(start).pos, graph.node(end).pos];
            graph.add_edge(super::super::build_surface_edge(
                start,
                end,
                points,
                1,
                1,
                EdgeClass::Standard,
                None,
            ));
        }
        for order in [[0, 1], [1, 0]] {
            for id in [0, 1] {
                graph.remove_from_spatial_index(id);
            }
            for id in order {
                graph.add_to_spatial_index(id);
            }
            assert_eq!(
                get_closest_network_snap_xz(&graph, Vector3::ZERO, 3.0)
                    .unwrap()
                    .target,
                NetworkSnapTarget::Edge(0),
                "tree insertion order {order:?}"
            );
        }
    }

    #[test]
    #[ignore = "matched release edge-hover query locality measurement"]
    fn benchmark_editor_edge_query_locality() {
        use std::{hint::black_box, time::Instant};
        const QUERIES: usize = 64;
        for background in [0, 1_000, 10_000, 100_000] {
            let setup = Instant::now();
            let mut graph = RegionGraph::new();
            for i in 0..=background {
                let x = if i == 0 {
                    0.0
                } else {
                    1_000.0 + (i % 512) as f32 * 64.0
                };
                let z = if i == 0 {
                    0.0
                } else {
                    1_000.0 + (i / 512) as f32 * 64.0
                };
                let points: Vec<_> = (0..8)
                    .map(|k| Vector3::new(x + k as f32 * 4.0, 0.0, z))
                    .collect();
                let start = graph.add_node(points[0], NodeType::Junction);
                let end = graph.add_node(points[7], NodeType::Junction);
                graph.add_edge(super::super::build_surface_edge(
                    start,
                    end,
                    points,
                    1,
                    1,
                    EdgeClass::Standard,
                    None,
                ));
            }
            let setup_us = setup.elapsed().as_secs_f64() * 1e6;
            let mut samples = Vec::new();
            for sample in 0..10 {
                let mut outputs = [None; QUERIES];
                let start = Instant::now();
                for (i, output) in outputs.iter_mut().enumerate() {
                    let (x, z) =
                        black_box([(0.5, 2.0), (14.0, -2.0), (27.5, 3.0), (-50.0, -50.0)][i % 4]);
                    *output = super::get_closest_edge_xz(black_box(&graph), x, z, black_box(5.0));
                }
                let elapsed_us = start.elapsed().as_secs_f64() * 1e6 / QUERIES as f64;
                for (i, output) in outputs.into_iter().enumerate() {
                    assert_eq!(output, if i % 4 == 3 { None } else { Some(0) });
                }
                if sample > 0 {
                    samples.push(elapsed_us);
                }
            }
            samples.sort_by(f64::total_cmp);
            println!(
                "EDGE_QUERY_BENCH {}",
                serde_json::json!({
                    "background":background,"edges":graph.edge_count(),"points_per_edge":8,
                    "median_us":samples[samples.len()/2],"setup_us":setup_us,
                    "queries_per_sample":QUERIES,"fingerprint":[0,0,0,-1],
                })
            );
        }
    }

    #[test]
    #[ignore = "matched release node-query cost after retaining large index capacity"]
    fn benchmark_node_grid_retained_capacity() {
        use std::{hint::black_box, time::Instant};
        const QUERIES: usize = 64;
        for reserved in [0, 1_000, 100_000, 1_000_000] {
            let setup = Instant::now();
            let mut graph = RegionGraph::new();
            let start = graph.add_node(Vector3::ZERO, NodeType::Junction);
            let end = graph.add_node(Vector3::new(0.0, 0.0, 20.0), NodeType::Junction);
            graph.add_edge(test_edge(start, end));
            // Reserve models the table allocation left by a large previous graph. The real
            // rebuild clears/repopulates the same table; setup is outside query timing.
            graph.spatial_node_grid.reserve(reserved);
            let capacity_before = graph.spatial_node_grid.capacity();
            graph.rebuild_all_indices();
            assert_eq!(graph.spatial_node_grid.capacity(), capacity_before);
            assert_eq!(graph.spatial_node_grid.len(), 2);
            let setup_us = setup.elapsed().as_secs_f64() * 1e6;
            for operation in ["editor", "snap"] {
                let mut samples = Vec::new();
                for sample in 0..10 {
                    let mut outputs = [None; QUERIES];
                    let start = Instant::now();
                    for (i, output) in outputs.iter_mut().enumerate() {
                        let position = black_box(
                            [
                                Vector3::new(0.25, 0.0, 0.5),
                                Vector3::new(0.25, 0.0, 19.5),
                                Vector3::new(-0.25, 0.0, -0.5),
                                Vector3::new(-50.0, 0.0, -50.0),
                            ][i % 4],
                        );
                        *output = if operation == "editor" {
                            super::get_closest_node_xz(black_box(&graph), position, black_box(5.0))
                        } else {
                            get_closest_network_snap_xz(black_box(&graph), position, black_box(5.0))
                                .map(|snap| match snap.target {
                                    NetworkSnapTarget::Node(id) => id,
                                    NetworkSnapTarget::Edge(_) => u32::MAX,
                                })
                        };
                    }
                    let elapsed_us = start.elapsed().as_secs_f64() * 1e6 / QUERIES as f64;
                    for (i, output) in outputs.into_iter().enumerate() {
                        assert_eq!(output, [Some(0), Some(1), Some(0), None][i % 4]);
                    }
                    if sample > 0 {
                        samples.push(elapsed_us);
                    }
                }
                samples.sort_by(f64::total_cmp);
                println!(
                    "NODE_GRID_CAPACITY_BENCH {}",
                    serde_json::json!({
                        "reserved":reserved,"capacity":graph.spatial_node_grid.capacity(),
                        "populated_cells":graph.spatial_node_grid.len(),"operation":operation,
                        "median_us":samples[samples.len()/2],"setup_us":setup_us,
                        "queries_per_sample":QUERIES,"fingerprint":[0,1,0,-1],
                    })
                );
            }
        }
    }

    #[test]
    fn node_selection_ties_use_lowest_id_across_lookup_chunks() {
        for reverse_edge in [false, true] {
            let mut graph = RegionGraph::new();
            let low = graph.add_node(Vector3::new(1.0, 0.0, 0.0), NodeType::Junction);
            let high = graph.add_node(Vector3::new(-1.0, 0.0, 0.0), NodeType::Junction);
            let (start, end) = if reverse_edge {
                (high, low)
            } else {
                (low, high)
            };
            let mut edge = test_edge(start, end);
            edge.geometry = vec![graph.node(start).pos, graph.node(end).pos];
            edge.physical_geometry = edge.geometry.clone();
            let edge_id = graph.add_edge(edge);
            assert_eq!(
                super::get_closest_node(&graph, Vector3::ZERO, 2.0),
                Some(low)
            );
            assert_eq!(
                get_closest_network_snap_xz(&graph, Vector3::ZERO, 2.0)
                    .unwrap()
                    .target,
                NetworkSnapTarget::Node(low)
            );
            assert_eq!(
                retained_network_snap_xz(
                    &graph,
                    Vector3::ZERO,
                    NetworkSnapTarget::Edge(edge_id),
                    2.0,
                    5.0
                )
                .unwrap()
                .target,
                NetworkSnapTarget::Node(low)
            );
        }
    }

    #[test]
    fn cursor_snap_is_continuous_across_interior_polyline_knots() {
        let mut graph = RegionGraph::new();
        let start = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let end = graph.add_node(Vector3::new(0.0, 2.0, 20.0), NodeType::Junction);
        let mut edge = test_edge(start, end);
        edge.geometry = vec![
            Vector3::ZERO,
            Vector3::new(2.0, 1.0, 10.0),
            Vector3::new(0.0, 2.0, 20.0),
        ];
        edge.physical_geometry = edge.geometry.clone();
        let edge_id = graph.add_edge(edge);
        let target = NetworkSnapTarget::Edge(edge_id);
        for step in 0..81 {
            let z = 8.0 + step as f32 * 0.05;
            let point = Vector3::new(2.0 - (z - 10.0).abs() * 0.2, z * 0.1, z);
            let fresh = get_closest_network_snap_xz(&graph, point, 5.0).unwrap();
            let retained = retained_network_snap_xz(&graph, point, target, 5.0, 8.0).unwrap();
            assert_eq!(fresh.target, target);
            assert_eq!(retained.target, target);
            assert!(fresh.position.distance_to(point) < 0.00001);
            assert!(retained.position.distance_to(point) < 0.00001);
        }
    }

    #[test]
    fn retained_edge_does_not_jump_to_parallel_road_and_acquires_its_endpoint() {
        let mut graph = RegionGraph::new();
        let start = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let end = graph.add_node(Vector3::new(0.0, 0.0, 20.0), NodeType::Junction);
        let first = graph.add_edge(test_edge(start, end));
        let second_start = graph.add_node(Vector3::new(3.0, 0.0, 0.0), NodeType::Junction);
        let second_end = graph.add_node(Vector3::new(3.0, 0.0, 20.0), NodeType::Junction);
        let mut second_edge = test_edge(second_start, second_end);
        for point in &mut second_edge.geometry {
            point.x += 3.0;
        }
        second_edge.physical_geometry = second_edge.geometry.clone();
        let second = graph.add_edge(second_edge);
        let point = Vector3::new(2.9, 0.0, 10.0);
        assert_eq!(
            get_closest_network_snap_xz(&graph, point, 5.0)
                .unwrap()
                .target,
            NetworkSnapTarget::Edge(second)
        );
        let target = NetworkSnapTarget::Edge(first);
        assert_eq!(
            retained_network_snap_xz(&graph, point, target, 5.0, 8.0)
                .unwrap()
                .position,
            Vector3::new(0.0, 0.0, 10.0)
        );
        assert_eq!(
            retained_network_snap_xz(&graph, Vector3::new(0.5, 0.0, 19.0), target, 5.0, 8.0)
                .unwrap()
                .target,
            NetworkSnapTarget::Node(end)
        );
        assert!(
            retained_network_snap_xz(&graph, point, NetworkSnapTarget::Edge(999), 5.0, 8.0)
                .is_none()
        );
        assert!(
            retained_network_snap_xz(&graph, point, NetworkSnapTarget::Node(999), 5.0, 8.0)
                .is_none()
        );
    }

    #[test]
    fn edge_snap_uses_exact_projection_instead_of_quantized_steps() {
        let snapped = get_edge_snap_point(
            Vector3::new(3.7, 0.0, 1.0),
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(10.0, 0.0, 0.0),
            true,
            true,
        )
        .unwrap();

        assert!((snapped.x - 3.7).abs() < 0.001);
        assert!(snapped.z.abs() < 0.001);
    }

    #[test]
    fn closest_point_ignores_aliased_merged_nodes() {
        let mut graph = RegionGraph::new();
        let keep = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let remove = graph.add_node(Vector3::new(1.0, 0.0, 0.0), NodeType::Junction);
        let far = graph.add_node(Vector3::new(0.0, 0.0, 20.0), NodeType::Junction);
        graph.add_edge(test_edge(keep, far));
        graph.unite_nodes(keep, remove);

        let snapped = get_closest_point_xz(&graph, Vector3::new(0.8, 0.0, 0.0), 5.0).unwrap();
        assert!(snapped.distance_to(Vector3::ZERO) <= 0.001);
    }

    #[test]
    fn xz_closest_point_ignores_height_delta_for_editor_snap() {
        let mut graph = RegionGraph::new();
        let start = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let end = graph.add_node(Vector3::new(0.0, 0.0, 20.0), NodeType::Junction);
        graph.add_edge(test_edge(start, end));

        let snapped = get_closest_point_xz(&graph, Vector3::new(0.1, 20.0, 0.1), 5.0).unwrap();
        assert!(snapped.distance_to(Vector3::ZERO) <= 0.001);
    }

    #[test]
    fn xz_edge_snap_projects_along_horizontal_footprint() {
        let snapped = get_edge_snap_point(
            Vector3::new(8.0, 30.0, 0.0),
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(10.0, 10.0, 0.0),
            true,
            true,
        )
        .unwrap();

        assert!((snapped.x - 8.0).abs() < 0.001);
        assert!((snapped.y - 8.0).abs() < 0.001);
        assert!(snapped.z.abs() < 0.001);
    }

    fn test_edge(start_node: u32, end_node: u32) -> Edge {
        super::super::build_surface_edge(
            start_node,
            end_node,
            vec![Vector3::ZERO, Vector3::new(0.0, 0.0, 20.0)],
            1,
            1,
            EdgeClass::Standard,
            None,
        )
    }

    #[test]
    fn segment_intersection_returns_parameters_on_both_segments() {
        let (t, u) = find_intersection_2d(
            Vector3::new(0.0, 0.0, -100.0),
            Vector3::new(0.0, 0.0, 100.0),
            Vector3::new(-100.0, 0.0, 0.0),
            Vector3::new(100.0, 0.0, 0.0),
        )
        .expect("perpendicular centerlines should intersect");

        assert!((t - 0.5).abs() < 0.0001);
        assert!((u - 0.5).abs() < 0.0001);
    }

    #[test]
    fn stroke_angle_snaps_within_tolerance_and_keeps_length() {
        use super::snap_stroke_angle;
        let start = Vector3::new(10.0, 0.0, 10.0);
        let east = Vector3::new(1.0, 0.0, 0.0);
        // 3 degrees off the reference snaps straight on, same length.
        let a = 3f32.to_radians();
        let cursor = start + Vector3::new(a.cos(), 0.0, a.sin()) * 20.0;
        let snapped = snap_stroke_angle(start, cursor, east, 4f32.to_radians()).unwrap();
        assert!((snapped - Vector3::new(30.0, 0.0, 10.0)).length() < 1e-3);
        // 88 degrees snaps to the perpendicular.
        let a = 88f32.to_radians();
        let cursor = start + Vector3::new(a.cos(), 0.0, a.sin()) * 20.0;
        let snapped = snap_stroke_angle(start, cursor, east, 4f32.to_radians()).unwrap();
        assert!((snapped - Vector3::new(10.0, 0.0, 30.0)).length() < 1e-3);
        // 20 degrees is free.
        let a = 20f32.to_radians();
        let cursor = start + Vector3::new(a.cos(), 0.0, a.sin()) * 20.0;
        assert!(snap_stroke_angle(start, cursor, east, 4f32.to_radians()).is_none());
    }

    #[test]
    fn stroke_angle_uses_45_degree_steps_relative_to_the_reference() {
        use super::snap_stroke_angle;
        let start = Vector3::new(10.0, 2.0, -10.0);
        let base = 30f32.to_radians();
        let reference = Vector3::new(base.cos(), 0.0, base.sin());
        let tolerance = 4f32.to_radians();
        for turn in [-135.0_f32, -45.0, 45.0, 180.0] {
            let angle = base + (turn + 3.0).to_radians();
            let cursor = start + Vector3::new(angle.cos() * 20.0, 5.0, angle.sin() * 20.0);
            let snapped = snap_stroke_angle(start, cursor, reference, tolerance).unwrap();
            let expected_angle = base + turn.to_radians();
            let expected = start
                + Vector3::new(
                    expected_angle.cos() * 20.0,
                    5.0,
                    expected_angle.sin() * 20.0,
                );
            assert!((snapped - expected).length() < 1e-3);
        }
        // The removed arc chord step must not snap a straight stroke at 22.5 degrees.
        let angle = base + 22.5f32.to_radians();
        let cursor = start + Vector3::new(angle.cos(), 0.0, angle.sin()) * 20.0;
        assert!(snap_stroke_angle(start, cursor, reference, tolerance).is_none());
        assert!(snap_stroke_angle(start, cursor, Vector3::ZERO, tolerance).is_none());
        assert!(snap_stroke_angle(start, start + reference * 0.5, reference, tolerance).is_none());
    }

    #[test]
    fn start_direction_continues_a_dead_end_and_follows_a_through_road() {
        use super::{StartDirection, road_start_direction};
        let mut graph = RegionGraph::new();
        let a = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let b = graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
        let c = graph.add_node(Vector3::new(20.0, 0.0, 30.0), NodeType::Junction);
        graph.add_edge(super::super::build_surface_edge(
            a,
            b,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
            1,
            1,
            EdgeClass::Standard,
            None,
        ));
        graph.add_edge(super::super::build_surface_edge(
            b,
            c,
            vec![Vector3::new(20.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 30.0)],
            1,
            1,
            EdgeClass::Standard,
            None,
        ));
        // Dead end at a: continue westward, away from the road.
        assert_eq!(
            road_start_direction(&graph, NetworkSnapTarget::Node(a), Vector3::ZERO),
            Some(StartDirection::Continue(Vector3::new(-1.0, 0.0, 0.0)))
        );
        // Dead end at c: continue north (+z).
        assert_eq!(
            road_start_direction(
                &graph,
                NetworkSnapTarget::Node(c),
                Vector3::new(20.0, 0.0, 30.0)
            ),
            Some(StartDirection::Continue(Vector3::new(0.0, 0.0, 1.0)))
        );
        // Mid-edge: along the road.
        assert_eq!(
            road_start_direction(
                &graph,
                NetworkSnapTarget::Edge(0),
                Vector3::new(8.0, 0.0, 1.0)
            ),
            Some(StartDirection::Along(Vector3::new(1.0, 0.0, 0.0)))
        );
        assert!(matches!(
            road_start_direction(
                &graph,
                NetworkSnapTarget::Node(b),
                Vector3::new(20.0, 0.0, 0.0)
            ),
            Some(StartDirection::Along(_))
        ));
    }

    // `ROAD-99` follow-up B1: a tunnel's bore is never what a cursor's ray hit, so it never takes
    // the snap, and neither does a bridge the ray did not hit; the ground's snap is unchanged. A
    // bore along x at y = -10, a bridge along z at y = 6 and a ground road along z at x = 40.
    #[test]
    fn the_snap_takes_only_the_structure_the_ray_hit() {
        use crate::simulation::network::roadbed::PieceId;
        let mut graph = RegionGraph::new();
        for (class, [a, b]) in [
            (
                EdgeClass::Tunnel,
                [
                    Vector3::new(-30.0, -10.0, 0.0),
                    Vector3::new(30.0, -10.0, 0.0),
                ],
            ),
            (
                EdgeClass::Bridge,
                [Vector3::new(0.0, 6.0, 20.0), Vector3::new(0.0, 6.0, 80.0)],
            ),
            (
                EdgeClass::Standard,
                [Vector3::new(40.0, 0.0, 20.0), Vector3::new(40.0, 0.0, 80.0)],
            ),
        ] {
            let start = graph.add_node(a, NodeType::Junction);
            let end = graph.add_node(b, NodeType::Junction);
            graph.add_edge(super::super::build_surface_edge(
                start,
                end,
                vec![a, b],
                1,
                1,
                class,
                None,
            ));
        }
        // A ray's piece: a bore's is the ground, a bridge's span its own, a missing one the ground.
        assert_eq!(
            StructureHit::of_piece(&graph, Some(PieceId::Span(0))),
            StructureHit::Ground
        );
        assert_eq!(
            StructureHit::of_piece(&graph, Some(PieceId::Span(1))),
            StructureHit::Span(1)
        );
        assert_eq!(
            StructureHit::of_piece(&graph, Some(PieceId::Span(2))),
            StructureHit::Ground
        );
        assert_eq!(StructureHit::of_piece(&graph, None), StructureHit::Ground);
        let snap = |cursor: Vector3, hit: StructureHit| {
            get_closest_network_snap_at_hit(&graph, cursor, 5.0, hit).map(|snap| snap.target)
        };
        // On the hill over the bore: before, the bore; now nothing.
        let over_bore = Vector3::new(0.0, 15.0, 1.0);
        assert_eq!(
            get_closest_network_snap_xz(&graph, over_bore, 5.0).map(|snap| snap.target),
            Some(NetworkSnapTarget::Edge(0))
        );
        assert_eq!(snap(over_bore, StructureHit::Ground), None);
        // Beside the bridge on the ground: nothing; on its deck: the bridge.
        assert_eq!(
            snap(Vector3::new(3.0, 0.0, 50.0), StructureHit::Ground),
            None
        );
        assert_eq!(
            snap(Vector3::new(1.0, 6.0, 50.0), StructureHit::Span(1)),
            Some(NetworkSnapTarget::Edge(1))
        );
        // The ground road snaps as before, bitwise, whatever the ray hit.
        let road = Vector3::new(41.0, 0.0, 50.0);
        let before = get_closest_network_snap_xz(&graph, road, 5.0);
        for hit in [StructureHit::Ground, StructureHit::Span(1)] {
            assert_eq!(
                get_closest_network_snap_at_hit(&graph, road, 5.0, hit),
                before
            );
        }
    }

    // `ROAD-99`: under a bridge the cursor snaps to the level its ray hit. A ground road along x
    // and a bridge 6 m over it along z: at the crossing, a cursor on the ground takes the road at
    // its height, a cursor at the deck the bridge at its height; away from the bridge the road.
    #[test]
    fn the_snap_takes_the_level_the_cursor_hit() {
        let mut graph = RegionGraph::new();
        for (class, [a, b]) in [
            (
                EdgeClass::Standard,
                [Vector3::new(-30.0, 0.0, 0.0), Vector3::new(30.0, 0.0, 0.0)],
            ),
            (
                EdgeClass::Bridge,
                [Vector3::new(0.0, 6.0, -30.0), Vector3::new(0.0, 6.0, 30.0)],
            ),
        ] {
            let start = graph.add_node(a, NodeType::Junction);
            let end = graph.add_node(b, NodeType::Junction);
            graph.add_edge(super::super::build_surface_edge(
                start,
                end,
                vec![a, b],
                1,
                1,
                class,
                None,
            ));
        }
        for (cursor, edge, height) in [
            (Vector3::new(0.5, 0.1, 0.5), 0, 0.0),
            (Vector3::new(0.5, 5.9, 0.5), 1, 6.0),
            (Vector3::new(10.0, 5.9, 0.5), 0, 0.0),
        ] {
            let snap = get_closest_network_snap_xz(&graph, cursor, 5.0).unwrap();
            assert_eq!(snap.target, NetworkSnapTarget::Edge(edge), "{cursor:?}");
            assert_eq!(snap.position.y, height, "{cursor:?}");
        }
    }
}
