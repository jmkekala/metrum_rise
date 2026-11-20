// SPDX-License-Identifier: GPL-3.0-or-later

//! Congestion-aware car routing in a played town (`TRAFFIC-17`): the periodic customization is
//! the same for any worker count, and a game saved while one is pending continues bit for bit.

use super::save_cases::{differing_tables, save};
use super::*;
use crate::nodes::sim::core::schedule::{CONGESTION_LATENCY, CONGESTION_LAUNCH_SLOT};
use crate::simulation::core::scheduler::SLOTS_PER_HOUR;
use std::path::PathBuf;

fn temp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "metrum-congestion-{name}-{}.sqlite",
        std::process::id()
    ))
}

fn assert_same_saves(first: &Path, second: &Path, what: &str) {
    assert!(
        std::fs::read(first).unwrap() == std::fs::read(second).unwrap(),
        "{what}: tables differing in storage order: {:?}",
        differing_tables(first, second)
    );
}

// Whether a congestion customization launched this quarter hour waits for its adoption.
fn pending(core: &SimCore) -> bool {
    let of_quarter = core.time.absolute_slot() % (SLOTS_PER_HOUR / 4);
    (CONGESTION_LAUNCH_SLOT..CONGESTION_LAUNCH_SLOT + CONGESTION_LATENCY).contains(&of_quarter)
}

