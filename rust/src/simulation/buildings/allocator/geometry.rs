//! Geometry helpers for sampling world-space positions along road edges.

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::lanes::Lane;
use godot::prelude::{Vector2, Vector3};

impl BuildingAllocator {
    /// Returns the world-space (X, Z) position at fractional distance `t` along an edge.
    pub fn get_pos_on_edge(&self, graph: &RegionGraph, edge_idx: usize, t: f32) -> Vector2 {
        Self::sample_pos_on_edge(graph, edge_idx, t)
    }

    pub(crate) fn sample_pos_on_edge(graph: &RegionGraph, edge_idx: usize, t: f32) -> Vector2 {
        let edge = graph.edge(edge_idx);
        Self::sample_pos_on_polyline(
            &edge.physical_geometry,
            edge.physical_length,
            t.clamp(0.0, 1.0) * edge.physical_length,
        )
    }

    /// Returns the tangent vector (X, Z) at fractional distance `t` along an edge.
    pub fn get_tangent_on_edge(&self, graph: &RegionGraph, edge_idx: usize, t: f32) -> Vector2 {
        Self::sample_tangent_on_edge(graph, edge_idx, t)
    }

    pub(crate) fn sample_tangent_on_edge(graph: &RegionGraph, edge_idx: usize, t: f32) -> Vector2 {
        let edge = graph.edge(edge_idx);
        Self::sample_tangent_on_polyline(
            &edge.physical_geometry,
            edge.physical_length,
            t.clamp(0.0, 1.0) * edge.physical_length,
        )
    }

    pub(crate) fn sample_pos_on_lane(lane: &Lane, lane_d: f32) -> Vector2 {
        Self::sample_pos_on_polyline(&lane.geometry, lane.length, lane_d.clamp(0.0, lane.length))
    }

    pub(crate) fn project_point_to_polyline_s(points: &[Vector3], world_pos: Vector2) -> f32 {
        if points.len() < 2 {
            return 0.0;
        }

        let mut acc_len = 0.0;
        let mut best_dist2 = f32::INFINITY;
        let mut best_idx = usize::MAX;
        let mut best_t = f32::INFINITY;
        let mut best_s = 0.0;

        for i in 0..points.len() - 1 {
            let p0 = Vector2::new(points[i].x, points[i].z);
            let p1 = Vector2::new(points[i + 1].x, points[i + 1].z);
            let seg = p1 - p0;
            let seg_len2 = seg.length_squared();
            let local_t = if seg_len2 > 1e-12 {
                ((world_pos - p0).dot(seg) / seg_len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let closest = p0 + seg * local_t;
            let dist2 = (world_pos - closest).length_squared();
            let seg_len = points[i].distance_to(points[i + 1]);

            let better = dist2 < best_dist2
                || (dist2 == best_dist2 && (i < best_idx || (i == best_idx && local_t < best_t)));
            if better {
                best_dist2 = dist2;
                best_idx = i;
                best_t = local_t;
                best_s = acc_len + seg_len * local_t;
            }
            acc_len += seg_len;
        }

        best_s
    }

    pub(crate) fn sample_pos_on_polyline(points: &[Vector3], total_len: f32, s_m: f32) -> Vector2 {
        if points.is_empty() {
            return Vector2::ZERO;
        }
        if points.len() == 1 || total_len <= 1e-6 {
            return Vector2::new(points[0].x, points[0].z);
        }

        let target_s = s_m.clamp(0.0, total_len);
        let mut acc_len = 0.0;

        for i in 0..points.len() - 1 {
            let seg_len = points[i].distance_to(points[i + 1]);
            if seg_len <= 1e-6 {
                continue;
            }
            if acc_len + seg_len >= target_s {
                let local_t = ((target_s - acc_len) / seg_len).clamp(0.0, 1.0);
                let p0 = Vector2::new(points[i].x, points[i].z);
                let p1 = Vector2::new(points[i + 1].x, points[i + 1].z);
                return p0.lerp(p1, local_t);
            }
            acc_len += seg_len;
        }

        Vector2::new(points.last().unwrap().x, points.last().unwrap().z)
    }

    pub(crate) fn sample_tangent_on_polyline(
        points: &[Vector3],
        total_len: f32,
        s_m: f32,
    ) -> Vector2 {
        if points.len() <= 1 || total_len <= 1e-6 {
            return Vector2::RIGHT;
        }

        let target_s = s_m.clamp(0.0, total_len);
        let mut acc_len = 0.0;

        for i in 0..points.len() - 1 {
            let seg_len = points[i].distance_to(points[i + 1]);
            if seg_len <= 1e-6 {
                continue;
            }
            let next_acc = acc_len + seg_len;
            if next_acc >= target_s {
                let projected =
                    Vector2::new(points[i + 1].x - points[i].x, points[i + 1].z - points[i].z);
                let on_start_vertex = target_s == acc_len;
                let on_end_vertex = target_s == next_acc;
                if !on_start_vertex && !on_end_vertex && projected.length_squared() > 1e-12 {
                    return projected.normalized();
                }

                let last_seg = points.len() - 2;
                let forward_start = if on_end_vertex { i + 1 } else { i };
                if forward_start <= last_seg {
                    if let Some(tangent) = first_projected_tangent(points, forward_start, last_seg)
                    {
                        return tangent;
                    }
                }
                let backward_end = if on_start_vertex {
                    i.saturating_sub(1)
                } else {
                    i
                };
                if let Some(tangent) = last_projected_tangent(points, backward_end) {
                    return tangent;
                }
                return Vector2::RIGHT;
            }
            acc_len = next_acc;
        }

        if let Some(tangent) = last_projected_tangent(points, points.len() - 2) {
            tangent
        } else {
            Vector2::RIGHT
        }
    }
}

fn first_projected_tangent(
    points: &[Vector3],
    start_seg: usize,
    end_seg: usize,
) -> Option<Vector2> {
    for i in start_seg..=end_seg {
        let seg_len = points[i].distance_to(points[i + 1]);
        let projected = Vector2::new(points[i + 1].x - points[i].x, points[i + 1].z - points[i].z);
        if seg_len > 1e-6 && projected.length_squared() > 1e-12 {
            return Some(projected.normalized());
        }
    }
    None
}

fn last_projected_tangent(points: &[Vector3], end_seg: usize) -> Option<Vector2> {
    for i in (0..=end_seg).rev() {
        let seg_len = points[i].distance_to(points[i + 1]);
        let projected = Vector2::new(points[i + 1].x - points[i].x, points[i + 1].z - points[i].z);
        if seg_len > 1e-6 && projected.length_squared() > 1e-12 {
            return Some(projected.normalized());
        }
    }
    None
}
