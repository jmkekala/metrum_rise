// SPDX-License-Identifier: GPL-2.0-only

//! Project-level authored economy validation orchestration.

use super::common::{duplicate_ids, validate_range};
use super::messages::{ValidationMessage, error};
use super::runtime_tuning::validate_runtime_tuning;
use super::scenario::validate_scenario;
use crate::simulation::economy::definitions::runtime::RuntimeEconomyTuning;
use crate::simulation::economy::definitions::runtime_compile::compile_runtime_catalog;
use crate::simulation::economy::definitions::schema::{
    EconomyController, EconomyProfile, EconomyProject,
};
use std::collections::BTreeMap;

pub(in crate::simulation::economy::definitions) fn validate_project(
    project: &EconomyProject,
) -> Vec<ValidationMessage> {
    let mut messages = Vec::new();
    let profile_ids = duplicate_ids(project.profiles.iter().map(|profile| profile.id.as_str()));
    for duplicate in profile_ids {
        messages.push(error(
            "duplicate_profile_id",
            "project.profiles",
            format!("profile id '{duplicate}' is defined more than once"),
        ));
    }

    let controller_ids = duplicate_ids(
        project
            .controllers
            .iter()
            .map(|controller| controller.id.as_str()),
    );
    for duplicate in controller_ids {
        messages.push(error(
            "duplicate_controller_id",
            "project.controllers",
            format!("controller id '{duplicate}' is defined more than once"),
        ));
    }

    let scenario_ids = duplicate_ids(
        project
            .scenarios
            .iter()
            .map(|scenario| scenario.id.as_str()),
    );
    for duplicate in scenario_ids {
        messages.push(error(
            "duplicate_scenario_id",
            "project.scenarios",
            format!("scenario id '{duplicate}' is defined more than once"),
        ));
    }

    let profile_map: BTreeMap<&str, &EconomyProfile> = project
        .profiles
        .iter()
        .map(|profile| (profile.id.as_str(), profile))
        .collect();
    let controller_map: BTreeMap<&str, &EconomyController> = project
        .controllers
        .iter()
        .map(|controller| (controller.id.as_str(), controller))
        .collect();

    for profile in &project.profiles {
        if profile.display_name.trim().is_empty() {
            messages.push(error(
                "missing_profile_display_name",
                format!("profile.{}", profile.id),
                "profile display_name must not be empty".to_owned(),
            ));
        }
        if profile.kind.trim().is_empty() {
            messages.push(error(
                "missing_profile_kind",
                format!("profile.{}", profile.id),
                "profile kind must not be empty".to_owned(),
            ));
        }
    }
    validate_runtime_tuning_messages(&project.runtime_tuning, &mut messages);

    for controller in &project.controllers {
        for (field, value, minimum, maximum) in [
            ("default_weight", controller.default_weight, 0.0, 1.0),
            ("min_multiplier", controller.min_multiplier, 0.0, f32::MAX),
            (
                "max_multiplier",
                controller.max_multiplier,
                controller.min_multiplier,
                f32::MAX,
            ),
        ] {
            if let Err(message) = validate_range(value, minimum, maximum, field) {
                messages.push(error(
                    "invalid_controller_parameter",
                    format!("controller.{}.{}", controller.id, field),
                    message,
                ));
            }
        }
        if controller.display_name.trim().is_empty() {
            messages.push(error(
                "missing_controller_display_name",
                format!("controller.{}", controller.id),
                "controller display_name must not be empty".to_owned(),
            ));
        }
        if controller.kind.trim().is_empty() {
            messages.push(error(
                "missing_controller_kind",
                format!("controller.{}", controller.id),
                "controller kind must not be empty".to_owned(),
            ));
        }
    }

    for scenario in &project.scenarios {
        for resource in &scenario.owa_import_resources {
            if !project.resources.iter().any(|entry| &entry.id == resource) {
                messages.push(error(
                    "unknown_import_resource",
                    format!("scenario.{}", scenario.id),
                    format!("OWA import resource '{resource}' is not declared"),
                ));
            }
        }
        validate_scenario(scenario, &profile_map, &controller_map, &mut messages);
    }

    if let Err(err) = compile_runtime_catalog(
        &project.profiles,
        &project.resources,
        &project.runtime_tuning,
    ) {
        messages.push(error("invalid_runtime_catalog", "project.profiles", err));
    }

    messages
}

fn validate_runtime_tuning_messages(
    tuning: &RuntimeEconomyTuning,
    messages: &mut Vec<ValidationMessage>,
) {
    if let Err(err) = validate_runtime_tuning(tuning) {
        messages.push(error(
            "invalid_runtime_tuning",
            "project.runtime_tuning",
            err,
        ));
    }
}