#[test]
fn congestion_routing_is_identical_for_one_and_eight_workers() {
    let saves = [1, 8].map(|workers| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
            .install(|| {
                let mut town = GrowthScenario::starter_town();
                town.run_days(3);
                let network = &town.core.transit_network;
                // About 4 adoptions a game hour; cars queue somewhere in three days.
                assert!(network.cch_graph.congestion_revision > 200);
                assert!(
                    network
                        .lane_system
                        .lanes
                        .iter()
                        .any(|lane| lane.route_delay_s > 0.0)
                );
                let path = temp(&format!("workers-{workers}"));
                save(&mut town.core, &path);
                path
            })
    });
    assert_same_saves(&saves[0], &saves[1], "1 and 8 workers");
    for path in saves {
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn a_game_saved_with_a_customization_pending_continues_bit_for_bit() {
    let mut source = GrowthScenario::starter_town();
    source.run_days(2);
    while !pending(&source.core) {
        source.core.step_frame(FRAME_DT_S);
    }
    assert!(source.core.transit_network.pending_congestion().is_some());
    let saved = temp("pending");
    save(&mut source.core, &saved);
    let mut loaded = GrowthScenario::starter_town();
    loaded
        .core
        .load_game_internal(saved.to_str().unwrap())
        .unwrap();
    loaded.core.time.speed_multiplier = source.core.time.speed_multiplier;
    for core in [&source.core, &loaded.core] {
        assert!(core.transit_network.pending_congestion().is_some());
    }
    assert_eq!(
        source.core.transit_network.cch_graph.congestion_weights(),
        loaded.core.transit_network.cch_graph.congestion_weights()
    );
    // Two game hours: the pending adoption and eight more customizations.
    let end = source.core.time.absolute_slot() + 2 * SLOTS_PER_HOUR;
    while source.core.time.absolute_slot() < end {
        source.core.step_frame(FRAME_DT_S);
        loaded.core.step_frame(FRAME_DT_S);
    }
    let (first, second) = (temp("continued"), temp("continued-loaded"));
    save(&mut source.core, &first);
    save(&mut loaded.core, &second);
    assert_same_saves(&first, &second, "the saved and the loaded game");
    for path in [saved, first, second] {
        std::fs::remove_file(path).unwrap();
    }
}

// The game a pool of `workers` threads continues from the format-118 save `path` for two game
// hours, loaded into `fresh()`, saved.
fn continue_in_pool(
    workers: usize,
    path: &Path,
    fresh: fn() -> SimCore,
    speed: f32,
    name: &str,
) -> PathBuf {
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap()
        .install(|| {
            let core = &mut fresh();
            core.load_game_internal(path.to_str().unwrap()).unwrap();
            core.time.speed_multiplier = speed;
            assert!(
                core.transit_network
                    .cch_graph
                    .congestion_weights()
                    .is_none()
            );
            assert!(core.transit_network.pending_congestion().is_none());
            assert!(
                core.transit_network
                    .lane_system
                    .lanes
                    .iter()
                    .all(|lane| lane.route_delay_s == 0.0)
            );
            // The first running frame customizes from those zero delays: free flow exactly.
            core.step_frame(FRAME_DT_S);
            let graph = &core.region_graph;
            let weights = core.transit_network.cch_graph.congestion_weights().unwrap();
            assert!(graph.edges().iter().enumerate().all(|(id, edge)| {
                weights[2 * id] == edge.base_cost && weights[2 * id + 1] == edge.base_cost
            }));
            let revision = core.transit_network.cch_graph.congestion_revision;
            let end = core.time.absolute_slot() + 2 * SLOTS_PER_HOUR;
            while core.time.absolute_slot() < end {
                core.step_frame(FRAME_DT_S);
            }
            assert!(core.transit_network.cch_graph.congestion_revision >= revision + 8);
            let saved = temp(&format!("{name}-continued-{workers}"));
            save(core, &saved);
            saved
        })
}

#[test]
fn a_format_118_save_loads_on_free_flow_and_continues_alike_for_one_and_eight_workers() {
    use crate::simulation::save::schema::PRE_CONGESTION_SAVE_VERSION;
    let mut town = GrowthScenario::starter_town();
    town.run_days(2);
    // The save the pre-change build wrote: the same tables without the congestion state.
    let legacy = temp("format-118");
    save(&mut town.core, &legacy);
    rusqlite::Connection::open(&legacy)
        .unwrap()
        .execute_batch(&format!(
            "ALTER TABLE saved_lanes DROP COLUMN route_delay_s;
             DROP TABLE congestion_routing;
             UPDATE save_meta SET version = {PRE_CONGESTION_SAVE_VERSION};"
        ))
        .unwrap();
    let speed = town.core.time.speed_multiplier;
    let fresh = || GrowthScenario::starter_town().core;
    let saves =
        [1, 8].map(|workers| continue_in_pool(workers, &legacy, fresh, speed, "format-118"));
    assert_same_saves(&saves[0], &saves[1], "a format-118 save on 1 and 8 workers");
    for path in saves.into_iter().chain([legacy]) {
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
#[ignore = "loads a format-118 save written by the pre-change build (METRUM_LEGACY_SAVE, a copy)"]
fn a_format_118_save_of_the_pre_change_build_loads_and_continues() {
    // A populated-city snapshot the `dev` 46aea5b1 binary prepared, or any format-118 save of
    // bootstrap assets; opened read-only by the load.
    let path = PathBuf::from(std::env::var("METRUM_LEGACY_SAVE").expect("METRUM_LEGACY_SAVE"));
    let speed = SIMULATION_SPEED_STEPS[SIMULATION_SPEED_STEPS.len() - 1];
    let saves =
        [1, 8].map(|workers| continue_in_pool(workers, &path, populated_core, speed, "legacy"));
    assert_same_saves(
        &saves[0],
        &saves[1],
        "the pre-change save on 1 and 8 workers",
    );
    for path in saves {
        std::fs::remove_file(path).unwrap();
    }
}

// A core to load a populated-city snapshot into, as `populated_city/snapshot.rs` loads them.
fn populated_core() -> SimCore {
    let mut core = SimCore {
        benchmark_mode: true,
        ..SimCore::new(WorldConfig::new(2048.0, 2048.0, 40.0, 10.0))
    };
    install_bootstrap_assets(&mut core);
    core
}

#[test]
#[ignore = "release congestion statistics on a saved city (METRUM_CONGESTION_SAVE, a copy)"]
fn congestion_statistics_on_a_saved_city() {
    // Per adoption: how many directed weights changed (the `NET-09` partial-customization
    // question), and the frames' snapshot and wait times, at the fastest speed.
    let path = std::env::var("METRUM_CONGESTION_SAVE").expect("METRUM_CONGESTION_SAVE");
    let hours: u64 =
        std::env::var("METRUM_CONGESTION_HOURS").map_or(4, |hours| hours.parse().unwrap());
    let mut core = populated_core();
    core.load_game_internal(&path).unwrap();
    core.time.speed_multiplier = SIMULATION_SPEED_STEPS[SIMULATION_SPEED_STEPS.len() - 1];
    let end = core.time.absolute_slot() + hours * SLOTS_PER_HOUR;
    let mut revision = core.transit_network.cch_graph.congestion_revision;
    let mut previous: Option<Vec<f32>> = None;
    let (mut slot_ms, mut wait_ms) = (Vec::new(), Vec::new());
    while core.time.absolute_slot() < end {
        let report = core.step_frame(FRAME_DT_S);
        if report.congestion_ms > 0.0 {
            slot_ms.push(report.congestion_ms);
        }
        if report.congestion_wait_ms > 0.01 {
            wait_ms.push(report.congestion_wait_ms);
        }
        let network = &core.transit_network;
        if network.cch_graph.congestion_revision == revision {
            continue;
        }
        revision = network.cch_graph.congestion_revision;
        let weights = network.cch_graph.congestion_weights().unwrap().to_vec();
        let graph = &core.region_graph;
        let live: Vec<_> = (0..graph.edge_count())
            .filter(|&id| !graph.edge(id).deleted)
            .flat_map(|id| [2 * id, 2 * id + 1])
            .collect();
        let delayed = live
            .iter()
            .filter(|&&at| weights[at] > graph.edge(at / 2).base_cost + 0.5)
            .count();
        let changed = |tolerance: f32| {
            previous.as_ref().map(|previous| {
                live.iter()
                    .filter(|&&at| (weights[at] - previous[at]).abs() > tolerance)
                    .count()
            })
        };
        eprintln!(
            "CONGESTION_ADOPTION {}",
            serde_json::json!({ "slot": core.time.absolute_slot(), "directions": live.len(),
                "delayed_over_0_5_s": delayed, "changed": changed(0.0),
                "changed_over_0_5_s": changed(0.5), "changed_over_2_s": changed(2.0),
                "max_delay_s": live.iter().map(|&at| weights[at] - graph.edge(at / 2).base_cost)
                    .fold(0.0_f32, f32::max) })
        );
        previous = Some(weights);
    }
    slot_ms.sort_by(f64::total_cmp);
    wait_ms.sort_by(f64::total_cmp);
    eprintln!(
        "CONGESTION_FRAMES {}",
        serde_json::json!({ "slot_calls": slot_ms.len(),
            "slot_ms_median": slot_ms.get(slot_ms.len() / 2), "slot_ms_max": slot_ms.last(),
            "waits": wait_ms.len(), "wait_ms_max": wait_ms.last() })
    );
}
