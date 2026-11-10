// SPDX-License-Identifier: GPL-2.0-only

//! Cached loading of shipped runtime economy tuning and catalog data.

use super::io::parse_shipped_profiles;
use super::runtime::{RuntimeEconomyCatalog, RuntimeEconomyTuning};
use super::runtime_compile::compile_runtime_catalog;
#[cfg(test)]
use super::schema::ProfilesFile;
use super::validation::validate_runtime_tuning;
use std::sync::{Arc, OnceLock};

struct RuntimeDefinitions {
    tuning: Arc<RuntimeEconomyTuning>,
    catalog: Arc<RuntimeEconomyCatalog>,
}

static BUILTIN_RUNTIME: OnceLock<Result<RuntimeDefinitions, String>> = OnceLock::new();

/// Compiles an isolated authored catalog for market tests without changing cached definitions.
#[cfg(test)]
pub(crate) fn compile_test_catalog(source: &str) -> Result<RuntimeEconomyCatalog, String> {
    let profiles: ProfilesFile = toml::from_str(source).map_err(|error| error.to_string())?;
    validate_runtime_tuning(&profiles.runtime_tuning)?;
    compile_runtime_catalog(
        &profiles.profiles,
        &profiles.resources,
        &profiles.runtime_tuning,
    )
}

/// Loads the shipped economy-side runtime tuning from `economy/profiles.toml`.
pub(crate) fn load_runtime_economy_tuning() -> Result<Arc<RuntimeEconomyTuning>, String> {
    Ok(Arc::clone(&runtime_definitions()?.tuning))
}

/// Loads the shipped compiled runtime economy catalog from `economy/profiles.toml`.
pub(crate) fn load_runtime_economy_catalog() -> Result<Arc<RuntimeEconomyCatalog>, String> {
    Ok(Arc::clone(&runtime_definitions()?.catalog))
}

fn runtime_definitions() -> Result<&'static RuntimeDefinitions, String> {
    match BUILTIN_RUNTIME.get_or_init(load_runtime_definitions) {
        Ok(definitions) => Ok(definitions),
        Err(err) => Err(err.clone()),
    }
}

fn load_runtime_definitions() -> Result<RuntimeDefinitions, String> {
    let profiles = parse_shipped_profiles()?;
    validate_runtime_tuning(&profiles.runtime_tuning)?;
    let catalog = compile_runtime_catalog(
        &profiles.profiles,
        &profiles.resources,
        &profiles.runtime_tuning,
    )?;
    Ok(RuntimeDefinitions {
        tuning: Arc::new(profiles.runtime_tuning),
        catalog: Arc::new(catalog),
    })
}
