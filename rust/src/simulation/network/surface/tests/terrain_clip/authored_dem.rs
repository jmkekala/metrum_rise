// SPDX-License-Identifier: GPL-2.0-only

//! Production road-surface to terrain-CDT agreement tests on authored DEM-like terrain.

use super::*;

#[test]
fn authored_dem_span_tie_ins_preserve_production_terrain_agreement() {
    assert_production_dem_cases(vec![
        standard_span_dem_case(
            "supportive authored cross-slope span",
            planar_world_terrain(161, 161, 1.0, 18.0, 0.025, 0.018),
            Vector2::new(-40.0, -18.0),
            Vector2::new(40.0, 18.0),
            0.0,
            20,
            (-24.0, -18.0, 24.0, 18.0),
            false,
            false,
        ),
        standard_span_dem_case(
            "standard span running along a steep authored slope",
            planar_world_terrain(181, 181, 1.0, 20.0, 0.18, 0.0),
            Vector2::new(-44.0, -12.0),
            Vector2::new(44.0, -12.0),
            0.0,
            24,
            (-28.0, -26.0, 28.0, 2.0),
            false,
            false,
        ),
        standard_span_dem_case(
            "standard span crossing an extreme authored cross-slope",
            planar_world_terrain(181, 181, 1.0, 14.0, 0.0, 0.22),
            Vector2::new(-44.0, 0.0),
            Vector2::new(44.0, 0.0),
            0.0,
            24,
            (-28.0, -18.0, 28.0, 18.0),
            false,
            true,
        ),
        standard_span_dem_case(
            "raised standard span over extreme authored terrain",
            planar_world_terrain(161, 161, 1.0, 0.0, 0.0, 0.0),
            Vector2::new(-32.0, 0.0),
            Vector2::new(32.0, 0.0),
            3.0,
            16,
            (-24.0, -18.0, 24.0, 18.0),
            false,
            true,
        ),
    ]);
}

#[test]
fn authored_dem_bend_and_junction_keep_node_footprint_sources_through_cdt() {
    let terrain = authored_ridge_valley_terrain(181, 181, 1.0);
    let center = Vector2::new(0.0, 0.0);
    let (graph, center_node) =
        standard_node_graph_with_offset_roads(&terrain, center, 2.0, &[(-36.0, 0.0), (0.0, 36.0)]);

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    assert_eq!(
        surface
            .compiled_visual_node_pieces()
            .get(&center_node)
            .unwrap()
            .kind,
        RoadSurfaceVisualNodePieceKind::Bend
    );
    assert_production_dem_case(ProductionDemCase {
        name: "raised bend over authored ridge-valley terrain",
        terrain,
        graph,
        surface,
        bounds: (-52.0, -18.0, 18.0, 52.0),
        sample_step_m: 2.0,
        expect_retaining_wall: false,
        expect_widened_tie_in: true,
        expected_node_piece: None,
    });

    let terrain = authored_ridge_valley_terrain(181, 181, 1.0);
    let (graph, surface, _, terminal_node) = compile_standard_span_with_nodes_on_terrain(
        &terrain,
        Vector2::new(-44.0, -18.0),
        Vector2::new(10.0, -18.0),
        1.5,
        16,
    );
    assert_production_dem_case(ProductionDemCase {
        name: "raised terminal near authored ridge-valley terrain",
        terrain,
        graph,
        surface,
        bounds: (-8.0, -42.0, 34.0, 6.0),
        sample_step_m: 2.0,
        expect_retaining_wall: false,
        expect_widened_tie_in: true,
        expected_node_piece: Some((terminal_node, RoadSurfaceVisualNodePieceKind::Terminal)),
    });

    assert_production_dem_cases(vec![
        standard_node_dem_case(
            "raised three-way junction over authored flat DEM",
            flat_terrain(181, 181),
            center,
            3.0,
            &[(-40.0, 0.0), (40.0, 0.0), (0.0, 40.0)],
            (-56.0, -24.0, 56.0, 56.0),
            false,
            true,
            RoadSurfaceVisualNodePieceKind::JunctionN,
        ),
        standard_node_dem_case(
            "raised four-way junction over authored ridge-valley terrain",
            authored_ridge_valley_terrain(181, 181, 1.0),
            center,
            3.0,
            &[(-40.0, 0.0), (40.0, 0.0), (0.0, -40.0), (0.0, 40.0)],
            (-56.0, -56.0, 56.0, 56.0),
            false,
            true,
            RoadSurfaceVisualNodePieceKind::JunctionN,
        ),
    ]);
}

#[test]
fn authored_dem_junction_edit_order_does_not_change_terrain_cdt_output() {
    let center = Vector2::new(0.0, 0.0);
    let first = standard_node_dem_case(
        "raised four-way junction over authored ridge-valley terrain",
        authored_ridge_valley_terrain(181, 181, 1.0),
        center,
        3.0,
        &[(-40.0, 0.0), (40.0, 0.0), (0.0, -40.0), (0.0, 40.0)],
        (-56.0, -56.0, 56.0, 56.0),
        false,
        true,
        RoadSurfaceVisualNodePieceKind::JunctionN,
    );
    let reordered = standard_node_dem_case(
        "raised four-way junction over authored ridge-valley terrain with reordered edits",
        authored_ridge_valley_terrain(181, 181, 1.0),
        center,
        3.0,
        &[(0.0, 40.0), (0.0, -40.0), (40.0, 0.0), (-40.0, 0.0)],
        (-56.0, -56.0, 56.0, 56.0),
        false,
        true,
        RoadSurfaceVisualNodePieceKind::JunctionN,
    );

    let first_mesh = assert_production_dem_case(first);
    let reordered_mesh = assert_production_dem_case(reordered);
    assert_eq!(
        first_mesh.stats, reordered_mesh.stats,
        "reordered authored-road edits changed terrain-CDT diagnostics"
    );
    assert_eq!(
        canonical_emitted_face_set(&first_mesh),
        canonical_emitted_face_set(&reordered_mesh),
        "reordered authored-road edits changed emitted terrain topology"
    );
}

fn authored_ridge_valley_terrain(width: usize, height: usize, cell_size_m: f32) -> TerrainSystem {
    let mut terrain = TerrainSystem::with_chunking(width, height, cell_size_m, 8, 0.0);
    for z in 0..height {
        for x in 0..width {
            let (world_x, world_z) = terrain.grid_to_world_coords(x, z);
            let ridge_dx = world_x - 8.0;
            let valley_dz = world_z + 18.0;
            let ridge = 4.5 * (-(ridge_dx * ridge_dx) / (2.0 * 8.0 * 8.0)).exp();
            let valley = -3.0 * (-(valley_dz * valley_dz) / (2.0 * 10.0 * 10.0)).exp();
            let base = 7.0 + world_x * 0.02 - world_z * 0.015;
            terrain.set_height(x, z, (base + ridge + valley) / crate::config::HEIGHT_SCALE);
        }
    }
    terrain
}
