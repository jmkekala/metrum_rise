// SPDX-License-Identifier: GPL-2.0-only

//! Focused routing and economy measurements; city setup is outside subsystem timing.

use super::PopulatedCity;
use crate::simulation::network::types::TransitFlags;
use crate::simulation::pathing::cch::{CchPathScratch, CchQueryScratch};
use rand::{Rng, SeedableRng, rngs::StdRng};
use std::hint::black_box;
use std::io::Write;
use std::time::Instant;

#[test]
#[ignore = "release populated routing benchmark; builds the requested city outside timing"]
fn benchmark_populated_cch_queries() {
    let residents = std::env::var("METRUM_CCH_BENCH_RESIDENTS")
        .map(|value| value.parse().unwrap())
        .unwrap_or(10_000);
    let city = PopulatedCity::build(residents);
    let graph = &city.core.region_graph;
    let cch = &city.core.transit_network.cch_graph;
    let mut rng = StdRng::seed_from_u64(0x4e45_542d_3032);
    let queries: Vec<_> = (0..4096)
        .map(|i| {
            let start = rng.gen_range(0..graph.node_count() as u32);
            let end = rng.gen_range(0..graph.node_count() as u32);
            let arms = graph.node_adjacency(start);
            let edge = if i % 3 == 0 && !arms.is_empty() {
                arms[rng.gen_range(0..arms.len())]
            } else {
                usize::MAX
            };
            let mode = if i % 2 == 0 {
                TransitFlags::CAR
            } else {
                TransitFlags::FOOT
            };
            (start, end, edge, mode)
        })
        .collect();
    let mut costs = CchQueryScratch::default();
    let mut paths = CchPathScratch::default();
    costs.prepare(cch, graph);
    paths.prepare(cch, graph);
    let mut report = Vec::new();
    writeln!(report, "{:?}", city.record()).unwrap();
    for &(start, end, edge, mode) in &queries {
        let cost = cch.find_cost(start, end, edge, graph, mode, &mut costs);
        let path = cch.find_path_in(start, end, edge, graph, mode, &mut paths);
        assert_eq!(cost.map(f32::to_bits), path.as_ref().map(|p| p.0.to_bits()));
        writeln!(
            report,
            "{start} {end} {edge} {mode} {:?}",
            path.map(|(cost, distance, nodes)| (cost.to_bits(), distance.to_bits(), nodes))
        )
        .unwrap();
    }
    if let Ok(path) = std::env::var("METRUM_CCH_QUERY_REPORT") {
        std::fs::write(path, &report).unwrap();
    }
    for materialize in [false, true] {
        let mut samples = Vec::new();
        for _ in 0..11 {
            let begin = Instant::now();
            for &(start, end, edge, mode) in &queries {
                if materialize {
                    black_box(cch.find_path_in(start, end, edge, graph, mode, &mut paths));
                } else {
                    black_box(cch.find_cost(start, end, edge, graph, mode, &mut costs));
                }
            }
            samples.push(begin.elapsed().as_secs_f64() * 1e6 / queries.len() as f64);
        }
        eprintln!(
            "CCH_POPULATED_BENCH {}",
            serde_json::json!({
                "residents": residents, "nodes": graph.node_count(), "edges": graph.edge_count(),
                "queries": queries.len(), "materialize": materialize, "samples_us": samples,
            })
        );
    }
}

