// SPDX-License-Identifier: GPL-2.0-only

//! Work schedule timing helpers for building-origin agent trips.

use super::planning::estimate_building_origin_trip_seconds;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::scheduler::slot_in_minute_for_key;
use crate::simulation::core::time::{MINUTES_PER_DAY, TimeSystem};
use crate::simulation::economy::agents::{ACTIVITY_HOME, ACTIVITY_SHOPPING, ACTIVITY_WORK};
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
    parking: Option<(&crate::simulation::parking::ParkingSystem, crate::simulation::core::handles::Handle<crate::simulation::parking::Vehicle>, Option<&super::SharedNetworkQuotes>)>,
) -> Option<(usize, u8)> {
    // Shopping owns the agent through pickup and delivery acknowledgement, including
    // visits to the worker's own workplace and a return that reached home between hours.
    if current_activity == ACTIVITY_SHOPPING {
        cache.clear_departure_if_cached();
        return None;
    }
    // A job can end or change while its worker is still inside the old workplace.
    // Return through the ordinary trip planner before using the new job's schedule.
    if home_building != usize::MAX
        && current_building != home_building
        && current_building != work_building
        && current_activity == ACTIVITY_WORK
    {
        cache.clear_departure_if_cached();
        return Some((home_building, ACTIVITY_HOME));
    }
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
                && !departure_slot_reached(time, *cache.next_departure_minute, schedule_seed))
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
        && let Some(estimate) = if let Some((parking, car, quotes)) = parking {
            super::planning::estimate_parking_trip_seconds(home_building, work_building, car, has_car, parking, allocator, transit_network, graph, pathfind_count, quotes)
        } else {
            estimate_building_origin_trip_seconds(home_building, work_building, has_car, allocator, transit_network, graph, pathfind_count, None)
        }
    {
        cache.record_commute_estimate(estimate, sim_time, time, operational_clock);
    }
    let commute_minutes = (*cache.cached_commute_minutes).max(1);
    let shift_index = (schedule_seed % work_profile.arrival_windows.len() as u32) as usize;
    let arrival_window = &work_profile.arrival_windows[shift_index];
    let arrival_minute = stable_minute_in_window(work_profile, arrival_window, schedule_seed);
    let commute_lead = commute_minutes.saturating_add(work_profile.reliability_buffer_minutes);
    let departure_window = &work_profile.departure_windows[shift_index];
    let departure_minute = stable_minute_in_window(
        work_profile,
        departure_window,
        schedule_seed.rotate_left(11),
    );

    let (target_building, activity, departure_day, scheduled_minute) =
        if current_building == home_building {
            let (day, minute) = commute_departure(
                time.day_index,
                time.minute_of_day,
                arrival_minute,
                commute_lead,
                arrival_window.end_minute,
            );
            (work_building, 1, day, minute)
        } else {
            let day = next_departure_day_for_minute(
                time.day_index,
                time.minute_of_day,
                departure_minute,
                departure_window.end_minute,
            );
            (home_building, 0, day, departure_minute)
        };
    *cache.next_departure_day = departure_day;
    *cache.next_departure_minute = scheduled_minute;
    *cache.next_departure_origin_building = current_building;
    *cache.next_departure_target_building = target_building;
    *cache.next_departure_activity = activity;

    if departure_day == time.day_index
        && departure_slot_reached(time, scheduled_minute, schedule_seed)
    {
        return Some((target_building, activity));
    }

    None
}

// Whether the clock has reached the departure slot of an agent due to leave in authored minute
// `minute` (`SIM-02-K` part 4): the slot of the minute its seed keys, so one minute's departures
// leave over the minute's slots and not all in the frame that crosses into it. The day's last
// minute leaves at its first slot, so a frame spanning midnight cannot skip the departure.
fn departure_slot_reached(time: &TimeSystem, minute: u16, schedule_seed: u32) -> bool {
    time.minute_of_day > minute
        || (time.minute_of_day == minute
            && time.slot_in_minute() >= departure_slot_in_minute(minute, schedule_seed))
}

