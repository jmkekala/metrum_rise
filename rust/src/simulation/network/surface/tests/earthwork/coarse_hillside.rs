//! Coarse-grid and hillside roadbed tests.

use super::*;

#[test]
fn flat_diagonal_10m_grid_keeps_paved_footprint_below_roadbed() {
    let terrain = TerrainSystem::with_chunking(129, 129, 10.0, 8, 0.0);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(-160.0, 0.0, -160.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(160.0, 0.0, 160.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        start,
        end,
        vec![
            Vector3::new(-160.0, 0.0, -160.0),
            Vector3::new(160.0, 0.0, 160.0),
        ],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut terrain = terrain;
    let mut surface = RoadSurfaceSystem::new(128.0);
    surface.rebuild_all_earthworks(&graph, &mut terrain);
    let metrics = measure_max_footprint_overflow(&surface, &graph, edge_idx, &terrain);

    assert!(
        metrics.max_overflow_m <= 0.05,
        "expected a flat 45 degree road on a 10 m grid to keep the paved footprint below the roadbed, got {metrics:?}"
    );
}

#[test]
fn shallow_angle_10m_grid_keeps_paved_footprint_below_roadbed() {
    let mut terrain = coarse_hillside_world_terrain(97, 97, 10.0);
    let points = grounded_polyline_points_from_terrain(
        &terrain,
        Vector2::new(-180.0, 5.0),
        Vector2::new(180.0, 1.0),
        28,
    );

    let mut graph = RegionGraph::new();
    let start = graph.add_node(points[0], NodeType::Junction);
    let end = graph.add_node(*points.last().unwrap(), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        start,
        end,
        points,
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(128.0);
    surface.rebuild_all_earthworks(&graph, &mut terrain);
    let metrics = measure_max_footprint_overflow(&surface, &graph, edge_idx, &terrain);

    assert!(
        metrics.max_overflow_m <= 0.05,
        "expected a shallow-angle road on a 10 m grid to keep the paved footprint below the roadbed, got {metrics:?}"
    );
}

#[test]
fn coarse_10m_hillside_case_keeps_paved_footprint_below_roadbed() {
    let (surface, terrain, graph, edge_idx) = build_coarse_grid_hillside_case(10.0);
    let metrics = measure_max_footprint_overflow(&surface, &graph, edge_idx, &terrain);

    assert!(
        metrics.max_overflow_m <= 0.05,
        "expected the coarse 10 m hillside case to keep the paved footprint below the roadbed, got {metrics:?}"
    );
}

#[test]
fn coarse_5m_hillside_case_stays_below_paved_roadbed_too() {
    let (coarse_surface, coarse_terrain, coarse_graph, coarse_edge_idx) =
        build_coarse_grid_hillside_case(10.0);
    let (fine_surface, fine_terrain, fine_graph, fine_edge_idx) =
        build_coarse_grid_hillside_case(5.0);
    let coarse_metrics = measure_max_footprint_overflow(
        &coarse_surface,
        &coarse_graph,
        coarse_edge_idx,
        &coarse_terrain,
    );
    let fine_metrics =
        measure_max_footprint_overflow(&fine_surface, &fine_graph, fine_edge_idx, &fine_terrain);

    assert!(
        coarse_metrics.max_overflow_m <= 0.05,
        "expected the coarse reference case to stay below the paved roadbed, got coarse={coarse_metrics:?} fine={fine_metrics:?}"
    );
    assert!(
        fine_metrics.max_overflow_m <= 0.05,
        "expected the same hillside case on a 5 m grid to stay below the paved roadbed too, got coarse={coarse_metrics:?} fine={fine_metrics:?}"
    );
}

#[test]
fn grounded_hillside_terrain_outside_paved_footprint_stays_near_source() {
    let mut terrain = TerrainSystem::with_chunking(129, 97, 1.0, 8, 0.0);
    for z in 0..97 {
        for x in 0..129 {
            terrain.set_height(x, z, x as f32 * 0.04);
        }
    }

    let mut graph = RegionGraph::new();
    let grounded_height = terrain.sample_height_world(0.0, 0.0) * crate::config::HEIGHT_SCALE;
    let start = graph.add_node(
        Vector3::new(0.0, grounded_height, -24.0),
        NodeType::Junction,
    );
    let end = graph.add_node(Vector3::new(0.0, grounded_height, 24.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        start,
        end,
        vec![
            Vector3::new(0.0, grounded_height, -24.0),
            Vector3::new(0.0, grounded_height, 24.0),
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
    let (left_outer, right_outer) = outer_surface_lateral_bounds(section).unwrap();

    let side_a_lateral = left_outer - 2.0;
    let side_b_lateral = right_outer + 2.0;
    let side_a_x = section.center_xz.x + section.lateral_xz.x * side_a_lateral;
    let side_a_z = section.center_xz.y + section.lateral_xz.y * side_a_lateral;
    let side_b_x = section.center_xz.x + section.lateral_xz.x * side_b_lateral;
    let side_b_z = section.center_xz.y + section.lateral_xz.y * side_b_lateral;
    let side_a_actual =
        terrain.sample_visual_height_world(side_a_x, side_a_z) * crate::config::HEIGHT_SCALE;
    let side_b_actual =
        terrain.sample_visual_height_world(side_b_x, side_b_z) * crate::config::HEIGHT_SCALE;
    let side_a_source =
        terrain.sample_height_world(side_a_x, side_a_z) * crate::config::HEIGHT_SCALE;
    let side_b_source =
        terrain.sample_height_world(side_b_x, side_b_z) * crate::config::HEIGHT_SCALE;
    assert!(
        (side_a_actual - side_a_source).abs() <= 0.12,
        "expected terrain outside the paved footprint to remain near source on hillside side A, got actual={side_a_actual:.3} source={side_a_source:.3}"
    );
    assert!(
        (side_b_actual - side_b_source).abs() <= 0.12,
        "expected terrain outside the paved footprint to remain near source on hillside side B, got actual={side_b_actual:.3} source={side_b_source:.3}"
    );

    let far_side_a_lateral = left_outer - EARTHWORK_MAX_MARGIN_M - 6.0;
    let far_side_b_lateral = right_outer + EARTHWORK_MAX_MARGIN_M + 6.0;
    let far_side_a_x = section.center_xz.x + section.lateral_xz.x * far_side_a_lateral;
    let far_side_a_z = section.center_xz.y + section.lateral_xz.y * far_side_a_lateral;
    let far_side_b_x = section.center_xz.x + section.lateral_xz.x * far_side_b_lateral;
    let far_side_b_z = section.center_xz.y + section.lateral_xz.y * far_side_b_lateral;
    let far_side_a_actual = terrain.sample_visual_height_world(far_side_a_x, far_side_a_z)
        * crate::config::HEIGHT_SCALE;
    let far_side_b_actual = terrain.sample_visual_height_world(far_side_b_x, far_side_b_z)
        * crate::config::HEIGHT_SCALE;
    let far_side_a_source =
        terrain.sample_height_world(far_side_a_x, far_side_a_z) * crate::config::HEIGHT_SCALE;
    let far_side_b_source =
        terrain.sample_height_world(far_side_b_x, far_side_b_z) * crate::config::HEIGHT_SCALE;

    assert!((far_side_a_actual - far_side_a_source).abs() <= 0.12);
    assert!((far_side_b_actual - far_side_b_source).abs() <= 0.12);
}
