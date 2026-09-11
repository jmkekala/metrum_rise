// SPDX-License-Identifier: GPL-2.0-only

//! Building-site derivation, grading, terrain ownership, and spatial queries.

mod derive;
mod geometry;
mod grading;
mod model;
mod paving;
mod query;
mod terrain_clip;

pub(crate) use grading::{BuildingSiteGradingRequest, building_site_support_tie_in_is_valid};
pub(crate) use grading::{
    site_feasibility_dependency_margin_m, solve_building_site_support_height,
};
pub(crate) use model::BuildingSiteSurfaceClient;
pub(crate) use model::{BuildingSiteClient, BuildingSiteTerrainSnapshot};
pub(crate) use paving::SitePavingPartition;

#[cfg(test)]
mod tests;
