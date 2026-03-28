//! Side-aware pedestrian routing over the existing road graph.
//!
//! Road edges stay centerline-based in [`crate::simulation::network::graph::RegionGraph`], but
//! pedestrians need to reason about which shoulder they occupy. This module builds a thin virtual
//! graph at query time:
//! - each sidewalk-capable road edge contributes two shoulder lanes (`side = -1` / `+1`)
//! - each explicit `TransitType::Foot` edge contributes one centered lane (`side = 0`)
//! - node-local transfers connect the incident pedestrian arms using their actual world-space
//!   positions, so footpaths can join road shoulders without collapsing everything to the road
//!   centerline

use crate::config::SIDEWALK_WIDTH;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{TransitFlags, TransitType};
use godot::prelude::*;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

const WALK_SPEED_MPS: f32 = 4.0;
const MIN_SEGMENT_LEN: f32 = 0.01;

/// A concrete pedestrian attachment point on the network.
///
/// `edge_idx` selects the incident edge that the pedestrian is attached to at `node`.
/// For sidewalk-capable road edges, `side` is the shoulder relative to the edge's stored
/// start→end orientation: `+1` = left, `-1` = right. For explicit footpaths, `side` is ignored.
/// When `edge_idx` is `None`, the query may start or finish from any pedestrian-capable arm at
/// the node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PedestrianEndpoint {
    /// The graph node containing the attachment point.
    pub node: u32,
    /// The incident edge to anchor to, or `None` to allow any pedestrian-capable arm at `node`.
    pub edge_idx: Option<usize>,
    /// The requested shoulder for road attachments: `+1` left, `-1` right, `0` either.
    pub side: i8,
}

/// One traversed edge in a side-aware pedestrian route.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PedestrianPathStep {
    /// The concrete edge being traversed.
    pub edge_idx: usize,
    /// `true` when travelling from `start_node` to `end_node`, `false` for the reverse.
    pub forward: bool,
    /// The recorded shoulder for road edges: `+1` left, `-1` right, `0` for centered footpaths.
    pub side: i8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum PedestrianArm {
    Road {
        edge_idx: usize,
        at_start: bool,
        side: i8,
    },
    Foot {
        edge_idx: usize,
        at_start: bool,
    },
}

#[derive(Clone, Copy)]
struct QueueState {
    cost: f32,
    arm: PedestrianArm,
}

impl PartialEq for QueueState {
    fn eq(&self, other: &Self) -> bool {
        self.cost == other.cost && self.arm == other.arm
    }
}

impl Eq for QueueState {}

impl Ord for QueueState {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .cost
            .partial_cmp(&self.cost)
            .unwrap_or(Ordering::Equal)
    }
}

impl PartialOrd for QueueState {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Copy)]
struct ArmLink {
    to: PedestrianArm,
    cost: f32,
    dist: f32,
    step: Option<PedestrianPathStep>,
}

/// Finds a side-aware walking path between two concrete pedestrian endpoints.
pub fn find_path(
    graph: &RegionGraph,
    start: PedestrianEndpoint,
    goal: PedestrianEndpoint,
) -> Option<(f32, f32, Vec<PedestrianPathStep>)> {
    let start_arms = endpoint_arms(graph, start);
    let goal_arms = endpoint_arms(graph, goal);
    if start_arms.is_empty() || goal_arms.is_empty() {
        return None;
    }

    let mut heap = BinaryHeap::new();
    let mut best: HashMap<PedestrianArm, (f32, f32)> = HashMap::new();
    let mut prev: HashMap<PedestrianArm, (PedestrianArm, Option<PedestrianPathStep>)> =
        HashMap::new();

    for arm in &start_arms {
        best.insert(*arm, (0.0, 0.0));
        heap.push(QueueState {
            cost: 0.0,
            arm: *arm,
        });
    }

    let mut meeting = None;
    while let Some(state) = heap.pop() {
        let Some(&(known_cost, known_dist)) = best.get(&state.arm) else {
            continue;
        };
        if state.cost > known_cost + 0.0001 {
            continue;
        }
        if goal_arms.contains(&state.arm) {
            meeting = Some((state.arm, known_cost, known_dist));
            break;
        }

        for link in arm_links(graph, state.arm) {
            let next_cost = known_cost + link.cost;
            let next_dist = known_dist + link.dist;
            let should_update = best
                .get(&link.to)
                .map(|&(cost, _)| next_cost + 0.0001 < cost)
                .unwrap_or(true);
            if should_update {
                best.insert(link.to, (next_cost, next_dist));
                prev.insert(link.to, (state.arm, link.step));
                heap.push(QueueState {
                    cost: next_cost,
                    arm: link.to,
                });
            }
        }
    }

    let (mut cursor, total_cost, total_dist) = meeting?;
    let mut steps = Vec::new();
    while let Some(&(from, step)) = prev.get(&cursor) {
        if let Some(step) = step {
            steps.push(step);
        }
        cursor = from;
    }
    steps.reverse();

    Some((total_cost, total_dist, steps))
}

