// SPDX-License-Identifier: GPL-2.0-only

//! Road edit latency next to junctions without a retained topology (`ROAD-44`); fixture
//! construction, warming and undo are never timed.

use super::super::ROAD_LOCKED_TERRAIN_RENDER_STEP_M;
use super::super::populated_city::street_grid;
use super::super::road_commit::RoadCommitRequest;
use super::{RoadPreviewRequest, SimCore, road_tool_snapshots_from_core};
use crate::nodes::sim::core::road_preview::compile_road_preview_from_context;
use crate::simulation::network::surface::{RoadSurfaceVisualNodePiece, RoadSurfaceVisualSpanPiece};
use godot::prelude::Vector3;
use std::time::Instant;

// The street grid of the 10k-resident `PopulatedCity`: streets every 50 m from z = -1000 to 1000,
// cross streets every 400 m from x = -1000 to 1000, so rows end in T junctions on the east edge.
// It is left unzoned: the city's zoning covers the first 24 m past every street end, so these
// extensions would need a bulldoze first, and the junction compiles timed here do not read
// zoning.
const CITY_SIDE_M: f32 = 2041.0;
const EAST_EDGE_X: f32 = 1000.0;
const ROW_Z: f32 = 300.0;
const TOP_ROW_Z: f32 = 1000.0;
const EXTENSION_M: f32 = 80.0;
// Wide enough to reach the neighbouring junctions on the edge street, not the next cross street.
const NEIGHBOURHOOD_M: f32 = 120.0;
const REPETITIONS: usize = 15;

struct Case {
    name: &'static str,
    points: [(f32, f32); 2],
}

const CASES: [Case; 3] = [
    // A row end becomes a four-way junction.
    Case {
        name: "t_to_four_way",
        points: [(EAST_EDGE_X, ROW_Z), (EAST_EDGE_X + EXTENSION_M, ROW_Z)],
    },
    // The north-east corner bend becomes a T.
    Case {
        name: "bend_to_t",
        points: [(EAST_EDGE_X, TOP_ROW_Z), (EAST_EDGE_X + EXTENSION_M, TOP_ROW_Z)],
    },
    // A new T on the edge street 25 m from the T junctions on either side.
    Case {
        name: "split_between_ts",
        points: [
            (EAST_EDGE_X, ROW_Z + 25.0),
            (EAST_EDGE_X + EXTENSION_M, ROW_Z + 25.0),
        ],
    },
];

#[derive(Default)]
struct Samples {
    preview_ms: Vec<f64>,
    commit_ms: Vec<f64>,
    commit_surface_ms: Vec<f64>,
}

type LocalProducts = (
    Vec<(u32, RoadSurfaceVisualNodePiece)>,
    Vec<(usize, RoadSurfaceVisualSpanPiece)>,
);

#[test]
#[ignore = "unprofiled latency measurement; run alone with --release --ignored --nocapture"]
fn cold_node_edit_latency() {
    let (mut core, _, _) = street_grid(CITY_SIDE_M);
    finalize_network(&mut core);
    // Gameplay keeps undo history; the benchmark city does not.
    core.benchmark_mode = false;
    for case in &CASES {
        let points = case
            .points
            .map(|(x, z)| Vector3::new(x, 0.0, z))
            .to_vec();
        let mut samples = [Samples::default(), Samples::default()];
        let mut expected: Option<LocalProducts> = None;
        for repetition in 0..REPETITIONS * 2 {
            // Alternate which state goes first so drift affects both alike.
            let warm = (repetition % 2 == 0) == (repetition / 2 % 2 == 0);
            set_neighbourhood_topologies(&mut core, points[0], warm);
            let generation = core
                .transit_network
                .road_surface
                .compile_invalidation_generation;
            let (context, _) = road_tool_snapshots_from_core(&core).unwrap();
            let request = RoadPreviewRequest {
                enqueued_at: None,
                include_terrain: false,
                request_id: repetition as u64,
                surface_generation: core.road_tool_surface_generation,
                points: points.clone(),
                fwd_lanes: 1,
                bkw_lanes: 1,
                snap_to_existing_roads: true,
            };
            let started = Instant::now();
            let preview = compile_road_preview_from_context(&context, request);
            let preview_ms = started.elapsed().as_secs_f64() * 1000.0;
            drop(context);
            assert!(
                preview.is_valid,
                "{}: {:?}",
                case.name, preview.validation.invalid_reason
            );
            let started = Instant::now();
            let outcome = core.commit_road(
                RoadCommitRequest {
                    points: points.clone(),
                    fwd_lanes: 1,
                    bkw_lanes: 1,
                    snap_to_existing_roads: true,
                    edit_plan: preview.edit_plan(),
                },
                Instant::now(),
                Default::default(),
            );
            let commit_ms = started.elapsed().as_secs_f64() * 1000.0;
            assert!(outcome.committed, "{}: {}", case.name, outcome.rejection);
            assert!(core.last_road_edit_metrics.preview_plan_reused);
            let products = local_products(&core, points[0]);
            match &expected {
                Some(expected) => assert!(
                    &products == expected,
                    "{}: products differ between cold and warm commits",
                    case.name
                ),
                None => expected = Some(products),
            }
            let state = &mut samples[usize::from(warm)];
            state.preview_ms.push(preview_ms);
            state.commit_ms.push(commit_ms);
            state.commit_surface_ms.push(outcome.surface_ms);
            assert!(core.undo_action_internal(), "{}: undo failed", case.name);
            finalize_network(&mut core);
            assert_ne!(
                generation,
                core.transit_network
                    .road_surface
                    .compile_invalidation_generation,
                "the next preview must not replay this one's cursor cache"
            );
        }
        for (state, samples) in ["cold", "warm"].iter().zip(&mut samples) {
            println!(
                "COLD_NODE_EDIT_LATENCY {}",
                serde_json::json!({
                    "case": case.name, "state": state,
                    "rayon_threads": rayon::current_num_threads(),
                    "samples": samples.preview_ms.len(),
                    "preview_ms": summary(&mut samples.preview_ms),
                    "commit_ms": summary(&mut samples.commit_ms),
                    "commit_surface_ms": summary(&mut samples.commit_surface_ms),
                })
            );
        }
    }
}

