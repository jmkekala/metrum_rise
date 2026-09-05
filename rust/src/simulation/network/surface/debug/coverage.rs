// SPDX-License-Identifier: GPL-2.0-only

//! Footprint, mouth-seam, and material-coverage debug diagnostics.

use super::*;
use crate::simulation::network::surface::NODE_OVERLAY_NUMERIC_DUST_WIDTH_M;

impl RoadSurfaceSystem {
    pub(super) fn append_material_footprint_coverage_debug_literal(
        dump: &mut String,
        piece: &RoadSurfaceVisualNodePiece,
    ) {
        let footprint_contours =
            Self::debug_overlay_contours_from_polygons(&piece.outer_boundary_loops);
        let Some(mut footprint_shapes) = Self::overlay_union_contours(&footprint_contours) else {
            dump.push_str("{\"status\":\"overlay_failed\"}");
            return;
        };
        Self::sort_overlay_shapes(&mut footprint_shapes);

        let top_contours = Self::debug_overlay_contours_from_top_polygons(
            piece
                .road_surface_polygons
                .iter()
                .chain(piece.curb_surface_polygons.iter())
                .chain(piece.sidewalk_surface_polygons.iter()),
        );
        let Some(mut top_shapes) = Self::overlay_union_contours(&top_contours) else {
            dump.push_str("{\"status\":\"overlay_failed\"}");
            return;
        };
        Self::sort_overlay_shapes(&mut top_shapes);

        let Some(mut missing_shapes) =
            Self::overlay_binary_shapes(&footprint_shapes, &top_shapes, OverlayRule::Difference)
        else {
            dump.push_str("{\"status\":\"overlay_failed\"}");
            return;
        };
        Self::sort_overlay_shapes(&mut missing_shapes);

        let Some(mut extra_shapes) =
            Self::overlay_binary_shapes(&top_shapes, &footprint_shapes, OverlayRule::Difference)
        else {
            dump.push_str("{\"status\":\"overlay_failed\"}");
            return;
        };
        Self::sort_overlay_shapes(&mut extra_shapes);
        let top_edges = Self::debug_owned_top_boundary_edges(piece);
        let missing_boundary_touch_count = missing_shapes
            .iter()
            .filter(|shape| Self::debug_overlay_shape_touches_top_boundary(shape, &top_edges))
            .count();
        let suspicious_missing_shape_count = missing_shapes
            .iter()
            .filter(|shape| Self::debug_missing_footprint_shape_is_suspicious(shape, &top_edges))
            .count();
        let canonical_numeric_dust_missing_shape_count = missing_shapes
            .iter()
            .filter(|shape| Self::debug_missing_footprint_shape_is_canonical_numeric_dust(shape))
            .count();

        let stats = DebugCoverageStats {
            footprint_area_m2: Self::debug_overlay_area_m2(&footprint_shapes),
            top_area_m2: Self::debug_overlay_area_m2(&top_shapes),
            missing_area_m2: Self::debug_overlay_area_m2(&missing_shapes),
            extra_area_m2: Self::debug_overlay_area_m2(&extra_shapes),
            area_budget_m2: Self::overlay_numeric_area_budget_for_shapes(&footprint_shapes)
                .max(Self::overlay_numeric_area_budget_for_shapes(&top_shapes)),
            missing_shape_count: missing_shapes.len(),
            extra_shape_count: extra_shapes.len(),
        };

        dump.push('{');
        let problem = stats.missing_area_m2 > stats.area_budget_m2
            || stats.extra_area_m2 > stats.area_budget_m2
            || suspicious_missing_shape_count > 0;
        let _ = write!(
            dump,
            "\"status\":\"ok\",\"problem\":{},\"footprint_area_m2\":{:.6},\"top_area_m2\":{:.6},\"missing_area_m2\":{:.6},\"extra_area_m2\":{:.6},\"area_budget_m2\":{:.6},\"missing_shape_count\":{},\"missing_boundary_touch_count\":{},\"suspicious_missing_shape_count\":{},\"canonical_numeric_dust_missing_shape_count\":{},\"extra_shape_count\":{}",
            problem,
            stats.footprint_area_m2,
            stats.top_area_m2,
            stats.missing_area_m2,
            stats.extra_area_m2,
            stats.area_budget_m2,
            stats.missing_shape_count,
            missing_boundary_touch_count,
            suspicious_missing_shape_count,
            canonical_numeric_dust_missing_shape_count,
            stats.extra_shape_count
        );
        dump.push_str(",\"missing_samples\":[");
        Self::append_overlay_shape_samples(dump, &missing_shapes);
        dump.push_str("],\"extra_samples\":[");
        Self::append_overlay_shape_samples(dump, &extra_shapes);
        dump.push_str("]}");
    }

