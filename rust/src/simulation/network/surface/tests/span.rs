// SPDX-License-Identifier: GPL-2.0-only

//! Span compilation and span-owned surface regression tests.

use super::*;
use crate::simulation::network::surface::{RoadVec3, SpanQuad};

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
    assert_ne!(span_piece.raised_step_face_polygons().len(), 0);
    assert!(
        span_piece.raised_step_face_polygons().any(|face| {
            face.points().iter().any(|point| {
                (point.y - f64::from(CURB_STEP_HEIGHT_M)).abs() <= f64::from(SAMPLE_EPSILON_M)
            }) && face
                .points()
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
        span_piece.raised_step_face_polygons().any(|face| {
            face.points().iter().any(|point| {
                (point.y - f64::from(sidewalk_height_m)).abs() <= f64::from(SAMPLE_EPSILON_M)
            }) && face.points().iter().any(|point| {
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

fn assert_quad_matches_strip_polygon(corners: [RoadVec3; 4]) {
    assert_eq!(
        SpanQuad::from_vertical_points(corners).map(|quad| quad.to_polygon()),
        RoadSurfaceSystem::make_vertical_quad_polygon(corners),
        "vertical corners {corners:?}"
    );
    assert_eq!(
        SpanQuad::from_corners(corners).map(|quad| quad.to_polygon()),
        RoadSurfaceSystem::make_visual_strip_polygon(corners.to_vec()),
        "corners {corners:?}"
    );
}

#[test]
fn derived_surface_products_match_stored_forms() {
    let strip = [
        RoadVec3::new(0.0, 0.0, 0.0),
        RoadVec3::new(0.0, 0.1, 8.0),
        RoadVec3::new(3.5, 0.1, 8.0),
        RoadVec3::new(3.5, 0.0, 0.0),
    ];
    let mut cases = vec![strip];
    // Collapsed tapers, numeric-dust duplicates on either side of the dedup distance, and
    // closing duplicates.
    for moved in 0..4 {
        for onto in 0..4 {
            for offset_m in [0.0, 5.0e-5, 9.9e-5, 1.01e-4, 1.0e-3] {
                let mut corners = strip;
                corners[moved] = strip[onto] + RoadVec3::new(offset_m, 0.0, 0.0);
                cases.push(corners);
            }
        }
    }
    // Self-crossing and collinear strips.
    cases.push([strip[0], strip[2], strip[1], strip[3]]);
    cases.push([0.0, 1.0, 2.0, 3.0].map(|t| RoadVec3::new(t, 0.0, 2.0 * t)));
    // Arbitrary small quads from a fixed linear congruential sequence.
    let mut state = 0x2545_f491_4f6c_dd1d_u64;
    let mut next = || {
        state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        ((state >> 33) as f64 / f64::from(u32::MAX >> 1)) * 2.0 - 1.0
    };
    for _ in 0..20_000 {
        cases.push(std::array::from_fn(|_| {
            RoadVec3::new(next() * 2.0, next() * 0.1, next() * 2.0)
        }));
    }
    for corners in cases {
        assert_quad_matches_strip_polygon(corners);
    }

    // Every region of a compiled sloped double-T network.
    let terrain = sloped_terrain(192, 192);
    let mut graph = RegionGraph::new();
    let mut network = TransitNetwork::new_with_surface_chunk_span(32.0);
    let mut zoning = crate::simulation::zoning::ZoningSystem::new(
        &crate::simulation::core::config::WorldConfig::default(),
    );
    let mut allocator = crate::simulation::buildings::allocator::BuildingAllocator::new();
    let ground = |x: f32, z: f32| {
        Vector3::new(
            x,
            terrain.sample_height_world(x, z) * crate::config::HEIGHT_SCALE,
            z,
        )
    };
    for stroke in [
        vec![ground(-60.0, 0.0), ground(-20.0, 10.0), ground(60.0, 0.0)],
        vec![ground(-16.0, 48.0), ground(-16.0, 4.0)],
        vec![ground(16.0, -48.0), ground(16.0, -4.0)],
    ] {
        network.bulk_load = true;
        network.add_road(&mut graph, stroke, 1, 1, EdgeClass::Standard, &mut zoning, &mut allocator);
        network.bulk_load = false;
    }
    network.finalize_road_geometry(&mut graph);
    assert!(network.road_surface.compile_dirty(&graph, &terrain));
    let mut regions = 0;
    let mut samples = 0;
    for piece in network.road_surface.compiled_visual_span_pieces.values() {
        // The packed-id grid samples exactly like a triangle index over the same polygons.
        let polygons = |quads: &mut dyn Iterator<Item = SpanQuad>| {
            quads.map(|quad| quad.to_polygon()).collect::<Vec<_>>()
        };
        let index = crate::simulation::network::surface::RoadSurfaceTriangleQueryIndex::from_surface_polygons(
            &polygons(&mut piece.road_surface_polygons()),
            &polygons(&mut piece.curb_surface_polygons()),
            &polygons(&mut piece.sidewalk_surface_polygons()),
        );
        let points: Vec<_> = piece.surface_polygons().flat_map(|quad| quad.points().to_vec()).collect();
        let (min_x, max_x) = points.iter().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.x), b.max(p.x)));
        let (min_z, max_z) = points.iter().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.z), b.max(p.z)));
        let mut x = min_x - 1.0;
        while x <= max_x + 1.0 {
            let mut z = min_z - 1.0;
            while z <= max_z + 1.0 {
                let point = crate::simulation::network::surface::RoadVec2::new(x, z);
                for carriageway_only in [false, true] {
                    assert_eq!(
                        piece.sample_height(point, carriageway_only),
                        index.sample_height(point, carriageway_only)
                    );
                }
                assert_eq!(piece.sample_visible_height(point), index.sample_visible_height(point));
                samples += 1;
                z += 0.37;
            }
            x += 0.37;
        }
        for region in piece.span_owned_regions.iter().chain(piece.span_earthwork_support_regions.iter()) {
            assert_quad_matches_strip_polygon(region.corners(&piece.sections));
            regions += 1;
        }
    }
    let (mut junctions, mut nodes) = (0, 0);
    for piece in network.road_surface.compiled_visual_node_pieces.values() {
        // Node top polygons are the owned regions partitioned by material and sorted.
        let (mut road, mut curb, mut sidewalk) =
            RoadSurfaceSystem::top_polygons_from_owned_regions_by_material(&piece.owned_regions);
        for polygons in [&mut road, &mut curb, &mut sidewalk] {
            RoadSurfaceSystem::sort_visual_polygons(polygons);
        }
        assert_eq!(piece.road_surface_polygons().cloned().collect::<Vec<_>>(), road);
        assert_eq!(piece.curb_surface_polygons().cloned().collect::<Vec<_>>(), curb);
        assert_eq!(piece.sidewalk_surface_polygons().cloned().collect::<Vec<_>>(), sidewalk);
        junctions += usize::from(piece.kind == RoadSurfaceVisualNodePieceKind::JunctionN);
        nodes += 1;
    }
    assert!(junctions >= 1 && nodes >= 4, "{junctions} junctions of {nodes} nodes checked");
    assert!(regions > 100, "only {regions} compiled regions checked");
    assert!(samples > 10_000, "only {samples} surface samples checked");
}
