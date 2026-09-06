// SPDX-License-Identifier: GPL-2.0-only

//! Deterministic edge-polyline sampling helpers.

use super::super::{
    RoadSurfaceSystem, SAMPLE_EPSILON_M,
    backend::{RoadVec2, RoadVec3},
};

/// Span framing is continuous between the exact cross-sections owned by its endpoint nodes.
pub(super) struct SpanTangentFrame {
    /// Start mouth station along the stored 3D polyline.
    pub(super) start_s_m: f32,
    /// End mouth station along the stored 3D polyline.
    pub(super) end_s_m: f32,
    /// Exact start-mouth direction supplied to both span and node compilers.
    pub(super) start_tangent: RoadVec2,
    /// Exact end-mouth direction supplied to both span and node compilers.
    pub(super) end_tangent: RoadVec2,
}

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface::edge) fn sample_polyline(
        &self,
        points: &[RoadVec3],
        cumulative: &[f32],
        s_m: f32,
        span_frame: Option<&SpanTangentFrame>,
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
            let Some(frame) =
                span_frame.filter(|frame| clamped_s > frame.start_s_m && clamped_s < frame.end_s_m)
            else {
                return (point, self.segment_tangent_xz(points, index));
            };
            // Node-owned rails and exact mouths retain their source directions. Within the
            // span, interpolate centered secants at every original or inserted section.
            // A one-sided segment tangent can fold offset bands across a centimetre interval.
            let (frame_start_s, start_tangent) = if start_s <= frame.start_s_m {
                (frame.start_s_m, frame.start_tangent)
            } else {
                (start_s, self.vertex_tangent_xz(points, index))
            };
            let (frame_end_s, end_tangent) = if end_s >= frame.end_s_m {
                (frame.end_s_m, frame.end_tangent)
            } else {
                (end_s, self.vertex_tangent_xz(points, index + 1))
            };
            let frame_t = f64::from(
                (clamped_s - frame_start_s) / (frame_end_s - frame_start_s).max(SAMPLE_EPSILON_M),
            );
            let tangent = start_tangent.lerp(end_tangent, frame_t);
            let tangent_xz = if tangent.length_squared() > 1e-8 {
                tangent.normalize()
            } else {
                self.segment_tangent_xz(points, index)
            };
            return (point, tangent_xz);
        }

        (
            *points.last().unwrap(),
            self.segment_tangent_xz(points, points.len().saturating_sub(2)),
        )
    }

    fn vertex_tangent_xz(&self, points: &[RoadVec3], index: usize) -> RoadVec2 {
        if index == 0 {
            return self.segment_tangent_xz(points, 0);
        }
        if index + 1 >= points.len() {
            return self.segment_tangent_xz(points, index - 1);
        }
        // Preserve segment lengths before normalization: a centimetre profile interval must
        // not amplify float-coordinate roundoff as much as its metre-long neighbours.
        let delta = points[index + 1] - points[index - 1];
        let tangent = RoadVec2::new(delta.x, delta.z);
        if tangent.length_squared() > 1e-8 {
            tangent.normalize()
        } else {
            self.segment_tangent_xz(points, index)
        }
    }

    fn segment_tangent_xz(&self, points: &[RoadVec3], preferred_index: usize) -> RoadVec2 {
        if points.len() < 2 {
            return RoadVec2::X;
        }

        let candidates = [
            Some(preferred_index.min(points.len() - 2)),
            preferred_index.checked_sub(1),
            (preferred_index + 1 < points.len() - 1).then_some(preferred_index + 1),
        ];
        for index in candidates.into_iter().flatten() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserted_samples_keep_short_profile_interval_bands_unfolded() {
        let surface = RoadSurfaceSystem::new(512.0);
        // Logged junction profile: a 13 cm vertical interval has only 1.6 cm of plan run.
        let points = [
            RoadVec3::new(2654.783, 96.00182, -8441.406),
            RoadVec3::new(2649.0454, 96.112976, -8443.134),
            RoadVec3::new(2649.03, 95.98347, -8443.139),
            RoadVec3::new(2647.115, 96.19704, -8443.715),
        ];
        let cumulative = surface.build_cumulative_distances(&points);
        let frame = SpanTangentFrame {
            start_s_m: 0.0,
            end_s_m: cumulative[3],
            start_tangent: surface.sample_polyline(&points, &cumulative, 0.0, None).1,
            end_tangent: surface
                .sample_polyline(&points, &cumulative, cumulative[3], None)
                .1,
        };
        let section = |fraction: f32| {
            let s = cumulative[1] + (cumulative[2] - cumulative[1]) * fraction;
            let (center, tangent) = surface.sample_polyline(&points, &cumulative, s, Some(&frame));
            let lateral = RoadVec3::new(-tangent.y, 0.0, tangent.x) * 3.5;
            [center - lateral, center, center + lateral]
        };
        let mut previous = section(0.0);
        for i in 1..=16 {
            let next = section(i as f32 / 16.0);
            for band in 0..2 {
                let corners = [
                    previous[band],
                    next[band],
                    next[band + 1],
                    previous[band + 1],
                ];
                assert!(
                    RoadSurfaceSystem::make_boundary_loop_polygon(corners.to_vec()).is_some(),
                    "inserted sample {i} folds band {band}"
                );
            }
            previous = next;
        }
    }
}
