//! Node input rail, interval, and path construction.

use super::*;

pub(super) fn profile_rails(
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

pub(super) fn boundary_rails(mouth: &OrderedIncidentPieceMouth) -> Vec<NodeInputBoundaryRail> {
    mouth
        .profile
        .boundary_points_world
        .iter()
        .zip(&mouth.endpoint_profile.boundary_points_world)
        .enumerate()
        .map(|(boundary_index, (mouth_point, endpoint_point))| {
            let mouth_world = *mouth_point;
            let endpoint_world = *endpoint_point;
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

pub(super) fn band_intervals(mouth: &OrderedIncidentPieceMouth) -> Vec<NodeInputBandInterval> {
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
    path_world: Option<&[RoadVec3]>,
    mouth_world: RoadVec3,
    endpoint_world: RoadVec3,
) -> Vec<RoadVec3> {
    if let Some(path_world) = path_world.filter(|path| path.len() >= 2) {
        let mut points = path_world.to_vec();
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
    band_point_world: RoadVec3,
    boundary_point_world: RoadVec3,
) -> RoadVec3 {
    RoadVec3::new(
        boundary_point_world.x,
        band_point_world.y,
        boundary_point_world.z,
    )
}

pub(super) fn replace_profile_paths_with_chords(
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

pub(super) fn quantize_profile_rails_xz(rails: &mut [NodeInputProfileRail]) {
    for rail in rails {
        rail.start_world = quantize_road_vec3_xz_to_overlay_grid(rail.start_world);
        rail.end_world = quantize_road_vec3_xz_to_overlay_grid(rail.end_world);
    }
}

pub(super) fn quantize_boundary_rails_xz(rails: &mut [NodeInputBoundaryRail]) {
    for rail in rails {
        rail.mouth_world = quantize_road_vec3_xz_to_overlay_grid(rail.mouth_world);
        rail.endpoint_world = quantize_road_vec3_xz_to_overlay_grid(rail.endpoint_world);
        quantize_road_vec3_path_xz_to_overlay_grid(&mut rail.path_world);
    }
}

pub(super) fn quantize_band_intervals_xz(intervals: &mut [NodeInputBandInterval]) {
    for interval in intervals {
        interval.mouth_start_world =
            quantize_road_vec3_xz_to_overlay_grid(interval.mouth_start_world);
        interval.mouth_end_world = quantize_road_vec3_xz_to_overlay_grid(interval.mouth_end_world);
        interval.endpoint_start_world =
            quantize_road_vec3_xz_to_overlay_grid(interval.endpoint_start_world);
        interval.endpoint_end_world =
            quantize_road_vec3_xz_to_overlay_grid(interval.endpoint_end_world);
        quantize_road_vec3_path_xz_to_overlay_grid(&mut interval.start_path_world);
        quantize_road_vec3_path_xz_to_overlay_grid(&mut interval.end_path_world);
    }
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
