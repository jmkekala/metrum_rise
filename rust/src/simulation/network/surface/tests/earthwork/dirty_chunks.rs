// SPDX-License-Identifier: GPL-2.0-only

//! Earthwork dirty chunk coverage tests.

use super::*;

#[test]
fn dirty_terrain_earthworks_stay_bounded_to_touched_chunks() {
    let mut terrain = flat_terrain(161, 65);
    let mut graph = RegionGraph::new();
    let left_a = graph.add_node(Vector3::new(-56.0, 0.0, 0.0), NodeType::Junction);
    let left_b = graph.add_node(Vector3::new(-24.0, 0.0, 0.0), NodeType::Junction);
    let right_a = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
    let right_b = graph.add_node(Vector3::new(56.0, 0.0, 0.0), NodeType::Junction);
    let left_edge = graph.add_edge(test_edge(
        left_a,
        left_b,
        vec![Vector3::new(-56.0, 0.0, 0.0), Vector3::new(-24.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        right_a,
        right_b,
        vec![Vector3::new(24.0, 0.0, 0.0), Vector3::new(56.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.rebuild_all_earthworks(&graph, &mut terrain);
    let far_before = terrain.sample_visual_height_world(40.0, 0.0) * crate::config::HEIGHT_SCALE;

    surface.mark_edge_dirty(&graph, left_edge);
    let stamped_chunks = surface.rebuild_dirty_earthworks(&graph, &mut terrain);
    let far_after = terrain.sample_visual_height_world(40.0, 0.0) * crate::config::HEIGHT_SCALE;
    let right_chunk = surface.chunk_coords_for_world(40.0, 0.0);

    assert!(!stamped_chunks.is_empty());
    assert!(!stamped_chunks.contains(&right_chunk));
    assert!((far_after - far_before).abs() <= 0.001);
}

#[test]
fn compile_dirty_derives_edge_chunks_from_compiled_piece_coverage() {
    let terrain = flat_terrain(64, 64);
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(5.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(25.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        n0,
        n1,
        vec![Vector3::new(5.0, 0.0, 0.0), Vector3::new(25.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(10.0);
    surface.compile_dirty(&graph, &terrain);

    let surface_chunks = surface
        .surface_span_chunks
        .get(&edge_idx)
        .expect("compiled span must own surface chunks")
        .clone();
    let terrain_chunks = surface
        .earthwork_span_chunks
        .get(&edge_idx)
        .expect("compiled span must own terrain chunks")
        .clone();
    assert!(!surface_chunks.is_empty());
    assert!(terrain_chunks.len() >= surface_chunks.len());

    surface.mark_edge_dirty(&graph, edge_idx);
    surface.compile_dirty(&graph, &terrain);

    for chunk in surface_chunks {
        let entry = surface
            .surface_chunk_cache
            .get(&chunk)
            .unwrap_or_else(|| panic!("surface chunk {chunk:?} must be rebuilt"));
        assert!(entry.edge_indices.contains(&edge_idx));
    }
    for chunk in terrain_chunks {
        let entry = surface
            .earthwork_chunk_cache
            .get(&chunk)
            .unwrap_or_else(|| panic!("terrain chunk {chunk:?} must be rebuilt"));
        assert!(entry.edge_indices.contains(&edge_idx));
    }
}
