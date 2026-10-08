// SPDX-License-Identifier: GPL-2.0-only

//! Authored zoning-profile TOML loading.

use serde::Deserialize;
use std::path::PathBuf;

const PROFILES_FILE: &str = "zoning/profiles.toml";

#[derive(Deserialize)]
struct ProfilesFile {
    #[serde(default)]
    profiles: Vec<AuthoredZoneProfile>,
}

#[derive(Deserialize)]
pub(super) struct AuthoredZoneProfile {
    pub(super) id: String,
    pub(super) display_name: String,
    pub(super) ui_order: u32,
    pub(super) zone_type: String,
    pub(super) density: String,
    #[serde(default)]
    pub(super) required_asset_tags: Vec<String>,
    pub(super) ui: AuthoredZoneProfileUi,
}

#[derive(Deserialize)]
pub(super) struct AuthoredZoneProfileUi {
    pub(super) color: String,
    pub(super) icon: String,
    pub(super) description: String,
}

pub(super) fn load_authored_zone_profiles() -> Result<Vec<AuthoredZoneProfile>, String> {
    let profiles_path = repo_relative_path(PROFILES_FILE);
    let content = std::fs::read_to_string(&profiles_path)
        .map_err(|err| format!("could not read '{}': {err}", profiles_path.display()))?;
    let authored: ProfilesFile = toml::from_str(&content)
        .map_err(|err| format!("could not parse '{}': {err}", profiles_path.display()))?;
    Ok(authored.profiles)
}

fn repo_relative_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(relative)
}
