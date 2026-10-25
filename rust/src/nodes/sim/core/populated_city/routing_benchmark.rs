// SPDX-License-Identifier: GPL-2.0-only

//! Focused routing and economy measurements; city setup is outside subsystem timing.

use super::PopulatedCity;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;
use crate::simulation::pathing::cch::{
    CchBucketSource, CchBuckets, CchGraph, CchPathScratch, CchQueryScratch,
};
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
    profile_cch(
        &format!("populated-{residents}"),
        &format!("{:?}", city.record()),
        &city.core.region_graph,
        &city.core.transit_network.cch_graph,
    );
}

#[test]
#[ignore = "release routing benchmark on a saved city's road graph (METRUM_CCH_GRAPH_SAVE)"]
fn benchmark_saved_cch_queries() {
    // Opened read-only; point it at a copy, never at a save in the active game profile.
    let path = std::env::var("METRUM_CCH_GRAPH_SAVE").expect("METRUM_CCH_GRAPH_SAVE");
    let graph = crate::simulation::save::load_road_graph(std::path::Path::new(&path)).unwrap();
    let cch = crate::simulation::pathing::cch::CchGraph::build(&graph);
    let label = std::path::Path::new(&path)
        .file_stem()
        .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
    profile_cch(&label, "", &graph, &cch);
}

// Hierarchy size/build time, 4,096 mixed point queries and a car bucket batch. The query
// mix and seed are unchanged from the original populated benchmark for comparability.
fn profile_cch(label: &str, header: &str, graph: &RegionGraph, cch: &CchGraph) {
    let mut build_ms = Vec::new();
    for _ in 0..5 {
        let begin = Instant::now();
        black_box(CchGraph::build(graph));
        build_ms.push(begin.elapsed().as_secs_f64() * 1e3);
    }
    let (states, arcs) = cch.size();
    eprintln!(
        "CCH_HIERARCHY {}",
        serde_json::json!({ "label": label, "nodes": graph.node_count(),
            "edges": graph.edge_count(), "states": states, "arcs": arcs, "build_ms": build_ms })
    );
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
    writeln!(report, "{header}").unwrap();
    for &(start, end, edge, mode) in &queries {
        let cost = cch.find_cost(start, end, edge, usize::MAX, graph, mode, &mut costs);
        let path = cch.find_path_in(start, end, edge, usize::MAX, graph, mode, &mut paths);
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
                    black_box(cch.find_path_in(
                        start,
                        end,
                        edge,
                        usize::MAX,
                        graph,
                        mode,
                        &mut paths,
                    ));
                } else {
                    black_box(cch.find_cost(start, end, edge, usize::MAX, graph, mode, &mut costs));
                }
            }
            samples.push(begin.elapsed().as_secs_f64() * 1e6 / queries.len() as f64);
        }
        eprintln!(
            "CCH_POPULATED_BENCH {}",
            serde_json::json!({
                "label": label, "nodes": graph.node_count(), "edges": graph.edge_count(),
                "queries": queries.len(), "materialize": materialize, "samples_us": samples,
            })
        );
    }
    // Car buckets: 64 targets, each left on one of its roads, and 256 sources, a third of
    // them with an incoming road edge.
    let mut targets: Vec<_> = (0..64)
        .map(|_| {
            let node = rng.gen_range(0..graph.node_count() as u32);
            let arms = graph.node_adjacency(node);
            let exit = if arms.is_empty() {
                usize::MAX
            } else {
                arms[rng.gen_range(0..arms.len())]
            };
            (node, exit)
        })
        .collect();
    targets.sort_unstable();
    targets.dedup();
    let sources: Vec<_> = (0..256)
        .map(|i| {
            let start = rng.gen_range(0..graph.node_count() as u32);
            let arms = graph.node_adjacency(start);
            let edge = if i % 3 == 0 && !arms.is_empty() {
                arms[rng.gen_range(0..arms.len())]
            } else {
                usize::MAX
            };
            (start, edge)
        })
        .collect();
    let mut bucket_build_us = Vec::new();
    for _ in 0..5 {
        let begin = Instant::now();
        black_box(CchBuckets::build(cch, &targets, TransitFlags::CAR));
        bucket_build_us.push(begin.elapsed().as_secs_f64() * 1e6);
    }
    let buckets = CchBuckets::build(cch, &targets, TransitFlags::CAR);
    let mut source = CchBucketSource::default();
    source.prepare(&buckets, cch);
    let mut samples = Vec::new();
    for _ in 0..11 {
        let begin = Instant::now();
        for &(start, edge) in &sources {
            buckets.query_source(cch, start, edge, &mut source);
            black_box(buckets.cost(0, &source));
        }
        samples.push(begin.elapsed().as_secs_f64() * 1e6 / sources.len() as f64);
    }
    eprintln!(
        "CCH_BUCKET_BENCH {}",
        serde_json::json!({ "label": label, "targets": targets.len(),
            "sources": sources.len(), "build_us": bucket_build_us, "source_us": samples })
    );
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
                .network_targets(MODE_CAR)
                .into_iter()
                .flatten()
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
                // Node-only exits keep the comparison with the node-target RPHAST prototype.
                BuildingTripEnds::destination(i, allocator, network, graph)
                    .network_targets(MODE_CAR)
                    .into_iter()
                    .flatten()
                    .map(|(node, _)| node)
            })
            .collect();
        let target_collect_us = target_collect_start.elapsed().as_secs_f64() * 1e6;
        let sources: Vec<_> = if workload == "freight" {
            borders.iter().map(|&node| (node, usize::MAX)).collect()
        } else {
            homes.clone()
        };
        let begin = Instant::now();
        let bucket_targets: Vec<_> = targets.iter().map(|&node| (node, usize::MAX)).collect();
        let buckets = CchBuckets::build(cch, &bucket_targets, TransitFlags::CAR);
        let bucket_build_us = begin.elapsed().as_secs_f64() * 1e6;
        let begin = Instant::now();
        let restricted = RestrictedTargets::build(cch, &targets);
        let restricted_build_us = begin.elapsed().as_secs_f64() * 1e6;
        assert!(
            buckets
                .targets()
                .iter()
                .map(|&(node, _)| node)
                .eq(restricted.targets().iter().copied())
        );
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
                                        usize::MAX,
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

