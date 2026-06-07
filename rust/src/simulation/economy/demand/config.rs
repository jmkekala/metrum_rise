//! Demand tuning loading and validation.

use super::types::{DemandChannel, EPSILON, GrowthProfileRuntime, UseTuningF32};
use crate::simulation::zoning::ZoneType;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

const GROWTH_PROFILES_FILE: &str = "demand/growth_profiles.toml";
const SHIPPED_PROFILE_ORDER: [(&str, DemandChannel); 9] = [
    ("residential_low_default", DemandChannel::ResidentialGrowth),
    (
        "residential_medium_default",
        DemandChannel::ResidentialGrowth,
    ),
    ("residential_high_default", DemandChannel::ResidentialGrowth),
    ("commercial_low_default", DemandChannel::CommercialGrowth),
    ("commercial_medium_default", DemandChannel::CommercialGrowth),
    ("commercial_high_default", DemandChannel::CommercialGrowth),
    ("industrial_low_default", DemandChannel::IndustrialGrowth),
    ("industrial_medium_default", DemandChannel::IndustrialGrowth),
    ("industrial_high_default", DemandChannel::IndustrialGrowth),
];

#[derive(Clone, Debug)]
pub(super) struct SignalNormalizationConfig {
    pub(super) household_affordability_target_reserve_days: f32,
    pub(super) household_stock_stability_target_days: f32,
}

