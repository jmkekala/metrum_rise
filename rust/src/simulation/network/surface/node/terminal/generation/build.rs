//! Terminal-cap band construction before canonical cleanup.

use super::*;

mod emit;
mod paired;
mod side_corner;

use emit::*;
use paired::*;
use side_corner::*;

pub(in crate::simulation::network::surface::node::terminal) fn terminal_cap_bands(
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
