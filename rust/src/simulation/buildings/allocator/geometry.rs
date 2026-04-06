//! Geometry helpers for sampling world-space positions along road edges.

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::graph::RegionGraph;
use godot::prelude::Vector2;

impl BuildingAllocator {
    /// Returns the world-space (X, Z) position at fractional distance `t` along an edge.
    pub fn get_pos_on_edge(&self, graph: &RegionGraph, edge_idx: usize, t: f32) -> Vector2 {
        Self::sample_pos_on_edge(graph, edge_idx, t)
    }

    pub(super) fn sample_pos_on_edge(graph: &RegionGraph, edge_idx: usize, t: f32) -> Vector2 {
        let edge = graph.edge(edge_idx);
        let geo = &edge.physical_geometry;
        if geo.is_empty() {
            return Vector2::ZERO;
        }

        let target_dist = t * edge.physical_length;
        let mut curr_dist = 0.0;

        for i in 0..geo.len() - 1 {
            let p1 = Vector2::new(geo[i].x, geo[i].z);
            let p2 = Vector2::new(geo[i + 1].x, geo[i + 1].z);
            let d = (p2 - p1).length();
            if curr_dist + d >= target_dist {
                let local_t = (target_dist - curr_dist) / d;
                return p1 + (p2 - p1) * local_t;
            }
            curr_dist += d;
        }
        Vector2::new(geo.last().unwrap().x, geo.last().unwrap().z)
    }

    /// Returns the tangent vector (X, Z) at fractional distance `t` along an edge.
    pub fn get_tangent_on_edge(&self, graph: &RegionGraph, edge_idx: usize, t: f32) -> Vector2 {
        Self::sample_tangent_on_edge(graph, edge_idx, t)
    }

    pub(super) fn sample_tangent_on_edge(graph: &RegionGraph, edge_idx: usize, t: f32) -> Vector2 {
        let edge = graph.edge(edge_idx);
        let geo = &edge.physical_geometry;
        if geo.len() < 2 {
            return Vector2::new(1.0, 0.0);
        }

        let target_dist = t * edge.physical_length;
        let mut curr_dist = 0.0;
        for i in 0..geo.len() - 1 {
            let p1 = Vector2::new(geo[i].x, geo[i].z);
            let p2 = Vector2::new(geo[i + 1].x, geo[i + 1].z);
            let dist = p2 - p1;
            let d = dist.length();
            if curr_dist + d >= target_dist {
                return if d > 1e-6 {
                    dist.normalized()
                } else {
                    Vector2::new(1.0, 0.0)
                };
            }
            curr_dist += d;
        }
        let p_end = Vector2::new(geo.last().unwrap().x, geo.last().unwrap().z);
        let p_prev = Vector2::new(geo[geo.len() - 2].x, geo[geo.len() - 2].z);
        let dist = p_end - p_prev;
        if dist.length() > 1e-6 {
            dist.normalized()
        } else {
            Vector2::new(1.0, 0.0)
        }
    }
}
