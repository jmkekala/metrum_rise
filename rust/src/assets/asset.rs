// SPDX-License-Identifier: GPL-2.0-only

//! Per-asset manifest (`asset.toml`) schema and validation API.
//!
//! Every imported asset ships with one manifest describing its class, dimensions,
//! meshes, anchors, and class-specific gameplay metadata. The implementation is
//! split by ownership while this module preserves the established public API.

mod appearance;
mod building;
mod character;
pub(crate) mod geometry;
mod model;
pub(crate) mod validation;
mod vehicle;

pub use appearance::{BuildingAppearance, ColourScheme, MaterialOverride, SpawnAppearance};
pub use building::{
    BuildingData, BuildingExtractorData, BuildingFieldData, PlacementMode, ZoneClass,
};
pub use character::{ArchetypeFamily, CharacterData, SkinVariant};
pub use model::{
    Anchor, AnchorType, AssetClass, AssetManifest, LodEntry, MeshPart, PropData, SiteSurface,
    SiteSurfaceMaterial, SnapMode, TerrainBehavior,
};
pub use vehicle::{ColorVariant, VehicleClass, VehicleData, VehicleFamily};

#[cfg(test)]
mod tests;
