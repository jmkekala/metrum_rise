// SPDX-License-Identifier: GPL-2.0-only

//! Span compilation and span-owned surface regression tests.

use super::*;

#[test]
fn span_raised_step_generation_uses_resolved_regions() {
    let span_source = include_str!("../span.rs");
    for forbidden in [
        "curb_vertical_face_polygon_for_section_pair",
        "curb_vertical_face",
        "curb_asphalt_boundary",
        "compile_surface_polygons_for_ranges",
        "compile_span_explicit_vertical_step_faces_for_ranges",
        "SpanExplicitVerticalStepBoundary",
    ] {
        assert!(
            !span_source.contains(forbidden),
            "span output must consume resolved regions and generic raised-step constraints, not legacy section-window helper `{forbidden}`"
        );
    }
    assert!(
        span_source.contains("resolve_span_regions_for_ranges")
            && span_source.contains("span_raised_step_faces_from_constraints"),
        "span output must route through resolved regions and raised-step constraints"
    );
}

#[test]
fn span_vertical_steps_include_carriageway_sidewalk_boundaries_when_profile_has_no_curb() {
    let mut graph = RegionGraph::new();
    let a = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let b = graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        a,
        b,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
        5.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR,
    ));
    let section_at = |s_m: f32| RoadSurfaceSection {
        edge_idx,
        s_m,
        center_xz: backend::RoadVec2::new(f64::from(s_m), 0.0),
        center_height_m: 0.0,
        tangent_xz: backend::RoadVec2::new(1.0, 0.0),
        lateral_xz: backend::RoadVec2::new(0.0, 1.0),
        bands: vec![
            RoadSurfaceBand {
                kind: RoadSurfaceBandKind::Carriageway,
                lateral_start_m: -3.0,
                lateral_end_m: 0.0,
                height_start_m: 0.0,
                height_end_m: 0.0,
            },
            RoadSurfaceBand {
                kind: RoadSurfaceBandKind::Sidewalk,
                lateral_start_m: 0.0,
                lateral_end_m: 2.0,
                height_start_m: CURB_STEP_HEIGHT_M,
                height_end_m: CURB_STEP_HEIGHT_M,
            },
        ],
    };
    let sections = vec![
        section_at(0.0),
        section_at(8.0),
        section_at(12.0),
        section_at(20.0),
    ];
    let mut surface = RoadSurfaceSystem::new(64.0);
    surface
        .compiled_sections
        .insert(edge_idx, std::sync::Arc::new(sections));

    let span_piece = surface
        .compile_visual_span_piece(&graph, &flat_terrain(32, 32), edge_idx)
        .expect("direct carriageway-sidewalk span should compile");
    assert!(!span_piece.raised_step_face_polygons.is_empty());
    assert!(
        span_piece.raised_step_face_polygons.iter().any(|face| {
            face.points_world.iter().any(|point| {
                (point.y - f64::from(CURB_STEP_HEIGHT_M)).abs() <= f64::from(SAMPLE_EPSILON_M)
            }) && face
                .points_world
                .iter()
                .any(|point| point.y.abs() <= f64::from(SAMPLE_EPSILON_M))
        }),
        "direct carriageway-sidewalk span boundary must emit a raised vertical face"
    );
}

