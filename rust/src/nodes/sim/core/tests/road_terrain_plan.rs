// SPDX-License-Identifier: GPL-2.0-only

//! Road edit plan readiness, synchronous commit adoption and stale-dependency regressions.

use super::*;
use crate::nodes::sim::core::road_commit::RoadCommitRequest;
use crate::nodes::sim::core::road_preview::road_tool_snapshot_from_core;
use crate::nodes::sim::core::{RefinedTerrainPatchBuildInput, RoadEditPlan};
use godot::prelude::Vector2;

#[test]
fn cold_road_commit_completes_terrain_plan_before_mutation() {
    let mut core = test_core();
    core.precompute_road_mesh_data();
    core.allocator
        .prepare_building_site_query_index(core.config.zone_cell_m);
    let edges_before = core.region_graph.edge_count();

    let outcome = core.commit_road(
        RoadCommitRequest {
            points: vec![Vector3::new(-48.0, 0.0, 0.0), Vector3::new(48.0, 0.0, 0.0)],
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
        },
        std::time::Instant::now(),
        Default::default(),
    );

    assert!(outcome.committed, "{}", outcome.rejection);
    assert_eq!(core.region_graph.edge_count(), edges_before + 1);
    assert!(!core.last_road_edit_metrics.preview_plan_reused);
    assert!(!core.transit_network.road_edit_is_staged());
}

fn prepare_with_lanes(
    core: &mut SimCore,
    points: Vec<Vector3>,
    fwd_lanes: i32,
    bkw_lanes: i32,
) -> RoadEditPlan {
    core.precompute_road_mesh_data();
    core.allocator
        .prepare_building_site_query_index(core.config.zone_cell_m);
    let surface_generation = core.road_tool_surface_generation;
    RoadEditPlan::compile(
        core,
        RoadPreviewRequest {
            include_terrain: false,
            request_id: 1,
            surface_generation,
            points,
            fwd_lanes,
            bkw_lanes,
            snap_to_existing_roads: true,
        },
    )
}

fn inputs(core: &mut SimCore) -> Vec<RefinedTerrainPatchBuildInput> {
    core.collect_refined_terrain_patch_build_inputs(2.0)
}

pub(super) fn commit_ready(core: &mut SimCore, points: Vec<Vector3>) -> RoadEditPlan {
    commit_ready_with_lanes(core, points, 1, 1)
}

fn commit_ready_with_lanes(
    core: &mut SimCore,
    points: Vec<Vector3>,
    fwd_lanes: i32,
    bkw_lanes: i32,
) -> RoadEditPlan {
    let plan = prepare_with_lanes(core, points.clone(), fwd_lanes, bkw_lanes);
    assert_eq!(plan.status(core), "ready", "{:?}", plan.validation());
    core.transit_network.begin_road_edit();
    core.transit_network.bulk_load = true;
    let added = core.add_road_internal_with_snap_and_validation(
        points,
        fwd_lanes,
        bkw_lanes,
        true,
        Some(&plan),
    );
    assert!(added.committed, "{}", core.last_road_timing);
    assert!(added.finalized_geometry.is_some());
    core.transit_network.bulk_load = false;
    assert!(
        core.validate_staged_road_render(),
        "{}",
        core.last_road_timing
    );
    assert!(!core.transit_network.road_edit_is_staged());
    plan
}

#[test]
fn kuopio_third_road_preserves_existing_junction_boundary_owners() {
    for (trunk_end_x, branch_end) in [
        (2_889.749_8, (3020.0, -8_622.83)),
        (2_889.749_5, (3020.73, -8_622.673)),
    ] {
        let mut core = test_core();
        core.load_world_definition_internal(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../godot/bootstrap/worlds/kuopio_324km2_10m.sqlite"
            ),
            crate::simulation::vegetation::VegetationConfig::default(),
        )
        .unwrap();
        // The first case reproduces road.log's exact missing outer-boundary coordinates;
        // the second exposes the same short-edge deletion in the third road's new junction.
        for (start, end) in [
            ((2_816.885_7, -8_504.609), (trunk_end_x, -9000.0)),
            ((2839.961, -8661.495), branch_end),
            ((2940.0, -8640.0), (2960.0, -8870.0)),
        ] {
            let points = [start, end]
                .map(|(x, z)| {
                    Vector3::new(
                        x,
                        core.get_world_surface_height_internal(Vector2::new(x, z)),
                        z,
                    )
                })
                .to_vec();
            commit_ready(&mut core, points);
        }
        assert_eq!(core.region_graph.edge_count(), 5);
    }
}

