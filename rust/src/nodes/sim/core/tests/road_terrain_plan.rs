// SPDX-License-Identifier: GPL-2.0-only

//! Actual preview/commit terrain tile/patch parity and stale-dependency regression tests.

use super::*;
use crate::nodes::sim::core::{RefinedTerrainPatchBuildInput, RoadEditPlan};
use std::sync::Arc;

fn stage(core: &mut SimCore, points: Vec<Vector3>) -> Arc<RoadEditPlan> {
    stage_with_lanes(core, points, 1, 1)
}

fn stage_with_lanes(
    core: &mut SimCore,
    points: Vec<Vector3>,
    fwd_lanes: i32,
    bkw_lanes: i32,
) -> Arc<RoadEditPlan> {
    let plan = prepare_with_lanes(core, points.clone(), fwd_lanes, bkw_lanes);
    adopt(core, points, fwd_lanes, bkw_lanes, &plan);
    plan
}

fn prepare_with_lanes(
    core: &mut SimCore,
    points: Vec<Vector3>,
    fwd_lanes: i32,
    bkw_lanes: i32,
) -> Arc<RoadEditPlan> {
    core.precompute_road_mesh_data();
    core.allocator
        .prepare_building_site_query_index(core.config.zone_cell_m);
    let (context, query) = road_tool_snapshots_from_core(core).unwrap();
    let preview = crate::nodes::sim::core::road_preview::compile_road_preview_with_sites(
        &context,
        RoadPreviewRequest {
            request_id: 1,
            surface_generation: query.surface_generation,
            points: points.clone(),
            fwd_lanes,
            bkw_lanes,
            snap_to_existing_roads: true,
        },
        core,
    );
    preview
        .edit_plan()
        .unwrap_or_else(|| panic!("preview invalid: {:?}", preview.validation))
}

fn adopt(
    core: &mut SimCore,
    points: Vec<Vector3>,
    fwd_lanes: i32,
    bkw_lanes: i32,
    plan: &RoadEditPlan,
) {
    core.transit_network.begin_road_edit();
    core.transit_network.bulk_load = true;
    let mut result = core.add_road_internal_with_snap_and_validation(
        points,
        fwd_lanes,
        bkw_lanes,
        true,
        Some(plan),
    );
    assert!(result.committed && result.finalized_geometry.is_some());
    if let Some(reuse) = result.preview_topology_reuse.take() {
        core.transit_network
            .road_surface
            .enqueue_preview_topology_reuse(reuse);
    }
    core.transit_network.bulk_load = false;
    core.rebuild_network_surface_terrain_internal_with_entrance_rebuild(false);
}

fn inputs(core: &mut SimCore) -> Vec<RefinedTerrainPatchBuildInput> {
    core.collect_refined_terrain_patch_build_inputs(2.0)
}

pub(super) fn commit_ready(core: &mut SimCore, points: Vec<Vector3>) -> Arc<RoadEditPlan> {
    commit_ready_with_lanes(core, points, 1, 1)
}

fn commit_ready_with_lanes(
    core: &mut SimCore,
    points: Vec<Vector3>,
    fwd_lanes: i32,
    bkw_lanes: i32,
) -> Arc<RoadEditPlan> {
    let plan = prepare_with_lanes(core, points.clone(), fwd_lanes, bkw_lanes);
    assert_eq!(plan.status(core), "ready");
    core.transit_network.begin_road_edit();
    core.transit_network.bulk_load = true;
    let mut added = core.add_road_internal_with_snap_and_validation(
        points,
        fwd_lanes,
        bkw_lanes,
        true,
        Some(&plan),
    );
    assert!(added.committed);
    assert!(added.finalized_geometry.is_some());
    core.transit_network.bulk_load = false;
    core.transit_network
        .road_surface
        .enqueue_preview_topology_reuse(added.preview_topology_reuse.take().unwrap());
    assert!(
        core.validate_staged_road_render_with_plan(plan.terrain()),
        "{}",
        core.last_road_timing
    );
    assert!(!core.transit_network.road_edit_is_staged());
    let entries: Vec<_> = plan
        .terrain()
        .unwrap()
        .preview_patches()
        .unwrap()
        .iter()
        .filter_map(|patch| core.refined_terrain_patch_cache.get(&patch.key).cloned())
        .collect();
    assert!(!entries.is_empty());
    assert!(plan.terrain().unwrap().entries_match(&entries));
    assert!(plan.terrain().unwrap().reused_patch_buffer_count(&entries) > 0);
    plan
}

#[test]
fn ready_plan_atomically_adopts_crossings_extensions_and_close_double_t() {
    let mut core = test_core();
    core.heightmap = TerrainSystem::with_chunking(257, 257, 1.0, 129, 0.0);
    core.transit_network = TransitNetwork::new_with_surface_chunk_span(64.0);
    for ends in [
        ((-60.0, 0.0), (60.0, 0.0)),
        ((0.0, -48.0), (0.0, 48.0)),
        ((60.0, 0.0), (100.0, 0.0)),
        ((32.0, -48.0), (32.0, 0.0)),
        ((48.0, 48.0), (48.0, 0.0)),
    ] {
        let points = [ends.0, ends.1]
            .map(|(x, z)| Vector3::new(x, 0.0, z))
            .to_vec();
        let plan = commit_ready(&mut core, points.clone());
        assert_ne!(plan.status(&core), "ready");
        let count = core.region_graph.edge_count();
        assert!(
            !core
                .add_road_internal_with_snap_and_validation(points, 1, 1, true, Some(&plan))
                .committed
        );
        assert_eq!(core.region_graph.edge_count(), count);
    }
}

