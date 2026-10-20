// SPDX-License-Identifier: GPL-2.0-only

//! Short, snapshot-backed economy checks. Run serially through `tools/perf_contract.py`.

use super::snapshot;
use crate::debug::work::{WORK_COUNT, WORK_NAMES, phases, snapshot as work_snapshot};
use crate::simulation::core::scheduler::{SLOTS_PER_DAY, SLOTS_PER_HOUR};
use std::path::PathBuf;
use std::time::Instant;

pub(super) fn cache_dir() -> PathBuf {
    std::env::var_os("METRUM_PERF_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/perf-contract"))
}

pub(super) fn residents() -> u32 {
    std::env::var("METRUM_PERF_RESIDENTS")
        .unwrap_or_else(|_| "10000".into())
        .parse()
        .expect("METRUM_PERF_RESIDENTS must be an integer")
}

fn counts_since(before: [u64; WORK_COUNT]) -> serde_json::Value {
    serde_json::Value::Object(
        WORK_NAMES
            .iter()
            .zip(work_snapshot().iter().zip(before))
            .map(|(name, (after, before))| (name.to_string(), (after - before).into()))
            .collect(),
    )
}

// Runtime guard: the ignored check must still compile in a debug test build.
#[allow(clippy::assertions_on_constants)]
pub(super) fn timing_mode() {
    assert!(
        !cfg!(debug_assertions),
        "performance contracts require --release"
    );
    // Ignored library tests bypass GDExtension startup. Honor explicit debug-channel
    // requests so short phase diagnostics can use the existing funding/search traces.
    crate::debug::init();
    // Full-book fingerprints are correctness oracles, not gameplay work. Targeted regression
    // tests retain them; this serialized manual runner disables only that extra test work.
    crate::simulation::economy::demand::VERIFY_LIVE_PASSES
        .store(false, std::sync::atomic::Ordering::Relaxed);
    // So are the operational hour's whole-city consistency checks (`SIM-02-K`).
    crate::debug::work::set_city_checks(false);
}

#[test]
#[ignore = "one-time PERF-01 snapshot generation and exact continuation check"]
fn perf_contract_prepare() {
    timing_mode();
    let elapsed = snapshot::ensure_snapshot(residents(), &cache_dir());
    eprintln!(
        "PERF_PREPARE {}",
        serde_json::json!({"residents":residents(),"generation_s":elapsed.map(|value| value.as_secs_f64())})
    );
}

#[test]
#[ignore = "one-time PERF-01 fiscal snapshot generation and exact continuation check"]
fn perf_contract_fiscal_prepare() {
    timing_mode();
    let elapsed = snapshot::ensure_fiscal_snapshot(residents(), &cache_dir());
    eprintln!(
        "PERF_FISCAL_PREPARE {}",
        serde_json::json!({"residents":residents(),"generation_s":elapsed.map(|value| value.as_secs_f64())})
    );
}

#[test]
#[ignore = "manual save-plan diagnosis; requires METRUM_PERF_DIAG_SAVE"]
fn perf_contract_saved_agent_plans() {
    let path = std::env::var("METRUM_PERF_DIAG_SAVE").expect("diagnostic save path");
    let mut core = super::SimCore {
        benchmark_mode: true,
        ..super::SimCore::new(super::WorldConfig::new(2048.0, 2048.0, 40.0, 10.0))
    };
    super::install_bootstrap_assets(&mut core);
    core.load_game_internal(&path).unwrap();
    let connection =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let mut statement = connection.prepare(
        "SELECT agent_id, planned_target_building, planned_activity FROM agents ORDER BY agent_id",
    ).unwrap();
    let mut rows = statement.query([]).unwrap();
    while let Some(row) = rows.next().unwrap() {
        let id = row.get::<_, i64>(0).unwrap() as usize;
        let target = usize::try_from(row.get::<_, i64>(1).unwrap()).unwrap_or(usize::MAX);
        let activity = row.get::<_, u8>(2).unwrap();
        assert_eq!(
            (
                core.agents.planned_target_building[id],
                core.agents.planned_activity[id]
            ),
            (target, activity),
            "agent {id} plan changed during load before any tick",
        );
    }
}

