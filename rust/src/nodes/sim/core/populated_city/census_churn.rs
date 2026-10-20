// SPDX-License-Identifier: GPL-2.0-only

//! What happens during the funding census's window, the hour's second half (`SIM-02-I`): per
//! hour, the appends and structural changes there, census restarts, planning passes whose
//! published books were not valid for the live city and so were skipped, and construction
//! epochs, opened or skipped. Counts come from the test-only
//! [`crate::debug::work`]
//! counters, so run one of these tests alone.

use super::super::frame::FRAME_DT_S;
use super::super::state::SimCore;
use super::PopulatedCity;
use crate::debug::work::{WORK_COUNT, Work, snapshot, window_snapshot};
use crate::simulation::core::scheduler::SLOTS_PER_HOUR;

const COUNTED: [(Work, &str); 15] = [
    (Work::BuildingsPlaced, "buildings_placed"),
    (Work::BuildingsRemoved, "buildings_removed"),
    (Work::HouseholdsRemoved, "households_removed"),
    (Work::StructuralChanges, "structural_changes"),
    (Work::CensusRestarts, "census_restarts"),
    (Work::LiveCensuses, "live_censuses"),
    (Work::PlanningPasses, "planning_passes"),
    (Work::ResidentialPlanningSkips, "residential_planning_skips"),
    (Work::CommercialPlanningSkips, "commercial_planning_skips"),
    (Work::IndustrialPlanningSkips, "industrial_planning_skips"),
    (Work::ActionsPlanningSkips, "actions_planning_skips"),
    (Work::ConstructionEpochs, "construction_epochs"),
    (Work::ConstructionEpochSkips, "construction_epoch_skips"),
    (Work::ReproofDrops, "reproof_drops"),
    (Work::CensusBuildings, "census_buildings"),
];

fn delta(after: &[u64; WORK_COUNT], before: &[u64; WORK_COUNT], work: Work) -> u64 {
    after[work as usize] - before[work as usize]
}

/// Steps `core` frame by frame over `hours` whole hours and prints one `CENSUS_CHURN` JSON line:
/// totals and window totals of each counter, and the hours with any structural change in the
/// window or any planning pass or construction epoch skipped on books not valid for the live
/// city. Returns the skips.
pub(in crate::nodes::sim::core) fn report_census_churn(
    core: &mut SimCore,
    hours: u32,
    label: &str,
) -> u64 {
    let hour = |core: &SimCore| core.time.absolute_slot() / SLOTS_PER_HOUR;
    // Start at an hour boundary so every counted hour is whole.
    let first = hour(core);
    while hour(core) == first {
        core.step_frame(FRAME_DT_S);
    }
    let (start, start_window) = (snapshot(), window_snapshot());
    let mut hours_with = [0_u32; 2];
    let mut current = hour(core);
    let (mut before, mut before_window) = (start, start_window);
    let mut counted = 0;
    while counted < hours {
        core.step_frame(FRAME_DT_S);
        if hour(core) == current {
            continue;
        }
        current = hour(core);
        counted += 1;
        let (after, after_window) = (snapshot(), window_snapshot());
        for (seen, any) in hours_with.iter_mut().zip([
            delta(&after_window, &before_window, Work::StructuralChanges) > 0,
            skips(&after, &before) > 0,
        ]) {
            *seen += u32::from(any);
        }
        (before, before_window) = (after, after_window);
    }
    let (end, end_window) = (snapshot(), window_snapshot());
    let totals: serde_json::Map<_, _> = COUNTED
        .iter()
        .map(|&(work, name)| {
            (
                name.to_string(),
                serde_json::json!([
                    delta(&end, &start, work),
                    delta(&end_window, &start_window, work)
                ]),
            )
        })
        .collect();
    eprintln!(
        "CENSUS_CHURN {}",
        serde_json::json!({
            "label": label,
            "hours": hours,
            "buildings": core.allocator.buildings.len(),
            "households": core.households.households.len(),
            "totals_and_window": totals,
            "hours_with_window_structural_change": hours_with[0],
            "hours_with_skip": hours_with[1],
        })
    );
    skips(&end, &start)
}

// Planning slots and construction epochs skipped between two snapshots.
fn skips(after: &[u64; WORK_COUNT], before: &[u64; WORK_COUNT]) -> u64 {
    [
        Work::ResidentialPlanningSkips,
        Work::CommercialPlanningSkips,
        Work::IndustrialPlanningSkips,
        Work::ActionsPlanningSkips,
        Work::ConstructionEpochSkips,
    ]
    .into_iter()
    .map(|work| delta(after, before, work))
    .sum()
}