#[test]
fn topology_attachment_repairs_preserve_planned_sites() {
    let mut core = test_core();
    core.heightmap = TerrainSystem::with_chunking(257, 257, 1.0, 129, 0.0);
    core.transit_network = TransitNetwork::new_with_surface_chunk_span(64.0);
    commit_ready(
        &mut core,
        vec![Vector3::new(-60.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)],
    );
    let asset = register_test_asset(
        &mut core.allocator,
        "post_topology_site",
        ZoneType::Residential,
    );
    add_test_complete_building(&mut core, asset, ZoneType::Residential);
    let plan = commit_ready(
        &mut core,
        vec![Vector3::new(32.0, 0.0, -48.0), Vector3::new(32.0, 0.0, 0.0)],
    );
    assert!(plan.terrain().unwrap().sites_match(&core));
    core.rebuild_building_entrances_internal();
    core.allocator
        .rebuild_building_site_client(0, core.config.zone_cell_m);
    core.allocator
        .prepare_building_site_query_index(core.config.zone_cell_m);
    assert!(plan.terrain().unwrap().sites_match(&core));
    assert!(plan.terrain().unwrap().adopted_dependencies_match(&core));
}

#[test]
fn rejected_adoption_restores_exact_visual_samples_and_graph() {
    let mut core = test_core();
    core.heightmap = TerrainSystem::with_chunking(257, 257, 1.0, 129, 0.0);
    core.transit_network = TransitNetwork::new_with_surface_chunk_span(64.0);
    core.heightmap
        .set_visual_heights_at_grid_unmarked(&[(128, 128, 0.1), (0, 0, 0.2)], |p| *p);
    let before = core.heightmap.clone_visual_dense();
    let points = vec![Vector3::new(-60.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)];
    let plan = stage(&mut core, points);
    // Inject a post-topology dependency failure before publication.
    let asset = register_test_asset(&mut core.allocator, "injected_site", ZoneType::Residential);
    add_test_complete_building(&mut core, asset, ZoneType::Residential);
    assert!(!core.validate_staged_road_render_with_plan(plan.terrain()));
    assert_eq!(core.region_graph.edge_count(), 0);
    assert_eq!(core.heightmap.clone_visual_dense(), before);
    assert!(core.undo_stack.is_empty());
    assert!(!core.transit_network.road_edit_is_staged());
}

#[test]
fn accepted_road_undo_restores_visual_overrides() {
    let mut core = test_core();
    core.benchmark_mode = false;
    core.heightmap = TerrainSystem::with_chunking(257, 257, 1.0, 129, 0.0);
    core.transit_network = TransitNetwork::new_with_surface_chunk_span(64.0);
    core.heightmap
        .set_visual_heights_at_grid_unmarked(&[(128, 128, 0.1)], |p| *p);
    let before = core.heightmap.clone_visual_dense();
    commit_ready(
        &mut core,
        vec![Vector3::new(-60.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)],
    );
    assert_ne!(core.heightmap.clone_visual_dense(), before);
    assert!(core.undo_action_internal());
    assert_eq!(core.heightmap.clone_visual_dense(), before);
}

#[test]
fn complete_readiness_rechecks_water_inputs_and_pending_dependencies() {
    let mut core = test_core();
    core.heightmap = TerrainSystem::with_chunking(257, 257, 1.0, 129, 0.0);
    core.watermap = WaterSystem::new(257, 257);
    core.transit_network = TransitNetwork::new_with_surface_chunk_span(64.0);
    let points = vec![Vector3::new(-60.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)];
    let plan = prepare_with_lanes(&mut core, points.clone(), 1, 1);
    assert_eq!(plan.status(&core), "ready");
    let mut wrong = points.clone();
    wrong[0].y += 0.01;
    assert!(
        !core
            .add_road_internal_with_snap_and_validation(wrong, 1, 1, true, Some(&plan))
            .committed
    );
    assert_eq!(
        plan.status(&core),
        "ready",
        "an input mismatch must not consume the plan"
    );
    core.watermap
        .replace_baseline_depth_from_dense(&vec![2.0; 257 * 257])
        .unwrap();
    assert_eq!(plan.status(&core), "invalid");
    core.watermap
        .replace_baseline_depth_from_dense(&vec![0.0; 257 * 257])
        .unwrap();
    assert_eq!(plan.status(&core), "ready");
    core.terrain_stroke_active = true;
    assert_eq!(plan.status(&core), "pending");
    core.terrain_stroke_active = false;
    core.allocator.dirty_index = true;
    assert_eq!(
        plan.status(&core),
        "ready",
        "an empty building store needs no index rebuild"
    );
    assert!(core.allocator.dirty_index);
    core.allocator
        .prepare_building_site_query_index(core.config.zone_cell_m);
    core.heightmap.set_height(0, 0, 0.25);
    assert_eq!(plan.status(&core), "stale");
    core.precompute_road_mesh_data();
    let rebuilt = RoadEditPlan::compile(
        &core,
        RoadPreviewRequest {
            request_id: 2,
            surface_generation: core.road_tool_surface_generation,
            points,
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
        },
    );
    assert_eq!(rebuilt.status(&core), "ready");
    assert_eq!(core.region_graph.edge_count(), 0);
    assert!(core.undo_stack.is_empty());
}

