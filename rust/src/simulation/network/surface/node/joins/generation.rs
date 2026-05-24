//! Side-join band generation between adjacent node mouths.

use super::heights::endpoint_height_plane_for_band_kind;
use super::paths::{
    endpoint_layer_inner_world, endpoint_layer_outer_world, reheight_side_join_path_world,
    side_join_boundary_path_world,
};
use super::*;

pub(super) fn add_bend_side_join_bands(
    mouths: &[NodeInputMouth],
    bands_by_mouth: &mut [Vec<NodeInputSideJoinBand>],
) -> Result<(), SideJoinGenerationError> {
    if mouths.len() != 2 {
        return Ok(());
    }

    append_adjacent_side_join_bands(mouths, bands_by_mouth, 0, 1, SideJoinPathMode::BendArc)?;
    append_adjacent_side_join_bands(mouths, bands_by_mouth, 1, 0, SideJoinPathMode::BendArc)?;
    Ok(())
}

pub(super) fn add_junction_side_join_bands(
    mouths: &[NodeInputMouth],
    bands_by_mouth: &mut [Vec<NodeInputSideJoinBand>],
) -> Result<(), SideJoinGenerationError> {
    if mouths.len() < 2 {
        return Ok(());
    }

    for from_index in 0..mouths.len() {
        let to_index = if from_index + 1 == mouths.len() {
            0
        } else {
            from_index + 1
        };
        append_adjacent_side_join_bands(
            mouths,
            bands_by_mouth,
            from_index,
            to_index,
            SideJoinPathMode::JunctionNonRoad,
        )?;
    }
    Ok(())
}

fn append_adjacent_side_join_bands(
    mouths: &[NodeInputMouth],
    bands_by_mouth: &mut [Vec<NodeInputSideJoinBand>],
    from_index: usize,
    to_index: usize,
    path_mode: SideJoinPathMode,
) -> Result<(), SideJoinGenerationError> {
    let from_mouth = &mouths[from_index];
    let to_mouth = &mouths[to_index];
    let from_layers = side_join_layers(from_mouth, SideJoinProfileSide::End);
    let to_layers = side_join_layers(to_mouth, SideJoinProfileSide::Start);
    if from_layers.is_empty() || to_layers.is_empty() {
        return Ok(());
    }

    let mut join_bands = side_join_bands(
        mouths,
        from_mouth,
        &from_layers,
        to_mouth,
        &to_layers,
        path_mode,
    )?;
    canonicalize_side_join_bands(&mut join_bands);
    bands_by_mouth[from_index].extend(join_bands);
    Ok(())
}

fn canonicalize_side_join_bands(join_bands: &mut Vec<NodeInputSideJoinBand>) {
    for join_band in join_bands.iter_mut() {
        quantize_side_join_band_xz(join_band);
    }
    join_bands.retain(side_join_band_has_quantized_area);
}

fn side_join_layers(mouth: &NodeInputMouth, side: SideJoinProfileSide) -> Vec<SideJoinLayer> {
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
        SideJoinProfileSide::Start => (0..=first_carriageway)
            .rev()
            .filter_map(|band_index| {
                mouth
                    .band_intervals
                    .get(band_index)
                    .map(|band| SideJoinLayer {
                        band_index,
                        band_kind: band.band_kind,
                        inner_boundary_index: band_index + 1,
                        outer_boundary_index: band_index,
                    })
            })
            .collect(),
        SideJoinProfileSide::End => (last_carriageway..mouth.band_intervals.len())
            .filter_map(|band_index| {
                mouth
                    .band_intervals
                    .get(band_index)
                    .map(|band| SideJoinLayer {
                        band_index,
                        band_kind: band.band_kind,
                        inner_boundary_index: band_index,
                        outer_boundary_index: band_index + 1,
                    })
            })
            .collect(),
    }
}

