//! Patch clipping and piece-footprint preservation tests.

use super::*;

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
