// SPDX-License-Identifier: GPL-2.0-only

//! Raised-step face detail debug literal writers.

use super::*;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface::debug) fn append_raised_step_face_details_debug_literal(
        dump: &mut String,
        piece: &RoadSurfaceVisualNodePiece,
    ) {
        let top_edges = Self::debug_owned_top_boundary_edges(piece);
        let expected_steps = Self::debug_expected_raised_steps(&top_edges);
        let canonical_steps = Self::debug_canonical_raised_steps(piece, &top_edges);

        let face_span_edges: Vec<Option<DebugVerticalFaceSpanEdges>> = piece
            .raised_step_face_polygons
            .iter()
            .map(Self::debug_vertical_face_span_edges)
            .collect();
        let mut face_expected_matches = vec![Vec::new(); face_span_edges.len()];
        let mut expected_face_matches = vec![Vec::new(); expected_steps.len()];
        let mut face_canonical_matches = vec![Vec::new(); face_span_edges.len()];
        let mut canonical_face_matches = vec![Vec::new(); canonical_steps.len()];
        let canonical_step_indices_by_source: BTreeMap<
            (usize, NodeExplicitVerticalStepSegment),
            usize,
        > = canonical_steps
            .iter()
            .enumerate()
            .map(|(step_index, step)| {
                (
                    (step.explicit_vertical_step_index, step.segment),
                    step_index,
                )
            })
            .collect();
        for (face_index, source) in piece.raised_step_face_sources.iter().copied().enumerate() {
            if face_index >= face_canonical_matches.len() {
                continue;
            }
            let Some(explicit_vertical_step_index) = source.explicit_vertical_step_index() else {
                continue;
            };
            let segment = source.segment();
            if let Some(&canonical_step_index) =
                canonical_step_indices_by_source.get(&(explicit_vertical_step_index, segment))
            {
                face_canonical_matches[face_index].push(canonical_step_index);
                canonical_face_matches[canonical_step_index].push(face_index);
            }
        }

        for (face_index, span_edges) in face_span_edges.iter().enumerate() {
            let Some(span_edges) = span_edges else {
                continue;
            };
            let Some(lower_key) =
                DebugRenderEdgeKey::normalized(span_edges.lower_start, span_edges.lower_end)
            else {
                continue;
            };
            let Some(upper_key) =
                DebugRenderEdgeKey::normalized(span_edges.upper_start, span_edges.upper_end)
            else {
                continue;
            };
            for (step_index, step) in expected_steps.iter().enumerate() {
                if step.lower.key == lower_key && step.upper.key == upper_key {
                    face_expected_matches[face_index].push(step_index);
                    expected_face_matches[step_index].push(face_index);
                }
            }
        }
        let coverage_report =
            Self::debug_raised_step_coverage_report(piece, &top_edges, &face_span_edges);

        let mut face_problem_count = 0usize;
        for (face_index, span_edges) in face_span_edges.iter().enumerate() {
            let Some(span_edges) = span_edges else {
                face_problem_count += 1;
                continue;
            };
            let lower_matches = Self::debug_top_edges_containing_span(
                &top_edges,
                span_edges.lower_start,
                span_edges.lower_end,
            );
            let upper_matches = Self::debug_top_edges_containing_span(
                &top_edges,
                span_edges.upper_start,
                span_edges.upper_end,
            );
            let matches_raised_step_owner_pair =
                Self::debug_top_matches_form_raised_step_owner_pair(&lower_matches, &upper_matches);
            let visible_dot = Self::debug_polygon_winding_normal(
                &piece.raised_step_face_polygons[face_index].points_world,
            )
            .map(|normal| -normal)
            .and_then(|direction| {
                Self::debug_visible_dot_to_lower_raised_step_owner(
                    piece,
                    (span_edges.lower_start + span_edges.lower_end) * 0.5,
                    direction,
                    &lower_matches,
                    &upper_matches,
                )
            });
            let visible_from_lower_owner = visible_dot.is_some_and(|dot| dot > 0.0);
            let face_problem = !matches_raised_step_owner_pair
                || (face_expected_matches[face_index].is_empty()
                    && face_canonical_matches[face_index].is_empty())
                || !visible_from_lower_owner;
            if face_problem {
                face_problem_count += 1;
            }
        }
        let missing_required_face_count = expected_face_matches
            .iter()
            .filter(|matches| matches.is_empty())
            .count();
        let duplicate_raised_step_face_count = Self::debug_duplicate_raised_step_face_count(
            &face_span_edges,
            &piece.raised_step_face_sources,
        );
        let final_required_problem_count = face_problem_count
            + missing_required_face_count
            + coverage_report.required_gap_count
            + duplicate_raised_step_face_count;
        let non_exposed_source_constraint_count = canonical_face_matches
            .iter()
            .filter(|matches| matches.is_empty())
            .count();
        let canonical_problem_count = canonical_steps
            .iter()
            .zip(&canonical_face_matches)
            .filter(|(step, matches)| {
                !matches.is_empty()
                    && !Self::debug_canonical_step_visible_from_lower_owner(
                        piece,
                        step,
                        matches,
                        &face_span_edges,
                        &top_edges,
                    )
                    .unwrap_or(false)
            })
            .count();
        dump.push('{');
        let _ = write!(
            dump,
            "\"face_count\":{},\"emitted_face_count\":{},\"top_boundary_edge_count\":{},\"expected_raised_step_count\":{},\"final_required_face_count\":{},\"required_interval_count\":{},\"missing_required_face_count\":{},\"required_gap_count\":{},\"missing_length_m\":{:.6},\"face_problem_count\":{},\"duplicate_raised_step_face_count\":{},\"final_required_problem_count\":{},\"canonical_raised_step_count\":{},\"source_constraint_count\":{},\"non_exposed_source_constraint_count\":{},\"canonical_raised_step_problem_count\":{},\"problem_count\":{}",
            piece.raised_step_face_polygons.len(),
            piece.raised_step_face_polygons.len(),
            top_edges.len(),
            expected_steps.len(),
            expected_steps.len(),
            coverage_report.required_interval_count,
            missing_required_face_count,
            coverage_report.required_gap_count,
            coverage_report.missing_length_m,
            face_problem_count,
            duplicate_raised_step_face_count,
            final_required_problem_count,
            canonical_steps.len(),
            canonical_steps.len(),
            non_exposed_source_constraint_count,
            canonical_problem_count,
            final_required_problem_count
        );
        dump.push_str(",\"required_gap_samples\":[");
        for (index, sample) in coverage_report.samples.iter().enumerate() {
            if index > 0 {
                dump.push_str(", ");
            }
            Self::append_raised_step_coverage_gap_literal(dump, sample);
        }
        dump.push_str("],\"faces\":[");
        for (face_index, polygon) in piece.raised_step_face_polygons.iter().enumerate() {
            if face_index > 0 {
                dump.push_str(", ");
            }
            Self::append_raised_step_face_detail_literal(
                dump,
                piece,
                face_index,
                polygon,
                piece.raised_step_face_sources.get(face_index).copied(),
                face_span_edges[face_index],
                &top_edges,
                &face_expected_matches[face_index],
                &face_canonical_matches[face_index],
            );
        }
        dump.push_str("],\"expected_raised_steps\":[");
        for (step_index, step) in expected_steps.iter().enumerate() {
            if step_index > 0 {
                dump.push_str(", ");
            }
            Self::append_expected_vertical_step_literal(
                dump,
                step_index,
                *step,
                &expected_face_matches[step_index],
            );
        }
        dump.push_str("],\"canonical_raised_steps\":[");
        for (step_index, step) in canonical_steps.iter().enumerate() {
            if step_index > 0 {
                dump.push_str(", ");
            }
            Self::append_canonical_vertical_step_literal(
                dump,
                piece,
                step_index,
                step,
                &canonical_face_matches[step_index],
                &face_span_edges,
                &top_edges,
            );
        }
        dump.push_str("]}");
    }

    fn debug_duplicate_raised_step_face_count(
        face_span_edges: &[Option<DebugVerticalFaceSpanEdges>],
        sources: &[RoadSurfaceVerticalFaceSource],
    ) -> usize {
        let mut seen = BTreeSet::<DebugRenderedRaisedStepFaceKey>::new();
        let mut duplicate_count = 0usize;
        for (span_edges, source) in face_span_edges.iter().copied().zip(sources.iter().copied()) {
            let Some(span_edges) = span_edges else {
                continue;
            };
            let Some((lower_owner, raised_owner)) = source.lower_and_raised_owners() else {
                continue;
            };
            let Some(lower_edge) =
                DebugRenderedEdgeMmKey::normalized(span_edges.lower_start, span_edges.lower_end)
            else {
                continue;
            };
            let Some(upper_edge) =
                DebugRenderedEdgeMmKey::normalized(span_edges.upper_start, span_edges.upper_end)
            else {
                continue;
            };
            let key = DebugRenderedRaisedStepFaceKey {
                lower_owner,
                raised_owner,
                lower_edge,
                upper_edge,
            };
            if !seen.insert(key) {
                duplicate_count += 1;
            }
        }
        duplicate_count
    }

    pub(in crate::simulation::network::surface::debug) fn append_raised_step_face_detail_literal(
        dump: &mut String,
        piece: &RoadSurfaceVisualNodePiece,
        face_index: usize,
        polygon: &RoadSurfaceVisualPolygon,
        source: Option<RoadSurfaceVerticalFaceSource>,
        span_edges: Option<DebugVerticalFaceSpanEdges>,
        top_edges: &[DebugTopBoundaryEdge],
        expected_step_matches: &[usize],
        canonical_step_matches: &[usize],
    ) {
        let normal = Self::debug_polygon_winding_normal(&polygon.points_world);
        let visible_direction = normal.map(|normal| -normal);

        dump.push('{');
        let _ = write!(
            dump,
            "\"face\":{},\"polygon_vertex_count\":{},\"triangle_count\":{}",
            face_index,
            polygon.points_world.len(),
            polygon.triangles_world.len()
        );
        dump.push_str(",\"points_world\":");
        Self::append_vector3_precise_list_literal(dump, &polygon.points_world);
        dump.push_str(",\"winding_normal\":");
        Self::append_optional_vector3_precise_literal(dump, normal);
        dump.push_str(",\"godot_cull_back_visible_direction\":");
        Self::append_optional_vector3_precise_literal(dump, visible_direction);
        Self::append_raised_step_face_source_literal(dump, source);

        let Some(span_edges) = span_edges else {
            dump.push_str(",\"status\":\"non_vertical_quad_span\"}");
            return;
        };

        let lower_key =
            DebugRenderEdgeKey::normalized(span_edges.lower_start, span_edges.lower_end);
        let upper_key =
            DebugRenderEdgeKey::normalized(span_edges.upper_start, span_edges.upper_end);
        dump.push_str(",\"lower_edge_world\":");
        Self::append_vector3_pair_precise_literal(
            dump,
            span_edges.lower_start,
            span_edges.lower_end,
        );
        dump.push_str(",\"upper_edge_world\":");
        Self::append_vector3_pair_precise_literal(
            dump,
            span_edges.upper_start,
            span_edges.upper_end,
        );
        dump.push_str(",\"lower_edge_key\":");
        Self::append_optional_debug_render_edge_key_literal(dump, lower_key);
        dump.push_str(",\"upper_edge_key\":");
        Self::append_optional_debug_render_edge_key_literal(dump, upper_key);

        let lower_matches = Self::debug_top_edges_containing_span(
            top_edges,
            span_edges.lower_start,
            span_edges.lower_end,
        );
        let upper_matches = Self::debug_top_edges_containing_span(
            top_edges,
            span_edges.upper_start,
            span_edges.upper_end,
        );
        dump.push_str(",\"lower_top_matches\":");
        Self::append_debug_top_boundary_edge_list_literal(dump, &lower_matches);
        dump.push_str(",\"upper_top_matches\":");
        Self::append_debug_top_boundary_edge_list_literal(dump, &upper_matches);
        dump.push_str(",\"matching_expected_step_indices\":");
        Self::append_usize_list_literal(dump, expected_step_matches);
        dump.push_str(",\"matching_canonical_step_indices\":");
        Self::append_usize_list_literal(dump, canonical_step_matches);

        let lower_midpoint = (span_edges.lower_start + span_edges.lower_end) * 0.5;
        let visible_dot = visible_direction.and_then(|direction| {
            Self::debug_visible_dot_to_lower_raised_step_owner(
                piece,
                lower_midpoint,
                direction,
                &lower_matches,
                &upper_matches,
            )
        });
        dump.push_str(",\"visible_dot_lower_owner\":");
        Self::append_optional_f32_precise_literal(dump, visible_dot);
        dump.push_str(",\"visible_from_lower_owner\":");
        if let Some(dot) = visible_dot {
            let _ = write!(dump, "{}", dot > 0.0);
        } else {
            dump.push_str("null");
        }

        let matches_raised_step_owner_pair =
            Self::debug_top_matches_form_raised_step_owner_pair(&lower_matches, &upper_matches);
        let face_problem = !matches_raised_step_owner_pair
            || (expected_step_matches.is_empty() && canonical_step_matches.is_empty())
            || visible_dot.is_none_or(|dot| dot <= 0.0);
        let _ = write!(
            dump,
            ",\"matches_raised_step_owner_pair\":{},\"problem\":{}",
            matches_raised_step_owner_pair, face_problem
        );
        dump.push('}');
    }

    fn debug_raised_step_coverage_report(
        piece: &RoadSurfaceVisualNodePiece,
        top_edges: &[DebugTopBoundaryEdge],
        face_span_edges: &[Option<DebugVerticalFaceSpanEdges>],
    ) -> DebugRaisedStepCoverageReport {
        let mut report = DebugRaisedStepCoverageReport {
            required_interval_count: 0,
            required_gap_count: 0,
            missing_length_m: 0.0,
            samples: Vec::new(),
        };

        for (left_index, left_edge) in top_edges.iter().copied().enumerate() {
            for right_edge in top_edges.iter().copied().skip(left_index + 1) {
                let Some((lower, raised)) =
                    Self::debug_final_top_edges_form_raised_step_pair(left_edge, right_edge)
                else {
                    continue;
                };
                if lower.avg_y_m + SAMPLE_EPSILON_M >= raised.avg_y_m {
                    continue;
                }
                let Some((required_start_t, required_end_t)) =
                    Self::debug_top_boundary_overlap_interval(lower, raised)
                else {
                    continue;
                };
                report.required_interval_count += 1;
                let length_m = Self::debug_top_boundary_edge_length_m(lower);
                if length_m <= f32::EPSILON {
                    continue;
                }
                let covered_intervals = Self::debug_raised_step_face_coverage_intervals(
                    lower,
                    raised,
                    (required_start_t, required_end_t),
                    face_span_edges,
                );
                let missing_intervals = Self::debug_missing_coverage_intervals(
                    (required_start_t, required_end_t),
                    &covered_intervals,
                    length_m,
                );
                for (missing_start_t, missing_end_t) in missing_intervals {
                    let missing_length_m =
                        ((missing_end_t - missing_start_t) * f64::from(length_m)) as f32;
                    report.required_gap_count += 1;
                    report.missing_length_m += missing_length_m;
                    if report.samples.len() >= DEBUG_MAX_PROBLEM_SAMPLES {
                        continue;
                    }
                    let lower_start =
                        Self::debug_top_boundary_edge_point_at_parameter(lower, missing_start_t);
                    let lower_end =
                        Self::debug_top_boundary_edge_point_at_parameter(lower, missing_end_t);
                    let raised_start =
                        Self::debug_top_boundary_edge_point_at_parameter(raised, missing_start_t);
                    let raised_end =
                        Self::debug_top_boundary_edge_point_at_parameter(raised, missing_end_t);
                    let source = Self::debug_raised_step_coverage_source(
                        piece,
                        lower,
                        raised,
                        missing_start_t,
                        missing_end_t,
                    );
                    let gap_midpoint = (Self::debug_top_boundary_edge_point_at_parameter(
                        lower,
                        (missing_start_t + missing_end_t) * 0.5,
                    ) + Self::debug_top_boundary_edge_point_at_parameter(
                        raised,
                        (missing_start_t + missing_end_t) * 0.5,
                    )) * 0.5;
                    let nearest_face =
                        Self::debug_nearest_raised_step_face(gap_midpoint, face_span_edges);
                    report.samples.push(DebugRaisedStepCoverageGap {
                        lower,
                        raised,
                        missing_start_t,
                        missing_end_t,
                        lower_start,
                        lower_end,
                        raised_start,
                        raised_end,
                        missing_length_m,
                        source,
                        nearest_face,
                    });
                }
            }
        }

        report
    }

    fn debug_final_top_edges_form_raised_step_pair(
        left_edge: DebugTopBoundaryEdge,
        right_edge: DebugTopBoundaryEdge,
    ) -> Option<(DebugTopBoundaryEdge, DebugTopBoundaryEdge)> {
        let (lower_kind, raised_kind) =
            ordered_raised_step_kinds(left_edge.owner.kind, right_edge.owner.kind)?;
        if left_edge.owner.kind == lower_kind && right_edge.owner.kind == raised_kind {
            Some((left_edge, right_edge))
        } else {
            Some((right_edge, left_edge))
        }
    }

    fn debug_top_boundary_overlap_interval(
        lower: DebugTopBoundaryEdge,
        raised: DebugTopBoundaryEdge,
    ) -> Option<(f64, f64)> {
        let lower_start = Self::debug_surface_xz_key(lower.key.start.xz());
        let lower_end = Self::debug_surface_xz_key(lower.key.end.xz());
        let raised_start = Self::debug_surface_xz_key(raised.key.start.xz());
        let raised_end = Self::debug_surface_xz_key(raised.key.end.xz());
        let raised_start_t =
            Self::debug_top_edge_vertex_parameter(raised_start, lower_start, lower_end)?;
        let raised_end_t =
            Self::debug_top_edge_vertex_parameter(raised_end, lower_start, lower_end)?;
        let start = raised_start_t.as_f64().min(raised_end_t.as_f64()).max(0.0);
        let end = raised_start_t.as_f64().max(raised_end_t.as_f64()).min(1.0);
        (end > start).then_some((start, end))
    }

    fn debug_raised_step_face_coverage_intervals(
        lower: DebugTopBoundaryEdge,
        raised: DebugTopBoundaryEdge,
        required: (f64, f64),
        face_span_edges: &[Option<DebugVerticalFaceSpanEdges>],
    ) -> Vec<(f64, f64)> {
        let mut intervals = Vec::new();
        for span_edges in face_span_edges.iter().copied().flatten() {
            let Some(lower_key) =
                DebugRenderEdgeKey::normalized(span_edges.lower_start, span_edges.lower_end)
            else {
                continue;
            };
            let Some(upper_key) =
                DebugRenderEdgeKey::normalized(span_edges.upper_start, span_edges.upper_end)
            else {
                continue;
            };
            if !Self::debug_top_edge_contains_span_at_height(lower, lower_key)
                || !Self::debug_top_edge_contains_span_at_height(raised, upper_key)
            {
                continue;
            }
            let Some((face_start_t, face_end_t)) = Self::debug_span_interval_on_top_edge(
                lower,
                span_edges.lower_start,
                span_edges.lower_end,
            ) else {
                continue;
            };
            let start = face_start_t.min(face_end_t).max(required.0);
            let end = face_start_t.max(face_end_t).min(required.1);
            if end > start {
                intervals.push((start, end));
            }
        }
        intervals
            .sort_by(|left, right| left.0.total_cmp(&right.0).then(left.1.total_cmp(&right.1)));
        intervals
    }

    fn debug_span_interval_on_top_edge(
        edge: DebugTopBoundaryEdge,
        start: backend::RoadVec3,
        end: backend::RoadVec3,
    ) -> Option<(f64, f64)> {
        let edge_start = Self::debug_surface_xz_key(edge.key.start.xz());
        let edge_end = Self::debug_surface_xz_key(edge.key.end.xz());
        let start_key = SurfaceXzKey::from_world_xz(start);
        let end_key = SurfaceXzKey::from_world_xz(end);
        let start_t = Self::debug_top_edge_vertex_parameter(start_key, edge_start, edge_end)?;
        let end_t = Self::debug_top_edge_vertex_parameter(end_key, edge_start, edge_end)?;
        Some((start_t.as_f64(), end_t.as_f64()))
    }

    fn debug_missing_coverage_intervals(
        required: (f64, f64),
        covered: &[(f64, f64)],
        length_m: f32,
    ) -> Vec<(f64, f64)> {
        let tolerance_t = (0.001 / length_m).max(1.0e-7) as f64;
        let mut cursor = required.0;
        let mut missing = Vec::new();
        for &(start, end) in covered {
            if end <= cursor + tolerance_t {
                continue;
            }
            if start > cursor + tolerance_t {
                missing.push((cursor, start));
            }
            cursor = cursor.max(end);
            if cursor + tolerance_t >= required.1 {
                return missing;
            }
        }
        if cursor + tolerance_t < required.1 {
            missing.push((cursor, required.1));
        }
        missing
    }

    fn debug_raised_step_coverage_source(
        piece: &RoadSurfaceVisualNodePiece,
        lower: DebugTopBoundaryEdge,
        raised: DebugTopBoundaryEdge,
        start_t: f64,
        end_t: f64,
    ) -> DebugRaisedStepCoverageSource {
        let lower_owner = NodeBandOwner::new(lower.owner.kind, lower.owner.owner_index);
        let raised_owner = NodeBandOwner::new(raised.owner.kind, raised.owner.owner_index);
        let span_start = Self::debug_top_boundary_xz_at_parameter(lower, start_t);
        let span_end = Self::debug_top_boundary_xz_at_parameter(lower, end_t);
        for (explicit_vertical_step_index, segment) in piece
            .explicit_vertical_step_segments
            .iter()
            .copied()
            .enumerate()
        {
            let Some((source_lower_owner, source_raised_owner)) =
                Self::debug_canonical_step_lower_and_raised_owners(segment)
            else {
                continue;
            };
            let source = if source_lower_owner == lower_owner && source_raised_owner == raised_owner
            {
                DebugRaisedStepCoverageSource::Canonical {
                    explicit_vertical_step_index,
                    segment,
                }
            } else if source_lower_owner.kind() == lower_owner.kind()
                && source_raised_owner.kind() == raised_owner.kind()
                && (source_lower_owner == lower_owner || source_raised_owner == raised_owner)
            {
                DebugRaisedStepCoverageSource::CanonicalSameMaterialHandoff {
                    explicit_vertical_step_index,
                    segment,
                    lower_owner,
                    raised_owner,
                }
            } else {
                continue;
            };
            let source_start =
                SurfaceXzKey::from_raw_keys(segment.start().x_key(), segment.start().z_key());
            let source_end =
                SurfaceXzKey::from_raw_keys(segment.end().x_key(), segment.end().z_key());
            if segments::key_lies_on_segment(span_start, source_start, source_end)
                && segments::key_lies_on_segment(span_end, source_start, source_end)
            {
                return source;
            }
        }
        DebugRaisedStepCoverageSource::FinalTopBoundaryPair
    }

    fn debug_nearest_raised_step_face(
        point: backend::RoadVec3,
        face_span_edges: &[Option<DebugVerticalFaceSpanEdges>],
    ) -> Option<DebugNearestRaisedStepFace> {
        face_span_edges
            .iter()
            .copied()
            .enumerate()
            .filter_map(|(face_index, span_edges)| {
                let span_edges = span_edges?;
                let distance_m = Self::debug_distance_xz_to_segment_m(
                    point,
                    span_edges.lower_start,
                    span_edges.lower_end,
                );
                Some(DebugNearestRaisedStepFace {
                    face_index,
                    distance_m,
                    lower_start: span_edges.lower_start,
                    lower_end: span_edges.lower_end,
                    upper_start: span_edges.upper_start,
                    upper_end: span_edges.upper_end,
                })
            })
            .min_by(|left, right| left.distance_m.total_cmp(&right.distance_m))
    }

    fn debug_distance_xz_to_segment_m(
        point: backend::RoadVec3,
        start: backend::RoadVec3,
        end: backend::RoadVec3,
    ) -> f32 {
        let segment = backend::RoadVec2::new(end.x - start.x, end.z - start.z);
        let length_sq = segment.length_squared();
        if length_sq <= f64::EPSILON {
            return backend::RoadVec2::new(point.x - start.x, point.z - start.z).length() as f32;
        }
        let offset = backend::RoadVec2::new(point.x - start.x, point.z - start.z);
        let t = (offset.dot(segment) / length_sq).clamp(0.0, 1.0);
        let closest = backend::RoadVec2::new(start.x, start.z) + segment * t;
        (backend::RoadVec2::new(point.x, point.z) - closest).length() as f32
    }

    fn debug_top_boundary_edge_length_m(edge: DebugTopBoundaryEdge) -> f32 {
        backend::RoadVec2::new(edge.end.x - edge.start.x, edge.end.z - edge.start.z).length() as f32
    }

    fn debug_top_boundary_xz_at_parameter(
        edge: DebugTopBoundaryEdge,
        parameter: f64,
    ) -> SurfaceXzKey {
        let parameter = SurfaceSegmentParameter::new(
            (parameter * 1_000_000_000.0).round() as i128,
            1_000_000_000,
        )
        .expect("debug raised-step coverage parameter denominator is positive");
        let start = Self::debug_surface_xz_key(edge.key.start.xz());
        let end = Self::debug_surface_xz_key(edge.key.end.xz());
        segments::interpolate_key(start, end, parameter)
    }

    fn debug_top_boundary_edge_point_at_parameter(
        edge: DebugTopBoundaryEdge,
        parameter: f64,
    ) -> backend::RoadVec3 {
        let parameter = SurfaceSegmentParameter::new(
            (parameter * 1_000_000_000.0).round() as i128,
            1_000_000_000,
        )
        .expect("debug raised-step coverage parameter denominator is positive");
        let start = Self::debug_surface_xz_key(edge.key.start.xz());
        let end = Self::debug_surface_xz_key(edge.key.end.xz());
        let xz = segments::interpolate_key(start, end, parameter).to_road_xz();
        let y_mm =
            segments::interpolate_height_i64(edge.key.start.y_mm, edge.key.end.y_mm, parameter);
        backend::RoadVec3::new(xz.x, y_mm as f64 / 1000.0, xz.y)
    }

    fn debug_surface_xz_key(key: DebugRenderXzVertexKey) -> SurfaceXzKey {
        SurfaceXzKey::from_raw_keys(key.x_key, key.z_key)
    }

    fn append_raised_step_coverage_gap_literal(
        dump: &mut String,
        sample: &DebugRaisedStepCoverageGap,
    ) {
        dump.push('{');
        let _ = write!(
            dump,
            "\"missing_length_m\":{:.6},\"missing_interval_t\":[{:.6}, {:.6}],\"lower_owner\":",
            sample.missing_length_m, sample.missing_start_t, sample.missing_end_t
        );
        Self::append_debug_boundary_owner_literal(dump, sample.lower.owner);
        dump.push_str(",\"raised_owner\":");
        Self::append_debug_boundary_owner_literal(dump, sample.raised.owner);
        dump.push_str(",\"source_lower_edge\":");
        Self::append_debug_top_boundary_edge_literal(dump, sample.lower);
        dump.push_str(",\"source_raised_edge\":");
        Self::append_debug_top_boundary_edge_literal(dump, sample.raised);
        dump.push_str(",\"missing_lower_edge_world\":");
        Self::append_vector3_pair_precise_literal(dump, sample.lower_start, sample.lower_end);
        dump.push_str(",\"missing_raised_edge_world\":");
        Self::append_vector3_pair_precise_literal(dump, sample.raised_start, sample.raised_end);
        dump.push_str(",\"source_edge\":");
        Self::append_raised_step_coverage_source_literal(dump, &sample.source);
        dump.push_str(",\"nearest_emitted_face\":");
        if let Some(face) = &sample.nearest_face {
            Self::append_nearest_raised_step_face_literal(dump, face);
        } else {
            dump.push_str("null");
        }
        dump.push('}');
    }

    fn append_raised_step_coverage_source_literal(
        dump: &mut String,
        source: &DebugRaisedStepCoverageSource,
    ) {
        match source {
            DebugRaisedStepCoverageSource::Canonical {
                explicit_vertical_step_index,
                segment,
            } => {
                let _ = write!(
                    dump,
                    "{{\"kind\":\"canonical_step\",\"explicit_vertical_step_index\":{},\"canonical_edge_key\":",
                    explicit_vertical_step_index
                );
                Self::append_node_arrangement_segment_key_literal(
                    dump,
                    segment.start(),
                    segment.end(),
                );
                dump.push_str(",\"owner\":");
                Self::append_node_band_owner_literal(dump, segment.owner());
                dump.push_str(",\"opposite_owner\":");
                Self::append_node_band_owner_literal(dump, segment.opposite_owner());
                dump.push('}');
            }
            DebugRaisedStepCoverageSource::CanonicalSameMaterialHandoff {
                explicit_vertical_step_index,
                segment,
                lower_owner,
                raised_owner,
            } => {
                let _ = write!(
                    dump,
                    "{{\"kind\":\"canonical_step_same_material_handoff\",\"explicit_vertical_step_index\":{},\"canonical_edge_key\":",
                    explicit_vertical_step_index
                );
                Self::append_node_arrangement_segment_key_literal(
                    dump,
                    segment.start(),
                    segment.end(),
                );
                dump.push_str(",\"owner\":");
                Self::append_node_band_owner_literal(dump, segment.owner());
                dump.push_str(",\"opposite_owner\":");
                Self::append_node_band_owner_literal(dump, segment.opposite_owner());
                dump.push_str(",\"final_lower_owner\":");
                Self::append_node_band_owner_literal(dump, *lower_owner);
                dump.push_str(",\"final_raised_owner\":");
                Self::append_node_band_owner_literal(dump, *raised_owner);
                dump.push('}');
            }
            DebugRaisedStepCoverageSource::FinalTopBoundaryPair => {
                dump.push_str("{\"kind\":\"final_top_boundary_pair\"}");
            }
        }
    }

    fn append_nearest_raised_step_face_literal(
        dump: &mut String,
        face: &DebugNearestRaisedStepFace,
    ) {
        let _ = write!(
            dump,
            "{{\"face\":{},\"distance_m\":{:.6},\"lower_edge_world\":",
            face.face_index, face.distance_m
        );
        Self::append_vector3_pair_precise_literal(dump, face.lower_start, face.lower_end);
        dump.push_str(",\"upper_edge_world\":");
        Self::append_vector3_pair_precise_literal(dump, face.upper_start, face.upper_end);
        dump.push('}');
    }

    pub(in crate::simulation::network::surface::debug) fn append_raised_step_face_source_literal(
        dump: &mut String,
        source: Option<RoadSurfaceVerticalFaceSource>,
    ) {
        dump.push_str(",\"source_kind\":");
        match source {
            Some(RoadSurfaceVerticalFaceSource::CanonicalStep { .. }) => {
                dump.push_str("\"canonical_step\"");
            }
            Some(RoadSurfaceVerticalFaceSource::CanonicalStepSameMaterialHandoff { .. }) => {
                dump.push_str("\"canonical_step_same_material_handoff\"");
            }
            None => dump.push_str("null"),
        }
        dump.push_str(",\"source_explicit_vertical_step_index\":");
        if let Some(source_index) =
            source.and_then(RoadSurfaceVerticalFaceSource::explicit_vertical_step_index)
        {
            let _ = write!(dump, "{source_index}");
        } else {
            dump.push_str("null");
        }
        dump.push_str(",\"source_owner_pair\":");
        if let Some(source) = source {
            let segment = source.segment();
            dump.push_str("{\"owner\":");
            Self::append_node_band_owner_literal(dump, segment.owner());
            dump.push_str(",\"opposite_owner\":");
            Self::append_node_band_owner_literal(dump, segment.opposite_owner());
            dump.push('}');
        } else {
            dump.push_str("null");
        }
        dump.push_str(",\"source_final_owner_pair\":");
        if let Some(source) = source {
            if let Some((lower_owner, raised_owner)) = source.lower_and_raised_owners() {
                dump.push_str("{\"lower_owner\":");
                Self::append_node_band_owner_literal(dump, lower_owner);
                dump.push_str(",\"raised_owner\":");
                Self::append_node_band_owner_literal(dump, raised_owner);
                dump.push('}');
            } else {
                dump.push_str("null");
            }
        } else {
            dump.push_str("null");
        }
        dump.push_str(",\"source_canonical_edge_key\":");
        if let Some(source) = source {
            let segment = source.segment();
            Self::append_node_arrangement_segment_key_literal(dump, segment.start(), segment.end());
        } else {
            dump.push_str("null");
        }
    }
}
