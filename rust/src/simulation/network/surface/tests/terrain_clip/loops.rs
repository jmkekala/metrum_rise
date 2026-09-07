// SPDX-License-Identifier: GPL-2.0-only

//! Terrain clip loop export tests.

use super::*;

#[test]
fn long_road_terrain_exports_stay_local_and_preserve_sources() {
    let terrain = TerrainSystem::with_chunking(2041, 2041, 10.0, 51, 100.0);
    let mut graph = RegionGraph::new();
    let a = Vector3::new(2048.0, 100.0, 2048.0);
    let b = Vector3::new(4838.0, 100.0, 2048.0);
    let start = graph.add_node(a, NodeType::Junction);
    let end = graph.add_node(b, NodeType::Junction);
    graph.add_edge(crate::simulation::network::build_surface_edge(
        start,
        end,
        vec![a, b],
        1,
        1,
        EdgeClass::Standard,
    ));
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();
    let mut surface = RoadSurfaceSystem::new(512.0);
    assert!(surface.compile_dirty(&graph, &terrain));
    for min_x in [1466.0, 1976.0, 2486.0, 2996.0, 3506.0, 4016.0, 4526.0] {
        let (loops, _) = surface
            .terrain_cdt_road_loops_for_world_bounds(&graph, min_x, 1976.0, min_x + 638.0, 2614.0)
            .expect("ROAD-14: a patch query must not union a whole 2790 m footprint");
        assert!(!loops.is_empty());
        for road_loop in loops {
            assert!(!road_loop.source_edges.is_empty());
            for point in road_loop.vertices {
                assert!((point.height_m - 100.0).abs() < 1.0);
                assert!(point.x >= f64::from(min_x) - 128.0 && point.x <= f64::from(min_x) + 766.0);
            }
        }
    }
}

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