fn endpoint_arms(graph: &RegionGraph, endpoint: PedestrianEndpoint) -> Vec<PedestrianArm> {
    let node = graph.get_valid_node(endpoint.node);
    match endpoint.edge_idx {
        Some(edge_idx) if edge_idx < graph.edges.len() => {
            let edge = &graph.edges[edge_idx];
            if edge.deleted || !edge_supports_pedestrians(edge) {
                return Vec::new();
            }

            let at_start = graph.get_valid_node(edge.start_node) == node;
            let at_end = graph.get_valid_node(edge.end_node) == node;
            if !at_start && !at_end {
                return Vec::new();
            }

            if edge.primary_type == TransitType::Foot {
                vec![PedestrianArm::Foot { edge_idx, at_start }]
            } else {
                let at_start = at_start;
                match endpoint.side {
                    1 | -1 => vec![PedestrianArm::Road {
                        edge_idx,
                        at_start,
                        side: endpoint.side,
                    }],
                    _ => vec![
                        PedestrianArm::Road {
                            edge_idx,
                            at_start,
                            side: -1,
                        },
                        PedestrianArm::Road {
                            edge_idx,
                            at_start,
                            side: 1,
                        },
                    ],
                }
            }
        }
        _ => collect_node_arms(graph, node),
    }
}

fn collect_node_arms(graph: &RegionGraph, node: u32) -> Vec<PedestrianArm> {
    let node = graph.get_valid_node(node);
    let mut arms = Vec::new();
    if (node as usize) >= graph.adjacency.len() {
        return arms;
    }

    for &edge_idx in &graph.adjacency[node as usize] {
        let edge = &graph.edges[edge_idx];
        if edge.deleted || !edge_supports_pedestrians(edge) {
            continue;
        }

        if edge.primary_type == TransitType::Foot {
            if graph.get_valid_node(edge.start_node) == node {
                arms.push(PedestrianArm::Foot {
                    edge_idx,
                    at_start: true,
                });
            }
            if graph.get_valid_node(edge.end_node) == node && edge.end_node != edge.start_node {
                arms.push(PedestrianArm::Foot {
                    edge_idx,
                    at_start: false,
                });
            }
        } else {
            if graph.get_valid_node(edge.start_node) == node {
                arms.push(PedestrianArm::Road {
                    edge_idx,
                    at_start: true,
                    side: -1,
                });
                arms.push(PedestrianArm::Road {
                    edge_idx,
                    at_start: true,
                    side: 1,
                });
            }
            if graph.get_valid_node(edge.end_node) == node && edge.end_node != edge.start_node {
                arms.push(PedestrianArm::Road {
                    edge_idx,
                    at_start: false,
                    side: -1,
                });
                arms.push(PedestrianArm::Road {
                    edge_idx,
                    at_start: false,
                    side: 1,
                });
            }
        }
    }

    arms
}

fn arm_links(graph: &RegionGraph, arm: PedestrianArm) -> Vec<ArmLink> {
    let mut links = Vec::new();

    if let Some(link) = traverse_edge_link(graph, arm) {
        links.push(link);
    }

    let node = arm_node(graph, arm);
    let Some(origin) = arm_position(graph, arm) else {
        return links;
    };
    for other in collect_node_arms(graph, node) {
        if other == arm {
            continue;
        }
        let Some(target) = arm_position(graph, other) else {
            continue;
        };
        let dist = origin.distance_to(target);
        if dist < 0.001 {
            links.push(ArmLink {
                to: other,
                cost: 0.0,
                dist: 0.0,
                step: None,
            });
        } else {
            links.push(ArmLink {
                to: other,
                cost: dist / WALK_SPEED_MPS,
                dist,
                step: None,
            });
        }
    }

    links
}

