//! Edge input conditioning before preview or committed section compilation.

use super::super::RoadSurfaceSystem;
use crate::config;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::EdgeClass;
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::Vector3;

// Road input conditioning thresholds shared by preview and committed placement.
const ROAD_POINT_SIMPLIFY_DISTANCE_M: f32 = 0.5;
const TAUBIN_SMOOTHING_ITERS: usize = 50;
const TAUBIN_LAMBDA: f32 = 0.5;
const TAUBIN_MU: f32 = -0.53;
pub(in crate::simulation::network::surface::edge) const PREVIEW_CLEARANCE_M: f32 = 1.0;

impl RoadSurfaceSystem {
    /// Grounds standard-road input to terrain and classifies bridge / tunnel previews using the
    /// same threshold as committed placement.
    pub(crate) fn classify_and_ground_road_points(
        raw_points: &[Vector3],
        terrain: &TerrainSystem,
    ) -> (Vec<Vector3>, EdgeClass) {
        Self::classify_and_ground_road_points_with_support(raw_points, terrain, |_, _| None)
    }

    /// Conditions road-tool input against already-compiled visible road support before falling
    /// back to source terrain.
    pub(crate) fn classify_and_ground_road_points_to_visible_surface(
        raw_points: &[Vector3],
        terrain: &TerrainSystem,
        graph: &RegionGraph,
        road_surface: &RoadSurfaceSystem,
    ) -> (Vec<Vector3>, EdgeClass) {
        Self::classify_and_ground_road_points_with_support(raw_points, terrain, |x, z| {
            road_surface.sample_visible_surface_height(graph, terrain, x, z)
        })
    }

    fn classify_and_ground_road_points_with_support(
        raw_points: &[Vector3],
        terrain: &TerrainSystem,
        mut support_height_at: impl FnMut(f32, f32) -> Option<f32>,
    ) -> (Vec<Vector3>, EdgeClass) {
        let mut fixed_points = raw_points.to_vec();
        let mut support_heights = Vec::with_capacity(fixed_points.len());
        let mut all_points_above_clearance = !fixed_points.is_empty();
        let mut all_points_below_clearance = !fixed_points.is_empty();

        for point in &fixed_points {
            let terrain_h = terrain.sample_height_world(point.x, point.z) * config::HEIGHT_SCALE;
            let support_h = support_height_at(point.x, point.z).unwrap_or(terrain_h);
            support_heights.push(support_h);
            let clearance_m = point.y - support_h;
            if clearance_m <= PREVIEW_CLEARANCE_M {
                all_points_above_clearance = false;
            }
            if clearance_m >= -PREVIEW_CLEARANCE_M {
                all_points_below_clearance = false;
            }
        }

        let class = if all_points_above_clearance {
            EdgeClass::Bridge
        } else if all_points_below_clearance {
            EdgeClass::Tunnel
        } else {
            EdgeClass::Standard
        };

        if class == EdgeClass::Standard {
            for (point, support_h) in fixed_points.iter_mut().zip(support_heights) {
                point.y = support_h;
            }
        }

        (fixed_points, class)
    }

    /// Applies the same point simplification threshold used by committed road placement.
    pub(crate) fn simplify_road_input_points(points: &[Vector3]) -> Vec<Vector3> {
        let mut simplified_points = Vec::with_capacity(points.len());
        if !points.is_empty() {
            simplified_points.push(points[0]);
            for point in points.iter().skip(1) {
                if point.distance_to(*simplified_points.last().unwrap())
                    > ROAD_POINT_SIMPLIFY_DISTANCE_M
                {
                    simplified_points.push(*point);
                }
            }
            if simplified_points.len() > 1
                && simplified_points.last().unwrap() != points.last().unwrap()
            {
                simplified_points.pop();
                simplified_points.push(*points.last().unwrap());
            }
        }
        simplified_points
    }

    /// Applies the Taubin height-smoothing pass shared by committed placement and preview.
    pub(crate) fn taubin_smooth_road_heights(points: &mut [Vector3]) {
        if points.len() <= 2 {
            return;
        }

        let mut temp_h = vec![0.0; points.len()];
        for _ in 0..TAUBIN_SMOOTHING_ITERS {
            for index in 1..points.len() - 1 {
                let laplacian = 0.5 * (points[index - 1].y + points[index + 1].y) - points[index].y;
                temp_h[index] = points[index].y + TAUBIN_LAMBDA * laplacian;
            }
            for index in 1..points.len() - 1 {
                points[index].y = temp_h[index];
            }
            for index in 1..points.len() - 1 {
                let laplacian = 0.5 * (points[index - 1].y + points[index + 1].y) - points[index].y;
                temp_h[index] = points[index].y + TAUBIN_MU * laplacian;
            }
            for index in 1..points.len() - 1 {
                points[index].y = temp_h[index];
            }
        }
    }
}
