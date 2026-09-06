// SPDX-License-Identifier: GPL-2.0-only

//! Canonical source mapping and boundary validation tests.

use super::*;

#[test]
fn cdt_splits_loop_segments_through_source_vertices_before_source_mapping() {
    let source_a = test_span_boundary_source_range(
        92,
        TerrainCdtRoadBandKind::Sidewalk,
        5,
        15,
        16,
        10.0,
        11.0,
    );
    let source_b = test_span_boundary_source_range(
        92,
        TerrainCdtRoadBandKind::Sidewalk,
        5,
        16,
        17,
        11.0,
        12.0,
    );
    let source_c = test_span_boundary_source_range(
        92,
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
    let input = TerrainCdtInput::new(
        TerrainCdtPatch::new(0.0, 0.0, 10.0, 10.0, [0.0; 4]),
        vec![TerrainCdtRoadLoop::new_with_source_edges(
            92,
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
        )],
        Vec::new(),
    );

    let mesh = build_road_touched_terrain_patch(input)
        .expect("CDT road loops must split through source vertices before mapping sources");

    assert_eq!(mesh.stats.invalid_constraint_edges, 0);
    assert!(
        mesh.stats.road_constraint_edges >= 5,
        "the p0..p2 boundary segment must be split through p1"
    );
    assert!(
        mesh.emitted_faces
            .iter()
            .flat_map(|face| face.sources.iter())
            .any(|source| *source == source_a),
        "first split road boundary source must survive CDT output"
    );
    assert!(
        mesh.emitted_faces
            .iter()
            .flat_map(|face| face.sources.iter())
            .any(|source| *source == source_b),
        "second split road boundary source must survive CDT output"
    );
}

#[test]
fn cdt_keeps_source_for_submillimeter_boundary_across_identity_cells() {
    let p0 = TerrainCdtVertex::new(3.0004, 1.0, 3.0);
    let p1 = TerrainCdtVertex::new(3.0006, 1.0, 3.0);
    let p2 = TerrainCdtVertex::new(7.0, 1.0, 3.0);
    let p3 = TerrainCdtVertex::new(7.0, 1.0, 7.0);
    let p4 = TerrainCdtVertex::new(3.0004, 1.0, 7.0);
    let vertices = vec![p0, p1, p2, p3, p4];
    let source_edges = vertices
        .iter()
        .copied()
        .enumerate()
        .map(|(index, start)| TerrainCdtRoadLoopSourceEdge {
            start,
            end: vertices[(index + 1) % vertices.len()],
            source: TerrainCdtRoadBoundarySource::SyntheticTestBoundary {
                stable_piece_id: 921,
                local_loop_index: 0,
                local_edge_index: u32::try_from(index).unwrap(),
            },
        })
        .collect();
    let input = TerrainCdtInput::new(
        TerrainCdtPatch::new(0.0, 0.0, 10.0, 10.0, [0.0; 4]),
        vec![TerrainCdtRoadLoop::new_with_source_edges(
            921,
            0,
            vertices,
            source_edges,
        )],
        Vec::new(),
    );

    let mesh = build_road_touched_terrain_patch(input)
        .expect("distinct CDT vertices must retain road ownership even when less than 1 mm apart");

    assert_eq!(mesh.stats.invalid_constraint_edges, 0);
    assert_eq!(mesh.stats.merged_subbudget_seam_edges, 1);
}

#[test]
fn cdt_rejects_unsourced_road_boundary_constraints() {
    let source = test_node_boundary_source(91, TerrainCdtRoadBandKind::Sidewalk, 2);
    let road = vec![
        TerrainCdtVertex::new(3.0, 0.0, 3.0),
        TerrainCdtVertex::new(7.0, 0.0, 3.0),
        TerrainCdtVertex::new(7.0, 0.0, 7.0),
        TerrainCdtVertex::new(3.0, 0.0, 7.0),
    ];
    let input = TerrainCdtInput::new(
        TerrainCdtPatch::new(0.0, 0.0, 10.0, 10.0, [0.0; 4]),
        vec![TerrainCdtRoadLoop::new_with_source_edges(
            91,
            0,
            road.clone(),
            vec![TerrainCdtRoadLoopSourceEdge {
                start: road[0],
                end: road[1],
                source,
            }],
        )],
        Vec::new(),
    );

    assert_eq!(
        build_road_touched_terrain_patch(input),
        Err(TerrainCdtError::MissingRoadBoundarySource)
    );
}

#[test]
fn cdt_rejects_conflicting_source_split_heights() {
    let source = test_span_boundary_source(93, TerrainCdtRoadBandKind::Sidewalk, 0);
    let points = square_road_loop(2.0, 8.0, 0.0);
    let conflicting_split_a = TerrainCdtVertex::new(5.0, 1.0, 2.0);
    let conflicting_split_b = TerrainCdtVertex::new(5.0, 2.0, 2.0);
    let source_edges = vec![
        TerrainCdtRoadLoopSourceEdge {
            start: conflicting_split_a,
            end: TerrainCdtVertex::new(5.0, 1.0, 3.0),
            source,
        },
        TerrainCdtRoadLoopSourceEdge {
            start: conflicting_split_b,
            end: TerrainCdtVertex::new(5.0, 2.0, 4.0),
            source,
        },
    ];

    assert_eq!(
        split_road_loop_segments_at_source_vertices(points, &source_edges),
        Err(TerrainCdtError::ConflictingRoadBoundaryHeight)
    );
}

#[test]
fn source_edge_vertices_require_matching_loop_vertices_and_heights() {
    let points = square_road_loop(2.0, 8.0, 4.0);
    let source = TerrainCdtRoadBoundarySource::SyntheticTestBoundary {
        stable_piece_id: 7,
        local_loop_index: 2,
        local_edge_index: 0,
    };
    let mut source_edges = points
        .iter()
        .copied()
        .enumerate()
        .map(|(index, start)| TerrainCdtRoadLoopSourceEdge {
            start,
            end: points[(index + 1) % points.len()],
            source,
        })
        .collect::<Vec<_>>();

    assert!(road_loop_contains_source_edge_vertices(
        &points,
        &source_edges[1..3]
    ));

    source_edges[1].end.height_m += 1.0;
    assert!(!road_loop_contains_source_edge_vertices(
        &points,
        &source_edges[1..3]
    ));
}

#[test]
fn road_seam_height_owns_a_shared_patch_corner() {
    let patch = TerrainCdtPatch::new(0.0, 0.0, 10.0, 10.0, [0.0; 4]);
    let road = vec![
        TerrainCdtVertex::new(0.0, 4.0, 0.0),
        TerrainCdtVertex::new(5.0, 4.0, 0.0),
        TerrainCdtVertex::new(5.0, 4.0, 5.0),
        TerrainCdtVertex::new(0.0, 4.0, 5.0),
    ];

    let canonical = canonicalize_input(TerrainCdtInput::new(
        patch,
        vec![TerrainCdtRoadLoop::new(94, 0, road)],
        Vec::new(),
    ))
    .expect("an exact road seam must own the height at a shared patch corner");

    assert!(canonical.vertices.iter().any(|vertex| {
        same_coord(vertex.x, patch.min_x)
            && same_coord(vertex.z, patch.min_z)
            && same_height(vertex.height_m, 4.0)
    }));
}

#[test]
fn clip_generated_outer_and_hole_rails_keep_retained_terrain_corner_heights() {
    let patch = TerrainCdtPatch::new(0.0, 0.0, 10.0, 10.0, [0.0, 1.0, 2.0, 3.0]);
    let enclosing_loop =
        |stable_piece_id: u64, local_loop_index: u32, is_hole: bool, extent: f64, height_m: f32| {
            let vertices = vec![
                TerrainCdtVertex::new(-extent, height_m, -extent),
                TerrainCdtVertex::new(10.0 + extent, height_m, -extent),
                TerrainCdtVertex::new(10.0 + extent, height_m, 10.0 + extent),
                TerrainCdtVertex::new(-extent, height_m, 10.0 + extent),
            ];
            let source = test_node_boundary_source(
                u32::try_from(stable_piece_id).unwrap(),
                TerrainCdtRoadBandKind::Sidewalk,
                local_loop_index,
            );
            let source_edges = vertices
                .iter()
                .copied()
                .enumerate()
                .map(|(index, start)| TerrainCdtRoadLoopSourceEdge {
                    start,
                    end: vertices[(index + 1) % vertices.len()],
                    source,
                })
                .collect();
            TerrainCdtRoadLoop::new_with_source_edges_and_topology(
                stable_piece_id,
                700,
                local_loop_index,
                is_hole,
                vertices,
                source_edges,
            )
        };
    let input = TerrainCdtInput::new(
        patch,
        vec![
            enclosing_loop(701, 0, false, 10.0, 4.0),
            enclosing_loop(702, 1, true, 5.0, 6.0),
        ],
        Vec::new(),
    );

    let mesh = build_road_touched_terrain_patch(input)
        .expect("clip-generated outer and hole rails must agree on terrain-owned corner heights");

    assert!(
        !mesh.triangles.is_empty(),
        "the enclosing hole must retain terrain across the clipped core"
    );
    for expected in patch.corners_cw() {
        assert!(
            mesh.vertices.iter().any(|vertex| {
                same_xz(*vertex, expected) && same_height(vertex.height_m, expected.height_m)
            }),
            "synthetic clip rails must not replace patch terrain corner {expected:?}"
        );
    }
}

#[test]
fn cdt_rejects_conflicting_road_owned_vertex_heights() {
    let first = square_road_loop(2.0, 5.0, 0.0);
    let second = square_road_loop(5.0, 8.0, 1.0);

    assert!(matches!(
        canonicalize_input(TerrainCdtInput::new(
            TerrainCdtPatch::new(0.0, 0.0, 10.0, 10.0, [0.0; 4]),
            vec![
                TerrainCdtRoadLoop::new(95, 0, first),
                TerrainCdtRoadLoop::new(96, 0, second),
            ],
            Vec::new(),
        )),
        Err(TerrainCdtError::ConflictingRoadBoundaryHeight)
    ));
}

#[test]
fn cdt_classifies_overbudget_road_seam_faces_as_retaining_walls() {
    let road = vec![
        TerrainCdtVertex::new(4.0, 4.0, 4.0),
        TerrainCdtVertex::new(6.0, 4.0, 4.0),
        TerrainCdtVertex::new(6.0, 4.0, 6.0),
        TerrainCdtVertex::new(4.0, 4.0, 6.0),
    ];
    let input = TerrainCdtInput::new(
        TerrainCdtPatch::new(0.0, 0.0, 10.0, 10.0, [0.0; 4]),
        vec![TerrainCdtRoadLoop::new(5, 0, road.clone())],
        Vec::new(),
    );

    let mesh = build_road_touched_terrain_patch(input)
        .expect("Spade should classify over-budget road seam tie-ins");

    assert_eq!(mesh.stats.invalid_constraint_edges, 0);
    assert!(mesh.stats.road_seam_faces > 0);
    assert!(mesh.stats.retaining_wall_faces > 0);
    assert_eq!(
        mesh.stats.accepted_faces,
        mesh.triangles.len() + mesh.retaining_wall_triangles.len()
    );
    assert_eq!(
        mesh.stats.retaining_wall_faces,
        mesh.retaining_wall_triangles.len()
    );
    assert!(
        mesh.stats.retaining_wall_max_slope_ratio > MAX_TERRAIN_TIE_IN_SLOPE_RATIO,
        "retaining wall classification must be driven by the documented slope budget"
    );
    assert!(
        mesh.road_seam_face_samples
            .iter()
            .any(|sample| sample.kind == TerrainCdtTieInKind::RetainingWall)
    );
    assert!(
        mesh.retaining_wall_face_samples
            .iter()
            .all(|sample| sample.kind == TerrainCdtTieInKind::RetainingWall)
    );
    assert!(
        mesh.retaining_wall_triangles.iter().all(|triangle| {
            let center = centroid([
                mesh.vertices[triangle[0]],
                mesh.vertices[triangle[1]],
                mesh.vertices[triangle[2]],
            ]);
            !point_in_polygon(center, &road)
        }),
        "retaining walls are explicit terrain tie-ins, not emitted road-footprint faces"
    );
}
