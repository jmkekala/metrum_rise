//! Runtime economy tuning validation.

use super::common::duplicate_ids;
use crate::simulation::economy::definitions::runtime::{
    MinuteWindow, OperationalClockRuntimeTuning, RuntimeEconomyTuning,
};

pub(in crate::simulation::economy::definitions) fn validate_runtime_tuning(
    tuning: &RuntimeEconomyTuning,
) -> Result<(), String> {
    validate_range(
        tuning.operational_clock.seconds_per_day as f32,
        60.0,
        f32::INFINITY,
        "runtime_tuning.operational_clock.seconds_per_day",
    )?;
    if tuning.operational_clock.travel_estimate_refresh_minutes == 0 {
        return Err(
            "runtime_tuning.operational_clock.travel_estimate_refresh_minutes must be > 0"
                .to_owned(),
        );
    }
    if tuning
        .operational_clock
        .household_replenishment_check_interval_hours
        == 0
    {
        return Err(
            "runtime_tuning.operational_clock.household_replenishment_check_interval_hours must be > 0"
                .to_owned(),
        );
    }
    if tuning
        .operational_clock
        .household_replenishment_retry_cooldown_hours
        == 0
    {
        return Err(
            "runtime_tuning.operational_clock.household_replenishment_retry_cooldown_hours must be > 0"
                .to_owned(),
        );
    }
    if tuning
        .operational_clock
        .household_replenishment_terminal_failure_count
        == 0
    {
        return Err(
            "runtime_tuning.operational_clock.household_replenishment_terminal_failure_count must be > 0"
                .to_owned(),
        );
    }
    if tuning
        .operational_clock
        .household_shopping_leg_timeout_hours
        == 0
    {
        return Err(
            "runtime_tuning.operational_clock.household_shopping_leg_timeout_hours must be > 0"
                .to_owned(),
        );
    }
    if tuning.operational_clock.shipment_retry_cooldown_hours == 0 {
        return Err(
            "runtime_tuning.operational_clock.shipment_retry_cooldown_hours must be > 0".to_owned(),
        );
    }
    validate_range(
        tuning.commercial_owa_utility_cost_per_day,
        0.0,
        f32::INFINITY,
        "runtime_tuning.commercial_owa_utility_cost_per_day",
    )?;
    validate_range(
        tuning.industrial_owa_utility_cost_per_day,
        0.0,
        f32::INFINITY,
        "runtime_tuning.industrial_owa_utility_cost_per_day",
    )?;
    validate_range(
        tuning.logistics.truck_load_units,
        f32::EPSILON,
        f32::INFINITY,
        "runtime_tuning.logistics.truck_load_units",
    )?;
    if tuning.logistics.border_active_jobs_per_node == 0 {
        return Err("runtime_tuning.logistics.border_active_jobs_per_node must be > 0".to_owned());
    }
    if tuning.logistics.border_queued_jobs_per_node == 0 {
        return Err("runtime_tuning.logistics.border_queued_jobs_per_node must be > 0".to_owned());
    }
    if tuning.logistics.queued_shipment_expiry_hours == 0 {
        return Err("runtime_tuning.logistics.queued_shipment_expiry_hours must be > 0".to_owned());
    }
    if tuning.logistics.terminal_failure_attempts == 0 {
        return Err("runtime_tuning.logistics.terminal_failure_attempts must be > 0".to_owned());
    }
    validate_nonempty_u16_level_array(
        &tuning.construction.residential_hours_by_level,
        "runtime_tuning.construction.residential_hours_by_level",
    )?;
    validate_nonempty_u16_level_array(
        &tuning.construction.commercial_hours_by_level,
        "runtime_tuning.construction.commercial_hours_by_level",
    )?;
    validate_nonempty_u16_level_array(
        &tuning.construction.industrial_hours_by_level,
        "runtime_tuning.construction.industrial_hours_by_level",
    )?;
    validate_range(
        tuning.fiscal.income_tax_rate,
        0.0,
        1.0,
        "runtime_tuning.fiscal.income_tax_rate",
    )?;
    validate_range(
        tuning.fiscal.household_vat_rate,
        0.0,
        1.0,
        "runtime_tuning.fiscal.household_vat_rate",
    )?;
    validate_range(
        tuning.fiscal.business_purchase_tax_rate,
        0.0,
        1.0,
        "runtime_tuning.fiscal.business_purchase_tax_rate",
    )?;
    validate_range(
        tuning.fiscal.residential_property_tax_base,
        0.0,
        f32::INFINITY,
        "runtime_tuning.fiscal.residential_property_tax_base",
    )?;
    validate_range(
        tuning.fiscal.commercial_property_tax_base,
        0.0,
        f32::INFINITY,
        "runtime_tuning.fiscal.commercial_property_tax_base",
    )?;
    validate_range(
        tuning.fiscal.industrial_property_tax_base,
        0.0,
        f32::INFINITY,
        "runtime_tuning.fiscal.industrial_property_tax_base",
    )?;
    validate_range(
        tuning.fiscal.property_tax_level_multiplier,
        1.0,
        f32::INFINITY,
        "runtime_tuning.fiscal.property_tax_level_multiplier",
    )?;
    validate_work_profiles(&tuning.operational_clock)?;
    validate_freight_profiles(&tuning.operational_clock)?;
    validate_range(
        tuning.households.immigrant_starting_stock_days,
        0.0,
        f32::INFINITY,
        "runtime_tuning.households.immigrant_starting_stock_days",
    )?;
    validate_range(
        tuning.households.immigrant_starting_budget_per_member,
        0.0,
        f32::INFINITY,
        "runtime_tuning.households.immigrant_starting_budget_per_member",
    )?;
    validate_range(
        tuning.households.household_starting_budget_floor,
        0.0,
        f32::INFINITY,
        "runtime_tuning.households.household_starting_budget_floor",
    )?;
    validate_range(
        tuning.households.utility_cost_per_member_per_day,
        0.0,
        f32::INFINITY,
        "runtime_tuning.households.utility_cost_per_member_per_day",
    )?;
    validate_nonempty_level_array(
        &tuning
            .households
            .residential_move_in_min_reserve_days_by_level,
        "runtime_tuning.households.residential_move_in_min_reserve_days_by_level",
        0.0,
        f32::INFINITY,
    )?;
    validate_nonempty_level_array(
        &tuning.households.residential_stay_min_reserve_days_by_level,
        "runtime_tuning.households.residential_stay_min_reserve_days_by_level",
        0.0,
        f32::INFINITY,
    )?;
    if tuning.households.stay_failure_days_before_eviction == 0 {
        return Err(
            "runtime_tuning.households.stay_failure_days_before_eviction must be > 0".to_owned(),
        );
    }
    validate_nonempty_level_array(
        &tuning.viability.residential_min_occupancy_ratio_for_upgrade,
        "runtime_tuning.viability.residential_min_occupancy_ratio_for_upgrade",
        0.0,
        1.0,
    )?;
    validate_nonempty_level_array(
        &tuning
            .viability
            .residential_max_occupancy_ratio_for_downgrade,
        "runtime_tuning.viability.residential_max_occupancy_ratio_for_downgrade",
        0.0,
        1.0,
    )?;
    validate_nonempty_level_array(
        &tuning.viability.nonresidential_min_buffer_days_by_level,
        "runtime_tuning.viability.nonresidential_min_buffer_days_by_level",
        0.0,
        f32::INFINITY,
    )?;
    validate_nonempty_level_array(
        &tuning
            .viability
            .nonresidential_max_buffer_days_for_downgrade,
        "runtime_tuning.viability.nonresidential_max_buffer_days_for_downgrade",
        0.0,
        f32::INFINITY,
    )?;
    validate_range(
        tuning
            .viability
            .nonresidential_min_staffing_ratio_for_upgrade,
        0.0,
        1.0,
        "runtime_tuning.viability.nonresidential_min_staffing_ratio_for_upgrade",
    )?;
    validate_range(
        tuning
            .viability
            .nonresidential_max_staffing_ratio_for_downgrade,
        0.0,
        1.0,
        "runtime_tuning.viability.nonresidential_max_staffing_ratio_for_downgrade",
    )?;
    validate_range(
        tuning.viability.industrial_min_input_coverage_for_upgrade,
        0.0,
        1.0,
        "runtime_tuning.viability.industrial_min_input_coverage_for_upgrade",
    )?;
    validate_range(
        tuning.viability.industrial_min_output_headroom_for_upgrade,
        0.0,
        1.0,
        "runtime_tuning.viability.industrial_min_output_headroom_for_upgrade",
    )?;
    validate_range(
        tuning.viability.industrial_max_input_coverage_for_downgrade,
        0.0,
        1.0,
        "runtime_tuning.viability.industrial_max_input_coverage_for_downgrade",
    )?;
    validate_range(
        tuning
            .viability
            .industrial_max_output_headroom_for_downgrade,
        0.0,
        1.0,
        "runtime_tuning.viability.industrial_max_output_headroom_for_downgrade",
    )?;
    validate_range(
        tuning.owa_import_price_multiplier,
        1.0,
        f32::INFINITY,
        "runtime_tuning.owa_import_price_multiplier",
    )?;
    validate_range(
        tuning.owa_export_price_multiplier,
        0.0,
        1.0,
        "runtime_tuning.owa_export_price_multiplier",
    )?;
    validate_range(
        tuning.owa_distress_liquidation_multiplier,
        0.0,
        tuning.owa_export_price_multiplier,
        "runtime_tuning.owa_distress_liquidation_multiplier",
    )?;
    Ok(())
}

