//! End-to-end builder, source export, and tie-in diagnostic tests.

use super::*;

#[test]
fn spade_cdt_preserves_road_constraints_and_omits_road_faces() {
    let road = diagonal_road_loop();
    let input = TerrainCdtInput::new(
        TerrainCdtPatch::new(0.0, 0.0, 40.0, 40.0, [0.0; 4]),
        vec![TerrainCdtRoadLoop::new(7, 0, road.clone())],
        vec![
            TerrainCdtVertex::new(5.0, 0.0, 5.0),
            TerrainCdtVertex::new(6.0, 0.0, 30.0),
            TerrainCdtVertex::new(18.0, 0.0, 34.0),
            TerrainCdtVertex::new(20.0, 0.0, 6.0),
            TerrainCdtVertex::new(34.0, 0.0, 10.0),
            TerrainCdtVertex::new(35.0, 0.0, 35.0),
        ],
    );

    let mesh = build_road_touched_terrain_patch(input)
        .expect("Spade should triangulate a road-touched terrain patch");

    assert!(!mesh.triangles.is_empty());
    assert_eq!(mesh.stats.road_constraint_edges, 4);
    assert_eq!(mesh.stats.invalid_constraint_edges, 0);
    assert_eq!(
        mesh.stats.preserved_road_constraint_edges,
        mesh.stats.road_constraint_edges
    );
    assert!(mesh.stats.rejected_road_faces > 0);
    for triangle in &mesh.triangles {
        let center = centroid([
            mesh.vertices[triangle[0]],
            mesh.vertices[triangle[1]],
            mesh.vertices[triangle[2]],
        ]);
        assert!(
            !point_in_polygon(center, &road),
            "accepted terrain triangle leaked into the road footprint"
        );
    }
}

#[test]
fn spade_cdt_face_set_is_deterministic_for_canonical_input() {
    let input = TerrainCdtInput::new(
        TerrainCdtPatch::new(0.0, 0.0, 40.0, 40.0, [0.0; 4]),
        vec![TerrainCdtRoadLoop::new(7, 0, diagonal_road_loop())],
        vec![
            TerrainCdtVertex::new(35.0, 0.0, 35.0),
            TerrainCdtVertex::new(5.0, 0.0, 5.0),
            TerrainCdtVertex::new(34.0, 0.0, 10.0),
            TerrainCdtVertex::new(20.0, 0.0, 6.0),
            TerrainCdtVertex::new(18.0, 0.0, 34.0),
            TerrainCdtVertex::new(6.0, 0.0, 30.0),
        ],
    );

    let first = build_road_touched_terrain_patch(input.clone()).unwrap();
    let second = build_road_touched_terrain_patch(input).unwrap();

    assert_eq!(
        canonical_triangle_set(&first.triangles),
        canonical_triangle_set(&second.triangles)
    );
    assert_eq!(first.stats, second.stats);
}

#[test]
fn cdt_reports_source_samples_that_widen_road_tie_ins() {
    let road = vec![
        TerrainCdtVertex::new(3.0, 0.12, 3.0),
        TerrainCdtVertex::new(7.0, 0.12, 3.0),
        TerrainCdtVertex::new(7.0, 0.12, 7.0),
        TerrainCdtVertex::new(3.0, 0.12, 7.0),
    ];
    let input = TerrainCdtInput::new(
        TerrainCdtPatch::new(0.0, 0.0, 10.0, 10.0, [0.0; 4]),
        vec![TerrainCdtRoadLoop::new(3, 0, road)],
        vec![
            TerrainCdtVertex::new(5.0, 0.0, 2.99),
            TerrainCdtVertex::new(2.99, 0.0, 5.0),
            TerrainCdtVertex::new(7.01, 0.0, 5.0),
            TerrainCdtVertex::new(5.0, 0.0, 7.01),
        ],
    );

    let mesh = build_road_touched_terrain_patch(input)
        .expect("Spade should triangulate a raised road seam");

    assert_eq!(
        mesh.stats.input_vertices, 8,
        "near-road source samples should be omitted from the tie-in input"
    );
    assert_eq!(mesh.stats.tie_in_widened_source_samples, 4);
    assert!(mesh.stats.tie_in_widened_max_y_delta_m >= 0.12);
    assert!(mesh.stats.tie_in_widened_max_slope_ratio > MAX_TERRAIN_TIE_IN_SLOPE_RATIO);
    assert_eq!(mesh.tie_in_widened_samples.len(), 4);
    assert!(
        mesh.tie_in_widened_samples
            .iter()
            .all(|sample| sample.required_distance_m > sample.distance_m)
    );
    assert!(mesh.stats.road_seam_faces > 0);
    assert_eq!(mesh.stats.retaining_wall_faces, 0);
    assert!(mesh.retaining_wall_triangles.is_empty());
    assert!(mesh.stats.road_seam_max_y_delta_m >= 0.12);
    assert!(
        mesh.stats.road_seam_max_slope_ratio <= MAX_TERRAIN_TIE_IN_SLOPE_RATIO + 0.0001,
        "terrain tie-in should not exceed the configured slope budget; stats={:?}",
        mesh.stats
    );
    assert!(!mesh.road_seam_face_samples.is_empty());
    assert!(
        mesh.road_seam_face_samples[0].max_slope_ratio
            >= mesh.stats.road_seam_max_slope_ratio - 0.0001
    );
}

