//! Terminal-cap band canonicalization after construction.

use super::*;

pub(in crate::simulation::network::surface::node::terminal) fn canonicalize_terminal_cap_bands(
    mouth: &NodeInputMouth,
    cap_bands: &mut [NodeTerminalCapBand],
) -> Result<(), TerminalCapGenerationError> {
    for cap_band in cap_bands.iter_mut() {
        quantize_terminal_cap_band_xz(cap_band);
        let Some(inner_path_world) =
            clean_terminal_cap_path_world(cap_band.inner_path_world.clone()).map_err(|_| {
                TerminalCapGenerationError::for_cap(
                    mouth,
                    cap_band.source_band_index,
                    cap_band.band_kind,
                    cap_band.provenance,
                    TerminalCapFailureReason::ConflictingPathHeight,
                )
            })?
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
            clean_terminal_cap_path_world(cap_band.outer_path_world.clone()).map_err(|_| {
                TerminalCapGenerationError::for_cap(
                    mouth,
                    cap_band.source_band_index,
                    cap_band.band_kind,
                    cap_band.provenance,
                    TerminalCapFailureReason::ConflictingPathHeight,
                )
            })?
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
            .map_err(|_| {
                TerminalCapGenerationError::for_cap(
                    mouth,
                    cap_band.source_band_index,
                    cap_band.band_kind,
                    cap_band.provenance,
                    TerminalCapFailureReason::ConflictingPathHeight,
                )
            })?
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
    quantize_road_vec3_path_xz_to_overlay_grid(&mut cap_band.inner_path_world);
    quantize_road_vec3_path_xz_to_overlay_grid(&mut cap_band.outer_path_world);
    quantize_road_vec3_path_xz_to_overlay_grid(&mut cap_band.contour_world);
}

fn terminal_cap_band_has_quantized_area(cap_band: &NodeTerminalCapBand) -> bool {
    closed_world_contour_has_area(
        &cap_band.contour_world,
        TERMINAL_CAP_POLYLINE_POINT_EQUAL_EPS_M,
        f64::from(NODE_OVERLAY_MIN_AREA_M2),
    )
}
