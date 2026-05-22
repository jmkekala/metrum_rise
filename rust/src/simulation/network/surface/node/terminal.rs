//! Canonical terminal-cap adapter for one-mouth visual node ownership.

use super::backend::{
    RoadPolyline, RoadVec2, RoadVec3, polyline_to_road_points,
    quantize_road_vec3_path_xz_to_overlay_grid, road_points_to_polyline, road_vec3_xz as xz,
};
use super::input::{NodeArrangementInput, NodeInputMouth};
use super::paths::{
    PathHeightResolutionError, cleaned_open_world_path_polyline, closed_world_contour_has_area,
    reheight_road_points_from_world_path,
};
use super::{NODE_OVERLAY_MIN_AREA_M2, RoadSurfaceBandKind, RoadSurfaceVisualNodePieceKind};
use cavalier_contours::polyline::{PlineCreation, PlineSource};

const TERMINAL_CAP_POLYLINE_POINT_EQUAL_EPS_M: f64 = 1.0e-6;
const TERMINAL_CAP_WIDTH_EPS_M: f64 = 0.001;

mod generation;
mod height_anchors;
mod paths;

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
    ConflictingPathHeight,
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
            Self::ConflictingPathHeight => "terminal_cap_conflicting_path_height",
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
        let mut bands = generation::terminal_cap_bands(mouth)?;
        generation::canonicalize_terminal_cap_bands(mouth, &mut bands)?;
        bands_by_mouth[mouth_index] = bands;
    }

    Ok(bands_by_mouth)
}

#[cfg(test)]
mod tests;