#[test]
#[ignore = "release bucket/RPHAST comparison on actual populated-city target sets"]
fn benchmark_populated_cch_batches() {
    use crate::simulation::economy::agents::{MODE_CAR, tick::BuildingTripEnds};
    use crate::simulation::economy::logistics::collect_connected_border_nodes;
    use crate::simulation::pathing::cch::{
        CchBucketSource, CchBuckets, RESTRICTED_BATCH_LANES, RestrictedBatchScratch,
        RestrictedScratch, RestrictedTargets,
    };
    use rayon::prelude::*;

    let residents = std::env::var("METRUM_CCH_BENCH_RESIDENTS")
        .map(|value| value.parse().unwrap())
        .unwrap_or(10_000);
    let mut city = PopulatedCity::build(residents);
    let graph = &city.core.region_graph;
    let network = &city.core.transit_network;
    let cch = &network.cch_graph;
    let allocator = &city.core.allocator;
    let capacities = &city.core.households.job_funding.capacities;
    let mut borders = Vec::new();
    collect_connected_border_nodes(graph, &mut borders);
    let mut homes: Vec<_> = (0..allocator.buildings.len())
        .into_par_iter()
        .filter(|&i| allocator.household_capacity(i) > 0)
        .flat_map_iter(|i| {
            BuildingTripEnds::origin(i, allocator, network, graph)
                .network_nodes(MODE_CAR)
                .into_iter()
                .flatten()
                .map(move |node| (node, allocator.entrances[i].edge_idx))
        })
        .collect();
    homes.sort_unstable();
    homes.dedup();
    // Evenly sample the full city, without allowing its number of homes to dominate setup.
    let homes: Vec<_> = homes
        .iter()
        .step_by(homes.len().div_ceil(512).max(1))
        .copied()
        .collect();
    for workload in ["freight", "open_jobs", "dense_jobs"] {
        let target_collect_start = Instant::now();
        let targets: Vec<_> = allocator
            .buildings
            .par_iter()
            .enumerate()
            .filter(|&(i, building)| {
                if workload == "freight" {
                    return true;
                }
                !building.broken
                    && !building.economy_broken
                    && !building.is_deserted
                    && !building.is_under_construction()
                    && building.edge_idx != usize::MAX
                    && capacities.get(i).copied().unwrap_or(0)
                        > if workload == "open_jobs" {
                            building.worker_count
                        } else {
                            0
                        }
            })
            .flat_map_iter(|(i, _)| {
                BuildingTripEnds::destination(i, allocator, network, graph)
                    .network_nodes(MODE_CAR)
                    .into_iter()
                    .flatten()
            })
            .collect();
        let target_collect_us = target_collect_start.elapsed().as_secs_f64() * 1e6;
        let sources: Vec<_> = if workload == "freight" {
            borders.iter().map(|&node| (node, usize::MAX)).collect()
        } else {
            homes.clone()
        };
        let begin = Instant::now();
        let buckets = CchBuckets::build(cch, &targets, TransitFlags::CAR);
        let bucket_build_us = begin.elapsed().as_secs_f64() * 1e6;
        let begin = Instant::now();
        let restricted = RestrictedTargets::build(cch, &targets);
        let restricted_build_us = begin.elapsed().as_secs_f64() * 1e6;
        assert_eq!(buckets.targets(), restricted.targets());
        let chunk_size = sources.len().div_ceil(rayon::current_num_threads()).max(1);
        let chunks: Vec<_> = sources.chunks(chunk_size).collect();
        let begin = Instant::now();
        let mut bucket_scratch: Vec<_> = chunks
            .iter()
            .map(|_| {
                let mut scratch = CchBucketSource::default();
                scratch.prepare(&buckets, cch);
                scratch
            })
            .collect();
        let bucket_scratch_prepare_us = begin.elapsed().as_secs_f64() * 1e6;
        let begin = Instant::now();
        let mut restricted_scratch: Vec<_> = chunks
            .iter()
            .map(|_| {
                let mut scratch = RestrictedScratch::default();
                restricted.prepare(cch, &mut scratch);
                scratch
            })
            .collect();
        let restricted_scratch_prepare_us = begin.elapsed().as_secs_f64() * 1e6;
        let blocked_chunks: Vec<_> = sources
            .chunks(chunk_size.div_ceil(RESTRICTED_BATCH_LANES) * RESTRICTED_BATCH_LANES)
            .collect();
        let begin = Instant::now();
        let mut blocked_scratch: Vec<_> = blocked_chunks
            .iter()
            .map(|_| {
                let mut scratch = RestrictedBatchScratch::default();
                restricted.prepare_batch(cch, &mut scratch);
                scratch
            })
            .collect();
        let blocked_scratch_prepare_us = begin.elapsed().as_secs_f64() * 1e6;
        let mut bits_differ = 0usize;
        let mut ceil_differ = 0usize;
        let mut max_delta = 0.0f32;
        let mut check_bucket = CchBucketSource::default();
        let mut check_restricted = RestrictedScratch::default();
        for &(start, incoming) in &sources {
            buckets.query_source(cch, start, incoming, &mut check_bucket);
            let costs = restricted.query(
                cch,
                start,
                incoming,
                TransitFlags::CAR,
                &mut check_restricted,
            );
            for (index, &cost) in costs.iter().enumerate() {
                let point = buckets.cost(index, &check_bucket).unwrap_or(f32::INFINITY);
                assert_eq!(point.is_finite(), cost.is_finite());
                if point.to_bits() != cost.to_bits() {
                    bits_differ += 1;
                    ceil_differ += usize::from(point.ceil() != cost.ceil());
                    max_delta = max_delta.max((point - cost).abs());
                }
            }
            if incoming == usize::MAX {
                let full = cch.costs_from_each(&[start], graph, TransitFlags::CAR);
                for (&target, &cost) in restricted.targets().iter().zip(costs) {
                    assert_eq!(
                        cost.to_bits(),
                        full[0].cost_to(target).unwrap_or(f32::INFINITY).to_bits()
                    );
                }
            }
        }
        let mut check_blocked = RestrictedBatchScratch::default();
        for starts in sources.chunks(RESTRICTED_BATCH_LANES) {
            let blocked =
                restricted.query_batch(cch, starts, TransitFlags::CAR, &mut check_blocked);
            for (lane, &(start, incoming)) in starts.iter().enumerate() {
                for (target, &cost) in restricted
                    .query(
                        cch,
                        start,
                        incoming,
                        TransitFlags::CAR,
                        &mut check_restricted,
                    )
                    .iter()
                    .enumerate()
                {
                    assert_eq!(blocked[target][lane].to_bits(), cost.to_bits());
                }
            }
        }
        let mut measure = |kind: &str| {
            let mut samples = Vec::new();
            for _ in 0..11 {
                let begin = Instant::now();
                for _ in 0..8 {
                    if kind == "bucket" {
                        bucket_scratch.par_iter_mut().zip(&chunks).for_each(
                            |(scratch, sources)| {
                                for &(start, incoming) in *sources {
                                    buckets.query_source(cch, start, incoming, scratch);
                                    black_box(&*scratch);
                                }
                            },
                        );
                    } else if kind == "blocked" {
                        blocked_scratch
                            .par_iter_mut()
                            .zip(&blocked_chunks)
                            .for_each(|(scratch, sources)| {
                                for starts in sources.chunks(RESTRICTED_BATCH_LANES) {
                                    black_box(restricted.query_batch(
                                        cch,
                                        starts,
                                        TransitFlags::CAR,
                                        scratch,
                                    ));
                                }
                            });
                    } else {
                        restricted_scratch.par_iter_mut().zip(&chunks).for_each(
                            |(scratch, sources)| {
                                for &(start, incoming) in *sources {
                                    black_box(restricted.query(
                                        cch,
                                        start,
                                        incoming,
                                        TransitFlags::CAR,
                                        scratch,
                                    ));
                                }
                            },
                        );
                    }
                }
                samples.push(begin.elapsed().as_secs_f64() * 1e6 / 8.0);
            }
            samples
        };
        let bucket_us = measure("bucket");
        let restricted_us = measure("restricted");
        let blocked_us = measure("blocked");
        let mut point_us = Vec::new();
        if workload == "open_jobs" {
            let mut scratch: Vec<_> = chunks
                .iter()
                .map(|_| {
                    let mut query = CchQueryScratch::default();
                    query.prepare(cch, graph);
                    query
                })
                .collect();
            for _ in 0..11 {
                let begin = Instant::now();
                for _ in 0..8 {
                    scratch
                        .par_iter_mut()
                        .zip(&chunks)
                        .for_each(|(scratch, sources)| {
                            for &(start, incoming) in *sources {
                                for &target in restricted.targets() {
                                    black_box(cch.find_cost(
                                        start,
                                        target,
                                        incoming,
                                        graph,
                                        TransitFlags::CAR,
                                        scratch,
                                    ));
                                }
                            }
                        });
                }
                point_us.push(begin.elapsed().as_secs_f64() * 1e6 / 8.0);
            }
        }
        let mut full_us = Vec::new();
        if workload == "freight" {
            for _ in 0..11 {
                let begin = Instant::now();
                black_box(cch.costs_from_each(&borders, graph, TransitFlags::CAR));
                full_us.push(begin.elapsed().as_secs_f64() * 1e6);
            }
        }
        eprintln!(
            "CCH_BATCH_BENCH {}",
            serde_json::json!({
                "residents": residents, "workload": workload, "sources": sources.len(),
                "targets": restricted.targets().len(), "closure": restricted.closure_len(),
                "states": RestrictedTargets::state_count(cch), "bucket_build_us": bucket_build_us,
                "target_collect_us": target_collect_us,
                "restricted_build_us": restricted_build_us, "bucket_us": bucket_us,
                "bucket_scratch_prepare_us": bucket_scratch_prepare_us,
                "restricted_scratch_prepare_us": restricted_scratch_prepare_us,
                "blocked_scratch_prepare_us": blocked_scratch_prepare_us, "blocked_us": blocked_us,
                "restricted_us": restricted_us, "full_us": full_us, "bits_differ": bits_differ,
                "point_us": point_us,
                "ceil_differ": ceil_differ, "max_delta": max_delta,
            })
        );
    }
    // Optional attribution starts after every unprofiled microbenchmark and city setup.
    // It uses existing phase logs; diagnostic times are never acceptance samples.
    if let Ok(hours) = std::env::var("METRUM_CCH_BENCH_DIAGNOSTIC_HOURS") {
        crate::debug::init();
        for _ in 0..hours.parse::<usize>().unwrap() {
            black_box(city.advance_hour());
        }
    }
}