#[test]
fn cdt_omits_tie_in_guides_that_are_illegal_near_another_road_loop() {
    let source_a = test_node_boundary_source(51, TerrainCdtRoadBandKind::Sidewalk, 1);
    let source_b = test_node_boundary_source(52, TerrainCdtRoadBandKind::Sidewalk, 1);
    let road_a = vec![
        TerrainCdtVertex::new(0.0, 0.0, 3.0),
        TerrainCdtVertex::new(2.0, 0.0, 3.0),
        TerrainCdtVertex::new(2.0, 0.0, 7.0),
        TerrainCdtVertex::new(0.0, 0.0, 7.0),
    ];
    let road_b = vec![
        TerrainCdtVertex::new(12.0, 4.0, 3.0),
        TerrainCdtVertex::new(14.0, 4.0, 3.0),
        TerrainCdtVertex::new(14.0, 4.0, 7.0),
        TerrainCdtVertex::new(12.0, 4.0, 7.0),
    ];
    let legal_between_roads = TerrainCdtVertex::new(7.0, 2.0, 5.0);
    let illegal_near_b = TerrainCdtVertex::new(11.99, 0.0, 5.0);
    let input = TerrainCdtInput::new(
        TerrainCdtPatch::new(0.0, 0.0, 16.0, 10.0, [0.0; 4]),
        vec![
            sourced_road_loop(51, 0, road_a, source_a),
            sourced_road_loop(52, 0, road_b, source_b),
        ],
        Vec::new(),
    )
    .with_tie_in_guide_samples(vec![
        TerrainCdtTieInGuideSample {
            vertex: legal_between_roads,
        },
        TerrainCdtTieInGuideSample {
            vertex: illegal_near_b,
        },
    ]);

    let mesh = build_road_touched_terrain_patch(input)
        .expect("multi-loop guide filtering should triangulate");

    assert!(
        mesh.vertices
            .iter()
            .any(|vertex| same_xz(*vertex, legal_between_roads)),
        "legal guide between road loops should remain available to the CDT"
    );
    assert!(
        mesh.vertices
            .iter()
            .all(|vertex| !same_xz(*vertex, illegal_near_b)),
        "guide samples that exceed another road seam's tie-in budget must be omitted"
    );
}

#[test]
fn cdt_diagnostics_preserve_explicit_boundary_sources() {
    let source = test_node_boundary_source(42, TerrainCdtRoadBandKind::Sidewalk, 3);
    let road = vec![
        TerrainCdtVertex::new(4.0, 4.0, 4.0),
        TerrainCdtVertex::new(6.0, 4.0, 4.0),
        TerrainCdtVertex::new(6.0, 4.0, 6.0),
        TerrainCdtVertex::new(4.0, 4.0, 6.0),
    ];
    let input = TerrainCdtInput::new(
        TerrainCdtPatch::new(0.0, 0.0, 10.0, 10.0, [0.0; 4]),
        vec![sourced_road_loop(42, 0, road, source)],
        Vec::new(),
    );

    let mesh =
        build_road_touched_terrain_patch(input).expect("sourced road loop should triangulate");

    assert!(!mesh.road_seam_face_samples.is_empty());
    assert!(
        mesh.road_seam_face_samples
            .iter()
            .all(|sample| sample.sources.contains(&source)),
        "road seam diagnostics must name the explicit road boundary source"
    );
    assert!(mesh.retaining_wall_face_samples.is_empty());
    assert!(mesh.retaining_wall_triangles.is_empty());
    assert!(
        mesh.emitted_faces.iter().any(|face| {
            face.kind == TerrainCdtTieInKind::OrdinaryTerrain && face.sources.contains(&source)
        }),
        "ordinary grounded node boundary faces must preserve their road boundary source"
    );
}

