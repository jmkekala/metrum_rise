//! Building-site derivation, grading, terrain ownership, and spatial queries.

mod derive;
mod geometry;
mod grading;
mod model;
mod query;
mod terrain_clip;

pub(crate) use grading::{BuildingSiteGradingRequest, building_site_support_tie_in_is_valid};
pub(crate) use model::BuildingSiteClient;

#[cfg(test)]
mod tests;
