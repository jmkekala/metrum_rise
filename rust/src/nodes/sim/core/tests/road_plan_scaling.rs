// SPDX-License-Identifier: GPL-2.0-only

//! Isolated populated-city planning benchmark; fixture construction is never timed.

use super::*;
use crate::nodes::sim::core::RoadEditPlan;
use godot::prelude::Vector2;
use std::time::Instant;

#[test]
#[ignore = "unprofiled scaling measurement; run alone with --release --ignored --nocapture"]
fn populated_road_plan_scaling() {
    measure_populated_road_plan_scaling(false, false);
}

#[test]
#[ignore = "unprofiled paved-site scaling measurement; run alone with --release --ignored --nocapture"]
fn populated_paved_road_plan_scaling() {
    measure_populated_road_plan_scaling(true, false);
}

#[test]
#[ignore = "unprofiled cached zoning feasibility scaling; run alone with --release --ignored --nocapture"]
fn populated_zoning_feasibility_scaling() {
    measure_populated_road_plan_scaling(true, true);
}

fn measure_populated_road_plan_scaling(paved: bool, zoning_only: bool) {
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
            insert_populated_site(&mut core, building);
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
                    core.demand.runtime_catalog(),
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
