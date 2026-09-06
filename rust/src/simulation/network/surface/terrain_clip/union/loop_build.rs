// SPDX-License-Identifier: GPL-2.0-only

//! Terrain-clip sourced loop construction.

use super::*;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface::terrain_clip::union) fn terrain_clip_boundary_loop_from_overlay_contour_with_source_edges(
        outer_contour: &NodeOverlayContour,
        topology: RoadSurfaceTerrainClipLoopTopology,
        source_edges: &[TerrainClipSourceEdge],
        source_edge_index: &TerrainClipSourceEdgeIndex,
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
            let direct_source_edges =
                source_edge_index.candidates_for_segment(start, end, source_edges);
            let direct_prepared_sources =
                Self::terrain_clip_prepared_sources_on_segment(start, end, &direct_source_edges);
            let mut direct_coverage = true;
            let segment_points = match Self::terrain_clip_segment_points_from_prepared_sources(
                start,
                end,
                &direct_prepared_sources,
            ) {
                TerrainClipSegmentPointRecovery::Degenerate => continue,
                TerrainClipSegmentPointRecovery::Covered(points) => points,
                TerrainClipSegmentPointRecovery::Partial => {
                    direct_coverage = false;
                    match Self::terrain_clip_source_chain_points_from_source_edges(
                        start,
                        end,
                        source_edges,
                    ) {
                        TerrainClipSourceChainRecovery::Covered(points) => points,
                        TerrainClipSourceChainRecovery::Missing => {
                            let context = format!(
                                "partial_coverage {}",
                                Self::terrain_clip_missing_source_context_label(
                                    start,
                                    end,
                                    source_edges,
                                )
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
                                "terrain_clip_ambiguous_boundary_owner shape={} contour={} start=({:.3},{:.3}) end=({:.3},{:.3}) partial_coverage {}",
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
                                    context: format!("partial_coverage {context}"),
                                },
                            );
                        }
                    }
                }
                TerrainClipSegmentPointRecovery::Missing => {
                    direct_coverage = false;
                    match Self::terrain_clip_dust_connector_points_from_source_edges(
                        &contour,
                        index,
                        source_edges,
                        source_edge_index,
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
            let append_result = if direct_coverage {
                Self::append_terrain_clip_prepared_segment_points(
                    &mut output_edges,
                    segment_points,
                    start,
                    end,
                    &direct_prepared_sources,
                    &direct_source_edges,
                )
            } else {
                Self::append_terrain_clip_sourced_segment_points(
                    &mut output_edges,
                    segment_points,
                    source_edges,
                )
            };
            if let Err(error) = append_result {
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
}
