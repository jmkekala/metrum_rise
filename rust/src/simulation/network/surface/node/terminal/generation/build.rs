//! Terminal-cap band construction before canonical cleanup.

use super::*;

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
        .map(clean_terminal_cap_path_world)
        .transpose()
        .map_err(|_| {
            TerminalCapGenerationError::for_cap(
                mouth,
                source_band_index,
                band_kind,
                provenance,
                TerminalCapFailureReason::ConflictingPathHeight,
            )
        })?
        .flatten()
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
        .map(clean_terminal_cap_path_world)
        .transpose()
        .map_err(|_| {
            TerminalCapGenerationError::for_cap(
                mouth,
                source_band_index,
                band_kind,
                provenance,
                TerminalCapFailureReason::ConflictingPathHeight,
            )
        })?
        .flatten()
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
        .map_err(|_| {
            TerminalCapGenerationError::for_cap(
                mouth,
                source_band_index,
                band_kind,
                provenance,
                TerminalCapFailureReason::ConflictingPathHeight,
            )
        })?
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

fn band_width_m(band: &super::super::super::input::NodeInputBandInterval) -> f64 {
    xz(band.endpoint_start_world).distance(xz(band.endpoint_end_world))
}
