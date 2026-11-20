// SPDX-License-Identifier: GPL-3.0-or-later

//! Road-network spatial queries (closest-point identification, edge projection).

use crate::config::DEFAULT_ZONING_DEPTH;
use crate::nodes::sim::core::SimCore;
use crate::simulation::network::interaction;
use crate::simulation::network::roadbed::PieceId;
use godot::prelude::*;

// How far in front of the road a ray may meet the visible surface (terrain, a site) and still
// pick the road: the graded surface carries the roadbed's own top, so a visible road's two hits
// agree to rounding.
const ROAD_PICK_OCCLUSION_M: f32 = 0.25;

/// What the select tool's ray points at on the road network ([`SimCore::pick_road_piece_internal`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RoadPick {
    /// The road whose surface the ray hit (its span, or the bend or end piece of a node with
    /// fewer than three roads).
    pub edge: Option<usize>,
    /// The junction (three or more roads) whose junction piece the ray hit.
    pub junction: Option<u32>,
}

impl SimCore {
    /// Returns the ID of the edge closest to the given world position.
    pub fn get_hovered_edge_internal(&self, world_x: f32, world_z: f32) -> i32 {
        let max_depth = (DEFAULT_ZONING_DEPTH as f32 * self.config.zone_cell_m) + 10.0;
        interaction::get_closest_edge_xz(&self.region_graph, world_x, world_z, max_depth)
            .map_or(-1, |id| id as i32)
    }

    /// Returns the ID of the closest network node.
    pub fn get_closest_node_internal(&self, world_pos: Vector3, max_dist: f32) -> i32 {
        interaction::get_closest_node_xz(&self.region_graph, world_pos, max_dist)
            .map_or(-1, |id| id as i32)
    }

    /// The junctions (live nodes where three or more roads meet) inside the XZ rectangle `min` to
    /// `max`, in ascending node id order, with their positions. The work follows the node grid
    /// cells the rectangle covers and each junction's own roads, never the whole network.
    pub fn junction_markers_internal(&self, min: Vector3, max: Vector3) -> Vec<(u32, Vector3)> {
        let graph = &self.region_graph;
        let mut markers = Vec::new();
        for node_id in graph.get_nodes_near_aabb(min, max) {
            let pos = graph.node(node_id).pos;
            if pos.x < min.x || pos.x > max.x || pos.z < min.z || pos.z > max.z {
                continue;
            }
            if self.is_junction(node_id) {
                markers.push((node_id, pos));
            }
        }
        markers
    }

    /// The road piece a ray from `origin` along `direction` meets first on the visible surface:
    /// a junction's piece picks the junction, a span or a lesser node's piece picks its road;
    /// nothing when it meets terrain (or a site) first or misses the roads. Exact, never the
    /// nearest road: the roadbed's own ray walk over its triangle grid, `O(cells along the ray ×
    /// triangles per cell)`, plus a bend's arms.
    pub fn pick_road_piece_internal(&mut self, origin: Vector3, direction: Vector3) -> RoadPick {
        let Some((road_hit, piece)) = self
            .transit_network
            .roadbed
            .raycast_piece(origin, direction)
        else {
            return RoadPick::default();
        };
        self.prepare_site_queries();
        if let Some(surface_hit) = self.intersect_world_surface_internal(origin, direction)
            && surface_hit.distance_to(origin) + ROAD_PICK_OCCLUSION_M
                < road_hit.distance_to(origin)
        {
            return RoadPick::default();
        }
        let graph = &self.region_graph;
        match piece {
            PieceId::Span(edge) if !graph.edge(edge).deleted => RoadPick {
                edge: Some(edge),
                junction: None,
            },
            PieceId::Span(_) => RoadPick::default(),
            PieceId::Node(node) if self.is_junction(node) => RoadPick {
                edge: None,
                junction: Some(node),
            },
            PieceId::Node(node) => RoadPick {
                edge: self.nearest_arm(node, road_hit),
                junction: None,
            },
        }
    }

