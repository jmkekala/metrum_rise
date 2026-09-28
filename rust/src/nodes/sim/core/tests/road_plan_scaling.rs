// SPDX-License-Identifier: GPL-2.0-only

//! Isolated populated-city planning benchmark; fixture construction is never timed.

use super::*;
use crate::nodes::sim::core::RoadEditPlan;
use crate::simulation::network::surface::RoadSurfaceSystem;
use godot::prelude::Vector2;
use std::time::Instant;

mod memory;

#[derive(Clone, Copy, PartialEq)]
enum CellScenario {
    None,
    Placement,
    NodeMovement,
    Snapshot,
    Memory,
}

#[test]
#[ignore = "unprofiled scaling measurement; run alone with --release --ignored --nocapture"]
fn populated_road_plan_scaling() {
    measure_populated_road_plan_scaling(false, false, false, CellScenario::None);
}

#[test]
#[ignore = "unprofiled field reservation scaling; run alone with --release --ignored --nocapture"]
fn populated_field_road_plan_scaling() {
    measure_populated_road_plan_scaling(false, false, true, CellScenario::None);
}

#[test]
#[ignore = "unprofiled paved-site scaling measurement; run alone with --release --ignored --nocapture"]
fn populated_paved_road_plan_scaling() {
    measure_populated_road_plan_scaling(true, false, false, CellScenario::None);
}

#[test]
#[ignore = "unprofiled cached zoning feasibility scaling; run alone with --release --ignored --nocapture"]
fn populated_zoning_feasibility_scaling() {
    measure_populated_road_plan_scaling(true, true, false, CellScenario::None);
}

#[test]
#[ignore = "matched populated cell-reservation planning locality; run without competing work"]
fn populated_cell_road_plan_scaling() {
    measure_populated_road_plan_scaling(false, false, false, CellScenario::Placement);
}

#[test]
#[ignore = "snapshot component diagnostic; run release alone without profiling"]
fn populated_cell_snapshot_scaling() {
    measure_populated_road_plan_scaling(false, false, false, CellScenario::Snapshot);
}

#[test]
#[ignore = "populated node-move reservation validation; run release alone without profiling"]
fn populated_cell_node_move_validation_scaling() {
    // Opt into existing compiler phase diagnostics only for a separate diagnostic run.
    // Acceptance timings keep METRUM_DEBUG=0 and never initialize logging here.
    if std::env::var("METRUM_DEBUG").is_ok_and(|value| value == "1") {
        crate::debug::init();
    }
    measure_populated_road_plan_scaling(false, false, false, CellScenario::NodeMovement);
}

