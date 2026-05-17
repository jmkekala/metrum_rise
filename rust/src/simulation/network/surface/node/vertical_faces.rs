//! Raised-step vertical face construction from canonical arrangement intervals.

use super::arrangement_faces::*;
use super::boundary_edges::*;
use super::*;

impl RoadSurfaceSystem {
    pub(super) fn raised_step_face_polygons_from_arrangement(
        arrangement: &NodeArrangement,
        explicit_vertical_step_segments: &[NodeExplicitVerticalStepSegment],
    ) -> Vec<(RoadSurfaceVisualPolygon, RoadSurfaceVerticalFaceSource)> {
        let mut emitted = BTreeSet::new();
        let mut faces = Vec::new();
        for (step_index, segment) in explicit_vertical_step_segments.iter().copied().enumerate() {
            let Some((lower_owner, raised_owner)) =
                canonical_vertical_step_lower_and_raised_owners(segment)
            else {
                continue;
            };
            let segment_key = (segment.start(), segment.end());
            let lower_intervals = arrangement_owner_face_boundary_intervals_for_segment(
                arrangement,
                lower_owner,
                segment_key,
            );
            let raised_intervals = arrangement_owner_face_boundary_intervals_for_segment(
                arrangement,
                raised_owner,
                segment_key,
            );
            let shared_intervals =
                arrangement_shared_face_boundary_intervals(&lower_intervals, &raised_intervals);
            Self::push_arrangement_vertical_step_faces_from_intervals(
                segment_key,
                segment_key,
                shared_intervals,
                step_index,
                segment,
                &mut emitted,
                &mut faces,
            );
        }
        faces
    }

    fn push_arrangement_vertical_step_faces_from_intervals(
        lower_segment_key: (NodeArrangementKey, NodeArrangementKey),
        raised_segment_key: (NodeArrangementKey, NodeArrangementKey),
        shared_intervals: Vec<(
            ArrangementFaceBoundaryInterval,
            ArrangementFaceBoundaryInterval,
            ArrangementSegmentParameter,
            ArrangementSegmentParameter,
        )>,
        step_index: usize,
        segment: NodeExplicitVerticalStepSegment,
        emitted: &mut BTreeSet<(
            (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
            (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
        )>,
        faces: &mut Vec<(RoadSurfaceVisualPolygon, RoadSurfaceVerticalFaceSource)>,
    ) {
        for (lower_interval, raised_interval, start_t, end_t) in shared_intervals {
            let Some(lower_start) = arrangement_face_boundary_interval_point_at(
                lower_segment_key,
                lower_interval,
                start_t,
            ) else {
                continue;
            };
            let Some(lower_end) = arrangement_face_boundary_interval_point_at(
                lower_segment_key,
                lower_interval,
                end_t,
            ) else {
                continue;
            };
            let Some(raised_start) = arrangement_face_boundary_interval_point_at(
                raised_segment_key,
                raised_interval,
                start_t,
            ) else {
                continue;
            };
            let Some(raised_end) = arrangement_face_boundary_interval_point_at(
                raised_segment_key,
                raised_interval,
                end_t,
            ) else {
                continue;
            };
            let Some((dedup_key, face)) = Self::arrangement_vertical_step_face_polygon(
                lower_segment_key,
                lower_interval,
                lower_start,
                lower_end,
                raised_start,
                raised_end,
            ) else {
                continue;
            };
            if !emitted.insert(dedup_key) {
                continue;
            }
            faces.push((
                face,
                RoadSurfaceVerticalFaceSource::CanonicalStep {
                    explicit_vertical_step_index: step_index,
                    segment,
                },
            ));
        }
    }

    fn arrangement_vertical_step_face_polygon(
        segment_key: (NodeArrangementKey, NodeArrangementKey),
        lower_interval: ArrangementFaceBoundaryInterval,
        lower_start: Vector3,
        lower_end: Vector3,
        raised_start: Vector3,
        raised_end: Vector3,
    ) -> Option<(
        (
            (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
            (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
        ),
        RoadSurfaceVisualPolygon,
    )> {
        let lower_span_xz = Vector2::new(lower_end.x - lower_start.x, lower_end.z - lower_start.z);
        let raised_span_xz =
            Vector2::new(raised_end.x - raised_start.x, raised_end.z - raised_start.z);
        if lower_span_xz.length_squared() <= VERTICAL_STEP_MIN_SPAN_M * VERTICAL_STEP_MIN_SPAN_M
            || raised_span_xz.length_squared()
                <= VERTICAL_STEP_MIN_SPAN_M * VERTICAL_STEP_MIN_SPAN_M
        {
            return None;
        }
        if raised_start.y <= lower_start.y + SAMPLE_EPSILON_M
            || raised_end.y <= lower_end.y + SAMPLE_EPSILON_M
        {
            return None;
        }
        let dedup_key = vertical_face_dedup_key(lower_start, lower_end, raised_start, raised_end);
        let lower_owner_on_right =
            lower_interval_owner_lies_right_of_segment(segment_key, lower_interval)?;
        let points = if lower_owner_on_right {
            [raised_start, lower_start, lower_end, raised_end]
        } else {
            [raised_end, lower_end, lower_start, raised_start]
        };
        Self::make_vertical_quad_polygon(points).map(|face| (dedup_key, face))
    }

    pub(super) fn sort_raised_step_faces(
        faces: &mut [(RoadSurfaceVisualPolygon, RoadSurfaceVerticalFaceSource)],
    ) {
        faces.sort_by(
            |(left_polygon, left_source), (right_polygon, right_source)| {
                Self::visual_polygon_ordering(left_polygon, right_polygon)
                    .then(left_source.sort_key().cmp(&right_source.sort_key()))
            },
        );
    }
}

fn lower_interval_owner_lies_right_of_segment(
    segment_key: (NodeArrangementKey, NodeArrangementKey),
    lower_interval: ArrangementFaceBoundaryInterval,
) -> Option<bool> {
    let segment_start = arrangement_key_boundary_point(segment_key.0, 0);
    let segment_end = arrangement_key_boundary_point(segment_key.1, 0);
    let edge_start_t =
        boundary_segment_parameter_xz(lower_interval.edge_start, segment_start, segment_end)?;
    let edge_end_t =
        boundary_segment_parameter_xz(lower_interval.edge_end, segment_start, segment_end)?;
    Some(edge_end_t < edge_start_t)
}

fn vertical_face_dedup_key(
    lower_start: Vector3,
    lower_end: Vector3,
    upper_start: Vector3,
    upper_end: Vector3,
) -> (
    (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
    (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
) {
    (
        normalized_arrangement_boundary_segment_key(lower_start, lower_end),
        normalized_arrangement_boundary_segment_key(upper_start, upper_end),
    )
}

pub(super) fn canonical_vertical_step_lower_and_raised_owners(
    segment: NodeExplicitVerticalStepSegment,
) -> Option<(NodeBandOwner, NodeBandOwner)> {
    let owner = segment.owner();
    let opposite_owner = segment.opposite_owner();
    let (lower_kind, _) = ordered_raised_step_kinds(owner.kind(), opposite_owner.kind())?;
    if owner.kind() == lower_kind {
        Some((owner, opposite_owner))
    } else {
        Some((opposite_owner, owner))
    }
}