fn side_join_bands(
    mouths: &[NodeInputMouth],
    from_mouth: &NodeInputMouth,
    from_layers: &[SideJoinLayer],
    to_mouth: &NodeInputMouth,
    to_layers: &[SideJoinLayer],
    path_mode: SideJoinPathMode,
) -> Result<Vec<NodeInputSideJoinBand>, SideJoinGenerationError> {
    let mut join_bands = Vec::new();
    let mut inner_path_world = None;
    for (from_layer, to_layer) in from_layers.iter().zip(to_layers) {
        if from_layer.band_kind != to_layer.band_kind {
            break;
        }
        if path_mode == SideJoinPathMode::JunctionNonRoad
            && from_layer.band_kind == RoadSurfaceBandKind::Carriageway
        {
            inner_path_world = None;
            continue;
        }
        let height_plane = if path_mode == SideJoinPathMode::JunctionNonRoad {
            endpoint_height_plane_for_band_kind(mouths, from_layer.band_kind)?
        } else {
            None
        };

        let Some(band_inner_path_world) = side_join_band_inner_path(
            from_mouth,
            from_layer,
            to_mouth,
            to_layer,
            inner_path_world,
            path_mode,
            height_plane,
        )?
        else {
            break;
        };
        let Some(outer_start_world) = endpoint_layer_outer_world(from_mouth, from_layer) else {
            break;
        };
        let Some(outer_end_world) = endpoint_layer_outer_world(to_mouth, to_layer) else {
            break;
        };
        let Some(band_outer_path_world) = side_join_boundary_path_world(
            from_mouth,
            outer_start_world,
            to_mouth,
            outer_end_world,
            path_mode,
            height_plane,
        )?
        else {
            break;
        };

        let boundary_mode = match from_layer.band_kind {
            RoadSurfaceBandKind::Carriageway
            | RoadSurfaceBandKind::CurbOrShoulder
            | RoadSurfaceBandKind::Sidewalk => NodeInputSideJoinBandBoundaryMode::MaterialBand,
            _ => NodeInputSideJoinBandBoundaryMode::MaterialBandWithSameOwnerOuterCap,
        };
        let Some(next_inner_path_world) = pushed_side_join_band(
            &mut join_bands,
            from_layer.band_index,
            from_layer.band_kind,
            boundary_mode,
            band_inner_path_world,
            band_outer_path_world.clone(),
        )?
        else {
            break;
        };
        inner_path_world = Some(next_inner_path_world);
    }
    Ok(join_bands)
}

fn side_join_band_inner_path(
    from_mouth: &NodeInputMouth,
    from_layer: &SideJoinLayer,
    to_mouth: &NodeInputMouth,
    to_layer: &SideJoinLayer,
    previous_outer_path_world: Option<Vec<RoadVec3>>,
    path_mode: SideJoinPathMode,
    height_plane: Option<SideJoinHeightPlane>,
) -> Result<Option<Vec<RoadVec3>>, SideJoinGenerationError> {
    if let Some(path_world) = previous_outer_path_world {
        if path_mode != SideJoinPathMode::BendArc {
            return Ok(Some(path_world));
        }
        let Some(inner_start_world) = endpoint_layer_inner_world(from_mouth, from_layer) else {
            return Ok(None);
        };
        let Some(inner_end_world) = endpoint_layer_inner_world(to_mouth, to_layer) else {
            return Ok(None);
        };
        return reheight_side_join_path_world(path_world, inner_start_world.y, inner_end_world.y);
    }
    let Some(inner_start_world) = endpoint_layer_inner_world(from_mouth, from_layer) else {
        return Ok(None);
    };
    let Some(inner_end_world) = endpoint_layer_inner_world(to_mouth, to_layer) else {
        return Ok(None);
    };
    side_join_boundary_path_world(
        from_mouth,
        inner_start_world,
        to_mouth,
        inner_end_world,
        path_mode,
        height_plane,
    )
}

fn pushed_side_join_band(
    join_bands: &mut Vec<NodeInputSideJoinBand>,
    source_band_index: usize,
    band_kind: RoadSurfaceBandKind,
    boundary_mode: NodeInputSideJoinBandBoundaryMode,
    inner_path_world: Vec<RoadVec3>,
    outer_path_world: Vec<RoadVec3>,
) -> Result<Option<Vec<RoadVec3>>, SideJoinGenerationError> {
    if inner_path_world.len() < 2 || outer_path_world.len() < 2 {
        return Ok(None);
    }

    let mut contour_world = inner_path_world.clone();
    contour_world.extend(outer_path_world.iter().rev().copied());
    remove_repeated_road_vec3_xz_points(&mut contour_world)
        .map_err(SideJoinGenerationError::from_path_height_error)?;
    let mut join_band = NodeInputSideJoinBand {
        source_band_index,
        band_kind,
        boundary_mode,
        inner_path_world,
        outer_path_world,
        contour_world,
    };
    quantize_side_join_band_xz(&mut join_band);
    if !side_join_band_has_quantized_area(&join_band) {
        return Ok(None);
    }
    let next_inner_path_world = join_band.outer_path_world.clone();
    join_bands.push(join_band);
    Ok(Some(next_inner_path_world))
}

fn quantize_side_join_band_xz(join_band: &mut NodeInputSideJoinBand) {
    quantize_road_vec3_path_xz_to_overlay_grid(&mut join_band.inner_path_world);
    quantize_road_vec3_path_xz_to_overlay_grid(&mut join_band.outer_path_world);
    quantize_road_vec3_path_xz_to_overlay_grid(&mut join_band.contour_world);
}

pub(super) fn side_join_band_has_quantized_area(join_band: &NodeInputSideJoinBand) -> bool {
    closed_world_contour_has_area(
        &join_band.contour_world,
        SIDE_JOIN_POLYLINE_POINT_EQUAL_EPS_M,
        f64::from(NODE_OVERLAY_MIN_AREA_M2),
    )
}
