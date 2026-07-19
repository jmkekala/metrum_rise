//! Raised-step vertical face construction from canonical arrangement intervals.

#[cfg(test)]
use super::arrangement_faces::*;
#[cfg(test)]
use super::boundary_edges::*;
use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(in crate::simulation::network::surface::node) struct RoadSurfaceRaisedStepFace {
    pub(in crate::simulation::network::surface::node) polygon: RoadSurfaceVisualPolygon,
    pub(in crate::simulation::network::surface::node) source: RoadSurfaceVerticalFaceSource,
    pub(in crate::simulation::network::surface::node) lower_edge:
        (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
}

impl RoadSurfaceSystem {
    #[cfg(test)]
    pub(super) fn raised_step_face_polygons_from_arrangement(
        arrangement: &NodeArrangement,
        explicit_vertical_step_segments: &[NodeExplicitVerticalStepSegment],
    ) -> Vec<RoadSurfaceRaisedStepFace> {
        let derived_overlap_segments =
            arrangement.derived_overlap_explicit_vertical_step_segments();
        let mut emitted = BTreeSet::new();
        let mut faces = Vec::new();
        for (step_index, segment) in explicit_vertical_step_segments.iter().copied().enumerate() {
            let Some((lower_owner, raised_owner)) =
                canonical_vertical_step_lower_and_raised_owners(segment)
            else {
                continue;
            };
            let segment_key = (segment.start(), segment.end());
            let allow_overlay_only_overlap = derived_overlap_segments.contains(&segment);
            let lower_intervals = arrangement_owner_face_boundary_intervals_for_segment(
                arrangement,
                lower_owner,
                segment_key,
                false,
                allow_overlay_only_overlap,
            );
            let raised_intervals = arrangement_owner_face_boundary_intervals_for_segment(
                arrangement,
                raised_owner,
                segment_key,
                true,
                allow_overlay_only_overlap,
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

    #[cfg(test)]
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
        faces: &mut Vec<RoadSurfaceRaisedStepFace>,
    ) {
        for (lower_interval, raised_interval, start_t, end_t) in shared_intervals {
            let Some(lower_start_key) = arrangement_face_boundary_interval_point_at(
                lower_segment_key,
                lower_interval,
                start_t,
            ) else {
                continue;
            };
            let Some(lower_end_key) = arrangement_face_boundary_interval_point_at(
                lower_segment_key,
                lower_interval,
                end_t,
            ) else {
                continue;
            };
            let Some(raised_start_key) = arrangement_face_boundary_interval_point_at(
                raised_segment_key,
                raised_interval,
                start_t,
            ) else {
                continue;
            };
            let Some(raised_end_key) = arrangement_face_boundary_interval_point_at(
                raised_segment_key,
                raised_interval,
                end_t,
            ) else {
                continue;
            };
            let lower_start = arrangement_boundary_point_to_world(lower_start_key);
            let lower_end = arrangement_boundary_point_to_world(lower_end_key);
            let raised_start = arrangement_boundary_point_to_world(raised_start_key);
            let raised_end = arrangement_boundary_point_to_world(raised_end_key);
            let Some((dedup_key, face)) = Self::arrangement_vertical_step_face_polygon(
                lower_segment_key,
                lower_interval,
                lower_start_key,
                lower_end_key,
                raised_start_key,
                raised_end_key,
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
            faces.push(RoadSurfaceRaisedStepFace {
                polygon: face,
                source: RoadSurfaceVerticalFaceSource::CanonicalStep {
                    explicit_vertical_step_index: step_index,
                    segment,
                },
                lower_edge: (lower_start_key, lower_end_key),
            });
        }
    }

    #[cfg(test)]
    fn arrangement_vertical_step_face_polygon(
        segment_key: (NodeArrangementKey, NodeArrangementKey),
        lower_interval: ArrangementFaceBoundaryInterval,
        lower_start_key: ArrangementBoundaryPointKey,
        lower_end_key: ArrangementBoundaryPointKey,
        raised_start_key: ArrangementBoundaryPointKey,
        raised_end_key: ArrangementBoundaryPointKey,
        lower_start: RoadVec3,
        lower_end: RoadVec3,
        raised_start: RoadVec3,
        raised_end: RoadVec3,
    ) -> Option<(
        (
            (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
            (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
        ),
        RoadSurfaceVisualPolygon,
    )> {
        if lower_start_key.xz_key() == lower_end_key.xz_key()
            || raised_start_key.xz_key() == raised_end_key.xz_key()
        {
            return None;
        }
        let start_height_delta_mm = raised_start_key.y_mm - lower_start_key.y_mm;
        let end_height_delta_mm = raised_end_key.y_mm - lower_end_key.y_mm;
        if start_height_delta_mm < 0
            || end_height_delta_mm < 0
            || (start_height_delta_mm == 0 && end_height_delta_mm == 0)
        {
            return None;
        }
        let dedup_key = vertical_face_dedup_key(
            lower_start_key,
            lower_end_key,
            raised_start_key,
            raised_end_key,
        );
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

#[cfg(test)]
fn lower_interval_owner_lies_right_of_segment(
    segment_key: (NodeArrangementKey, NodeArrangementKey),
    lower_interval: ArrangementFaceBoundaryInterval,
) -> Option<bool> {
    let segment_start = arrangement_key_boundary_point(segment_key.0, 0);
    let segment_end = arrangement_key_boundary_point(segment_key.1, 0);
    let edge_start_t = arrangement_boundary_segment_order_key(
        lower_interval.edge_start,
        segment_start,
        segment_end,
    )?;
    let edge_end_t = arrangement_boundary_segment_order_key(
        lower_interval.edge_end,
        segment_start,
        segment_end,
    )?;
    Some(edge_end_t < edge_start_t)
}

#[cfg(test)]
fn arrangement_boundary_segment_order_key(
    point: ArrangementBoundaryPointKey,
    start: ArrangementBoundaryPointKey,
    end: ArrangementBoundaryPointKey,
) -> Option<i128> {
    let dx = i128::from(end.x_key - start.x_key);
    let dz = i128::from(end.z_key - start.z_key);
    let px = i128::from(point.x_key - start.x_key);
    let pz = i128::from(point.z_key - start.z_key);
    (dx != 0 || dz != 0).then_some(px * dx + pz * dz)
}

#[cfg(test)]
fn vertical_face_dedup_key(
    lower_start: ArrangementBoundaryPointKey,
    lower_end: ArrangementBoundaryPointKey,
    upper_start: ArrangementBoundaryPointKey,
    upper_end: ArrangementBoundaryPointKey,
) -> (
    (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
    (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
) {
    (
        normalized_arrangement_boundary_segment_key(lower_start, lower_end),
        normalized_arrangement_boundary_segment_key(upper_start, upper_end),
    )
}

#[cfg(test)]
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
