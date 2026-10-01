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
        if !is_valid_semver(&self.version) {
            return Err(ManifestError::Validation(format!(
                "pack '{}': version '{}' must be a semantic version such as 1.2.0",
                self.pack_id, self.version
            )));
        }
        Ok(())
    }
}

/// Editable `pack.toml` metadata; `pack_id` and the schema version are fixed.
pub(crate) struct PackSettings<'a> {
    pub(crate) display_name: &'a str,
    pub(crate) version: &'a str,
    pub(crate) author: &'a str,
    pub(crate) license: &'a str,
    pub(crate) description: &'a str,
}

impl<'a> PackSettings<'a> {
    pub(crate) fn of(manifest: &'a PackManifest) -> Self {
        Self {
            display_name: &manifest.display_name,
            version: &manifest.version,
            author: &manifest.author,
            license: &manifest.license,
            description: manifest.description.as_deref().unwrap_or_default(),
        }
    }
}

/// Semantic Versioning 2.0.0: `MAJOR.MINOR.PATCH[-pre.release][+build.metadata]`.
fn is_valid_semver(version: &str) -> bool {
    fn identifiers(text: &str, numeric_rules: bool) -> bool {
        text.split('.').all(|id| {
            !id.is_empty()
                && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && !(numeric_rules
                    && id.len() > 1
                    && id.starts_with('0')
                    && id.bytes().all(|b| b.is_ascii_digit()))
        })
    }
    let (rest, build) = version
        .split_once('+')
        .map_or((version, None), |(r, b)| (r, Some(b)));
    let (core, pre) = rest
        .split_once('-')
        .map_or((rest, None), |(c, p)| (c, Some(p)));
    let numbers: Vec<&str> = core.split('.').collect();
    numbers.len() == 3
        && numbers.iter().all(|n| {
            !n.is_empty()
                && n.bytes().all(|b| b.is_ascii_digit())
                && (n.len() == 1 || !n.starts_with('0'))
        })
        && pre.is_none_or(|p| identifiers(p, true))
        && build.is_none_or(|b| identifiers(b, false))
}

/// Semantic Versioning precedence of `a` against `b`; `None` if either is not a semver.
/// Build metadata is ignored, and a pre-release sorts before its release.
pub(crate) fn compare_versions(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering;
    // Numeric identifiers compare as numbers and sort before alphanumeric ones.
    fn identifier(a: &str, b: &str) -> Ordering {
        match (a.parse::<u64>(), b.parse::<u64>()) {
            (Ok(x), Ok(y)) => x.cmp(&y),
            (Ok(_), Err(_)) => Ordering::Less,
            (Err(_), Ok(_)) => Ordering::Greater,
            _ => a.cmp(b),
        }
    }
    fn parts(version: &str) -> (&str, Option<&str>) {
        let rest = version.split('+').next().unwrap_or_default();
        rest.split_once('-')
            .map_or((rest, None), |(core, pre)| (core, Some(pre)))
    }
    if !is_valid_semver(a) || !is_valid_semver(b) {
        return None;
    }
    let ((core_a, pre_a), (core_b, pre_b)) = (parts(a), parts(b));
    let order = core_a
        .split('.')
        .zip(core_b.split('.'))
        .map(|(x, y)| identifier(x, y))
        .find(|order| order.is_ne())
        .unwrap_or(Ordering::Equal);
    Some(order.then_with(|| {
        match (pre_a, pre_b) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
            (Some(x), Some(y)) => x
                .split('.')
                .zip(y.split('.'))
                .map(|(p, q)| identifier(p, q))
                .find(|order| order.is_ne())
                .unwrap_or_else(|| x.split('.').count().cmp(&y.split('.').count())),
        }
    }))
}

/// Next release for `part` (`patch`, `minor` or `major`), following npm's rules: a
/// pre-release of the target version is released rather than skipped, and build
/// metadata is dropped.
pub(crate) fn bump_version(version: &str, part: &str) -> Result<String, String> {
    if !is_valid_semver(version) {
        return Err(format!("'{version}' is not a semantic version"));
    }
    let release = version.split(['-', '+']).next().unwrap_or_default();
    let pre = version.split('+').next().unwrap_or_default().contains('-');
    let mut numbers = [0u64; 3];
    for (slot, text) in numbers.iter_mut().zip(release.split('.')) {
        *slot = text.parse().map_err(|_| "Version number is too large")?;
    }
    let [major, minor, patch] = numbers;
    let next = |n: u64| n.checked_add(1).ok_or("Version number is too large");
    Ok(match part {
        "patch" if pre => format!("{major}.{minor}.{patch}"),
        "patch" => format!("{major}.{minor}.{}", next(patch)?),
        "minor" if pre && patch == 0 => format!("{major}.{minor}.0"),
        "minor" => format!("{major}.{}.0", next(minor)?),
        "major" if pre && minor == 0 && patch == 0 => format!("{major}.0.0"),
        "major" => format!("{}.0.0", next(major)?),
        _ => return Err(format!("Unknown version bump '{part}'")),
    })
}