#[test]
#[ignore = "release exact border-quote comparison; populated fixture setup excluded"]
fn benchmark_populated_border_quotes() {
    use crate::simulation::economy::definitions::{
        load_runtime_economy_catalog, load_runtime_economy_tuning,
    };
    use crate::simulation::economy::logistics::{ShipmentSystem, collect_connected_border_nodes};
    use rayon::prelude::*;

    let residents = std::env::var("METRUM_CCH_BENCH_RESIDENTS")
        .map(|value| value.parse().unwrap())
        .unwrap_or(10_000);
    let mut city = PopulatedCity::build(residents);
    let allocator = &city.core.allocator;
    let graph = &city.core.region_graph;
    let network = &mut city.core.transit_network;
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let mut borders = Vec::new();
    collect_connected_border_nodes(graph, &mut borders);
    let costs = network
        .cch_graph
        .costs_from_each(&borders, graph, TransitFlags::CAR);
    let mut quotes = vec![None; allocator.buildings.len()];
    let mut expected = quotes.clone();
    let mut logistics = ShipmentSystem::new();
    let cold = Instant::now();
    logistics.write_border_carrier_quotes(
        allocator,
        network,
        graph,
        &borders,
        &catalog,
        &tuning,
        &mut quotes,
    );
    let cold_us = cold.elapsed().as_secs_f64() * 1e6;
    for revision in 0..2 {
        if revision != 0 {
            for (i, lane) in network.lane_system.lanes.iter_mut().enumerate() {
                lane.live_frontage_delay_s = (i % 7) as f32 * 0.731;
            }
            network.publish_frontage_prices();
        }
        let original = |quotes: &mut [Option<f32>]| {
            quotes
                .par_iter_mut()
                .enumerate()
                .for_each(|(index, quote)| {
                    *quote = None;
                    if !catalog
                        .profile_by_runtime_id(
                            allocator.buildings[index].economy_profile_runtime_id,
                        )
                        .is_some_and(|profile| profile.worker_capacity > 0)
                    {
                        return;
                    }
                    *quote = borders
                        .iter()
                        .zip(&costs)
                        .filter_map(|(&border, costs)| {
                            allocator.freight_car_eta_from_border_costs(
                                border, index, costs, network, graph,
                            )
                        })
                        .min_by(f32::total_cmp)
                        .map(|seconds| {
                            // Original carrier formula, applied after the per-border ETA minimum.
                            let hours = f64::from(seconds) * 48.0
                                / tuning.operational_clock.seconds_per_day;
                            tuning.logistics.dispatch_fee_currency
                                + (hours * f64::from(tuning.logistics.vehicle_hour_cost_currency))
                                    as f32
                        });
                })
        };
        let mut before = Vec::new();
        let mut after = Vec::new();
        for sample in 0..11 {
            // Alternate order so a persistent cache-temperature advantage is not credited.
            for candidate in if sample % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                let start = Instant::now();
                for _ in 0..4 {
                    if candidate {
                        logistics.write_border_carrier_quotes(
                            allocator,
                            network,
                            graph,
                            &borders,
                            &catalog,
                            &tuning,
                            &mut quotes,
                        );
                        black_box(&quotes);
                    } else {
                        original(&mut expected);
                        black_box(&expected);
                    }
                }
                let elapsed = start.elapsed().as_secs_f64() * 1e6 / 4.0;
                if candidate {
                    after.push(elapsed);
                } else {
                    before.push(elapsed);
                }
            }
        }
        assert!(
            quotes
                .iter()
                .zip(&expected)
                .all(|(a, b)| a.map(f32::to_bits) == b.map(f32::to_bits))
        );
        eprintln!(
            "BORDER_QUOTE_BENCH {}",
            serde_json::json!({"residents":residents,
            "buildings":quotes.len(),"borders":borders.len(),"revision":revision,
            "cold_us":cold_us,"original_us":before,"minimum_us":after,"bits_equal":true})
        );
    }
}