#[test]
fn span_vertical_steps_include_generic_non_road_owner_pairs() {
    let mut graph = RegionGraph::new();
    let a = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let b = graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        a,
        b,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
        5.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR,
    ));
    let sidewalk_height_m = CURB_STEP_HEIGHT_M * 2.0;
    let section_at = |s_m: f32| RoadSurfaceSection {
        edge_idx,
        s_m,
        center_xz: backend::RoadVec2::new(f64::from(s_m), 0.0),
        center_height_m: 0.0,
        tangent_xz: backend::RoadVec2::new(1.0, 0.0),
        lateral_xz: backend::RoadVec2::new(0.0, 1.0),
        bands: vec![
            RoadSurfaceBand {
                kind: RoadSurfaceBandKind::Carriageway,
                lateral_start_m: -3.0,
                lateral_end_m: 0.0,
                height_start_m: 0.0,
                height_end_m: 0.0,
            },
            RoadSurfaceBand {
                kind: RoadSurfaceBandKind::CurbOrShoulder,
                lateral_start_m: 0.0,
                lateral_end_m: 0.5,
                height_start_m: CURB_STEP_HEIGHT_M,
                height_end_m: CURB_STEP_HEIGHT_M,
            },
            RoadSurfaceBand {
                kind: RoadSurfaceBandKind::Sidewalk,
                lateral_start_m: 0.5,
                lateral_end_m: 2.0,
                height_start_m: sidewalk_height_m,
                height_end_m: sidewalk_height_m,
            },
        ],
    };
    let mut surface = RoadSurfaceSystem::new(64.0);
    surface.compiled_sections.insert(
        edge_idx,
        std::sync::Arc::new(vec![
            section_at(0.0),
            section_at(8.0),
            section_at(12.0),
            section_at(20.0),
        ]),
    );

    let span_piece = surface
        .compile_visual_span_piece(&graph, &flat_terrain(32, 32), edge_idx)
        .expect("curb-sidewalk stepped span should compile");
    assert!(
        span_piece.raised_step_face_polygons.iter().any(|face| {
            face.points_world.iter().any(|point| {
                (point.y - f64::from(sidewalk_height_m)).abs() <= f64::from(SAMPLE_EPSILON_M)
            }) && face.points_world.iter().any(|point| {
                (point.y - f64::from(CURB_STEP_HEIGHT_M)).abs() <= f64::from(SAMPLE_EPSILON_M)
            })
        }),
        "span raised-step output must be owner-pair generic, including curb / sidewalk"
    );
}

#[test]
fn span_profile_band_count_mismatch_rejects_partial_output() {
    let single_band = vec![RoadSurfaceBand {
        kind: RoadSurfaceBandKind::Carriageway,
        lateral_start_m: -2.0,
        lateral_end_m: 2.0,
        height_start_m: 0.0,
        height_end_m: 0.0,
    }];
    let split_bands = vec![
        RoadSurfaceBand {
            kind: RoadSurfaceBandKind::Carriageway,
            lateral_start_m: -2.0,
            lateral_end_m: 0.0,
            height_start_m: 0.0,
            height_end_m: 0.0,
        },
        RoadSurfaceBand {
            kind: RoadSurfaceBandKind::Sidewalk,
            lateral_start_m: 0.0,
            lateral_end_m: 2.0,
            height_start_m: CURB_STEP_HEIGHT_M,
            height_end_m: CURB_STEP_HEIGHT_M,
        },
    ];

    assert_rejects_invalid_span_profile(
        |edge_idx| {
            vec![
                span_profile_test_section(edge_idx, 0.0, single_band.clone()),
                span_profile_test_section(edge_idx, 5.0, single_band),
                span_profile_test_section(edge_idx, 35.0, split_bands.clone()),
                span_profile_test_section(edge_idx, 40.0, split_bands),
            ]
        },
        "band count mismatch",
    );
}

#[test]
fn span_profile_band_kind_mismatch_rejects_partial_output() {
    let sidewalk_bands = vec![
        RoadSurfaceBand {
            kind: RoadSurfaceBandKind::Carriageway,
            lateral_start_m: -2.0,
            lateral_end_m: 0.0,
            height_start_m: 0.0,
            height_end_m: 0.0,
        },
        RoadSurfaceBand {
            kind: RoadSurfaceBandKind::Sidewalk,
            lateral_start_m: 0.0,
            lateral_end_m: 2.0,
            height_start_m: CURB_STEP_HEIGHT_M,
            height_end_m: CURB_STEP_HEIGHT_M,
        },
    ];
    let curb_bands = vec![
        RoadSurfaceBand {
            kind: RoadSurfaceBandKind::Carriageway,
            lateral_start_m: -2.0,
            lateral_end_m: 0.0,
            height_start_m: 0.0,
            height_end_m: 0.0,
        },
        RoadSurfaceBand {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
            lateral_start_m: 0.0,
            lateral_end_m: 2.0,
            height_start_m: CURB_STEP_HEIGHT_M,
            height_end_m: CURB_STEP_HEIGHT_M,
        },
    ];

    assert_rejects_invalid_span_profile(
        |edge_idx| {
            vec![
                span_profile_test_section(edge_idx, 0.0, sidewalk_bands.clone()),
                span_profile_test_section(edge_idx, 5.0, sidewalk_bands),
                span_profile_test_section(edge_idx, 35.0, curb_bands.clone()),
                span_profile_test_section(edge_idx, 40.0, curb_bands),
            ]
        },
        "band kind mismatch",
    );
}

