//! Building allocator unit tests.

use super::*;
use crate::assets::asset::{BuildingData, LodEntry, ZoneClass};
use crate::assets::AssetManifest;
use crate::simulation::core::config::MapConfig;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::households::HouseholdSystem;
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::grid::zoning::ZoneType;
use godot::prelude::Vector2;
use rand::SeedableRng;

/// Registers a minimal 1×1 building asset for the given zone type so placement tests pass.
fn register_test_asset(allocator: &mut BuildingAllocator, pack_id: &str, asset_id: &str, zone: ZoneClass) {
    let manifest = AssetManifest {
        asset_id: asset_id.to_owned(),
        display_name: "Test".to_owned(),
        asset_set: None,
        tags: vec![],
        thumbnail: None,
        lods: vec![LodEntry { file: "lod0.glb".to_owned(), distance_min_m: 0.0, distance_max_m: None }],
        anchors: vec![],
        building: Some(BuildingData {
            zone_type: zone,
            density: "low".to_owned(),
            lot_width_cells: 1,
            lot_depth_cells: 1,
            level: 1,
            residents_capacity: Some(6),
            worker_capacity: None,
            service_class: None,
            economy_profile: None,
            preview_scale: None,
        }),
        prop: None,
        vehicle: None,
        character: None,
        pivot_offset: None,
    };
    allocator.registry.register(pack_id, manifest, String::new());
}

#[test]
fn test_zone_index_consistency() {
    let mut allocator = BuildingAllocator::new();
    let mut rng = rand::rngs::StdRng::seed_from_u64(42);

    for i in 0..10 {
        allocator.buildings.push(Building {
            center_x: i as f32,
            center_y: 0.0,
            width_cells: 3,
            depth_cells: 3,
            zone_type: if i % 2 == 0 {
                ZoneType::Residential
            } else {
                ZoneType::Commercial
            },
            facing_dir: Vector2::new(0.0, 1.0),
            frontage_t: 0.5,
            side_offset: 0.0,
            abandoned_timer: 0,
            edge_idx: 0,
            side: 1,
            cell_x: i,
            cell_y: 0,
            occupancy: 0,
            worker_count: 0,
            asset_id: String::new(), level: 1,
            broken: false,
            stock: 0.0,
            revenue: 0.0,
            operating_budget: 500.0,
            utility_service_available: false,
            shipment_cooldown_days: 0,
        });
    }
    allocator.dirty_index = true;
    allocator.rebuild_zone_index();

    assert_eq!(
        allocator.zone_index[ZoneType::Residential as usize].len(),
        5
    );
    assert_eq!(allocator.zone_index[ZoneType::Commercial as usize].len(), 5);

    allocator.buildings.swap_remove(0);
    allocator.dirty_index = true;
    allocator.rebuild_zone_index();

    assert_eq!(allocator.buildings.len(), 9);
    assert_eq!(
        allocator.zone_index[ZoneType::Residential as usize].len(),
        4
    );
    assert_eq!(allocator.zone_index[ZoneType::Commercial as usize].len(), 5);

    let pick = allocator.get_random_building_by_zone(ZoneType::Commercial, &mut rng);
    assert!(pick.is_some());
    assert_eq!(
        allocator.buildings[pick.unwrap()].zone_type,
        ZoneType::Commercial
    );
}