// ROAD-68-G: one player road commit on a populated city, stage by stage. Tiers are residents
// (`METRUM_COMMIT_TIERS`, default 0,1300,13000); 0 is the 1,300-resident street grid without
// lots. Each sample branches a 120 m road off a border trunk west of the grid, splitting the
// trunk into a T junction, which changes the routing topology. City setup is never timed.
#[test]
#[ignore = "release ROAD-68-G commit breakdown; builds the requested cities outside timing"]
fn benchmark_populated_commit_breakdown() {
    let tiers: Vec<u32> = std::env::var("METRUM_COMMIT_TIERS")
        .map(|value| value.split(',').map(|t| t.trim().parse().unwrap()).collect())
        .unwrap_or_else(|_| vec![0, 1_300, 13_000]);
    for residents in tiers {
        let mut core = if residents == 0 {
            super::street_grid(super::side_m_for(1_300)).0
        } else {
            PopulatedCity::build(residents).core
        };
        profile_commits(residents, &mut core);
    }
}

fn profile_commits(residents: u32, core: &mut crate::nodes::sim::core::state::SimCore) {
    use crate::nodes::sim::core::frame::FRAME_DT_S;
    use crate::nodes::sim::core::road_preview::road_tool_snapshot_from_core;

    core.transit_network
        .rebuild_pathing_if_dirty(&mut core.region_graph);
    if core.time.speed_multiplier <= 0.0 {
        core.time.speed_multiplier = 1.0;
    }
    // Two ordinary frames: the flow fields are current and the second is the reference frame.
    core.step_frame(FRAME_DT_S);
    let reference = core.step_frame(FRAME_DT_S);
    // The player's commit records undo and charges the treasury.
    core.benchmark_mode = false;
    let starts = trunk_branch_starts(core);
    let header = serde_json::json!({
        "residents": residents,
        "buildings": core.allocator.buildings.len(),
        "households": core.households.households.len(),
        "agents": core.agents.len(),
        "nodes": core.region_graph.node_count(),
        "edges": core.region_graph.edge_count(),
        "lanes": core.transit_network.lane_system.lanes.len(),
        "reference_frame_ms": reference.pathing_ms + reference.agent_ms + reference.minute_ms,
        "reference_pathing_ms": reference.pathing_ms,
        "reference_agent_ms": reference.agent_ms,
    });
    eprintln!("COMMIT_CITY {header}");
    // Godot uploaded and acknowledged the city's road chunks long before the click, so a commit
    // republishes only the chunks it changes (`acknowledge_network_render`).
    core.precompute_road_mesh_data();
    assert!(core.acknowledge_network_render_generation(core.cached_road_mesh_generation));
    let published = crate::nodes::sim::core::RoadToolShared::new(
        road_tool_snapshot_from_core(core).expect("the fixture roadbed is current"),
    );
    // Godot also requested and uploaded the refined terrain of the road patches it showed, so a
    // commit rebuilds only the tiles it changes. The fixture builds every road patch once here,
    // which also checks that the whole city's terrain builds (`ROAD-68-D` part 6).
    let patches: Vec<(usize, usize)> = (0..core.heightmap.render_patch_rows())
        .flat_map(|z| (0..core.heightmap.render_patch_cols()).map(move |x| (x, z)))
        .collect();
    let (refined, _) = crate::nodes::simulation_node::SimulationNode::validate_staged_road_terrain(
        core, &patches, None,
    )
    .unwrap_or_else(|reason| panic!("the city's refined terrain must build: {reason}"));
    core.insert_refined_terrain_patch_cache_entries(refined);
    // `TERRAIN-07-A` `T2` and `T7`: the whole city's road tiles at `GL0`, outside timing.
    let census = crate::simulation::terrain::graded::tests::graded_census(
        &core.heightmap,
        &core.transit_network.roadbed,
        crate::simulation::terrain::graded::Level::Gl0,
    );
    eprintln!("GRADED_CENSUS {}", serde_json::json!({"residents": residents, "census": census}));
    let walls = crate::simulation::terrain::graded::tests::wall_sites(
        &core.heightmap,
        &core.transit_network.roadbed,
        5,
    );
    eprintln!("GRADED_WALLS {}", serde_json::json!({"residents": residents, "walls": walls}));
    // `METRUM_COMMIT_PHASES` logs the commit's cell-lot regions (`economy_phases`).
    let phases = std::env::var_os("METRUM_COMMIT_PHASES").is_some();
    for (sample, start) in starts.into_iter().enumerate() {
        // Godot also uploads and acknowledges every terrain patch dirtied before the click
        // (`acknowledge_uploads`), so the commit's road-locked terrain state walks its own.
        let terrain = core.terrain_dirty_patch_states();
        core.acknowledge_terrain_render_patches(&terrain);
        // The roadbed before the commit, for the graded terrain's dirty tiles (`O(1)`).
        let before_roadbed = core.transit_network.roadbed.snapshot();
        crate::debug::Channel::EconomyPhases.set_enabled(phases);
        profile_control("enable");
        let road_total = Instant::now();
        let outcome = commit_trunk_branch(core, start, road_total);
        let commit_ms = road_total.elapsed().as_secs_f64() * 1e3;
        profile_control("disable");
        crate::debug::Channel::EconomyPhases.set_enabled(false);
        assert!(
            outcome.committed,
            "commit {sample} rejected: {}",
            outcome.rejection_reason
        );
        // The sim thread publishes the tool snapshot outside the core, dropping the previous.
        let publish = Instant::now();
        if let Some(publication) = outcome.road_snapshot {
            published.publish(publication);
        }
        let publish_ms = publish.elapsed().as_secs_f64() * 1e3;
        // Godot uploads the republished chunks (not timed here) and acknowledges them.
        let published_chunks = core.pending_road_mesh_chunks.len();
        assert!(core.acknowledge_network_render_generation(core.cached_road_mesh_generation));
        let click_to_ack_ms = road_total.elapsed().as_secs_f64() * 1e3;
        // The commit's zoning products, to compare builds that must not change them: cell
        // paint and lot claims, parcels in store order and the queued lot regions; and the
        // generated cell cache, which a build may legitimately fill less.
        let (claims, generated) = core.zoning.cells.digests();
        let lots_digest = format!(
            "{:016x}",
            crate::debug::work::debug_hash(&(
                claims,
                core.zoning.parcels(),
                core.zoning.saved_cell_lot_queue(),
            ))
        );
        let m = &core.last_road_edit_metrics;
        let stages = serde_json::json!({
            "plan_ms": m.road_plan_ms, "undo_ms": m.undo_ms, "topology_ms": m.topology_ms,
            "zoning_ms": m.zoning_ms, "add_ms": m.add_ms, "surface_ms": m.surface_ms,
            "surface_roadbed_ms": m.surface_roadbed_ms,
            "surface_overlap_ms": m.surface_overlap_ms,
            "surface_terrain_ms": m.surface_terrain_ms,
            "agent_invalidate_ms": m.agent_invalidate_ms,
            "lanes_and_reattach_ms": m.lanes_and_reattach_ms, "buildings_ms": m.buildings_ms,
            "routing_ms": m.routing_ms, "finalize_ms": m.finalize_ms, "mesh_ms": m.mesh_ms,
            "snapshot_ms": m.snapshot_ms, "refined_state_ms": m.refined_state_ms,
            "core_work_ms": m.core_work_ms, "dirty_edges": m.dirty_edges,
        });
        // Frames until the rebuilt hierarchy is adopted (D7), then the adoption frame, which
        // refreshes the flow fields on it and serves the routes that waited.
        let adopt = Instant::now();
        let mut held_frames = 0;
        let next = loop {
            let frame = core.step_frame(FRAME_DT_S);
            if !frame.routing_held {
                break frame;
            }
            held_frames += 1;
        };
        let adoption_latency_ms = adopt.elapsed().as_secs_f64() * 1e3;
        let after = core.step_frame(FRAME_DT_S);
        // Isolated costs, after the measured path: the snapshot's clones and the hierarchy.
        let time = |run: &mut dyn FnMut()| {
            let begin = Instant::now();
            run();
            begin.elapsed().as_secs_f64() * 1e3
        };
        let terrain_clone_ms = time(&mut || drop(black_box(core.heightmap.clone())));
        let graph_clone_ms = time(&mut || drop(black_box(core.region_graph.clone())));
        // What dropping the previous snapshot's graph, which the live core no longer shares,
        // costs the publication.
        let graph = black_box(core.region_graph.clone());
        let begin = Instant::now();
        drop(black_box(graph));
        let graph_drop_ms = begin.elapsed().as_secs_f64() * 1e3;
        let roadbed_clone_ms =
            time(&mut || drop(black_box(core.transit_network.roadbed.clone())));
        let water_clone_ms = time(&mut || drop(black_box(core.watermap.clone())));
        let zoning_snap_ms = time(&mut || {
            drop(black_box(
                core.zoning
                    .cells
                    .road_grid_snap(f64::from(core.config.zone_cell_m)),
            ))
        });
        let cch_build_ms = time(&mut || drop(black_box(CchGraph::build(&core.region_graph))));
        // The local publication left no lane off its surface: a full sync would move none.
        let lanes_off_surface = core
            .transit_network
            .lane_system
            .lanes_off_surface(&core.region_graph, &core.transit_network.roadbed);
        let record = serde_json::json!({
            "residents": residents, "sample": sample, "commit_ms": commit_ms,
            "publish_ms": publish_ms, "click_to_ack_ms": click_to_ack_ms,
            "published_chunks": published_chunks, "stages": stages,
            "held_frames": held_frames, "adoption_wait_ms": next.adoption_ms,
            "adoption_latency_ms": adoption_latency_ms,
            "adoption_frame_pathing_ms": next.pathing_ms,
            "adoption_frame_agent_ms": next.agent_ms,
            "adoption_frame_minute_ms": next.minute_ms,
            "following_frame_pathing_ms": after.pathing_ms,
            "following_frame_agent_ms": after.agent_ms,
            "terrain_clone_ms": terrain_clone_ms, "graph_clone_ms": graph_clone_ms,
            "graph_drop_ms": graph_drop_ms,
            "roadbed_clone_ms": roadbed_clone_ms, "water_clone_ms": water_clone_ms,
            "zoning_snap_ms": zoning_snap_ms, "cch_build_ms": cch_build_ms,
            "lanes_off_surface": lanes_off_surface,
            "lots_digest": lots_digest, "generated_digest": format!("{generated:016x}"),
        });
        eprintln!("COMMIT_BREAKDOWN {record}");
        graded_tiles(residents, sample, core, &before_roadbed);
    }
    // After the timed commits: the road tool view's persistent cell copy (`ROAD-68` open
    // question 32), its entries and estimated bytes (each persistent entry also holds a
    // pointer and an `Arc` header, counted as 24 bytes), and the process's resident-set growth
    // while one more copy is alive.
    let resident = || {
        std::fs::read_to_string("/proc/self/statm")
            .ok()
            .and_then(|statm| statm.split_whitespace().nth(1)?.parse::<usize>().ok())
            .unwrap_or(0)
            * 4096
    };
    let (entries, bytes) = core.zoning.cells.memory_estimate();
    let before = resident();
    let begin = Instant::now();
    let copy = crate::simulation::zoning::cells::CellStore::copy_of(&core.zoning.cells);
    let copy_ms = begin.elapsed().as_secs_f64() * 1e3;
    let grown = resident().saturating_sub(before);
    assert_eq!(copy.memory_estimate(), (entries, bytes));
    drop(black_box(copy));
    eprintln!(
        "COMMIT_CELL_COPY {}",
        serde_json::json!({"residents": residents, "entries": entries, "payload_bytes": bytes,
            "copy_estimate_bytes": bytes + 24 * entries, "copy_resident_growth_bytes": grown,
            "copy_ms": copy_ms})
    );
}

