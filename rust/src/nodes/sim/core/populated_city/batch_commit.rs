// SPDX-License-Identifier: GPL-2.0-only

//! Batch commits on populated cities (`ROAD-112` round F, `docs/roads.md` "Road tools").
//!
//! `benchmark_populated_batch_commit`: on each `METRUM_COMMIT_TIERS` city as the commit
//! breakdown loads it (`ROAD-68-K22`), eight 120 m branches north off the border trunk, 50 m
//! apart, committed one by one with the frames to routing adoption between them (untimed), then,
//! on a fresh load of the same city, as one batch. `BATCH_ROUND` prints each single commit's wall
//! time split into its core and its tail (mesh precompute, publication, routing schedule, lot
//! spawn) and the batch's dry run, cores and tail, each with its stages. The bar (the user's
//! option (a), 2026-11-14) is the batch's wall time against the eight single cores plus the
//! intermediate strokes' lot plans plus the batch's one tail (`batch_over_bar`), also with a
//! single commit's mean tail; `batch_over_proxy` is round LBF's reading, the batch's own cores
//! (which take their plan and pieces from the dry run) plus its tail. Rounds alternate
//! (`METRUM_BATCH_ROUNDS`, default 3).
//!
//! `benchmark_batch_locality`: the same batch on the commit locality's neighbourhood
//! (`commit_locality`) with a remote background of `METRUM_LOCALITY_TIERS` sites; the batch's
//! local products must be bitwise equal at every tier and its stages flat, the tail apart.

use std::time::Instant;

use godot::prelude::Vector3;

use super::routing_benchmark::{commit_city, trunk_branch_end, trunk_branch_starts_at};
use crate::debug::work::debug_hash;
use crate::nodes::sim::core::frame::FRAME_DT_S;
use crate::nodes::sim::core::road_batch::{CORE_PARTS, core_parts};
use crate::nodes::sim::core::state::SimCore;
use crate::nodes::sim::core::{RoadBatchRequest, RoadCommitRequest, RoadToolShared};
use crate::simulation::network::roadbed::PieceId;

const OFFSETS: [f32; 8] = [100.0, 150.0, 200.0, 250.0, 300.0, 350.0, 400.0, 450.0];

fn stroke(points: [Vector3; 2]) -> RoadCommitRequest {
    RoadCommitRequest {
        points: points.to_vec(),
        fwd_lanes: 1,
        bkw_lanes: 1,
        snap_to_existing_roads: true,
        profile: crate::simulation::network::road_edit::RoadProfile::FollowGround,
        median: None,
        parking: crate::simulation::network::graph::RoadParking::NONE,
        attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
        preview: None,
    }
}

// The city as the breakdown prepares it before its first commit: pathing built, two frames, the
// player's undo and charges on, the road chunks acknowledged and the tool's first view published.
fn prepare(core: &mut SimCore) -> RoadToolShared {
    core.transit_network
        .rebuild_pathing_if_dirty(&mut core.region_graph);
    if core.time.speed_multiplier <= 0.0 {
        core.time.speed_multiplier = 1.0;
    }
    core.step_frame(FRAME_DT_S);
    core.step_frame(FRAME_DT_S);
    core.benchmark_mode = false;
    core.precompute_road_mesh_data();
    assert!(core.acknowledge_network_render_generation(core.cached_road_mesh_generation));
    RoadToolShared::from_publication(
        core.publish_road_tool_view()
            .expect("the fixture roadbed is current"),
    )
}

// Godot's acknowledgements before a click (terrain) and after a commit (its chunks and view),
// then the frames to the routing rebuild's adoption, which adopt the deferred lots. Untimed.
fn settle(
    core: &mut SimCore,
    published: &RoadToolShared,
    publication: Option<crate::nodes::sim::core::road_tool_context::RoadToolPublication>,
) {
    if let Some(publication) = publication {
        published.publish(publication);
    }
    assert!(core.acknowledge_network_render_generation(core.cached_road_mesh_generation));
    loop {
        let frame = core.step_frame(FRAME_DT_S);
        if let Some(publication) = core.take_adopted_road_tool_view() {
            published.publish(publication);
        }
        if !frame.routing_held {
            break;
        }
    }
    let terrain = core.terrain_dirty_patch_states();
    core.acknowledge_terrain_render_patches(&terrain);
}

