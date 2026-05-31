//! Authored zoning-profile TOML loading.

use serde::Deserialize;
use std::collections::HashSet;
use std::path::PathBuf;

const PROFILES_FILE: &str = "zoning/profiles.toml";
const GROWTH_PROFILES_FILE: &str = "demand/growth_profiles.toml";

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
    pub(super) growth_profile_id: String,
    pub(super) ui: AuthoredZoneProfileUi,
}

#[derive(Deserialize)]
pub(super) struct AuthoredZoneProfileUi {
    pub(super) color: String,
    pub(super) icon: String,
    pub(super) description: String,
}

#[derive(Deserialize)]
struct GrowthProfilesFile {
    #[serde(default)]
    profiles: Vec<AuthoredGrowthProfile>,
}

#[derive(Deserialize)]
struct AuthoredGrowthProfile {
    id: String,
}

pub(super) fn load_authored_zone_profiles() -> Result<Vec<AuthoredZoneProfile>, String> {
    let profiles_path = repo_relative_path(PROFILES_FILE);
    let content = std::fs::read_to_string(&profiles_path)
        .map_err(|err| format!("could not read '{}': {err}", profiles_path.display()))?;
    let authored: ProfilesFile = toml::from_str(&content)
        .map_err(|err| format!("could not parse '{}': {err}", profiles_path.display()))?;
    Ok(authored.profiles)
}

pub(super) fn load_builtin_growth_profile_ids() -> Result<HashSet<String>, String> {
    let growth_profiles_path = repo_relative_path(GROWTH_PROFILES_FILE);
    let content = std::fs::read_to_string(&growth_profiles_path)
        .map_err(|err| format!("could not read '{}': {err}", growth_profiles_path.display()))?;
    let authored: GrowthProfilesFile = toml::from_str(&content).map_err(|err| {
        format!(
            "could not parse '{}': {err}",
            growth_profiles_path.display()
        )
    })?;
    Ok(authored
        .profiles
        .into_iter()
        .map(|profile| profile.id)
        .collect())
}

fn repo_relative_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(relative)
}
