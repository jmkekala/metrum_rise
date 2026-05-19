//! Terrain clip and road-touched CDT stage contract tests.

use super::super::RoadSurfaceTerrainClipContourRole;
use super::super::keys::{SurfaceHeightMmKey, SurfaceXzKey};
use super::*;

#[test]
fn terrain_clip_loops_include_standard_grounded_footprints() {
    let terrain = flat_terrain(97, 97);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(0.0, 0.0, -24.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(0.0, 0.0, 24.0), NodeType::Junction);
    graph.add_edge(test_edge(
        start,
        end,
        vec![Vector3::new(0.0, 0.0, -24.0), Vector3::new(0.0, 0.0, 24.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let (cdt_road_loops, cdt_source_count) = surface
        .terrain_cdt_road_loops_for_world_bounds(&graph, -16.0, -32.0, 16.0, 32.0)
        .expect("production terrain clip export should keep source-owned loops");

    assert!(
        !cdt_road_loops.is_empty(),
        "expected grounded standard road footprint loops to clip terrain topology"
    );
    assert!(
        cdt_road_loops
            .iter()
            .flat_map(|road_loop| road_loop.vertices.iter())
            .any(|point| point.x.abs() > 5.0),
        "expected terrain clip loops to include the full sidewalk / shoulder footprint"
    );
    assert!(
        cdt_road_loops
            .iter()
            .all(|road_loop| road_loop.vertices.len() >= 3),
        "expected every terrain clip loop to be a valid road footprint contour"
    );
    let expected_terrain_clip_source_loop_count: usize = surface
        .compiled_visual_span_pieces()
        .values()
        .map(|piece| piece.terrain_clip_boundary_loops.len())
        .sum::<usize>()
        + surface
            .compiled_visual_node_pieces()
            .values()
            .map(|piece| piece.terrain_clip_boundary_loops.len())
            .sum::<usize>();
    assert!(
        cdt_road_loops.len() <= expected_terrain_clip_source_loop_count,
        "expected terrain clip cutters to be the boolean-unioned piece footprint, got {} loops for {} raw clip loops",
        cdt_road_loops.len(),
        expected_terrain_clip_source_loop_count
    );
    assert_eq!(cdt_source_count, expected_terrain_clip_source_loop_count);
    assert!(
        cdt_road_loops
            .iter()
            .flat_map(|road_loop| road_loop.source_edges.iter())
            .all(|edge| !matches!(
                edge.source,
                TerrainCdtRoadBoundarySource::SyntheticTestBoundary { .. }
            )),
        "production terrain CDT loops must carry real span/node boundary sources, not synthetic polygon ids"
    );
    assert!(
        cdt_road_loops
            .iter()
            .flat_map(|road_loop| road_loop.source_edges.iter())
            .any(|edge| matches!(
                edge.source,
                TerrainCdtRoadBoundarySource::SpanSupportBoundary { .. }
                    | TerrainCdtRoadBoundarySource::NodeFootprintBoundary { .. }
            )),
        "expected source-preserving CDT export to expose final owned terrain boundary provenance"
    );
}

#[test]
fn surface_terrain_cdt_authored_piece_matrix_preserves_owned_sources() {
    {
        let terrain = coarse_hillside_world_terrain(161, 161, 1.0);
        let points = grounded_polyline_points_from_terrain(
            &terrain,
            Vector2::new(-40.0, -24.0),
            Vector2::new(40.0, -24.0),
            20,
        );
        let mut graph = RegionGraph::new();
        let start = graph.add_node(points[0], NodeType::Junction);
        let end = graph.add_node(*points.last().unwrap(), NodeType::Junction);
        let edge_idx = graph.add_edge(test_edge(
            start,
            end,
            points,
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        graph.rebuild_intersection_clips();

        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.compile_dirty(&graph, &terrain);
        assert!(
            surface
                .compiled_visual_span_pieces()
                .contains_key(&edge_idx)
        );
        assert_eq!(
            surface
                .compiled_visual_node_pieces()
                .values()
                .filter(|piece| piece.kind == RoadSurfaceVisualNodePieceKind::Terminal)
                .count(),
            2,
            "straight road should cover span plus both terminal footprints"
        );
        assert_surface_terrain_cdt_contract(
            "straight span with terminal footprints on authored hillside",
            &surface,
            &graph,
            &terrain,
            (-56.0, -44.0, 56.0, 4.0),
            false,
        );
    }

    {
        let terrain = flat_terrain(161, 161);
        let center_xz = Vector2::new(0.0, 0.0);
        let west_xz = Vector2::new(-36.0, 0.0);
        let north_xz = Vector2::new(0.0, 36.0);
        let center_pos = Vector3::new(
            center_xz.x,
            terrain_height_m(&terrain, center_xz.x, center_xz.y),
            center_xz.y,
        );
        let west_points = grounded_polyline_points_from_terrain(&terrain, west_xz, center_xz, 12);
        let north_points = grounded_polyline_points_from_terrain(&terrain, center_xz, north_xz, 12);
        let mut graph = RegionGraph::new();
        let west = graph.add_node(west_points[0], NodeType::Junction);
        let center = graph.add_node(center_pos, NodeType::Junction);
        let north = graph.add_node(*north_points.last().unwrap(), NodeType::Junction);
        graph.add_edge(test_edge(
            west,
            center,
            west_points,
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        graph.add_edge(test_edge(
            center,
            north,
            north_points,
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        graph.rebuild_intersection_clips();

        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.compile_dirty(&graph, &terrain);
        assert_eq!(
            surface
                .compiled_visual_node_pieces()
                .get(&center)
                .unwrap()
                .kind,
            RoadSurfaceVisualNodePieceKind::Bend
        );
        assert_surface_terrain_cdt_contract(
            "bend footprint",
            &surface,
            &graph,
            &terrain,
            (-52.0, -16.0, 16.0, 52.0),
            false,
        );
    }

    {
        let terrain = flat_terrain(129, 129);
        let mut graph = RegionGraph::new();
        let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        for endpoint in [
            Vector3::new(-40.0, 0.0, 0.0),
            Vector3::new(40.0, 0.0, 0.0),
            Vector3::new(0.0, 0.0, 40.0),
        ] {
            let node = graph.add_node(endpoint, NodeType::Junction);
            let (start, end, points) = if endpoint.x < 0.0 {
                (node, center, vec![endpoint, Vector3::new(0.0, 0.0, 0.0)])
            } else {
                (center, node, vec![Vector3::new(0.0, 0.0, 0.0), endpoint])
            };
            graph.add_edge(test_edge(
                start,
                end,
                points,
                10.0,
                EdgeClass::Standard,
                TransitType::Road,
                TransitFlags::CAR | TransitFlags::FOOT,
            ));
        }
        graph.rebuild_intersection_clips();

        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.compile_dirty(&graph, &terrain);
        assert_eq!(
            surface
                .compiled_visual_node_pieces()
                .get(&center)
                .unwrap()
                .kind,
            RoadSurfaceVisualNodePieceKind::JunctionN
        );
        assert_surface_terrain_cdt_contract(
            "flat JunctionN footprint",
            &surface,
            &graph,
            &terrain,
            (-56.0, -24.0, 56.0, 56.0),
            false,
        );
    }

    {
        let terrain = flat_terrain(129, 129);
        let road_y = 3.0;
        let center_pos = Vector3::new(0.0, road_y, 0.0);
        let mut graph = RegionGraph::new();
        let center = graph.add_node(center_pos, NodeType::Junction);
        for endpoint in [
            Vector3::new(-40.0, road_y, 0.0),
            Vector3::new(40.0, road_y, 0.0),
            Vector3::new(0.0, road_y, 40.0),
        ] {
            let node = graph.add_node(endpoint, NodeType::Junction);
            let (start, end, points) = if endpoint.x < 0.0 {
                (node, center, vec![endpoint, center_pos])
            } else {
                (center, node, vec![center_pos, endpoint])
            };
            graph.add_edge(test_edge(
                start,
                end,
                points,
                10.0,
                EdgeClass::Standard,
                TransitType::Road,
                TransitFlags::CAR | TransitFlags::FOOT,
            ));
        }
        graph.rebuild_intersection_clips();

        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.compile_dirty(&graph, &terrain);
        assert_eq!(
            surface
                .compiled_visual_node_pieces()
                .get(&center)
                .unwrap()
                .kind,
            RoadSurfaceVisualNodePieceKind::JunctionN
        );
        assert_surface_terrain_cdt_contract(
            "elevated Standard JunctionN footprint over flat authored terrain",
            &surface,
            &graph,
            &terrain,
            (-56.0, -24.0, 56.0, 56.0),
            false,
        );
    }
}

#[test]
fn terrain_clip_union_splits_union_segment_through_source_owned_boundary_chain() {
    let p0 = Vector3::new(0.0, 10.0, 0.0);
    let p1 = Vector3::new(0.45, 10.8, 0.18);
    let p2 = Vector3::new(1.0, 11.4, 0.0);
    let p3 = Vector3::new(1.0, 11.4, 0.5);
    let p4 = Vector3::new(0.0, 10.0, 0.5);
    let source_loop = RoadSurfaceTerrainClipLoop {
        source_edges: vec![
            terrain_clip_source_edge_for_test(p0, p1),
            terrain_clip_source_edge_for_test(p1, p2),
            terrain_clip_source_edge_for_test(p2, p3),
            terrain_clip_source_edge_for_test(p3, p4),
            terrain_clip_source_edge_for_test(p4, p0),
        ],
        points_world: vec![p0, p2, p3, p4],
    };
    let unioned = RoadSurfaceSystem::union_terrain_clip_boundary_loops_with_sources(&[source_loop])
        .expect("source-owned split segment should survive terrain clip union");

    assert_eq!(
        unioned.len(),
        1,
        "unioned terrain clip contour must not drop a source-owned segment that spans adjacent boundary edges"
    );
    let points = &unioned[0].points_world;
    assert!(
        points
            .iter()
            .any(|point| (point.x - p1.x).abs() <= SAMPLE_EPSILON_M
                && (point.z - p1.z).abs() <= SAMPLE_EPSILON_M),
        "source-owned split vertex must be stitched back into the unioned cutter"
    );
    assert_eq!(
        unioned[0].source_edges.len(),
        points.len(),
        "every emitted clip segment must keep an owner-backed source edge"
    );
    assert!(
        unioned[0]
            .source_edges
            .iter()
            .any(|edge| (edge.start.x - p0.x).abs() <= SAMPLE_EPSILON_M
                && (edge.end.x - p1.x).abs() <= SAMPLE_EPSILON_M),
        "first split subsegment must preserve its original source edge"
    );
    assert!(
        unioned[0]
            .source_edges
            .iter()
            .any(|edge| (edge.start.x - p1.x).abs() <= SAMPLE_EPSILON_M
                && (edge.end.x - p2.x).abs() <= SAMPLE_EPSILON_M),
        "second split subsegment must preserve its original source edge"
    );
}

#[test]
fn terrain_clip_union_stitches_source_chain_across_same_xz_height_step() {
    let p0 = Vector3::new(0.0, 10.0, 0.0);
    let p1_low = Vector3::new(0.45, 10.8, 0.18);
    let p1_high = Vector3::new(0.45, 11.0, 0.18);
    let p2 = Vector3::new(1.0, 11.4, 0.0);
    let p3 = Vector3::new(1.0, 11.4, 0.5);
    let p4 = Vector3::new(0.0, 10.0, 0.5);
    let source_loop = RoadSurfaceTerrainClipLoop {
        source_edges: vec![
            terrain_clip_source_edge_for_test(p0, p1_low),
            terrain_clip_source_edge_for_test(p1_high, p2),
            terrain_clip_source_edge_for_test(p2, p3),
            terrain_clip_source_edge_for_test(p3, p4),
            terrain_clip_source_edge_for_test(p4, p0),
        ],
        points_world: vec![p0, p2, p3, p4],
    };
    let unioned = RoadSurfaceSystem::union_terrain_clip_boundary_loops_with_sources(&[source_loop])
        .expect("source-owned height step should survive terrain clip union");

    assert_eq!(
        unioned.len(),
        1,
        "same-XZ source chain height steps must not drop the road-owned terrain clip loop"
    );
    let stitched = unioned[0]
        .points_world
        .iter()
        .find(|point| {
            (point.x - p1_low.x).abs() <= SAMPLE_EPSILON_M
                && (point.z - p1_low.z).abs() <= SAMPLE_EPSILON_M
        })
        .copied()
        .expect("source chain split vertex should survive terrain clip union");
    assert!(
        stitched.y >= p1_high.y - SAMPLE_EPSILON_M,
        "shared XZ source chain vertex should use the highest visible road height"
    );
    assert_eq!(
        unioned[0].source_edges.len(),
        unioned[0].points_world.len(),
        "every stitched segment must keep owner-backed source provenance"
    );
}

#[test]
fn terrain_clip_union_rejects_ambiguous_source_chain_recovery() {
    let p0 = Vector3::new(0.0, 9.0, 0.0);
    let p1_a = Vector3::new(0.45, 9.4, 0.18);
    let p1_b = Vector3::new(0.55, 9.6, -0.16);
    let p2 = Vector3::new(1.0, 10.0, 0.0);
    let p3 = Vector3::new(1.0, 10.0, 0.5);
    let p4 = Vector3::new(0.0, 9.0, 0.5);
    let loop_for_node = |node_id, midpoint| RoadSurfaceTerrainClipLoop {
        source_edges: vec![
            terrain_clip_source_edge_for_node_test(p0, midpoint, node_id),
            terrain_clip_source_edge_for_node_test(midpoint, p2, node_id),
            terrain_clip_source_edge_for_node_test(p2, p3, node_id),
            terrain_clip_source_edge_for_node_test(p3, p4, node_id),
            terrain_clip_source_edge_for_node_test(p4, p0, node_id),
        ],
        points_world: vec![p0, p2, p3, p4],
    };

    let unioned = RoadSurfaceSystem::union_terrain_clip_boundary_loops_with_sources(&[
        loop_for_node(1, p1_a),
        loop_for_node(1, p1_b),
    ]);

    let Err(RoadSurfaceTerrainClipExportError::MissingOuterBoundaryOwner { context, .. }) = unioned
    else {
        panic!("ambiguous source-chain recovery must reject provenance, got {unioned:?}");
    };
    assert!(
        context.contains("ambiguous_source_chain"),
        "ambiguous source-chain diagnostic should stay visible: {context}"
    );
}

#[test]
fn terrain_clip_union_rejects_matching_height_output_source_ambiguity() {
    let y = 8.0;
    let points = vec![
        Vector3::new(0.0, y, 0.0),
        Vector3::new(2.0, y, 0.0),
        Vector3::new(2.0, y, 1.0),
        Vector3::new(0.0, y, 1.0),
    ];
    let raw_clip_sources = vec![
        terrain_clip_loop_for_node_test(&points, 1),
        terrain_clip_loop_for_node_test(&points, 2),
    ];

    let unioned =
        RoadSurfaceSystem::union_terrain_clip_boundary_loops_with_sources(&raw_clip_sources);

    let Err(RoadSurfaceTerrainClipExportError::AmbiguousOutputBoundaryOwner { context, .. }) =
        unioned
    else {
        panic!(
            "coincident matching-height source edges with different provenance must reject, got {unioned:?}"
        );
    };
    assert!(
        context.contains("sources_disagree"),
        "ambiguous output source diagnostic should name provenance disagreement: {context}"
    );
}

#[test]
fn terrain_clip_union_rejects_output_source_ambiguity_across_kind_priority() {
    let y = 8.0;
    let points = vec![
        Vector3::new(0.0, y, 0.0),
        Vector3::new(2.0, y, 0.0),
        Vector3::new(2.0, y, 1.0),
        Vector3::new(0.0, y, 1.0),
    ];
    let raw_clip_sources = vec![
        terrain_clip_loop_for_node_kind_test(
            &points,
            1,
            RoadSurfaceTerrainClipEdgeKind::SidewalkOuter,
        ),
        terrain_clip_loop_for_node_kind_test(
            &points,
            2,
            RoadSurfaceTerrainClipEdgeKind::FootprintBoundary,
        ),
    ];

    let unioned =
        RoadSurfaceSystem::union_terrain_clip_boundary_loops_with_sources(&raw_clip_sources);

    let Err(RoadSurfaceTerrainClipExportError::AmbiguousOutputBoundaryOwner { context, .. }) =
        unioned
    else {
        panic!(
            "output source selection must reject differing provenance instead of choosing the highest-priority edge kind, got {unioned:?}"
        );
    };
    assert!(
        context.contains("sources_disagree"),
        "ambiguous output source diagnostic should name cross-kind provenance disagreement: {context}"
    );
}

#[test]
fn terrain_clip_union_exports_hole_contours_for_ring_footprint() {
    let y = 2.0;
    let rectangles = [
        vec![
            Vector3::new(0.0, y, 0.0),
            Vector3::new(4.0, y, 0.0),
            Vector3::new(4.0, y, 1.0),
            Vector3::new(0.0, y, 1.0),
        ],
        vec![
            Vector3::new(0.0, y, 3.0),
            Vector3::new(4.0, y, 3.0),
            Vector3::new(4.0, y, 4.0),
            Vector3::new(0.0, y, 4.0),
        ],
        vec![
            Vector3::new(0.0, y, 1.0),
            Vector3::new(1.0, y, 1.0),
            Vector3::new(1.0, y, 3.0),
            Vector3::new(0.0, y, 3.0),
        ],
        vec![
            Vector3::new(3.0, y, 1.0),
            Vector3::new(4.0, y, 1.0),
            Vector3::new(4.0, y, 3.0),
            Vector3::new(3.0, y, 3.0),
        ],
    ];
    let raw_clip_sources = rectangles
        .iter()
        .enumerate()
        .map(|(index, points)| terrain_clip_loop_for_node_test(points, index as u32))
        .collect::<Vec<_>>();

    let export = RoadSurfaceSystem::union_terrain_clip_boundary_export(&raw_clip_sources)
        .expect("ring road footprint should export both outer and hole contours");

    assert_eq!(
        export.loops.len(),
        2,
        "unioned ring footprint must preserve its inner terrain-island contour"
    );
    assert_eq!(
        export
            .loop_topologies
            .iter()
            .filter(|topology| topology.role == RoadSurfaceTerrainClipContourRole::Outer)
            .count(),
        1
    );
    assert_eq!(
        export
            .loop_topologies
            .iter()
            .filter(|topology| topology.role == RoadSurfaceTerrainClipContourRole::Hole)
            .count(),
        1
    );
    let hole_loop = export
        .loops
        .iter()
        .zip(export.loop_topologies.iter())
        .find(|(_, topology)| topology.role == RoadSurfaceTerrainClipContourRole::Hole)
        .map(|(boundary_loop, _)| boundary_loop)
        .expect("hole topology should identify the inner contour");
    assert!(
        hole_loop.points_world.iter().all(|point| {
            point.x >= 1.0 - SAMPLE_EPSILON_M
                && point.x <= 3.0 + SAMPLE_EPSILON_M
                && point.z >= 1.0 - SAMPLE_EPSILON_M
                && point.z <= 3.0 + SAMPLE_EPSILON_M
        }),
        "hole contour must stay on the inner terrain island boundary: {hole_loop:?}"
    );
}

#[test]
fn terrain_cdt_preserves_source_samples_inside_road_hole() {
    let outer = vec![
        TerrainCdtVertex::new(0.0, 0.0, 0.0),
        TerrainCdtVertex::new(4.0, 0.0, 0.0),
        TerrainCdtVertex::new(4.0, 0.0, 4.0),
        TerrainCdtVertex::new(0.0, 0.0, 4.0),
    ];
    let hole = vec![
        TerrainCdtVertex::new(1.0, 0.0, 1.0),
        TerrainCdtVertex::new(1.0, 0.0, 3.0),
        TerrainCdtVertex::new(3.0, 0.0, 3.0),
        TerrainCdtVertex::new(3.0, 0.0, 1.0),
    ];
    let hole_sample = TerrainCdtVertex::new(2.0, 0.0, 2.0);

    let mesh = build_road_touched_terrain_patch(TerrainCdtInput::new(
        TerrainCdtPatch::new(-1.0, -1.0, 5.0, 5.0, [0.0; 4]),
        vec![
            TerrainCdtRoadLoop::new_with_source_edges_and_topology(
                10,
                10,
                0,
                false,
                outer,
                Vec::new(),
            ),
            TerrainCdtRoadLoop::new_with_source_edges_and_topology(
                11,
                10,
                1,
                true,
                hole,
                Vec::new(),
            ),
        ],
        vec![hole_sample],
    ))
    .expect("CDT should accept road footprint holes as constrained terrain islands");

    assert!(
        mesh.vertices
            .iter()
            .any(
                |vertex| (vertex.x - hole_sample.x).abs() <= f64::from(SAMPLE_EPSILON_M)
                    && (vertex.z - hole_sample.z).abs() <= f64::from(SAMPLE_EPSILON_M)
            ),
        "source terrain sample inside a road footprint hole must not be rejected as road-owned"
    );
}

#[test]
fn terrain_clip_union_blocks_partial_export_when_shape_has_no_source_owner() {
    let y = 4.0;
    let valid = [
        Vector3::new(0.0, y, 0.0),
        Vector3::new(1.0, y, 0.0),
        Vector3::new(1.0, y, 1.0),
        Vector3::new(0.0, y, 1.0),
    ];
    let unowned = [
        Vector3::new(3.0, y, 0.0),
        Vector3::new(4.0, y, 0.0),
        Vector3::new(4.0, y, 1.0),
        Vector3::new(3.0, y, 1.0),
    ];
    let raw_clip_sources = vec![
        RoadSurfaceTerrainClipLoop {
            source_edges: valid
                .iter()
                .zip(valid.iter().cycle().skip(1))
                .take(valid.len())
                .map(|(&start, &end)| terrain_clip_source_edge_for_test(start, end))
                .collect(),
            points_world: valid.to_vec(),
        },
        RoadSurfaceTerrainClipLoop {
            source_edges: Vec::new(),
            points_world: unowned.to_vec(),
        },
    ];

    let unioned =
        RoadSurfaceSystem::union_terrain_clip_boundary_loops_with_sources(&raw_clip_sources);

    assert!(
        matches!(
            unioned,
            Err(RoadSurfaceTerrainClipExportError::MissingOuterBoundaryOwner { .. })
                | Err(RoadSurfaceTerrainClipExportError::MissingOutputBoundaryOwner { .. })
        ),
        "terrain clip union must return a blocking source-ownership error instead of silently dropping an unowned cutter shape"
    );
}

#[test]
fn terrain_clip_union_blocks_partially_covered_source_segment() {
    let y = 5.0;
    let p0 = Vector3::new(0.0, y, 0.0);
    let p1 = Vector3::new(1.0, y, 0.0);
    let p2 = Vector3::new(1.0, y, 1.0);
    let p3 = Vector3::new(0.0, y, 1.0);
    let mid = Vector3::new(0.5, y, 0.0);
    let raw_clip_sources = vec![RoadSurfaceTerrainClipLoop {
        source_edges: vec![
            terrain_clip_source_edge_for_test(p0, mid),
            terrain_clip_source_edge_for_test(p1, p2),
            terrain_clip_source_edge_for_test(p2, p3),
            terrain_clip_source_edge_for_test(p3, p0),
        ],
        points_world: vec![p0, p1, p2, p3],
    }];

    let unioned =
        RoadSurfaceSystem::union_terrain_clip_boundary_loops_with_sources(&raw_clip_sources);

    let Err(RoadSurfaceTerrainClipExportError::MissingOuterBoundaryOwner { context, .. }) = unioned
    else {
        panic!(
            "partially covered terrain clip segment must be a blocking source-coverage error, got {unioned:?}"
        );
    };
    assert!(
        context.contains("partial_coverage"),
        "partial source coverage should be diagnosed distinctly from ordinary missing ownership: {context}"
    );
}

#[test]
fn terrain_clip_union_skips_same_key_dust_only_when_degenerate() {
    let y = 6.0;
    let p0 = Vector3::new(0.0, y, 0.0);
    let p0_same_key = Vector3::new(0.0000002, y + 0.25, 0.0000002);
    let p1 = Vector3::new(1.0, y, 0.0);
    let p2 = Vector3::new(1.0, y, 1.0);
    let p3 = Vector3::new(0.0, y, 1.0);
    let raw_clip_sources = vec![RoadSurfaceTerrainClipLoop {
        source_edges: vec![
            terrain_clip_source_edge_for_test(p0, p0_same_key),
            terrain_clip_source_edge_for_test(p0_same_key, p1),
            terrain_clip_source_edge_for_test(p1, p2),
            terrain_clip_source_edge_for_test(p2, p3),
            terrain_clip_source_edge_for_test(p3, p0),
        ],
        points_world: vec![p0, p0_same_key, p1, p2, p3],
    }];

    let unioned =
        RoadSurfaceSystem::union_terrain_clip_boundary_loops_with_sources(&raw_clip_sources)
            .expect(
                "same-key degenerate dust should be skipped without losing the sourced clip loop",
            );

    assert_eq!(unioned.len(), 1);
    assert!(
        unioned[0]
            .source_edges
            .iter()
            .all(|edge| SurfaceXzKey::from_godot_world_xz(edge.start)
                != SurfaceXzKey::from_godot_world_xz(edge.end)),
        "same-key dust may be skipped only as a degenerate segment, never emitted as a sourced edge"
    );
}

#[test]
fn terrain_clip_source_endpoint_groups_use_key_derived_coordinates() {
    let p0_loop = Vector3::new(0.0, 10.0004, 0.0);
    let p0_source = Vector3::new(0.0000002, 10.00049, 0.0000002);
    let p1 = Vector3::new(1.0, 10.0, 0.0);
    let p2 = Vector3::new(1.0, 10.0, 1.0);
    let p3 = Vector3::new(0.0, 10.0, 1.0);
    let raw_clip_sources = vec![RoadSurfaceTerrainClipLoop {
        source_edges: vec![
            terrain_clip_source_edge_for_test(p0_source, p1),
            terrain_clip_source_edge_for_test(p1, p2),
            terrain_clip_source_edge_for_test(p2, p3),
            terrain_clip_source_edge_for_test(p3, p0_loop),
        ],
        points_world: vec![p0_loop, p1, p2, p3],
    }];

    let unioned =
        RoadSurfaceSystem::union_terrain_clip_boundary_loops_with_sources(&raw_clip_sources)
            .expect("same key endpoint group should canonicalize through keys, not raw majority");
    let origin = unioned[0]
        .source_edges
        .iter()
        .flat_map(|edge| [edge.start, edge.end])
        .find(|point| {
            SurfaceXzKey::from_godot_world_xz(*point) == SurfaceXzKey::from_raw_keys(0, 0)
        })
        .expect("origin endpoint should remain present after terrain clip union");

    assert_eq!(
        SurfaceHeightMmKey::from_m_f32(origin.y),
        SurfaceHeightMmKey::from_m_f32(10.0),
        "same-key source endpoints must be emitted at the canonical height key, not at a majority raw Vector3"
    );
    assert!(
        origin.x.abs() <= SAMPLE_EPSILON_M && origin.z.abs() <= SAMPLE_EPSILON_M,
        "same-key source endpoints must be emitted at the canonical XZ key"
    );
}

#[test]
fn terrain_clip_union_preserves_endpoint_owned_numeric_connector() {
    let y = 12.0;
    let points = vec![
        Vector3::new(0.0, y, 0.0),
        Vector3::new(0.5, y, 0.0),
        Vector3::new(0.501, y, 0.0001),
        Vector3::new(1.0, y, 0.0),
        Vector3::new(1.0, y, 0.1),
        Vector3::new(0.0, y, 0.1),
    ];
    let raw_clip_sources = vec![RoadSurfaceTerrainClipLoop {
        source_edges: vec![
            terrain_clip_source_edge_for_test(points[0], points[1]),
            terrain_clip_source_edge_for_test(points[2], points[3]),
            terrain_clip_source_edge_for_test(points[3], points[4]),
            terrain_clip_source_edge_for_test(points[4], points[5]),
            terrain_clip_source_edge_for_test(points[5], points[0]),
        ],
        points_world: points,
    }];

    let clip_export = RoadSurfaceSystem::union_terrain_clip_boundary_export(&raw_clip_sources)
        .expect("endpoint-owned connector should preserve terrain clip export");

    assert_eq!(
        clip_export.loops.len(),
        1,
        "unioned terrain clip contour must keep source-owned endpoint continuity instead of dropping the road cutter"
    );
    assert!(
        clip_export.loops[0]
            .points_world
            .iter()
            .all(|point| (point.y - y).abs() <= SAMPLE_EPSILON_M),
        "accepted connector must reuse canonical source endpoint heights"
    );
}

#[test]
fn terrain_clip_union_rejects_dust_connector_conflicting_same_xz_heights() {
    let raw_boundary_y = -99.0;
    let p0 = Vector3::new(0.0, 10.0, 0.0);
    let p1 = Vector3::new(0.5, 10.5, 0.0);
    let d0 = Vector3::new(0.50002, raw_boundary_y, 0.00008);
    let d1 = Vector3::new(0.49998, raw_boundary_y, 0.00016);
    let d2 = Vector3::new(0.50001, raw_boundary_y, 0.00024);
    let p2 = Vector3::new(0.5, 10.7, 0.00032);
    let p3 = Vector3::new(1.0, 11.0, 0.0);
    let p4 = Vector3::new(1.0, 11.0, 0.1);
    let p5 = Vector3::new(0.0, 10.0, 0.1);
    let conflict_a0 = Vector3::new(d1.x - 0.0002, 20.0, d1.z);
    let conflict_a1 = Vector3::new(d1.x + 0.0002, 20.0, d1.z);
    let conflict_b0 = Vector3::new(d1.x - 0.0002, 21.0, d1.z);
    let conflict_b1 = Vector3::new(d1.x + 0.0002, 21.0, d1.z);
    let raw_clip_sources = vec![RoadSurfaceTerrainClipLoop {
        source_edges: vec![
            terrain_clip_source_edge_for_test(p0, p1),
            terrain_clip_source_edge_for_test(p2, p3),
            terrain_clip_source_edge_for_test(p3, p4),
            terrain_clip_source_edge_for_test(p4, p5),
            terrain_clip_source_edge_for_test(p5, p0),
            terrain_clip_source_edge_for_test(conflict_a0, conflict_a1),
            terrain_clip_source_edge_for_test(conflict_b0, conflict_b1),
        ],
        points_world: vec![
            Vector3::new(p0.x, raw_boundary_y, p0.z),
            Vector3::new(p1.x, raw_boundary_y, p1.z),
            d0,
            d1,
            d2,
            Vector3::new(p2.x, raw_boundary_y, p2.z),
            Vector3::new(p3.x, raw_boundary_y, p3.z),
            Vector3::new(p4.x, raw_boundary_y, p4.z),
            Vector3::new(p5.x, raw_boundary_y, p5.z),
        ],
    }];

    let unioned =
        RoadSurfaceSystem::union_terrain_clip_boundary_loops_with_sources(&raw_clip_sources);

    let Err(RoadSurfaceTerrainClipExportError::AmbiguousDustConnectorHeight { context, .. }) =
        unioned
    else {
        panic!(
            "dust connector with conflicting same-XZ source heights must reject, got {unioned:?}"
        );
    };
    assert!(
        context.contains("conflicting_source_heights"),
        "dust connector height ambiguity should name conflicting height keys: {context}"
    );
}

#[test]
fn terrain_clip_union_preserves_boundary_only_connector_by_interpolation() {
    let raw_boundary_y = -99.0;
    let p0 = Vector3::new(0.0, 10.0, 0.0);
    let p1 = Vector3::new(0.5, 10.5, 0.0);
    let d0 = Vector3::new(0.50002, raw_boundary_y, 0.00008);
    let d1 = Vector3::new(0.49998, raw_boundary_y, 0.00016);
    let d2 = Vector3::new(0.50001, raw_boundary_y, 0.00024);
    let p2 = Vector3::new(0.5, 10.7, 0.00032);
    let p3 = Vector3::new(1.0, 11.0, 0.0);
    let p4 = Vector3::new(1.0, 11.0, 0.1);
    let p5 = Vector3::new(0.0, 10.0, 0.1);
    let raw_clip_sources = vec![RoadSurfaceTerrainClipLoop {
        source_edges: vec![
            terrain_clip_source_edge_for_test(p0, p1),
            terrain_clip_source_edge_for_test(p2, p3),
            terrain_clip_source_edge_for_test(p3, p4),
            terrain_clip_source_edge_for_test(p4, p5),
            terrain_clip_source_edge_for_test(p5, p0),
        ],
        points_world: vec![
            Vector3::new(p0.x, raw_boundary_y, p0.z),
            Vector3::new(p1.x, raw_boundary_y, p1.z),
            d0,
            d1,
            d2,
            Vector3::new(p2.x, raw_boundary_y, p2.z),
            Vector3::new(p3.x, raw_boundary_y, p3.z),
            Vector3::new(p4.x, raw_boundary_y, p4.z),
            Vector3::new(p5.x, raw_boundary_y, p5.z),
        ],
    }];

    let clip_export = RoadSurfaceSystem::union_terrain_clip_boundary_export(&raw_clip_sources)
        .expect("boundary-only connector should preserve terrain clip export");

    assert_eq!(
        clip_export.loops.len(),
        1,
        "unioned terrain clip cutter must survive a sub-budget boundary-only connector"
    );
    assert!(
        RoadSurfaceSystem::polygon_has_area_xz(&clip_export.loops[0].points_world),
        "preserved terrain clip cutter must remain a valid road footprint polygon"
    );
    assert!(
        clip_export.loops[0]
            .points_world
            .iter()
            .all(|point| (point.y - raw_boundary_y).abs() > SAMPLE_EPSILON_M),
        "boundary-only connector heights must come from solved source contour interpolation"
    );
    assert!(
        clip_export.loops[0]
            .points_world
            .iter()
            .any(|point| point.y > p1.y && point.y < p2.y),
        "sub-budget connector must carry interpolated seam heights between adjacent solved footprint vertices"
    );
}

#[test]
fn terrain_clip_loops_are_unioned_before_cdt_for_arbitrary_multiway_nodes() {
    let terrain = flat_terrain(257, 257);
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    for angle_degrees in [0.0_f32, 23.0, 61.0, 137.0, 211.0, 304.0] {
        let angle = angle_degrees.to_radians();
        let endpoint = Vector3::new(angle.cos() * 64.0, 0.0, angle.sin() * 64.0);
        let node = graph.add_node(endpoint, NodeType::Junction);
        graph.add_edge(test_edge(
            center,
            node,
            vec![Vector3::new(0.0, 0.0, 0.0), endpoint],
            14.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
    }
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let (road_loops, _) = surface
        .terrain_cdt_road_loops_for_world_bounds(&graph, -96.0, -96.0, 96.0, 96.0)
        .expect("arbitrary multiway terrain clip export should succeed");
    assert!(
        !road_loops.is_empty(),
        "expected arbitrary multiway node to produce terrain clip loops"
    );

    let mesh = build_road_touched_terrain_patch(TerrainCdtInput::new(
        TerrainCdtPatch::new(-96.0, -96.0, 96.0, 96.0, [0.0; 4]),
        road_loops,
        Vec::new(),
    ))
    .expect("unioned terrain clip footprint must be accepted by the terrain CDT");

    assert_eq!(
        mesh.stats.invalid_constraint_edges, 0,
        "terrain CDT must not see crossing constraints from arbitrary-angle piece loops"
    );
}

#[test]
fn road_locked_terrain_patches_are_bounded_to_visible_footprint() {
    let terrain = flat_terrain(257, 257);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(0.0, 0.0, -48.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(0.0, 0.0, 48.0), NodeType::Junction);
    graph.add_edge(test_edge(
        start,
        end,
        vec![Vector3::new(0.0, 0.0, -48.0), Vector3::new(0.0, 0.0, 48.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let mut footprint_min_x = f32::MAX;
    let mut footprint_max_x = f32::MIN;
    let mut footprint_min_z = f32::MAX;
    let mut footprint_max_z = f32::MIN;
    for point in surface
        .compiled_visual_span_pieces()
        .values()
        .flat_map(|piece| piece.outer_boundary_loops.iter())
        .chain(
            surface
                .compiled_visual_node_pieces()
                .values()
                .flat_map(|piece| piece.outer_boundary_loops.iter()),
        )
        .flat_map(|polygon| polygon.points_world.iter())
    {
        footprint_min_x = footprint_min_x.min(point.x);
        footprint_max_x = footprint_max_x.max(point.x);
        footprint_min_z = footprint_min_z.min(point.z);
        footprint_max_z = footprint_max_z.max(point.z);
    }

    let keys = surface.terrain_render_patch_keys_with_visible_road(&terrain);
    assert!(!keys.is_empty());
    assert!(
        keys.len() < terrain.render_patch_cols() * terrain.render_patch_rows() / 8,
        "road-locked render patches must stay local to the visible road footprint"
    );
    for (patch_x, patch_z) in keys {
        let patch = terrain.visual_patch_snapshot(patch_x, patch_z).unwrap();
        let patch_max_x = patch.world_origin_x + patch.world_size_x;
        let patch_max_z = patch.world_origin_z + patch.world_size_z;
        assert!(
            patch.world_origin_x <= footprint_max_x
                && patch_max_x >= footprint_min_x
                && patch.world_origin_z <= footprint_max_z
                && patch_max_z >= footprint_min_z,
            "road-locked patch ({patch_x}, {patch_z}) must overlap the road footprint, not only the earthwork envelope"
        );
    }
}

#[test]
fn terrain_clip_loops_skip_bridge_midspans() {
    let terrain = flat_terrain(97, 97);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(0.0, 0.0, -24.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(0.0, 0.0, 24.0), NodeType::Junction);
    graph.add_edge(test_edge(
        start,
        end,
        vec![Vector3::new(0.0, 8.0, -24.0), Vector3::new(0.0, 8.0, 24.0)],
        10.0,
        EdgeClass::Bridge,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let (road_loops, source_count) = surface
        .terrain_cdt_road_loops_for_world_bounds(&graph, -16.0, -32.0, 16.0, 32.0)
        .expect("bridge midspan terrain clip export should succeed");

    assert!(
        road_loops.is_empty() && source_count == 0,
        "bridge midspans must not cut terrain topology like grounded standard roads"
    );
}

#[test]
fn surface_terrain_cdt_skips_bridge_and_tunnel_midspan_support() {
    for (case_name, edge_class, points) in [
        (
            "bridge endpoint abutments",
            EdgeClass::Bridge,
            vec![
                Vector3::new(-24.0, 6.0, 0.0),
                Vector3::new(0.0, 6.0, 0.0),
                Vector3::new(24.0, 6.0, 0.0),
            ],
        ),
        (
            "tunnel visible portals",
            EdgeClass::Tunnel,
            vec![
                Vector3::new(-24.0, 0.0, 0.0),
                Vector3::new(-10.0, -6.0, 0.0),
                Vector3::new(10.0, -6.0, 0.0),
                Vector3::new(24.0, 0.0, 0.0),
            ],
        ),
    ] {
        let terrain = flat_terrain(97, 97);
        let mut graph = RegionGraph::new();
        let start = graph.add_node(points[0], NodeType::Junction);
        let end = graph.add_node(*points.last().unwrap(), NodeType::Junction);
        let edge_idx = graph.add_edge(test_edge(
            start,
            end,
            points,
            10.0,
            edge_class,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        graph.rebuild_intersection_clips();

        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.compile_dirty(&graph, &terrain);
        let span_piece = surface
            .compiled_visual_span_pieces()
            .get(&edge_idx)
            .unwrap_or_else(|| panic!("{case_name}: span should compile"));
        assert!(!span_piece.span_earthwork_support_regions.is_empty());
        assert_span_earthwork_faces_have_support_provenance(span_piece, edge_idx, edge_class);
        assert!(
            span_piece
                .span_earthwork_support_regions
                .iter()
                .all(|region| !(region.start_s_m < 24.0 && region.end_s_m > 24.0)),
            "{case_name}: support regions must stay out of the midspan"
        );
        let (road_loops, source_count) = surface
            .terrain_cdt_road_loops_for_world_bounds(&graph, -8.0, -12.0, 8.0, 12.0)
            .expect("bridge/tunnel midspan query should not fail terrain clip export");
        assert!(
            road_loops.is_empty() && source_count == 0,
            "{case_name}: bridge/tunnel midspans must not feed road-touched terrain CDT"
        );
    }
}

fn terrain_clip_source_edge_for_node_test(
    start: Vector3,
    end: Vector3,
    node_id: u32,
) -> RoadSurfaceTerrainClipSourceEdge {
    terrain_clip_source_edge_for_node_kind_test(
        start,
        end,
        node_id,
        RoadSurfaceTerrainClipEdgeKind::SidewalkOuter,
    )
}

fn terrain_clip_source_edge_for_node_kind_test(
    start: Vector3,
    end: Vector3,
    node_id: u32,
    edge_kind: RoadSurfaceTerrainClipEdgeKind,
) -> RoadSurfaceTerrainClipSourceEdge {
    RoadSurfaceTerrainClipSourceEdge {
        start,
        end,
        kind: edge_kind,
        source: RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
            node_id,
            kind: RoadSurfaceVisualNodePieceKind::Terminal,
            owner_kind: RoadSurfaceBandKind::Sidewalk,
            owner_index: 0,
            boundary_source: None,
        },
    }
}

fn terrain_clip_loop_for_node_test(points: &[Vector3], node_id: u32) -> RoadSurfaceTerrainClipLoop {
    terrain_clip_loop_for_node_kind_test(
        points,
        node_id,
        RoadSurfaceTerrainClipEdgeKind::SidewalkOuter,
    )
}

fn terrain_clip_loop_for_node_kind_test(
    points: &[Vector3],
    node_id: u32,
    edge_kind: RoadSurfaceTerrainClipEdgeKind,
) -> RoadSurfaceTerrainClipLoop {
    RoadSurfaceTerrainClipLoop {
        source_edges: points
            .iter()
            .zip(points.iter().cycle().skip(1))
            .take(points.len())
            .map(|(&start, &end)| {
                terrain_clip_source_edge_for_node_kind_test(start, end, node_id, edge_kind)
            })
            .collect(),
        points_world: points.to_vec(),
    }
}