/// Rewrite `source` with new settings. Tags and keys this crate does not read are kept,
/// so an older editor never strips metadata a newer one wrote.
pub(crate) fn rewrite_manifest(source: &str, settings: &PackSettings) -> Result<String, String> {
    let original = PackManifest::from_str(source).map_err(|e| e.to_string())?;
    let mut rest: toml::Table = toml::from_str(source).map_err(|e| e.to_string())?;
    for key in [
        "pack_id",
        "schema_version",
        "display_name",
        "version",
        "author",
        "license",
        "description",
    ] {
        rest.remove(key);
    }
    let mut text = manifest_toml_with(
        &original.pack_id,
        settings.display_name,
        settings.version,
        settings.author,
        settings.license,
        settings.description,
    );
    text.push_str(&toml::to_string(&rest).map_err(|e| e.to_string())?);
    PackManifest::from_str(&text).map_err(|e| e.to_string())?;
    Ok(text)
}

/// Shared TOML writer for pack creation and asset publication.
pub(crate) fn manifest_toml(
    id: &str,
    name: &str,
    version: &str,
    author: &str,
    license: &str,
) -> String {
    manifest_toml_with(id, name, version, author, license, "")
}

fn manifest_toml_with(
    id: &str,
    name: &str,
    version: &str,
    author: &str,
    license: &str,
    description: &str,
) -> String {
    format!(
        "pack_id = {}\nschema_version = {}\ndisplay_name = {}\nversion = {}\nauthor = {}\nlicense = {}\ndescription = {}\n",
        toml_string(id),
        CURRENT_SCHEMA_VERSION,
        toml_string(name),
        toml_string(version),
        toml_string(author),
        toml_string(license),
        toml_string(description)
    )
}

/// Escape all TOML basic-string control characters without changing authored text.
pub(crate) fn toml_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\u{08}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{0c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            ch if ch.is_control() => {
                use std::fmt::Write as _;
                let _ = write!(out, "\\u{:04X}", ch as u32);
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
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
    fn authoring_and_export_share_lossless_pack_escaping() {
        let name = "Quoted \"pack\"\nwith\ttabs";
        let author = "Backslash \\ and carriage\rreturn";
        let parsed =
            PackManifest::from_str(&manifest_toml("test-pack", name, "0.1.0", author, "CC0"))
                .unwrap();
        assert_eq!(parsed.display_name, name);
        assert_eq!(parsed.author, author);
        assert!(
            PackManifest::from_str(&manifest_toml("../escape", name, "0.1.0", author, "CC0"))
                .is_err()
        );
    }

    #[test]
    fn versions_follow_semver_and_bump_like_npm() {
        for valid in [
            "0.1.0",
            "1.0.0-alpha",
            "1.0.0-alpha.1-x",
            "1.0.0+20260101",
            "2.0.0-rc.1+b-7",
        ] {
            assert!(is_valid_semver(valid), "{valid}");
        }
        for invalid in [
            "",
            "1",
            "1.0",
            "01.0.0",
            "1.0.0-",
            "1.0.0-01",
            "1.0.0+",
            "1.0.0-a..b",
            "v1.0.0",
            "1.0.0 ",
        ] {
            assert!(!is_valid_semver(invalid), "{invalid}");
        }
        let toml = VALID_PACK_TOML.replace("\"1.0.0\"", "\"1.0\"");
        assert!(PackManifest::from_str(&toml).is_err());
        for (from, part, to) in [
            ("1.2.3", "patch", "1.2.4"),
            ("1.2.3", "minor", "1.3.0"),
            ("1.2.3+build", "major", "2.0.0"),
            ("1.2.3-rc.1", "patch", "1.2.3"),
            ("1.2.0-rc.1", "minor", "1.2.0"),
            ("1.2.3-rc.1", "minor", "1.3.0"),
            ("2.0.0-rc.1", "major", "2.0.0"),
        ] {
            assert_eq!(bump_version(from, part).unwrap(), to, "{from} {part}");
        }
        // Precedence example from the Semantic Versioning 2.0.0 specification.
        let ordered = [
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
            "1.0.1",
            "1.2.0",
            "10.0.0",
        ];
        for pair in ordered.windows(2) {
            assert_eq!(
                compare_versions(pair[0], pair[1]),
                Some(std::cmp::Ordering::Less),
                "{pair:?}"
            );
        }
        assert_eq!(
            compare_versions("1.0.0+a", "1.0.0+b"),
            Some(std::cmp::Ordering::Equal)
        );
        assert_eq!(compare_versions("1.0", "1.0.0"), None);
        assert!(bump_version("1.0", "patch").is_err());
        assert!(bump_version("1.0.0", "build").is_err());
        assert!(bump_version("1.0.18446744073709551615", "patch").is_err());
    }

    #[test]
    fn rewrite_keeps_identity_tags_and_unknown_keys() {
        let source = format!("{VALID_PACK_TOML}future = {{ kept = true }}\n");
        let original = PackManifest::from_str(&source).unwrap();
        let settings = PackSettings {
            description: "New \"quoted\" text",
            version: "1.1.0",
            ..PackSettings::of(&original)
        };
        let text = rewrite_manifest(&source, &settings).unwrap();
        let parsed = PackManifest::from_str(&text).unwrap();
        assert_eq!(parsed.pack_id, "kenney-city-pack");
        assert_eq!(parsed.version, "1.1.0");
        assert_eq!(parsed.description.as_deref(), Some("New \"quoted\" text"));
        assert_eq!(parsed.tags, original.tags);
        assert!(text.contains("kept = true"));
        let bad = PackSettings {
            version: "next",
            ..PackSettings::of(&original)
        };
        assert!(rewrite_manifest(&source, &bad).is_err());
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
