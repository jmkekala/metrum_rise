// SPDX-License-Identifier: GPL-2.0-only

//! Cdt regression tests for the simulation-node bridge.

use super::*;

fn test_cached_cdt_window(
    min_x_mm: i64,
    mesh_result: Result<TerrainCdtMesh, TerrainCdtError>,
) -> CachedRefinedTerrainCdtWindow {
    CachedRefinedTerrainCdtWindow {
        key: RefinedTerrainCdtWindowKey {
            min_x_mm,
            min_z_mm: 0,
            max_x_mm: min_x_mm + 1_000,
            max_z_mm: 1_000,
            fingerprint: min_x_mm as u64,
        },
        input_road_loops: 1,
        road_input: None,
        input_source_samples: 0,
        cdt_patch: TerrainCdtPatch::new(
            min_x_mm as f64 / 1_000.0,
            0.0,
            min_x_mm as f64 / 1_000.0 + 1.0,
            1.0,
            [0.0; 4],
        ),
        mesh_result,
        mesh_buffers: None,
        cdt_ms: 0.0,
        has_engineered_contributor: true,
        road_clip_fingerprints: vec![min_x_mm as u64],
        site_clip_fingerprints: Vec::new(),
    }
}

fn empty_test_cdt_mesh() -> TerrainCdtMesh {
    TerrainCdtMesh {
        vertices: Vec::new(),
        emitted_faces: Vec::new(),
        triangles: Vec::new(),
        terrain_triangle_sources: Vec::new(),
        retaining_wall_triangles: Vec::new(),
        retaining_wall_triangle_sources: Vec::new(),
        stats: empty_cdt_stats(),
        invalid_constraint_samples: Vec::new(),
        road_seam_face_samples: Vec::new(),
        retaining_wall_face_samples: Vec::new(),
        tie_in_widened_samples: Vec::new(),
        seam_quality_samples: Vec::new(),
        unpreserved_road_constraint_samples: Vec::new(),
    }
}

fn planner_test_patch(width_m: f32, height_m: f32) -> TerrainPatchSnapshot {
    TerrainPatchSnapshot {
        patch_x: 0,
        patch_z: 0,
        sample_width: 2,
        sample_height: 2,
        texture_width: 2,
        texture_height: 2,
        inner_offset_x: 0,
        inner_offset_z: 0,
        world_origin_x: 0.0,
        world_origin_z: 0.0,
        world_size_x: width_m,
        world_size_z: height_m,
        height_data: vec![0.0; 4],
    }
}

fn planner_test_loop(
    stable_piece_id: u64,
    min_x: f64,
    min_z: f64,
    max_x: f64,
    max_z: f64,
) -> TerrainCdtRoadLoop {
    TerrainCdtRoadLoop::new(
        stable_piece_id,
        0,
        vec![
            TerrainCdtVertex::new(min_x, 0.0, min_z),
            TerrainCdtVertex::new(max_x, 0.0, min_z),
            TerrainCdtVertex::new(max_x, 0.0, max_z),
            TerrainCdtVertex::new(min_x, 0.0, max_z),
        ],
    )
}

fn cached_patch_from_planned_windows(
    patch: TerrainPatchSnapshot,
    windows: Vec<RefinedTerrainCdtWindowBuildInput>,
    input_road_loops: usize,
) -> CachedRefinedTerrainPatch {
    let mut cached = test_cached_refined_terrain_patch(TERRAIN_CDT_CONTRACT_REVISION, 1);
    cached.patch = patch;
    cached.input_road_loops = input_road_loops;
    cached.windows = windows
        .into_iter()
        .map(|window| {
            Arc::new(CachedRefinedTerrainCdtWindow {
                key: window.key,
                road_input: window.road_input,
                input_road_loops: window.cdt_input.road_loops.len(),
                input_source_samples: window.cdt_input.source_samples.len(),
                cdt_patch: window.cdt_input.patch,
                mesh_result: Ok(empty_test_cdt_mesh()),
                mesh_buffers: None,
                cdt_ms: 0.0,
                has_engineered_contributor: window.has_engineered_contributor,
                road_clip_fingerprints: window.road_clip_fingerprints,
                site_clip_fingerprints: window.site_clip_fingerprints,
            })
        })
        .collect();
    cached
}

#[test]
fn terrain_cdt_structured_face_sources_preserve_span_fields() {
    let source = span_source();
    let export = SimulationNode::terrain_cdt_triangle_buffers(
        &test_patch(),
        &[
            TerrainCdtVertex::new(0.0, 0.0, 0.0),
            TerrainCdtVertex::new(1.0, 0.0, 0.0),
            TerrainCdtVertex::new(0.0, 0.0, 1.0),
        ],
        &[[0, 1, 2]],
        &[vec![source]],
        true,
        false,
    );

    assert_eq!(export.emitted_faces, 1);
    assert_eq!(export.face_sources.counts, vec![1]);
    assert_eq!(export.face_sources.labels.len(), 1);
    assert_eq!(export.face_sources.kind_codes, vec![0]);
    assert_eq!(export.face_sources.primary_ids, vec![123]);
    assert_eq!(export.face_sources.node_kind_codes, vec![-1]);
    assert_eq!(export.face_sources.edge_class_codes, vec![1]);
    assert_eq!(export.face_sources.owner_kinds, vec![2]);
    assert_eq!(export.face_sources.owner_indices, vec![7]);
    assert_eq!(export.face_sources.support_policies, vec![1]);
    assert_eq!(export.face_sources.roles, vec![2]);
    assert_eq!(export.face_sources.section_ranges, vec![2, 5]);
    assert_eq!(export.face_sources.s_ranges, vec![10.5, 14.0]);
}

#[test]
fn terrain_cdt_structured_face_sources_preserve_node_fields() {
    let source = node_source();
    let export = SimulationNode::terrain_cdt_triangle_buffers(
        &test_patch(),
        &[
            TerrainCdtVertex::new(0.0, 0.0, 0.0),
            TerrainCdtVertex::new(1.0, 0.0, 0.0),
            TerrainCdtVertex::new(0.0, 0.0, 1.0),
        ],
        &[[0, 1, 2]],
        &[vec![source]],
        true,
        false,
    );

    assert_eq!(export.emitted_faces, 1);
    assert_eq!(export.face_sources.counts, vec![1]);
    assert_eq!(export.face_sources.labels.len(), 1);
    assert_eq!(export.face_sources.kind_codes, vec![1]);
    assert_eq!(export.face_sources.primary_ids, vec![77]);
    assert_eq!(export.face_sources.node_kind_codes, vec![2]);
    assert_eq!(export.face_sources.edge_class_codes, vec![-1]);
    assert_eq!(export.face_sources.owner_kinds, vec![1]);
    assert_eq!(export.face_sources.owner_indices, vec![3]);
    assert_eq!(export.face_sources.support_policies, vec![-1]);
    assert_eq!(export.face_sources.roles, vec![-1]);
    assert_eq!(export.face_sources.section_ranges, vec![-1, -1]);
    assert_eq!(export.face_sources.s_ranges, vec![-1.0, -1.0]);
}

#[test]
fn terrain_cdt_face_source_counts_skip_degenerate_triangles() {
    let span = span_source();
    let node = node_source();
    let export = SimulationNode::terrain_cdt_triangle_buffers(
        &test_patch(),
        &[
            TerrainCdtVertex::new(0.0, 0.0, 0.0),
            TerrainCdtVertex::new(1.0, 0.0, 0.0),
            TerrainCdtVertex::new(2.0, 0.0, 0.0),
            TerrainCdtVertex::new(0.0, 0.0, 1.0),
        ],
        &[[0, 1, 2], [0, 1, 3]],
        &[vec![span], vec![span, node]],
        true,
        false,
    );

    assert_eq!(export.emitted_faces, 1);
    assert_eq!(export.face_sources.counts, vec![2]);
    assert_eq!(export.face_sources.labels.len(), 2);
    assert_eq!(export.face_sources.kind_codes, vec![0, 1]);
    assert_eq!(export.face_sources.primary_ids, vec![123, 77]);
    assert_eq!(export.face_sources.section_ranges, vec![2, 5, -1, -1]);
    assert_eq!(export.face_sources.s_ranges, vec![10.5, 14.0, -1.0, -1.0]);
}

#[test]
fn terrain_cdt_triangle_buffer_stats_measure_emitted_faces() {
    let export = SimulationNode::terrain_cdt_triangle_buffers(
        &test_patch(),
        &[
            TerrainCdtVertex::new(0.0, 0.0, 0.0),
            TerrainCdtVertex::new(4.0, 2.0, 0.0),
            TerrainCdtVertex::new(0.0, 0.0, 3.0),
        ],
        &[[0, 1, 2]],
        &[Vec::new()],
        false,
        false,
    );

    assert_eq!(export.emitted_faces, 1);
    assert!((export.max_face_y_delta_m - 2.0).abs() <= 0.0001);
    assert!((export.longest_triangle_edge_m - 5.0).abs() <= 0.0001);
    assert!(export.max_face_slope_ratio > 0.0);
}

#[test]
fn terrain_cdt_triangle_buffers_can_omit_pathological_faces() {
    let export = SimulationNode::terrain_cdt_triangle_buffers(
        &test_patch(),
        &[
            TerrainCdtVertex::new(0.0, 0.0, 0.0),
            TerrainCdtVertex::new(0.01, 10.0, 0.0),
            TerrainCdtVertex::new(0.0, 0.0, 4.0),
        ],
        &[[0, 1, 2]],
        &[Vec::new()],
        false,
        true,
    );

    assert_eq!(export.emitted_faces, 0);
    assert_eq!(export.omitted_pathological_faces, 1);
    assert!(export.vertices.is_empty());
    assert!(export.indices.is_empty());
    assert_eq!(export.max_face_y_delta_m, 0.0);
    assert_eq!(export.max_face_slope_ratio, 0.0);
    assert_eq!(export.longest_triangle_edge_m, 0.0);
}

