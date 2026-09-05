// SPDX-License-Identifier: GPL-2.0-only

//! Span raised-step constraint extraction and vertical face generation.

use super::super::{
    RoadSurfaceSection, RoadSurfaceSystem, RoadSurfaceVisualPolygon, SAMPLE_EPSILON_M,
    backend::RoadVec3,
};
use super::{
    RoadSurfaceSpanBandOwner, RoadSurfaceSpanRaisedStepSource, SpanRaisedStepConstraint,
    SpanRaisedStepSample, SpanResolvedRaisedStepSample,
};

impl RoadSurfaceSystem {
    pub(super) fn sort_span_raised_step_faces(
        faces: &mut [(RoadSurfaceVisualPolygon, RoadSurfaceSpanRaisedStepSource)],
    ) {
        faces.sort_by(|(polygon_a, source_a), (polygon_b, source_b)| {
            source_a
                .lower_owner
                .sort_key()
                .cmp(&source_b.lower_owner.sort_key())
                .then(
                    source_a
                        .raised_owner
                        .sort_key()
                        .cmp(&source_b.raised_owner.sort_key()),
                )
                .then(
                    source_a
                        .start_section_index
                        .cmp(&source_b.start_section_index),
                )
                .then(source_a.end_section_index.cmp(&source_b.end_section_index))
                .then(source_a.start_s_m.total_cmp(&source_b.start_s_m))
                .then(source_a.end_s_m.total_cmp(&source_b.end_s_m))
                .then_with(|| Self::visual_polygon_ordering(polygon_a, polygon_b))
        });
    }

    pub(super) fn span_raised_step_constraints_for_resolved_segment(
        pair: &[RoadSurfaceSection],
        start_section_index: usize,
        end_section_index: usize,
    ) -> Vec<SpanRaisedStepConstraint> {
        if pair.len() != 2 {
            return Vec::new();
        }
        debug_assert_eq!(
            pair[0].bands.len(),
            pair[1].bands.len(),
            "span region resolution rejects mismatched section profiles before raised-step extraction"
        );

        let mut constraints = Vec::new();
        for boundary_index in 0..pair[0].bands.len().saturating_sub(1) {
            let Some(start) = Self::span_raised_step_sample(&pair[0], boundary_index) else {
                continue;
            };
            let Some(end) = Self::span_raised_step_sample(&pair[1], boundary_index) else {
                continue;
            };
            if start.lower_owner != end.lower_owner || start.raised_owner != end.raised_owner {
                continue;
            }
            constraints.push(SpanRaisedStepConstraint {
                lower_owner: start.lower_owner,
                raised_owner: start.raised_owner,
                start_section_index,
                end_section_index,
                start_s_m: pair[0].s_m,
                end_s_m: pair[1].s_m,
                start: start.sample,
                end: end.sample,
            });
        }
        constraints
    }

    fn span_raised_step_sample(
        section: &RoadSurfaceSection,
        boundary_index: usize,
    ) -> Option<SpanResolvedRaisedStepSample> {
        let lower_index = boundary_index;
        let upper_index = boundary_index + 1;
        let left = section.bands.get(lower_index)?;
        let right = section.bands.get(upper_index)?;
        if left.lateral_end_m != right.lateral_start_m {
            return None;
        }

        let boundary_lateral_m = left.lateral_end_m;
        let left_mid_lateral_m = (left.lateral_start_m + left.lateral_end_m) * 0.5;
        let right_mid_lateral_m = (right.lateral_start_m + right.lateral_end_m) * 0.5;
        if left.kind == right.kind {
            return None;
        }
        if (left.height_end_m - right.height_start_m).abs() <= SAMPLE_EPSILON_M {
            return None;
        }

        let left_owner = RoadSurfaceSpanBandOwner {
            source_band_index: lower_index,
            kind: left.kind,
        };
        let right_owner = RoadSurfaceSpanBandOwner {
            source_band_index: upper_index,
            kind: right.kind,
        };
        let (lower_owner, raised_owner, lower_height_m, raised_height_m, lower_mid_lateral_m) =
            if left.height_end_m < right.height_start_m {
                (
                    left_owner,
                    right_owner,
                    left.height_end_m,
                    right.height_start_m,
                    left_mid_lateral_m,
                )
            } else {
                (
                    right_owner,
                    left_owner,
                    right.height_start_m,
                    left.height_end_m,
                    right_mid_lateral_m,
                )
            };
        if raised_height_m <= lower_height_m {
            return None;
        }

        let lower_world =
            Self::section_boundary_world_point_static(section, boundary_lateral_m, lower_height_m);
        let raised_world =
            Self::section_boundary_world_point_static(section, boundary_lateral_m, raised_height_m);
        let lower_direction_xz =
            section.lateral_xz * f64::from(lower_mid_lateral_m - boundary_lateral_m);

        Some(SpanResolvedRaisedStepSample {
            lower_owner,
            raised_owner,
            sample: SpanRaisedStepSample {
                lower_world,
                raised_world,
                lower_direction: RoadVec3::new(lower_direction_xz.x, 0.0, lower_direction_xz.y),
            },
        })
    }

    pub(super) fn span_raised_step_faces_from_constraints(
        constraints: &[SpanRaisedStepConstraint],
    ) -> Vec<(RoadSurfaceVisualPolygon, RoadSurfaceSpanRaisedStepSource)> {
        constraints
            .iter()
            .filter_map(Self::span_raised_step_face_from_constraint)
            .collect()
    }

    fn span_raised_step_face_from_constraint(
        constraint: &SpanRaisedStepConstraint,
    ) -> Option<(RoadSurfaceVisualPolygon, RoadSurfaceSpanRaisedStepSource)> {
        let mut points = [
            constraint.start.raised_world,
            constraint.start.lower_world,
            constraint.end.lower_world,
            constraint.end.raised_world,
        ];
        let lower_direction = constraint.start.lower_direction + constraint.end.lower_direction;
        let face_normal = (points[1] - points[0]).cross(points[2] - points[0]);
        if face_normal.dot(lower_direction) > 0.0 {
            points = [points[3], points[2], points[1], points[0]];
        }

        let polygon = Self::make_vertical_quad_polygon(points)?;
        let source = RoadSurfaceSpanRaisedStepSource {
            lower_owner: constraint.lower_owner,
            raised_owner: constraint.raised_owner,
            start_section_index: constraint.start_section_index,
            end_section_index: constraint.end_section_index,
            start_s_m: constraint.start_s_m,
            end_s_m: constraint.end_s_m,
            start_lower_world: constraint.start.lower_world,
            start_raised_world: constraint.start.raised_world,
            end_lower_world: constraint.end.lower_world,
            end_raised_world: constraint.end.raised_world,
        };
        Some((polygon, source))
    }
}
