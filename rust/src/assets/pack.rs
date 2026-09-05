// SPDX-License-Identifier: GPL-2.0-only

//! Pack-level manifest (`pack.toml`).
//!
//! Every content pack ships exactly one `pack.toml` at the pack root.
//! It records the pack's stable identity, display metadata, and authorship.

use super::{CURRENT_SCHEMA_VERSION, ManifestError, is_valid_pack_id};
use serde::Deserialize;

/// Top-level pack manifest deserialized from `pack.toml`.
///
/// `pack_id` is the stable runtime key. `version` and `display_name` are for the
/// content manager UI. Everything else is attribution metadata.
#[derive(Debug, Clone, Deserialize)]
pub struct PackManifest {
    /// Stable kebab-case identifier for this pack. Globally unique by convention.
    /// Used as the namespace in fully-qualified asset IDs (`pack_id:asset_id`).
    pub pack_id: String,
    /// Manifest schema version. Must equal [`CURRENT_SCHEMA_VERSION`] to load.
    pub schema_version: u32,
    /// Human-readable name shown in the content manager.
    pub display_name: String,
    /// Pack release version as a semver string (`MAJOR.MINOR.PATCH`).
    pub version: String,
    /// Author name or organisation.
    pub author: String,
    /// SPDX license identifier or plain license name.
    pub license: String,
    /// Optional short description shown in the content manager.
    pub description: Option<String>,
    /// Optional search tags for the content browser.
    #[serde(default)]
    pub tags: Vec<String>,
}

impl PackManifest {
    /// Parses a `pack.toml` TOML string into a [`PackManifest`].
    pub fn from_str(s: &str) -> Result<Self, ManifestError> {
        let manifest: Self = toml::from_str(s)?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Validates structural and semantic constraints on this manifest.
    ///
    /// Returns `Err` if `pack_id` is not valid kebab-case or if `schema_version`
    /// does not match [`CURRENT_SCHEMA_VERSION`].
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.schema_version != CURRENT_SCHEMA_VERSION {
            return Err(ManifestError::Validation(format!(
                "pack '{}': schema_version {} is not supported (expected {})",
                self.pack_id, self.schema_version, CURRENT_SCHEMA_VERSION
            )));
        }
        if !is_valid_pack_id(&self.pack_id) {
            return Err(ManifestError::Validation(format!(
                "invalid pack_id '{}': must be non-empty kebab-case (lowercase letters, digits, hyphens)",
                self.pack_id
            )));
        }
        if self.display_name.is_empty() {
            return Err(ManifestError::Validation(format!(
                "pack '{}': display_name must not be empty",
                self.pack_id
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_PACK_TOML: &str = r#"
pack_id = "kenney-city-pack"
schema_version = 1
display_name = "Kenney City Pack"
version = "1.0.0"
author = "Kenney"
license = "CC0"
description = "Open game art city assets."
tags = ["city", "lowpoly", "kenney"]
"#;

    #[test]
    fn pack_manifest_round_trip() {
        let m = PackManifest::from_str(VALID_PACK_TOML).expect("parse failed");
        assert_eq!(m.pack_id, "kenney-city-pack");
        assert_eq!(m.schema_version, 1);
        assert_eq!(m.display_name, "Kenney City Pack");
        assert_eq!(m.version, "1.0.0");
        assert_eq!(m.author, "Kenney");
        assert_eq!(m.license, "CC0");
        assert_eq!(m.description.as_deref(), Some("Open game art city assets."));
        assert_eq!(m.tags, ["city", "lowpoly", "kenney"]);
    }

    #[test]
    fn pack_manifest_defaults_empty_tags() {
        let toml = r#"
pack_id = "minimal-pack"
schema_version = 1
display_name = "Minimal"
version = "0.1.0"
author = "Test"
license = "MIT"
"#;
        let m = PackManifest::from_str(toml).expect("parse failed");
        assert!(m.tags.is_empty());
        assert!(m.description.is_none());
    }

    #[test]
    fn pack_manifest_rejects_wrong_schema_version() {
        let toml = r#"
pack_id = "test-pack"
schema_version = 99
display_name = "Test"
version = "1.0.0"
author = "Test"
license = "MIT"
"#;
        assert!(PackManifest::from_str(toml).is_err());
    }

    #[test]
    fn pack_manifest_rejects_invalid_pack_id() {
        let toml = r#"
pack_id = "Bad_Pack"
schema_version = 1
display_name = "Bad"
version = "1.0.0"
author = "Test"
license = "MIT"
"#;
        assert!(PackManifest::from_str(toml).is_err());
    }
}
