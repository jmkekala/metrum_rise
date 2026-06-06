//! Work schedule timing helpers for building-origin agent trips.

use super::planning::estimate_building_origin_trip_minutes;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::definitions::{
    MinuteWindow, OperationalClockRuntimeTuning, RuntimeEconomyCatalog, WorkTimingProfile,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use std::sync::atomic::AtomicU32;

/// Returns a scheduled work/home trip target when the current simulation minute reaches a window.
#[allow(clippy::too_many_arguments)]
pub(super) fn maybe_schedule_work_trip(
    current_building: usize,
    home_building: usize,
    work_building: usize,
    has_car: bool,
    schedule_seed: u32,
    cached_commute_minutes: &mut u16,
    next_commute_refresh_time: &mut f32,
    sim_time: f32,
    day_index: u32,
    minute_of_day: u16,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
    operational_clock: &OperationalClockRuntimeTuning,
    economy_catalog: &RuntimeEconomyCatalog,
) -> Option<(usize, u8)> {
    if home_building == usize::MAX || work_building == usize::MAX {
        return None;
    }
    let work_building_ref = allocator.buildings.get(work_building)?;
    let work_profile = economy_catalog
        .profile_by_runtime_id(work_building_ref.economy_profile_runtime_id)
        .and_then(|profile| profile.work_schedule_profile.as_deref())
        .and_then(|profile_id| {
            operational_clock
                .work_profiles
                .iter()
                .find(|profile| profile.id == profile_id)
        })
        .or_else(|| {
            let current_zone = work_building_ref.zone_type;
            operational_clock.work_profile_for_zone_type(match current_zone {
                crate::simulation::zoning::ZoneType::Commercial => "commercial",
                crate::simulation::zoning::ZoneType::Industrial => "industrial",
                crate::simulation::zoning::ZoneType::Residential
                | crate::simulation::zoning::ZoneType::Office
                | crate::simulation::zoning::ZoneType::Mixed
                | crate::simulation::zoning::ZoneType::None => return None,
            })
        })?;

    if (*cached_commute_minutes == 0 || sim_time >= *next_commute_refresh_time)
        && let Some(estimate) = estimate_building_origin_trip_minutes(
            home_building,
            work_building,
            has_car,
            allocator,
            transit_network,
            graph,
            pathfind_count,
        )
    {
        *cached_commute_minutes = estimate;
        *next_commute_refresh_time =
            sim_time + f32::from(operational_clock.travel_estimate_refresh_minutes);
    }
    let commute_minutes = (*cached_commute_minutes).max(1);
    let shift_index = (schedule_seed % work_profile.arrival_windows.len() as u32) as usize;
    let arrival_window = &work_profile.arrival_windows[shift_index];
    let arrival_minute = stable_minute_in_window(work_profile, arrival_window, schedule_seed);
    let arrival_departure_minute = arrival_minute
        .saturating_sub(commute_minutes.saturating_add(work_profile.reliability_buffer_minutes));
    let departure_window = &work_profile.departure_windows[shift_index];
    let departure_minute = stable_minute_in_window(
        work_profile,
        departure_window,
        schedule_seed.rotate_left(11),
    );

    if current_building == home_building
        && minute_reached_schedule(
            minute_of_day,
            arrival_departure_minute,
            arrival_window.end_minute,
        )
    {
        return Some((work_building, 1));
    }
    if current_building == work_building
        && minute_reached_schedule(minute_of_day, departure_minute, departure_window.end_minute)
    {
        return Some((home_building, 0));
    }

    let _ = day_index;
    None
}

fn stable_minute_in_window(
    profile: &WorkTimingProfile,
    window: &MinuteWindow,
    schedule_seed: u32,
) -> u16 {
    let span = window.end_minute.saturating_sub(window.start_minute).max(1);
    let mixed_seed = schedule_seed ^ profile.id.len() as u32;
    window.start_minute + (mixed_seed % u32::from(span)) as u16
}

fn minute_reached_schedule(
    minute_of_day: u16,
    scheduled_minute: u16,
    window_end_minute: u16,
) -> bool {
    minute_of_day >= scheduled_minute && minute_of_day < window_end_minute
}