fn measure_populated_road_plan_scaling(
    paved: bool,
    zoning_only: bool,
    fields: bool,
    scenario: CellScenario,
) {
    let mut memory_phase = 0;
    let mut previous_memory_products = None;
    if scenario == CellScenario::Memory {
        memory::calibrate(&mut memory_phase);
    }
    let cells = scenario != CellScenario::None;
    let mut core = test_core();
    road_terrain_plan::commit_ready(
        &mut core,
        vec![Vector3::new(-96.0, 0.0, 0.0), Vector3::new(96.0, 0.0, 0.0)],
    );
    let asset = register_test_asset(&mut core.allocator, "scaling_site", ZoneType::Residential);
    if paved {
        let mut manifest = core
            .allocator
            .registry
            .get(&asset)
            .unwrap()
            .manifest
            .clone();
        // Only the fixed four local sites are paved; remote records retain the matched baseline.
        manifest.asset_id = "scaling_paved_site".into();
        manifest.site_surfaces = toml::from_str::<AssetManifest>(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../benchmarks/fixtures/kuopio-terrain/building-site.toml"
        )))
        .unwrap()
        .site_surfaces;
        core.allocator
            .registry
            .register("test", manifest, String::new());
    }
    add_test_complete_building(&mut core, asset, ZoneType::Residential);
    let template = core.allocator.buildings[0].clone();
    // Four unchanged occupied sites flank the measured T. These exercise real site
    // grading and parcel dependencies, even when the remote population is empty.
    core.allocator.buildings.clear();
    for x in [-72.0, -24.0, 24.0, 72.0] {
        let mut building = template.clone();
        if paved {
            building.asset_id = "test:scaling_paved_site".into();
        }
        building.center_x = x;
        building.center_y = 32.0;
        building.frontage_t = (x + 96.0) / 192.0;
        building.facing_dir = Vector2::new(0.0, -1.0);
        building.width_cells = 1;
        building.depth_cells = 1;
        if paved {
            building.width_cells = 2;
            building.depth_cells = 2;
        }
        building.occupancy = 6;
        insert_populated_site(&mut core, building);
    }
    let edge_template = core.region_graph.edge(0).clone();
    let mut background_roads = 0;
    let mut previous_patches: Option<Vec<_>> = None;
    let mut initial_site_solves = None;
    let mut background_cells = 0;
    let mut previous_move_surfaces: [Option<RoadSurfaceSystem>; 2] = [None, None];
    let cell_grid = if cells {
        use crate::simulation::zoning::cells::{CellBounds, CellSelectionShape, GridFrame};
        use glam::DVec2;
        core.zoning.generate_cells(
            &core.region_graph,
            CellBounds {
                min: DVec2::splat(-200.0),
                max: DVec2::splat(200.0),
            },
            |_| false,
        );
        let selection = core
            .zoning
            .cells
            .select(CellSelectionShape::Cell, &[DVec2::new(55.0, -12.0)]);
        assert!(!selection.cells.is_empty());
        core.zoning.paint_cells(&selection, 1, |_| false).unwrap();
        let grid = core.zoning.cells.saved_frames().count() as u64 + 1;
        assert!(
            core.zoning
                .cells
                .restore_saved_frame(grid, GridFrame::new(DVec2::ZERO, DVec2::X, 10.0).unwrap())
        );
        grid
    } else {
        0
    };
    if fields {
        core.allocator.field_clearance.set(
            0,
            &[
                Vector2::new(100.0, 60.0),
                Vector2::new(120.0, 60.0),
                Vector2::new(120.0, 80.0),
                Vector2::new(100.0, 80.0),
            ],
        );
    }
    for remote_buildings in [0usize, 1_000, 10_000, 100_000] {
        // Detached fixture insertion uses indexed graph/parcel mutators. Each remote
        // street fronts a row of 256 lots, outside the measured edit's neighborhood.
        let target_roads = remote_buildings.div_ceil(256);
        while background_roads < target_roads {
            let x = -8204.0;
            let z = 1014.0 + background_roads as f32 * 20.0;
            let mut edge = edge_template.clone();
            edge.geometry = vec![Vector3::new(x, 0.0, z), Vector3::new(x + 6144.0, 0.0, z)];
            edge.physical_geometry = edge.geometry.clone();
            edge.start_node = core
                .region_graph
                .add_node(edge.geometry[0], NodeType::Junction);
            edge.end_node = core
                .region_graph
                .add_node(edge.geometry[1], NodeType::Junction);
            edge.start_clip = 0.0;
            edge.end_clip = 0.0;
            edge.physical_length = 6144.0;
            let id = core.region_graph.add_edge(edge);
            core.transit_network
                .road_surface
                .mark_edge_dirty(&core.region_graph, id);
            background_roads += 1;
        }
        while core.allocator.buildings.len() < remote_buildings + 4 {
            let index = core.allocator.buildings.len() - 4;
            let mut building = template.clone();
            building.center_x = -8192.0 + (index % 256) as f32 * 24.0;
            building.center_y = 1024.0 + (index / 256) as f32 * 20.0;
            building.width_cells = 1;
            building.depth_cells = 1;
            building.facing_dir = Vector2::new(0.0, -1.0);
            building.edge_idx = 1 + index / 256;
            building.frontage_t = ((index % 256) as f32 * 24.0 + 12.0) / 6144.0;
            building.occupancy = 6;
            if fields {
                let p = Vector2::new(building.center_x, building.center_y + 12.0);
                core.allocator.field_clearance.set(
                    index + 4,
                    &[
                        p,
                        p + Vector2::new(8.0, 0.0),
                        p + Vector2::new(8.0, 4.0),
                        p + Vector2::new(0.0, 4.0),
                    ],
                );
            }
            insert_populated_site(&mut core, building);
        }
        if cells {
            // Reserved cells occupy separate distant land, outside the background parcels.
            for index in background_cells..remote_buildings {
                assert!(core.zoning.cells.restore_saved_cell(
                    crate::simulation::zoning::cells::CellKey {
                        grid: cell_grid,
                        x: 200 + (index % 700) as i32,
                        y: 200 + (index / 700) as i32,
                    },
                    1
                ));
            }
            background_cells = remote_buildings;
        }
        core.allocator.dirty_index = true;
        core.allocator
            .rebuild_building_site_clients(core.config.zone_cell_m);
        core.allocator
            .prepare_building_site_query_index(core.config.zone_cell_m);
        core.precompute_road_mesh_data();
        assert!(
            core.transit_network
                .road_surface
                .published_generation_matches_source()
        );
        if scenario == CellScenario::Memory {
            let products = memory::measure(&mut core, remote_buildings, &mut memory_phase);
            if let Some(previous) = &previous_memory_products {
                assert_eq!(
                    &products, previous,
                    "remote population changed local selections"
                );
            }
            previous_memory_products = Some(products);
            continue;
        }
        if scenario == CellScenario::Snapshot {
            measure_snapshot_components(&core, remote_buildings);
            continue;
        }
        if scenario == CellScenario::NodeMovement {
            let node = core.region_graph.edge(0).end_node;
            let original_position = core.region_graph.node(node).pos;
            for (index, (position, blocked)) in [
                (Vector3::new(120.0, 0.0, 0.0), false),
                (Vector3::new(96.0, 0.0, -12.0), true),
            ]
            .into_iter()
            .enumerate()
            {
                let check = || {
                    let surface = core
                        .transit_network
                        .road_surface
                        .compile_node_move_surface(
                            &core.region_graph,
                            &core.heightmap,
                            &core.zoning,
                            node,
                            position,
                        )
                        .unwrap();
                    assert_eq!(surface.overlaps_cell_zoning(&core.zoning), blocked);
                    surface
                };
                for _ in 0..5 {
                    std::hint::black_box(check());
                }
                let mut samples = Vec::with_capacity(100);
                for _ in 0..100 {
                    let start = Instant::now();
                    let surface = std::hint::black_box(check());
                    drop(surface);
                    samples.push(start.elapsed().as_secs_f64() * 1000.0);
                }
                samples.sort_by(f64::total_cmp);
                let surface = check();
                if let Some(previous) = &previous_move_surfaces[index] {
                    assert_eq!(
                        surface.compiled_visual_span_pieces(),
                        previous.compiled_visual_span_pieces()
                    );
                    assert_eq!(
                        surface.compiled_visual_node_pieces(),
                        previous.compiled_visual_node_pieces()
                    );
                }
                assert_eq!(core.region_graph.node(node).pos, original_position);
                println!(
                    "CELL_NODE_MOVE_VALIDATION_SCALING {}",
                    serde_json::json!({
                        "remote_buildings": remote_buildings, "remote_roads": background_roads,
                        "agents": core.agents.len(), "parcels": core.zoning.parcels().len(),
                        "remote_cells": background_cells, "blocked": blocked,
                        "rayon_threads": rayon::current_num_threads(), "samples": samples.len(),
                        "p50_ms": (samples[49] + samples[50]) * 0.5, "p95_ms": samples[94],
                        "spans": surface.compiled_visual_span_pieces().len(),
                        "nodes": surface.compiled_visual_node_pieces().len(),
                        "identical_local_products": true,
                    })
                );
                previous_move_surfaces[index] = Some(surface);
            }
            continue;
        }
        if zoning_only {
            let geometry = core
                .zoning
                .preview_parcel_at(0.0, 16.0, 20.0, 20.0, &core.region_graph)
                .unwrap();
            let check = || {
                core.allocator.zoning_site_feasibility(
                    &geometry,
                    1,
                    &core.zoning,
                    &core.region_graph,
                    crate::simulation::buildings::allocator::BuildingSiteEnvironment {
                        road_surface: &core.transit_network.road_surface,
                        terrain: &core.heightmap,
                    },
                )
            };
            let first = Instant::now();
            assert!(check().is_ok());
            let first_ms = first.elapsed().as_secs_f64() * 1000.0;
            let solves = core.allocator.site_feasibility_solve_count();
            assert_eq!(
                solves,
                *initial_site_solves.get_or_insert(solves),
                "remote edits repeated local geometry solving"
            );
            let mut samples = Vec::new();
            for _ in 0..300 {
                let start = Instant::now();
                std::hint::black_box(check()).unwrap();
                samples.push(start.elapsed().as_secs_f64() * 1e6);
            }
            samples.sort_by(f64::total_cmp);
            println!(
                "ZONING_FEASIBILITY_SCALING {}",
                serde_json::json!({
                    "remote_buildings": remote_buildings, "remote_roads": background_roads,
                    "agents": core.agents.len(), "parcels": core.zoning.parcels().len(),
                    "first_or_revalidate_ms": first_ms, "warm_p50_us": samples[150], "warm_p95_us": samples[284],
                    "local_solves": solves, "samples": samples.len(), "rayon_threads": rayon::current_num_threads(),
                })
            );
            continue;
        }
        let snapshot_start = Instant::now();
        let (context, _) = road_tool_snapshots_from_core(&core).unwrap();
        let snapshot_ms = snapshot_start.elapsed().as_secs_f64() * 1000.0;
        let mut samples = Vec::new();
        let mut status_samples = Vec::new();
        let mut worker_samples = Vec::new();
        let generation = core.road_tool_surface_generation;
        let request = || RoadPreviewRequest {
            request_id: 1,
            surface_generation: generation,
            points: vec![Vector3::new(0.0, 0.0, -80.0), Vector3::new(0.0, 0.0, 0.0)],
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
        };
        for iteration in 0..103 {
            let start = Instant::now();
            let plan = RoadEditPlan::compile(&core, request());
            let compile_ms = start.elapsed().as_secs_f64() * 1000.0;
            let status_start = Instant::now();
            assert_eq!(plan.status(&core), "ready");
            let status_ms = status_start.elapsed().as_secs_f64() * 1000.0;
            let patches = plan.terrain().unwrap().preview_patches().unwrap();
            if let Some(previous) = previous_patches.as_ref() {
                assert!(
                    plan.terrain().unwrap().entries_match(previous),
                    "remote state changed local products"
                );
            } else {
                assert!(patches.iter().any(|patch| patch.site_clip_loop_count > 0));
                previous_patches = Some(patches);
            }
            let worker_start = Instant::now();
            let preview = crate::nodes::sim::core::road_preview::compile_road_preview_with_sites(
                &context,
                request(),
                &mut core,
            );
            let worker_ms = worker_start.elapsed().as_secs_f64() * 1000.0;
            assert!(preview.is_valid && preview.junction_preview.is_some());
            let worker_plan = preview.edit_plan().unwrap();
            assert_eq!(worker_plan.status(&core), "ready");
            assert!(
                worker_plan
                    .terrain()
                    .unwrap()
                    .entries_match(previous_patches.as_ref().unwrap())
            );
            if iteration >= 3 {
                samples.push(compile_ms);
                status_samples.push(status_ms);
                worker_samples.push(worker_ms);
            }
        }
        samples.sort_by(f64::total_cmp);
        status_samples.sort_by(f64::total_cmp);
        worker_samples.sort_by(f64::total_cmp);
        println!(
            "ROAD_PLAN_SCALING {}",
            serde_json::json!({
                "remote_buildings": remote_buildings, "remote_roads": background_roads,
                "field_reservations": fields,
                "cell_reservations": cells, "remote_painted_cells": background_cells,
                "agents": core.agents.len(), "parcels": core.zoning.parcels.parcels().len(),
                "rayon_threads": rayon::current_num_threads(), "samples": samples.len(),
                "compile_p50_ms": (samples[49] + samples[50]) * 0.5, "compile_p95_ms": samples[94],
                "readiness_p50_ms": (status_samples[49] + status_samples[50]) * 0.5, "readiness_p95_ms": status_samples[94],
                "worker_p50_ms": (worker_samples[49] + worker_samples[50]) * 0.5, "worker_p95_ms": worker_samples[94],
                "snapshot_once_per_edit_ms": snapshot_ms,
                "identical_local_products": true,
            })
        );
    }
}

