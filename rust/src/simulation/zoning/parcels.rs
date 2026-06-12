//! Road-aligned zoning parcel storage and placement geometry.

mod geometry;
mod placement;
mod store;
mod types;

const OVERLAP_EPSILON_M: f32 = 0.001;
const CURVE_RUN_SPACING_SEARCH_STEP_M: f32 = 0.5;
const CURVE_RUN_SPACING_BINARY_STEPS: usize = 10;
const ROAD_OVERLAP_QUERY_PAD_M: f32 = 128.0;

pub use store::ParcelStore;
pub use types::{ParcelGeometry, ParcelId, ParcelPlacementError, ZoningParcel};

pub(crate) use geometry::{
    chunks_for_aabb, geometries_overlap, geometry_for_parcel, geometry_from_attachment,
    geometry_inside_world, geometry_overlaps_road,
};
pub(crate) use placement::{
    project_default_parcel_at, project_parcel_run_at, project_parcel_run_from_existing,
};
