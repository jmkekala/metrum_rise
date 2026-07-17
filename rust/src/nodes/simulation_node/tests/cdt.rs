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
        input_source_samples: 0,
        cdt_patch: TerrainCdtPatch::new(
            min_x_mm as f64 / 1_000.0,
            0.0,
            min_x_mm as f64 / 1_000.0 + 1.0,
            1.0,
            [0.0; 4],
        ),
        mesh_result,
        cdt_ms: 0.0,
        reused: false,
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
fn terrain_cdt_constraint_conflicts_include_unpreserved_road_edges() {
    let mut stats = empty_cdt_stats();
    stats.road_constraint_edges = 10;
    stats.preserved_road_constraint_edges = 9;
    stats.invalid_constraint_edges = 0;

    assert!(SimulationNode::terrain_cdt_stats_have_constraint_conflicts(
        stats
    ));

    stats.preserved_road_constraint_edges = 10;
    assert!(!SimulationNode::terrain_cdt_stats_have_constraint_conflicts(stats));
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
        test_cached_cdt_window(0, Ok(empty_test_cdt_mesh())),
        test_cached_cdt_window(2_000, Err(TerrainCdtError::InvalidPatch)),
    ];

    assert_eq!(
        SimulationNode::cached_refined_cdt_failure_label(&cached),
        Some("invalid_patch")
    );
}

#[test]
fn road_locked_refined_patch_rejects_site_only_clip_payload() {
    assert_eq!(
        SimulationNode::terrain_clip_input_failure_label(true, 1, 0, 0, 1, 1, None,),
        Some("missing_road_clip_sources")
    );
    assert_eq!(
        SimulationNode::terrain_clip_input_failure_label(false, 1, 0, 0, 1, 1, None,),
        None,
        "site-only patches remain valid when no grounded road owns the patch"
    );
}

#[test]
fn refined_patch_rejects_dropped_clip_loop_before_cdt() {
    assert_eq!(
        SimulationNode::terrain_clip_input_failure_label(true, 2, 1, 1, 1, 1, None,),
        Some("incomplete_terrain_clip_windows")
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
            .any(|sample| sample.x == 3.0 && sample.z == 9.0),
        "local CDT windows must seed non-corner vertices along arbitrary vertical boundaries"
    );
    assert!(
        input
            .source_samples
            .iter()
            .any(|sample| sample.x == 18.0 && sample.z == 29.0),
        "local CDT windows must seed non-corner vertices along arbitrary horizontal boundaries"
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