#[test]
fn span_earthwork_outer_loops_stay_outside_paved_footprint() {
    let terrain = flat_terrain(97, 97);
    let mut graph = RegionGraph::new();
    let a = graph.add_node(Vector3::new(0.0, 0.0, -24.0), NodeType::Junction);
    let b = graph.add_node(Vector3::new(0.0, 0.0, 24.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        a,
        b,
        vec![Vector3::new(0.0, 0.0, -24.0), Vector3::new(0.0, 0.0, 24.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let span_piece = surface
        .compiled_visual_span_pieces()
        .get(&edge_idx)
        .expect("standard edge should compile a visual span piece");
    let earthwork_outer_points = span_piece
        .earthwork_outer_boundary_loops
        .iter()
        .flat_map(|polygon| polygon.points_world.iter())
        .copied()
        .collect::<Vec<_>>();
    let min_outer_footprint_distance_m = earthwork_outer_points
        .iter()
        .map(|outer_point| {
            span_piece
                .outer_boundary_loops
                .iter()
                .flat_map(|footprint| {
                    (0..footprint.points_world.len()).map(|index| {
                        let start = footprint.points_world[index];
                        let end =
                            footprint.points_world[(index + 1) % footprint.points_world.len()];
                        let start_xz = backend::RoadVec2::new(start.x, start.z);
                        let end_xz = backend::RoadVec2::new(end.x, end.z);
                        let point_xz = backend::RoadVec2::new(outer_point.x, outer_point.z);
                        let segment = end_xz - start_xz;
                        if segment.length_squared() <= f64::from(SAMPLE_EPSILON_M) {
                            point_xz.distance(start_xz)
                        } else {
                            let t = ((point_xz - start_xz).dot(segment) / segment.length_squared())
                                .clamp(0.0, 1.0);
                            point_xz.distance(start_xz + segment * t)
                        }
                    })
                })
                .fold(f64::INFINITY, f64::min)
        })
        .fold(f64::INFINITY, f64::min);
    assert!(
        earthwork_outer_points.iter().all(|outer_point| {
            let point_xz = backend::RoadVec2::new(outer_point.x, outer_point.z);
            span_piece.outer_boundary_loops.iter().all(|footprint| {
                !RoadSurfaceSystem::polygon_contains_point_xz(&footprint.points_world, point_xz)
            })
        }) && min_outer_footprint_distance_m >= 0.5,
        "expected span earthwork tie-in to stay outside the paved footprint, got min_outer_footprint_distance_m={min_outer_footprint_distance_m:.3}"
    );
}

#[test]
fn earthwork_face_classification_distinguishes_slopes_from_walls() {
    assert_eq!(
        RoadSurfaceSystem::classify_earthwork_face_kind(
            backend::RoadVec3::new(0.0, 0.0, 0.0),
            backend::RoadVec3::new(1.0, 0.0, 0.0),
            backend::RoadVec3::new(2.0, 0.5, 0.0),
            backend::RoadVec3::new(1.0, 0.5, 0.0),
        ),
        RoadSurfaceEarthworkFaceKind::Slope
    );
    assert_eq!(
        RoadSurfaceSystem::classify_earthwork_face_kind(
            backend::RoadVec3::new(0.0, 0.0, 0.0),
            backend::RoadVec3::new(1.0, 0.0, 0.0),
            backend::RoadVec3::new(1.1, 3.0, 0.0),
            backend::RoadVec3::new(0.1, 3.0, 0.0),
        ),
        RoadSurfaceEarthworkFaceKind::RetainingWall
    );
}
