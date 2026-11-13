// SPDX-License-Identifier: GPL-2.0-only

//! Batch commits on populated cities (`ROAD-112` round F, `docs/roads.md` "Road tools").
//!
//! `benchmark_populated_batch_commit`: on each `METRUM_COMMIT_TIERS` city as the commit
//! breakdown loads it (`ROAD-68-K22`), eight 120 m branches north off the border trunk, 50 m
//! apart, committed one by one with the frames to routing adoption between them (untimed), then,
//! on a fresh load of the same city, as one batch. `BATCH_ROUND` prints each single commit's wall
//! time split into its core and its tail (mesh precompute, publication, routing schedule, lot
//! spawn) and the batch's dry run, cores and tail; the bar is the batch's wall time against the
//! eight single cores plus one mean tail. Rounds alternate (`METRUM_BATCH_ROUNDS`, default 3).
//!
//! `benchmark_batch_locality`: the same batch on the commit locality's neighbourhood
//! (`commit_locality`) with a remote background of `METRUM_LOCALITY_TIERS` sites; the batch's
//! local products must be bitwise equal at every tier and its stages flat, the tail apart.

use std::time::Instant;

use godot::prelude::Vector3;

use super::routing_benchmark::{commit_city, trunk_branch_end, trunk_branch_starts_at};
use crate::debug::work::debug_hash;
use crate::nodes::sim::core::frame::FRAME_DT_S;
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
        tunnel: false,
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
                "snapshot_ms": m.snapshot_ms});
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
    let batch = core.commit_road_batch(RoadBatchRequest { strokes }, start, Default::default());
    let batch_ms = start.elapsed().as_secs_f64() * 1e3;
    assert!(
        batch.outcome.committed,
        "{}",
        batch.outcome.rejection_reason
    );
    let times = batch.times;
    settle(core, &published, batch.outcome.road_snapshot);
    serde_json::json!({"batch_ms": batch_ms, "dry_run_ms": times.dry_run_ms,
        "core_ms": times.core_ms, "tail_ms": times.tail_ms, "reused": times.reused})
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
            let bound = cores + tails.iter().sum::<f64>() / tails.len() as f64;
            let batch_ms = batch["batch_ms"].as_f64().unwrap();
            eprintln!(
                "BATCH_ROUND {}",
                serde_json::json!({"residents": residents, "round": round,
                    "singles": singles, "batch": batch,
                    "eight_singles_ms": singles.iter().map(|row| row["commit_ms"].as_f64().unwrap()).sum::<f64>(),
                    "eight_cores_one_tail_ms": bound, "batch_over_bound": batch_ms / bound,
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
