//! Top-surface intrusion and overlay checks.

use super::*;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface::earthwork) fn earthwork_candidate_intrudes_top(
        points: [RoadVec3; 4],
        top_surface_shapes: &NodeOverlayShapes,
    ) -> bool {
        let Some((overlap_area_m2, budget_m2)) =
            Self::earthwork_candidate_top_overlap_metrics_m2(points, top_surface_shapes)
        else {
            return true;
        };
        overlap_area_m2 > budget_m2
    }

    pub(in crate::simulation::network::surface::earthwork) fn earthwork_candidate_top_overlap_area_m2(
        points: [RoadVec3; 4],
        top_surface_shapes: &NodeOverlayShapes,
    ) -> Option<f32> {
        Self::earthwork_candidate_top_overlap_metrics_m2(points, top_surface_shapes)
            .map(|(overlap_area_m2, _)| overlap_area_m2)
    }

    pub(in crate::simulation::network::surface::earthwork) fn earthwork_candidate_top_overlap_metrics_m2(
        mut points: [RoadVec3; 4],
        top_surface_shapes: &NodeOverlayShapes,
    ) -> Option<(f32, f32)> {
        if Self::earthwork_signed_polygon_area_xz(&points) < 0.0 {
            points.reverse();
        }
        let candidate_shapes =
            Self::overlay_union_contours(&[Self::earthwork_overlay_contour_from_points(&points)])?;
        let overlap = Self::overlay_binary_shapes(
            &candidate_shapes,
            top_surface_shapes,
            OverlayRule::Intersect,
        )?;
        let overlap_area_m2 = overlap.iter().map(Self::overlay_shape_area_m2).sum();
        let budget_m2 = Self::overlay_numeric_area_budget_for_shapes(&candidate_shapes).max(
            Self::overlay_numeric_area_budget_for_shapes(top_surface_shapes),
        );
        Some((overlap_area_m2, budget_m2))
    }

    pub(in crate::simulation::network::surface::earthwork) fn earthwork_overlay_contour_from_points(
        points: &[RoadVec3],
    ) -> NodeOverlayContour {
        let mut contour = Vec::with_capacity(points.len());
        for point in points {
            let point = backend::road_vec2_to_overlay_point(backend::road_vec3_xz(*point));
            if contour.last().is_none_or(|last| *last != point) {
                contour.push(point);
            }
        }
        if contour.len() >= 2 && contour.first() == contour.last() {
            contour.pop();
        }
        contour
    }

    pub(in crate::simulation::network::surface) fn top_surface_overlay_shapes<'a>(
        polygons: impl IntoIterator<Item = &'a RoadSurfaceVisualPolygon>,
    ) -> Option<NodeOverlayShapes> {
        let mut contours = Vec::new();
        for polygon in polygons {
            if polygon.points_world.len() >= 3 {
                contours.push(Self::earthwork_overlay_contour_from_points(
                    &polygon.points_world,
                ));
            }
        }
        Self::overlay_union_contours(&contours)
    }
}