#[test]
fn node_boundary_sources_keep_endpoint_provenance_in_ordering_and_merge() {
    let source_a = test_node_boundary_source_with_direct_provenance(
        42,
        TerrainCdtRoadBandKind::Sidewalk,
        3,
        30,
        31,
    );
    let source_b = test_node_boundary_source_with_direct_provenance(
        42,
        TerrainCdtRoadBandKind::Sidewalk,
        3,
        30,
        32,
    );

    assert!(terrain_cdt_boundary_source_cmp(source_a, source_b).is_lt());
    assert_eq!(
        mergeable_terrain_cdt_seam_source(source_a, source_a),
        Some(source_a)
    );
    assert_eq!(
        mergeable_terrain_cdt_seam_source(source_a, source_b),
        None,
        "node seam merging must not collapse distinct endpoint provenance"
    );
}

#[test]
fn cdt_emitted_retaining_wall_faces_preserve_boundary_sources() {
    let source = test_structural_span_boundary_source(43, TerrainCdtRoadBandKind::Sidewalk, 4);
    let road = vec![
        TerrainCdtVertex::new(4.0, 4.0, 4.0),
        TerrainCdtVertex::new(6.0, 4.0, 4.0),
        TerrainCdtVertex::new(6.0, 4.0, 6.0),
        TerrainCdtVertex::new(4.0, 4.0, 6.0),
    ];
    let input = TerrainCdtInput::new(
        TerrainCdtPatch::new(0.0, 0.0, 10.0, 10.0, [0.0; 4]),
        vec![sourced_road_loop(43, 0, road, source)],
        Vec::new(),
    );

    let mesh =
        build_road_touched_terrain_patch(input).expect("sourced road loop should triangulate");

    assert_eq!(mesh.emitted_faces.len(), mesh.stats.accepted_faces);
    assert_eq!(
        mesh.retaining_wall_triangle_sources.len(),
        mesh.retaining_wall_triangles.len()
    );
    assert!(
        !mesh.retaining_wall_triangles.is_empty(),
        "raised seam should emit explicit retaining-wall tie-in faces"
    );
    assert!(
        mesh.retaining_wall_triangle_sources
            .iter()
            .all(|sources| sources.contains(&source)),
        "emitted retaining-wall faces must carry their road boundary source"
    );
    assert!(
        mesh.emitted_faces
            .iter()
            .filter(|face| face.kind == TerrainCdtTieInKind::RetainingWall)
            .all(|face| face.sources.contains(&source)),
        "the first-class emitted-face model must preserve retaining-wall provenance"
    );
}

#[test]
fn cdt_standard_grounded_span_sources_do_not_emit_retaining_walls() {
    let source = test_span_boundary_source(45, TerrainCdtRoadBandKind::Sidewalk, 4);
    let road = vec![
        TerrainCdtVertex::new(4.0, 4.0, 4.0),
        TerrainCdtVertex::new(6.0, 4.0, 4.0),
        TerrainCdtVertex::new(6.0, 4.0, 6.0),
        TerrainCdtVertex::new(4.0, 4.0, 6.0),
    ];
    let input = TerrainCdtInput::new(
        TerrainCdtPatch::new(0.0, 0.0, 10.0, 10.0, [0.0; 4]),
        vec![sourced_road_loop(45, 0, road, source)],
        Vec::new(),
    );

    let mesh =
        build_road_touched_terrain_patch(input).expect("sourced road loop should triangulate");

    assert_eq!(mesh.stats.retaining_wall_faces, 0);
    assert!(mesh.retaining_wall_triangles.is_empty());
    assert!(
        mesh.emitted_faces.iter().any(|face| {
            face.kind == TerrainCdtTieInKind::OrdinaryTerrain && face.sources.contains(&source)
        }),
        "grounded Standard span seams must preserve provenance through ordinary terrain faces"
    );
}