fn branches(core: &SimCore) -> Vec<[Vector3; 2]> {
    trunk_branch_starts_at(core, &OFFSETS)
        .into_iter()
        .map(|start| [start, trunk_branch_end(core, start)])
        .collect()
}

// Commits the branches one by one: each commit's wall time, core and tail, in milliseconds.
fn singles(core: &mut SimCore) -> Vec<serde_json::Value> {
    let published = prepare(core);
    settle(core, &published, None);
    branches(core)
        .into_iter()
        .map(|points| {
            let start = Instant::now();
            let outcome = core.commit_road(stroke(points), start, Default::default());
            let commit_ms = start.elapsed().as_secs_f64() * 1e3;
            assert!(outcome.committed, "{}", outcome.rejection_reason);
            let m = &core.last_road_edit_metrics;
            let tail_ms = m.mesh_ms + m.snapshot_ms + m.routing_ms + m.lots_spawn_ms;
            let row = serde_json::json!({"commit_ms": commit_ms, "tail_ms": tail_ms,
                "core_ms": commit_ms - tail_ms, "mesh_ms": m.mesh_ms,
                "snapshot_ms": m.snapshot_ms, "routing_ms": m.routing_ms,
                "lots_spawn_ms": m.lots_spawn_ms, "core_parts": core_parts(m)});
            settle(core, &published, outcome.road_snapshot);
            row
        })
        .collect()
}

// Commits the branches as one batch: its wall time and stages, in milliseconds.
fn batch(core: &mut SimCore) -> serde_json::Value {
    let published = prepare(core);
    settle(core, &published, None);
    let strokes = branches(core).into_iter().map(stroke).collect();
    let start = Instant::now();
    let batch = core.commit_road_batch(
        RoadBatchRequest {
            strokes,
            preview: None,
        },
        start,
        Default::default(),
    );
    let batch_ms = start.elapsed().as_secs_f64() * 1e3;
    assert!(
        batch.outcome.committed,
        "{}",
        batch.outcome.rejection_reason
    );
    let times = batch.times;
    settle(core, &published, batch.outcome.road_snapshot);
    let dry = times.dry_parts;
    serde_json::json!({"batch_ms": batch_ms, "dry_run_ms": times.dry_run_ms,
        "core_ms": times.core_ms, "tail_ms": times.tail_ms, "reused": times.reused,
        "dry_parts": {"plan": dry.plan_ms, "checks": dry.checks_ms, "apply": dry.apply_ms,
            "compile": dry.compile_ms, "install": dry.install_ms, "land": dry.land_ms,
            "restore": dry.restore_ms},
        "core_parts": times.core_parts, "tail_parts": times.tail_parts})
}

// The network products the strokes leave: live edges' ends and geometry, the roadbed's pieces,
// the cells and the parcels.
fn network_products(core: &SimCore, edges_before: usize, nodes_before: u32) -> [u64; 4] {
    let edges: Vec<_> = core
        .region_graph
        .edges()
        .iter()
        .map(|edge| {
            let points: Vec<_> = edge
                .physical_geometry
                .iter()
                .map(|p| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()])
                .collect();
            (edge.deleted, edge.start_node, edge.end_node, points)
        })
        .collect();
    // The pieces of the records the strokes appended (hashing every piece of a city is slow).
    let roadbed = &core.transit_network.roadbed;
    let spans: Vec<_> = (edges_before..core.region_graph.edge_count())
        .map(|edge| roadbed.piece_mesh(PieceId::Span(edge)))
        .collect();
    let nodes: Vec<_> = (nodes_before..core.region_graph.node_count() as u32)
        .map(|node| roadbed.piece_mesh(PieceId::Node(node)))
        .collect();
    [
        debug_hash(&edges),
        debug_hash(&(spans, nodes)),
        core.zoning.cells.state_digest(),
        debug_hash(core.zoning.parcels()),
    ]
}

