// SPDX-License-Identifier: GPL-2.0-only

//! Source height-carrier path normalization.

use super::super::super::backend::{RoadVec3, road_vec3_xz as xz};
use super::super::super::input::NodeInputBandInterval;
use super::super::super::keys::SurfaceHeightMmKey;
use super::super::NodeRailHeightCarrierPaths;
use super::super::contours::subdivided_world_chord;
use super::super::geometry::road_point_key;

pub(in crate::simulation::network::surface::node::rails) fn interval_height_carrier_paths(
    interval: &NodeInputBandInterval,
) -> NodeRailHeightCarrierPaths {
    if interval.start_path_world.len() > 2
        && source_height_path_is_endpoint_chord(
            &interval.end_path_world,
            interval.mouth_end_world,
            interval.endpoint_end_world,
        )
    {
        return NodeRailHeightCarrierPaths {
            start_path_world: interval.start_path_world.clone(),
            end_path_world: subdivided_world_chord(
                interval.mouth_end_world,
                interval.endpoint_end_world,
                interval.start_path_world.len(),
            ),
        };
    }
    if interval.end_path_world.len() > 2
        && source_height_path_is_endpoint_chord(
            &interval.start_path_world,
            interval.mouth_start_world,
            interval.endpoint_start_world,
        )
    {
        return NodeRailHeightCarrierPaths {
            start_path_world: subdivided_world_chord(
                interval.mouth_start_world,
                interval.endpoint_start_world,
                interval.end_path_world.len(),
            ),
            end_path_world: interval.end_path_world.clone(),
        };
    }
    NodeRailHeightCarrierPaths {
        start_path_world: interval.start_path_world.clone(),
        end_path_world: interval.end_path_world.clone(),
    }
}

pub(in crate::simulation::network::surface::node::rails) fn interval_height_carrier_points(
    interval: &NodeInputBandInterval,
    paths: &NodeRailHeightCarrierPaths,
) -> Vec<RoadVec3> {
    [
        interval.endpoint_start_world,
        interval.endpoint_end_world,
        interval.mouth_end_world,
        interval.mouth_start_world,
    ]
    .into_iter()
    .chain(paths.start_path_world.iter().copied())
    .chain(paths.end_path_world.iter().copied())
    .collect()
}

fn source_height_path_is_endpoint_chord(
    path_world: &[RoadVec3],
    mouth_world: RoadVec3,
    endpoint_world: RoadVec3,
) -> bool {
    path_world.len() == 2
        && source_height_points_match(path_world[0], mouth_world)
        && source_height_points_match(path_world[1], endpoint_world)
}

fn source_height_points_match(a: RoadVec3, b: RoadVec3) -> bool {
    road_point_key(xz(a)) == road_point_key(xz(b))
        && SurfaceHeightMmKey::from_m_f64(a.y) == SurfaceHeightMmKey::from_m_f64(b.y)
}