#[test]
fn cdt_emitted_road_seam_terrain_faces_preserve_boundary_sources() {
    let source = test_node_boundary_source(44, TerrainCdtRoadBandKind::CurbOrShoulder, 5);
    let road = vec![
        TerrainCdtVertex::new(3.0, 0.12, 3.0),
        TerrainCdtVertex::new(7.0, 0.12, 3.0),
        TerrainCdtVertex::new(7.0, 0.12, 7.0),
        TerrainCdtVertex::new(3.0, 0.12, 7.0),
    ];
    let input = TerrainCdtInput::new(
        TerrainCdtPatch::new(0.0, 0.0, 10.0, 10.0, [0.0; 4]),
        vec![sourced_road_loop(44, 0, road, source)],
        vec![
            TerrainCdtVertex::new(5.0, 0.0, 2.99),
            TerrainCdtVertex::new(2.99, 0.0, 5.0),
            TerrainCdtVertex::new(7.01, 0.0, 5.0),
            TerrainCdtVertex::new(5.0, 0.0, 7.01),
        ],
    );

    let mesh =
        build_road_touched_terrain_patch(input).expect("sourced road loop should triangulate");

    assert_eq!(mesh.terrain_triangle_sources.len(), mesh.triangles.len());
    assert!(mesh.retaining_wall_triangles.is_empty());
    assert!(
        mesh.terrain_triangle_sources
            .iter()
            .any(|sources| sources.contains(&source)),
        "accepted road-seam terrain faces must carry their road boundary source"
    );
    assert!(
        mesh.emitted_faces.iter().any(|face| {
            face.kind == TerrainCdtTieInKind::OrdinaryTerrain && face.sources.contains(&source)
        }),
        "the first-class emitted-face model must preserve ordinary seam provenance"
    );
}

#[test]
fn cdt_emitted_non_road_terrain_faces_may_be_source_empty() {
    let input = TerrainCdtInput::new(
        TerrainCdtPatch::new(0.0, 0.0, 10.0, 10.0, [0.0; 4]),
        Vec::new(),
        Vec::new(),
    );

    let mesh = build_road_touched_terrain_patch(input)
        .expect("plain terrain patch should triangulate without road sources");

    assert!(!mesh.triangles.is_empty());
    assert_eq!(mesh.terrain_triangle_sources.len(), mesh.triangles.len());
    assert!(mesh.terrain_triangle_sources.iter().all(Vec::is_empty));
    assert!(
        mesh.emitted_faces
            .iter()
            .all(|face| face.sources.is_empty())
    );
}

#[test]
fn cdt_tie_in_widening_preserves_closest_seam_source() {
    let source = test_node_boundary_source(77, TerrainCdtRoadBandKind::CurbOrShoulder, 1);
    let road = vec![
        TerrainCdtVertex::new(3.0, 0.12, 3.0),
        TerrainCdtVertex::new(7.0, 0.12, 3.0),
        TerrainCdtVertex::new(7.0, 0.12, 7.0),
        TerrainCdtVertex::new(3.0, 0.12, 7.0),
    ];
    let input = TerrainCdtInput::new(
        TerrainCdtPatch::new(0.0, 0.0, 10.0, 10.0, [0.0; 4]),
        vec![sourced_road_loop(77, 0, road, source)],
        vec![TerrainCdtVertex::new(5.0, 0.0, 2.99)],
    );

    let mesh = build_road_touched_terrain_patch(input)
        .expect("sourced tie-in widening case should triangulate");

    assert_eq!(mesh.tie_in_widened_samples.len(), 1);
    assert_eq!(mesh.tie_in_widened_samples[0].seam_source, source);
}

