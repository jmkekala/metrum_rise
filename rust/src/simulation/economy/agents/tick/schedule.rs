// SPDX-License-Identifier: GPL-2.0-only

//! Work schedule timing helpers for building-origin agent trips.

use super::planning::estimate_building_origin_trip_seconds;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::time::TimeSystem;
use crate::simulation::economy::definitions::{
    MinuteWindow, OperationalClockRuntimeTuning, RuntimeEconomyCatalog, WorkTimingProfile,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::ZoneType;
use std::sync::atomic::AtomicU32;

/// Mutable view over one agent's cached schedule-derived fields.
pub(super) struct ScheduleCacheMut<'a> {
    /// Cached one-way commute estimate in authored minutes.
    pub(super) cached_commute_minutes: &'a mut u16,
    /// Earliest simulation time when the commute estimate may be refreshed.
    pub(super) next_commute_refresh_time: &'a mut f32,
    /// Operational day for the next cached departure.
    pub(super) next_departure_day: &'a mut u32,
    /// Minute-of-day for the next cached departure.
    pub(super) next_departure_minute: &'a mut u16,
    /// Building where the cached departure starts.
    pub(super) next_departure_origin_building: &'a mut usize,
    /// Building where the cached departure ends.
    pub(super) next_departure_target_building: &'a mut usize,
    /// Activity to apply when the cached departure starts.
    pub(super) next_departure_activity: &'a mut u8,
    /// Work building whose work-profile lookup is cached.
    pub(super) cached_schedule_work_building: &'a mut usize,
    /// Index into runtime work profiles, or `u16::MAX` when no profile exists.
    pub(super) cached_work_profile_index: &'a mut u16,
}

impl ScheduleCacheMut<'_> {
    fn record_commute_estimate(
        &mut self,
        travel_seconds: u16,
        sim_time: f32,
        time: &TimeSystem,
        operational_clock: &OperationalClockRuntimeTuning,
    ) {
        let seconds_per_minute = time.seconds_per_minute();
        *self.cached_commute_minutes = (f64::from(travel_seconds) / seconds_per_minute)
            .ceil()
            .clamp(1.0, f64::from(u16::MAX)) as u16;
        *self.next_commute_refresh_time = sim_time
            + (f64::from(operational_clock.travel_estimate_refresh_minutes) * seconds_per_minute)
                as f32;
    }

    fn clear_departure_if_cached(&mut self) {
        if *self.next_departure_day != u32::MAX
            || *self.next_departure_origin_building != usize::MAX
            || *self.next_departure_target_building != usize::MAX
        {
            self.clear_departure();
        }
    }

    fn clear_departure(&mut self) {
        *self.next_departure_day = u32::MAX;
        *self.next_departure_minute = 0;
        *self.next_departure_origin_building = usize::MAX;
        *self.next_departure_target_building = usize::MAX;
        *self.next_departure_activity = 0;
    }
}

