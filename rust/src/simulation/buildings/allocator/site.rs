// SPDX-License-Identifier: GPL-2.0-only

//! Building-site derivation, grading, terrain ownership, and spatial queries.

mod cells;
mod derive;
mod geometry;
mod grading;
mod model;
mod query;

pub(crate) use cells::SiteCells;
pub(crate) use derive::zoning_support_footprint;
pub(crate) use grading::building_site_support_tie_in_is_valid;
#[cfg(test)]
pub(crate) use grading::SiteRoads;
pub(crate) use grading::{
    site_feasibility_dependency_margin_m, solve_building_site_support_height,
};
pub(crate) use model::BuildingSiteSurfaceClient;
pub(crate) use model::{BuildingSiteClient, BuildingSiteTerrainSnapshot};
pub use model::SiteChange;
pub(crate) use query::site_footprint_contains;
pub(crate) use query::SiteSnapshot;

#[cfg(test)]
mod tests;
