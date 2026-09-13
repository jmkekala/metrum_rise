// SPDX-License-Identifier: GPL-2.0-only

//! Cached loading of shipped runtime economy tuning and catalog data.

use super::io::{PROFILES_FILE, parse_toml_file};
use super::runtime::{RuntimeEconomyCatalog, RuntimeEconomyTuning};
use super::runtime_compile::compile_runtime_catalog;
use super::schema::ProfilesFile;
use super::validation::validate_runtime_tuning;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

struct RuntimeDefinitions {
    tuning: Arc<RuntimeEconomyTuning>,
    catalog: Arc<RuntimeEconomyCatalog>,
}

static BUILTIN_RUNTIME: OnceLock<Result<RuntimeDefinitions, String>> = OnceLock::new();

/// Loads the shipped economy-side runtime tuning from `economy/profiles.toml`.
pub(crate) fn load_runtime_economy_tuning() -> Result<Arc<RuntimeEconomyTuning>, String> {
    Ok(Arc::clone(&runtime_definitions()?.tuning))
}

/// Loads the shipped compiled runtime economy catalog from `economy/profiles.toml`.
pub(crate) fn load_runtime_economy_catalog() -> Result<Arc<RuntimeEconomyCatalog>, String> {
    Ok(Arc::clone(&runtime_definitions()?.catalog))
}

fn runtime_definitions() -> Result<&'static RuntimeDefinitions, String> {
    match BUILTIN_RUNTIME.get_or_init(load_runtime_definitions_from_disk) {
        Ok(definitions) => Ok(definitions),
        Err(err) => Err(err.clone()),
    }
}

fn load_runtime_definitions_from_disk() -> Result<RuntimeDefinitions, String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("economy")
        .join(PROFILES_FILE);
    let profiles: ProfilesFile = parse_toml_file(&path)?;
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
