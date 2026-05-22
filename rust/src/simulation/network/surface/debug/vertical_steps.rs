//! Raised-step and vertical-face debug diagnostics.

use super::*;

impl RoadSurfaceSystem {
    pub(super) fn append_raised_step_face_details_debug_literal(
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

    pub(super) fn append_raised_step_face_detail_literal(
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

    pub(super) fn append_raised_step_face_source_literal(
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

    pub(super) fn append_expected_vertical_step_literal(
        dump: &mut String,
        step_index: usize,
        step: DebugExpectedVerticalStep,
        face_matches: &[usize],
    ) {
        dump.push('{');
        let _ = write!(dump, "\"step\":{},\"lower\":", step_index);
        Self::append_debug_top_boundary_edge_literal(dump, step.lower);
        dump.push_str(",\"upper\":");
        Self::append_debug_top_boundary_edge_literal(dump, step.upper);
        dump.push_str(",\"matching_face_indices\":");
        Self::append_usize_list_literal(dump, face_matches);
        let _ = write!(dump, ",\"problem\":{}", face_matches.is_empty());
        dump.push('}');
    }

    pub(super) fn append_canonical_vertical_step_literal(
        dump: &mut String,
        piece: &RoadSurfaceVisualNodePiece,
        step_index: usize,
        step: &DebugCanonicalVerticalStep,
        face_matches: &[usize],
        face_span_edges: &[Option<DebugVerticalFaceSpanEdges>],
        top_edges_by_key: &BTreeMap<DebugRenderEdgeKey, Vec<DebugTopBoundaryEdge>>,
    ) {
        let visible_dot = Self::debug_canonical_step_visible_dot_from_lower_owner(
            piece,
            step,
            face_matches,
            face_span_edges,
            top_edges_by_key,
        );
        let visible_from_lower_owner = visible_dot.map(|dot| dot > 0.0);
        let materialized = !face_matches.is_empty();
        let problem = materialized && visible_from_lower_owner != Some(true);

        dump.push('{');
        let _ = write!(
            dump,
            "\"step\":{},\"explicit_vertical_step_index\":{},\"owner_pair\":{{\"owner\":",
            step_index, step.explicit_vertical_step_index
        );
        Self::append_node_band_owner_literal(dump, step.segment.owner());
        dump.push_str(",\"opposite_owner\":");
        Self::append_node_band_owner_literal(dump, step.segment.opposite_owner());
        dump.push_str("},\"lower_owner\":");
        Self::append_node_band_owner_literal(dump, step.lower_owner);
        dump.push_str(",\"raised_owner\":");
        Self::append_node_band_owner_literal(dump, step.raised_owner);
        dump.push_str(",\"canonical_edge_key\":");
        Self::append_node_arrangement_segment_key_literal(
            dump,
            step.segment.start(),
            step.segment.end(),
        );
        dump.push_str(",\"height_delta_m\":");
        Self::append_optional_f32_precise_literal(
            dump,
            Self::debug_canonical_step_height_delta(step),
        );
        dump.push_str(",\"lower_top_matches\":");
        Self::append_debug_top_boundary_edge_list_literal(dump, &step.lower_top_matches);
        dump.push_str(",\"raised_top_matches\":");
        Self::append_debug_top_boundary_edge_list_literal(dump, &step.raised_top_matches);
        dump.push_str(",\"matching_face_indices\":");
        Self::append_usize_list_literal(dump, face_matches);
        dump.push_str(",\"materialization_status\":");
        if materialized {
            dump.push_str("\"materialized\"");
        } else {
            dump.push_str("\"not_exposed_after_boolean_ownership\"");
        }
        dump.push_str(",\"visible_dot_lower_owner\":");
        Self::append_optional_f32_precise_literal(dump, visible_dot);
        dump.push_str(",\"visible_from_lower_owner\":");
        if let Some(visible) = visible_from_lower_owner {
            let _ = write!(dump, "{visible}");
        } else {
            dump.push_str("null");
        }
        let _ = write!(dump, ",\"problem\":{problem}");
        dump.push('}');
    }

    pub(super) fn debug_canonical_raised_steps(
        piece: &RoadSurfaceVisualNodePiece,
        top_edges: &[DebugTopBoundaryEdge],
    ) -> Vec<DebugCanonicalVerticalStep> {
        let mut steps = Vec::new();
        for (explicit_vertical_step_index, segment) in piece
            .explicit_vertical_step_segments
            .iter()
            .copied()
            .enumerate()
        {
            let Some((lower_owner, raised_owner)) =
                Self::debug_canonical_step_lower_and_raised_owners(segment)
            else {
                continue;
            };
            let xz_key =
                DebugRenderXzEdgeKey::from_arrangement_segment(segment.start(), segment.end());
            let lower_top_matches = top_edges
                .iter()
                .copied()
                .filter(|edge| {
                    edge.xz_key == xz_key
                        && Self::debug_boundary_owner_matches_band(edge.owner, lower_owner)
                })
                .collect();
            let raised_top_matches = top_edges
                .iter()
                .copied()
                .filter(|edge| {
                    edge.xz_key == xz_key
                        && Self::debug_boundary_owner_matches_band(edge.owner, raised_owner)
                })
                .collect();
            steps.push(DebugCanonicalVerticalStep {
                explicit_vertical_step_index,
                segment,
                lower_owner,
                raised_owner,
                lower_top_matches,
                raised_top_matches,
            });
        }
        steps.sort_by(|a, b| {
            a.explicit_vertical_step_index
                .cmp(&b.explicit_vertical_step_index)
                .then(a.segment.start().cmp(&b.segment.start()))
                .then(a.segment.end().cmp(&b.segment.end()))
                .then(a.lower_owner.cmp(&b.lower_owner))
                .then(a.raised_owner.cmp(&b.raised_owner))
        });
        steps
    }

    pub(super) fn debug_canonical_step_visible_from_lower_owner(
        piece: &RoadSurfaceVisualNodePiece,
        step: &DebugCanonicalVerticalStep,
        face_matches: &[usize],
        face_span_edges: &[Option<DebugVerticalFaceSpanEdges>],
        top_edges_by_key: &BTreeMap<DebugRenderEdgeKey, Vec<DebugTopBoundaryEdge>>,
    ) -> Option<bool> {
        Self::debug_canonical_step_visible_dot_from_lower_owner(
            piece,
            step,
            face_matches,
            face_span_edges,
            top_edges_by_key,
        )
        .map(|dot| dot > 0.0)
    }

    pub(super) fn debug_canonical_step_visible_dot_from_lower_owner(
        piece: &RoadSurfaceVisualNodePiece,
        step: &DebugCanonicalVerticalStep,
        face_matches: &[usize],
        face_span_edges: &[Option<DebugVerticalFaceSpanEdges>],
        top_edges_by_key: &BTreeMap<DebugRenderEdgeKey, Vec<DebugTopBoundaryEdge>>,
    ) -> Option<f32> {
        let mut best: Option<f32> = None;
        for &face_index in face_matches {
            let Some(span_edges) = face_span_edges.get(face_index).copied().flatten() else {
                continue;
            };
            let Some(visible_direction) = piece
                .raised_step_face_polygons
                .get(face_index)
                .and_then(|polygon| Self::debug_polygon_winding_normal(&polygon.points_world))
                .map(|normal| -normal)
            else {
                continue;
            };
            let lower_key =
                DebugRenderEdgeKey::normalized(span_edges.lower_start, span_edges.lower_end);
            let lower_matches = lower_key
                .and_then(|key| top_edges_by_key.get(&key))
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let Some(dot) = Self::debug_visible_dot_to_lower_owner(
                piece,
                (span_edges.lower_start + span_edges.lower_end) * 0.5,
                visible_direction,
                lower_matches,
                step.lower_owner,
            ) else {
                continue;
            };
            best = Some(best.map_or(dot, |current| current.max(dot)));
        }
        best
    }

    pub(super) fn debug_canonical_step_height_delta(
        step: &DebugCanonicalVerticalStep,
    ) -> Option<f32> {
        let lower = step.lower_top_matches.first()?;
        let raised = step.raised_top_matches.first()?;
        Some(raised.avg_y_m - lower.avg_y_m)
    }

    pub(super) fn debug_canonical_step_lower_and_raised_owners(
        segment: NodeExplicitVerticalStepSegment,
    ) -> Option<(NodeBandOwner, NodeBandOwner)> {
        let owner = segment.owner();
        let opposite_owner = segment.opposite_owner();
        let (lower_kind, _) = ordered_raised_step_kinds(owner.kind(), opposite_owner.kind())?;
        if owner.kind() == lower_kind {
            Some((owner, opposite_owner))
        } else {
            Some((opposite_owner, owner))
        }
    }

    pub(super) fn debug_owner_pair_forms_raised_step(
        lower_owner: DebugBoundaryOwner,
        raised_owner: DebugBoundaryOwner,
    ) -> bool {
        ordered_raised_step_kinds(lower_owner.kind, raised_owner.kind)
            == Some((lower_owner.kind, raised_owner.kind))
    }

    pub(super) fn debug_top_matches_form_raised_step_owner_pair(
        lower_matches: &[DebugTopBoundaryEdge],
        upper_matches: &[DebugTopBoundaryEdge],
    ) -> bool {
        lower_matches.iter().any(|lower| {
            upper_matches
                .iter()
                .any(|upper| Self::debug_owner_pair_forms_raised_step(lower.owner, upper.owner))
        })
    }

    pub(super) fn debug_boundary_owner_matches_band(
        owner: DebugBoundaryOwner,
        band_owner: NodeBandOwner,
    ) -> bool {
        owner.kind == band_owner.kind() && owner.owner_index == band_owner.owner_index()
    }

    pub(super) fn append_node_band_owner_literal(dump: &mut String, owner: NodeBandOwner) {
        let _ = write!(
            dump,
            "{{\"kind\":\"{:?}\",\"owner_index\":{}}}",
            owner.kind(),
            owner.owner_index()
        );
    }

    pub(super) fn append_node_arrangement_segment_key_literal(
        dump: &mut String,
        start: NodeArrangementKey,
        end: NodeArrangementKey,
    ) {
        let _ = write!(
            dump,
            "{{\"start\":{{\"x_key\":{},\"z_key\":{},\"x_mm\":{},\"z_mm\":{}}},\"end\":{{\"x_key\":{},\"z_key\":{},\"x_mm\":{},\"z_mm\":{}}}}}",
            start.x_key(),
            start.z_key(),
            start.x_mm(),
            start.z_mm(),
            end.x_key(),
            end.z_key(),
            end.x_mm(),
            end.z_mm()
        );
    }

    pub(super) fn append_debug_top_boundary_edge_list_literal(
        dump: &mut String,
        edges: &[DebugTopBoundaryEdge],
    ) {
        dump.push('[');
        for (index, edge) in edges.iter().enumerate() {
            if index > 0 {
                dump.push_str(", ");
            }
            Self::append_debug_top_boundary_edge_literal(dump, *edge);
        }
        dump.push(']');
    }

    pub(super) fn append_debug_top_boundary_edge_literal(
        dump: &mut String,
        edge: DebugTopBoundaryEdge,
    ) {
        dump.push('{');
        dump.push_str("\"owner\":");
        Self::append_debug_boundary_owner_literal(dump, edge.owner);
        dump.push_str(",\"edge_world\":");
        Self::append_vector3_pair_precise_literal(dump, edge.start, edge.end);
        dump.push_str(",\"edge_key\":");
        Self::append_debug_render_edge_key_literal(dump, edge.key);
        dump.push_str(",\"xz_key\":");
        Self::append_debug_render_xz_edge_key_literal(dump, edge.xz_key);
        let _ = write!(dump, ",\"avg_y_m\":{:.6}", edge.avg_y_m);
        dump.push('}');
    }

    pub(super) fn append_debug_boundary_owner_literal(
        dump: &mut String,
        owner: DebugBoundaryOwner,
    ) {
        let _ = write!(
            dump,
            "{{\"region\":{},\"kind\":\"{:?}\",\"owner_index\":{}}}",
            owner.region_index, owner.kind, owner.owner_index
        );
    }

    pub(super) fn debug_owned_top_boundary_edges(
        piece: &RoadSurfaceVisualNodePiece,
    ) -> Vec<DebugTopBoundaryEdge> {
        let mut boundary_edges = Vec::new();
        for (region_index, region) in piece.owned_regions.iter().enumerate() {
            let owner = DebugBoundaryOwner {
                region_index,
                kind: region.kind,
                owner_index: region.owner_index,
            };
            let mut edge_counts: BTreeMap<DebugRenderEdgeKey, (usize, Vector3, Vector3)> =
                BTreeMap::new();
            if region.polygon.triangles_world.is_empty() {
                let points = &region.polygon.points_world;
                if points.len() >= 2 {
                    for index in 0..points.len() {
                        Self::record_debug_top_boundary_edge_count(
                            &mut edge_counts,
                            points[index],
                            points[(index + 1) % points.len()],
                        );
                    }
                }
            } else {
                for triangle in &region.polygon.triangles_world {
                    for edge_index in 0..3 {
                        Self::record_debug_top_boundary_edge_count(
                            &mut edge_counts,
                            triangle[edge_index],
                            triangle[(edge_index + 1) % 3],
                        );
                    }
                }
            }
            for (key, (count, start, end)) in edge_counts {
                if count != 1 {
                    continue;
                }
                boundary_edges.push(DebugTopBoundaryEdge {
                    owner,
                    start,
                    end,
                    key,
                    xz_key: key.xz(),
                    avg_y_m: (start.y + end.y) * 0.5,
                });
            }
        }
        boundary_edges.sort_by(|a, b| {
            a.key
                .cmp(&b.key)
                .then(a.owner.region_index.cmp(&b.owner.region_index))
                .then(a.owner.kind.cmp(&b.owner.kind))
                .then(a.owner.owner_index.cmp(&b.owner.owner_index))
        });
        boundary_edges
    }

    pub(super) fn record_debug_top_boundary_edge_count(
        edge_counts: &mut BTreeMap<DebugRenderEdgeKey, (usize, Vector3, Vector3)>,
        start: Vector3,
        end: Vector3,
    ) {
        let Some(key) = DebugRenderEdgeKey::normalized(start, end) else {
            return;
        };
        edge_counts
            .entry(key)
            .and_modify(|entry| entry.0 += 1)
            .or_insert((1, start, end));
    }

    pub(super) fn debug_expected_raised_steps(
        top_edges: &[DebugTopBoundaryEdge],
    ) -> Vec<DebugExpectedVerticalStep> {
        let mut edges_by_xz: BTreeMap<DebugRenderXzEdgeKey, Vec<DebugTopBoundaryEdge>> =
            BTreeMap::new();
        for edge in top_edges {
            edges_by_xz.entry(edge.xz_key).or_default().push(*edge);
        }

        let mut steps = Vec::new();
        for edges in edges_by_xz.values() {
            for (left_index, left_edge) in edges.iter().enumerate() {
                for right_edge in edges.iter().skip(left_index + 1) {
                    if left_edge.key == right_edge.key {
                        continue;
                    }
                    let (lower, upper) = if left_edge.avg_y_m <= right_edge.avg_y_m {
                        (*left_edge, *right_edge)
                    } else {
                        (*right_edge, *left_edge)
                    };
                    if !Self::debug_owner_pair_forms_raised_step(lower.owner, upper.owner) {
                        continue;
                    }
                    steps.push(DebugExpectedVerticalStep { lower, upper });
                }
            }
        }

        steps.sort_by(|a, b| {
            a.lower
                .key
                .cmp(&b.lower.key)
                .then(a.upper.key.cmp(&b.upper.key))
                .then(a.lower.owner.region_index.cmp(&b.lower.owner.region_index))
                .then(a.upper.owner.region_index.cmp(&b.upper.owner.region_index))
        });
        steps
    }

    pub(super) fn debug_vertical_face_span_edges(
        polygon: &RoadSurfaceVisualPolygon,
    ) -> Option<DebugVerticalFaceSpanEdges> {
        if polygon.points_world.len() < 4 {
            return None;
        }
        let mut span_edges = Vec::new();
        for index in 0..polygon.points_world.len() {
            let start = polygon.points_world[index];
            let end = polygon.points_world[(index + 1) % polygon.points_world.len()];
            let start_key = DebugRenderVertexKey::from_point(start).xz();
            let end_key = DebugRenderVertexKey::from_point(end).xz();
            if start_key != end_key {
                span_edges.push((start, end, (start.y + end.y) * 0.5));
            }
        }
        if span_edges.len() != 2 {
            return None;
        }
        span_edges.sort_by(|a, b| a.2.total_cmp(&b.2));
        Some(DebugVerticalFaceSpanEdges {
            lower_start: span_edges[0].0,
            lower_end: span_edges[0].1,
            upper_start: span_edges[1].0,
            upper_end: span_edges[1].1,
        })
    }

    pub(super) fn debug_polygon_winding_normal(points: &[Vector3]) -> Option<Vector3> {
        if points.len() < 3 {
            return None;
        }
        for index in 1..points.len().saturating_sub(1) {
            let normal = (points[index] - points[0]).cross(points[index + 1] - points[0]);
            if normal.length_squared() > 1e-8 {
                return Some(normal.normalized());
            }
        }
        None
    }

    pub(super) fn debug_visible_dot_to_lower_raised_step_owner(
        piece: &RoadSurfaceVisualNodePiece,
        face_midpoint: Vector3,
        visible_direction: Vector3,
        lower_matches: &[DebugTopBoundaryEdge],
        upper_matches: &[DebugTopBoundaryEdge],
    ) -> Option<f32> {
        let visible_xz = Vector3::new(visible_direction.x, 0.0, visible_direction.z);
        if visible_xz.length_squared() <= 1e-8 {
            return None;
        }
        let visible_xz = visible_xz.normalized();
        let mut best: Option<f32> = None;
        for edge in lower_matches.iter().filter(|lower| {
            upper_matches
                .iter()
                .any(|upper| Self::debug_owner_pair_forms_raised_step(lower.owner, upper.owner))
        }) {
            let Some(centroid) = Self::debug_owned_region_centroid(piece, edge.owner.region_index)
            else {
                continue;
            };
            let owner_direction = Vector3::new(
                centroid.x - face_midpoint.x,
                0.0,
                centroid.z - face_midpoint.z,
            );
            if owner_direction.length_squared() <= 1e-8 {
                continue;
            }
            let dot = visible_xz.dot(owner_direction.normalized());
            best = Some(best.map_or(dot, |current| current.max(dot)));
        }
        best
    }

    pub(super) fn debug_visible_dot_to_lower_owner(
        piece: &RoadSurfaceVisualNodePiece,
        face_midpoint: Vector3,
        visible_direction: Vector3,
        lower_matches: &[DebugTopBoundaryEdge],
        lower_owner: NodeBandOwner,
    ) -> Option<f32> {
        let visible_xz = Vector3::new(visible_direction.x, 0.0, visible_direction.z);
        if visible_xz.length_squared() <= 1e-8 {
            return None;
        }
        let visible_xz = visible_xz.normalized();
        let mut best: Option<f32> = None;
        for edge in lower_matches
            .iter()
            .filter(|edge| Self::debug_boundary_owner_matches_band(edge.owner, lower_owner))
        {
            let Some(centroid) = Self::debug_owned_region_centroid(piece, edge.owner.region_index)
            else {
                continue;
            };
            let owner_direction = Vector3::new(
                centroid.x - face_midpoint.x,
                0.0,
                centroid.z - face_midpoint.z,
            );
            if owner_direction.length_squared() <= 1e-8 {
                continue;
            }
            let dot = visible_xz.dot(owner_direction.normalized());
            best = Some(best.map_or(dot, |current| current.max(dot)));
        }
        best
    }

    pub(super) fn debug_owned_region_centroid(
        piece: &RoadSurfaceVisualNodePiece,
        region_index: usize,
    ) -> Option<Vector3> {
        let region = piece.owned_regions.get(region_index)?;
        let mut sum = Vector3::ZERO;
        let mut count = 0usize;
        if region.polygon.points_world.is_empty() {
            for point in region
                .polygon
                .triangles_world
                .iter()
                .flat_map(|triangle| triangle.iter().copied())
            {
                sum += point;
                count += 1;
            }
        } else {
            for point in &region.polygon.points_world {
                sum += *point;
                count += 1;
            }
        }
        (count > 0).then_some(sum * (1.0 / count as f32))
    }
}