#[test]
#[ignore = "short PERF-01 full-day release contract on a prepared snapshot"]
fn perf_contract_day() {
    run_day(false);
}

#[test]
#[ignore = "short PERF-01 hourly-churn release contract on a prepared fiscal snapshot"]
fn perf_contract_fiscal_day() {
    run_day(true);
}

fn run_day(fiscal_stress: bool) {
    timing_mode();
    let started = Instant::now();
    let mut city = if fiscal_stress {
        snapshot::load_fiscal_snapshot(residents(), &cache_dir())
    } else {
        snapshot::load_snapshot(residents(), &cache_dir())
    };
    let load_s = started.elapsed().as_secs_f64();
    let day = city.core.time.day_index;
    phases::begin();
    let before = work_snapshot();
    let mut hourly_ms = 0.0;
    let mut daily_ms = 0.0;
    let mut hours = 0;
    let mut days = 0;
    let mut building_churn = [[0_u64; 2]; 24];
    for churn in &mut building_churn {
        let before_hour = work_snapshot();
        // Each window holds one hour's planning slots and building actions (`SIM-02-I`).
        let hour = city.advance_hour_through_actions();
        let after_hour = work_snapshot();
        for (slot, counter) in churn.iter_mut().zip([
            crate::debug::work::Work::BuildingsPlaced,
            crate::debug::work::Work::BuildingsRemoved,
        ]) {
            *slot = after_hour[counter as usize] - before_hour[counter as usize];
        }
        if hour.demand_pass {
            hourly_ms += hour.hourly.as_secs_f64() * 1000.0;
            hours += 1;
        }
        if let Some(daily) = hour.daily {
            daily_ms += daily.as_secs_f64() * 1000.0;
            days += 1;
        }
    }
    assert_eq!((hours, days), (23, 1));
    let counts = counts_since(before);
    let phase_report = phases::finish();
    eprintln!(
        "{} {}",
        if fiscal_stress {
            "PERF_FISCAL_DAY"
        } else {
            "PERF_DAY"
        },
        serde_json::json!({
            "workload":if fiscal_stress { "fiscal_stress" } else { "baseline" },
            "day":day,
            "residents":residents(), "load_s":load_s, "total_s":started.elapsed().as_secs_f64(),
            "hourly_ms":hourly_ms / f64::from(hours),"daily_ms":daily_ms,
            "households":city.record().households, "record":format!("{:?}",city.record()),
            "counts":counts,"phases":phase_report,"building_churn":building_churn,
        })
    );
}

#[test]
#[ignore = "bounded PERF-01 fixture diagnosis; seven days from an existing snapshot"]
fn perf_contract_churn_scan() {
    timing_mode();
    let city = snapshot::load_snapshot(residents(), &cache_dir());
    // Diagnose the existing fixture without regenerating it or changing economic inputs.
    // This is deliberately separate from the one-day timing contract.
    scan_churn(city, 7);
}

#[test]
#[ignore = "one-time PERF-01 early fixture diagnosis, including city generation"]
fn perf_contract_early_churn_scan() {
    timing_mode();
    let started = Instant::now();
    let mut city = super::PopulatedCity::build(residents());
    // Start at midnight after the ordinary warm-up, matching snapshot boundaries.
    while city.core.time.minute_of_day != 0 {
        city.advance_hour();
    }
    eprintln!(
        "PERF_CHURN_PREPARE {}",
        serde_json::json!({"residents":residents(),"generation_s":started.elapsed().as_secs_f64()})
    );
    scan_churn(city, 9);
}