/// Returns a scheduled work/home trip target when the current simulation minute reaches a window.
#[allow(clippy::too_many_arguments)]
pub(super) fn maybe_schedule_work_trip(
    current_building: usize,
    current_activity: u8,
    home_building: usize,
    work_building: usize,
    has_car: bool,
    schedule_seed: u32,
    cache: &mut ScheduleCacheMut<'_>,
    sim_time: f32,
    time: &TimeSystem,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
    operational_clock: &OperationalClockRuntimeTuning,
    economy_catalog: &RuntimeEconomyCatalog,
) -> Option<(usize, u8)> {
    if home_building == usize::MAX || work_building == usize::MAX {
        cache.clear_departure_if_cached();
        return None;
    }
    if current_building != home_building && current_building != work_building {
        cache.clear_departure_if_cached();
        return None;
    }

    if home_building == work_building {
        cache.clear_departure_if_cached();
        *cache.cached_commute_minutes = 0;
        let profile_index = cached_work_profile_index_for_building(
            work_building,
            cache.cached_schedule_work_building,
            cache.cached_work_profile_index,
            allocator,
            operational_clock,
            economy_catalog,
        )?;
        let profile = operational_clock.work_profiles.get(profile_index)?;
        let activity = on_site_work_activity(profile, schedule_seed, time.minute_of_day)?;
        return (activity != current_activity).then_some((home_building, activity));
    }

    if *cache.next_departure_target_building != usize::MAX
        && *cache.next_departure_origin_building == current_building
        && cached_departure_matches_assignment(
            current_building,
            home_building,
            work_building,
            *cache.next_departure_target_building,
            *cache.next_departure_activity,
        )
    {
        if time.day_index < *cache.next_departure_day
            || (time.day_index == *cache.next_departure_day
                && time.minute_of_day < *cache.next_departure_minute)
        {
            return None;
        }
        if time.day_index == *cache.next_departure_day {
            return Some((
                *cache.next_departure_target_building,
                *cache.next_departure_activity,
            ));
        }
    }

    cache.clear_departure();

    let work_profile_index = cached_work_profile_index_for_building(
        work_building,
        cache.cached_schedule_work_building,
        cache.cached_work_profile_index,
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

    if (*cache.cached_commute_minutes == 0 || sim_time >= *cache.next_commute_refresh_time)
        && let Some(estimate) = estimate_building_origin_trip_seconds(
            home_building,
            work_building,
            has_car,
            allocator,
            transit_network,
            graph,
            pathfind_count,
        )
    {
        cache.record_commute_estimate(estimate, sim_time, time, operational_clock);
    }
    let commute_minutes = (*cache.cached_commute_minutes).max(1);
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
        time.day_index,
        time.minute_of_day,
        scheduled_minute,
        window_end_minute,
    );
    *cache.next_departure_day = departure_day;
    *cache.next_departure_minute = scheduled_minute;
    *cache.next_departure_origin_building = current_building;
    *cache.next_departure_target_building = target_building;
    *cache.next_departure_activity = activity;

    if departure_day == time.day_index && time.minute_of_day >= scheduled_minute {
        return Some((target_building, activity));
    }

    None
}