// Evicts every topology (the state after a bulk build), then optionally recompiles the nodes
// around `centre` so they retain theirs, as every node did before `ROAD-44`.
fn set_neighbourhood_topologies(core: &mut SimCore, centre: Vector3, warm: bool) {
    core.transit_network
        .road_surface
        .compiled_visual_node_topologies
        .clear();
    if !warm {
        return;
    }
    let nodes = neighbourhood_nodes(core, centre);
    let surface = &mut core.transit_network.road_surface;
    for &node in &nodes {
        // A node whose input is unchanged is reused or only refreshes earthwork; dropping the
        // stored input forces the full compile that produces a topology.
        surface.compiled_visual_node_inputs.remove(&node);
        surface.mark_node_dirty(&core.region_graph, node);
    }
    finalize_network(core);
    let topologies = &core.transit_network.road_surface.compiled_visual_node_topologies;
    assert!(nodes.iter().all(|node| topologies.contains_key(node)));
}

// As the simulation thread finalizes a loaded or undone network, so the next commit's terrain
// plan sees no surface or earthwork work left over from setup.
fn finalize_network(core: &mut SimCore) {
    core.rebuild_network_surface_terrain_internal();
    core.precompute_road_mesh_data();
    core.refresh_road_locked_terrain_patch_state(ROAD_LOCKED_TERRAIN_RENDER_STEP_M);
    let inputs = core.collect_refined_terrain_patch_build_inputs(ROAD_LOCKED_TERRAIN_RENDER_STEP_M);
    let entries = SimCore::build_refined_terrain_patch_cache_entries(inputs);
    core.insert_refined_terrain_patch_cache_entries(entries);
}

fn neighbourhood_nodes(core: &SimCore, centre: Vector3) -> Vec<u32> {
    let mut nodes: Vec<u32> = core
        .transit_network
        .road_surface
        .compiled_visual_node_pieces
        .keys()
        .copied()
        .filter(|&node| {
            let pos = core.region_graph.node(node).pos;
            (pos.x - centre.x).hypot(pos.z - centre.z) <= NEIGHBOURHOOD_M
        })
        .collect();
    nodes.sort_unstable();
    assert!(!nodes.is_empty());
    nodes
}

fn local_products(core: &SimCore, centre: Vector3) -> LocalProducts {
    let surface = &core.transit_network.road_surface;
    let nodes = neighbourhood_nodes(core, centre);
    let mut edges: Vec<usize> = nodes
        .iter()
        .flat_map(|&node| core.region_graph.node_adjacency(node).iter().copied())
        .collect();
    edges.sort_unstable();
    edges.dedup();
    (
        nodes
            .iter()
            .map(|&node| {
                let piece = surface.compiled_visual_node_pieces.get(&node).unwrap();
                (node, piece.as_ref().clone())
            })
            .collect(),
        edges
            .into_iter()
            .filter_map(|edge| {
                surface
                    .compiled_visual_span_pieces
                    .get(&edge)
                    .map(|piece| (edge, piece.as_ref().clone()))
            })
            .collect(),
    )
}

fn summary(samples: &mut [f64]) -> serde_json::Value {
    samples.sort_by(f64::total_cmp);
    let n = samples.len();
    serde_json::json!({
        "min": samples[0],
        "p50": samples[n / 2],
        "max": samples[n - 1],
    })
}
