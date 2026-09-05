// SPDX-License-Identifier: GPL-2.0-only

//! Terrain clip loop export tests.

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