// A resident farmer starts/ends the authored shift in place, including overnight shifts.
fn on_site_work_activity(profile: &WorkTimingProfile, seed: u32, minute: u16) -> Option<u8> {
    if profile.arrival_windows.is_empty()
        || profile.arrival_windows.len() != profile.departure_windows.len()
    {
        return None;
    }
    let shift = (seed % profile.arrival_windows.len() as u32) as usize;
    let start = stable_minute_in_window(profile, &profile.arrival_windows[shift], seed);
    let end = stable_minute_in_window(
        profile,
        &profile.departure_windows[shift],
        seed.rotate_left(11),
    );
    Some(u8::from(if start <= end {
        minute >= start && minute < end
    } else {
        minute >= start || minute < end
    }))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::core::time::test_clock;

    fn with_schedule_cache(run: impl FnOnce(&mut ScheduleCacheMut<'_>)) {
        let mut agents = crate::simulation::economy::agents::AgentSystem::new();
        agents.spawn_housed_agent(0, 0.0, 0.0);
        let data = &mut agents.agents;
        data.cached_schedule_work_building[0] = 1;
        data.cached_work_profile_index[0] = 0;
        let mut cache = ScheduleCacheMut {
            cached_commute_minutes: &mut data.cached_commute_minutes[0],
            next_commute_refresh_time: &mut data.next_commute_refresh_time[0],
            next_departure_day: &mut data.next_departure_day[0],
            next_departure_minute: &mut data.next_departure_minute[0],
            next_departure_origin_building: &mut data.next_departure_origin_building[0],
            next_departure_target_building: &mut data.next_departure_target_building[0],
            next_departure_activity: &mut data.next_departure_activity[0],
            cached_schedule_work_building: &mut data.cached_schedule_work_building[0],
            cached_work_profile_index: &mut data.cached_work_profile_index[0],
        };
        run(&mut cache);
    }

    #[test]
    fn work_departures_and_refresh_deadlines_follow_active_day_length() {
        let tuning =
            crate::simulation::economy::definitions::load_runtime_economy_tuning().unwrap();
        let catalog =
            crate::simulation::economy::definitions::load_runtime_economy_catalog().unwrap();
        let allocator = BuildingAllocator::new();
        let network = TransitNetwork::new();
        let graph = RegionGraph::new();
        let pathfind_count = AtomicU32::new(0);
        let mut clock = tuning.operational_clock.clone();
        clock.work_profiles = vec![WorkTimingProfile {
            id: "test_shift".to_owned(),
            arrival_windows: vec![MinuteWindow {
                start_minute: 480,
                end_minute: 481,
            }],
            departure_windows: vec![MinuteWindow {
                start_minute: 1_020,
                end_minute: 1_021,
            }],
            reliability_buffer_minutes: 10,
        }];
        clock.travel_estimate_refresh_minutes = 360;
        // Seed a completed 120-second route estimate and cached profile; this fixture
        // isolates schedule units from graph construction and route-planner cost.
        for (seconds_per_day, expected_minutes, expected_refresh, expected_departure) in [
            (720.0, 240, 280.0, 230),
            (1_440.0, 120, 460.0, 350),
            (2_880.0, 60, 820.0, 410),
        ] {
            let mut time = test_clock(1, 0);
            time.seconds_per_day = seconds_per_day;
            with_schedule_cache(|cache| {
                cache.record_commute_estimate(120, 100.0, &time, &clock);
                assert_eq!(*cache.cached_commute_minutes, expected_minutes);
                assert_eq!(*cache.next_commute_refresh_time, expected_refresh);
                assert_eq!(
                    maybe_schedule_work_trip(
                        0,
                        0,
                        0,
                        1,
                        false,
                        0,
                        cache,
                        100.0,
                        &time,
                        &allocator,
                        &network,
                        &graph,
                        &pathfind_count,
                        &clock,
                        &catalog,
                    ),
                    None
                );
                assert_eq!(*cache.next_departure_minute, expected_departure);
                time.minute_of_day = expected_departure;
                assert_eq!(
                    maybe_schedule_work_trip(
                        0,
                        0,
                        0,
                        1,
                        false,
                        0,
                        cache,
                        100.0,
                        &time,
                        &allocator,
                        &network,
                        &graph,
                        &pathfind_count,
                        &clock,
                        &catalog,
                    ),
                    Some((1, 1))
                );
            });
        }
        assert_eq!(pathfind_count.load(std::sync::atomic::Ordering::Relaxed), 0);
    }

    #[test]
    #[ignore = "manual matched release timing of commute cache unit conversion"]
    fn benchmark_commute_cache_recording() {
        use std::{hint::black_box, time::Instant};
        let clock = crate::simulation::economy::definitions::load_runtime_economy_tuning()
            .unwrap()
            .operational_clock
            .clone();
        for seconds_per_day in [720.0, 1_440.0, 2_880.0] {
            let mut time = test_clock(1, 0);
            time.seconds_per_day = seconds_per_day;
            with_schedule_cache(|cache| {
                let mut samples = [0.0; 11];
                for sample in &mut samples {
                    let start = Instant::now();
                    for _ in 0..1_000_000 {
                        cache.record_commute_estimate(
                            black_box(120),
                            black_box(100.0),
                            black_box(&time),
                            black_box(&clock),
                        );
                        black_box(*cache.cached_commute_minutes);
                        black_box(*cache.next_commute_refresh_time);
                    }
                    *sample = start.elapsed().as_secs_f64() * 1_000.0;
                }
                samples.sort_by(f64::total_cmp);
                eprintln!(
                    "commute_cache seconds_per_day={seconds_per_day} updates=1000000 median_ms={:.3}",
                    samples[5]
                );
            });
        }
    }

    #[test]
    fn resident_shifts_switch_activity_at_authored_boundaries_and_across_midnight() {
        let tuning =
            crate::simulation::economy::definitions::load_runtime_economy_tuning().unwrap();
        let profile = tuning
            .operational_clock
            .work_profiles
            .iter()
            .find(|profile| profile.id == "three_shift_work")
            .unwrap();
        for seed in 0..3 {
            let shift = seed as usize;
            let start = stable_minute_in_window(profile, &profile.arrival_windows[shift], seed);
            let end = stable_minute_in_window(
                profile,
                &profile.departure_windows[shift],
                seed.rotate_left(11),
            );
            assert_eq!(on_site_work_activity(profile, seed, start - 1), Some(0));
            assert_eq!(on_site_work_activity(profile, seed, start), Some(1));
            assert_eq!(on_site_work_activity(profile, seed, end - 1), Some(1));
            assert_eq!(on_site_work_activity(profile, seed, end), Some(0));
        }
        assert_eq!(on_site_work_activity(profile, 2, 0), Some(1));
    }
}