fn env_u32(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

#[test]
#[ignore = "SIM-02-I census window churn on a populated city; run alone"]
fn populated_census_window_churn() {
    // Honors explicit debug-channel requests, so the queues' hourly service can be logged.
    crate::debug::init();
    let residents = env_u32("METRUM_CHURN_RESIDENTS", 10_000);
    let hours = env_u32("METRUM_CHURN_HOURS", 24);
    let mut city = PopulatedCity::build(residents);
    // `fiscal_stress` raises business property tax as the behaviour envelope's workload does,
    // so firms fail and households leave.
    let workload = std::env::var("METRUM_CHURN_WORKLOAD").unwrap_or_else(|_| "baseline".into());
    match workload.as_str() {
        "baseline" => {}
        "fiscal_stress" => super::envelope::apply_fiscal_stress(&mut city),
        other => panic!("unknown METRUM_CHURN_WORKLOAD {other}"),
    }
    let skips = report_census_churn(
        &mut city.core,
        hours,
        &format!("populated-{residents}-{workload}"),
    );
    // Planning and permits lend the census published at their slot (`SIM-02-I`).
    assert_eq!(skips, 0);
}

// Wall time of the census steps so far, per step, in milliseconds.
fn step_millis() -> Vec<f64> {
    crate::simulation::economy::hiring::STEP_NANOS
        .iter()
        .map(|nanos| nanos.load(std::sync::atomic::Ordering::Relaxed) as f64 / 1e6)
        .collect()
}

/// Diagnostic, not acceptance timing: the census of one window on a populated city, stepped
/// slot by slot apart from the rest of the slot's systems. Prints per-step totals, the slowest
/// slots and the one-run cost of the same census.
#[test]
#[ignore = "SIM-02-I census step costs on a populated city; run alone, release"]
fn populated_census_step_costs() {
    let residents = env_u32("METRUM_CHURN_RESIDENTS", 10_000);
    let mut city = PopulatedCity::build(residents);
    let core = &mut city.core;
    // Warm: one ordinary hour, then up to the last fill slot of the next.
    let hour = core.time.absolute_slot() / SLOTS_PER_HOUR;
    while core.time.absolute_slot() / SLOTS_PER_HOUR < hour + 2 {
        core.step_frame(FRAME_DT_S);
    }
    while core.time.absolute_slot() % SLOTS_PER_HOUR != SLOTS_PER_HOUR / 2 - 1 {
        core.step_frame(FRAME_DT_S / 16.0);
    }
    let first = core.time.absolute_slot() + 1;
    let before = step_millis();
    let mut slots = Vec::new();
    for slot in first..first + SLOTS_PER_HOUR / 2 {
        let started = std::time::Instant::now();
        core.advance_census(slot);
        slots.push((
            started.elapsed().as_secs_f64() * 1000.0,
            slot % SLOTS_PER_HOUR,
        ));
    }
    assert!(core.households.census_run.is_done());
    let stepped: Vec<f64> = step_millis()
        .iter()
        .zip(&before)
        .map(|(after, before)| after - before)
        .collect();
    let started = std::time::Instant::now();
    let once = core
        .households
        .census_of_front_for_test(&core.transit_network, &core.region_graph);
    let once_ms = started.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(
        once.capacities,
        core.households.census_run.funding().capacities
    );
    let total: f64 = slots.iter().map(|(ms, _)| ms).sum();
    slots.sort_by(|a, b| b.0.total_cmp(&a.0));
    let steps: serde_json::Map<_, _> = crate::simulation::economy::hiring::STEP_NAMES
        .iter()
        .zip(&stepped)
        .filter(|(_, ms)| **ms > 0.0)
        .map(|(name, ms)| {
            (
                name.to_string(),
                serde_json::json!((ms * 1000.0).round() / 1000.0),
            )
        })
        .collect();
    eprintln!(
        "CENSUS_STEPS {}",
        serde_json::json!({
            "residents": residents,
            "buildings": core.allocator.buildings.len(),
            "households": core.households.households.len(),
            "passes": core.households.census_run.funding().closure_passes,
            "window_total_ms": total,
            "one_run_fresh_storage_ms": once_ms,
            "slowest_slots_ms_and_chunk": &slots[..slots.len().min(8)],
            "median_slot_ms": slots[slots.len() / 2].0,
            "step_ms": steps,
        })
    );
}

/// Diagnostic, not acceptance timing: the census of `METRUM_CENSUS_WINDOWS` windows (default
/// 2) of a prepared `PERF-07` city (`METRUM_PERF_RESIDENTS`, `METRUM_PERF_CACHE`), stepped slot
/// by slot apart from the rest of the slot's systems. Prints one `CENSUS_SLOTS` line a window:
/// its slowest slots with their steps, the last slot with census work and the window total.
#[test]
#[ignore = "SIM-02-K census slot costs on a prepared PERF-07 city; run alone, release"]
fn perf07_census_slot_costs() {
    super::contract::timing_mode();
    let residents = super::contract::residents();
    let mut city = super::snapshot::load_snapshot(residents, &super::contract::cache_dir());
    let core = &mut city.core;
    core.time.speed_multiplier = 32.0;
    let windows = env_u32("METRUM_CENSUS_WINDOWS", 2);
    for window in 0..windows {
        // Up to the last fill slot of the hour in progress.
        let target =
            (core.time.absolute_slot() / SLOTS_PER_HOUR) * SLOTS_PER_HOUR + SLOTS_PER_HOUR / 2 - 1;
        let target = if core.time.absolute_slot() > target {
            target + SLOTS_PER_HOUR
        } else {
            target
        };
        while core.time.absolute_slot() + 16 < target {
            core.step_frame(FRAME_DT_S);
        }
        while core.time.absolute_slot() < target {
            core.step_frame(FRAME_DT_S / 16.0);
        }
        assert_eq!(core.time.absolute_slot(), target);
        let first = target + 1;
        let mut slots = Vec::new();
        let window_before = step_millis();
        for slot in first..first + SLOTS_PER_HOUR / 2 {
            let before = step_millis();
            let started = std::time::Instant::now();
            core.advance_census(slot);
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            let steps: Vec<(&str, f64)> = crate::simulation::economy::hiring::STEP_NAMES
                .iter()
                .zip(step_millis().iter().zip(&before))
                .map(|(name, (after, before))| (*name, after - before))
                .filter(|(_, ms)| *ms >= 0.05)
                .map(|(name, ms)| (name, (ms * 100.0).round() / 100.0))
                .collect();
            slots.push((ms, slot % SLOTS_PER_HOUR, steps));
        }
        assert!(core.households.census_run.is_done());
        let step_totals: serde_json::Map<_, _> = crate::simulation::economy::hiring::STEP_NAMES
            .iter()
            .zip(step_millis().iter().zip(&window_before))
            .filter(|(_, (after, before))| *after - *before >= 0.01)
            .map(|(name, (after, before))| {
                (
                    name.to_string(),
                    serde_json::json!(((after - before) * 100.0).round() / 100.0),
                )
            })
            .collect();
        let total: f64 = slots.iter().map(|(ms, ..)| ms).sum();
        let busy = slots.iter().filter(|(ms, ..)| *ms >= 0.05).count();
        let last = slots
            .iter()
            .rev()
            .find(|(ms, ..)| *ms >= 0.05)
            .map_or(0, |(_, chunk, _)| *chunk);
        let over_1 = slots.iter().filter(|(ms, ..)| *ms > 1.0).count();
        let mut sorted: Vec<_> = slots.iter().collect();
        sorted.sort_by(|a, b| b.0.total_cmp(&a.0));
        let worst: Vec<_> = sorted[..12]
            .iter()
            .map(|(ms, chunk, steps)| {
                serde_json::json!([(ms * 100.0).round() / 100.0, chunk, steps])
            })
            .collect();
        eprintln!(
            "CENSUS_SLOTS {}",
            serde_json::json!({
                "residents": residents,
                "window": window,
                "buildings": core.allocator.buildings.len(),
                "households": core.households.households.len(),
                "passes": core.households.census_run.funding().closure_passes,
                "window_total_ms": (total * 100.0).round() / 100.0,
                "slots_with_work": busy,
                "last_slot_with_work": last,
                "slots_over_1ms": over_1,
                "max_ms": (sorted[0].0 * 100.0).round() / 100.0,
                "step_ms": step_totals,
                "worst": worst,
            })
        );
        // Into the next hour, past the publication.
        while core.time.absolute_slot() < first + SLOTS_PER_HOUR / 2 + 1 {
            core.step_frame(FRAME_DT_S);
        }
    }
}