    fn debug_missing_footprint_shape_is_suspicious(
        shape: &NodeOverlayShape,
        top_edges: &[DebugTopBoundaryEdge],
    ) -> bool {
        Self::debug_overlay_shape_touches_top_boundary(shape, top_edges)
            && Self::overlay_shape_area_m2(shape) > f32::EPSILON
            && !Self::debug_missing_footprint_shape_is_canonical_numeric_dust(shape)
    }

    fn debug_missing_footprint_shape_is_canonical_numeric_dust(shape: &NodeOverlayShape) -> bool {
        Self::debug_overlay_shape_thin_width_m(shape) <= NODE_OVERLAY_NUMERIC_DUST_WIDTH_M
            && Self::overlay_shape_area_m2(shape)
                <= Self::debug_overlay_shape_numeric_area_budget_m2(shape)
    }

    fn debug_overlay_shape_numeric_area_budget_m2(shape: &NodeOverlayShape) -> f32 {
        let perimeter_m = shape
            .iter()
            .map(|contour| Self::overlay_contour_perimeter_m(contour))
            .sum::<f32>();
        let vertex_count = shape.iter().map(Vec::len).sum::<usize>();
        Self::overlay_numeric_area_budget_m2(perimeter_m, vertex_count)
    }

    fn debug_overlay_shape_touches_top_boundary(
        shape: &NodeOverlayShape,
        top_edges: &[DebugTopBoundaryEdge],
    ) -> bool {
        shape
            .iter()
            .flat_map(|contour| contour.iter().copied())
            .any(|point| {
                let point = SurfaceXzKey::from_overlay_point(point);
                top_edges.iter().copied().any(|edge| {
                    let start = SurfaceXzKey::from_raw_keys(
                        edge.xz_key.start.x_key,
                        edge.xz_key.start.z_key,
                    );
                    let end =
                        SurfaceXzKey::from_raw_keys(edge.xz_key.end.x_key, edge.xz_key.end.z_key);
                    Self::debug_top_edge_vertex_parameter(point, start, end)
                        .is_some_and(|parameter| (-0.001..=1.001).contains(&parameter.as_f64()))
                })
            })
    }