fn measure_snapshot_components(core: &SimCore, background: usize) {
    use std::hint::black_box;
    let mut samples: [Vec<f64>; 6] = std::array::from_fn(|_| Vec::with_capacity(21));
    for iteration in 0..24 {
        let start = Instant::now();
        let terrain = black_box(core.heightmap.clone());
        let terrain_us = start.elapsed().as_secs_f64() * 1_000_000.0;
        let start = Instant::now();
        let graph = black_box(core.region_graph.clone());
        let graph_us = start.elapsed().as_secs_f64() * 1_000_000.0;
        let start = Instant::now();
        let surface = black_box(core.transit_network.road_surface.clone());
        let surface_us = start.elapsed().as_secs_f64() * 1_000_000.0;
        let start = Instant::now();
        let water = black_box(core.watermap.clone());
        let water_us = start.elapsed().as_secs_f64() * 1_000_000.0;
        assert_eq!(graph.edge_count(), core.region_graph.edge_count());
        assert!(surface.published_generation_matches_source());
        drop((terrain, graph, surface, water));
        let start = Instant::now();
        let snapshot = black_box(road_tool_snapshots_from_core(core).unwrap());
        let snapshot_us = start.elapsed().as_secs_f64() * 1_000_000.0;
        let start = Instant::now();
        drop(snapshot);
        let release_us = start.elapsed().as_secs_f64() * 1_000_000.0;
        if iteration >= 3 {
            for (values, value) in samples.iter_mut().zip([
                terrain_us,
                graph_us,
                surface_us,
                water_us,
                snapshot_us,
                release_us,
            ]) {
                values.push(value);
            }
        }
    }
    let mut result = serde_json::json!({
        "background": background, "roads": core.region_graph.edge_count(),
        "agents": core.agents.len(), "parcels": core.zoning.parcels().len(),
        "samples": 21, "rayon_threads": rayon::current_num_threads(),
    });
    for (name, values) in [
        "terrain", "graph", "surface", "water", "snapshot", "release",
    ]
    .into_iter()
    .zip(samples.iter_mut())
    {
        values.sort_by(f64::total_cmp);
        result[name] = serde_json::json!({ "p50_us": values[10], "p95_us": values[19] });
    }
    println!("CELL_SNAPSHOT_COMPONENTS {result}");
    measure_surface_snapshot_fields(&core.transit_network.road_surface, background);
}

