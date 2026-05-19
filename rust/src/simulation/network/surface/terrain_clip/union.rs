//! Terrain-clip union orchestration.

use super::super::{NodeOverlayShape, RoadSurfaceSystem, RoadSurfaceVisualPolygon};
use super::model::*;
use super::output::TerrainClipOutputSourceError;
use super::recovery::TerrainClipSourceChainRecovery;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn union_terrain_clip_boundary_loops(
        boundary_loops: &[RoadSurfaceTerrainClipLoop],
    ) -> Result<Vec<RoadSurfaceVisualPolygon>, RoadSurfaceTerrainClipExportError> {
        Self::union_terrain_clip_boundary_loops_with_sources(boundary_loops).map(|loops| {
            loops
                .into_iter()
                .filter_map(|boundary_loop| {
                    Self::make_boundary_loop_polygon(boundary_loop.points_world)
                })
                .collect()
        })
    }

    pub(in crate::simulation::network::surface) fn union_terrain_clip_boundary_export(
        boundary_loops: &[RoadSurfaceTerrainClipLoop],
    ) -> Result<RoadSurfaceTerrainClipExport, RoadSurfaceTerrainClipExportError> {
        let loops = Self::union_terrain_clip_boundary_loops_with_sources(boundary_loops)?;
        let mut polygons = loops
            .iter()
            .filter_map(|boundary_loop| {
                Self::make_boundary_loop_polygon(boundary_loop.points_world.clone())
            })
            .collect::<Vec<_>>();
        Self::sort_visual_polygons(&mut polygons);
        Ok(RoadSurfaceTerrainClipExport { loops, polygons })
    }

    pub(in crate::simulation::network::surface) fn union_terrain_clip_boundary_loops_with_sources(
        boundary_loops: &[RoadSurfaceTerrainClipLoop],
    ) -> Result<Vec<RoadSurfaceTerrainClipLoop>, RoadSurfaceTerrainClipExportError> {
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
        Self::terrain_clip_boundary_loops_from_overlay_shapes_with_source_edges(
            &shapes,
            &source_edges,
        )
    }

    fn terrain_clip_boundary_loops_from_overlay_shapes_with_source_edges(
        shapes: &[NodeOverlayShape],
        source_edges: &[TerrainClipSourceEdge],
    ) -> Result<Vec<RoadSurfaceTerrainClipLoop>, RoadSurfaceTerrainClipExportError> {
        let mut loops = Vec::new();
        for (shape_index, shape) in shapes.iter().enumerate() {
            let boundary_loop =
                Self::terrain_clip_boundary_loop_from_overlay_shape_with_source_edges(
                    shape,
                    shape_index,
                    source_edges,
                )?;
            loops.push(boundary_loop);
        }
        Self::sort_terrain_clip_loops(&mut loops);
        Ok(loops)
    }

    fn terrain_clip_boundary_loop_from_overlay_shape_with_source_edges(
        shape: &NodeOverlayShape,
        shape_index: usize,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Result<RoadSurfaceTerrainClipLoop, RoadSurfaceTerrainClipExportError> {
        let outer_contour =
            shape
                .first()
                .ok_or_else(|| RoadSurfaceTerrainClipExportError::OverlayUnionFailed {
                    source_loop_count: 0,
                })?;
        let contour = Self::compact_overlay_contour_by_key(outer_contour);
        if contour.len() < 3 {
            return Err(RoadSurfaceTerrainClipExportError::OverlayUnionFailed {
                source_loop_count: 0,
            });
        }

        let mut output_edges = Vec::new();
        for index in 0..contour.len() {
            let start = contour[index];
            let end = contour[(index + 1) % contour.len()];
            let segment_points = match Self::terrain_clip_segment_points_from_source_edges(
                start,
                end,
                source_edges,
            ) {
                TerrainClipSegmentPointRecovery::Degenerate => continue,
                TerrainClipSegmentPointRecovery::Covered(points) => points,
                TerrainClipSegmentPointRecovery::Partial => {
                    let context = format!(
                        "partial_coverage {}",
                        Self::terrain_clip_missing_source_context_label(start, end, source_edges)
                    );
                    crate::debug_log!(
                        "road",
                        "terrain_clip_missing_outer_boundary_owner shape={} start=({:.3},{:.3}) end=({:.3},{:.3}) {}",
                        shape_index,
                        start[0],
                        start[1],
                        end[0],
                        end[1],
                        context
                    );
                    return Err(
                        RoadSurfaceTerrainClipExportError::MissingOuterBoundaryOwner {
                            shape_index,
                            start,
                            end,
                            context,
                        },
                    );
                }
                TerrainClipSegmentPointRecovery::Missing => {
                    if let Some(points) = Self::terrain_clip_dust_connector_points_from_source_edges(
                        &contour,
                        index,
                        source_edges,
                    ) {
                        points
                    } else {
                        match Self::terrain_clip_source_chain_points_from_source_edges(
                            start,
                            end,
                            source_edges,
                        ) {
                            TerrainClipSourceChainRecovery::Covered(points) => points,
                            TerrainClipSourceChainRecovery::Missing => {
                                let context = Self::terrain_clip_missing_source_context_label(
                                    start,
                                    end,
                                    source_edges,
                                );
                                crate::debug_log!(
                                    "road",
                                    "terrain_clip_missing_outer_boundary_owner shape={} start=({:.3},{:.3}) end=({:.3},{:.3}) {}",
                                    shape_index,
                                    start[0],
                                    start[1],
                                    end[0],
                                    end[1],
                                    context
                                );
                                return Err(
                                    RoadSurfaceTerrainClipExportError::MissingOuterBoundaryOwner {
                                        shape_index,
                                        start,
                                        end,
                                        context,
                                    },
                                );
                            }
                            TerrainClipSourceChainRecovery::Ambiguous(context) => {
                                crate::debug_log!(
                                    "road",
                                    "terrain_clip_ambiguous_outer_boundary_owner shape={} start=({:.3},{:.3}) end=({:.3},{:.3}) {}",
                                    shape_index,
                                    start[0],
                                    start[1],
                                    end[0],
                                    end[1],
                                    context
                                );
                                return Err(
                                    RoadSurfaceTerrainClipExportError::MissingOuterBoundaryOwner {
                                        shape_index,
                                        start,
                                        end,
                                        context,
                                    },
                                );
                            }
                        }
                    }
                }
            };
            if let Err(error) = Self::append_terrain_clip_sourced_segment_points(
                &mut output_edges,
                segment_points,
                source_edges,
            ) {
                match error {
                    TerrainClipOutputSourceError::Missing { start, end } => {
                        crate::debug_log!(
                            "road",
                            "terrain_clip_missing_output_boundary_owner shape={} start=({:.3},{:.3}) end=({:.3},{:.3})",
                            shape_index,
                            start.x,
                            start.z,
                            end.x,
                            end.z
                        );
                        return Err(
                            RoadSurfaceTerrainClipExportError::MissingOutputBoundaryOwner {
                                shape_index,
                                start,
                                end,
                            },
                        );
                    }
                    TerrainClipOutputSourceError::Ambiguous {
                        start,
                        end,
                        context,
                    } => {
                        crate::debug_log!(
                            "road",
                            "terrain_clip_ambiguous_output_boundary_owner shape={} start=({:.3},{:.3}) end=({:.3},{:.3}) {}",
                            shape_index,
                            start.x,
                            start.z,
                            end.x,
                            end.z,
                            context
                        );
                        return Err(
                            RoadSurfaceTerrainClipExportError::AmbiguousOutputBoundaryOwner {
                                shape_index,
                                start,
                                end,
                                context,
                            },
                        );
                    }
                }
            }
        }

        Self::close_terrain_clip_source_edges(&mut output_edges);
        if output_edges.len() < 3 {
            return Err(RoadSurfaceTerrainClipExportError::OverlayUnionFailed {
                source_loop_count: 0,
            });
        }
        let first_start = output_edges.first().map(|edge| edge.start).ok_or_else(|| {
            RoadSurfaceTerrainClipExportError::OverlayUnionFailed {
                source_loop_count: 0,
            }
        })?;
        let last_end = output_edges.last().map(|edge| edge.end).ok_or_else(|| {
            RoadSurfaceTerrainClipExportError::OverlayUnionFailed {
                source_loop_count: 0,
            }
        })?;
        if !Self::world_points_same_for_boundary(first_start, last_end) {
            crate::debug_log!(
                "road",
                "terrain_clip_unclosed_output_boundary shape={} start=({:.3},{:.3}) end=({:.3},{:.3})",
                shape_index,
                first_start.x,
                first_start.z,
                last_end.x,
                last_end.z
            );
            return Err(RoadSurfaceTerrainClipExportError::UnclosedOutputBoundary {
                shape_index,
                start: first_start,
                end: last_end,
            });
        }
        let points_world = output_edges.iter().map(|edge| edge.start).collect();
        Ok(RoadSurfaceTerrainClipLoop {
            points_world,
            source_edges: output_edges,
        })
    }
}