#[test]
fn terrain_cdt_triangle_buffers_omitted_faces_do_not_poison_export_metrics() {
    let export = SimulationNode::terrain_cdt_triangle_buffers(
        &test_patch(),
        &[
            TerrainCdtVertex::new(0.0, 0.0, 0.0),
            TerrainCdtVertex::new(4.0, 0.0, 0.0),
            TerrainCdtVertex::new(0.0, 0.0, 3.0),
            TerrainCdtVertex::new(0.01, 10.0, 0.0),
        ],
        &[[0, 1, 2], [0, 3, 2]],
        &[Vec::new(), Vec::new()],
        false,
        true,
    );

    assert_eq!(export.emitted_faces, 1);
    assert_eq!(export.omitted_pathological_faces, 1);
    assert!((export.max_face_y_delta_m - 0.0).abs() <= 0.0001);
    assert_eq!(export.max_face_slope_ratio, 0.0);
    assert!((export.longest_triangle_edge_m - 5.0).abs() <= 0.0001);
}

#[test]
fn terrain_cdt_output_status_marks_pathological_meshes() {
    assert_eq!(
        SimulationNode::terrain_cdt_output_status(false, 45.0, 20.0),
        "ok"
    );
    assert_eq!(
        SimulationNode::terrain_cdt_output_status(
            false,
            TERRAIN_CDT_PATHOLOGICAL_FACE_SLOPE_RATIO + 1.0,
            20.0,
        ),
        "pathological"
    );
    assert_eq!(
        SimulationNode::terrain_cdt_output_status(
            false,
            1.0,
            TERRAIN_CDT_PATHOLOGICAL_TRIANGLE_EDGE_M + 1.0,
        ),
        "pathological"
    );
    assert_eq!(
        SimulationNode::terrain_cdt_output_status(
            true,
            TERRAIN_CDT_PATHOLOGICAL_FACE_SLOPE_RATIO + 1.0,
            20.0,
        ),
        "conflicted"
    );
}

#[test]
fn terrain_cdt_constraint_conflicts_ignore_rejected_road_owned_edges() {
    let mut stats = empty_cdt_stats();
    stats.road_constraint_edges = 10;
    stats.preserved_road_constraint_edges = 9;
    stats.invalid_constraint_edges = 0;

    assert!(!SimulationNode::terrain_cdt_stats_have_constraint_conflicts(stats));
}

#[test]
fn terrain_cdt_constraint_conflicts_include_invalid_or_missing_edges() {
    let mut stats = empty_cdt_stats();
    stats.invalid_constraint_edges = 1;
    assert!(SimulationNode::terrain_cdt_stats_have_constraint_conflicts(
        stats
    ));

    stats.invalid_constraint_edges = 0;
    stats.spade_missing_road_constraint_edges = 1;
    assert!(SimulationNode::terrain_cdt_stats_have_constraint_conflicts(
        stats
    ));
}

#[test]
fn terrain_cdt_constraint_conflicts_ignore_unpreserved_site_only_edges() {
    let mut stats = empty_cdt_stats();
    stats.road_constraint_edges = 10;
    stats.building_site_constraint_edges = 10;
    stats.preserved_road_constraint_edges = 9;
    stats.preserved_building_site_constraint_edges = 9;
    stats.invalid_constraint_edges = 0;

    assert!(
        !SimulationNode::terrain_cdt_stats_have_constraint_conflicts(stats),
        "site meshes own their top surface; unpreserved site boundary edges must not force old terrain fallback"
    );

    stats.spade_missing_road_constraint_edges = 1;
    assert!(
        SimulationNode::terrain_cdt_stats_have_constraint_conflicts(stats),
        "a road seam missing from the CDT must still force fallback"
    );
}

#[test]
fn cached_refined_patch_rejects_partial_window_success() {
    let mut cached = test_cached_refined_terrain_patch(TERRAIN_CDT_CONTRACT_REVISION, 1);
    cached.requires_road_clipping = true;
    cached.clip_source_count = 2;
    cached.road_clip_source_count = 1;
    cached.road_clip_loop_count = 1;
    cached.site_clip_loop_count = 1;
    cached.input_road_loops = 2;
    cached.windows = vec![
        Arc::new(test_cached_cdt_window(0, Ok(empty_test_cdt_mesh()))),
        Arc::new(test_cached_cdt_window(
            2_000,
            Err(TerrainCdtError::InvalidPatch),
        )),
    ];

    assert_eq!(
        SimulationNode::cached_refined_cdt_failure_label(&cached),
        Some("invalid_patch")
    );
}

#[test]
fn cached_refined_patch_rejects_constraint_conflicts() {
    let mut mesh = empty_test_cdt_mesh();
    mesh.stats.road_constraint_edges = 1;
    mesh.stats.spade_missing_road_constraint_edges = 1;

    let mut cached = test_cached_refined_terrain_patch(TERRAIN_CDT_CONTRACT_REVISION, 1);
    cached.requires_road_clipping = true;
    cached.clip_source_count = 1;
    cached.road_clip_source_count = 1;
    cached.road_clip_loop_count = 1;
    cached.input_road_loops = 1;
    cached.windows = vec![Arc::new(test_cached_cdt_window(0, Ok(mesh)))];

    assert_eq!(
        SimulationNode::cached_refined_cdt_failure_label(&cached),
        Some("terrain_cdt_constraint_conflicts")
    );
}

#[test]
fn refined_patch_build_reuses_successful_window_by_arc_identity() {
    let mut previous_window = test_cached_cdt_window(0, Ok(empty_test_cdt_mesh()));
    let previous_buffers = Arc::new(
        SimulationNode::prepare_cached_refined_terrain_window_mesh_buffers(
            &test_patch(),
            previous_window.cdt_patch,
            2.0,
            previous_window
                .mesh_result
                .as_ref()
                .expect("test window should be successful"),
        ),
    );
    previous_window.mesh_buffers = Some(Arc::clone(&previous_buffers));
    let previous = Arc::new(previous_window);
    let mut previous_patch = test_cached_refined_terrain_patch(TERRAIN_CDT_CONTRACT_REVISION, 1);
    previous_patch.input_road_loops = 1;
    previous_patch.requires_engineered_refinement = true;
    previous_patch.requires_road_clipping = true;
    previous_patch.clip_source_count = 1;
    previous_patch.road_clip_source_count = 1;
    previous_patch.road_clip_loop_count = 1;
    previous_patch.windows = vec![Arc::clone(&previous)];
    let previous_patch_buffers =
        Arc::new(SimulationNode::prepare_cached_refined_terrain_mesh_buffers(
            &previous_patch.patch,
            &[(&previous, previous.mesh_result.as_ref().unwrap())],
            2.0,
        ));
    previous_patch.mesh_buffers = Some(Arc::clone(&previous_patch_buffers));
    let key = previous.key;
    let cdt_input = TerrainCdtInput::new(previous.cdt_patch, Vec::new(), Vec::new());
    let input = RefinedTerrainPatchBuildInput {
        key: RefinedTerrainPatchCacheKey {
            patch_x: 0,
            patch_z: 0,
            render_step_mm: 2000,
        },
        surface_generation: 2,
        patch: test_patch(),
        previous_patch: Some(Arc::new(previous_patch)),
        windows: vec![RefinedTerrainCdtWindowBuildInput {
            key,
            cdt_input,
            road_input: None,
            previous: Some(Arc::clone(&previous)),
            has_engineered_contributor: true,
            road_clip_fingerprints: vec![0],
            site_clip_fingerprints: Vec::new(),
        }],
        reused_windows: Vec::new(),
        input_clip_loop_count: 1,
        omitted_margin_clip_loop_count: 0,
        expected_road_clip_fingerprints: vec![0],
        expected_site_clip_fingerprints: Vec::new(),
        requires_engineered_refinement: true,
        requires_road_clipping: true,
        clip_source_count: 1,
        road_clip_source_count: 1,
        road_clip_loop_count: 1,
        site_clip_loop_count: 0,
        clip_error_label: None,
        clip_query_margin_m: 8.0,
        derive_clip_counts_from_windows: false,
    };

    let mut entries = SimCore::build_refined_terrain_patch_cache_entries(vec![input]);
    let entry = entries.pop().expect("one refined patch should be built");

    assert_eq!(entry.surface_generation, 2);
    assert_eq!(entry.reused_windows, 1);
    assert_eq!(entry.windows.len(), 1);
    assert!(Arc::ptr_eq(&entry.windows[0], &previous));
    assert!(Arc::ptr_eq(
        entry.windows[0]
            .mesh_buffers
            .as_ref()
            .expect("reused successful window should retain output buffers"),
        &previous_buffers
    ));
    assert!(Arc::ptr_eq(
        entry
            .mesh_buffers
            .as_ref()
            .expect("matching-generation mesh composition must finish on the worker"),
        &previous_patch_buffers
    ));
}

