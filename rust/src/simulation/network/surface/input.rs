//! Canonical node-arrangement input extracted from solved road-surface profiles.

use super::backend::{
    ROAD_OVERLAY_COORDINATE_SCALE, RoadVec2, RoadVec3, godot_vec2_to_road, godot_vec3_to_road,
    godot_vec3_xz_to_road, quantize_road_vec2_to_overlay_grid,
};
use super::{
    IncidentEdgeSide, IncidentMouthBand, IncidentMouthProfile, NODE_OVERLAY_MIN_AREA_M2,
    OrderedIncidentPieceMouth, RoadSurfaceBandKind, RoadSurfaceSystem,
    RoadSurfaceVisualNodePieceKind,
};
use godot::prelude::Vector3;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) enum NodeInputProfileKind {
    Mouth,
    Endpoint,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) enum NodeInputBoundaryRailRole {
    OuterFootprint {
        adjacent_kind: RoadSurfaceBandKind,
    },
    InteriorBandBoundary {
        left_kind: RoadSurfaceBandKind,
        right_kind: RoadSurfaceBandKind,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeArrangementInput {
    pub(crate) node_id: u32,
    pub(crate) piece_kind: RoadSurfaceVisualNodePieceKind,
    pub(crate) mouths: Vec<NodeInputMouth>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeInputMouth {
    pub(crate) order_index: usize,
    pub(crate) edge_idx: usize,
    pub(crate) side: IncidentEdgeSide,
    pub(crate) direction_xz: RoadVec2,
    pub(crate) direction_angle_ccw: f64,
    pub(crate) conflict_handoff_distance_m: f64,
    pub(crate) mouth_rails: Vec<NodeInputProfileRail>,
    pub(crate) endpoint_rails: Vec<NodeInputProfileRail>,
    pub(crate) boundary_rails: Vec<NodeInputBoundaryRail>,
    pub(crate) band_intervals: Vec<NodeInputBandInterval>,
    pub(crate) terminal_end_bands: Vec<NodeInputTerminalEndBand>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeInputProfileRail {
    pub(crate) profile_kind: NodeInputProfileKind,
    pub(crate) band_index: usize,
    pub(crate) band_kind: RoadSurfaceBandKind,
    pub(crate) start_world: RoadVec3,
    pub(crate) end_world: RoadVec3,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeInputBoundaryRail {
    pub(crate) boundary_index: usize,
    pub(crate) role: NodeInputBoundaryRailRole,
    pub(crate) mouth_world: RoadVec3,
    pub(crate) endpoint_world: RoadVec3,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeInputBandInterval {
    pub(crate) band_index: usize,
    pub(crate) band_kind: RoadSurfaceBandKind,
    pub(crate) mouth_start_world: RoadVec3,
    pub(crate) mouth_end_world: RoadVec3,
    pub(crate) endpoint_start_world: RoadVec3,
    pub(crate) endpoint_end_world: RoadVec3,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeInputTerminalEndBand {
    pub(crate) source_band_index: usize,
    pub(crate) band_kind: RoadSurfaceBandKind,
    pub(crate) boundary_mode: NodeInputTerminalEndBandBoundaryMode,
    pub(crate) inner_start_world: RoadVec3,
    pub(crate) inner_end_world: RoadVec3,
    pub(crate) outer_start_world: RoadVec3,
    pub(crate) outer_end_world: RoadVec3,
    pub(crate) contour_world: Vec<RoadVec3>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NodeInputTerminalEndBandBoundaryMode {
    MaterialBand,
    TerminalMaterialBand,
    MaterialBandWithinFootprint,
    CurbGuardWithinFootprint,
    MaterialBandWithSameOwnerOuterCap,
    SameOwnerOuterCap,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NodeInputExtractionError {
    EmptyMouthSet {
        node_id: u32,
    },
    DegenerateDirection {
        edge_idx: usize,
        side: IncidentEdgeSide,
    },
    ProfileBoundaryCountMismatch {
        edge_idx: usize,
        side: IncidentEdgeSide,
        profile_kind: NodeInputProfileKind,
        expected: usize,
        actual: usize,
    },
    EmptyProfileBands {
        edge_idx: usize,
        side: IncidentEdgeSide,
        profile_kind: NodeInputProfileKind,
    },
    ProfileBandCountMismatch {
        edge_idx: usize,
        side: IncidentEdgeSide,
        mouth_band_count: usize,
        endpoint_band_count: usize,
    },
    ProfileBandKindMismatch {
        edge_idx: usize,
        side: IncidentEdgeSide,
        band_index: usize,
        mouth_kind: RoadSurfaceBandKind,
        endpoint_kind: RoadSurfaceBandKind,
    },
    InvalidHandoffDistance {
        edge_idx: usize,
        side: IncidentEdgeSide,
        distance_m: f64,
    },
}

impl RoadSurfaceSystem {
    pub(super) fn build_node_arrangement_input_from_mouths(
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
        add_node_corner_join_bands(piece_kind, &mut input_mouths);

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
        quantize_profile_rails_xz(&mut mouth_rails);
        quantize_profile_rails_xz(&mut endpoint_rails);
        quantize_boundary_rails_xz(&mut boundary_rails);
        quantize_band_intervals_xz(&mut band_intervals);
        let mut terminal_end_bands = terminal_end_bands(piece_kind, mouth, band_intervals.len());
        for end_band in &mut terminal_end_bands {
            quantize_terminal_end_band_xz(end_band);
        }
        terminal_end_bands.retain(|end_band| terminal_end_band_has_quantized_area(end_band));

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
            terminal_end_bands,
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

fn profile_rails(
    profile_kind: NodeInputProfileKind,
    profile: &IncidentMouthProfile,
) -> Vec<NodeInputProfileRail> {
    profile
        .bands
        .iter()
        .enumerate()
        .map(|(band_index, band)| NodeInputProfileRail {
            profile_kind,
            band_index,
            band_kind: band.kind,
            start_world: band_endpoint_with_boundary_xz(
                band.start_point_world,
                profile.boundary_points_world[band_index],
            ),
            end_world: band_endpoint_with_boundary_xz(
                band.end_point_world,
                profile.boundary_points_world[band_index + 1],
            ),
        })
        .collect()
}

fn boundary_rails(mouth: &OrderedIncidentPieceMouth) -> Vec<NodeInputBoundaryRail> {
    mouth
        .profile
        .boundary_points_world
        .iter()
        .zip(&mouth.endpoint_profile.boundary_points_world)
        .enumerate()
        .map(
            |(boundary_index, (mouth_point, endpoint_point))| NodeInputBoundaryRail {
                boundary_index,
                role: boundary_rail_role(boundary_index, &mouth.profile.bands),
                mouth_world: godot_vec3_to_road(*mouth_point),
                endpoint_world: godot_vec3_to_road(*endpoint_point),
            },
        )
        .collect()
}

fn band_intervals(mouth: &OrderedIncidentPieceMouth) -> Vec<NodeInputBandInterval> {
    mouth
        .profile
        .bands
        .iter()
        .zip(&mouth.endpoint_profile.bands)
        .enumerate()
        .map(
            |(band_index, (mouth_band, endpoint_band))| NodeInputBandInterval {
                band_index,
                band_kind: mouth_band.kind,
                mouth_start_world: band_endpoint_with_boundary_xz(
                    mouth_band.start_point_world,
                    mouth.profile.boundary_points_world[band_index],
                ),
                mouth_end_world: band_endpoint_with_boundary_xz(
                    mouth_band.end_point_world,
                    mouth.profile.boundary_points_world[band_index + 1],
                ),
                endpoint_start_world: band_endpoint_with_boundary_xz(
                    endpoint_band.start_point_world,
                    mouth.endpoint_profile.boundary_points_world[band_index],
                ),
                endpoint_end_world: band_endpoint_with_boundary_xz(
                    endpoint_band.end_point_world,
                    mouth.endpoint_profile.boundary_points_world[band_index + 1],
                ),
            },
        )
        .collect()
}

fn band_endpoint_with_boundary_xz(
    band_point_world: Vector3,
    boundary_point_world: Vector3,
) -> RoadVec3 {
    let boundary = godot_vec3_to_road(boundary_point_world);
    RoadVec3::new(boundary.x, f64::from(band_point_world.y), boundary.z)
}

#[derive(Clone, Copy)]
enum BendCornerProfileSide {
    Start,
    End,
}

const BEND_CORNER_HEIGHT_EDGE_EPS_M: f64 = 0.001;
const BEND_CORNER_CURVE_SEGMENTS: usize = 4;

fn bend_corner_miter_split_step() -> usize {
    BEND_CORNER_CURVE_SEGMENTS / 2
}

#[derive(Clone, Copy)]
struct BendCornerLayer {
    band_index: usize,
    band_kind: RoadSurfaceBandKind,
    inner_boundary_index: usize,
    outer_boundary_index: usize,
}

fn add_node_corner_join_bands(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    mouths: &mut [NodeInputMouth],
) {
    match piece_kind {
        RoadSurfaceVisualNodePieceKind::Bend => add_bend_corner_join_bands(mouths),
        RoadSurfaceVisualNodePieceKind::JunctionN => add_junction_corner_join_bands(mouths),
        RoadSurfaceVisualNodePieceKind::Terminal => {}
    }
}

fn add_bend_corner_join_bands(mouths: &mut [NodeInputMouth]) {
    if mouths.len() != 2 {
        return;
    }

    append_adjacent_corner_join_bands(mouths, 0, 1, true);
    append_adjacent_corner_join_bands(mouths, 1, 0, true);
}

fn add_junction_corner_join_bands(mouths: &mut [NodeInputMouth]) {
    if mouths.len() < 2 {
        return;
    }

    for from_index in 0..mouths.len() {
        let to_index = if from_index + 1 == mouths.len() {
            0
        } else {
            from_index + 1
        };
        append_adjacent_corner_join_bands(mouths, from_index, to_index, false);
    }
}

fn append_adjacent_corner_join_bands(
    mouths: &mut [NodeInputMouth],
    from_index: usize,
    to_index: usize,
    include_curb_miter_caps: bool,
) {
    let from_mouth = mouths[from_index].clone();
    let to_mouth = mouths[to_index].clone();
    // Adjacent mouth gaps need explicit owner-preserving bands before heighting; the canonical
    // topology must not repair same-band joins by moving vertices after ownership is solved.
    let from_layers = bend_corner_layers(&from_mouth, BendCornerProfileSide::End);
    let to_layers = bend_corner_layers(&to_mouth, BendCornerProfileSide::Start);
    if from_layers.is_empty() || to_layers.is_empty() {
        return;
    }

    let mut end_bands = bend_corner_end_bands(
        &from_mouth,
        &from_layers,
        &to_mouth,
        &to_layers,
        include_curb_miter_caps,
    );
    for end_band in &mut end_bands {
        quantize_terminal_end_band_xz(end_band);
    }
    end_bands.retain(|end_band| terminal_end_band_has_quantized_area(end_band));
    mouths[from_index].terminal_end_bands.extend(end_bands);

    if include_curb_miter_caps {
        let mut counterpart_end_bands =
            bend_corner_counterpart_end_bands(&from_mouth, &from_layers, &to_mouth, &to_layers);
        for end_band in &mut counterpart_end_bands {
            quantize_terminal_end_band_xz(end_band);
        }
        counterpart_end_bands.retain(|end_band| terminal_end_band_has_quantized_area(end_band));
        mouths[to_index]
            .terminal_end_bands
            .extend(counterpart_end_bands);
    }
}

fn quantize_terminal_end_band_xz(end_band: &mut NodeInputTerminalEndBand) {
    end_band.inner_start_world = quantize_road_vec3_xz(end_band.inner_start_world);
    end_band.inner_end_world = quantize_road_vec3_xz(end_band.inner_end_world);
    end_band.outer_start_world = quantize_road_vec3_xz(end_band.outer_start_world);
    end_band.outer_end_world = quantize_road_vec3_xz(end_band.outer_end_world);
    for point in &mut end_band.contour_world {
        *point = quantize_road_vec3_xz(*point);
    }
}

fn quantize_profile_rails_xz(rails: &mut [NodeInputProfileRail]) {
    for rail in rails {
        rail.start_world = quantize_road_vec3_xz(rail.start_world);
        rail.end_world = quantize_road_vec3_xz(rail.end_world);
    }
}

fn quantize_boundary_rails_xz(rails: &mut [NodeInputBoundaryRail]) {
    for rail in rails {
        rail.mouth_world = quantize_road_vec3_xz(rail.mouth_world);
        rail.endpoint_world = quantize_road_vec3_xz(rail.endpoint_world);
    }
}

fn quantize_band_intervals_xz(intervals: &mut [NodeInputBandInterval]) {
    for interval in intervals {
        interval.mouth_start_world = quantize_road_vec3_xz(interval.mouth_start_world);
        interval.mouth_end_world = quantize_road_vec3_xz(interval.mouth_end_world);
        interval.endpoint_start_world = quantize_road_vec3_xz(interval.endpoint_start_world);
        interval.endpoint_end_world = quantize_road_vec3_xz(interval.endpoint_end_world);
    }
}

fn quantize_road_vec3_xz(point: RoadVec3) -> RoadVec3 {
    let point_xz = quantize_road_vec2_to_overlay_grid(xz_from_road_vec3(point));
    RoadVec3::new(point_xz.x, point.y, point_xz.y)
}

fn terminal_end_band_has_quantized_area(end_band: &NodeInputTerminalEndBand) -> bool {
    let mut keys = Vec::with_capacity(end_band.contour_world.len());
    for point in &end_band.contour_world {
        let key = quantized_xz_key(xz_from_road_vec3(*point));
        if keys.last().copied() != Some(key) {
            keys.push(key);
        }
    }
    if keys.len() > 1 && keys.first() == keys.last() {
        keys.pop();
    }
    if keys.len() < 3 {
        return false;
    }

    let mut double_area: i128 = 0;
    for index in 0..keys.len() {
        let (x0, z0) = keys[index];
        let (x1, z1) = keys[(index + 1) % keys.len()];
        double_area += i128::from(x0) * i128::from(z1) - i128::from(x1) * i128::from(z0);
    }
    let area_m2 = double_area.unsigned_abs() as f64 * 0.5 / ROAD_OVERLAY_COORDINATE_SCALE.powi(2);
    area_m2 > f64::from(NODE_OVERLAY_MIN_AREA_M2)
}

fn quantized_xz_key(point: RoadVec2) -> (i64, i64) {
    (
        (point.x * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
        (point.y * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
    )
}

fn bend_corner_layers(mouth: &NodeInputMouth, side: BendCornerProfileSide) -> Vec<BendCornerLayer> {
    let Some(first_carriageway) = mouth
        .band_intervals
        .iter()
        .position(|band| band.band_kind == RoadSurfaceBandKind::Carriageway)
    else {
        return Vec::new();
    };
    let Some(last_carriageway) = mouth
        .band_intervals
        .iter()
        .rposition(|band| band.band_kind == RoadSurfaceBandKind::Carriageway)
    else {
        return Vec::new();
    };

    match side {
        BendCornerProfileSide::Start => (0..=first_carriageway)
            .rev()
            .filter_map(|band_index| {
                mouth
                    .band_intervals
                    .get(band_index)
                    .map(|band| BendCornerLayer {
                        band_index,
                        band_kind: band.band_kind,
                        inner_boundary_index: band_index + 1,
                        outer_boundary_index: band_index,
                    })
            })
            .collect(),
        BendCornerProfileSide::End => (last_carriageway..mouth.band_intervals.len())
            .filter_map(|band_index| {
                mouth
                    .band_intervals
                    .get(band_index)
                    .map(|band| BendCornerLayer {
                        band_index,
                        band_kind: band.band_kind,
                        inner_boundary_index: band_index,
                        outer_boundary_index: band_index + 1,
                    })
            })
            .collect(),
    }
}

fn bend_corner_end_bands(
    from_mouth: &NodeInputMouth,
    from_layers: &[BendCornerLayer],
    to_mouth: &NodeInputMouth,
    to_layers: &[BendCornerLayer],
    include_curb_miter_caps: bool,
) -> Vec<NodeInputTerminalEndBand> {
    let mut end_bands = Vec::new();
    let mut last_curb_layers = None;
    for (from_layer, to_layer) in from_layers.iter().zip(to_layers) {
        if from_layer.band_kind != to_layer.band_kind {
            break;
        }
        let source_band_index = from_layer.band_index;
        if from_layer.band_kind == RoadSurfaceBandKind::CurbOrShoulder {
            last_curb_layers = Some((*from_layer, *to_layer));
            push_bend_corner_curve_strips(
                &mut end_bands,
                from_mouth,
                from_layer,
                to_mouth,
                to_layer,
                source_band_index,
                include_curb_miter_caps,
            );
            if include_curb_miter_caps {
                push_bend_corner_curb_miter_cap(
                    &mut end_bands,
                    from_mouth,
                    from_layer,
                    to_mouth,
                    to_layer,
                    source_band_index,
                );
            }
        } else if from_layer.band_kind == RoadSurfaceBandKind::Sidewalk {
            if let Some((from_curb_layer, to_curb_layer)) = last_curb_layers {
                let uses_full_sidewalk_curve =
                    bend_corner_uses_full_sidewalk_curve(from_mouth, to_mouth);
                push_bend_corner_curb_curve_guard_bands(
                    &mut end_bands,
                    from_mouth,
                    &from_curb_layer,
                    from_mouth,
                    from_layer,
                    to_mouth,
                    to_layer,
                    from_curb_layer.band_index,
                    include_curb_miter_caps,
                    uses_full_sidewalk_curve,
                );
                if uses_full_sidewalk_curve {
                    push_bend_corner_curved_outer_band(
                        &mut end_bands,
                        from_mouth,
                        from_layer,
                        to_mouth,
                        to_layer,
                        source_band_index,
                        false,
                    );
                } else {
                    push_bend_corner_sidewalk_curved_outer_band(
                        &mut end_bands,
                        from_mouth,
                        from_layer,
                        &from_curb_layer,
                        to_mouth,
                        to_layer,
                        &to_curb_layer,
                        source_band_index,
                        false,
                    );
                }
                if include_curb_miter_caps && uses_full_sidewalk_curve {
                    push_bend_corner_miter_cap(
                        &mut end_bands,
                        from_mouth,
                        from_layer,
                        to_mouth,
                        to_layer,
                        source_band_index,
                    );
                }
            } else {
                push_bend_corner_curved_outer_band(
                    &mut end_bands,
                    from_mouth,
                    from_layer,
                    to_mouth,
                    to_layer,
                    source_band_index,
                    false,
                );
            }
        } else if from_layer.band_kind == RoadSurfaceBandKind::Carriageway {
            push_bend_corner_curve_strips(
                &mut end_bands,
                from_mouth,
                from_layer,
                to_mouth,
                to_layer,
                source_band_index,
                false,
            );
        } else {
            push_bend_corner_chord_band(
                &mut end_bands,
                from_mouth,
                from_layer,
                to_mouth,
                to_layer,
                source_band_index,
            );
            push_bend_corner_miter_cap(
                &mut end_bands,
                from_mouth,
                from_layer,
                to_mouth,
                to_layer,
                source_band_index,
            );
        }
    }
    end_bands
}

fn bend_corner_counterpart_end_bands(
    from_mouth: &NodeInputMouth,
    from_layers: &[BendCornerLayer],
    to_mouth: &NodeInputMouth,
    to_layers: &[BendCornerLayer],
) -> Vec<NodeInputTerminalEndBand> {
    let mut end_bands = Vec::new();
    let mut last_curb_layers = None;
    for (from_layer, to_layer) in from_layers.iter().zip(to_layers) {
        if from_layer.band_kind != to_layer.band_kind {
            break;
        }
        if from_layer.band_kind == RoadSurfaceBandKind::CurbOrShoulder {
            last_curb_layers = Some((*from_layer, *to_layer));
            push_bend_corner_curb_counterpart_miter_cap(
                &mut end_bands,
                from_mouth,
                from_layer,
                to_mouth,
                to_layer,
                to_layer.band_index,
            );
        } else if from_layer.band_kind == RoadSurfaceBandKind::Sidewalk
            && !bend_corner_uses_full_sidewalk_curve(from_mouth, to_mouth)
        {
            if let Some((_, to_curb_layer)) = last_curb_layers {
                push_bend_corner_curb_curve_guard_bands(
                    &mut end_bands,
                    to_mouth,
                    &to_curb_layer,
                    from_mouth,
                    from_layer,
                    to_mouth,
                    to_layer,
                    to_curb_layer.band_index,
                    true,
                    false,
                );
            }
        }
    }
    end_bands
}

fn bend_corner_uses_full_sidewalk_curve(
    from_mouth: &NodeInputMouth,
    to_mouth: &NodeInputMouth,
) -> bool {
    cross_xz(from_mouth.direction_xz, to_mouth.direction_xz) < 0.0
        && from_mouth.direction_xz.dot(to_mouth.direction_xz) <= 0.0
}

#[derive(Clone, Copy)]
enum BendCurbGuardBoundary {
    Lower,
    Raised,
}

#[derive(Clone, Copy)]
struct BendCurbGuardPoint {
    xz: RoadVec2,
    boundary: Option<BendCurbGuardBoundary>,
}

fn push_bend_corner_curb_curve_guard_bands(
    end_bands: &mut Vec<NodeInputTerminalEndBand>,
    mouth: &NodeInputMouth,
    curb_layer: &BendCornerLayer,
    from_mouth: &NodeInputMouth,
    from_layer: &BendCornerLayer,
    to_mouth: &NodeInputMouth,
    to_layer: &BendCornerLayer,
    source_band_index: usize,
    miter_prefix_owned_by_cap: bool,
    sidewalk_inner_boundary: bool,
) {
    let Some(lower_world) = endpoint_boundary_world(mouth, curb_layer.inner_boundary_index) else {
        return;
    };
    let Some(raised_world) = endpoint_boundary_world(mouth, curb_layer.outer_boundary_index) else {
        return;
    };
    let Some(inner_start_world) =
        endpoint_boundary_world(from_mouth, from_layer.inner_boundary_index)
    else {
        return;
    };
    let Some(inner_end_world) = endpoint_boundary_world(to_mouth, to_layer.inner_boundary_index)
    else {
        return;
    };
    let Some(outer_start_world) =
        endpoint_boundary_world(from_mouth, from_layer.outer_boundary_index)
    else {
        return;
    };
    let Some(outer_end_world) = endpoint_boundary_world(to_mouth, to_layer.outer_boundary_index)
    else {
        return;
    };
    let Some(inner_control_xz) = line_intersection_xz(
        xz_from_road_vec3(inner_start_world),
        from_mouth.direction_xz,
        xz_from_road_vec3(inner_end_world),
        to_mouth.direction_xz,
    ) else {
        return;
    };
    let Some(outer_control_xz) = line_intersection_xz(
        xz_from_road_vec3(outer_start_world),
        from_mouth.direction_xz,
        xz_from_road_vec3(outer_end_world),
        to_mouth.direction_xz,
    ) else {
        return;
    };

    let inner_control_world = RoadVec3::new(
        inner_control_xz.x,
        (inner_start_world.y + inner_end_world.y) * 0.5,
        inner_control_xz.y,
    );
    let outer_control_world = RoadVec3::new(
        outer_control_xz.x,
        (outer_start_world.y + outer_end_world.y) * 0.5,
        outer_control_xz.y,
    );

    let first_step = if miter_prefix_owned_by_cap {
        BEND_CORNER_CURVE_SEGMENTS / 2 + 1
    } else {
        1
    };
    let previous_t = (first_step - 1) as f64 / BEND_CORNER_CURVE_SEGMENTS as f64;
    let mut previous_inner = if first_step == 1 {
        inner_start_world
    } else {
        quadratic_bezier_world(
            inner_start_world,
            inner_control_world,
            inner_end_world,
            previous_t,
        )
    };
    let mut previous_outer = if first_step == 1 {
        outer_start_world
    } else {
        quadratic_bezier_world(
            outer_start_world,
            outer_control_world,
            outer_end_world,
            previous_t,
        )
    };
    for step in first_step..=BEND_CORNER_CURVE_SEGMENTS {
        let t = step as f64 / BEND_CORNER_CURVE_SEGMENTS as f64;
        let next_inner =
            quadratic_bezier_world(inner_start_world, inner_control_world, inner_end_world, t);
        let next_outer =
            quadratic_bezier_world(outer_start_world, outer_control_world, outer_end_world, t);
        if let Some(contour_world) = bend_curb_guard_contour_for_strip(
            previous_inner,
            next_inner,
            next_outer,
            previous_outer,
            lower_world,
            raised_world,
            mouth.direction_xz,
        ) {
            let inner_start_world = contour_world[0];
            let inner_end_world = contour_world[1];
            let outer_start_world = *contour_world
                .last()
                .expect("guard contour has at least 3 points");
            let outer_end_world = contour_world[2];
            end_bands.push(NodeInputTerminalEndBand {
                source_band_index,
                band_kind: RoadSurfaceBandKind::CurbOrShoulder,
                boundary_mode: if sidewalk_inner_boundary {
                    NodeInputTerminalEndBandBoundaryMode::CurbGuardWithinFootprint
                } else {
                    NodeInputTerminalEndBandBoundaryMode::MaterialBandWithinFootprint
                },
                inner_start_world,
                inner_end_world,
                outer_start_world,
                outer_end_world,
                contour_world,
            });
        }
        previous_inner = next_inner;
        previous_outer = next_outer;
    }
}

fn bend_curb_guard_contour_for_strip(
    previous_inner: RoadVec3,
    next_inner: RoadVec3,
    next_outer: RoadVec3,
    previous_outer: RoadVec3,
    lower_world: RoadVec3,
    raised_world: RoadVec3,
    direction_xz: RoadVec2,
) -> Option<Vec<RoadVec3>> {
    let lower_xz = xz_from_road_vec3(lower_world);
    let raised_xz = xz_from_road_vec3(raised_world);
    let signed_width = cross_xz(direction_xz, raised_xz - lower_xz);
    if signed_width.abs() <= f64::EPSILON {
        return None;
    }
    let side_sign = signed_width.signum();
    let mut points = vec![
        BendCurbGuardPoint {
            xz: xz_from_road_vec3(previous_inner),
            boundary: None,
        },
        BendCurbGuardPoint {
            xz: xz_from_road_vec3(next_inner),
            boundary: None,
        },
        BendCurbGuardPoint {
            xz: xz_from_road_vec3(next_outer),
            boundary: None,
        },
        BendCurbGuardPoint {
            xz: xz_from_road_vec3(previous_outer),
            boundary: None,
        },
    ];
    points = clip_bend_curb_guard_points(
        points,
        lower_xz,
        direction_xz,
        side_sign,
        BendCurbGuardBoundary::Lower,
    );
    points = clip_bend_curb_guard_points(
        points,
        raised_xz,
        direction_xz,
        -side_sign,
        BendCurbGuardBoundary::Raised,
    );
    rotate_bend_curb_guard_contour_to_lower_edge(points, raised_world)
}

fn clip_bend_curb_guard_points(
    points: Vec<BendCurbGuardPoint>,
    boundary_xz: RoadVec2,
    boundary_direction_xz: RoadVec2,
    inside_sign: f64,
    boundary: BendCurbGuardBoundary,
) -> Vec<BendCurbGuardPoint> {
    if points.is_empty() {
        return Vec::new();
    }

    let mut clipped = Vec::new();
    let mut previous = *points.last().expect("points are non-empty");
    let mut previous_inside =
        bend_curb_guard_point_inside(previous.xz, boundary_xz, boundary_direction_xz, inside_sign);
    for current in points {
        let current_inside = bend_curb_guard_point_inside(
            current.xz,
            boundary_xz,
            boundary_direction_xz,
            inside_sign,
        );
        if current_inside {
            if !previous_inside
                && let Some(intersection) = line_intersection_xz(
                    previous.xz,
                    current.xz - previous.xz,
                    boundary_xz,
                    boundary_direction_xz,
                )
            {
                clipped.push(BendCurbGuardPoint {
                    xz: intersection,
                    boundary: Some(boundary),
                });
            }
            clipped.push(current);
        } else if previous_inside
            && let Some(intersection) = line_intersection_xz(
                previous.xz,
                current.xz - previous.xz,
                boundary_xz,
                boundary_direction_xz,
            )
        {
            clipped.push(BendCurbGuardPoint {
                xz: intersection,
                boundary: Some(boundary),
            });
        }
        previous = current;
        previous_inside = current_inside;
    }
    remove_repeated_bend_curb_guard_points(&mut clipped);
    clipped
}

fn bend_curb_guard_point_inside(
    point: RoadVec2,
    boundary_xz: RoadVec2,
    boundary_direction_xz: RoadVec2,
    inside_sign: f64,
) -> bool {
    cross_xz(boundary_direction_xz, point - boundary_xz) * inside_sign >= 0.0
}

fn remove_repeated_bend_curb_guard_points(points: &mut Vec<BendCurbGuardPoint>) {
    points.dedup_by(|a, b| quantized_xz_key(a.xz) == quantized_xz_key(b.xz));
    if points.len() > 1
        && quantized_xz_key(points[0].xz)
            == quantized_xz_key(points.last().expect("points are non-empty").xz)
    {
        points.pop();
    }
}

fn rotate_bend_curb_guard_contour_to_lower_edge(
    points: Vec<BendCurbGuardPoint>,
    raised_world: RoadVec3,
) -> Option<Vec<RoadVec3>> {
    if points.len() < 3 {
        return None;
    }
    if !points
        .iter()
        .any(|point| matches!(point.boundary, Some(BendCurbGuardBoundary::Raised)))
    {
        return None;
    }
    let lower_edge_index = points.iter().enumerate().find_map(|(index, point)| {
        let next_index = (index + 1) % points.len();
        let next = points[next_index];
        if matches!(point.boundary, Some(BendCurbGuardBoundary::Lower))
            && matches!(next.boundary, Some(BendCurbGuardBoundary::Lower))
            && quantized_xz_key(point.xz) != quantized_xz_key(next.xz)
        {
            Some(index)
        } else {
            None
        }
    })?;

    let mut contour_world = Vec::with_capacity(points.len());
    for offset in 0..points.len() {
        let point = points[(lower_edge_index + offset) % points.len()];
        contour_world.push(bend_curb_guard_world_point(point, raised_world));
    }
    if contour_world.len() >= 3 {
        Some(contour_world)
    } else {
        None
    }
}

fn bend_curb_guard_world_point(point: BendCurbGuardPoint, raised_world: RoadVec3) -> RoadVec3 {
    RoadVec3::new(point.xz.x, raised_world.y, point.xz.y)
}

fn push_bend_corner_curb_miter_cap(
    end_bands: &mut Vec<NodeInputTerminalEndBand>,
    from_mouth: &NodeInputMouth,
    from_layer: &BendCornerLayer,
    to_mouth: &NodeInputMouth,
    to_layer: &BendCornerLayer,
    source_band_index: usize,
) {
    push_bend_corner_curb_curve_cap(
        end_bands,
        from_mouth,
        from_layer,
        to_mouth,
        to_layer,
        source_band_index,
        0,
        bend_corner_miter_split_step(),
    );
}

fn push_bend_corner_curb_counterpart_miter_cap(
    end_bands: &mut Vec<NodeInputTerminalEndBand>,
    from_mouth: &NodeInputMouth,
    from_layer: &BendCornerLayer,
    to_mouth: &NodeInputMouth,
    to_layer: &BendCornerLayer,
    source_band_index: usize,
) {
    push_bend_corner_curb_curve_cap(
        end_bands,
        from_mouth,
        from_layer,
        to_mouth,
        to_layer,
        source_band_index,
        bend_corner_miter_split_step(),
        BEND_CORNER_CURVE_SEGMENTS,
    );
}

fn push_bend_corner_curb_curve_cap(
    end_bands: &mut Vec<NodeInputTerminalEndBand>,
    from_mouth: &NodeInputMouth,
    from_layer: &BendCornerLayer,
    to_mouth: &NodeInputMouth,
    to_layer: &BendCornerLayer,
    source_band_index: usize,
    start_step: usize,
    end_step: usize,
) {
    let Some(from_inner_world) = endpoint_layer_inner_world(from_mouth, from_layer) else {
        return;
    };
    let Some(to_inner_world) = endpoint_layer_inner_world(to_mouth, to_layer) else {
        return;
    };
    let Some(from_outer_world) = endpoint_layer_outer_world(from_mouth, from_layer) else {
        return;
    };
    let Some(to_outer_world) = endpoint_layer_outer_world(to_mouth, to_layer) else {
        return;
    };
    let Some(inner_miter_xz) = line_intersection_xz(
        xz_from_road_vec3(from_inner_world),
        from_mouth.direction_xz,
        xz_from_road_vec3(to_inner_world),
        to_mouth.direction_xz,
    ) else {
        return;
    };
    let Some(outer_miter_xz) = line_intersection_xz(
        xz_from_road_vec3(from_outer_world),
        from_mouth.direction_xz,
        xz_from_road_vec3(to_outer_world),
        to_mouth.direction_xz,
    ) else {
        return;
    };

    let inner_control_world = RoadVec3::new(
        inner_miter_xz.x,
        (from_inner_world.y + to_inner_world.y) * 0.5,
        inner_miter_xz.y,
    );
    let outer_control_world = RoadVec3::new(
        outer_miter_xz.x,
        (from_outer_world.y + to_outer_world.y) * 0.5,
        outer_miter_xz.y,
    );
    let inner_points = bend_corner_curve_points(
        from_inner_world,
        inner_control_world,
        to_inner_world,
        start_step,
        end_step,
    );
    let outer_points = bend_corner_curve_points(
        from_outer_world,
        outer_control_world,
        to_outer_world,
        start_step,
        end_step,
    );
    if inner_points.len() < 2 || outer_points.len() != inner_points.len() {
        return;
    }
    let inner_start_world = inner_points[0];
    let inner_end_world = *inner_points
        .last()
        .expect("curb cap inner curve has at least two points");
    let outer_start_world = outer_points[0];
    let outer_end_world = *outer_points
        .last()
        .expect("curb cap outer curve has at least two points");
    let (height_inner_start_world, height_inner_end_world) = nondegenerate_height_edge(
        inner_start_world,
        inner_end_world,
        outer_start_world,
        outer_end_world,
    );
    let (height_outer_start_world, height_outer_end_world) = nondegenerate_height_edge(
        outer_start_world,
        outer_end_world,
        inner_start_world,
        inner_end_world,
    );
    let mut contour_world = inner_points;
    contour_world.extend(outer_points.into_iter().rev());

    end_bands.push(NodeInputTerminalEndBand {
        source_band_index,
        band_kind: RoadSurfaceBandKind::CurbOrShoulder,
        boundary_mode: NodeInputTerminalEndBandBoundaryMode::TerminalMaterialBand,
        inner_start_world: height_inner_start_world,
        inner_end_world: height_inner_end_world,
        outer_start_world: height_outer_start_world,
        outer_end_world: height_outer_end_world,
        contour_world,
    });
}

fn push_bend_corner_chord_band(
    end_bands: &mut Vec<NodeInputTerminalEndBand>,
    from_mouth: &NodeInputMouth,
    from_layer: &BendCornerLayer,
    to_mouth: &NodeInputMouth,
    to_layer: &BendCornerLayer,
    source_band_index: usize,
) {
    let Some(inner_start_world) = endpoint_layer_inner_world(from_mouth, from_layer) else {
        return;
    };
    let Some(inner_end_world) = endpoint_layer_inner_world(to_mouth, to_layer) else {
        return;
    };
    let Some(outer_start_world) = endpoint_layer_outer_world(from_mouth, from_layer) else {
        return;
    };
    let Some(outer_end_world) = endpoint_layer_outer_world(to_mouth, to_layer) else {
        return;
    };
    let (height_inner_start_world, height_inner_end_world) = nondegenerate_height_edge(
        inner_start_world,
        inner_end_world,
        outer_start_world,
        outer_end_world,
    );
    let contour_world = if xz_from_road_vec3(inner_start_world)
        .distance_squared(xz_from_road_vec3(inner_end_world))
        <= BEND_CORNER_HEIGHT_EDGE_EPS_M * BEND_CORNER_HEIGHT_EDGE_EPS_M
    {
        vec![inner_start_world, outer_end_world, outer_start_world]
    } else {
        vec![
            inner_start_world,
            inner_end_world,
            outer_end_world,
            outer_start_world,
        ]
    };
    end_bands.push(NodeInputTerminalEndBand {
        source_band_index,
        band_kind: from_layer.band_kind,
        boundary_mode: NodeInputTerminalEndBandBoundaryMode::MaterialBandWithSameOwnerOuterCap,
        inner_start_world: height_inner_start_world,
        inner_end_world: height_inner_end_world,
        outer_start_world,
        outer_end_world,
        contour_world,
    });
}

fn push_bend_corner_curved_outer_band(
    end_bands: &mut Vec<NodeInputTerminalEndBand>,
    from_mouth: &NodeInputMouth,
    from_layer: &BendCornerLayer,
    to_mouth: &NodeInputMouth,
    to_layer: &BendCornerLayer,
    source_band_index: usize,
    miter_prefix_owned_by_cap: bool,
) {
    let Some(inner_start_world) = endpoint_layer_inner_world(from_mouth, from_layer) else {
        return;
    };
    let Some(inner_end_world) = endpoint_layer_inner_world(to_mouth, to_layer) else {
        return;
    };
    let Some(outer_start_world) = endpoint_layer_outer_world(from_mouth, from_layer) else {
        return;
    };
    let Some(outer_end_world) = endpoint_layer_outer_world(to_mouth, to_layer) else {
        return;
    };
    let Some(inner_miter_xz) = line_intersection_xz(
        xz_from_road_vec3(inner_start_world),
        from_mouth.direction_xz,
        xz_from_road_vec3(inner_end_world),
        to_mouth.direction_xz,
    ) else {
        return;
    };
    let Some(miter_xz) = line_intersection_xz(
        xz_from_road_vec3(outer_start_world),
        from_mouth.direction_xz,
        xz_from_road_vec3(outer_end_world),
        to_mouth.direction_xz,
    ) else {
        return;
    };

    let inner_miter_world = RoadVec3::new(
        inner_miter_xz.x,
        (inner_start_world.y + inner_end_world.y) * 0.5,
        inner_miter_xz.y,
    );
    let miter_height_m = (outer_start_world.y + outer_end_world.y) * 0.5;
    let miter_world = RoadVec3::new(miter_xz.x, miter_height_m, miter_xz.y);

    let first_step = if miter_prefix_owned_by_cap {
        BEND_CORNER_CURVE_SEGMENTS / 2 + 1
    } else {
        1
    };
    let previous_t = (first_step - 1) as f64 / BEND_CORNER_CURVE_SEGMENTS as f64;
    let mut previous_inner = if first_step == 1 {
        inner_start_world
    } else {
        quadratic_bezier_world(
            inner_start_world,
            inner_miter_world,
            inner_end_world,
            previous_t,
        )
    };
    let mut previous_outer = if first_step == 1 {
        outer_start_world
    } else {
        quadratic_bezier_world(outer_start_world, miter_world, outer_end_world, previous_t)
    };
    for step in first_step..=BEND_CORNER_CURVE_SEGMENTS {
        let t = step as f64 / BEND_CORNER_CURVE_SEGMENTS as f64;
        let next_inner =
            quadratic_bezier_world(inner_start_world, inner_miter_world, inner_end_world, t);
        let next_outer = quadratic_bezier_world(outer_start_world, miter_world, outer_end_world, t);
        let (
            height_endpoint_start_world,
            height_endpoint_end_world,
            height_mouth_start_world,
            height_mouth_end_world,
        ) = bend_corner_strip_height_edges(previous_inner, next_inner, previous_outer, next_outer);
        let contour_world = if xz_from_road_vec3(previous_inner)
            .distance_squared(xz_from_road_vec3(next_inner))
            <= BEND_CORNER_HEIGHT_EDGE_EPS_M * BEND_CORNER_HEIGHT_EDGE_EPS_M
        {
            vec![previous_inner, next_outer, previous_outer]
        } else {
            vec![previous_inner, next_inner, next_outer, previous_outer]
        };
        end_bands.push(NodeInputTerminalEndBand {
            source_band_index,
            band_kind: from_layer.band_kind,
            boundary_mode: NodeInputTerminalEndBandBoundaryMode::MaterialBand,
            inner_start_world: height_endpoint_start_world,
            inner_end_world: height_endpoint_end_world,
            outer_start_world: height_mouth_start_world,
            outer_end_world: height_mouth_end_world,
            contour_world,
        });
        previous_inner = next_inner;
        previous_outer = next_outer;
    }
}

fn push_bend_corner_sidewalk_curved_outer_band(
    end_bands: &mut Vec<NodeInputTerminalEndBand>,
    from_mouth: &NodeInputMouth,
    from_layer: &BendCornerLayer,
    from_curb_layer: &BendCornerLayer,
    to_mouth: &NodeInputMouth,
    to_layer: &BendCornerLayer,
    to_curb_layer: &BendCornerLayer,
    source_band_index: usize,
    miter_prefix_owned_by_cap: bool,
) {
    let Some(inner_start_world) = endpoint_layer_inner_world(from_mouth, from_layer) else {
        return;
    };
    let Some(inner_end_world) = endpoint_layer_inner_world(to_mouth, to_layer) else {
        return;
    };
    let Some(outer_start_world) = endpoint_layer_outer_world(from_mouth, from_layer) else {
        return;
    };
    let Some(outer_end_world) = endpoint_layer_outer_world(to_mouth, to_layer) else {
        return;
    };
    let Some(inner_miter_xz) = line_intersection_xz(
        xz_from_road_vec3(inner_start_world),
        from_mouth.direction_xz,
        xz_from_road_vec3(inner_end_world),
        to_mouth.direction_xz,
    ) else {
        return;
    };
    let Some(outer_miter_xz) = line_intersection_xz(
        xz_from_road_vec3(outer_start_world),
        from_mouth.direction_xz,
        xz_from_road_vec3(outer_end_world),
        to_mouth.direction_xz,
    ) else {
        return;
    };
    let Some(from_lower_world) =
        endpoint_boundary_world(from_mouth, from_curb_layer.inner_boundary_index)
    else {
        return;
    };
    let Some(from_raised_world) =
        endpoint_boundary_world(from_mouth, from_curb_layer.outer_boundary_index)
    else {
        return;
    };
    let Some(to_lower_world) =
        endpoint_boundary_world(to_mouth, to_curb_layer.inner_boundary_index)
    else {
        return;
    };
    let Some(to_raised_world) =
        endpoint_boundary_world(to_mouth, to_curb_layer.outer_boundary_index)
    else {
        return;
    };
    let Some(from_raised_inside_sign) =
        sidewalk_raised_boundary_inside_sign(from_mouth, from_lower_world, from_raised_world)
    else {
        return;
    };
    let Some(to_raised_inside_sign) =
        sidewalk_raised_boundary_inside_sign(to_mouth, to_lower_world, to_raised_world)
    else {
        return;
    };

    let inner_miter_world = RoadVec3::new(
        inner_miter_xz.x,
        (inner_start_world.y + inner_end_world.y) * 0.5,
        inner_miter_xz.y,
    );
    let outer_miter_world = RoadVec3::new(
        outer_miter_xz.x,
        (outer_start_world.y + outer_end_world.y) * 0.5,
        outer_miter_xz.y,
    );
    let first_step = if miter_prefix_owned_by_cap {
        BEND_CORNER_CURVE_SEGMENTS / 2 + 1
    } else {
        1
    };
    let previous_t = (first_step - 1) as f64 / BEND_CORNER_CURVE_SEGMENTS as f64;
    let mut previous_inner = if first_step == 1 {
        inner_start_world
    } else {
        quadratic_bezier_world(
            inner_start_world,
            inner_miter_world,
            inner_end_world,
            previous_t,
        )
    };
    let mut previous_outer = if first_step == 1 {
        outer_start_world
    } else {
        quadratic_bezier_world(
            outer_start_world,
            outer_miter_world,
            outer_end_world,
            previous_t,
        )
    };
    for step in first_step..=BEND_CORNER_CURVE_SEGMENTS {
        let t = step as f64 / BEND_CORNER_CURVE_SEGMENTS as f64;
        let next_inner =
            quadratic_bezier_world(inner_start_world, inner_miter_world, inner_end_world, t);
        let next_outer =
            quadratic_bezier_world(outer_start_world, outer_miter_world, outer_end_world, t);
        let mut contour_world = vec![previous_inner, next_inner, next_outer, previous_outer];
        contour_world = clip_bend_world_contour_to_half_plane(
            contour_world,
            xz_from_road_vec3(from_raised_world),
            from_mouth.direction_xz,
            from_raised_inside_sign,
        );
        contour_world = clip_bend_world_contour_to_half_plane(
            contour_world,
            xz_from_road_vec3(to_raised_world),
            to_mouth.direction_xz,
            to_raised_inside_sign,
        );
        if contour_world.len() >= 3 && road_vec3_contour_has_quantized_area(&contour_world) {
            let (height_inner_start_world, height_inner_end_world) = nondegenerate_height_edge(
                contour_world[0],
                contour_world[1],
                next_outer,
                previous_outer,
            );
            let outer_start_world = *contour_world
                .last()
                .expect("sidewalk contour has at least 3 points");
            let outer_end_world = contour_world[2];
            let (height_outer_start_world, height_outer_end_world) = nondegenerate_height_edge(
                outer_start_world,
                outer_end_world,
                contour_world[0],
                contour_world[1],
            );
            end_bands.push(NodeInputTerminalEndBand {
                source_band_index,
                band_kind: from_layer.band_kind,
                boundary_mode: NodeInputTerminalEndBandBoundaryMode::MaterialBand,
                inner_start_world: height_inner_start_world,
                inner_end_world: height_inner_end_world,
                outer_start_world: height_outer_start_world,
                outer_end_world: height_outer_end_world,
                contour_world,
            });
        }
        previous_inner = next_inner;
        previous_outer = next_outer;
    }
}

fn sidewalk_raised_boundary_inside_sign(
    mouth: &NodeInputMouth,
    lower_world: RoadVec3,
    raised_world: RoadVec3,
) -> Option<f64> {
    let signed_width = cross_xz(
        mouth.direction_xz,
        xz_from_road_vec3(raised_world) - xz_from_road_vec3(lower_world),
    );
    (signed_width.abs() > f64::EPSILON).then_some(signed_width.signum())
}

fn clip_bend_world_contour_to_half_plane(
    points: Vec<RoadVec3>,
    boundary_xz: RoadVec2,
    boundary_direction_xz: RoadVec2,
    inside_sign: f64,
) -> Vec<RoadVec3> {
    if points.is_empty() {
        return Vec::new();
    }

    let mut clipped = Vec::new();
    let mut previous = *points.last().expect("points are non-empty");
    let mut previous_inside = bend_curb_guard_point_inside(
        xz_from_road_vec3(previous),
        boundary_xz,
        boundary_direction_xz,
        inside_sign,
    );
    for current in points {
        let current_inside = bend_curb_guard_point_inside(
            xz_from_road_vec3(current),
            boundary_xz,
            boundary_direction_xz,
            inside_sign,
        );
        if current_inside {
            if !previous_inside
                && let Some(intersection) = bend_world_segment_line_intersection(
                    previous,
                    current,
                    boundary_xz,
                    boundary_direction_xz,
                )
            {
                clipped.push(intersection);
            }
            clipped.push(current);
        } else if previous_inside
            && let Some(intersection) = bend_world_segment_line_intersection(
                previous,
                current,
                boundary_xz,
                boundary_direction_xz,
            )
        {
            clipped.push(intersection);
        }
        previous = current;
        previous_inside = current_inside;
    }
    remove_repeated_road_vec3_points(&mut clipped);
    clipped
}

fn bend_world_segment_line_intersection(
    start_world: RoadVec3,
    end_world: RoadVec3,
    line_start_xz: RoadVec2,
    line_direction_xz: RoadVec2,
) -> Option<RoadVec3> {
    let start_xz = xz_from_road_vec3(start_world);
    let end_xz = xz_from_road_vec3(end_world);
    let intersection_xz = line_intersection_xz(
        start_xz,
        end_xz - start_xz,
        line_start_xz,
        line_direction_xz,
    )?;
    let axis = end_xz - start_xz;
    let denominator = if axis.x.abs() >= axis.y.abs() {
        axis.x
    } else {
        axis.y
    };
    if denominator.abs() <= f64::EPSILON {
        return None;
    }
    let numerator = if axis.x.abs() >= axis.y.abs() {
        intersection_xz.x - start_xz.x
    } else {
        intersection_xz.y - start_xz.y
    };
    let t = numerator / denominator;
    Some(RoadVec3::new(
        intersection_xz.x,
        start_world.y + (end_world.y - start_world.y) * t,
        intersection_xz.y,
    ))
}

fn remove_repeated_road_vec3_points(points: &mut Vec<RoadVec3>) {
    points.dedup_by(|a, b| {
        quantized_xz_key(xz_from_road_vec3(*a)) == quantized_xz_key(xz_from_road_vec3(*b))
    });
    if points.len() > 1
        && quantized_xz_key(xz_from_road_vec3(points[0]))
            == quantized_xz_key(xz_from_road_vec3(
                *points.last().expect("points are non-empty"),
            ))
    {
        points.pop();
    }
}

fn road_vec3_contour_has_quantized_area(contour_world: &[RoadVec3]) -> bool {
    let mut keys = Vec::with_capacity(contour_world.len());
    for point in contour_world {
        let key = quantized_xz_key(xz_from_road_vec3(*point));
        if keys.last().copied() != Some(key) {
            keys.push(key);
        }
    }
    if keys.len() > 1 && keys.first() == keys.last() {
        keys.pop();
    }
    if keys.len() < 3 {
        return false;
    }
    let mut double_area = 0_i128;
    for index in 0..keys.len() {
        let (x0, z0) = keys[index];
        let (x1, z1) = keys[(index + 1) % keys.len()];
        double_area += x0 as i128 * z1 as i128 - x1 as i128 * z0 as i128;
    }
    let area_m2 = double_area.unsigned_abs() as f64 * 0.5 / ROAD_OVERLAY_COORDINATE_SCALE.powi(2);
    area_m2 > NODE_OVERLAY_MIN_AREA_M2 as f64
}

fn push_bend_corner_curve_strips(
    end_bands: &mut Vec<NodeInputTerminalEndBand>,
    from_mouth: &NodeInputMouth,
    from_layer: &BendCornerLayer,
    to_mouth: &NodeInputMouth,
    to_layer: &BendCornerLayer,
    source_band_index: usize,
    miter_prefix_owned_by_cap: bool,
) {
    let Some(from_inner_world) = endpoint_layer_inner_world(from_mouth, from_layer) else {
        return;
    };
    let Some(to_inner_world) = endpoint_layer_inner_world(to_mouth, to_layer) else {
        return;
    };
    let Some(from_outer_world) = endpoint_layer_outer_world(from_mouth, from_layer) else {
        return;
    };
    let Some(to_outer_world) = endpoint_layer_outer_world(to_mouth, to_layer) else {
        return;
    };
    let Some(inner_control_xz) = line_intersection_xz(
        xz_from_road_vec3(from_inner_world),
        from_mouth.direction_xz,
        xz_from_road_vec3(to_inner_world),
        to_mouth.direction_xz,
    ) else {
        return;
    };
    let Some(outer_control_xz) = line_intersection_xz(
        xz_from_road_vec3(from_outer_world),
        from_mouth.direction_xz,
        xz_from_road_vec3(to_outer_world),
        to_mouth.direction_xz,
    ) else {
        return;
    };

    let inner_control_world = RoadVec3::new(
        inner_control_xz.x,
        (from_inner_world.y + to_inner_world.y) * 0.5,
        inner_control_xz.y,
    );
    let outer_control_world = RoadVec3::new(
        outer_control_xz.x,
        (from_outer_world.y + to_outer_world.y) * 0.5,
        outer_control_xz.y,
    );
    let first_step = if miter_prefix_owned_by_cap {
        BEND_CORNER_CURVE_SEGMENTS / 2 + 1
    } else {
        1
    };
    let previous_t = (first_step - 1) as f64 / BEND_CORNER_CURVE_SEGMENTS as f64;
    let mut previous_inner = if first_step == 1 {
        from_inner_world
    } else {
        quadratic_bezier_world(
            from_inner_world,
            inner_control_world,
            to_inner_world,
            previous_t,
        )
    };
    let mut previous_outer = if first_step == 1 {
        from_outer_world
    } else {
        quadratic_bezier_world(
            from_outer_world,
            outer_control_world,
            to_outer_world,
            previous_t,
        )
    };
    for step in first_step..=BEND_CORNER_CURVE_SEGMENTS {
        let t = step as f64 / BEND_CORNER_CURVE_SEGMENTS as f64;
        let next_inner =
            quadratic_bezier_world(from_inner_world, inner_control_world, to_inner_world, t);
        let next_outer =
            quadratic_bezier_world(from_outer_world, outer_control_world, to_outer_world, t);
        let (
            height_endpoint_start_world,
            height_endpoint_end_world,
            height_mouth_start_world,
            height_mouth_end_world,
        ) = bend_corner_strip_height_edges(previous_inner, next_inner, previous_outer, next_outer);
        let contour_world = if xz_from_road_vec3(previous_inner)
            .distance_squared(xz_from_road_vec3(next_inner))
            <= BEND_CORNER_HEIGHT_EDGE_EPS_M * BEND_CORNER_HEIGHT_EDGE_EPS_M
        {
            vec![previous_inner, next_outer, previous_outer]
        } else {
            vec![previous_inner, next_inner, next_outer, previous_outer]
        };
        end_bands.push(NodeInputTerminalEndBand {
            source_band_index,
            band_kind: from_layer.band_kind,
            boundary_mode: NodeInputTerminalEndBandBoundaryMode::MaterialBand,
            inner_start_world: height_endpoint_start_world,
            inner_end_world: height_endpoint_end_world,
            outer_start_world: height_mouth_start_world,
            outer_end_world: height_mouth_end_world,
            contour_world,
        });
        previous_inner = next_inner;
        previous_outer = next_outer;
    }
}

fn push_bend_corner_miter_cap(
    end_bands: &mut Vec<NodeInputTerminalEndBand>,
    from_mouth: &NodeInputMouth,
    from_layer: &BendCornerLayer,
    to_mouth: &NodeInputMouth,
    to_layer: &BendCornerLayer,
    source_band_index: usize,
) {
    let Some(from_outer_world) = endpoint_layer_outer_world(from_mouth, from_layer) else {
        return;
    };
    let Some(to_outer_world) = endpoint_layer_outer_world(to_mouth, to_layer) else {
        return;
    };
    let Some(miter_xz) = line_intersection_xz(
        xz_from_road_vec3(from_outer_world),
        from_mouth.direction_xz,
        xz_from_road_vec3(to_outer_world),
        to_mouth.direction_xz,
    ) else {
        return;
    };

    let inner_axis = xz_from_road_vec3(to_outer_world) - xz_from_road_vec3(from_outer_world);
    let inner_axis_len = inner_axis.length();
    if inner_axis_len <= f64::EPSILON {
        return;
    }
    let miter_height_m = (from_outer_world.y + to_outer_world.y) * 0.5;
    let miter_world = RoadVec3::new(miter_xz.x, miter_height_m, miter_xz.y);
    let miter_axis = inner_axis / inner_axis_len * BEND_CORNER_HEIGHT_EDGE_EPS_M;
    let outer_start_world = RoadVec3::new(
        miter_xz.x - miter_axis.x,
        miter_height_m,
        miter_xz.y - miter_axis.y,
    );
    let outer_end_world = RoadVec3::new(
        miter_xz.x + miter_axis.x,
        miter_height_m,
        miter_xz.y + miter_axis.y,
    );
    end_bands.push(NodeInputTerminalEndBand {
        source_band_index,
        band_kind: from_layer.band_kind,
        boundary_mode: NodeInputTerminalEndBandBoundaryMode::SameOwnerOuterCap,
        inner_start_world: from_outer_world,
        inner_end_world: to_outer_world,
        outer_start_world,
        outer_end_world,
        contour_world: bend_corner_miter_cap_contour(from_outer_world, to_outer_world, miter_world),
    });
}

fn bend_corner_strip_height_edges(
    endpoint_start_world: RoadVec3,
    endpoint_end_world: RoadVec3,
    mouth_start_world: RoadVec3,
    mouth_end_world: RoadVec3,
) -> (RoadVec3, RoadVec3, RoadVec3, RoadVec3) {
    let (endpoint_start_world, endpoint_end_world) = nondegenerate_height_edge(
        endpoint_start_world,
        endpoint_end_world,
        mouth_start_world,
        mouth_end_world,
    );
    let (mouth_start_world, mouth_end_world) = nondegenerate_height_edge(
        mouth_start_world,
        mouth_end_world,
        endpoint_start_world,
        endpoint_end_world,
    );
    (
        endpoint_start_world,
        endpoint_end_world,
        mouth_start_world,
        mouth_end_world,
    )
}

fn nondegenerate_height_edge(
    start_world: RoadVec3,
    end_world: RoadVec3,
    fallback_start_world: RoadVec3,
    fallback_end_world: RoadVec3,
) -> (RoadVec3, RoadVec3) {
    if xz_from_road_vec3(start_world).distance_squared(xz_from_road_vec3(end_world))
        > BEND_CORNER_HEIGHT_EDGE_EPS_M * BEND_CORNER_HEIGHT_EDGE_EPS_M
    {
        return (start_world, end_world);
    }

    let fallback_axis =
        xz_from_road_vec3(fallback_end_world) - xz_from_road_vec3(fallback_start_world);
    let axis = if fallback_axis.length() > f64::EPSILON {
        fallback_axis.normalize() * BEND_CORNER_HEIGHT_EDGE_EPS_M
    } else {
        RoadVec2::new(BEND_CORNER_HEIGHT_EDGE_EPS_M, 0.0)
    };
    let center = RoadVec3::new(
        (start_world.x + end_world.x) * 0.5,
        (start_world.y + end_world.y) * 0.5,
        (start_world.z + end_world.z) * 0.5,
    );
    (
        RoadVec3::new(center.x - axis.x, center.y, center.z - axis.y),
        RoadVec3::new(center.x + axis.x, center.y, center.z + axis.y),
    )
}

fn bend_corner_miter_cap_contour(
    from_outer_world: RoadVec3,
    to_outer_world: RoadVec3,
    control_world: RoadVec3,
) -> Vec<RoadVec3> {
    vec![from_outer_world, to_outer_world, control_world]
}

fn bend_corner_curve_points(
    start: RoadVec3,
    control: RoadVec3,
    end: RoadVec3,
    start_step: usize,
    end_step: usize,
) -> Vec<RoadVec3> {
    let start_step = start_step.min(BEND_CORNER_CURVE_SEGMENTS);
    let end_step = end_step.min(BEND_CORNER_CURVE_SEGMENTS);
    if start_step > end_step {
        return Vec::new();
    }
    (start_step..=end_step)
        .map(|step| {
            if step == 0 {
                start
            } else if step == BEND_CORNER_CURVE_SEGMENTS {
                end
            } else {
                let t = step as f64 / BEND_CORNER_CURVE_SEGMENTS as f64;
                quadratic_bezier_world(start, control, end, t)
            }
        })
        .collect()
}

fn quadratic_bezier_world(start: RoadVec3, control: RoadVec3, end: RoadVec3, t: f64) -> RoadVec3 {
    let one_minus_t = 1.0 - t;
    start * (one_minus_t * one_minus_t) + control * (2.0 * one_minus_t * t) + end * (t * t)
}

fn endpoint_boundary_world(mouth: &NodeInputMouth, boundary_index: usize) -> Option<RoadVec3> {
    mouth
        .boundary_rails
        .get(boundary_index)
        .map(|rail| rail.endpoint_world)
}

fn endpoint_layer_inner_world(mouth: &NodeInputMouth, layer: &BendCornerLayer) -> Option<RoadVec3> {
    endpoint_layer_boundary_world(mouth, layer, layer.inner_boundary_index)
}

fn endpoint_layer_outer_world(mouth: &NodeInputMouth, layer: &BendCornerLayer) -> Option<RoadVec3> {
    endpoint_layer_boundary_world(mouth, layer, layer.outer_boundary_index)
}

fn endpoint_layer_boundary_world(
    mouth: &NodeInputMouth,
    layer: &BendCornerLayer,
    boundary_index: usize,
) -> Option<RoadVec3> {
    let interval = mouth.band_intervals.get(layer.band_index)?;
    if boundary_index == layer.band_index {
        Some(interval.endpoint_start_world)
    } else if boundary_index == layer.band_index + 1 {
        Some(interval.endpoint_end_world)
    } else {
        endpoint_boundary_world(mouth, boundary_index)
    }
}

fn line_intersection_xz(
    start_a: RoadVec2,
    direction_a: RoadVec2,
    start_b: RoadVec2,
    direction_b: RoadVec2,
) -> Option<RoadVec2> {
    let det = cross_xz(direction_a, direction_b);
    if det.abs() <= f64::EPSILON {
        return None;
    }
    let offset = start_b - start_a;
    let t = cross_xz(offset, direction_b) / det;
    Some(start_a + direction_a * t)
}

fn cross_xz(a: RoadVec2, b: RoadVec2) -> f64 {
    a.x * b.y - a.y * b.x
}

fn xz_from_road_vec3(point: RoadVec3) -> RoadVec2 {
    RoadVec2::new(point.x, point.z)
}

fn terminal_end_bands(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    mouth: &OrderedIncidentPieceMouth,
    next_source_band_index: usize,
) -> Vec<NodeInputTerminalEndBand> {
    if piece_kind != RoadSurfaceVisualNodePieceKind::Terminal {
        return Vec::new();
    }

    let Some(first_carriageway) = mouth
        .endpoint_profile
        .bands
        .iter()
        .position(|band| band.kind == RoadSurfaceBandKind::Carriageway)
    else {
        return Vec::new();
    };
    let Some(last_carriageway) = mouth
        .endpoint_profile
        .bands
        .iter()
        .rposition(|band| band.kind == RoadSurfaceBandKind::Carriageway)
    else {
        return Vec::new();
    };
    if first_carriageway == 0
        || last_carriageway + 1 >= mouth.endpoint_profile.bands.len()
        || mouth.endpoint_profile.boundary_points_world.len()
            != mouth.endpoint_profile.bands.len() + 1
    {
        return Vec::new();
    }

    let inward = godot_vec2_to_road(mouth.endpoint_profile.inward_direction_xz);
    let inward_len = inward.length();
    if inward_len <= f64::EPSILON {
        return Vec::new();
    }
    let outward = -(inward / inward_len);
    let paired_layers =
        first_carriageway.min(mouth.endpoint_profile.bands.len() - last_carriageway - 1);
    let mut end_bands = Vec::new();
    let mut inner_offset_m = 0.0;
    let mut next_terminal_source_band_index = next_source_band_index;

    for layer_index in 0..paired_layers {
        let left_band_index = first_carriageway - 1 - layer_index;
        let right_band_index = last_carriageway + 1 + layer_index;
        let left_band = &mouth.endpoint_profile.bands[left_band_index];
        let right_band = &mouth.endpoint_profile.bands[right_band_index];
        if left_band.kind != right_band.kind || left_band.kind == RoadSurfaceBandKind::Carriageway {
            break;
        }

        let depth_m = band_width_m(left_band).min(band_width_m(right_band));
        if depth_m <= f64::EPSILON {
            continue;
        }
        let outer_offset_m = inner_offset_m + depth_m;
        if layer_index == 0 && left_band.kind == RoadSurfaceBandKind::CurbOrShoulder {
            push_terminal_curb_end_bands(
                &mut end_bands,
                mouth,
                outward,
                next_terminal_source_band_index,
                left_band_index,
                right_band_index,
                outer_offset_m,
            );
            next_terminal_source_band_index += 1;
            inner_offset_m = outer_offset_m;
            continue;
        }

        push_terminal_paired_end_bands(
            &mut end_bands,
            mouth,
            outward,
            next_terminal_source_band_index,
            left_band_index,
            right_band_index,
            inner_offset_m,
            outer_offset_m,
        );
        next_terminal_source_band_index += 1;
        inner_offset_m = outer_offset_m;
    }

    end_bands
}

fn push_terminal_paired_end_bands(
    end_bands: &mut Vec<NodeInputTerminalEndBand>,
    mouth: &OrderedIncidentPieceMouth,
    outward: RoadVec2,
    center_source_band_index: usize,
    left_band_index: usize,
    right_band_index: usize,
    inner_offset_m: f64,
    outer_offset_m: f64,
) {
    let band_kind = mouth.endpoint_profile.bands[left_band_index].kind;
    push_terminal_end_band(
        end_bands,
        center_source_band_index,
        band_kind,
        offset_endpoint_boundary_point(&mouth.endpoint_profile, left_band_index, outward, 0.0),
        offset_endpoint_boundary_point(&mouth.endpoint_profile, left_band_index + 1, outward, 0.0),
        offset_endpoint_boundary_point(
            &mouth.endpoint_profile,
            left_band_index,
            outward,
            outer_offset_m,
        ),
        offset_endpoint_boundary_point(
            &mouth.endpoint_profile,
            left_band_index + 1,
            outward,
            outer_offset_m,
        ),
    );
    push_terminal_center_end_band(
        end_bands,
        center_source_band_index,
        band_kind,
        &mouth.endpoint_profile,
        outward,
        left_band_index + 1,
        right_band_index,
        inner_offset_m,
        outer_offset_m,
    );
    push_terminal_end_band(
        end_bands,
        center_source_band_index,
        band_kind,
        offset_endpoint_boundary_point(&mouth.endpoint_profile, right_band_index, outward, 0.0),
        offset_endpoint_boundary_point(&mouth.endpoint_profile, right_band_index + 1, outward, 0.0),
        offset_endpoint_boundary_point(
            &mouth.endpoint_profile,
            right_band_index,
            outward,
            outer_offset_m,
        ),
        offset_endpoint_boundary_point(
            &mouth.endpoint_profile,
            right_band_index + 1,
            outward,
            outer_offset_m,
        ),
    );
}

fn push_terminal_curb_end_bands(
    end_bands: &mut Vec<NodeInputTerminalEndBand>,
    mouth: &OrderedIncidentPieceMouth,
    outward: RoadVec2,
    center_source_band_index: usize,
    left_band_index: usize,
    right_band_index: usize,
    outer_offset_m: f64,
) {
    let curb_height_m = f64::from(
        mouth.endpoint_profile.boundary_points_world[left_band_index]
            .y
            .max(mouth.endpoint_profile.boundary_points_world[right_band_index + 1].y),
    );
    push_terminal_end_band(
        end_bands,
        center_source_band_index,
        RoadSurfaceBandKind::CurbOrShoulder,
        offset_endpoint_boundary_point_with_height(
            &mouth.endpoint_profile,
            left_band_index,
            outward,
            0.0,
            curb_height_m,
        ),
        offset_endpoint_boundary_point_with_height(
            &mouth.endpoint_profile,
            left_band_index + 1,
            outward,
            0.0,
            curb_height_m,
        ),
        offset_endpoint_boundary_point(
            &mouth.endpoint_profile,
            left_band_index,
            outward,
            outer_offset_m,
        ),
        offset_endpoint_boundary_point_with_height(
            &mouth.endpoint_profile,
            left_band_index + 1,
            outward,
            outer_offset_m,
            curb_height_m,
        ),
    );
    push_terminal_curb_center_end_band(
        end_bands,
        center_source_band_index,
        &mouth.endpoint_profile,
        outward,
        left_band_index + 1,
        right_band_index,
        outer_offset_m,
        curb_height_m,
    );
    push_terminal_end_band(
        end_bands,
        center_source_band_index,
        RoadSurfaceBandKind::CurbOrShoulder,
        offset_endpoint_boundary_point_with_height(
            &mouth.endpoint_profile,
            right_band_index,
            outward,
            0.0,
            curb_height_m,
        ),
        offset_endpoint_boundary_point_with_height(
            &mouth.endpoint_profile,
            right_band_index + 1,
            outward,
            0.0,
            curb_height_m,
        ),
        offset_endpoint_boundary_point_with_height(
            &mouth.endpoint_profile,
            right_band_index,
            outward,
            outer_offset_m,
            curb_height_m,
        ),
        offset_endpoint_boundary_point(
            &mouth.endpoint_profile,
            right_band_index + 1,
            outward,
            outer_offset_m,
        ),
    );
}

fn push_terminal_curb_center_end_band(
    end_bands: &mut Vec<NodeInputTerminalEndBand>,
    source_band_index: usize,
    profile: &IncidentMouthProfile,
    outward: RoadVec2,
    start_boundary_index: usize,
    end_boundary_index: usize,
    outer_offset_m: f64,
    curb_height_m: f64,
) {
    if start_boundary_index >= end_boundary_index
        || end_boundary_index >= profile.boundary_points_world.len()
    {
        return;
    }

    let inner_points = (start_boundary_index..=end_boundary_index)
        .map(|boundary_index| {
            offset_endpoint_boundary_point_with_height(
                profile,
                boundary_index,
                outward,
                0.0,
                curb_height_m,
            )
        })
        .collect::<Vec<_>>();
    let outer_points = (start_boundary_index..=end_boundary_index)
        .map(|boundary_index| {
            offset_endpoint_boundary_point_with_height(
                profile,
                boundary_index,
                outward,
                outer_offset_m,
                curb_height_m,
            )
        })
        .collect::<Vec<_>>();
    if inner_points.len() < 2 || outer_points.len() != inner_points.len() {
        return;
    }

    let inner_start_world = inner_points[0];
    let inner_end_world = *inner_points
        .last()
        .expect("curb terminal inner edge has at least two points");
    let outer_start_world = outer_points[0];
    let outer_end_world = *outer_points
        .last()
        .expect("curb terminal outer edge has at least two points");
    let mut contour_world = inner_points;
    contour_world.extend(outer_points.into_iter().rev());

    end_bands.push(NodeInputTerminalEndBand {
        source_band_index,
        band_kind: RoadSurfaceBandKind::CurbOrShoulder,
        boundary_mode: NodeInputTerminalEndBandBoundaryMode::TerminalMaterialBand,
        inner_start_world,
        inner_end_world,
        outer_start_world,
        outer_end_world,
        contour_world,
    });
}

fn push_terminal_center_end_band(
    end_bands: &mut Vec<NodeInputTerminalEndBand>,
    source_band_index: usize,
    band_kind: RoadSurfaceBandKind,
    profile: &IncidentMouthProfile,
    outward: RoadVec2,
    start_boundary_index: usize,
    end_boundary_index: usize,
    inner_offset_m: f64,
    outer_offset_m: f64,
) {
    push_terminal_center_end_band_with_heights(
        end_bands,
        source_band_index,
        band_kind,
        profile,
        outward,
        start_boundary_index,
        end_boundary_index,
        inner_offset_m,
        outer_offset_m,
        None,
        None,
    );
}

fn push_terminal_center_end_band_with_heights(
    end_bands: &mut Vec<NodeInputTerminalEndBand>,
    source_band_index: usize,
    band_kind: RoadSurfaceBandKind,
    profile: &IncidentMouthProfile,
    outward: RoadVec2,
    start_boundary_index: usize,
    end_boundary_index: usize,
    inner_offset_m: f64,
    outer_offset_m: f64,
    inner_heights_m: Option<(f64, f64)>,
    outer_heights_m: Option<(f64, f64)>,
) {
    if start_boundary_index >= end_boundary_index
        || end_boundary_index >= profile.boundary_points_world.len()
    {
        return;
    }

    let inner_points = offset_endpoint_boundary_points_with_linear_height(
        profile,
        outward,
        start_boundary_index,
        end_boundary_index,
        inner_offset_m,
        inner_heights_m,
    );
    let outer_points = offset_endpoint_boundary_points_with_linear_height(
        profile,
        outward,
        start_boundary_index,
        end_boundary_index,
        outer_offset_m,
        outer_heights_m,
    );
    if inner_points.len() < 2 || outer_points.len() != inner_points.len() {
        return;
    }

    let inner_start_world = inner_points[0];
    let inner_end_world = *inner_points
        .last()
        .expect("center terminal inner edge has at least two points");
    let outer_start_world = outer_points[0];
    let outer_end_world = *outer_points
        .last()
        .expect("center terminal outer edge has at least two points");
    let mut contour_world = inner_points;
    contour_world.extend(outer_points.into_iter().rev());

    end_bands.push(NodeInputTerminalEndBand {
        source_band_index,
        band_kind,
        boundary_mode: NodeInputTerminalEndBandBoundaryMode::TerminalMaterialBand,
        inner_start_world,
        inner_end_world,
        outer_start_world,
        outer_end_world,
        contour_world,
    });
}

fn offset_endpoint_boundary_points_with_linear_height(
    profile: &IncidentMouthProfile,
    outward: RoadVec2,
    start_boundary_index: usize,
    end_boundary_index: usize,
    offset_m: f64,
    endpoint_heights_m: Option<(f64, f64)>,
) -> Vec<RoadVec3> {
    let start_base = xz_from_road_vec3(godot_vec3_to_road(
        profile.boundary_points_world[start_boundary_index],
    ));
    let end_base = xz_from_road_vec3(godot_vec3_to_road(
        profile.boundary_points_world[end_boundary_index],
    ));
    let axis = end_base - start_base;
    let axis_len2 = axis.length_squared();
    let start_height_m = endpoint_heights_m
        .map(|(height_m, _)| height_m)
        .unwrap_or_else(|| f64::from(profile.boundary_points_world[start_boundary_index].y));
    let end_height_m = endpoint_heights_m
        .map(|(_, height_m)| height_m)
        .unwrap_or_else(|| f64::from(profile.boundary_points_world[end_boundary_index].y));

    (start_boundary_index..=end_boundary_index)
        .map(|boundary_index| {
            let point = profile.boundary_points_world[boundary_index];
            let base = godot_vec3_to_road(point);
            let base_xz = xz_from_road_vec3(base);
            let t = if axis_len2 > f64::EPSILON {
                ((base_xz - start_base).dot(axis) / axis_len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            RoadVec3::new(
                base.x + outward.x * offset_m,
                start_height_m + (end_height_m - start_height_m) * t,
                base.z + outward.y * offset_m,
            )
        })
        .collect()
}

fn push_terminal_end_band(
    end_bands: &mut Vec<NodeInputTerminalEndBand>,
    source_band_index: usize,
    band_kind: RoadSurfaceBandKind,
    inner_start_world: RoadVec3,
    inner_end_world: RoadVec3,
    outer_start_world: RoadVec3,
    outer_end_world: RoadVec3,
) {
    end_bands.push(NodeInputTerminalEndBand {
        source_band_index,
        band_kind,
        boundary_mode: NodeInputTerminalEndBandBoundaryMode::TerminalMaterialBand,
        inner_start_world,
        inner_end_world,
        outer_start_world,
        outer_end_world,
        contour_world: vec![
            inner_start_world,
            inner_end_world,
            outer_end_world,
            outer_start_world,
        ],
    });
}

fn offset_endpoint_boundary_point(
    profile: &IncidentMouthProfile,
    boundary_index: usize,
    outward: RoadVec2,
    offset_m: f64,
) -> RoadVec3 {
    let point = profile.boundary_points_world[boundary_index];
    RoadVec3::new(
        f64::from(point.x) + outward.x * offset_m,
        f64::from(point.y),
        f64::from(point.z) + outward.y * offset_m,
    )
}

fn offset_endpoint_boundary_point_with_height(
    profile: &IncidentMouthProfile,
    boundary_index: usize,
    outward: RoadVec2,
    offset_m: f64,
    height_m: f64,
) -> RoadVec3 {
    let point = profile.boundary_points_world[boundary_index];
    RoadVec3::new(
        f64::from(point.x) + outward.x * offset_m,
        height_m,
        f64::from(point.z) + outward.y * offset_m,
    )
}

fn band_width_m(band: &IncidentMouthBand) -> f64 {
    let dx = f64::from(band.end_point_world.x - band.start_point_world.x);
    let dz = f64::from(band.end_point_world.z - band.start_point_world.z);
    (dx * dx + dz * dz).sqrt()
}

fn boundary_rail_role(
    boundary_index: usize,
    bands: &[IncidentMouthBand],
) -> NodeInputBoundaryRailRole {
    match (
        boundary_index
            .checked_sub(1)
            .and_then(|index| bands.get(index)),
        bands.get(boundary_index),
    ) {
        (None, Some(right_band)) => NodeInputBoundaryRailRole::OuterFootprint {
            adjacent_kind: right_band.kind,
        },
        (Some(left_band), None) => NodeInputBoundaryRailRole::OuterFootprint {
            adjacent_kind: left_band.kind,
        },
        (Some(left_band), Some(right_band)) => NodeInputBoundaryRailRole::InteriorBandBoundary {
            left_kind: left_band.kind,
            right_kind: right_band.kind,
        },
        (None, None) => unreachable!("validated profile must have at least one band"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use godot::prelude::{Vector2, Vector3};

    fn test_band(kind: RoadSurfaceBandKind, start: Vector3, end: Vector3) -> IncidentMouthBand {
        IncidentMouthBand {
            kind,
            start_point_world: start,
            end_point_world: end,
        }
    }

    fn test_profile(x: f32, direction: Vector2) -> IncidentMouthProfile {
        let boundary_points_world = vec![
            Vector3::new(x, 4.0, -4.0),
            Vector3::new(x, 4.1, -2.0),
            Vector3::new(x, 4.2, 0.0),
            Vector3::new(x, 4.3, 2.0),
            Vector3::new(x, 4.4, 4.0),
        ];
        let bands = vec![
            test_band(
                RoadSurfaceBandKind::Sidewalk,
                boundary_points_world[0],
                boundary_points_world[1],
            ),
            test_band(
                RoadSurfaceBandKind::CurbOrShoulder,
                boundary_points_world[1],
                boundary_points_world[2],
            ),
            test_band(
                RoadSurfaceBandKind::Carriageway,
                boundary_points_world[2],
                boundary_points_world[3],
            ),
            test_band(
                RoadSurfaceBandKind::Sidewalk,
                boundary_points_world[3],
                boundary_points_world[4],
            ),
        ];
        IncidentMouthProfile {
            inward_direction_xz: direction,
            boundary_points_world,
            bands,
        }
    }

    fn test_mouth() -> OrderedIncidentPieceMouth {
        OrderedIncidentPieceMouth {
            profile: test_profile(10.0, Vector2::RIGHT),
            endpoint_profile: test_profile(0.0, Vector2::RIGHT),
            direction_angle_ccw: 0.0,
            direction_xz: Vector2::RIGHT,
            edge_idx: 7,
            side: IncidentEdgeSide::Start,
        }
    }

    fn two_carriageway_profile(x: f32, direction: Vector2) -> IncidentMouthProfile {
        let boundary_points_world = vec![
            Vector3::new(x, 4.12, -5.0),
            Vector3::new(x, 4.12, -3.65),
            Vector3::new(x, 4.12, -3.5),
            Vector3::new(x, 4.0, 0.0),
            Vector3::new(x, 4.0, 3.5),
            Vector3::new(x, 4.12, 3.65),
            Vector3::new(x, 4.12, 5.0),
        ];
        let bands = vec![
            test_band(
                RoadSurfaceBandKind::Sidewalk,
                boundary_points_world[0],
                boundary_points_world[1],
            ),
            test_band(
                RoadSurfaceBandKind::CurbOrShoulder,
                boundary_points_world[1],
                boundary_points_world[2],
            ),
            test_band(
                RoadSurfaceBandKind::Carriageway,
                boundary_points_world[2],
                boundary_points_world[3],
            ),
            test_band(
                RoadSurfaceBandKind::Carriageway,
                boundary_points_world[3],
                boundary_points_world[4],
            ),
            test_band(
                RoadSurfaceBandKind::CurbOrShoulder,
                boundary_points_world[4],
                boundary_points_world[5],
            ),
            test_band(
                RoadSurfaceBandKind::Sidewalk,
                boundary_points_world[5],
                boundary_points_world[6],
            ),
        ];
        IncidentMouthProfile {
            inward_direction_xz: direction,
            boundary_points_world,
            bands,
        }
    }

    fn two_carriageway_terminal_mouth() -> OrderedIncidentPieceMouth {
        OrderedIncidentPieceMouth {
            profile: two_carriageway_profile(10.0, Vector2::RIGHT),
            endpoint_profile: two_carriageway_profile(0.0, Vector2::RIGHT),
            direction_angle_ccw: 0.0,
            direction_xz: Vector2::RIGHT,
            edge_idx: 8,
            side: IncidentEdgeSide::Start,
        }
    }

    #[test]
    fn extracts_profile_rails_intervals_and_handoff() {
        let input = NodeArrangementInput::from_ordered_mouths(
            42,
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &[test_mouth()],
        )
        .expect("valid solved profiles should produce canonical input");

        assert_eq!(input.node_id, 42);
        assert_eq!(input.piece_kind, RoadSurfaceVisualNodePieceKind::JunctionN);
        assert_eq!(input.mouths.len(), 1);

        let mouth = &input.mouths[0];
        assert_eq!(mouth.order_index, 0);
        assert_eq!(mouth.edge_idx, 7);
        assert_eq!(mouth.side, IncidentEdgeSide::Start);
        assert_eq!(mouth.mouth_rails.len(), 4);
        assert_eq!(mouth.endpoint_rails.len(), 4);
        assert_eq!(mouth.boundary_rails.len(), 5);
        assert_eq!(mouth.band_intervals.len(), 4);
        assert!((mouth.conflict_handoff_distance_m - 10.0).abs() <= f64::EPSILON);
        assert_eq!(
            mouth.boundary_rails[0].role,
            NodeInputBoundaryRailRole::OuterFootprint {
                adjacent_kind: RoadSurfaceBandKind::Sidewalk
            }
        );
        assert_eq!(
            mouth.boundary_rails[2].role,
            NodeInputBoundaryRailRole::InteriorBandBoundary {
                left_kind: RoadSurfaceBandKind::CurbOrShoulder,
                right_kind: RoadSurfaceBandKind::Carriageway,
            }
        );
    }

    #[test]
    fn terminal_curb_end_bands_keep_cap_inner_boundary_raised() {
        let input = NodeArrangementInput::from_ordered_mouths(
            42,
            RoadSurfaceVisualNodePieceKind::Terminal,
            &[two_carriageway_terminal_mouth()],
        )
        .expect("valid two-carriageway terminal should produce canonical input");
        let mouth = &input.mouths[0];
        let center_boundary = mouth.boundary_rails[3].endpoint_world;
        let mut center_is_raised = false;

        for end_band in &mouth.terminal_end_bands {
            if end_band.band_kind != RoadSurfaceBandKind::CurbOrShoulder {
                continue;
            }
            center_is_raised |= end_band.contour_world.iter().any(|point| {
                (point.x - center_boundary.x).abs() <= 0.001
                    && (point.z - center_boundary.z).abs() <= 0.001
                    && point.y > center_boundary.y + 0.001
            });
        }
        assert!(
            center_is_raised,
            "terminal curb cap inner edge must stay raised over the carriageway center split"
        );
    }

    #[test]
    fn rejects_mismatched_profile_band_kinds() {
        let mut mouth = test_mouth();
        mouth.endpoint_profile.bands[1].kind = RoadSurfaceBandKind::Median;

        assert_eq!(
            NodeArrangementInput::from_ordered_mouths(
                42,
                RoadSurfaceVisualNodePieceKind::JunctionN,
                &[mouth],
            ),
            Err(NodeInputExtractionError::ProfileBandKindMismatch {
                edge_idx: 7,
                side: IncidentEdgeSide::Start,
                band_index: 1,
                mouth_kind: RoadSurfaceBandKind::CurbOrShoulder,
                endpoint_kind: RoadSurfaceBandKind::Median,
            })
        );
    }

    #[test]
    fn rejects_profile_boundary_count_mismatch() {
        let mut mouth = test_mouth();
        mouth.profile.boundary_points_world.pop();

        assert_eq!(
            NodeArrangementInput::from_ordered_mouths(
                42,
                RoadSurfaceVisualNodePieceKind::JunctionN,
                &[mouth],
            ),
            Err(NodeInputExtractionError::ProfileBoundaryCountMismatch {
                edge_idx: 7,
                side: IncidentEdgeSide::Start,
                profile_kind: NodeInputProfileKind::Mouth,
                expected: 5,
                actual: 4,
            })
        );
    }
}