#[test]
fn new_parcel_rejects_ready_plan_without_consuming_it() {
    let mut core = test_core();
    core.heightmap = TerrainSystem::with_chunking(257, 257, 1.0, 129, 0.0);
    core.transit_network = TransitNetwork::new_with_surface_chunk_span(64.0);
    commit_ready(
        &mut core,
        vec![Vector3::new(-60.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)],
    );
    let points = vec![Vector3::new(0.0, 0.0, -48.0), Vector3::ZERO];
    let plan = prepare_with_lanes(&mut core, points.clone(), 1, 1);
    assert_eq!(plan.status(&core), "ready");
    let profile = core
        .zoning
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Residential)
        .unwrap();
    let parcel = core
        .zoning
        .place_or_rezone_default_parcel_at(0.0, -20.0, profile, &core.region_graph)
        .unwrap();
    assert_eq!(plan.status(&core), "invalid");
    let before = core.region_graph.edge_count();
    assert!(
        !core
            .add_road_internal_with_snap_and_validation(points, 1, 1, true, Some(&plan))
            .committed
    );
    assert_eq!(core.region_graph.edge_count(), before);
    core.zoning
        .remove_parcels_by_raw_ids(&std::collections::HashSet::from([parcel.raw()]));
    assert_eq!(plan.status(&core), "ready");
}

#[test]
fn structural_road_plans_adopt_with_and_without_terrain_cutouts() {
    for height in [-3.0, 10.0] {
        let mut core = test_core();
        core.heightmap = TerrainSystem::with_chunking(257, 257, 1.0, 129, 0.0);
        core.transit_network = TransitNetwork::new_with_surface_chunk_span(64.0);
        // The tunnel needs a visible portal; a wholly buried span has no display product
        // and is rejected by the existing surface compiler. The bridge has no cutouts.
        let points = if height < 0.0 {
            vec![
                Vector3::new(-60.0, 0.0, 0.0),
                Vector3::new(-40.0, height, 0.0),
                Vector3::new(60.0, height, 0.0),
            ]
        } else {
            vec![
                Vector3::new(-40.0, height, 0.0),
                Vector3::new(40.0, height, 0.0),
            ]
        };
        let plan = prepare_with_lanes(&mut core, points.clone(), 1, 1);
        assert_eq!(plan.status(&core), "ready");
        core.transit_network.begin_road_edit();
        core.transit_network.bulk_load = true;
        let mut added =
            core.add_road_internal_with_snap_and_validation(points, 1, 1, true, Some(&plan));
        assert!(added.committed);
        core.transit_network.bulk_load = false;
        core.transit_network
            .road_surface
            .enqueue_preview_topology_reuse(added.preview_topology_reuse.take().unwrap());
        assert!(
            core.validate_staged_road_render_with_plan(plan.terrain()),
            "height {height}: {}",
            core.last_road_timing
        );
    }
}

#[test]
fn kuopio_saved_reference_repairs_rejected_junction_before_publication() {
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../benchmarks/fixtures/kuopio-terrain/kuopio-terrain-map.sqlite"
    );
    let mut core = test_core();
    core.load_game_internal(fixture).unwrap();
    // The rejected junction is a local repair, not permission to regrade the city.
    let connection =
        rusqlite::Connection::open_with_flags(fixture, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let mut statement = connection.prepare(
        "SELECT edge_id, point_index, x, y, z, physical FROM network_edge_geometry ORDER BY edge_id, physical, point_index",
    ).unwrap();
    let mut rows = statement.query([]).unwrap();
    let repaired_edges = core.region_graph.node_adjacency(34);
    let mut repaired_points = 0;
    while let Some(row) = rows.next().unwrap() {
        let edge_id: usize = row.get(0).unwrap();
        let point_index: usize = row.get(1).unwrap();
        let stored = Vector3::new(
            row.get(2).unwrap(),
            row.get(3).unwrap(),
            row.get(4).unwrap(),
        );
        let physical: bool = row.get(5).unwrap();
        let edge = core.region_graph.edge(edge_id);
        if repaired_edges.contains(&edge_id) {
            repaired_points +=
                usize::from(physical && edge.physical_geometry.get(point_index) != Some(&stored));
        } else {
            let restored = if physical {
                &edge.physical_geometry
            } else {
                &edge.geometry
            };
            assert_eq!(
                restored[point_index], stored,
                "unrelated saved geometry changed: edge {edge_id}"
            );
        }
    }
    assert!(
        repaired_points > 0,
        "fixture no longer exercises rejected-junction repair"
    );
    core.precompute_road_mesh_data();
    assert!(
        core.transit_network
            .road_surface
            .published_generation_matches_source(),
        "{:?}\n{}",
        core.transit_network
            .road_surface
            .last_compile_failure_label(),
        core.transit_network
            .road_surface
            .build_edge_geometry_debug_dump(
                &core.region_graph,
                &core.heightmap,
                core.region_graph.node_adjacency(34)
            )
    );
    let patches = SimCore::build_refined_terrain_patch_cache_entries(inputs(&mut core));
    assert_eq!(patches.len(), 27);
    for patch in patches {
        assert!(
            crate::nodes::simulation_node::SimulationNode::cached_refined_cdt_failure_label(&patch)
                .is_none(),
            "saved terrain {:?}: {:?}",
            patch.key,
            patch
                .mesh_buffers
                .as_ref()
                .map(|b| b.omitted_pathological_terrain_faces)
        );
    }
}

