//! Work schedule timing helpers for building-origin agent trips.

use super::planning::estimate_building_origin_trip_minutes;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::definitions::{
    MinuteWindow, OperationalClockRuntimeTuning, RuntimeEconomyCatalog, WorkTimingProfile,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::ZoneType;
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
    next_departure_day: &mut u32,
    next_departure_minute: &mut u16,
    next_departure_origin_building: &mut usize,
    next_departure_target_building: &mut usize,
    next_departure_activity: &mut u8,
    cached_schedule_work_building: &mut usize,
    cached_work_profile_index: &mut u16,
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
        clear_departure_cache_if_cached(
            next_departure_day,
            next_departure_minute,
            next_departure_origin_building,
            next_departure_target_building,
            next_departure_activity,
        );
        return None;
    }
    if current_building != home_building && current_building != work_building {
        clear_departure_cache_if_cached(
            next_departure_day,
            next_departure_minute,
            next_departure_origin_building,
            next_departure_target_building,
            next_departure_activity,
        );
        return None;
    }

    if *next_departure_target_building != usize::MAX
        && *next_departure_origin_building == current_building
        && cached_departure_matches_assignment(
            current_building,
            home_building,
            work_building,
            *next_departure_target_building,
            *next_departure_activity,
        )
    {
        if day_index < *next_departure_day
            || (day_index == *next_departure_day && minute_of_day < *next_departure_minute)
        {
            return None;
        }
        if day_index == *next_departure_day {
            return Some((*next_departure_target_building, *next_departure_activity));
        }
    }

    clear_departure_cache(
        next_departure_day,
        next_departure_minute,
        next_departure_origin_building,
        next_departure_target_building,
        next_departure_activity,
    );

    let work_profile_index = cached_work_profile_index_for_building(
        work_building,
        cached_schedule_work_building,
        cached_work_profile_index,
        allocator,
        operational_clock,
        economy_catalog,
    )?;
    let work_profile = operational_clock.work_profiles.get(work_profile_index)?;
    if work_profile.arrival_windows.is_empty()
        || work_profile.arrival_windows.len() != work_profile.departure_windows.len()
    {
        return None;
    }

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

    let (target_building, activity, scheduled_minute, window_end_minute) =
        if current_building == home_building {
            (
                work_building,
                1,
                arrival_departure_minute,
                arrival_window.end_minute,
            )
        } else {
            (
                home_building,
                0,
                departure_minute,
                departure_window.end_minute,
            )
        };
    let departure_day = next_departure_day_for_minute(
        day_index,
        minute_of_day,
        scheduled_minute,
        window_end_minute,
    );
    *next_departure_day = departure_day;
    *next_departure_minute = scheduled_minute;
    *next_departure_origin_building = current_building;
    *next_departure_target_building = target_building;
    *next_departure_activity = activity;

    if departure_day == day_index && minute_of_day >= scheduled_minute {
        return Some((target_building, activity));
    }

    None
}

fn clear_departure_cache_if_cached(
    next_departure_day: &mut u32,
    next_departure_minute: &mut u16,
    next_departure_origin_building: &mut usize,
    next_departure_target_building: &mut usize,
    next_departure_activity: &mut u8,
) {
    if *next_departure_day != u32::MAX
        || *next_departure_origin_building != usize::MAX
        || *next_departure_target_building != usize::MAX
    {
        clear_departure_cache(
            next_departure_day,
            next_departure_minute,
            next_departure_origin_building,
            next_departure_target_building,
            next_departure_activity,
        );
    }
}

fn clear_departure_cache(
    next_departure_day: &mut u32,
    next_departure_minute: &mut u16,
    next_departure_origin_building: &mut usize,
    next_departure_target_building: &mut usize,
    next_departure_activity: &mut u8,
) {
    *next_departure_day = u32::MAX;
    *next_departure_minute = 0;
    *next_departure_origin_building = usize::MAX;
    *next_departure_target_building = usize::MAX;
    *next_departure_activity = 0;
}

fn cached_departure_matches_assignment(
    current_building: usize,
    home_building: usize,
    work_building: usize,
    cached_target_building: usize,
    cached_activity: u8,
) -> bool {
    (current_building == home_building
        && cached_target_building == work_building
        && cached_activity == 1)
        || (current_building == work_building
            && cached_target_building == home_building
            && cached_activity == 0)
}

fn cached_work_profile_index_for_building(
    work_building: usize,
    cached_schedule_work_building: &mut usize,
    cached_work_profile_index: &mut u16,
    allocator: &BuildingAllocator,
    operational_clock: &OperationalClockRuntimeTuning,
    economy_catalog: &RuntimeEconomyCatalog,
) -> Option<usize> {
    if *cached_schedule_work_building == work_building {
        if *cached_work_profile_index == u16::MAX {
            return None;
        }
        let index = usize::from(*cached_work_profile_index);
        if index < operational_clock.work_profiles.len() {
            return Some(index);
        }
    }

    let index =
        resolve_work_profile_index(work_building, allocator, operational_clock, economy_catalog);
    *cached_schedule_work_building = work_building;
    *cached_work_profile_index = index
        .and_then(|idx| u16::try_from(idx).ok())
        .filter(|idx| *idx != u16::MAX)
        .unwrap_or(u16::MAX);
    index
}

fn resolve_work_profile_index(
    work_building: usize,
    allocator: &BuildingAllocator,
    operational_clock: &OperationalClockRuntimeTuning,
    economy_catalog: &RuntimeEconomyCatalog,
) -> Option<usize> {
    let work_building_ref = allocator.buildings.get(work_building)?;
    if let Some(profile_id) = economy_catalog
        .profile_by_runtime_id(work_building_ref.economy_profile_runtime_id)
        .and_then(|profile| profile.work_schedule_profile.as_deref())
        && let Some(index) = operational_clock
            .work_profiles
            .iter()
            .position(|profile| profile.id == profile_id)
    {
        return Some(index);
    }

    let zone_key = match work_building_ref.zone_type {
        ZoneType::Commercial => "commercial",
        ZoneType::Industrial => "industrial",
        ZoneType::Residential | ZoneType::Office | ZoneType::Mixed | ZoneType::None => {
            return None;
        }
    };
    let profile_id = operational_clock
        .work_profile_for_zone_type(zone_key)?
        .id
        .as_str();
    operational_clock
        .work_profiles
        .iter()
        .position(|profile| profile.id == profile_id)
}

fn next_departure_day_for_minute(
    day_index: u32,
    minute_of_day: u16,
    scheduled_minute: u16,
    window_end_minute: u16,
) -> u32 {
    if minute_of_day < scheduled_minute || minute_of_day < window_end_minute {
        day_index
    } else {
        day_index.saturating_add(1)
    }
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
