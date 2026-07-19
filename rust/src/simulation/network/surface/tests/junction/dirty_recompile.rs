//! Junction dirty-recompile cache tests.

use super::*;

fn flat_four_way_junction() -> (RegionGraph, u32, Vec<u32>, Vec<usize>) {
    let mut graph = RegionGraph::new();
    let center_pos = Vector3::ZERO;
    let center = graph.add_node(center_pos, NodeType::Junction);
    let mut endpoints = Vec::new();
    let mut edge_ids = Vec::new();
    for (endpoint_pos, starts_at_center) in [
        (Vector3::new(-80.0, 0.0, 0.0), false),
        (Vector3::new(80.0, 0.0, 0.0), true),
        (Vector3::new(0.0, 0.0, -80.0), false),
        (Vector3::new(0.0, 0.0, 80.0), true),
    ] {
        let endpoint = graph.add_node(endpoint_pos, NodeType::Junction);
        let (start, end, points) = if starts_at_center {
            (center, endpoint, vec![center_pos, endpoint_pos])
        } else {
            (endpoint, center, vec![endpoint_pos, center_pos])
        };
        endpoints.push(endpoint);
        edge_ids.push(graph.add_edge(test_edge(
            start,
            end,
            points,
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        )));
    }
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();
    (graph, center, endpoints, edge_ids)
}

fn assert_node_export_sources_bind_current_tables(piece: &RoadSurfaceVisualNodePiece) {
    for source in &piece.raised_step_face_sources {
        let index = source
            .explicit_vertical_step_index()
            .expect("raised-step source must identify its current segment");
        assert_eq!(
            piece.explicit_vertical_step_segments.get(index),
            Some(&source.segment()),
            "cached raised-step geometry must bind the current segment index"
        );
    }
    for source in &piece.node_top_surface_sources {
        for (vertex_key, vertex_source) in source.vertex_keys.iter().zip(&source.vertex_sources) {
            let authority = piece
                .node_grade_authorities
                .get(vertex_source.grade_authority_index)
                .expect("top vertex source must bind the current authority table");
            assert_eq!(
                authority.key.raw_tuple(),
                (vertex_key.x_key(), vertex_key.z_key()),
                "cached top geometry must not retain a previous authority index"
            );
        }
    }
}

#[test]
fn exact_node_export_rebuild_reuses_semantic_products_and_matches_cold() {
    let (graph, center, _, _) = flat_four_way_junction();
    let terrain = flat_terrain(128, 128);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let input = surface
        .compiled_visual_node_inputs
        .get(&center)
        .expect("cold four-way compilation must retain its node input")
        .clone();
    let topology = surface
        .compiled_visual_node_topologies
        .get(&center)
        .expect("cold four-way compilation must retain semantic caches")
        .clone();

    let warm = surface
        .compile_visual_node_piece_with_earthwork_boundaries(
            &graph,
            &terrain,
            center,
            &input,
            Some(topology.as_ref()),
        )
        .expect("exact semantic export replay must compile");
    let cold = surface
        .compile_visual_node_piece_with_earthwork_boundaries(&graph, &terrain, center, &input, None)
        .expect("independent cold export must compile");

    assert_eq!(warm.piece, cold.piece);
    assert_eq!(warm.earthwork_boundaries, cold.earthwork_boundaries);
    assert_node_export_sources_bind_current_tables(&warm.piece);
    assert!(warm.export_reuse_stats.explicit_step_previous_hits > 0);
    assert!(warm.export_reuse_stats.height_split_previous_hits > 0);
    assert!(warm.export_reuse_stats.top_edge_previous_hits > 0);
    assert!(warm.export_reuse_stats.raised_step_previous_hits > 0);
    assert_eq!(cold.export_reuse_stats.explicit_step_previous_hits, 0);
    assert_eq!(cold.export_reuse_stats.height_split_previous_hits, 0);
    assert_eq!(cold.export_reuse_stats.top_edge_previous_hits, 0);
    assert_eq!(cold.export_reuse_stats.raised_step_previous_hits, 0);
    assert!(cold.export_reuse_stats.explicit_step_misses > 0);
    assert!(
        cold.export_reuse_stats.explicit_step_pair_misses < 10_000,
        "the fixed four-way cold build must stay below the former global all-pairs scan: {:?}",
        cold.export_reuse_stats
    );
    assert!(cold.export_reuse_stats.height_split_misses > 0);
    assert!(cold.export_reuse_stats.top_edge_cache_misses > 0);
    assert!(cold.export_reuse_stats.raised_step_cache_misses > 0);
}

fn set_four_way_endpoint_heights(
    graph: &mut RegionGraph,
    center: u32,
    endpoints: &[u32],
    edge_ids: &[usize],
    heights_m: [f32; 4],
) {
    for (&endpoint, height_m) in endpoints.iter().zip(heights_m) {
        let mut pos = graph.node(endpoint).pos;
        pos.y = height_m;
        graph.set_node_pos(endpoint, pos);
    }
    for &edge_idx in edge_ids {
        let edge = graph.edge(edge_idx);
        let points = vec![
            graph.node(edge.start_node).pos,
            graph.node(edge.end_node).pos,
        ];
        let edge = graph.edge_mut(edge_idx);
        edge.geometry = points.clone();
        edge.physical_geometry = points;
    }
    graph.solve_junction_endpoint_profiles_for_edges(
        &HashSet::from([center]),
        &edge_ids.iter().copied().collect(),
    );
    graph.rebuild_intersection_clips();
}

