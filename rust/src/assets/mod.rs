//! Asset pack manifest schema and deserialization.
//!
//! Defines the v1 content contract between the asset editor, the runtime, and modders.
//! Rust owns manifest parsing and validation; Godot owns mesh/texture loading.
//!
//! Two manifest files govern every content pack:
//! - `pack.toml` — pack identity and top-level metadata ([`PackManifest`]).
//! - `asset.toml` — per-asset metadata and class-specific fields ([`AssetManifest`]).
//!
//! Neither file ships binary data. All mesh and texture paths inside these manifests
//! are relative to the asset's folder within the pack.

pub mod asset;
pub mod pack;
pub mod registry;
pub mod scanner;

pub use asset::{
    Anchor, AnchorType, ArchetypeFamily, AssetClass, AssetManifest, BuildingData, CharacterData,
    ColorVariant, LodEntry, MeshPart, PropData, SiteSurface, SiteSurfaceMaterial, SkinVariant,
    SnapMode, TerrainBehavior, VehicleClass, VehicleData, VehicleFamily, ZoneClass,
};
pub use pack::PackManifest;
pub use registry::{AssetEntry, AssetRegistry};
pub use scanner::{ScanResult, ScannedPack, scan_pack_dir};

use std::fmt;

/// Schema version this crate reads and writes. Increment when the manifest format changes
/// in a backwards-incompatible way.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Errors produced by manifest parsing or validation.
#[derive(Debug)]
pub enum ManifestError {
    /// TOML syntax error from the `toml` crate.
    TomlParse(toml::de::Error),
    /// Structural or semantic validation failure.
    Validation(String),
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TomlParse(e) => write!(f, "TOML parse error: {e}"),
            Self::Validation(msg) => write!(f, "manifest validation error: {msg}"),
        }
    }
}

impl From<toml::de::Error> for ManifestError {
    fn from(e: toml::de::Error) -> Self {
        Self::TomlParse(e)
    }
}

/// Returns `true` when `s` is a valid pack ID: non-empty, kebab-case,
/// containing only ASCII lowercase letters, digits, and hyphens.
pub fn is_valid_pack_id(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && !s.ends_with('-')
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Returns `true` when `s` is a valid asset ID: one or more dot-separated segments,
/// each containing only ASCII lowercase letters, digits, and underscores.
pub fn is_valid_asset_id(s: &str) -> bool {
    !s.is_empty()
        && s.split('.').all(|seg| {
            !seg.is_empty()
                && seg
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_pack_ids() {
        assert!(is_valid_pack_id("kenney-city-pack"));
        assert!(is_valid_pack_id("base-buildings"));
        assert!(is_valid_pack_id("pack1"));
    }

    #[test]
    fn invalid_pack_ids() {
        assert!(!is_valid_pack_id(""));
        assert!(!is_valid_pack_id("Kenney")); // uppercase
        assert!(!is_valid_pack_id("my_pack")); // underscore not allowed
        assert!(!is_valid_pack_id("-leading-dash")); // leading hyphen
        assert!(!is_valid_pack_id("trailing-")); // trailing hyphen
    }

    #[test]
    fn valid_asset_ids() {
        assert!(is_valid_asset_id("building.residential.lowrise_corner"));
        assert!(is_valid_asset_id("vehicle.civil.sedan"));
        assert!(is_valid_asset_id("prop.street.bench_01"));
    }

    #[test]
    fn invalid_asset_ids() {
        assert!(!is_valid_asset_id(""));
        assert!(!is_valid_asset_id(".leading.dot"));
        assert!(!is_valid_asset_id("trailing.dot."));
        assert!(!is_valid_asset_id("has-hyphen.segment"));
        assert!(!is_valid_asset_id("HasUpper.case"));
    }
}