// Optional perf --control FIFO excludes fixture construction from sampled attribution.
// Only ignored manual benchmarks use it; ordinary test execution needs no external tools.
fn profile_control(command: &str) {
    if let Ok(path) = std::env::var("METRUM_ECON_PERF_CONTROL") {
        let mut pipe = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        writeln!(pipe, "{command}").unwrap();
    }
}

#[test]
#[ignore = "release populated cell-generation attribution; fixed local envelope"]
fn benchmark_populated_cell_preparation() {
    use crate::simulation::zoning::cells::CellBounds;
    use glam::DVec2;

    let residents = std::env::var("METRUM_CCH_BENCH_RESIDENTS")
        .map(|value| value.parse().unwrap())
        .unwrap_or(10_000);
    let repeats = std::env::var("METRUM_ECON_BENCH_REPEATS")
        .map(|value| value.parse().unwrap())
        .unwrap_or(44);
    let mut city = PopulatedCity::build(residents);
    // An interior point queues exactly the same 512 m neighborhood as a local removal.
    let bounds = CellBounds {
        min: DVec2::new(100.0, -400.0),
        max: DVec2::new(120.0, -380.0),
    };
    city.core.zoning.mark_cell_lots_dirty(bounds);
    black_box(city.core.prepare_cell_lots_internal());
    let cells = city.core.zoning.cells.saved_cells();
    let parcels = city.core.zoning.parcels.parcels().len();
    let mut samples = Vec::with_capacity(repeats);
    profile_control("enable");
    for _ in 0..repeats {
        city.core.zoning.mark_cell_lots_dirty(bounds);
        let start = Instant::now();
        let report = city.core.prepare_cell_lots_internal();
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
        assert!(
            report.created.is_empty() && report.retired.is_empty() && report.reattached.is_empty()
        );
    }
    profile_control("disable");
    assert_eq!(cells, city.core.zoning.cells.saved_cells());
    assert_eq!(parcels, city.core.zoning.parcels.parcels().len());
    eprintln!(
        "CELL_PREPARATION_BENCH {}",
        serde_json::json!({
        "residents":residents,"samples_ms":samples,"unchanged_products":true})
    );
}

