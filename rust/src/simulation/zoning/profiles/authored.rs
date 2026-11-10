// SPDX-License-Identifier: GPL-2.0-only

//! Authored zoning-profile TOML loading.

use serde::Deserialize;

// Compiled in: a shipped game has no repository checkout to read `zoning/profiles.toml` from.
const PROFILES_TOML: &str = include_str!("../../../../../zoning/profiles.toml");

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
    let authored: ProfilesFile = toml::from_str(PROFILES_TOML)
        .map_err(|err| format!("could not parse 'zoning/profiles.toml': {err}"))?;
    Ok(authored.profiles)
}
