// SPDX-License-Identifier: GPL-2.0-only

//! Junction ownership and overlay tests.

use super::*;

#[test]
fn junction_node_non_road_surface_is_footprint_minus_asphalt() {
    let mut graph = RegionGraph::new();
    let left = graph.add_node(Vector3::new(-24.0, 0.0, 0.0), NodeType::Junction);
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let right = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
    let up = graph.add_node(Vector3::new(0.0, 0.0, 24.0), NodeType::Junction);
    graph.add_edge(test_edge(
        left,
        center,
        vec![Vector3::new(-24.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        right,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        up,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 24.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_adjacency_list();

    let terrain = flat_terrain(96, 96);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    assert_compiled_junction_piece(&surface, &graph, center);
}

#[test]
fn footpath_join_claims_only_the_near_sidewalk_side() {
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-32.0, 0.0, 0.0), NodeType::Junction);
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let east = graph.add_node(Vector3::new(32.0, 0.0, 0.0), NodeType::Junction);
    let north = graph.add_node(Vector3::new(0.0, 0.0, 32.0), NodeType::Junction);
    graph.add_edge(test_edge(
        west,
        center,
        vec![Vector3::new(-32.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        east,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(32.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        north,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 32.0)],
        3.0,
        EdgeClass::Standard,
        TransitType::Foot,
        TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();
    graph.rebuild_adjacency_list();

    let terrain = flat_terrain(128, 128);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let piece = assert_compiled_junction_piece(&surface, &graph, center);
    let footpath_regions = piece
        .owned_regions
        .iter()
        .filter(|region| region.kind == RoadSurfaceBandKind::Footpath)
        .collect::<Vec<_>>();
    assert!(
        !footpath_regions.is_empty(),
        "foot-only incident edges must export explicit footpath-owned node regions"
    );

    let footpath_centroids = footpath_regions
        .iter()
        .flat_map(|region| {
            region
                .polygon
                .triangles_world
                .iter()
                .copied()
                .map(triangle_centroid_xz)
        })
        .collect::<Vec<_>>();
    assert!(
        footpath_centroids.iter().any(|centroid| centroid.y > 2.0),
        "the footpath join must reach the sidewalk side it connects to; centroids={footpath_centroids:?}"
    );
    assert!(
        footpath_centroids
            .iter()
            .all(|centroid| centroid.y >= -0.25),
        "the footpath join must not mirror across the road into the opposite sidewalk; centroids={footpath_centroids:?}"
    );

    let footpath_polygons = footpath_regions
        .iter()
        .map(|region| region.polygon.clone())
        .collect::<Vec<_>>();
    assert!(
        point_inside_visual_polygons(&footpath_polygons, Vector2::new(0.0, 7.0)),
        "the near sidewalk side must be connected to the footpath-owned region"
    );
    assert!(
        !point_inside_visual_polygons(&footpath_polygons, Vector2::new(0.0, -7.0)),
        "the opposite sidewalk side must keep its own owner instead of being claimed by the footpath"
    );
}

#[test]
fn node_overlay_preserves_skinny_closure_shapes() {
    let shapes = RoadSurfaceSystem::overlay_union_contours(&[vec![
        [0.0, 0.0],
        [2.0, 0.0],
        [2.0, 0.0005],
        [0.0, 0.0005],
    ]])
    .unwrap();

    assert_eq!(
        shapes.len(),
        1,
        "millimetre-scale deterministic closure slivers must not be filtered before rendering"
    );
}
