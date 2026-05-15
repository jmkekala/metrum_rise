//! Canonical terminal-cap adapter for one-mouth visual node ownership.

use super::backend::{
    ROAD_OVERLAY_COORDINATE_SCALE, RoadPolyline, RoadVec2, RoadVec3, polyline_to_road_points,
    quantize_road_vec2_to_overlay_grid, road_points_to_polyline,
};
use super::input::{NodeArrangementInput, NodeInputMouth};
use super::{NODE_OVERLAY_MIN_AREA_M2, RoadSurfaceBandKind, RoadSurfaceVisualNodePieceKind};
use cavalier_contours::polyline::{PlineCreation, PlineSource};

const TERMINAL_CAP_HEIGHT_EDGE_EPS_M: f64 = 0.001;
const TERMINAL_CAP_POLYLINE_POINT_EQUAL_EPS_M: f64 = 1.0e-6;
const TERMINAL_CAP_WIDTH_EPS_M: f64 = 0.001;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalCapBandRole {
    LeftSide,
    LeftCorner,
    EndBand,
    RightCorner,
    RightSide,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerminalCapBandProvenance {
    pub(crate) layer_index: usize,
    pub(crate) role: TerminalCapBandRole,
    pub(crate) left_source_band_index: usize,
    pub(crate) right_source_band_index: usize,
    pub(crate) source_boundary_start_index: usize,
    pub(crate) source_boundary_end_index: usize,
    pub(crate) inner_offset_m: f64,
    pub(crate) outer_offset_m: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalCapFailureReason {
    MissingBoundaryRails,
    MissingOutwardDirection,
    MismatchedPairedBandKind,
    MismatchedPairedBandWidth,
    DegenerateBandWidth,
    DegeneratePath,
    DegenerateContour,
    InvalidCapArea,
}

impl TerminalCapFailureReason {
    pub(crate) fn diagnostic_reason(self) -> &'static str {
        match self {
            Self::MissingBoundaryRails => "terminal_cap_missing_boundary_rails",
            Self::MissingOutwardDirection => "terminal_cap_missing_outward_direction",
            Self::MismatchedPairedBandKind => "terminal_cap_mismatched_paired_band_kind",
            Self::MismatchedPairedBandWidth => "terminal_cap_mismatched_paired_band_width",
            Self::DegenerateBandWidth => "terminal_cap_degenerate_band_width",
            Self::DegeneratePath => "terminal_cap_degenerate_path",
            Self::DegenerateContour => "terminal_cap_degenerate_contour",
            Self::InvalidCapArea => "terminal_cap_invalid_area",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TerminalCapGenerationError {
    pub(crate) mouth_order_index: usize,
    pub(crate) layer_index: Option<usize>,
    pub(crate) source_band_index: Option<usize>,
    pub(crate) left_source_band_index: Option<usize>,
    pub(crate) right_source_band_index: Option<usize>,
    pub(crate) band_kind: Option<RoadSurfaceBandKind>,
    pub(crate) reason: TerminalCapFailureReason,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeTerminalCapBand {
    pub(crate) source_band_index: usize,
    pub(crate) band_kind: RoadSurfaceBandKind,
    pub(crate) provenance: TerminalCapBandProvenance,
    pub(crate) inner_path_world: Vec<RoadVec3>,
    pub(crate) outer_path_world: Vec<RoadVec3>,
    pub(crate) contour_world: Vec<RoadVec3>,
}

impl TerminalCapGenerationError {
    fn for_mouth(mouth: &NodeInputMouth, reason: TerminalCapFailureReason) -> Self {
        Self {
            mouth_order_index: mouth.order_index,
            layer_index: None,
            source_band_index: None,
            left_source_band_index: None,
            right_source_band_index: None,
            band_kind: None,
            reason,
        }
    }

    fn for_layer(
        mouth: &NodeInputMouth,
        layer_index: usize,
        source_band_index: usize,
        left_source_band_index: usize,
        right_source_band_index: usize,
        band_kind: RoadSurfaceBandKind,
        reason: TerminalCapFailureReason,
    ) -> Self {
        Self {
            mouth_order_index: mouth.order_index,
            layer_index: Some(layer_index),
            source_band_index: Some(source_band_index),
            left_source_band_index: Some(left_source_band_index),
            right_source_band_index: Some(right_source_band_index),
            band_kind: Some(band_kind),
            reason,
        }
    }

    fn for_cap(
        mouth: &NodeInputMouth,
        source_band_index: usize,
        band_kind: RoadSurfaceBandKind,
        provenance: TerminalCapBandProvenance,
        reason: TerminalCapFailureReason,
    ) -> Self {
        Self::for_layer(
            mouth,
            provenance.layer_index,
            source_band_index,
            provenance.left_source_band_index,
            provenance.right_source_band_index,
            band_kind,
            reason,
        )
    }
}

pub(crate) fn terminal_cap_bands_by_mouth(
    input: &NodeArrangementInput,
) -> Result<Vec<Vec<NodeTerminalCapBand>>, TerminalCapGenerationError> {
    let mut bands_by_mouth = vec![Vec::new(); input.mouths.len()];
    if input.piece_kind != RoadSurfaceVisualNodePieceKind::Terminal {
        return Ok(bands_by_mouth);
    }

    for (mouth_index, mouth) in input.mouths.iter().enumerate() {
        let mut bands = terminal_cap_bands(mouth)?;
        canonicalize_terminal_cap_bands(mouth, &mut bands)?;
        bands_by_mouth[mouth_index] = bands;
    }

    Ok(bands_by_mouth)
}

fn terminal_cap_bands(
    mouth: &NodeInputMouth,
) -> Result<Vec<NodeTerminalCapBand>, TerminalCapGenerationError> {
    let Some(first_carriageway) = mouth
        .band_intervals
        .iter()
        .position(|band| band.band_kind == RoadSurfaceBandKind::Carriageway)
    else {
        return Ok(Vec::new());
    };
    let Some(last_carriageway) = mouth
        .band_intervals
        .iter()
        .rposition(|band| band.band_kind == RoadSurfaceBandKind::Carriageway)
    else {
        return Ok(Vec::new());
    };
    if first_carriageway == 0 || last_carriageway + 1 >= mouth.band_intervals.len() {
        return Ok(Vec::new());
    }
    if mouth.boundary_rails.len() != mouth.band_intervals.len() + 1 {
        return Err(TerminalCapGenerationError::for_mouth(
            mouth,
            TerminalCapFailureReason::MissingBoundaryRails,
        ));
    }

    let Some(outward) = normalized_terminal_cap_direction(-mouth.direction_xz) else {
        return Err(TerminalCapGenerationError::for_mouth(
            mouth,
            TerminalCapFailureReason::MissingOutwardDirection,
        ));
    };
    let paired_layers = first_carriageway.min(mouth.band_intervals.len() - last_carriageway - 1);
    let mut cap_bands = Vec::new();
    let mut inner_offset_m = 0.0;
    let mut next_terminal_source_band_index = mouth.band_intervals.len();

    for layer_index in 0..paired_layers {
        let left_band_index = first_carriageway - 1 - layer_index;
        let right_band_index = last_carriageway + 1 + layer_index;
        let left_band = &mouth.band_intervals[left_band_index];
        let right_band = &mouth.band_intervals[right_band_index];
        if left_band.band_kind != right_band.band_kind {
            return Err(TerminalCapGenerationError::for_layer(
                mouth,
                layer_index,
                next_terminal_source_band_index,
                left_band_index,
                right_band_index,
                left_band.band_kind,
                TerminalCapFailureReason::MismatchedPairedBandKind,
            ));
        }
        if left_band.band_kind == RoadSurfaceBandKind::Carriageway {
            return Err(TerminalCapGenerationError::for_layer(
                mouth,
                layer_index,
                next_terminal_source_band_index,
                left_band_index,
                right_band_index,
                left_band.band_kind,
                TerminalCapFailureReason::MismatchedPairedBandKind,
            ));
        }

        let left_depth_m = band_width_m(left_band);
        let right_depth_m = band_width_m(right_band);
        if left_depth_m <= TERMINAL_CAP_WIDTH_EPS_M || right_depth_m <= TERMINAL_CAP_WIDTH_EPS_M {
            return Err(TerminalCapGenerationError::for_layer(
                mouth,
                layer_index,
                next_terminal_source_band_index,
                left_band_index,
                right_band_index,
                left_band.band_kind,
                TerminalCapFailureReason::DegenerateBandWidth,
            ));
        }
        if (left_depth_m - right_depth_m).abs() > TERMINAL_CAP_WIDTH_EPS_M {
            return Err(TerminalCapGenerationError::for_layer(
                mouth,
                layer_index,
                next_terminal_source_band_index,
                left_band_index,
                right_band_index,
                left_band.band_kind,
                TerminalCapFailureReason::MismatchedPairedBandWidth,
            ));
        }
        let depth_m = left_depth_m;
        let outer_offset_m = inner_offset_m + depth_m;
        push_terminal_paired_cap_bands(
            &mut cap_bands,
            mouth,
            outward,
            next_terminal_source_band_index,
            layer_index,
            left_band_index,
            right_band_index,
            inner_offset_m,
            outer_offset_m,
        )?;
        next_terminal_source_band_index += 1;
        inner_offset_m = outer_offset_m;
    }

    Ok(cap_bands)
}

fn push_terminal_paired_cap_bands(
    cap_bands: &mut Vec<NodeTerminalCapBand>,
    mouth: &NodeInputMouth,
    outward: RoadVec2,
    source_band_index: usize,
    layer_index: usize,
    left_band_index: usize,
    right_band_index: usize,
    inner_offset_m: f64,
    outer_offset_m: f64,
) -> Result<(), TerminalCapGenerationError> {
    let band_kind = mouth.band_intervals[left_band_index].band_kind;
    push_terminal_side_corner_cap_band(
        cap_bands,
        mouth,
        outward,
        source_band_index,
        band_kind,
        layer_index,
        TerminalCapBandRole::LeftCorner,
        left_band_index,
        right_band_index,
        left_band_index,
        left_band_index + 1,
        inner_offset_m,
    )?;
    push_terminal_cap_band(
        cap_bands,
        mouth,
        source_band_index,
        band_kind,
        TerminalCapBandProvenance {
            layer_index,
            role: TerminalCapBandRole::LeftSide,
            left_source_band_index: left_band_index,
            right_source_band_index: right_band_index,
            source_boundary_start_index: left_band_index,
            source_boundary_end_index: left_band_index + 1,
            inner_offset_m,
            outer_offset_m,
        },
        terminal_offset_boundary_path(
            mouth,
            left_band_index,
            left_band_index + 1,
            outward,
            inner_offset_m,
            terminal_side_band_height_anchors(mouth, left_band_index),
        ),
        terminal_offset_boundary_path(
            mouth,
            left_band_index,
            left_band_index + 1,
            outward,
            outer_offset_m,
            terminal_side_band_height_anchors(mouth, left_band_index),
        ),
    )?;
    push_terminal_cap_band(
        cap_bands,
        mouth,
        source_band_index,
        band_kind,
        TerminalCapBandProvenance {
            layer_index,
            role: TerminalCapBandRole::EndBand,
            left_source_band_index: left_band_index,
            right_source_band_index: right_band_index,
            source_boundary_start_index: left_band_index + 1,
            source_boundary_end_index: right_band_index,
            inner_offset_m,
            outer_offset_m,
        },
        terminal_offset_boundary_path_with_linear_height(
            mouth,
            left_band_index + 1,
            right_band_index,
            outward,
            inner_offset_m,
            terminal_end_band_inner_height_anchors(mouth, left_band_index, right_band_index),
        ),
        terminal_offset_boundary_path_with_linear_height(
            mouth,
            left_band_index + 1,
            right_band_index,
            outward,
            outer_offset_m,
            terminal_end_band_outer_height_anchors(mouth, left_band_index, right_band_index),
        ),
    )?;
    push_terminal_side_corner_cap_band(
        cap_bands,
        mouth,
        outward,
        source_band_index,
        band_kind,
        layer_index,
        TerminalCapBandRole::RightCorner,
        left_band_index,
        right_band_index,
        right_band_index,
        right_band_index + 1,
        inner_offset_m,
    )?;
    push_terminal_cap_band(
        cap_bands,
        mouth,
        source_band_index,
        band_kind,
        TerminalCapBandProvenance {
            layer_index,
            role: TerminalCapBandRole::RightSide,
            left_source_band_index: left_band_index,
            right_source_band_index: right_band_index,
            source_boundary_start_index: right_band_index,
            source_boundary_end_index: right_band_index + 1,
            inner_offset_m,
            outer_offset_m,
        },
        terminal_offset_boundary_path(
            mouth,
            right_band_index,
            right_band_index + 1,
            outward,
            inner_offset_m,
            terminal_side_band_height_anchors(mouth, right_band_index),
        ),
        terminal_offset_boundary_path(
            mouth,
            right_band_index,
            right_band_index + 1,
            outward,
            outer_offset_m,
            terminal_side_band_height_anchors(mouth, right_band_index),
        ),
    )?;
    Ok(())
}

fn push_terminal_side_corner_cap_band(
    cap_bands: &mut Vec<NodeTerminalCapBand>,
    mouth: &NodeInputMouth,
    outward: RoadVec2,
    source_band_index: usize,
    band_kind: RoadSurfaceBandKind,
    layer_index: usize,
    role: TerminalCapBandRole,
    left_band_index: usize,
    right_band_index: usize,
    start_boundary_index: usize,
    end_boundary_index: usize,
    corner_depth_m: f64,
) -> Result<(), TerminalCapGenerationError> {
    if corner_depth_m <= TERMINAL_CAP_WIDTH_EPS_M {
        return Ok(());
    }

    push_terminal_cap_band(
        cap_bands,
        mouth,
        source_band_index,
        band_kind,
        TerminalCapBandProvenance {
            layer_index,
            role,
            left_source_band_index: left_band_index,
            right_source_band_index: right_band_index,
            source_boundary_start_index: start_boundary_index,
            source_boundary_end_index: end_boundary_index,
            inner_offset_m: 0.0,
            outer_offset_m: corner_depth_m,
        },
        terminal_offset_boundary_path(
            mouth,
            start_boundary_index,
            end_boundary_index,
            outward,
            0.0,
            terminal_side_band_height_anchors(mouth, start_boundary_index),
        ),
        terminal_offset_boundary_path(
            mouth,
            start_boundary_index,
            end_boundary_index,
            outward,
            corner_depth_m,
            terminal_side_band_height_anchors(mouth, start_boundary_index),
        ),
    )
}

fn push_terminal_cap_band(
    cap_bands: &mut Vec<NodeTerminalCapBand>,
    mouth: &NodeInputMouth,
    source_band_index: usize,
    band_kind: RoadSurfaceBandKind,
    provenance: TerminalCapBandProvenance,
    inner_path_world: Option<Vec<RoadVec3>>,
    outer_path_world: Option<Vec<RoadVec3>>,
) -> Result<(), TerminalCapGenerationError> {
    let inner_path_world = inner_path_world
        .and_then(clean_terminal_cap_path_world)
        .ok_or_else(|| {
            TerminalCapGenerationError::for_cap(
                mouth,
                source_band_index,
                band_kind,
                provenance,
                TerminalCapFailureReason::DegeneratePath,
            )
        })?;
    let outer_path_world = outer_path_world
        .and_then(clean_terminal_cap_path_world)
        .ok_or_else(|| {
            TerminalCapGenerationError::for_cap(
                mouth,
                source_band_index,
                band_kind,
                provenance,
                TerminalCapFailureReason::DegeneratePath,
            )
        })?;
    let contour_world = terminal_cap_contour_world(&inner_path_world, &outer_path_world)
        .ok_or_else(|| {
            TerminalCapGenerationError::for_cap(
                mouth,
                source_band_index,
                band_kind,
                provenance,
                TerminalCapFailureReason::DegenerateContour,
            )
        })?;

    cap_bands.push(NodeTerminalCapBand {
        source_band_index,
        band_kind,
        provenance,
        inner_path_world,
        outer_path_world,
        contour_world,
    });
    Ok(())
}

fn terminal_offset_boundary_path(
    mouth: &NodeInputMouth,
    start_boundary_index: usize,
    end_boundary_index: usize,
    outward: RoadVec2,
    offset_m: f64,
    endpoint_heights_m: Option<(f64, f64)>,
) -> Option<Vec<RoadVec3>> {
    terminal_offset_boundary_path_with_linear_height(
        mouth,
        start_boundary_index,
        end_boundary_index,
        outward,
        offset_m,
        endpoint_heights_m,
    )
}

fn terminal_offset_boundary_path_with_linear_height(
    mouth: &NodeInputMouth,
    start_boundary_index: usize,
    end_boundary_index: usize,
    outward: RoadVec2,
    offset_m: f64,
    endpoint_heights_m: Option<(f64, f64)>,
) -> Option<Vec<RoadVec3>> {
    if start_boundary_index >= end_boundary_index
        || end_boundary_index >= mouth.boundary_rails.len()
    {
        return None;
    }

    let start_base = xz(endpoint_boundary_world(mouth, start_boundary_index)?);
    let end_base = xz(endpoint_boundary_world(mouth, end_boundary_index)?);
    let axis = end_base - start_base;
    let axis_len2 = axis.length_squared();
    let mut points = Vec::with_capacity(end_boundary_index - start_boundary_index + 1);

    for boundary_index in start_boundary_index..=end_boundary_index {
        let base = endpoint_boundary_world(mouth, boundary_index)?;
        let base_xz = xz(base);
        let t = if axis_len2 > f64::EPSILON {
            ((base_xz - start_base).dot(axis) / axis_len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let height_m = endpoint_heights_m.map_or(base.y, |(start_height_m, end_height_m)| {
            start_height_m + (end_height_m - start_height_m) * t
        });
        points.push(RoadVec3::new(
            base.x + outward.x * offset_m,
            height_m,
            base.z + outward.y * offset_m,
        ));
    }

    Some(points)
}

fn terminal_side_band_height_anchors(
    mouth: &NodeInputMouth,
    band_index: usize,
) -> Option<(f64, f64)> {
    let band = mouth.band_intervals.get(band_index)?;
    Some((band.endpoint_start_world.y, band.endpoint_end_world.y))
}

fn terminal_end_band_inner_height_anchors(
    mouth: &NodeInputMouth,
    left_band_index: usize,
    right_band_index: usize,
) -> Option<(f64, f64)> {
    let left_band = mouth.band_intervals.get(left_band_index)?;
    let right_band = mouth.band_intervals.get(right_band_index)?;
    Some((
        left_band.endpoint_end_world.y,
        right_band.endpoint_start_world.y,
    ))
}

fn terminal_end_band_outer_height_anchors(
    mouth: &NodeInputMouth,
    left_band_index: usize,
    right_band_index: usize,
) -> Option<(f64, f64)> {
    let left_band = mouth.band_intervals.get(left_band_index)?;
    let right_band = mouth.band_intervals.get(right_band_index)?;
    let left_height_m = left_band.endpoint_start_world.y;
    let right_height_m = right_band.endpoint_end_world.y;
    Some((left_height_m, right_height_m))
}

fn endpoint_boundary_world(mouth: &NodeInputMouth, boundary_index: usize) -> Option<RoadVec3> {
    mouth
        .boundary_rails
        .get(boundary_index)
        .map(|rail| rail.endpoint_world)
}

fn band_width_m(band: &super::input::NodeInputBandInterval) -> f64 {
    xz(band.endpoint_start_world).distance(xz(band.endpoint_end_world))
}

fn canonicalize_terminal_cap_bands(
    mouth: &NodeInputMouth,
    cap_bands: &mut [NodeTerminalCapBand],
) -> Result<(), TerminalCapGenerationError> {
    for cap_band in cap_bands.iter_mut() {
        quantize_terminal_cap_band_xz(cap_band);
        let Some(inner_path_world) =
            clean_terminal_cap_path_world(cap_band.inner_path_world.clone())
        else {
            return Err(TerminalCapGenerationError::for_cap(
                mouth,
                cap_band.source_band_index,
                cap_band.band_kind,
                cap_band.provenance,
                TerminalCapFailureReason::DegeneratePath,
            ));
        };
        let Some(outer_path_world) =
            clean_terminal_cap_path_world(cap_band.outer_path_world.clone())
        else {
            return Err(TerminalCapGenerationError::for_cap(
                mouth,
                cap_band.source_band_index,
                cap_band.band_kind,
                cap_band.provenance,
                TerminalCapFailureReason::DegeneratePath,
            ));
        };
        let Some(contour_world) = terminal_cap_contour_world(&inner_path_world, &outer_path_world)
        else {
            return Err(TerminalCapGenerationError::for_cap(
                mouth,
                cap_band.source_band_index,
                cap_band.band_kind,
                cap_band.provenance,
                TerminalCapFailureReason::DegenerateContour,
            ));
        };
        cap_band.inner_path_world = inner_path_world;
        cap_band.outer_path_world = outer_path_world;
        cap_band.contour_world = contour_world;
        if !terminal_cap_band_has_quantized_area(cap_band) {
            return Err(TerminalCapGenerationError::for_cap(
                mouth,
                cap_band.source_band_index,
                cap_band.band_kind,
                cap_band.provenance,
                TerminalCapFailureReason::InvalidCapArea,
            ));
        }
    }
    Ok(())
}

fn quantize_terminal_cap_band_xz(cap_band: &mut NodeTerminalCapBand) {
    for point in &mut cap_band.inner_path_world {
        *point = quantize_road_vec3_xz(*point);
    }
    for point in &mut cap_band.outer_path_world {
        *point = quantize_road_vec3_xz(*point);
    }
    for point in &mut cap_band.contour_world {
        *point = quantize_road_vec3_xz(*point);
    }
}

fn quantize_road_vec3_xz(point: RoadVec3) -> RoadVec3 {
    let point_xz = quantize_road_vec2_to_overlay_grid(xz(point));
    RoadVec3::new(point_xz.x, point.y, point_xz.y)
}

fn terminal_cap_contour_world(
    inner_path_world: &[RoadVec3],
    outer_path_world: &[RoadVec3],
) -> Option<Vec<RoadVec3>> {
    if inner_path_world.len() < 2 || outer_path_world.len() < 2 {
        return None;
    }
    let mut contour_world = inner_path_world.to_vec();
    contour_world.extend(outer_path_world.iter().rev().copied());
    clean_terminal_cap_contour_world(contour_world)
}

fn clean_terminal_cap_path_world(path_world: Vec<RoadVec3>) -> Option<Vec<RoadVec3>> {
    if path_world.len() < 2 {
        return None;
    }
    let polyline = cleaned_open_world_path_polyline(&path_world)?;
    if polyline.vertex_count() < 2 {
        return None;
    }
    let points_xz = polyline_to_road_points(&polyline);
    let mut cleaned_world = points_xz
        .into_iter()
        .map(|point_xz| {
            let height_m = height_on_world_path(point_xz, &path_world)?;
            Some(RoadVec3::new(point_xz.x, height_m, point_xz.y))
        })
        .collect::<Option<Vec<_>>>()?;
    remove_repeated_road_vec3_points(&mut cleaned_world);
    (cleaned_world.len() >= 2).then_some(cleaned_world)
}

fn clean_terminal_cap_contour_world(contour_world: Vec<RoadVec3>) -> Option<Vec<RoadVec3>> {
    let raw = road_points_to_polyline(contour_world.iter().copied().map(xz), true);
    let mut cleaned =
        RoadPolyline::create_from_remove_repeat(&raw, TERMINAL_CAP_POLYLINE_POINT_EQUAL_EPS_M);
    if let Some(reduced) = cleaned.remove_redundant(TERMINAL_CAP_POLYLINE_POINT_EQUAL_EPS_M) {
        cleaned = reduced;
    }
    if cleaned.vertex_count() < 3
        || cleaned.area().abs() <= f64::from(NODE_OVERLAY_MIN_AREA_M2)
        || cleaned.scan_for_self_intersect()
    {
        return None;
    }
    let mut cleaned_world = polyline_to_road_points(&cleaned)
        .into_iter()
        .map(|point_xz| {
            let height_m = height_on_world_path(point_xz, &contour_world)?;
            Some(RoadVec3::new(point_xz.x, height_m, point_xz.y))
        })
        .collect::<Option<Vec<_>>>()?;
    remove_repeated_road_vec3_points(&mut cleaned_world);
    (cleaned_world.len() >= 3).then_some(cleaned_world)
}

fn cleaned_open_world_path_polyline(path_world: &[RoadVec3]) -> Option<RoadPolyline> {
    let raw = road_points_to_polyline(path_world.iter().copied().map(xz), false);
    let cleaned =
        RoadPolyline::create_from_remove_repeat(&raw, TERMINAL_CAP_POLYLINE_POINT_EQUAL_EPS_M);
    (cleaned.vertex_count() >= 2).then_some(cleaned)
}

fn height_on_world_path(point_xz: RoadVec2, path_world: &[RoadVec3]) -> Option<f64> {
    let key = quantized_xz_key(point_xz);
    for point_world in path_world {
        if quantized_xz_key(xz(*point_world)) == key {
            return Some(point_world.y);
        }
    }
    for segment in path_world.windows(2) {
        if let Some(height_m) = height_on_world_segment(point_xz, segment[0], segment[1]) {
            return Some(height_m);
        }
    }
    None
}

fn height_on_world_segment(
    point_xz: RoadVec2,
    start_world: RoadVec3,
    end_world: RoadVec3,
) -> Option<f64> {
    let start_xz = xz(start_world);
    let end_xz = xz(end_world);
    let axis = end_xz - start_xz;
    let axis_len2 = axis.length_squared();
    if axis_len2 <= f64::EPSILON {
        return None;
    }
    let t = ((point_xz - start_xz).dot(axis) / axis_len2).clamp(0.0, 1.0);
    let closest = start_xz + axis * t;
    if closest.distance_squared(point_xz)
        > TERMINAL_CAP_HEIGHT_EDGE_EPS_M * TERMINAL_CAP_HEIGHT_EDGE_EPS_M
    {
        return None;
    }
    Some(start_world.y + (end_world.y - start_world.y) * t)
}

fn terminal_cap_band_has_quantized_area(cap_band: &NodeTerminalCapBand) -> bool {
    let raw = road_points_to_polyline(cap_band.contour_world.iter().copied().map(xz), true);
    let contour =
        RoadPolyline::create_from_remove_repeat(&raw, TERMINAL_CAP_POLYLINE_POINT_EQUAL_EPS_M);
    contour.vertex_count() >= 3
        && contour.area().abs() > f64::from(NODE_OVERLAY_MIN_AREA_M2)
        && !contour.scan_for_self_intersect()
}

fn remove_repeated_road_vec3_points(points: &mut Vec<RoadVec3>) {
    points.dedup_by(|a, b| quantized_xz_key(xz(*a)) == quantized_xz_key(xz(*b)));
    if points.len() > 1
        && quantized_xz_key(xz(points[0]))
            == quantized_xz_key(xz(*points.last().expect("points are non-empty")))
    {
        points.pop();
    }
}

fn normalized_terminal_cap_direction(direction: RoadVec2) -> Option<RoadVec2> {
    let length = direction.length();
    (length > TERMINAL_CAP_POLYLINE_POINT_EQUAL_EPS_M).then_some(direction / length)
}

fn quantized_xz_key(point: RoadVec2) -> (i64, i64) {
    (
        (point.x * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
        (point.y * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
    )
}

fn xz(point: RoadVec3) -> RoadVec2 {
    RoadVec2::new(point.x, point.z)
}

#[cfg(test)]
mod tests {
    use super::super::{
        IncidentEdgeSide, IncidentMouthBand, IncidentMouthProfile, OrderedIncidentPieceMouth,
    };
    use super::*;
    use godot::prelude::{Vector2, Vector3};

    fn band(kind: RoadSurfaceBandKind, start: Vector3, end: Vector3) -> IncidentMouthBand {
        IncidentMouthBand {
            kind,
            start_point_world: start,
            end_point_world: end,
        }
    }

    fn symmetric_profile_x(x: f32, inward_direction_xz: Vector2) -> IncidentMouthProfile {
        let boundary_points_world = vec![
            Vector3::new(x, 4.12, -5.0),
            Vector3::new(x, 4.12, -3.65),
            Vector3::new(x, 4.0, -3.5),
            Vector3::new(x, 4.0, 0.0),
            Vector3::new(x, 4.0, 3.5),
            Vector3::new(x, 4.12, 3.65),
            Vector3::new(x, 4.12, 5.0),
        ];
        let bands = vec![
            band(
                RoadSurfaceBandKind::Sidewalk,
                boundary_points_world[0],
                boundary_points_world[1],
            ),
            band(
                RoadSurfaceBandKind::CurbOrShoulder,
                boundary_points_world[1],
                Vector3::new(boundary_points_world[2].x, 4.12, boundary_points_world[2].z),
            ),
            band(
                RoadSurfaceBandKind::Carriageway,
                boundary_points_world[2],
                boundary_points_world[3],
            ),
            band(
                RoadSurfaceBandKind::Carriageway,
                boundary_points_world[3],
                boundary_points_world[4],
            ),
            band(
                RoadSurfaceBandKind::CurbOrShoulder,
                Vector3::new(boundary_points_world[4].x, 4.12, boundary_points_world[4].z),
                boundary_points_world[5],
            ),
            band(
                RoadSurfaceBandKind::Sidewalk,
                boundary_points_world[5],
                boundary_points_world[6],
            ),
        ];
        IncidentMouthProfile {
            inward_direction_xz,
            boundary_points_world,
            bands,
        }
    }

    fn car_only_profile_x(x: f32, inward_direction_xz: Vector2) -> IncidentMouthProfile {
        let boundary_points_world = vec![
            Vector3::new(x, 4.0, -3.5),
            Vector3::new(x, 4.0, 0.0),
            Vector3::new(x, 4.0, 3.5),
        ];
        let bands = vec![
            band(
                RoadSurfaceBandKind::Carriageway,
                boundary_points_world[0],
                boundary_points_world[1],
            ),
            band(
                RoadSurfaceBandKind::Carriageway,
                boundary_points_world[1],
                boundary_points_world[2],
            ),
        ];
        IncidentMouthProfile {
            inward_direction_xz,
            boundary_points_world,
            bands,
        }
    }

    fn asymmetric_sidewalk_profile_x(x: f32, inward_direction_xz: Vector2) -> IncidentMouthProfile {
        let mut profile = symmetric_profile_x(x, inward_direction_xz);
        profile.boundary_points_world[6] = Vector3::new(x, 4.12, 5.5);
        profile.bands[5].end_point_world = profile.boundary_points_world[6];
        profile
    }

    fn terminal_input(profile: IncidentMouthProfile) -> NodeArrangementInput {
        let endpoint_profile = profile.clone();
        let mouth_profile = IncidentMouthProfile {
            inward_direction_xz: profile.inward_direction_xz,
            boundary_points_world: profile
                .boundary_points_world
                .iter()
                .map(|point| Vector3::new(point.x + 10.0, point.y, point.z))
                .collect(),
            bands: profile
                .bands
                .iter()
                .map(|band| {
                    let start = band.start_point_world;
                    let end = band.end_point_world;
                    IncidentMouthBand {
                        kind: band.kind,
                        start_point_world: Vector3::new(start.x + 10.0, start.y, start.z),
                        end_point_world: Vector3::new(end.x + 10.0, end.y, end.z),
                    }
                })
                .collect(),
        };
        let mouth = OrderedIncidentPieceMouth {
            profile: mouth_profile,
            endpoint_profile,
            boundary_paths_world: Vec::new(),
            band_start_paths_world: Vec::new(),
            band_end_paths_world: Vec::new(),
            uses_sampled_band_domain_paths: false,
            direction_angle_ccw: 0.0,
            direction_xz: Vector2::RIGHT,
            edge_idx: 8,
            side: IncidentEdgeSide::Start,
        };

        NodeArrangementInput::from_ordered_mouths(
            42,
            RoadSurfaceVisualNodePieceKind::Terminal,
            &[mouth],
        )
        .expect("valid terminal profile should produce canonical input")
    }

    #[test]
    fn terminal_cap_adapter_uses_source_band_interval_heights() {
        let input = terminal_input(symmetric_profile_x(0.0, Vector2::RIGHT));
        let mouth = &input.mouths[0];
        let cap_bands_by_mouth =
            terminal_cap_bands_by_mouth(&input).expect("symmetric terminal cap is valid");
        let center_boundary = mouth.boundary_rails[3].endpoint_world;
        let expected_height_m = mouth.band_intervals[1]
            .endpoint_end_world
            .y
            .max(mouth.band_intervals[4].endpoint_start_world.y);

        assert!(cap_bands_by_mouth[0].iter().any(|cap_band| {
            cap_band.band_kind == RoadSurfaceBandKind::CurbOrShoulder
                && cap_band.inner_path_world.iter().any(|point| {
                    (point.x - center_boundary.x).abs() <= 0.001
                        && (point.z - center_boundary.z).abs() <= 0.001
                        && (point.y - expected_height_m).abs() <= 0.001
                })
        }));
    }

    #[test]
    fn terminal_cap_adapter_records_cap_source_provenance() {
        let input = terminal_input(symmetric_profile_x(0.0, Vector2::RIGHT));
        let mouth = &input.mouths[0];
        let cap_bands_by_mouth =
            terminal_cap_bands_by_mouth(&input).expect("symmetric terminal cap is valid");
        let first_terminal_source_band = mouth.band_intervals.len();
        let end_band = cap_bands_by_mouth[0]
            .iter()
            .find(|cap_band| {
                cap_band.source_band_index == first_terminal_source_band
                    && cap_band.provenance.role == TerminalCapBandRole::EndBand
            })
            .expect("curb terminal cap should include an endpoint span");

        assert_eq!(end_band.band_kind, RoadSurfaceBandKind::CurbOrShoulder);
        assert_eq!(end_band.provenance.layer_index, 0);
        assert_eq!(end_band.provenance.left_source_band_index, 1);
        assert_eq!(end_band.provenance.right_source_band_index, 4);
        assert_eq!(end_band.provenance.source_boundary_start_index, 2);
        assert_eq!(end_band.provenance.source_boundary_end_index, 4);
    }

    #[test]
    fn terminal_cap_adapter_emits_side_corner_closures_from_source_rails() {
        let input = terminal_input(symmetric_profile_x(0.0, Vector2::RIGHT));
        let mouth = &input.mouths[0];
        let cap_bands_by_mouth =
            terminal_cap_bands_by_mouth(&input).expect("symmetric terminal cap is valid");
        let sidewalk_terminal_source_band = mouth.band_intervals.len() + 1;
        let left_corner = cap_bands_by_mouth[0]
            .iter()
            .find(|cap_band| {
                cap_band.source_band_index == sidewalk_terminal_source_band
                    && cap_band.provenance.role == TerminalCapBandRole::LeftCorner
            })
            .expect("sidewalk terminal cap must close the left endpoint-to-cap corner");
        let right_corner = cap_bands_by_mouth[0]
            .iter()
            .find(|cap_band| {
                cap_band.source_band_index == sidewalk_terminal_source_band
                    && cap_band.provenance.role == TerminalCapBandRole::RightCorner
            })
            .expect("sidewalk terminal cap must close the right endpoint-to-cap corner");

        assert_eq!(left_corner.band_kind, RoadSurfaceBandKind::Sidewalk);
        assert_eq!(left_corner.provenance.source_boundary_start_index, 0);
        assert_eq!(left_corner.provenance.source_boundary_end_index, 1);
        assert!((left_corner.provenance.inner_offset_m - 0.0).abs() <= 0.001);
        assert!((left_corner.provenance.outer_offset_m - 0.15).abs() <= 0.001);
        assert_eq!(right_corner.band_kind, RoadSurfaceBandKind::Sidewalk);
        assert_eq!(right_corner.provenance.source_boundary_start_index, 5);
        assert_eq!(right_corner.provenance.source_boundary_end_index, 6);
        assert!((right_corner.provenance.inner_offset_m - 0.0).abs() <= 0.001);
        assert!((right_corner.provenance.outer_offset_m - 0.15).abs() <= 0.001);

        let left_endpoint_outer = mouth.boundary_rails[0].endpoint_world;
        assert!(left_corner.inner_path_world.iter().any(|point| {
            (point.x - left_endpoint_outer.x).abs() <= 0.001
                && (point.z - left_endpoint_outer.z).abs() <= 0.001
                && (point.y - left_endpoint_outer.y).abs() <= 0.001
        }));
        assert!(left_corner.outer_path_world.iter().any(|point| {
            (point.x - (left_endpoint_outer.x - 0.15)).abs() <= 0.001
                && (point.z - left_endpoint_outer.z).abs() <= 0.001
                && (point.y - left_endpoint_outer.y).abs() <= 0.001
        }));
    }

    #[test]
    fn terminal_cap_adapter_rejects_asymmetric_paired_band_widths() {
        let input = terminal_input(asymmetric_sidewalk_profile_x(0.0, Vector2::RIGHT));
        let error = terminal_cap_bands_by_mouth(&input)
            .expect_err("paired terminal caps must not silently truncate asymmetric widths");

        assert_eq!(
            error.reason,
            TerminalCapFailureReason::MismatchedPairedBandWidth
        );
        assert_eq!(error.layer_index, Some(1));
        assert_eq!(error.band_kind, Some(RoadSurfaceBandKind::Sidewalk));
    }

    #[test]
    fn car_only_terminal_emits_no_non_road_cap() {
        let input = terminal_input(car_only_profile_x(0.0, Vector2::RIGHT));
        let cap_bands_by_mouth =
            terminal_cap_bands_by_mouth(&input).expect("car-only terminal has no cap bands");

        assert!(cap_bands_by_mouth.iter().flatten().next().is_none());
    }
}
