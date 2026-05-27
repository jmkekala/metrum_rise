//! Deterministic edge-polyline sampling helpers.

use super::super::{
    RoadSurfaceSystem, SAMPLE_EPSILON_M,
    backend::{RoadVec2, RoadVec3},
};

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface::edge) fn sample_polyline(
        &self,
        points: &[RoadVec3],
        cumulative: &[f32],
        s_m: f32,
    ) -> (RoadVec3, RoadVec2) {
        if points.len() == 1 {
            return (points[0], RoadVec2::X);
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
            let local_t = f64::from(((clamped_s - start_s) / segment_length).clamp(0.0, 1.0));
            let point = start.lerp(end, local_t);
            let tangent_xz = self.segment_tangent_xz(points, index);
            return (point, tangent_xz);
        }

        (
            *points.last().unwrap(),
            self.segment_tangent_xz(points, points.len().saturating_sub(2)),
        )
    }

    fn segment_tangent_xz(&self, points: &[RoadVec3], preferred_index: usize) -> RoadVec2 {
        if points.len() < 2 {
            return RoadVec2::X;
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
            let tangent_xz = RoadVec2::new(delta.x, delta.z);
            if tangent_xz.length_squared() > 1e-8 {
                return tangent_xz.normalize();
            }
        }

        for window in points.windows(2) {
            let delta = window[1] - window[0];
            let tangent_xz = RoadVec2::new(delta.x, delta.z);
            if tangent_xz.length_squared() > 1e-8 {
                return tangent_xz.normalize();
            }
        }

        RoadVec2::X
    }

    pub(in crate::simulation::network::surface::edge) fn build_cumulative_distances(
        &self,
        points: &[RoadVec3],
    ) -> Vec<f32> {
        let mut cumulative = Vec::with_capacity(points.len());
        let mut running = 0.0;
        cumulative.push(0.0);
        for segment in points.windows(2) {
            running += segment[0].distance(segment[1]) as f32;
            cumulative.push(running);
        }
        cumulative
    }
}