fn assert_all_spans_compile_but_junction_fails(
    graph: &RegionGraph,
    terrain: &TerrainSystem,
    center: u32,
    edge_ids: &[usize],
) {
    let mut staging = RoadSurfaceSystem::new(16.0);
    for &edge_idx in edge_ids {
        staging
            .compiled_sections
            .insert(edge_idx, staging.compile_edge_sections(graph, edge_idx));
    }
    for &edge_idx in edge_ids {
        let piece = staging
            .compile_visual_span_piece(graph, terrain, edge_idx)
            .expect("every incident span must compile before the node-only failure");
        staging.apply_span_compile_result(edge_idx, Some(piece));
    }
    let input = staging
        .visual_node_compile_input(graph, center)
        .expect("conflicting four-way fixture must produce a JunctionN input");
    assert_eq!(input.mouths.len(), edge_ids.len());
    assert!(
        staging
            .compile_visual_node_piece_with_earthwork_boundaries(
                graph, terrain, center, &input, None,
            )
            .is_none(),
        "the conflicting fixture must fail at node compilation after all spans succeed"
    );
}

#[test]
fn terrain_only_junction_recompile_reuses_canonical_topology_and_refreshes_earthwork() {
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::new(0.0, 4.0, 0.0), NodeType::Junction);
    for endpoint in [
        Vector3::new(-32.0, 4.0, 0.0),
        Vector3::new(32.0, 4.0, 0.0),
        Vector3::new(0.0, 4.0, 32.0),
    ] {
        let endpoint_node = graph.add_node(endpoint, NodeType::Junction);
        graph.add_edge(test_edge(
            center,
            endpoint_node,
            vec![Vector3::new(0.0, 4.0, 0.0), endpoint],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
    }
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();

    let mut terrain = flat_terrain(96, 96);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let before = surface
        .compiled_visual_node_pieces()
        .get(&center)
        .expect("three-way junction should compile")
        .clone();
    assert!(
        surface
            .compiled_visual_node_earthwork_boundaries
            .contains_key(&center),
        "fresh node compilation must retain the terrain-independent earthwork boundary"
    );

    for z in 0..terrain.height {
        for x in 0..terrain.width {
            terrain.set_height(x, z, 0.1);
        }
    }
    surface.mark_node_dirty(&graph, center);
    surface.mark_world_aabb_dirty(
        Vector3::new(-20.0, 0.0, -20.0),
        Vector3::new(20.0, 0.0, 20.0),
    );
    surface.compile_dirty(&graph, &terrain);

    assert_eq!(
        surface.last_reused_node_topology_count, 1,
        "the dirty JunctionN must bypass rail contacts and boolean ownership when its compile input is unchanged"
    );
    let after = surface
        .compiled_visual_node_pieces()
        .get(&center)
        .expect("terrain refresh must preserve the compiled junction");
    assert_eq!(after.kind, RoadSurfaceVisualNodePieceKind::JunctionN);
    assert_eq!(after.outer_boundary_loops, before.outer_boundary_loops);
    assert_eq!(after.road_surface_polygons, before.road_surface_polygons);
    assert_eq!(after.curb_surface_polygons, before.curb_surface_polygons);
    assert_eq!(
        after.raised_step_face_polygons,
        before.raised_step_face_polygons
    );
    assert_eq!(
        after.sidewalk_surface_polygons,
        before.sidewalk_surface_polygons
    );
    assert_eq!(after.owned_regions, before.owned_regions);
    assert_ne!(
        after.render_earthwork_faces, before.render_earthwork_faces,
        "terrain-dependent earthwork must still be rebuilt against the edited heightmap"
    );
}

#[test]
fn bend_cache_can_seed_junction_topology_change_without_changing_output() {
    let terrain = flat_terrain(96, 96);
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::ZERO, NodeType::Junction);
    for endpoint in [Vector3::new(36.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 36.0)] {
        let endpoint_node = graph.add_node(endpoint, NodeType::Junction);
        graph.add_edge(test_edge(
            center,
            endpoint_node,
            vec![Vector3::ZERO, endpoint],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
    }
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();

    let mut warm = RoadSurfaceSystem::new(16.0);
    warm.compile_dirty(&graph, &terrain);
    assert_eq!(
        warm.compiled_visual_node_pieces[&center].kind,
        RoadSurfaceVisualNodePieceKind::Bend
    );
    let bend_topology = warm
        .compiled_visual_node_topologies
        .get(&center)
        .expect("successful Bend compilation must publish its contributor topology")
        .clone();

    let west = graph.add_node(Vector3::new(-36.0, 0.0, 0.0), NodeType::Junction);
    let west_edge = graph.add_edge(test_edge(
        west,
        center,
        vec![Vector3::new(-36.0, 0.0, 0.0), Vector3::ZERO],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();

    let mut cold = RoadSurfaceSystem::new(16.0);
    cold.compile_dirty(&graph, &terrain);
    let junction_input = cold.compiled_visual_node_inputs[&center].clone();
    assert_eq!(
        junction_input.kind,
        RoadSurfaceVisualNodePieceKind::JunctionN
    );
    let seeded = cold
        .compile_visual_node_piece_with_earthwork_boundaries(
            &graph,
            &terrain,
            center,
            &junction_input,
            Some(bend_topology.as_ref()),
        )
        .expect("JunctionN compilation must accept the prior Bend contributor cache");
    let independent = cold
        .compile_visual_node_piece_with_earthwork_boundaries(
            &graph,
            &terrain,
            center,
            &junction_input,
            None,
        )
        .expect("independent cold JunctionN compilation must succeed");
    assert_eq!(
        seeded.piece, independent.piece,
        "a Bend-seeded JunctionN compile must match the cold canonical output"
    );
    assert_eq!(
        seeded.earthwork_boundaries, independent.earthwork_boundaries,
        "a Bend-seeded JunctionN compile must preserve cold boundary provenance"
    );
    assert!(
        seeded.export_reuse_stats.previous_hits() > 0,
        "a topology edit must retain unchanged semantic export contributors: {:?}",
        seeded.export_reuse_stats
    );
    assert!(
        seeded.export_reuse_stats.top_edge_previous_hits > 0,
        "a topology edit must reuse unchanged top-boundary contributors: {:?}",
        seeded.export_reuse_stats
    );
    assert!(
        seeded.export_reuse_stats.raised_step_previous_hits > 0,
        "a topology edit must reuse unchanged raised-step spans before rebinding their indices: {:?}",
        seeded.export_reuse_stats
    );
    assert!(
        seeded.export_reuse_stats.explicit_step_pair_misses
            < independent.export_reuse_stats.explicit_step_pair_misses,
        "a Bend-seeded topology edit must evaluate fewer local final-step pairs than a cold build: seeded={:?} cold={:?}",
        seeded.export_reuse_stats,
        independent.export_reuse_stats
    );
    assert!(
        seeded.export_reuse_stats.misses() > 0,
        "a topology edit must rebuild changed semantic export contributors: {:?}",
        seeded.export_reuse_stats
    );
    assert_node_export_sources_bind_current_tables(&seeded.piece);

    warm.mark_edge_dirty(&graph, west_edge);
    warm.mark_node_dirty(&graph, center);
    warm.mark_node_dirty(&graph, west);
    warm.compile_dirty(&graph, &terrain);
    assert_eq!(
        warm.compiled_visual_node_pieces.get(&center),
        cold.compiled_visual_node_pieces.get(&center),
        "the dirty Bend-to-JunctionN path must match an independent cold build"
    );
    assert_eq!(
        warm.compiled_visual_node_earthwork_boundaries.get(&center),
        cold.compiled_visual_node_earthwork_boundaries.get(&center),
        "the dirty topology transition must preserve cold boundary provenance"
    );
}

#[test]
fn failed_dirty_junction_compile_preserves_published_generation_and_pending_earthwork() {
    let (mut graph, center, endpoints, edge_ids) = flat_four_way_junction();
    let mut terrain = flat_terrain(96, 96);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    assert_eq!(
        surface
            .compiled_visual_node_pieces
            .get(&center)
            .expect("baseline JunctionN must compile")
            .kind,
        RoadSurfaceVisualNodePieceKind::JunctionN
    );
    let published = surface.clone();

    set_four_way_endpoint_heights(
        &mut graph,
        center,
        &endpoints,
        &edge_ids,
        [80.0, -80.0, 64.0, -64.0],
    );
    assert_all_spans_compile_but_junction_fails(&graph, &terrain, center, &edge_ids);
    for &edge_idx in &edge_ids {
        surface.mark_edge_dirty(&graph, edge_idx);
    }
    surface.mark_node_dirty(&graph, center);
    let pending_edges = surface.dirty_edges.clone();
    let pending_nodes = surface.dirty_nodes.clone();
    let pending_surface_chunks = surface.dirty_surface_chunks.clone();
    let pending_terrain_chunks = surface.dirty_terrain_chunks.clone();
    let pending_query_chunks = surface.dirty_query_chunks.clone();

    let rebuilt_chunks = surface.rebuild_dirty_earthworks(&graph, &mut terrain);

    assert!(
        rebuilt_chunks.is_empty(),
        "earthworks must not consume the previous generation after staged node failure"
    );
    assert!(surface.compile_generation_is_latched());
    assert!(!surface.published_generation_matches_source());
    assert_eq!(surface.dirty_edges, pending_edges);
    assert_eq!(surface.dirty_nodes, pending_nodes);
    assert_eq!(surface.dirty_surface_chunks, pending_surface_chunks);
    assert_eq!(surface.dirty_terrain_chunks, pending_terrain_chunks);
    assert_eq!(surface.dirty_query_chunks, pending_query_chunks);
    assert_eq!(surface.compiled_sections, published.compiled_sections);
    assert_eq!(
        surface.compiled_visual_span_pieces,
        published.compiled_visual_span_pieces
    );
    assert_eq!(
        surface.compiled_visual_node_pieces,
        published.compiled_visual_node_pieces
    );
    assert_eq!(
        surface.compiled_visual_node_inputs,
        published.compiled_visual_node_inputs
    );
    assert_eq!(
        surface.compiled_visual_node_earthwork_boundaries,
        published.compiled_visual_node_earthwork_boundaries
    );
    assert_eq!(surface.surface_span_chunks, published.surface_span_chunks);
    assert_eq!(surface.surface_node_chunks, published.surface_node_chunks);
    assert_eq!(
        surface.earthwork_span_chunks,
        published.earthwork_span_chunks
    );
    assert_eq!(
        surface.earthwork_node_chunks,
        published.earthwork_node_chunks
    );
    assert_eq!(surface.surface_chunk_cache, published.surface_chunk_cache);
    assert_eq!(
        surface.earthwork_chunk_cache,
        published.earthwork_chunk_cache
    );
    assert_eq!(
        surface.last_rebuilt_surface_chunks,
        published.last_rebuilt_surface_chunks
    );
    assert_eq!(
        surface.last_rebuilt_terrain_chunks,
        published.last_rebuilt_terrain_chunks
    );
    assert_eq!(
        surface.last_rebuilt_query_chunks,
        published.last_rebuilt_query_chunks
    );
    assert_eq!(
        surface.compiled_visual_node_topologies.len(),
        published.compiled_visual_node_topologies.len()
    );
    for (node_id, topology) in &published.compiled_visual_node_topologies {
        assert!(std::sync::Arc::ptr_eq(
            surface
                .compiled_visual_node_topologies
                .get(node_id)
                .expect("published topology must remain present"),
            topology
        ));
    }

    // A repeated consumer request for the same invalidation generation must not spend another
    // JunctionN compile. Even repairing the source silently cannot bypass the explicit latch.
    let failed_generation = surface.failed_compile_generation;
    set_four_way_endpoint_heights(&mut graph, center, &endpoints, &edge_ids, [0.0; 4]);
    surface.compile_dirty(&graph, &terrain);
    assert_eq!(surface.failed_compile_generation, failed_generation);
    assert_eq!(
        surface.compiled_visual_node_pieces,
        published.compiled_visual_node_pieces
    );

    // A newer explicit invalidation clears the latch and publishes the repaired generation.
    surface.mark_node_dirty(&graph, center);
    surface.compile_dirty(&graph, &terrain);
    assert!(surface.published_generation_matches_source());
    assert!(surface.failed_compile_generation.is_none());
}

#[test]
fn failed_initial_node_compile_publishes_nothing_until_newer_invalidation() {
    let (mut graph, center, endpoints, edge_ids) = flat_four_way_junction();
    let terrain = flat_terrain(96, 96);
    set_four_way_endpoint_heights(
        &mut graph,
        center,
        &endpoints,
        &edge_ids,
        [80.0, -80.0, 64.0, -64.0],
    );
    assert_all_spans_compile_but_junction_fails(&graph, &terrain, center, &edge_ids);

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    assert!(!surface.compiled_once);
    assert!(surface.compile_generation_is_latched());
    assert!(!surface.published_generation_matches_source());
    assert!(surface.compiled_sections.is_empty());
    assert!(surface.compiled_visual_span_pieces.is_empty());
    assert!(surface.compiled_visual_node_pieces.is_empty());
    assert!(surface.surface_chunk_cache.is_empty());
    assert!(surface.earthwork_chunk_cache.is_empty());

    set_four_way_endpoint_heights(&mut graph, center, &endpoints, &edge_ids, [0.0; 4]);
    surface.compile_dirty(&graph, &terrain);
    assert!(
        surface.compiled_visual_span_pieces.is_empty(),
        "silent source mutation must not retry a latched failed generation"
    );

    surface.mark_node_dirty(&graph, center);
    surface.compile_dirty(&graph, &terrain);
    assert!(surface.published_generation_matches_source());
    assert_eq!(surface.compiled_visual_span_pieces.len(), edge_ids.len());
    assert_eq!(
        surface
            .compiled_visual_node_pieces
            .get(&center)
            .expect("repaired initial generation must publish its JunctionN")
            .kind,
        RoadSurfaceVisualNodePieceKind::JunctionN
    );
}

#[test]
fn mesh_generation_refuses_a_failed_surface_generation() {
    let (mut graph, center, endpoints, edge_ids) = flat_four_way_junction();
    let terrain = flat_terrain(96, 96);
    let mut network = TransitNetwork::new_with_surface_chunk_span(16.0);
    assert!(
        network.try_generate_mesh_data(&graph, &terrain).is_some(),
        "baseline mesh generation must publish the flat JunctionN generation"
    );
    let published_nodes = network.road_surface.compiled_visual_node_pieces.clone();

    set_four_way_endpoint_heights(
        &mut graph,
        center,
        &endpoints,
        &edge_ids,
        [80.0, -80.0, 64.0, -64.0],
    );
    for &edge_idx in &edge_ids {
        network.road_surface.mark_edge_dirty(&graph, edge_idx);
    }
    network.road_surface.mark_node_dirty(&graph, center);

    assert!(
        network.try_generate_mesh_data(&graph, &terrain).is_none(),
        "renderer must not combine the prior published surface with the newer source graph"
    );
    assert_eq!(
        network.road_surface.compiled_visual_node_pieces, published_nodes,
        "failed mesh preparation must preserve the complete prior surface generation"
    );
}

#[test]
fn ordinary_dirty_and_topology_restore_evict_in_range_alias_node_caches() {
    let mut graph = RegionGraph::new();
    let canonical = graph.add_node(Vector3::new(-64.0, 4.0, -64.0), NodeType::Junction);
    let alias = graph.add_node(Vector3::new(0.0, 4.0, 0.0), NodeType::Junction);
    for endpoint in [
        Vector3::new(-32.0, 4.0, 0.0),
        Vector3::new(32.0, 4.0, 0.0),
        Vector3::new(0.0, 4.0, 32.0),
    ] {
        let endpoint_node = graph.add_node(endpoint, NodeType::Junction);
        graph.add_edge(test_edge(
            alias,
            endpoint_node,
            vec![graph.node(alias).pos, endpoint],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
    }
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(128, 128);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    assert!(surface.compiled_visual_node_pieces.contains_key(&alias));
    assert!(surface.compiled_visual_node_topologies.contains_key(&alias));
    let mut ordinary_surface = surface.clone();

    graph.unite_nodes(canonical, alias);
    ordinary_surface.mark_node_dirty(&graph, alias);
    surface.mark_topology_restore_dirty(&graph, &HashSet::new(), &HashSet::from([alias]));

    for dirty_surface in [&surface, &ordinary_surface] {
        assert!(dirty_surface.dirty_nodes.contains(&canonical));
        assert!(
            !dirty_surface
                .compiled_visual_node_pieces
                .contains_key(&alias)
        );
        assert!(
            !dirty_surface
                .compiled_visual_node_inputs
                .contains_key(&alias)
        );
        assert!(
            !dirty_surface
                .compiled_visual_node_earthwork_boundaries
                .contains_key(&alias)
        );
        assert!(
            !dirty_surface
                .compiled_visual_node_topologies
                .contains_key(&alias)
        );
        assert!(!dirty_surface.surface_node_chunks.contains_key(&alias));
        assert!(!dirty_surface.earthwork_node_chunks.contains_key(&alias));
    }
}

#[test]
fn incident_edge_class_change_invalidates_whole_node_reuse() {
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let mut edge_ids = Vec::new();
    for endpoint in [Vector3::new(32.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 32.0)] {
        let endpoint_id = graph.add_node(endpoint, NodeType::Junction);
        edge_ids.push(graph.add_edge(test_edge(
            center,
            endpoint_id,
            vec![graph.node(center).pos, endpoint],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        )));
    }
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(96, 96);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let changed_edge = edge_ids[1];
    assert!(
        surface.compiled_visual_node_pieces[&center]
            .earthwork_owner_sources
            .iter()
            .any(|source| {
                source.edge_idx == changed_edge && source.edge_class == EdgeClass::Standard
            })
    );

    graph.edge_mut(changed_edge).class = EdgeClass::Bridge;
    surface.mark_node_dirty(&graph, center);
    surface.compile_dirty(&graph, &terrain);

    assert!(
        surface.dirty_edges.is_empty() && surface.dirty_nodes.is_empty(),
        "the edge-class generation must compile before its node provenance can be inspected"
    );
    let changed_input = &surface.compiled_visual_node_inputs[&center];
    let mouth_index = changed_input
        .mouths
        .iter()
        .position(|mouth| mouth.edge_idx == changed_edge)
        .expect("changed incident edge must retain a node mouth");
    assert_eq!(
        changed_input.mouth_edge_classes[mouth_index],
        EdgeClass::Bridge
    );
    let changed_piece = &surface.compiled_visual_node_pieces[&center];
    assert!(
        changed_piece.earthwork_owner_sources.iter().any(|source| {
            source.edge_idx == changed_edge && source.edge_class == EdgeClass::Bridge
        }),
        "edge-class changes must refresh node earthwork provenance instead of reusing the whole cached piece"
    );
    assert!(!changed_piece.earthwork_owner_sources.iter().any(|source| {
        source.edge_idx == changed_edge && source.edge_class == EdgeClass::Standard
    }));
}

#[test]
fn graded_elevated_junction_height_topology_reuse_matches_cold_and_xz_change_misses() {
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::new(0.0, 6.0, 0.0), NodeType::Junction);
    let endpoints = [
        graph.add_node(Vector3::new(-36.0, 4.0, 0.0), NodeType::Junction),
        graph.add_node(Vector3::new(36.0, 7.0, 0.0), NodeType::Junction),
        graph.add_node(Vector3::new(0.0, 5.0, 36.0), NodeType::Junction),
        graph.add_node(Vector3::new(0.0, 8.0, -36.0), NodeType::Junction),
    ];
    let mut edge_ids = Vec::new();
    for endpoint in endpoints {
        edge_ids.push(graph.add_edge(test_edge(
            center,
            endpoint,
            vec![graph.node(center).pos, graph.node(endpoint).pos],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        )));
    }
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(128, 128);
    let mut warm = RoadSurfaceSystem::new(16.0);
    warm.compile_dirty(&graph, &terrain);
    assert!(
        warm.compiled_visual_node_topologies.contains_key(&center),
        "cold JunctionN compilation must publish reusable plan topology"
    );

    for (node_id, delta_y) in std::iter::once(center).chain(endpoints).zip([1.25; 5]) {
        let mut point = graph.node(node_id).pos;
        point.y += delta_y;
        graph.set_node_pos(node_id, point);
    }
    for edge_idx in edge_ids.iter().copied() {
        let edge = graph.edge(edge_idx);
        let start = graph.node(edge.start_node).pos;
        let end = graph.node(edge.end_node).pos;
        let edge = graph.edge_mut(edge_idx);
        edge.geometry = vec![start, end];
        edge.physical_geometry = vec![start, end];
    }
    graph.rebuild_intersection_clips();
    warm.mark_node_dirty(&graph, center);
    warm.compile_dirty(&graph, &terrain);

    assert_eq!(
        warm.last_reused_node_height_topology_count, 1,
        "Y-only JunctionN input changes must reuse rail/contact topology"
    );
    assert_eq!(
        warm.last_reused_node_ownership_topology_count, 0,
        "ownership must rebuild when canonical millimetre deltas are not exactly uniform"
    );
    let mut cold = RoadSurfaceSystem::new(16.0);
    cold.compile_dirty(&graph, &terrain);
    assert_eq!(
        warm.compiled_visual_node_pieces().get(&center),
        cold.compiled_visual_node_pieces().get(&center),
        "height-topology reuse must be byte-for-byte identical to a forced cold compile"
    );
    assert_eq!(
        warm.compiled_visual_node_earthwork_boundaries.get(&center),
        cold.compiled_visual_node_earthwork_boundaries.get(&center),
        "reused and cold boundary provenance must match"
    );

    let changed_edge = edge_ids[0];
    let changed_endpoint = graph.edge(changed_edge).end_node;
    let mut moved_endpoint = graph.node(changed_endpoint).pos;
    moved_endpoint.x += 1.0;
    graph.set_node_pos(changed_endpoint, moved_endpoint);
    {
        let edge = graph.edge_mut(changed_edge);
        edge.geometry.last_mut().expect("edge has endpoint").x += 1.0;
        edge.physical_geometry
            .last_mut()
            .expect("edge has physical endpoint")
            .x += 1.0;
    }
    graph.rebuild_intersection_clips();
    warm.mark_node_dirty(&graph, center);
    warm.compile_dirty(&graph, &terrain);

    assert_eq!(
        warm.last_reused_node_height_topology_count, 0,
        "changed canonical XZ keys must force a cold contact and ownership compile"
    );
    assert_eq!(
        warm.last_reused_node_ownership_topology_count, 0,
        "changed canonical XZ keys must not reuse boolean ownership"
    );
    let mut moved_cold = RoadSurfaceSystem::new(16.0);
    moved_cold.compile_dirty(&graph, &terrain);
    assert_eq!(
        warm.compiled_visual_node_pieces().get(&center),
        moved_cold.compiled_visual_node_pieces().get(&center),
        "the deterministic cold fallback must match an independent cold compile"
    );
}

#[test]
fn exact_uniform_mm_translation_reuses_ownership_topology() {
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::new(0.0, 4.0, 0.0), NodeType::Junction);
    let endpoints = [
        graph.add_node(Vector3::new(-36.0, 4.0, 0.0), NodeType::Junction),
        graph.add_node(Vector3::new(36.0, 4.0, 0.0), NodeType::Junction),
        graph.add_node(Vector3::new(0.0, 4.0, 36.0), NodeType::Junction),
        graph.add_node(Vector3::new(0.0, 4.0, -36.0), NodeType::Junction),
    ];
    let mut edge_ids = Vec::new();
    for endpoint in endpoints {
        edge_ids.push(graph.add_edge(test_edge(
            center,
            endpoint,
            vec![graph.node(center).pos, graph.node(endpoint).pos],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        )));
    }
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(128, 128);
    let mut warm = RoadSurfaceSystem::new(16.0);
    warm.compile_dirty(&graph, &terrain);

    for node_id in std::iter::once(center).chain(endpoints) {
        let mut point = graph.node(node_id).pos;
        point.y += 2.0;
        graph.set_node_pos(node_id, point);
    }
    for edge_idx in edge_ids {
        let edge = graph.edge(edge_idx);
        let start = graph.node(edge.start_node).pos;
        let end = graph.node(edge.end_node).pos;
        let edge = graph.edge_mut(edge_idx);
        edge.geometry = vec![start, end];
        edge.physical_geometry = vec![start, end];
    }
    graph.rebuild_intersection_clips();
    warm.mark_node_dirty(&graph, center);
    warm.compile_dirty(&graph, &terrain);

    assert_eq!(warm.last_reused_node_height_topology_count, 1);
    assert_eq!(
        warm.last_reused_node_ownership_topology_count, 1,
        "an exact uniform canonical-mm translation preserves every ownership height predicate"
    );
    let mut cold = RoadSurfaceSystem::new(16.0);
    cold.compile_dirty(&graph, &terrain);
    assert_eq!(
        warm.compiled_visual_node_pieces().get(&center),
        cold.compiled_visual_node_pieces().get(&center)
    );
    assert_eq!(
        warm.compiled_visual_node_earthwork_boundaries.get(&center),
        cold.compiled_visual_node_earthwork_boundaries.get(&center)
    );
}

#[test]
fn exact_xz_nonuniform_height_topology_reuse_matches_cold_compile() {
    fn adjust_point_height(point: &mut backend::RoadVec3) {
        point.y += 0.75 + 0.002 * point.x - 0.003 * point.z;
    }

    fn adjust_profile_heights(profile: &mut IncidentMouthProfile) {
        for point in &mut profile.boundary_points_world {
            adjust_point_height(point);
        }
        for band in &mut profile.bands {
            adjust_point_height(&mut band.start_point_world);
            adjust_point_height(&mut band.end_point_world);
        }
    }

    fn adjust_path_heights(paths: &mut [Vec<backend::RoadVec3>]) {
        for path in paths {
            for point in path {
                adjust_point_height(point);
            }
        }
    }

    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::new(0.0, 6.0, 0.0), NodeType::Junction);
    for endpoint in [
        Vector3::new(-36.0, 4.0, 0.0),
        Vector3::new(36.0, 7.0, 0.0),
        Vector3::new(0.0, 5.0, 36.0),
        Vector3::new(0.0, 8.0, -36.0),
    ] {
        let endpoint_node = graph.add_node(endpoint, NodeType::Junction);
        graph.add_edge(test_edge(
            center,
            endpoint_node,
            vec![graph.node(center).pos, endpoint],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
    }
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(128, 128);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let mut changed_input = surface
        .compiled_visual_node_inputs
        .get(&center)
        .expect("cold JunctionN compilation must retain its input")
        .clone();
    let topology = surface
        .compiled_visual_node_topologies
        .get(&center)
        .expect("cold JunctionN compilation must retain reusable topology")
        .clone();
    let snapshot = surface.clone();
    assert!(std::sync::Arc::ptr_eq(
        surface
            .compiled_visual_node_topologies
            .get(&center)
            .expect("live topology cache"),
        snapshot
            .compiled_visual_node_topologies
            .get(&center)
            .expect("snapshot topology cache"),
    ));
    assert!(std::sync::Arc::ptr_eq(
        surface
            .compiled_visual_node_earthwork_boundaries
            .get(&center)
            .expect("live earthwork boundary cache"),
        snapshot
            .compiled_visual_node_earthwork_boundaries
            .get(&center)
            .expect("snapshot earthwork boundary cache"),
    ));

    for mouth in &mut changed_input.mouths {
        adjust_profile_heights(&mut mouth.profile);
        adjust_profile_heights(&mut mouth.endpoint_profile);
        adjust_path_heights(&mut mouth.boundary_paths_world);
        adjust_path_heights(&mut mouth.band_start_paths_world);
        adjust_path_heights(&mut mouth.band_end_paths_world);
    }

    let reused = surface
        .compile_visual_node_piece_with_earthwork_boundaries(
            &graph,
            &terrain,
            center,
            &changed_input,
            Some(topology.as_ref()),
        )
        .expect("exact-XZ height-only topology reuse must compile");
    let cold = surface
        .compile_visual_node_piece_with_earthwork_boundaries(
            &graph,
            &terrain,
            center,
            &changed_input,
            None,
        )
        .expect("independent cold compile must succeed");

    assert!(
        reused.rail_topology_reused,
        "non-uniform Y changes with exact canonical XZ keys must reuse rail/contact topology"
    );
    assert!(
        !reused.ownership_reused,
        "non-uniform height changes must recompute ownership because exact carrier-height predicates can change"
    );
    assert_eq!(
        reused.piece, cold.piece,
        "projected height topology must be byte-for-byte identical to a cold compile"
    );
    assert_eq!(
        reused.earthwork_boundaries, cold.earthwork_boundaries,
        "projected and cold boundary provenance must match"
    );
}

#[test]
fn dirty_node_recompile_refreshes_incident_span_sections_for_new_junction() {
    let mut graph = RegionGraph::new();
    let left = graph.add_node(Vector3::new(-24.0, 0.0, 0.0), NodeType::Junction);
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let right = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
    let left_edge = graph.add_edge(test_edge(
        left,
        center,
        vec![Vector3::new(-24.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        right,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(96, 96);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let up = graph.add_node(Vector3::new(0.0, 0.0, 24.0), NodeType::Junction);
    let up_edge = graph.add_edge(test_edge(
        center,
        up,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 24.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();

    surface.mark_node_dirty(&graph, center);
    surface.mark_node_dirty(&graph, up);
    surface.mark_edge_dirty(&graph, up_edge);
    surface.compile_dirty(&graph, &terrain);

    let edge = graph.edge(left_edge);
    let total_length: f32 = edge
        .geometry
        .windows(2)
        .map(|pair| pair[0].distance_to(pair[1]))
        .sum();
    let start_kind = surface.classify_surface_node_kind_from_graph_geometry(
        &graph,
        graph.get_valid_node(edge.start_node),
    );
    let end_kind = surface.classify_surface_node_kind_from_graph_geometry(
        &graph,
        graph.get_valid_node(edge.end_node),
    );
    let (_, expected_handoff_s) = surface
        .visual_edge_mouth_policy_for_edge(
            &graph,
            left_edge,
            edge,
            total_length,
            start_kind,
            end_kind,
            false,
            false,
        )
        .ownership_range
        .expect("left edge should have a visible span range after pairwise handoff");
    let local_end_handoff_m = edge
        .end_clip
        .max(RoadSurfaceSystem::visual_node_handoff_limit_m(edge))
        .clamp(0.0, total_length);
    let local_handoff_s = (total_length - local_end_handoff_m).clamp(0.0, total_length);
    assert!(
        expected_handoff_s < local_handoff_s - SAMPLE_EPSILON_M,
        "pairwise node ownership must extend the visual handoff before the old local limit"
    );
    let sections = surface.compiled_sections().get(&left_edge).unwrap();
    assert!(
        sections
            .iter()
            .any(|section| (section.s_m - expected_handoff_s).abs() <= SAMPLE_EPSILON_M),
        "dirty node recompilation must refresh incident span sections at the new visual handoff; expected_s={expected_handoff_s:.3} sections={:?}",
        sections
            .iter()
            .map(|section| section.s_m)
            .collect::<Vec<_>>()
    );
}

#[test]
fn four_way_junction_input_keeps_graph_mouths_when_span_piece_is_missing() {
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let west = graph.add_node(Vector3::new(-36.0, 0.0, 0.0), NodeType::Junction);
    let east = graph.add_node(Vector3::new(36.0, 0.0, 0.0), NodeType::Junction);
    let north = graph.add_node(Vector3::new(0.0, 0.0, -36.0), NodeType::Junction);
    let south = graph.add_node(Vector3::new(0.0, 0.0, 36.0), NodeType::Junction);

    graph.add_edge(test_edge(
        west,
        center,
        vec![Vector3::new(-36.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        east,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(36.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        north,
        center,
        vec![Vector3::new(0.0, 0.0, -36.0), Vector3::new(0.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        south,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 36.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(96, 96);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let missing_span_edge = *graph
        .node_adjacency(center)
        .first()
        .expect("four-way center must have at least one incident edge");
    surface
        .compiled_visual_span_pieces
        .remove(&missing_span_edge);

    let input = surface
        .visual_node_compile_input(&graph, center)
        .expect("four-way center must still produce a JunctionN input");

    assert_eq!(input.kind, RoadSurfaceVisualNodePieceKind::JunctionN);
    assert_eq!(
        input.mouths.len(),
        4,
        "a missing span cache entry must not drop a graph incident edge"
    );
    assert!(
        input
            .mouths
            .iter()
            .any(|mouth| mouth.edge_idx == missing_span_edge),
        "the edge with the missing span piece must be represented by a fallback mouth"
    );
    assert!(
        surface
            .compile_visual_node_piece_from_input(&graph, &terrain, center, &input)
            .is_some(),
        "fallback mouths must remain usable by the canonical node compiler"
    );
}

#[test]
fn dirty_recompile_expanded_arbitrary_node_piece_compiles_with_explicit_height_carriers() {
    let terrain = flat_terrain(192, 192);
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    for angle_degrees in [35.0_f32, 158.0, 276.0] {
        let angle = angle_degrees.to_radians();
        let endpoint = Vector3::new(angle.cos() * 88.0, 0.0, angle.sin() * 88.0);
        let node = graph.add_node(endpoint, NodeType::Junction);
        graph.add_edge(test_edge(
            center,
            node,
            vec![Vector3::new(0.0, 0.0, 0.0), endpoint],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
    }
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(4.0);
    surface.compile_dirty(&graph, &terrain);

    let angle = 318.0_f32.to_radians();
    let endpoint = Vector3::new(angle.cos() * 88.0, 0.0, angle.sin() * 88.0);
    let new_node = graph.add_node(endpoint, NodeType::Junction);
    let new_edge = graph.add_edge(test_edge(
        center,
        new_node,
        vec![Vector3::new(0.0, 0.0, 0.0), endpoint],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();

    surface.mark_node_dirty(&graph, center);
    surface.mark_node_dirty(&graph, new_node);
    for &edge_idx in graph.node_adjacency(center) {
        surface.mark_edge_dirty(&graph, edge_idx);
    }
    surface.mark_edge_dirty(&graph, new_edge);
    surface.compile_dirty(&graph, &terrain);

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "expanded arbitrary JunctionN did not compile: {}",
            canonical_junction_pipeline_report(&surface, &graph, center)
        );
    }
    assert_compiled_junction_piece(&surface, &graph, center);

    let mut cold = RoadSurfaceSystem::new(4.0);
    cold.compile_dirty(&graph, &terrain);
    assert_eq!(
        surface.compiled_visual_node_pieces().get(&center),
        cold.compiled_visual_node_pieces().get(&center),
        "incrementally expanded JunctionN output must match an independent cold compile"
    );
    assert_eq!(
        surface
            .compiled_visual_node_earthwork_boundaries
            .get(&center),
        cold.compiled_visual_node_earthwork_boundaries.get(&center),
        "incrementally expanded JunctionN boundary provenance must match a cold compile"
    );
}

#[test]
fn carrier_provenance_closure_five_hundred_meter_multi_junction_edit_compiles() {
    let terrain = TerrainSystem::with_chunking(1025, 1025, 1.0, 512, 0.0);
    let mut graph = RegionGraph::new();
    let node_positions = [
        Vector3::new(-99.096, 164.529, -202.255),
        Vector3::new(213.395, 149.180, 167.508),
        Vector3::new(-44.146, 159.327, -137.233),
        Vector3::new(-20.282, 153.579, -258.083),
        Vector3::new(1.727, 151.822, -82.953),
        Vector3::new(-42.936, 144.308, -43.106),
        Vector3::new(45.419, 143.379, -31.253),
        Vector3::new(135.812, 141.427, -21.842),
        Vector3::new(112.629, 148.295, 48.275),
        Vector3::new(38.920, 143.849, 61.654),
        Vector3::new(162.304, 147.082, 107.054),
        Vector3::new(225.642, 146.702, 77.249),
    ];
    let nodes = node_positions.map(|point| graph.add_node(point, NodeType::Junction));
    for (start, end) in [
        (0, 2),
        (2, 4),
        (4, 6),
        (6, 8),
        (8, 10),
        (10, 1),
        (2, 3),
        (4, 5),
        (6, 7),
        (8, 9),
        (10, 11),
    ] {
        let start_node = nodes[start];
        let end_node = nodes[end];
        graph.add_edge(test_edge(
            start_node,
            end_node,
            vec![node_positions[start], node_positions[end]],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
    }
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    for junction in [nodes[2], nodes[4], nodes[6], nodes[8], nodes[10]] {
        let piece = surface
            .compiled_visual_node_pieces()
            .get(&junction)
            .unwrap_or_else(|| {
                panic!(
                    "500 m multi-junction edit must compile every 3-way node: {}",
                    canonical_junction_pipeline_report(&surface, &graph, junction)
                )
            });
        assert_eq!(piece.kind, RoadSurfaceVisualNodePieceKind::JunctionN);
        assert_node_piece_uses_band_owned_regions(piece);
        assert_node_earthwork_faces_have_footprint_provenance(piece);
    }
}

#[test]
fn dirty_recompile_removes_node_from_previous_chunks_after_topology_shrink() {
    let terrain = flat_terrain(192, 192);
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let west = graph.add_node(Vector3::new(-64.0, 0.0, 0.0), NodeType::Junction);
    let east = graph.add_node(Vector3::new(64.0, 0.0, 0.0), NodeType::Junction);
    let north = graph.add_node(Vector3::new(0.0, 0.0, 64.0), NodeType::Junction);
    graph.add_edge(test_edge(
        west,
        center,
        vec![Vector3::new(-64.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        east,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(64.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    let removed_edge = graph.add_edge(test_edge(
        center,
        north,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 64.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(2.0);
    surface.compile_dirty(&graph, &terrain);
    let previous_node_chunks = surface
        .surface_node_chunks
        .get(&center)
        .expect("three-way node must own chunks before shrink")
        .clone();
    assert!(
        previous_node_chunks.len() > 1,
        "test requires node coverage wide enough to prove stale chunk removal"
    );

    graph.edges[removed_edge].deleted = true;
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();
    surface.mark_edge_dirty(&graph, removed_edge);
    surface.mark_node_dirty(&graph, center);
    surface.compile_dirty(&graph, &terrain);

    let new_node_chunks = surface
        .surface_node_chunks
        .get(&center)
        .cloned()
        .unwrap_or_default();
    let removed_chunks: Vec<SurfaceChunkKey> = previous_node_chunks
        .into_iter()
        .filter(|chunk| !new_node_chunks.contains(chunk))
        .collect();
    assert!(
        !removed_chunks.is_empty(),
        "topology shrink must remove at least one old node-owned chunk"
    );
    for chunk in removed_chunks {
        if let Some(entry) = surface.surface_chunk_cache.get(&chunk) {
            assert!(
                !entry.node_ids.contains(&center),
                "stale node contributor remained in removed chunk {chunk:?}"
            );
        }
    }
}