#[test]
fn cached_window_buffers_preserve_multi_window_patch_assembly() {
    let mut left_mesh = empty_test_cdt_mesh();
    left_mesh.vertices = vec![
        TerrainCdtVertex::new(0.0, 0.0, 0.0),
        TerrainCdtVertex::new(1.0, 0.0, 0.0),
        TerrainCdtVertex::new(0.0, 0.0, 1.0),
        TerrainCdtVertex::new(0.0, 2.0, 0.0),
        TerrainCdtVertex::new(0.001, 1.0, 0.0),
        TerrainCdtVertex::new(0.0, 0.0, 0.001),
    ];
    left_mesh.triangles = vec![[0, 1, 2], [0, 4, 5]];
    left_mesh.terrain_triangle_sources = vec![Vec::new(), Vec::new()];
    left_mesh.retaining_wall_triangles = vec![[0, 3, 2]];
    left_mesh.retaining_wall_triangle_sources = vec![vec![span_source()]];

    let mut right_mesh = empty_test_cdt_mesh();
    right_mesh.vertices = vec![
        TerrainCdtVertex::new(1.0, 0.0, 0.0),
        TerrainCdtVertex::new(2.0, 0.5, 0.0),
        TerrainCdtVertex::new(1.0, 0.0, 1.0),
    ];
    right_mesh.triangles = vec![[0, 1, 2]];
    right_mesh.terrain_triangle_sources = vec![Vec::new()];

    let patch = test_patch();
    let mut left_window = test_cached_cdt_window(0, Ok(empty_test_cdt_mesh()));
    let mut right_window = test_cached_cdt_window(1_000, Ok(empty_test_cdt_mesh()));
    let fallback = SimulationNode::prepare_cached_refined_terrain_mesh_buffers(
        &patch,
        &[(&left_window, &left_mesh), (&right_window, &right_mesh)],
        2.0,
    );

    left_window.mesh_buffers = Some(Arc::new(
        SimulationNode::prepare_cached_refined_terrain_window_mesh_buffers(
            &patch,
            left_window.cdt_patch,
            2.0,
            &left_mesh,
        ),
    ));
    right_window.mesh_buffers = Some(Arc::new(
        SimulationNode::prepare_cached_refined_terrain_window_mesh_buffers(
            &patch,
            right_window.cdt_patch,
            2.0,
            &right_mesh,
        ),
    ));
    let cached = SimulationNode::prepare_cached_refined_terrain_mesh_buffers(
        &patch,
        &[(&left_window, &left_mesh), (&right_window, &right_mesh)],
        2.0,
    );

    assert!(
        cached.variant_payload_valid,
        "composed cached buffers must carry the off-thread validation certificate"
    );
    assert_eq!(
        cached, fallback,
        "cached tile conversion must preserve deterministic complete-patch assembly"
    );
    assert_eq!(cached.omitted_pathological_terrain_faces, 1);
    assert_eq!(cached.retaining_emitted_faces, 1);
    assert!(
        cached.terrain_emitted_faces > 2,
        "regular filler must cover the part of the render patch outside both CDT windows"
    );
    let shared_vertex = Vector3::new(-4.0, 0.0, -5.0);
    let shared_normals = cached
        .terrain_vertices
        .iter()
        .zip(&cached.terrain_normals)
        .filter_map(|(vertex, normal)| (*vertex == shared_vertex).then_some(*normal))
        .collect::<Vec<_>>();
    assert!(
        shared_normals.len() >= 2,
        "adjacent window assembly must retain duplicate seam vertices"
    );
    assert!(
        shared_normals
            .windows(2)
            .all(|pair| (pair[0] - pair[1]).length_squared() <= 0.000_001),
        "global normal reconciliation must assign one normal to duplicate seam vertices"
    );
}

#[test]
fn local_refined_build_derives_complete_clip_counts_from_window_manifests() {
    let mut road_window = test_cached_cdt_window(0, Ok(empty_test_cdt_mesh()));
    road_window.road_clip_fingerprints = vec![7];
    let mut mixed_window = test_cached_cdt_window(1_000, Ok(empty_test_cdt_mesh()));
    mixed_window.road_clip_fingerprints = vec![7];
    mixed_window.site_clip_fingerprints = vec![9];
    let input = RefinedTerrainPatchBuildInput {
        key: RefinedTerrainPatchCacheKey {
            patch_x: 0,
            patch_z: 0,
            render_step_mm: 2000,
        },
        surface_generation: 2,
        patch: test_patch(),
        previous_patch: None,
        windows: Vec::new(),
        reused_windows: vec![Arc::new(road_window), Arc::new(mixed_window)],
        input_clip_loop_count: 99,
        omitted_margin_clip_loop_count: 0,
        expected_road_clip_fingerprints: vec![7],
        expected_site_clip_fingerprints: vec![9],
        requires_engineered_refinement: true,
        requires_road_clipping: true,
        clip_source_count: 99,
        road_clip_source_count: 99,
        road_clip_loop_count: 99,
        site_clip_loop_count: 99,
        clip_error_label: None,
        clip_query_margin_m: 8.0,
        derive_clip_counts_from_windows: true,
    };

    let entry = SimCore::build_refined_terrain_patch_cache_entries(vec![input])
        .pop()
        .expect("one local generation should be built");

    assert_eq!(entry.input_road_loops, 2);
    assert_eq!(entry.clip_source_count, 2);
    assert_eq!(entry.road_clip_source_count, 1);
    assert_eq!(entry.road_clip_loop_count, 1);
    assert_eq!(entry.site_clip_loop_count, 1);
    assert_eq!(entry.omitted_margin_clip_loop_count, 0);
    assert!(SimulationNode::cached_refined_cdt_failure_label(&entry).is_none());
}

#[test]
fn local_refined_build_rejects_equal_sized_wrong_contributor_manifest() {
    let mut road_window = test_cached_cdt_window(0, Ok(empty_test_cdt_mesh()));
    road_window.road_clip_fingerprints = vec![7];
    let input = RefinedTerrainPatchBuildInput {
        key: RefinedTerrainPatchCacheKey {
            patch_x: 0,
            patch_z: 0,
            render_step_mm: 2000,
        },
        surface_generation: 2,
        patch: test_patch(),
        previous_patch: None,
        windows: Vec::new(),
        reused_windows: vec![Arc::new(road_window)],
        input_clip_loop_count: 1,
        omitted_margin_clip_loop_count: 0,
        expected_road_clip_fingerprints: vec![8],
        expected_site_clip_fingerprints: Vec::new(),
        requires_engineered_refinement: true,
        requires_road_clipping: true,
        clip_source_count: 1,
        road_clip_source_count: 1,
        road_clip_loop_count: 1,
        site_clip_loop_count: 0,
        clip_error_label: None,
        clip_query_margin_m: 8.0,
        derive_clip_counts_from_windows: true,
    };

    let entry = SimCore::build_refined_terrain_patch_cache_entries(vec![input])
        .pop()
        .expect("one local generation should be built");

    assert_eq!(
        SimulationNode::cached_refined_cdt_failure_label(&entry),
        Some("incomplete_terrain_clip_windows")
    );
    assert!(
        entry.mesh_buffers.is_none(),
        "a contributor-identity mismatch must not publish a complete mesh"
    );
}

#[test]
fn refined_patch_build_never_reuses_failed_window() {
    let previous = Arc::new(test_cached_cdt_window(
        0,
        Err(TerrainCdtError::InvalidPatch),
    ));
    let key = previous.key;
    let invalid_patch = TerrainCdtPatch::new(0.0, 0.0, 0.0, 1.0, [0.0; 4]);
    let input = RefinedTerrainPatchBuildInput {
        key: RefinedTerrainPatchCacheKey {
            patch_x: 0,
            patch_z: 0,
            render_step_mm: 2000,
        },
        surface_generation: 2,
        patch: test_patch(),
        previous_patch: None,
        windows: vec![RefinedTerrainCdtWindowBuildInput {
            key,
            cdt_input: TerrainCdtInput::new(invalid_patch, Vec::new(), Vec::new()),
            road_input: None,
            previous: Some(Arc::clone(&previous)),
            has_engineered_contributor: true,
            road_clip_fingerprints: vec![1],
            site_clip_fingerprints: Vec::new(),
        }],
        reused_windows: Vec::new(),
        input_clip_loop_count: 1,
        omitted_margin_clip_loop_count: 0,
        expected_road_clip_fingerprints: vec![1],
        expected_site_clip_fingerprints: Vec::new(),
        requires_engineered_refinement: true,
        requires_road_clipping: true,
        clip_source_count: 1,
        road_clip_source_count: 1,
        road_clip_loop_count: 1,
        site_clip_loop_count: 0,
        clip_error_label: None,
        clip_query_margin_m: 8.0,
        derive_clip_counts_from_windows: false,
    };

    let mut entries = SimCore::build_refined_terrain_patch_cache_entries(vec![input]);
    let entry = entries.pop().expect("one refined patch should be built");

    assert_eq!(entry.reused_windows, 0);
    assert_eq!(entry.windows.len(), 1);
    assert!(!Arc::ptr_eq(&entry.windows[0], &previous));
    assert_eq!(
        entry.windows[0].mesh_result,
        Err(TerrainCdtError::InvalidPatch)
    );
    assert!(
        entry.mesh_buffers.is_none(),
        "failed generations must not publish partial mesh buffers"
    );
}

#[test]
fn failed_current_build_does_not_replace_last_successful_baseline() {
    let mut core = test_core_with_flat_terrain(0.0);
    core.terrain_payload_generation_counter = 2;
    core.terrain_payload_global_generation = 2;
    let successful_window = Arc::new(test_cached_cdt_window(0, Ok(empty_test_cdt_mesh())));
    let mut baseline = test_cached_refined_terrain_patch(TERRAIN_CDT_CONTRACT_REVISION, 1);
    baseline.input_road_loops = 1;
    baseline.requires_engineered_refinement = true;
    baseline.requires_road_clipping = true;
    baseline.clip_source_count = 1;
    baseline.road_clip_source_count = 1;
    baseline.road_clip_loop_count = 1;
    baseline.windows = vec![Arc::clone(&successful_window)];
    let key = baseline.key;
    core.refined_terrain_patch_cache
        .insert(key, Arc::new(baseline));

    let mut failed = test_cached_refined_terrain_patch(TERRAIN_CDT_CONTRACT_REVISION, 2);
    failed.input_road_loops = 1;
    failed.requires_engineered_refinement = true;
    failed.requires_road_clipping = true;
    failed.clip_source_count = 1;
    failed.road_clip_source_count = 1;
    failed.road_clip_loop_count = 1;
    failed.windows = vec![Arc::new(test_cached_cdt_window(
        0,
        Err(TerrainCdtError::InvalidPatch),
    ))];

    let inserted = core.insert_refined_terrain_patch_cache_entries(vec![Arc::new(failed)]);
    let retained = core
        .refined_terrain_patch_cache
        .get(&key)
        .expect("failed current build must retain the previous successful baseline");

    assert_eq!(inserted, 0);
    assert_eq!(retained.surface_generation, 1);
    assert!(Arc::ptr_eq(&retained.windows[0], &successful_window));
}

#[test]
fn road_locked_refined_patch_rejects_site_only_clip_payload() {
    assert_eq!(
        SimulationNode::terrain_clip_input_failure_label(true, 1, 0, 0, 1, 0, 1, None,),
        Some("missing_road_clip_sources")
    );
    assert_eq!(
        SimulationNode::terrain_clip_input_failure_label(false, 1, 0, 0, 1, 0, 1, None,),
        None,
        "site-only patches remain valid when no grounded road owns the patch"
    );
}

