// SPDX-License-Identifier: GPL-2.0-only

//! Explicit-site and zoning-overlap validation tests.

use super::support::*;
use super::*;

#[test]
fn sloped_frontage_projection_round_trips_the_shared_attachment() {
    let mut graph = RegionGraph::new();
    let mut network = TransitNetwork::new();
    let mut zoning = ZoningSystem::new(&WorldConfig::default());
    let mut allocator = BuildingAllocator::new();
    network.add_road(
        &mut graph,
        vec![
            Vector3::new(0.0, 10.0, 0.0),
            Vector3::new(40.0, 30.0, 0.0),
            Vector3::new(80.0, 10.0, 20.0),
            Vector3::new(120.0, 25.0, 20.0),
        ],
        1,
        1,
        crate::simulation::network::types::EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    for t in [0.1, 0.4, 0.6, 0.9] {
        for side in [-1.0, 1.0] {
            let point = BuildingAllocator::sample_pos_on_edge(&graph, 0, t);
            let tangent = BuildingAllocator::sample_tangent_on_edge(&graph, 0, t);
            let offset = point + Vector2::new(tangent.y, -tangent.x) * side * 5.0;
            let projection =
                BuildingAllocator::project_point_to_edge_centerline(0, graph.edge(0), offset)
                    .unwrap();
            assert!(
                (projection.t - t).abs() < 0.00001,
                "slope changed frontage: {projection:?}, expected {t}"
            );
            assert_eq!(projection.side, side as i8);
            assert!(
                BuildingAllocator::sample_pos_on_edge(&graph, 0, projection.t).distance_to(point)
                    < 0.001
            );
        }
    }
}

#[test]
fn explicit_roadside_site_rejects_missing_surface_without_raw_terrain_fallback() {
    let config = WorldConfig::default();
    let mut zoning = ZoningSystem::new(&config);
    let mut graph = RegionGraph::new();
    let mut network = TransitNetwork::new();
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_power_service_asset(&mut allocator, "test", "power");
    network.add_road(
        &mut graph,
        vec![Vector3::ZERO, Vector3::new(100.0, 0.0, 0.0)],
        1,
        1,
        crate::simulation::network::types::EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = crate::simulation::economy::definitions::load_runtime_economy_tuning().unwrap();
    let terrain = flat_test_terrain();
    let revision = allocator.building_ref_revision();
    let result = allocator.execute_explicit_service_placement(
        &asset,
        Vector2::new(50.0, 20.0),
        config.zone_cell_m,
        &graph,
        &network.road_surface,
        &terrain,
        &catalog,
        &tuning,
    );
    assert_eq!(
        result,
        Err(ExplicitServicePlacementRejection::FrontageRoadSurfaceMissing)
    );
    assert!(allocator.buildings.is_empty() && allocator.building_sites.is_empty());
    assert_eq!(allocator.building_ref_revision(), revision);
    assert!(allocator.take_pending_site_dirty_bounds().is_none());
}

#[test]
fn explicit_placement_survives_transform_rebuild_on_both_road_sides() {
    let config = WorldConfig::default();
    let mut zoning = ZoningSystem::new(&config);
    let mut graph = RegionGraph::new();
    let mut network = TransitNetwork::new();
    let mut allocator = BuildingAllocator::new();
    let asset_id = register_test_power_service_asset(&mut allocator, "base", "building.power.test");
    network.add_road(
        &mut graph,
        vec![Vector3::ZERO, Vector3::new(200.0, 0.0, 0.0)],
        1,
        1,
        crate::simulation::network::types::EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    let terrain = compiled_flat_test_terrain(&mut network, &graph);
    let catalog = load_runtime_economy_catalog().expect("catalog");
    let tuning =
        crate::simulation::economy::definitions::load_runtime_economy_tuning().expect("tuning");
    for z in [-8.0, 8.0] {
        allocator
            .execute_explicit_service_placement(
                &asset_id,
                Vector2::new(170.0, z),
                config.zone_cell_m,
                &graph,
                &network.road_surface,
                &terrain,
                &catalog,
                &tuning,
            )
            .expect("explicit placement");
    }
    let placed = allocator.buildings.clone();
    let sites = allocator.building_sites.clone();
    for building in &mut allocator.buildings {
        assert_eq!(building.parcel_id, 0);
        assert_eq!(building.cell_x, 0);
        assert!(building.frontage_t > 0.8);
        // SQL stores the attachment and support plane, but reconstructs these fields.
        building.center_x = 0.0;
        building.center_y = 0.0;
        building.facing_dir = Vector2::ZERO;
        building.side_offset = 0.0;
    }
    allocator
        .recompute_derived_transforms(&graph, &zoning)
        .unwrap();
    for (idx, expected) in placed.iter().enumerate() {
        let actual = &allocator.buildings[idx];
        assert_eq!(
            (actual.center_x, actual.center_y),
            (expected.center_x, expected.center_y)
        );
        assert_eq!(actual.facing_dir, expected.facing_dir);
        assert_eq!(actual.side_offset, expected.side_offset);
        assert_eq!(actual.support_height_m, expected.support_height_m);
        assert_eq!(
            allocator.building_sites[idx].footprint_world,
            sites[idx].footprint_world
        );
        assert_eq!(
            allocator.building_sites[idx].lot_footprint_world,
            sites[idx].lot_footprint_world
        );
        assert_eq!(
            allocator.building_sites[idx].support_height_m,
            sites[idx].support_height_m
        );
    }
}

#[test]
fn explicit_service_preview_rejects_site_overlapping_nearby_road() {
    let map_cfg = WorldConfig::default();
    let mut zoning = ZoningSystem::new(&map_cfg);
    let mut graph = RegionGraph::new();
    let mut network = TransitNetwork::new();
    let mut allocator = BuildingAllocator::new();
    let asset_id = register_test_power_service_asset(&mut allocator, "base", "building.power.test");

    network.add_road(
        &mut graph,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        1,
        1,
        crate::simulation::network::types::EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    network.add_road(
        &mut graph,
        vec![Vector3::new(0.0, 0.0, 20.0), Vector3::new(100.0, 0.0, 20.0)],
        1,
        1,
        crate::simulation::network::types::EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    let terrain = compiled_flat_test_terrain(&mut network, &graph);
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");

    let preview = allocator
        .preview_explicit_service_placement(
            &asset_id,
            Vector2::new(50.0, 8.0),
            map_cfg.zone_cell_m,
            &graph,
            &network.road_surface,
            &terrain,
            &catalog,
        )
        .expect("service placement should resolve to the nearest frontage");

    assert!(!preview.valid);
    assert_eq!(
        preview.rejection,
        Some(ExplicitServicePlacementRejection::RoadOverlap)
    );
}

#[test]
fn zoning_projection_overlaps_explicit_service_site_is_blocked() {
    let map_cfg = WorldConfig::default();
    let mut zoning = ZoningSystem::new(&map_cfg);
    let mut graph = RegionGraph::new();
    let mut network = TransitNetwork::new();
    let mut allocator = BuildingAllocator::new();
    let asset_id = register_test_power_service_asset(&mut allocator, "base", "building.power.test");

    network.add_road(
        &mut graph,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        1,
        1,
        crate::simulation::network::types::EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    let terrain = compiled_flat_test_terrain(&mut network, &graph);
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let tuning =
        crate::simulation::economy::definitions::load_runtime_economy_tuning().expect("tuning");

    allocator
        .execute_explicit_service_placement(
            &asset_id,
            Vector2::new(50.0, 8.0),
            map_cfg.zone_cell_m,
            &graph,
            &network.road_surface,
            &terrain,
            &catalog,
            &tuning,
        )
        .expect("flat service placement should commit");
    let parcel_geometry = zoning
        .preview_parcel_at(50.0, 8.0, 20.0, 20.0, &graph)
        .expect("zoning projection should still resolve geometrically");

    assert!(
        allocator.parcel_geometry_overlaps_explicit_site(&parcel_geometry),
        "explicit service lot should reserve land against zoning placement"
    );
}
