//! Freight operating-window timing and price adjustments.

use crate::simulation::economy::definitions::FreightTimingProfile;

const OPERATIONAL_HOUR_SECONDS: f32 = 60.0 * 60.0;

pub(super) fn freight_profile_prefers_minute(
    profile: &FreightTimingProfile,
    minute_of_day: u16,
) -> bool {
    profile
        .preferred_windows
        .iter()
        .any(|window| minute_of_day >= window.start_minute && minute_of_day < window.end_minute)
}

pub(super) fn adjusted_travel_seconds(
    travel_seconds: f32,
    profile: &FreightTimingProfile,
    minute_of_day: u16,
) -> f32 {
    if freight_profile_prefers_minute(profile, minute_of_day) {
        travel_seconds
    } else {
        travel_seconds + f32::from(profile.outside_window_eta_penalty_minutes) * 60.0
    }
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

pub(super) fn eta_hours_from_travel_seconds(travel_seconds: f32) -> u16 {
    ((travel_seconds / OPERATIONAL_HOUR_SECONDS).ceil() as u16).max(1)
}