fn departure_slot_in_minute(minute: u16, schedule_seed: u32) -> u64 {
    if minute + 1 >= MINUTES_PER_DAY {
        return 0;
    }
    slot_in_minute_for_key(u64::from(schedule_seed))
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

// The day and minute a worker at home leaves to reach `arrival_minute`, `lead` minutes of commute
// and buffer ahead of it. A lead longer than the morning wraps: the true minute is the evening
// before the arrival. The arrival is today's until its window ends, then tomorrow's, so a wrapped
// minute is today or already past, never later. A past one is held as today's first minute, which
// the clock has reached, so the worker leaves now and not a day or a shift late.
fn commute_departure(
    day_index: u32,
    minute_of_day: u16,
    arrival_minute: u16,
    lead: u16,
    window_end_minute: u16,
) -> (u32, u16) {
    if lead <= arrival_minute {
        let minute = arrival_minute - lead;
        let day =
            next_departure_day_for_minute(day_index, minute_of_day, minute, window_end_minute);
        return (day, minute);
    }
    let arrival_day =
        next_departure_day_for_minute(day_index, minute_of_day, arrival_minute, window_end_minute);
    let before_midnight = lead - arrival_minute;
    if arrival_day > day_index && before_midnight <= MINUTES_PER_DAY {
        (day_index, MINUTES_PER_DAY - before_midnight)
    } else {
        (day_index, 0)
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
    fn job_loss_or_change_returns_a_worker_from_the_former_workplace() {
        let tuning =
            crate::simulation::economy::definitions::load_runtime_economy_tuning().unwrap();
        let catalog =
            crate::simulation::economy::definitions::load_runtime_economy_catalog().unwrap();
        let allocator = BuildingAllocator::new();
        let network = TransitNetwork::new();
        let graph = RegionGraph::new();
        let pathfind_count = AtomicU32::new(0);
        for work in [usize::MAX, 2] {
            with_schedule_cache(|cache| {
                *cache.next_departure_day = 9;
                *cache.next_departure_target_building = 1;
                let trip = maybe_schedule_work_trip(
                    1,
                    ACTIVITY_WORK,
                    0,
                    work,
                    false,
                    0,
                    cache,
                    0.0,
                    &test_clock(1, 0),
                    &allocator,
                    &network,
                    &graph,
                    &pathfind_count,
                    &tuning.operational_clock,
                    &catalog,
                    None,
                );
                assert_eq!(trip, Some((0, ACTIVITY_HOME)));
                assert_eq!(*cache.next_departure_target_building, usize::MAX);
                assert_eq!(*cache.next_departure_day, u32::MAX);
                let shopping = maybe_schedule_work_trip(
                    1,
                    crate::simulation::economy::agents::ACTIVITY_SHOPPING,
                    0,
                    work,
                    false,
                    0,
                    cache,
                    0.0,
                    &test_clock(1, 0),
                    &allocator,
                    &network,
                    &graph,
                    &pathfind_count,
                    &tuning.operational_clock,
                    &catalog,
                    None,
                );
                assert_eq!(
                    shopping, None,
                    "job changes must not override a shopping visit"
                );
            });
        }
        assert_eq!(pathfind_count.load(std::sync::atomic::Ordering::Relaxed), 0);
    }

    #[test]
    fn active_shopping_blocks_due_work_departures_at_home_and_work() {
        let tuning =
            crate::simulation::economy::definitions::load_runtime_economy_tuning().unwrap();
        let catalog =
            crate::simulation::economy::definitions::load_runtime_economy_catalog().unwrap();
        let allocator = BuildingAllocator::new();
        let network = TransitNetwork::new();
        let graph = RegionGraph::new();
        let pathfind_count = AtomicU32::new(0);
        for (origin, target, activity) in [(0, 1, ACTIVITY_WORK), (1, 0, ACTIVITY_HOME)] {
            with_schedule_cache(|cache| {
                *cache.next_departure_day = 1;
                *cache.next_departure_minute = 480;
                *cache.next_departure_origin_building = origin;
                *cache.next_departure_target_building = target;
                *cache.next_departure_activity = activity;
                assert_eq!(
                    maybe_schedule_work_trip(
                        origin,
                        ACTIVITY_SHOPPING,
                        0,
                        1,
                        false,
                        0,
                        cache,
                        0.0,
                        &test_clock(1, 480),
                        &allocator,
                        &network,
                        &graph,
                        &pathfind_count,
                        &tuning.operational_clock,
                        &catalog,
                        None,
                    ),
                    None
                );
                assert_eq!(*cache.next_departure_target_building, usize::MAX);
            });
        }
        assert_eq!(pathfind_count.load(std::sync::atomic::Ordering::Relaxed), 0);
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
                        None,
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
                        None,
                    ),
                    Some((1, 1))
                );
            });
        }
        assert_eq!(pathfind_count.load(std::sync::atomic::Ordering::Relaxed), 0);
    }

    // One shift, arrival at minute 480 and home time at 1,020, ten minutes of buffer.
    fn one_shift_clock() -> OperationalClockRuntimeTuning {
        let mut clock = crate::simulation::economy::definitions::load_runtime_economy_tuning()
            .unwrap()
            .operational_clock
            .clone();
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
        clock
    }

    // Asks the schedule of a worker with a known estimate at home (building 0) or at work (1).
    fn ask_schedule(
        cache: &mut ScheduleCacheMut<'_>,
        clock: &OperationalClockRuntimeTuning,
        at: u32,
        (day, minute): (u32, u16),
    ) -> Option<(usize, u8)> {
        let catalog =
            crate::simulation::economy::definitions::load_runtime_economy_catalog().unwrap();
        let pathfind_count = AtomicU32::new(0);
        let time = test_clock(day, minute);
        let trip = maybe_schedule_work_trip(
            at as usize,
            at as u8,
            0,
            1,
            false,
            0,
            cache,
            0.0,
            &time,
            &BuildingAllocator::new(),
            &TransitNetwork::new(),
            &RegionGraph::new(),
            &pathfind_count,
            clock,
            &catalog,
            None,
        );
        assert_eq!(pathfind_count.load(std::sync::atomic::Ordering::Relaxed), 0);
        trip
    }

    fn with_estimate(commute: u16, run: impl FnOnce(&mut ScheduleCacheMut<'_>)) {
        with_schedule_cache(|cache| {
            *cache.cached_commute_minutes = commute;
            *cache.next_commute_refresh_time = f32::MAX;
            run(cache);
        });
    }

    #[test]
    fn a_departure_that_fits_the_morning_keeps_its_day_and_minute() {
        for minute_of_day in (0..MINUTES_PER_DAY).step_by(7) {
            for (arrival, end) in [(330, 390), (480, 540), (810, 870), (1_290, 1_350)] {
                for lead in (1..=arrival).step_by(11).chain([arrival]) {
                    let minute = arrival - lead;
                    // The rule as it stood before wrapped departures, written out.
                    let day = if minute_of_day < minute || minute_of_day < end {
                        7
                    } else {
                        8
                    };
                    assert_eq!(
                        commute_departure(7, minute_of_day, arrival, lead, end),
                        (day, minute),
                        "at {minute_of_day}, arrival {arrival}, lead {lead}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_commute_longer_than_the_morning_leaves_the_evening_before() {
        let clock = one_shift_clock();
        // 590 + 10 minutes ahead of 480 is minute 1,320 of the day before.
        with_estimate(590, |cache| {
            assert_eq!(ask_schedule(cache, &clock, 0, (3, 1_000)), None);
            assert_eq!(*cache.next_departure_day, 3);
            assert_eq!(*cache.next_departure_minute, 1_320);
            assert_eq!(*cache.cached_commute_minutes, 590);
            assert_eq!(ask_schedule(cache, &clock, 0, (3, 1_319)), None);
            assert_eq!(ask_schedule(cache, &clock, 0, (3, 1_321)), Some((1, 1)));
        });
        // A worker who reaches home after the true minute leaves now, that evening.
        with_estimate(590, |cache| {
            assert_eq!(ask_schedule(cache, &clock, 0, (3, 1_400)), Some((1, 1)));
        });
        // Or after midnight on the arrival day, up to the end of its window. Never that evening,
        // 23 hours late.
        for minute in [0, 1, 300, 480] {
            with_estimate(590, |cache| {
                assert_eq!(ask_schedule(cache, &clock, 0, (4, minute)), Some((1, 1)));
                assert_eq!(*cache.next_departure_day, 4);
                assert_eq!(*cache.next_departure_minute, 0, "at {minute}");
            });
        }
        // The arrival's window has ended: the next arrival's true minute is tonight's.
        with_estimate(590, |cache| {
            assert_eq!(ask_schedule(cache, &clock, 0, (4, 600)), None);
            assert_eq!(*cache.next_departure_day, 4);
            assert_eq!(*cache.next_departure_minute, 1_320);
        });
    }

    #[test]
    fn a_wrapped_departure_a_frame_carries_past_midnight_still_leaves() {
        let clock = one_shift_clock();
        with_estimate(590, |cache| {
            assert_eq!(ask_schedule(cache, &clock, 0, (3, 1_000)), None);
            assert_eq!(ask_schedule(cache, &clock, 0, (4, 0)), Some((1, 1)));
            assert_eq!(*cache.next_departure_day, 4);
        });
    }

    #[test]
    fn an_estimate_of_a_day_or_more_leaves_now() {
        let clock = one_shift_clock();
        // 480 ahead of the arrival is midnight, 1,440 more the midnight before that.
        for commute in [1_910, 1_911, 3_000, u16::MAX] {
            for (day, minute) in [(0, 1_000), (3, 1_000), (3, 200)] {
                with_estimate(commute, |cache| {
                    assert_eq!(
                        ask_schedule(cache, &clock, 0, (day, minute)),
                        Some((1, 1)),
                        "commute {commute}, day {day}, minute {minute}"
                    );
                    assert_eq!(*cache.next_departure_day, day);
                    assert_eq!(*cache.next_departure_minute, 0);
                });
            }
        }
    }

    #[test]
    fn a_worker_who_reaches_work_the_evening_before_stays_for_the_shift() {
        let clock = one_shift_clock();
        with_estimate(590, |cache| {
            assert_eq!(ask_schedule(cache, &clock, 1, (3, 1_400)), None);
            assert_eq!(*cache.next_departure_day, 4);
            assert_eq!(*cache.next_departure_minute, 1_020);
            assert_eq!(*cache.next_departure_target_building, 0);
        });
    }

    #[test]
    fn every_worker_at_home_leaves_by_the_true_minute_or_at_once() {
        let clock = one_shift_clock();
        for commute in (1..2_200).step_by(23) {
            let lead = i64::from(commute) + 10;
            for start in (0..MINUTES_PER_DAY).step_by(17) {
                let day = 5_i64;
                let now = day * 1_440 + i64::from(start);
                // The first arrival whose window is still open, and its true minute.
                let arrival_day = if start < 481 { day } else { day + 1 };
                let due = (arrival_day * 1_440 + 480 - lead).max(now);
                with_estimate(commute, |cache| {
                    let mut left = None;
                    for clock_minute in now..now + 2 * 1_440 {
                        let at = (
                            (clock_minute / 1_440) as u32,
                            (clock_minute % 1_440) as u16,
                        );
                        if ask_schedule(cache, &clock, 0, at).is_some() {
                            left = Some(clock_minute);
                            break;
                        }
                    }
                    assert_eq!(left, Some(due), "commute {commute}, start {start}");
                });
            }
        }
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
    fn due_departures_leave_at_their_seeds_slot_of_the_minute() {
        use crate::simulation::core::scheduler::SLOTS_PER_MINUTE;
        let tuning =
            crate::simulation::economy::definitions::load_runtime_economy_tuning().unwrap();
        let catalog =
            crate::simulation::economy::definitions::load_runtime_economy_catalog().unwrap();
        let allocator = BuildingAllocator::new();
        let network = TransitNetwork::new();
        let graph = RegionGraph::new();
        let pathfind_count = AtomicU32::new(0);
        let at = |minute: u16, slot: u64| {
            let mut time = test_clock(1, minute);
            time.set_absolute_slot_for_test(u64::from(minute) * SLOTS_PER_MINUTE + slot);
            time
        };
        let seed = (1..)
            .find(|&seed| departure_slot_in_minute(480, seed) >= 2)
            .unwrap();
        let slot = departure_slot_in_minute(480, seed);
        for (time, due) in [
            (at(479, SLOTS_PER_MINUTE - 1), false),
            (at(480, 0), false),
            (at(480, slot - 1), false),
            (at(480, slot), true),
            (at(481, 0), true),
        ] {
            with_schedule_cache(|cache| {
                *cache.next_departure_day = 1;
                *cache.next_departure_minute = 480;
                *cache.next_departure_origin_building = 0;
                *cache.next_departure_target_building = 1;
                *cache.next_departure_activity = ACTIVITY_WORK;
                let trip = maybe_schedule_work_trip(
                    0,
                    ACTIVITY_HOME,
                    0,
                    1,
                    false,
                    seed,
                    cache,
                    0.0,
                    &time,
                    &allocator,
                    &network,
                    &graph,
                    &pathfind_count,
                    &tuning.operational_clock,
                    &catalog,
                    None,
                );
                assert_eq!(
                    trip.is_some(),
                    due,
                    "minute {} slot {}",
                    time.minute_of_day,
                    time.slot_in_minute()
                );
            });
        }
        // The day's last minute leaves at its first slot, so no frame can carry it past midnight.
        assert_eq!(departure_slot_in_minute(MINUTES_PER_DAY - 1, seed), 0);

        // Agents due in one minute of a shift window leave over all of its slots.
        let profile = &tuning.operational_clock.work_profiles[0];
        let window = &profile.arrival_windows[0];
        let minute = stable_minute_in_window(profile, window, 0);
        let mut per_slot = [0_u32; SLOTS_PER_MINUTE as usize];
        for seed in 0..200_000 {
            if stable_minute_in_window(profile, window, seed) == minute {
                per_slot[departure_slot_in_minute(minute, seed) as usize] += 1;
            }
        }
        let total: u32 = per_slot.iter().sum();
        let fair = total / SLOTS_PER_MINUTE as u32;
        assert!(fair >= 50, "{total} seeds in the minute");
        assert!(
            per_slot
                .iter()
                .all(|&count| count >= fair / 2 && count <= fair * 2),
            "{per_slot:?}"
        );
        assert_eq!(pathfind_count.load(std::sync::atomic::Ordering::Relaxed), 0);
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
