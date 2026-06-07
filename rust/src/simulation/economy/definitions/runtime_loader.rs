//! Cached loading of shipped runtime economy tuning and catalog data.

use super::io::{PROFILES_FILE, parse_toml_file};
use super::runtime::{RuntimeEconomyCatalog, RuntimeEconomyTuning};
use super::runtime_compile::compile_runtime_catalog;
use super::schema::ProfilesFile;
use super::validation::validate_runtime_tuning;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

static BUILTIN_RUNTIME_TUNING: OnceLock<Result<Arc<RuntimeEconomyTuning>, String>> =
    OnceLock::new();
static BUILTIN_RUNTIME_CATALOG: OnceLock<Result<Arc<RuntimeEconomyCatalog>, String>> =
    OnceLock::new();

/// Loads the shipped economy-side runtime tuning from `economy/profiles.toml`.
pub(crate) fn load_runtime_economy_tuning() -> Result<Arc<RuntimeEconomyTuning>, String> {
    match BUILTIN_RUNTIME_TUNING
        .get_or_init(|| load_runtime_economy_tuning_from_disk().map(Arc::new))
    {
        Ok(config) => Ok(Arc::clone(config)),
        Err(err) => Err(err.clone()),
    }
}

fn load_runtime_economy_tuning_from_disk() -> Result<RuntimeEconomyTuning, String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("economy")
        .join(PROFILES_FILE);
    let profiles: ProfilesFile = parse_toml_file(&path)?;
    validate_runtime_tuning(&profiles.runtime_tuning)?;
    Ok(profiles.runtime_tuning)
}

/// Loads the shipped compiled runtime economy catalog from `economy/profiles.toml`.
pub(crate) fn load_runtime_economy_catalog() -> Result<Arc<RuntimeEconomyCatalog>, String> {
    match BUILTIN_RUNTIME_CATALOG
        .get_or_init(|| load_runtime_economy_catalog_from_disk().map(Arc::new))
    {
        Ok(catalog) => Ok(Arc::clone(catalog)),
        Err(err) => Err(err.clone()),
    }
}

fn load_runtime_economy_catalog_from_disk() -> Result<RuntimeEconomyCatalog, String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("economy")
        .join(PROFILES_FILE);
    let profiles: ProfilesFile = parse_toml_file(&path)?;
    validate_runtime_tuning(&profiles.runtime_tuning)?;
    compile_runtime_catalog(&profiles.profiles, &profiles.runtime_tuning)
}
