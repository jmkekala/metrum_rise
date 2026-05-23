//! Paired terminal cap-band construction.

use super::*;

pub(super) fn push_terminal_paired_cap_bands(
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