#[test]
#[ignore = "release populated funding attribution; fixture and frames excluded"]
fn benchmark_populated_funding_preparation() {
    use crate::simulation::economy::definitions::{
        load_runtime_economy_catalog, load_runtime_economy_tuning,
    };

    let residents = std::env::var("METRUM_CCH_BENCH_RESIDENTS")
        .map(|value| value.parse().unwrap())
        .unwrap_or(10_000);
    let repeats = std::env::var("METRUM_ECON_BENCH_REPEATS")
        .map(|value| value.parse().unwrap())
        .unwrap_or(44);
    let mut city = PopulatedCity::build(residents);
    let core = &mut city.core;
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let service = core.electricity_funding_by_building();
    let mut samples = Vec::with_capacity(repeats);
    let before = core.logistics.node_cache_counts_for_test();
    profile_control("enable");
    for _ in 0..repeats {
        // A new published price revision reproduces the live census's conservative
        // invalidation while keeping the exact household and firm inputs fixed.
        core.transit_network.frontage_delay_revision += 1;
        let start = Instant::now();
        core.households.job_funding.refresh(
            &core.allocator,
            &core.households.households,
            &mut core.logistics,
            &core.transit_network,
            &core.region_graph,
            &catalog,
            &tuning,
            &core.fiscal_policy,
            &service,
            core.treasury.balance,
            60,
        );
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
        black_box(&core.households.job_funding.capacities);
    }
    profile_control("disable");
    let after = core.logistics.node_cache_counts_for_test();
    let counts = std::array::from_fn::<_, 3, _>(|i| after[i] - before[i]);
    eprintln!(
        "FUNDING_PREPARATION_BENCH {}",
        serde_json::json!({
        "residents":residents,"samples_ms":samples,"node_requests_hits_replacement_misses":counts})
    );
}