#[test]
#[ignore = "release ROAD-112 round F batch commit on the K22 cities; loads them outside timing"]
fn benchmark_populated_batch_commit() {
    super::contract::timing_mode();
    let rounds: usize = std::env::var("METRUM_BATCH_ROUNDS")
        .map(|value| value.parse().unwrap())
        .unwrap_or(3);
    let tiers: Vec<u32> = std::env::var("METRUM_COMMIT_TIERS")
        .map(|value| {
            value
                .split(',')
                .map(|t| t.trim().parse().unwrap())
                .collect()
        })
        .unwrap_or_else(|_| vec![30_000]);
    for residents in tiers {
        for round in 0..rounds {
            let (mut core, _) = commit_city(residents);
            let before = (
                core.region_graph.edge_count(),
                core.region_graph.node_count() as u32,
            );
            let singles = singles(&mut core);
            let single_products = network_products(&core, before.0, before.1);
            drop(core);
            let (mut core, _) = commit_city(residents);
            let batch = batch(&mut core);
            let batch_products = network_products(&core, before.0, before.1);
            let cores: f64 = singles
                .iter()
                .map(|row| row["core_ms"].as_f64().unwrap())
                .sum();
            let tails: Vec<f64> = singles
                .iter()
                .map(|row| row["tail_ms"].as_f64().unwrap())
                .collect();
            let mean_tail = tails.iter().sum::<f64>() / tails.len() as f64;
            let bound = cores + mean_tail;
            let batch_ms = batch["batch_ms"].as_f64().unwrap();
            // The redefined bar (the user's option (a), 2026-11-14): the per-stroke cores (the
            // single commits'), the intermediate strokes' lot plans (the batch plans them in
            // their cores) and the batch's one tail; also with a single commit's mean tail.
            let lots: f64 = batch["core_parts"].as_array().unwrap()[..OFFSETS.len() - 1]
                .iter()
                .map(|parts| parts[CORE_PARTS - 1].as_f64().unwrap())
                .sum();
            let bar = cores + lots + batch["tail_ms"].as_f64().unwrap();
            let bar_single_tail = cores + lots + mean_tail;
            // Round LBF's proxy: the batch's own cores, which hold those lot plans but not the
            // plan and compile they install from the dry run (round F2).
            let batch_cores: f64 = batch["core_ms"]
                .as_array()
                .unwrap()
                .iter()
                .map(|ms| ms.as_f64().unwrap())
                .sum();
            let proxy = batch_cores + batch["tail_ms"].as_f64().unwrap();
            eprintln!(
                "BATCH_ROUND {}",
                serde_json::json!({"residents": residents, "round": round,
                    "singles": singles, "batch": batch,
                    "eight_singles_ms": singles.iter().map(|row| row["commit_ms"].as_f64().unwrap()).sum::<f64>(),
                    "eight_cores_one_tail_ms": bound, "batch_over_bound": batch_ms / bound,
                    "intermediate_lots_ms": lots, "bar_ms": bar, "batch_over_bar": batch_ms / bar,
                    "bar_single_tail_ms": bar_single_tail,
                    "batch_over_bar_single_tail": batch_ms / bar_single_tail,
                    "proxy_ms": proxy, "batch_over_proxy": batch_ms / proxy,
                    "graph_roadbed_equal": single_products[..2] == batch_products[..2],
                    "cells_parcels_equal": single_products[2..] == batch_products[2..]})
            );
        }
    }
}

#[test]
#[ignore = "release ROAD-112 round F batch locality; builds each tier's neighbourhood and background outside timing"]
fn benchmark_batch_locality() {
    super::contract::timing_mode();
    let tiers: Vec<usize> = std::env::var("METRUM_LOCALITY_TIERS")
        .map(|value| {
            value
                .split(',')
                .map(|t| t.trim().parse().unwrap())
                .collect()
        })
        .unwrap_or_else(|_| vec![0, 1_000, 10_000, 30_000]);
    let mut reference = None;
    for background in tiers {
        let mut core = super::zoned_city_in_world(
            super::commit_locality::NEIGHBOURHOOD_SIDE_M,
            super::commit_locality::WORLD_M,
        );
        super::commit_locality::add_background(&mut core, background);
        let edges_before = core.region_graph.edge_count();
        let times = batch(&mut core);
        // The batch's own edges by geometry (their ids follow the background's) and the pieces
        // they own.
        let local: Vec<_> = (edges_before..core.region_graph.edge_count())
            .filter(|&edge| !core.region_graph.edge(edge).deleted)
            .map(|edge| {
                let piece = core.transit_network.roadbed.piece_mesh(PieceId::Span(edge));
                let points: Vec<_> = core
                    .region_graph
                    .edge(edge)
                    .physical_geometry
                    .iter()
                    .map(|p| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()])
                    .collect();
                (points, debug_hash(&piece))
            })
            .collect();
        let products = debug_hash(&local);
        eprintln!(
            "BATCH_LOCALITY {}",
            serde_json::json!({"background": background, "edges": edges_before,
                "local_edges": local.len(), "products": format!("{products:016x}"),
                "batch": times})
        );
        match reference {
            None => reference = Some(products),
            Some(reference) => assert_eq!(
                products, reference,
                "a remote background of {background} changed the batch's local products"
            ),
        }
    }
}

