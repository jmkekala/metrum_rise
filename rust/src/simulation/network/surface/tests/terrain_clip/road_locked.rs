//! Road-locked terrain patch selection tests.

use super::*;

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

    let mut footprint_min_x = f64::MAX;
    let mut footprint_max_x = f64::MIN;
    let mut footprint_min_z = f64::MAX;
    let mut footprint_max_z = f64::MIN;
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

    let keys = surface.terrain_render_patch_keys_with_visible_road_margin(&graph, &terrain, 0.0);
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
            f64::from(patch.world_origin_x) <= footprint_max_x
                && f64::from(patch_max_x) >= footprint_min_x
                && f64::from(patch.world_origin_z) <= footprint_max_z
                && f64::from(patch_max_z) >= footprint_min_z,
            "road-locked patch ({patch_x}, {patch_z}) must overlap the road footprint, not only the earthwork envelope"
        );
    }
}

#[test]
fn road_locked_terrain_patches_expand_for_cdt_seam_margin() {
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

    let unexpanded =
        surface.terrain_render_patch_keys_with_visible_road_margin(&graph, &terrain, 0.0);
    let expanded =
        surface.terrain_render_patch_keys_with_visible_road_margin(&graph, &terrain, 8.0);
    assert!(
        unexpanded.iter().all(|key| expanded.contains(key)),
        "seam-expanded road-locked patch selection must preserve visible road patches"
    );
    assert!(
        expanded.len() > unexpanded.len(),
        "local CDT seam margin should road-lock neighboring patches before a patch-edge crack can form"
    );
}

#[test]
fn road_locked_terrain_patches_skip_bridge_and_tunnel_only_surfaces() {
    let terrain = flat_terrain(257, 257);
    let mut graph = RegionGraph::new();
    let bridge_start = graph.add_node(Vector3::new(-32.0, 8.0, -48.0), NodeType::Junction);
    let bridge_end = graph.add_node(Vector3::new(-32.0, 8.0, 48.0), NodeType::Junction);
    graph.add_edge(test_edge(
        bridge_start,
        bridge_end,
        vec![
            Vector3::new(-32.0, 8.0, -48.0),
            Vector3::new(-32.0, 8.0, 48.0),
        ],
        10.0,
        EdgeClass::Bridge,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    let tunnel_start = graph.add_node(Vector3::new(32.0, -8.0, -48.0), NodeType::Junction);
    let tunnel_end = graph.add_node(Vector3::new(32.0, -8.0, 48.0), NodeType::Junction);
    graph.add_edge(test_edge(
        tunnel_start,
        tunnel_end,
        vec![
            Vector3::new(32.0, -8.0, -48.0),
            Vector3::new(32.0, -8.0, 48.0),
        ],
        10.0,
        EdgeClass::Tunnel,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let keys = surface.terrain_render_patch_keys_with_visible_road_margin(&graph, &terrain, 0.0);
    assert!(
        keys.is_empty(),
        "bridge/tunnel-only surfaces must not request grounded-road CDT terrain clips"
    );
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