fn validate_work_profiles(clock: &OperationalClockRuntimeTuning) -> Result<(), String> {
    if clock.work_profiles.is_empty() {
        return Err("runtime_tuning.operational_clock.work_profiles must not be empty".to_owned());
    }
    let duplicates = duplicate_ids(
        clock
            .work_profiles
            .iter()
            .map(|profile| profile.id.as_str()),
    );
    if let Some(duplicate) = duplicates.into_iter().next() {
        return Err(format!(
            "runtime_tuning.operational_clock.work_profiles contains duplicate id '{duplicate}'"
        ));
    }
    for profile in &clock.work_profiles {
        if profile.id.trim().is_empty() {
            return Err(
                "runtime_tuning.operational_clock.work_profiles[*].id must not be empty".to_owned(),
            );
        }
        if profile.arrival_windows.is_empty() || profile.departure_windows.is_empty() {
            return Err(format!(
                "runtime_tuning.operational_clock.work_profiles.{} must define arrival_windows and departure_windows",
                profile.id
            ));
        }
        if profile.arrival_windows.len() != profile.departure_windows.len() {
            return Err(format!(
                "runtime_tuning.operational_clock.work_profiles.{} must use matching arrival/departure window counts",
                profile.id
            ));
        }
        if profile.reliability_buffer_minutes == 0 {
            return Err(format!(
                "runtime_tuning.operational_clock.work_profiles.{}.reliability_buffer_minutes must be > 0",
                profile.id
            ));
        }
        for (idx, window) in profile.arrival_windows.iter().enumerate() {
            validate_minute_window(
                *window,
                &format!(
                    "runtime_tuning.operational_clock.work_profiles.{}.arrival_windows[{idx}]",
                    profile.id
                ),
            )?;
        }
        for (idx, window) in profile.departure_windows.iter().enumerate() {
            validate_minute_window(
                *window,
                &format!(
                    "runtime_tuning.operational_clock.work_profiles.{}.departure_windows[{idx}]",
                    profile.id
                ),
            )?;
        }
    }
    for (zone_type, profile_id) in &clock.work_profile_by_zone_type {
        validate_zone_type_key(
            zone_type,
            "runtime_tuning.operational_clock.work_profile_by_zone_type",
        )?;
        if !clock
            .work_profiles
            .iter()
            .any(|profile| profile.id == *profile_id)
        {
            return Err(format!(
                "runtime_tuning.operational_clock.work_profile_by_zone_type.{zone_type} references missing profile '{profile_id}'"
            ));
        }
    }
    Ok(())
}

