// SPDX-License-Identifier: GPL-2.0-only

//! Variant export helpers for `SimulationNode`.

mod cdt;
mod economy;
mod terrain_patch;
mod water;
mod world;
mod zoning;

pub(super) use cdt::{TERRAIN_CDT_TILE_NEIGHBORS, TerrainCdtTileId};
#[cfg(test)]
pub(super) use cdt::{TerrainCdtSourceExport, TerrainCdtTriangleBufferExport};
pub(super) use economy::budget_ledger_entry_dict;
pub(super) use zoning::{
    zoning_geometries_without_explicit_sites, zoning_parcel_cell_dimensions, zoning_parcel_color,
    zoning_parcel_geometries_array, zoning_parcel_geometries_packed_dict,
    zoning_parcel_geometry_dict, zoning_parcel_surface_corners,
};
