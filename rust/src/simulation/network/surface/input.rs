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
    pub(crate) uses_sampled_band_domain_paths: bool,
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
    pub(crate) path_world: Vec<RoadVec3>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeInputBandInterval {
    pub(crate) band_index: usize,
    pub(crate) band_kind: RoadSurfaceBandKind,
    pub(crate) mouth_start_world: RoadVec3,
    pub(crate) mouth_end_world: RoadVec3,
    pub(crate) endpoint_start_world: RoadVec3,
    pub(crate) endpoint_end_world: RoadVec3,
    pub(crate) start_path_world: Vec<RoadVec3>,
    pub(crate) end_path_world: Vec<RoadVec3>,
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
    TerminalMaterialBand,
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
            uses_sampled_band_domain_paths: mouth.uses_sampled_band_domain_paths,
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
        .map(|(boundary_index, (mouth_point, endpoint_point))| {
            let mouth_world = godot_vec3_to_road(*mouth_point);
            let endpoint_world = godot_vec3_to_road(*endpoint_point);
            NodeInputBoundaryRail {
                boundary_index,
                role: boundary_rail_role(boundary_index, &mouth.profile.bands),
                mouth_world,
                endpoint_world,
                path_world: input_path_or_endpoints(
                    mouth
                        .boundary_paths_world
                        .get(boundary_index)
                        .map(Vec::as_slice),
                    mouth_world,
                    endpoint_world,
                ),
            }
        })
        .collect()
}

fn band_intervals(mouth: &OrderedIncidentPieceMouth) -> Vec<NodeInputBandInterval> {
    mouth
        .profile
        .bands
        .iter()
        .zip(&mouth.endpoint_profile.bands)
        .enumerate()
        .map(|(band_index, (mouth_band, endpoint_band))| {
            let mouth_start_world = band_endpoint_with_boundary_xz(
                mouth_band.start_point_world,
                mouth.profile.boundary_points_world[band_index],
            );
            let mouth_end_world = band_endpoint_with_boundary_xz(
                mouth_band.end_point_world,
                mouth.profile.boundary_points_world[band_index + 1],
            );
            let endpoint_start_world = band_endpoint_with_boundary_xz(
                endpoint_band.start_point_world,
                mouth.endpoint_profile.boundary_points_world[band_index],
            );
            let endpoint_end_world = band_endpoint_with_boundary_xz(
                endpoint_band.end_point_world,
                mouth.endpoint_profile.boundary_points_world[band_index + 1],
            );
            NodeInputBandInterval {
                band_index,
                band_kind: mouth_band.kind,
                mouth_start_world,
                mouth_end_world,
                endpoint_start_world,
                endpoint_end_world,
                start_path_world: input_path_or_endpoints(
                    mouth
                        .band_start_paths_world
                        .get(band_index)
                        .map(Vec::as_slice),
                    mouth_start_world,
                    endpoint_start_world,
                ),
                end_path_world: input_path_or_endpoints(
                    mouth
                        .band_end_paths_world
                        .get(band_index)
                        .map(Vec::as_slice),
                    mouth_end_world,
                    endpoint_end_world,
                ),
            }
        })
        .collect()
}

fn input_path_or_endpoints(
    path_world: Option<&[Vector3]>,
    mouth_world: RoadVec3,
    endpoint_world: RoadVec3,
) -> Vec<RoadVec3> {
    if let Some(path_world) = path_world.filter(|path| path.len() >= 2) {
        let mut points = path_world
            .iter()
            .map(|point| godot_vec3_to_road(*point))
            .collect::<Vec<_>>();
        if let Some(first) = points.first_mut() {
            *first = mouth_world;
        }
        if let Some(last) = points.last_mut() {
            *last = endpoint_world;
        }
        points
    } else {
        vec![mouth_world, endpoint_world]
    }
}

fn band_endpoint_with_boundary_xz(
    band_point_world: Vector3,
    boundary_point_world: Vector3,
) -> RoadVec3 {
    let boundary = godot_vec3_to_road(boundary_point_world);
    RoadVec3::new(boundary.x, f64::from(band_point_world.y), boundary.z)
}

