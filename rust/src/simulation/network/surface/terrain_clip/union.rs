//! Terrain-clip union orchestration.

use super::super::{
    NodeOverlayContour, NodeOverlayShape, RoadSurfaceSystem, RoadSurfaceVisualPolygon,
};
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
        let contours = Self::union_terrain_clip_boundary_contours_with_sources(boundary_loops)?;
        let loops = contours
            .iter()
            .map(|contour| contour.boundary_loop.clone())
            .collect::<Vec<_>>();
        let loop_topologies = contours
            .iter()
            .map(|contour| contour.topology)
            .collect::<Vec<_>>();
        let mut polygons = contours
            .iter()
            .filter_map(|contour| {
                Self::make_boundary_loop_polygon(contour.boundary_loop.points_world.clone())
            })
            .collect::<Vec<_>>();
        Self::sort_visual_polygons(&mut polygons);
        Ok(RoadSurfaceTerrainClipExport {
            loops,
            loop_topologies,
            polygons,
        })
    }

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

    fn terrain_clip_boundary_contours_from_overlay_shapes_with_source_edges(
        shapes: &[NodeOverlayShape],
        source_edges: &[TerrainClipSourceEdge],
    ) -> Result<Vec<TerrainClipOutputContour>, RoadSurfaceTerrainClipExportError> {
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

    fn terrain_clip_boundary_loop_from_overlay_contour_with_source_edges(
        outer_contour: &NodeOverlayContour,
        topology: RoadSurfaceTerrainClipLoopTopology,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Result<RoadSurfaceTerrainClipLoop, RoadSurfaceTerrainClipExportError> {
        let contour = Self::compact_overlay_contour_by_key(outer_contour).map_err(|error| {
            RoadSurfaceTerrainClipExportError::RepeatedOverlayPointCycle {
                shape_index: topology.shape_index,
                contour_index: topology.contour_index,
                x_key: error.x_key,
                z_key: error.z_key,
                cycle_area_m2: error.cycle_area_m2,
                remainder_area_m2: error.remainder_area_m2,
                dust_budget_m2: error.dust_budget_m2,
            }
        })?;
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
                        "terrain_clip_missing_boundary_owner shape={} contour={} start=({:.3},{:.3}) end=({:.3},{:.3}) {}",
                        topology.shape_index,
                        topology.contour_index,
                        start[0],
                        start[1],
                        end[0],
                        end[1],
                        context
                    );
                    return Err(
                        RoadSurfaceTerrainClipExportError::MissingOuterBoundaryOwner {
                            shape_index: topology.shape_index,
                            start,
                            end,
                            context,
                        },
                    );
                }
                TerrainClipSegmentPointRecovery::Missing => {
                    match Self::terrain_clip_dust_connector_points_from_source_edges(
                        &contour,
                        index,
                        source_edges,
                    ) {
                        TerrainClipDustConnectorRecovery::Covered(points) => points,
                        TerrainClipDustConnectorRecovery::Ambiguous(context) => {
                            crate::debug_log!(
                                "road",
                                "terrain_clip_ambiguous_dust_connector_height shape={} contour={} start=({:.3},{:.3}) end=({:.3},{:.3}) {}",
                                topology.shape_index,
                                topology.contour_index,
                                start[0],
                                start[1],
                                end[0],
                                end[1],
                                context
                            );
                            return Err(
                                RoadSurfaceTerrainClipExportError::AmbiguousDustConnectorHeight {
                                    shape_index: topology.shape_index,
                                    start,
                                    end,
                                    context,
                                },
                            );
                        }
                        TerrainClipDustConnectorRecovery::Missing => {
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
                                        "terrain_clip_missing_boundary_owner shape={} contour={} start=({:.3},{:.3}) end=({:.3},{:.3}) {}",
                                        topology.shape_index,
                                        topology.contour_index,
                                        start[0],
                                        start[1],
                                        end[0],
                                        end[1],
                                        context
                                    );
                                    return Err(
                                        RoadSurfaceTerrainClipExportError::MissingOuterBoundaryOwner {
                                            shape_index: topology.shape_index,
                                            start,
                                            end,
                                            context,
                                        },
                                    );
                                }
                                TerrainClipSourceChainRecovery::Ambiguous(context) => {
                                    crate::debug_log!(
                                        "road",
                                        "terrain_clip_ambiguous_boundary_owner shape={} contour={} start=({:.3},{:.3}) end=({:.3},{:.3}) {}",
                                        topology.shape_index,
                                        topology.contour_index,
                                        start[0],
                                        start[1],
                                        end[0],
                                        end[1],
                                        context
                                    );
                                    return Err(
                                        RoadSurfaceTerrainClipExportError::MissingOuterBoundaryOwner {
                                            shape_index: topology.shape_index,
                                            start,
                                            end,
                                            context,
                                        },
                                    );
                                }
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
                            "terrain_clip_missing_output_boundary_owner shape={} contour={} start=({:.3},{:.3}) end=({:.3},{:.3})",
                            topology.shape_index,
                            topology.contour_index,
                            start.x,
                            start.z,
                            end.x,
                            end.z
                        );
                        return Err(
                            RoadSurfaceTerrainClipExportError::MissingOutputBoundaryOwner {
                                shape_index: topology.shape_index,
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
                            "terrain_clip_ambiguous_output_boundary_owner shape={} contour={} start=({:.3},{:.3}) end=({:.3},{:.3}) {}",
                            topology.shape_index,
                            topology.contour_index,
                            start.x,
                            start.z,
                            end.x,
                            end.z,
                            context
                        );
                        return Err(
                            RoadSurfaceTerrainClipExportError::AmbiguousOutputBoundaryOwner {
                                shape_index: topology.shape_index,
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
                "terrain_clip_unclosed_output_boundary shape={} contour={} start=({:.3},{:.3}) end=({:.3},{:.3})",
                topology.shape_index,
                topology.contour_index,
                first_start.x,
                first_start.z,
                last_end.x,
                last_end.z
            );
            return Err(RoadSurfaceTerrainClipExportError::UnclosedOutputBoundary {
                shape_index: topology.shape_index,
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

    fn terrain_clip_loop_ordering(
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