#[test]
fn refined_patch_rejects_dropped_clip_loop_before_cdt() {
    assert_eq!(
        SimulationNode::terrain_clip_input_failure_label(true, 2, 1, 1, 1, 0, 1, None,),
        Some("incomplete_terrain_clip_windows")
    );
}

#[test]
fn refined_patch_accepts_query_margin_loop_without_patch_influence() {
    assert_eq!(
        SimulationNode::terrain_clip_input_failure_label(true, 2, 1, 2, 0, 1, 1, None,),
        None
    );
}

#[test]
fn road_clip_failure_cannot_be_masked_by_site_loops() {
    assert_eq!(
        SimulationNode::terrain_clip_input_failure_label(
            true,
            1,
            0,
            0,
            1,
            0,
            1,
            Some("terrain_clip_missing_output_boundary_owner"),
        ),
        Some("terrain_clip_missing_output_boundary_owner")
    );
}

#[test]
fn terrain_cdt_conflicting_road_height_error_has_stable_label() {
    assert_eq!(
        SimulationNode::terrain_cdt_error_label(&TerrainCdtError::ConflictingRoadBoundaryHeight),
        "conflicting_road_boundary_height"
    );
}

#[test]
fn terrain_cdt_regular_filler_stats_measure_emitted_faces() {
    let mut export = TerrainCdtTriangleBufferExport::empty();
    let patch = TerrainPatchSnapshot {
        height_data: vec![0.0, 2.0, 0.0, 0.0],
        ..test_patch()
    };

    SimulationNode::append_regular_terrain_mesh_outside_cdt_windows(&mut export, &patch, &[]);

    assert!(export.emitted_faces > 0);
    assert!(export.max_face_y_delta_m >= 2.0);
    assert!(export.longest_triangle_edge_m > 0.0);
    assert!(export.max_face_slope_ratio > 0.0);
}

#[test]
fn terrain_cdt_road_seam_sample_sources_export_structured_rows() {
    let span = span_source();
    let node = node_source();
    let sources = [span, node];
    let export = source_export_for_samples(&[&sources]);

    assert_eq!(export.counts, vec![2]);
    assert_eq!(export.labels.len(), 2);
    assert_eq!(export.kind_codes, vec![0, 1]);
    assert_eq!(export.primary_ids, vec![123, 77]);
    assert_eq!(export.node_kind_codes, vec![-1, 2]);
    assert_eq!(export.edge_class_codes, vec![1, -1]);
    assert_eq!(export.owner_kinds, vec![2, 1]);
    assert_eq!(export.owner_indices, vec![7, 3]);
    assert_eq!(export.support_policies, vec![1, -1]);
    assert_eq!(export.roles, vec![2, -1]);
    assert_eq!(export.section_ranges, vec![2, 5, -1, -1]);
    assert_eq!(export.s_ranges, vec![10.5, 14.0, -1.0, -1.0]);
}

#[test]
fn terrain_cdt_retaining_wall_sample_sources_export_structured_rows() {
    let span = [span_source()];
    let node = [node_source()];
    let export = source_export_for_samples(&[&span, &node]);

    assert_eq!(export.counts, vec![1, 1]);
    assert_eq!(export.labels.len(), 2);
    assert_eq!(export.kind_codes, vec![0, 1]);
    assert_eq!(export.primary_ids, vec![123, 77]);
    assert_eq!(export.owner_kinds, vec![2, 1]);
    assert_eq!(export.owner_indices, vec![7, 3]);
    assert_eq!(export.section_ranges, vec![2, 5, -1, -1]);
    assert_eq!(export.s_ranges, vec![10.5, 14.0, -1.0, -1.0]);
}

#[test]
fn terrain_cdt_tie_in_widened_sample_sources_export_one_source_per_sample() {
    let first = [span_source()];
    let second = [span_source()];
    let export = source_export_for_samples(&[&first, &second]);

    assert_eq!(export.counts, vec![1, 1]);
    assert_eq!(export.labels.len(), 2);
    assert_eq!(export.kind_codes, vec![0, 0]);
    assert_eq!(export.primary_ids, vec![123, 123]);
    assert_eq!(export.section_ranges, vec![2, 5, 2, 5]);
    assert_eq!(export.s_ranges, vec![10.5, 14.0, 10.5, 14.0]);
}

#[test]
fn terrain_cdt_invalid_constraint_sample_sources_keep_absence_visible() {
    let present = [node_source()];
    let export = source_export_for_samples(&[&[], &present]);

    assert_eq!(export.counts, vec![0, 1]);
    assert_eq!(export.labels.len(), 1);
    assert_eq!(export.kind_codes, vec![1]);
    assert_eq!(export.primary_ids, vec![77]);
    assert_eq!(export.owner_kinds, vec![1]);
    assert_eq!(export.owner_indices, vec![3]);
    assert_eq!(export.section_ranges, vec![-1, -1]);
    assert_eq!(export.s_ranges, vec![-1.0, -1.0]);
}

#[test]
fn road_clip_query_metadata_keeps_clip_failure_visible_without_loops() {
    let query = RoadClipLoopQuery {
        cdt_road_loops: Vec::new(),
        source_count: 1,
        road_source_count: 1,
        road_loop_count: 0,
        site_loop_count: 0,
        clip_error_label: Some("terrain_clip_missing_output_boundary_owner"),
    };

    let (status, error, source_count) = SimulationNode::road_clip_status_values(&query);

    assert_eq!(status, "failed");
    assert_eq!(error, "terrain_clip_missing_output_boundary_owner");
    assert_eq!(source_count, 1);
    assert!(SimulationNode::road_clip_query_requires_road_clipping(
        &query, false
    ));
    assert!(
        query.cdt_road_loops.is_empty(),
        "the failure status must survive even when there are no loops to upload"
    );
}

#[test]
fn road_clip_query_metadata_marks_absent_road_clip_as_ok() {
    let query = RoadClipLoopQuery {
        cdt_road_loops: Vec::new(),
        source_count: 0,
        road_source_count: 0,
        road_loop_count: 0,
        site_loop_count: 0,
        clip_error_label: None,
    };

    let (status, error, source_count) = SimulationNode::road_clip_status_values(&query);

    assert_eq!(status, "ok");
    assert_eq!(error, "none");
    assert_eq!(source_count, 0);
    assert!(!SimulationNode::road_clip_query_requires_road_clipping(
        &query, false
    ));
}

#[test]
fn terrain_cdt_local_window_input_samples_arbitrary_boundary() {
    let terrain = TerrainSystem::with_chunking(8, 8, 10.0, 4, 0.0);
    let patch = TerrainPatchSnapshot {
        patch_x: 0,
        patch_z: 0,
        sample_width: 5,
        sample_height: 5,
        texture_width: 5,
        texture_height: 5,
        inner_offset_x: 0,
        inner_offset_z: 0,
        world_origin_x: 0.0,
        world_origin_z: 0.0,
        world_size_x: 40.0,
        world_size_z: 40.0,
        height_data: vec![0.0; 25],
    };

    let input = SimulationNode::terrain_cdt_input_for_bounds(
        &terrain,
        &patch,
        &[],
        5.0,
        (3.0, 4.0, 23.0, 29.0),
        None,
    );

    assert!(
        input
            .source_samples
            .iter()
            .any(|sample| sample.x == 3.0 && sample.z == 5.0),
        "local CDT windows must seed globally aligned vertices along arbitrary vertical boundaries"
    );
    assert!(
        input
            .source_samples
            .iter()
            .any(|sample| sample.x == 5.0 && sample.z == 29.0),
        "local CDT windows must seed globally aligned vertices along arbitrary horizontal boundaries"
    );
}

#[test]
fn terrain_cdt_background_grid_matches_source_resolution_but_keeps_fine_seams() {
    let terrain = TerrainSystem::with_chunking(65, 65, 1.0, 64, 0.0);
    let patch = TerrainPatchSnapshot {
        patch_x: 0,
        patch_z: 0,
        sample_width: 9,
        sample_height: 9,
        texture_width: 9,
        texture_height: 9,
        inner_offset_x: 0,
        inner_offset_z: 0,
        world_origin_x: 0.0,
        world_origin_z: 0.0,
        world_size_x: 64.0,
        world_size_z: 64.0,
        height_data: vec![0.0; 81],
    };

    let input = SimulationNode::terrain_cdt_input_for_bounds(
        &terrain,
        &patch,
        &[],
        2.0,
        (0.0, 0.0, 64.0, 64.0),
        None,
    );

    assert!(
        input
            .source_samples
            .iter()
            .any(|sample| sample.x == 8.0 && sample.z == 8.0),
        "the background grid must retain source-resolution interior samples"
    );
    assert!(
        !input
            .source_samples
            .iter()
            .any(|sample| sample.x == 2.0 && sample.z == 2.0),
        "the background grid must not invent render-resolution interior terrain samples"
    );
    assert!(
        input
            .source_samples
            .iter()
            .any(|sample| sample.x == 2.0 && sample.z == 0.0),
        "window seams must retain render-resolution samples"
    );
}

#[test]
fn terrain_cdt_planner_splits_crossing_loop_into_fixed_world_tiles() {
    let terrain = TerrainSystem::with_chunking(257, 65, 1.0, 64, 0.0);
    let patch = planner_test_patch(256.0, 64.0);
    let road_loop = planner_test_loop(123, 10.0, 24.0, 150.0, 32.0);

    let plan = SimulationNode::terrain_cdt_window_build_inputs(
        &terrain,
        &patch,
        &[road_loop],
        2.0,
        None,
        None,
    );

    assert_eq!(plan.represented_road_loop_count, 1);
    let actual_road_manifest = plan
        .windows
        .iter()
        .flat_map(|window| window.road_clip_fingerprints.iter().copied())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        actual_road_manifest,
        plan.expected_road_clip_fingerprints
            .iter()
            .copied()
            .collect(),
        "the independently planned contributor set must match every emitted road window"
    );
    let road_tile_min_x = plan
        .windows
        .iter()
        .filter(|window| !window.cdt_input.road_loops.is_empty())
        .map(|window| window.key.min_x_mm)
        .collect::<Vec<_>>();
    assert_eq!(road_tile_min_x, vec![0, 64_000, 128_000]);
    assert!(
        plan.windows.iter().all(|window| {
            window.key.min_x_mm % 64_000 == 0
                && window.key.max_x_mm - window.key.min_x_mm <= 64_000
                && window.key.max_z_mm - window.key.min_z_mm <= 64_000
        }),
        "every core must be bounded and anchored to the 64 m world grid"
    );
    assert!(
        plan.windows
            .iter()
            .all(|window| window.cdt_input.source_samples.len() <= 1_089),
        "a 64 m core at the production 2 m step must keep its base grid bounded"
    );
    assert!(plan.windows.iter().all(|window| {
        window
            .cdt_input
            .road_loops
            .iter()
            .all(|road_loop| road_loop.stable_piece_id != 123)
    }));
}