// The road tool's view of a prepared city as a context, with the land it takes.
fn preview_context(
    core: &mut SimCore,
    published: &RoadToolShared,
) -> crate::nodes::sim::core::RoadPreviewContext {
    let mut context = crate::nodes::sim::core::RoadPreviewContext::capture(
        published.view.read().unwrap().clone(),
    );
    context.land = core.current_road_tool_land();
    context
}

fn preview_request(
    context: &crate::nodes::sim::core::RoadPreviewContext,
    points: &[Vector3],
) -> crate::nodes::sim::core::RoadPreviewRequest {
    crate::nodes::sim::core::RoadPreviewRequest {
        surface_generation: context.view.surface_generation,
        points: points.to_vec(),
        fwd_lanes: 1,
        bkw_lanes: 1,
        snap_to_existing_roads: true,
        profile: crate::simulation::network::road_edit::RoadProfile::FollowGround,
        median: None,
        parking: crate::simulation::network::graph::RoadParking::NONE,
        attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
    }
}

fn median_max(mut samples: Vec<f64>) -> [f64; 2] {
    samples.sort_by(f64::total_cmp);
    [samples[samples.len() / 2], samples[samples.len() - 1]]
}

/// `benchmark_populated_batch_preview` (`ROAD-112` round G1): on each `METRUM_COMMIT_TIERS` city
/// (default 30k) as the commit breakdown loads it, `BATCH_PREVIEW` prints tier 1 of a capped
/// batch (40 strokes of 200 m, 8 km: the eight round-F branch starts, five copies each 10 m
/// apart) as one stroke's hover check after another and as `validate_batch` with the ghost
/// ribbons (median and max of 21 runs; the bar is 2.0 ms); a single stroke's preview stages
/// (the first branch); tier 2 of the eight branches frame by frame (each frame's stages against
/// the single-stroke stage budgets); the batch commit's dry run with tier 2's products and, on
/// a fresh load, without them, with the products compared.
#[test]
#[ignore = "release ROAD-112 round G1 batch preview on the K22 cities; loads them outside timing"]
fn benchmark_populated_batch_preview() {
    use crate::nodes::sim::road_tool::batch::{ghost_half_width_m, push_ghost_ribbon};
    super::contract::timing_mode();
    let rounds: usize = std::env::var("METRUM_BATCH_ROUNDS")
        .map(|value| value.parse().unwrap())
        .unwrap_or(2);
    let tiers: Vec<u32> = std::env::var("METRUM_COMMIT_TIERS")
        .map(|value| value.split(',').map(|t| t.trim().parse().unwrap()).collect())
        .unwrap_or_else(|_| vec![30_000]);
    for residents in tiers {
        for round in 0..rounds {
            let (mut core, _) = commit_city(residents);
            let before = (
                core.region_graph.edge_count(),
                core.region_graph.node_count() as u32,
            );
            let published = prepare(&mut core);
            settle(&mut core, &published, None);
            let mut context = preview_context(&mut core, &published);
            let branches = branches(&core);
            let capped: Vec<_> = branches
                .iter()
                .flat_map(|[start, _]| {
                    (0..5).map(move |copy| {
                        let start = *start + Vector3::new(10.0 * copy as f32, 0.0, 0.0);
                        [start, start + Vector3::new(0.0, 0.0, 200.0)]
                    })
                })
                .map(|points| preview_request(&context, &points))
                .collect();
            // Tier 1: the hover check one stroke after another, and the batch call.
            let (mut serial, mut tier1) = (Vec::new(), Vec::new());
            let mut ghost = Vec::new();
            for _ in 0..21 {
                let start = Instant::now();
                for request in &capped {
                    std::hint::black_box(context.view.validate(request));
                }
                serial.push(start.elapsed().as_secs_f64() * 1e3);
                let start = Instant::now();
                let hover = context.view.validate_batch(&capped).expect("at the caps");
                ghost.clear();
                for (request, (prepared, _)) in capped.iter().zip(&hover) {
                    push_ghost_ribbon(prepared, ghost_half_width_m(request), &mut ghost);
                }
                tier1.push(start.elapsed().as_secs_f64() * 1e3);
            }
            // A single stroke's preview of the first branch, then the eight branches' tier 2.
            let single = context.preview_road(preview_request(&context, &branches[0]));
            let single = single.times;
            let requests: Vec<_> = branches
                .iter()
                .map(|points| preview_request(&context, points))
                .collect();
            let mut frames = Vec::new();
            let verdict = loop {
                let (batch, frame) = context.preview_batch_step(&requests);
                if let Some(frame) = frame {
                    frames.push([frame.plan_ms, frame.roadbed_ms, frame.payload_ms, frame.graded_ms]);
                }
                // An accepted batch's overlay follows over the next frames (round G1b).
                if batch.pieces.is_some() || batch.verdict.as_ref().is_some_and(|verdict| {
                    *verdict != crate::nodes::sim::core::BatchVerdict::Accepted
                }) {
                    break batch.verdict.clone().expect("tier 2 ended");
                }
            };
            assert_eq!(verdict, crate::nodes::sim::core::BatchVerdict::Accepted);
            let products = context.take_batch_products(&requests);
            let strokes: Vec<_> = branches.iter().copied().map(stroke).collect();
            let start = Instant::now();
            let previewed = core.commit_road_batch(
                RoadBatchRequest {
                    strokes,
                    preview: products,
                },
                start,
                Default::default(),
            );
            let previewed_ms = start.elapsed().as_secs_f64() * 1e3;
            assert!(previewed.outcome.committed);
            settle(&mut core, &published, previewed.outcome.road_snapshot);
            let previewed_products = network_products(&core, before.0, before.1);
            drop(context);
            drop(core);
            let (mut core, _) = commit_city(residents);
            let reference = batch(&mut core);
            let reference_products = network_products(&core, before.0, before.1);
            eprintln!(
                "BATCH_PREVIEW {}",
                serde_json::json!({"residents": residents, "round": round,
                    "capped_strokes": capped.len(),
                    "hover_serial_ms": median_max(serial.clone()),
                    "hover_per_stroke_ms": median_max(serial).map(|ms| ms / capped.len() as f64),
                    "tier1_ms": median_max(tier1),
                    "single_preview": [single.plan_ms, single.roadbed_ms, single.payload_ms, single.graded_ms],
                    "tier2_frames": frames,
                    "commit_previewed_ms": previewed_ms,
                    "dry_run_previewed_ms": previewed.times.dry_run_ms,
                    "preview_reused": previewed.times.preview_reused,
                    "previewed_dry": [previewed.times.dry_parts.plan_ms, previewed.times.dry_parts.compile_ms],
                    "commit_reference": reference,
                    "products_equal": previewed_products == reference_products})
            );
        }
    }
}

