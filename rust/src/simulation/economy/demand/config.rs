// SPDX-License-Identifier: GPL-2.0-only

//! Demand tuning loading and validation.

use super::types::{EPSILON, UseTuningF32};
use crate::simulation::economy::households::HouseholdExitPolicy;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

const GROWTH_PROFILES_FILE: &str = "demand/growth_profiles.toml";
#[derive(Clone, Debug)]
pub(super) struct HouseholdActionConfig {
    pub(super) move_in_benefit_treasury_coverage_days: f32,
    pub(super) exit_policy: HouseholdExitPolicy,
    pub(super) persistent_exit_daily_fraction: f32,
}

#[derive(Clone, Debug)]
pub(super) struct ActionBudgetConfig {
    pub(super) max_household_removals_per_day: u32,
    pub(super) upgrade_batch_fraction_by_use: UseTuningF32,
    pub(super) downgrade_batch_fraction_by_use: UseTuningF32,
    pub(super) despawn_batch_fraction_by_use: UseTuningF32,
}

#[derive(Clone, Debug)]
pub(super) struct DemandConfig {
    pub(super) household_action: HouseholdActionConfig,
    pub(super) action_budget: ActionBudgetConfig,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoredDemandConfig {
    household_action: AuthoredHouseholdAction,
    action_budget: AuthoredActionBudget,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoredHouseholdAction {
    move_in_benefit_treasury_coverage_days: f32,
    persistent_exit_destitute_stock_days: f32,
    persistent_exit_destitute_unhoused_days: u32,
    persistent_exit_max_unhoused_days: u32,
    persistent_exit_daily_fraction: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoredActionBudget {
    max_household_removals_per_day: u32,
    upgrade_batch_fraction_by_use: AuthoredUseTuningF32,
    downgrade_batch_fraction_by_use: AuthoredUseTuningF32,
    despawn_batch_fraction_by_use: AuthoredUseTuningF32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoredUseTuningF32 {
    residential: f32,
    commercial: f32,
    industrial: f32,
}

static BUILTIN_CONFIG: OnceLock<Result<Arc<DemandConfig>, String>> = OnceLock::new();

pub(super) fn load_builtin_demand_config() -> Result<Arc<DemandConfig>, String> {
    match BUILTIN_CONFIG.get_or_init(load_config_from_disk) {
        Ok(config) => Ok(Arc::clone(config)),
        Err(err) => Err(err.clone()),
    }
}

fn load_config_from_disk() -> Result<Arc<DemandConfig>, String> {
    let path = repo_relative_path(GROWTH_PROFILES_FILE);
    let content = std::fs::read_to_string(&path)
        .map_err(|err| format!("could not read '{}': {err}", path.display()))?;
    let authored: AuthoredDemandConfig = toml::from_str(&content)
        .map_err(|err| format!("could not parse '{}': {err}", path.display()))?;
    compile_config(authored).map(Arc::new)
}

fn repo_relative_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(relative)
}

fn compile_config(authored: AuthoredDemandConfig) -> Result<DemandConfig, String> {
    validate_positive_f32(
        authored
            .household_action
            .move_in_benefit_treasury_coverage_days,
        "household_action.move_in_benefit_treasury_coverage_days",
    )?;
    validate_range_f32(
        authored
            .household_action
            .persistent_exit_destitute_stock_days,
        0.0,
        365.0,
        "household_action.persistent_exit_destitute_stock_days",
    )?;
    if authored
        .household_action
        .persistent_exit_destitute_unhoused_days
        == 0
    {
        return Err(
            "household_action.persistent_exit_destitute_unhoused_days must be >= 1".to_owned(),
        );
    }
    if authored.household_action.persistent_exit_max_unhoused_days == 0 {
        return Err("household_action.persistent_exit_max_unhoused_days must be >= 1".to_owned());
    }
    validate_range_f32(
        authored.household_action.persistent_exit_daily_fraction,
        0.0,
        1.0,
        "household_action.persistent_exit_daily_fraction",
    )?;

    let upgrade_batch_fraction_by_use = validate_use_tuning(
        authored.action_budget.upgrade_batch_fraction_by_use,
        0.0,
        1.0,
        "action_budget.upgrade_batch_fraction_by_use",
    )?;
    let downgrade_batch_fraction_by_use = validate_use_tuning(
        authored.action_budget.downgrade_batch_fraction_by_use,
        0.0,
        1.0,
        "action_budget.downgrade_batch_fraction_by_use",
    )?;
    let despawn_batch_fraction_by_use = validate_use_tuning(
        authored.action_budget.despawn_batch_fraction_by_use,
        0.0,
        1.0,
        "action_budget.despawn_batch_fraction_by_use",
    )?;

    Ok(DemandConfig {
        household_action: HouseholdActionConfig {
            move_in_benefit_treasury_coverage_days: authored
                .household_action
                .move_in_benefit_treasury_coverage_days,
            exit_policy: HouseholdExitPolicy {
                destitute_stock_days: authored
                    .household_action
                    .persistent_exit_destitute_stock_days,
                destitute_unhoused_days: authored
                    .household_action
                    .persistent_exit_destitute_unhoused_days,
                max_unhoused_days: authored.household_action.persistent_exit_max_unhoused_days,
            },
            persistent_exit_daily_fraction: authored
                .household_action
                .persistent_exit_daily_fraction,
        },
        action_budget: ActionBudgetConfig {
            max_household_removals_per_day: authored.action_budget.max_household_removals_per_day,
            upgrade_batch_fraction_by_use,
            downgrade_batch_fraction_by_use,
            despawn_batch_fraction_by_use,
        },
    })
}

fn validate_use_tuning(
    authored: AuthoredUseTuningF32,
    min_value: f32,
    max_value: f32,
    label: &str,
) -> Result<UseTuningF32, String> {
    validate_range_f32(
        authored.residential,
        min_value,
        max_value,
        &format!("{label}.residential"),
    )?;
    validate_range_f32(
        authored.commercial,
        min_value,
        max_value,
        &format!("{label}.commercial"),
    )?;
    validate_range_f32(
        authored.industrial,
        min_value,
        max_value,
        &format!("{label}.industrial"),
    )?;
    Ok(UseTuningF32 {
        residential: authored.residential,
        commercial: authored.commercial,
        industrial: authored.industrial,
    })
}

pub(super) fn validate_positive_f32(value: f32, label: &str) -> Result<(), String> {
    validate_range_f32(value, EPSILON, f32::INFINITY, label)
}

pub(super) fn validate_range_f32(
    value: f32,
    min_value: f32,
    max_value: f32,
    label: &str,
) -> Result<(), String> {
    if !value.is_finite() || value < min_value || value > max_value {
        Err(format!(
            "{label} must be finite and in [{}..={}]",
            min_value, max_value
        ))
    } else {
        Ok(())
    }
}