#[test]
fn terrain_cdt_tile_rekey_preserves_source_outer_hole_ownership_after_clipping() {
    let terrain = TerrainSystem::with_chunking(129, 65, 1.0, 64, 0.0);
    let patch = planner_test_patch(128.0, 64.0);
    let grouped_loop = |stable_piece_id, local_loop_index, is_hole, vertices| {
        TerrainCdtRoadLoop::new_with_source_edges_and_topology(
            stable_piece_id,
            77,
            local_loop_index,
            is_hole,
            vertices,
            Vec::new(),
        )
    };
    let outer = grouped_loop(
        10,
        0,
        false,
        vec![
            TerrainCdtVertex::new(-20.0, 0.0, 10.0),
            TerrainCdtVertex::new(100.0, 0.0, 10.0),
            TerrainCdtVertex::new(100.0, 0.0, 54.0),
            TerrainCdtVertex::new(-20.0, 0.0, 54.0),
        ],
    );
    let hole = grouped_loop(
        11,
        1,
        true,
        vec![
            TerrainCdtVertex::new(-10.0, 0.0, 20.0),
            TerrainCdtVertex::new(80.0, 0.0, 20.0),
            TerrainCdtVertex::new(80.0, 0.0, 44.0),
            TerrainCdtVertex::new(-10.0, 0.0, 44.0),
        ],
    );

    let plan = SimulationNode::terrain_cdt_window_build_inputs(
        &terrain,
        &patch,
        &[outer, hole],
        2.0,
        None,
        None,
    );
    let clipped_tile = plan
        .windows
        .iter()
        .find(|window| window.key.min_x_mm == 64_000)
        .expect("the grouped footprint should influence the right tile");
    let outer_group = clipped_tile
        .cdt_input
        .road_loops
        .iter()
        .find(|road_loop| !road_loop.is_hole)
        .map(|road_loop| road_loop.footprint_group_id)
        .expect("the clipped tile should retain the outer contour");
    let hole_group = clipped_tile
        .cdt_input
        .road_loops
        .iter()
        .find(|road_loop| road_loop.is_hole)
        .map(|road_loop| road_loop.footprint_group_id)
        .expect("the clipped tile should retain the hole contour");

    assert_eq!(
        outer_group, hole_group,
        "tile-local topology must preserve the authoritative source group even when clipping puts the hole on the halo boundary"
    );
}

