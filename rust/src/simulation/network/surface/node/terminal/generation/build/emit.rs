// SPDX-License-Identifier: GPL-2.0-only

//! Terminal cap-band emission helpers.

use super::*;

pub(super) fn push_terminal_cap_band(
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

pub(super) fn band_width_m(
    band: &crate::simulation::network::surface::node::input::NodeInputBandInterval,
) -> f64 {
    xz(band.endpoint_start_world).distance(xz(band.endpoint_end_world))
}
