//! Terrain-clip overlay contour conversion.

use super::*;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface::terrain_clip::union) fn terrain_clip_boundary_contours_from_overlay_shapes_with_source_edges(
        shapes: &[NodeOverlayShape],
        source_edges: &[TerrainClipSourceEdge],
    ) -> Result<Vec<TerrainClipOutputContour>, RoadSurfaceTerrainClipExportError> {
        let source_edge_index = TerrainClipSourceEdgeIndex::new(source_edges);
        let mut contours = Vec::new();
        for (shape_index, shape) in shapes.iter().enumerate() {
            for (contour_index, contour) in shape.iter().enumerate() {
                let topology = RoadSurfaceTerrainClipLoopTopology {
                    shape_index,
                    contour_index,
                    role: if contour_index == 0 {
                        RoadSurfaceTerrainClipContourRole::Outer
                    } else {
                        RoadSurfaceTerrainClipContourRole::Hole
                    },
                };
                let boundary_loop =
                    Self::terrain_clip_boundary_loop_from_overlay_contour_with_source_edges(
                        contour,
                        topology,
                        source_edges,
                        &source_edge_index,
                    )?;
                contours.push(TerrainClipOutputContour {
                    boundary_loop,
                    topology,
                });
            }
        }
        contours.sort_by(|a, b| {
            Self::terrain_clip_loop_ordering(&a.boundary_loop, &b.boundary_loop)
                .then(a.topology.shape_index.cmp(&b.topology.shape_index))
                .then(a.topology.contour_index.cmp(&b.topology.contour_index))
        });
        Ok(contours)
    }

    pub(in crate::simulation::network::surface::terrain_clip::union) fn terrain_clip_loop_ordering(
        a: &RoadSurfaceTerrainClipLoop,
        b: &RoadSurfaceTerrainClipLoop,
    ) -> std::cmp::Ordering {
        match (a.points_world.first(), b.points_world.first()) {
            (Some(point_a), Some(point_b)) => point_a
                .x
                .total_cmp(&point_b.x)
                .then(point_a.z.total_cmp(&point_b.z))
                .then(point_a.y.total_cmp(&point_b.y)),
            (None, Some(_)) => std::cmp::Ordering::Less,
            (Some(_), None) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
        .then(a.points_world.len().cmp(&b.points_world.len()))
        .then_with(|| {
            a.points_world
                .iter()
                .zip(&b.points_world)
                .find_map(|(point_a, point_b)| {
                    let ordering = point_a
                        .x
                        .total_cmp(&point_b.x)
                        .then(point_a.z.total_cmp(&point_b.z))
                        .then(point_a.y.total_cmp(&point_b.y));
                    (ordering != std::cmp::Ordering::Equal).then_some(ordering)
                })
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }
}