fn validate_freight_profiles(clock: &OperationalClockRuntimeTuning) -> Result<(), String> {
    if clock.freight_profiles.is_empty() {
        return Err(
            "runtime_tuning.operational_clock.freight_profiles must not be empty".to_owned(),
        );
    }
    let duplicates = duplicate_ids(
        clock
            .freight_profiles
            .iter()
            .map(|profile| profile.id.as_str()),
    );
    if let Some(duplicate) = duplicates.into_iter().next() {
        return Err(format!(
            "runtime_tuning.operational_clock.freight_profiles contains duplicate id '{duplicate}'"
        ));
    }
    for profile in &clock.freight_profiles {
        if profile.id.trim().is_empty() {
            return Err(
                "runtime_tuning.operational_clock.freight_profiles[*].id must not be empty"
                    .to_owned(),
            );
        }
        if profile.preferred_windows.is_empty() {
            return Err(format!(
                "runtime_tuning.operational_clock.freight_profiles.{} must define preferred_windows",
                profile.id
            ));
        }
        for (idx, window) in profile.preferred_windows.iter().enumerate() {
            validate_minute_window(
                *window,
                &format!(
                    "runtime_tuning.operational_clock.freight_profiles.{}.preferred_windows[{idx}]",
                    profile.id
                ),
            )?;
        }
        validate_range(
            profile.outside_window_cost_multiplier,
            1.0,
            10.0,
            &format!(
                "runtime_tuning.operational_clock.freight_profiles.{}.outside_window_cost_multiplier",
                profile.id
            ),
        )?;
    }
    for (zone_type, profile_id) in &clock.freight_profile_by_zone_type {
        validate_zone_type_key(
            zone_type,
            "runtime_tuning.operational_clock.freight_profile_by_zone_type",
        )?;
        if !clock
            .freight_profiles
            .iter()
            .any(|profile| profile.id == *profile_id)
        {
            return Err(format!(
                "runtime_tuning.operational_clock.freight_profile_by_zone_type.{zone_type} references missing profile '{profile_id}'"
            ));
        }
    }
    Ok(())
}

