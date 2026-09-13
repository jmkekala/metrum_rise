// SPDX-License-Identifier: GPL-2.0-only

//! Road attachment projection and parcel-run placement.

mod projection;
mod run;

use super::geometry::geometry_from_attachment;
use super::types::{ParcelGeometry, ParcelPlacementError};
use crate::simulation::network::graph::RegionGraph;
use godot::prelude::Vector2;

pub(crate) use projection::{project_buildable_road_point_at, project_point_to_edge, road_edge_at};
pub(crate) use run::{
    ParcelRunProjection, next_non_overlapping_run_geometry_with, project_parcel_run_layouts_at,
};

pub(crate) fn project_default_parcel_at(
    graph: &RegionGraph,
    world_pos: Vector2,
    frontage_m: f32,
    depth_m: f32,
) -> Result<ParcelGeometry, ParcelPlacementError> {
    let projected =
        projection::project_buildable_road_point_at(graph, world_pos, frontage_m, depth_m)?;
    if projected.s_m < frontage_m * 0.5 || projected.s_m > projected.edge_len_m - frontage_m * 0.5 {
        return Err(ParcelPlacementError::FrontageOutOfBounds);
    }

    Ok(geometry_from_attachment(
        graph,
        projected.edge_idx,
        projected.side,
        projected.s_m / projected.edge_len_m,
        frontage_m,
        depth_m,
    ))
}