    pub(super) fn append_outer_boundary_top_match_debug_literal(
        &self,
        dump: &mut String,
        _terrain: &TerrainSystem,
        piece: &RoadSurfaceVisualNodePiece,
    ) {
        let mut edge_count = 0_usize;
        let mut vertex_count = 0_usize;
        let mut direct_source_count = 0_usize;
        let mut boundary_interpolation_source_count = 0_usize;
        let mut missing_source_count = 0_usize;
        let mut non_node_source_count = 0_usize;
        let mut samples = Vec::new();
        for (loop_index, boundary_loop) in piece.terrain_clip_boundary_loops.iter().enumerate() {
            for (edge_index, edge) in boundary_loop.source_edges.iter().enumerate() {
                edge_count += 1;
                let RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
                    owner_kind,
                    owner_index,
                    boundary_source,
                    ..
                } = edge.source
                else {
                    non_node_source_count += 1;
                    continue;
                };
                for (endpoint, point, source) in [
                    (
                        "start",
                        edge.start,
                        boundary_source.map(|source| source.start),
                    ),
                    ("end", edge.end, boundary_source.map(|source| source.end)),
                ] {
                    vertex_count += 1;
                    match source {
                        Some(NodeFootprintBoundaryVertexSource::Direct(_)) => {
                            direct_source_count += 1;
                        }
                        Some(NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint {
                            ..
                        }) => {}
                        Some(NodeFootprintBoundaryVertexSource::BoundaryInterpolation {
                            ..
                        }) => {
                            boundary_interpolation_source_count += 1;
                        }
                        None => {
                            missing_source_count += 1;
                        }
                    }
                    if (source.is_none()
                        || matches!(
                            source,
                            Some(NodeFootprintBoundaryVertexSource::BoundaryInterpolation { .. })
                        ))
                        && samples.len() < DEBUG_MAX_PROBLEM_SAMPLES
                    {
                        let mut sample = String::new();
                        let _ = write!(
                            sample,
                            "{{\"loop\":{},\"edge\":{},\"endpoint\":\"{}\",\"owner_kind\":\"{:?}\",\"owner_index\":{},\"boundary\":",
                            loop_index, edge_index, endpoint, owner_kind, owner_index
                        );
                        Self::append_vector3_literal(&mut sample, point);
                        sample.push_str(",\"boundary_key\":");
                        Self::append_node_boundary_key_debug_literal(&mut sample, point);
                        sample.push_str(",\"source\":");
                        if let Some(source) = source {
                            Self::append_node_footprint_boundary_vertex_source_debug_literal(
                                &mut sample,
                                source,
                            );
                        } else {
                            sample.push_str("null");
                        }
                        sample.push('}');
                        samples.push(sample);
                    }
                }
            }
        }
        let _ = write!(
            dump,
            "{{\"edge_count\":{},\"vertex_count\":{},\"direct_source_count\":{},\"boundary_interpolation_source_count\":{},\"missing_source_count\":{},\"non_node_source_count\":{},\"samples\":[",
            edge_count,
            vertex_count,
            direct_source_count,
            boundary_interpolation_source_count,
            missing_source_count,
            non_node_source_count
        );
        Self::append_raw_json_samples(dump, &samples);
        dump.push_str("]}");
    }

    pub(super) fn append_mouth_seam_debug_literal(
        &self,
        dump: &mut String,
        graph: &RegionGraph,
        node_id: u32,
        piece: &RoadSurfaceVisualNodePiece,
    ) {
        dump.push('[');
        let incident_edges = self.debug_incident_edges_for_node(graph, node_id);
        let mut first = true;
        for edge_idx in incident_edges {
            if edge_idx >= graph.edge_count() {
                continue;
            }
            let edge = graph.edge(edge_idx);
            if edge.deleted {
                continue;
            }
            let start_node = graph.get_valid_node(edge.start_node);
            let side = if start_node == node_id {
                IncidentEdgeSide::Start
            } else {
                IncidentEdgeSide::End
            };
            let Some(span_piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                continue;
            };
            let mouth = match side {
                IncidentEdgeSide::Start => span_piece.start_mouth_profile.as_ref(),
                IncidentEdgeSide::End => span_piece.end_mouth_profile.as_ref(),
            };
            let Some(mouth) = mouth else {
                continue;
            };

            if !first {
                dump.push_str(", ");
            }
            first = false;

            let mut stats = DebugMatchStats::default();
            let mut samples = Vec::new();
            for anchor in Self::mouth_top_match_anchors(mouth) {
                let Some(closest) = Self::closest_debug_top_support_for_material(
                    anchor.point,
                    anchor.material,
                    piece,
                ) else {
                    continue;
                };
                Self::update_debug_match_stats(&mut stats, closest);
                if Self::debug_match_is_problem(closest)
                    && samples.len() < DEBUG_MAX_PROBLEM_SAMPLES
                {
                    let mut sample = String::new();
                    let _ = write!(
                        sample,
                        "{{\"point\":{},\"band\":{},\"role\":\"{}\",\"expected_material\":\"{}\",\"mouth\":",
                        anchor.point_index, anchor.band_index, anchor.role, anchor.material
                    );
                    Self::append_vector3_literal(&mut sample, anchor.point);
                    sample.push_str(",\"closest_material\":\"");
                    sample.push_str(closest.material);
                    sample.push_str("\",\"closest\":");
                    Self::append_vector3_literal(&mut sample, closest.point);
                    let _ = write!(
                        sample,
                        ",\"xz_error_m\":{:.4},\"y_delta_m\":{:.4}}}",
                        closest.xz_error_m, closest.y_delta_m
                    );
                    samples.push(sample);
                }
            }

            let _ = write!(
                dump,
                "{{\"edge_idx\":{},\"side\":\"{:?}\",\"mouth_vertex_count\":{},",
                edge_idx,
                side,
                mouth.boundary_points_world.len()
            );
            Self::append_match_stats_fields(dump, &stats);
            dump.push_str(",\"samples\":[");
            Self::append_raw_json_samples(dump, &samples);
            dump.push_str("]}");
        }
        dump.push(']');
    }

    pub(super) fn append_earthwork_face_top_match_debug_literal(
        &self,
        dump: &mut String,
        _terrain: &TerrainSystem,
        piece: &RoadSurfaceVisualNodePiece,
    ) {
        let mut samples = Vec::new();
        let mut slope_count = 0_usize;
        let mut retaining_wall_count = 0_usize;
        let mut span_support_source_count = 0_usize;
        let mut node_footprint_source_count = 0_usize;
        let mut direct_source_count = 0_usize;
        let mut boundary_interpolation_source_count = 0_usize;
        let mut missing_source_count = 0_usize;
        for (face_index, face) in piece.render_earthwork_faces.iter().enumerate() {
            match face.kind {
                RoadSurfaceEarthworkFaceKind::Slope => slope_count += 1,
                RoadSurfaceEarthworkFaceKind::RetainingWall => retaining_wall_count += 1,
            }
            match face.source {
                RoadSurfaceEarthworkFaceSource::SpanSupportBoundary { .. } => {
                    span_support_source_count += 1
                }
                RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
                    boundary_source, ..
                }
                | RoadSurfaceEarthworkFaceSource::NodeSameMaterialBoundaryHandoff {
                    boundary_source,
                    ..
                } => {
                    node_footprint_source_count += 1;
                    for source in [
                        boundary_source.map(|source| source.start),
                        boundary_source.map(|source| source.end),
                    ] {
                        match source {
                            Some(NodeFootprintBoundaryVertexSource::Direct(_)) => {
                                direct_source_count += 1;
                            }
                            Some(NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint {
                                ..
                            }) => {}
                            Some(NodeFootprintBoundaryVertexSource::BoundaryInterpolation {
                                ..
                            }) => {
                                boundary_interpolation_source_count += 1;
                            }
                            None => {
                                missing_source_count += 1;
                            }
                        }
                    }
                }
            }
            if matches!(
                face.source,
                RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
                    boundary_source: None,
                    ..
                } | RoadSurfaceEarthworkFaceSource::NodeSameMaterialBoundaryHandoff {
                    boundary_source: None,
                    ..
                }
            ) && samples.len() < DEBUG_MAX_PROBLEM_SAMPLES
            {
                let mut sample = String::new();
                let _ = write!(
                    sample,
                    "{{\"face\":{},\"kind\":\"{:?}\",\"source\":",
                    face_index, face.kind
                );
                Self::append_earthwork_face_source_debug_literal(&mut sample, face.source);
                sample.push_str(",\"inner_start\":");
                Self::append_vector3_literal(&mut sample, face.inner_start);
                sample.push_str(",\"inner_start_key\":");
                Self::append_node_boundary_key_debug_literal(&mut sample, face.inner_start);
                sample.push_str(",\"inner_end\":");
                Self::append_vector3_literal(&mut sample, face.inner_end);
                sample.push_str(",\"inner_end_key\":");
                Self::append_node_boundary_key_debug_literal(&mut sample, face.inner_end);
                sample.push('}');
                samples.push(sample);
            }
        }

        dump.push('{');
        let _ = write!(
            dump,
            "\"face_count\":{},\"slope_count\":{},\"retaining_wall_count\":{},\"span_support_source_count\":{},\"node_footprint_source_count\":{},\"direct_source_count\":{},\"boundary_interpolation_source_count\":{},\"missing_source_count\":{},",
            piece.render_earthwork_faces.len(),
            slope_count,
            retaining_wall_count,
            span_support_source_count,
            node_footprint_source_count,
            direct_source_count,
            boundary_interpolation_source_count,
            missing_source_count
        );
        dump.push_str("\"samples\":[");
        Self::append_raw_json_samples(dump, &samples);
        dump.push_str("]}");
    }

    pub(super) fn debug_band_kind_order() -> [RoadSurfaceBandKind; 8] {
        [
            RoadSurfaceBandKind::Carriageway,
            RoadSurfaceBandKind::CurbOrShoulder,
            RoadSurfaceBandKind::Sidewalk,
            RoadSurfaceBandKind::Footpath,
            RoadSurfaceBandKind::Median,
            RoadSurfaceBandKind::Parking,
            RoadSurfaceBandKind::CycleTrack,
            RoadSurfaceBandKind::TramReservation,
        ]
    }

    pub(super) fn mouth_top_match_anchors(
        mouth: &IncidentMouthProfile,
    ) -> Vec<DebugMouthTopAnchor> {
        let mut anchors = Vec::new();
        if mouth.bands.is_empty() || mouth.boundary_points_world.len() != mouth.bands.len() + 1 {
            return anchors;
        }

        for point_index in 0..mouth.boundary_points_world.len() {
            if Self::mouth_boundary_point_is_outer_footprint(mouth, point_index) {
                let (band_index, point) = if point_index == 0 {
                    (0, mouth.bands[0].start_point_world)
                } else {
                    let band_index = mouth.bands.len() - 1;
                    (band_index, mouth.bands[band_index].end_point_world)
                };
                anchors.push(DebugMouthTopAnchor {
                    point_index,
                    band_index,
                    role: "outer_footprint",
                    material: Self::debug_material_for_band_kind(mouth.bands[band_index].kind),
                    point,
                });
                continue;
            }

            if !Self::mouth_boundary_point_is_material_seam(mouth, point_index) {
                continue;
            }

            let before_index = point_index - 1;
            let after_index = point_index;
            anchors.push(DebugMouthTopAnchor {
                point_index,
                band_index: before_index,
                role: "material_seam_before",
                material: Self::debug_material_for_band_kind(mouth.bands[before_index].kind),
                point: mouth.bands[before_index].end_point_world,
            });
            anchors.push(DebugMouthTopAnchor {
                point_index,
                band_index: after_index,
                role: "material_seam_after",
                material: Self::debug_material_for_band_kind(mouth.bands[after_index].kind),
                point: mouth.bands[after_index].start_point_world,
            });
        }
        anchors
    }

    pub(super) fn debug_material_for_band_kind(kind: RoadSurfaceBandKind) -> &'static str {
        match kind {
            RoadSurfaceBandKind::Carriageway => "road",
            RoadSurfaceBandKind::CurbOrShoulder => "curb",
            _ => "sidewalk",
        }
    }

    pub(super) fn mouth_boundary_point_is_outer_footprint(
        mouth: &IncidentMouthProfile,
        point_index: usize,
    ) -> bool {
        point_index == 0 || point_index + 1 == mouth.boundary_points_world.len()
    }

    pub(super) fn mouth_boundary_point_is_material_seam(
        mouth: &IncidentMouthProfile,
        point_index: usize,
    ) -> bool {
        if point_index == 0 || point_index >= mouth.boundary_points_world.len().saturating_sub(1) {
            return false;
        }
        let Some(before) = mouth.bands.get(point_index - 1) else {
            return false;
        };
        let Some(after) = mouth.bands.get(point_index) else {
            return false;
        };
        before.kind != after.kind
    }
}
