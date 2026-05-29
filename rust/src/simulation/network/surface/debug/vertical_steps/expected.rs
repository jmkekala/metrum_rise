//! Expected and canonical raised-step debug extraction.

use super::*;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface::debug) fn append_expected_vertical_step_literal(
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

    pub(in crate::simulation::network::surface::debug) fn append_canonical_vertical_step_literal(
        dump: &mut String,
        piece: &RoadSurfaceVisualNodePiece,
        step_index: usize,
        step: &DebugCanonicalVerticalStep,
        face_matches: &[usize],
        face_span_edges: &[Option<DebugVerticalFaceSpanEdges>],
        top_edges: &[DebugTopBoundaryEdge],
    ) {
        let visible_dot = Self::debug_canonical_step_visible_dot_from_lower_owner(
            piece,
            step,
            face_matches,
            face_span_edges,
            top_edges,
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

    pub(in crate::simulation::network::surface::debug) fn debug_canonical_raised_steps(
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

    pub(in crate::simulation::network::surface::debug) fn debug_canonical_step_visible_from_lower_owner(
        piece: &RoadSurfaceVisualNodePiece,
        step: &DebugCanonicalVerticalStep,
        face_matches: &[usize],
        face_span_edges: &[Option<DebugVerticalFaceSpanEdges>],
        top_edges: &[DebugTopBoundaryEdge],
    ) -> Option<bool> {
        Self::debug_canonical_step_visible_dot_from_lower_owner(
            piece,
            step,
            face_matches,
            face_span_edges,
            top_edges,
        )
        .map(|dot| dot > 0.0)
    }

    pub(in crate::simulation::network::surface::debug) fn debug_canonical_step_visible_dot_from_lower_owner(
        piece: &RoadSurfaceVisualNodePiece,
        step: &DebugCanonicalVerticalStep,
        face_matches: &[usize],
        face_span_edges: &[Option<DebugVerticalFaceSpanEdges>],
        top_edges: &[DebugTopBoundaryEdge],
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
            let lower_matches = Self::debug_top_edges_containing_span(
                top_edges,
                span_edges.lower_start,
                span_edges.lower_end,
            );
            let Some(dot) = Self::debug_visible_dot_to_lower_owner(
                piece,
                (span_edges.lower_start + span_edges.lower_end) * 0.5,
                visible_direction,
                &lower_matches,
                step.lower_owner,
            ) else {
                continue;
            };
            best = Some(best.map_or(dot, |current| current.max(dot)));
        }
        best
    }

    pub(in crate::simulation::network::surface::debug) fn debug_canonical_step_height_delta(
        step: &DebugCanonicalVerticalStep,
    ) -> Option<f32> {
        let lower = step.lower_top_matches.first()?;
        let raised = step.raised_top_matches.first()?;
        Some(raised.avg_y_m - lower.avg_y_m)
    }

    pub(in crate::simulation::network::surface::debug) fn debug_canonical_step_lower_and_raised_owners(
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

    pub(in crate::simulation::network::surface::debug) fn debug_owner_pair_forms_raised_step(
        lower_owner: DebugBoundaryOwner,
        raised_owner: DebugBoundaryOwner,
    ) -> bool {
        ordered_raised_step_kinds(lower_owner.kind, raised_owner.kind)
            == Some((lower_owner.kind, raised_owner.kind))
    }

    pub(in crate::simulation::network::surface::debug) fn debug_expected_raised_steps(
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
}
