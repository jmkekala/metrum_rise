// SPDX-License-Identifier: GPL-2.0-only

//! Startup demand family and variant selection tests.

use super::super::placement::stable_parcel_selection_hash;
use super::support::*;
use super::*;

fn startup_residential_selection(mut allocator: BuildingAllocator) -> (String, u16, u64) {
    register_test_asset(&mut allocator, "base", "b.com.shop", ZoneClass::Commercial);

    let map_cfg = WorldConfig::default();
    let mut zoning = ZoningSystem::new(&map_cfg);
    let mut agents = AgentSystem::new();
    let mut households = HouseholdSystem::new();
    let mut logistics = ShipmentSystem::new();
    let mut graph = RegionGraph::new();
    let mut network = TransitNetwork::new();

    network.add_road(
        &mut graph,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        1,
        1,
        crate::simulation::network::types::EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    graph.set_node_type(0, crate::simulation::network::types::NodeType::Border);
    paint_zone_rect(
        &mut zoning,
        &graph,
        -50.0,
        -50.0,
        45.0,
        50.0,
        ZoneType::Residential,
    );
    paint_zone_rect(
        &mut zoning,
        &graph,
        55.0,
        -50.0,
        150.0,
        50.0,
        ZoneType::Commercial,
    );

    execute_startup_demand_building_pass(
        &mut allocator,
        &mut zoning,
        &mut agents,
        &mut households,
        &mut logistics,
        &mut network,
        &graph,
    );

    let residential = allocator
        .buildings
        .into_iter()
        .find(|building| building.zone_type == ZoneType::Residential)
        .expect("pioneer demand should place a residential building");
    let profile = zoning
        .parcel_by_raw_id(residential.parcel_id)
        .expect("spawned parcel")
        .zone_profile_runtime_id();
    (residential.asset_id, profile, residential.parcel_id)
}

#[test]
fn test_startup_demand_residential_family_selection_uses_parcel_hash_order() {
    let mut allocator = BuildingAllocator::new();
    let family_a_id = register_test_asset_with_family(
        &mut allocator,
        "base",
        "b.res.family_a_variant",
        ZoneClass::Residential,
        Some("family_a"),
    );
    let family_b_id = register_test_asset_with_family(
        &mut allocator,
        "base",
        "b.res.family_b_variant",
        ZoneClass::Residential,
        Some("family_b"),
    );
    let (selected_asset, profile_runtime_id, parcel_id) = startup_residential_selection(allocator);
    let expected_asset_id =
        if stable_parcel_selection_hash(profile_runtime_id, parcel_id, "family_a")
            <= stable_parcel_selection_hash(profile_runtime_id, parcel_id, "family_b")
        {
            family_a_id
        } else {
            family_b_id
        };

    assert_eq!(selected_asset, expected_asset_id);
}

#[test]
fn test_startup_demand_residential_variant_selection_uses_parcel_hash_order() {
    let mut allocator = BuildingAllocator::new();
    let variant_a_id = register_test_asset_with_family(
        &mut allocator,
        "base",
        "b.res.shared_family_a",
        ZoneClass::Residential,
        Some("shared_family"),
    );
    let variant_b_id = register_test_asset_with_family(
        &mut allocator,
        "base",
        "b.res.shared_family_b",
        ZoneClass::Residential,
        Some("shared_family"),
    );
    let (selected_asset, profile_runtime_id, parcel_id) = startup_residential_selection(allocator);
    let expected_asset_id =
        if stable_parcel_selection_hash(profile_runtime_id, parcel_id, &variant_a_id)
            <= stable_parcel_selection_hash(profile_runtime_id, parcel_id, &variant_b_id)
        {
            variant_a_id
        } else {
            variant_b_id
        };

    assert_eq!(selected_asset, expected_asset_id);
}
