//! Visible-surface query, debug export, and sync regression tests.

use super::*;

#[test]
fn visible_surface_height_prefers_compiled_roadbed() {
    let mut terrain = TerrainSystem::with_chunking(65, 65, 1.0, 8, 0.0);
    for z in 0..65 {
        for x in 0..65 {
            terrain.set_height(x, z, x as f32 * 0.01);
        }
    }

    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(0.0, 0.0, -16.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(0.0, 0.0, 16.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        start,
        end,
        vec![Vector3::new(0.0, 0.0, -16.0), Vector3::new(0.0, 0.0, 16.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let sampled = surface
        .sample_visible_surface_height(&graph, &terrain, 0.0, 0.0)
        .expect("standard road should own its paved footprint");
    let section = surface
        .compiled_sections()
        .get(&edge_idx)
        .unwrap()
        .iter()
        .min_by(|a, b| a.center_xz.y.abs().total_cmp(&b.center_xz.y.abs()))
        .unwrap();
    let expected = section_height_at_lateral_offset(section, 0.0).unwrap();
    assert!((sampled - expected).abs() <= 0.05);
}

#[test]
fn paved_support_height_matches_grounded_visible_roadbed() {
    let mut terrain = TerrainSystem::with_chunking(65, 65, 1.0, 8, 0.0);
    for z in 0..65 {
        for x in 0..65 {
            terrain.set_height(x, z, x as f32 * 0.01);
        }
    }

    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(0.0, 0.0, -16.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(0.0, 0.0, 16.0), NodeType::Junction);
    graph.add_edge(test_edge(
        start,
        end,
        vec![Vector3::new(0.0, 0.0, -16.0), Vector3::new(0.0, 0.0, 16.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.rebuild_all_earthworks(&graph, &mut terrain);

    let visible_height = surface
        .sample_visible_surface_height(&graph, &terrain, 0.0, 0.0)
        .expect("grounded road should own its paved footprint");
    let support_height = surface
        .sample_paved_support_height(&graph, &terrain, 0.0, 0.0)
        .expect("grounded road should expose paved support clearance");

    assert!(
        (visible_height - support_height).abs() <= 0.05,
        "expected grounded-road integrated support height to match the visible roadbed instead of staying one pavement depth below it: visible_height={visible_height:.3} support_height={support_height:.3}"
    );
}

#[test]
fn visible_surface_height_skips_grounded_terminal_earthwork_margin() {
    let terrain = flat_terrain(97, 97);
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
    graph.add_edge(test_edge(
        center,
        end,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let terminal_piece = surface
        .compiled_visual_node_pieces()
        .get(&center)
        .expect("terminal should compile a visual node piece");
    let inner_point = terminal_piece.outer_boundary_loops[0].points_world[0];
    let outer_point = terminal_piece.earthwork_outer_boundary_loops[0].points_world[0];
    let sample_x = (inner_point.x + outer_point.x) * 0.5;
    let sample_z = (inner_point.z + outer_point.z) * 0.5;

    assert!(
        surface
            .sample_visible_surface_height(&graph, &terrain, sample_x as f32, sample_z as f32)
            .is_none(),
        "grounded standard terminal earthwork margin stays outside visible-surface queries; Rust-generated terrain topology owns the ordinary seam"
    );
}

#[test]
fn visible_surface_height_skips_grounded_span_earthwork_margin() {
    let terrain = flat_terrain(97, 97);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(0.0, 0.0, -24.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(0.0, 0.0, 24.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
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
    let span_piece = surface
        .compiled_visual_span_pieces()
        .get(&edge_idx)
        .expect("standard edge should compile a visual span piece");
    let inner_point = span_piece.outer_boundary_loops[0].points_world[0];
    let outer_point = span_piece.earthwork_outer_boundary_loops[0].points_world[0];
    let sample_x = (inner_point.x + outer_point.x) * 0.5;
    let sample_z = (inner_point.z + outer_point.z) * 0.5;

    assert!(
        surface
            .sample_visible_surface_height(&graph, &terrain, sample_x as f32, sample_z as f32)
            .is_none(),
        "grounded standard span earthwork margin stays outside visible-surface queries; Rust-generated terrain topology owns the ordinary seam"
    );
}

#[test]
fn visible_surface_height_skips_buried_tunnel_midspan() {
    let terrain = flat_terrain(97, 33);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(-24.0, 0.0, 0.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
    graph.add_edge(test_edge(
        start,
        end,
        vec![
            Vector3::new(-24.0, 0.0, 0.0),
            Vector3::new(-10.0, -6.0, 0.0),
            Vector3::new(10.0, -6.0, 0.0),
            Vector3::new(24.0, 0.0, 0.0),
        ],
        10.0,
        EdgeClass::Tunnel,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    assert!(
        surface
            .sample_visible_surface_height(&graph, &terrain, 0.0, 0.0)
            .is_none()
    );
    assert!(
        surface
            .sample_visible_surface_height(&graph, &terrain, -20.0, 0.0)
            .is_some()
    );
}

#[test]
fn visible_surface_height_ignores_non_surface_node_adjacency() {
    let terrain = flat_terrain(97, 97);
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let road_end = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
    graph.add_edge(test_edge(
        center,
        road_end,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    let rail_end = graph.add_node(Vector3::new(0.0, 0.0, 24.0), NodeType::Junction);
    graph.add_edge(test_edge(
        center,
        rail_end,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 24.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Rail,
        TransitFlags::RAIL,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let piece = surface
        .compiled_visual_node_pieces()
        .get(&center)
        .expect("surface node piece should compile from the road adjacency");
    let sample = piece
        .road_surface_polygons
        .iter()
        .chain(&piece.curb_surface_polygons)
        .chain(&piece.sidewalk_surface_polygons)
        .flat_map(|polygon| polygon.triangles_world.iter().copied())
        .map(triangle_centroid_xz)
        .next()
        .expect("compiled node piece should contain visible top-surface triangles");
    assert!(
        surface
            .sample_visible_surface_height(&graph, &terrain, sample.x, sample.y)
            .is_some(),
        "non-surface adjacency must not hide a valid road-owned node surface"
    );
}

#[test]
fn visible_surface_raycast_hits_bridge_before_terrain() {
    let terrain = flat_terrain(97, 33);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(-24.0, 6.0, 0.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(24.0, 6.0, 0.0), NodeType::Junction);
    graph.add_edge(test_edge(
        start,
        end,
        vec![
            Vector3::new(-24.0, 6.0, 0.0),
            Vector3::new(0.0, 6.0, 0.0),
            Vector3::new(24.0, 6.0, 0.0),
        ],
        10.0,
        EdgeClass::Bridge,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let hit = surface
        .raycast_visible_surface(
            &graph,
            &terrain,
            Vector3::new(0.0, 20.0, 0.0),
            Vector3::DOWN,
        )
        .expect("bridge should be hittable by the combined world-surface ray");
    assert!((hit.y - 6.0).abs() <= 0.05);
}

#[test]
fn visible_surface_raycast_hits_road_without_terrain_hit() {
    let terrain = flat_terrain(97, 33);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(-24.0, 6.0, 0.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(24.0, 6.0, 0.0), NodeType::Junction);
    graph.add_edge(test_edge(
        start,
        end,
        vec![
            Vector3::new(-24.0, 6.0, 0.0),
            Vector3::new(0.0, 6.0, 0.0),
            Vector3::new(24.0, 6.0, 0.0),
        ],
        10.0,
        EdgeClass::Bridge,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let hit = surface
        .raycast_visible_surface(&graph, &terrain, Vector3::new(0.0, 2.0, 0.0), Vector3::UP)
        .expect("road-owned visible surface should be hittable even when terrain is not");
    assert!((hit.y - 6.0).abs() <= 0.05);
}

#[test]
fn debug_line_data_exposes_sections_bands_patches_and_earthwork_chunks() {
    let mut terrain = flat_terrain(65, 65);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(0.0, 0.0, -16.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(0.0, 0.0, 16.0), NodeType::Junction);
    graph.add_edge(test_edge(
        start,
        end,
        vec![Vector3::new(0.0, 0.0, -16.0), Vector3::new(0.0, 0.0, 16.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.rebuild_all_earthworks(&graph, &mut terrain);
    let debug = surface.build_debug_line_data(&graph, &terrain);

    assert!(!debug.section_lines.is_empty());
    assert!(!debug.band_lines.is_empty());
    assert!(!debug.piece_boundary_lines.is_empty());
    assert!(!debug.earthwork_chunk_lines.is_empty());
}

#[test]
fn debug_geometry_dump_exposes_edge_sections_and_terrain_samples() {
    let mut terrain = sloped_terrain(65, 65);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(-16.0, 0.0, 0.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(16.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        start,
        end,
        vec![
            Vector3::new(-16.0, -0.8, 0.0),
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(16.0, 0.8, 0.0),
        ],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.rebuild_all_earthworks(&graph, &mut terrain);
    let dump = surface.build_edge_geometry_debug_dump(&graph, &terrain, &[edge_idx]);

    assert!(dump.contains("ROAD_GEOMETRY_DUMP_BEGIN"));
    assert!(dump.contains("\"edge_idx\": 0"));
    assert!(dump.contains("\"geometry_world_precise\""));
    assert!(dump.contains("\"physical_geometry_world\""));
    assert!(dump.contains("\"physical_geometry_world_precise\""));
    assert!(dump.contains("\"sections\""));
    assert!(dump.contains("\"span_ownership\""));
    assert!(dump.contains("\"owned_region_count\""));
    assert!(dump.contains("\"source_band_index\""));
    assert!(dump.contains("\"start_section_index\""));
    assert!(dump.contains("\"span_earthwork_support\""));
    assert!(dump.contains("\"support_region_count\""));
    assert!(dump.contains("\"support_policy\""));
    assert!(dump.contains("\"span_earthwork_face_sources\""));
    assert!(dump.contains("\"source_kind\":\"span_support_boundary\""));
    assert!(dump.contains("\"sourced_earthwork_face_count\""));
    assert!(dump.contains("\"missing_earthwork_face_source_count\":0"));
    assert!(dump.contains("\"span_raised_step_face_sources\""));
    assert!(dump.contains("\"lower_owner\""));
    assert!(dump.contains("\"raised_owner\""));
    assert!(dump.contains("\"terrain_clip_source_edges\""));
    assert!(dump.contains("\"span_projection_diagnostics\""));
    assert!(dump.contains("\"road_projection_matches\":true"));
    assert!(dump.contains("\"curb_projection_matches\":true"));
    assert!(dump.contains("\"sidewalk_projection_matches\":true"));
    assert!(dump.contains("\"earthwork_support_region_count\""));
    assert!(dump.contains("\"raised_step_source_count_matches\":true"));
    assert!(dump.contains("\"source_center_y_m\""));
    assert!(dump.contains("\"visual_center_y_m\""));
    assert!(dump.contains("\"left_outer_margin\""));
    assert!(dump.contains("\"right_outer_margin\""));
    assert!(dump.contains("\"node_compile_status\""));
    assert!(dump.contains("\"compiled\": true"));
    assert!(dump.contains("\"nodes\""));
    assert!(dump.contains("\"road_topology\""));
    assert!(dump.contains("\"sidewalk_topology\""));
    assert!(dump.contains("\"raised_step_face_details\""));
    assert!(dump.contains("\"expected_raised_steps\""));
    assert!(dump.contains("\"source_constraint_count\""));
    assert!(dump.contains("\"final_required_face_count\""));
    assert!(dump.contains("\"missing_required_face_count\""));
    assert!(dump.contains("\"non_exposed_source_constraint_count\""));
    assert!(dump.contains("\"materialization_status\""));
    assert!(dump.contains("\"band_ownership\""));
    assert!(dump.contains("\"height_owner\""));
    assert!(dump.contains("\"node_grade_authority\""));
    assert!(dump.contains("\"decision\":\"source_carrier\""));
    assert!(dump.contains("\"seam_constraints\""));
    assert!(dump.contains("\"material_footprint_coverage\""));
    assert!(dump.contains("\"outer_boundary_top_match\""));
    assert!(dump.contains("\"direct_source_count\""));
    assert!(dump.contains("\"top_surface_source_index\""));
    assert!(dump.contains("\"grade_authority_index\""));
    assert!(dump.contains("\"mouth_seams\""));
    assert!(dump.contains("\"earthwork_face_sources\""));
    assert!(dump.contains("\"source_kind\":\"node_footprint_boundary\""));
    assert!(dump.contains("\"boundary_source\""));
    assert!(dump.contains("\"node_footprint_source_count\""));
    assert!(dump.contains("\"missing_source_count\":0"));
    assert!(dump.contains("\"earthwork_face_top_match\""));
    assert!(dump.contains("ROAD_GEOMETRY_DUMP_END"));
}

#[test]
fn transit_sync_to_terrain_invalidates_compiled_sections() {
    let terrain_before = flat_terrain(65, 65);
    let mut terrain_after = flat_terrain(65, 65);
    for z in 0..terrain_after.height {
        for x in 0..terrain_after.width {
            terrain_after.set_height(x, z, 0.5);
        }
    }

    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(0.0, 0.0, -16.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(0.0, 0.0, 16.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        start,
        end,
        vec![
            Vector3::new(0.0, 0.0, -16.0),
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(0.0, 0.0, 16.0),
        ],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_adjacency_list();

    let mut network = TransitNetwork::new();
    network.road_surface.compile_dirty(&graph, &terrain_before);
    let before_height = network
        .road_surface
        .compiled_sections()
        .get(&edge_idx)
        .unwrap()[1]
        .center_height_m;

    network.sync_to_terrain(&mut graph, &terrain_after);
    assert!(
        graph.edge(edge_idx).geometry[1].y >= 9.5,
        "sync_to_terrain should resample edge geometry from terrain before recompilation"
    );

    network.road_surface.compile_dirty(&graph, &terrain_after);
    let after_height = network
        .road_surface
        .compiled_sections()
        .get(&edge_idx)
        .unwrap()[1]
        .center_height_m;

    assert!(
        after_height >= before_height + 9.5,
        "compiled roadbed cache should be invalidated after terrain sync, got before={before_height:.3} after={after_height:.3}"
    );
}
