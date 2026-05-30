//! Temporary road preview regression tests.

use super::*;

#[test]
fn visual_polygon_builder_preserves_skinny_closure_geometry() {
    let polygon = RoadSurfaceSystem::make_visual_polygon(vec![
        backend::RoadVec3::new(0.0, 0.0, 0.0),
        backend::RoadVec3::new(0.15, 0.0, 0.0),
        backend::RoadVec3::new(0.0, 0.0, 0.02),
    ])
    .expect("centimetre-scale curb closure polygons must survive the visual polygon builder");

    assert!(
        !polygon.triangles_world.is_empty(),
        "curb closure polygons must keep renderable CDT triangles"
    );
}

#[test]
fn preview_matches_committed_sections_on_flat_terrain() {
    let terrain = flat_terrain(64, 64);
    let surface = RoadSurfaceSystem::new(16.0);
    let raw_points = vec![Vector3::new(0.0, 0.2, 0.0), Vector3::new(24.0, 0.2, 0.0)];

    let (preview, committed_sections, committed_visual_pieces) =
        compile_committed_preview_reference(&surface, &raw_points, &terrain, 1, 1);

    assert_eq!(preview.edge_class, EdgeClass::Standard);
    assert!(preview.is_valid);
    assert_eq!(preview.compiled_sections, committed_sections);
    assert_eq!(preview.compiled_visual_node_pieces, committed_visual_pieces);
    assert_preview_vertices_use_solved_section_height_keys(&preview);
}

#[test]
fn preview_matches_committed_sections_on_cross_slope() {
    let mut terrain = TerrainSystem::with_chunking(80, 16, 1.0, 8, 0.0);
    for z in 0..16 {
        for x in 0..80 {
            terrain.set_height(x, z, x as f32 * 0.005);
        }
    }
    let surface = RoadSurfaceSystem::new(16.0);
    let y0 = terrain.sample_height_world(-16.0, 0.0) * crate::config::HEIGHT_SCALE + 0.2;
    let y1 = terrain.sample_height_world(0.0, 0.0) * crate::config::HEIGHT_SCALE + 0.2;
    let y2 = terrain.sample_height_world(16.0, 0.0) * crate::config::HEIGHT_SCALE + 0.2;
    let raw_points = vec![
        Vector3::new(-16.0, y0, 0.0),
        Vector3::new(0.0, y1, 0.0),
        Vector3::new(16.0, y2, 0.0),
    ];

    let (preview, committed_sections, committed_visual_pieces) =
        compile_committed_preview_reference(&surface, &raw_points, &terrain, 1, 1);

    assert_eq!(preview.edge_class, EdgeClass::Standard);
    assert!(preview.is_valid);
    assert_eq!(preview.compiled_sections, committed_sections);
    assert_eq!(preview.compiled_visual_node_pieces, committed_visual_pieces);
}

#[test]
fn preview_matches_committed_sections_for_bridges() {
    let terrain = flat_terrain(96, 16);
    let surface = RoadSurfaceSystem::new(16.0);
    let raw_points = vec![
        Vector3::new(0.0, 3.0, 0.0),
        Vector3::new(16.0, 3.0, 0.0),
        Vector3::new(32.0, 3.0, 0.0),
    ];

    let (preview, committed_sections, committed_visual_pieces) =
        compile_committed_preview_reference(&surface, &raw_points, &terrain, 1, 1);

    assert_eq!(preview.edge_class, EdgeClass::Bridge);
    assert!(preview.is_valid);
    assert_eq!(preview.compiled_sections, committed_sections);
    assert_eq!(preview.compiled_visual_node_pieces, committed_visual_pieces);
}

#[test]
fn preview_matches_committed_sections_for_tunnels() {
    let terrain = flat_terrain(96, 16);
    let surface = RoadSurfaceSystem::new(16.0);
    let raw_points = vec![
        Vector3::new(0.0, -3.0, 0.0),
        Vector3::new(16.0, -3.0, 0.0),
        Vector3::new(32.0, -3.0, 0.0),
    ];

    let (preview, committed_sections, committed_visual_pieces) =
        compile_committed_preview_reference(&surface, &raw_points, &terrain, 1, 1);

    assert_eq!(preview.edge_class, EdgeClass::Tunnel);
    assert!(preview.is_valid);
    assert_eq!(preview.compiled_sections, committed_sections);
    assert_eq!(preview.compiled_visual_node_pieces, committed_visual_pieces);
}

