//! Patch clipping and piece-footprint preservation tests.

use super::*;

#[test]
fn concave_loop_rectangle_clip_preserves_disconnected_components() {
    let road_loop = disconnected_clip_test_loop();
    let patch = TerrainCdtPatch::new(-1.0, 4.0, 11.0, 9.0, [0.0; 4]);

    let first = clip_terrain_cdt_road_loop_to_patch(&road_loop, patch);
    let second = clip_terrain_cdt_road_loop_to_patch(&road_loop, patch);

    assert_eq!(first, second, "component ordering must be deterministic");
    assert_eq!(
        first.len(),
        2,
        "the clipped U arms must remain disconnected"
    );
    let bounds = first
        .iter()
        .map(|component| terrain_cdt_loop_bounds(&component.vertices))
        .collect::<Vec<_>>();
    assert_eq!(
        bounds
            .iter()
            .map(|bounds| {
                (
                    quantized_coord(bounds.min_x),
                    quantized_coord(bounds.min_z),
                    quantized_coord(bounds.max_x),
                    quantized_coord(bounds.max_z),
                )
            })
            .collect::<Vec<_>>(),
        vec![(0, 4_000, 3_000, 9_000), (7_000, 4_000, 10_000, 9_000)]
    );
    assert!(
        first
            .iter()
            .all(|component| component.source_edges.len() == 2),
        "each arm must retain only its own two source-owned vertical sides"
    );
    let source_edge_indices = first
        .iter()
        .map(|component| {
            component
                .source_edges
                .iter()
                .filter_map(|edge| match edge.source {
                    TerrainCdtRoadBoundarySource::SyntheticTestBoundary {
                        local_edge_index,
                        ..
                    } => Some(local_edge_index),
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        source_edge_indices,
        vec![vec![5, 7], vec![1, 3]],
        "disconnected components must not inherit provenance from the other arm"
    );
    assert!(
        first
            .iter()
            .all(|component| !point_in_polygon(test_vertex(5.0, 6.0), &component.vertices)),
        "clipping must not synthesize a road-filled bridge across the U opening"
    );
}

#[test]
fn canonicalizer_preserves_disconnected_rectangle_intersection_components() {
    let patch = TerrainCdtPatch::new(-1.0, 4.0, 11.0, 9.0, [0.0; 4]);
    let canonical = canonicalize_input(TerrainCdtInput::new(
        patch,
        vec![disconnected_clip_test_loop()],
        Vec::new(),
    ))
    .expect("disconnected clipped components should remain valid CDT constraints");

    assert_eq!(
        canonical.road_loops.len(),
        2,
        "the final CDT-core clip must not reconnect disconnected U arms"
    );
    assert!(
        canonical
            .road_loops
            .iter()
            .all(|component| !point_in_polygon(test_vertex(5.0, 6.0), &component.vertices)),
        "the canonical road ownership loops must leave the U opening as terrain"
    );
}

#[test]
fn submillimetre_curved_edge_intersection_retains_exact_boundary_source() {
    let patch = TerrainCdtPatch::new(-16.0, -43.0, -15.0, -41.0, [0.0; 4]);
    let road_loop = TerrainCdtRoadLoop::new(
        42,
        0,
        vec![
            TerrainCdtVertex::new(-16.451_802, 0.0, -42.235_62),
            TerrainCdtVertex::new(-15.841_945, 0.0, -42.090_879),
            TerrainCdtVertex::new(-15.841_945, 0.0, -41.5),
            TerrainCdtVertex::new(-16.451_802, 0.0, -41.5),
        ],
    );

    let clipped = clip_terrain_cdt_road_loop_to_patch(&road_loop, patch);

    assert_eq!(clipped.len(), 1);
    assert!(
        clipped[0].source_edges.iter().any(|edge| {
            matches!(
                edge.source,
                TerrainCdtRoadBoundarySource::SyntheticTestBoundary {
                    local_edge_index: 0,
                    ..
                }
            )
        }),
        "the exact curved edge source must survive an overlay intersection that rounds into the adjacent 1 mm bin"
    );
    canonicalize_input(TerrainCdtInput::new(patch, clipped, Vec::new()))
        .expect("the exact clipped source must cover every non-rail road constraint");
}

#[test]
fn clipping_preserves_multiple_sources_that_partition_one_output_edge() {
    let source_a = test_span_boundary_source_range(
        93,
        TerrainCdtRoadBandKind::Sidewalk,
        5,
        15,
        16,
        10.0,
        11.0,
    );
    let source_b = test_span_boundary_source_range(
        93,
        TerrainCdtRoadBandKind::Sidewalk,
        5,
        16,
        17,
        11.0,
        12.0,
    );
    let source_c = test_span_boundary_source_range(
        93,
        TerrainCdtRoadBandKind::Sidewalk,
        5,
        17,
        18,
        12.0,
        13.0,
    );
    let p0 = TerrainCdtVertex::new(3.0, 1.0, 3.0);
    let p1 = TerrainCdtVertex::new(5.0, 1.0, 3.0);
    let p2 = TerrainCdtVertex::new(7.0, 1.0, 3.0);
    let p3 = TerrainCdtVertex::new(7.0, 1.0, 7.0);
    let p4 = TerrainCdtVertex::new(3.0, 1.0, 7.0);
    let road_loop = TerrainCdtRoadLoop::new_with_source_edges(
        93,
        0,
        vec![p0, p2, p3, p4],
        vec![
            TerrainCdtRoadLoopSourceEdge {
                start: p0,
                end: p1,
                source: source_a,
            },
            TerrainCdtRoadLoopSourceEdge {
                start: p1,
                end: p2,
                source: source_b,
            },
            TerrainCdtRoadLoopSourceEdge {
                start: p2,
                end: p3,
                source: source_c,
            },
            TerrainCdtRoadLoopSourceEdge {
                start: p3,
                end: p4,
                source: source_c,
            },
            TerrainCdtRoadLoopSourceEdge {
                start: p4,
                end: p0,
                source: source_c,
            },
        ],
    );
    let patch = TerrainCdtPatch::new(4.0, 0.0, 10.0, 10.0, [0.0; 4]);

    let clipped = clip_terrain_cdt_road_loop_to_patch(&road_loop, patch);

    assert_eq!(clipped.len(), 1);
    assert!(
        clipped[0]
            .source_edges
            .iter()
            .any(|edge| edge.source == source_a),
        "the first source partition must survive a spanning clipped edge"
    );
    assert!(
        clipped[0]
            .source_edges
            .iter()
            .any(|edge| edge.source == source_b),
        "the second source partition must survive a spanning clipped edge"
    );
    build_road_touched_terrain_patch(TerrainCdtInput::new(patch, clipped, Vec::new()))
        .expect("source-partitioned clipped loops must retain every interior boundary source");
}

#[test]
fn default_patch_scale_short_source_partition_preserves_final_provenance() {
    let short_source = test_span_boundary_source_range(
        94,
        TerrainCdtRoadBandKind::Sidewalk,
        5,
        15,
        16,
        10.0,
        10.1,
    );
    let long_source = test_span_boundary_source_range(
        94,
        TerrainCdtRoadBandKind::Sidewalk,
        5,
        16,
        17,
        10.1,
        520.0,
    );
    let remaining_source = test_span_boundary_source_range(
        94,
        TerrainCdtRoadBandKind::Sidewalk,
        5,
        17,
        18,
        520.0,
        540.0,
    );
    let point_contact_source = test_span_boundary_source_range(
        94,
        TerrainCdtRoadBandKind::Sidewalk,
        6,
        18,
        19,
        540.0,
        541.0,
    );
    let bottom_left = TerrainCdtVertex::new(-1.0, 1.0, 100.0);
    let short_end = TerrainCdtVertex::new(0.1, 1.0, 100.0);
    let bottom_right = TerrainCdtVertex::new(511.0, 1.0, 100.0);
    let top_right = TerrainCdtVertex::new(511.0, 1.0, 110.0);
    let top_left = TerrainCdtVertex::new(-1.0, 1.0, 110.0);
    let road_loop = TerrainCdtRoadLoop::new_with_source_edges(
        94,
        0,
        vec![bottom_left, bottom_right, top_right, top_left],
        vec![
            TerrainCdtRoadLoopSourceEdge {
                start: bottom_left,
                end: short_end,
                source: short_source,
            },
            TerrainCdtRoadLoopSourceEdge {
                start: short_end,
                end: bottom_right,
                source: long_source,
            },
            TerrainCdtRoadLoopSourceEdge {
                start: bottom_right,
                end: top_right,
                source: remaining_source,
            },
            TerrainCdtRoadLoopSourceEdge {
                start: top_right,
                end: top_left,
                source: remaining_source,
            },
            TerrainCdtRoadLoopSourceEdge {
                start: top_left,
                end: bottom_left,
                source: remaining_source,
            },
            TerrainCdtRoadLoopSourceEdge {
                start: TerrainCdtVertex::new(250.0, 1.0, 99.0),
                end: TerrainCdtVertex::new(250.0, 1.0, 100.0),
                source: point_contact_source,
            },
        ],
    );
    let patch = TerrainCdtPatch::new(0.0, 0.0, 510.0, 510.0, [0.0; 4]);

    let clipped = clip_terrain_cdt_road_loop_to_patch(&road_loop, patch);

    assert_eq!(clipped.len(), 1);
    assert!(
        clipped[0]
            .source_edges
            .iter()
            .any(|edge| edge.source == short_source),
        "a 10 cm source partition must survive clipping against a 510 m patch edge"
    );
    assert!(
        clipped[0]
            .source_edges
            .iter()
            .all(|edge| edge.source != point_contact_source),
        "a source touching the output boundary at only one point must not be inherited"
    );
    let mesh = build_road_touched_terrain_patch(TerrainCdtInput::new(patch, clipped, Vec::new()))
        .expect("metric source splitting must preserve a short source through final CDT output");
    assert!(
        mesh.emitted_faces
            .iter()
            .flat_map(|face| face.sources.iter())
            .any(|source| *source == short_source),
        "the short clipped source must remain attached to final CDT provenance"
    );
}

fn disconnected_clip_test_loop() -> TerrainCdtRoadLoop {
    TerrainCdtRoadLoop::new(
        41,
        0,
        vec![
            test_vertex(0.0, 0.0),
            test_vertex(10.0, 0.0),
            test_vertex(10.0, 10.0),
            test_vertex(7.0, 10.0),
            test_vertex(7.0, 3.0),
            test_vertex(3.0, 3.0),
            test_vertex(3.0, 10.0),
            test_vertex(0.0, 10.0),
        ],
    )
}

#[test]
fn sourced_patch_edge_matrix_preserves_sources_after_clipping() {
    let patch = TerrainCdtPatch::new(0.0, 0.0, 40.0, 40.0, [0.0; 4]);
    let source = test_span_boundary_source(201, TerrainCdtRoadBandKind::CurbOrShoulder, 2);
    let cases = [
        (
            "road footprint crossing one patch edge",
            road_loop_from_centerline(
                TerrainCdtVertex::new(-10.0, 0.0, 20.0),
                TerrainCdtVertex::new(20.0, 0.0, 20.0),
                6.0,
            ),
        ),
        (
            "road footprint crossing two patch edges",
            road_loop_from_centerline(
                TerrainCdtVertex::new(-10.0, 0.0, 20.0),
                TerrainCdtVertex::new(50.0, 0.0, 20.0),
                6.0,
            ),
        ),
        (
            "road footprint crossing a patch corner",
            road_loop_from_centerline(
                TerrainCdtVertex::new(-10.0, 0.0, -10.0),
                TerrainCdtVertex::new(20.0, 0.0, 20.0),
                6.0,
            ),
        ),
    ];

    for (case_name, road) in cases {
        let mesh = build_road_touched_terrain_patch(TerrainCdtInput::new(
            patch,
            vec![sourced_road_loop(201, 0, road.clone(), source)],
            piece_source_samples(),
        ))
        .unwrap_or_else(|_| panic!("{case_name}: clipped terrain CDT should build"));

        assert_sourced_road_touched_mesh_contract(case_name, &mesh, patch, &[road], source);
    }
}

#[test]
fn road_loop_crossing_one_patch_edge_is_clipped_to_shared_boundary_vertices() {
    let patch = TerrainCdtPatch::new(0.0, 0.0, 40.0, 40.0, [0.0; 4]);
    let road = road_loop_from_centerline(
        TerrainCdtVertex::new(-10.0, 0.0, 20.0),
        TerrainCdtVertex::new(20.0, 0.0, 20.0),
        6.0,
    );

    let mesh = build_crossing_patch(patch, road.clone());
    assert_valid_clipped_mesh(&mesh, patch, &road);
    assert!(
        mesh.vertices
            .iter()
            .any(|vertex| same_coord(vertex.x, patch.min_x))
    );
}

#[test]
fn road_loop_patch_clipping_preserves_boundary_sources() {
    let patch = TerrainCdtPatch::new(0.0, 0.0, 40.0, 40.0, [0.0; 4]);
    let road = road_loop_from_centerline(
        TerrainCdtVertex::new(-10.0, 0.0, 20.0),
        TerrainCdtVertex::new(20.0, 0.0, 20.0),
        6.0,
    );
    let source = test_node_boundary_source(88, TerrainCdtRoadBandKind::Sidewalk, 5);

    let mesh = build_road_touched_terrain_patch(TerrainCdtInput::new(
        patch,
        vec![sourced_road_loop(88, 0, road.clone(), source)],
        Vec::new(),
    ))
    .expect("source-preserving clipped road loop should triangulate");

    assert_valid_clipped_mesh(&mesh, patch, &road);
    assert!(
        !mesh.road_seam_face_samples.is_empty(),
        "clipped sourced road loop should still report seam diagnostics"
    );
    assert!(
        mesh.road_seam_face_samples
            .iter()
            .all(|sample| sample.sources.contains(&source)),
        "patch-clipped road seam constraints must inherit their original source edge"
    );
}

#[test]
fn road_loop_crossing_two_patch_edges_splits_both_patch_boundary_constraints() {
    let patch = TerrainCdtPatch::new(0.0, 0.0, 40.0, 40.0, [0.0; 4]);
    let road = road_loop_from_centerline(
        TerrainCdtVertex::new(-10.0, 0.0, 20.0),
        TerrainCdtVertex::new(50.0, 0.0, 20.0),
        6.0,
    );

    let mesh = build_crossing_patch(patch, road.clone());
    assert_valid_clipped_mesh(&mesh, patch, &road);
    assert!(
        mesh.vertices
            .iter()
            .any(|vertex| same_coord(vertex.x, patch.min_x))
    );
    assert!(
        mesh.vertices
            .iter()
            .any(|vertex| same_coord(vertex.x, patch.max_x))
    );
}

#[test]
fn road_loop_crossing_patch_corner_uses_corner_as_constraint_endpoint() {
    let patch = TerrainCdtPatch::new(0.0, 0.0, 40.0, 40.0, [0.0; 4]);
    let road = road_loop_from_centerline(
        TerrainCdtVertex::new(-10.0, 0.0, -10.0),
        TerrainCdtVertex::new(20.0, 0.0, 20.0),
        6.0,
    );

    let mesh = build_crossing_patch(patch, road.clone());
    assert_valid_clipped_mesh(&mesh, patch, &road);
    assert!(
        mesh.vertices
            .iter()
            .any(|vertex| same_coord(vertex.x, patch.min_x) && same_coord(vertex.z, patch.min_z))
    );
}

#[test]
fn multiple_road_loops_in_one_patch_preserve_all_seam_constraints_deterministically() {
    let patch = TerrainCdtPatch::new(0.0, 0.0, 40.0, 40.0, [0.0; 4]);
    let road_a = road_loop_from_centerline(
        TerrainCdtVertex::new(8.0, 0.0, 10.0),
        TerrainCdtVertex::new(18.0, 0.0, 18.0),
        4.0,
    );
    let road_b = road_loop_from_centerline(
        TerrainCdtVertex::new(22.0, 0.0, 28.0),
        TerrainCdtVertex::new(34.0, 0.0, 28.0),
        4.0,
    );
    let input = TerrainCdtInput::new(
        patch,
        vec![
            TerrainCdtRoadLoop::new(99, 0, road_b.clone()),
            TerrainCdtRoadLoop::new(7, 0, road_a.clone()),
        ],
        vec![
            TerrainCdtVertex::new(5.0, 0.0, 5.0),
            TerrainCdtVertex::new(5.0, 0.0, 35.0),
            TerrainCdtVertex::new(20.0, 0.0, 5.0),
            TerrainCdtVertex::new(20.0, 0.0, 35.0),
            TerrainCdtVertex::new(35.0, 0.0, 5.0),
            TerrainCdtVertex::new(35.0, 0.0, 35.0),
        ],
    );

    let first = build_road_touched_terrain_patch(input.clone())
        .expect("Spade should triangulate multiple road loops");
    let second = build_road_touched_terrain_patch(input)
        .expect("Spade should deterministically triangulate multiple road loops");

    assert_eq!(first.stats.road_constraint_edges, 8);
    assert_eq!(first.stats.invalid_constraint_edges, 0);
    assert_eq!(
        first.stats.preserved_road_constraint_edges,
        first.stats.road_constraint_edges
    );
    assert_eq!(
        canonical_triangle_set(&first.triangles),
        canonical_triangle_set(&second.triangles)
    );
    for triangle in &first.triangles {
        let center = centroid([
            first.vertices[triangle[0]],
            first.vertices[triangle[1]],
            first.vertices[triangle[2]],
        ]);
        assert!(!point_in_polygon(center, &road_a));
        assert!(!point_in_polygon(center, &road_b));
    }
}

#[test]
fn bend_footprint_loop_preserves_piece_owned_constraints() {
    let patch = piece_test_patch();
    let road = vec![
        test_vertex(10.0, 10.0),
        test_vertex(26.0, 10.0),
        test_vertex(26.0, 20.0),
        test_vertex(42.0, 20.0),
        test_vertex(42.0, 34.0),
        test_vertex(10.0, 34.0),
    ];

    let mesh = build_piece_patch(patch, 11, road.clone());

    assert_valid_piece_footprint_mesh(&mesh, patch, &road);
}

#[test]
fn terminal_footprint_loop_preserves_piece_owned_constraints() {
    let patch = piece_test_patch();
    let road = vec![
        test_vertex(22.0, 8.0),
        test_vertex(38.0, 8.0),
        test_vertex(38.0, 36.0),
        test_vertex(44.0, 40.0),
        test_vertex(38.0, 44.0),
        test_vertex(22.0, 44.0),
        test_vertex(16.0, 40.0),
        test_vertex(22.0, 36.0),
    ];

    let mesh = build_piece_patch(patch, 12, road.clone());

    assert_valid_piece_footprint_mesh(&mesh, patch, &road);
}

#[test]
fn junction_n_footprint_loop_preserves_piece_owned_constraints() {
    let patch = piece_test_patch();
    let road = vec![
        test_vertex(24.0, 4.0),
        test_vertex(36.0, 4.0),
        test_vertex(36.0, 24.0),
        test_vertex(56.0, 24.0),
        test_vertex(56.0, 36.0),
        test_vertex(36.0, 36.0),
        test_vertex(36.0, 56.0),
        test_vertex(24.0, 56.0),
        test_vertex(24.0, 36.0),
        test_vertex(4.0, 36.0),
        test_vertex(4.0, 24.0),
        test_vertex(24.0, 24.0),
    ];

    let first = build_piece_patch(patch, 13, road.clone());
    let second = build_piece_patch(patch, 13, road.clone());

    assert_valid_piece_footprint_mesh(&first, patch, &road);
    assert_eq!(
        canonical_triangle_set(&first.triangles),
        canonical_triangle_set(&second.triangles)
    );
    assert_eq!(first.stats, second.stats);
}