#[test]
fn terrain_cdt_planner_reuses_unchanged_tiles_and_rebuilds_cardinal_seams() {
    let terrain = TerrainSystem::with_chunking(257, 65, 1.0, 64, 0.0);
    let patch = planner_test_patch(256.0, 64.0);
    let left = planner_test_loop(10, 10.0, 24.0, 20.0, 32.0);
    let right = planner_test_loop(20, 210.0, 24.0, 220.0, 32.0);
    let first = SimulationNode::terrain_cdt_window_build_inputs(
        &terrain,
        &patch,
        &[left, right.clone()],
        2.0,
        None,
        None,
    );
    assert_eq!(first.windows.len(), 2);
    assert!(
        first
            .windows
            .iter()
            .all(|window| window.has_engineered_contributor),
        "contributor-free neighbors belong to regular filler, not Spade CDT"
    );
    let previous = cached_patch_from_planned_windows(
        patch.clone(),
        first.windows,
        first.represented_road_loop_count,
    );
    let moved_left = planner_test_loop(10, 12.0, 24.0, 20.0, 32.0);

    let changed = SimulationNode::terrain_cdt_window_build_inputs(
        &terrain,
        &patch,
        &[moved_left, right.clone()],
        2.0,
        None,
        Some(&previous),
    );
    let reuse_by_min_x = changed
        .windows
        .iter()
        .map(|window| (window.key.min_x_mm, window.previous.is_some()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(reuse_by_min_x.get(&0), Some(&false));
    assert_eq!(reuse_by_min_x.get(&192_000), Some(&true));
    assert_eq!(reuse_by_min_x.len(), 2);

    let removed = SimulationNode::terrain_cdt_window_build_inputs(
        &terrain,
        &patch,
        &[right],
        2.0,
        None,
        Some(&previous),
    );
    assert_eq!(removed.represented_road_loop_count, 1);
    assert_eq!(
        removed
            .windows
            .iter()
            .map(|window| window.key.min_x_mm)
            .collect::<Vec<_>>(),
        vec![192_000],
        "old-only tiles must be absent from the complete current generation"
    );
    assert!(removed.windows[0].previous.is_some());
}

#[test]
fn terrain_cdt_road_input_reuse_matches_cold_planning_and_invalidates_exact_changes() {
    let terrain = TerrainSystem::with_chunking(129, 65, 1.0, 64, 0.0);
    let patch = planner_test_patch(128.0, 64.0);
    let road_loop = planner_test_loop(7, 10.0, 24.0, 90.0, 32.0);
    let plan = |terrain: &TerrainSystem,
                patch: &TerrainPatchSnapshot,
                road: &TerrainCdtRoadLoop,
                step,
                previous: Option<&CachedRefinedTerrainPatch>| {
        SimulationNode::terrain_cdt_window_build_inputs(
            terrain,
            patch,
            std::slice::from_ref(road),
            step,
            None,
            previous,
        )
    };
    let first = plan(&terrain, &patch, &road_loop, 2.0, None);
    let previous = cached_patch_from_planned_windows(patch.clone(), first.windows, 1);
    let check = |terrain: &TerrainSystem,
                 patch: &TerrainPatchSnapshot,
                 road: &TerrainCdtRoadLoop,
                 step,
                 expected_hits| {
        let cold = plan(terrain, patch, road, step, None);
        let warm = plan(terrain, patch, road, step, Some(&previous));
        assert_eq!(cold.windows.len(), warm.windows.len());
        let mut hits = 0;
        for (cold, warm) in cold.windows.iter().zip(&warm.windows) {
            assert_eq!(cold.key, warm.key);
            assert_eq!(cold.cdt_input, warm.cdt_input);
            assert_eq!(cold.road_clip_fingerprints, warm.road_clip_fingerprints);
            assert_eq!(cold.site_clip_fingerprints, warm.site_clip_fingerprints);
            let road_input = warm.road_input.as_ref().unwrap();
            hits += usize::from(
                previous
                    .windows
                    .iter()
                    .any(|old| Arc::ptr_eq(old.road_input.as_ref().unwrap(), road_input)),
            );
        }
        assert_eq!(hits, expected_hits);
        warm
    };
    check(&terrain, &patch, &road_loop, 2.0, 2);
    // Render-step and non-corner terrain changes must still resample the complete CDT input.
    let coarser = check(&terrain, &patch, &road_loop, 4.0, 2);
    assert_ne!(coarser.windows[0].key, previous.windows[0].key);
    let mut changed_terrain = terrain.clone();
    // Terrain storage is centered on world (0, 0): grid (96, 32) is world (32, 0).
    changed_terrain.set_height(96, 32, 1.0);
    let resampled = check(&changed_terrain, &patch, &road_loop, 2.0, 2);
    assert_ne!(resampled.windows[0].key, previous.windows[0].key);
    // A core corner also participates in road clipping's boundary-height fallback.
    changed_terrain.set_height(64, 32, 1.0);
    check(&changed_terrain, &patch, &road_loop, 2.0, 1);
    let mut resized_patch = patch.clone();
    resized_patch.world_size_x = 127.0;
    check(&terrain, &resized_patch, &road_loop, 2.0, 1);
    for change in 0..5 {
        let mut changed_road = road_loop.clone();
        match change {
            0 => changed_road.vertices[0].x += 0.000_001,
            1 => changed_road.vertices[0].height_m = -0.0,
            2 => changed_road.source_edges[0].start.height_m = -0.0,
            3 => {
                changed_road.source_edges[0].source =
                    TerrainCdtRoadBoundarySource::BuildingSiteBoundary {
                        building_idx: 8,
                        local_loop_index: 0,
                        local_edge_index: 0,
                    }
            }
            _ => changed_road.footprint_group_id += 1,
        }
        check(&terrain, &patch, &changed_road, 2.0, 0);
    }
}

#[test]
#[ignore = "controlled release timing: cargo test --release benchmark_terrain_cdt_road_input_reuse -- --ignored --nocapture"]
fn benchmark_terrain_cdt_road_input_reuse() {
    assert!(
        !cfg!(debug_assertions),
        "benchmark requires a release build"
    );
    let terrain = TerrainSystem::with_chunking(1025, 1025, 1.0, 64, 0.0);
    let patch = planner_test_patch(512.0, 512.0);
    let road_loops = [TerrainCdtRoadLoop::new(
        7,
        0,
        (0..128)
            .map(|index| {
                let angle = std::f64::consts::TAU * index as f64 / 128.0;
                TerrainCdtVertex::new(
                    256.0 + 220.0 * angle.cos(),
                    0.0,
                    256.0 + 180.0 * angle.sin(),
                )
            })
            .collect(),
    )];
    let plan = |previous| {
        SimulationNode::terrain_cdt_window_build_inputs(
            &terrain,
            &patch,
            &road_loops,
            2.0,
            None,
            previous,
        )
    };
    let first = plan(None);
    let warm = cached_patch_from_planned_windows(patch.clone(), first.windows, 1);
    let mut cold = warm.clone();
    for window in &mut cold.windows {
        Arc::make_mut(window).road_input = None;
    }
    let cold_plan = plan(Some(&cold));
    let warm_plan = plan(Some(&warm));
    assert_eq!(cold_plan.windows.len(), warm_plan.windows.len());
    for (cold, warm) in cold_plan.windows.iter().zip(&warm_plan.windows) {
        assert_eq!(cold.key, warm.key);
        assert_eq!(cold.cdt_input, warm.cdt_input);
    }
    // Alternate order to limit warm-up / thermal bias. This measures input assembly, not CDT or
    // end-to-end gameplay; both arms retain the same final-window cache and normal Rayon pool.
    let mut timings = [Vec::new(), Vec::new()];
    for round in 0..22 {
        for index in [round % 2, 1 - round % 2] {
            let previous = if index == 0 { &cold } else { &warm };
            let start = Instant::now();
            for _ in 0..10 {
                std::hint::black_box(plan(Some(previous)));
            }
            if round >= 2 {
                timings[index].push(start.elapsed().as_secs_f64() * 100.0);
            }
        }
    }
    for samples in &mut timings {
        samples.sort_by(f64::total_cmp);
    }
    let cold_ms = (timings[0][9] + timings[0][10]) * 0.5;
    let warm_ms = (timings[1][9] + timings[1][10]) * 0.5;
    println!(
        "terrain input assembly: tiles={} cold_ms={cold_ms:.3} warm_ms={warm_ms:.3} speedup={:.2}x",
        warm.windows.len(),
        cold_ms / warm_ms
    );
}

#[test]
fn terrain_cdt_planner_does_not_reuse_stale_halo_contributor_manifest() {
    let terrain = TerrainSystem::with_chunking(129, 65, 1.0, 64, 0.0);
    let patch = planner_test_patch(128.0, 64.0);
    let road_loop = planner_test_loop(7, 10.0, 24.0, 90.0, 32.0);
    let first = SimulationNode::terrain_cdt_window_build_inputs(
        &terrain,
        &patch,
        std::slice::from_ref(&road_loop),
        2.0,
        None,
        None,
    );
    let represented_road_loop_count = first.represented_road_loop_count;
    let mut previous = cached_patch_from_planned_windows(
        patch.clone(),
        first.windows,
        represented_road_loop_count,
    );
    let stale_key = previous.windows[0].key;
    Arc::make_mut(&mut previous.windows[0]).road_clip_fingerprints = vec![u64::MAX];

    let repeated = SimulationNode::terrain_cdt_window_build_inputs(
        &terrain,
        &patch,
        &[road_loop],
        2.0,
        None,
        Some(&previous),
    );
    let stale_tile = repeated
        .windows
        .iter()
        .find(|window| window.key == stale_key)
        .expect("the unchanged geometry must retain the same tile key");

    assert!(
        stale_tile.previous.is_none(),
        "mesh-key equality must not reuse an Arc carrying stale contributor metadata"
    );
    assert!(
        repeated
            .windows
            .iter()
            .filter(|window| window.key != stale_key)
            .all(|window| window.previous.is_some()),
        "tiles with matching geometry and contributor manifests must remain reusable"
    );
}

#[test]
fn terrain_cdt_incremental_planner_drops_removed_local_tiles_without_assembling_remote_tiles() {
    let terrain = TerrainSystem::with_chunking(257, 65, 1.0, 64, 0.0);
    let patch = planner_test_patch(256.0, 64.0);
    let cached_window = |tile_x: i64, has_engineered_contributor| {
        let min_x_mm = tile_x * 64_000;
        Arc::new(CachedRefinedTerrainCdtWindow {
            key: RefinedTerrainCdtWindowKey {
                min_x_mm,
                min_z_mm: 0,
                max_x_mm: min_x_mm + 64_000,
                max_z_mm: 64_000,
                fingerprint: tile_x as u64,
            },
            input_road_loops: usize::from(has_engineered_contributor),
            road_input: None,
            input_source_samples: 0,
            cdt_patch: TerrainCdtPatch::new(
                min_x_mm as f64 / 1_000.0,
                0.0,
                min_x_mm as f64 / 1_000.0 + 64.0,
                64.0,
                [0.0; 4],
            ),
            mesh_result: Ok(empty_test_cdt_mesh()),
            mesh_buffers: None,
            cdt_ms: 0.0,
            has_engineered_contributor,
            road_clip_fingerprints: has_engineered_contributor
                .then_some(tile_x as u64)
                .into_iter()
                .collect(),
            site_clip_fingerprints: Vec::new(),
        })
    };
    let removed_contributor = cached_window(0, true);
    let removed_seam = cached_window(1, false);
    let remote_seam = cached_window(2, false);
    let remote_contributor = cached_window(3, true);
    let mut previous = test_cached_refined_terrain_patch(TERRAIN_CDT_CONTRACT_REVISION, 1);
    previous.patch = patch.clone();
    previous.windows = vec![
        Arc::clone(&removed_contributor),
        Arc::clone(&removed_seam),
        Arc::clone(&remote_seam),
        Arc::clone(&remote_contributor),
    ];
    let graph = crate::simulation::network::graph::RegionGraph::new();
    let road_surface = RoadSurfaceSystem::new(512.0);
    let sites = BuildingSiteTerrainSnapshot::default();

    let (plan, query) = SimulationNode::terrain_cdt_incremental_window_build_inputs(
        &terrain,
        &patch,
        &graph,
        &road_surface,
        &sites,
        &[(0, 0), (1, 0)],
        2.0,
        8.0,
        &previous,
    );

    assert!(plan.windows.is_empty());
    assert_eq!(plan.reused_windows.len(), 2);
    assert!(Arc::ptr_eq(&plan.reused_windows[0], &remote_seam));
    assert!(Arc::ptr_eq(&plan.reused_windows[1], &remote_contributor));
    assert_eq!(query.source_count, 0);
    assert_eq!(plan.represented_road_loop_count, 0);
    assert_eq!(
        plan.expected_road_clip_fingerprints, remote_contributor.road_clip_fingerprints,
        "local removal must retain only the remote contributor ids in the expected generation"
    );
}

#[test]
fn terrain_cdt_incremental_planner_work_stays_bounded_with_many_remote_tiles() {
    let terrain = TerrainSystem::with_chunking(65, 65, 1.0, 64, 0.0);
    let patch = planner_test_patch(512.0, 512.0);
    let mut previous = test_cached_refined_terrain_patch(TERRAIN_CDT_CONTRACT_REVISION, 1);
    previous.patch = patch.clone();
    for z in 0_i64..8 {
        for x in 0_i64..8 {
            let min_x_mm = x * 64_000;
            let min_z_mm = z * 64_000;
            previous
                .windows
                .push(Arc::new(CachedRefinedTerrainCdtWindow {
                    key: RefinedTerrainCdtWindowKey {
                        min_x_mm,
                        min_z_mm,
                        max_x_mm: min_x_mm + 64_000,
                        max_z_mm: min_z_mm + 64_000,
                        fingerprint: (z * 8 + x) as u64,
                    },
                    input_road_loops: 1,
                    road_input: None,
                    input_source_samples: 0,
                    cdt_patch: TerrainCdtPatch::new(
                        min_x_mm as f64 / 1_000.0,
                        min_z_mm as f64 / 1_000.0,
                        min_x_mm as f64 / 1_000.0 + 64.0,
                        min_z_mm as f64 / 1_000.0 + 64.0,
                        [0.0; 4],
                    ),
                    mesh_result: Ok(empty_test_cdt_mesh()),
                    mesh_buffers: None,
                    cdt_ms: 0.0,
                    has_engineered_contributor: true,
                    road_clip_fingerprints: vec![(z * 8 + x) as u64],
                    site_clip_fingerprints: Vec::new(),
                }));
        }
    }
    let graph = crate::simulation::network::graph::RegionGraph::new();
    let road_surface = RoadSurfaceSystem::new(512.0);
    let sites = BuildingSiteTerrainSnapshot::default();
    let local_scope = [(0, 0), (1, 0), (0, 1)];

    let (plan, _) = SimulationNode::terrain_cdt_incremental_window_build_inputs(
        &terrain,
        &patch,
        &graph,
        &road_surface,
        &sites,
        &local_scope,
        2.0,
        8.0,
        &previous,
    );

    assert!(
        plan.windows.is_empty(),
        "dirty tiles without current contributors must return to regular filler"
    );
    assert_eq!(plan.reused_windows.len(), 64 - local_scope.len());
    assert!(
        plan.windows
            .iter()
            .all(|window| window.cdt_input.source_samples.len() <= 1_089),
        "only the bounded local scope may allocate CDT samples"
    );
}

#[test]
fn terrain_cdt_adjacent_tiles_emit_identical_shared_side_vertices() {
    let terrain = TerrainSystem::with_chunking(129, 65, 1.0, 64, 0.0);
    let patch = planner_test_patch(128.0, 64.0);
    let road_loop = TerrainCdtRoadLoop::new(
        123,
        0,
        vec![
            TerrainCdtVertex::new(48.0, 1.25, 23.3),
            TerrainCdtVertex::new(80.0, 1.25, 23.3),
            TerrainCdtVertex::new(80.0, 3.75, 31.7),
            TerrainCdtVertex::new(48.0, 3.75, 31.7),
        ],
    );
    let plan = SimulationNode::terrain_cdt_window_build_inputs(
        &terrain,
        &patch,
        &[road_loop],
        2.0,
        None,
        None,
    );
    let mut shared_side_vertices = BTreeMap::new();

    for window in plan.windows {
        let min_x_mm = window.key.min_x_mm;
        let mesh = build_road_touched_terrain_patch(window.cdt_input)
            .expect("adjacent fixed CDT tiles should triangulate");
        let referenced_vertices = mesh
            .triangles
            .iter()
            .flatten()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        let mut side_vertices = mesh
            .vertices
            .iter()
            .enumerate()
            .filter(|(index, vertex)| {
                referenced_vertices.contains(index) && (vertex.x - 64.0).abs() <= 0.001
            })
            .map(|(_, vertex)| {
                (
                    SimulationNode::quantize_cdt_coord_mm(vertex.z),
                    SimulationNode::quantize_cdt_coord_mm(f64::from(vertex.height_m)),
                )
            })
            .filter(|(z_mm, _)| (0..=64_000).contains(z_mm))
            .collect::<Vec<_>>();
        side_vertices.sort_unstable();
        side_vertices.dedup();
        shared_side_vertices.insert(min_x_mm, side_vertices);
    }

    assert_eq!(
        shared_side_vertices.get(&0),
        shared_side_vertices.get(&64_000),
        "adjacent fixed tiles must emit the same complete shared-side vertex sequence"
    );
    assert!(
        shared_side_vertices.get(&0).is_some_and(|vertices| {
            vertices.contains(&(23_300, 1_250)) && vertices.contains(&(31_700, 3_750))
        }),
        "the shared side must retain non-lattice road crossings and their authored heights"
    );
}

#[test]
fn terrain_cdt_tile_keys_ignore_remote_union_ids_but_hash_local_provenance() {
    let terrain = TerrainSystem::with_chunking(65, 65, 1.0, 64, 0.0);
    let patch = planner_test_patch(64.0, 64.0);
    let vertices = vec![
        TerrainCdtVertex::new(10.0, 0.0, 24.0),
        TerrainCdtVertex::new(20.0, 0.0, 24.0),
        TerrainCdtVertex::new(20.0, 0.0, 32.0),
        TerrainCdtVertex::new(10.0, 0.0, 32.0),
    ];
    let road_loop = |stable_piece_id, footprint_group_id, building_idx| {
        let source_edges = vertices
            .iter()
            .copied()
            .enumerate()
            .map(|(index, start)| TerrainCdtRoadLoopSourceEdge {
                start,
                end: vertices[(index + 1) % vertices.len()],
                source: TerrainCdtRoadBoundarySource::BuildingSiteBoundary {
                    building_idx,
                    local_loop_index: 0,
                    local_edge_index: index as u32,
                },
            })
            .collect();
        TerrainCdtRoadLoop::new_with_source_edges_and_topology(
            stable_piece_id,
            footprint_group_id,
            0,
            false,
            vertices.clone(),
            source_edges,
        )
    };
    let original = SimulationNode::terrain_cdt_window_build_inputs(
        &terrain,
        &patch,
        &[road_loop(10, 20, 7)],
        2.0,
        None,
        None,
    );
    let remote_rekey = SimulationNode::terrain_cdt_window_build_inputs(
        &terrain,
        &patch,
        &[road_loop(999, 888, 7)],
        2.0,
        None,
        None,
    );
    let local_provenance_change = SimulationNode::terrain_cdt_window_build_inputs(
        &terrain,
        &patch,
        &[road_loop(999, 888, 8)],
        2.0,
        None,
        None,
    );

    assert_eq!(original.windows[0].key, remote_rekey.windows[0].key);
    assert_ne!(
        original.windows[0].key.fingerprint,
        local_provenance_change.windows[0].key.fingerprint
    );

    let mut changed_corner_input = original.windows[0].cdt_input.clone();
    changed_corner_input.patch.corner_heights_m[0] = 0.25;
    assert_ne!(
        original.windows[0].key.fingerprint,
        SimulationNode::terrain_cdt_input_fingerprint(&changed_corner_input),
        "tile fingerprint must include sampled terrain corners"
    );
}

#[test]
fn terrain_cdt_local_bounds_skip_margin_only_loop_outside_patch() {
    let terrain = TerrainSystem::with_chunking(8, 8, 10.0, 4, 0.0);
    let patch = TerrainPatchSnapshot {
        patch_x: 0,
        patch_z: 0,
        sample_width: 5,
        sample_height: 5,
        texture_width: 5,
        texture_height: 5,
        inner_offset_x: 0,
        inner_offset_z: 0,
        world_origin_x: 0.0,
        world_origin_z: 0.0,
        world_size_x: 40.0,
        world_size_z: 40.0,
        height_data: vec![0.0; 25],
    };
    let road_loop = TerrainCdtRoadLoop::new(
        123,
        0,
        vec![
            TerrainCdtVertex::new(60.0, 0.0, 10.0),
            TerrainCdtVertex::new(70.0, 0.0, 10.0),
            TerrainCdtVertex::new(70.0, 0.0, 20.0),
            TerrainCdtVertex::new(60.0, 0.0, 20.0),
        ],
    );

    assert!(
        SimulationNode::terrain_cdt_local_sample_bounds(&terrain, &patch, &[road_loop], 2.0)
            .is_none(),
        "a loop found only through the query margin must not create a collapsed CDT window"
    );
}

#[test]
fn terrain_cdt_input_adds_grade_limited_guides_for_grounded_standard_roads() {
    let terrain = TerrainSystem::with_chunking(8, 8, 10.0, 4, 0.0);
    let patch = TerrainPatchSnapshot {
        patch_x: 0,
        patch_z: 0,
        sample_width: 5,
        sample_height: 5,
        texture_width: 5,
        texture_height: 5,
        inner_offset_x: 0,
        inner_offset_z: 0,
        world_origin_x: 0.0,
        world_origin_z: 0.0,
        world_size_x: 40.0,
        world_size_z: 40.0,
        height_data: vec![0.0; 25],
    };
    let source = standard_span_source();
    let road = vec![
        TerrainCdtVertex::new(10.0, 3.0, 10.0),
        TerrainCdtVertex::new(30.0, 3.0, 10.0),
        TerrainCdtVertex::new(30.0, 3.0, 20.0),
        TerrainCdtVertex::new(10.0, 3.0, 20.0),
    ];
    let source_edges = road
        .iter()
        .copied()
        .enumerate()
        .map(|(index, start)| TerrainCdtRoadLoopSourceEdge {
            start,
            end: road[(index + 1) % road.len()],
            source,
        })
        .collect();
    let road_loop = TerrainCdtRoadLoop::new_with_source_edges(123, 0, road, source_edges);

    let input = SimulationNode::terrain_cdt_input_for_bounds(
        &terrain,
        &patch,
        &[road_loop],
        2.0,
        (0.0, 0.0, 40.0, 40.0),
        None,
    );

    assert!(
        input.tie_in_guide_samples.iter().any(|sample| {
            (sample.vertex.x - 10.0).abs() <= 0.001
                && (sample.vertex.z - 8.0).abs() <= 0.001
                && (sample.vertex.height_m - 2.0).abs() <= 0.001
        }),
        "grounded Standard road tie-ins should add explicit guide vertices at the slope budget"
    );
    assert!(
        input.tie_in_guide_samples.iter().any(|sample| {
            (sample.vertex.x - 10.0).abs() <= 0.001
                && (sample.vertex.z - 36.0).abs() <= 0.001
                && sample.vertex.height_m.abs() <= 0.001
        }),
        "steep grounded Standard road cuts should get a wider legal tie-in ring"
    );
    let corner_offset = 2.0_f64 / 2.0_f64.sqrt();
    assert!(
        input.tie_in_guide_samples.iter().any(|sample| {
            (sample.vertex.x - (10.0 - corner_offset)).abs() <= 0.001
                && (sample.vertex.z - (10.0 - corner_offset)).abs() <= 0.001
                && (sample.vertex.height_m - 2.0).abs() <= 0.001
        }),
        "grounded Standard road corners should get diagonal tie-in guides"
    );
    assert!(
        input.tie_in_guide_constraints.iter().any(|constraint| {
            (constraint.start.x - 10.0).abs() <= 0.001
                && (constraint.start.z - 8.0).abs() <= 0.001
                && (constraint.end.x - 12.0).abs() <= 0.001
                && (constraint.end.z - 8.0).abs() <= 0.001
        }),
        "grounded Standard road guide rings should emit constrained tie-in rails"
    );

    let mesh = build_road_touched_terrain_patch(input)
        .expect("grade-limited grounded road tie-in should triangulate");
    assert_eq!(mesh.stats.retaining_wall_faces, 0);
    assert!(mesh.retaining_wall_triangles.is_empty());
}

#[test]
fn terrain_cdt_local_bounds_expand_for_required_grading_distance() {
    let terrain = TerrainSystem::with_chunking(101, 101, 1.0, 100, 0.0);
    let patch = TerrainPatchSnapshot {
        patch_x: 0,
        patch_z: 0,
        sample_width: 101,
        sample_height: 101,
        texture_width: 101,
        texture_height: 101,
        inner_offset_x: 0,
        inner_offset_z: 0,
        world_origin_x: 0.0,
        world_origin_z: 0.0,
        world_size_x: 100.0,
        world_size_z: 100.0,
        height_data: vec![0.0; 101 * 101],
    };
    let road_loop = TerrainCdtRoadLoop::new(
        17,
        0,
        vec![
            TerrainCdtVertex::new(45.0, 16.0, 45.0),
            TerrainCdtVertex::new(55.0, 16.0, 45.0),
            TerrainCdtVertex::new(55.0, 16.0, 55.0),
            TerrainCdtVertex::new(45.0, 16.0, 55.0),
        ],
    );

    let bounds =
        SimulationNode::terrain_cdt_local_sample_bounds(&terrain, &patch, &[road_loop], 2.0)
            .expect("raised road loop should produce local CDT bounds");

    assert!(
        bounds.0 < 12.0 && bounds.1 < 12.0 && bounds.2 > 88.0 && bounds.3 > 88.0,
        "high fill should expand local CDT bounds to the required grading distance; bounds={bounds:?}"
    );
}

#[test]
fn terrain_cdt_input_keeps_multi_loop_tie_in_guides_unconstrained() {
    let terrain = TerrainSystem::with_chunking(8, 8, 10.0, 4, 0.0);
    let patch = TerrainPatchSnapshot {
        patch_x: 0,
        patch_z: 0,
        sample_width: 5,
        sample_height: 5,
        texture_width: 5,
        texture_height: 5,
        inner_offset_x: 0,
        inner_offset_z: 0,
        world_origin_x: 0.0,
        world_origin_z: 0.0,
        world_size_x: 60.0,
        world_size_z: 60.0,
        height_data: vec![0.0; 25],
    };
    let source = standard_span_source();
    let road_loops = [
        vec![
            TerrainCdtVertex::new(10.0, 3.0, 10.0),
            TerrainCdtVertex::new(24.0, 3.0, 10.0),
            TerrainCdtVertex::new(24.0, 3.0, 18.0),
            TerrainCdtVertex::new(10.0, 3.0, 18.0),
        ],
        vec![
            TerrainCdtVertex::new(32.0, 3.0, 32.0),
            TerrainCdtVertex::new(46.0, 3.0, 32.0),
            TerrainCdtVertex::new(46.0, 3.0, 40.0),
            TerrainCdtVertex::new(32.0, 3.0, 40.0),
        ],
    ]
    .into_iter()
    .enumerate()
    .map(|(loop_index, road)| {
        let source_edges = road
            .iter()
            .copied()
            .enumerate()
            .map(|(index, start)| TerrainCdtRoadLoopSourceEdge {
                start,
                end: road[(index + 1) % road.len()],
                source,
            })
            .collect();
        TerrainCdtRoadLoop::new_with_source_edges(123 + loop_index as u64, 0, road, source_edges)
    })
    .collect::<Vec<_>>();

    let input = SimulationNode::terrain_cdt_input_for_bounds(
        &terrain,
        &patch,
        &road_loops,
        2.0,
        (0.0, 0.0, 60.0, 60.0),
        None,
    );

    assert!(
        !input.tie_in_guide_samples.is_empty(),
        "multi-loop patches should still get soft guide vertices"
    );
    assert!(
        input.tie_in_guide_constraints.is_empty(),
        "multi-loop patches should not emit hard guide rails that can cross another roadbed loop"
    );
}

#[test]
fn terrain_cdt_grid_sampling_is_bounded_for_large_local_windows() {
    let small_step = SimulationNode::terrain_cdt_grid_sample_step_m(0.0, 0.0, 32.0, 32.0, 1.0);
    assert_eq!(small_step, 1.0);

    let large_step = SimulationNode::terrain_cdt_grid_sample_step_m(0.0, 0.0, 512.0, 512.0, 1.0);
    assert!(
        large_step > 1.0,
        "large CDT windows must not keep one source sample per metre across the whole area"
    );
}

#[test]
fn regular_terrain_filler_refines_cdt_window_sides() {
    let patch = TerrainPatchSnapshot {
        patch_x: 0,
        patch_z: 0,
        sample_width: 5,
        sample_height: 5,
        texture_width: 5,
        texture_height: 5,
        inner_offset_x: 0,
        inner_offset_z: 0,
        world_origin_x: 0.0,
        world_origin_z: 0.0,
        world_size_x: 40.0,
        world_size_z: 40.0,
        height_data: vec![0.0; 25],
    };
    let cdt_patch = TerrainCdtPatch::new(10.0, 10.0, 30.0, 30.0, [0.0; 4]);
    let window = SimulationNode::terrain_cdt_window_bounds(&patch, cdt_patch, 5.0).unwrap();
    let mut export = TerrainCdtTriangleBufferExport::empty();

    SimulationNode::append_regular_terrain_mesh_outside_cdt_windows(&mut export, &patch, &[window]);

    assert!(
        export_has_world_xz(&export, &patch, 10.0, 15.0),
        "regular filler must share non-corner vertical CDT-window boundary samples"
    );
    assert!(
        export_has_world_xz(&export, &patch, 15.0, 10.0),
        "regular filler must share non-corner horizontal CDT-window boundary samples"
    );
}

#[test]
fn regular_terrain_filler_splits_at_non_lattice_cdt_side_vertices() {
    let patch = TerrainPatchSnapshot {
        patch_x: 0,
        patch_z: 0,
        sample_width: 5,
        sample_height: 5,
        texture_width: 5,
        texture_height: 5,
        inner_offset_x: 0,
        inner_offset_z: 0,
        world_origin_x: 0.0,
        world_origin_z: 0.0,
        world_size_x: 40.0,
        world_size_z: 40.0,
        height_data: vec![0.0; 25],
    };
    let cdt_patch = TerrainCdtPatch::new(10.0, 10.0, 30.0, 30.0, [0.0; 4]);
    let mut window = SimulationNode::terrain_cdt_window_bounds(&patch, cdt_patch, 5.0).unwrap();
    SimulationNode::append_terrain_cdt_mesh_side_samples(
        &mut window,
        &[TerrainCdtVertex::new(10.0, 0.0, 17.3)],
    );
    let mut export = TerrainCdtTriangleBufferExport::empty();

    SimulationNode::append_regular_terrain_mesh_outside_cdt_windows(&mut export, &patch, &[window]);

    assert!(
        export_has_world_xz(&export, &patch, 10.0, 17.3),
        "regular filler must split its shared edge at every actual CDT side vertex"
    );
}

#[test]
fn regular_terrain_filler_propagates_cdt_breakpoints_across_filler_partitions() {
    let patch = TerrainPatchSnapshot {
        patch_x: 0,
        patch_z: 0,
        sample_width: 5,
        sample_height: 5,
        texture_width: 5,
        texture_height: 5,
        inner_offset_x: 0,
        inner_offset_z: 0,
        world_origin_x: 0.0,
        world_origin_z: 0.0,
        world_size_x: 40.0,
        world_size_z: 40.0,
        height_data: vec![0.0; 25],
    };
    let mut road_window = SimulationNode::terrain_cdt_window_bounds(
        &patch,
        TerrainCdtPatch::new(10.0, 10.0, 30.0, 30.0, [0.0; 4]),
        5.0,
    )
    .unwrap();
    SimulationNode::append_terrain_cdt_mesh_side_samples(
        &mut road_window,
        &[TerrainCdtVertex::new(10.0, 0.0, 17.3)],
    );
    let partitioning_window = SimulationNode::terrain_cdt_window_bounds(
        &patch,
        TerrainCdtPatch::new(5.0, 32.0, 8.0, 36.0, [0.0; 4]),
        5.0,
    )
    .unwrap();
    let mut export = TerrainCdtTriangleBufferExport::empty();

    SimulationNode::append_regular_terrain_mesh_outside_cdt_windows(
        &mut export,
        &patch,
        &[road_window, partitioning_window],
    );

    assert!(
        export_has_world_xz(&export, &patch, 5.0, 17.3),
        "a CDT side breakpoint must cross unrelated filler partitions instead of ending in a T-junction"
    );
}

#[test]
fn regular_terrain_filler_does_not_globalize_canonical_window_lattices() {
    let patch = TerrainPatchSnapshot {
        patch_x: 0,
        patch_z: 0,
        sample_width: 9,
        sample_height: 9,
        texture_width: 9,
        texture_height: 9,
        inner_offset_x: 0,
        inner_offset_z: 0,
        world_origin_x: 0.0,
        world_origin_z: 0.0,
        world_size_x: 512.0,
        world_size_z: 512.0,
        height_data: vec![0.0; 81],
    };
    let mut windows = Vec::new();
    for tile_z in 0..8 {
        for tile_x in 0..8 {
            if (tile_x + tile_z) % 2 != 0 {
                continue;
            }
            let min_x = f64::from(tile_x * 64);
            let min_z = f64::from(tile_z * 64);
            windows.push(
                SimulationNode::terrain_cdt_window_bounds(
                    &patch,
                    TerrainCdtPatch::new(min_x, min_z, min_x + 64.0, min_z + 64.0, [0.0; 4]),
                    2.0,
                )
                .expect("fixed window should intersect the test patch"),
            );
        }
    }
    let mut export = TerrainCdtTriangleBufferExport::empty();

    SimulationNode::append_regular_terrain_mesh_outside_cdt_windows(&mut export, &patch, &windows);

    assert!(!export.vertices.is_empty());
    assert!(
        export.vertices.len() < 50_000,
        "canonical 2 m side lattices must not create a patch-wide Cartesian filler grid; emitted {} vertices",
        export.vertices.len()
    );
}

#[test]
fn terrain_mesh_payload_certificate_rejects_non_finite_and_out_of_bounds_data() {
    let vertices = [
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(1.0, 0.0, 0.0),
        Vector3::new(0.0, 0.0, 1.0),
    ];
    let normals = [Vector3::UP; 3];
    let uvs = [Vector2::ZERO; 3];

    assert!(SimulationNode::triangle_mesh_buffers_are_valid(
        &vertices,
        &normals,
        &uvs,
        &[0, 1, 2],
        true,
    ));

    let mut non_finite_vertices = vertices;
    non_finite_vertices[1].x = f32::NAN;
    assert!(!SimulationNode::triangle_mesh_buffers_are_valid(
        &non_finite_vertices,
        &normals,
        &uvs,
        &[0, 1, 2],
        true,
    ));
    assert!(!SimulationNode::triangle_mesh_buffers_are_valid(
        &vertices,
        &normals,
        &uvs,
        &[0, 1, 3],
        true,
    ));
}

#[test]
fn terrain_mesh_duplicate_positions_receive_identical_normals() {
    let vertices = vec![
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(1.0, 0.0, 0.0),
        Vector3::new(0.0, 0.0, 1.0),
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(0.0, 1.0, 1.0),
        Vector3::new(-1.0, 0.0, 0.0),
    ];
    let indices = vec![0, 2, 1, 3, 5, 4];
    let mut fallback = TerrainCdtTriangleBufferExport::empty();
    fallback.vertices = vertices.clone();
    fallback.normals = vec![Vector3::ZERO; fallback.vertices.len()];
    fallback.indices = indices.clone();

    SimulationNode::reconcile_terrain_mesh_duplicate_normals(&mut fallback);

    assert_eq!(
        fallback.normals[0], fallback.normals[3],
        "duplicate seam vertices must use one deterministic accumulated normal"
    );
    assert!(fallback.normals[0].length_squared() > 0.99);

    let first_face_normal = (vertices[2] - vertices[0])
        .cross(vertices[1] - vertices[0])
        .normalized();
    let second_face_normal = (vertices[5] - vertices[3])
        .cross(vertices[4] - vertices[3])
        .normalized();
    let mut cached = TerrainCdtTriangleBufferExport::empty();
    cached.vertices = vertices;
    cached.normals = vec![
        first_face_normal,
        first_face_normal,
        first_face_normal,
        second_face_normal,
        second_face_normal,
        second_face_normal,
    ];
    cached.normal_sum_lengths = vec![1.0; cached.vertices.len()];
    cached.indices = indices;

    SimulationNode::reconcile_terrain_mesh_duplicate_normals(&mut cached);

    assert_eq!(
        cached.normals, fallback.normals,
        "cached local normal sums must preserve the triangle-walk seam result"
    );
}