// One stroke's preview stages of the first branch, `METRUM_SINGLE_RUNS` times (default 11):
// the median of each stage.
fn single_preview_stages(
    context: &mut crate::nodes::sim::core::RoadPreviewContext,
    points: &[Vector3],
) -> [f64; 4] {
    let runs: usize = std::env::var("METRUM_SINGLE_RUNS")
        .map(|value| value.parse().unwrap())
        .unwrap_or(11);
    let mut stages: [Vec<f64>; 4] = Default::default();
    for _ in 0..runs {
        let times = context.preview_road(preview_request(context, points)).times;
        for (stage, ms) in stages.iter_mut().zip([
            times.plan_ms,
            times.roadbed_ms,
            times.payload_ms,
            times.graded_ms,
        ]) {
            stage.push(ms);
        }
    }
    stages.map(|samples| median_max(samples)[0])
}

/// `benchmark_populated_batch_overlay` (`ROAD-112` round G1b): on each `METRUM_COMMIT_TIERS`
/// city (default 30k) as the commit breakdown loads it, `BATCH_OVERLAY` prints a single
/// stroke's preview stages (the first branch, medians of `METRUM_SINGLE_RUNS`) and tier 2 of
/// the eight branches `METRUM_TIER2_RUNS` times (default 5) to its whole overlay: each frame's
/// median stages `[plan, roadbed, payload, graded]`, the largest frame payload, the one-call
/// overlay's time on the same replica (round G1's accepting frame) and whether the frames'
/// overlay equals it bit for bit. The bar: no frame's payload over the control's single stroke.
#[test]
#[ignore = "release ROAD-112 round G1b batch overlay on the K22 cities; loads them outside timing"]
fn benchmark_populated_batch_overlay() {
    super::contract::timing_mode();
    let rounds: usize = std::env::var("METRUM_BATCH_ROUNDS")
        .map(|value| value.parse().unwrap())
        .unwrap_or(1);
    let tier2_runs: usize = std::env::var("METRUM_TIER2_RUNS")
        .map(|value| value.parse().unwrap())
        .unwrap_or(5);
    let tiers: Vec<u32> = std::env::var("METRUM_COMMIT_TIERS")
        .map(|value| value.split(',').map(|t| t.trim().parse().unwrap()).collect())
        .unwrap_or_else(|_| vec![30_000]);
    for residents in tiers {
        for round in 0..rounds {
            let (mut core, _) = commit_city(residents);
            let published = prepare(&mut core);
            settle(&mut core, &published, None);
            let mut context = preview_context(&mut core, &published);
            let branches = branches(&core);
            let single = single_preview_stages(&mut context, &branches[0]);
            let requests: Vec<_> = branches
                .iter()
                .map(|points| preview_request(&context, points))
                .collect();
            let mut frames: Vec<[Vec<f64>; 4]> = Vec::new();
            let (mut one_call, mut equal) = (Vec::new(), true);
            for _ in 0..tier2_runs {
                context.clear_batch();
                let mut index = 0;
                loop {
                    let (batch, frame) = context.preview_batch_step(&requests);
                    let frame = frame.expect("tier 2 runs to its overlay");
                    if frames.len() == index {
                        frames.push(Default::default());
                    }
                    for (stage, ms) in frames[index].iter_mut().zip([
                        frame.plan_ms,
                        frame.roadbed_ms,
                        frame.payload_ms,
                        frame.graded_ms,
                    ]) {
                        stage.push(ms);
                    }
                    index += 1;
                    if batch.pieces.is_some() {
                        break;
                    }
                    assert!(
                        !matches!(
                            batch.verdict,
                            Some(crate::nodes::sim::core::BatchVerdict::Refused { .. })
                        ),
                        "the branches are accepted"
                    );
                }
                let start = Instant::now();
                let whole = context
                    .batch_overlay_in_one_call()
                    .expect("a finished batch")
                    .expect("the view's chunks compose");
                one_call.push(start.elapsed().as_secs_f64() * 1e3);
                let built = context.batch().and_then(|batch| batch.pieces.as_ref());
                let built = built.and_then(|pieces| pieces.chunks.as_ref()).expect("the overlay");
                equal &= built.len() == whole.len()
                    && built.iter().all(|(chunk, preview)| {
                        whole
                            .get(chunk)
                            .is_some_and(|other| preview.same_preview(other))
                    });
            }
            let frames: Vec<[f64; 4]> = frames
                .into_iter()
                .map(|stages| stages.map(|samples| median_max(samples)[0]))
                .collect();
            let max_payload = frames.iter().map(|frame| frame[2]).fold(0.0, f64::max);
            eprintln!(
                "BATCH_OVERLAY {}",
                serde_json::json!({"residents": residents, "round": round,
                    "single_preview": single,
                    "tier2_frames": frames,
                    "max_frame_payload_ms": max_payload,
                    "one_call_overlay_ms": median_max(one_call),
                    "overlay_equal": equal})
            );
        }
    }
}
