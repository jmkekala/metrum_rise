// SPDX-License-Identifier: GPL-2.0-only

//! Terminal side-corner cap-band construction.

use super::*;

pub(super) fn push_terminal_side_corner_cap_band(
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