#[test]
#[ignore = "manual PERF-01 fiscal stress fixture diagnosis; not an acceptance workload"]
fn perf_contract_fiscal_churn_scan() {
    timing_mode();
    let started = Instant::now();
    let city = snapshot::fiscal_stress_city(residents(), &cache_dir());
    eprintln!(
        "PERF_CHURN_PREPARE {}",
        serde_json::json!({
            "residents":residents(), "variant":"property_tax_stress",
            "load_and_policy_s":started.elapsed().as_secs_f64(),
            "record":format!("{:?}",city.record()),
        })
    );
    scan_churn(city, 14);
}

fn scan_churn(mut city: super::PopulatedCity, days: u32) {
    for _ in 0..days {
        let day = city.core.time.day_index;
        let mut building_churn = [[0_u64; 2]; 24];
        for churn in &mut building_churn {
            let before = work_snapshot();
            city.advance_hour_through_actions();
            let after = work_snapshot();
            for (slot, counter) in churn.iter_mut().zip([
                crate::debug::work::Work::BuildingsPlaced,
                crate::debug::work::Work::BuildingsRemoved,
            ]) {
                *slot = after[counter as usize] - before[counter as usize];
            }
        }
        eprintln!(
            "PERF_CHURN {}",
            serde_json::json!({
                "residents":residents(), "day":day, "building_churn":building_churn,
                "record":format!("{:?}",city.record()),
            })
        );
    }
}

