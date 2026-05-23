//! Terrain-clip union public entry points.

use super::*;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn union_terrain_clip_boundary_export(
        boundary_loops: &[RoadSurfaceTerrainClipLoop],
    ) -> Result<RoadSurfaceTerrainClipExport, RoadSurfaceTerrainClipExportError> {
        let contours = Self::union_terrain_clip_boundary_contours_with_sources(boundary_loops)?;
        let loops = contours
            .iter()
            .map(|contour| contour.boundary_loop.clone())
            .collect::<Vec<_>>();
        let loop_topologies = contours
            .iter()
            .map(|contour| contour.topology)
            .collect::<Vec<_>>();
        Ok(RoadSurfaceTerrainClipExport {
            loops,
            loop_topologies,
        })
    }

    #[cfg(test)]
    pub(in crate::simulation::network::surface) fn union_terrain_clip_boundary_loops_with_sources(
        boundary_loops: &[RoadSurfaceTerrainClipLoop],
    ) -> Result<Vec<RoadSurfaceTerrainClipLoop>, RoadSurfaceTerrainClipExportError> {
        Self::union_terrain_clip_boundary_contours_with_sources(boundary_loops).map(|contours| {
            contours
                .into_iter()
                .map(|contour| contour.boundary_loop)
                .collect()
        })
    }

    fn union_terrain_clip_boundary_contours_with_sources(
        boundary_loops: &[RoadSurfaceTerrainClipLoop],
    ) -> Result<Vec<TerrainClipOutputContour>, RoadSurfaceTerrainClipExportError> {
        if boundary_loops.is_empty() {
            return Ok(Vec::new());
        }

        let contours = Self::overlay_contours_from_terrain_clip_boundary_loops(boundary_loops);
        let Some(mut shapes) = Self::overlay_union_contours(&contours) else {
            return Err(RoadSurfaceTerrainClipExportError::OverlayUnionFailed {
                source_loop_count: boundary_loops.len(),
            });
        };
        Self::sort_overlay_shapes(&mut shapes);
        let source_edges = Self::terrain_clip_source_edges_from_boundary_loops(boundary_loops);
        Self::terrain_clip_boundary_contours_from_overlay_shapes_with_source_edges(
            &shapes,
            &source_edges,
        )
    }
}
