//! Outer boundary polygon export from solved arrangement regions.

use super::super::*;

impl RoadSurfaceSystem {
    pub(super) fn outer_boundary_polygons_from_arrangement_regions(
        arrangement: &NodeArrangement,
    ) -> Result<Vec<RoadSurfaceVisualPolygon>, NodeBoundaryExportError> {
        let mut polygons = Vec::new();
        for region in arrangement.regions() {
            let points = region
                .outer_loop()
                .iter()
                .copied()
                .filter_map(|vertex_id| Self::arrangement_vertex_world(arrangement, vertex_id))
                .collect::<Vec<_>>();
            if points.len() < 3 {
                continue;
            }
            if Self::signed_polygon_area_xz(&points).abs()
                <= boundary_points_numeric_area_budget_m2(&points)
            {
                continue;
            }
            let Some(polygon) = Self::make_boundary_loop_polygon_preserving_winding(points) else {
                return Err(NodeBoundaryExportError::DegenerateOuterBoundaryLoop);
            };
            polygons.push(polygon);
        }
        (!polygons.is_empty())
            .then_some(polygons)
            .ok_or(NodeBoundaryExportError::EmptyOuterBoundary)
    }
}