#[derive(Clone, Debug)]
pub(super) struct HouseholdActionConfig {
    pub(super) admission_threshold: f32,
    pub(super) admission_unhoused_ratio_penalty: f32,
    pub(super) admission_zero_budget_penalty: f32,
    pub(super) admission_recent_failure_penalty: f32,
    pub(super) move_in_min_search_runway_days: f32,
    pub(super) move_in_target_search_runway_days: f32,
    pub(super) move_in_benefit_treasury_coverage_days: f32,
    pub(super) recent_failure_daily_decay: f32,
    pub(super) removal_threshold: f32,
    pub(super) persistent_exit_destitute_stock_days: f32,
    pub(super) persistent_exit_destitute_unhoused_days: u32,
    pub(super) persistent_exit_max_unhoused_days: u32,
    pub(super) persistent_exit_daily_fraction: f32,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(super) struct ActionBudgetConfig {
    pub(super) max_households_per_day: u32,
    pub(super) upgrade_batch_fraction_by_use: UseTuningF32,
    pub(super) downgrade_batch_fraction_by_use: UseTuningF32,
    pub(super) despawn_batch_fraction_by_use: UseTuningF32,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(super) struct DemandConfig {
    pub(super) signal_normalization: SignalNormalizationConfig,
    pub(super) household_action: HouseholdActionConfig,
    pub(super) action_budget: ActionBudgetConfig,
    pub(super) profiles: Vec<GrowthProfileRuntime>,
}

impl DemandConfig {
    pub(super) fn profile_for_zone_density(
        &self,
        zone_type: ZoneType,
        density: &str,
    ) -> Option<&GrowthProfileRuntime> {
        let idx = match (zone_type, density) {
            (ZoneType::Residential, "low") => 0,
            (ZoneType::Residential, "medium") => 1,
            (ZoneType::Residential, "high") => 2,
            (ZoneType::Commercial, "low") => 3,
            (ZoneType::Commercial, "medium") => 4,
            (ZoneType::Commercial, "high") => 5,
            (ZoneType::Industrial, "low") => 6,
            (ZoneType::Industrial, "medium") => 7,
            (ZoneType::Industrial, "high") => 8,
            _ => return None,
        };
        self.profiles.get(idx)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoredGrowthProfilesFile {
    signal_normalization: AuthoredSignalNormalization,
    household_action: AuthoredHouseholdAction,
    action_budget: AuthoredActionBudget,
    profiles: Vec<AuthoredGrowthProfile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoredSignalNormalization {
    household_affordability_target_reserve_days: f32,
    household_stock_stability_target_days: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoredHouseholdAction {
    admission_threshold: f32,
    admission_unhoused_ratio_penalty: f32,
    admission_zero_budget_penalty: f32,
    admission_recent_failure_penalty: f32,
    move_in_min_search_runway_days: f32,
    move_in_target_search_runway_days: f32,
    move_in_benefit_treasury_coverage_days: f32,
    recent_failure_daily_decay: f32,
    removal_threshold: f32,
    persistent_exit_destitute_stock_days: f32,
    persistent_exit_destitute_unhoused_days: u32,
    persistent_exit_max_unhoused_days: u32,
    persistent_exit_daily_fraction: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoredActionBudget {
    max_households_per_day: u32,
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoredGrowthProfile {
    id: String,
    demand_channel: String,
    spawn_threshold: f32,
    despawn_threshold: f32,
    upgrade_threshold: f32,
    downgrade_threshold: f32,
    hysteresis_margin: f32,
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
    let authored: AuthoredGrowthProfilesFile = toml::from_str(&content)
        .map_err(|err| format!("could not parse '{}': {err}", path.display()))?;
    compile_config(authored).map(Arc::new)
}

fn repo_relative_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(relative)
}

fn compile_config(authored: AuthoredGrowthProfilesFile) -> Result<DemandConfig, String> {
    validate_positive_f32(
        authored
            .signal_normalization
            .household_affordability_target_reserve_days,
        "signal_normalization.household_affordability_target_reserve_days",
    )?;
    validate_positive_f32(
        authored
            .signal_normalization
            .household_stock_stability_target_days,
        "signal_normalization.household_stock_stability_target_days",
    )?;

    validate_range_f32(
        authored.household_action.admission_threshold,
        0.0,
        1.0,
        "household_action.admission_threshold",
    )?;
    validate_range_f32(
        authored.household_action.admission_unhoused_ratio_penalty,
        0.0,
        1.0,
        "household_action.admission_unhoused_ratio_penalty",
    )?;
    validate_range_f32(
        authored.household_action.admission_zero_budget_penalty,
        0.0,
        1.0,
        "household_action.admission_zero_budget_penalty",
    )?;
    validate_range_f32(
        authored.household_action.admission_recent_failure_penalty,
        0.0,
        1.0,
        "household_action.admission_recent_failure_penalty",
    )?;
    validate_positive_f32(
        authored.household_action.move_in_min_search_runway_days,
        "household_action.move_in_min_search_runway_days",
    )?;
    validate_positive_f32(
        authored.household_action.move_in_target_search_runway_days,
        "household_action.move_in_target_search_runway_days",
    )?;
    if authored.household_action.move_in_target_search_runway_days
        <= authored.household_action.move_in_min_search_runway_days
    {
        return Err(
            "household_action.move_in_target_search_runway_days must be greater than move_in_min_search_runway_days"
                .to_owned(),
        );
    }
    validate_positive_f32(
        authored
            .household_action
            .move_in_benefit_treasury_coverage_days,
        "household_action.move_in_benefit_treasury_coverage_days",
    )?;
    validate_range_f32(
        authored.household_action.recent_failure_daily_decay,
        0.0,
        1.0,
        "household_action.recent_failure_daily_decay",
    )?;
    validate_range_f32(
        authored.household_action.removal_threshold,
        0.0,
        1.0,
        "household_action.removal_threshold",
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

    let mut by_id = std::collections::HashMap::new();
    for profile in authored.profiles {
        if by_id.contains_key(&profile.id) {
            return Err(format!("duplicate GrowthProfile id '{}'", profile.id));
        }
        let Some(demand_channel) = DemandChannel::from_str_name(&profile.demand_channel) else {
            return Err(format!(
                "unknown demand_channel '{}' for GrowthProfile '{}'",
                profile.demand_channel, profile.id
            ));
        };
        validate_range_f32(
            profile.spawn_threshold,
            0.0,
            1.0,
            &format!("profiles.{}.spawn_threshold", profile.id),
        )?;
        validate_range_f32(
            profile.despawn_threshold,
            0.0,
            1.0,
            &format!("profiles.{}.despawn_threshold", profile.id),
        )?;
        validate_range_f32(
            profile.upgrade_threshold,
            0.0,
            1.0,
            &format!("profiles.{}.upgrade_threshold", profile.id),
        )?;
        validate_range_f32(
            profile.downgrade_threshold,
            0.0,
            1.0,
            &format!("profiles.{}.downgrade_threshold", profile.id),
        )?;
        validate_range_f32(
            profile.hysteresis_margin,
            0.0,
            1.0,
            &format!("profiles.{}.hysteresis_margin", profile.id),
        )?;
        if profile.upgrade_threshold < profile.downgrade_threshold {
            return Err(format!(
                "profiles.{}.upgrade_threshold must be >= downgrade_threshold",
                profile.id
            ));
        }
        by_id.insert(
            profile.id.clone(),
            GrowthProfileRuntime {
                demand_channel,
                spawn_threshold: profile.spawn_threshold,
                despawn_threshold: profile.despawn_threshold,
                upgrade_threshold: profile.upgrade_threshold,
                downgrade_threshold: profile.downgrade_threshold,
            },
        );
    }

    if by_id.len() != SHIPPED_PROFILE_ORDER.len() {
        return Err(format!(
            "expected {} shipped GrowthProfiles, found {}",
            SHIPPED_PROFILE_ORDER.len(),
            by_id.len()
        ));
    }

    let mut profiles = Vec::with_capacity(SHIPPED_PROFILE_ORDER.len());
    for (id, expected_channel) in SHIPPED_PROFILE_ORDER {
        let Some(profile) = by_id.remove(id) else {
            return Err(format!("missing shipped GrowthProfile '{}'", id));
        };
        if profile.demand_channel != expected_channel {
            return Err(format!(
                "GrowthProfile '{}' must use demand_channel {:?}",
                id, expected_channel
            ));
        }
        profiles.push(profile);
    }
    if let Some(extra_id) = by_id.keys().next() {
        return Err(format!(
            "unexpected extra shipped GrowthProfile '{}'",
            extra_id
        ));
    }

    Ok(DemandConfig {
        signal_normalization: SignalNormalizationConfig {
            household_affordability_target_reserve_days: authored
                .signal_normalization
                .household_affordability_target_reserve_days,
            household_stock_stability_target_days: authored
                .signal_normalization
                .household_stock_stability_target_days,
        },

        household_action: HouseholdActionConfig {
            admission_threshold: authored.household_action.admission_threshold,
            admission_unhoused_ratio_penalty: authored
                .household_action
                .admission_unhoused_ratio_penalty,
            admission_zero_budget_penalty: authored.household_action.admission_zero_budget_penalty,
            admission_recent_failure_penalty: authored
                .household_action
                .admission_recent_failure_penalty,
            move_in_min_search_runway_days: authored
                .household_action
                .move_in_min_search_runway_days,
            move_in_target_search_runway_days: authored
                .household_action
                .move_in_target_search_runway_days,
            move_in_benefit_treasury_coverage_days: authored
                .household_action
                .move_in_benefit_treasury_coverage_days,
            recent_failure_daily_decay: authored.household_action.recent_failure_daily_decay,
            removal_threshold: authored.household_action.removal_threshold,
            persistent_exit_destitute_stock_days: authored
                .household_action
                .persistent_exit_destitute_stock_days,
            persistent_exit_destitute_unhoused_days: authored
                .household_action
                .persistent_exit_destitute_unhoused_days,
            persistent_exit_max_unhoused_days: authored
                .household_action
                .persistent_exit_max_unhoused_days,
            persistent_exit_daily_fraction: authored
                .household_action
                .persistent_exit_daily_fraction,
        },
        action_budget: ActionBudgetConfig {
            max_households_per_day: authored.action_budget.max_households_per_day,
            upgrade_batch_fraction_by_use,
            downgrade_batch_fraction_by_use,
            despawn_batch_fraction_by_use,
        },
        profiles,
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
