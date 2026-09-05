// SPDX-License-Identifier: GPL-2.0-only

//! Edge traversal cost calculation for the pathfinding system.
//!
//! Cost is expressed in **time** (seconds), not distance, so that roads with
//! higher speed limits are correctly preferred over shorter but slower roads.
//! A slope penalty multiplier is applied on top of the time cost for steep grades.

use crate::simulation::network::graph::Edge;
use godot::prelude::Vector3;

use crate::config::MIN_ROUTE_SPEED_MS;

/// Stateless helper that computes traversal costs for a single [`Edge`].
pub struct CostCalculator;

impl CostCalculator {
    /// Calculates the baseline intrinsic cost of traversing an edge.
    /// Returns (base_cost, physical_length)
    pub fn calculate_costs(edge: &Edge) -> (f32, f32) {
        let mut total_distance = 0.0;
        let mut max_slope = 0.0_f32;

        let points = if edge.physical_geometry.len() >= 2 {
            &edge.physical_geometry
        } else {
            &edge.geometry
        };
        if points.len() < 2 {
            return (0.0, 0.0);
        }

        for i in 0..points.len() - 1 {
            let p1 = points[i];
            let p2 = points[i + 1];

            let dist = p1.distance_to(p2);
            total_distance += dist;

            if dist > 0.01 {
                let d_y = (p2.y - p1.y).abs();
                let d_xz =
                    (Vector3::new(p1.x, 0.0, p1.z)).distance_to(Vector3::new(p2.x, 0.0, p2.z));
                if d_xz > 0.001 {
                    let slope = d_y / d_xz;
                    if slope > max_slope {
                        max_slope = slope;
                    }
                }
            }
        }

        // Base time cost
        let speed = edge.speed_limit.max(MIN_ROUTE_SPEED_MS);
        let time_cost = total_distance / speed;

        // Exponential penalty for slopes above 10% (0.1)
        // A 41% grade will heavily penalize this route for heavy vehicles
        let slope_penalty = if max_slope > 0.1 {
            1.0 + (max_slope * 5.0).powi(2)
        } else {
            1.0
        };

        (time_cost * slope_penalty, total_distance)
    }
}