#[test]
#[ignore = "release hourly boundary-frame attribution with ordinary intervening simulation"]
fn benchmark_populated_hour_boundaries() {
    profile_populated_boundaries(false);
}

#[test]
#[ignore = "release midnight boundary-frame attribution with ordinary intervening simulation"]
fn benchmark_populated_midnight_boundaries() {
    profile_populated_boundaries(true);
}

fn profile_populated_boundaries(midnight: bool) {
    use super::super::frame::FRAME_DT_S;
    use crate::simulation::core::time::TimeSystem;

    struct DailyProfile {
        armed: bool,
    }
    impl super::super::frame::EconomyObserver for DailyProfile {
        fn hourly_economy(&mut self, _core: &super::SimCore) {
            // At midnight the existing observer fires after the operational hour and
            // immediately before daily settlement. Leave fixture/intervening hours off.
            if self.armed {
                profile_control("enable");
            }
        }
    }
    let daily_only = midnight && std::env::var_os("METRUM_ECON_PROFILE_DAILY_ONLY").is_some();

    let residents = std::env::var("METRUM_CCH_BENCH_RESIDENTS")
        .map(|value| value.parse().unwrap())
        .unwrap_or(100_000);
    let mut city = PopulatedCity::build(residents);
    crate::debug::init();
    // Debug channels stay quiet outside the sampled boundary frames.
    let diagnostics = crate::debug::mask();
    crate::debug::set_mask(0);
    let count = std::env::var("METRUM_ECON_BENCH_REPEATS")
        .map(|value| value.parse().unwrap())
        .unwrap_or(if midnight { 3 } else { 23 });
    let mut samples = Vec::with_capacity(count);
    let before = city.core.logistics.node_cache_counts_for_test();
    let building_before = city.core.logistics.building_cache_counts_for_test();
    let customer_before = city
        .core
        .households
        .job_funding
        .customers
        .refresh_counts_for_test();
    let business_before = city
        .core
        .households
        .job_funding
        .business
        .quote_counts_for_test();
    let causes_before = city
        .core
        .households
        .job_funding
        .business
        .rejection_causes_for_test();
    let supply_before = city
        .core
        .households
        .job_funding
        .input_supply
        .search_counts_for_test();
    let access_before = city
        .core
        .households
        .job_funding
        .customers
        .access_resamples_for_test();
    while samples.len() < count {
        let time = &city.core.time;
        // Preview the exact clock calculation without changing the actual simulation.
        let mut preview = TimeSystem {
            time_elapsed: time.time_elapsed,
            speed_multiplier: time.speed_multiplier,
            day_index: time.day_index,
            minute_of_day: time.minute_of_day,
            seconds_per_day: time.seconds_per_day,
        };
        let boundary = preview
            .process_delta(FRAME_DT_S)
            .iter_elapsed_minutes()
            .any(|(_, minute)| {
                if midnight {
                    minute == 0
                } else {
                    minute != 0 && minute % 60 == 0
                }
            });
        if boundary {
            crate::debug::set_mask(diagnostics);
            if !daily_only {
                profile_control("enable");
            }
        }
        let frame = if daily_only {
            city.core
                .step_frame_observed(FRAME_DT_S, &mut DailyProfile { armed: boundary })
        } else {
            city.core.step_frame(FRAME_DT_S)
        };
        if boundary {
            profile_control("disable");
            crate::debug::set_mask(0);
            assert_eq!(frame.hourly_ticks, 1);
            assert_eq!(frame.daily_ticks, usize::from(midnight));
            samples.push(if midnight {
                frame.daily_ms
            } else {
                frame.hourly_ms
            });
        }
    }
    // Sampling includes the one agent frame immediately before each hourly tick; the other
    // movement frames and fixture setup are excluded. Midnight also samples its preceding
    // operational hour unless DAILY_ONLY arms from the existing end-of-hour observer.
    // These are diagnostic timings, not unprofiled acceptance runs.
    let after = city.core.logistics.node_cache_counts_for_test();
    let counts = std::array::from_fn::<_, 3, _>(|i| after[i] - before[i]);
    let building_after = city.core.logistics.building_cache_counts_for_test();
    let building_counts =
        std::array::from_fn::<_, 4, _>(|i| building_after[i] - building_before[i]);
    let customer_after = city
        .core
        .households
        .job_funding
        .customers
        .refresh_counts_for_test();
    let customer_counts =
        std::array::from_fn::<_, 7, _>(|i| customer_after[i] - customer_before[i]);
    let business_after = city
        .core
        .households
        .job_funding
        .business
        .quote_counts_for_test();
    let business_counts =
        std::array::from_fn::<_, 3, _>(|i| business_after[i] - business_before[i]);
    let causes_after = city
        .core
        .households
        .job_funding
        .business
        .rejection_causes_for_test();
    let rejection_causes = std::array::from_fn::<_, 5, _>(|i| causes_after[i] - causes_before[i]);
    let supply_after = city
        .core
        .households
        .job_funding
        .input_supply
        .search_counts_for_test();
    let supply_counts = std::array::from_fn::<_, 6, _>(|i| supply_after[i] - supply_before[i]);
    eprintln!(
        "BOUNDARY_BENCH {}",
        serde_json::json!({
        "residents":residents,"midnight":midnight,"daily_only_profile":daily_only,"samples_ms":samples,
        "node_requests_hits_replacement_misses_all_intervening_frames":counts,
        "building_requests_hits_replacement_misses_stale_all_intervening_frames":building_counts,
        "business_prechecks_rejected_added_rejects_all_intervening_frames":business_counts,
        "business_rejected_accounts_quantity_finance_free_carrier_all_intervening_frames":rejection_causes,
        "supply_nearest_candidates_routes_regions_reused_retries_all_intervening_frames":supply_counts,
        "customer_access_resamples_all_intervening_frames":city.core.households.job_funding.customers.access_resamples_for_test() - access_before,
        "customer_refreshes_retained_budget_network_home_geometry_price_all_intervening_frames":customer_counts})
    );
    eprintln!("boundary final {:?}", city.record());
}