fn measure_surface_snapshot_fields(surface: &RoadSurfaceSystem, background: usize) {
    fn clone_us<T: Clone>(value: &T) -> f64 {
        let mut samples = [0.0_f64; 21];
        for iteration in 0..24 {
            let start = Instant::now();
            let cloned = std::hint::black_box(value.clone());
            let elapsed = start.elapsed().as_secs_f64() * 1_000_000.0;
            drop(cloned);
            if iteration >= 3 {
                samples[iteration - 3] = elapsed;
            }
        }
        samples.sort_by(f64::total_cmp);
        samples[10]
    }
    let mut result = serde_json::json!({ "background": background });
    macro_rules! measure {
        ($($field:ident),+ $(,)?) => {
            $(result[stringify!($field)] = serde_json::json!(clone_us(&surface.$field));)+
        };
    }
    measure!(
        compiled_sections,
        compiled_visual_span_pieces,
        compiled_visual_node_pieces,
        compiled_visual_node_inputs,
        compiled_visual_node_earthwork_boundaries,
        compiled_visual_node_topologies,
        surface_span_chunks,
        surface_node_chunks,
        earthwork_span_chunks,
        earthwork_node_chunks,
        query_span_chunks,
        query_node_chunks,
        surface_chunk_cache,
        earthwork_chunk_cache,
        last_rebuilt_surface_chunks,
        last_rebuilt_terrain_chunks,
        last_rebuilt_query_chunks,
    );
    println!("CELL_SURFACE_SNAPSHOT_FIELDS {result}");
}