    /// The boundary of junction `node`'s roadbed piece, closed across its mouths: its kerb
    /// chains in turn order around the node, each mouth a straight step from one chain's end to
    /// the next one's start. Empty when the node has no published piece. `O(piece boundary)`.
    pub fn junction_outline_internal(&self, node: u32) -> Vec<Vector3> {
        let Some(mesh) = self.transit_network.roadbed.piece_mesh(PieceId::Node(node)) else {
            return Vec::new();
        };
        let centre = self.region_graph.node(node).pos;
        let positions = mesh.positions();
        let point = |index: u32| {
            let p = positions[index as usize];
            Vector3::new(p.x as f32, p.y as f32, p.z as f32)
        };
        let mut chains: Vec<_> = mesh
            .outer_chains()
            .iter()
            .filter(|chain| !chain.vertices().is_empty())
            .map(|chain| {
                let first = point(chain.vertices()[0]);
                ((first.z - centre.z).atan2(first.x - centre.x), chain)
            })
            .collect();
        chains.sort_by(|a, b| a.0.total_cmp(&b.0));
        chains
            .iter()
            .flat_map(|(_, chain)| chain.vertices().iter().map(|&index| point(index)))
            .collect()
    }

    // A node where three or more live roads meet: one the Junction window edits.
    fn is_junction(&self, node: u32) -> bool {
        let graph = &self.region_graph;
        graph.get_valid_node(node) == node && graph.live_node_connection_count(node) >= 3
    }

    // The road of a bend or end node's piece at `point`: the live arm whose centreline passes
    // nearest. `O(arms × arm points)`.
    fn nearest_arm(&self, node: u32, point: Vector3) -> Option<usize> {
        let graph = &self.region_graph;
        if (node as usize) >= graph.node_adjacency_count() {
            return None;
        }
        let at = Vector2::new(point.x, point.z);
        let distance = |edge: usize| {
            graph
                .edge(edge)
                .physical_geometry
                .windows(2)
                .map(|span| {
                    let a = Vector2::new(span[0].x, span[0].z);
                    let b = Vector2::new(span[1].x, span[1].z);
                    let ab = b - a;
                    let t = if ab.length_squared() > 0.0 {
                        ((at - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    (a + ab * t).distance_squared_to(at)
                })
                .fold(f32::MAX, f32::min)
        };
        graph
            .node_adjacency(node)
            .iter()
            .copied()
            .filter(|&edge| !graph.edge(edge).deleted)
            .map(|edge| (distance(edge), edge))
            .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
            .map(|(_, edge)| edge)
    }

    /// Returns the number of non-deleted road connections for a node.
    pub fn get_node_connection_count_internal(&self, node_id: i32) -> i32 {
        if node_id < 0 || node_id as usize >= self.region_graph.node_count() {
            return 0;
        }
        self.region_graph.live_node_connection_count(node_id as u32) as i32
    }

    /// Returns all live junction node positions.
    pub fn get_network_nodes_internal(&self) -> PackedVector3Array {
        let mut arr = PackedVector3Array::new();
        for (i, node) in self.region_graph.nodes().iter().enumerate() {
            if self.region_graph.is_live_canonical_node(i as u32) {
                arr.push(node.pos);
            }
        }
        arr
    }

    /// Returns the world-space position of a node.
    pub fn get_node_pos_internal(&self, node_id: u32) -> Vector3 {
        let valid_id = self.region_graph.get_valid_node(node_id);
        if (valid_id as usize) < self.region_graph.node_count() {
            self.region_graph.node(valid_id).pos
        } else {
            Vector3::ZERO
        }
    }

    /// Returns the world-space positions of all active border (external connection) nodes.
    pub fn get_border_nodes_internal(&self) -> PackedFloat32Array {
        let mut arr = PackedFloat32Array::new();
        for (i, node) in self.region_graph.nodes().iter().enumerate() {
            if node.node_type != crate::simulation::network::types::NodeType::Border {
                continue;
            }
            if !self.region_graph.is_live_canonical_node(i as u32) {
                continue;
            }
            arr.push(node.pos.x);
            arr.push(node.pos.y);
            arr.push(node.pos.z);
        }
        arr
    }
}