// `TERRAIN-07-A`: the graded terrain stage of the same commit on the same core, after it
// (`graded_stage`: dirty tile selection and the `GL0` bake, median of 7); the exact cut's
// `surface_ms` of the commit is the same-run control.
fn graded_tiles(
    residents: u32,
    sample: usize,
    core: &crate::nodes::sim::core::state::SimCore,
    before: &crate::simulation::network::roadbed::RoadbedSystem,
) {
    let mut record = crate::simulation::terrain::graded::tests::graded_stage(
        &core.heightmap,
        before,
        &core.transit_network.roadbed,
        &core.last_road_terrain_changes,
    );
    record["residents"] = residents.into();
    record["sample"] = sample.into();
    record["surface_ms"] = core.last_road_edit_metrics.surface_ms.into();
    eprintln!("GRADED_TILES_COMMIT {record}");
}

/// Points on a west border trunk outside the painted grid, 100, 250 and 400 m west of it,
/// trunk nearest the middle first. A branch there splits the trunk into a T junction and
/// crosses no zoned cell.
pub(super) fn trunk_branch_starts(
    core: &crate::nodes::sim::core::state::SimCore,
) -> Vec<godot::prelude::Vector3> {
    use godot::prelude::Vector3;
    let graph = &core.region_graph;
    let (half_world, _) = core.heightmap.half_world_extents();
    let live = |node: u32| {
        (node as usize) < graph.node_adjacency_count()
            && graph
                .node_adjacency(node)
                .iter()
                .any(|&edge| !graph.edge(edge).deleted)
    };
    let nodes: Vec<Vector3> = (0..graph.node_count() as u32)
        .filter(|&node| live(node))
        .map(|node| graph.node(node).pos)
        .collect();
    let west_x = nodes
        .iter()
        .filter(|pos| pos.x > 50.0 - half_world)
        .map(|pos| pos.x)
        .fold(f32::MAX, f32::min);
    // West trunks end at border nodes on the world's west edge.
    let mut trunk_z: Vec<f32> = nodes
        .iter()
        .filter(|pos| pos.x <= 50.0 - half_world)
        .map(|pos| pos.z)
        .collect();
    trunk_z.sort_by(|a, b| a.abs().total_cmp(&b.abs()));
    let z = trunk_z[0];
    [100.0, 250.0, 400.0]
        .iter()
        .map(|offset| {
            let x = west_x - offset;
            Vector3::new(x, core.heightmap.sample_height_world(x, z), z)
        })
        .collect()
}

/// Commits a 120 m two-lane road north from `start` on a trunk through the player's path.
pub(super) fn commit_trunk_branch(
    core: &mut crate::nodes::sim::core::state::SimCore,
    start: godot::prelude::Vector3,
    road_total: Instant,
) -> crate::nodes::sim::core::road_commit::RoadCommitOutcome {
    use crate::nodes::sim::core::road_commit::RoadCommitRequest;
    let mut end = start + godot::prelude::Vector3::new(0.0, 0.0, 120.0);
    end.y = core.heightmap.sample_height_world(end.x, end.z);
    core.commit_road(
        RoadCommitRequest {
            points: vec![start, end],
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
            preview: None,
        },
        road_total,
        Default::default(),
    )
}