#[test]
fn test_vacancy_index_consistency() {
    let mut allocator = BuildingAllocator::new();
    let _rng = rand::rngs::StdRng::seed_from_u64(42);

    for i in 0..5 {
        allocator.buildings.push(Building {
            center_x: i as f32,
            center_y: 0.0,
            width_cells: 3,
            depth_cells: 3,
            zone_type: ZoneType::Residential,
            facing_dir: Vector2::new(0.0, 1.0),
            frontage_t: 0.5,
            side_offset: 0.0,
            abandoned_timer: 0,
            edge_idx: 0,
            side: 1,
            cell_x: i,
            cell_y: 0,
            occupancy: 0,
            worker_count: 0,
            asset_id: String::new(), level: 1,
            broken: false,
            stock: 0.0,
            revenue: 0.0,
            operating_budget: 500.0,
            utility_service_available: false,
            shipment_cooldown_days: 0,
        });
    }
    allocator.rebuild_zone_index();

    assert_eq!(
        allocator.vacancy_index[ZoneType::Residential as usize].len(),
        5
    );

    allocator.claim_vacancy(0);
    allocator.claim_vacancy(0);
    allocator.claim_vacancy(0);
    allocator.claim_vacancy(0);
    allocator.claim_vacancy(0);
    assert_eq!(
        allocator.vacancy_index[ZoneType::Residential as usize].len(),
        5
    );
    allocator.claim_vacancy(0);

    assert_eq!(
        allocator.vacancy_index[ZoneType::Residential as usize].len(),
        4
    );
    assert!(!allocator.vacancy_index[ZoneType::Residential as usize].contains(&0));

    allocator.release_vacancy(0);
    assert_eq!(
        allocator.vacancy_index[ZoneType::Residential as usize].len(),
        5
    );
    assert!(allocator.vacancy_index[ZoneType::Residential as usize].contains(&0));

    let mut agents = AgentSystem::new();
    for _ in 0..5 {
        agents.spawn_agent(usize::MAX, 0, 0.0, 0.0, 0, 0.0, 0.0);
    }

    let last_idx = allocator.buildings.len() - 1;
    let i = 1;
    let mut mapping = std::collections::HashMap::new();
    mapping.insert(last_idx, i);
    agents.remap_building_indices(&mapping);

    allocator.buildings.swap_remove(i);
    allocator.rebuild_zone_index();

    assert_eq!(allocator.buildings.len(), 4);
    assert_eq!(
        allocator.zone_index[ZoneType::Residential as usize].len(),
        4
    );
    assert_eq!(
        allocator.vacancy_index[ZoneType::Residential as usize].len(),
        4
    );
}