#[test]
fn preview_conditioning_preserves_snapped_visible_road_height() {
    let terrain = flat_terrain(96, 64);
    let mut graph = RegionGraph::new();
    let existing_y = 5.0;
    let start = graph.add_node(Vector3::new(0.0, existing_y, -16.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(0.0, existing_y, 16.0), NodeType::Junction);
    graph.add_edge(test_edge(
        start,
        end,
        vec![
            Vector3::new(0.0, existing_y, -16.0),
            Vector3::new(0.0, existing_y, 16.0),
        ],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut existing_surface = RoadSurfaceSystem::new(16.0);
    existing_surface.compile_dirty(&graph, &terrain);
    let visible_y = existing_surface
        .sample_visible_surface_height(&graph, &terrain, 0.0, 0.0)
        .expect("existing elevated standard road must expose visible snap support");
    assert!(
        (visible_y - existing_y).abs() <= 0.05,
        "test setup expected visible support at the elevated road height: visible={visible_y:.3}"
    );

    let raw_points = vec![
        Vector3::new(0.0, visible_y, 0.0),
        Vector3::new(24.0, 0.0, 0.0),
    ];
    let preview_surface = RoadSurfaceSystem::new(16.0);
    let preview = preview_surface.compile_preview_surface_mesh_only_with_existing_surface(
        &raw_points,
        1,
        1,
        &terrain,
        &graph,
        &existing_surface,
    );

    assert_eq!(preview.edge_class, EdgeClass::Standard);
    assert_eq!(preview.prepared_points.len(), 2);
    assert!(
        (preview.prepared_points[0].y - visible_y).abs() <= 0.05,
        "snapped endpoint must keep visible road height instead of source terrain: prepared={:.3} visible={visible_y:.3}",
        preview.prepared_points[0].y
    );
    assert!(
        preview.prepared_points[1].y.abs() <= 0.001,
        "non-road endpoint should still ground to source terrain"
    );
}

#[test]
fn standard_road_footprint_uses_stitched_mesh_instead_of_visual_terrain_stamp() {
    let mut terrain = TerrainSystem::with_chunking(65, 65, 1.0, 8, 0.0);
    for z in 0..65 {
        for x in 0..65 {
            terrain.set_height(x, z, x as f32 * 0.01);
        }
    }

    let mut graph = RegionGraph::new();
    let grounded_height = terrain.sample_height_world(0.0, 0.0) * crate::config::HEIGHT_SCALE;
    let start = graph.add_node(
        Vector3::new(0.0, grounded_height, -16.0),
        NodeType::Junction,
    );
    let end = graph.add_node(Vector3::new(0.0, grounded_height, 16.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        start,
        end,
        vec![
            Vector3::new(0.0, grounded_height, -16.0),
            Vector3::new(0.0, grounded_height, 16.0),
        ],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.rebuild_all_earthworks(&graph, &mut terrain);

    let sections = surface.compiled_sections().get(&edge_idx).unwrap();
    let section = sections
        .iter()
        .min_by(|a, b| a.center_xz.y.abs().total_cmp(&b.center_xz.y.abs()))
        .unwrap();
    for lateral_offset in [-4.0_f32, 0.0, 4.0] {
        let road_height = section_height_at_lateral_offset(section, lateral_offset).unwrap();
        let sample_x = section.center_xz.x + section.lateral_xz.x * f64::from(lateral_offset);
        let sample_z = section.center_xz.y + section.lateral_xz.y * f64::from(lateral_offset);
        let source_height = terrain.sample_height_world(sample_x as f32, sample_z as f32)
            * crate::config::HEIGHT_SCALE;
        let visual_height = terrain.sample_visual_height_world(sample_x as f32, sample_z as f32)
            * crate::config::HEIGHT_SCALE;
        let support_height = surface
            .sample_paved_support_height(&graph, &terrain, sample_x as f32, sample_z as f32)
            .expect("standard paved footprint should expose a solved support surface");
        assert!(
            (visual_height - source_height).abs() <= 0.05,
            "ordinary standard roads must not stamp visual terrain at lateral_offset={lateral_offset:.1}: visual={visual_height:.3} source={source_height:.3}"
        );
        assert!(
            (support_height - road_height).abs() <= 0.05,
            "expected solved paved support to match the compiled road surface at lateral_offset={lateral_offset:.1}: support={support_height:.3} road_height={road_height:.3}"
        );
    }
}
