// SPDX-License-Identifier: GPL-2.0-only

//! TOML file IO for authored economy projects.

use super::schema::{ControllersFile, EconomyProject, ProfilesFile, ScenariosFile};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub(super) const PROFILES_FILE: &str = "profiles.toml";
pub(super) const CONTROLLERS_FILE: &str = "controllers.toml";
pub(super) const SCENARIOS_FILE: &str = "scenarios.toml";
pub(super) const INDEX_FILE: &str = "economy.index.bin";

pub(super) fn load_project(dir_path: &Path) -> Result<EconomyProject, String> {
    let profiles: ProfilesFile = parse_toml_file(&dir_path.join(PROFILES_FILE))?;
    let controllers: ControllersFile = parse_toml_file(&dir_path.join(CONTROLLERS_FILE))?;
    let scenarios: ScenariosFile = parse_toml_file(&dir_path.join(SCENARIOS_FILE))?;
    Ok(EconomyProject {
        profiles: profiles.profiles,
        runtime_tuning: profiles.runtime_tuning,
        controllers: controllers.controllers,
        scenarios: scenarios.scenarios,
    })
}
pub(super) fn parse_toml_file<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|err| format!("could not read '{}': {err}", path.display()))?;
    toml::from_str(&content).map_err(|err| format!("could not parse '{}': {err}", path.display()))
}

pub(super) fn write_pretty_toml<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let encoded = toml::to_string_pretty(value)
        .map_err(|err| format!("could not encode '{}': {err}", path.display()))?;
    std::fs::write(path, encoded)
        .map_err(|err| format!("could not write '{}': {err}", path.display()))
}
