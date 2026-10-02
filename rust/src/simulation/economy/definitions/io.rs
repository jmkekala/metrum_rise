// SPDX-License-Identifier: GPL-2.0-only

//! TOML file IO for authored economy profiles.

use serde::Deserialize;
use std::path::Path;

pub(super) const PROFILES_FILE: &str = "profiles.toml";

pub(super) fn parse_toml_file<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|err| format!("could not read '{}': {err}", path.display()))?;
    toml::from_str(&content).map_err(|err| format!("could not parse '{}': {err}", path.display()))
}
