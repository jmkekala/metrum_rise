// SPDX-License-Identifier: GPL-2.0-only

//! Guide cache parity and invalidation regressions using the real mesh publication path.

use super::*;

fn guide_core() -> SimCore {
    let mut core = test_core();
    for x in [-1_000.0, 900.0] {
        let start = Vector3::new(x, 0.0, 64.0);
        let end = Vector3::new(x + 100.0, 0.0, 64.0);
        let a = core.region_graph.add_node(start, NodeType::Junction);
        let b = core.region_graph.add_node(end, NodeType::Junction);
        core.region_graph
            .add_edge(test_road_edge(a, b, vec![start, end]));
    }
    core.region_graph.rebuild_adjacency_list();
    core.region_graph.rebuild_intersection_clips();
    core.mark_network_render_dirty();
    core.precompute_road_mesh_data();
    assert!(core.acknowledge_network_render_generation(core.road_tool_surface_generation));
    core.refresh_road_ghost_lines();
    assert_eq!(core.road_ghost_lines.rebuilt_edges, 2);
    core
}

fn assert_fresh_parity(core: &mut SimCore) {
    let vertices = core.road_ghost_lines.vertices.clone();
    let colors = core.road_ghost_lines.colors.clone();
    core.road_ghost_lines = Default::default();
    core.refresh_road_ghost_lines();
    assert_eq!(core.road_ghost_lines.vertices, vertices);
    assert_eq!(core.road_ghost_lines.colors, colors);
}

#[test]
fn guide_cache_reuses_distant_edges_and_matches_fresh_output() {
    let mut core = guide_core();
    core.refresh_road_ghost_lines();
    assert_eq!(core.road_ghost_lines.rebuilt_edges, 0);
    let before = core.road_ghost_lines.vertices.clone();

    // Change nearby visible height without changing either guide's source geometry.
    let a = Vector3::new(-980.0, 2.0, 144.0);
    let b = Vector3::new(-920.0, 2.0, 144.0);
    let start = core.region_graph.add_node(a, NodeType::Junction);
    let end = core.region_graph.add_node(b, NodeType::Junction);
    let id = core
        .region_graph
        .add_edge(test_road_edge(start, end, vec![a, b]));
    core.region_graph.rebuild_adjacency_list();
    core.region_graph.rebuild_intersection_clips();
    core.transit_network
        .road_surface
        .mark_edge_dirty(&core.region_graph, id);
    core.mark_local_network_render_dirty();
    core.precompute_road_mesh_data();
    core.refresh_road_ghost_lines();
    assert_eq!(
        core.road_ghost_lines.rebuilt_edges, 2,
        "near guide and new road only"
    );
    assert_ne!(&core.road_ghost_lines.vertices[..before.len()], before);
    assert_fresh_parity(&mut core);
}

#[test]
fn guide_cache_accumulates_edits_and_removes_deleted_slots() {
    let mut core = guide_core();
    // Two mesh publications without fetching guides must not lose the first invalidation.
    for id in 0..2 {
        core.transit_network
            .road_surface
            .mark_edge_dirty(&core.region_graph, id);
        core.mark_local_network_render_dirty();
        core.precompute_road_mesh_data();
    }
    core.refresh_road_ghost_lines();
    assert_eq!(core.road_ghost_lines.rebuilt_edges, 2);
    assert_fresh_parity(&mut core);

    core.region_graph.edge_mut(0).deleted = true;
    core.region_graph.rebuild_adjacency_list();
    core.transit_network
        .road_surface
        .mark_edge_dirty(&core.region_graph, 0);
    core.mark_local_network_render_dirty();
    core.precompute_road_mesh_data();
    core.refresh_road_ghost_lines();
    assert_eq!(
        core.road_ghost_lines.rebuilt_edges, 0,
        "distant edge still reused"
    );
    assert_fresh_parity(&mut core);
}

#[test]
fn guide_cache_tracks_terrain_patches_brushes_and_world_reset() {
    let mut core = guide_core();
    let patches = core
        .heightmap
        .render_patch_keys_for_world_bounds(-980.0, 140.0, -920.0, 150.0);
    core.bump_terrain_payload_patch_generations(&patches);
    core.refresh_road_ghost_lines();
    assert_eq!(core.road_ghost_lines.rebuilt_edges, 1);
    assert_fresh_parity(&mut core);

    // Source brushes invalidate independently of road geometry or mesh publication.
    let (x, z) = core.heightmap.world_to_grid_coords(-950.0, 144.0);
    core.heightmap.set_height(x as usize, z as usize, 2.0);
    core.refresh_road_ghost_lines();
    assert_eq!(core.road_ghost_lines.rebuilt_edges, 2);
    assert_fresh_parity(&mut core);

    core.mark_network_render_dirty();
    core.precompute_road_mesh_data();
    core.refresh_road_ghost_lines();
    assert_eq!(core.road_ghost_lines.rebuilt_edges, 2);
    assert_fresh_parity(&mut core);
}
