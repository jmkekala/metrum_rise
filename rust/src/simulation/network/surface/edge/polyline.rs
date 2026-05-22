//! Deterministic edge-polyline sampling helpers.

use super::super::{RoadSurfaceSystem, SAMPLE_EPSILON_M};
use godot::prelude::{Vector2, Vector3};

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface::edge) fn sample_polyline(
        &self,
        points: &[Vector3],
        cumulative: &[f32],
        s_m: f32,
    ) -> (Vector3, Vector2) {
        if points.len() == 1 {
            return (points[0], Vector2::RIGHT);
        }

        let total_length = cumulative.last().copied().unwrap_or(0.0);
        let clamped_s = s_m.clamp(0.0, total_length);

        for index in 0..points.len() - 1 {
            let start_s = cumulative[index];
            let end_s = cumulative[index + 1];
            if clamped_s > end_s && index + 2 < points.len() {
                continue;
            }

            let start = points[index];
            let end = points[index + 1];
            let segment_length = (end_s - start_s).max(SAMPLE_EPSILON_M);
            let local_t = ((clamped_s - start_s) / segment_length).clamp(0.0, 1.0);
            let point = start.lerp(end, local_t);
            let tangent_xz = self.segment_tangent_xz(points, index);
            return (point, tangent_xz);
        }

        (
            *points.last().unwrap(),
            self.segment_tangent_xz(points, points.len().saturating_sub(2)),
        )
    }

    fn segment_tangent_xz(&self, points: &[Vector3], preferred_index: usize) -> Vector2 {
        if points.len() < 2 {
            return Vector2::RIGHT;
        }

        let mut candidates = Vec::new();
        candidates.push(preferred_index.min(points.len() - 2));
        if preferred_index > 0 {
            candidates.push(preferred_index - 1);
        }
        if preferred_index + 1 < points.len() - 1 {
            candidates.push(preferred_index + 1);
        }

        for index in candidates {
            let delta = points[index + 1] - points[index];
            let tangent_xz = Vector2::new(delta.x, delta.z);
            if tangent_xz.length_squared() > 1e-8 {
                return tangent_xz.normalized();
            }
        }

        for window in points.windows(2) {
            let delta = window[1] - window[0];
            let tangent_xz = Vector2::new(delta.x, delta.z);
            if tangent_xz.length_squared() > 1e-8 {
                return tangent_xz.normalized();
            }
        }

        Vector2::RIGHT
    }

    pub(in crate::simulation::network::surface::edge) fn build_cumulative_distances(
        &self,
        points: &[Vector3],
    ) -> Vec<f32> {
        let mut cumulative = Vec::with_capacity(points.len());
        let mut running = 0.0;
        cumulative.push(0.0);
        for segment in points.windows(2) {
            running += segment[0].distance_to(segment[1]);
            cumulative.push(running);
        }
        cumulative
    }
}