fn validate_minute_window(window: MinuteWindow, label: &str) -> Result<(), String> {
    let day_minutes: u16 = 24 * 60;
    if window.start_minute >= day_minutes
        || window.end_minute > day_minutes
        || window.start_minute >= window.end_minute
    {
        return Err(format!(
            "{label} must satisfy 0 <= start_minute < end_minute <= {}",
            day_minutes
        ));
    }
    Ok(())
}

fn validate_zone_type_key(zone_type: &str, label: &str) -> Result<(), String> {
    match zone_type {
        "commercial" | "industrial" | "residential" => Ok(()),
        other => Err(format!(
            "{label} contains unsupported zone_type key '{other}'"
        )),
    }
}

fn validate_nonempty_level_array(
    values: &[f32],
    label: &str,
    min_value: f32,
    max_value: f32,
) -> Result<(), String> {
    if values.is_empty() {
        return Err(format!("{label} must contain at least one level value"));
    }
    for (idx, value) in values.iter().copied().enumerate() {
        validate_range(value, min_value, max_value, &format!("{label}[{idx}]"))?;
    }
    Ok(())
}

fn validate_nonempty_u16_level_array(values: &[u16], label: &str) -> Result<(), String> {
    if values.is_empty() {
        return Err(format!("{label} must contain at least one level value"));
    }
    for (idx, value) in values.iter().copied().enumerate() {
        if value == 0 {
            return Err(format!("{label}[{idx}] must be > 0"));
        }
    }
    Ok(())
}

fn validate_range(value: f32, min_value: f32, max_value: f32, label: &str) -> Result<(), String> {
    if !value.is_finite() || value < min_value || value > max_value {
        Err(format!(
            "{label} must be finite and in [{}..={}]",
            min_value, max_value
        ))
    } else {
        Ok(())
    }
}
