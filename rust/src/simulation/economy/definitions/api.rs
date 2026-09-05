// SPDX-License-Identifier: GPL-2.0-only

//! JSON bridge used by the economy editor and Godot-facing tooling.

use super::index::build_index;
use super::io::{
    CONTROLLERS_FILE, INDEX_FILE, PROFILES_FILE, SCENARIOS_FILE, load_project, write_pretty_toml,
};
use super::sandbox::run_sandbox;
use super::schema::{ControllersFile, EconomyProject, ProfilesFile, ScenariosFile};
use super::validation::validate_project;
use std::path::Path;

/// Loads an authored economy project from the canonical `economy/` folder and
/// returns a JSON envelope containing the parsed project plus validation output.
pub fn load_project_json(dir_path: &Path) -> Result<String, String> {
    let project = load_project(dir_path)?;
    let validation = validate_project(&project);
    let payload = serde_json::json!({
        "ok": true,
        "source_dir": dir_path,
        "project": project,
        "validation": validation,
    });
    serde_json::to_string(&payload)
        .map_err(|err| format!("could not encode economy project JSON: {err}"))
}

/// Validates and exports an authored economy project back to canonical TOML
/// files and regenerates the derived `economy.index.bin` cache.
pub fn export_project_json(project_json: &str, dir_path: &Path) -> Result<String, String> {
    let project: EconomyProject = serde_json::from_str(project_json)
        .map_err(|err| format!("economy project JSON parse error: {err}"))?;
    let validation = validate_project(&project);
    if validation.iter().any(|msg| msg.is_error()) {
        let payload = serde_json::json!({
            "ok": false,
            "error": "validation failed; export aborted",
            "validation": validation,
        });
        return serde_json::to_string(&payload)
            .map_err(|err| format!("could not encode failed export JSON: {err}"));
    }

    std::fs::create_dir_all(dir_path).map_err(|err| {
        format!(
            "could not create economy dir '{}': {err}",
            dir_path.display()
        )
    })?;

    write_pretty_toml(
        &dir_path.join(PROFILES_FILE),
        &ProfilesFile {
            profiles: project.profiles.clone(),
            runtime_tuning: project.runtime_tuning.clone(),
        },
    )?;
    write_pretty_toml(
        &dir_path.join(CONTROLLERS_FILE),
        &ControllersFile {
            controllers: project.controllers.clone(),
        },
    )?;
    write_pretty_toml(
        &dir_path.join(SCENARIOS_FILE),
        &ScenariosFile {
            scenarios: project.scenarios.clone(),
        },
    )?;

    let compiled = build_index(&project);
    let compiled_bytes = serde_json::to_vec(&compiled)
        .map_err(|err| format!("could not encode economy cache: {err}"))?;
    std::fs::write(dir_path.join(INDEX_FILE), compiled_bytes)
        .map_err(|err| format!("could not write economy cache: {err}"))?;

    let payload = serde_json::json!({
        "ok": true,
        "validation": validation,
        "cache_path": dir_path.join(INDEX_FILE),
    });
    serde_json::to_string(&payload).map_err(|err| format!("could not encode export JSON: {err}"))
}

/// Runs the small authored-economy sandbox for a selected scenario and returns
/// a JSON envelope with summary metrics and daily series data.
pub fn run_sandbox_json(project_json: &str, scenario_id: &str) -> Result<String, String> {
    let project: EconomyProject = serde_json::from_str(project_json)
        .map_err(|err| format!("economy project JSON parse error: {err}"))?;
    let validation = validate_project(&project);
    if validation.iter().any(|msg| msg.is_error()) {
        let payload = serde_json::json!({
            "ok": false,
            "error": "validation failed; sandbox aborted",
            "validation": validation,
        });
        return serde_json::to_string(&payload)
            .map_err(|err| format!("could not encode failed sandbox JSON: {err}"));
    }

    let result = run_sandbox(&project, scenario_id)?;
    let payload = serde_json::json!({
        "ok": true,
        "validation": validation,
        "result": result,
    });
    serde_json::to_string(&payload).map_err(|err| format!("could not encode sandbox JSON: {err}"))
}