fn traverse_edge_link(graph: &RegionGraph, arm: PedestrianArm) -> Option<ArmLink> {
    match arm {
        PedestrianArm::Road {
            edge_idx,
            at_start,
            side,
        } => {
            let edge = graph.edges.get(edge_idx)?;
            if !road_supports_sidewalk(edge) {
                return None;
            }
            let dist = edge.physical_length.max(0.0);
            Some(ArmLink {
                to: PedestrianArm::Road {
                    edge_idx,
                    at_start: !at_start,
                    side,
                },
                cost: dist / WALK_SPEED_MPS,
                dist,
                step: Some(PedestrianPathStep {
                    edge_idx,
                    forward: at_start,
                    side,
                }),
            })
        }
        PedestrianArm::Foot { edge_idx, at_start } => {
            let edge = graph.edges.get(edge_idx)?;
            if edge.primary_type != TransitType::Foot
                || (edge.allowed_types & TransitFlags::FOOT == 0)
            {
                return None;
            }
            let dist = edge.physical_length.max(0.0);
            Some(ArmLink {
                to: PedestrianArm::Foot {
                    edge_idx,
                    at_start: !at_start,
                },
                cost: dist / WALK_SPEED_MPS,
                dist,
                step: Some(PedestrianPathStep {
                    edge_idx,
                    forward: at_start,
                    side: 0,
                }),
            })
        }
    }
}

fn arm_node(graph: &RegionGraph, arm: PedestrianArm) -> u32 {
    match arm {
        PedestrianArm::Road {
            edge_idx, at_start, ..
        } => {
            let edge = &graph.edges[edge_idx];
            graph.get_valid_node(if at_start {
                edge.start_node
            } else {
                edge.end_node
            })
        }
        PedestrianArm::Foot { edge_idx, at_start } => {
            let edge = &graph.edges[edge_idx];
            graph.get_valid_node(if at_start {
                edge.start_node
            } else {
                edge.end_node
            })
        }
    }
}

fn arm_position(graph: &RegionGraph, arm: PedestrianArm) -> Option<Vector2> {
    match arm {
        PedestrianArm::Road {
            edge_idx,
            at_start,
            side,
        } => road_arm_position(&graph.edges[edge_idx], at_start, side),
        PedestrianArm::Foot { edge_idx, at_start } => {
            foot_arm_position(&graph.edges[edge_idx], at_start)
        }
    }
}

fn road_arm_position(edge: &Edge, at_start: bool, side: i8) -> Option<Vector2> {
    let points = edge_points(edge);
    let direction = canonical_endpoint_direction(points, at_start)?;
    let endpoint = if at_start {
        points.first().copied()?
    } else {
        points.last().copied()?
    };
    let normal = Vector2::new(-direction.y, direction.x);
    let offset = edge.width.max(2.0) * 0.5 + SIDEWALK_WIDTH * 0.5;
    let point = Vector2::new(endpoint.x, endpoint.z) + normal * offset * side as f32;
    Some(point)
}

fn foot_arm_position(edge: &Edge, at_start: bool) -> Option<Vector2> {
    let points = edge_points(edge);
    let endpoint = if at_start {
        points.first().copied()?
    } else {
        points.last().copied()?
    };
    Some(Vector2::new(endpoint.x, endpoint.z))
}

fn edge_points(edge: &Edge) -> &[Vector3] {
    if edge.geometry.len() >= 2 {
        &edge.geometry
    } else {
        &edge.physical_geometry
    }
}

fn canonical_endpoint_direction(points: &[Vector3], at_start: bool) -> Option<Vector2> {
    if points.len() < 2 {
        return None;
    }

    if at_start {
        let origin = points[0];
        for point in &points[1..] {
            let delta = Vector2::new(point.x - origin.x, point.z - origin.z);
            if delta.length_squared() > MIN_SEGMENT_LEN * MIN_SEGMENT_LEN {
                return Some(delta.normalized());
            }
        }
    } else {
        let origin = *points.last()?;
        for point in points[..points.len() - 1].iter().rev() {
            let delta = Vector2::new(origin.x - point.x, origin.z - point.z);
            if delta.length_squared() > MIN_SEGMENT_LEN * MIN_SEGMENT_LEN {
                return Some(delta.normalized());
            }
        }
    }

    None
}

