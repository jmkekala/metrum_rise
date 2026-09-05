// SPDX-License-Identifier: GPL-2.0-only

//! Closed earthwork loop geometry assembly.

use super::*;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn build_closed_earthwork_geometry_from_boundary_segments(
        &self,
        boundary_segment_loops: &[Vec<RoadSurfaceEarthworkBoundarySegment>],
        terrain: &TerrainSystem,
        top_surface_shapes: Option<&NodeOverlayShapes>,
    ) -> Result<
        (
            Vec<RoadSurfaceVisualPolygon>,
            Vec<RoadSurfaceVisualPolygon>,
            Vec<RoadSurfaceEarthworkRenderFace>,
        ),
        RoadSurfaceEarthworkGeometryError,
    > {
        let mut earthwork_surface_polygons = Vec::new();
        let mut earthwork_outer_boundary_loops = Vec::new();
        let mut render_earthwork_faces = Vec::new();

        for boundary_segments in boundary_segment_loops {
            let Some((outer_loop, side_polygons, render_faces)) = self
                .build_closed_earthwork_loop_geometry(
                    boundary_segments,
                    terrain,
                    top_surface_shapes,
                )?
            else {
                continue;
            };
            if let Some(outer_loop) = outer_loop {
                earthwork_outer_boundary_loops.push(outer_loop);
            }
            earthwork_surface_polygons.extend(side_polygons);
            render_earthwork_faces.extend(render_faces);
        }

        Self::sort_visual_polygons(&mut earthwork_surface_polygons);
        Self::sort_visual_polygons(&mut earthwork_outer_boundary_loops);
        Self::sort_earthwork_render_faces(&mut render_earthwork_faces);
        Ok((
            earthwork_surface_polygons,
            earthwork_outer_boundary_loops,
            render_earthwork_faces,
        ))
    }

    pub(in crate::simulation::network::surface::earthwork) fn build_closed_earthwork_loop_geometry(
        &self,
        boundary_segments: &[RoadSurfaceEarthworkBoundarySegment],
        terrain: &TerrainSystem,
        top_surface_shapes: Option<&NodeOverlayShapes>,
    ) -> Result<
        Option<(
            Option<RoadSurfaceVisualPolygon>,
            Vec<RoadSurfaceVisualPolygon>,
            Vec<RoadSurfaceEarthworkRenderFace>,
        )>,
        RoadSurfaceEarthworkGeometryError,
    > {
        if boundary_segments.len() < 3 {
            return Err(RoadSurfaceEarthworkGeometryError::DegenerateBoundaryLoop {
                point_count: boundary_segments.len(),
            });
        }
        let boundary_points = boundary_segments
            .iter()
            .map(|segment| segment.inner_start)
            .collect::<Vec<_>>();

        let mut vertex_outer_points = Vec::with_capacity(boundary_points.len());
        for (index, point) in boundary_points.iter().enumerate() {
            let Some(outward) = Self::closed_loop_vertex_outward_xz(&boundary_points, index) else {
                vertex_outer_points.clear();
                break;
            };
            let Some(outer_point) = self.earthwork_transition_point(*point, outward, terrain)
            else {
                vertex_outer_points.clear();
                break;
            };
            vertex_outer_points.push(outer_point);
        }
        // A cusp has no unique vertex miter. Keep edge-based faces, but do not invent a loop point.
        let outer_loop = if vertex_outer_points.len() == boundary_points.len() {
            // This polygon is a coverage boundary, never a rendered surface. Avoid an unnecessary
            // constrained triangulation whose triangles no consumer reads.
            Self::make_boundary_loop_polygon(vertex_outer_points)
        } else {
            None
        };
        let mut side_polygons = Vec::new();
        let mut render_faces = Vec::new();
        let winding_ccw = Self::earthwork_signed_polygon_area_xz(&boundary_points) > 0.0;
        for segment in boundary_segments {
            let current = segment.inner_start;
            let next = segment.inner_end;
            let (outer_current, outer_next) = self.earthwork_edge_transition_points(
                current,
                next,
                winding_ccw,
                terrain,
                top_surface_shapes,
            )?;
            // Handoff and internal seam edges can be part of a closed footprint loop. They are not
            // terrain tie-ins, so a skirt whose plan area enters solved top ownership is rejected.
            if let Some(top_surface_shapes) = top_surface_shapes
                && Self::earthwork_candidate_intrudes_top(
                    [current, next, outer_next, outer_current],
                    top_surface_shapes,
                )
            {
                continue;
            }
            let Some(polygon) = Self::earthwork_visual_polygon_from_road_points(vec![
                current,
                next,
                outer_next,
                outer_current,
            ]) else {
                continue;
            };
            let face_kind = Self::classify_earthwork_face_kind_for_source(
                segment.source,
                current,
                next,
                outer_next,
                outer_current,
            );
            render_faces.push(RoadSurfaceEarthworkRenderFace {
                kind: face_kind,
                source: segment.source,
                inner_start: current,
                inner_end: next,
                polygon: polygon.clone(),
            });
            side_polygons.push(polygon);
        }

        if outer_loop.is_none() && side_polygons.is_empty() {
            return Ok(None);
        }
        Ok(Some((outer_loop, side_polygons, render_faces)))
    }
}