fn insert_populated_site(core: &mut SimCore, mut building: Building) {
    let p = Vector2::new(building.center_x, building.center_y);
    let half = core.config.zone_cell_m * 0.5;
    building.parcel_id = core
        .zoning
        .parcels
        .insert_new(
            crate::simulation::zoning::ParcelGeometry {
                edge_idx: building.edge_idx,
                side: 1,
                frontage_center_t: building.frontage_t,
                frontage_m: half * 2.0,
                depth_m: half * 2.0,
                front_center: p - Vector2::new(0.0, half),
                center: p,
                tangent: Vector2::RIGHT,
                normal: Vector2::DOWN,
                corners: [
                    p + Vector2::new(-half, -half),
                    p + Vector2::new(half, -half),
                    p + Vector2::new(half, half),
                    p + Vector2::new(-half, half),
                ],
                aabb_min: p - Vector2::new(half, half),
                aabb_max: p + Vector2::new(half, half),
            },
            building.zone_profile_runtime_id,
        )
        .raw();
    let id = core.allocator.buildings.len();
    assert!(core.zoning.occupy_parcel(building.parcel_id, id));
    for _ in 0..building.occupancy {
        core.agents.spawn_housed_agent(id, p.x, p.y);
    }
    core.allocator.buildings.push(building);
}