#[test]
fn ready_plan_atomically_adopts_crossings_extensions_and_close_double_t() {
    let mut core = test_core();
    core.heightmap = TerrainSystem::with_chunking(257, 257, 1.0, 129, 0.0);
    core.transit_network = TransitNetwork::new_with_surface_chunk_span(64.0);
    // Each junction end needs half of its arm for the 3.5 m carriageway edge, the 2 m minimum
    // curb return and the 3 m crosswalk (D1, D8), so neighbouring junctions sit 18 m apart.
    for ends in [
        ((-60.0, 0.0), (60.0, 0.0)),
        ((0.0, -48.0), (0.0, 48.0)),
        ((60.0, 0.0), (100.0, 0.0)),
        ((24.0, -48.0), (24.0, 0.0)),
        ((42.0, 48.0), (42.0, 0.0)),
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
    core.heightmap.set_height(0, 0, 0.25);
    assert_eq!(plan.status(&core), "stale");
    core.precompute_road_mesh_data();
    let rebuilt = RoadEditPlan::compile(
        &core,
        RoadPreviewRequest {
            include_terrain: false,
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
fn new_field_rejects_ready_road_plan_and_releasing_land_restores_readiness() {
    let mut core = test_core();
    commit_ready(
        &mut core,
        vec![Vector3::new(-60.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)],
    );
    let points = vec![Vector3::new(0.0, 0.0, -48.0), Vector3::ZERO];
    let plan = prepare_with_lanes(&mut core, points.clone(), 1, 1);
    assert_eq!(plan.status(&core), "ready");
    let field = [
        godot::prelude::Vector2::new(-10.0, -40.0),
        godot::prelude::Vector2::new(10.0, -40.0),
        godot::prelude::Vector2::new(10.0, -15.0),
        godot::prelude::Vector2::new(-10.0, -15.0),
    ];
    core.allocator.field_clearance.set(0, &field);
    assert_eq!(plan.status(&core), "invalid");
    let before = (core.region_graph.edge_count(), core.undo_stack.len());
    assert!(
        !core
            .add_road_internal_with_snap_and_validation(points.clone(), 1, 1, true, Some(&plan))
            .committed
    );
    assert!(!core.add_road_internal(points, 1, 1).committed);
    assert_eq!(
        (core.region_graph.edge_count(), core.undo_stack.len()),
        before
    );
    core.allocator.field_clearance.clear();
    assert_eq!(plan.status(&core), "ready");
}

#[test]
fn painted_cells_reject_a_ready_road_plan_and_erase_restores_readiness() {
    use crate::simulation::zoning::cells::{CellBounds, CellSelectionShape};
    use glam::DVec2;
    let mut core = test_core();
    commit_ready(
        &mut core,
        vec![Vector3::new(-60.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)],
    );
    let points = vec![Vector3::new(0.0, 0.0, -48.0), Vector3::ZERO];
    let plan = prepare_with_lanes(&mut core, points.clone(), 1, 1);
    assert_eq!(plan.status(&core), "ready");
    core.zoning.generate_cells(
        &core.region_graph,
        CellBounds {
            min: DVec2::splat(-100.0),
            max: DVec2::splat(100.0),
        },
        |_| false,
    );
    let point = DVec2::new(0.0, -25.0);
    let selection = core.zoning.cells.select(CellSelectionShape::Cell, &[point]);
    assert!(!selection.cells.is_empty());
    core.zoning.paint_cells(&selection, 1, |_| false).unwrap();
    assert_eq!(plan.status(&core), "invalid");
    let before = core.region_graph.edge_count();
    assert!(
        !core
            .add_road_internal_with_snap_and_validation(points, 1, 1, true, Some(&plan))
            .committed
    );
    assert_eq!(core.region_graph.edge_count(), before);
    let selection = core.zoning.cells.select(CellSelectionShape::Cell, &[point]);
    core.zoning.paint_cells(&selection, 0, |_| false).unwrap();
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
        let added =
            core.add_road_internal_with_snap_and_validation(points, 1, 1, true, Some(&plan));
        assert!(added.committed);
        core.transit_network.bulk_load = false;
        assert!(
            core.validate_staged_road_render(),
            "height {height}: {}",
            core.last_road_timing
        );
    }
}

#[test]
#[ignore = "ROAD-68-E: a pre-ROAD-68-C save re-pins every junction whose profile reaches only the legacy hard zone, not just node 34; the load rebuild owns save migration"]
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
    let roadbed = &core.transit_network.roadbed;
    assert!(
        roadbed.is_current() && roadbed.failed_nodes().is_empty(),
        "{:?}",
        roadbed.failed_nodes()
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
#[ignore = "ROAD-68-D: road terrain patches"]
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
        core.load_world_definition_internal(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../godot/bootstrap/worlds/kuopio_324km2_10m.sqlite"
            ),
            crate::simulation::vegetation::VegetationConfig::default(),
        )
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
            commit_ready_with_lanes(
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
        }
    }
    assert_eq!(
        profile_count, 158,
        "must exercise every recorded checkpoint"
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn orthogonal_zoning_block_terrain_previews_and_commits() {
    orthogonal_zoning_block_at_angle(0.0);
}

#[test]
fn rotated_orthogonal_zoning_block_terrain_previews_and_commits() {
    for angle in [0.35, -0.55, 0.8, 2.2] {
        orthogonal_zoning_block_at_angle(angle);
    }
}

fn orthogonal_zoning_block_at_angle(angle: f64) {
    let u = glam::DVec2::new(angle.cos(), angle.sin());
    let v = u.perp();
    let rotate = |x, z| u * x + v * z;
    let mut core = test_core();
    core.benchmark_mode = false;
    core.config = WorldConfig::new(2048.0, 2048.0, 50.0, 10.0)
        .with_terrain_resolution(8.0)
        .with_chunking(512.0, 0.0);
    core.heightmap = TerrainSystem::from_world_config(&core.config);
    core.watermap = WaterSystem::from_world_config(&core.config);
    core.transit_network = TransitNetwork::new_for_world(&core.config);
    core.zoning = ZoningSystem::new(&core.config);
    core.pollution = PollutionSystem::new(&core.config);
    core.noise = NoiseSystem::new(&core.config);
    for segment in [
        [(-100.0, -200.0), (-100.0, 200.0)],
        [(-100.0, -150.0), (200.0, -150.0)],
        [(30.0, -150.0), (30.0, 150.0)],
        [(-100.0, 150.0), (30.0, 150.0)],
    ] {
        commit_ready(
            &mut core,
            segment
                .map(|(x, z)| {
                    let p = rotate(x, z);
                    Vector3::new(p.x as f32, 0.0, p.y as f32)
                })
                .to_vec(),
        );
    }
    let corner = rotate(30.0, -150.0);
    let junction = core
        .region_graph
        .find_node_within(Vector3::new(corner.x as f32, 0.0, corner.y as f32), 0.001)
        .unwrap();
    if angle == 0.0 {
        assert_eq!(core.region_graph.node(junction).pos.x, 30.0);
    }
    let path = temp_save_path("orthogonal_road_frames");
    core.save_game_internal(path.to_str().unwrap(), None)
        .unwrap();
    let before: Vec<_> = core
        .zoning
        .cells
        .saved_road_alignments()
        .into_iter()
        .filter(|(id, _)| !core.region_graph.edge(*id).deleted)
        .map(|(_, alignment)| serde_json::to_value(alignment).unwrap())
        .collect();
    let original = assert_orthogonal_block_cells(&mut core, angle);
    core.load_game_internal(path.to_str().unwrap()).unwrap();
    std::fs::remove_file(path).unwrap();
    let after: Vec<_> = core
        .zoning
        .cells
        .saved_road_alignments()
        .into_iter()
        .map(|(_, alignment)| serde_json::to_value(alignment).unwrap())
        .collect();
    assert_eq!(after, before, "road choices survive compacted edge ids");
    assert!(core.prepare_cell_chunk_internal((0, -1)));
    assert!(core.prepare_cell_chunk_internal((-1, 0)));
    assert_eq!(
        assert_orthogonal_block_cells(&mut core, angle),
        original,
        "cold reload and different viewport order retain exact geometry"
    );
}

fn assert_orthogonal_block_cells(
    core: &mut SimCore,
    angle: f64,
) -> Vec<(crate::simulation::zoning::cells::GridFrame, i32, i32)> {
    let u = glam::DVec2::new(angle.cos(), angle.sin());
    let v = u.perp();
    let preview = core.preview_cell_selection_internal(
        crate::simulation::zoning::cells::CellSelectionShape::Brush { radius_m: 600.0 },
        &[glam::DVec2::ZERO],
    );
    assert!(!preview.selection.cells.is_empty());
    let mut frame = None;
    let mut missing = Vec::new();
    let mut mismatched = Vec::new();
    for x in 0..12 {
        for y in 0..25 {
            let point = u * (-90.0 + x as f64 * 10.0) + v * (-120.0 + y as f64 * 10.0);
            let Some(cell) = core.zoning.cells.pick(point) else {
                missing.push((x, y));
                continue;
            };
            if *frame.get_or_insert(cell.grid) != cell.grid {
                mismatched.push((x, y, cell.grid));
            }
        }
    }
    assert!(
        missing.is_empty() && mismatched.is_empty(),
        "block angle {angle}: missing={missing:?}, different grids={mismatched:?}, frames={:?}",
        core.zoning.cells.saved_frames().collect::<Vec<_>>()
    );
    let mut cells: Vec<_> = preview
        .selection
        .cells
        .into_iter()
        .map(|key| (core.zoning.cells.frame(key.grid).unwrap(), key.x, key.y))
        .collect();
    cells.sort_unstable();
    cells
}

#[test]
fn branch_from_existing_t_compiles_four_way_junction_on_flat_and_slope() {
    // The sloped benchmark leg ending at (-70.6, -44.4) put a shared carriageway vertex on
    // an exact half-millimetre height; f32 rounding split its bands into a false conflict.
    for slope_end_m in [0.0, 6.4] {
        let mut core = test_core();
        core.create_blank_world_internal(512.0, 512.0, 8.0, 128.0, 0.0)
            .unwrap();
        core.slope_terrain_internal(
            Vector2::ZERO,
            256.0,
            Vector2::new(-128.0, 0.0),
            0.0,
            Vector2::new(128.0, 0.0),
            slope_end_m,
            1.0,
        );
        let surface = |core: &SimCore, x: f32, z: f32| {
            Vector3::new(
                x,
                core.get_world_surface_height_internal(Vector2::new(x, z)),
                z,
            )
        };
        for ((ax, az), (bx, bz)) in [((-110.0, 0.0), (110.0, 0.0)), ((-65.0, 0.0), (-65.0, 60.0))] {
            let points = vec![surface(&core, ax, az), surface(&core, bx, bz)];
            assert!(
                core.add_road_internal_with_snap(points, 1, 1, true)
                    .committed
            );
        }
        core.precompute_road_mesh_data();
        core.allocator
            .prepare_building_site_query_index(core.config.zone_cell_m);
        let snapshot = road_tool_snapshot_from_core(&core).unwrap();
        for step in 0..16 {
            let angle = std::f32::consts::TAU * step as f32 / 16.0;
            let (x, z) = (-60.0 + 15.0 * angle.cos(), -55.0 + 15.0 * angle.sin());
            let preview = snapshot.preview(&RoadPreviewRequest {
                include_terrain: false,
                request_id: step + 1,
                surface_generation: snapshot.surface_generation,
                points: vec![surface(&core, -65.0, 0.0), surface(&core, x, z)],
                fwd_lanes: 1,
                bkw_lanes: 1,
                snap_to_existing_roads: true,
            });
            assert!(
                preview.validation.is_valid,
                "slope {slope_end_m} leg end ({x}, {z}): {:?}",
                preview.validation
            );
        }
    }
}
