//! Startup demand family, variant, and rezoning selection tests.

use super::support::*;
use super::*;

#[test]
fn test_startup_demand_residential_family_selection_uses_strip_hash_order() {
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::zoning::ZoningSystem;
    use godot::prelude::Vector3;

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
        .iter()
        .find(|building| building.zone_type == ZoneType::Residential)
        .expect("pioneer demand should place a residential building");
    let profile_runtime_id =
        frontage_profile_runtime_id_for_building(&allocator, residential, &zoning, &graph);
    let expected_asset_id =
        if stable_strip_family_hash(profile_runtime_id, residential.parcel_id, "family_a")
            <= stable_strip_family_hash(profile_runtime_id, residential.parcel_id, "family_b")
        {
            family_a_id
        } else {
            family_b_id
        };

    assert_eq!(residential.asset_id, expected_asset_id);
}

#[test]
fn test_startup_demand_residential_variant_selection_uses_site_hash() {
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::zoning::ZoningSystem;
    use godot::prelude::Vector3;

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
        .iter()
        .find(|building| building.zone_type == ZoneType::Residential)
        .expect("pioneer demand should place a residential building");
    let profile_runtime_id =
        frontage_profile_runtime_id_for_building(&allocator, residential, &zoning, &graph);
    let expected_asset_id =
        if stable_site_variant_hash(profile_runtime_id, residential.parcel_id, &variant_a_id)
            <= stable_site_variant_hash(profile_runtime_id, residential.parcel_id, &variant_b_id)
        {
            variant_a_id
        } else {
            variant_b_id
        };

    assert_eq!(residential.asset_id, expected_asset_id);
}

#[test]
fn test_incompatible_rezoning_enters_pending_redevelopment_before_removal() {
    let (
        mut allocator,
        mut zoning,
        mut agents,
        mut households,
        mut logistics,
        mut network,
        mut graph,
        residential_idx,
    ) = setup_startup_spawn_city_for_rezoning();

    let residential = allocator.buildings[residential_idx].clone();
    paint_zone_rect(
        &mut zoning,
        &graph,
        residential.center_x - 10.0,
        residential.center_y - 10.0,
        residential.center_x + 10.0,
        residential.center_y + 10.0,
        ZoneType::Commercial,
    );

    allocator.tick(
        &mut zoning,
        &mut agents,
        &mut households,
        &mut logistics,
        &mut network,
        &mut graph,
    );

    assert_eq!(allocator.buildings.len(), 2);
    assert!(allocator.buildings[residential_idx].pending_redevelopment);
    assert_eq!(
        allocator.buildings[residential_idx].rezone_grace_days_remaining,
        3
    );

    for _ in 0..3 {
        allocator.tick(
            &mut zoning,
            &mut agents,
            &mut households,
            &mut logistics,
            &mut network,
            &mut graph,
        );
    }

    assert_eq!(allocator.buildings.len(), 1);
    assert!(
        allocator
            .buildings
            .iter()
            .all(|building| building.zone_type != ZoneType::Residential)
    );
}

#[test]
fn test_rezoning_recovery_clears_pending_redevelopment() {
    let (
        mut allocator,
        mut zoning,
        mut agents,
        mut households,
        mut logistics,
        mut network,
        mut graph,
        residential_idx,
    ) = setup_startup_spawn_city_for_rezoning();

    let residential = allocator.buildings[residential_idx].clone();
    paint_zone_rect(
        &mut zoning,
        &graph,
        residential.center_x - 10.0,
        residential.center_y - 10.0,
        residential.center_x + 10.0,
        residential.center_y + 10.0,
        ZoneType::Commercial,
    );
    allocator.tick(
        &mut zoning,
        &mut agents,
        &mut households,
        &mut logistics,
        &mut network,
        &mut graph,
    );

    assert!(allocator.buildings[residential_idx].pending_redevelopment);

    paint_zone_rect(
        &mut zoning,
        &graph,
        residential.center_x - 10.0,
        residential.center_y - 10.0,
        residential.center_x + 10.0,
        residential.center_y + 10.0,
        ZoneType::Residential,
    );
    allocator.tick(
        &mut zoning,
        &mut agents,
        &mut households,
        &mut logistics,
        &mut network,
        &mut graph,
    );

    assert_eq!(allocator.buildings.len(), 2);
    assert!(!allocator.buildings[residential_idx].pending_redevelopment);
    assert_eq!(
        allocator.buildings[residential_idx].rezone_grace_days_remaining,
        0
    );
}
