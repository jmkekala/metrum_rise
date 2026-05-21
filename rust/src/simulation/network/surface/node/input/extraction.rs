//! Node arrangement input extraction and validation.

use super::rails::{
    band_intervals, boundary_rails, profile_rails, quantize_band_intervals_xz,
    quantize_boundary_rails_xz, quantize_profile_rails_xz, replace_profile_paths_with_chords,
};
use super::*;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn build_node_arrangement_input_from_mouths(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        mouths: &[OrderedIncidentPieceMouth],
    ) -> Result<NodeArrangementInput, NodeInputExtractionError> {
        NodeArrangementInput::from_ordered_mouths(node_id, piece_kind, mouths)
    }
}

impl NodeArrangementInput {
    pub(crate) fn from_ordered_mouths(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        mouths: &[OrderedIncidentPieceMouth],
    ) -> Result<Self, NodeInputExtractionError> {
        if mouths.is_empty() {
            return Err(NodeInputExtractionError::EmptyMouthSet { node_id });
        }

        let mut input_mouths = Vec::with_capacity(mouths.len());
        for (order_index, mouth) in mouths.iter().enumerate() {
            input_mouths.push(NodeInputMouth::from_ordered_mouth(
                piece_kind,
                order_index,
                mouth,
            )?);
        }

        Ok(Self {
            node_id,
            piece_kind,
            mouths: input_mouths,
        })
    }
}

impl NodeInputMouth {
    fn from_ordered_mouth(
        piece_kind: RoadSurfaceVisualNodePieceKind,
        order_index: usize,
        mouth: &OrderedIncidentPieceMouth,
    ) -> Result<Self, NodeInputExtractionError> {
        validate_profile_shape(
            mouth.edge_idx,
            mouth.side,
            NodeInputProfileKind::Mouth,
            &mouth.profile,
        )?;
        validate_profile_shape(
            mouth.edge_idx,
            mouth.side,
            NodeInputProfileKind::Endpoint,
            &mouth.endpoint_profile,
        )?;
        validate_profile_pair(mouth)?;

        let direction_xz = normalized_direction(mouth)?;
        let conflict_handoff_distance_m = conflict_handoff_distance_m(mouth, direction_xz)?;
        let mut mouth_rails = profile_rails(NodeInputProfileKind::Mouth, &mouth.profile);
        let mut endpoint_rails =
            profile_rails(NodeInputProfileKind::Endpoint, &mouth.endpoint_profile);
        let mut boundary_rails = boundary_rails(mouth);
        let mut band_intervals = band_intervals(mouth);
        if piece_kind == RoadSurfaceVisualNodePieceKind::Terminal {
            replace_profile_paths_with_chords(&mut boundary_rails, &mut band_intervals);
        }
        quantize_profile_rails_xz(&mut mouth_rails);
        quantize_profile_rails_xz(&mut endpoint_rails);
        quantize_boundary_rails_xz(&mut boundary_rails);
        quantize_band_intervals_xz(&mut band_intervals);

        Ok(Self {
            order_index,
            edge_idx: mouth.edge_idx,
            side: mouth.side,
            direction_xz,
            direction_angle_ccw: f64::from(mouth.direction_angle_ccw),
            conflict_handoff_distance_m,
            mouth_rails,
            endpoint_rails,
            boundary_rails,
            band_intervals,
            uses_explicit_band_domain_paths: mouth.uses_explicit_band_domain_paths,
        })
    }
}

fn validate_profile_shape(
    edge_idx: usize,
    side: IncidentEdgeSide,
    profile_kind: NodeInputProfileKind,
    profile: &IncidentMouthProfile,
) -> Result<(), NodeInputExtractionError> {
    if profile.bands.is_empty() {
        return Err(NodeInputExtractionError::EmptyProfileBands {
            edge_idx,
            side,
            profile_kind,
        });
    }

    let expected = profile.bands.len() + 1;
    let actual = profile.boundary_points_world.len();
    if expected != actual {
        return Err(NodeInputExtractionError::ProfileBoundaryCountMismatch {
            edge_idx,
            side,
            profile_kind,
            expected,
            actual,
        });
    }
    Ok(())
}

fn validate_profile_pair(
    mouth: &OrderedIncidentPieceMouth,
) -> Result<(), NodeInputExtractionError> {
    if mouth.profile.bands.len() != mouth.endpoint_profile.bands.len() {
        return Err(NodeInputExtractionError::ProfileBandCountMismatch {
            edge_idx: mouth.edge_idx,
            side: mouth.side,
            mouth_band_count: mouth.profile.bands.len(),
            endpoint_band_count: mouth.endpoint_profile.bands.len(),
        });
    }

    for (band_index, (mouth_band, endpoint_band)) in mouth
        .profile
        .bands
        .iter()
        .zip(&mouth.endpoint_profile.bands)
        .enumerate()
    {
        if mouth_band.kind != endpoint_band.kind {
            return Err(NodeInputExtractionError::ProfileBandKindMismatch {
                edge_idx: mouth.edge_idx,
                side: mouth.side,
                band_index,
                mouth_kind: mouth_band.kind,
                endpoint_kind: endpoint_band.kind,
            });
        }
    }
    Ok(())
}

fn normalized_direction(
    mouth: &OrderedIncidentPieceMouth,
) -> Result<RoadVec2, NodeInputExtractionError> {
    let direction = godot_vec2_to_road(mouth.direction_xz);
    let length = direction.length();
    if length <= f64::EPSILON {
        return Err(NodeInputExtractionError::DegenerateDirection {
            edge_idx: mouth.edge_idx,
            side: mouth.side,
        });
    }
    Ok(direction / length)
}

fn conflict_handoff_distance_m(
    mouth: &OrderedIncidentPieceMouth,
    direction_xz: RoadVec2,
) -> Result<f64, NodeInputExtractionError> {
    let mut total = 0.0;
    let mut count = 0usize;

    for (mouth_point, endpoint_point) in mouth
        .profile
        .boundary_points_world
        .iter()
        .zip(&mouth.endpoint_profile.boundary_points_world)
    {
        let mouth_xz = godot_vec3_xz_to_road(*mouth_point);
        let endpoint_xz = godot_vec3_xz_to_road(*endpoint_point);
        total += (mouth_xz - endpoint_xz).dot(direction_xz);
        count += 1;
    }

    let distance_m = total / count as f64;
    if !distance_m.is_finite() || distance_m < 0.0 {
        return Err(NodeInputExtractionError::InvalidHandoffDistance {
            edge_idx: mouth.edge_idx,
            side: mouth.side,
            distance_m,
        });
    }
    Ok(distance_m)
}
