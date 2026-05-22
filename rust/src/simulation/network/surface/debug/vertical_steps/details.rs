//! Raised-step face detail debug literal writers.

use super::*;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface::debug) fn append_raised_step_face_details_debug_literal(
        dump: &mut String,
        piece: &RoadSurfaceVisualNodePiece,
    ) {
        let top_edges = Self::debug_owned_top_boundary_edges(piece);
        let mut top_edges_by_key: BTreeMap<DebugRenderEdgeKey, Vec<DebugTopBoundaryEdge>> =
            BTreeMap::new();
        for edge in &top_edges {
            top_edges_by_key.entry(edge.key).or_default().push(*edge);
        }
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
            let RoadSurfaceVerticalFaceSource::CanonicalStep {
                explicit_vertical_step_index,
                segment,
            } = source;
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

        let mut face_problem_count = 0usize;
        for (face_index, span_edges) in face_span_edges.iter().enumerate() {
            let Some(span_edges) = span_edges else {
                face_problem_count += 1;
                continue;
            };
            let lower_key =
                DebugRenderEdgeKey::normalized(span_edges.lower_start, span_edges.lower_end);
            let upper_key =
                DebugRenderEdgeKey::normalized(span_edges.upper_start, span_edges.upper_end);
            let lower_matches = lower_key
                .and_then(|key| top_edges_by_key.get(&key))
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let upper_matches = upper_key
                .and_then(|key| top_edges_by_key.get(&key))
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let matches_raised_step_owner_pair =
                Self::debug_top_matches_form_raised_step_owner_pair(lower_matches, upper_matches);
            let visible_dot = Self::debug_polygon_winding_normal(
                &piece.raised_step_face_polygons[face_index].points_world,
            )
            .map(|normal| -normal)
            .and_then(|direction| {
                Self::debug_visible_dot_to_lower_raised_step_owner(
                    piece,
                    (span_edges.lower_start + span_edges.lower_end) * 0.5,
                    direction,
                    lower_matches,
                    upper_matches,
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
        let final_required_problem_count = face_problem_count + missing_required_face_count;
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
                        &top_edges_by_key,
                    )
                    .unwrap_or(false)
            })
            .count();
        dump.push('{');
        let _ = write!(
            dump,
            "\"face_count\":{},\"emitted_face_count\":{},\"top_boundary_edge_count\":{},\"expected_raised_step_count\":{},\"final_required_face_count\":{},\"missing_required_face_count\":{},\"face_problem_count\":{},\"final_required_problem_count\":{},\"canonical_raised_step_count\":{},\"source_constraint_count\":{},\"non_exposed_source_constraint_count\":{},\"canonical_raised_step_problem_count\":{},\"problem_count\":{}",
            piece.raised_step_face_polygons.len(),
            piece.raised_step_face_polygons.len(),
            top_edges.len(),
            expected_steps.len(),
            expected_steps.len(),
            missing_required_face_count,
            face_problem_count,
            final_required_problem_count,
            canonical_steps.len(),
            canonical_steps.len(),
            non_exposed_source_constraint_count,
            canonical_problem_count,
            final_required_problem_count
        );
        dump.push_str(",\"faces\":[");
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
                &top_edges_by_key,
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
                &top_edges_by_key,
            );
        }
        dump.push_str("]}");
    }

    pub(in crate::simulation::network::surface::debug) fn append_raised_step_face_detail_literal(
        dump: &mut String,
        piece: &RoadSurfaceVisualNodePiece,
        face_index: usize,
        polygon: &RoadSurfaceVisualPolygon,
        source: Option<RoadSurfaceVerticalFaceSource>,
        span_edges: Option<DebugVerticalFaceSpanEdges>,
        top_edges_by_key: &BTreeMap<DebugRenderEdgeKey, Vec<DebugTopBoundaryEdge>>,
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

        let lower_matches = lower_key.and_then(|key| top_edges_by_key.get(&key));
        let upper_matches = upper_key.and_then(|key| top_edges_by_key.get(&key));
        dump.push_str(",\"lower_top_matches\":");
        Self::append_debug_top_boundary_edge_list_literal(
            dump,
            lower_matches.map(Vec::as_slice).unwrap_or(&[]),
        );
        dump.push_str(",\"upper_top_matches\":");
        Self::append_debug_top_boundary_edge_list_literal(
            dump,
            upper_matches.map(Vec::as_slice).unwrap_or(&[]),
        );
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
                lower_matches.map(Vec::as_slice).unwrap_or(&[]),
                upper_matches.map(Vec::as_slice).unwrap_or(&[]),
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

        let matches_raised_step_owner_pair = Self::debug_top_matches_form_raised_step_owner_pair(
            lower_matches.map(Vec::as_slice).unwrap_or(&[]),
            upper_matches.map(Vec::as_slice).unwrap_or(&[]),
        );
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

    pub(in crate::simulation::network::surface::debug) fn append_raised_step_face_source_literal(
        dump: &mut String,
        source: Option<RoadSurfaceVerticalFaceSource>,
    ) {
        dump.push_str(",\"source_kind\":");
        match source {
            Some(RoadSurfaceVerticalFaceSource::CanonicalStep { .. }) => {
                dump.push_str("\"canonical_step\"");
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
        dump.push_str(",\"source_canonical_edge_key\":");
        if let Some(source) = source {
            let segment = source.segment();
            Self::append_node_arrangement_segment_key_literal(dump, segment.start(), segment.end());
        } else {
            dump.push_str("null");
        }
    }
}