#[test]
#[ignore = "release populated work-count attribution; compares 10k/100k growth"]
fn benchmark_populated_work_counts() {
    use crate::debug::work::{WORK_COUNT, WORK_NAMES, snapshot};

    let residents = std::env::var("METRUM_CCH_BENCH_RESIDENTS")
        .map(|value| value.parse().unwrap())
        .unwrap_or(10_000);
    // One demand-hour day plus the benchmark's ten settlement days.
    let days = std::env::var("METRUM_ECON_WORK_DAYS")
        .map(|value| value.parse().unwrap())
        .unwrap_or(11);
    // Each live planning pass is checked against a full book fingerprint unless skipped.
    if std::env::var_os("METRUM_SKIP_PASS_VERIFY").is_some() {
        crate::simulation::economy::demand::VERIFY_LIVE_PASSES
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }
    let mut city = PopulatedCity::build(residents);
    // METRUM_DEBUG_FILTER=economy_phases adds aggregate phase timings after the build.
    crate::debug::init();
    let scale = |city: &PopulatedCity| {
        serde_json::json!({
        "buildings":city.core.allocator.buildings.len(),
        "parcels":city.core.zoning.parcels().len(),
        "households":city.core.households.households.len()})
    };
    let initial = scale(&city);
    let mut totals = [0u64; WORK_COUNT];
    for day in 0..days {
        let before = snapshot();
        let mut hourly_ms = 0.0;
        let daily_ms = loop {
            let hour = city.advance_hour();
            hourly_ms += hour.hourly.as_secs_f64() * 1000.0;
            if let Some(daily) = hour.daily {
                break daily.as_secs_f64() * 1000.0;
            }
        };
        let after = snapshot();
        let counts: serde_json::Map<_, _> = WORK_NAMES
            .iter()
            .zip(before.iter().zip(after))
            .map(|(name, (before, after))| (name.to_string(), (after - before).into()))
            .collect();
        for (total, (before, after)) in totals.iter_mut().zip(before.iter().zip(after)) {
            *total += after - before;
        }
        eprintln!(
            "WORK_DAY {}",
            serde_json::json!({"residents":residents,"day":day,
            "hourly_ms":hourly_ms,"daily_ms":daily_ms,"counts":counts})
        );
    }
    let totals: serde_json::Map<_, _> = WORK_NAMES
        .iter()
        .zip(totals)
        .map(|(name, total)| (name.to_string(), total.into()))
        .collect();
    eprintln!(
        "WORK_TOTAL {}",
        serde_json::json!({"residents":residents,"days":days,
        "initial":initial,"final":scale(&city),"counts":totals})
    );
    eprintln!("work final {:?}", city.record());
}
