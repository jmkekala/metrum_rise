// SPDX-License-Identifier: GPL-2.0-only

//! Explicit-site and zoning-overlap validation tests.

use super::support::*;
use super::*;

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