fn replace_profile_paths_with_chords(
    boundary_rails: &mut [NodeInputBoundaryRail],
    band_intervals: &mut [NodeInputBandInterval],
) {
    for rail in boundary_rails {
        rail.path_world = vec![rail.mouth_world, rail.endpoint_world];
    }
    for interval in band_intervals {
        interval.start_path_world = vec![interval.mouth_start_world, interval.endpoint_start_world];
        interval.end_path_world = vec![interval.mouth_end_world, interval.endpoint_end_world];
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
        for point in &mut rail.path_world {
            *point = quantize_road_vec3_xz(*point);
        }
    }
}

fn quantize_band_intervals_xz(intervals: &mut [NodeInputBandInterval]) {
    for interval in intervals {
        interval.mouth_start_world = quantize_road_vec3_xz(interval.mouth_start_world);
        interval.mouth_end_world = quantize_road_vec3_xz(interval.mouth_end_world);
        interval.endpoint_start_world = quantize_road_vec3_xz(interval.endpoint_start_world);
        interval.endpoint_end_world = quantize_road_vec3_xz(interval.endpoint_end_world);
        for point in &mut interval.start_path_world {
            *point = quantize_road_vec3_xz(*point);
        }
        for point in &mut interval.end_path_world {
            *point = quantize_road_vec3_xz(*point);
        }
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
    let closure_heights_m = terminal_paired_closure_height_anchors(
        &mouth.endpoint_profile,
        left_band_index,
        right_band_index,
    );
    push_terminal_center_end_band_with_heights(
        end_bands,
        center_source_band_index,
        band_kind,
        &mouth.endpoint_profile,
        outward,
        left_band_index + 1,
        right_band_index,
        inner_offset_m,
        outer_offset_m,
        closure_heights_m,
        closure_heights_m,
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

fn terminal_paired_closure_height_anchors(
    profile: &IncidentMouthProfile,
    left_band_index: usize,
    right_band_index: usize,
) -> Option<(f64, f64)> {
    let left_height_m = profile
        .boundary_points_world
        .get(left_band_index)
        .map(|point| f64::from(point.y))?;
    let right_height_m = profile
        .boundary_points_world
        .get(right_band_index + 1)
        .map(|point| f64::from(point.y))?;
    Some((left_height_m, right_height_m))
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
    let override_heights_m = endpoint_heights_m.map(|(start_height_m, end_height_m)| {
        let start_height_m = start_height_m;
        let end_height_m = end_height_m;
        (start_height_m, end_height_m)
    });

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
            let height_m = override_heights_m.map_or(base.y, |(start_height_m, end_height_m)| {
                start_height_m + (end_height_m - start_height_m) * t
            });
            RoadVec3::new(
                base.x + outward.x * offset_m,
                height_m,
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
            boundary_paths_world: Vec::new(),
            band_start_paths_world: Vec::new(),
            band_end_paths_world: Vec::new(),
            uses_sampled_band_domain_paths: false,
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
            boundary_paths_world: Vec::new(),
            band_start_paths_world: Vec::new(),
            band_end_paths_world: Vec::new(),
            uses_sampled_band_domain_paths: false,
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
    fn terminal_center_caps_use_paired_outer_source_heights() {
        let input = NodeArrangementInput::from_ordered_mouths(
            42,
            RoadSurfaceVisualNodePieceKind::Terminal,
            &[two_carriageway_terminal_mouth()],
        )
        .expect("valid two-carriageway terminal should produce canonical input");
        let mouth = &input.mouths[0];
        let center_boundary = mouth.boundary_rails[3].endpoint_world;
        let expected_height_m = mouth.boundary_rails[1]
            .endpoint_world
            .y
            .max(mouth.boundary_rails[5].endpoint_world.y);
        let mut center_uses_paired_source_height = false;

        for end_band in &mouth.terminal_end_bands {
            if end_band.band_kind != RoadSurfaceBandKind::CurbOrShoulder {
                continue;
            }
            center_uses_paired_source_height |= end_band.contour_world.iter().any(|point| {
                (point.x - center_boundary.x).abs() <= 0.001
                    && (point.z - center_boundary.z).abs() <= 0.001
                    && (point.y - expected_height_m).abs() <= 0.001
            });
        }
        assert!(
            center_uses_paired_source_height,
            "terminal center caps must take height from paired outer source rails instead of the carriageway-center boundary"
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
