//! Visual node-piece compilation orchestration.

use super::*;
use crate::simulation::network::types::EdgeClass;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn compile_visual_node_piece(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
    ) -> Option<RoadSurfaceVisualNodePiece> {
        let valid = graph.get_valid_node(node_id);
        let incidents = self.sorted_incident_surface_edges(graph, valid);
        match self.classify_visual_node_kind(&incidents) {
            CompiledNodeKind::Terminal => incidents.first().and_then(|incident| {
                self.build_terminal_visual_node_piece(graph, terrain, valid, *incident)
            }),
            CompiledNodeKind::PassThrough => None,
            CompiledNodeKind::Bend => {
                self.build_bend_visual_node_piece(graph, terrain, valid, &incidents)
            }
            CompiledNodeKind::JunctionN => {
                self.build_junction_visual_node_piece(graph, terrain, valid, &incidents)
            }
        }
    }
    fn build_terminal_visual_node_piece(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
        incident: IncidentSurfaceEdge,
    ) -> Option<RoadSurfaceVisualNodePiece> {
        let mouths = self.build_ordered_piece_mouths(&[incident])?;
        self.build_canonical_visual_node_piece(
            graph,
            terrain,
            node_id,
            RoadSurfaceVisualNodePieceKind::Terminal,
            &mouths,
        )
    }

    fn build_bend_visual_node_piece(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
        incidents: &[IncidentSurfaceEdge],
    ) -> Option<RoadSurfaceVisualNodePiece> {
        if incidents.len() != 2 {
            return None;
        }
        let mouths = self.build_ordered_piece_mouths(incidents)?;
        self.build_canonical_visual_node_piece(
            graph,
            terrain,
            node_id,
            RoadSurfaceVisualNodePieceKind::Bend,
            &mouths,
        )
    }

    fn build_junction_visual_node_piece(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
        incidents: &[IncidentSurfaceEdge],
    ) -> Option<RoadSurfaceVisualNodePiece> {
        if incidents.len() < 3 {
            return None;
        }
        if incidents
            .iter()
            .any(|incident| self.incident_edge_visual_handoff_is_overconstrained(graph, *incident))
        {
            return None;
        }
        let Some(mouths) = self.build_ordered_piece_mouths(incidents) else {
            return None;
        };
        self.build_canonical_visual_node_piece(
            graph,
            terrain,
            node_id,
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &mouths,
        )
    }

    fn build_canonical_visual_node_piece(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
        kind: RoadSurfaceVisualNodePieceKind,
        mouths: &[OrderedIncidentPieceMouth],
    ) -> Option<RoadSurfaceVisualNodePiece> {
        let node_regions = self.compile_canonical_node_surface_regions(node_id, kind, mouths)?;
        let top_surface_shapes = Self::top_surface_overlay_shapes(
            node_regions
                .road_surface_polygons
                .iter()
                .chain(node_regions.curb_surface_polygons.iter())
                .chain(node_regions.sidewalk_surface_polygons.iter()),
        );
        let (earthwork_surface_polygons, earthwork_outer_boundary_loops, render_earthwork_faces) =
            self.build_closed_earthwork_geometry_from_boundary_segments(
                &node_regions.earthwork_boundary_segments,
                terrain,
                top_surface_shapes.as_ref(),
            )
            .ok()?;
        let earthwork_owner_sources = Self::node_earthwork_owner_sources_from_regions(
            graph,
            mouths,
            &node_regions.owned_regions,
            &node_regions.node_top_surface_sources,
        );

        self.assemble_explicit_node_piece(
            node_id,
            kind,
            node_regions.outer_boundary_loops,
            node_regions.terrain_clip_boundary_loops,
            node_regions.road_surface_polygons,
            node_regions.curb_surface_polygons,
            node_regions.raised_step_faces,
            node_regions.sidewalk_surface_polygons,
            node_regions.explicit_vertical_step_segments,
            node_regions.node_grade_authorities,
            node_regions.node_top_surface_sources,
            node_regions.owned_regions,
            earthwork_owner_sources,
            earthwork_surface_polygons,
            earthwork_outer_boundary_loops,
            render_earthwork_faces,
        )
    }

    fn node_earthwork_owner_sources_from_regions(
        graph: &RegionGraph,
        mouths: &[OrderedIncidentPieceMouth],
        owned_regions: &[NodeOwnedRegion],
        node_top_surface_sources: &[NodeTopSurfacePolygonSource],
    ) -> Vec<NodeEarthworkOwnerSource> {
        let mut sources = Vec::new();
        for (region, top_source) in owned_regions.iter().zip(node_top_surface_sources) {
            let mouth_order_index = top_source.height_field_id.mouth_order_index();
            let Some(mouth) = mouths.get(mouth_order_index) else {
                continue;
            };
            if mouth.edge_idx >= graph.edge_count() {
                continue;
            }
            sources.push(NodeEarthworkOwnerSource {
                owner_kind: region.kind,
                owner_index: region.owner_index,
                mouth_order_index,
                edge_idx: mouth.edge_idx,
                edge_class: graph.edge(mouth.edge_idx).class,
            });
        }
        sources.sort_by(|a, b| {
            a.owner_kind
                .cmp(&b.owner_kind)
                .then(a.owner_index.cmp(&b.owner_index))
                .then(a.mouth_order_index.cmp(&b.mouth_order_index))
                .then(a.edge_idx.cmp(&b.edge_idx))
                .then(edge_class_sort_key(a.edge_class).cmp(&edge_class_sort_key(b.edge_class)))
        });
        sources.dedup_by(|a, b| {
            a.owner_kind == b.owner_kind
                && a.owner_index == b.owner_index
                && a.mouth_order_index == b.mouth_order_index
                && a.edge_idx == b.edge_idx
                && a.edge_class == b.edge_class
        });
        sources
    }

    fn compile_canonical_node_surface_regions(
        &self,
        node_id: u32,
        kind: RoadSurfaceVisualNodePieceKind,
        mouths: &[OrderedIncidentPieceMouth],
    ) -> Option<super::NodeSurfaceRegionResult> {
        let input = match Self::build_node_arrangement_input_from_mouths(node_id, kind, mouths) {
            Ok(input) => input,
            Err(error) => {
                self.log_node_input_extraction_error(node_id, kind, &error);
                return None;
            }
        };
        let rails = match Self::build_node_rail_contours_from_input(&input) {
            Ok(rails) => rails,
            Err(error) => {
                self.log_node_validation_report(&NodeValidationReport::from_rail_generation_error(
                    node_id, kind, &error,
                ));
                return None;
            }
        };
        let ownership = match Self::build_node_boolean_ownership_from_rails(&rails) {
            Ok(ownership) => ownership,
            Err(error) => {
                self.log_node_validation_report(
                    &NodeValidationReport::from_boolean_ownership_error(node_id, kind, &error),
                );
                return None;
            }
        };
        if let Some(report) = NodeValidationReport::from_owned_region_arrangement_diagnostics(
            &ownership.owned_region_arrangement,
        ) {
            self.log_node_validation_report(&report);
            return None;
        }
        let heights =
            match Self::build_node_height_solution_from_ownership(&input, &rails, &ownership) {
                Ok(heights) => heights,
                Err(error) => {
                    self.log_node_validation_report(
                        &NodeValidationReport::from_height_field_error(node_id, kind, &error),
                    );
                    return None;
                }
            };
        let mut arrangement = match NodeArrangement::from_height_solution(&heights) {
            Ok(arrangement) => arrangement,
            Err(error) => {
                self.log_node_validation_report(&NodeValidationReport::from_arrangement_error(
                    node_id, kind, &error,
                ));
                return None;
            }
        };
        if let Some(report) = NodeValidationReport::from_arrangement_diagnostics(&arrangement) {
            self.log_node_validation_report(&report);
            return None;
        }

        let triangulation = match Self::build_node_triangulation_from_arrangement(&arrangement) {
            Ok(triangulation) => triangulation,
            Err(error) => {
                self.log_node_validation_report(&NodeValidationReport::from_triangulation_error(
                    node_id, kind, &error,
                ));
                return None;
            }
        };
        match Self::validate_node_triangulation_solution(&triangulation) {
            Ok(report) => self.log_node_validation_report(&report),
            Err(error) => {
                self.log_node_validation_report(&error.report);
                if error.report.has_blocking_diagnostics() {
                    return None;
                }
            }
        }

        if let Err(error) = arrangement.attach_triangulation(&triangulation) {
            self.log_node_validation_report(&NodeValidationReport::from_arrangement_error(
                node_id, kind, &error,
            ));
            return None;
        }

        match Self::node_surface_regions_from_arrangement(&arrangement, &ownership.footprint_shapes)
        {
            Ok(regions) => Some(regions),
            Err(error) => {
                self.log_node_boundary_export_error(&arrangement, &error);
                None
            }
        }
    }

    fn log_node_input_extraction_error(
        &self,
        node_id: u32,
        kind: RoadSurfaceVisualNodePieceKind,
        error: &NodeInputExtractionError,
    ) {
        if !self.node_validation_logging_enabled {
            return;
        }
        crate::debug_log!(
            "road",
            "node_canonical_input_failed node={} piece={:?} error={:?}",
            node_id,
            kind,
            error
        );
    }

    fn log_node_validation_report(&self, report: &NodeValidationReport) {
        if !self.node_validation_logging_enabled || report.diagnostics.is_empty() {
            return;
        }
        crate::debug_log!("road", "node_canonical_validation {}", report.debug_dump());
    }

    fn log_node_boundary_export_error(
        &self,
        arrangement: &NodeArrangement,
        error: &NodeBoundaryExportError,
    ) {
        if !self.node_validation_logging_enabled {
            return;
        }
        let report = match error {
            NodeBoundaryExportError::MissingFootprintBoundaryHeight { x_key, z_key } => {
                let _ = (*x_key, *z_key);
                NodeValidationReport::from_boundary_export_error(
                    arrangement.node_id(),
                    arrangement.piece_kind(),
                    "missing_footprint_boundary_height",
                )
            }
            NodeBoundaryExportError::ConflictingFootprintBoundaryHeight {
                x_key,
                z_key,
                existing_y_mm,
                incoming_y_mm,
                existing_owner_kind,
                existing_owner_index,
                existing_source,
                incoming_owner_kind,
                incoming_owner_index,
                incoming_source,
            } => {
                let _ = (
                    *x_key,
                    *z_key,
                    *existing_y_mm,
                    *incoming_y_mm,
                    *existing_owner_kind,
                    *existing_owner_index,
                    *existing_source,
                    *incoming_owner_kind,
                    *incoming_owner_index,
                    *incoming_source,
                );
                NodeValidationReport::from_boundary_export_error(
                    arrangement.node_id(),
                    arrangement.piece_kind(),
                    "conflicting_footprint_boundary_height",
                )
            }
            NodeBoundaryExportError::ConflictingFootprintBoundarySplitHeight {
                x_key,
                z_key,
                existing_y_mm,
                incoming_y_mm,
            } => {
                let _ = (*x_key, *z_key, *existing_y_mm, *incoming_y_mm);
                NodeValidationReport::from_boundary_export_error(
                    arrangement.node_id(),
                    arrangement.piece_kind(),
                    "conflicting_footprint_boundary_split_height",
                )
            }
            NodeBoundaryExportError::AmbiguousEarthworkBoundarySegmentSource {
                start_x_key,
                start_z_key,
                end_x_key,
                end_z_key,
                existing_source,
                incoming_source,
            } => {
                let _ = (
                    *start_x_key,
                    *start_z_key,
                    *end_x_key,
                    *end_z_key,
                    *existing_source,
                    *incoming_source,
                );
                NodeValidationReport::from_boundary_export_error(
                    arrangement.node_id(),
                    arrangement.piece_kind(),
                    "ambiguous_earthwork_boundary_segment_source",
                )
            }
            NodeBoundaryExportError::EmptyOuterBoundary => {
                NodeValidationReport::from_boundary_export_error(
                    arrangement.node_id(),
                    arrangement.piece_kind(),
                    "empty_outer_boundary",
                )
            }
            NodeBoundaryExportError::DegenerateOuterBoundaryLoop => {
                NodeValidationReport::from_boundary_export_error(
                    arrangement.node_id(),
                    arrangement.piece_kind(),
                    "degenerate_outer_boundary_loop",
                )
            }
            NodeBoundaryExportError::MissingEarthworkBoundarySource => {
                NodeValidationReport::from_boundary_export_error(
                    arrangement.node_id(),
                    arrangement.piece_kind(),
                    "missing_earthwork_boundary_source",
                )
            }
            NodeBoundaryExportError::MissingNodeTopSurfaceGradeAuthority => {
                NodeValidationReport::from_boundary_export_error(
                    arrangement.node_id(),
                    arrangement.piece_kind(),
                    "missing_node_top_surface_grade_authority",
                )
            }
        };
        self.log_node_validation_report(&report);
    }
}

fn edge_class_sort_key(edge_class: EdgeClass) -> u8 {
    match edge_class {
        EdgeClass::Standard => 0,
        EdgeClass::Bridge => 1,
        EdgeClass::Tunnel => 2,
    }
}