#[test]
#[ignore = "PERF-02 isolated phase on frozen snapshot inputs"]
fn perf_contract_phase() {
    timing_mode();
    let name = std::env::var("METRUM_PERF_PHASE").expect("METRUM_PERF_PHASE is required");
    let load = Instant::now();
    let mut city = snapshot::load_snapshot(residents(), &cache_dir());
    let load_s = load.elapsed().as_secs_f64();
    let core = &mut city.core;
    let day = core.time.day_index;
    // Prepare phase-specific inputs outside the timed interval. Each invocation starts from
    // the same saved world; setup does not depend on the previous selected phase.
    let setup = Instant::now();
    if matches!(name.as_str(), "lots" | "settlement") {
        // Loading invalidates all derived lots. A local edit or steady-state settlement
        // must not accidentally measure that one-time whole-map reconstruction.
        core.prepare_cell_lots_internal();
    }
    if name == "lots" {
        core.zoning
            .mark_cell_lots_dirty(crate::simulation::zoning::cells::CellBounds {
                min: glam::DVec2::new(100.0, -400.0),
                max: glam::DVec2::new(120.0, -380.0),
            });
    }
    let catalog = crate::simulation::economy::definitions::load_runtime_economy_catalog().unwrap();
    let tuning = crate::simulation::economy::definitions::load_runtime_economy_tuning().unwrap();
    let mut route_scratch = crate::simulation::pathing::cch::CchQueryScratch::default();
    if name == "routing" {
        route_scratch.prepare(&core.transit_network.cch_graph, &core.region_graph);
    }
    if name == "execution" {
        // The epoch reads the workforce its slot's planning pass counted.
        core.demand.count_workforce_for_test(
            &core.allocator,
            &core.households,
            &core.fiscal_policy,
        );
    }
    let setup_s = setup.elapsed().as_secs_f64();
    phases::begin();
    let before = work_snapshot();
    let started = Instant::now();
    match name.as_str() {
        // One hour's census as the slots run it: the input copy over the hour's first half,
        // the census's steps over its second half and the next hour's publication.
        "census" => {
            let hour = core.time.absolute_slot() / SLOTS_PER_HOUR * SLOTS_PER_HOUR;
            for slot in core.time.absolute_slot() + 1..hour + SLOTS_PER_HOUR {
                core.allocator.now_slot = slot;
                core.advance_census_inputs(slot);
                core.advance_census(slot);
            }
            core.allocator.now_slot = hour + SLOTS_PER_HOUR;
            core.publish_census(hour + SLOTS_PER_HOUR);
        }
        "planning" => core.demand.run_planning_hour_for_test(
            &core.allocator,
            &mut core.households,
            &core.region_graph,
            &core.zoning,
            core.treasury.balance,
            &core.fiscal_policy,
            &core.transit_network,
            &mut core.logistics,
            crate::simulation::buildings::allocator::BuildingSiteEnvironment {
                road_surface: &core.transit_network.road_surface,
                terrain: &core.heightmap,
            },
            60,
        ),
        "execution" => {
            // The planning slot's construction epoch over the whole queue and back, as the
            // hour's first slot opens it after planning (`SIM-02-I`).
            assert!(
                core.demand.open_construction_epoch(
                    &core.allocator,
                    &mut core.households,
                    &core.zoning,
                    &core.region_graph,
                    &core.transit_network,
                    &mut core.logistics,
                    &core.fiscal_policy,
                    core.treasury.balance,
                    crate::nodes::sim::core::state::absolute_operational_minute(day, 60),
                ),
                "the snapshot's published census is current for the epoch"
            );
            core.demand.close_construction_epoch(
                &mut core.households,
                &core.allocator,
                &core.region_graph,
            );
        }
        // One hour of freight requests as the slots run them: the freight checks of each
        // slot's hourly building phase, then the queue's quota.
        "logistics" => {
            let start = core.time.absolute_slot() + 1;
            let mut phase = Vec::new();
            for slot in start..start + SLOTS_PER_HOUR {
                core.allocator.now_slot = slot;
                let handles = &mut core.allocator.building_handles;
                handles.register_appended(core.allocator.buildings.len());
                handles
                    .phases()
                    .hourly((slot % SLOTS_PER_HOUR) as u32, &mut phase);
                for &handle in &phase {
                    if let Some(building) = core.allocator.building_handles.resolve(handle) {
                        core.logistics.request_freight_at_phase(
                            slot,
                            building,
                            &core.allocator,
                            &catalog,
                            &tuning,
                        );
                    }
                }
                core.serve_freight_request_queue(
                    slot,
                    crate::nodes::sim::core::schedule::FREIGHT_REQUEST_QUOTA as usize,
                );
            }
        }
        "lots" => {
            std::hint::black_box(core.prepare_cell_lots_internal());
        }
        // One day of settlement as the slots run it: each slot's daily building, household and
        // agent phases and environment sweep chunk, then the next day's calendar close.
        "settlement" => {
            let first = u64::from(day - 1) * SLOTS_PER_DAY;
            for slot in first..first + SLOTS_PER_DAY {
                core.allocator.now_slot = slot;
                core.run_building_days(slot, &mut ());
                core.run_household_days(slot, &mut ());
                core.run_agent_days(slot, &mut ());
                core.advance_environment_sweep(slot);
            }
            core.allocator.now_slot = first + SLOTS_PER_DAY;
            core.close_calendar_day(day + 1, &mut ());
        }
        "routing" => {
            let nodes = core.region_graph.node_count() as u32;
            assert!(nodes > 1);
            // Fixed integer sequence, independent of RNG libraries and city iteration order.
            for i in 0..4096_u32 {
                std::hint::black_box(core.transit_network.cch_graph.find_cost(
                    i.wrapping_mul(2654435761) % nodes,
                    i.wrapping_mul(2246822519).wrapping_add(1) % nodes,
                    usize::MAX,
                    usize::MAX,
                    &core.region_graph,
                    crate::simulation::network::types::TransitFlags::CAR,
                    &mut route_scratch,
                ));
            }
        }
        _ => panic!("unknown contract phase: {name}"),
    }
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    let counts = counts_since(before);
    eprintln!(
        "PERF_PHASE {}",
        serde_json::json!({
            "phase":name,"residents":residents(),"load_s":load_s,"setup_s":setup_s,"ms":elapsed_ms,
            "counts":counts,"phases":phases::finish(),"record":format!("{:?}",city.record()),
        })
    );
}