#[test]
fn kuopio_recorded_profiles_and_terrain_preserve_quality_limits() {
    let manifest: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../benchmarks/fixtures/kuopio-terrain/placements.json",
    )))
    .unwrap();
    let mut failures = Vec::new();
    let mut profile_count = 0;
    for case in manifest["cases"].as_array().unwrap() {
        let case_id = case["case_id"].as_str().unwrap();
        let mut core = test_core();
        core.load_world_definition_internal(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../godot/bootstrap/worlds/kuopio_324km2_10m.sqlite"
        ))
        .unwrap();
        for stroke in case["segments"].as_array().unwrap() {
            let points = ["start_xz", "end_xz"]
                .map(|key| {
                    let x = stroke[key][0].as_f64().unwrap() as f32;
                    let z = stroke[key][1].as_f64().unwrap() as f32;
                    Vector3::new(
                        x,
                        core.heightmap.sample_visual_height_world(x, z)
                            * crate::config::HEIGHT_SCALE,
                        z,
                    )
                })
                .to_vec();
            let plan = commit_ready_with_lanes(
                &mut core,
                points,
                stroke["fwd_lanes"].as_i64().unwrap() as i32,
                stroke["bkw_lanes"].as_i64().unwrap() as i32,
            );
            let cold = SimCore::build_refined_terrain_patch_cache_entries(inputs(&mut core));
            let mut tile_heights = std::collections::BTreeMap::new();
            for patch in &cold {
                if let Some(buffers) = &patch.mesh_buffers {
                    let mut exported_heights = HashMap::new();
                    for point in &buffers.terrain_vertices {
                        if let Some(previous) =
                            exported_heights.insert((point.x.to_bits(), point.z.to_bits()), point.y)
                        {
                            assert!(
                                (previous - point.y).abs() < 0.001,
                                "{case_id} {} patch {:?}: exported seam at {point:?}, previous height {previous}",
                                stroke["attempt"],
                                patch.key
                            );
                        }
                    }
                    assert_eq!(
                        buffers.omitted_pathological_terrain_faces, 0,
                        "{case_id} attempt {} patch {:?}: terrain export removed faces",
                        stroke["attempt"], patch.key
                    );
                }
                for window in &patch.windows {
                    let mesh = window.mesh_result.as_ref().unwrap();
                    let bounds = window.cdt_patch;
                    for vertex in &mesh.vertices {
                        assert!(
                            vertex.x >= bounds.min_x
                                && vertex.x <= bounds.max_x
                                && vertex.z >= bounds.min_z
                                && vertex.z <= bounds.max_z,
                            "{case_id} {}: vertex outside {:?}: {vertex:?}",
                            stroke["attempt"],
                            window.key
                        );
                        if vertex.x == bounds.min_x
                            || vertex.x == bounds.max_x
                            || vertex.z == bounds.min_z
                            || vertex.z == bounds.max_z
                        {
                            let key = (
                                (vertex.x * 1000.0).round() as i64,
                                (vertex.z * 1000.0).round() as i64,
                            );
                            if let Some((previous, previous_window)) =
                                tile_heights.insert(key, (vertex.height_m, window.key))
                            {
                                assert!(
                                    (previous - vertex.height_m).abs() < 0.001,
                                    "{case_id} {}: tile seam {key:?} height {previous} vs {} in {previous_window:?} and {:?}",
                                    stroke["attempt"],
                                    vertex.height_m,
                                    window.key
                                );
                            }
                        }
                    }
                    assert_eq!(
                        mesh.stats.invalid_constraint_edges, 0,
                        "{case_id} attempt {} window {:?}: {:?}",
                        stroke["attempt"], window.key, mesh.invalid_constraint_samples
                    );
                    assert_eq!(
                        mesh.stats.spade_missing_road_constraint_edges, 0,
                        "{case_id} attempt {} window {:?}: {:?}",
                        stroke["attempt"], window.key, mesh.unpreserved_road_constraint_samples
                    );
                }
            }
            if let Some(reason) = plan.terrain().unwrap().failure_reason() {
                failures.push(format!("{case_id} attempt {}: {reason}", stroke["attempt"]));
            }
            // Same fixed quality gates as the rendered replay, across every live edge
            // after each edit. Exercise the actual plan adoption, not just preparation.
            for (edge_idx, edge) in core.region_graph.edges().iter().enumerate() {
                if edge.deleted {
                    continue;
                }
                profile_count += 1;
                let mut offset = 0.0_f32;
                let mut grade = 0.0_f32;
                let mut pitch_change = 0.0_f32;
                let mut last_pitch: Option<f32> = None;
                for point in &edge.physical_geometry {
                    let source = core.heightmap.sample_height_world(point.x, point.z)
                        * crate::config::HEIGHT_SCALE;
                    offset = offset.max((point.y - source).abs());
                }
                for span in edge.physical_geometry.windows(2) {
                    let run = (span[1].x - span[0].x).hypot(span[1].z - span[0].z);
                    let dy = span[1].y - span[0].y;
                    if run == 0.0 {
                        assert_eq!(dy, 0.0, "{case_id}: vertical road span {span:?}");
                        continue;
                    }
                    grade = grade.max(dy.abs() / run);
                    let pitch = dy.atan2(run).to_degrees();
                    if let Some(previous) = last_pitch {
                        pitch_change = pitch_change.max((pitch - previous).abs());
                    }
                    last_pitch = Some(pitch);
                }
                if offset > 5.0 || grade > 1.0 || pitch_change > 30.0 {
                    failures.push(format!(
                        "{case_id} attempt {} edge {edge_idx}: offset {offset}, grade {grade}, pitch change {pitch_change}",
                        stroke["attempt"]
                    ));
                }
            }
            let keys = plan
                .terrain()
                .unwrap()
                .preview_patches()
                .unwrap()
                .iter()
                .map(|patch| (patch.key.patch_x, patch.key.patch_z))
                .collect::<Vec<_>>();
            let adopted =
                crate::nodes::simulation_node::SimulationNode::validate_staged_road_terrain(
                    &mut core,
                    &keys,
                    plan.terrain(),
                )
                .unwrap_or_else(|reason| panic!("{case_id}: {reason}"));
            assert!(plan.terrain().unwrap().entries_match(&adopted));
            core.insert_refined_terrain_patch_cache_entries(adopted);
            core.accept_staged_road_edit();
        }
    }
    assert_eq!(
        profile_count, 158,
        "must exercise every recorded checkpoint"
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn planned_patch_ownership_excludes_margin_only_roads_and_matches_commit() {
    let mut core = test_core();
    core.heightmap = TerrainSystem::with_chunking(257, 257, 1.0, 129, 0.0);
    core.transit_network = TransitNetwork::new_with_surface_chunk_span(64.0);
    // The northern patches see the first road through the padded query, but its actual
    // grading stops south of them. The branch then changes their ownership for real.
    for (stroke, points) in [
        vec![
            Vector3::new(-48.0, 0.0, 20.0),
            Vector3::new(48.0, 0.0, 20.0),
        ],
        vec![Vector3::new(0.0, 0.0, 20.0), Vector3::new(0.0, 0.0, -48.0)],
    ]
    .into_iter()
    .enumerate()
    {
        let plan = stage(&mut core, points);
        let terrain = plan.terrain().unwrap();
        assert_eq!(terrain.failure_reason(), None);
        let patches = terrain.preview_patches().unwrap();
        let northern = patches
            .iter()
            .find(|patch| (patch.key.patch_x, patch.key.patch_z) == (0, 0))
            .unwrap();
        assert_eq!(northern.requires_engineered_refinement, stroke > 0);
        let cold = SimCore::build_refined_terrain_patch_cache_entries(inputs(&mut core));
        for patch in patches {
            let owned = core
                .terrain_patch_requires_engineered_refinement(patch.key.patch_x, patch.key.patch_z);
            assert_eq!(
                patch.requires_engineered_refinement, owned,
                "{:?}",
                patch.key
            );
            if owned {
                let actual = cold.iter().find(|actual| actual.key == patch.key).unwrap();
                assert_eq!(patch.requires_road_clipping, actual.requires_road_clipping);
                assert_eq!(patch.clip_query_margin_m, actual.clip_query_margin_m);
                assert_eq!(patch.mesh_buffers, actual.mesh_buffers);
            } else {
                assert!(!patch.requires_road_clipping);
                assert_eq!(patch.input_road_loops, 0);
                assert!(patch.windows.is_empty());
            }
        }
        core.accept_staged_road_edit();
    }
}

#[test]
fn planned_cdt_tiles_match_fresh_commit_meshes_and_reject_changed_inputs() {
    let mut core = test_core();
    // 128 m patches span multiple fixed 64 m CDT tiles: exercise real joined tile seams.
    core.heightmap = TerrainSystem::with_chunking(257, 257, 1.0, 129, 0.0);
    core.transit_network = TransitNetwork::new_with_surface_chunk_span(64.0);
    for points in [
        vec![Vector3::new(-60.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)],
        vec![Vector3::new(0.0, 0.0, -48.0), Vector3::new(0.0, 0.0, 48.0)],
    ] {
        let plan = stage(&mut core, points);
        let terrain_plan = plan.terrain().expect("preview must retain CDT products");
        let published = terrain_plan
            .preview_patches()
            .expect("complete ground-road patches must be publishable");
        assert!(published.windows(2).all(|pair| {
            (pair[0].key.patch_x, pair[0].key.patch_z) < (pair[1].key.patch_x, pair[1].key.patch_z)
        }));
        let cold = SimCore::build_refined_terrain_patch_cache_entries(inputs(&mut core));
        for planned in &published {
            if let Some(actual) = cold.iter().find(|patch| patch.key == planned.key) {
                assert_eq!(
                    planned.mesh_buffers, actual.mesh_buffers,
                    "published terrain must be the unmodified production result"
                );
            }
        }
        let mut warm_inputs = inputs(&mut core);
        let count = terrain_plan
            .reuse_matching_windows(core.heightmap.source_generation(), &mut warm_inputs);
        assert!(
            count > 0,
            "must reuse real CDT tiles, not only empty structural stamps"
        );
        let reused = warm_inputs
            .iter()
            .flat_map(|patch| patch.windows.iter())
            .filter_map(|window| window.previous.clone())
            .collect::<Vec<_>>();
        assert!(
            reused
                .iter()
                .any(|window| !window.mesh_result.as_ref().unwrap().triangles.is_empty())
        );
        let warm = SimCore::build_refined_terrain_patch_cache_entries(warm_inputs);
        assert!(
            terrain_plan.reused_patch_buffer_count(&warm) > 0,
            "must adopt joined terrain buffers, not merely reassemble the planned tiles"
        );
        assert!(warm.iter().any(|patch| patch.windows.len() > 1
            && terrain_plan.reused_patch_buffer_count(&[Arc::clone(patch)]) == 1));
        assert_eq!(cold.len(), warm.len());
        for actual in &warm {
            let expected = cold.iter().find(|patch| patch.key == actual.key).unwrap();
            assert_eq!(actual.mesh_buffers, expected.mesh_buffers);
            assert_eq!(actual.surface_generation, expected.surface_generation);
            assert_eq!(actual.clip_source_count, expected.clip_source_count);
            assert_eq!(
                actual.road_clip_source_count,
                expected.road_clip_source_count
            );
            assert_eq!(actual.road_clip_loop_count, expected.road_clip_loop_count);
            assert_eq!(actual.site_clip_loop_count, expected.site_clip_loop_count);
            assert_eq!(actual.clip_error_label, expected.clip_error_label);
            assert_eq!(actual.windows.len(), expected.windows.len());
            for (a, b) in actual.windows.iter().zip(&expected.windows) {
                assert_eq!(a.key, b.key);
                assert_eq!(a.mesh_result, b.mesh_result);
                assert_eq!(a.mesh_buffers, b.mesh_buffers);
            }
        }
        assert!(reused.iter().all(|previous| warm.iter().any(|patch| {
            patch
                .windows
                .iter()
                .any(|window| Arc::ptr_eq(previous, window))
        })));

        let mut changed = inputs(&mut core);
        for patch in &mut changed {
            for window in &mut patch.windows {
                window.cdt_input.patch.corner_heights_m[0] += 0.01;
            }
        }
        assert_eq!(
            terrain_plan.reuse_matching_windows(core.heightmap.source_generation(), &mut changed),
            0,
            "an unchanged fingerprint alone must not accept changed samples or constraints"
        );
        assert_eq!(
            terrain_plan.reused_patch_buffer_count(
                &SimCore::build_refined_terrain_patch_cache_entries(changed)
            ),
            0
        );
        let mut changed_frame = inputs(&mut core);
        for patch in &mut changed_frame {
            patch.patch.world_origin_x += 1.0;
        }
        assert_eq!(
            terrain_plan
                .reuse_matching_windows(core.heightmap.source_generation(), &mut changed_frame),
            0
        );
        assert_eq!(
            terrain_plan.reused_patch_buffer_count(
                &SimCore::build_refined_terrain_patch_cache_entries(changed_frame)
            ),
            0
        );
        let mut changed_sites = inputs(&mut core);
        for patch in &mut changed_sites {
            for window in &mut patch.windows {
                // A newly discovered site changes the contributor manifest even when a caller
                // accidentally retains the road-only tile key. It must not adopt that tile.
                window.site_clip_fingerprints.push(999);
            }
        }
        assert_eq!(
            terrain_plan
                .reuse_matching_windows(core.heightmap.source_generation(), &mut changed_sites),
            0
        );
        assert_eq!(
            terrain_plan.reused_patch_buffer_count(
                &SimCore::build_refined_terrain_patch_cache_entries(changed_sites)
            ),
            0
        );

        let mut incomplete = inputs(&mut core);
        for patch in &mut incomplete {
            patch.windows.pop();
        }
        terrain_plan.reuse_matching_windows(core.heightmap.source_generation(), &mut incomplete);
        assert_eq!(
            terrain_plan.reused_patch_buffer_count(
                &SimCore::build_refined_terrain_patch_cache_entries(incomplete)
            ),
            0,
            "a subset of matching tiles cannot authorize complete patch buffer adoption"
        );

        // Even exact tile matches cannot replace the fresh patch's completeness/ownership checks.
        for mutate in [
            (|patch: &mut RefinedTerrainPatchBuildInput| {
                patch.expected_site_clip_fingerprints.push(999);
            }) as fn(&mut RefinedTerrainPatchBuildInput),
            |patch| patch.expected_road_clip_fingerprints.push(999),
            |patch| patch.clip_error_label = Some("planned_test_clip_failure"),
            |patch| patch.road_clip_source_count = 0,
        ] {
            let mut changed = inputs(&mut core);
            for patch in &mut changed {
                mutate(patch);
            }
            terrain_plan.reuse_matching_windows(core.heightmap.source_generation(), &mut changed);
            let failed = SimCore::build_refined_terrain_patch_cache_entries(changed);
            assert_eq!(terrain_plan.reused_patch_buffer_count(&failed), 0);
            assert!(failed.iter().all(|patch| {
                crate::nodes::simulation_node::SimulationNode::cached_refined_cdt_failure_label(
                    patch,
                )
                .is_some()
            }));
        }
        assert_eq!(
            terrain_plan.reuse_matching_windows(
                core.heightmap.source_generation() + 1,
                &mut inputs(&mut core)
            ),
            0
        );
        core.accept_staged_road_edit();
        core.insert_refined_terrain_patch_cache_entries(warm);
    }
}

#[test]
fn planned_patch_reuse_keeps_new_building_site_geometry_authoritative() {
    let mut core = test_core();
    core.heightmap = TerrainSystem::with_chunking(257, 257, 1.0, 129, 0.0);
    core.transit_network = TransitNetwork::new_with_surface_chunk_span(64.0);
    let plan = stage(
        &mut core,
        vec![Vector3::new(-60.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)],
    );
    let terrain_plan = plan.terrain().unwrap();
    assert!(terrain_plan.sites_match(&mut core));
    // Derive an actual site through the allocator after the road-only preview was prepared.
    let asset_id = register_test_asset(&mut core.allocator, "plan_site", ZoneType::Residential);
    add_test_complete_building(&mut core, asset_id, ZoneType::Residential);
    core.allocator
        .prepare_building_site_query_index(core.config.zone_cell_m);
    assert!(!terrain_plan.sites_match(&mut core));
    assert_eq!(terrain_plan.status(&mut core), "stale");
    let cold = SimCore::build_refined_terrain_patch_cache_entries(inputs(&mut core));
    let mut warm_inputs = inputs(&mut core);
    assert!(
        warm_inputs
            .iter()
            .any(|patch| patch.site_clip_loop_count > 0)
    );
    terrain_plan.reuse_matching_windows(core.heightmap.source_generation(), &mut warm_inputs);
    let warm = SimCore::build_refined_terrain_patch_cache_entries(warm_inputs);
    assert_eq!(warm.len(), cold.len());
    assert!(warm.iter().any(|patch| {
        patch
            .windows
            .iter()
            .any(|window| !window.site_clip_fingerprints.is_empty())
            && patch
                .mesh_buffers
                .as_ref()
                .is_some_and(|buffers| !buffers.terrain_indices.is_empty())
    }));
    for (actual, expected) in warm.iter().zip(&cold) {
        assert_eq!(actual.key, expected.key);
        assert_eq!(actual.mesh_buffers, expected.mesh_buffers);
        assert_eq!(actual.site_clip_loop_count, expected.site_clip_loop_count);
        assert_eq!(actual.clip_error_label, expected.clip_error_label);
        if actual.site_clip_loop_count > 0 {
            assert!(
                crate::nodes::simulation_node::SimulationNode::cached_refined_cdt_failure_label(
                    actual
                )
                .is_none(),
                "site patch {:?}: {:?}; building=({}, {})",
                actual.key,
                crate::nodes::simulation_node::SimulationNode::cached_refined_cdt_failure_label(
                    actual
                ),
                core.allocator.buildings[0].center_x,
                core.allocator.buildings[0].center_y
            );
            assert!(
                actual
                    .mesh_buffers
                    .as_ref()
                    .is_some_and(|buffers| buffers.variant_payload_valid)
            );
        }
        // A padded query can discover the new site without it influencing this patch. Only
        // actual site-contributing windows must miss the road-only product; neighbors may reuse.
        if actual
            .windows
            .iter()
            .any(|window| !window.site_clip_fingerprints.is_empty())
        {
            assert_eq!(
                terrain_plan.reused_patch_buffer_count(&[Arc::clone(actual)]),
                0
            );
        }
    }
}

#[test]
fn planned_patches_include_existing_site_grading_and_track_readiness() {
    let mut core = test_core();
    core.heightmap = TerrainSystem::with_chunking(257, 257, 1.0, 129, 0.0);
    core.transit_network = TransitNetwork::new_with_surface_chunk_span(64.0);
    stage(
        &mut core,
        vec![Vector3::new(-60.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)],
    );
    core.accept_staged_road_edit();
    let asset = register_test_asset(
        &mut core.allocator,
        "existing_plan_site",
        ZoneType::Residential,
    );
    add_test_complete_building(&mut core, asset, ZoneType::Residential);
    // A new junction changes the road contribution near a pre-existing site. Planning must use
    // the finalized replacement surfaces, while preserving the site's own support/footprint.
    let points = vec![Vector3::new(32.0, 0.0, -48.0), Vector3::new(32.0, 0.0, 0.0)];
    let plan = prepare_with_lanes(&mut core, points.clone(), 1, 1);
    let terrain = plan.terrain().unwrap();
    assert_eq!(terrain.status(&mut core), "compiled");
    core.allocator.dirty_index = true;
    assert_eq!(terrain.status(&mut core), "pending");
    assert!(
        core.allocator.dirty_index,
        "checking readiness must not rebuild the city index"
    );
    core.allocator
        .prepare_building_site_query_index(core.config.zone_cell_m);
    assert_eq!(terrain.status(&mut core), "compiled");
    core.terrain_stroke_active = true;
    assert_eq!(terrain.status(&core), "pending");
    core.terrain_stroke_active = false;
    assert_eq!(terrain.status(&core), "compiled");
    adopt(&mut core, points, 1, 1, &plan);
    assert_eq!(
        terrain.status(&core),
        "stale",
        "pre-edit dependencies cannot authorize a second commit"
    );
    let cold = SimCore::build_refined_terrain_patch_cache_entries(inputs(&mut core));
    let mut warm_inputs = inputs(&mut core);
    terrain.reuse_matching_windows(core.heightmap.source_generation(), &mut warm_inputs);
    let warm = SimCore::build_refined_terrain_patch_cache_entries(warm_inputs);
    assert!(
        warm.iter().any(|patch| {
            patch
                .windows
                .iter()
                .any(|window| !window.site_clip_fingerprints.is_empty())
                && terrain.reused_patch_buffer_count(&[Arc::clone(patch)]) == 1
        }),
        "must reuse a complete patch actually influenced by the existing site"
    );
    assert_eq!(warm.len(), cold.len());
    for (actual, expected) in warm.iter().zip(&cold) {
        assert_eq!(actual.key, expected.key);
        assert_eq!(actual.mesh_buffers, expected.mesh_buffers);
        assert_eq!(actual.site_clip_loop_count, expected.site_clip_loop_count);
        assert_eq!(actual.clip_error_label, expected.clip_error_label);
    }
    core.heightmap.set_height(0, 0, 0.25);
    assert_eq!(terrain.status(&mut core), "stale");
}

#[test]
fn terrain_preview_includes_final_visual_stamp_resets() {
    let mut core = test_core();
    core.heightmap = TerrainSystem::with_chunking(257, 257, 1.0, 129, 0.0);
    core.transit_network = TransitNetwork::new_with_surface_chunk_span(64.0);
    core.heightmap
        .set_visual_heights_at_grid_unmarked(&[(128, 128, 0.1)], |sample| *sample);
    assert!(!core.heightmap.visual_patch_matches_source(0, 0));
    let plan = stage(
        &mut core,
        vec![Vector3::new(-60.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)],
    );
    assert!(
        plan.terrain().unwrap().preview_patches().is_some(),
        "post-stamp ownership must permit the paired structural preview"
    );
    let terrain = plan.terrain().unwrap();
    let cold = SimCore::build_refined_terrain_patch_cache_entries(inputs(&mut core));
    let mut warm_inputs = inputs(&mut core);
    assert!(
        terrain.reuse_matching_windows(core.heightmap.source_generation(), &mut warm_inputs) > 0
    );
    let warm = SimCore::build_refined_terrain_patch_cache_entries(warm_inputs);
    assert_eq!(
        terrain.reused_patch_buffer_count(&warm),
        cold.len(),
        "all post-reset texture and CDT inputs must match"
    );
    assert_eq!(warm.len(), cold.len());
    for (actual, expected) in warm.iter().zip(&cold) {
        assert_eq!(actual.patch, expected.patch);
        assert_eq!(actual.mesh_buffers, expected.mesh_buffers);
    }
}

#[test]
fn visual_only_edits_stale_the_prepared_terrain_candidate() {
    let mut core = test_core();
    core.heightmap = TerrainSystem::with_chunking(257, 257, 1.0, 129, 0.0);
    core.transit_network = TransitNetwork::new_with_surface_chunk_span(64.0);
    let points = vec![Vector3::new(-60.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)];
    let plan = prepare_with_lanes(&mut core, points, 1, 1);
    let terrain = plan.terrain().unwrap();
    assert_eq!(terrain.status(&core), "compiled");
    let source_generation = core.heightmap.source_generation();
    core.heightmap
        .set_visual_heights_at_grid_unmarked(&[(128, 128, 0.1)], |sample| *sample);
    assert_eq!(core.heightmap.source_generation(), source_generation);
    assert_eq!(terrain.status(&core), "stale");
}