#[test]
fn test_tick_does_not_auto_spawn_private_buildings_from_zones() {
    use crate::simulation::grid::zoning::ZoningSystem;
    use crate::simulation::network::TransitNetwork;
    use godot::prelude::Vector3;

    let mut allocator = BuildingAllocator::new();
    register_test_asset(&mut allocator, "base", "b.res.house", ZoneClass::Residential);

    let map_cfg = MapConfig::default();
    let mut zoning = ZoningSystem::new(&map_cfg);
    let mut agents = AgentSystem::new();
    let mut households = HouseholdSystem::new();
    let mut network = TransitNetwork::new();
    let mut logistics = ShipmentSystem::new();
    let mut graph = RegionGraph::new();

    network.add_road(
        &mut graph,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        1,
        1,
        crate::simulation::network::types::EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    zoning.set_zone_rect(-50.0, -50.0, 150.0, 50.0, ZoneType::Residential);

    allocator.tick(
        &mut zoning,
        &mut agents,
        &mut households,
        &mut logistics,
        &mut network,
        &mut graph,
    );

    assert_eq!(
        allocator.buildings.len(),
        0,
        "Zoning alone should not create buildings when the founding prerequisites are incomplete"
    );
}

#[test]
fn test_tick_runs_one_time_founding_bootstrap_from_border_and_zoning() {
    use crate::simulation::grid::zoning::ZoningSystem;
    use crate::simulation::network::TransitNetwork;
    use godot::prelude::Vector3;

    let mut allocator = BuildingAllocator::new();
    register_test_asset(&mut allocator, "base", "b.res.house", ZoneClass::Residential);
    register_test_asset(&mut allocator, "base", "b.com.shop", ZoneClass::Commercial);
    let map_cfg = MapConfig::default();
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
    zoning.set_zone_rect(-50.0, -50.0, 45.0, 50.0, ZoneType::Residential);
    zoning.set_zone_rect(55.0, -50.0, 150.0, 50.0, ZoneType::Commercial);

    allocator.tick(
        &mut zoning,
        &mut agents,
        &mut households,
        &mut logistics,
        &mut network,
        &mut graph,
    );

    assert!(allocator.founding_bootstrap_consumed);
    assert_eq!(allocator.buildings.len(), 2);
    assert_eq!(
        allocator
            .buildings
            .iter()
            .filter(|building| building.zone_type == ZoneType::Residential)
            .count(),
        1
    );
    assert_eq!(
        allocator
            .buildings
            .iter()
            .filter(|building| building.zone_type == ZoneType::Commercial)
            .count(),
        1
    );

    allocator.tick(
        &mut zoning,
        &mut agents,
        &mut households,
        &mut logistics,
        &mut network,
        &mut graph,
    );

    assert_eq!(
        allocator.buildings.len(),
        2,
        "Founding bootstrap should only seed one residential and one commercial building"
    );
}

#[test]
fn test_building_removal_clears_zoning_occupancy() {
    use crate::simulation::grid::zoning::ZoningSystem;
    use crate::simulation::network::TransitNetwork;
    use godot::prelude::Vector3;

    let mut allocator = BuildingAllocator::new();
    register_test_asset(&mut allocator, "base", "b.res.house", ZoneClass::Residential);
    let map_cfg = MapConfig::default();
    let mut zoning = ZoningSystem::new(&map_cfg);
    let mut agents = AgentSystem::new();
    let mut households = HouseholdSystem::new();
    let mut network = TransitNetwork::new();
    let mut logistics = ShipmentSystem::new();
    let mut graph = RegionGraph::new();

    network.add_road(
        &mut graph,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        1,
        1,
        crate::simulation::network::types::EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );

    allocator.buildings.push(Building {
        center_x: 5.0,
        center_y: 10.0,
        width_cells: 3,
        depth_cells: 3,
        zone_type: ZoneType::Residential,
        facing_dir: Vector2::new(0.0, 1.0),
        frontage_t: 0.05,
        side_offset: 1.0,
        abandoned_timer: 0,
        edge_idx: 0,
        side: 1,
        cell_x: 0,
        cell_y: 0,
        occupancy: 0,
        worker_count: 0,
        asset_id: String::new(), level: 1,
        broken: false,
        stock: 0.0,
        revenue: 0.0,
        operating_budget: 500.0,
        utility_service_available: false,
        shipment_cooldown_days: 0,
    });
    zoning.mark_occupied_rect(5.0, 10.0, godot::prelude::Vector2::new(0.0, 1.0), 30.0, 30.0, true);

    allocator.tick(
        &mut zoning,
        &mut agents,
        &mut households,
        &mut logistics,
        &mut network,
        &mut graph,
    );

    assert_eq!(
        allocator.buildings.len(),
        0,
        "Building should have been removed"
    );
    assert!(
        !zoning.is_rect_occupied(5.0, 10.0, godot::prelude::Vector2::new(0.0, 1.0), 5.0, 5.0),
        "Zoning occupancy should be cleared after building removal"
    );
}

#[test]
fn test_immigration_claims_vacant_home() {
    use crate::simulation::core::config::MapConfig;
    use crate::simulation::economy::agents::AgentSystem;
    use crate::simulation::grid::zoning::{ZoneType, ZoningSystem};
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::TransitNetwork;
    use godot::prelude::{Vector2, Vector3};

    let mut allocator = BuildingAllocator::new();
    register_test_asset(&mut allocator, "base", "b.res.house", ZoneClass::Residential);
    let map_cfg = MapConfig::default();
    let mut zoning = ZoningSystem::new(&map_cfg);
    let mut agents = AgentSystem::new();
    let mut households = HouseholdSystem::new();
    let mut network = TransitNetwork::new();
    let mut logistics = ShipmentSystem::new();
    let mut graph = RegionGraph::new();

    network.add_road(
        &mut graph,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        1,
        1,
        crate::simulation::network::types::EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    let edge_id = graph.edge_count() - 1;

    graph.set_node_type(0, crate::simulation::network::types::NodeType::Border);
    zoning.set_zone_rect(-50.0, -50.0, 150.0, 50.0, ZoneType::Residential);
    allocator.buildings.push(Building {
        center_x: 10.0,
        center_y: 10.0,
        width_cells: 3,
        depth_cells: 3,
        zone_type: ZoneType::Residential,
        facing_dir: Vector2::new(0.0, 1.0),
        frontage_t: 0.1,
        side_offset: 1.0,
        abandoned_timer: 0,
        edge_idx: edge_id,
        side: 1,
        cell_x: 0,
        cell_y: 0,
        occupancy: 0,
        worker_count: 0,
        asset_id: String::new(), level: 1,
        broken: false,
        stock: 0.0,
        revenue: 0.0,
        operating_budget: 500.0,
        utility_service_available: false,
        shipment_cooldown_days: 0,
    });
    allocator.rebuild_zone_index();

    allocator.tick(
        &mut zoning,
        &mut agents,
        &mut households,
        &mut logistics,
        &mut network,
        &mut graph,
    );

    assert_eq!(agents.len(), 2, "One two-resident household should have immigrated");
    assert_eq!(
        agents.home_building[0], 0,
        "Immigrant should have claimed home index 0"
    );
    assert_eq!(
        agents.target_building[0], 0,
        "Immigrant target_building should be set to home"
    );
    assert_eq!(agents.household_id[0], agents.household_id[1]);
    assert_eq!(households.households.len(), 1);
    assert_eq!(households.households[0].member_count, 2);
    assert_eq!(
        allocator.buildings[0].occupancy, 2,
        "Building occupancy should match the admitted household size"
    );
}

#[test]
fn test_startup_immigration_floor_avoids_zero_rounding() {
    use godot::prelude::Vector3;

    let mut allocator = BuildingAllocator::new();
    register_test_asset(&mut allocator, "base", "b.res.house", ZoneClass::Residential);
    register_test_asset(&mut allocator, "base", "b.com.shop", ZoneClass::Commercial);
    let mut agents = AgentSystem::new();
    let mut households = HouseholdSystem::new();
    let mut graph = RegionGraph::new();

    let mut zoning = crate::simulation::grid::zoning::ZoningSystem::new(&MapConfig::default());
    let mut network = crate::simulation::network::TransitNetwork::new();
    network.add_road(
        &mut graph,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        1,
        1,
        crate::simulation::network::types::EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    let edge_id = graph.edge_count() - 1;
    graph.set_node_type(0, crate::simulation::network::types::NodeType::Border);

    allocator.buildings.push(Building {
        center_x: 10.0,
        center_y: 10.0,
        width_cells: 2,
        depth_cells: 2,
        zone_type: ZoneType::Residential,
        facing_dir: Vector2::new(0.0, 1.0),
        frontage_t: 0.1,
        side_offset: 1.0,
        abandoned_timer: 0,
        edge_idx: edge_id,
        side: 1,
        cell_x: 0,
        cell_y: 0,
        occupancy: 2,
        worker_count: 0,
        asset_id: "base:b.res.house".to_owned(),
        level: 1,
        broken: false,
        stock: 0.0,
        revenue: 0.0,
        operating_budget: 500.0,
        utility_service_available: true,
        shipment_cooldown_days: 0,
    });
    allocator.buildings.push(Building {
        center_x: 40.0,
        center_y: 10.0,
        width_cells: 2,
        depth_cells: 2,
        zone_type: ZoneType::Commercial,
        facing_dir: Vector2::new(0.0, 1.0),
        frontage_t: 0.4,
        side_offset: 1.0,
        abandoned_timer: 0,
        edge_idx: edge_id,
        side: 1,
        cell_x: 4,
        cell_y: 0,
        occupancy: 0,
        worker_count: 0,
        asset_id: "base:b.com.shop".to_owned(),
        level: 1,
        broken: false,
        stock: 0.0,
        revenue: 0.0,
        operating_budget: 500.0,
        utility_service_available: true,
        shipment_cooldown_days: 0,
    });
    allocator.rebuild_zone_index();

    let household_id = households.admit_immigrant_household(0, 2);
    for _ in 0..2 {
        let idx = agents.spawn_agent(0, 1, 0.0, 0.0, 0, 0.0, 0.0);
        agents.household_id[idx] = household_id;
    }
    households.households[household_id].stock = 0.0;
    households.households[household_id].stock_days = 0.0;

    allocator.spawn_immigrants(&mut agents, &mut households, &graph);

    assert!(
        households.households.len() >= 2,
        "player-seeded startup city should admit another household even when the raw formula rounds to zero"
    );
}
