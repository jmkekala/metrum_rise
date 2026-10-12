// SPDX-License-Identifier: GPL-2.0-only

//! Freight operating-window timing and price adjustments.

use crate::simulation::economy::definitions::{FreightTimingProfile, LogisticsRuntimeTuning};

/// Quotes one carrier contract from the route estimate already needed for dispatch.
/// Round-trip vehicle time uses the operational clock; queue/window waiting adds no fee.
/// This is constant arithmetic with no route queries or allocations.
pub(super) fn freight_cost(
    travel_seconds: f32,
    seconds_per_day: f64,
    tuning: &LogisticsRuntimeTuning,
) -> f32 {
    let vehicle_hours = f64::from(travel_seconds) * 48.0 / seconds_per_day;
    tuning.dispatch_fee_currency
        + (vehicle_hours * f64::from(tuning.vehicle_hour_cost_currency)) as f32
}

pub(super) fn freight_profile_prefers_minute(
    profile: &FreightTimingProfile,
    minute_of_day: u16,
) -> bool {
    profile
        .preferred_windows
        .iter()
        .any(|window| minute_of_day >= window.start_minute && minute_of_day < window.end_minute)
}

/// Converts movement seconds to delivery-clock hours, then adds the operating-window delay.
/// Window penalties are already operational minutes and must not be accelerated again.
pub(super) fn eta_hours_from_travel_seconds(
    travel_seconds: f32,
    profile: &FreightTimingProfile,
    minute_of_day: u16,
    seconds_per_day: f64,
) -> u16 {
    let penalty_hours = if freight_profile_prefers_minute(profile, minute_of_day) {
        0.0
    } else {
        f64::from(profile.outside_window_eta_penalty_minutes) / 60.0
    };
    ((f64::from(travel_seconds) * 24.0 / seconds_per_day + penalty_hours).ceil() as u16).max(1)
}

pub(super) fn adjusted_unit_price(
    unit_price: f32,
    profile: &FreightTimingProfile,
    minute_of_day: u16,
) -> f32 {
    if freight_profile_prefers_minute(profile, minute_of_day) {
        unit_price
    } else {
        unit_price * profile.outside_window_cost_multiplier
    }
}

/// Identifies the current goods-price interval without invalidating quotes every minute.
/// O(authored freight windows), no city scan or allocation; profiles with constant prices
/// create no boundary. The same minute in another day has the same price interval.
pub(crate) fn freight_price_phase(
    tuning: &crate::simulation::economy::definitions::RuntimeEconomyTuning,
    minute_of_day: u16,
) -> u16 {
    tuning
        .operational_clock
        .freight_profiles
        .iter()
        .filter(|profile| profile.outside_window_cost_multiplier != 1.0)
        .flat_map(|profile| &profile.preferred_windows)
        .flat_map(|window| [window.start_minute, window.end_minute])
        .filter(|&boundary| boundary <= minute_of_day)
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::economy::definitions::load_runtime_economy_tuning;

    #[test]
    fn price_phase_changes_only_at_authored_receiving_boundaries() {
        let tuning = load_runtime_economy_tuning().unwrap();
        assert_eq!(
            freight_price_phase(&tuning, 420),
            freight_price_phase(&tuning, 479)
        );
        assert_ne!(
            freight_price_phase(&tuning, 419),
            freight_price_phase(&tuning, 420)
        );
        assert_ne!(
            freight_price_phase(&tuning, 479),
            freight_price_phase(&tuning, 480)
        );
        assert_eq!(
            freight_price_phase(&tuning, 480),
            freight_price_phase(&tuning, 1079)
        );
        assert_ne!(
            freight_price_phase(&tuning, 1079),
            freight_price_phase(&tuning, 1080)
        );
    }

    #[test]
    fn carrier_quote_covers_dispatch_and_round_trip_vehicle_time() {
        let tuning = load_runtime_economy_tuning().unwrap();
        let short = freight_cost(30.0, 1440.0, &tuning.logistics);
        assert_eq!(short, 8.0); // Half an operational hour each way, plus dispatch.
        assert_eq!(freight_cost(240.0, 1440.0, &tuning.logistics), 36.0);
        assert_eq!(freight_cost(60.0, 2880.0, &tuning.logistics), short);
        assert!(short / 40.0 > short / tuning.logistics.truck_payload_kg);
    }

    #[test]
    fn freight_eta_uses_authored_clock_without_rescaling_window_penalties() {
        let tuning = load_runtime_economy_tuning().unwrap();
        let mut profile = tuning.operational_clock.freight_profiles[0].clone();
        profile.preferred_windows.clear();
        profile.outside_window_eta_penalty_minutes = 60;
        // A ten-minute physical journey takes ten game hours on a 24-minute day.
        assert_eq!(
            eta_hours_from_travel_seconds(600.0, &profile, 0, 1440.0),
            11
        );
        assert_eq!(eta_hours_from_travel_seconds(600.0, &profile, 0, 2880.0), 6);
        assert_eq!(
            eta_hours_from_travel_seconds(600.0, &profile, 0, 86400.0),
            2
        );
        profile.outside_window_eta_penalty_minutes = 0;
        assert_eq!(
            eta_hours_from_travel_seconds(600.0, &profile, 0, 1440.0),
            10
        );
    }
}