fn edge_supports_pedestrians(edge: &Edge) -> bool {
    match edge.primary_type {
        TransitType::Road => road_supports_sidewalk(edge),
        TransitType::Foot => (edge.allowed_types & TransitFlags::FOOT) != 0,
        _ => false,
    }
}

fn road_supports_sidewalk(edge: &Edge) -> bool {
    edge.primary_type == TransitType::Road && (edge.allowed_types & TransitFlags::FOOT) != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::types::{EdgeClass, NodeType};

    fn road_edge(n1: u32, n2: u32, p1: Vector3, p2: Vector3) -> Edge {
        Edge {
            start_node: n1,
            end_node: n2,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 8.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 13.0,
            base_cost: 0.0,
            physical_length: (p2 - p1).length(),
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![p1, p2],
            physical_geometry: vec![p1, p2],
            zoning_left: false,
            zoning_right: false,
            deleted: false,
        }
    }

    fn foot_edge(n1: u32, n2: u32, p1: Vector3, p2: Vector3) -> Edge {
        Edge {
            start_node: n1,
            end_node: n2,
            primary_type: TransitType::Foot,
            allowed_types: TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 2.0,
            fwd_lanes: 0,
            bkw_lanes: 0,
            speed_limit: 4.0,
            base_cost: 0.0,
            physical_length: (p2 - p1).length(),
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![p1, p2],
            physical_geometry: vec![p1, p2],
            zoning_left: false,
            zoning_right: false,
            deleted: false,
        }
    }

    #[test]
    fn walkway_to_road_path_picks_a_specific_shoulder() {
        let mut graph = RegionGraph::new();
        let n0 = graph.add_node(Vector3::new(-10.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
        let n3 = graph.add_node(Vector3::new(0.0, 0.0, -10.0), NodeType::Junction);

        let _road_left_idx = graph.add_edge(road_edge(
            n0,
            n1,
            Vector3::new(-10.0, 0.0, 0.0),
            Vector3::ZERO,
        ));
        let road_right_idx = graph.add_edge(road_edge(
            n1,
            n2,
            Vector3::ZERO,
            Vector3::new(10.0, 0.0, 0.0),
        ));
        let foot_idx = graph.add_edge(foot_edge(
            n3,
            n1,
            Vector3::new(0.0, 0.0, -10.0),
            Vector3::ZERO,
        ));
        graph.rebuild_adjacency_list();

        let path = find_path(
            &graph,
            PedestrianEndpoint {
                node: n3,
                edge_idx: Some(foot_idx),
                side: 0,
            },
            PedestrianEndpoint {
                node: n2,
                edge_idx: Some(road_right_idx),
                side: 1,
            },
        )
        .unwrap();

        assert_eq!(path.2.len(), 2);
        assert_eq!(path.2[0].edge_idx, foot_idx);
        assert_eq!(path.2[0].side, 0);
        assert_eq!(path.2[1].edge_idx, road_right_idx);
        assert_eq!(path.2[1].side, 1);
    }

    #[test]
    fn opposite_shoulder_destination_uses_requested_side() {
        let mut graph = RegionGraph::new();
        let n0 = graph.add_node(Vector3::new(0.0, 0.0, -10.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(0.0, 0.0, 10.0), NodeType::Junction);

        let road_idx = graph.add_edge(road_edge(
            n0,
            n1,
            Vector3::new(0.0, 0.0, -10.0),
            Vector3::new(0.0, 0.0, 10.0),
        ));
        graph.rebuild_adjacency_list();

        let left_path = find_path(
            &graph,
            PedestrianEndpoint {
                node: n0,
                edge_idx: Some(road_idx),
                side: 1,
            },
            PedestrianEndpoint {
                node: n1,
                edge_idx: Some(road_idx),
                side: 1,
            },
        )
        .unwrap();
        let right_path = find_path(
            &graph,
            PedestrianEndpoint {
                node: n0,
                edge_idx: Some(road_idx),
                side: -1,
            },
            PedestrianEndpoint {
                node: n1,
                edge_idx: Some(road_idx),
                side: -1,
            },
        )
        .unwrap();

        assert_eq!(
            left_path.2,
            vec![PedestrianPathStep {
                edge_idx: road_idx,
                forward: true,
                side: 1,
            }]
        );
        assert_eq!(
            right_path.2,
            vec![PedestrianPathStep {
                edge_idx: road_idx,
                forward: true,
                side: -1,
            }]
        );
    }
}
