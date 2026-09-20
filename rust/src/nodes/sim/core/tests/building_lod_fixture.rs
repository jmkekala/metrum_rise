// SPDX-License-Identifier: GPL-2.0-only

//! Generated save fixtures for end-to-end Godot building LOD checks and matched rendering
//! benchmarks. World/road/save preparation is deliberately outside all measured paths.

use super::*;

#[test]
#[ignore = "writes generated gameplay LOD saves to METRUM_BUILDING_LOD_FIXTURE_DIR"]
fn generate_building_lod_fixtures() {
    let directory =
        std::env::var("METRUM_BUILDING_LOD_FIXTURE_DIR").expect("set fixture destination");
    std::fs::create_dir_all(&directory).unwrap();
    let mut core = test_core();
    core.time.speed_multiplier = 0.0;
    add_test_border_road(&mut core);
    let assets: Vec<_> = (0..3)
        .map(|index| {
            register_test_asset(
                &mut core.allocator,
                &format!("building.residential.lod_house_{index}"),
                ZoneType::Residential,
            )
        })
        .collect();
    add_test_complete_building(&mut core, assets[0].clone(), ZoneType::Residential);
    let template = core.allocator.buildings[0].clone();
    core.allocator.buildings.clear();
    // Separate strips keep frontage geometry valid without expensive gameplay spawn flows.
    let prototype = core.region_graph.edge(0).clone();
    for row in 0..32 {
        let z = -992.0 + row as f32 * 64.0;
        let a = Vector3::new(-1024.0, 0.0, z);
        let b = Vector3::new(1024.0, 0.0, z);
        let mut edge = prototype.clone();
        edge.start_node = core.region_graph.add_node(a, NodeType::Junction);
        edge.end_node = core.region_graph.add_node(b, NodeType::Junction);
        edge.geometry = vec![a, b];
        edge.physical_geometry = vec![a, b];
        edge.physical_length = 2048.0;
        edge.base_cost = 2048.0;
        let edge_index = core.region_graph.edge_count();
        core.region_graph.add_edge(edge);
        for column in 0..64 {
            let mut building = template.clone();
            building.asset_id = assets[(row * 64 + column) % assets.len()].clone();
            building.edge_idx = edge_index;
            building.frontage_t = (column as f32 + 0.5) / 64.0;
            building.cell_x = column;
            building.side = if column % 2 == 0 { 1 } else { -1 };
            building.width_cells = 2;
            building.depth_cells = 2;
            core.allocator.buildings.push(building);
        }
    }
    core.transit_network
        .lane_system
        .rebuild(&mut core.region_graph);
    core.allocator
        .recompute_derived_transforms(&core.region_graph, &core.zoning)
        .unwrap();
    core.allocator.rebuild_zone_index();
    let save = |core: &SimCore, name: &str| {
        let path = std::path::Path::new(&directory).join(name);
        core.save_game_internal(path.to_str().unwrap(), None)
            .unwrap();
    };
    save(&core, "complete.sqlite");
    core.allocator.buildings[0].construction_total_hours = 4;
    core.allocator.buildings[0].construction_remaining_hours = 2;
    core.allocator.mark_building_deserted(1);
    save(&core, "lifecycle.sqlite");
    core.allocator.buildings.swap_remove(0);
    save(&core, "removed.sqlite");
    eprintln!("building LOD fixtures: {directory}; buildings=2048; assets=3; variants=3");
}
