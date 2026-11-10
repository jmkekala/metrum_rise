// SPDX-License-Identifier: GPL-2.0-only

//! The shipped authored economy profiles.

use super::schema::ProfilesFile;

// Compiled in: a shipped game has no repository checkout to read `economy/profiles.toml` from.
const PROFILES_TOML: &str = include_str!("../../../../../economy/profiles.toml");

pub(super) fn parse_shipped_profiles() -> Result<ProfilesFile, String> {
    toml::from_str(PROFILES_TOML)
        .map_err(|err| format!("could not parse 'economy/profiles.toml': {err}"))
}
