//! Earthwork face classification and sorting.

use super::*;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn classify_earthwork_face_kind_for_source(
        source: RoadSurfaceEarthworkFaceSource,
        inner_start: RoadVec3,
        inner_end: RoadVec3,
        outer_end: RoadVec3,
        outer_start: RoadVec3,
    ) -> RoadSurfaceEarthworkFaceKind {
        if Self::node_curb_or_sidewalk_drop_requires_retaining_wall(
            source,
            inner_start,
            inner_end,
            outer_end,
            outer_start,
        ) {
            return RoadSurfaceEarthworkFaceKind::RetainingWall;
        }
        Self::classify_earthwork_face_kind(inner_start, inner_end, outer_end, outer_start)
    }

    pub(in crate::simulation::network::surface) fn classify_earthwork_face_kind(
        inner_start: RoadVec3,
        inner_end: RoadVec3,
        outer_end: RoadVec3,
        outer_start: RoadVec3,
    ) -> RoadSurfaceEarthworkFaceKind {
        let setback_a =
            RoadVec2::new(outer_start.x - inner_start.x, outer_start.z - inner_start.z).length();
        let setback_b =
            RoadVec2::new(outer_end.x - inner_end.x, outer_end.z - inner_end.z).length();
        let avg_setback = (setback_a + setback_b) * 0.5;
        if avg_setback <= f64::from(SAMPLE_EPSILON_M) {
            return RoadSurfaceEarthworkFaceKind::RetainingWall;
        }

        let max_height_delta = (outer_start.y - inner_start.y)
            .abs()
            .max((outer_end.y - inner_end.y).abs());
        let slope_ratio = max_height_delta / avg_setback.max(f64::from(SAMPLE_EPSILON_M));
        if slope_ratio >= f64::from(EARTHWORK_RETAINING_WALL_SLOPE_THRESHOLD) {
            RoadSurfaceEarthworkFaceKind::RetainingWall
        } else {
            RoadSurfaceEarthworkFaceKind::Slope
        }
    }

    fn node_curb_or_sidewalk_drop_requires_retaining_wall(
        source: RoadSurfaceEarthworkFaceSource,
        inner_start: RoadVec3,
        inner_end: RoadVec3,
        outer_end: RoadVec3,
        outer_start: RoadVec3,
    ) -> bool {
        let owner_kind = match source {
            RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary { owner_kind, .. }
            | RoadSurfaceEarthworkFaceSource::NodeSameMaterialBoundaryHandoff {
                owner_kind, ..
            } => owner_kind,
            RoadSurfaceEarthworkFaceSource::SpanSupportBoundary { .. } => return false,
        };
        if !matches!(
            owner_kind,
            RoadSurfaceBandKind::CurbOrShoulder | RoadSurfaceBandKind::Sidewalk
        ) {
            return false;
        }
        let inner_min_y = inner_start.y.min(inner_end.y);
        let outer_max_y = outer_start.y.max(outer_end.y);
        inner_min_y - outer_max_y >= f64::from(CURB_STEP_HEIGHT_M) * 0.5
    }

    pub(in crate::simulation::network::surface) fn sort_earthwork_render_faces(
        faces: &mut [RoadSurfaceEarthworkRenderFace],
    ) {
        faces.sort_by(|a, b| {
            let kind_order = match (a.kind, b.kind) {
                (
                    RoadSurfaceEarthworkFaceKind::Slope,
                    RoadSurfaceEarthworkFaceKind::RetainingWall,
                ) => std::cmp::Ordering::Less,
                (
                    RoadSurfaceEarthworkFaceKind::RetainingWall,
                    RoadSurfaceEarthworkFaceKind::Slope,
                ) => std::cmp::Ordering::Greater,
                _ => std::cmp::Ordering::Equal,
            };
            if kind_order != std::cmp::Ordering::Equal {
                return kind_order;
            }
            a.source
                .source_ordering(b.source)
                .then(
                    a.inner_start
                        .x
                        .total_cmp(&b.inner_start.x)
                        .then(a.inner_start.z.total_cmp(&b.inner_start.z))
                        .then(a.inner_start.y.total_cmp(&b.inner_start.y)),
                )
                .then(
                    a.polygon
                        .points_world
                        .len()
                        .cmp(&b.polygon.points_world.len()),
                )
                .then_with(|| {
                    match a
                        .polygon
                        .points_world
                        .iter()
                        .zip(&b.polygon.points_world)
                        .find_map(|(point_a, point_b)| {
                            let ordering = point_a
                                .x
                                .total_cmp(&point_b.x)
                                .then(point_a.z.total_cmp(&point_b.z))
                                .then(point_a.y.total_cmp(&point_b.y));
                            (ordering != std::cmp::Ordering::Equal).then_some(ordering)
                        }) {
                        Some(ordering) => ordering,
                        None => std::cmp::Ordering::Equal,
                    }
                })
        });
    }
}
