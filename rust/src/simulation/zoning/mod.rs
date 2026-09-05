// SPDX-License-Identifier: GPL-2.0-only

//! Road-aligned zoning parcels and built-in zoning-profile registry.
//!
//! User-authored parcels are the zoning authority for private building spawn. Broad zoning-family
//! values remain derived helpers for systems that consume residential/commercial/industrial
//! families. The simulation, demand, allocator, and saves consume stable parcel ids.

mod constants;
pub mod parcels;
pub mod profiles;
mod system;
mod zone_type;

pub use constants::{
    DEFAULT_PARCEL_DEPTH_M, DEFAULT_PARCEL_FRONTAGE_M, MAX_PARCEL_DEPTH_M, MAX_PARCEL_FRONTAGE_M,
    MAX_PARCEL_GAP_M, MIN_PARCEL_DEPTH_M, MIN_PARCEL_FRONTAGE_M, MIN_PARCEL_GAP_M,
};
pub use parcels::{ParcelGeometry, ParcelId, ParcelPlacementError, ParcelStore, ZoningParcel};
pub use profiles::{
    ZoneDensity, ZoneProfileRuntime, ZoningProfileRegistry, load_builtin_profile_registry,
};
pub(crate) use system::ZoningParcelRemovalUndo;
pub use system::ZoningSystem;
pub use zone_type::ZoneType;

/// Unit tests for the parcel zoning system.
#[cfg(test)]
pub mod tests;