#[test]
fn cdt_tie_in_widening_ties_choose_seam_geometry_before_source_identity() {
    let patch = TerrainCdtPatch::new(0.0, 0.0, 10.0, 10.0, [0.0; 4]);
    let horizontal_source = test_node_boundary_source(88, TerrainCdtRoadBandKind::Sidewalk, 1);
    let vertical_source = test_node_boundary_source(88, TerrainCdtRoadBandKind::Sidewalk, 2);
    let source_samples = vec![TerrainCdtVertex::new(5.0, 0.0, 5.0)];

    let first = build_road_touched_terrain_patch(TerrainCdtInput::new(
        patch,
        vec![sourced_l_road_loop_with_notch_sources(
            horizontal_source,
            vertical_source,
        )],
        source_samples.clone(),
    ))
    .expect("first sourced L road loop should triangulate");
    let second = build_road_touched_terrain_patch(TerrainCdtInput::new(
        patch,
        vec![sourced_l_road_loop_with_notch_sources(
            vertical_source,
            horizontal_source,
        )],
        source_samples,
    ))
    .expect("reordered sourced L road loop should triangulate");

    assert_eq!(
        first.stats, second.stats,
        "source identity order must not change tie-in widening diagnostics"
    );
    assert_eq!(first.tie_in_widened_samples.len(), 1);
    assert_eq!(second.tie_in_widened_samples.len(), 1);
    assert_eq!(
        first.tie_in_widened_samples[0].seam_point, second.tie_in_widened_samples[0].seam_point,
        "equidistant seam candidates must choose by geometry before provenance"
    );
    assert!(same_coord(
        first.tie_in_widened_samples[0].seam_point.x,
        4.0
    ));
    assert!(same_coord(
        first.tie_in_widened_samples[0].seam_point.z,
        5.0
    ));
}

#[test]
fn cdt_bridge_omitted_tie_in_source_does_not_promote_grade_compliant_faces() {
    let source = test_structural_span_boundary_source(1, TerrainCdtRoadBandKind::Sidewalk, 0);
    let input = high_delta_omitted_tie_in_input(source);

    let mesh = build_road_touched_terrain_patch(input)
        .expect("high-delta bridge tie-in widening should triangulate");

    assert_eq!(mesh.stats.tie_in_widened_source_samples, 1);
    assert_eq!(
        mesh.stats.retaining_wall_faces, 0,
        "one omitted bridge sample must not promote every grade-compliant abutment face"
    );
    let sourced_faces = mesh
        .emitted_faces
        .iter()
        .filter(|face| face.sources.contains(&source))
        .collect::<Vec<_>>();
    assert!(
        !sourced_faces.is_empty(),
        "bridge regression must exercise emitted faces carrying the structural source"
    );
    assert!(
        sourced_faces
            .iter()
            .all(|face| face.kind == TerrainCdtTieInKind::OrdinaryTerrain),
        "grade-compliant bridge-abutment faces must remain on the terrain material path"
    );
}

#[test]
fn cdt_promotes_high_delta_omitted_tunnel_source_to_retaining_wall() {
    let source = test_tunnel_span_boundary_source(1, TerrainCdtRoadBandKind::Sidewalk, 0);
    let input = high_delta_omitted_tie_in_input(source);

    let mesh = build_road_touched_terrain_patch(input)
        .expect("high-delta tunnel tie-in widening should triangulate");

    assert_eq!(mesh.stats.tie_in_widened_source_samples, 1);
    assert!(mesh.stats.retaining_wall_faces > 0);
    assert!(
        mesh.retaining_wall_triangle_sources
            .iter()
            .any(|sources| sources.contains(&source)),
        "source-required retaining walls must preserve the omitted tunnel portal source"
    );
    assert!(
        mesh.emitted_faces.iter().any(|face| {
            face.kind == TerrainCdtTieInKind::RetainingWall && face.sources.contains(&source)
        }),
        "first-class emitted retaining-wall faces must carry the required tunnel source"
    );
}

fn high_delta_omitted_tie_in_input(source: TerrainCdtRoadBoundarySource) -> TerrainCdtInput {
    let road = vec![
        TerrainCdtVertex::new(48.0, 1.2, 48.0),
        TerrainCdtVertex::new(52.0, 1.2, 48.0),
        TerrainCdtVertex::new(52.0, 1.2, 52.0),
        TerrainCdtVertex::new(48.0, 1.2, 52.0),
    ];
    TerrainCdtInput::new(
        TerrainCdtPatch::new(0.0, 0.0, 100.0, 100.0, [0.0; 4]),
        vec![sourced_road_loop(1, 0, road, source)],
        vec![TerrainCdtVertex::new(50.0, 0.0, 47.99)],
    )
}
